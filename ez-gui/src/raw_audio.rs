//! GUI adapter for the shared capture-clock audio converter.

pub(crate) use ez_daemon::audio_rate::IqAudioConverter as RawIqAudio;
#[cfg(test)]
use num_complex::Complex32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_audio_clocks_count_frames_exactly_and_preserve_every_partition() {
        let rates: [f64; 9] = [
            125.0,
            8_000.0,
            48_000.0,
            192_000.0,
            384_003.0 / 7.0,
            1_000_003.0,
            2_048_000.0,
            2_400_000.0,
            20_000_000.0,
        ];
        let mut cases = 0;
        for input_rate in rates {
            for output_rate in [8_000, 44_100, 48_000, 192_000, 384_000] {
                let n = (input_rate * 0.002).ceil().max(7.0) as usize;
                let input: Vec<_> = (0..n)
                    .map(|i| {
                        let phase = std::f64::consts::TAU * 17.0 * i as f64 / input_rate;
                        Complex32::new((phase.cos() * 0.2) as f32, (phase.sin() * 0.1) as f32)
                    })
                    .collect();
                let whole = RawIqAudio::default().process(&input, input_rate, output_rate, 0.3);
                let expected = (n as f64 * f64::from(output_rate) / input_rate).ceil() as usize;
                assert_eq!(whole.len(), expected, "{input_rate}/{output_rate}");
                let mut split = RawIqAudio::default();
                let mut actual = Vec::new();
                for chunk in input.chunks(37) {
                    actual.extend(split.process(chunk, input_rate, output_rate, 0.3));
                    assert!(split.process(&[], input_rate, output_rate, 0.3).is_empty());
                }
                assert_eq!(actual, whole, "{input_rate}/{output_rate}");
                cases += 1;
            }
        }
        eprintln!("raw_audio duration_and_partition_cases={cases} exact_frames=passed");
    }

    #[test]
    fn raw_audio_rf_stride_tails_follow_root_duration_in_tiny_partitions() {
        let mut cases = 0;
        for source_rate in [8_000_u32, 192_003, 2_400_000] {
            for factor in [2_u32, 8, 64] {
                for output_rate in [8_000_u32, 48_000, 384_000] {
                    let mut raw = RawIqAudio::default();
                    let mut root_total = 0_usize;
                    let mut iq_total = 0_usize;
                    let mut output_total = 0_usize;
                    for n in [1_usize, 7, 37, 997, 3] {
                        root_total += n;
                        let iq_count = root_total / factor as usize - iq_total;
                        iq_total += iq_count;
                        let output = raw.process_capture(
                            &vec![Complex32::new(0.1, -0.2); iq_count],
                            f64::from(source_rate) / f64::from(factor),
                            output_rate,
                            0.3,
                            n,
                            f64::from(source_rate),
                        );
                        output_total += output.len();
                        assert_eq!(
                            output_total,
                            (root_total as u128 * u128::from(output_rate))
                                .div_ceil(u128::from(source_rate))
                                as usize
                        );
                    }
                    cases += 1;
                }
            }
        }
        eprintln!(
            "raw_audio root_stride_partition_cases={cases} incomplete_stride_duration=passed"
        );
    }

    #[test]
    fn raw_audio_channels_preserve_independent_tones_and_reject_output_aliases() {
        let mut worst_gain_error = 0.0_f64;
        let mut worst_alias = 0.0_f64;
        for output_rate in [8_000_u32, 44_100, 48_000] {
            let input_rate = 192_000.0;
            let create = |alias: bool| {
                (0..19_200)
                    .map(|n| {
                        let t = n as f64 / input_rate;
                        let i_hz = if alias {
                            f64::from(output_rate) + 1_000.0
                        } else {
                            1_000.0
                        };
                        let q_hz = if alias {
                            f64::from(output_rate) + 2_000.0
                        } else {
                            2_000.0
                        };
                        Complex32::new(
                            (0.2 * (std::f64::consts::TAU * i_hz * t).cos()) as f32,
                            (0.1 * (std::f64::consts::TAU * q_hz * t).sin()) as f32,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let wanted =
                RawIqAudio::default().process(&create(false), input_rate, output_rate, 0.3);
            let alias = RawIqAudio::default().process(&create(true), input_rate, output_rate, 0.3);
            let amplitude = |frames: &[[f32; 2]], channel: usize, hz: f64| {
                let frames = &frames[frames.len() / 2..];
                let (mut c, mut s) = (0.0, 0.0);
                for (n, frame) in frames.iter().enumerate() {
                    let phase = std::f64::consts::TAU * hz * n as f64 / f64::from(output_rate);
                    c += f64::from(frame[channel]) * phase.cos();
                    s += f64::from(frame[channel]) * phase.sin();
                }
                2.0 * c.hypot(s) / frames.len() as f64
            };
            for (channel, hz, expected) in [(0, 1_000.0, 0.06), (1, 2_000.0, 0.03)] {
                let gain_error =
                    (20.0 * (amplitude(&wanted, channel, hz) / expected).log10()).abs();
                worst_gain_error = worst_gain_error.max(gain_error);
                assert!(
                    gain_error < 0.1,
                    "channel={channel} audioFs={output_rate} gain_error={gain_error}"
                );
                let rejected = amplitude(&alias, channel, hz) / expected;
                worst_alias = worst_alias.max(rejected);
                assert!(
                    rejected < 1e-4,
                    "channel={channel} audioFs={output_rate} alias={rejected}"
                );
                assert!(
                    amplitude(&wanted, 1 - channel, hz) < expected * 1e-4,
                    "I/Q crosstalk"
                );
            }
        }
        eprintln!("raw_audio independent_channel_cases=6 max_gain_error_db={worst_gain_error:.8} min_alias_rejection_db={:.4}", -20.0 * worst_alias.log10());
    }

    #[test]
    fn raw_audio_equal_clocks_preserve_iq_without_added_delay() {
        let input = [
            Complex32::new(0.1, -0.2),
            Complex32::new(0.0, 0.7),
            Complex32::new(f32::NAN, f32::INFINITY),
        ];
        let output = RawIqAudio::default().process(&input, 48_000.0, 48_000, 0.3);
        assert_eq!(
            output,
            vec![[0.1 * 0.3, -0.2 * 0.3], [0.0, 0.7 * 0.3], [0.0, 0.0]]
        );
    }

    #[test]
    fn raw_audio_reset_rate_changes_and_nonfinite_input_retire_history() {
        let input = vec![Complex32::new(0.3, -0.1); 997];
        let mut used = RawIqAudio::default();
        used.process(&input, 192_000.0, 48_000, 0.3);
        used.reset();
        assert_eq!(
            used.process(&input, 192_000.0, 48_000, 0.3),
            RawIqAudio::default().process(&input, 192_000.0, 48_000, 0.3)
        );
        assert_eq!(
            used.process(&input, 192_003.0, 44_100, 0.3),
            RawIqAudio::default().process(&input, 192_003.0, 44_100, 0.3)
        );
        let bad = vec![Complex32::new(f32::NAN, f32::INFINITY); 997];
        used.reset();
        assert!(used
            .process(&bad, 192_000.0, 48_000, 0.3)
            .iter()
            .all(|x| *x == [0.0; 2]));
        for invalid in [f64::NAN, f64::INFINITY, 0.0, 99.0, 20_000_001.0] {
            assert!(used.process(&input, invalid, 48_000, 0.3).is_empty());
        }
    }
}
