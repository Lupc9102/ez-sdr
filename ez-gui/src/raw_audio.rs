//! RAW listening is band-limited stereo I/Q at the sound-device clock.
//! Coarse RF filtering bounds FIR storage at high capture rates. Independent
//! fractional-delay filters retain channel identity and exact stream duration.

use crate::audio_resampler::RateFilter;
use ez_daemon::channelizer::FirDecimator;
use num_complex::Complex32;
use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct RawIqAudio {
    settings: Option<(f64, u32)>,
    coarse: Option<FirDecimator>,
    filters: Option<[RateFilter; 2]>,
    numerator: u128,
    output_denominator: u128,
    coarse_denominator: u128,
    next: u128,
    duration_phase: u128,
    pending: VecDeque<[f32; 2]>,
    capture: Option<CaptureDuration>,
    unclipped: bool,
}

struct CaptureDuration {
    source_rate: f64,
    numerator: u128,
    denominator: u128,
    phase: u128,
    pending: VecDeque<[f32; 2]>,
}

fn exact_ratio(rate: f64) -> (u128, u128) {
    let bits = rate.to_bits();
    let mantissa = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let common = mantissa.trailing_zeros().min((-exponent).max(0) as u32);
    (
        u128::from(mantissa >> common),
        1_u128 << (-exponent as u32 - common),
    )
}

impl RawIqAudio {
    /// Detected envelopes can contain DC greater than full-scale. Their
    /// amplitude controls run after rate conversion, so preserve that range.
    pub fn for_detector() -> Self {
        Self {
            unclipped: true,
            ..Self::default()
        }
    }

    pub fn reset(&mut self) {
        let unclipped = self.unclipped;
        *self = Self::default();
        self.unclipped = unclipped;
    }

    fn output_sample(&self, sample: f32, gain: f32) -> f32 {
        let output = sample * gain;
        if self.unclipped {
            output
        } else {
            output.clamp(-1.0, 1.0)
        }
    }

    fn configure(&mut self, input_rate: f64, output_rate: u32) -> bool {
        if !input_rate.is_finite()
            || !(100.0..=20_000_000.0).contains(&input_rate)
            || !(8_000..=384_000).contains(&output_rate)
        {
            self.reset();
            return false;
        }
        if self.settings == Some((input_rate, output_rate)) {
            return true;
        }
        self.reset();
        // Represent the actual f64 clock exactly, including fractional RF
        // decimation. A bounded integer remainder avoids accumulated drift.
        let (numerator, denominator) = exact_ratio(input_rate);
        let coarse = (input_rate / (4.0 * f64::from(output_rate)).max(250_000.0))
            .floor()
            .max(1.0) as u32;
        self.numerator = numerator;
        self.output_denominator = denominator * u128::from(output_rate);
        self.coarse_denominator = self.output_denominator * u128::from(coarse);
        self.duration_phase = numerator - 1; // ceil(input duration * output Fs)
        self.coarse = Some(FirDecimator::new(coarse as usize));
        self.filters = (input_rate != f64::from(output_rate)).then(|| {
            std::array::from_fn(|_| {
                RateFilter::new(input_rate / f64::from(coarse) / f64::from(output_rate))
            })
        });
        self.settings = Some((input_rate, output_rate));
        true
    }

    /// When a preceding RF decimator emits floor(N/D) samples, retain its
    /// causal startup as one zero IQ interval. Output remains on the root
    /// capture duration, including incomplete RF strides and tiny chunks.
    pub fn process_capture(
        &mut self,
        input: &[Complex32],
        input_rate: f64,
        output_rate: u32,
        gain: f32,
        source_samples: usize,
        source_rate: f64,
    ) -> Vec<[f32; 2]> {
        if !source_rate.is_finite()
            || !(100.0..=20_000_000.0).contains(&source_rate)
            || source_rate < input_rate
        {
            self.reset();
            return Vec::new();
        }
        if self
            .capture
            .as_ref()
            .is_some_and(|old| old.source_rate != source_rate)
        {
            self.reset();
        }
        if !self.configure(input_rate, output_rate) {
            return Vec::new();
        }
        let mut capture = self.capture.take().unwrap_or_else(|| {
            let (numerator, denominator) = exact_ratio(source_rate);
            let mut pending = VecDeque::new();
            if source_rate > input_rate {
                pending.extend(self.process(
                    &[Complex32::new(0.0, 0.0)],
                    input_rate,
                    output_rate,
                    gain,
                ));
            }
            CaptureDuration {
                source_rate,
                numerator,
                denominator: denominator * u128::from(output_rate),
                phase: numerator - 1,
                pending,
            }
        });
        capture
            .pending
            .extend(self.process(input, input_rate, output_rate, gain));
        capture.phase += source_samples as u128 * capture.denominator;
        let frames = (capture.phase / capture.numerator) as usize;
        capture.phase %= capture.numerator;
        assert!(
            capture.pending.len() >= frames,
            "RF samples must match root capture duration"
        );
        let output = capture.pending.drain(..frames).collect();
        self.capture = Some(capture);
        output
    }

    pub fn process(
        &mut self,
        input: &[Complex32],
        input_rate: f64,
        output_rate: u32,
        gain: f32,
    ) -> Vec<[f32; 2]> {
        if !self.configure(input_rate, output_rate) {
            return Vec::new();
        }
        if input.is_empty() {
            return Vec::new();
        }
        if input_rate == f64::from(output_rate) {
            let gain = if gain.is_finite() { gain } else { 0.0 };
            return input
                .iter()
                .map(|sample| {
                    [sample.re, sample.im].map(|x| {
                        if x.is_finite() {
                            self.output_sample(x, gain)
                        } else {
                            0.0
                        }
                    })
                })
                .collect();
        }
        self.duration_phase += input.len() as u128 * self.output_denominator;
        let frames = (self.duration_phase / self.numerator) as usize;
        self.duration_phase %= self.numerator;
        let clean: Vec<_> = input
            .iter()
            .map(|sample| {
                let clean = |x: f32| if x.is_finite() { x } else { 0.0 };
                Complex32::new(clean(sample.re), clean(sample.im))
            })
            .collect();
        let coarse = self.coarse.as_mut().unwrap().process(&clean);
        let filters = self.filters.as_mut().unwrap();
        for sample in coarse {
            filters[0].push(sample.re);
            filters[1].push(sample.im);
            while self.next < self.coarse_denominator {
                let phase = self.next as f64 / self.coarse_denominator as f64;
                self.pending
                    .push_back([filters[0].output(phase), filters[1].output(phase)]);
                self.next += self.numerator;
            }
            self.next -= self.coarse_denominator;
        }
        // Coarse samples describe an entire RF stride. Hold any frames beyond
        // the root input duration until those input samples actually arrive.
        // This preserves counts even when N is not a multiple of the stride.
        assert!(self.pending.len() >= frames);
        let gain = if gain.is_finite() { gain } else { 0.0 };
        let unclipped = self.unclipped;
        self.pending
            .drain(..frames)
            .map(|frame| {
                frame.map(|x| {
                    let output = x * gain;
                    if unclipped {
                        output
                    } else {
                        output.clamp(-1.0, 1.0)
                    }
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_audio_clocks_count_frames_exactly_and_preserve_every_partition() {
        let rates: [f64; 9] = [
            125.0,
            8_000.0,
            48_000.0,
            192_000.0,
            384_003.0 / 7.0,
            1_000_003.0,
            2_048_000.0,
            2_400_000.0,
            20_000_000.0,
        ];
        let mut cases = 0;
        for input_rate in rates {
            for output_rate in [8_000, 44_100, 48_000, 192_000, 384_000] {
                let n = (input_rate * 0.002).ceil().max(7.0) as usize;
                let input: Vec<_> = (0..n)
                    .map(|i| {
                        let phase = std::f64::consts::TAU * 17.0 * i as f64 / input_rate;
                        Complex32::new((phase.cos() * 0.2) as f32, (phase.sin() * 0.1) as f32)
                    })
                    .collect();
                let whole = RawIqAudio::default().process(&input, input_rate, output_rate, 0.3);
                let expected = (n as f64 * f64::from(output_rate) / input_rate).ceil() as usize;
                assert_eq!(whole.len(), expected, "{input_rate}/{output_rate}");
                let mut split = RawIqAudio::default();
                let mut actual = Vec::new();
                for chunk in input.chunks(37) {
                    actual.extend(split.process(chunk, input_rate, output_rate, 0.3));
                    assert!(split.process(&[], input_rate, output_rate, 0.3).is_empty());
                }
                assert_eq!(actual, whole, "{input_rate}/{output_rate}");
                cases += 1;
            }
        }
        eprintln!("raw_audio duration_and_partition_cases={cases} exact_frames=passed");
    }

    #[test]
    fn raw_audio_rf_stride_tails_follow_root_duration_in_tiny_partitions() {
        let mut cases = 0;
        for source_rate in [8_000_u32, 192_003, 2_400_000] {
            for factor in [2_u32, 8, 64] {
                for output_rate in [8_000_u32, 48_000, 384_000] {
                    let mut raw = RawIqAudio::default();
                    let mut root_total = 0_usize;
                    let mut iq_total = 0_usize;
                    let mut output_total = 0_usize;
                    for n in [1_usize, 7, 37, 997, 3] {
                        root_total += n;
                        let iq_count = root_total / factor as usize - iq_total;
                        iq_total += iq_count;
                        let output = raw.process_capture(
                            &vec![Complex32::new(0.1, -0.2); iq_count],
                            f64::from(source_rate) / f64::from(factor),
                            output_rate,
                            0.3,
                            n,
                            f64::from(source_rate),
                        );
                        output_total += output.len();
                        assert_eq!(
                            output_total,
                            (root_total as u128 * u128::from(output_rate))
                                .div_ceil(u128::from(source_rate))
                                as usize
                        );
                    }
                    cases += 1;
                }
            }
        }
        eprintln!(
            "raw_audio root_stride_partition_cases={cases} incomplete_stride_duration=passed"
        );
    }

    #[test]
    fn raw_audio_channels_preserve_independent_tones_and_reject_output_aliases() {
        let mut worst_gain_error = 0.0_f64;
        let mut worst_alias = 0.0_f64;
        for output_rate in [8_000_u32, 44_100, 48_000] {
            let input_rate = 192_000.0;
            let create = |alias: bool| {
                (0..19_200)
                    .map(|n| {
                        let t = n as f64 / input_rate;
                        let i_hz = if alias {
                            f64::from(output_rate) + 1_000.0
                        } else {
                            1_000.0
                        };
                        let q_hz = if alias {
                            f64::from(output_rate) + 2_000.0
                        } else {
                            2_000.0
                        };
                        Complex32::new(
                            (0.2 * (std::f64::consts::TAU * i_hz * t).cos()) as f32,
                            (0.1 * (std::f64::consts::TAU * q_hz * t).sin()) as f32,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let wanted =
                RawIqAudio::default().process(&create(false), input_rate, output_rate, 0.3);
            let alias = RawIqAudio::default().process(&create(true), input_rate, output_rate, 0.3);
            let amplitude = |frames: &[[f32; 2]], channel: usize, hz: f64| {
                let frames = &frames[frames.len() / 2..];
                let (mut c, mut s) = (0.0, 0.0);
                for (n, frame) in frames.iter().enumerate() {
                    let phase = std::f64::consts::TAU * hz * n as f64 / f64::from(output_rate);
                    c += f64::from(frame[channel]) * phase.cos();
                    s += f64::from(frame[channel]) * phase.sin();
                }
                2.0 * c.hypot(s) / frames.len() as f64
            };
            for (channel, hz, expected) in [(0, 1_000.0, 0.06), (1, 2_000.0, 0.03)] {
                let gain_error =
                    (20.0 * (amplitude(&wanted, channel, hz) / expected).log10()).abs();
                worst_gain_error = worst_gain_error.max(gain_error);
                assert!(
                    gain_error < 0.1,
                    "channel={channel} audioFs={output_rate} gain_error={gain_error}"
                );
                let rejected = amplitude(&alias, channel, hz) / expected;
                worst_alias = worst_alias.max(rejected);
                assert!(
                    rejected < 1e-4,
                    "channel={channel} audioFs={output_rate} alias={rejected}"
                );
                assert!(
                    amplitude(&wanted, 1 - channel, hz) < expected * 1e-4,
                    "I/Q crosstalk"
                );
            }
        }
        eprintln!("raw_audio independent_channel_cases=6 max_gain_error_db={worst_gain_error:.8} min_alias_rejection_db={:.4}", -20.0 * worst_alias.log10());
    }

    #[test]
    fn raw_audio_equal_clocks_preserve_iq_without_added_delay() {
        let input = [
            Complex32::new(0.1, -0.2),
            Complex32::new(0.0, 0.7),
            Complex32::new(f32::NAN, f32::INFINITY),
        ];
        let output = RawIqAudio::default().process(&input, 48_000.0, 48_000, 0.3);
        assert_eq!(
            output,
            vec![[0.1 * 0.3, -0.2 * 0.3], [0.0, 0.7 * 0.3], [0.0, 0.0]]
        );
    }

    #[test]
    fn raw_audio_reset_rate_changes_and_nonfinite_input_retire_history() {
        let input = vec![Complex32::new(0.3, -0.1); 997];
        let mut used = RawIqAudio::default();
        used.process(&input, 192_000.0, 48_000, 0.3);
        used.reset();
        assert_eq!(
            used.process(&input, 192_000.0, 48_000, 0.3),
            RawIqAudio::default().process(&input, 192_000.0, 48_000, 0.3)
        );
        assert_eq!(
            used.process(&input, 192_003.0, 44_100, 0.3),
            RawIqAudio::default().process(&input, 192_003.0, 44_100, 0.3)
        );
        let bad = vec![Complex32::new(f32::NAN, f32::INFINITY); 997];
        used.reset();
        assert!(used
            .process(&bad, 192_000.0, 48_000, 0.3)
            .iter()
            .all(|x| *x == [0.0; 2]));
        for invalid in [f64::NAN, f64::INFINITY, 0.0, 99.0, 20_000_001.0] {
            assert!(used.process(&input, invalid, 48_000, 0.3).is_empty());
        }
    }
}
