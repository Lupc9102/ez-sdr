//! Real SampleBus/DDC measurements, not coefficient-only response estimates.
//! Run: cargo run -p ez-daemon --release --example audit_channelizer_sweep
use ez_daemon::{
    bus::{OverflowPolicy, SampleBlock, SampleBus},
    channelizer::Channelizer,
};
use num_complex::Complex32;
use std::{sync::Arc, time::Duration};

fn receive(rate: u32, minimum: u32, components: &[(f64, f64)], chunk: usize) -> Vec<Complex32> {
    let bus = SampleBus::new();
    let mut channelizer = Channelizer::new(
        bus.subscribe(8, OverflowPolicy::DropIncoming),
        100_000_000,
        rate,
    );
    let offset = rate / 16;
    let (_, output) = channelizer
        .add_channel(100_000_000 + u64::from(offset), minimum)
        .unwrap();
    let d = rate / minimum;
    let count = 256 * d as usize;
    let mut received = Vec::new();
    for first in (0..count).step_by(chunk) {
        let end = (first + chunk).min(count);
        let samples = (first..end)
            .map(|n| {
                let mut re = 0.0;
                let mut im = 0.0;
                for &(frequency, amplitude) in components {
                    let angle = std::f64::consts::TAU * (frequency + f64::from(offset)) * n as f64
                        / f64::from(rate);
                    re += amplitude * angle.cos();
                    im += amplitude * angle.sin();
                }
                Complex32::new(re as f32, im as f32)
            })
            .collect::<Vec<_>>();
        bus.publish(SampleBlock {
            product_session: 42,
            observed_at: None,
            capture: Some(ez_proto::CaptureClock::new(7, first as u64, rate, None)),
            start_sample: first as u64,
            sample_rate_hz: rate,
            center_freq_hz: 100_000_000,
            samples: Arc::from(samples),
        });
        assert!(channelizer.tick(Duration::ZERO));
        if let Some(block) = output.try_recv() {
            assert_eq!(
                block.capture_clock().source_sample_index,
                received.len() as u64 * u64::from(d)
            );
            assert_eq!(block.capture_clock().sample_stride, d);
            assert_eq!(block.product_session, 42);
            received.extend_from_slice(&block.samples);
        }
    }
    assert_eq!(received.len(), 256);
    received
}

fn rms(samples: &[Complex32]) -> f64 {
    (samples.iter().map(|x| f64::from(x.norm_sqr())).sum::<f64>() / samples.len() as f64).sqrt()
}

fn main() {
    let fractions = [
        0.0_f64, 0.1, 0.2, 0.3, 0.38, 0.39, 0.4, 0.5, 0.51, 0.52, 0.6, 0.75, 0.9, 1.0, 1.08, 1.5,
        2.0, 2.08, 3.0, 4.0, 7.0, 8.08, 12.0,
    ];
    let mut measurements = 0;
    let mut max_pass_error = 0.0_f64;
    let mut minimum_rejection = f64::INFINITY;
    let mut maximum_mixture_error = 0.0_f64;
    for rate in [2_000_000, 2_048_003, 2_400_000] {
        for minimum in [12_500, 25_000, 200_000, 288_000, 320_000] {
            let d = rate / minimum;
            let out_rate = f64::from(rate) / f64::from(d);
            for fraction in fractions {
                for direction in [-1.0, 1.0] {
                    let frequency = fraction * out_rate * direction;
                    if (frequency + f64::from(rate / 16)).abs() >= f64::from(rate) / 2.0 {
                        continue;
                    }
                    let samples = receive(rate, minimum, &[(frequency, 0.5)], 997);
                    // Complete causal FIR startup is excluded, identically for
                    // desired and interfering carriers. No samples are flushed.
                    let gain_db = 20.0 * (rms(&samples[128..]) / 0.5).log10();
                    println!("rate={rate} requested={minimum} d={d} relative={frequency:.6} gain_db={gain_db:.6}");
                    if fraction <= 0.4 {
                        max_pass_error = max_pass_error.max(gain_db.abs());
                        assert!(gain_db.abs() <= 0.02, "passband gain {gain_db}");
                    } else {
                        minimum_rejection = minimum_rejection.min(-gain_db);
                        assert!(gain_db <= -80.0, "stopband gain {gain_db}");
                    }
                    measurements += 1;
                }
            }
            let wanted = 0.08 * out_rate;
            let reference = receive(rate, minimum, &[(wanted, 0.01)], 997);
            let mixture = receive(
                rate,
                minimum,
                &[
                    (wanted, 0.01),
                    (out_rate + wanted, 0.24),
                    (wanted - out_rate, 0.24),
                    (2.0 * out_rate + wanted, 0.24),
                    (wanted - 2.0 * out_rate, 0.24),
                ],
                4093,
            );
            let differences: Vec<_> = mixture[128..]
                .iter()
                .zip(&reference[128..])
                .map(|(&a, &b)| a - b)
                .collect();
            let relative_error = rms(&differences) / rms(&reference[128..]);
            maximum_mixture_error = maximum_mixture_error.max(relative_error);
            println!("mixture rate={rate} requested={minimum} interferers=4 each_over_wanted_db=27.604225 relative_error={relative_error:.9}");
            assert!(
                relative_error < 0.005,
                "strong-adjacent mixture error {relative_error}"
            );
        }
    }
    println!("summary measurements={measurements} max_pass_error_db={max_pass_error:.9} minimum_rejection_db={minimum_rejection:.9} max_mixture_relative_error={maximum_mixture_error:.9}");
}
