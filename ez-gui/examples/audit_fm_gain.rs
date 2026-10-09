//! Physical deviation/gain measurements through actual GUI demodulation.
//! cargo run -p ez-gui --no-default-features --example audit_fm_gain
use ez_gui::{
    demod::{Demodulator, FmIfPreset},
    sdr_panel::DemodMode,
};
use num_complex::Complex32;

fn amplitude(samples: &[f32], frequency: f64, rate: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &x) in samples.iter().enumerate() {
        let phase = std::f64::consts::TAU * frequency * n as f64 / rate;
        re += f64::from(x) * phase.cos();
        im += f64::from(x) * phase.sin();
    }
    2.0 * re.hypot(im) / samples.len() as f64
}

fn sweep(audio_rate: u32) {
    for rate in [192003, 384000, 2048003] {
        for decimation in [1, 2, 4] {
            for nr in [
                None,
                Some(FmIfPreset::NoaaApt),
                Some(FmIfPreset::Voice),
                Some(FmIfPreset::NarrowBand),
            ] {
                for mode in [DemodMode::Fm, DemodMode::Wfm] {
                    for stereo in [false, true] {
                        if stereo && mode == DemodMode::Fm {
                            continue;
                        }
                        let deviation = if mode == DemodMode::Fm {
                            2500.0
                        } else {
                            10000.0
                        };
                        let expected = deviation
                            / if mode == DemodMode::Fm {
                                5000.0
                            } else {
                                75000.0
                            };
                        let count = (f64::from(rate) * 0.25) as usize;
                        let input: Vec<_> = (0..count)
                            .map(|n| {
                                let phase = deviation / 1000.0
                                    * (std::f64::consts::TAU * 1000.0 * n as f64 / f64::from(rate))
                                        .sin();
                                Complex32::new(
                                    (0.5 * phase.cos()) as f32,
                                    (0.5 * phase.sin()) as f32,
                                )
                            })
                            .collect();
                        let mut demod = Demodulator::new();
                        demod.set_sample_rates(rate, audio_rate);
                        demod.set_rf_decim(decimation);
                        demod.set_agc_enabled(false);
                        demod.set_fm_lowpass_enabled(false);
                        demod.set_deemph_tau(0.0);
                        demod.set_rf_bandwidth(if mode == DemodMode::Fm {
                            15000.0
                        } else {
                            150000.0
                        });
                        demod.set_fm_if_noise_reduction(nr.is_some(), nr.unwrap_or_default());
                        let audio = if stereo {
                            demod
                                .demodulate_stereo_complex(&input, mode)
                                .iter()
                                .map(|x| (x[0] + x[1]) * 0.5)
                                .collect()
                        } else {
                            demod.demodulate_complex(&input, mode)
                        };
                        let skip = (f64::from(audio_rate) * 0.15) as usize;
                        let observed = amplitude(&audio[skip..], 1000.0, f64::from(audio_rate));
                        let gain_db = 20.0 * (observed / expected).log10();
                        let deviation_error =
                            (f64::from(demod.last_fm_deviation_hz) / deviation - 1.0).abs();
                        assert!(gain_db.abs() <= 0.15, "physical gain changed: {gain_db} dB");
                        assert!(
                            deviation_error <= 0.03,
                            "physical deviation changed: {deviation_error}"
                        );
                        assert!(
                            audio
                                .len()
                                .abs_diff((f64::from(audio_rate) * 0.25) as usize)
                                <= 1,
                            "wrong audio timebase"
                        );
                        let settled = &audio[skip..];
                        let crossings = settled
                            .windows(2)
                            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                            .count();
                        assert!(
                            (crossings as f64
                                - settled.len() as f64 * 1000.0 / f64::from(audio_rate))
                            .abs()
                                <= 1.0,
                            "wrong 1 kHz pitch"
                        );
                        println!("audio_rate={audio_rate} rate={rate} rf_decim={decimation} nr={nr:?} mode={mode:?} stereo={stereo} gain={:.9} gain_db={:.6} deviation={:.6} deviation_ratio={:.9} samples={}",observed/expected,20.0*(observed/expected).log10(),demod.last_fm_deviation_hz,demod.last_fm_deviation_hz/deviation as f32,audio.len());
                    }
                }
            }
        }
    }
}

fn main() {
    for audio_rate in [44_100, 48_000] {
        sweep(audio_rate);
    }
}
