//! Synthetic local Radio checks: CPU DSP/FFT work, not native rendering,
//! USB reception, speaker callbacks or listening latency.
use ez_gui::demod::{Demodulator, FmIfPreset};
use ez_gui::radio_iq::{RadioIqConfig, RadioIqProcessor, VfoMixer};
use ez_gui::radio_rds::RdsDecoder;
use ez_gui::sdr_panel::DemodMode;
use ez_gui::spectrum::SpectrumAnalyzer;
use std::f64::consts::TAU;
use std::time::Instant;

const AUDIO_RATE: u32 = 48_000;
const CAPTURE_CENTER: u64 = 100_000_000;

fn encode_iq(i: f64, q: f64) -> [u8; 2] {
    [i, q].map(|sample| (127.4 + 128.0 * sample).round().clamp(0.0, 255.0) as u8)
}

fn configured_demod(rate: f64, mode: DemodMode) -> Demodulator {
    let mut demod = Demodulator::new();
    demod.set_sample_rates_exact(rate, AUDIO_RATE);
    demod.set_rf_bandwidth(mode.default_rf_bandwidth_hz());
    demod.set_lpf_cutoff(if mode == DemodMode::Wfm {
        15_000.0
    } else {
        4_000.0
    });
    demod
}

fn one_second_iq(rate: u32, mode: DemodMode, stereo: bool, offset: f64) -> Vec<u8> {
    one_second_control_iq(rate, mode, stereo, offset, false)
}

// Independent transmitter, shared only by the fixtures in this test module.
// The serial register forms the RDS checkword without calling decoder helpers.
fn rds_symbols() -> Vec<f64> {
    let mut bits = vec![false; 128];
    let name = b"ASTRA FM";
    for segment in (0..4).cycle().take(12) {
        let words = [
            0x54a7_u16,
            0x0400 | (10 << 5) | 0x18 | segment as u16,
            0,
            u16::from_be_bytes([name[2 * segment], name[2 * segment + 1]]),
        ];
        for (word, offset) in words.into_iter().zip([0x0fc, 0x198, 0x168, 0x1b4]) {
            let mut register = 0_u16;
            for bit in (0..16).rev() {
                let feedback = ((register >> 9) ^ (word >> bit)) & 1;
                register = (register << 1) & 0x3ff;
                if feedback != 0 {
                    register ^= 0x1b9;
                }
            }
            let encoded = (u32::from(word) << 10) | u32::from(register ^ offset);
            bits.extend((0..26).rev().map(|bit| encoded & (1 << bit) != 0));
        }
    }
    let mut sign = 1.0;
    bits.into_iter()
        .map(|bit| {
            if bit {
                sign = -sign;
            }
            sign
        })
        .collect()
}

fn one_second_control_iq(
    rate: u32,
    mode: DemodMode,
    stereo: bool,
    offset: f64,
    rds: bool,
) -> Vec<u8> {
    let mut iq = Vec::with_capacity(rate as usize * 2);
    let mut phase = 0.73_f64;
    let symbols = if rds { rds_symbols() } else { Vec::new() };
    for index in 0..rate {
        let time = f64::from(index) / f64::from(rate);
        let (amplitude, angle) = match mode {
            DemodMode::Wfm => {
                let left = 0.22 * (TAU * 1_000.0 * time).cos();
                let right = 0.17 * (TAU * 1_700.0 * time).cos();
                let pilot = TAU * 19_000.0 * time;
                let mut multiplex = if stereo {
                    0.5 * (left + right)
                        + 0.5 * (left - right) * (2.0 * pilot).cos()
                        + 0.1 * pilot.cos()
                } else {
                    left
                };
                if rds {
                    let position = time * 1_187.5 - 0.47;
                    if position >= 0.0 {
                        let biphase = symbols[position as usize]
                            * if position.fract() < 0.5 { 1.0 } else { -1.0 };
                        multiplex += 0.065 * biphase * (TAU * 57_000.0 * time + 1.17).cos();
                    }
                    if !stereo {
                        multiplex += 0.1 * pilot.cos();
                    }
                }
                phase = (phase + TAU * (offset + 75_000.0 * multiplex) / f64::from(rate))
                    .rem_euclid(TAU);
                (0.65, phase)
            }
            DemodMode::Fm => {
                let modulation =
                    1_200.0 * (TAU * 1_000.0 * time).cos() + 400.0 * (TAU * 100.0 * time).sin();
                phase = (phase + TAU * (offset + modulation) / f64::from(rate)).rem_euclid(TAU);
                (0.65, phase)
            }
            DemodMode::Am => (
                0.4 * (1.0 + 0.5 * (TAU * 900.0 * time).cos()),
                TAU * offset * time,
            ),
            DemodMode::Cw => (0.65, TAU * offset * time),
            _ => unreachable!("throughput fixture only covers AM/NFM/WFM/CW"),
        };
        iq.extend(encode_iq(
            amplitude * angle.cos() + 0.02,
            amplitude * angle.sin() - 0.03,
        ));
    }
    iq
}

#[test]
#[ignore = "release-only enabled-control throughput; run with --release --ignored --nocapture"]
fn radio_enabled_controls_throughput() {
    const RATE: u32 = 2_400_003;
    const VFO_OFFSET: f64 = 18_000.0;
    for (mode, stereo, nr, rds) in [
        (DemodMode::Fm, false, true, false),
        (DemodMode::Wfm, false, true, false),
        (DemodMode::Wfm, true, true, false),
        (DemodMode::Wfm, false, false, true),
        (DemodMode::Wfm, true, false, true),
    ] {
        // Fixture construction is outside every timed region.
        let iq = one_second_control_iq(RATE, mode, stereo, VFO_OFFSET, rds);
        assert_eq!(iq.len(), RATE as usize * 2, "exactly one source second");
        for decimation in [1, 8] {
            let mut processor = RadioIqProcessor::new(RadioIqConfig {
                input_rate: RATE,
                dc_remove: true,
                invert: false,
                decimation,
            });
            let rate = processor.output_rate();
            assert_eq!(rate, f64::from(RATE) / f64::from(decimation));
            let mut mixer = VfoMixer::new(rate);
            mixer.configure(rate, VFO_OFFSET);
            let mut demod = configured_demod(rate, mode);
            demod.set_fm_if_noise_reduction(nr, FmIfPreset::Voice);
            demod.set_rds_tap_enabled(rds);
            let mut decoder = RdsDecoder::new();
            decoder.set_incremental(false);
            let mut spectrum = SpectrumAnalyzer::new();
            assert!(spectrum.set_fft_size(65_536));
            spectrum.set_fft_rate(20);
            spectrum.update_params_exact(CAPTURE_CENTER, rate);
            spectrum.vfo_freq_hz = Some(CAPTURE_CENTER + VFO_OFFSET as u64);
            spectrum.demod_mode = mode.label().into();
            spectrum.vfo_bw_hz = mode.default_rf_bandwidth_hz() as u32;

            let start = Instant::now();
            let mut processed_count = 0;
            let mut audio_count = 0usize;
            let mut multiplex_count = 0usize;
            for block in iq.chunks(32_767) {
                let corrected = processor.process(block);
                processed_count += corrected.len();
                spectrum.push_complex_samples(&corrected);
                let tuned = mixer.process(&corrected);
                audio_count += if stereo {
                    demod.demodulate_stereo_complex(&tuned, mode).len()
                } else {
                    demod.demodulate_complex(&tuned, mode).len()
                };
                if rds {
                    let (multiplex, multiplex_rate) = demod.take_wfm_multiplex();
                    assert_eq!(multiplex_rate, rate);
                    multiplex_count += multiplex.len();
                    decoder.process_multiplex(&multiplex, multiplex_rate);
                }
            }
            let elapsed = start.elapsed().as_secs_f64();
            assert_eq!(processed_count, RATE as usize / decimation as usize);
            assert!(audio_count.abs_diff(AUDIO_RATE as usize) <= 1);
            assert!(spectrum.peak_level().is_finite());
            assert!(demod.last_audio_peak.is_finite());
            let status = decoder.snapshot();
            if rds {
                assert_eq!(multiplex_count, processed_count);
                assert!(status.valid_groups >= 8, "{status:?}");
                assert_eq!(status.pi, Some(0x54a7), "{status:?}");
                assert_eq!(
                    status.program_service.as_deref(),
                    Some("ASTRA FM"),
                    "{status:?}"
                );
                assert!(status.synchronized);
            }
            println!("{}{} FMIF={nr} RDS={rds} /{decimation}: {RATE} source pairs (1.000000 s), {rate:.3} Hz effective, IQ correction + FFT65536/20Hz + mixer + demod{} in {elapsed:.3} s ({:.1}x realtime); {audio_count} audio frames / {} channels, {multiplex_count} MPX samples, {} CRC-valid RDS groups, PS {:?}", mode.label(), if stereo { " stereo" } else { "" }, if rds { " + RDS decoder" } else { "" }, 1.0 / elapsed, if stereo { 2 } else { 1 }, status.valid_groups, status.program_service);
        }
    }
}

#[test]
#[ignore = "release-only throughput evidence; run with --release --ignored --nocapture"]
fn radio_pipeline_throughput() {
    // Not divisible by 8: reduced rate must stay 300000.375 Hz.
    const RATE: u32 = 2_400_003;
    const VFO_OFFSET: f64 = 18_000.0;
    for (mode, stereo) in [
        (DemodMode::Am, false),
        (DemodMode::Wfm, false),
        (DemodMode::Cw, false),
        (DemodMode::Wfm, true),
    ] {
        let iq = one_second_iq(RATE, mode, stereo, VFO_OFFSET);
        assert_eq!(iq.len(), RATE as usize * 2, "exactly one source second");
        for decimation in [1, 8] {
            let mut processor = RadioIqProcessor::new(RadioIqConfig {
                input_rate: RATE,
                dc_remove: true,
                invert: false,
                decimation,
            });
            let rate = processor.output_rate();
            assert_eq!(rate, f64::from(RATE) / f64::from(decimation));
            let mut mixer = VfoMixer::new(rate);
            mixer.configure(rate, VFO_OFFSET);
            let mut demod = configured_demod(rate, mode);
            let mut spectrum = SpectrumAnalyzer::new();
            assert!(spectrum.set_fft_size(65_536));
            spectrum.set_fft_rate(20);
            spectrum.update_params_exact(CAPTURE_CENTER, rate);
            spectrum.vfo_freq_hz = Some(CAPTURE_CENTER + VFO_OFFSET as u64);
            spectrum.demod_mode = mode.label().into();
            spectrum.vfo_bw_hz = mode.default_rf_bandwidth_hz() as u32;

            let start = Instant::now();
            let mut audio_count = 0usize;
            let mut processed_count = 0;
            // Odd chunks exercise source I/Q byte-pair retention.
            for block in iq.chunks(32_767) {
                let corrected = processor.process(block);
                processed_count += corrected.len();
                spectrum.push_complex_samples(&corrected);
                let tuned = mixer.process(&corrected);
                audio_count += if stereo {
                    demod.demodulate_stereo_complex(&tuned, mode).len()
                } else {
                    demod.demodulate_complex(&tuned, mode).len()
                };
            }
            let elapsed = start.elapsed().as_secs_f64();
            assert_eq!(processed_count, RATE as usize / decimation as usize);
            assert!(
                audio_count.abs_diff(AUDIO_RATE as usize) <= 1,
                "{} decimation {decimation}: {audio_count} output frames",
                mode.label()
            );
            assert!(spectrum.peak_level().is_finite());
            assert!(demod.last_audio_peak.is_finite());
            println!("{}{} /{decimation}: {RATE} source pairs (1.000000 s), {rate:.3} Hz effective, VFO {VFO_OFFSET:+.0} Hz, IQ correction + FFT65536/20Hz + mixer + demod in {elapsed:.3} s ({:.1}x realtime); {audio_count} audio frames, stereo lock {}", mode.label(), if stereo { " stereo" } else { "" }, 1.0 / elapsed, demod.last_stereo_locked);
        }
    }
}

fn tone_amplitude(audio: &[f32], frequency: f64) -> f64 {
    let (real, imaginary) =
        audio
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(real, imaginary), (index, sample)| {
                let phase = TAU * frequency * index as f64 / f64::from(AUDIO_RATE);
                (
                    real + f64::from(*sample) * phase.cos(),
                    imaginary + f64::from(*sample) * phase.sin(),
                )
            });
    2.0 * real.hypot(imaginary) / audio.len() as f64
}

#[test]
fn offcenter_am_selection_survives_corrected_decimated_float_pipeline() {
    const RATE: u32 = 768_003;
    const SELECTED_OFFSET: f64 = 18_000.0;
    const NEIGHBOR_OFFSET: f64 = -22_000.0;
    let iq: Vec<u8> = (0..RATE / 4)
        .flat_map(|index| {
            let time = f64::from(index) / f64::from(RATE);
            let selected = 0.25 * (1.0 + 0.45 * (TAU * 1_000.0 * time).cos());
            let neighbor = 0.4 * (1.0 + 0.45 * (TAU * 2_300.0 * time).cos());
            encode_iq(
                selected * (TAU * SELECTED_OFFSET * time).cos()
                    + neighbor * (TAU * NEIGHBOR_OFFSET * time).cos()
                    + 0.02,
                selected * (TAU * SELECTED_OFFSET * time).sin()
                    + neighbor * (TAU * NEIGHBOR_OFFSET * time).sin()
                    - 0.03,
            )
        })
        .collect();
    for decimation in [1, 8] {
        let mut processor = RadioIqProcessor::new(RadioIqConfig {
            input_rate: RATE,
            dc_remove: true,
            invert: false,
            decimation,
        });
        let rate = processor.output_rate();
        let mut spectrum = SpectrumAnalyzer::new();
        spectrum.set_fft_size(2048);
        spectrum.set_avg_alpha(1.0);
        spectrum.update_params_exact(CAPTURE_CENTER, rate);
        spectrum.vfo_freq_hz = Some(CAPTURE_CENTER + SELECTED_OFFSET as u64);
        spectrum.vfo_bw_hz = 8_000;
        spectrum.demod_mode = "AM".into();
        let mut mixers = [VfoMixer::new(rate), VfoMixer::new(rate)];
        mixers[0].configure(rate, SELECTED_OFFSET);
        mixers[1].configure(rate, NEIGHBOR_OFFSET);
        let mut demods = [
            configured_demod(rate, DemodMode::Am),
            configured_demod(rate, DemodMode::Am),
        ];
        for demod in &mut demods {
            demod.set_agc_enabled(false);
            demod.set_rf_bandwidth(8_000.0);
        }
        let mut audio = [Vec::new(), Vec::new()];
        for bytes in iq.chunks(4093) {
            let corrected = processor.process(bytes);
            spectrum.push_complex_samples(&corrected);
            for channel in 0..2 {
                let tuned = mixers[channel].process(&corrected);
                audio[channel].extend(demods[channel].demodulate_complex(&tuned, DemodMode::Am));
            }
        }
        for channel in &audio {
            assert!(
                channel.len().abs_diff(12_000) <= 1,
                "incorrect audio clock: {}",
                channel.len()
            );
            assert!(channel.iter().all(|sample| sample.is_finite()));
        }
        let selected_tone = tone_amplitude(&audio[0][2400..], 1000.0);
        let leaked_neighbor = tone_amplitude(&audio[0][2400..], 2300.0);
        let neighbor_tone = tone_amplitude(&audio[1][2400..], 2300.0);
        let leaked_selected = tone_amplitude(&audio[1][2400..], 1000.0);
        assert!(
            selected_tone > 0.05,
            "/{decimation} lost selected AM tone: {selected_tone}"
        );
        assert!(
            neighbor_tone > 0.08,
            "/{decimation} failed neighbor tuning: {neighbor_tone}"
        );
        assert!(
            leaked_neighbor < selected_tone * 0.03,
            "/{decimation} passed untuned neighbor: {leaked_neighbor} vs {selected_tone}"
        );
        assert!(
            leaked_selected < neighbor_tone * 0.03,
            "/{decimation} failed to change selected channel: {leaked_selected} vs {neighbor_tone}"
        );
        // Spectrum receives pre-mixer IQ: its strongest carrier stays at the
        // neighbor's original RF frequency, independently of selected audio.
        let expected_peak = (CAPTURE_CENTER as f64 + NEIGHBOR_OFFSET) as u64;
        assert!(spectrum.peak_freq_hz().abs_diff(expected_peak) as f64 <= rate / 2048.0);
        assert!(spectrum.vfo_signal_level() > -20.0);
        println!("AM /{decimation}, effective {rate:.3} Hz: selected1k={selected_tone:.5}, leaked2.3k={leaked_neighbor:.5}; neighbor2.3k={neighbor_tone:.5}, leaked1k={leaked_selected:.5}");
    }
}
