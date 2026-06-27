//! FFT helpers and window functions.
//!
//! Translated from `legacy_src/core/src/dsp/window/*.h` and the FFT logic in
//! `legacy_src/core/src/signal_path/iq_frontend.cpp`.

use num_complex::Complex;
use rustfft::{Fft, FftDirection, FftNum, FftPlanner};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Generalised cosine window (from `dsp/window/cosine.h`)
// ---------------------------------------------------------------------------

/// Compute a single sample of a generalised cosine window.
///
/// Formula (matching the C++ original):
/// `sum((-1)^i * coefs[i] * cos(i * 2 * PI * n / n_total))`
fn cosine_window(n: usize, n_total: usize, coefs: &[f64]) -> f64 {
    assert!(!coefs.is_empty());
    let mut win = 0.0;
    let mut sign = 1.0;
    for (i, &coef) in coefs.iter().enumerate() {
        win += sign
            * coef
            * ((i as f64) * 2.0 * std::f64::consts::PI * (n as f64) / (n_total as f64)).cos();
        sign = -sign;
    }
    win
}

// ---------------------------------------------------------------------------
// Window types (from `dsp/window/*.h`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    Rectangular,
    Hann,
    Hamming,
    Blackman,
    BlackmanHarris,
    BlackmanNuttall,
    Nuttall,
}

impl WindowType {
    /// Generate a window vector of length `len`.
    ///
    /// The C++ code evaluated the cosine series with denominator `N` (not
    /// `N-1`), yielding a periodic / DFT-even window.
    pub fn generate(self, len: usize) -> Vec<f32> {
        match self {
            WindowType::Rectangular => vec![1.0f32; len],
            WindowType::Hann => (0..len)
                .map(|i| cosine_window(i, len, &[0.5, 0.5]) as f32)
                .collect(),
            WindowType::Hamming => (0..len)
                .map(|i| cosine_window(i, len, &[0.54, 0.46]) as f32)
                .collect(),
            WindowType::Blackman => (0..len)
                .map(|i| cosine_window(i, len, &[0.42, 0.5, 0.08]) as f32)
                .collect(),
            WindowType::BlackmanHarris => (0..len)
                .map(|i| cosine_window(i, len, &[0.35875, 0.48829, 0.14128, 0.01168]) as f32)
                .collect(),
            WindowType::BlackmanNuttall => (0..len)
                .map(|i| {
                    cosine_window(i, len, &[0.3635819, 0.4891775, 0.1365995, 0.0106411]) as f32
                })
                .collect(),
            WindowType::Nuttall => (0..len)
                .map(|i| cosine_window(i, len, &[0.355768, 0.487396, 0.144232, 0.012604]) as f32)
                .collect(),
        }
    }

    /// Generate a window with an implicit FFT-shift (`(-1)^n`).
    ///
    /// The legacy `iq_frontend.cpp` multiplied the window by `(-1)^i` in
    /// `updateFFTPath` so that the spectrum ends up centred without an explicit
    /// `fftshift` after the transform.
    pub fn generate_shifted(self, len: usize) -> Vec<f32> {
        let mut win = self.generate(len);
        for (i, v) in win.iter_mut().enumerate() {
            if i % 2 != 0 {
                *v = -*v;
            }
        }
        win
    }
}

// ---------------------------------------------------------------------------
// FFTW-like planner wrapper
// ---------------------------------------------------------------------------

/// Thin wrapper around `rustfft::FftPlanner` with an FFTW-style naming convention.
pub struct FftPlannerWrapper<T: FftNum> {
    inner: FftPlanner<T>,
}

impl<T: FftNum> Default for FftPlannerWrapper<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: FftNum> FftPlannerWrapper<T> {
    pub fn new() -> Self {
        Self {
            inner: FftPlanner::new(),
        }
    }

    /// Plan a forward complex-to-complex FFT.
    pub fn plan_fft_forward(&mut self, len: usize) -> Arc<dyn Fft<T>> {
        self.inner.plan_fft(len, FftDirection::Forward)
    }

    /// Plan an inverse complex-to-complex FFT.
    pub fn plan_fft_inverse(&mut self, len: usize) -> Arc<dyn Fft<T>> {
        self.inner.plan_fft(len, FftDirection::Inverse)
    }
}

// ---------------------------------------------------------------------------
// FftBlock – owned buffer + window + plan
// ---------------------------------------------------------------------------

/// Owned FFT processing block (float precision, matching the original `fftwf`
/// usage in `iq_frontend.cpp`).
pub struct FftBlock {
    fft_size: usize,
    plan: Arc<dyn Fft<f32>>,
    /// Input / scratch buffer – holds the windowed time-domain samples and,
    /// after `process`, the frequency-domain result (in-place).
    buf: Vec<Complex<f32>>,
    window: Vec<f32>,
    shifted: bool,
}

impl FftBlock {
    /// Create a new block and pre-plan a forward FFT of the requested size.
    pub fn new(fft_size: usize) -> Self {
        let mut planner = FftPlannerWrapper::new();
        let plan = planner.plan_fft_forward(fft_size);
        Self {
            fft_size,
            plan,
            buf: vec![Complex::default(); fft_size],
            window: vec![1.0; fft_size],
            shifted: false,
        }
    }

    /// Number of FFT bins.
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// Current window coefficients.
    pub fn window(&self) -> &[f32] {
        &self.window
    }

    /// Whether the current window includes the `(-1)^n` shift.
    pub fn is_shifted(&self) -> bool {
        self.shifted
    }

    /// Replace the internal window (and optional shift).
    pub fn set_window(&mut self, win_type: WindowType, shifted: bool) {
        self.window = if shifted {
            win_type.generate_shifted(self.fft_size)
        } else {
            win_type.generate(self.fft_size)
        };
        self.shifted = shifted;
    }

    /// Apply the stored window to `input`, zero-pad to `fft_size`, execute the
    /// forward FFT in-place, and return a view of the complex output.
    ///
    /// Mirrors the logic in `IQFrontEnd::handler` (window → zero-pad → FFT).
    pub fn process_windowed(&mut self, input: &[Complex<f32>]) -> &[Complex<f32>] {
        let n = input.len().min(self.fft_size);

        for i in 0..n {
            self.buf[i] = input[i].scale(self.window[i]);
        }
        for i in n..self.fft_size {
            self.buf[i] = Complex::default();
        }

        self.plan.process(&mut self.buf);
        &self.buf
    }

    /// Convert the *last* FFT result to a power spectrum in dB.
    ///
    /// `scale` is the normalisation factor.  The legacy code called
    /// `volk_32fc_s32f_power_spectrum_32f(..., fftSize, fftSize)`, which
    /// computes `10 * log10(|z|^2 / scale)` with a small epsilon against `-inf`.
    ///
    /// # Panics
    /// Panics if `spectrum.len() != fft_size`.
    pub fn power_spectrum_db(&self, spectrum: &mut [f32], scale: f32) {
        assert_eq!(spectrum.len(), self.fft_size);
        for (i, out) in spectrum.iter_mut().enumerate() {
            let z = self.buf[i];
            let mag_sq = z.re * z.re + z.im * z.im;
            *out = 10.0 * ((mag_sq / scale).max(1e-20)).log10();
        }
    }

    /// Convenience: windowed FFT **and** power spectrum in one call.
    pub fn process_power_spectrum(
        &mut self,
        input: &[Complex<f32>],
        spectrum: &mut [f32],
        scale: f32,
    ) {
        self.process_windowed(input);
        self.power_spectrum_db(spectrum, scale);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_values() {
        let w = WindowType::Hann.generate(8);
        assert!((w[0] - 0.0).abs() < 1e-6);
        assert!((w[4] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hamming_midpoint() {
        let w = WindowType::Hamming.generate(64);
        assert!((w[0] - 0.08).abs() < 1e-6); // a0 - a1
        assert!((w[32] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn blackman_values() {
        let w = WindowType::Blackman.generate(8);
        assert!((w[0] - 0.0).abs() < 1e-6);
        assert!((w[4] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fft_block_roundtrip() {
        let mut block = FftBlock::new(8);
        block.set_window(WindowType::Rectangular, false);

        // DC impulse
        let mut input: Vec<Complex<f32>> = vec![Complex::default(); 8];
        input[0] = Complex::new(1.0, 0.0);

        let out = block.process_windowed(&input);
        // All bins should be ~1.0 (DC impulse → flat spectrum, magnitude 1.0)
        for &z in out.iter() {
            assert!((z.re - 1.0).abs() < 1e-4, "unexpected real part: {}", z.re);
            assert!(z.im.abs() < 1e-4, "unexpected imag part: {}", z.im);
        }
    }

    #[test]
    fn power_spectrum_finite() {
        let mut block = FftBlock::new(8);
        let input: Vec<Complex<f32>> = (0..8)
            .map(|i| Complex::new((i as f32).sin(), 0.0))
            .collect();
        let mut spec = vec![0.0f32; 8];
        block.process_power_spectrum(&input, &mut spec, 8.0);
        assert!(spec.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn shifted_window_sign_alternation() {
        let w = WindowType::Rectangular.generate_shifted(8);
        assert_eq!(w[0], 1.0);
        assert_eq!(w[1], -1.0);
        assert_eq!(w[2], 1.0);
        assert_eq!(w[3], -1.0);
    }
}
