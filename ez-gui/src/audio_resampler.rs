//! Streaming playback-rate conversion for mono daemon audio.
//! Retains interpolation phase across network frames, avoiding pitch changes
//! when the system audio device runs at a different rate from the daemon.
#[derive(Default)]
pub(crate) struct AudioResampler {
    rates: (u32, u32),
    previous: Option<f32>,
    next: f64,
}

impl AudioResampler {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn process(
        &mut self,
        samples: &[f32],
        input_rate: u32,
        output_rate: u32,
        gain: f32,
    ) -> Vec<f32> {
        // Reject corrupt wire rates before a hostile ratio can amplify allocation.
        if !(8_000..=384_000).contains(&input_rate) || !(8_000..=384_000).contains(&output_rate) {
            self.reset();
            return Vec::new();
        }
        let clean = |sample: f32| {
            if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if input_rate == output_rate {
            self.reset();
            return samples.iter().map(|sample| clean(*sample) * gain).collect();
        }
        if self.rates != (input_rate, output_rate) {
            self.reset();
            self.rates = (input_rate, output_rate);
        }
        let step = input_rate as f64 / output_rate as f64;
        let mut output = Vec::with_capacity((samples.len() as f64 / step).ceil() as usize + 1);
        for &sample in samples {
            let sample = clean(sample);
            if let Some(previous) = self.previous {
                while self.next <= 1.0 {
                    output.push((previous + (sample - previous) * self.next as f32) * gain);
                    self.next += step;
                }
                self.next -= 1.0;
            } else {
                output.push(sample * gain);
                self.next = step;
            }
            self.previous = Some(sample);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_conversion_is_independent_of_network_frame_boundaries() {
        let input: Vec<_> = (0..48_000)
            .map(|i| (std::f32::consts::TAU * 1_000.0 * i as f32 / 48_000.0).sin())
            .collect();
        let mut reference = AudioResampler::default();
        let expected = reference.process(&input, 48_000, 44_100, 0.5);
        assert!((expected.len() as isize - 44_100).abs() <= 1);
        let mut streaming = AudioResampler::default();
        let actual: Vec<_> = input
            .chunks(137)
            .flat_map(|chunk| streaming.process(chunk, 48_000, 44_100, 0.5))
            .collect();
        assert_eq!(actual, expected);
        let crossings = actual
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!(
            (crossings as isize - 1000).abs() <= 1,
            "tone pitch changed: {crossings}"
        );
    }

    #[test]
    fn gain_applies_at_equal_and_different_rates() {
        for rate in [44_100, 48_000, 96_000] {
            let mut resampler = AudioResampler::default();
            assert!(resampler
                .process(&[0.8; 64], 48_000, rate, 0.0)
                .iter()
                .all(|value| *value == 0.0));
            assert!(resampler
                .process(&[0.8; 64], 48_000, rate, 0.5)
                .iter()
                .all(|value| (*value - 0.4).abs() < 1e-6));
        }
    }

    #[test]
    fn rejects_invalid_rates_and_cleans_nonfinite_samples() {
        let mut resampler = AudioResampler::default();
        assert!(resampler.process(&[1.0], 0, 48_000, 1.0).is_empty());
        assert!(resampler.process(&[1.0], 1, 384_000, 1.0).is_empty());
        assert_eq!(
            resampler.process(&[f32::NAN, f32::INFINITY, 0.5], 48_000, 48_000, 1.0),
            [0.0, 0.0, 0.5]
        );
    }
}
