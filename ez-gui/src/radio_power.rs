//! Selected-channel carrier peak amplitude, independent of display FFT settings.

use crate::demod::DsbSideband;
use crate::radio_iq::VfoMixer;
use crate::sdr_panel::DemodMode;
use ez_daemon::channelizer::FirDecimator;
use num_complex::{Complex32, Complex64};
use std::sync::Arc;

/// Nominal RF selection relative to the VFO. SSB bandwidth is one-sided.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RfPassband {
    pub lower_hz: f64,
    pub upper_hz: f64,
}

impl RfPassband {
    pub fn for_mode(mode: DemodMode, width: f32, sideband: DsbSideband) -> Self {
        let mode = match (mode, sideband) {
            (DemodMode::Dsb, DsbSideband::Upper) => DemodMode::Usb,
            (DemodMode::Dsb, DsbSideband::Lower) => DemodMode::Lsb,
            _ => mode,
        };
        let width = f64::from(if width.is_finite() && width > 0.0 {
            width
        } else {
            mode.default_rf_bandwidth_hz()
        });
        let (lower_hz, upper_hz) = match mode {
            DemodMode::Usb => (0.0, width),
            DemodMode::Lsb => (-width, 0.0),
            _ => (-width / 2.0, width / 2.0),
        };
        Self { lower_hz, upper_hz }
    }

    pub fn width(self) -> f64 {
        self.upper_hz - self.lower_hz
    }

    pub fn center(self) -> f64 {
        (self.upper_hz + self.lower_hz) / 2.0
    }

    pub fn shifted(self, offset: f64) -> (f64, f64) {
        (self.lower_hz + offset, self.upper_hz + offset)
    }
}

/// Strongest estimated complex sinusoid amplitude in the selected RF band,
/// in dBFS (unit complex carrier = 0 dBFS). This is not integrated RF power.
/// Complete windows only; gaps/retunes retire both pending and published data.
pub(crate) struct PowerMeter {
    fft: Arc<dyn rustfft::Fft<f32>>,
    samples: Vec<Complex32>,
    scratch: Vec<Complex32>,
    window: Vec<f32>,
    normalization: f64,
    peak_model: Vec<(f64, f64)>,
    mixer: VfoMixer,
    decimator: FirDecimator,
    settings: Option<(f64, f64, RfPassband)>,
    rate: f64,
    level: f32,
    completed_windows: u64,
    observation_revision: u64,
}

impl PowerMeter {
    pub fn new() -> Self {
        let fft = rustfft::FftPlanner::new().plan_fft_forward(4096);
        Self {
            scratch: vec![Complex32::default(); fft.get_inplace_scratch_len()],
            fft,
            samples: Vec::new(),
            window: Vec::new(),
            normalization: 1.0,
            peak_model: Vec::new(),
            mixer: VfoMixer::new(1.0),
            decimator: FirDecimator::new(1),
            settings: None,
            rate: 1.0,
            level: -120.0,
            completed_windows: 0,
            observation_revision: 0,
        }
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.mixer.reset();
        self.decimator.reset(0);
        self.level = -120.0;
        self.completed_windows = 0;
        self.observation_revision = self.observation_revision.wrapping_add(1);
    }

    pub fn observation_revision(&self) -> u64 {
        self.observation_revision
    }

    pub fn has_measurement(&self) -> bool {
        self.completed_windows > 0
    }

    fn configure(&mut self, rate: f64, offset: f64, band: RfPassband) -> bool {
        let (lower, upper) = band.shifted(offset);
        if !rate.is_finite()
            || rate <= 0.0
            || !offset.is_finite()
            || !band.width().is_finite()
            || band.width() <= 0.0
            || lower < -rate / 2.0
            || upper > rate / 2.0
        {
            self.reset();
            self.settings = None;
            return false;
        }
        if self.settings == Some((rate, offset, band)) {
            return true;
        }
        // Coarse downconversion bounds FFT storage even at MHz capture rates.
        // All nominal passband edges lie inside the FIR's flat passband.
        let working_rate = 250_000.0_f64.max(4.0 * band.width());
        let factor = (rate / working_rate).floor().max(1.0) as usize;
        self.rate = rate / factor as f64;
        self.mixer.configure(rate, offset);
        self.decimator = FirDecimator::new(factor);
        // About 32 bins across normal channels; a grid no coarser than
        // 10 Hz at the narrow CW end. Complete narrow observations take up
        // to 200 ms. Peak estimation accounts for fractional bin placement.
        let resolution = (band.width() / 32.0).max(10.0);
        let minimum_size = ((self.rate * 0.1).ceil() as usize)
            .next_power_of_two()
            .min(4096);
        let size = ((self.rate / resolution).ceil() as usize)
            .next_power_of_two()
            .max(minimum_size);
        if self.window.len() != size {
            self.fft = rustfft::FftPlanner::new().plan_fft_forward(size);
            self.scratch = vec![Complex32::default(); self.fft.get_inplace_scratch_len()];
            self.window = (0..size)
                .map(|n| {
                    let x = std::f64::consts::TAU * n as f64 / size as f64;
                    (0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos()
                        - 0.01168 * (3.0 * x).cos()) as f32
                })
                .collect();
            self.normalization = 1.0 / self.window.iter().map(|&x| f64::from(x)).sum::<f64>();
            // Fit adjacent-bin magnitude ratios to the exact finite window
            // transform. Correcting its scalloping also calibrates amplitude
            // without a bin-center assumption or a parabolic frequency bias.
            self.peak_model = (0..=512)
                .map(|n| {
                    let delta = n as f64 / 1024.0;
                    let ratio = (window_response(size, 1.0 - delta)
                        / window_response(size, 1.0 + delta))
                    .ln();
                    (ratio, 1.0 / window_response(size, delta))
                })
                .collect();
        }
        self.samples = Vec::with_capacity(size);
        self.settings = Some((rate, offset, band));
        self.reset();
        true
    }

    pub fn process(
        &mut self,
        samples: &[Complex32],
        rate: f64,
        offset: f64,
        band: RfPassband,
    ) -> f32 {
        if !self.configure(rate, offset, band) {
            return -120.0;
        }
        let mut mixed = self.mixer.process(samples);
        for sample in &mut mixed {
            if !sample.re.is_finite() || !sample.im.is_finite() {
                *sample = Complex32::default();
            }
        }
        let selected = self.decimator.process(&mixed);
        let size = self.window.len();
        for sample in selected {
            self.samples.push(sample);
            if self.samples.len() == size {
                self.measure(band);
                self.samples.clear();
                self.completed_windows += 1;
                self.observation_revision = self.observation_revision.wrapping_add(1);
            }
        }
        self.level
    }

    fn measure(&mut self, band: RfPassband) {
        for (sample, &window) in self.samples.iter_mut().zip(&self.window) {
            *sample *= window;
        }
        self.fft
            .process_with_scratch(&mut self.samples, &mut self.scratch);
        let size = self.samples.len();
        let magnitude = |n: usize| {
            let x = self.samples[n];
            f64::from(x.re).hypot(f64::from(x.im))
        };
        let mut amplitude = 0.0_f64;
        for index in 0..size {
            let center = magnitude(index);
            let left = magnitude((index + size - 1) % size);
            let right = magnitude((index + 1) % size);
            if center == 0.0 || center < left || center < right {
                continue;
            }
            let ratio = (right.max(f64::MIN_POSITIVE) / left.max(f64::MIN_POSITIVE)).ln();
            let n = self
                .peak_model
                .partition_point(|&(x, _)| x < ratio.abs())
                .min(512);
            let (delta, correction) = if n == 0 {
                (0.0, self.peak_model[0].1)
            } else {
                let (a, ca) = self.peak_model[n - 1];
                let (b, cb) = self.peak_model[n];
                let blend = ((ratio.abs() - a) / (b - a)).clamp(0.0, 1.0);
                (
                    (n as f64 - 1.0 + blend) / 1024.0 * ratio.signum(),
                    ca + (cb - ca) * blend,
                )
            };
            let bin = if index < size / 2 {
                index as f64
            } else {
                index as f64 - size as f64
            };
            let hz = (bin + delta) * self.rate / size as f64;
            if hz >= band.lower_hz - 1e-6 && hz <= band.upper_hz + 1e-6 {
                amplitude = amplitude.max(center * self.normalization * correction);
            }
        }
        self.level = (20.0 * amplitude.max(1e-6).log10()).max(-120.0) as f32;
    }
}

/// Fourier transform of the periodic four-term Blackman-Harris window,
/// normalized at DC. A sum of four shifted finite Dirichlet kernels.
fn window_response(size: usize, bins: f64) -> f64 {
    let dirichlet = |x: f64| {
        if x.abs() < 1e-12 {
            Complex64::new(size as f64, 0.0)
        } else {
            let gain =
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x / size as f64).sin();
            Complex64::from_polar(
                gain,
                std::f64::consts::PI * x * (size - 1) as f64 / size as f64,
            )
        }
    };
    let mut value = dirichlet(bins) * 0.35875;
    for (k, coefficient) in [(1.0, -0.48829), (2.0, 0.14128), (3.0, -0.01168)] {
        value += (dirichlet(bins - k) + dirichlet(bins + k)) * (coefficient / 2.0);
    }
    value.norm() / (size as f64 * 0.35875)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tones(rate: f64, count: usize, components: &[(f64, f64)]) -> Vec<Complex32> {
        (0..count)
            .map(|n| {
                let mut value = Complex64::default();
                for &(hz, amplitude) in components {
                    value += Complex64::from_polar(
                        amplitude,
                        std::f64::consts::TAU * hz * n as f64 / rate,
                    );
                }
                Complex32::new(value.re as f32, value.im as f32)
            })
            .collect()
    }

    #[test]
    fn sideband_geometry_matches_demodulator_including_dsb_selection_and_defaults() {
        for width in [0.0, 500.0, 2800.0, 3000.0, 12000.0] {
            for mode in [
                DemodMode::Usb,
                DemodMode::Lsb,
                DemodMode::Dsb,
                DemodMode::Cw,
            ] {
                for side in [DsbSideband::Both, DsbSideband::Upper, DsbSideband::Lower] {
                    let mut demod = crate::demod::Demodulator::new();
                    demod.set_rf_bandwidth(width);
                    demod.set_dsb_sideband(side);
                    let actual = demod.rf_passband(mode);
                    let resolved = match (mode, side) {
                        (DemodMode::Dsb, DsbSideband::Upper) => DemodMode::Usb,
                        (DemodMode::Dsb, DsbSideband::Lower) => DemodMode::Lsb,
                        _ => mode,
                    };
                    let w = f64::from(if width == 0.0 {
                        resolved.default_rf_bandwidth_hz()
                    } else {
                        width
                    });
                    let expected = match resolved {
                        DemodMode::Usb => (0.0, w),
                        DemodMode::Lsb => (-w, 0.0),
                        _ => (-w / 2.0, w / 2.0),
                    };
                    assert_eq!((actual.lower_hz, actual.upper_hz), expected);
                }
            }
        }
    }

    #[test]
    fn ssb_sweep_preserves_wanted_peaks_rejects_opposite_and_keeps_mixed_weak_tones() {
        let mut meter = PowerMeter::new();
        let mut cases = 0;
        let mut maximum_error = 0.0_f32;
        let mut maximum_opposite = -120.0_f32;
        for rate in [192003.0, 300000.0, 2400000.0] {
            for width in [500.0_f32, 3000.0, 12000.0] {
                for offset in [0.0, rate * 0.17, -rate * 0.19] {
                    for mode in [DemodMode::Usb, DemodMode::Lsb] {
                        let band = RfPassband::for_mode(mode, width, DsbSideband::Both);
                        assert!(meter.configure(rate, offset, band));
                        let factor = (rate / meter.rate).round() as usize;
                        let count = 2 * meter.window.len() * factor;
                        let sign = if mode == DemodMode::Usb { 1.0 } else { -1.0 };
                        for fraction in [0.1, 0.25, 0.5, 0.75, 0.9] {
                            let hz = sign * f64::from(width) * fraction;
                            for components in [
                                vec![(offset + hz, 0.05)],
                                vec![(offset + hz, 0.05), (offset - hz, 0.7)],
                            ] {
                                meter.reset();
                                let actual = meter.process(
                                    &tones(rate, count, &components),
                                    rate,
                                    offset,
                                    band,
                                );
                                let error = (actual - 20.0_f32 * 0.05_f32.log10()).abs();
                                maximum_error = maximum_error.max(error);
                                assert!(error<=0.15,"SSB gain rate={rate} width={width} offset={offset} mode={mode:?} f={fraction} level={actual} components={components:?}");
                                cases += 1;
                            }
                            meter.reset();
                            let opposite = meter.process(
                                &tones(rate, count, &[(offset - hz, 0.7)]),
                                rate,
                                offset,
                                band,
                            );
                            maximum_opposite = maximum_opposite.max(opposite);
                            assert!(opposite < -80.0,"opposite entered {mode:?} rate={rate} width={width} offset={offset} f={fraction}: {opposite}");
                            cases += 1;
                        }
                    }
                }
            }
        }
        println!("power_ssb cases={cases} max_gain_error_db={maximum_error} maximum_opposite_dbfs={maximum_opposite}");
    }

    #[test]
    fn every_integer_cw_width_and_fractional_off_bin_carrier_keeps_calibrated_amplitude() {
        let rate = 300000.0;
        let offset = rate / 4096.0 / 2.0;
        let mut meter = PowerMeter::new();
        let mut maximum_error = 0.0_f32;
        for width in 50..=500 {
            let band = RfPassband::for_mode(DemodMode::Cw, width as f32, DsbSideband::Both);
            assert!(meter.configure(rate, offset, band));
            let relative =
                (if width % 2 == 0 { 1.0 } else { -1.0 }) * (width as f64 * 0.37 + 0.173);
            let samples = tones(rate, 2 * meter.window.len(), &[(offset + relative, 0.1)]);
            meter.reset();
            let actual = meter.process(&samples, rate, offset, band);
            maximum_error = maximum_error.max((actual + 20.0).abs());
            assert!(
                (actual + 20.0).abs() <= 0.15,
                "CW width={width} relative={relative} measured={actual}"
            );
        }
        println!("power_cw integer_widths=451 max_gain_error_db={maximum_error}");
    }

    #[test]
    fn cw_edge_peaks_fractional_clocks_low_and_high_capture_rates_have_no_bin_blind_spots() {
        let mut maximum_error = 0.0_f32;
        let mut cases = 0;
        let mut meter = PowerMeter::new();
        for rate in [8000.0, 192003.0, 2400000.0, 20000000.0] {
            for width in [50.0_f32, 100.0, 199.5, 200.0, 313.5, 500.0] {
                let offset = rate * 0.217 + 0.137;
                let band = RfPassband::for_mode(DemodMode::Cw, width, DsbSideband::Both);
                assert!(meter.configure(rate, offset, band));
                let factor = (rate / meter.rate).round() as usize;
                assert!(meter.window.len() <= 65536);
                assert!(meter.window.len() as f64 / meter.rate <= 0.2);
                for fraction in [-0.48, 0.0, 0.48] {
                    meter.reset();
                    let actual = meter.process(
                        &tones(
                            rate,
                            2 * meter.window.len() * factor,
                            &[(offset + f64::from(width) * fraction, 0.1)],
                        ),
                        rate,
                        offset,
                        band,
                    );
                    maximum_error = maximum_error.max((actual + 20.0).abs());
                    assert!(
                        (actual + 20.0).abs() <= 0.15,
                        "rate={rate} width={width} fraction={fraction} measured={actual}"
                    );
                    cases += 1;
                }
            }
        }
        println!("power_cw_edges cases={cases} max_gain_error_db={maximum_error}");
    }

    #[test]
    fn full_stream_counts_chunks_and_resets_do_not_depend_on_caller_partitions() {
        let rate = 2400000.0;
        let offset = 100001.375;
        let band = RfPassband::for_mode(DemodMode::Usb, 3000.0, DsbSideband::Both);
        let input = tones(
            rate,
            240000,
            &[(offset + 1111.137, 0.05), (offset - 1333.173, 0.7)],
        );
        let mut whole = PowerMeter::new();
        let expected = whole.process(&input, rate, offset, band);
        for chunk in [1, 37, 997, 4093] {
            let mut split = PowerMeter::new();
            for part in input.chunks(chunk) {
                split.process(part, rate, offset, band);
            }
            assert_eq!(split.level, expected, "chunk={chunk}");
            assert_eq!(split.completed_windows, whole.completed_windows);
            assert_eq!(split.samples, whole.samples);
            split.reset();
            assert_eq!(split.level, -120.0);
            assert_eq!(split.completed_windows, 0);
            assert_eq!(split.process(&input, rate, offset, band), expected);
            assert_eq!(
                split.process(&input[..10], rate, offset + 10000.0, band),
                -120.0
            );
            assert_eq!(split.completed_windows, 0);
            assert_eq!(
                split.process(&input[..10], rate / 2.0, offset, band),
                -120.0
            );
        }
    }

    #[test]
    fn all_windows_in_a_large_block_are_consumed_and_invalid_geometry_retires_readings() {
        let rate = 300000.0;
        let band = RfPassband::for_mode(DemodMode::Usb, 3000.0, DsbSideband::Both);
        let mut meter = PowerMeter::new();
        assert!(meter.configure(rate, 0.0, band));
        let n = meter.window.len();
        let mut input = tones(rate, 2 * n, &[(1000.137, 0.1)]);
        input.extend(tones(rate, 2 * n, &[(1000.137, 0.02)]));
        let value = meter.process(&input, rate, 0.0, band);
        assert_eq!(meter.completed_windows, 4);
        assert!((value - 20.0_f32 * 0.02_f32.log10()).abs() < 0.15);
        assert_eq!(meter.process(&[], rate, 0.0, band), value);
        assert_eq!(meter.process(&input, rate, rate / 2.0, band), -120.0);
        assert_eq!(meter.process(&input, f64::NAN, 0.0, band), -120.0);
        assert_eq!(meter.process(&input, rate, f64::INFINITY, band), -120.0);
    }
}
