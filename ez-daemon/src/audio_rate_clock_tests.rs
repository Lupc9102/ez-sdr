use super::*;

#[test]
fn rational_capture_conversion_has_exact_coherent_counts_and_chunk_independent_pcm() {
    for (source_rate, stride, output_rate, count, expected_count) in [
        (384_003, 7, 48_000, 128_001, 112_000),
        (192_005, 3, 44_100, 38_401, 26_460),
        (2_048_025, 10, 48_000, 27_307, 6_400),
    ] {
        let input: Vec<_> = (0..count)
            .map(|n| {
                let phase = std::f64::consts::TAU * 1000.0 * n as f64 * f64::from(stride)
                    / f64::from(source_rate);
                Complex32::new((0.1 * phase.cos()) as f32, (0.1 * phase.sin()) as f32)
            })
            .collect();
        let mut whole = IqAudioConverter::for_detector();
        let expected = whole.process_clocked(&input, source_rate, stride, output_rate, 1.0);
        assert_eq!(
            expected.len(),
            expected_count,
            "coherent boundary {source_rate}/{stride}->{output_rate}"
        );
        let mut split = IqAudioConverter::for_detector();
        let mut actual = Vec::new();
        let mut offset = 0;
        for size in [1, 37, 997].into_iter().cycle() {
            if offset == input.len() {
                break;
            }
            let end = (offset + size).min(input.len());
            actual.extend(split.process_clocked(
                &input[offset..end],
                source_rate,
                stride,
                output_rate,
                1.0,
            ));
            assert_eq!(
                actual.len() as u128,
                (end as u128 * u128::from(stride) * u128::from(output_rate))
                    .div_ceil(u128::from(source_rate))
            );
            offset = end;
        }
        assert_eq!(actual, expected);
        assert!(split
            .process_clocked(&input[..1], 0, stride, output_rate, 1.0)
            .is_empty());
        let after = split.process_clocked(&input[..997], source_rate, stride, output_rate, 1.0);
        let mut fresh = IqAudioConverter::for_detector();
        assert_eq!(
            after,
            fresh.process_clocked(&input[..997], source_rate, stride, output_rate, 1.0)
        );
    }
}

#[test]
fn rational_capture_rate_changes_reset_phase_and_filter_history() {
    let samples = vec![Complex32::new(0.2, -0.1); 1007];
    let mut converter = IqAudioConverter::for_detector();
    converter.process_clocked(&samples, 2_048_003, 10, 48_000, 1.0);
    for (source_rate, stride, output_rate) in [
        (288_000, 1, 48_000),
        (288_000, 1, 8_000),
        (384_003, 7, 44_100),
    ] {
        let actual = converter.process_clocked(&samples, source_rate, stride, output_rate, 1.0);
        let mut fresh = IqAudioConverter::for_detector();
        assert_eq!(
            actual,
            fresh.process_clocked(&samples, source_rate, stride, output_rate, 1.0)
        );
    }
}
