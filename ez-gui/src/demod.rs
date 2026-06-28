use crate::sdr_panel::DemodMode;

pub struct Demodulator {
    prev_i: f32,
    prev_q: f32,
    prev_phase: f32,
    decimation: usize,
    decim_counter: usize,
    audio_sample_rate: u32,
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
        }
    }

    pub fn set_sample_rates(&mut self, input_rate: u32, audio_rate: u32) {
        self.audio_sample_rate = audio_rate;
        self.decimation = (input_rate / audio_rate).max(1) as usize;
        if self.decimation < 1 {
            self.decimation = 1;
        }
    }

    pub fn demodulate(&mut self, iq: &[u8], mode: DemodMode) -> Vec<f32> {
        match mode {
            DemodMode::Raw => self.demod_raw(iq),
            DemodMode::Am => self.demod_am(iq),
            DemodMode::Fm => self.demod_fm(iq),
            DemodMode::Wfm => self.demod_wfm(iq),
            DemodMode::Lsb => self.demod_ssb(iq, false),
            DemodMode::Usb => self.demod_ssb(iq, true),
        }
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

            self.prev_i = i;
            self.prev_q = q;
            self.prev_phase = phase;

            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(diff * 0.5);
            }
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

    pub fn reset(&mut self) {
        self.prev_i = 0.0;
        self.prev_q = 0.0;
        self.prev_phase = 0.0;
        self.decim_counter = 0;
    }
}
