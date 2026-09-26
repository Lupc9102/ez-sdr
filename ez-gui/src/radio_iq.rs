//! Floating-point processing for the local Radio view.
//!
//! Raw recording and ADS-B must keep the original byte stream and sample clock.
//! Feed the spectrum with `RadioIqProcessor` output, then feed a separate
//! `VfoMixer` output to the demodulator. No stage converts processed IQ to bytes.

use num_complex::{Complex32, Complex64};

/// Source-side Radio settings. "IQ correction" currently means removal of
/// slowly varying I and Q DC offsets; it is not gain/phase imbalance calibration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RadioIqConfig {
    pub input_rate: u32,
    pub dc_remove: bool,
    pub invert: bool,
    pub decimation: u32,
}

impl Default for RadioIqConfig {
    fn default() -> Self {
        Self {
            input_rate: 2_048_000,
            dc_remove: false,
            invert: false,
            decimation: 1,
        }
    }
}

impl RadioIqConfig {
    /// Clamp to 1..=1024 and round down to a power of two. Never produce a
    /// nominal clock below one sample/second, including malformed saved values.
    pub fn normalized(mut self) -> Self {
        self.input_rate = self.input_rate.max(1);
        let requested = self.decimation.clamp(1, 1024).min(self.input_rate);
        self.decimation = 1 << (31 - requested.leading_zeros());
        self
    }
}

/// A 63-tap Blackman half-band FIR, evaluated only at retained samples.
/// Symmetry and zero half-band taps reduce each complex output to 16 paired
/// products plus the center tap. Each stage passes approximately 80% of its
/// output Nyquist bandwidth; the remaining edge is the antialias transition.
struct HalfBandDecimator {
    paired_taps: [f32; 16],
    center_tap: f32,
    history: [Complex32; 126],
    write: usize,
    phase: bool,
}

impl HalfBandDecimator {
    fn new() -> Self {
        let mut paired_taps = [0.0; 16];
        for (index, tap) in paired_taps.iter_mut().enumerate() {
            let position = index as f64 * 2.0;
            let offset = position - 31.0;
            let sinc =
                (std::f64::consts::FRAC_PI_2 * offset).sin() / (std::f64::consts::PI * offset);
            let window = 0.42 - 0.5 * (std::f64::consts::TAU * position / 62.0).cos()
                + 0.08 * (2.0 * std::f64::consts::TAU * position / 62.0).cos();
            *tap = (sinc * window) as f32;
        }
        let sum = 0.5 + 2.0 * paired_taps.iter().sum::<f32>();
        for tap in &mut paired_taps {
            *tap /= sum;
        }
        Self {
            paired_taps,
            center_tap: 0.5 / sum,
            history: [Complex32::new(0.0, 0.0); 126],
            write: 0,
            phase: false,
        }
    }

    fn push(&mut self, sample: Complex32) -> Option<Complex32> {
        self.history[self.write] = sample;
        self.history[self.write + 63] = sample;
        self.write += 1;
        if self.write == 63 {
            self.write = 0;
        }
        self.phase = !self.phase;
        if self.phase {
            return None;
        }
        let samples = &self.history[self.write..self.write + 63];
        let mut output = samples[31] * self.center_tap;
        for (index, &tap) in self.paired_taps.iter().enumerate() {
            output += (samples[index * 2] + samples[62 - index * 2]) * tap;
        }
        Some(output)
    }
}

/// Stateful byte-to-complex conversion, DC correction, inversion and filtered
/// decimation. A trailing I byte is retained for the next call's Q byte.
/// After N complete source pairs this emits exactly floor(N / decimation)
/// pairs, including the FIR startup transient; no output is padded or flushed.
pub struct RadioIqProcessor {
    config: RadioIqConfig,
    pending_i: Option<u8>,
    dc_estimate: Complex64,
    dc_alpha: f64,
    stages: Vec<HalfBandDecimator>,
}

impl Default for RadioIqProcessor {
    fn default() -> Self {
        Self::new(RadioIqConfig::default())
    }
}

impl RadioIqProcessor {
    pub fn new(config: RadioIqConfig) -> Self {
        let config = config.normalized();
        Self {
            config,
            pending_i: None,
            dc_estimate: Complex64::new(0.0, 0.0),
            dc_alpha: 1.0 - (-std::f64::consts::TAU * 5.0 / f64::from(config.input_rate)).exp(),
            stages: (0..config.decimation.trailing_zeros())
                .map(|_| HalfBandDecimator::new())
                .collect(),
        }
    }

    /// Return true if settings changed and stream state was reset. Reapplying
    /// unchanged settings preserves DC, FIR, byte pairing and decimator phase.
    pub fn configure(&mut self, config: RadioIqConfig) -> bool {
        let config = config.normalized();
        if self.config == config {
            return false;
        }
        *self = Self::new(config);
        true
    }

    pub fn config(&self) -> RadioIqConfig {
        self.config
    }

    pub fn output_rate(&self) -> f64 {
        f64::from(self.config.input_rate) / f64::from(self.config.decimation)
    }

    /// Use on capture restart/retune, not on ordinary source-buffer boundaries.
    pub fn reset(&mut self) {
        *self = Self::new(self.config);
    }

    pub fn process(&mut self, bytes: &[u8]) -> Vec<Complex32> {
        let pair_count = (bytes.len() + usize::from(self.pending_i.is_some())) / 2;
        let mut output = Vec::with_capacity(pair_count / self.config.decimation as usize + 1);
        let mut offset = 0;
        if let (Some(i), Some(&q)) = (self.pending_i, bytes.first()) {
            self.pending_i = None;
            self.push_pair(i, q, &mut output);
            offset = 1;
        }
        let mut pairs = bytes[offset..].chunks_exact(2);
        for pair in &mut pairs {
            self.push_pair(pair[0], pair[1], &mut output);
        }
        if let Some(&i) = pairs.remainder().first() {
            self.pending_i = Some(i);
        }
        output
    }

    fn push_pair(&mut self, i: u8, q: u8, output: &mut Vec<Complex32>) {
        // Same normalization as the existing byte demodulation and FFT paths.
        let mut sample = Complex32::new(
            (f32::from(i) - 127.4) / 128.0,
            (f32::from(q) - 127.4) / 128.0,
        );
        if self.config.dc_remove {
            let precise = Complex64::new(f64::from(sample.re), f64::from(sample.im));
            self.dc_estimate += (precise - self.dc_estimate) * self.dc_alpha;
            sample.re -= self.dc_estimate.re as f32;
            sample.im -= self.dc_estimate.im as f32;
        }
        if self.config.invert {
            sample.im = -sample.im;
        }
        for stage in &mut self.stages {
            let Some(filtered) = stage.push(sample) else {
                return;
            };
            sample = filtered;
        }
        output.push(sample);
    }
}

/// Independent digital VFO. For offset_hz = target RF − capture center RF,
/// multiply by exp(−j 2π offset_hz t), so a carrier at the target moves to DC.
/// Configuration preserves oscillator phase across VFO changes. Call reset()
/// for a discontinuous new capture. Out-of-band targets must be limited by UI.
pub struct VfoMixer {
    sample_rate: f64,
    offset_hz: f64,
    oscillator: Complex64,
    step: Complex64,
    normalization_phase: usize,
}

impl VfoMixer {
    pub fn new(sample_rate: f64) -> Self {
        let mut mixer = Self {
            sample_rate: 1.0,
            offset_hz: 0.0,
            oscillator: Complex64::new(1.0, 0.0),
            step: Complex64::new(1.0, 0.0),
            normalization_phase: 0,
        };
        mixer.configure(sample_rate, 0.0);
        mixer
    }

    pub fn configure(&mut self, sample_rate: f64, offset_hz: f64) {
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.max(1.0)
        } else {
            1.0
        };
        let offset_hz = if offset_hz.is_finite() {
            offset_hz
        } else {
            0.0
        };
        if self.sample_rate == sample_rate && self.offset_hz == offset_hz {
            return;
        }
        self.sample_rate = sample_rate;
        self.offset_hz = offset_hz;
        let angle = -std::f64::consts::TAU * (offset_hz / sample_rate).rem_euclid(1.0);
        self.step = Complex64::from_polar(1.0, angle);
    }

    pub fn reset(&mut self) {
        self.oscillator = Complex64::new(1.0, 0.0);
        self.normalization_phase = 0;
    }

    pub fn process(&mut self, samples: &[Complex32]) -> Vec<Complex32> {
        let mut output = Vec::with_capacity(samples.len());
        for &sample in samples {
            output.push(
                sample * Complex32::new(self.oscillator.re as f32, self.oscillator.im as f32),
            );
            self.oscillator *= self.step;
            self.normalization_phase += 1;
            if self.normalization_phase == 4096 {
                self.oscillator /= self.oscillator.norm();
                self.normalization_phase = 0;
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complex_tone(rate: f64, frequency: f64, count: usize) -> Vec<Complex32> {
        (0..count)
            .map(|n| {
                let angle = std::f64::consts::TAU * frequency * n as f64 / rate;
                Complex32::new(angle.cos() as f32 * 0.6, angle.sin() as f32 * 0.6)
            })
            .collect()
    }

    fn bytes(samples: &[Complex32]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| {
                [sample.re, sample.im]
                    .map(|value| (value * 128.0 + 127.4).round().clamp(0.0, 255.0) as u8)
            })
            .collect()
    }

    fn amplitude(samples: &[Complex32], rate: f64, frequency: f64) -> f64 {
        let sum: Complex64 = samples
            .iter()
            .enumerate()
            .map(|(n, sample)| {
                Complex64::new(f64::from(sample.re), f64::from(sample.im))
                    * Complex64::from_polar(
                        1.0,
                        -std::f64::consts::TAU * frequency * n as f64 / rate,
                    )
            })
            .sum();
        sum.norm() / samples.len() as f64
    }

    #[test]
    fn unity_path_preserves_normalization_and_odd_byte_boundaries() {
        let input = [0, 255, 127, 128, 48, 213, 75, 42];
        let mut processor = RadioIqProcessor::default();
        let mut actual = Vec::new();
        for byte in input {
            actual.extend(processor.process(&[byte]));
            assert!(processor.process(&[]).is_empty());
        }
        let expected: Vec<_> = input
            .chunks_exact(2)
            .map(|pair| {
                Complex32::new(
                    (f32::from(pair[0]) - 127.4) / 128.0,
                    (f32::from(pair[1]) - 127.4) / 128.0,
                )
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn every_decimation_factor_has_exact_counts_and_chunk_invariant_state() {
        let input = bytes(&complex_tone(2_048_003.0, 13_317.5, 65_539));
        for exponent in 0..=10 {
            let factor = 1 << exponent;
            let config = RadioIqConfig {
                input_rate: 2_048_003,
                dc_remove: true,
                invert: true,
                decimation: factor,
            };
            let reference = RadioIqProcessor::new(config).process(&input);
            let mut processor = RadioIqProcessor::new(config);
            let mut actual = Vec::new();
            for chunk in input.chunks(1031) {
                assert!(!processor.configure(config));
                actual.extend(processor.process(chunk));
            }
            assert_eq!(actual.len(), 65_539 / factor as usize, "factor {factor}");
            assert_eq!(actual, reference, "factor {factor}");
            assert_eq!(processor.output_rate(), 2_048_003.0 / f64::from(factor));
        }
    }

    #[test]
    fn antialias_filter_preserves_wanted_tones_and_rejects_folded_channels() {
        let input_rate = 1_024_000.0;
        for factor in [2, 4, 8, 16, 32, 64] {
            let output_rate = input_rate / f64::from(factor);
            let wanted_frequency = output_rate * 0.137;
            let config = RadioIqConfig {
                input_rate: input_rate as u32,
                decimation: factor,
                ..RadioIqConfig::default()
            };
            let measure = |frequency: f64, result_frequency: f64| {
                let source = bytes(&complex_tone(input_rate, frequency, 4096 * factor as usize));
                let output = RadioIqProcessor::new(config).process(&source);
                amplitude(&output[256..], output_rate, result_frequency)
            };
            let wanted = measure(wanted_frequency, wanted_frequency);
            let alias = measure(output_rate - wanted_frequency, -wanted_frequency);
            // Also exercise rejection at the first stage of a multistage chain.
            let early_alias = measure(input_rate * 0.5 + wanted_frequency, wanted_frequency);
            assert!((wanted - 0.6).abs() < 0.003, "factor {factor}: {wanted}");
            assert!(alias < wanted * 0.0032, "factor {factor}: {alias}/{wanted}");
            assert!(
                early_alias < wanted * 0.0032,
                "factor {factor}: {early_alias}/{wanted}"
            );
        }
    }

    #[test]
    fn dc_correction_rejects_offsets_without_erasing_a_nearby_tone() {
        let rate = 48_000.0;
        let input: Vec<_> = complex_tone(rate, 1200.0, rate as usize)
            .into_iter()
            .map(|value| value + Complex32::new(0.2, -0.15))
            .collect();
        let source = bytes(&input);
        let enabled = RadioIqConfig {
            input_rate: rate as u32,
            dc_remove: true,
            ..RadioIqConfig::default()
        };
        let corrected = RadioIqProcessor::new(enabled).process(&source);
        let uncorrected = RadioIqProcessor::new(RadioIqConfig {
            dc_remove: false,
            ..enabled
        })
        .process(&source);
        assert!(amplitude(&uncorrected[24_000..], rate, 0.0) > 0.24);
        assert!(amplitude(&corrected[24_000..], rate, 0.0) < 1e-5);
        assert!(amplitude(&corrected[24_000..], rate, 1200.0) > 0.595);
    }

    #[test]
    fn inversion_conjugates_iq_and_flips_the_frequency_sign() {
        let source = bytes(&complex_tone(48_000.0, 3000.0, 4800));
        let mut processor = RadioIqProcessor::new(RadioIqConfig {
            input_rate: 48_000,
            invert: true,
            ..RadioIqConfig::default()
        });
        let actual = processor.process(&source);
        let reference = RadioIqProcessor::default().process(&source);
        assert!(actual.iter().zip(reference).all(|(a, b)| *a == b.conj()));
        assert!(amplitude(&actual, 48_000.0, -3000.0) > 0.59);
        assert!(amplitude(&actual, 48_000.0, 3000.0) < 0.001);
    }

    #[test]
    fn configuration_is_bounded_and_reset_starts_a_fresh_stream() {
        for (requested, expected) in [(0, 1), (3, 2), (63, 32), (129, 128), (u32::MAX, 1024)] {
            let mut processor = RadioIqProcessor::new(RadioIqConfig {
                decimation: requested,
                ..RadioIqConfig::default()
            });
            assert_eq!(processor.config().decimation, expected);
            processor.process(&[12]);
            processor.reset();
            let input = bytes(&complex_tone(2_048_000.0, 33_000.0, 4096));
            assert_eq!(
                processor.process(&input),
                RadioIqProcessor::new(processor.config()).process(&input)
            );
        }
        let mut processor = RadioIqProcessor::default();
        processor.process(&[0]);
        assert!(processor.configure(RadioIqConfig {
            input_rate: 0,
            decimation: 64,
            ..RadioIqConfig::default()
        }));
        assert_eq!(processor.output_rate(), 1.0);
        assert!(processor.process(&[255]).is_empty());
    }

    #[test]
    fn vfo_moves_positive_and_negative_target_carriers_to_dc() {
        let rate = 192_003.0 / 4.0;
        for offset in [-10_321.25, 13_513.75, 0.0] {
            let mut mixer = VfoMixer::new(rate);
            mixer.configure(rate, offset);
            let actual = mixer.process(&complex_tone(rate, offset, 48_000));
            assert!(actual
                .iter()
                .all(|value| (*value - Complex32::new(0.6, 0.0)).norm() < 2e-6));
        }
    }

    #[test]
    fn vfo_subtracts_offset_and_preserves_fractional_phase_across_chunks() {
        let rate = 2_048_003.0 / 16.0;
        let offset = 27_171.375;
        let input = complex_tone(rate, offset + 1500.0, 80_007);
        let make_mixer = || {
            let mut mixer = VfoMixer::new(rate);
            mixer.configure(rate, offset);
            mixer
        };
        let expected = make_mixer().process(&input);
        let mut mixer = make_mixer();
        let actual: Vec<_> = input
            .chunks(509)
            .flat_map(|chunk| {
                mixer.configure(rate, offset);
                mixer.process(chunk)
            })
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), input.len());
        assert!(amplitude(&actual, rate, 1500.0) > 0.5999);
    }

    #[test]
    fn changing_vfo_preserves_phase_and_reset_is_explicit() {
        let mut mixer = VfoMixer::new(48_000.0);
        mixer.configure(48_000.0, 1375.25);
        mixer.process(&vec![Complex32::new(1.0, 0.0); 137]);
        let phase = mixer.oscillator;
        mixer.configure(48_000.0, -2411.5);
        let actual = mixer.process(&[Complex32::new(1.0, 0.0)]);
        assert_eq!(actual[0], Complex32::new(phase.re as f32, phase.im as f32));
        mixer.reset();
        assert_eq!(
            mixer.process(&[Complex32::new(1.0, 0.0)])[0],
            Complex32::new(1.0, 0.0)
        );
    }
}
