//! Audio and baseband DSP blocks translated from SDR++ legacy C++ sources.
//!
//! Covers:
//! - Volume control (`Volume`)
//! - Rational sample-rate conversion (`RationalResampler`)
//! - Simple impulse noise blanking (`NoiseBlanker`)
//!
//! The designs below keep the signal-processing behaviour of the originals
//! (polyphase FIR resampling, cascaded power-of-two decimation, exponential
//! moving-average blanker) while using idiomatic Rust (traits for generic
//! dot-products, `Vec` for dynamic storage, safe slice operations, etc.).

use num_complex::Complex;
use std::f64::consts::PI;

// Re-use the canonical DSP types so the whole crate speaks one sample language.
pub use crate::dsp::Stereo;

// ===========================================================================
// Generic dot-product trait used by FIR/polyphase kernels
// ===========================================================================

/// A sample type that can participate in a real-coefficient FIR convolution.
pub trait ResampleSample:
    Copy + Default + std::ops::Mul<f32, Output = Self> + std::iter::Sum<Self>
{
    fn dot(a: &[Self], b: &[f32]) -> Self;
}

impl ResampleSample for f32 {
    #[inline]
    fn dot(a: &[Self], b: &[f32]) -> Self {
        a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
    }
}

impl ResampleSample for Complex<f32> {
    #[inline]
    fn dot(a: &[Self], b: &[f32]) -> Self {
        a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
    }
}

impl ResampleSample for Stereo {
    #[inline]
    fn dot(a: &[Self], b: &[f32]) -> Self {
        a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
    }
}

// ===========================================================================
// Window / tap generation helpers
// ===========================================================================

/// Nuttall window (4-term) evaluated at fractional position `n / n_max`.
fn nuttall(n: f64, n_max: f64) -> f64 {
    let coefs = [0.355768, 0.487396, 0.144232, 0.012604];
    let mut win = 0.0;
    let mut sign = 1.0;
    for (i, &c) in coefs.iter().enumerate() {
        win += sign * c * ((i as f64) * 2.0 * PI * n / n_max).cos();
        sign = -sign;
    }
    win
}

#[inline]
fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        x.sin() / x
    }
}

/// Generate a real low-pass FIR via windowed sinc (Nuttall window).
///
/// `cutoff` and `trans_width` are in Hz; `sample_rate` is in Hz.
fn low_pass_taps(cutoff: f64, trans_width: f64, sample_rate: f64) -> Vec<f32> {
    let count = ((3.8 * sample_rate / trans_width) as usize).max(1);
    let half = count as f64 / 2.0;
    let omega = 2.0 * PI * cutoff / sample_rate;
    let corr = omega / PI;
    (0..count)
        .map(|i| {
            let t = i as f64 - half + 0.5;
            let val = sinc(t * omega) * nuttall(t - half, count as f64) * corr;
            val as f32
        })
        .collect()
}

// ===========================================================================
// Volume
// ===========================================================================

/// Simple per-sample gain block with squared-volume mapping and soft-mute.
pub struct Volume {
    volume: f32,
    muted: bool,
}

impl Volume {
    pub fn new(volume: f32, muted: bool) -> Self {
        let mut s = Self { volume: 0.0, muted };
        s.set_volume(volume);
        s
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume * volume;
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    #[inline]
    pub fn process(&mut self, samples: &mut [Stereo]) {
        let gain = if self.muted { 0.0 } else { self.volume };
        if gain == 1.0 {
            return;
        }
        for s in samples {
            s.l *= gain;
            s.r *= gain;
        }
    }
}

// ===========================================================================
// Decimating FIR
// ===========================================================================

/// FIR filter with an integer decimation factor.  Keeps its own delay line.
pub struct DecimatingFir<T: ResampleSample> {
    taps: Vec<f32>,
    decimation: usize,
    buffer: Vec<T>,
    buf_start: usize,
    offset: isize,
}

impl<T: ResampleSample> DecimatingFir<T> {
    pub fn new(taps: Vec<f32>, decimation: usize) -> Self {
        let delay = taps.len().saturating_sub(1);
        Self {
            taps,
            decimation: decimation.max(1),
            buffer: vec![T::default(); delay + 1_064_000],
            buf_start: delay,
            offset: 0,
        }
    }

    pub fn reset(&mut self) {
        self.offset = 0;
        self.buffer.fill(T::default());
    }

    #[inline]
    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        if self.taps.is_empty() {
            let n = input.len().min(output.len());
            output[..n].copy_from_slice(&input[..n]);
            return n;
        }

        let count = input.len();
        self.buffer[self.buf_start..self.buf_start + count]
            .copy_from_slice(input);

        let mut out_count = 0;
        let mut offset = self.offset;
        while offset < count as isize {
            let idx = offset as usize;
            output[out_count] = T::dot(&self.buffer[idx..idx + self.taps.len()], &self.taps);
            out_count += 1;
            if out_count >= output.len() {
                break;
            }
            offset += self.decimation as isize;
        }
        self.offset = offset - count as isize;

        let delay = self.taps.len() - 1;
        self.buffer.copy_within(count..count + delay, 0);
        out_count
    }
}

// ===========================================================================
// Polyphase rational resampler
// ===========================================================================

/// Polyphase FIR resampler supporting arbitrary integer `interp/decim` ratios.
pub struct PolyphaseResampler<T: ResampleSample> {
    interp: usize,
    decim: usize,
    phases: Vec<Vec<f32>>,
    taps_per_phase: usize,
    buffer: Vec<T>,
    buf_start: usize,
    phase: usize,
    offset: isize,
}

impl<T: ResampleSample> PolyphaseResampler<T> {
    pub fn new(interp: usize, decim: usize, taps: &[f32]) -> Self {
        let interp = interp.max(1);
        let decim = decim.max(1);
        let taps_per_phase = (taps.len() + interp - 1) / interp;
        let mut phases = vec![vec![0.0f32; taps_per_phase]; interp];
        let total = interp * taps_per_phase;
        for i in 0..total {
            phases[(interp - 1) - (i % interp)][i / interp] =
                if i < taps.len() { taps[i] } else { 0.0 };
        }

        let delay = taps_per_phase.saturating_sub(1);
        Self {
            interp,
            decim,
            phases,
            taps_per_phase,
            buffer: vec![T::default(); delay + 1_064_000],
            buf_start: delay,
            phase: 0,
            offset: 0,
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0;
        self.offset = 0;
        self.buffer.fill(T::default());
    }

    #[inline]
    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        let count = input.len();
        self.buffer[self.buf_start..self.buf_start + count].copy_from_slice(input);

        let mut out_count = 0;
        let mut offset = self.offset;
        let mut phase = self.phase;

        while offset < count as isize {
            let idx = offset as usize;
            output[out_count] = T::dot(
                &self.buffer[idx..idx + self.taps_per_phase],
                &self.phases[phase],
            );
            out_count += 1;
            if out_count >= output.len() {
                break;
            }

            phase += self.decim;
            offset += (phase / self.interp) as isize;
            phase %= self.interp;
        }

        self.phase = phase;
        self.offset = offset - count as isize;

        let delay = self.taps_per_phase.saturating_sub(1);
        self.buffer.copy_within(count..count + delay, 0);

        out_count
    }
}

// ===========================================================================
// Rational resampler (combines optional power-of-two decim + polyphase)
// ===========================================================================

/// High-level rational resampler matching the SDR++ architecture:
/// a power-of-two pre-decimator (to ease the FIR length at large ratios)
/// followed by a polyphase interpolator/decimator for the residual fraction.
pub struct RationalResampler<T: ResampleSample> {
    in_rate: f64,
    out_rate: f64,
    mode: ResampMode,
    predec: Option<DecimatingFir<T>>,
    resamp: Option<PolyphaseResampler<T>>,
    scratch: Vec<T>,
}

#[derive(Clone, Copy, PartialEq)]
enum ResampMode {
    None,
    DecimOnly,
    ResampOnly,
    Both,
}

impl<T: ResampleSample> RationalResampler<T> {
    pub fn new(in_rate: f64, out_rate: f64) -> Self {
        let mut s = Self {
            in_rate,
            out_rate,
            mode: ResampMode::None,
            predec: None,
            resamp: None,
            scratch: vec![T::default(); 1_064_000],
        };
        s.reconfigure();
        s
    }

    pub fn set_rates(&mut self, in_rate: f64, out_rate: f64) {
        self.in_rate = in_rate;
        self.out_rate = out_rate;
        self.reconfigure();
    }

    #[inline]
    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        match self.mode {
            ResampMode::None => {
                let n = input.len().min(output.len());
                output[..n].copy_from_slice(&input[..n]);
                n
            }
            ResampMode::DecimOnly => self.predec.as_mut().unwrap().process(input, output),
            ResampMode::ResampOnly => {
                self.resamp.as_mut().unwrap().process(input, output)
            }
            ResampMode::Both => {
                let decim_out = self
                    .predec
                    .as_mut()
                    .unwrap()
                    .process(input, &mut self.scratch);
                self.resamp
                    .as_mut()
                    .unwrap()
                    .process(&self.scratch[..decim_out], output)
            }
        }
    }

    pub fn reset(&mut self) {
        if let Some(ref mut d) = self.predec {
            d.reset();
        }
        if let Some(ref mut r) = self.resamp {
            r.reset();
        }
    }

    fn reconfigure(&mut self) {
        let ratio = self.in_rate / self.out_rate;
        // Largest power-of-two pre-decimation (same cap as legacy: 8192 = 2^13)
        let max_power = 13usize;
        let predec_power = if ratio > 1.0 {
            (ratio.log2().floor() as usize).min(max_power)
        } else {
            0
        };
        let predec_ratio = 1usize << predec_power;
        let mut int_samplerate = self.in_rate;

        let use_decim = self.in_rate > self.out_rate && predec_power > 0;
        if use_decim {
            int_samplerate = self.in_rate / predec_ratio as f64;
            let cutoff = int_samplerate / 2.0;
            let trans = cutoff * 0.1;
            let taps = low_pass_taps(cutoff, trans, self.in_rate);
            self.predec = Some(DecimatingFir::new(taps, predec_ratio));
        } else {
            self.predec = None;
        }

        // Reduce remaining ratio to lowest terms
        let int_sr = int_samplerate.round() as i64;
        let out_sr = self.out_rate.round() as i64;
        let g = gcd(int_sr, out_sr);
        let interp = (out_sr / g) as usize;
        let decim = (int_sr / g) as usize;

        // Sanity-check drift
        let actual_out = int_samplerate * interp as f64 / decim as f64;
        let error = ((actual_out - self.out_rate) / self.out_rate).abs() * 100.0;
        if error > 0.01 {
            eprintln!("Warning: resampling error is over 0.01%: {}", error);
        }

        if interp == decim {
            self.mode = if use_decim {
                ResampMode::DecimOnly
            } else {
                ResampMode::None
            };
            self.resamp = None;
            return;
        }

        // Design polyphase filter
        let tap_sample_rate = int_samplerate * interp as f64;
        let tap_bw = self.in_rate.min(self.out_rate) / 2.0;
        let tap_trans = tap_bw * 0.1;
        let mut taps = low_pass_taps(tap_bw, tap_trans, tap_sample_rate);
        for t in &mut taps {
            *t *= interp as f32;
        }
        self.resamp = Some(PolyphaseResampler::new(interp, decim, &taps));
        self.mode = if use_decim {
            ResampMode::Both
        } else {
            ResampMode::ResampOnly
        };
    }
}

#[inline]
fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

// ===========================================================================
// Noise Blanker
// ===========================================================================

/// Simple exponential moving-average noise blanker operating on complex
/// baseband.  Samples whose instantaneous amplitude exceeds the running
/// average by `level` are attenuated inversely proportional to the excess.
pub struct NoiseBlanker {
    rate: f32,
    inv_rate: f32,
    level: f32,
    amp: f32,
}

impl NoiseBlanker {
    pub fn new(rate: f32, level: f32) -> Self {
        Self {
            rate,
            inv_rate: 1.0 - rate,
            level,
            amp: 1.0,
        }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
        self.inv_rate = 1.0 - rate;
    }

    pub fn set_level(&mut self, level: f32) {
        self.level = level;
    }

    pub fn reset(&mut self) {
        self.amp = 1.0;
    }

    #[inline]
    pub fn process(&mut self, samples: &mut [Complex<f32>]) {
        for s in samples.iter_mut() {
            let in_amp = s.norm();
            let mut gain = 1.0f32;
            if in_amp != 0.0 {
                self.amp = self.amp * self.inv_rate + in_amp * self.rate;
                let excess = in_amp / self.amp;
                if excess > self.level {
                    gain = 1.0 / excess;
                }
            }
            *s = *s * gain;
        }
    }
}
