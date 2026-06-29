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
        }
    }

    pub fn set_lpf_cutoff(&mut self, cutoff_hz: f32) {
        self.lpf_cutoff = cutoff_hz.max(100.0).min(20000.0);
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
        const ATTACK: f32 = 0.01;   // fast attack (gain drops quickly on loud signal)
        const DECAY: f32 = 0.0001;  // slow decay (gain rises slowly when quiet)
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
        if self.lpf_alpha >= 0.999 { return samples; }
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
            if chunk.len() < 2 { break; }
            let i = (chunk[0] as f32 - 127.4) / 128.0;
            let q = (chunk[1] as f32 - 127.4) / 128.0;
            out.push(i * 0.3);
            out.push(q * 0.3);
        }
        out
    }

    fn demod_am(&mut self, iq: &[u8]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len() / 2 / self.decimation.max(1));
        for (idx, chunk) in iq.chunks(2).enumerate() {
            if chunk.len() < 2 { break; }
            let i = (chunk[0] as f32 - 127.4) / 128.0;
            let q = (chunk[1] as f32 - 127.4) / 128.0;
            let env = (i * i + q * q).sqrt();
            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(env);
            }
            let _ = idx;
        }
        out
    }

    fn demod_fm(&mut self, iq: &[u8]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len() / 2 / self.decimation.max(1));
        let mut max_diff: f32 = 0.0;
        for chunk in iq.chunks(2) {
            if chunk.len() < 2 { break; }
            let i = (chunk[0] as f32 - 127.4) / 128.0;
            let q = (chunk[1] as f32 - 127.4) / 128.0;

            let phase = q.atan2(i);
            let mut diff = phase - self.prev_phase;

            // Phase unwrap
            while diff > std::f32::consts::PI {
                diff -= 2.0 * std::f32::consts::PI;
            }
            while diff < -std::f32::consts::PI {
                diff += 2.0 * std::f32::consts::PI;
            }

            if diff.abs() > max_diff { max_diff = diff.abs(); }

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
            self.last_fm_deviation_hz = max_diff * self.input_rate as f32 / (2.0 * std::f32::consts::PI);
        }
        // Track audio peak
        if let Some(&p) = out.iter().max_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap_or(std::cmp::Ordering::Equal)) {
            self.last_audio_peak = 0.9 * self.last_audio_peak + 0.1 * p.abs();
        }
        out
    }

    fn demod_wfm(&mut self, iq: &[u8]) -> Vec<f32> {
        // Wide FM: same as FM but wider de-emphasis
        let mut out = Vec::with_capacity(iq.len() / 2 / self.decimation.max(1));
        let alpha = 1.0 / (1.0 + self.audio_sample_rate as f32 / (2.0 * std::f32::consts::PI * 50.0));
        let mut deemph_state = 0.0f32;

        for chunk in iq.chunks(2) {
            if chunk.len() < 2 { break; }
            let i = (chunk[0] as f32 - 127.4) / 128.0;
            let q = (chunk[1] as f32 - 127.4) / 128.0;

            let phase = q.atan2(i);
            let mut diff = phase - self.prev_phase;
            while diff > std::f32::consts::PI { diff -= 2.0 * std::f32::consts::PI; }
            while diff < -std::f32::consts::PI { diff += 2.0 * std::f32::consts::PI; }

            self.prev_phase = phase;

            // De-emphasis
            deemph_state = deemph_state * (1.0 - alpha) + diff * alpha;

            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(deemph_state * 0.4);
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

        for (n, chunk) in iq.chunks(2).enumerate() {
            if chunk.len() < 2 { break; }
            let i = (chunk[0] as f32 - 127.4) / 128.0;
            let q = (chunk[1] as f32 - 127.4) / 128.0;

            let angle = sign * shift_rad * n as f32;
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

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.prev_i = 0.0;
        self.prev_q = 0.0;
        self.prev_phase = 0.0;
        self.decim_counter = 0;
    }
}
