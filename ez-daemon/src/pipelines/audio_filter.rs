//! Complex RF selection with a response specified in Hz, independent of the
//! incoming channel clock. Double-precision sections preserve narrow filters
//! at high channel rates without a rate-dependent FIR transition width.

use num_complex::Complex32;

const ORDER: usize = 36;

#[derive(Clone, Copy, Default)]
struct Section {
    b: [f64; 3],
    a: [f64; 2],
    // Transposed direct-form II history, separately for I and Q.
    state: [[f64; 2]; 2],
}

#[derive(Default)]
pub(super) struct ComplexLowpass {
    sections: [Section; ORDER / 2],
    settings: Option<(f64, f64)>,
}

impl ComplexLowpass {
    pub fn reset(&mut self) {
        for section in &mut self.sections {
            section.state = [[0.0; 2]; 2];
        }
    }

    fn configure(&mut self, cutoff_hz: f64, sample_rate_hz: f64) -> bool {
        if !sample_rate_hz.is_finite()
            || sample_rate_hz <= 0.0
            || !cutoff_hz.is_finite()
            || cutoff_hz <= 0.0
        {
            self.reset();
            self.settings = None;
            return false;
        }
        let cutoff_hz = cutoff_hz.min(0.45 * sample_rate_hz);
        if self.settings == Some((cutoff_hz, sample_rate_hz)) {
            return true;
        }
        let omega = std::f64::consts::TAU * cutoff_hz / sample_rate_hz;
        let (sin, cos) = omega.sin_cos();
        for (index, section) in self.sections.iter_mut().enumerate() {
            let angle = std::f64::consts::PI * (2 * index + 1) as f64 / (2 * ORDER) as f64;
            let q = 1.0 / (2.0 * angle.cos());
            let alpha = sin / (2.0 * q);
            let a0 = 1.0 + alpha;
            section.b = [
                (1.0 - cos) / (2.0 * a0),
                (1.0 - cos) / a0,
                (1.0 - cos) / (2.0 * a0),
            ];
            section.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
            section.state = [[0.0; 2]; 2];
        }
        self.settings = Some((cutoff_hz, sample_rate_hz));
        true
    }

    pub fn process(&mut self, input: Complex32, cutoff_hz: f64, sample_rate_hz: f64) -> Complex32 {
        if !self.configure(cutoff_hz, sample_rate_hz) {
            return Complex32::new(0.0, 0.0);
        }
        let mut value =
            [input.re, input.im].map(|x| if x.is_finite() { f64::from(x) } else { 0.0 });
        for section in &mut self.sections {
            for (value, state) in value.iter_mut().zip(&mut section.state) {
                let output = section.b[0] * *value + state[0];
                state[0] = section.b[1] * *value - section.a[0] * output + state[1];
                state[1] = section.b[2] * *value - section.a[1] * output;
                *value = output;
            }
        }
        Complex32::new(value[0] as f32, value[1] as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssb_production_filter_reset_clock_changes_and_nonfinite_samples_retire_history() {
        for rate in [8000.0, 48_000.0, 200_000.0, 2_048_003.0 / 10.0, 2_400_000.0] {
            let mut used = ComplexLowpass::default();
            for _ in 0..4096 {
                assert!(used
                    .process(Complex32::new(0.5, -0.25), 1450.0, rate)
                    .norm()
                    .is_finite());
            }
            assert!(used
                .process(Complex32::new(f32::NAN, f32::INFINITY), 1450.0, rate)
                .norm()
                .is_finite());
            used.reset();
            let mut fresh = ComplexLowpass::default();
            for _ in 0..100 {
                let input = Complex32::new(0.1, 0.2);
                assert_eq!(
                    used.process(input, 1450.0, rate),
                    fresh.process(input, 1450.0, rate)
                );
            }
            let mut changed = ComplexLowpass::default();
            for _ in 0..100 {
                let input = Complex32::new(0.1, 0.2);
                assert_eq!(
                    used.process(input, 1450.0, rate + 1000.0),
                    changed.process(input, 1450.0, rate + 1000.0)
                );
            }
        }
    }
}
