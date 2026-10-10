//! Shared band-limited I/Q and detected-audio rate conversion.
//! Exact clock phase, finite-input handling and root-duration accounting are
//! shared by daemon producers and GUI listening.

use crate::channelizer::FirDecimator;
use num_complex::Complex32;
use std::collections::VecDeque;

const PHASE_INTERVALS: usize = 256;

#[cfg(test)]
#[path = "audio_rate_clock_tests.rs"]
mod clock_tests;

pub struct RateFilter {
    banks: Vec<Vec<f32>>,
    history: Vec<f32>,
    write: usize,
    len: usize,
}

impl RateFilter {
    /// Tap and phase-bank counts, for bounded-storage validation.
    pub fn storage_shape(&self) -> (usize, usize) {
        (self.len, self.banks.len())
    }

    pub fn new(input_per_output: f64) -> Self {
        let ratio = input_per_output.max(1.0);
        let len = (100.0 * ratio).ceil() as usize | 1;
        let cutoff = 0.9 / ratio;
        let midpoint = (len - 1) as f64 / 2.0;
        let banks = (0..=PHASE_INTERVALS)
            .map(|bank| {
                let phase = bank as f64 / PHASE_INTERVALS as f64;
                let mut taps: Vec<f64> = (0..len)
                    .map(|n| {
                        let x = n as f64 - midpoint - phase;
                        let sinc = if x.abs() < 1e-12 {
                            cutoff
                        } else {
                            (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                        };
                        let angle = std::f64::consts::TAU * n as f64 / (len - 1) as f64;
                        let window = 0.35875 - 0.48829 * angle.cos()
                            + 0.14128 * (2.0 * angle).cos()
                            - 0.01168 * (3.0 * angle).cos();
                        sinc * window
                    })
                    .collect();
                let gain: f64 = taps.iter().sum();
                taps.iter_mut().for_each(|tap| *tap /= gain);
                taps.into_iter().map(|tap| tap as f32).collect()
            })
            .collect();
        Self {
            banks,
            history: vec![0.0; len * 2],
            write: 0,
            len,
        }
    }

    pub fn push(&mut self, sample: f32) {
        self.history[self.write] = sample;
        self.history[self.write + self.len] = sample;
        self.write = (self.write + 1) % self.len;
    }

    pub fn output(&self, phase: f64) -> f32 {
        let coordinate = phase * PHASE_INTERVALS as f64;
        let bank = coordinate.floor() as usize;
        let blend = (coordinate - bank as f64) as f32;
        let samples = &self.history[self.write..self.write + self.len];
        if blend == 0.0 {
            return samples
                .iter()
                .zip(&self.banks[bank])
                .map(|(x, tap)| x * tap)
                .sum();
        }
        // Select the fractional delay before rate conversion. Interpolating
        // coefficients is safe; interpolating unfiltered PCM would alias.
        samples
            .iter()
            .zip(&self.banks[bank])
            .zip(&self.banks[bank + 1])
            .map(|((&x, &a), &b)| x * (a + (b - a) * blend))
            .sum()
    }
}

#[derive(Default)]
pub struct IqAudioConverter {
    settings: Option<(f64, u32)>,
    clock_ratio: Option<(u32, u32)>,
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

impl IqAudioConverter {
    /// Convert an exact rational capture clock without rounding its duration or
    /// accumulating a dyadic approximation at coherent clock boundaries.
    pub fn process_clocked(
        &mut self,
        input: &[Complex32],
        source_rate: u32,
        stride: u32,
        output_rate: u32,
        gain: f32,
    ) -> Vec<[f32; 2]> {
        if source_rate == 0 || stride == 0 {
            self.reset();
            return Vec::new();
        }
        let input_rate = f64::from(source_rate) / f64::from(stride);
        let (mut a, mut b) = (source_rate, stride);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let ratio = (source_rate / a, stride / a);
        if self.clock_ratio != Some(ratio) || self.settings != Some((input_rate, output_rate)) {
            self.reset();
            if !self.configure(input_rate, output_rate) {
                return Vec::new();
            }
            let coarse = self.coarse_denominator / self.output_denominator;
            self.numerator = u128::from(ratio.0);
            self.output_denominator = u128::from(ratio.1) * u128::from(output_rate);
            self.coarse_denominator = self.output_denominator * coarse;
            self.duration_phase = self.numerator - 1;
            self.clock_ratio = Some(ratio);
        }
        self.process(input, input_rate, output_rate, gain)
    }

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
