//! Headless audio/demod producer: turns a tuned virtual channel's complex baseband into
//! demodulated mono PCM [`AudioFrame`]s, fanned out to any number of network clients.
//!
//! All modes consume complex baseband samples that the [`crate::channelizer::Channelizer`]
//! has already NCO-mixed and lowpass-decimated to center on the tuned frequency — this
//! pipeline only ever does the final demodulation step, never any tuning or channel
//! selection of its own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use num_complex::Complex32;

use ez_proto::{AudioFrame, ChannelId, DemodMode};

use crate::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};
use crate::bus::{SampleBlock, SampleBusHandle};

/// Typical narrowband FM deviation (analog voice: NOAA weather radio, ham FM, PMR/GMRS).
const NFM_DEVIATION_HZ: f32 = 5_000.0;
/// Typical wideband (broadcast) FM deviation.
const WFM_DEVIATION_HZ: f32 = 75_000.0;
/// Pole of the one-pole DC blocker applied after AM envelope detection: closer to 1.0 tracks
/// slower drift and preserves more low-frequency audio content.
const AM_DC_BLOCKER_POLE: f32 = 0.999;
/// Upper bound accepted by [`AudioPipeline::set_volume`] — enough headroom for a weak
/// signal without letting a client silently drive the output into extreme gain.
const MAX_VOLUME: f32 = 4.0;
/// Default squelch threshold (dBFS of mean input power): low enough that squelch is
/// effectively disabled until a client raises it, matching [`SpectrumPipeline`]'s own
/// noise-floor convention for "quiet" (`crate::pipelines::spectrum`).
const DEFAULT_SQUELCH_DB: f32 = -120.0;

#[derive(Debug, Clone, Copy)]
pub struct AudioConfig {
    pub channel_id: ChannelId,
    pub mode: DemodMode,
}

/// Demodulates one virtual channel's baseband into audio. Carries whatever per-sample state
/// its current mode needs (FM discriminator's previous sample, AM's DC-blocker history)
/// across both blocks and ticks, so switching a client's demod mode mid-stream is the only
/// thing that resets it (see [`Self::set_mode`]) — arriving in oddly-sized blocks never
/// introduces a discontinuity.
pub struct AudioPipeline {
    input: SampleBusHandle,
    channel_id: ChannelId,
    mode: DemodMode,
    dc_prev_x: f32,
    dc_prev_y: f32,
    prev_sample: Complex32,
    volume: f32,
    squelch_db: f32,
    output: Broadcaster<AudioFrame>,
}

impl AudioPipeline {
    #[must_use]
    pub fn new(input: SampleBusHandle, config: AudioConfig) -> Self {
        Self {
            input,
            channel_id: config.channel_id,
            mode: config.mode,
            dc_prev_x: 0.0,
            dc_prev_y: 0.0,
            prev_sample: Complex32::new(0.0, 0.0),
            volume: 1.0,
            squelch_db: DEFAULT_SQUELCH_DB,
            output: Broadcaster::new(),
        }
    }

    /// Switches demod mode, resetting carried discriminator/DC-blocker state so the old
    /// mode's history can never leak a click or transient into the new mode's first samples.
    pub fn set_mode(&mut self, mode: DemodMode) {
        if mode != self.mode {
            self.mode = mode;
            self.dc_prev_x = 0.0;
            self.dc_prev_y = 0.0;
            self.prev_sample = Complex32::new(0.0, 0.0);
        }
    }

    #[must_use]
    pub fn mode(&self) -> DemodMode {
        self.mode
    }

    /// Sets output gain applied after demodulation, clamped to `[0, MAX_VOLUME]`. `0.0`
    /// mutes without stopping the pipeline (state, e.g. the FM discriminator's carried
    /// previous sample, keeps advancing) so unmuting resumes cleanly with no discontinuity.
    pub fn set_volume(&mut self, level: f32) {
        self.volume = level.clamp(0.0, MAX_VOLUME);
    }

    #[must_use]
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Sets the squelch threshold in dBFS: a block whose mean input power falls below this
    /// is still demodulated (to keep carried state continuous) but published as silence.
    pub fn set_squelch(&mut self, db: f32) {
        self.squelch_db = db;
    }

    #[must_use]
    pub fn squelch_db(&self) -> f32 {
        self.squelch_db
    }

    #[must_use]
    pub fn subscribe(&self, capacity: usize) -> BroadcasterHandle<AudioFrame> {
        self.output.subscribe(capacity, OverflowPolicy::DropOldest)
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.output.subscriber_count()
    }

    /// Waits up to `poll_timeout` for the next input block and, if one arrived, demodulates
    /// and publishes it. Returns whether a block was actually consumed — callers driving a
    /// shutdown-aware loop should check an external stop signal on `false` rather than
    /// treating it as "source gone" (see [`SampleBusHandle::recv_timeout`]'s doc comment).
    pub fn tick(&mut self, poll_timeout: Duration) -> bool {
        let Some(block) = self.input.recv_timeout(poll_timeout) else {
            return false;
        };
        self.process_block(&block);
        true
    }

    /// Drives `tick` in a loop until `running` is set to `false`.
    pub fn run(&mut self, running: &AtomicBool) {
        while running.load(Ordering::Relaxed) {
            self.tick(Duration::from_millis(100));
        }
    }

    fn process_block(&mut self, block: &SampleBlock) {
        let mut samples: Vec<f32> = match self.mode {
            DemodMode::Raw => block.samples.iter().map(|s| s.re).collect(),
            DemodMode::Am => self.demod_am(&block.samples),
            DemodMode::Fm => self.demod_fm(&block.samples, block.sample_rate_hz, NFM_DEVIATION_HZ),
            DemodMode::Wfm => self.demod_fm(&block.samples, block.sample_rate_hz, WFM_DEVIATION_HZ),
            // Product detection: once the channelizer has quadrature-downconverted the
            // tuned sideband to baseband, Re{2*sample} recovers the audio for EITHER
            // sideband identically — USB vs. LSB is entirely a matter of which side of the
            // carrier the channel's center frequency was placed on, not a difference in
            // this formula. See the module tests for a worked demonstration.
            DemodMode::Usb | DemodMode::Lsb => {
                block.samples.iter().map(|s| (2.0 * s.re).clamp(-1.0, 1.0)).collect()
            }
        };

        // Squelch gates on the block's own mean input power so it reacts to the RF signal
        // actually present, not to whatever the demodulator happened to output for it
        // (e.g. FM's discriminator is meaningless on noise alone). Carried demod state
        // above has already advanced against the real input either way, so un-squelching
        // never reintroduces a discontinuity.
        let mean_power: f32 =
            block.samples.iter().map(|s| s.norm_sqr()).sum::<f32>() / block.samples.len().max(1) as f32;
        let power_db = if mean_power > 1e-12 { 10.0 * mean_power.log10() } else { -240.0 };
        if power_db < self.squelch_db {
            samples.iter_mut().for_each(|s| *s = 0.0);
        } else if self.volume != 1.0 {
            samples.iter_mut().for_each(|s| *s = (*s * self.volume).clamp(-1.0, 1.0));
        }

        self.output.publish(AudioFrame {
            channel_id: self.channel_id,
            sample_rate_hz: block.sample_rate_hz,
            samples,
        });
    }

    fn demod_am(&mut self, samples: &[Complex32]) -> Vec<f32> {
        samples
            .iter()
            .map(|s| {
                let envelope = s.norm();
                let y = envelope - self.dc_prev_x + AM_DC_BLOCKER_POLE * self.dc_prev_y;
                self.dc_prev_x = envelope;
                self.dc_prev_y = y;
                y.clamp(-1.0, 1.0)
            })
            .collect()
    }

    fn demod_fm(&mut self, samples: &[Complex32], sample_rate_hz: u32, deviation_hz: f32) -> Vec<f32> {
        let gain = sample_rate_hz as f32 / (2.0 * std::f32::consts::PI * deviation_hz);
        samples
            .iter()
            .map(|&s| {
                let d = s * self.prev_sample.conj();
                self.prev_sample = s;
                (d.arg() * gain).clamp(-1.0, 1.0)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{OverflowPolicy as BusOverflowPolicy, SampleBus};
    use std::sync::Arc;

    fn block_of(samples: Vec<Complex32>, sample_rate_hz: u32) -> SampleBlock {
        SampleBlock {
            start_sample: 0,
            sample_rate_hz,
            center_freq_hz: 100_000_000,
            samples: Arc::from(samples),
        }
    }

    fn tone(n: usize, freq_hz: f64, sample_rate_hz: f64) -> Vec<Complex32> {
        (0..n)
            .map(|k| {
                let phase = 2.0 * std::f64::consts::PI * freq_hz * (k as f64) / sample_rate_hz;
                Complex32::from_polar(1.0, phase as f32)
            })
            .collect()
    }

    fn pipeline_with(mode: DemodMode) -> (SampleBus, AudioPipeline) {
        let bus = SampleBus::new();
        let handle = bus.subscribe(8, BusOverflowPolicy::DropIncoming);
        let pipeline = AudioPipeline::new(
            handle,
            AudioConfig {
                channel_id: 7,
                mode,
            },
        );
        (bus, pipeline)
    }

    #[test]
    fn raw_mode_passes_through_the_i_channel() {
        let (bus, mut ap) = pipeline_with(DemodMode::Raw);
        let out = ap.subscribe(4);

        bus.publish(block_of(vec![Complex32::new(0.5, 0.25), Complex32::new(-0.3, 0.9)], 48_000));
        ap.tick(Duration::from_millis(50));

        let frame = out.try_recv().expect("expected a frame");
        assert_eq!(frame.samples, vec![0.5, -0.3]);
        assert_eq!(frame.channel_id, 7);
        assert_eq!(frame.sample_rate_hz, 48_000);
    }

    #[test]
    fn am_envelope_settles_near_zero_under_a_constant_carrier() {
        let (bus, mut ap) = pipeline_with(DemodMode::Am);
        let out = ap.subscribe(4);

        // Constant-envelope carrier (varying phase, |s| == 1 always): once the DC blocker
        // settles, a truly constant envelope should demodulate to ~silence. The blocker's
        // pole (0.999) gives it a ~1000-sample time constant, so this needs several
        // thousand samples of runway before the tail is a fair "has it settled" check.
        bus.publish(block_of(tone(10_000, 1_000.0, 48_000.0), 48_000));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");

        let tail_rms = {
            let tail = &frame.samples[frame.samples.len() - 200..];
            (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
        };
        assert!(tail_rms < 0.05, "constant envelope should settle near silence, got rms {tail_rms}");
    }

    #[test]
    fn am_envelope_responds_to_an_amplitude_step() {
        let (bus, mut ap) = pipeline_with(DemodMode::Am);
        let out = ap.subscribe(4);

        let mut samples = vec![Complex32::new(0.2, 0.0); 500]; // low envelope, let it settle
        samples.extend(vec![Complex32::new(1.0, 0.0); 50]); // step up in envelope
        bus.publish(block_of(samples, 48_000));
        ap.tick(Duration::from_millis(50));

        let frame = out.try_recv().expect("expected a frame");
        let just_after_step = frame.samples[502];
        assert!(just_after_step > 0.1, "expected a positive swing right after the envelope step, got {just_after_step}");
    }

    #[test]
    fn fm_discriminator_recovers_constant_offset_as_constant_level() {
        let sample_rate = 48_000.0;
        let offset = 2_000.0; // well within NFM's 5 kHz deviation assumption
        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        let out = ap.subscribe(4);

        bus.publish(block_of(tone(1_000, offset, sample_rate), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");

        let expected = (offset / NFM_DEVIATION_HZ as f64) as f32;
        // Skip sample 0 (discriminator has no prior sample yet, carried state starts at 0).
        for &s in &frame.samples[1..] {
            assert!((s - expected).abs() < 0.01, "sample {s} far from expected constant level {expected}");
        }
    }

    #[test]
    fn fm_discriminator_state_is_continuous_across_ticks() {
        let sample_rate = 48_000.0;
        let offset = 1_500.0;
        let full_tone = tone(400, offset, sample_rate);

        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        let out = ap.subscribe(8);

        bus.publish(block_of(full_tone[..200].to_vec(), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let first = out.try_recv().expect("first frame");

        bus.publish(block_of(full_tone[200..].to_vec(), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let second = out.try_recv().expect("second frame");

        // The discriminator's very first output (sample 0 of the whole run) is the only one
        // allowed to be off (no prior sample yet); every other sample, including the first
        // of the second block, should reflect the same constant offset continuously.
        let expected = (offset / NFM_DEVIATION_HZ as f64) as f32;
        assert!((second.samples[0] - expected).abs() < 0.01, "discontinuity at block boundary: {}", second.samples[0]);
        assert!((first.samples[199] - expected).abs() < 0.01);
    }

    #[test]
    fn fm_output_is_clamped_to_unit_range_under_excessive_deviation() {
        let sample_rate = 48_000.0;
        // Offset far beyond NFM's assumed deviation drives the raw discriminator output
        // well past +-1 before clamping.
        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        let out = ap.subscribe(4);

        bus.publish(block_of(tone(200, 20_000.0, sample_rate), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");

        assert!(frame.samples.iter().all(|&s| (-1.0..=1.0).contains(&s)));
    }

    #[test]
    fn usb_and_lsb_apply_the_identical_product_detector() {
        let samples = tone(500, 300.0, 48_000.0);

        let (bus_u, mut usb) = pipeline_with(DemodMode::Usb);
        let out_u = usb.subscribe(4);
        bus_u.publish(block_of(samples.clone(), 48_000));
        usb.tick(Duration::from_millis(50));
        let frame_u = out_u.try_recv().expect("usb frame");

        let (bus_l, mut lsb) = pipeline_with(DemodMode::Lsb);
        let out_l = lsb.subscribe(4);
        bus_l.publish(block_of(samples, 48_000));
        lsb.tick(Duration::from_millis(50));
        let frame_l = out_l.try_recv().expect("lsb frame");

        assert_eq!(frame_u.samples, frame_l.samples);
    }

    #[test]
    fn set_mode_resets_carried_state_without_panicking() {
        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        let out = ap.subscribe(4);

        bus.publish(block_of(tone(200, 5_000.0, 48_000.0), 48_000));
        ap.tick(Duration::from_millis(50));
        out.try_recv().expect("fm frame");

        ap.set_mode(DemodMode::Am);
        assert_eq!(ap.mode(), DemodMode::Am);
        bus.publish(block_of(tone(200, 5_000.0, 48_000.0), 48_000));
        ap.tick(Duration::from_millis(50));
        let am_frame = out.try_recv().expect("am frame after mode switch");
        assert_eq!(am_frame.samples.len(), 200);

        ap.set_mode(DemodMode::Fm);
        bus.publish(block_of(tone(200, 5_000.0, 48_000.0), 48_000));
        ap.tick(Duration::from_millis(50));
        let fm_frame = out.try_recv().expect("fm frame after switching back");
        // First sample after a reset always reflects the zeroed carried state (see
        // demod_fm), independent of whatever the discriminator saw before the switch.
        assert_eq!(fm_frame.samples[0], 0.0);
    }

    #[test]
    fn tick_times_out_cleanly_when_idle() {
        let (_bus, mut ap) = pipeline_with(DemodMode::Raw);
        assert!(!ap.tick(Duration::from_millis(20)));
    }

    #[test]
    fn run_exits_promptly_when_running_flag_clears() {
        let (_bus, mut ap) = pipeline_with(DemodMode::Raw);
        let running = AtomicBool::new(false);
        ap.run(&running); // must return immediately, not hang
    }

    #[test]
    fn subscriber_count_reflects_subscriptions() {
        let (_bus, ap) = pipeline_with(DemodMode::Raw);
        assert_eq!(ap.subscriber_count(), 0);
        let a = ap.subscribe(4);
        assert_eq!(ap.subscriber_count(), 1);
        drop(a);
        assert_eq!(ap.subscriber_count(), 0);
    }

    #[test]
    fn set_volume_clamps_to_the_documented_range() {
        let (_bus, mut ap) = pipeline_with(DemodMode::Raw);
        ap.set_volume(-1.0);
        assert_eq!(ap.volume(), 0.0);
        ap.set_volume(1000.0);
        assert_eq!(ap.volume(), MAX_VOLUME);
        ap.set_volume(1.5);
        assert_eq!(ap.volume(), 1.5);
    }

    #[test]
    fn volume_scales_raw_output_and_stays_clamped_to_unit_range() {
        let (bus, mut ap) = pipeline_with(DemodMode::Raw);
        ap.set_volume(2.0);
        let out = ap.subscribe(4);

        bus.publish(block_of(vec![Complex32::new(0.5, 0.0), Complex32::new(-0.3, 0.0)], 48_000));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");
        // 0.5 * 2.0 == 1.0 exactly; -0.3 * 2.0 == -0.6 (no clamping needed).
        assert_eq!(frame.samples, vec![1.0, -0.6]);
    }

    #[test]
    fn default_squelch_passes_a_normal_signal_through() {
        let (bus, mut ap) = pipeline_with(DemodMode::Raw);
        let out = ap.subscribe(4);

        bus.publish(block_of(tone(100, 1_000.0, 48_000.0), 48_000));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");
        assert!(frame.samples.iter().any(|&s| s != 0.0), "default squelch should not silence a real signal");
    }

    #[test]
    fn raising_squelch_above_signal_power_silences_output() {
        let (bus, mut ap) = pipeline_with(DemodMode::Raw);
        ap.set_squelch(1.0); // unit-amplitude tone sits at ~0 dBFS, so 1.0 dB clearly gates it
        assert_eq!(ap.squelch_db(), 1.0);
        let out = ap.subscribe(4);

        bus.publish(block_of(tone(100, 1_000.0, 48_000.0), 48_000));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame even when squelched");
        assert!(frame.samples.iter().all(|&s| s == 0.0), "squelch above signal power should silence output");
    }

    #[test]
    fn squelch_gates_output_without_discarding_discriminator_continuity() {
        // Regression guard: squelching must zero the OUTPUT, not skip demodulation, so FM
        // carried state (prev_sample) keeps advancing and un-squelching is glitch-free. Uses
        // one continuous tone sliced in two (not two independently-phased tones), exactly
        // like fm_discriminator_state_is_continuous_across_ticks, since prev_sample
        // continuity is only meaningful against a genuinely continuous signal.
        let sample_rate = 48_000.0;
        let offset = 2_000.0;
        let full_tone = tone(400, offset, sample_rate);
        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        ap.set_squelch(100.0); // above any signal power this tone can reach: always gated
        let out = ap.subscribe(4);

        bus.publish(block_of(full_tone[..200].to_vec(), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");
        assert!(frame.samples.iter().all(|&s| s == 0.0));

        ap.set_squelch(DEFAULT_SQUELCH_DB);
        bus.publish(block_of(full_tone[200..].to_vec(), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let unsquelched = out.try_recv().expect("expected a frame");
        let expected = (offset / NFM_DEVIATION_HZ as f64) as f32;
        // Had prev_sample not kept advancing under squelch, sample 0 here would show the
        // same "no prior sample" reset artifact tested in
        // fm_discriminator_state_is_continuous_across_ticks instead of the steady tone level.
        assert!((unsquelched.samples[0] - expected).abs() < 0.01);
    }
}
