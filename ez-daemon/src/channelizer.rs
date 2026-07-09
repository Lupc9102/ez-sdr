//! Wideband -> narrowband channelizer: digital down-conversion (DDC) of the shared wideband
//! [`SampleBus`] into any number of independently tuned virtual channels, each its own
//! [`SampleBus`] that pipelines (spectrum, audio, packet, telemetry) subscribe to.
//!
//! Each virtual channel is a plain, hand-rolled DDC chain — NCO mix to baseband, windowed-
//! sinc FIR low-pass, decimate — driven directly by [`Channelizer::tick`]/[`Channelizer::run`].
//! No generic pipeline/stage abstraction: one struct per concept, one function per step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use num_complex::Complex32;

use crate::bus::{OverflowPolicy, SampleBlock, SampleBus, SampleBusHandle};

const FIR_TAPS: usize = 63;
/// Fraction of the ideal brick-wall cutoff actually used when designing the anti-alias
/// filter, leaving transition-band headroom so a modest tap count doesn't already attenuate
/// the passband edge (the decimated Nyquist).
const CUTOFF_GUARD: f32 = 0.8;

/// A numerically controlled oscillator: generates `e^{-j*2*pi*f*n/fs}` incrementally so a
/// wideband block can be mixed to baseband one sample at a time, with phase carried
/// correctly across successive blocks.
struct Nco {
    phase: f32,
    phase_increment: f32,
}

impl Nco {
    fn new(freq_offset_hz: f64, sample_rate_hz: f64) -> Self {
        let phase_increment = (-2.0 * std::f64::consts::PI * freq_offset_hz / sample_rate_hz) as f32;
        Self {
            phase: 0.0,
            phase_increment,
        }
    }

    /// Mixes `samples` down (or up) by this NCO's tuned offset, in place order preserved as
    /// a new `Vec` (the input itself, e.g. straight off the wideband bus, is never mutated).
    fn mix(&mut self, samples: &[Complex32]) -> Vec<Complex32> {
        samples
            .iter()
            .map(|&s| {
                let osc = Complex32::from_polar(1.0, self.phase);
                self.phase += self.phase_increment;
                if self.phase > std::f32::consts::PI {
                    self.phase -= 2.0 * std::f32::consts::PI;
                } else if self.phase < -std::f32::consts::PI {
                    self.phase += 2.0 * std::f32::consts::PI;
                }
                s * osc
            })
            .collect()
    }
}

/// Windowed-sinc low-pass FIR design (Hamming window, unity DC gain). `cutoff_fraction` is
/// the cutoff as a fraction of Nyquist, in `(0, 1)`. `num_taps` must be odd (Type-I linear
/// phase, symmetric around a single center tap).
fn design_lowpass(num_taps: usize, cutoff_fraction: f32) -> Vec<f32> {
    assert!(num_taps % 2 == 1, "num_taps must be odd, got {num_taps}");
    let m = (num_taps - 1) as f32;
    let mut taps: Vec<f32> = (0..num_taps)
        .map(|n| {
            let x = n as f32 - m / 2.0;
            let sinc = if x == 0.0 {
                cutoff_fraction
            } else {
                (std::f32::consts::PI * cutoff_fraction * x).sin() / (std::f32::consts::PI * x)
            };
            let window = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * n as f32 / m).cos();
            sinc * window
        })
        .collect();
    let dc_gain: f32 = taps.iter().sum();
    for t in &mut taps {
        *t /= dc_gain;
    }
    taps
}

/// Streaming FIR filter + integer decimator. Carries both filter history and decimation
/// phase across calls, so feeding it the same samples in different-sized chunks produces the
/// same output — required since wideband blocks arrive at whatever size the ingestion
/// thread happened to read, not aligned to any particular decimation boundary.
struct FirDecimator {
    taps: Vec<f32>,
    decimation: usize,
    /// Tail of the previous call's (history ++ input), length `taps.len() - 1`, that seeds
    /// this call's convolution window so filtering is continuous across block boundaries.
    history: Vec<Complex32>,
    /// How many samples into the *next* call's input the next output sample falls, carrying
    /// the decimation phase across calls so uneven block lengths never introduce jitter.
    skip: usize,
}

impl FirDecimator {
    fn new(taps: Vec<f32>, decimation: usize) -> Self {
        let history = vec![Complex32::new(0.0, 0.0); taps.len() - 1];
        Self {
            taps,
            decimation: decimation.max(1),
            history,
            skip: 0,
        }
    }

    fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let taps_len = self.taps.len();
        let mut extended = Vec::with_capacity(self.history.len() + input.len());
        extended.extend_from_slice(&self.history);
        extended.extend_from_slice(input);

        let mut output = Vec::new();
        let mut i = self.skip;
        while i < input.len() {
            let window = &extended[i..i + taps_len];
            let acc: Complex32 = window.iter().zip(self.taps.iter()).map(|(s, t)| *s * *t).sum();
            output.push(acc);
            i += self.decimation;
        }
        self.skip = i - input.len();

        let new_history_start = extended.len() - (taps_len - 1);
        self.history = extended[new_history_start..].to_vec();

        output
    }
}

/// One tuned narrowband tap on the wideband stream: an independent DDC chain plus the
/// [`SampleBus`] its decimated output is published to.
struct VirtualChannel {
    id: u64,
    center_freq_hz: u64,
    output_rate_hz: u32,
    nco: Nco,
    decimator: FirDecimator,
    output_sample_counter: u64,
    bus: SampleBus,
}

impl VirtualChannel {
    fn new(
        id: u64,
        wideband_center_hz: u64,
        wideband_rate_hz: u32,
        center_freq_hz: u64,
        decimation: usize,
    ) -> Self {
        let offset_hz = center_freq_hz as f64 - wideband_center_hz as f64;
        let nco = Nco::new(offset_hz, wideband_rate_hz as f64);
        let cutoff = (CUTOFF_GUARD / decimation as f32).clamp(0.01, 0.99);
        let taps = design_lowpass(FIR_TAPS, cutoff);
        let decimator = FirDecimator::new(taps, decimation);
        Self {
            id,
            center_freq_hz,
            output_rate_hz: (wideband_rate_hz as usize / decimation) as u32,
            nco,
            decimator,
            output_sample_counter: 0,
            bus: SampleBus::new(),
        }
    }

    /// Mixes and decimates one wideband block, returning this channel's next output block —
    /// or `None` if too few samples have accumulated yet to produce one (e.g. a short block
    /// against a large decimation factor).
    fn process(&mut self, block: &SampleBlock) -> Option<SampleBlock> {
        let mixed = self.nco.mix(&block.samples);
        let decimated = self.decimator.process(&mixed);
        if decimated.is_empty() {
            return None;
        }
        let start_sample = self.output_sample_counter;
        self.output_sample_counter += decimated.len() as u64;
        Some(SampleBlock {
            start_sample,
            sample_rate_hz: self.output_rate_hz,
            center_freq_hz: self.center_freq_hz,
            samples: Arc::from(decimated),
        })
    }
}

/// Routes one wideband [`SampleBus`] into any number of independently tuned virtual
/// channels. Owns the wideband subscription and the channel registry; `add_channel`/
/// `remove_channel` are the only mutation points, so registration is as cheap and simple as
/// a `Vec` push/retain — no need for the bus's lock-free machinery here, since only the one
/// thread driving `tick`/`run` ever touches the registry.
pub struct Channelizer {
    wideband: SampleBusHandle,
    wideband_center_hz: u64,
    wideband_rate_hz: u32,
    channels: Vec<VirtualChannel>,
    next_id: u64,
}

impl Channelizer {
    #[must_use]
    pub fn new(wideband: SampleBusHandle, wideband_center_hz: u64, wideband_rate_hz: u32) -> Self {
        Self {
            wideband,
            wideband_center_hz,
            wideband_rate_hz,
            channels: Vec::new(),
            next_id: 0,
        }
    }

    /// Registers a new virtual channel tuned to `center_freq_hz`, decimated down to (at
    /// least) `min_output_rate_hz`. Returns the channel's id (for [`Self::remove_channel`])
    /// and a bus handle its consumer(s) receive decimated output from; dropping every handle
    /// subscribed to it does not remove the channel itself — call `remove_channel` for that.
    pub fn add_channel(
        &mut self,
        center_freq_hz: u64,
        min_output_rate_hz: u32,
    ) -> Result<(u64, SampleBusHandle)> {
        if min_output_rate_hz == 0 {
            return Err(anyhow!("virtual channel output rate must be non-zero"));
        }
        let offset_hz = center_freq_hz as f64 - self.wideband_center_hz as f64;
        let nyquist_hz = self.wideband_rate_hz as f64 / 2.0;
        if offset_hz.abs() > nyquist_hz {
            return Err(anyhow!(
                "requested channel {center_freq_hz} Hz is outside the wideband capture's {:.0}-{:.0} Hz span",
                self.wideband_center_hz as f64 - nyquist_hz,
                self.wideband_center_hz as f64 + nyquist_hz,
            ));
        }

        let decimation = (self.wideband_rate_hz / min_output_rate_hz).max(1) as usize;
        let id = self.next_id;
        self.next_id += 1;
        let channel = VirtualChannel::new(
            id,
            self.wideband_center_hz,
            self.wideband_rate_hz,
            center_freq_hz,
            decimation,
        );
        let handle = channel.bus.subscribe(64, OverflowPolicy::DropOldest);
        self.channels.push(channel);
        Ok((id, handle))
    }

    pub fn remove_channel(&mut self, id: u64) {
        self.channels.retain(|c| c.id != id);
    }

    #[must_use]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Waits up to `poll_timeout` for the next wideband block and, if one arrived, routes it
    /// through every active channel. Returns whether a block was actually processed —
    /// callers driving a shutdown-aware loop should check an external stop signal on `false`
    /// rather than treating it as "source gone" (see [`SampleBusHandle::recv_timeout`]).
    pub fn tick(&mut self, poll_timeout: Duration) -> bool {
        let Some(block) = self.wideband.recv_timeout(poll_timeout) else {
            return false;
        };
        for channel in &mut self.channels {
            if let Some(out) = channel.process(&block) {
                channel.bus.publish(out);
            }
        }
        true
    }

    /// Drives `tick` in a loop until `running` is set to `false`.
    pub fn run(&mut self, running: &AtomicBool) {
        while running.load(Ordering::Relaxed) {
            self.tick(Duration::from_millis(250));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wideband_block(center_freq_hz: u64, sample_rate_hz: u32, samples: Vec<Complex32>) -> SampleBlock {
        SampleBlock {
            start_sample: 0,
            sample_rate_hz,
            center_freq_hz,
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

    #[test]
    fn nco_mix_removes_matching_offset_tone_to_near_dc() {
        let sample_rate = 1_000.0;
        let offset = 100.0;
        let input = tone(256, offset, sample_rate);

        let mut nco = Nco::new(offset, sample_rate);
        let mixed = nco.mix(&input);

        for w in mixed.windows(2) {
            let dphi = (w[1] * w[0].conj()).arg();
            assert!(dphi.abs() < 1e-3, "residual phase step {dphi}");
        }
    }

    #[test]
    fn nco_mix_preserves_magnitude_over_many_samples() {
        let mut nco = Nco::new(12_345.678, 2_048_000.0);
        let input = vec![Complex32::new(1.0, 0.0); 100_000];
        let mixed = nco.mix(&input);
        for s in mixed.iter().step_by(997) {
            assert!((s.norm() - 1.0).abs() < 1e-4, "magnitude drifted: {}", s.norm());
        }
    }

    #[test]
    fn design_lowpass_has_unity_dc_gain() {
        let taps = design_lowpass(63, 0.2);
        let dc_gain: f32 = taps.iter().sum();
        assert!((dc_gain - 1.0).abs() < 1e-5, "DC gain was {dc_gain}");
    }

    #[test]
    fn fir_decimator_downsamples_by_expected_factor() {
        let taps = design_lowpass(31, 0.5);
        let mut dec = FirDecimator::new(taps, 4);
        let input = vec![Complex32::new(1.0, 0.0); 4_000];
        let out = dec.process(&input);
        // 4000 / 4 = 1000, +/-1 for warm-up/decimation-phase rounding at the boundary.
        assert!((out.len() as i64 - 1000).abs() <= 1, "got {} outputs", out.len());
    }

    #[test]
    fn fir_decimator_is_continuous_across_chunk_boundaries() {
        let taps = design_lowpass(31, 0.3);
        let full_input: Vec<Complex32> = tone(2_000, 10.0, 1_000.0);

        let mut whole = FirDecimator::new(taps.clone(), 5);
        let out_whole = whole.process(&full_input);

        let mut chunked = FirDecimator::new(taps, 5);
        let mut out_chunks = Vec::new();
        for chunk in full_input.chunks(37) {
            out_chunks.extend(chunked.process(chunk));
        }

        assert_eq!(out_whole.len(), out_chunks.len());
        for (a, b) in out_whole.iter().zip(out_chunks.iter()) {
            assert!((a - b).norm() < 1e-4, "diverged: {a} vs {b}");
        }
    }

    #[test]
    fn fir_lowpass_attenuates_out_of_band_tone() {
        let sample_rate = 1_000.0;
        let taps = design_lowpass(63, 0.1); // passband up to ~50 Hz
        let mut dec = FirDecimator::new(taps, 1);

        let in_band = tone(2_000, 10.0, sample_rate);
        let out_of_band = tone(2_000, 400.0, sample_rate);

        let passed = dec.process(&in_band);
        let mut dec2 = FirDecimator::new(design_lowpass(63, 0.1), 1);
        let rejected = dec2.process(&out_of_band);

        let tail_rms = |v: &[Complex32]| {
            let tail = &v[v.len() / 2..];
            (tail.iter().map(|c| c.norm_sqr()).sum::<f32>() / tail.len() as f32).sqrt()
        };
        let passed_rms = tail_rms(&passed);
        let rejected_rms = tail_rms(&rejected);
        assert!(passed_rms > 0.7, "in-band tone over-attenuated: {passed_rms}");
        assert!(
            rejected_rms < 0.1,
            "out-of-band tone insufficiently attenuated: {rejected_rms}"
        );
    }

    #[test]
    fn virtual_channel_process_shifts_and_decimates() {
        let mut ch = VirtualChannel::new(0, 100_000_000, 2_000_000, 100_050_000, 10);
        let block = wideband_block(100_000_000, 2_000_000, tone(2_000, 50_000.0, 2_000_000.0));

        let out = ch.process(&block).expect("expected decimated output");
        assert_eq!(out.center_freq_hz, 100_050_000);
        assert_eq!(out.sample_rate_hz, 200_000);
        assert!((out.samples.len() as i64 - 200).abs() <= 2, "got {}", out.samples.len());
    }

    #[test]
    fn channelizer_add_channel_rejects_out_of_nyquist_frequency() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let err = match chan.add_channel(150_000_000, 48_000) {
            Err(e) => e,
            Ok(_) => panic!("expected out-of-range channel to be rejected"),
        };
        assert!(err.to_string().contains("outside"));
    }

    #[test]
    fn channelizer_add_channel_rejects_zero_output_rate() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert!(chan.add_channel(100_000_000, 0).is_err());
    }

    #[test]
    fn channelizer_tick_routes_wideband_block_to_subscribed_channel() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let (_, out_handle) = chan.add_channel(100_000_000, 100_000).unwrap();

        bus.publish(wideband_block(100_000_000, 2_000_000, tone(4_000, 0.0, 2_000_000.0)));
        let processed = chan.tick(Duration::from_millis(200));
        assert!(processed);

        let out = out_handle
            .recv_timeout(Duration::from_millis(200))
            .expect("expected decimated output on the channel bus");
        assert_eq!(out.center_freq_hz, 100_000_000);
    }

    #[test]
    fn channelizer_supports_multiple_independently_tuned_channels() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let (_, a) = chan.add_channel(99_900_000, 200_000).unwrap();
        let (_, b) = chan.add_channel(100_100_000, 50_000).unwrap();
        assert_eq!(chan.channel_count(), 2);

        bus.publish(wideband_block(100_000_000, 2_000_000, tone(8_000, 0.0, 2_000_000.0)));
        assert!(chan.tick(Duration::from_millis(200)));

        let out_a = a.recv_timeout(Duration::from_millis(200)).expect("channel a output");
        let out_b = b.recv_timeout(Duration::from_millis(200)).expect("channel b output");
        assert_eq!(out_a.center_freq_hz, 99_900_000);
        assert_eq!(out_b.center_freq_hz, 100_100_000);
        assert_ne!(out_a.sample_rate_hz, out_b.sample_rate_hz);
    }

    #[test]
    fn channelizer_remove_channel_drops_it_from_registry() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let (id, _out_handle) = chan.add_channel(100_000_000, 100_000).unwrap();
        assert_eq!(chan.channel_count(), 1);
        chan.remove_channel(id);
        assert_eq!(chan.channel_count(), 0);
    }

    #[test]
    fn channelizer_tick_times_out_cleanly_when_idle() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert!(!chan.tick(Duration::from_millis(20)));
    }

    #[test]
    fn channelizer_run_exits_promptly_when_running_flag_clears() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        let running = AtomicBool::new(false);
        chan.run(&running); // must return immediately, not hang
    }
}
