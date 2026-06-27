//! Resampling blocks - translated from src-core/dsp/resampling/
//!
//! Provides rational resampling (polyphase FIR), integer interpolation,
//! and integer decimation with FIR filtering.

use num_complex::Complex32;
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Sample trait
// ---------------------------------------------------------------------------

/// Types that can be fed through FIR-based resamplers.
pub trait Sample:
    Copy
    + Default
    + std::ops::Add<Output = Self>
    + std::ops::Mul<f32, Output = Self>
    + Send
    + Sync
    + 'static
{
}

impl Sample for f32 {}
impl Sample for Complex32 {}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

// ---------------------------------------------------------------------------
// Modified Bessel function I0 (Kaiser window)
// ---------------------------------------------------------------------------

fn i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut u = 1.0;
    let mut n = 1;
    let halfx = x / 2.0;
    const EPSILON: f64 = 1e-21;
    loop {
        let temp = halfx / n as f64;
        n += 1;
        u *= temp * temp;
        sum += u;
        if u < EPSILON * sum {
            break;
        }
    }
    sum
}

/// Kaiser window of `ntaps` coefficients with shape parameter `beta`.
pub fn kaiser_window(ntaps: usize, beta: f64) -> Vec<f32> {
    assert!(beta >= 0.0, "kaiser_window: beta must be >= 0");
    if ntaps == 0 {
        return Vec::new();
    }
    if ntaps == 1 {
        return vec![1.0];
    }
    let mut w = vec![0.0f32; ntaps];
    let ibeta = 1.0 / i0(beta);
    let inm1 = 1.0 / ((ntaps - 1) as f64);
    w[0] = ibeta as f32;
    for i in 1..ntaps - 1 {
        let temp = 2.0 * i as f64 * inm1 - 1.0;
        w[i] = (i0(beta * (1.0 - temp * temp).sqrt()) * ibeta) as f32;
    }
    w[ntaps - 1] = ibeta as f32;
    w
}

// ---------------------------------------------------------------------------
// FIR design helpers
// ---------------------------------------------------------------------------

/// Window-method low-pass FIR (Kaiser window only).
///
/// `gain`      – overall DC gain (typically interpolation factor).  
/// `fs`        – sampling frequency (Hz).  
/// `cutoff`    – centre of transition band (Hz).  
/// `trans`     – transition-band width (Hz).  
/// `beta`      – Kaiser beta.
pub fn low_pass_fir(gain: f32, fs: f32, cutoff: f32, trans: f32, beta: f64) -> Vec<f32> {
    assert!(fs > 0.0);
    assert!(trans > 0.0);
    let attenuation = beta / 0.1102 + 8.7;
    let mut ntaps = (attenuation as f32 * fs / (22.0 * trans)).ceil() as usize;
    if ntaps % 2 == 0 {
        ntaps += 1; // make odd
    }
    let window = kaiser_window(ntaps, beta);

    let m = (ntaps - 1) / 2;
    let fwt0 = 2.0 * PI * cutoff / fs;
    let mut taps = vec![0.0f32; ntaps];

    for n in -(m as i32)..=(m as i32) {
        let idx = (n + m as i32) as usize;
        let val = if n == 0 {
            fwt0 / PI
        } else {
            (n as f32 * fwt0).sin() / (n as f32 * PI)
        };
        taps[idx] = val * window[idx];
    }

    // Normalise so that gain @ DC == `gain`
    let mut fmax = taps[m];
    for n in 1..=m {
        fmax += 2.0 * taps[m + n];
    }
    let norm = gain / fmax;
    for t in &mut taps {
        *t *= norm;
    }
    taps
}

/// Design a resampler anti-aliasing filter.
///
/// Mirrors `dsp::firdes::design_resampler_filter_float`.
pub fn design_resampler_filter(interpolation: usize, decimation: usize, fractional_bw: f32) -> Vec<f32> {
    assert!(interpolation > 0);
    assert!(decimation > 0);
    let halfband = 0.5f32;
    let rate = interpolation as f32 / decimation as f32;
    let (trans_width, mid_transition_band) = if rate >= 1.0 {
        let tw = halfband - fractional_bw;
        (tw, halfband - tw / 2.0)
    } else {
        let tw = rate * (halfband - fractional_bw);
        (tw, rate * halfband - tw / 2.0)
    };
    low_pass_fir(
        interpolation as f32,   // gain
        interpolation as f32,   // fs
        mid_transition_band,
        trans_width,
        7.0,                    // Kaiser beta
    )
}

// ---------------------------------------------------------------------------
// Polyphase helpers
// ---------------------------------------------------------------------------

/// Decompose `taps` into `nphase` polyphase sub-filters.
/// Preserves the reversed-phase convention used by the original C++
/// `PolyphaseBank` so that the resampler schedule stays identical.
fn build_polyphase(taps: &[f32], nphase: usize) -> (Vec<Vec<f32>>, usize) {
    let ntaps_raw = taps.len();
    let mut ntaps = (ntaps_raw + nphase - 1) / nphase;
    if ntaps_raw % nphase > 0 {
        ntaps += 1;
    }
    let mut sub = vec![vec![0.0f32; ntaps]; nphase];
    for i in 0..(nphase * ntaps) {
        let phase = (nphase - 1) - (i % nphase);
        let tap_idx = i / nphase;
        if i < ntaps_raw {
            sub[phase][tap_idx] = taps[i];
        }
    }
    (sub, ntaps)
}

// ---------------------------------------------------------------------------
// RationalResampler
// ---------------------------------------------------------------------------

/// Rational resampler: combines interpolation-by-L, FIR filtering,
/// and decimation-by-M in a single polyphase pass.
pub struct RationalResampler<T: Sample> {
    interpolation: usize,
    decimation: usize,
    phase: usize,
    input_pos: usize,
    buffer: Vec<T>,
    subfilters: Vec<Vec<f32>>,
    ntaps: usize,
}

impl<T: Sample> RationalResampler<T> {
    /// Create a new resampler.
    ///
    /// If `taps` is `None` an appropriate Kaiser-window LPF is generated
    /// automatically (fractional BW = 0.4).
    pub fn new(interpolation: usize, decimation: usize, taps: Option<Vec<f32>>) -> Self {
        assert!(interpolation > 0, "interpolation must be > 0");
        assert!(decimation > 0, "decimation must be > 0");

        let g = gcd(interpolation, decimation);
        let interpolation = interpolation / g;
        let decimation = decimation / g;

        let taps = taps.unwrap_or_else(|| design_resampler_filter(interpolation, decimation, 0.4));
        let (subfilters, ntaps) = build_polyphase(&taps, interpolation);

        Self {
            interpolation,
            decimation,
            phase: 0,
            input_pos: 0,
            buffer: Vec::with_capacity(8192),
            subfilters,
            ntaps,
        }
    }

    /// Resample `input` into `output`.  Returns the number of samples written.
    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        if input.is_empty() {
            return 0;
        }

        // Guard against unbounded buffering when the filter is longer than
        // any chunk we ever see.
        const MAX_BUF: usize = 1 << 20;
        if self.buffer.len() + input.len() > MAX_BUF {
            eprintln!("RationalResampler: internal buffer would exceed safety limit");
            return 0;
        }

        self.buffer.extend_from_slice(input);

        let mut produced = 0;
        while self.input_pos + self.ntaps < self.buffer.len() && produced < output.len() {
            let branch = &self.subfilters[self.phase];
            let mut acc = T::default();
            for (k, &tap) in branch.iter().enumerate() {
                acc = acc + self.buffer[self.input_pos + k] * tap;
            }
            output[produced] = acc;
            produced += 1;

            self.phase += self.decimation;
            self.input_pos += self.phase / self.interpolation;
            self.phase %= self.interpolation;
        }

        // Discard consumed input
        self.buffer.drain(0..self.input_pos);
        self.input_pos = 0;

        produced
    }

    /// Reset internal state (buffers, phase, position).
    pub fn reset(&mut self) {
        self.phase = 0;
        self.input_pos = 0;
        self.buffer.clear();
    }

    pub fn interpolation(&self) -> usize {
        self.interpolation
    }

    pub fn decimation(&self) -> usize {
        self.decimation
    }
}

// ---------------------------------------------------------------------------
// InterpolationFilter
// ---------------------------------------------------------------------------

/// Integer interpolator: inserts `factor`-1 zeros between samples and
/// filters with a polyphase FIR, producing `factor` outputs per input.
pub struct InterpolationFilter<T: Sample> {
    factor: usize,
    buffer: Vec<T>,
    subfilters: Vec<Vec<f32>>,
    ntaps: usize,
}

impl<T: Sample> InterpolationFilter<T> {
    pub fn new(factor: usize, taps: Vec<f32>) -> Self {
        assert!(factor > 0, "interpolation factor must be > 0");
        let (subfilters, ntaps) = build_polyphase(&taps, factor);
        Self {
            factor,
            buffer: Vec::with_capacity(8192),
            subfilters,
            ntaps,
        }
    }

    /// Produce interpolated samples.  Returns number of output samples written.
    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        if input.is_empty() {
            return 0;
        }
        self.buffer.extend_from_slice(input);

        let mut produced = 0;
        let max_input = self.buffer.len().saturating_sub(self.ntaps - 1);
        let out_per_in = self.factor;

        for i in 0..max_input {
            if produced + out_per_in > output.len() {
                break;
            }
            for p in 0..self.factor {
                let branch = &self.subfilters[p];
                let mut acc = T::default();
                for (k, &tap) in branch.iter().enumerate() {
                    acc = acc + self.buffer[i + k] * tap;
                }
                output[produced + p] = acc;
            }
            produced += out_per_in;
        }

        self.buffer.drain(0..max_input);
        produced
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}

// ---------------------------------------------------------------------------
// DecimationFilter
// ---------------------------------------------------------------------------

/// Integer decimator: FIR filters then keeps every `decimation`-th sample.
pub struct DecimationFilter<T: Sample> {
    taps: Vec<f32>,
    ntaps: usize,
    buffer: Vec<T>,
    decimation: usize,
    decim_pos: usize,
}

impl<T: Sample> DecimationFilter<T> {
    pub fn new(decimation: usize, taps: Vec<f32>) -> Self {
        let ntaps = taps.len();
        Self {
            taps,
            ntaps,
            buffer: Vec::with_capacity(8192),
            decimation: decimation.max(1),
            decim_pos: 1,
        }
    }

    /// Decimate `input` into `output`.  Returns number of output samples written.
    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        if input.is_empty() {
            return 0;
        }
        self.buffer.extend_from_slice(input);

        let max_input = self.buffer.len().saturating_sub(self.ntaps - 1);
        let mut produced = 0;

        for i in 0..max_input {
            if self.decim_pos >= self.decimation {
                if produced >= output.len() {
                    break;
                }
                let mut acc = T::default();
                for (k, &tap) in self.taps.iter().enumerate() {
                    acc = acc + self.buffer[i + k] * tap;
                }
                output[produced] = acc;
                produced += 1;
                self.decim_pos = 1;
            } else {
                self.decim_pos += 1;
            }
        }

        self.buffer.drain(0..max_input);
        produced
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.decim_pos = 1;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rational_resampler_float_identity() {
        // 1:1 resampler with a simple delta-like tap should pass data through
        let taps = vec![1.0f32];
        let mut r = RationalResampler::<f32>::new(1, 1, Some(taps));
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let mut output = vec![0.0f32; input.len()];
        let n = r.process(&input, &mut output);
        // One sample is consumed by the 1-tap FIR, so 99 outputs
        assert_eq!(n, 99);
        assert_eq!(&output[..n], &input[..n]);
    }

    #[test]
    fn test_interpolation_filter_basic() {
        // Interpolate by 2 with [1,0]: reversed-phase convention puts tap 1.0
        // in phase 1 and 0.0 in phase 0, so output pairs are (0, x[i]).
        let taps = vec![1.0f32, 0.0];
        let mut interp = InterpolationFilter::<f32>::new(2, taps);
        let input = vec![1.0f32, 2.0, 3.0];
        let mut output = vec![0.0f32; 10];
        let n = interp.process(&input, &mut output);
        assert!(n >= 4);
        assert_eq!(output[0], 0.0);
        assert_eq!(output[1], 1.0);
    }

    #[test]
    fn test_decimation_filter_basic() {
        // Decimate by 2 with [1] taps: decim_pos starts at 1, so first sample
        // is skipped, then every 2nd sample is kept.
        let taps = vec![1.0f32];
        let mut dec = DecimationFilter::<f32>::new(2, taps);
        let input = vec![1.0f32, 2.0, 3.0, 4.0, 5.0];
        let mut output = vec![0.0f32; 10];
        let n = dec.process(&input, &mut output);
        assert_eq!(n, 2);
        assert_eq!(output[0], 2.0);
        assert_eq!(output[1], 4.0);
    }

    #[test]
    fn test_kaiser_window_basic() {
        let w = kaiser_window(11, 7.0);
        assert_eq!(w.len(), 11);
        // Symmetric
        assert!((w[0] - w[10]).abs() < 1e-6);
        // Peak in the middle
        assert!(w[5] > w[0]);
    }

    #[test]
    fn test_design_resampler_filter_non_empty() {
        let taps = design_resampler_filter(3, 2, 0.4);
        assert!(!taps.is_empty());
    }
}
