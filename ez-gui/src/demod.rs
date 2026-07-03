use crate::sdr_panel::DemodMode;

pub struct Demodulator {
    prev_i: f32,
    prev_q: f32,
    prev_phase: f32,
    decimation: usize,
    decim_counter: usize,
    audio_sample_rate: u32,
    lpf_cutoff: f32,
    lpf_state_l: f32,
    lpf_alpha: f32,
    pub last_fm_deviation_hz: f32,
    pub last_audio_peak: f32,
    input_rate: u32,
    // AGC state
    agc_gain: f32,
    pub agc_enabled: bool,
    // Persistent DSP state across demod chunks (avoids per-call transients).
    wfm_deemph_state: f32,
    ssb_osc_phase: f32,
}

impl Demodulator {
    pub fn new() -> Self {
        Self {
            prev_i: 0.0,
            prev_q: 0.0,
            prev_phase: 0.0,
            decimation: 1,
            decim_counter: 0,
            audio_sample_rate: 48000,
            lpf_cutoff: 15000.0,
            lpf_state_l: 0.0,
            lpf_alpha: 1.0,
            last_fm_deviation_hz: 0.0,
            last_audio_peak: 0.0,
            input_rate: 2_048_000,
            agc_gain: 1.0,
            agc_enabled: true,
            wfm_deemph_state: 0.0,
            ssb_osc_phase: 0.0,
        }
    }

    pub fn set_lpf_cutoff(&mut self, cutoff_hz: f32) {
        self.lpf_cutoff = cutoff_hz.clamp(100.0, 20000.0);
        let rc = 1.0 / (2.0 * std::f32::consts::PI * self.lpf_cutoff);
        let dt = 1.0 / self.audio_sample_rate as f32;
        self.lpf_alpha = (dt / (rc + dt)).clamp(0.001, 1.0);
    }

    pub fn set_sample_rates(&mut self, input_rate: u32, audio_rate: u32) {
        self.audio_sample_rate = audio_rate;
        self.input_rate = input_rate;
        self.decimation = (input_rate / audio_rate).max(1) as usize;
        if self.decimation < 1 {
            self.decimation = 1;
        }
    }

    pub fn demodulate(&mut self, iq: &[u8], mode: DemodMode) -> Vec<f32> {
        let samples = match mode {
            DemodMode::Raw => self.demod_raw(iq),
            DemodMode::Am => self.demod_am(iq),
            DemodMode::Fm => self.demod_fm(iq),
            DemodMode::Wfm => self.demod_wfm(iq),
            DemodMode::Lsb => self.demod_ssb(iq, false),
            DemodMode::Usb => self.demod_ssb(iq, true),
        };
        let filtered = self.apply_lpf(samples);
        if self.agc_enabled {
            self.apply_agc(filtered)
        } else {
            filtered
        }
    }

    fn apply_agc(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        // Soft-knee AGC: target RMS ~0.25, attack fast, decay slow
        const TARGET: f32 = 0.25;
        const ATTACK: f32 = 0.01; // fast attack (gain drops quickly on loud signal)
        const DECAY: f32 = 0.0001; // slow decay (gain rises slowly when quiet)
        const MAX_GAIN: f32 = 40.0;
        const MIN_GAIN: f32 = 0.1;

        for s in &mut samples {
            let out = *s * self.agc_gain;
            let abs = out.abs();
            if abs > TARGET {
                self.agc_gain *= 1.0 - ATTACK * (abs / TARGET - 1.0).min(1.0);
            } else {
                self.agc_gain *= 1.0 + DECAY;
            }
            self.agc_gain = self.agc_gain.clamp(MIN_GAIN, MAX_GAIN);
            *s = out.clamp(-1.0, 1.0);
        }
        samples
    }

    fn apply_lpf(&mut self, samples: Vec<f32>) -> Vec<f32> {
        if self.lpf_alpha >= 0.999 {
            return samples;
        }
        let mut out = Vec::with_capacity(samples.len());
        for s in samples {
            self.lpf_state_l = self.lpf_state_l + self.lpf_alpha * (s - self.lpf_state_l);
            out.push(self.lpf_state_l);
        }
        out
    }

    fn demod_raw(&mut self, iq: &[u8]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len() / 2);
        for chunk in iq.chunks(2) {
            if chunk.len() < 2 {
                break;
            }
            let i = (f32::from(chunk[0]) - 127.4) / 128.0;
            let q = (f32::from(chunk[1]) - 127.4) / 128.0;
            out.push(i * 0.3);
            out.push(q * 0.3);
        }
        out
    }

    fn demod_am(&mut self, iq: &[u8]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len() / 2 / self.decimation.max(1));
        for chunk in iq.chunks(2) {
            if chunk.len() < 2 {
                break;
            }
            let i = (f32::from(chunk[0]) - 127.4) / 128.0;
            let q = (f32::from(chunk[1]) - 127.4) / 128.0;
            let env = (i * i + q * q).sqrt();
            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(env);
            }
        }
        out
    }

    fn demod_fm(&mut self, iq: &[u8]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len() / 2 / self.decimation.max(1));
        let mut max_diff: f32 = 0.0;
        for chunk in iq.chunks(2) {
            if chunk.len() < 2 {
                break;
            }
            let i = (f32::from(chunk[0]) - 127.4) / 128.0;
            let q = (f32::from(chunk[1]) - 127.4) / 128.0;

            let phase = q.atan2(i);
            let mut diff = phase - self.prev_phase;

            // Phase unwrap
            while diff > std::f32::consts::PI {
                diff -= 2.0 * std::f32::consts::PI;
            }
            while diff < -std::f32::consts::PI {
                diff += 2.0 * std::f32::consts::PI;
            }

            if diff.abs() > max_diff {
                max_diff = diff.abs();
            }

            self.prev_i = i;
            self.prev_q = q;
            self.prev_phase = phase;

            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(diff * 0.5);
            }
        }
        // FM deviation = max_phase_diff * sample_rate / (2π)
        if self.input_rate > 0 {
            self.last_fm_deviation_hz =
                max_diff * self.input_rate as f32 / (2.0 * std::f32::consts::PI);
        }
        // Track audio peak
        if let Some(&p) = out.iter().max_by(|a, b| {
            a.abs()
                .partial_cmp(&b.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            self.last_audio_peak = 0.9 * self.last_audio_peak + 0.1 * p.abs();
        }
        out
    }

    fn demod_wfm(&mut self, iq: &[u8]) -> Vec<f32> {
        // Wide FM demodulation with correct 50 μs de-emphasis (EU standard).
        // De-emphasis time constant τ = 50 μs → pole at f_c = 1/(2π·τ) ≈ 3183 Hz.
        // Discrete IIR: alpha = dt/(τ + dt) where dt = 1/sample_rate
        let tau = 50.0e-6_f32; // 50 microseconds
                               // The de-emphasis IIR advances once per input IQ pair, so dt must use
                               // the input sample rate (≈2.048 MHz), NOT the audio rate. Using the
                               // audio rate here (≈48 kHz) made alpha ~0.294 instead of the correct
                               // ≈1.6e-5, effectively disabling the 50 µs pole and leaving WFM audio
                               // harsh with no bass restoration.
        let dt = 1.0 / self.input_rate as f32;
        let alpha = dt / (tau + dt);
        let mut out = Vec::with_capacity(iq.len() / 2 / self.decimation.max(1));

        for chunk in iq.chunks(2) {
            if chunk.len() < 2 {
                break;
            }
            let i = (f32::from(chunk[0]) - 127.4) / 128.0;
            let q = (f32::from(chunk[1]) - 127.4) / 128.0;

            let phase = q.atan2(i);
            let mut diff = phase - self.prev_phase;
            while diff > std::f32::consts::PI {
                diff -= 2.0 * std::f32::consts::PI;
            }
            while diff < -std::f32::consts::PI {
                diff += 2.0 * std::f32::consts::PI;
            }

            self.prev_phase = phase;

            // 1st-order IIR low-pass de-emphasis: y[n] = y[n-1] + α*(x[n] - y[n-1]).
            // State persists across chunks (field) to avoid a settling transient
            // at every source-buffer boundary.
            self.wfm_deemph_state += alpha * (diff - self.wfm_deemph_state);

            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(self.wfm_deemph_state * 0.8);
            }
        }
        out
    }

    fn demod_ssb(&mut self, iq: &[u8], usb: bool) -> Vec<f32> {
        // Weaver SSB: shift by filter BW/2, then AM detect
        let mut out = Vec::with_capacity(iq.len() / 2 / self.decimation.max(1));
        let shift_hz: f32 = 1500.0;
        let shift_rad = 2.0 * std::f32::consts::PI * shift_hz / self.audio_sample_rate as f32;
        let sign = if usb { 1.0 } else { -1.0 };

        for chunk in iq.chunks(2) {
            if chunk.len() < 2 {
                break;
            }
            let i = (f32::from(chunk[0]) - 127.4) / 128.0;
            let q = (f32::from(chunk[1]) - 127.4) / 128.0;

            // Weaver oscillator phase persists across chunks; advancing it per
            // sample (modulo 2π) avoids a phase reset at every source-buffer
            // boundary which would inject a click into LSB/USB audio.
            let angle = sign * self.ssb_osc_phase;
            self.ssb_osc_phase += shift_rad;
            if self.ssb_osc_phase >= 2.0 * std::f32::consts::PI {
                self.ssb_osc_phase -= 2.0 * std::f32::consts::PI;
            }
            let i_shift = i * angle.cos() - q * angle.sin();
            let q_shift = i * angle.sin() + q * angle.cos();

            // Low-pass filter approximation
            let bp_i = i_shift - self.prev_i;
            let bp_q = q_shift - self.prev_q;
            self.prev_i = i_shift;
            self.prev_q = q_shift;

            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push((bp_i * bp_i + bp_q * bp_q).sqrt() * 0.5);
            }
        }
        out
    }

    pub fn reset(&mut self) {
        self.prev_i = 0.0;
        self.prev_q = 0.0;
        self.prev_phase = 0.0;
        self.decim_counter = 0;
        self.lpf_state_l = 0.0;
        self.wfm_deemph_state = 0.0;
        self.ssb_osc_phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdr_panel::DemodMode;

    fn make_iq_dc(i_val: u8, q_val: u8, count: usize) -> Vec<u8> {
        // Constant IQ = same byte pair repeated
        let mut v = Vec::with_capacity(count * 2);
        for _ in 0..count {
            v.push(i_val);
            v.push(q_val);
        }
        v
    }

    #[test]
    fn demod_raw_output_length() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        let iq = make_iq_dc(127, 127, 64);
        let out = d.demodulate(&iq, DemodMode::Raw);
        // raw produces 2 samples per IQ pair
        assert_eq!(out.len(), 128);
    }

    #[test]
    fn demod_am_dc_signal_envelope() {
        // IQ = (1, 0) normalised → envelope should be ~1.0 before AGC
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        d.decimation = 1;
        // i=255 → (255-127.4)/128 ≈ 1.0, q=127 → ~0
        let iq = make_iq_dc(255, 127, 256);
        let out = d.demodulate(&iq, DemodMode::Am);
        assert!(!out.is_empty());
        let mean: f32 = out.iter().sum::<f32>() / out.len() as f32;
        assert!(
            mean > 0.5,
            "AM envelope of near-1.0 IQ should be > 0.5, got {mean}"
        );
    }

    #[test]
    fn demod_fm_constant_phase_near_zero() {
        // Constant IQ → constant phase → phase diff ≈ 0
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        d.decimation = 1;
        let iq = make_iq_dc(200, 150, 512);
        let out = d.demodulate(&iq, DemodMode::Fm);
        assert!(!out.is_empty());
        // After the first sample sets prev_phase, all subsequent diffs should be ~0
        let tail_mean: f32 = out[1..].iter().map(|x| x.abs()).sum::<f32>() / (out.len() - 1) as f32;
        assert!(
            tail_mean < 0.01,
            "FM constant-phase output should be ~0, got {tail_mean}"
        );
    }

    #[test]
    fn set_lpf_cutoff_clamps() {
        let mut d = Demodulator::new();
        d.set_lpf_cutoff(0.0);
        assert_eq!(d.lpf_cutoff, 100.0);
        d.set_lpf_cutoff(999_999.0);
        assert_eq!(d.lpf_cutoff, 20000.0);
        d.set_lpf_cutoff(5000.0);
        assert_eq!(d.lpf_cutoff, 5000.0);
    }

    #[test]
    fn set_sample_rates_decimation() {
        let mut d = Demodulator::new();
        d.set_sample_rates(2_048_000, 48_000);
        // 2048000 / 48000 = 42
        assert_eq!(d.decimation, 42);
        // audio_rate > input_rate should clamp to 1
        d.set_sample_rates(48_000, 96_000);
        assert_eq!(d.decimation, 1);
    }

    #[test]
    fn demod_wfm_produces_output() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        d.decimation = 1;
        let iq = make_iq_dc(180, 100, 256);
        let out = d.demodulate(&iq, DemodMode::Wfm);
        assert!(!out.is_empty());
        // De-emphasis settles — output should be finite and bounded
        for &s in &out {
            assert!(s.is_finite(), "WFM output must be finite");
            assert!(s.abs() <= 1.5, "WFM output unexpectedly large: {s}");
        }
    }

    /// Regression: de-emphasis `dt` must use `input_rate`, not `audio_sample_rate`.
    /// With input_rate=2_048_000 the correct alpha ≈ 1.6e-5, so after one 256-sample
    /// chunk the de-emphasis state has barely advanced (~0.4% of the step). The
    /// prior bug used audio_sample_rate=48_000 → alpha≈0.294, fully settling the
    /// filter inside a single chunk. A small per-chunk gain catches that regression
    /// without depending on exact sample values.
    #[test]
    fn demod_wfm_deemph_runs_at_input_rate() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        d.decimation = 1;
        d.set_sample_rates(2_048_000, 48_000);
        let iq = make_iq_dc(180, 100, 256);
        let out = d.demodulate(&iq, DemodMode::Wfm);
        assert!(!out.is_empty());
        let peak = out.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        // Correct alpha → after 256 samples the de-emphasis state reaches
        // `1 - (1 - alpha)^256` of the step. With alpha≈1.6e-5 that's ≈0.004.
        // With the broken alpha≈0.294 the state would reach ≈1.0 (settled) and
        // the peak (after the *0.8 scale in demod_wfm) would approach ~0.8.
        // A 0.05 cap cleanly separates the two regimes.
        assert!(
            peak < 0.05,
            "de-emphasis advanced too fast; expected slow attack at \
             input_rate (alpha≈1.6e-5) but got peak={peak} (likely using \
             audio_sample_rate → alpha≈0.294)",
        );
    }

    #[test]
    fn demod_lsb_produces_finite_output() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        d.decimation = 1;
        let iq = make_iq_dc(180, 100, 256);
        let out = d.demodulate(&iq, DemodMode::Lsb);
        assert!(!out.is_empty());
        for &s in &out {
            assert!(s.is_finite());
        }
    }

    #[test]
    fn demod_usb_produces_finite_output() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        d.decimation = 1;
        let iq = make_iq_dc(180, 100, 256);
        let out = d.demodulate(&iq, DemodMode::Usb);
        assert!(!out.is_empty());
        for &s in &out {
            assert!(s.is_finite());
        }
    }

    #[test]
    fn reset_clears_state() {
        let mut d = Demodulator::new();
        d.prev_i = 0.5;
        d.prev_q = -0.3;
        d.prev_phase = 1.2;
        d.decim_counter = 42;
        d.reset();
        assert_eq!(d.prev_i, 0.0);
        assert_eq!(d.prev_q, 0.0);
        assert_eq!(d.prev_phase, 0.0);
        assert_eq!(d.decim_counter, 0);
    }

    #[test]
    fn apply_lpf_bypass_when_alpha_near_one() {
        let mut d = Demodulator::new();
        d.lpf_alpha = 0.999;
        let samples = vec![0.5, -0.3, 0.8, -0.1];
        let out = d.apply_lpf(samples.clone());
        assert_eq!(out, samples);
    }

    #[test]
    fn apply_lpf_filters_when_alpha_small() {
        let mut d = Demodulator::new();
        d.lpf_alpha = 0.1;
        let samples = vec![1.0, 0.0, 0.0, 0.0];
        let out = d.apply_lpf(samples);
        assert_eq!(out.len(), 4);
        // First sample: state = 0 + 0.1 * (1.0 - 0) = 0.1
        assert!((out[0] - 0.1).abs() < 1e-6);
        // Second sample: state = 0.1 + 0.1 * (0.0 - 0.1) = 0.09
        assert!((out[1] - 0.09).abs() < 1e-6);
    }

    #[test]
    fn agc_clamps_gain_and_output() {
        let mut d = Demodulator::new();
        d.agc_enabled = false; // We'll call apply_agc directly
        d.agc_gain = 50.0; // above MAX_GAIN = 40.0
        let samples = vec![0.5, -0.5];
        let out = d.apply_agc(samples);
        // Gain is clamped to 40 after each sample, then attack pulls it below 40
        assert!(d.agc_gain <= 40.0);
        assert!(d.agc_gain >= 0.1);
        for &s in &out {
            assert!(s.abs() <= 1.0);
        }
    }

    #[test]
    fn agc_attack_on_loud_signal() {
        let mut d = Demodulator::new();
        d.agc_gain = 1.0;
        // Loud signal (out = 1.0 * 1.0 = 1.0 > 0.25 target) → gain should drop
        let samples = vec![1.0];
        d.apply_agc(samples);
        assert!(d.agc_gain < 1.0);
    }

    #[test]
    fn agc_decay_on_quiet_signal() {
        let mut d = Demodulator::new();
        d.agc_gain = 0.5;
        // Quiet signal (out = 0.0 * 0.5 = 0.0 < 0.25 target) → gain should rise
        let samples = vec![0.0];
        d.apply_agc(samples);
        assert!(d.agc_gain > 0.5);
    }

    #[test]
    fn set_lpf_cutoff_computes_alpha() {
        let mut d = Demodulator::new();
        d.set_lpf_cutoff(10000.0);
        assert!(d.lpf_alpha > 0.0);
        assert!(d.lpf_alpha <= 1.0);
    }

    #[test]
    fn empty_iq_returns_empty() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        for mode in &[
            DemodMode::Raw,
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Lsb,
            DemodMode::Usb,
        ] {
            let out = d.demodulate(&[], *mode);
            assert!(
                out.is_empty(),
                "mode {mode:?} with empty IQ should return empty"
            );
        }
    }
}
