//! Headless audio/demod producer: turns a tuned virtual channel's complex baseband into
//! demodulated mono PCM [`AudioFrame`]s, fanned out to any number of network clients.
//!
//! All modes consume complex baseband samples that the [`crate::channelizer::Channelizer`]
//! has already NCO-mixed and lowpass-decimated to center on the tuned frequency.
//! Mode-specific complex RF selection precedes detection and squelch measurement.
//! Filters and oscillators use the exact channel clock; PCM can use an independent
//! output clock without changing the physical capture provenance.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use num_complex::Complex32;

use ez_proto::{AudioFrame, AudioParameters, ChannelId, DemodMode};

use super::audio_filter::ComplexLowpass;
use crate::audio_rate::IqAudioConverter;
use crate::audio_rf::RfSelector;
use crate::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};
use crate::bus::{SampleBlock, SampleBusHandle, SampleContinuity};

/// Typical narrowband FM deviation (analog voice: NOAA weather radio, ham FM, PMR/GMRS).
const NFM_DEVIATION_HZ: f32 = 5_000.0;
/// Typical wideband (broadcast) FM deviation.
const WFM_DEVIATION_HZ: f32 = 75_000.0;
/// Pole of the one-pole DC blocker applied after AM envelope detection: closer to 1.0 tracks
/// slower drift and preserves more low-frequency audio content.
const AM_DC_BLOCKER_POLE: f32 = 0.999;
/// Broadcast FM de-emphasis time constant (75 µs standard, e.g. US/ITU).
const WFM_DEEMPHASIS_TAU: f32 = 75e-6;
/// Upper bound accepted by [`AudioPipeline::set_volume`] — enough headroom for a weak
/// signal without letting a client silently drive the output into extreme gain.
const MAX_VOLUME: f32 = 4.0;
/// Default squelch threshold (dBFS of mean input power): low enough that squelch is
/// effectively disabled until a client raises it, matching [`SpectrumPipeline`]'s own
/// noise-floor convention for "quiet" (`crate::pipelines::spectrum`).
const DEFAULT_SQUELCH_DB: f32 = -120.0;
const SSB_SHIFT_HZ: f32 = 1_650.0;
const SSB_LOWPASS_HZ: f32 = 1_450.0;
// At the 1,650 Hz sideband center, voice at 300..3,000 Hz lies within
// +/-1,350 Hz. The 36th-order response is flat within 0.1 dB there and rejects
// the opposite sideband from +/-1,950 Hz by at least 80 dB at production rates.

#[derive(Debug, Clone, Copy)]
pub struct AudioConfig {
    pub channel_id: ChannelId,
    pub mode: DemodMode,
    pub audio: Option<AudioParameters>,
}

/// Demodulates one virtual channel's baseband into audio. Carries whatever per-sample state
/// its current mode needs (FM discriminator's previous sample, AM's DC-blocker history)
/// across continuous blocks and ticks. Capture gaps, retunes and mode changes reset it;
/// merely arriving in oddly-sized blocks never introduces a discontinuity.
pub struct AudioPipeline {
    input: SampleBusHandle,
    continuity: SampleContinuity,
    channel_id: ChannelId,
    mode: DemodMode,
    parameters: Option<AudioParameters>,
    rf_selector: Option<(f64, u32, RfSelector)>,
    converter: IqAudioConverter,
    audio_clock: Option<ez_proto::CaptureClock>,
    audio_epoch: u64,
    dc_prev_x: f32,
    dc_prev_y: f32,
    prev_sample: Complex32,
    deemph_prev: f32,
    ssb_phase: f64,
    ssb_filter: ComplexLowpass,
    volume: f32,
    squelch_db: f32,
    squelch_revision: u64,
    sequence: u64,
    reset_pending: bool,
    output: Broadcaster<AudioFrame>,
}

impl AudioPipeline {
    #[must_use]
    pub fn new(input: SampleBusHandle, config: AudioConfig) -> Self {
        Self {
            input,
            continuity: SampleContinuity::default(),
            channel_id: config.channel_id,
            mode: config.mode,
            parameters: config.audio,
            rf_selector: None,
            converter: IqAudioConverter::for_detector(),
            audio_clock: None,
            audio_epoch: 0,
            dc_prev_x: 0.0,
            dc_prev_y: 0.0,
            prev_sample: Complex32::new(0.0, 0.0),
            deemph_prev: 0.0,
            ssb_phase: 0.0,
            ssb_filter: ComplexLowpass::default(),
            volume: 1.0,
            squelch_db: DEFAULT_SQUELCH_DB,
            squelch_revision: 0,
            sequence: 0,
            reset_pending: true,
            output: Broadcaster::new(),
        }
    }

    /// Switches demod mode, resetting carried discriminator/DC-blocker state so the old
    /// mode's history can never leak a click or transient into the new mode's first samples.
    pub fn set_mode(&mut self, mode: DemodMode) {
        if mode != self.mode {
            self.mode = mode;
            if let Some(parameters) = &mut self.parameters {
                parameters.rf_bandwidth_hz = AudioParameters::for_mode(mode).rf_bandwidth_hz;
            }
            self.reset_history();
        }
    }

    /// Apply already-validated RF and PCM settings as one DSP boundary.
    pub fn configure(&mut self, mode: DemodMode, parameters: AudioParameters) {
        if self.mode != mode || self.parameters != Some(parameters) {
            self.mode = mode;
            self.parameters = Some(parameters);
            self.reset_history();
        }
    }

    pub fn validate_parameters(
        mode: DemodMode,
        parameters: AudioParameters,
        channel_rate_hz: f64,
    ) -> Result<(), String> {
        if !channel_rate_hz.is_finite() || !(100.0..=20_000_000.0).contains(&channel_rate_hz) {
            return Err("audio channel rate must be finite and between 100 Hz and 20 MHz".into());
        }
        if parameters.output_sample_rate_hz != 0
            && !(8_000..=384_000).contains(&parameters.output_sample_rate_hz)
        {
            return Err(
                "audio output rate must be 8..384 kS/s or zero to follow the channel clock".into(),
            );
        }
        if matches!(mode, DemodMode::Lsb | DemodMode::Usb) {
            if parameters.rf_bandwidth_hz == 0
                || f64::from(parameters.rf_bandwidth_hz) > 0.44 * channel_rate_hz
            {
                return Err(
                    "SSB one-sided RF extent must fit within 44% of the channel clock".into(),
                );
            }
        } else if mode != DemodMode::Raw || parameters.rf_bandwidth_hz != 0 {
            RfSelector::validate(channel_rate_hz, parameters.rf_bandwidth_hz)?;
        }
        Ok(())
    }

    fn reset_history(&mut self) {
        self.reset_pending = true;
        self.dc_prev_x = 0.0;
        self.dc_prev_y = 0.0;
        self.prev_sample = Complex32::new(0.0, 0.0);
        self.deemph_prev = 0.0;
        self.ssb_phase = 0.0;
        self.ssb_filter.reset();
        if let Some((_, _, selector)) = &mut self.rf_selector {
            selector.reset();
        }
        self.converter.reset();
        self.audio_clock = None;
        self.audio_epoch = self.audio_epoch.wrapping_add(1);
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

    /// Sets the squelch threshold in dBFS: a block whose selected RF power falls below this
    /// is still demodulated (to keep carried state continuous) but published as silence.
    pub fn set_squelch(&mut self, db: f32) {
        if self.squelch_db != db {
            self.squelch_revision = self.squelch_revision.wrapping_add(1);
        }
        self.squelch_db = db;
    }

    #[must_use]
    pub fn squelch_db(&self) -> f32 {
        self.squelch_db
    }

    #[must_use]
    pub fn squelch_state(&self) -> ez_proto::SquelchState {
        ez_proto::SquelchState {
            db: self.squelch_db,
            revision: self.squelch_revision,
        }
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
        if !self.continuity.observe(block) {
            self.reset_history();
        }
        let mut clock = block.capture_clock();
        let rate = clock.exact_sample_rate_hz();
        let parameters = self.parameters.unwrap_or_else(|| {
            let mut p = AudioParameters::for_mode(self.mode);
            let fraction = if matches!(self.mode, DemodMode::Usb | DemodMode::Lsb) {
                0.44
            } else {
                0.8
            };
            p.rf_bandwidth_hz = p.rf_bandwidth_hz.min((fraction * rate) as u32);
            p.output_sample_rate_hz = 0;
            p
        });
        if Self::validate_parameters(self.mode, parameters, rate).is_err() {
            self.reset_history();
            return;
        }
        let selected = if matches!(self.mode, DemodMode::Usb | DemodMode::Lsb) {
            self.select_ssb(
                &block.samples,
                rate,
                self.mode == DemodMode::Usb,
                parameters.rf_bandwidth_hz,
            )
        } else if parameters.rf_bandwidth_hz == 0 {
            block
                .samples
                .iter()
                .map(|s| {
                    Complex32::new(
                        if s.re.is_finite() { s.re } else { 0.0 },
                        if s.im.is_finite() { s.im } else { 0.0 },
                    )
                })
                .collect()
        } else {
            if self
                .rf_selector
                .as_ref()
                .is_none_or(|(old_rate, old_width, _)| {
                    *old_rate != rate || *old_width != parameters.rf_bandwidth_hz
                })
            {
                let Ok(filter) = RfSelector::new(rate, parameters.rf_bandwidth_hz) else {
                    return;
                };
                self.rf_selector = Some((rate, parameters.rf_bandwidth_hz, filter));
            }
            self.rf_selector.as_mut().unwrap().2.process(&block.samples)
        };
        let mut samples: Vec<f32> = match self.mode {
            DemodMode::Raw => selected.iter().map(|s| s.re).collect(),
            DemodMode::Am => self.demod_am(&selected),
            DemodMode::Fm => self.demod_fm(&selected, rate, NFM_DEVIATION_HZ),
            DemodMode::Wfm => {
                let mut raw = self.demod_fm(&selected, rate, WFM_DEVIATION_HZ);
                self.apply_deemphasis(&mut raw, rate);
                raw
            }
            DemodMode::Usb | DemodMode::Lsb => selected.iter().map(|s| 2.0 * s.re).collect(),
        };
        // Measure the same selected complex signal that feeds detection. An
        // excluded neighboring carrier cannot open power squelch.
        let mean_power: f64 = selected
            .iter()
            .map(|s| f64::from(s.re).powi(2) + f64::from(s.im).powi(2))
            .sum::<f64>()
            / selected.len().max(1) as f64;
        let power_db = if mean_power > 0.0 {
            10.0 * mean_power.log10()
        } else {
            -240.0
        };
        let discontinuous = std::mem::take(&mut self.reset_pending);
        clock.discontinuous |= discontinuous;
        let (audio_clock, sample_rate_hz) = if parameters.output_sample_rate_hz == 0 {
            (clock, block.sample_rate_hz)
        } else {
            let input: Vec<_> = samples.iter().map(|&x| Complex32::new(x, 0.0)).collect();
            samples = self
                .converter
                .process_clocked(
                    &input,
                    clock.source_sample_rate_hz,
                    clock.sample_stride,
                    parameters.output_sample_rate_hz,
                    1.0,
                )
                .into_iter()
                .map(|x| x[0])
                .collect();
            let mut output_clock = *self.audio_clock.get_or_insert_with(|| {
                ez_proto::CaptureClock::new(
                    self.audio_epoch,
                    0,
                    parameters.output_sample_rate_hz,
                    clock.unix_time_ns(0),
                )
            });
            output_clock.discontinuous = discontinuous;
            self.audio_clock = output_clock
                .offset_samples(samples.len() as u64)
                .map(|mut c| {
                    c.discontinuous = false;
                    c
                });
            (output_clock, parameters.output_sample_rate_hz)
        };
        for sample in &mut samples {
            *sample =
                if self.squelch_db > DEFAULT_SQUELCH_DB && power_db < f64::from(self.squelch_db) {
                    0.0
                } else {
                    (*sample * self.volume).clamp(-1.0, 1.0)
                };
        }
        self.sequence += 1;
        self.output.publish(AudioFrame {
            clock,
            audio_clock,
            sequence: self.sequence,
            squelch: self.squelch_state(),
            channel_id: self.channel_id,
            sample_rate_hz,
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
                y
            })
            .collect()
    }

    fn demod_fm(
        &mut self,
        samples: &[Complex32],
        sample_rate_hz: f64,
        deviation_hz: f32,
    ) -> Vec<f32> {
        let gain = (sample_rate_hz / (std::f64::consts::TAU * f64::from(deviation_hz))) as f32;
        samples
            .iter()
            .map(|&s| {
                let d = s * self.prev_sample.conj();
                self.prev_sample = s;
                d.arg() * gain
            })
            .collect()
    }

    fn apply_deemphasis(&mut self, samples: &mut [f32], sample_rate_hz: f64) {
        if sample_rate_hz <= 0.0 {
            return;
        }
        let dt = 1.0 / sample_rate_hz;
        let alpha = (dt / (f64::from(WFM_DEEMPHASIS_TAU) + dt)) as f32;
        for s in samples.iter_mut() {
            self.deemph_prev += alpha * (*s - self.deemph_prev);
            *s = self.deemph_prev;
        }
    }

    fn select_ssb(
        &mut self,
        samples: &[Complex32],
        sample_rate_hz: f64,
        usb: bool,
        bandwidth_hz: u32,
    ) -> Vec<Complex32> {
        let scale = f64::from(bandwidth_hz) / 3300.0;
        let phase_step =
            std::f64::consts::TAU * f64::from(SSB_SHIFT_HZ) * scale / sample_rate_hz.max(1.0);
        let mixer_sign = if usb { -1.0 } else { 1.0 };
        let mut output = Vec::with_capacity(samples.len());
        for &sample in samples {
            let (sin, cos) = self.ssb_phase.sin_cos();
            let mixed = Complex32::new(
                (f64::from(sample.re) * cos - mixer_sign * f64::from(sample.im) * sin) as f32,
                (mixer_sign * f64::from(sample.re) * sin + f64::from(sample.im) * cos) as f32,
            );
            let selected =
                self.ssb_filter
                    .process(mixed, f64::from(SSB_LOWPASS_HZ) * scale, sample_rate_hz);
            // Restore the RF translation before real projection: filtering must
            // select the wanted sideband without shifting every voice tone.
            let audio = f64::from(selected.re) * cos + mixer_sign * f64::from(selected.im) * sin;
            let imaginary =
                f64::from(selected.im) * cos - mixer_sign * f64::from(selected.re) * sin;
            output.push(Complex32::new(audio as f32, imaginary as f32));
            self.ssb_phase = (self.ssb_phase + phase_step).rem_euclid(std::f64::consts::TAU);
        }
        output
    }
}

#[cfg(test)]
#[path = "audio_selection_tests.rs"]
mod selection_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{OverflowPolicy as BusOverflowPolicy, SampleBus};
    use std::sync::Arc;

    fn block_of(samples: Vec<Complex32>, sample_rate_hz: u32) -> SampleBlock {
        SampleBlock {
            product_session: 0,
            observed_at: None,
            capture: None,
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
                audio: None,
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

        bus.publish(block_of(
            vec![Complex32::new(0.5, 0.25), Complex32::new(-0.3, 0.9)],
            48_000,
        ));
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
        assert!(
            tail_rms < 0.05,
            "constant envelope should settle near silence, got rms {tail_rms}"
        );
    }

    #[test]
    fn am_envelope_responds_to_an_amplitude_step() {
        let (bus, mut ap) = pipeline_with(DemodMode::Am);
        let out = ap.subscribe(4);

        let latency = RfSelector::new(48_000.0, 15_000).unwrap().latency_samples();
        let mut samples = vec![Complex32::new(0.2, 0.0); 5_000];
        samples.extend(vec![Complex32::new(1.0, 0.0); latency + 500]);
        bus.publish(block_of(samples, 48_000));
        ap.tick(Duration::from_millis(50));

        let frame = out.try_recv().expect("expected a frame");
        let just_after_step = frame.samples[5_000 + latency + 20];
        assert!(
            just_after_step > 0.1,
            "expected a positive swing right after the envelope step, got {just_after_step}"
        );
    }

    #[test]
    fn fm_discriminator_recovers_constant_offset_as_constant_level() {
        let sample_rate = 48_000.0;
        let offset = 2_000.0; // well within NFM's 5 kHz deviation assumption
        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        let out = ap.subscribe(4);

        bus.publish(block_of(
            tone(1_000, offset, sample_rate),
            sample_rate as u32,
        ));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");

        let expected = (offset / NFM_DEVIATION_HZ as f64) as f32;
        // The causal RF selector has startup latency; check the settled tail.
        for &s in &frame.samples[800..] {
            assert!(
                (s - expected).abs() < 0.01,
                "sample {s} far from expected constant level {expected}"
            );
        }
    }

    #[test]
    fn fm_discriminator_state_is_continuous_across_ticks() {
        let sample_rate = 48_000.0;
        let offset = 1_500.0;
        let full_tone = tone(4_000, offset, sample_rate);

        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        let out = ap.subscribe(8);

        bus.publish(block_of(full_tone[..2_000].to_vec(), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let first = out.try_recv().expect("first frame");

        let mut next = block_of(full_tone[2_000..].to_vec(), sample_rate as u32);
        next.start_sample = 2_000;
        bus.publish(next);
        ap.tick(Duration::from_millis(50));
        let second = out.try_recv().expect("second frame");

        // Both sides of the network boundary are beyond RF filter startup and
        // must recover the same nonzero discriminator level.
        let expected = (offset / NFM_DEVIATION_HZ as f64) as f32;
        assert!(
            (second.samples[0] - expected).abs() < 0.01,
            "discontinuity at block boundary: {}",
            second.samples[0]
        );
        assert!((first.samples[1_999] - expected).abs() < 0.01);
    }

    #[test]
    fn missing_samples_or_changed_epoch_reset_every_audio_mode() {
        for mode in [
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Wfm,
            DemodMode::Usb,
            DemodMode::Lsb,
        ] {
            for gap in [false, true] {
                let (_, mut pipeline) = pipeline_with(mode);
                let output = pipeline.subscribe(8);
                let mut before = block_of(tone(800, 1000.0, 48_000.0), 48_000);
                before.capture = Some(ez_proto::CaptureClock::new(1, 0, 48_000, None));
                pipeline.process_block(&before);
                output.try_recv().unwrap();
                let mut after = block_of(tone(128, -1000.0, 48_000.0), 48_000);
                after.start_sample = if gap { 1000 } else { 800 };
                after.capture = Some(ez_proto::CaptureClock::new(
                    if gap { 1 } else { 2 },
                    after.start_sample,
                    48_000,
                    None,
                ));
                pipeline.process_block(&after);
                let actual = output.try_recv().unwrap();
                let (_, mut fresh) = pipeline_with(mode);
                let fresh_output = fresh.subscribe(4);
                fresh.process_block(&after);
                assert_eq!(
                    actual.samples,
                    fresh_output.try_recv().unwrap().samples,
                    "old {mode:?} history bridged a capture boundary"
                );
            }
        }
    }

    #[test]
    fn fm_output_is_clamped_to_unit_range_under_excessive_deviation() {
        let sample_rate = 48_000.0;
        // Offset far beyond NFM's assumed deviation drives the raw discriminator output
        // well past +-1 before clamping.
        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        let out = ap.subscribe(4);

        bus.publish(block_of(
            tone(2_000, 6_000.0, sample_rate),
            sample_rate as u32,
        ));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");

        assert!(frame.samples.iter().all(|&s| (-1.0..=1.0).contains(&s)));
        assert!(frame.samples[1_000..].iter().all(|&s| s > 0.99));
    }

    #[test]
    fn usb_and_lsb_reject_the_opposite_sideband() {
        fn output_rms(mode: DemodMode, tone_hz: f64) -> f32 {
            let samples: Vec<_> = tone(8_000, tone_hz, 48_000.0)
                .into_iter()
                .map(|sample| sample * 0.3)
                .collect();
            let (bus, mut pipeline) = pipeline_with(mode);
            let output = pipeline.subscribe(4);
            bus.publish(block_of(samples, 48_000));
            pipeline.tick(Duration::from_millis(50));
            let frame = output.try_recv().expect("SSB frame");
            let settled = &frame.samples[1_000..];
            (settled.iter().map(|sample| sample * sample).sum::<f32>() / settled.len() as f32)
                .sqrt()
        }

        let usb_wanted = output_rms(DemodMode::Usb, 1_000.0);
        let usb_rejected = output_rms(DemodMode::Usb, -1_000.0);
        let lsb_wanted = output_rms(DemodMode::Lsb, -1_000.0);
        let lsb_rejected = output_rms(DemodMode::Lsb, 1_000.0);
        assert!(
            usb_rejected < usb_wanted * 0.1,
            "USB rejection: wanted={usb_wanted}, opposite={usb_rejected}"
        );
        assert!(
            lsb_rejected < lsb_wanted * 0.1,
            "LSB rejection: wanted={lsb_wanted}, opposite={lsb_rejected}"
        );
    }

    fn ssb_tone_amplitude(samples: &[f32], hz: f64, rate: f64) -> f64 {
        let projection: num_complex::Complex64 = samples
            .iter()
            .enumerate()
            .map(|(n, &sample)| {
                num_complex::Complex64::from_polar(
                    f64::from(sample),
                    -std::f64::consts::TAU * hz * n as f64 / rate,
                )
            })
            .sum();
        2.0 * projection.norm() / samples.len() as f64
    }

    #[test]
    fn ssb_production_whole_voice_band_rejects_strong_opposite_signals_without_pitch_shift() {
        let frequencies: Vec<_> = (300..=3000).step_by(100).map(f64::from).collect();
        let mut max_gain_error = 0.0_f64;
        let mut max_mixture_error = 0.0_f64;
        let mut minimum_rejection = f64::INFINITY;
        let mut checks = 0;
        for (source_rate, stride) in [
            (8000_u32, 1_u32),
            (48_000, 1),
            (96_000, 1),
            (192_000, 1),
            (200_000, 1),
            (2_048_003, 10),
        ] {
            let rate = f64::from(source_rate) / f64::from(stride);
            let count = (rate * 0.2) as usize;
            let positive: Vec<_> = (0..count)
                .map(|n| {
                    let value: num_complex::Complex64 = frequencies
                        .iter()
                        .map(|&hz| {
                            num_complex::Complex64::from_polar(
                                0.002,
                                std::f64::consts::TAU * hz * n as f64 / rate,
                            )
                        })
                        .sum();
                    Complex32::new(value.re as f32, value.im as f32)
                })
                .collect();
            for mode in [DemodMode::Usb, DemodMode::Lsb] {
                let wanted: Vec<_> = positive
                    .iter()
                    .map(|&x| if mode == DemodMode::Usb { x } else { x.conj() })
                    .collect();
                let opposite: Vec<_> = wanted.iter().map(|x| x.conj() * 12.5).collect();
                let mixture: Vec<_> = wanted.iter().zip(&opposite).map(|(a, b)| a + b).collect();
                let measure = |input: Vec<Complex32>| {
                    let (_, mut pipeline) = pipeline_with(mode);
                    let output = pipeline.subscribe(2);
                    let mut block = block_of(input, source_rate / stride);
                    let mut clock =
                        ez_proto::CaptureClock::new(71, 0, source_rate, Some(1_000_000));
                    clock.sample_stride = stride;
                    block.capture = Some(clock);
                    pipeline.process_block(&block);
                    let frame = output.try_recv().unwrap();
                    assert_eq!(frame.samples.len(), count);
                    assert_eq!(frame.clock.exact_sample_rate_hz(), rate);
                    frame.samples[frame.samples.len() - (rate * 0.1) as usize..].to_vec()
                };
                let reference = measure(wanted);
                let rejected = measure(opposite);
                let mixed = measure(mixture);
                for &hz in &frequencies {
                    let amplitude = ssb_tone_amplitude(&reference, hz, rate);
                    let gain_error = (20.0 * (amplitude / 0.004).log10()).abs();
                    let rejection =
                        -20.0 * (ssb_tone_amplitude(&rejected, hz, rate) / 0.05).log10();
                    let mixture_error =
                        (20.0 * (ssb_tone_amplitude(&mixed, hz, rate) / amplitude).log10()).abs();
                    max_gain_error = max_gain_error.max(gain_error);
                    max_mixture_error = max_mixture_error.max(mixture_error);
                    minimum_rejection = minimum_rejection.min(rejection);
                    assert!(
                        gain_error < 0.1,
                        "{mode:?} {rate} {hz}Hz gain error={gain_error}dB"
                    );
                    assert!(
                        rejection > 80.0,
                        "{mode:?} {rate} {hz}Hz rejection={rejection}dB"
                    );
                    assert!(
                        mixture_error < 0.1,
                        "{mode:?} {rate} {hz}Hz mixture error={mixture_error}dB"
                    );
                    checks += 1;
                }
            }
        }
        eprintln!("ssb_production voice_checks={checks} max_gain_error_db={max_gain_error:.8} min_rejection_db={minimum_rejection:.6} max_21_94db_stronger_opposite_mixture_error_db={max_mixture_error:.8}");
    }

    #[test]
    fn ssb_production_fractional_clock_chunks_mode_changes_and_gaps_preserve_history() {
        let mut clock = ez_proto::CaptureClock::new(81, 0, 2_048_003, Some(1_000_000));
        clock.sample_stride = 10;
        let rate = clock.exact_sample_rate_hz();
        let input: Vec<_> = (0..12_007)
            .map(|n| {
                let t = n as f64 / rate;
                let wanted =
                    num_complex::Complex64::from_polar(0.05, std::f64::consts::TAU * 300.0 * t);
                let opposite =
                    num_complex::Complex64::from_polar(0.5, -std::f64::consts::TAU * 1800.0 * t);
                Complex32::new(
                    (wanted.re + opposite.re) as f32,
                    (wanted.im + opposite.im) as f32,
                )
            })
            .collect();
        let block = |samples: Vec<Complex32>, offset: usize, clock: ez_proto::CaptureClock| {
            let mut block = block_of(samples, 204_800);
            block.start_sample = offset as u64;
            block.capture = clock.offset_samples(offset as u64);
            block
        };
        for mode in [DemodMode::Usb, DemodMode::Lsb] {
            let (_, mut whole) = pipeline_with(mode);
            let whole_out = whole.subscribe(2);
            whole.process_block(&block(input.clone(), 0, clock));
            let expected = whole_out.try_recv().unwrap().samples;
            let (_, mut split) = pipeline_with(mode);
            let output = split.subscribe(2);
            let mut actual = Vec::new();
            let mut offset = 0;
            for size in [1_usize, 37, 997].into_iter().cycle() {
                if offset == input.len() {
                    break;
                }
                let end = (offset + size).min(input.len());
                split.process_block(&block(input[offset..end].to_vec(), offset, clock));
                actual.extend(output.try_recv().unwrap().samples);
                offset = end;
            }
            assert_eq!(actual, expected, "chunk history {mode:?}");
            let mut base_clock = clock;
            for boundary in 0..4 {
                if boundary == 0 {
                    base_clock.epoch += 1;
                }
                if boundary == 2 {
                    base_clock.source_sample_index += 100;
                }
                let mut new_clock = base_clock;
                match boundary {
                    0 => {}
                    1 => new_clock.discontinuous = true,
                    2 => {}
                    _ => {
                        split.set_mode(DemodMode::Raw);
                        split.set_mode(mode);
                    }
                }
                let after = block(
                    input[..997].to_vec(),
                    input.len() + boundary * 997,
                    new_clock,
                );
                split.process_block(&after);
                let frame = output.try_recv().unwrap();
                let (_, mut fresh) = pipeline_with(mode);
                let fresh_out = fresh.subscribe(2);
                fresh.process_block(&after);
                assert_eq!(
                    frame.samples,
                    fresh_out.try_recv().unwrap().samples,
                    "reset {mode:?}/{boundary}"
                );
                assert!(frame.clock.discontinuous);
            }
        }
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

        bus.publish(block_of(
            vec![Complex32::new(0.5, 0.0), Complex32::new(-0.3, 0.0)],
            48_000,
        ));
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
        assert!(
            frame.samples.iter().any(|&s| s != 0.0),
            "default squelch should not silence a real signal"
        );
    }

    #[test]
    fn raising_squelch_above_signal_power_silences_output() {
        let (bus, mut ap) = pipeline_with(DemodMode::Raw);
        ap.set_squelch(1.0); // unit-amplitude tone sits at ~0 dBFS, so 1.0 dB clearly gates it
        assert_eq!(ap.squelch_db(), 1.0);
        let out = ap.subscribe(4);

        bus.publish(block_of(tone(100, 1_000.0, 48_000.0), 48_000));
        ap.tick(Duration::from_millis(50));
        let frame = out
            .try_recv()
            .expect("expected a frame even when squelched");
        assert!(
            frame.samples.iter().all(|&s| s == 0.0),
            "squelch above signal power should silence output"
        );
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
        let full_tone = tone(4_000, offset, sample_rate);
        let (bus, mut ap) = pipeline_with(DemodMode::Fm);
        ap.set_squelch(100.0); // above any signal power this tone can reach: always gated
        let out = ap.subscribe(4);

        bus.publish(block_of(full_tone[..2_000].to_vec(), sample_rate as u32));
        ap.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");
        assert!(frame.samples.iter().all(|&s| s == 0.0));

        ap.set_squelch(DEFAULT_SQUELCH_DB);
        let mut next = block_of(full_tone[2_000..].to_vec(), sample_rate as u32);
        next.start_sample = 2_000;
        bus.publish(next);
        ap.tick(Duration::from_millis(50));
        let unsquelched = out.try_recv().expect("expected a frame");
        let expected = (offset / NFM_DEVIATION_HZ as f64) as f32;
        // Had prev_sample not kept advancing under squelch, sample 0 here would show the
        // same "no prior sample" reset artifact tested in
        // fm_discriminator_state_is_continuous_across_ticks instead of the steady tone level.
        assert!((unsquelched.samples[0] - expected).abs() < 0.01);
    }

    #[test]
    fn wfm_deemphasis_attenuates_higher_frequencies() {
        // Issue 17 regression test: Broadcast FM de-emphasis must attenuate high audio
        // frequencies (e.g. 10 kHz) relative to low audio frequencies (e.g. 500 Hz).
        let sample_rate = 192_000u32;
        let n_samples = 4000;

        // Generate FM signals modulated with 500 Hz and 10 kHz sinusoids (deviation 50 kHz)
        let gen_fm = |audio_freq: f64| -> Vec<Complex32> {
            let mut phase = 0.0f64;
            let dev = 50_000.0f64;
            (0..n_samples)
                .map(|k| {
                    let t = k as f64 / sample_rate as f64;
                    let m = (2.0 * std::f64::consts::PI * audio_freq * t).sin();
                    phase += 2.0 * std::f64::consts::PI * (m * dev) / sample_rate as f64;
                    Complex32::from_polar(1.0, phase as f32)
                })
                .collect()
        };

        let (bus_low, mut ap_low) = pipeline_with(DemodMode::Wfm);
        let out_low = ap_low.subscribe(4);
        bus_low.publish(block_of(gen_fm(500.0), sample_rate));
        ap_low.tick(Duration::from_millis(50));
        let frame_low = out_low.try_recv().expect("low frame");

        let (bus_high, mut ap_high) = pipeline_with(DemodMode::Wfm);
        let out_high = ap_high.subscribe(4);
        bus_high.publish(block_of(gen_fm(10_000.0), sample_rate));
        ap_high.tick(Duration::from_millis(50));
        let frame_high = out_high.try_recv().expect("high frame");

        // Compute RMS of the settled tail (last 1000 samples)
        let rms_low =
            (frame_low.samples[1000..].iter().map(|s| s * s).sum::<f32>() / 3000.0).sqrt();
        let rms_high = (frame_high.samples[1000..]
            .iter()
            .map(|s| s * s)
            .sum::<f32>()
            / 3000.0)
            .sqrt();

        // 10 kHz should be heavily attenuated by 75 µs de-emphasis (~ -14 dB vs 500 Hz)
        assert!(
            rms_high < rms_low * 0.35,
            "WFM de-emphasis should attenuate 10kHz audio: rms_low={rms_low}, rms_high={rms_high}"
        );
    }
}
