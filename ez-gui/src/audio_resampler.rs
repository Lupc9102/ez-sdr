//! Streaming, band-limited playback conversion for mono daemon audio.
//! Exact rational phase and FIR history survive network frame boundaries.
//! The passband extends through 0.4 * min(input Fs, output Fs), with a
//! transition to 0.5 * min(Fs) and at least 80 dB measured alias rejection.

pub(crate) use ez_daemon::audio_rate::RateFilter;

fn valid_rates(source_rate: u32, stride: u32, output_rate: u32) -> bool {
    stride != 0
        && u64::from(source_rate) >= 8_000 * u64::from(stride)
        && u64::from(source_rate) <= 384_000 * u64::from(stride)
        && (8_000..=384_000).contains(&output_rate)
}

#[derive(Default)]
pub(crate) struct AudioResampler {
    rates: (u32, u32, u32),
    filter: Option<RateFilter>,
    // Next output position relative to the current input sample, in units
    // of 1 / (output Fs * source stride). This remains bounded and never
    // accumulates floating-point clock error, including fractional input Fs.
    next: u64,
    expected: Option<(u32, u64, ez_proto::CaptureClock)>,
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
        self.process_ratio(samples, input_rate, 1, output_rate, gain)
    }

    fn process_ratio(
        &mut self,
        samples: &[f32],
        source_rate: u32,
        stride: u32,
        output_rate: u32,
        gain: f32,
    ) -> Vec<f32> {
        // Reject corrupt wire geometry before it can amplify allocation.
        if !valid_rates(source_rate, stride, output_rate) {
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
        let denominator = u64::from(output_rate) * u64::from(stride);
        if u64::from(source_rate) == denominator {
            self.reset();
            return samples.iter().map(|sample| clean(*sample) * gain).collect();
        }
        if self.rates != (source_rate, stride, output_rate) {
            self.reset();
            self.rates = (source_rate, stride, output_rate);
            self.filter = Some(RateFilter::new(f64::from(source_rate) / denominator as f64));
        }
        let filter = self.filter.as_mut().unwrap();
        let capacity = (samples.len() as u128 * u128::from(denominator))
            .div_ceil(u128::from(source_rate)) as usize;
        let mut output = Vec::with_capacity(capacity);
        for &sample in samples {
            filter.push(clean(sample));
            while self.next < denominator {
                let sample = filter.output(self.next as f64 / denominator as f64);
                output.push((sample * gain).clamp(-1.0, 1.0));
                self.next += u64::from(source_rate);
            }
            self.next -= denominator;
        }
        output
    }

    pub fn process_frame(
        &mut self,
        frame: &ez_proto::AudioFrame,
        output_rate: u32,
        gain: f32,
    ) -> Vec<f32> {
        let continuous = self.expected.is_some_and(|(id, sequence, clock)| {
            id == frame.channel_id && sequence == frame.sequence && clock == frame.audio_clock
        });
        if !continuous {
            self.reset();
        }
        let clock = frame.audio_clock;
        if clock.sample_stride == 0
            || frame.sample_rate_hz != clock.source_sample_rate_hz / clock.sample_stride
            || !valid_rates(
                clock.source_sample_rate_hz,
                clock.sample_stride,
                output_rate,
            )
        {
            self.reset();
            return Vec::new();
        }
        let output = if clock.sample_stride == 1 {
            self.process(
                &frame.samples,
                clock.source_sample_rate_hz,
                output_rate,
                gain,
            )
        } else {
            self.process_ratio(
                &frame.samples,
                clock.source_sample_rate_hz,
                clock.sample_stride,
                output_rate,
                gain,
            )
        };
        self.expected = frame
            .audio_clock
            .offset_samples(frame.samples.len() as u64)
            .map(|mut clock| {
                clock.discontinuous = false;
                (frame.channel_id, frame.sequence.saturating_add(1), clock)
            });
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_playback_uses_generated_clock_while_retaining_fractional_rf_metadata() {
        let samples: Vec<_> = (0..1007)
            .map(|n| (0.2 * (std::f64::consts::TAU * 1000.0 * n as f64 / 48_000.0).sin()) as f32)
            .collect();
        let mut rf_clock = ez_proto::CaptureClock::new(19, 700, 2_048_003, None);
        rf_clock.sample_stride = 10;
        let pcm_clock = ez_proto::CaptureClock::new(3, 0, 48_000, None);
        let first = ez_proto::AudioFrame {
            clock: rf_clock,
            audio_clock: pcm_clock,
            sequence: 1,
            squelch: ez_proto::SquelchState {
                db: -120.0,
                revision: 0,
            },
            channel_id: 7,
            sample_rate_hz: 48_000,
            samples: samples[..333].to_vec(),
        };
        let second = ez_proto::AudioFrame {
            clock: rf_clock.offset_samples(1421).unwrap(),
            audio_clock: pcm_clock.offset_samples(333).unwrap(),
            sequence: 2,
            samples: samples[333..].to_vec(),
            ..first.clone()
        };
        let expected = AudioResampler::default().process(&samples, 48_000, 44_100, 1.0);
        let mut playback = AudioResampler::default();
        let mut actual = playback.process_frame(&first, 44_100, 1.0);
        actual.extend(playback.process_frame(&second, 44_100, 1.0));
        assert_eq!(actual, expected);
        assert_eq!(first.clock.source_sample_rate_hz, 2_048_003);
        assert_eq!(first.clock.sample_stride, 10);
        assert!(actual.iter().any(|x| x.abs() > 0.1));
    }

    #[test]
    fn mixed_stream_audio_clock_and_sequence_gaps_reset_interpolation() {
        let first = ez_proto::AudioFrame {
            audio_clock: ez_proto::CaptureClock::new(10, 0, 48_000, None),
            clock: ez_proto::CaptureClock::new(10, 0, 48_000, None),
            sequence: 1,
            channel_id: 2,
            sample_rate_hz: 48_000,
            samples: vec![0.8; 64],
            squelch: ez_proto::SquelchState {
                db: -120.0,
                revision: 0,
            },
        };
        for boundary in 0..6 {
            let mut resampler = AudioResampler::default();
            resampler.process_frame(&first, 44_100, 1.0);
            let mut second = first.clone();
            second.audio_clock.source_sample_index = 64;
            second.sequence = 2;
            second.samples.fill(-0.8);
            match boundary {
                0 => second.sequence += 1,
                1 => second.audio_clock.source_sample_index += 1,
                2 => second.audio_clock.epoch += 1,
                3 => second.audio_clock.discontinuous = true,
                4 => second.audio_clock.origin_unix_ns = Some(123),
                _ => second.channel_id += 1,
            }
            let expected = AudioResampler::default().process_frame(&second, 44_100, 1.0);
            assert_eq!(
                resampler.process_frame(&second, 44_100, 1.0),
                expected,
                "boundary {boundary} joined interpolation across a gap"
            );
        }
        let mut resampler = AudioResampler::default();
        let mut actual = resampler.process_frame(&first, 44_100, 1.0);
        let mut second = first.clone();
        second.audio_clock.source_sample_index = 64;
        second.sequence = 2;
        second.samples.fill(-0.8);
        actual.extend(resampler.process_frame(&second, 44_100, 1.0));
        let mut joined = first.samples.clone();
        joined.extend_from_slice(&second.samples);
        assert_eq!(
            actual,
            AudioResampler::default().process(&joined, 48_000, 44_100, 1.0)
        );
    }

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
        // Exclude the causal FIR's startup before measuring pitch. Finite
        // window pre-ringing near zero can add crossings during acquisition.
        let settled = &actual[128..actual.len() - 128];
        let crossings = settled
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!(
            (crossings as f64 - settled.len() as f64 * 1000.0 / 44_100.0).abs() <= 1.0,
            "tone pitch changed: {crossings}"
        );
    }

    #[test]
    fn gain_applies_at_equal_and_different_rates() {
        for rate in [44_100, 48_000, 96_000] {
            let mut resampler = AudioResampler::default();
            assert!(resampler
                .process(&[0.8; 512], 48_000, rate, 0.0)
                .iter()
                .all(|value| *value == 0.0));
            assert!(resampler
                .process(&[0.8; 64], 48_000, rate, 0.5)
                .iter()
                .all(|value| (*value - 0.4).abs() < 1e-6));
        }
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
    }

    fn tone(count: usize, hz: f64, rate: f64) -> Vec<f32> {
        (0..count)
            .map(|n| (0.5 * (std::f64::consts::TAU * hz * n as f64 / rate).sin()) as f32)
            .collect()
    }

    fn assert_tone_clock(
        output: &[f32],
        input_rate: f64,
        output_rate: u32,
        hz: f64,
        filter_delay: f64,
        settling: usize,
    ) {
        let error = output
            .iter()
            .enumerate()
            .skip(settling)
            .map(|(n, &actual)| {
                let angle = std::f64::consts::TAU
                    * hz
                    * (n as f64 / f64::from(output_rate) - filter_delay / input_rate);
                (f64::from(actual) - 0.5 * angle.sin()).abs()
            })
            .fold(0.0_f64, f64::max);
        assert!(
            error < 0.0001,
            "input={input_rate} output={output_rate} tone={hz} phase/amplitude error={error}"
        );
    }

    #[test]
    fn passband_phase_and_stopband_rejection_hold_at_production_playback_rates() {
        let mut measured = 0;
        let mut minimum_rejection = f64::INFINITY;
        for input_rate in [48_000, 96_000, 192_000, 200_000, 204_800, 240_000, 384_000] {
            for output_rate in [44_100, 48_000] {
                let count = input_rate as usize / 50;
                let ratio = f64::from(input_rate) / f64::from(output_rate);
                let taps = (100.0 * ratio.max(1.0)).ceil() as usize | 1;
                let delay = if input_rate == output_rate {
                    0.0
                } else {
                    (taps - 1) as f64 / 2.0
                };
                for hz in [1000.0, 15000.0, 0.4 * f64::from(output_rate)] {
                    let input = tone(count, hz, f64::from(input_rate));
                    let actual =
                        AudioResampler::default().process(&input, input_rate, output_rate, 1.0);
                    assert_eq!(
                        actual.len(),
                        (count as u64 * u64::from(output_rate)).div_ceil(u64::from(input_rate))
                            as usize
                    );
                    assert_tone_clock(&actual, f64::from(input_rate), output_rate, hz, delay, 128);
                    measured += 1;
                }
                for hz in [
                    0.5 * f64::from(output_rate),
                    0.51 * f64::from(output_rate),
                    26000.0,
                    38000.0,
                    49000.0,
                    50000.0,
                    57000.0,
                ] {
                    if hz >= 0.5 * f64::from(input_rate) {
                        continue;
                    }
                    let input = tone(count, hz, f64::from(input_rate));
                    let actual =
                        AudioResampler::default().process(&input, input_rate, output_rate, 1.0);
                    let db =
                        20.0 * (rms(&actual[128..]) / (0.5 / std::f64::consts::SQRT_2)).log10();
                    assert!(
                        db <= -80.0,
                        "input={input_rate} output={output_rate} hz={hz} rejection={db}"
                    );
                    minimum_rejection = minimum_rejection.min(-db);
                    measured += 1;
                }
            }
        }
        println!("playback frequency sweep measurements={measured} minimum_rejection_db={minimum_rejection:.6}");
    }

    #[test]
    fn upsampling_and_extreme_ratios_keep_exact_counts_and_filter_delay() {
        for (input_rate, output_rate) in [
            (8000, 48000),
            (44100, 192000),
            (8000, 384000),
            (384000, 8000),
        ] {
            let count = input_rate as usize / 20;
            let hz = 0.4 * f64::from(input_rate.min(output_rate));
            let input = tone(count, hz, f64::from(input_rate));
            let mut resampler = AudioResampler::default();
            let actual: Vec<_> = input
                .chunks(37)
                .flat_map(|part| resampler.process(part, input_rate, output_rate, 1.0))
                .collect();
            assert_eq!(
                actual.len(),
                (count as u64 * u64::from(output_rate)).div_ceil(u64::from(input_rate)) as usize
            );
            let filter = resampler.filter.as_ref().unwrap();
            let (taps, phases) = filter.storage_shape();
            assert!(taps <= 4801 && phases <= 257);
            let settling = (filter.storage_shape().0 as f64 * f64::from(output_rate)
                / f64::from(input_rate))
            .ceil() as usize
                + 1;
            assert_tone_clock(
                &actual,
                f64::from(input_rate),
                output_rate,
                hz,
                (filter.storage_shape().0 - 1) as f64 / 2.0,
                settling,
            );
        }
    }

    #[test]
    fn fractional_capture_rate_controls_playback_phase_and_arbitrary_frame_splits() {
        let source_rate = 2_048_003;
        let stride = 10;
        let rate = f64::from(source_rate) / f64::from(stride);
        let samples = tone(32_000, 17_000.0, rate);
        let mut reference = AudioResampler::default();
        let whole = reference.process_ratio(&samples, source_rate, stride, 48_000, 1.0);
        let mut split = AudioResampler::default();
        let mut actual = Vec::new();
        for (sequence, part) in samples.chunks(137).enumerate() {
            let mut clock = ez_proto::CaptureClock::new(
                7,
                sequence as u64 * 137 * u64::from(stride),
                source_rate,
                None,
            );
            clock.sample_stride = stride;
            let frame = ez_proto::AudioFrame {
                audio_clock: clock,
                clock,
                sequence: sequence as u64 + 1,
                channel_id: 1,
                sample_rate_hz: source_rate / stride,
                samples: part.to_vec(),
                squelch: ez_proto::SquelchState {
                    db: -120.0,
                    revision: 0,
                },
            };
            actual.extend(split.process_frame(&frame, 48_000, 1.0));
        }
        assert_eq!(actual, whole);
        assert_eq!(
            actual.len(),
            (samples.len() as u64 * 48_000 * u64::from(stride)).div_ceil(u64::from(source_rate))
                as usize
        );
        let delay = (reference.filter.as_ref().unwrap().storage_shape().0 - 1) as f64 / 2.0;
        assert_tone_clock(&actual, rate, 48_000, 17_000.0, delay, 128);
    }

    fn daemon_playback(
        mode: ez_proto::DemodMode,
        rate: u32,
        output_rate: u32,
        components: &[(f64, f64)],
        chunk: usize,
    ) -> (Vec<f32>, Vec<f32>) {
        use ez_daemon::{
            bus::{OverflowPolicy, SampleBlock, SampleBus},
            pipelines::audio::{AudioConfig, AudioPipeline},
        };
        use num_complex::Complex32;
        use std::{sync::Arc, time::Duration};
        // AM's envelope DC-blocker has a 5 ms time constant at 200 kS/s.
        // Its legitimate startup is in-band and must not be mistaken for a
        // folded high-frequency carrier by a finite Fourier observation.
        let count = rate as usize
            / if mode == ez_proto::DemodMode::Am {
                5
            } else {
                50
            };
        let iq: Vec<_> = (0..count)
            .map(|n| {
                if mode == ez_proto::DemodMode::Am {
                    let envelope = 0.5
                        + components
                            .iter()
                            .map(|&(hz, amplitude)| {
                                amplitude
                                    * (std::f64::consts::TAU * hz * n as f64 / f64::from(rate))
                                        .sin()
                            })
                            .sum::<f64>();
                    Complex32::new(envelope as f32, 0.0)
                } else {
                    let phase = components
                        .iter()
                        .map(|&(hz, deviation)| {
                            -deviation / hz
                                * (std::f64::consts::TAU * hz * n as f64 / f64::from(rate)).cos()
                        })
                        .sum::<f64>();
                    Complex32::new((0.5 * phase.cos()) as f32, (0.5 * phase.sin()) as f32)
                }
            })
            .collect();
        let bus = SampleBus::new();
        // This fixture measures the subsequent audio-rate anti-alias stage.
        // Keep its 49 kHz AM/FM modulation inside the selected RF passband;
        // narrow-mode RF rejection is covered by the daemon selection tests.
        let parameters = ez_proto::AudioParameters {
            rf_bandwidth_hz: (0.8 * f64::from(rate)) as u32,
            output_sample_rate_hz: 0,
        };
        let mut pipeline = AudioPipeline::new(
            bus.subscribe(4, OverflowPolicy::DropIncoming),
            AudioConfig {
                audio: Some(parameters),
                channel_id: 1,
                mode,
            },
        );
        let frames = pipeline.subscribe(4);
        let mut resampler = AudioResampler::default();
        let mut pcm = Vec::new();
        let mut audio = Vec::new();
        for (sequence, part) in iq.chunks(chunk).enumerate() {
            let first = (sequence * chunk) as u64;
            bus.publish(SampleBlock {
                product_session: 42,
                observed_at: None,
                capture: Some(ez_proto::CaptureClock::new(3, first, rate, None)),
                start_sample: first,
                sample_rate_hz: rate,
                center_freq_hz: 100_000_000,
                samples: Arc::from(part.to_vec()),
            });
            assert!(pipeline.tick(Duration::ZERO));
            let frame = frames.try_recv().unwrap();
            pcm.extend_from_slice(&frame.samples);
            audio.extend(resampler.process_frame(&frame, output_rate, 1.0));
        }
        (pcm, audio)
    }

    #[test]
    fn real_daemon_wfm_mpx_and_strong_mixtures_do_not_alias_into_playback() {
        let mut cases = 0;
        let mut minimum_rejection = f64::INFINITY;
        let mut maximum_mixture_error = 0.0_f64;
        for rate in [192_000, 200_000, 204_800, 240_000] {
            for output_rate in [44_100, 48_000] {
                for hz in [26000.0, 38000.0, 50000.0, 57000.0] {
                    let (pcm, audio) = daemon_playback(
                        ez_proto::DemodMode::Wfm,
                        rate,
                        output_rate,
                        &[(hz, 12000.0)],
                        137,
                    );
                    assert!(rms(&pcm[1000..]) > 0.001, "no observed MPX input");
                    let db = 20.0 * (rms(&audio[256..]) / rms(&pcm[1000..])).log10();
                    assert!(
                        db < -80.0,
                        "WFM input={rate} output={output_rate} hz={hz} rejection={db}"
                    );
                    minimum_rejection = minimum_rejection.min(-db);
                    cases += 1;
                }
                let (_, reference) = daemon_playback(
                    ez_proto::DemodMode::Wfm,
                    rate,
                    output_rate,
                    &[(1000.0, 1500.0)],
                    rate as usize,
                );
                let components = [
                    (1000.0, 1500.0),
                    (26000.0, 5000.0),
                    (38000.0, 5000.0),
                    (50000.0, 5000.0),
                    (57000.0, 5000.0),
                ];
                let (_, mixture) = daemon_playback(
                    ez_proto::DemodMode::Wfm,
                    rate,
                    output_rate,
                    &components,
                    137,
                );
                let (_, whole) = daemon_playback(
                    ez_proto::DemodMode::Wfm,
                    rate,
                    output_rate,
                    &components,
                    rate as usize,
                );
                assert_eq!(mixture, whole, "input/frame partition altered playback");
                let differences: Vec<_> = mixture[256..]
                    .iter()
                    .zip(&reference[256..])
                    .map(|(a, b)| a - b)
                    .collect();
                let error = rms(&differences) / rms(&reference[256..]);
                assert!(error < 0.005, "WFM mixture relative error={error}");
                maximum_mixture_error = maximum_mixture_error.max(error);
            }
        }
        println!("real WFM stop cases={cases} minimum_rejection_db={minimum_rejection:.6} maximum_relative_mixture_error={maximum_mixture_error:.9}");
    }

    #[test]
    fn real_daemon_am_and_nfm_destination_rate_conversion_rejects_out_of_band_audio() {
        for output_rate in [44100, 48000] {
            for (mode, amplitude) in [
                (ez_proto::DemodMode::Am, 0.1),
                (ez_proto::DemodMode::Fm, 500.0),
            ] {
                let (pcm, audio) =
                    daemon_playback(mode, 200000, output_rate, &[(49000.0, amplitude)], 137);
                // Let AM's independent DC-blocker settle for 100 ms. The
                // converter's own passband/stopband checks exclude only its
                // finite FIR startup, and do not use this extra interval.
                let alias = (49000.0 - f64::from(output_rate)).abs();
                let am = mode == ez_proto::DemodMode::Am;
                let values = &audio[if am { output_rate as usize / 10 } else { 256 }..];
                let amplitude = |samples: &[f32], hz: f64, fs: f64| {
                    let re = samples
                        .iter()
                        .enumerate()
                        .map(|(n, x)| {
                            f64::from(*x) * (std::f64::consts::TAU * hz * n as f64 / fs).cos()
                        })
                        .sum::<f64>();
                    let im = samples
                        .iter()
                        .enumerate()
                        .map(|(n, x)| {
                            f64::from(*x) * (std::f64::consts::TAU * hz * n as f64 / fs).sin()
                        })
                        .sum::<f64>();
                    2.0 * re.hypot(im) / samples.len() as f64
                };
                let before = amplitude(&pcm[if am { 20000 } else { 1000 }..], 49000.0, 200000.0);
                let after = amplitude(values, alias, f64::from(output_rate));
                println!("mode={mode:?} output={output_rate} alias_hz={alias} before={before:.9} after={after:.9}");
                assert!(after / before < 0.0001, "{mode:?} alias not rejected");
            }
        }
    }

    #[test]
    fn empty_and_single_sample_partitions_preserve_rational_cadence_and_samples() {
        for (input_rate, output_rate) in [
            (200000, 44100),
            (200000, 48000),
            (48000, 44100),
            (8000, 384000),
            (384000, 8000),
            (204803, 48000),
        ] {
            let input: Vec<_> = (0..1337)
                .map(|n| (0.2 * (n as f64 * 0.13).cos()) as f32)
                .collect();
            let expected = AudioResampler::default().process(&input, input_rate, output_rate, 1.0);
            for chunk in [1, 37, 997] {
                let mut resampler = AudioResampler::default();
                let mut actual = Vec::new();
                for part in input.chunks(chunk) {
                    assert!(resampler
                        .process(&[], input_rate, output_rate, 1.0)
                        .is_empty());
                    actual.extend(resampler.process(part, input_rate, output_rate, 1.0));
                }
                assert_eq!(
                    actual, expected,
                    "input={input_rate} output={output_rate} chunk={chunk}"
                );
            }
        }
    }

    #[test]
    fn rate_changes_retire_history_including_equal_rate_passthrough() {
        for (old_input, old_output) in [(200000, 44100), (48000, 96000), (48000, 48000)] {
            let mut resampler = AudioResampler::default();
            resampler.process(&[0.8; 512], old_input, old_output, 1.0);
            let input = tone(512, 1000.0, 192000.0);
            let expected = AudioResampler::default().process(&input, 192000, 48000, 1.0);
            assert_eq!(resampler.process(&input, 192000, 48000, 1.0), expected);
            resampler.reset();
            assert_eq!(resampler.process(&input, 192000, 48000, 1.0), expected);
        }
    }

    #[test]
    fn corrupt_frame_geometry_retires_history_before_next_valid_audio() {
        let valid = ez_proto::AudioFrame {
            audio_clock: ez_proto::CaptureClock::new(7, 0, 200000, None),
            clock: ez_proto::CaptureClock::new(7, 0, 200000, None),
            sequence: 1,
            channel_id: 1,
            sample_rate_hz: 200000,
            samples: vec![0.8; 512],
            squelch: ez_proto::SquelchState {
                db: -120.0,
                revision: 0,
            },
        };
        for invalid in 0..4 {
            let mut resampler = AudioResampler::default();
            resampler.process_frame(&valid, 48000, 1.0);
            let mut bad = valid.clone();
            bad.audio_clock.source_sample_index = 512;
            bad.sequence = 2;
            match invalid {
                0 => bad.audio_clock.sample_stride = 0,
                1 => bad.audio_clock.source_sample_rate_hz = 0,
                2 => bad.sample_rate_hz += 1,
                _ => {
                    bad.sample_rate_hz = 400000;
                    bad.audio_clock.source_sample_rate_hz = 400000;
                }
            }
            assert!(resampler.process_frame(&bad, 48000, 1.0).is_empty());
            assert!(resampler.expected.is_none() && resampler.filter.is_none());
            assert_eq!(
                resampler.process_frame(&valid, 48000, 1.0),
                AudioResampler::default().process_frame(&valid, 48000, 1.0)
            );
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
