use super::*;
use crate::bus::{OverflowPolicy as BusOverflowPolicy, SampleBus};
use ez_proto::CaptureClock;
use std::sync::Arc;

fn pipeline(mode: DemodMode, parameters: AudioParameters) -> AudioPipeline {
    let bus = SampleBus::new();
    AudioPipeline::new(
        bus.subscribe(2, BusOverflowPolicy::DropIncoming),
        AudioConfig {
            channel_id: 71,
            mode,
            audio: Some(parameters),
        },
    )
}

fn block(samples: &[Complex32], clock: CaptureClock) -> SampleBlock {
    SampleBlock {
        product_session: 0,
        observed_at: None,
        capture: Some(clock),
        start_sample: clock.source_sample_index / u64::from(clock.sample_stride),
        sample_rate_hz: clock.source_sample_rate_hz / clock.sample_stride,
        center_freq_hz: 100_000_000,
        samples: Arc::from(samples),
    }
}

fn render(
    mode: DemodMode,
    parameters: AudioParameters,
    clock: CaptureClock,
    input: &[Complex32],
) -> Vec<f32> {
    let mut ap = pipeline(mode, parameters);
    let out = ap.subscribe(2);
    ap.process_block(&block(input, clock));
    let frame = out.try_recv().unwrap();
    assert_eq!(frame.sample_rate_hz, parameters.output_sample_rate_hz);
    assert_eq!(
        frame.audio_clock.source_sample_rate_hz,
        parameters.output_sample_rate_hz
    );
    assert_eq!(
        frame.clock.source_sample_rate_hz,
        clock.source_sample_rate_hz
    );
    assert_eq!(frame.clock.sample_stride, clock.sample_stride);
    frame.samples
}

fn rms(input: &[f32]) -> f64 {
    (input.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / input.len() as f64).sqrt()
}

#[test]
fn selected_am_fm_wfm_reject_strong_neighbors_before_detection_and_squelch() {
    let mut checks = 0;
    let mut maximum_relative_error = 0.0_f64;
    for (mode, source_rate, stride) in [
        (DemodMode::Am, 200_000, 1),
        (DemodMode::Am, 2_048_003, 10),
        (DemodMode::Fm, 200_000, 1),
        (DemodMode::Fm, 2_048_003, 10),
        (DemodMode::Wfm, 256_000, 1),
        (DemodMode::Wfm, 288_000, 1),
    ] {
        let mut clock = CaptureClock::new(91, 0, source_rate, None);
        clock.sample_stride = stride;
        let rate = clock.exact_sample_rate_hz();
        let parameters = AudioParameters::for_mode(mode);
        let count = (rate * 0.12).ceil() as usize;
        let wanted: Vec<_> = (0..count)
            .map(|n| {
                let t = n as f64 / rate;
                let m = (std::f64::consts::TAU * 1000.0 * t).cos();
                let signal = if mode == DemodMode::Am {
                    num_complex::Complex64::new(0.02 * (1.0 + 0.25 * m), 0.0)
                } else {
                    let deviation = if mode == DemodMode::Fm {
                        2000.0
                    } else {
                        30_000.0
                    };
                    num_complex::Complex64::from_polar(
                        0.02,
                        deviation / 1000.0 * (std::f64::consts::TAU * 1000.0 * t).sin(),
                    )
                };
                Complex32::new(signal.re as f32, signal.im as f32)
            })
            .collect();
        let reference = render(mode, parameters, clock, &wanted);
        let settled = reference.len() - 1920;
        let wanted_rms = rms(&reference[settled..]);
        assert!(
            wanted_rms > if mode == DemodMode::Am { 0.003 } else { 0.2 },
            "missing wanted audio {mode:?}/{rate}: {wanted_rms}"
        );
        for sign in [-1.0, 1.0] {
            let adjacent_hz = sign
                * if mode == DemodMode::Wfm {
                    120_000.0
                } else {
                    30_000.0
                };
            // Adjacent carrier is 25 times (27.96 dB) stronger than the wanted carrier.
            let neighbor: Vec<_> = (0..count)
                .map(|n| {
                    let phase = std::f64::consts::TAU * adjacent_hz * n as f64 / rate;
                    Complex32::new((0.5 * phase.cos()) as f32, (0.5 * phase.sin()) as f32)
                })
                .collect();
            let mixture: Vec<_> = wanted.iter().zip(&neighbor).map(|(a, b)| a + b).collect();
            let mixed = render(mode, parameters, clock, &mixture);
            let difference: Vec<_> = mixed[settled..]
                .iter()
                .zip(&reference[settled..])
                .map(|(a, b)| a - b)
                .collect();
            let relative_error = rms(&difference) / wanted_rms;
            maximum_relative_error = maximum_relative_error.max(relative_error);
            assert!(
                relative_error < 0.01,
                "{mode:?}/{rate}/{sign} neighbor error={relative_error}"
            );

            let mut ap = pipeline(mode, parameters);
            ap.set_squelch(-20.0); // wanted (-34 dBFS) is closed; adjacent (-6 dBFS) must stay excluded.
            let out = ap.subscribe(2);
            ap.process_block(&block(&mixture, clock));
            out.try_recv().unwrap();
            ap.process_block(&block(
                &mixture,
                clock.offset_samples(count as u64).unwrap(),
            ));
            assert!(
                out.try_recv().unwrap().samples.iter().all(|&x| x == 0.0),
                "excluded neighbor opened {mode:?} squelch"
            );
            ap.set_squelch(-40.0);
            ap.process_block(&block(
                &mixture,
                clock.offset_samples(2 * count as u64).unwrap(),
            ));
            assert!(
                rms(&out.try_recv().unwrap().samples[settled..]) > wanted_rms * 0.98,
                "selected wanted carrier failed to open squelch"
            );
            checks += 1;
        }
    }
    eprintln!("selected_rf_mixtures={checks} adjacent_to_wanted_db=27.9588 max_relative_pcm_error={maximum_relative_error:.9}");
}

#[test]
fn independent_pcm_clock_preserves_rational_rf_provenance_and_partition_history() {
    for (mode, source_rate, stride, pcm_rate) in [
        (DemodMode::Am, 2_048_003, 10, 48_000),
        (DemodMode::Fm, 200_000, 1, 44_100),
        (DemodMode::Wfm, 256_000, 1, 48_000),
        (DemodMode::Usb, 2_048_003, 10, 8_000),
    ] {
        for origin in [None, Some(1_000_000_000)] {
            let mut clock = CaptureClock::new(92, 170, source_rate, origin);
            clock.sample_stride = stride;
            let rate = clock.exact_sample_rate_hz();
            let input: Vec<_> = (0..12_007)
                .map(|n| {
                    let phase = std::f64::consts::TAU * 1000.0 * n as f64 / rate;
                    Complex32::new((0.1 * phase.cos()) as f32, (0.1 * phase.sin()) as f32)
                })
                .collect();
            let parameters = AudioParameters {
                output_sample_rate_hz: pcm_rate,
                ..AudioParameters::for_mode(mode)
            };
            let expected = render(mode, parameters, clock, &input);
            assert_eq!(
                expected.len() as u128,
                (input.len() as u128 * u128::from(stride) * u128::from(pcm_rate))
                    .div_ceil(u128::from(source_rate))
            );
            let mut ap = pipeline(mode, parameters);
            let out = ap.subscribe(2);
            let mut actual = Vec::new();
            let mut offset = 0;
            let mut pcm_index = 0;
            let mut epoch = None;
            for size in [1, 37, 997].into_iter().cycle() {
                if offset == input.len() {
                    break;
                }
                let end = (offset + size).min(input.len());
                let rf_clock = clock.offset_samples(offset as u64).unwrap();
                ap.process_block(&block(&input[offset..end], rf_clock));
                let frame = out.try_recv().unwrap();
                assert_eq!(
                    frame.clock.source_sample_index,
                    rf_clock.source_sample_index
                );
                assert_eq!(frame.clock.epoch, clock.epoch);
                assert_eq!(frame.audio_clock.source_sample_index, pcm_index);
                assert_eq!(frame.audio_clock.sample_stride, 1);
                assert_eq!(frame.audio_clock.origin_unix_ns, clock.unix_time_ns(0));
                assert_eq!(frame.audio_clock.discontinuous, offset == 0);
                if let Some(epoch) = epoch {
                    assert_eq!(frame.audio_clock.epoch, epoch);
                }
                epoch = Some(frame.audio_clock.epoch);
                pcm_index += frame.samples.len() as u64;
                actual.extend(frame.samples);
                offset = end;
            }
            assert_eq!(
                actual, expected,
                "{mode:?} PCM changed with packet boundaries"
            );
            let mut after_clock = clock.offset_samples(input.len() as u64 + 11).unwrap();
            after_clock.discontinuous = true;
            ap.process_block(&block(&input[..997], after_clock));
            let after = out.try_recv().unwrap();
            assert_ne!(Some(after.audio_clock.epoch), epoch);
            assert_eq!(after.audio_clock.source_sample_index, 0);
            assert_eq!(
                after.audio_clock.origin_unix_ns,
                after_clock.unix_time_ns(0)
            );
            assert!(after.audio_clock.discontinuous);
            assert_eq!(
                after.samples,
                render(mode, parameters, after_clock, &input[..997])
            );
        }
    }
}

#[test]
fn audio_parameter_geometry_is_validated_even_when_raw_selection_is_bypassed() {
    for rate in [0.0, f64::NAN, f64::INFINITY, 20_000_001.0] {
        assert!(AudioPipeline::validate_parameters(
            DemodMode::Raw,
            AudioParameters::for_mode(DemodMode::Raw),
            rate
        )
        .is_err());
    }
    for output_sample_rate_hz in [1, 7999, 384_001, u32::MAX] {
        assert!(AudioPipeline::validate_parameters(
            DemodMode::Fm,
            AudioParameters {
                output_sample_rate_hz,
                ..AudioParameters::for_mode(DemodMode::Fm)
            },
            200_000.0
        )
        .is_err());
    }
    assert!(AudioPipeline::validate_parameters(
        DemodMode::Wfm,
        AudioParameters::for_mode(DemodMode::Wfm),
        200_000.0
    )
    .is_err());
    assert!(AudioPipeline::validate_parameters(
        DemodMode::Wfm,
        AudioParameters::for_mode(DemodMode::Wfm),
        256_000.0
    )
    .is_ok());
}
