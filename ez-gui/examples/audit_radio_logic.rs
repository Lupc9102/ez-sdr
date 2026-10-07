//! Run: cargo run -p ez-gui --no-default-features --example audit_radio_logic
#![allow(dead_code)]
#[path = "../src/audio_resampler.rs"]
mod audio_resampler;
use ez_daemon::{
    bus::{OverflowPolicy, SampleBlock, SampleBus},
    pipelines::audio::{AudioConfig, AudioPipeline},
};
use ez_gui::{
    demod::Demodulator,
    radio_iq::{RadioIqConfig, RadioIqProcessor},
    sdr_panel::DemodMode,
};
use num_complex::Complex32;
use std::{sync::Arc, time::Duration};

fn amplitude(samples: &[f32], hz: f64, rate: f64) -> f64 {
    let mut real = 0.0;
    let mut imaginary = 0.0;
    for (index, sample) in samples.iter().enumerate() {
        let angle = std::f64::consts::TAU * hz * index as f64 / rate;
        real += f64::from(*sample) * angle.cos();
        imaginary += f64::from(*sample) * angle.sin();
    }
    2.0 * real.hypot(imaginary) / samples.len() as f64
}

fn main() {
    for rate in [48_000, 200_000] {
        for hz in [300.0, -300.0, 1_000.0, -1_000.0] {
            let iq = (0..rate / 5)
                .map(|i| {
                    Complex32::from_polar(
                        0.3,
                        (std::f64::consts::TAU * hz * f64::from(i) / f64::from(rate)) as f32,
                    )
                })
                .collect();
            let pcm = daemon_pcm(ez_proto::DemodMode::Usb, rate, iq);
            let rms = (pcm[1000..]
                .iter()
                .map(|x| f64::from(*x).powi(2))
                .sum::<f64>()
                / (pcm.len() - 1000) as f64)
                .sqrt();
            println!("daemon_USB rate={rate} tone_hz={hz} rms={rms:.6}");
        }
    }
    let rate = 200_000;
    let iq = (0..rate)
        .map(|i| {
            Complex32::from_polar(
                0.5,
                (-12_000.0 / 57_000.0
                    * (std::f64::consts::TAU * 57_000.0 * f64::from(i) / f64::from(rate)).cos())
                    as f32,
            )
        })
        .collect();
    let pcm = daemon_pcm(ez_proto::DemodMode::Wfm, rate, iq);
    let audio = audio_resampler::AudioResampler::default().process(&pcm, rate, 48_000, 1.0);
    println!(
        "real_daemon_WFM 57kHz_modulation_12kHz_deviation -> 9kHz_alias_amplitude={:.6}",
        amplitude(&audio[5000..], 9000.0, 48000.0)
    );
    let rate = 200_000u32;
    let input: Vec<_> = (0..rate)
        .map(|index| {
            (std::f64::consts::TAU * 49_000.0 * f64::from(index) / f64::from(rate)).sin() as f32
                * 0.5
        })
        .collect();
    let output = audio_resampler::AudioResampler::default().process(&input, rate, 48_000, 1.0);
    println!(
        "daemon_audio 49kHz_input -> 1kHz_alias_amplitude={:.6}",
        amplitude(&output, 1_000.0, 48_000.0)
    );
    let mut demod = Demodulator::new();
    demod.set_sample_rates(192_000, 48_000);
    demod.set_agc_enabled(false);
    let raw = demod.demodulate_complex(&vec![Complex32::new(0.2, 0.1); 192_000], DemodMode::Raw);
    println!(
        "raw input_1_second=192000 mono_samples={} declared_audio_rate=48000",
        raw.len()
    );
    let components = [0.0001f32, 0.0002, 1.2, -1.3];
    let bytes: Vec<_> = components
        .iter()
        .flat_map(|component| component.to_le_bytes())
        .collect();
    let uc8 = lrpt_decode::cf32_le_bytes_to_uc8(&bytes, lrpt_decode::Cf32Amplitude::Normalized);
    let restored = RadioIqProcessor::new(RadioIqConfig::default()).process(&uc8);
    println!("CF32 original={components:?} replay_CU8={uc8:?} restored={restored:?}");
    let mut demod = Demodulator::new();
    demod.set_sample_rates(192_000, 48_000);
    demod.set_agc_enabled(false);
    demod.set_rf_bandwidth(10_000.0);
    demod.set_audio_hpf(1_000.0);
    let iq: Vec<_> = (0..192_000)
        .map(|index| {
            Complex32::new(
                0.5 + 0.1 * (std::f64::consts::TAU * 200.0 * index as f64 / 192_000.0).sin() as f32,
                0.0,
            )
        })
        .collect();
    let before = demod.demodulate_complex(&iq, DemodMode::Am);
    demod.reset();
    let after = demod.demodulate_complex(&iq, DemodMode::Am);
    println!(
        "HPF configured_1000Hz 200Hz_amplitude_before_reset={:.6} after_reset={:.6}",
        amplitude(&before[5_000..], 200.0, 48_000.0),
        amplitude(&after[5_000..], 200.0, 48_000.0)
    );
    let mut demod = Demodulator::new();
    demod.set_sample_rates(192_000, 8_000);
    demod.set_agc_enabled(false);
    demod.set_rf_bandwidth(15_000.0);
    demod.set_lpf_cutoff(4_000.0);
    let iq: Vec<_> = (0..192_000)
        .map(|index| {
            Complex32::new(
                0.5 + 0.1
                    * (std::f64::consts::TAU * 6_000.0 * index as f64 / 192_000.0).sin() as f32,
                0.0,
            )
        })
        .collect();
    let output = demod.demodulate_complex(&iq, DemodMode::Am);
    println!(
        "AM audio_8kHz input_modulation_6kHz -> 2kHz_alias={:.6}",
        amplitude(&output[1_000..], 2_000.0, 8_000.0)
    );
    for rate in [192_000, 2_048_000] {
        let mut demod = Demodulator::new();
        demod.set_sample_rates(rate, 48_000);
        demod.set_agc_enabled(false);
        demod.set_deemph_tau(0.0);
        demod.set_rf_bandwidth(15_000.0);
        let iq: Vec<_> = (0..rate)
            .map(|index| {
                let phase = -2.5
                    * (std::f64::consts::TAU * 1_000.0 * f64::from(index) / f64::from(rate)).cos();
                Complex32::from_polar(0.5, phase as f32)
            })
            .collect();
        let output = demod.demodulate_complex(&iq, DemodMode::Fm);
        println!(
            "NFM deviation_2500Hz rate={rate} tone_amplitude={:.6}",
            amplitude(&output[5_000..], 1_000.0, 48_000.0)
        );
    }
}

fn daemon_pcm(mode: ez_proto::DemodMode, rate: u32, iq: Vec<Complex32>) -> Vec<f32> {
    let bus = SampleBus::new();
    let mut pipeline = AudioPipeline::new(
        bus.subscribe(4, OverflowPolicy::DropOldest),
        AudioConfig {
            channel_id: 1,
            mode,
        },
    );
    let output = pipeline.subscribe(4);
    bus.publish(SampleBlock {
        observed_at: None,
        capture: None,
        start_sample: 0,
        sample_rate_hz: rate,
        center_freq_hz: 100_000_000,
        samples: Arc::from(iq),
    });
    assert!(pipeline.tick(Duration::ZERO));
    output.try_recv().unwrap().samples
}
