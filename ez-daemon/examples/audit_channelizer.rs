//! Run: cargo run -p ez-daemon --example audit_channelizer --release
use ez_daemon::{
    bus::{OverflowPolicy, SampleBlock, SampleBus},
    channelizer::Channelizer,
};
use num_complex::Complex32;
use std::{sync::Arc, time::Duration};

fn main() {
    for ideal_settled in [false, true] {
        for frequency in [1_000.0, 13_500.0, 26_000.0, 101_000.0] {
            let bus = SampleBus::new();
            let mut channelizer = Channelizer::new(
                bus.subscribe(8, OverflowPolicy::DropOldest),
                100_000_000,
                2_000_000,
            );
            let (_, output) = channelizer.add_channel(100_000_000, 12_500).unwrap();
            let samples: Vec<_> = (0..200_000)
                .map(|index| {
                    if ideal_settled {
                        let phase = std::f64::consts::TAU * f64::from(frequency) * index as f64
                            / 2_000_000.0;
                        Complex32::new((0.5 * phase.cos()) as f32, (0.5 * phase.sin()) as f32)
                    } else {
                        Complex32::from_polar(
                            0.5,
                            std::f32::consts::TAU * frequency * index as f32 / 2_000_000.0,
                        )
                    }
                })
                .collect();
            bus.publish(SampleBlock {
                product_session: 0,
                observed_at: None,
                capture: None,
                start_sample: 0,
                sample_rate_hz: 2_000_000,
                center_freq_hz: 100_000_000,
                samples: Arc::from(samples),
            });
            assert!(channelizer.tick(Duration::ZERO));
            let block = output.recv_timeout(Duration::from_secs(1)).unwrap();
            let warmup = if ideal_settled { 128 } else { 50 };
            let rms = (block.samples[warmup..]
                .iter()
                .map(|sample| sample.norm_sqr() as f64)
                .sum::<f64>()
                / (block.samples.len() - warmup) as f64)
                .sqrt();
            println!(
            "ideal_settled={ideal_settled} channel_min_rate=12500 input_rate=2000000 tone_hz={frequency} output_rms={rms:.9}"
        );
        }
    }
}
