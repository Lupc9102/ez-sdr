use crate::sdr_panel::DemodMode;

/// 2nd-order IIR biquad (RBJ cookbook) used for audio notch + bass/treble shelves.
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Configure as a notch at `f0` Hz with bandwidth `bw` Hz.
    fn set_notch(&mut self, f0: f32, bw: f32, fs: f32) {
        let w0 = 2.0 * std::f32::consts::PI * f0 / fs;
        let q = if bw > 0.0 { (f0 / bw).max(0.1) } else { 0.1 };
        let alpha = w0.sin() / (2.0 * q);
        let cw = w0.cos();
        let a0 = 1.0 + alpha;
        self.b0 = 1.0 / a0;
        self.b1 = (-2.0 * cw) / a0;
        self.b2 = 1.0 / a0;
        self.a1 = (-2.0 * cw) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    /// Configure as a low/high shelf of `gain_db` at `fc` Hz.
    fn set_shelf(&mut self, fc: f32, gain_db: f32, fs: f32, high: bool) {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * fc / fs;
        let alpha = w0.sin() / 2.0; // S = 1
        let cw = w0.cos();
        let sqa = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = if !high {
            (
                a * ((a + 1.0) - (a - 1.0) * cw + sqa),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
                a * ((a + 1.0) - (a - 1.0) * cw - sqa),
                (a + 1.0) + (a - 1.0) * cw + sqa,
                -2.0 * ((a - 1.0) + (a + 1.0) * cw),
                (a + 1.0) + (a - 1.0) * cw - sqa,
            )
        } else {
            (
                a * ((a + 1.0) + (a - 1.0) * cw + sqa),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
                a * ((a + 1.0) + (a - 1.0) * cw - sqa),
                (a + 1.0) - (a - 1.0) * cw + sqa,
                2.0 * ((a - 1.0) - (a + 1.0) * cw),
                (a + 1.0) - (a - 1.0) * cw - sqa,
            )
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

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
    agc_target: f32,
    agc_attack: f32,
    agc_decay: f32,
    // Persistent DSP state across demod chunks (avoids per-call transients).
    wfm_deemph_state: f32,
    ssb_osc_phase: f32,
    // ---- Advanced audio DSP ----
    /// Audio high-pass cutoff (Hz); 0 = disabled.
    audio_hpf_hz: f32,
    hpf_state: f32,
    hpf_x_prev: f32,
    hpf_alpha_computed: f32,
    /// DC blocker blend 0..1.
    dc_blocker: f32,
    dc_block_state: f32,
    dc_block_x_prev: f32,
    /// FM de-emphasis time constant (microseconds).
    deemph_tau_us: f32,
    /// Extra audio gain multiplier.
    audio_gain: f32,
    /// Audio notch centre (Hz); 0 = disabled.
    notch_hz: f32,
    notch_width_hz: f32,
    notch: Biquad,
    /// Bass/treble shelf gains (dB).
    bass_db: f32,
    treble_db: f32,
    bass: Biquad,
    treble: Biquad,
    /// Noise blanker strength 0..1.
    noise_blanker: f32,
    nb_avg: f32,
    /// Pitch shift in octaves.
    pitch_octaves: f32,
    pitch_pos: f32,
    // ---- Advanced RF IQ pre-stage ----
    rf_dc_remove: bool,
    rf_noise_blanker: bool,
    rf_notch: bool,
    rf_notch_hz: f32,
    rf_decim: u32,
    rf_nb_avg: f32,
    rf_notch_b: (f32, f32, f32, f32, f32),
    rf_notch_x1: (f32, f32),
    rf_notch_x2: (f32, f32),
    rf_notch_y1: (f32, f32),
    rf_notch_y2: (f32, f32),
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
            agc_target: 0.25,
            agc_attack: 0.01,
            agc_decay: 0.0001,
            wfm_deemph_state: 0.0,
            ssb_osc_phase: 0.0,
            audio_hpf_hz: 0.0,
            hpf_state: 0.0,
            hpf_x_prev: 0.0,
            hpf_alpha_computed: 0.0,
            dc_blocker: 0.0,
            dc_block_state: 0.0,
            dc_block_x_prev: 0.0,
            deemph_tau_us: 50.0,
            audio_gain: 1.0,
            notch_hz: 0.0,
            notch_width_hz: 100.0,
            notch: Biquad::new(),
            bass_db: 0.0,
            treble_db: 0.0,
            bass: Biquad::new(),
            treble: Biquad::new(),
            noise_blanker: 0.0,
            nb_avg: 0.0,
            pitch_octaves: 0.0,
            pitch_pos: 0.0,
            rf_dc_remove: false,
            rf_noise_blanker: false,
            rf_notch: false,
            rf_notch_hz: 10_000.0,
            rf_decim: 1,
            rf_nb_avg: 0.0,
            rf_notch_b: (1.0, 0.0, 0.0, 0.0, 0.0),
            rf_notch_x1: (0.0, 0.0),
            rf_notch_x2: (0.0, 0.0),
            rf_notch_y1: (0.0, 0.0),
            rf_notch_y2: (0.0, 0.0),
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
        self.recompute_audio_filters();
    }

    /// Recompute derived filter coefficients from the current user params.
    /// Call after changing sample rate or any filter setting.
    pub fn recompute_audio_filters(&mut self) {
        let fs = self.audio_sample_rate as f32;
        if self.audio_hpf_hz > 0.0 {
            let rc = 1.0 / (2.0 * std::f32::consts::PI * self.audio_hpf_hz);
            let dt = 1.0 / fs;
            self.hpf_alpha_computed = (rc / (rc + dt)).clamp(0.001, 0.999);
        } else {
            self.hpf_alpha_computed = 0.0; // disabled
        }
        if self.notch_hz > 0.0 {
            self.notch.set_notch(self.notch_hz, self.notch_width_hz, fs);
        }
        if self.bass_db.abs() > 0.01 {
            self.bass.set_shelf(120.0, self.bass_db, fs, false);
        } else {
            self.bass = Biquad::new();
        }
        if self.treble_db.abs() > 0.01 {
            self.treble.set_shelf(4000.0, self.treble_db, fs, true);
        } else {
            self.treble = Biquad::new();
        }
        self.recompute_rf_notch();
    }

    fn recompute_rf_notch(&mut self) {
        if !self.rf_notch || self.rf_notch_hz <= 0.0 {
            self.rf_notch_b = (1.0, 0.0, 0.0, 0.0, 0.0);
            return;
        }
        let fs = self.input_rate as f32;
        let w0 = 2.0 * std::f32::consts::PI * self.rf_notch_hz / fs;
        let q = 10.0;
        let alpha = w0.sin() / (2.0 * q);
        let cw = w0.cos();
        let a0 = 1.0 + alpha;
        self.rf_notch_b = (
            1.0 / a0,
            (-2.0 * cw) / a0,
            1.0 / a0,
            (-2.0 * cw) / a0,
            (1.0 - alpha) / a0,
        );
    }

    // ---------------- Setters (driven by the Advanced panel) ----------------

    pub fn set_audio_hpf(&mut self, hz: f32) {
        self.audio_hpf_hz = hz.max(0.0);
        self.recompute_audio_filters();
    }
    pub fn set_dc_blocker(&mut self, amt: f32) {
        self.dc_blocker = amt.clamp(0.0, 1.0);
    }
    pub fn set_agc_enabled(&mut self, on: bool) {
        self.agc_enabled = on;
    }
    pub fn set_agc_target(&mut self, v: f32) {
        self.agc_target = v.clamp(0.01, 1.0);
    }
    pub fn set_agc_attack(&mut self, v: f32) {
        self.agc_attack = v.clamp(0.0001, 1.0);
    }
    pub fn set_agc_decay(&mut self, v: f32) {
        self.agc_decay = v.clamp(0.00001, 1.0);
    }
    pub fn set_deemph_tau(&mut self, us: f32) {
        self.deemph_tau_us = us.clamp(1.0, 200.0);
    }
    pub fn set_audio_gain(&mut self, g: f32) {
        self.audio_gain = g.clamp(0.0, 10.0);
    }
    pub fn set_notch(&mut self, hz: f32, width: f32) {
        self.notch_hz = hz.max(0.0);
        self.notch_width_hz = width.max(1.0);
        self.recompute_audio_filters();
    }
    pub fn set_bass(&mut self, db: f32) {
        self.bass_db = db.clamp(-24.0, 24.0);
        self.recompute_audio_filters();
    }
    pub fn set_treble(&mut self, db: f32) {
        self.treble_db = db.clamp(-24.0, 24.0);
        self.recompute_audio_filters();
    }
    pub fn set_noise_blanker(&mut self, v: f32) {
        self.noise_blanker = v.clamp(0.0, 1.0);
    }
    pub fn set_pitch(&mut self, oct: f32) {
        self.pitch_octaves = oct.clamp(-2.0, 2.0);
        self.pitch_pos = 0.0;
    }
    pub fn set_rf_dc_remove(&mut self, on: bool) {
        self.rf_dc_remove = on;
    }
    pub fn set_rf_noise_blanker(&mut self, on: bool) {
        self.rf_noise_blanker = on;
    }
    pub fn set_rf_notch(&mut self, on: bool, hz: f32) {
        self.rf_notch = on;
        self.rf_notch_hz = hz.max(0.0);
        self.recompute_rf_notch();
    }
    pub fn set_rf_decim(&mut self, factor: u32) {
        self.rf_decim = factor.max(1);
    }

    // ---------------- RF IQ pre-stage ----------------

    fn iq_preprocess(&mut self, iq: &[u8]) -> Vec<u8> {
        if !self.rf_dc_remove && !self.rf_noise_blanker && !self.rf_notch && self.rf_decim <= 1 {
            return iq.to_vec();
        }
        let mut v: Vec<f32> = Vec::with_capacity(iq.len());
        for c in iq.chunks(2) {
            if c.len() < 2 {
                break;
            }
            v.push((f32::from(c[0]) - 127.4) / 128.0);
            v.push((f32::from(c[1]) - 127.4) / 128.0);
        }
        let n = v.len() / 2;

        if self.rf_dc_remove && n > 0 {
            let mut mi = 0.0;
            let mut mq = 0.0;
            for k in 0..n {
                mi += v[2 * k];
                mq += v[2 * k + 1];
            }
            mi /= n as f32;
            mq /= n as f32;
            for k in 0..n {
                v[2 * k] -= mi;
                v[2 * k + 1] -= mq;
            }
        }

        if self.rf_noise_blanker && n > 0 {
            let mut avg = self.rf_nb_avg;
            for k in 0..n {
                let m = (v[2 * k].abs() + v[2 * k + 1].abs()) * 0.5;
                avg = 0.95 * avg + 0.05 * m;
            }
            self.rf_nb_avg = avg;
            let thr = avg * 8.0 + 1e-6;
            for k in 0..n {
                let mag = (v[2 * k].powi(2) + v[2 * k + 1].powi(2)).sqrt();
                if mag > thr {
                    let s = thr / (mag + 1e-9);
                    v[2 * k] *= s;
                    v[2 * k + 1] *= s;
                }
            }
        }

        if self.rf_notch && self.rf_notch_b.0 > 0.0 {
            let (b0, b1, b2, a1, a2) = self.rf_notch_b;
            let (mut x1, mut x2) = self.rf_notch_x1;
            let (mut y1, mut y2) = self.rf_notch_y1;
            let (mut x1q, mut x2q) = self.rf_notch_x2;
            let (mut y1q, mut y2q) = self.rf_notch_y2;
            for k in 0..n {
                let xi = v[2 * k];
                let yi = b0 * xi + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                x2 = x1;
                x1 = xi;
                y2 = y1;
                y1 = yi;
                let xq = v[2 * k + 1];
                let yq = b0 * xq + b1 * x1q + b2 * x2q - a1 * y1q - a2 * y2q;
                x2q = x1q;
                x1q = xq;
                y2q = y1q;
                y1q = yq;
                v[2 * k] = yi;
                v[2 * k + 1] = yq;
            }
            self.rf_notch_x1 = (x1, x1q);
            self.rf_notch_x2 = (x2, x2q);
            self.rf_notch_y1 = (y1, y1q);
            self.rf_notch_y2 = (y2, y2q);
        }

        // RF decimation: keep every `rf_decim`-th IQ pair.
        if self.rf_decim > 1 {
            let step = self.rf_decim as usize;
            let mut out = Vec::with_capacity(v.len() / step + 2);
            let mut idx = 0;
            while idx + 1 < v.len() {
                out.push(v[idx]);
                out.push(v[idx + 1]);
                idx += 2 * step;
            }
            v = out;
        }

        // Convert back to u8 IQ bytes.
        let mut out = Vec::with_capacity(v.len());
        for &s in &v {
            out.push((s * 128.0 + 127.4).clamp(0.0, 255.0) as u8);
        }
        out
    }

    // ---------------- Audio post-chain ----------------

    fn post_process(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        if self.hpf_alpha_computed > 0.0 {
            let mut out = Vec::with_capacity(samples.len());
            for s in samples {
                let y = self.hpf_alpha_computed * (self.hpf_state + s - self.hpf_x_prev);
                self.hpf_x_prev = s;
                self.hpf_state = y;
                out.push(y);
            }
            samples = out;
        }

        if self.dc_blocker > 0.0 {
            let mut out = Vec::with_capacity(samples.len());
            for s in samples {
                let hp = s - self.dc_block_x_prev + 0.995 * self.dc_block_state;
                self.dc_block_x_prev = s;
                self.dc_block_state = hp;
                out.push(s * (1.0 - self.dc_blocker) + hp * self.dc_blocker);
            }
            samples = out;
        }

        if self.notch_hz > 0.0 {
            for s in &mut samples {
                *s = self.notch.process(*s);
            }
        }

        if self.bass_db.abs() > 0.01 {
            for s in &mut samples {
                *s = self.bass.process(*s);
            }
        }
        if self.treble_db.abs() > 0.01 {
            for s in &mut samples {
                *s = self.treble.process(*s);
            }
        }

        if self.noise_blanker > 0.0 {
            let thr = 1.0 + (1.0 - self.noise_blanker) * 20.0;
            let mut out = Vec::with_capacity(samples.len());
            for s in samples {
                self.nb_avg = 0.95 * self.nb_avg + 0.05 * s.abs();
                let limit = self.nb_avg * thr;
                out.push(if s.abs() > limit {
                    s.signum() * limit
                } else {
                    s
                });
            }
            samples = out;
        }

        // Extra audio gain.
        if (self.audio_gain - 1.0).abs() > 1e-4 {
            for s in &mut samples {
                *s *= self.audio_gain;
            }
        }

        // Pitch shift via fractional linear resampling.
        if self.pitch_octaves.abs() > 1e-4 {
            let ratio = 2.0_f32.powf(self.pitch_octaves);
            let out_len = (samples.len() as f32 / ratio).round().max(0.0) as usize;
            let mut out = Vec::with_capacity(out_len);
            for i in 0..out_len {
                let pos = (i as f32) * ratio + self.pitch_pos;
                let i0 = pos.floor() as usize;
                let frac = pos - i0 as f32;
                let a = samples.get(i0).copied().unwrap_or(0.0);
                let b = samples.get(i0 + 1).copied().unwrap_or(a);
                out.push(a + frac * (b - a));
            }
            // Keep fractional carry so consecutive chunks stay phase-aligned.
            let carried = ((out_len as f32) * ratio + self.pitch_pos) - (out_len as f32 * ratio);
            self.pitch_pos = carried.fract();
            samples = out;
        }

        samples
    }

    pub fn demodulate(&mut self, iq: &[u8], mode: DemodMode) -> Vec<f32> {
        let iq = self.iq_preprocess(iq);
        let samples = match mode {
            // Auto should be resolved to a concrete mode before reaching here
            // (see `DemodMode::resolve`); pass through if it ever slips by.
            DemodMode::Auto | DemodMode::Raw => self.demod_raw(&iq),
            DemodMode::Am => self.demod_am(&iq),
            DemodMode::Fm => self.demod_fm(&iq),
            DemodMode::Wfm => self.demod_wfm(&iq),
            DemodMode::Lsb => self.demod_ssb(&iq, false),
            DemodMode::Usb => self.demod_ssb(&iq, true),
        };
        let filtered = self.apply_lpf(samples);
        let agc = if self.agc_enabled {
            self.apply_agc(filtered)
        } else {
            filtered
        };
        self.post_process(agc)
    }

    fn apply_agc(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        // Soft-knee AGC with user-tunable target/attack/decay.
        const MAX_GAIN: f32 = 40.0;
        const MIN_GAIN: f32 = 0.1;

        for s in &mut samples {
            let out = *s * self.agc_gain;
            let abs = out.abs();
            if abs > self.agc_target {
                self.agc_gain *= 1.0 - self.agc_attack * (abs / self.agc_target - 1.0).min(1.0);
            } else {
                self.agc_gain *= 1.0 + self.agc_decay;
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
        // Wide FM demodulation with correct de-emphasis. Time constant τ is
        // user-selectable (50 µs EU / 75 µs US). τ = 1/(2π·f_c).
        // Discrete IIR: alpha = dt/(τ + dt) where dt = 1/sample_rate
        let tau = (self.deemph_tau_us.max(1.0)) * 1e-6_f32;
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
        self.hpf_state = 0.0;
        self.hpf_x_prev = 0.0;
        self.hpf_alpha_computed = 0.0;
        self.dc_block_state = 0.0;
        self.dc_block_x_prev = 0.0;
        self.notch.reset();
        self.bass.reset();
        self.treble.reset();
        self.nb_avg = 0.0;
        self.pitch_pos = 0.0;
        self.rf_nb_avg = 0.0;
        self.rf_notch_x1 = (0.0, 0.0);
        self.rf_notch_x2 = (0.0, 0.0);
        self.rf_notch_y1 = (0.0, 0.0);
        self.rf_notch_y2 = (0.0, 0.0);
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

    #[test]
    fn test_demod_new_defaults() {
        let d = Demodulator::new();
        assert_eq!(d.prev_i, 0.0);
        assert_eq!(d.prev_q, 0.0);
        assert_eq!(d.prev_phase, 0.0);
        assert_eq!(d.decimation, 1);
        assert_eq!(d.decim_counter, 0);
        assert_eq!(d.audio_sample_rate, 48000);
        assert_eq!(d.agc_gain, 1.0);
        assert!(d.agc_enabled);
    }

    #[test]
    fn test_demod_reset() {
        let mut d = Demodulator::new();
        d.prev_i = 0.7;
        d.lpf_state_l = 0.5;
        d.reset();
        assert_eq!(d.prev_i, 0.0);
        assert_eq!(d.lpf_state_l, 0.0);
    }

    #[test]
    fn test_demod_process_empty_iq() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        for mode in &[
            DemodMode::Raw,
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Wfm,
            DemodMode::Lsb,
            DemodMode::Usb,
        ] {
            let out = d.demodulate(&[], *mode);
            assert!(
                out.is_empty(),
                "mode {mode:?} with empty IQ should be empty"
            );
        }
    }

    #[test]
    fn test_demod_mode_switch_does_not_panic() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        let iq = make_iq_dc(128, 128, 32);
        let _raw = d.demodulate(&iq, DemodMode::Raw);
        let _am = d.demodulate(&iq, DemodMode::Am);
        let _fm = d.demodulate(&iq, DemodMode::Fm);
        let _wfm = d.demodulate(&iq, DemodMode::Wfm);
        let _lsb = d.demodulate(&iq, DemodMode::Lsb);
        let _usb = d.demodulate(&iq, DemodMode::Usb);
    }
}
