//! Linear-phase RF selection without a long direct convolution per input sample.
//! The protected passband is +/- bandwidth/2; the stopband begins at
//! +/- 0.625 * bandwidth. Fixed overlap-save windows preserve arbitrary chunks.

use std::{collections::VecDeque, sync::Arc};

use num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

const MAX_TAPS: usize = 32_769;

pub struct RfSelector {
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    spectrum: Vec<Complex32>,
    window: Vec<Complex32>,
    scratch: Vec<Complex32>,
    overlap: Vec<Complex32>,
    pending: Vec<Complex32>,
    ready: VecDeque<Complex32>,
    hop: usize,
}

impl RfSelector {
    /// Check geometry and storage before allocating a channel's filter.
    pub fn validate(sample_rate_hz: f64, bandwidth_hz: u32) -> Result<usize, String> {
        if !sample_rate_hz.is_finite()
            || !(100.0..=20_000_000.0).contains(&sample_rate_hz)
            || bandwidth_hz == 0
            || f64::from(bandwidth_hz) > 0.8 * sample_rate_hz
        {
            return Err(
                "RF passband must be positive and fit within 80% of the channel clock".into(),
            );
        }
        let taps = (80.0 * sample_rate_hz / f64::from(bandwidth_hz)).ceil() as usize | 1;
        if taps > MAX_TAPS {
            return Err(
                "RF selection exceeds the filter storage limit; reduce the channel sample rate"
                    .into(),
            );
        }
        Ok(taps)
    }

    pub fn new(sample_rate_hz: f64, bandwidth_hz: u32) -> Result<Self, String> {
        let taps = Self::validate(sample_rate_hz, bandwidth_hz)?;
        let fft_len = (2 * taps).next_power_of_two();
        let midpoint = (taps - 1) as f64 / 2.0;
        let cutoff = 1.125 * f64::from(bandwidth_hz) / sample_rate_hz;
        let mut kernel: Vec<f64> = (0..taps)
            .map(|n| {
                let x = n as f64 - midpoint;
                let sinc = if x.abs() < 1e-12 {
                    cutoff
                } else {
                    (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                let angle = std::f64::consts::TAU * n as f64 / (taps - 1) as f64;
                let window = 0.35875 - 0.48829 * angle.cos() + 0.14128 * (2.0 * angle).cos()
                    - 0.01168 * (3.0 * angle).cos();
                sinc * window
            })
            .collect();
        let gain: f64 = kernel.iter().sum();
        kernel.iter_mut().for_each(|tap| *tap /= gain);
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(fft_len);
        let inverse = planner.plan_fft_inverse(fft_len);
        let mut spectrum = vec![Complex32::new(0.0, 0.0); fft_len];
        for (bin, tap) in spectrum.iter_mut().zip(kernel) {
            bin.re = tap as f32;
        }
        forward.process(&mut spectrum);
        let hop = fft_len - (taps - 1);
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        Ok(Self {
            forward,
            inverse,
            spectrum,
            window: vec![Complex32::new(0.0, 0.0); fft_len],
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
            overlap: vec![Complex32::new(0.0, 0.0); taps - 1],
            pending: Vec::with_capacity(hop),
            ready: VecDeque::with_capacity(hop),
            hop,
        })
    }

    /// Retire convolution and window histories without rebuilding the response.
    pub fn reset(&mut self) {
        self.overlap.fill(Complex32::new(0.0, 0.0));
        self.pending.clear();
        self.ready.clear();
    }

    /// Constant causal latency: FIR group delay plus overlap-save scheduling.
    pub fn latency_samples(&self) -> usize {
        self.hop - 1 + self.overlap.len() / 2
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut output = Vec::with_capacity(input.len());
        for sample in input {
            let clean = |x: f32| if x.is_finite() { x } else { 0.0 };
            self.pending
                .push(Complex32::new(clean(sample.re), clean(sample.im)));
            if self.pending.len() == self.hop {
                let overlap = self.overlap.len();
                self.window[..overlap].copy_from_slice(&self.overlap);
                self.window[overlap..].copy_from_slice(&self.pending);
                let tail = self.window.len() - overlap;
                self.overlap.copy_from_slice(&self.window[tail..]);
                self.forward
                    .process_with_scratch(&mut self.window, &mut self.scratch);
                for (bin, kernel) in self.window.iter_mut().zip(&self.spectrum) {
                    *bin *= kernel;
                }
                self.inverse
                    .process_with_scratch(&mut self.window, &mut self.scratch);
                let gain = 1.0 / self.window.len() as f32;
                self.ready
                    .extend(self.window[overlap..].iter().map(|x| x * gain));
                self.pending.clear();
            }
            output.push(self.ready.pop_front().unwrap_or(Complex32::new(0.0, 0.0)));
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: f64, n: usize, hz: f64, amplitude: f64) -> Vec<Complex32> {
        (0..n)
            .map(|index| {
                let value = num_complex::Complex64::from_polar(
                    amplitude,
                    std::f64::consts::TAU * hz * index as f64 / rate,
                );
                Complex32::new(value.re as f32, value.im as f32)
            })
            .collect()
    }

    fn amplitude(samples: &[Complex32], rate: f64, hz: f64) -> f64 {
        let projection: num_complex::Complex64 = samples
            .iter()
            .enumerate()
            .map(|(n, x)| {
                num_complex::Complex64::new(f64::from(x.re), f64::from(x.im))
                    * num_complex::Complex64::from_polar(
                        1.0,
                        -std::f64::consts::TAU * hz * n as f64 / rate,
                    )
            })
            .sum();
        projection.norm() / samples.len() as f64
    }

    #[test]
    fn rf_selection_production_clocks_protect_passband_and_reject_stronger_neighbors() {
        let mut checks = 0;
        let mut worst_gain = 0.0_f64;
        let mut worst_mixture = 0.0_f64;
        let mut min_rejection = f64::INFINITY;
        for rate in [
            48_000.0,
            96_000.0,
            192_000.0,
            200_000.0,
            2_048_003.0 / 10.0,
            288_000.0,
        ] {
            for bandwidth in [12_500_u32, 15_000, 25_000] {
                let count = (rate * 0.15) as usize;
                for sign in [-1.0, 1.0] {
                    for position in [0.0, 0.25, 0.5] {
                        let wanted_hz = sign * position * f64::from(bandwidth);
                        let rejected_hz = -sign * 0.625 * f64::from(bandwidth);
                        let wanted = tone(rate, count, wanted_hz, 0.02);
                        let neighbor = tone(rate, count, rejected_hz, 0.5);
                        let mixture: Vec<_> =
                            wanted.iter().zip(&neighbor).map(|(a, b)| a + b).collect();
                        let measure = |input: &[Complex32], hz| {
                            let filtered = RfSelector::new(rate, bandwidth).unwrap().process(input);
                            amplitude(
                                &filtered[filtered.len() - (rate * 0.05) as usize..],
                                rate,
                                hz,
                            )
                        };
                        let reference = measure(&wanted, wanted_hz);
                        let rejection = -20.0 * (measure(&neighbor, rejected_hz) / 0.5).log10();
                        let gain = (20.0 * (reference / 0.02).log10()).abs();
                        let mixed =
                            (20.0 * (measure(&mixture, wanted_hz) / reference).log10()).abs();
                        worst_gain = worst_gain.max(gain);
                        worst_mixture = worst_mixture.max(mixed);
                        min_rejection = min_rejection.min(rejection);
                        assert!(gain < 0.1, "{rate}/{bandwidth}/{wanted_hz}: gain={gain}");
                        assert!(
                            rejection > 80.0,
                            "{rate}/{bandwidth}/{rejected_hz}: rejection={rejection}"
                        );
                        assert!(
                            mixed < 0.1,
                            "{rate}/{bandwidth}/{wanted_hz}: mixture={mixed}"
                        );
                        checks += 1;
                    }
                }
            }
        }
        eprintln!("rf_selection cases={checks} max_gain_error_db={worst_gain:.8} min_rejection_db={min_rejection:.6} max_27_96db_stronger_neighbor_mixture_error_db={worst_mixture:.8}");
    }

    #[test]
    fn rf_selection_chunks_resets_and_nonfinite_inputs_preserve_bounded_history() {
        let rate = 2_048_003.0 / 10.0;
        let input = tone(rate, 12_007, 3_000.0, 0.3);
        let expected = RfSelector::new(rate, 15_000).unwrap().process(&input);
        let mut split = RfSelector::new(rate, 15_000).unwrap();
        let mut output = Vec::new();
        let mut offset = 0;
        for count in [1_usize, 37, 997].into_iter().cycle() {
            if offset == input.len() {
                break;
            }
            let end = (offset + count).min(input.len());
            output.extend(split.process(&input[offset..end]));
            offset = end;
        }
        assert_eq!(output, expected);
        split.reset();
        assert_eq!(split.process(&input), expected);
        split.reset();
        assert!(split
            .process(&vec![Complex32::new(f32::NAN, f32::INFINITY); 12_007])
            .iter()
            .all(|x| *x == Complex32::new(0.0, 0.0)));
        assert!(split.latency_samples() < 4096);
        for rate in [f64::NAN, f64::INFINITY, 0.0, 20_000_001.0] {
            assert!(RfSelector::new(rate, 15_000).is_err());
        }
        assert!(RfSelector::new(48_000.0, 0).is_err());
        assert!(RfSelector::new(48_000.0, 40_000).is_err());
        assert!(RfSelector::new(20_000_000.0, 1_000).is_err());
    }
}
