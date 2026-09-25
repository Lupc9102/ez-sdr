//! Streaming CTCSS detection on unscaled FM discriminator audio.
//!
//! Feed the raw, pre-highpass/pre-volume NFM tap at its actual audio sample rate.
//! `process` returns one gain per sample for the corresponding audible stream;
//! decode-only operation ignores those gains. Power squelch is independent.
//! Detection is causal: opening takes about 540 ms, and closing includes the
//! analysis-window response plus 160 ms hang. No input audio is buffered for replay.

use serde::{Deserialize, Serialize};

/// Exact tone choices recovered from the installed reference radio module.
/// 150.0 Hz is included there in addition to the usual 50 CTCSS tones.
pub const CTCSS_TONES: &[f32] = &[
    67.0, 69.3, 71.9, 74.4, 77.0, 79.7, 82.5, 85.4, 88.5, 91.5, 94.8, 97.4, 100.0, 103.5, 107.2,
    110.9, 114.8, 118.8, 123.0, 127.3, 131.8, 136.5, 141.3, 146.2, 150.0, 151.4, 156.7, 159.8,
    162.2, 165.5, 167.9, 171.3, 173.8, 177.3, 179.9, 183.5, 186.2, 189.9, 192.8, 196.6, 199.5,
    203.5, 206.5, 210.7, 218.1, 225.7, 229.1, 233.6, 241.8, 250.3, 254.1,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SquelchMode {
    #[default]
    Off,
    Power,
    CtcssMute,
    CtcssDecode,
}

impl SquelchMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Power => "Power",
            Self::CtcssMute => "CTCSS (Mute)",
            Self::CtcssDecode => "CTCSS (Decode Only)",
        }
    }

    pub fn uses_ctcss(self) -> bool {
        matches!(self, Self::CtcssMute | Self::CtcssDecode)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CtcssStatus {
    /// None selects Any. A specific choice authorizes only that detected tone.
    pub selected_tone_hz: Option<f32>,
    /// A confidently identified standard tone, independent of the selection.
    pub detected_tone_hz: Option<f32>,
    /// Normalized coherent energy of the strongest candidate, in 0..=1.
    /// This is a signal metric, not a calibrated probability.
    pub confidence: f32,
    /// Logical authorization including hang; the output additionally ramps gain.
    pub gate_open: bool,
}

const WINDOW_SECONDS: f64 = 0.5;
const HOP_SECONDS: f64 = 0.04;
const HANG_SECONDS: f64 = 0.16;
const ACQUIRE_CONFIDENCE: f64 = 0.60;
const HOLD_CONFIDENCE: f64 = 0.45;
const MIN_DOMINANCE: f64 = 1.35;
// A tone must carry at least 0.25% of pre-lowpass discriminator energy. This
// rejects tiny stopband/alias residuals without imposing an absolute volume gate.
const MIN_INPUT_ENERGY_FRACTION: f64 = 0.0025;

#[derive(Default)]
struct Lowpass {
    b: [f64; 3],
    a: [f64; 2],
    state: [f64; 2],
}

impl Lowpass {
    fn configure(&mut self, sample_rate: f64, quality: f64) {
        let omega = std::f64::consts::TAU * 300.0 / sample_rate;
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / (2.0 * quality);
        let a0 = 1.0 + alpha;
        self.b = [
            (1.0 - cos) / (2.0 * a0),
            (1.0 - cos) / a0,
            (1.0 - cos) / (2.0 * a0),
        ];
        self.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
        self.state = [0.0; 2];
    }

    fn process(&mut self, sample: f64) -> f64 {
        let output = self.b[0] * sample + self.state[0];
        self.state[0] = self.b[1] * sample - self.a[0] * output + self.state[1];
        self.state[1] = self.b[2] * sample - self.a[1] * output;
        output
    }
}

/// Stateful tone detector and click-suppressed authorization envelope.
///
/// A DC blocker and sixth-order 300 Hz lowpass precede decimation to roughly
/// 1.2 kHz. Overlapping Hann windows are correlated against every supported
/// tone using Goertzel recurrences. Both tonal energy and dominance over the
/// next candidate are required; two consecutive qualified windows acquire.
/// The four analysis buffers total at most 2,800 samples at supported rates.
#[derive(Default)]
pub struct CtcssSquelch {
    rate: u32,
    selected: Option<f32>,
    selected_index: Option<usize>,
    valid: bool,
    lowpass: [Lowpass; 3],
    dc_previous_input: f64,
    dc_previous_output: f64,
    dc_pole: f64,
    decimation: usize,
    phase: usize,
    decimation_energy: f64,
    history: Vec<f64>,
    input_energy_history: Vec<f64>,
    window: Vec<f64>,
    scratch: Vec<f64>,
    coefficients: Vec<f64>,
    window_sum: f64,
    write: usize,
    filled: usize,
    hop: usize,
    since_analysis: usize,
    candidate: Option<usize>,
    candidate_windows: usize,
    detected: Option<usize>,
    confidence: f32,
    hang_remaining: usize,
    gain: f32,
    attack_step: f32,
    release_step: f32,
}

impl CtcssSquelch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discard all history and close the gate, for source/mode changes.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn status(&self) -> CtcssStatus {
        CtcssStatus {
            selected_tone_hz: self.selected,
            detected_tone_hz: self.detected.map(|index| CTCSS_TONES[index]),
            confidence: self.confidence,
            gate_open: self.hang_remaining > 0,
        }
    }

    /// Return one gain in 0..=1 per discriminator sample, retaining state across
    /// arbitrary chunks. Rate/selection changes reset and close the detector.
    /// None means Any; unsupported tones/rates fail closed. Input NaN/Inf is silence.
    /// Supported audio rates are 8..=384 kHz. No absolute RF/audio level is used.
    pub fn process(
        &mut self,
        discriminator: &[f32],
        sample_rate_hz: u32,
        selected_tone_hz: Option<f32>,
    ) -> Vec<f32> {
        if self.rate != sample_rate_hz || self.selected != selected_tone_hz {
            self.configure(sample_rate_hz, selected_tone_hz);
        }
        if !self.valid {
            return vec![0.0; discriminator.len()];
        }
        let mut gains = Vec::with_capacity(discriminator.len());
        for &sample in discriminator {
            self.hang_remaining = self.hang_remaining.saturating_sub(1);
            let input = if sample.is_finite() {
                f64::from(sample)
            } else {
                0.0
            };
            let mut filtered =
                input - self.dc_previous_input + self.dc_pole * self.dc_previous_output;
            self.dc_previous_input = input;
            self.dc_previous_output = filtered;
            self.decimation_energy += filtered * filtered;
            for stage in &mut self.lowpass {
                filtered = stage.process(filtered);
            }
            self.phase += 1;
            if self.phase == self.decimation {
                self.phase = 0;
                self.push_decimated(filtered, self.decimation_energy / self.decimation as f64);
                self.decimation_energy = 0.0;
            }
            if self.hang_remaining > 0 {
                self.gain = (self.gain + self.attack_step).min(1.0);
            } else {
                self.gain = (self.gain - self.release_step).max(0.0);
            }
            gains.push(self.gain);
        }
        gains
    }

    fn configure(&mut self, rate: u32, selected: Option<f32>) {
        self.reset();
        self.rate = rate;
        self.selected = selected;
        self.selected_index = selected.and_then(|tone| {
            CTCSS_TONES
                .iter()
                .position(|&standard| (tone - standard).abs() < 0.05)
        });
        if !(8_000..=384_000).contains(&rate)
            || (selected.is_some() && self.selected_index.is_none())
        {
            return;
        }
        self.valid = true;
        let rate = f64::from(rate);
        self.dc_pole = (-std::f64::consts::TAU * 25.0 / rate).exp();
        for (stage, quality) in self.lowpass.iter_mut().zip([
            0.517_638_090_205_041_5,
            std::f64::consts::FRAC_1_SQRT_2,
            1.931_851_652_578_136_6,
        ]) {
            stage.configure(rate, quality);
        }
        self.decimation = (rate / 1_200.0).floor().max(1.0) as usize;
        let detector_rate = rate / self.decimation as f64;
        let length = (detector_rate * WINDOW_SECONDS).round() as usize;
        self.history = vec![0.0; length];
        self.input_energy_history = vec![0.0; length];
        self.scratch = vec![0.0; length];
        self.window = (0..length)
            .map(|index| {
                0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / (length - 1) as f64).cos()
            })
            .collect();
        self.window_sum = self.window.iter().sum();
        self.coefficients = CTCSS_TONES
            .iter()
            .map(|&tone| 2.0 * (std::f64::consts::TAU * f64::from(tone) / detector_rate).cos())
            .collect();
        self.hop = (detector_rate * HOP_SECONDS).round().max(1.0) as usize;
        self.attack_step = (1.0 / (rate * 0.005)) as f32;
        self.release_step = (1.0 / (rate * 0.010)) as f32;
    }

    fn push_decimated(&mut self, sample: f64, input_energy: f64) {
        self.history[self.write] = sample;
        self.input_energy_history[self.write] = input_energy;
        self.write = (self.write + 1) % self.history.len();
        self.filled = (self.filled + 1).min(self.history.len());
        self.since_analysis += 1;
        if self.filled == self.history.len() && self.since_analysis >= self.hop {
            self.since_analysis = 0;
            self.analyze();
        }
    }

    fn analyze(&mut self) {
        let mut energy = 0.0;
        let mut input_energy = 0.0;
        for index in 0..self.history.len() {
            let history_index = (self.write + index) % self.history.len();
            let sample = self.history[history_index];
            self.scratch[index] = sample * self.window[index];
            energy += sample * sample * self.window[index];
            input_energy += self.input_energy_history[history_index] * self.window[index];
        }
        let mut strongest = (0, 0.0_f64);
        let mut second_power = 0.0_f64;
        for (index, &coefficient) in self.coefficients.iter().enumerate() {
            let (mut previous, mut older) = (0.0, 0.0);
            for &sample in &self.scratch {
                let next = sample + coefficient * previous - older;
                older = previous;
                previous = next;
            }
            let power =
                (previous * previous + older * older - coefficient * previous * older).max(0.0);
            if power > strongest.1 {
                second_power = strongest.1;
                strongest = (index, power);
            } else {
                second_power = second_power.max(power);
            }
        }
        // Merely rejects numerical silence; discrimination is scale invariant.
        let coherence = if energy > 1e-20 {
            (2.0 * strongest.1 / (energy * self.window_sum)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.confidence = coherence as f32;
        let threshold = if self.detected == Some(strongest.0) {
            HOLD_CONFIDENCE
        } else {
            ACQUIRE_CONFIDENCE
        };
        let input_fraction = if input_energy > 1e-20 {
            2.0 * strongest.1 / (input_energy * self.window_sum)
        } else {
            0.0
        };
        let qualifies = coherence >= threshold
            && strongest.1 >= second_power * MIN_DOMINANCE
            && input_fraction >= MIN_INPUT_ENERGY_FRACTION;
        if !qualifies {
            self.candidate = None;
            self.candidate_windows = 0;
            self.detected = None;
            return;
        }
        if self.candidate == Some(strongest.0) {
            self.candidate_windows = self.candidate_windows.saturating_add(1);
        } else {
            self.candidate = Some(strongest.0);
            self.candidate_windows = 1;
            self.detected = None;
        }
        if self.candidate_windows >= 2 {
            self.detected = self.candidate;
            if self.selected_index.is_none() || self.selected_index == self.detected {
                self.hang_remaining = (f64::from(self.rate) * HANG_SECONDS).round() as usize;
            }
        }
    }
}

/// CW carrier power squelch: tracks the audio envelope and gates the output
/// while it stays below a user threshold in dB.
///
/// The envelope follows the same rate-independent asymmetric estimator as the
/// AGC in `demod.rs`: attack and release are speeds in inverse seconds derived
/// from the requested millisecond time constants. A separate gain ramp, also
/// timed in milliseconds, smooths gate clicks, so `process` returns the input
/// sample scaled by the current gain: the sample itself when fully open and
/// exactly `0.0` when closed.
#[derive(Default)]
pub struct CwPowerSquelch {
    envelope: f64,
    threshold_linear: f64,
    attack_step: f32,
    release_step: f32,
    attack_rate: f64,
    release_rate: f64,
    sample_rate: f64,
    gain: f32,
}

impl CwPowerSquelch {
    /// Time constants are full-range envelope/gate travel times in
    /// milliseconds; the envelope smoothing uses the same rates.
    pub fn new(attack_ms: f32, release_ms: f32) -> Self {
        let attack_rate = 1000.0 / attack_ms.max(1.0) as f64;
        let release_rate = 1000.0 / release_ms.max(1.0) as f64;
        let mut squelch = Self {
            attack_rate,
            release_rate,
            sample_rate: 48_000.0,
            threshold_linear: 1e-5,
            ..Self::default()
        };
        squelch.configure(48_000);
        squelch
    }

    /// Recompute per-sample gain steps after a rate change. Without this the
    /// millisecond time constants only hold at the 48 kHz default.
    pub fn configure(&mut self, sample_rate_hz: u32) {
        let rate = f64::from(sample_rate_hz).max(1.0);
        self.sample_rate = rate;
        self.attack_step = (self.attack_rate / rate).clamp(0.0, 1.0) as f32;
        self.release_step = (self.release_rate / rate).clamp(0.0, 1.0) as f32;
    }

    /// Gate threshold in dB relative to full-scale audio amplitude.
    pub fn set_cw_squelch_level(&mut self, level_db: f32) {
        let level_db = level_db.clamp(-120.0, 120.0);
        self.threshold_linear = 10_f64.powf(f64::from(level_db) / 20.0);
    }

    /// Current gate gain in 0..=1; 1 means the sample is passed unchanged.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Return the sample scaled by the gate gain: the sample while the
    /// envelope holds above threshold, ramping on attack and release, and
    /// exactly `0.0` while closed. Non-finite input counts as silence.
    pub fn process(&mut self, sample: f32) -> f32 {
        let input = if sample.is_finite() {
            sample
        } else {
            0.0
        };
        let magnitude = f64::from(input).abs();
        let speed = if magnitude > self.envelope {
            self.attack_rate
        } else {
            self.release_rate
        };
        let alpha = (speed / self.sample_rate).clamp(0.0, 1.0);
        self.envelope += alpha * (magnitude - self.envelope);
        let target = if self.envelope > self.threshold_linear {
            1.0_f32
        } else {
            0.0_f32
        };
        let step = if target > self.gain {
            self.attack_step
        } else {
            self.release_step
        };
        if (target - self.gain).abs() <= step {
            self.gain = target;
        } else if target > self.gain {
            self.gain += step;
        } else {
            self.gain -= step;
        }
        if input.is_finite() && self.gain > 0.0 {
            input * self.gain
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(rate: u32, seconds: f64, sample: impl Fn(f64) -> f32) -> Vec<f32> {
        (0..(f64::from(rate) * seconds).round() as usize)
            .map(|index| sample(index as f64 / f64::from(rate)))
            .collect()
    }

    fn tone(rate: u32, frequency: f32, seconds: f64, amplitude: f32) -> Vec<f32> {
        signal(rate, seconds, |time| {
            amplitude * (std::f64::consts::TAU * f64::from(frequency) * time + 0.31).sin() as f32
        })
    }

    fn noisy_voice(rate: u32, seconds: f64, add_tone: Option<f32>) -> Vec<f32> {
        let mut seed = 0x9766_35ab_u32;
        let mut samples = signal(rate, seconds, |time| {
            // Moving voiced fundamental plus harmonics; no sustained CTCSS note.
            let phase = std::f64::consts::TAU
                * (170.0 * time + 6.0 * (std::f64::consts::TAU * 3.0 * time).sin());
            (0.1 * phase.sin()
                + 0.08 * (2.0 * phase).sin()
                + 0.05 * (3.0 * phase).sin()
                + 0.03 * (std::f64::consts::TAU * 700.0 * time).sin()) as f32
        });
        for (index, sample) in samples.iter_mut().enumerate() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *sample += 0.06 * (seed as f64 / f64::from(u32::MAX) - 0.5) as f32;
            if let Some(frequency) = add_tone {
                *sample += 0.15
                    * (std::f64::consts::TAU * f64::from(frequency) * index as f64
                        / f64::from(rate))
                    .sin() as f32;
            }
        }
        samples
    }

    #[test]
    fn reference_modes_and_exact_tone_choices_round_trip() {
        assert_eq!(SquelchMode::default(), SquelchMode::Off);
        for (mode, key) in [
            (SquelchMode::Off, "off"),
            (SquelchMode::Power, "power"),
            (SquelchMode::CtcssMute, "ctcss_mute"),
            (SquelchMode::CtcssDecode, "ctcss_decode"),
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{key}\""));
            assert_eq!(serde_json::from_str::<SquelchMode>(&json).unwrap(), mode);
        }
        assert_eq!(CTCSS_TONES.len(), 51);
        assert!(CTCSS_TONES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(CTCSS_TONES.contains(&150.0));
    }

    #[test]
    fn all_reference_tones_acquire_at_44100_and_48000_hz() {
        for rate in [44_100, 48_000] {
            for &frequency in CTCSS_TONES {
                let mut detector = CtcssSquelch::new();
                let gains =
                    detector.process(&tone(rate, frequency, 0.8, 0.03), rate, Some(frequency));
                let status = detector.status();
                assert_eq!(
                    status.detected_tone_hz,
                    Some(frequency),
                    "{rate}: {frequency}"
                );
                assert_eq!(status.selected_tone_hz, Some(frequency));
                assert!(status.gate_open, "{rate}: {frequency}");
                assert!(status.confidence > 0.98, "{rate}: {frequency}: {status:?}");
                assert!(gains[gains.len() - rate as usize / 10..]
                    .iter()
                    .all(|gain| *gain == 1.0));
            }
        }
    }

    #[test]
    fn adjacent_tones_are_decoded_but_do_not_authorize_selected_tone() {
        for rate in [44_100, 48_000] {
            for (selected, received) in [
                (67.0, 69.3),
                (69.3, 67.0),
                (100.0, 103.5),
                (150.0, 151.4),
                (151.4, 150.0),
                (159.8, 162.2),
                (254.1, 250.3),
            ] {
                let mut detector = CtcssSquelch::new();
                let gains =
                    detector.process(&tone(rate, received, 0.8, 0.03), rate, Some(selected));
                assert!(
                    gains.iter().all(|gain| *gain == 0.0),
                    "{rate}: {selected}/{received}"
                );
                assert_eq!(detector.status().detected_tone_hz, Some(received));
                assert!(!detector.status().gate_open);
            }
        }
    }

    #[test]
    fn any_tone_authorizes_without_changing_detected_label() {
        let mut detector = CtcssSquelch::new();
        detector.process(&tone(48_000, 123.0, 0.8, 0.03), 48_000, None);
        let status = detector.status();
        assert_eq!(status.selected_tone_hz, None);
        assert_eq!(status.detected_tone_hz, Some(123.0));
        assert!(status.gate_open);
    }

    #[test]
    fn silence_dc_noise_and_moving_voice_do_not_open() {
        for rate in [44_100, 48_000] {
            let mut seed = 0xa1b2_c3d4_u32;
            let noise: Vec<f32> = (0..rate as usize * 2)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed as f64 / f64::from(u32::MAX) - 0.5) as f32
                })
                .collect();
            for samples in [
                vec![0.0; rate as usize],
                vec![0.3; rate as usize],
                noise,
                noisy_voice(rate, 2.0, None),
            ] {
                let mut detector = CtcssSquelch::new();
                let gains = detector.process(&samples, rate, None);
                assert!(
                    gains.iter().all(|gain| *gain == 0.0),
                    "{rate}: {:?}",
                    detector.status()
                );
                assert_eq!(detector.status().detected_tone_hz, None);
            }
        }
    }

    #[test]
    fn selected_tone_survives_voice_noise_and_dc_offset() {
        for rate in [44_100, 48_000] {
            let mut detector = CtcssSquelch::new();
            let samples: Vec<f32> = noisy_voice(rate, 1.3, Some(100.0))
                .into_iter()
                .map(|sample| sample + 0.4)
                .collect();
            let gains = detector.process(&samples, rate, Some(100.0));
            assert_eq!(detector.status().detected_tone_hz, Some(100.0));
            assert!(detector.status().gate_open);
            assert!(gains[gains.len() - rate as usize / 10..]
                .iter()
                .all(|gain| *gain == 1.0));
        }
    }

    #[test]
    fn confidence_is_scale_independent_and_ambiguous_tones_stay_closed() {
        let mut confidences = Vec::new();
        for amplitude in [0.03, 0.000_003] {
            let mut detector = CtcssSquelch::new();
            detector.process(&tone(48_000, 100.0, 0.8, amplitude), 48_000, Some(100.0));
            assert!(detector.status().gate_open);
            confidences.push(detector.status().confidence);
        }
        assert!((confidences[0] - confidences[1]).abs() < 0.001);
        let mixed = signal(48_000, 1.0, |time| {
            (0.1 * (std::f64::consts::TAU * 100.0 * time).sin()
                + 0.1 * (std::f64::consts::TAU * 123.0 * time).sin()) as f32
        });
        let mut detector = CtcssSquelch::new();
        assert!(detector
            .process(&mixed, 48_000, None)
            .iter()
            .all(|gain| *gain == 0.0));
        assert_eq!(detector.status().detected_tone_hz, None);
    }

    #[test]
    fn weak_tone_is_recovered_beneath_stronger_voice_band_signals() {
        for rate in [44_100, 48_000] {
            let samples = signal(rate, 1.0, |time| {
                (0.015 * (std::f64::consts::TAU * 100.0 * time).sin()
                    + 0.20 * (std::f64::consts::TAU * 500.0 * time).sin()
                    + 0.15 * (std::f64::consts::TAU * 1_300.0 * time).sin()) as f32
            });
            let mut detector = CtcssSquelch::new();
            detector.process(&samples, rate, Some(100.0));
            assert!(
                detector.status().gate_open,
                "{rate}: {:?}",
                detector.status()
            );
            assert_eq!(detector.status().detected_tone_hz, Some(100.0));
        }
    }

    #[test]
    fn voice_band_aliases_do_not_masquerade_as_subaudible_tones() {
        for rate in [44_100, 48_000] {
            let factor = (f64::from(rate) / 1_200.0).floor();
            let detector_rate = f64::from(rate) / factor;
            for desired_tone in [67.0, 100.0, 150.0, 254.1] {
                for alias in [detector_rate - desired_tone, detector_rate + desired_tone] {
                    let mut detector = CtcssSquelch::new();
                    let gains = detector.process(&tone(rate, alias as f32, 1.0, 0.5), rate, None);
                    assert!(
                        gains.iter().all(|gain| *gain == 0.0),
                        "{rate}: alias {alias}"
                    );
                    assert_eq!(detector.status().detected_tone_hz, None);
                }
            }
        }
    }

    #[test]
    fn small_oscillator_errors_are_accepted_but_close_tone_midpoint_is_ambiguous() {
        for (selected, received) in [
            (100.0, 99.8),
            (100.0, 100.2),
            (150.0, 149.9),
            (254.1, 254.5),
        ] {
            let mut detector = CtcssSquelch::new();
            detector.process(&tone(48_000, received, 0.8, 0.03), 48_000, Some(selected));
            assert!(
                detector.status().gate_open,
                "{selected}/{received}: {:?}",
                detector.status()
            );
        }
        let mut detector = CtcssSquelch::new();
        let gains = detector.process(&tone(48_000, 150.7, 1.0, 0.03), 48_000, None);
        assert!(gains.iter().all(|gain| *gain == 0.0));
        assert_eq!(detector.status().detected_tone_hz, None);
    }

    #[test]
    fn acquisition_confirmation_hang_and_smooth_release_have_bounded_timing() {
        let rate = 48_000;
        let mut detector = CtcssSquelch::new();
        let gains = detector.process(&tone(rate, 100.0, 1.0, 0.03), rate, Some(100.0));
        let opened = gains.iter().position(|gain| *gain > 0.0).unwrap();
        assert!((25_440..=26_000).contains(&opened), "opened at {opened}");
        assert!(gains[opened..opened + 200]
            .windows(2)
            .all(|pair| pair[1] > pair[0]));

        // A 100 ms fade must neither chatter nor close an established tone gate.
        let dropout = detector.process(&vec![0.0; rate as usize / 10], rate, Some(100.0));
        assert!(dropout.iter().all(|gain| *gain == 1.0));
        let rest = detector.process(&vec![0.0; rate as usize], rate, Some(100.0));
        let released = rest.iter().position(|gain| *gain < 1.0).unwrap() + rate as usize / 10;
        assert!(
            (rate as usize / 5..rate as usize * 3 / 5).contains(&released),
            "released at {released}"
        );
        let first_fall = rest.iter().position(|gain| *gain < 1.0).unwrap();
        let closed = rest.iter().position(|gain| *gain == 0.0).unwrap();
        assert!((470..=490).contains(&(closed - first_fall)));
        assert!(rest[first_fall..=closed]
            .windows(2)
            .all(|pair| pair[1] <= pair[0]));
        assert!(!detector.status().gate_open);
        assert_eq!(detector.status().detected_tone_hz, None);
        assert!(detector.status().confidence < 0.01);
    }

    #[test]
    fn detector_and_gate_are_exactly_chunk_invariant() {
        for rate in [44_100, 48_000] {
            let mut samples = noisy_voice(rate, 1.0, Some(100.0));
            samples.extend(vec![0.0; rate as usize * 3 / 4]);
            samples.extend(tone(rate, 103.5, 0.8, 0.03));
            let mut whole = CtcssSquelch::new();
            let expected = whole.process(&samples, rate, Some(100.0));
            let mut chunked = CtcssSquelch::new();
            let mut actual = Vec::new();
            let sizes = [1, 17, 512, 3, 2048, 63, 71];
            let mut offset = 0;
            let mut block = 0;
            while offset < samples.len() {
                let end = (offset + sizes[block % sizes.len()]).min(samples.len());
                actual.extend(chunked.process(&samples[offset..end], rate, Some(100.0)));
                offset = end;
                block += 1;
            }
            assert_eq!(actual, expected);
            assert_eq!(chunked.status(), whole.status());
        }
    }

    #[test]
    fn configuration_changes_and_reset_discard_previous_authorization() {
        let mut detector = CtcssSquelch::new();
        detector.process(&tone(44_100, 100.0, 0.8, 0.03), 44_100, Some(100.0));
        assert!(detector.status().gate_open);
        assert_eq!(detector.process(&[0.0; 10], 48_000, Some(100.0)), [0.0; 10]);
        assert!(!detector.status().gate_open);
        detector.process(&tone(48_000, 100.0, 0.8, 0.03), 48_000, Some(100.0));
        assert!(detector.status().gate_open);
        assert_eq!(detector.process(&[0.0; 10], 48_000, Some(123.0)), [0.0; 10]);
        assert_eq!(detector.status().detected_tone_hz, None);
        detector.reset();
        assert_eq!(detector.status(), CtcssStatus::default());
    }

    #[test]
    fn unsupported_configuration_and_nonfinite_input_fail_closed() {
        let mut detector = CtcssSquelch::new();
        for (rate, selected) in [
            (0, None),
            (1, None),
            (48_000, Some(101.0)),
            (48_000, Some(f32::NAN)),
        ] {
            let gains = detector.process(&[0.5; 10], rate, selected);
            assert_eq!(gains, [0.0; 10]);
            assert!(!detector.status().gate_open);
        }
        let gains = detector.process(
            &[f32::NAN, f32::INFINITY, f32::NEG_INFINITY].repeat(48_000),
            48_000,
            None,
        );
        assert!(gains.iter().all(|gain| *gain == 0.0));
        assert!(detector.status().confidence.is_finite());
        assert!(detector.history.len() <= 700);
        assert_eq!(detector.scratch.len(), detector.history.len());
    }

    fn cw_tone(rate: u32, seconds: f64, amplitude: f32) -> Vec<f32> {
        signal(rate, seconds, |time| {
            amplitude * (std::f64::consts::TAU * 600.0 * time).sin() as f32
        })
    }

    fn gated(squelch: &mut CwPowerSquelch, samples: &[f32]) -> Vec<f32> {
        samples.iter().map(|&sample| squelch.process(sample)).collect()
    }

    #[test]
    fn cw_squelch_passes_audio_when_signal_is_above_threshold() {
        let mut squelch = CwPowerSquelch::new(5.0, 25.0);
        squelch.configure(48_000);
        squelch.set_cw_squelch_level(-60.0);
        // Establish the gate with silence-adjacent headroom: a unit-amplitude
        // tone is far above the -60 dB threshold.
        let tone = cw_tone(48_000, 0.2, 1.0);
        let output = gated(&mut squelch, &tone);
        assert_eq!(squelch.gain(), 1.0);
        let settled = &output[output.len() - 1_000..];
        let expected = &tone[tone.len() - 1_000..];
        assert!(settled
            .iter()
            .zip(expected)
            .all(|(actual, passed)| (actual - passed).abs() < 1e-6));
    }

    #[test]
    fn cw_squelch_gates_audio_when_signal_is_below_threshold() {
        let mut squelch = CwPowerSquelch::new(5.0, 25.0);
        squelch.configure(48_000);
        squelch.set_cw_squelch_level(-20.0);
        // Silence never opens the gate.
        assert!(gated(&mut squelch, &[0.0; 4_800])
            .iter()
            .all(|sample| *sample == 0.0));
        assert_eq!(squelch.gain(), 0.0);
        // A -60 dB tone stays 40 dB under the threshold and is fully muted.
        let weak = cw_tone(48_000, 0.2, 1e-3);
        let output = gated(&mut squelch, &weak);
        assert!(output.iter().all(|sample| *sample == 0.0));
        assert_eq!(squelch.gain(), 0.0);
        // The same tone opens at a sensitive threshold.
        squelch.set_cw_squelch_level(-80.0);
        let output = gated(&mut squelch, &weak);
        assert_eq!(squelch.gain(), 1.0);
        assert!(output
            .iter()
            .rev()
            .take(1_000)
            .all(|sample| *sample != 0.0));
    }

    #[test]
    fn cw_squelch_smoothly_attacks_and_releases() {
        let rate = 48_000;
        let mut squelch = CwPowerSquelch::new(5.0, 25.0);
        squelch.configure(rate);
        squelch.set_cw_squelch_level(-60.0);
        // Quiet head: the gate starts closed.
        assert!(gated(&mut squelch, &[0.0; 4_800])
            .iter()
            .all(|sample| *sample == 0.0));
        assert_eq!(squelch.gain(), 0.0);
        // Attack: once the envelope crosses, gain rises monotonically to 1
        // and bounded attack timing keeps the ramp near the 5 ms constant.
        let tone = cw_tone(rate, 0.1, 1.0);
        let mut gains = Vec::with_capacity(tone.len());
        for &sample in &tone {
            squelch.process(sample);
            gains.push(squelch.gain());
        }
        let first = gains.iter().position(|gain| *gain > 0.0).unwrap();
        assert!(first > 0);
        assert!(gains[first..].windows(2).all(|pair| pair[1] >= pair[0]));
        assert_eq!(squelch.gain(), 1.0);
        // Release: a half second of silence closes the gate monotonically.
        let mut gains = Vec::new();
        for _ in 0..24_000 {
            squelch.process(0.0);
            gains.push(squelch.gain());
        }
        assert!(gains.windows(2).all(|pair| pair[1] <= pair[0]));
        assert_eq!(*gains.last().unwrap(), 0.0);
        assert_eq!(squelch.gain(), 0.0);
        // The envelope re-opens promptly on a fresh carrier.
        let reopened: Vec<f32> = tone
            .iter()
            .map(|&sample| squelch.process(sample))
            .collect();
        assert_eq!(squelch.gain(), 1.0);
        assert!(reopened
            .iter()
            .rev()
            .take(100)
            .all(|sample| *sample != 0.0));
    }

    #[test]
    fn cw_squelch_strong_signal_at_various_levels_passes_through() {
        for level_db in [-40.0, -20.0, 0.0] {
            let mut squelch = CwPowerSquelch::new(5.0, 25.0);
            squelch.configure(48_000);
            squelch.set_cw_squelch_level(level_db);
            let threshold_linear = 10_f64.powf(f64::from(level_db) / 20.0);
            let amplitude = (threshold_linear * 10.0) as f32;
            let tone = cw_tone(48_000, 0.2, amplitude);
            let output = gated(&mut squelch, &tone);
            assert_eq!(squelch.gain(), 1.0, "level_db: {level_db}");
            let settled = &output[output.len() - 1_000..];
            let expected = &tone[tone.len() - 1_000..];
            assert!(
                settled
                    .iter()
                    .zip(expected)
                    .all(|(actual, passed)| (actual - passed).abs() < 1e-6),
                "level_db: {level_db}"
            );
        }
    }

    #[test]
    fn cw_squelch_weak_signal_various_levels_gated() {
        for level_db in [-40.0, -20.0, 0.0] {
            let mut squelch = CwPowerSquelch::new(5.0, 25.0);
            squelch.configure(48_000);
            squelch.set_cw_squelch_level(level_db);
            let threshold_linear = 10_f64.powf(f64::from(level_db) / 20.0);
            let amplitude = (threshold_linear * 0.1) as f32;
            let tone = cw_tone(48_000, 0.2, amplitude);
            let output = gated(&mut squelch, &tone);
            assert!(
                output.iter().all(|&sample| sample == 0.0),
                "level_db: {level_db}"
            );
            assert_eq!(squelch.gain(), 0.0, "level_db: {level_db}");
        }
    }

    #[test]
    fn cw_squelch_attack_ramp_is_monotonic_and_bounded() {
        let rate = 48_000;
        let mut squelch = CwPowerSquelch::new(10.0, 50.0);
        squelch.configure(rate);
        squelch.set_cw_squelch_level(-60.0);
        for _ in 0..4_800 {
            squelch.process(0.0);
        }
        assert_eq!(squelch.gain(), 0.0);
        let tone = cw_tone(rate, 0.05, 1.0);
        let mut gains = Vec::with_capacity(tone.len());
        for &sample in &tone {
            squelch.process(sample);
            gains.push(squelch.gain());
        }
        let first = gains.iter().position(|&gain| gain > 0.0).unwrap();
        assert!(gains[first..].windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(gains.iter().all(|&gain| gain <= 1.0));
        let max_step = gains[first..]
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(0.0_f32, f32::max);
        assert!(max_step < 0.01, "max step: {max_step}");
        assert_eq!(squelch.gain(), 1.0);
    }

    #[test]
    fn cw_squelch_release_fade_is_monotonic_and_continuous() {
        let rate = 48_000;
        let mut squelch = CwPowerSquelch::new(5.0, 25.0);
        squelch.configure(rate);
        squelch.set_cw_squelch_level(-60.0);
        let tone = cw_tone(rate, 0.1, 1.0);
        for &sample in &tone {
            squelch.process(sample);
        }
        assert_eq!(squelch.gain(), 1.0);
        let mut gains = Vec::with_capacity(24_000);
        for _ in 0..24_000 {
            squelch.process(0.0);
            gains.push(squelch.gain());
        }
        assert!(gains.windows(2).all(|pair| pair[1] <= pair[0]));
        let max_delta = gains
            .windows(2)
            .map(|pair| pair[0] - pair[1])
            .fold(0.0_f32, f32::max);
        assert!(max_delta < 0.01, "max delta: {max_delta}");
        assert_eq!(*gains.last().unwrap(), 0.0);
        assert_eq!(squelch.gain(), 0.0);
    }

    #[test]
    fn cw_squelch_level_change_updates_threshold() {
        let tone = cw_tone(48_000, 0.1, 1e-3);
        let mut squelch = CwPowerSquelch::new(5.0, 25.0);
        squelch.configure(48_000);
        squelch.set_cw_squelch_level(-80.0);
        let output1 = gated(&mut squelch, &tone);
        assert_eq!(squelch.gain(), 1.0);
        assert!(output1.iter().rev().take(100).all(|&s| s != 0.0));
        squelch.set_cw_squelch_level(-20.0);
        let mut squelch2 = CwPowerSquelch::new(5.0, 25.0);
        squelch2.configure(48_000);
        squelch2.set_cw_squelch_level(-20.0);
        let output2 = gated(&mut squelch2, &tone);
        assert!(output2.iter().all(|&s| s == 0.0));
        assert_eq!(squelch2.gain(), 0.0);
        squelch2.set_cw_squelch_level(-80.0);
        let output3 = gated(&mut squelch2, &tone);
        assert_eq!(squelch2.gain(), 1.0);
        assert!(output3.iter().rev().take(100).all(|&s| s != 0.0));
    }
}
