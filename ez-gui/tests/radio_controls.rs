//! Signal-level control checks through the public local-receiver pipeline.
//! Fixtures model transmitters; no USB, native window, or speaker is involved.
use ez_gui::demod::Demodulator;
use ez_gui::radio_iq::{RadioIqConfig, RadioIqProcessor, VfoMixer};
use ez_gui::radio_squelch::{CtcssSquelch, CtcssStatus};
use ez_gui::sdr_panel::DemodMode;
use std::f64::consts::TAU;

const SOURCE_RATE: u32 = 384_003;
const DECIMATION: u32 = 4;
const VFO_OFFSET: f64 = 18_000.0;

fn transmitted_nfm(seconds: f64, tone: impl Fn(f64) -> Option<f64>) -> Vec<u8> {
    let count = (seconds * f64::from(SOURCE_RATE)).round() as usize;
    let mut phase = 0.73;
    (0..count)
        .flat_map(|index| {
            let t = index as f64 / f64::from(SOURCE_RATE);
            // Voice-band multi-tone energy is much stronger than the CTCSS tone.
            let voice = 1_200.0 * (TAU * 1_170.0 * t).sin() + 480.0 * (TAU * 2_130.0 * t).cos();
            let subaudible = tone(t).map_or(0.0, |hz| 400.0 * (TAU * hz * t).sin());
            phase = (phase + TAU * (VFO_OFFSET + voice + subaudible) / f64::from(SOURCE_RATE))
                .rem_euclid(TAU);
            [0.65 * phase.cos() + 0.02, 0.65 * phase.sin() - 0.03]
                .map(|sample| (127.4 + 128.0 * sample).round().clamp(0.0, 255.0) as u8)
        })
        .collect()
}

struct Receiver {
    source: RadioIqProcessor,
    mixer: VfoMixer,
    demod: Demodulator,
    squelch: CtcssSquelch,
    audio_rate: u32,
}

struct Reception {
    audio: Vec<f32>,
    tap: Vec<f32>,
    gains: Vec<f32>,
    gated: Vec<f32>,
    status: CtcssStatus,
}

impl Receiver {
    fn new(audio_rate: u32, suppress_audio: bool) -> Self {
        let source = RadioIqProcessor::new(RadioIqConfig {
            input_rate: SOURCE_RATE,
            dc_remove: true,
            invert: false,
            decimation: DECIMATION,
        });
        let rate = source.output_rate();
        assert_eq!(rate, 96_000.75);
        let mut mixer = VfoMixer::new(rate);
        mixer.configure(rate, VFO_OFFSET);
        let mut demod = Demodulator::new();
        demod.set_sample_rates_exact(rate, audio_rate);
        demod.set_rf_bandwidth(12_500.0);
        demod.set_lpf_cutoff(4_000.0);
        demod.set_agc_enabled(false);
        demod.set_deemph_tau(0.0);
        if suppress_audio {
            demod.set_audio_hpf(600.0);
            demod.set_dc_blocker(1.0);
            demod.set_notch(100.0, 20.0);
            demod.set_deemph_tau(75.0);
            demod.set_pitch(0.5);
            demod.set_audio_gain(0.0);
        }
        Self {
            source,
            mixer,
            demod,
            squelch: CtcssSquelch::new(),
            audio_rate,
        }
    }

    fn receive(&mut self, iq: &[u8], chunk_size: usize, volume: f32) -> Reception {
        let mut output = Reception {
            audio: Vec::new(),
            tap: Vec::new(),
            gains: Vec::new(),
            gated: Vec::new(),
            status: self.squelch.status(),
        };
        for bytes in iq.chunks(chunk_size) {
            let corrected = self.source.process(bytes);
            let tuned = self.mixer.process(&corrected);
            // Reapplying the clock is how the app streams each source block.
            self.demod
                .set_sample_rates_exact(self.source.output_rate(), self.audio_rate);
            let audio = self.demod.demodulate_complex(&tuned, DemodMode::Fm);
            let tap = self.demod.take_nfm_subaudible_audio();
            let gains = self.squelch.process(&tap, self.audio_rate, Some(100.0));
            assert_eq!(audio.len(), tap.len(), "tap/audio per-block alignment");
            assert_eq!(audio.len(), gains.len(), "gate/audio per-block alignment");
            assert!(audio.iter().chain(&tap).all(|sample| sample.is_finite()));
            assert!(gains.iter().all(|gain| (0.0..=1.0).contains(gain)));
            output.gated.extend(
                audio
                    .iter()
                    .zip(&gains)
                    .map(|(sample, gain)| sample * gain * volume),
            );
            output.audio.extend(audio);
            output.tap.extend(tap);
            output.gains.extend(gains);
        }
        output.status = self.squelch.status();
        output
    }
}

fn assert_duration(reception: &Reception, iq: &[u8], audio_rate: u32) {
    let processed_pairs = (iq.len() / 2) / DECIMATION as usize;
    let expected = (processed_pairs as f64 / (f64::from(SOURCE_RATE) / f64::from(DECIMATION))
        * f64::from(audio_rate))
    .floor() as usize;
    // A live stream retains the final interpolation interval for its next block.
    assert!(
        reception.audio.len().abs_diff(expected) <= 1,
        "{} frames for {expected} expected at {audio_rate} Hz",
        reception.audio.len()
    );
}

#[test]
fn ctcss_reads_pre_highpass_pre_gain_tap_with_playback_muted() {
    let iq = transmitted_nfm(1.2, |_| Some(100.0));
    for rate in [44_100, 48_000] {
        let reference = Receiver::new(rate, false).receive(&iq, 4_093, 1.0);
        let muted = Receiver::new(rate, true).receive(&iq, 4_093, 0.0);
        assert_duration(&reference, &iq, rate);
        assert_duration(&muted, &iq, rate);
        assert_eq!(
            muted.tap, reference.tap,
            "audio controls changed the detector tap"
        );
        assert_eq!(muted.gains, reference.gains);
        assert!(muted.audio.iter().all(|sample| *sample == 0.0));
        assert!(muted.gated.iter().all(|sample| *sample == 0.0));
        assert!(reference.audio.iter().any(|sample| sample.abs() > 0.001));
        assert_eq!(muted.status.detected_tone_hz, Some(100.0));
        assert!(muted.status.gate_open, "{rate} Hz: {:?}", muted.status);
        assert!(muted.gains[rate as usize..].iter().all(|gain| *gain == 1.0));
    }
}

#[test]
fn selected_ctcss_rejects_adjacent_tone_and_voice_only_after_demodulation() {
    for tone in [Some(103.5), None] {
        let iq = transmitted_nfm(1.2, |_| tone);
        for rate in [44_100, 48_000] {
            let output = Receiver::new(rate, false).receive(&iq, 997, 1.0);
            assert_duration(&output, &iq, rate);
            assert_eq!(output.status.detected_tone_hz, tone.map(|hz| hz as f32));
            assert!(!output.status.gate_open, "{rate} Hz: {:?}", output.status);
            assert!(output.gains.iter().all(|gain| *gain == 0.0));
            assert!(output.gated.iter().all(|sample| *sample == 0.0));
            assert!(output.audio.iter().any(|sample| sample.abs() > 0.001));
        }
    }
}

#[test]
fn ctcss_gate_transitions_are_sample_aligned_and_chunk_independent() {
    let iq = transmitted_nfm(2.4, |t| (t < 1.1).then_some(100.0));
    for rate in [44_100, 48_000] {
        let small = Receiver::new(rate, false).receive(&iq, 997, 0.37);
        let large = Receiver::new(rate, false).receive(&iq, 32_767, 0.37);
        assert_duration(&small, &iq, rate);
        assert_eq!(small.tap, large.tap);
        assert_eq!(small.audio, large.audio);
        assert_eq!(small.gains, large.gains);
        assert_eq!(small.gated, large.gated);
        let opens = small.gains.iter().position(|gain| *gain > 0.0).unwrap();
        let closes = small.gains.iter().rposition(|gain| *gain > 0.0).unwrap();
        let open_seconds = opens as f64 / f64::from(rate);
        let close_seconds = closes as f64 / f64::from(rate);
        assert!(
            (0.49..0.85).contains(&open_seconds),
            "opens {open_seconds:.6}s"
        );
        assert!(
            (1.1..1.9).contains(&close_seconds),
            "closes {close_seconds:.6}s"
        );
        assert!(small.gated[..opens].iter().all(|sample| *sample == 0.0));
        assert!(small.gated[closes + 1..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(small.gated[rate as usize * 9 / 10..rate as usize]
            .iter()
            .any(|sample| sample.abs() > 0.001));
        assert!(!small.status.gate_open);
        assert_eq!(small.status.detected_tone_hz, None);
        println!("CTCSS {rate} Hz: {} frames; gate opens {open_seconds:.6}s, closes {close_seconds:.6}s; odd chunk sizes give identical samples", small.audio.len());
    }
}
