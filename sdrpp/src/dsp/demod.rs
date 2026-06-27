//! Demodulators - translated from core/src/dsp/demod

use num_complex::Complex32;
use std::f32::consts::PI;

// ============================================================================
// Shared types
// ============================================================================

#[derive(Clone, Copy, Debug, Default)]
pub struct StereoSample {
    pub l: f32,
    pub r: f32,
}

impl StereoSample {
    pub fn mono(v: f32) -> Self {
        Self { l: v, r: v }
    }
}

// ============================================================================
// Math helpers
// ============================================================================

fn hz_to_rads(hz: f32, sample_rate: f32) -> f32 {
    2.0 * PI * hz / sample_rate
}

fn normalize_phase(phase: f32) -> f32 {
    let mut p = phase;
    while p > PI {
        p -= 2.0 * PI;
    }
    while p < -PI {
        p += 2.0 * PI;
    }
    p
}

fn phasor(theta: f32) -> Complex32 {
    Complex32::new(theta.cos(), theta.sin())
}

fn gcd(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.abs()
}

// ============================================================================
// Window / taps
// ============================================================================

fn nuttall_window(n: f64, n_taps: f64) -> f64 {
    const COEFS: [f64; 4] = [0.355768, 0.487396, 0.144232, 0.012604];
    let mut win = 0.0;
    let mut sign = 1.0;
    for (i, &coef) in COEFS.iter().enumerate() {
        win += sign * coef * ((i as f64) * 2.0 * std::f64::consts::PI * n / n_taps).cos();
        sign = -sign;
    }
    win
}

fn estimate_tap_count(trans_width: f32, sample_rate: f32) -> usize {
    (3.8 * sample_rate / trans_width) as usize
}

pub fn lowpass_taps(cutoff_hz: f32, transition_hz: f32, sample_rate: f32) -> Vec<f32> {
    let count = estimate_tap_count(transition_hz, sample_rate).max(1);
    let half = count as f64 / 2.0;
    let omega = hz_to_rads(cutoff_hz, sample_rate) as f64;
    let corr = omega / std::f64::consts::PI;
    (0..count)
        .map(|i| {
            let t = i as f64 - half + 0.5;
            let sinc = if t == 0.0 {
                1.0
            } else {
                (t * omega).sin() / (t * omega)
            };
            (sinc * nuttall_window(t - half, count as f64) * corr) as f32
        })
        .collect()
}

pub fn bandpass_taps_f32(
    low_hz: f32,
    high_hz: f32,
    transition_hz: f32,
    sample_rate: f32,
) -> Vec<f32> {
    let mut count = estimate_tap_count(transition_hz, sample_rate).max(1);
    if count % 2 == 0 {
        count += 1;
    }
    let half = count as f64 / 2.0;
    let omega = hz_to_rads((high_hz - low_hz) / 2.0, sample_rate) as f64;
    let offset_omega = hz_to_rads((low_hz + high_hz) / 2.0, sample_rate) as f64;
    let corr = omega / std::f64::consts::PI;
    (0..count)
        .map(|i| {
            let t = i as f64 - half + 0.5;
            let sinc = if t == 0.0 {
                1.0
            } else {
                (t * omega).sin() / (t * omega)
            };
            let win = nuttall_window(t - half, count as f64);
            (2.0 * (offset_omega * t).cos() * sinc * win * corr) as f32
        })
        .collect()
}

pub fn bandpass_taps_complex(
    low_hz: f32,
    high_hz: f32,
    transition_hz: f32,
    sample_rate: f32,
) -> Vec<Complex32> {
    let mut count = estimate_tap_count(transition_hz, sample_rate).max(1);
    if count % 2 == 0 {
        count += 1;
    }
    let half = count as f64 / 2.0;
    let omega = hz_to_rads((high_hz - low_hz) / 2.0, sample_rate) as f64;
    let offset_omega = hz_to_rads((low_hz + high_hz) / 2.0, sample_rate) as f64;
    let corr = omega / std::f64::consts::PI;
    (0..count)
        .map(|i| {
            let t = i as f64 - half + 0.5;
            let sinc = if t == 0.0 {
                1.0
            } else {
                (t * omega).sin() / (t * omega)
            };
            let win = nuttall_window(t - half, count as f64);
            let base = (sinc * win * corr) as f32;
            Complex32::new(base, 0.0) * phasor(-(offset_omega * t) as f32)
        })
        .collect()
}

// ============================================================================
// FIR filter (generic over data and tap types)
// ============================================================================

pub struct FirFilter<T, Tap = f32> {
    taps: Vec<Tap>,
    delay: Vec<T>,
}

impl<T, Tap> FirFilter<T, Tap>
where
    T: Copy + Default + std::ops::Add<Output = T> + std::ops::Mul<Tap, Output = T>,
    Tap: Copy,
{
    pub fn new(taps: &[Tap]) -> Self {
        Self {
            taps: taps.to_vec(),
            delay: vec![T::default(); taps.len()],
        }
    }

    pub fn process(&mut self, input: &[T], output: &mut [T]) {
        let n = self.taps.len();
        for (i, &sample) in input.iter().enumerate() {
            self.delay.copy_within(1.., 0);
            self.delay[n - 1] = sample;
            let mut acc = T::default();
            for (j, &tap) in self.taps.iter().enumerate() {
                acc = acc + self.delay[j] * tap;
            }
            output[i] = acc;
        }
    }

    pub fn reset(&mut self) {
        self.delay.fill(T::default());
    }
}

// ============================================================================
// AGC
// ============================================================================

pub trait AgcSample: Copy {
    fn amplitude(self) -> f32;
    fn scale(self, gain: f32) -> Self;
}

impl AgcSample for f32 {
    fn amplitude(self) -> f32 {
        self.abs()
    }
    fn scale(self, gain: f32) -> Self {
        self * gain
    }
}

impl AgcSample for Complex32 {
    fn amplitude(self) -> f32 {
        self.norm()
    }
    fn scale(self, gain: f32) -> Self {
        self * gain
    }
}

pub struct Agc<T> {
    set_point: f32,
    attack: f32,
    inv_attack: f32,
    decay: f32,
    inv_decay: f32,
    max_gain: f32,
    max_output_amp: f32,
    amp: f32,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: AgcSample> Agc<T> {
    pub fn new(
        set_point: f32,
        attack: f32,
        decay: f32,
        max_gain: f32,
        max_output_amp: f32,
        init_gain: f32,
    ) -> Self {
        Self {
            set_point,
            attack,
            inv_attack: 1.0 - attack,
            decay,
            inv_decay: 1.0 - decay,
            max_gain,
            max_output_amp,
            amp: set_point / init_gain,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn set_attack(&mut self, attack: f32) {
        self.attack = attack;
        self.inv_attack = 1.0 - attack;
    }

    pub fn set_decay(&mut self, decay: f32) {
        self.decay = decay;
        self.inv_decay = 1.0 - decay;
    }

    pub fn reset(&mut self) {
        self.amp = self.set_point;
    }

    pub fn process(&mut self, input: &[T], output: &mut [T]) {
        for i in 0..input.len() {
            let in_amp = input[i].amplitude();
            let gain = if in_amp != 0.0 {
                self.amp = if in_amp > self.amp {
                    self.amp * self.inv_attack + in_amp * self.attack
                } else {
                    self.amp * self.inv_decay + in_amp * self.decay
                };
                (self.set_point / self.amp).min(self.max_gain)
            } else {
                1.0
            };

            if in_amp * gain > self.max_output_amp {
                let mut max_amp = 0.0f32;
                for j in i..input.len() {
                    let a = input[j].amplitude();
                    if a > max_amp {
                        max_amp = a;
                    }
                }
                self.amp = max_amp;
                let gain2 = (self.set_point / self.amp).min(self.max_gain);
                output[i] = input[i].scale(gain2);
            } else {
                output[i] = input[i].scale(gain);
            }
        }
    }
}

// ============================================================================
// DC Blocker
// ============================================================================

pub struct DcBlocker {
    rate: f32,
    offset: f32,
}

impl DcBlocker {
    pub fn new(rate: f32) -> Self {
        Self { rate, offset: 0.0 }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
    }

    pub fn reset(&mut self) {
        self.offset = 0.0;
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        for i in 0..input.len() {
            output[i] = input[i] - self.offset;
            self.offset += output[i] * self.rate;
        }
    }
}

// ============================================================================
// Frequency Translator (complex rotator)
// ============================================================================

pub struct FrequencyXlator {
    phase: Complex32,
    phase_delta: Complex32,
}

impl FrequencyXlator {
    pub fn new(offset_hz: f32, sample_rate: f32) -> Self {
        let delta = hz_to_rads(offset_hz, sample_rate);
        Self {
            phase: Complex32::new(1.0, 0.0),
            phase_delta: phasor(delta),
        }
    }

    pub fn set_offset(&mut self, offset_hz: f32, sample_rate: f32) {
        let delta = hz_to_rads(offset_hz, sample_rate);
        self.phase_delta = phasor(delta);
    }

    pub fn reset(&mut self) {
        self.phase = Complex32::new(1.0, 0.0);
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) {
        for (i, &sample) in input.iter().enumerate() {
            output[i] = sample * self.phase;
            self.phase *= self.phase_delta;
            // Renormalize to prevent drift
            let norm = self.phase.norm();
            if norm > 0.0 {
                self.phase /= norm;
            }
        }
    }
}

// ============================================================================
// PLL
// ============================================================================

pub struct Pll {
    phase: f32,
    freq: f32,
    alpha: f32,
    beta: f32,
    min_freq: f32,
    max_freq: f32,
    init_phase: f32,
    init_freq: f32,
}

impl Pll {
    pub fn new(
        bandwidth: f32,
        init_phase: f32,
        init_freq: f32,
        min_freq: f32,
        max_freq: f32,
    ) -> Self {
        let (alpha, beta) = Self::critically_damped(bandwidth);
        Self {
            phase: init_phase,
            freq: init_freq,
            alpha,
            beta,
            min_freq,
            max_freq,
            init_phase,
            init_freq,
        }
    }

    fn critically_damped(bandwidth: f32) -> (f32, f32) {
        let damping = 2.0f32.sqrt() / 2.0;
        let denom = 1.0 + 2.0 * damping * bandwidth + bandwidth * bandwidth;
        let alpha = (4.0 * damping * bandwidth) / denom;
        let beta = (4.0 * bandwidth * bandwidth) / denom;
        (alpha, beta)
    }

    pub fn set_bandwidth(&mut self, bandwidth: f32) {
        let (alpha, beta) = Self::critically_damped(bandwidth);
        self.alpha = alpha;
        self.beta = beta;
    }

    pub fn set_freq_limits(&mut self, min_freq: f32, max_freq: f32) {
        self.min_freq = min_freq;
        self.max_freq = max_freq;
    }

    pub fn reset(&mut self) {
        self.phase = self.init_phase;
        self.freq = self.init_freq;
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) {
        for i in 0..input.len() {
            output[i] = phasor(self.phase);
            let error = normalize_phase(input[i].arg() - self.phase);
            self.freq += self.beta * error;
            self.freq = self.freq.clamp(self.min_freq, self.max_freq);
            self.phase += self.freq + self.alpha * error;
        }
    }
}

// ============================================================================
// Delay line
// ============================================================================

pub struct DelayLine<T> {
    buf: Vec<T>,
    pos: usize,
}

impl<T: Copy + Default> DelayLine<T> {
    pub fn new(delay: usize) -> Self {
        Self {
            buf: vec![T::default(); delay],
            pos: 0,
        }
    }

    pub fn set_delay(&mut self, delay: usize) {
        self.buf.resize(delay, T::default());
        self.buf.fill(T::default());
        self.pos = 0;
    }

    pub fn reset(&mut self) {
        self.buf.fill(T::default());
        self.pos = 0;
    }

    pub fn process(&mut self, input: &[T], output: &mut [T]) {
        let len = self.buf.len();
        if len == 0 {
            output.copy_from_slice(input);
            return;
        }
        for (i, &sample) in input.iter().enumerate() {
            output[i] = self.buf[self.pos];
            self.buf[self.pos] = sample;
            self.pos += 1;
            if self.pos >= len {
                self.pos = 0;
            }
        }
    }
}

// ============================================================================
// Quadrature demodulator (base for FM / WFM)
// ============================================================================

pub struct QuadratureDemod {
    inv_deviation: f32,
    last_phase: f32,
}

impl QuadratureDemod {
    pub fn new(deviation_hz: f32, sample_rate: f32) -> Self {
        Self {
            inv_deviation: 1.0 / hz_to_rads(deviation_hz, sample_rate),
            last_phase: 0.0,
        }
    }

    pub fn set_deviation(&mut self, deviation_hz: f32, sample_rate: f32) {
        self.inv_deviation = 1.0 / hz_to_rads(deviation_hz, sample_rate);
    }

    pub fn reset(&mut self) {
        self.last_phase = 0.0;
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut [f32]) {
        for (i, &sample) in input.iter().enumerate() {
            let phase = sample.arg();
            output[i] = normalize_phase(phase - self.last_phase) * self.inv_deviation;
            self.last_phase = phase;
        }
    }
}

// ============================================================================
// Rational resampler (simplified FIR-based L/M resampler)
// ============================================================================

pub struct RationalResampler<T> {
    interp: usize,
    decim: usize,
    fir: FirFilter<T, f32>,
    phase: usize,
}

impl<T: Copy + Default + std::ops::Add<Output = T> + std::ops::Mul<f32, Output = T>>
    RationalResampler<T>
{
    pub fn new(in_sample_rate: f32, out_sample_rate: f32) -> Self {
        let in_sr = in_sample_rate.round() as i32;
        let out_sr = out_sample_rate.round() as i32;
        let g = gcd(in_sr, out_sr);
        let interp = (out_sr / g) as usize;
        let decim = (in_sr / g) as usize;
        let tap_sr = in_sample_rate * interp as f32;
        let cutoff = in_sample_rate.min(out_sample_rate) / 2.0;
        let mut taps = lowpass_taps(cutoff, cutoff * 0.1, tap_sr);
        for t in &mut taps {
            *t *= interp as f32;
        }
        Self {
            interp,
            decim,
            fir: FirFilter::new(&taps),
            phase: 0,
        }
    }

    pub fn reset(&mut self) {
        self.fir.reset();
        self.phase = 0;
    }

    pub fn process(&mut self, input: &[T]) -> Vec<T> {
        let n = input.len();
        let mut upsample = vec![T::default(); n * self.interp];
        for (i, &sample) in input.iter().enumerate() {
            upsample[i * self.interp] = sample;
        }
        let mut filtered = vec![T::default(); n * self.interp];
        self.fir.process(&upsample, &mut filtered);

        let mut output = Vec::new();
        for sample in filtered {
            self.phase += 1;
            if self.phase >= self.decim {
                self.phase -= self.decim;
                output.push(sample);
            }
        }
        output
    }
}

// ============================================================================
// FM Demodulator
// ============================================================================

pub struct FmDemod {
    quadrature: QuadratureDemod,
    fir: Option<FirFilter<f32, f32>>,
    sample_rate: f32,
    bandwidth: f32,
    low_pass: bool,
}

impl FmDemod {
    pub fn new(sample_rate: f32, bandwidth: f32, low_pass: bool) -> Self {
        let mut d = Self {
            quadrature: QuadratureDemod::new(bandwidth / 2.0, sample_rate),
            fir: None,
            sample_rate,
            bandwidth,
            low_pass,
        };
        d.update_filter();
        d
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.quadrature
            .set_deviation(self.bandwidth / 2.0, sample_rate);
        self.update_filter();
    }

    pub fn set_bandwidth(&mut self, bandwidth: f32) {
        if (bandwidth - self.bandwidth).abs() < f32::EPSILON {
            return;
        }
        self.bandwidth = bandwidth;
        self.quadrature
            .set_deviation(self.bandwidth / 2.0, self.sample_rate);
        self.update_filter();
    }

    pub fn set_low_pass(&mut self, low_pass: bool) {
        self.low_pass = low_pass;
        self.update_filter();
    }

    pub fn reset(&mut self) {
        self.quadrature.reset();
        if let Some(ref mut fir) = self.fir {
            fir.reset();
        }
    }

    fn update_filter(&mut self) {
        if self.low_pass {
            let taps = lowpass_taps(self.bandwidth / 2.0, (self.bandwidth / 2.0) * 0.1, self.sample_rate);
            self.fir = Some(FirFilter::new(&taps));
        } else {
            self.fir = None;
        }
    }

    /// Mono FM output
    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut output = vec![0.0f32; input.len()];
        self.quadrature.process(input, &mut output);
        if let Some(ref mut fir) = self.fir {
            fir.process(&output.clone(), &mut output);
        }
        output
    }

    /// Stereo FM output (duplicated mono to both channels)
    pub fn process_stereo(&mut self, input: &[Complex32]) -> Vec<StereoSample> {
        self.process(input)
            .into_iter()
            .map(StereoSample::mono)
            .collect()
    }
}

// ============================================================================
// AM Demodulator
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgcMode {
    Carrier,
    Audio,
}

pub struct AmDemod {
    agc_mode: AgcMode,
    carrier_agc: Agc<Complex32>,
    audio_agc: Agc<f32>,
    dc_block: DcBlocker,
    lpf: FirFilter<f32, f32>,
    sample_rate: f32,
    bandwidth: f32,
}

impl AmDemod {
    pub fn new(
        agc_mode: AgcMode,
        bandwidth: f32,
        agc_attack: f32,
        agc_decay: f32,
        dc_block_rate: f32,
        sample_rate: f32,
    ) -> Self {
        let lpf_taps = lowpass_taps(bandwidth / 2.0, (bandwidth / 2.0) * 0.1, sample_rate);
        Self {
            agc_mode,
            carrier_agc: Agc::new(1.0, agc_attack, agc_decay, 10e6, 10.0, 1.0),
            audio_agc: Agc::new(1.0, agc_attack, agc_decay, 10e6, 10.0, 1.0),
            dc_block: DcBlocker::new(dc_block_rate),
            lpf: FirFilter::new(&lpf_taps),
            sample_rate,
            bandwidth,
        }
    }

    pub fn set_agc_mode(&mut self, mode: AgcMode) {
        self.agc_mode = mode;
        self.reset();
    }

    pub fn set_bandwidth(&mut self, bandwidth: f32) {
        if (bandwidth - self.bandwidth).abs() < f32::EPSILON {
            return;
        }
        self.bandwidth = bandwidth;
        let taps = lowpass_taps(bandwidth / 2.0, (bandwidth / 2.0) * 0.1, self.sample_rate);
        self.lpf = FirFilter::new(&taps);
    }

    pub fn reset(&mut self) {
        self.carrier_agc.reset();
        self.audio_agc.reset();
        self.dc_block.reset();
        self.lpf.reset();
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut tmp = input.to_vec();
        if self.agc_mode == AgcMode::Carrier {
            self.carrier_agc.process(input, &mut tmp);
        }

        let mut output: Vec<f32> = tmp.iter().map(|c| c.norm()).collect();
        self.dc_block.process(&output.clone(), &mut output);
        if self.agc_mode == AgcMode::Audio {
            self.audio_agc.process(&output.clone(), &mut output);
        }
        self.lpf.process(&output.clone(), &mut output);
        output
    }

    pub fn process_stereo(&mut self, input: &[Complex32]) -> Vec<StereoSample> {
        self.process(input)
            .into_iter()
            .map(StereoSample::mono)
            .collect()
    }
}

// ============================================================================
// SSB Demodulator
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SsbMode {
    Usb,
    Lsb,
    Dsb,
}

pub struct SsbDemod {
    mode: SsbMode,
    xlator: FrequencyXlator,
    agc: Agc<f32>,
    bandwidth: f32,
    sample_rate: f32,
}

impl SsbDemod {
    pub fn new(
        mode: SsbMode,
        bandwidth: f32,
        sample_rate: f32,
        agc_attack: f32,
        agc_decay: f32,
    ) -> Self {
        let offset = match mode {
            SsbMode::Usb => bandwidth / 2.0,
            SsbMode::Lsb => -bandwidth / 2.0,
            SsbMode::Dsb => 0.0,
        };
        Self {
            mode,
            xlator: FrequencyXlator::new(offset, sample_rate),
            agc: Agc::new(1.0, agc_attack, agc_decay, 10e6, 10.0, 1.0),
            bandwidth,
            sample_rate,
        }
    }

    pub fn set_mode(&mut self, mode: SsbMode) {
        self.mode = mode;
        let offset = match mode {
            SsbMode::Usb => self.bandwidth / 2.0,
            SsbMode::Lsb => -self.bandwidth / 2.0,
            SsbMode::Dsb => 0.0,
        };
        self.xlator.set_offset(offset, self.sample_rate);
    }

    pub fn set_bandwidth(&mut self, bandwidth: f32) {
        self.bandwidth = bandwidth;
        let offset = match self.mode {
            SsbMode::Usb => bandwidth / 2.0,
            SsbMode::Lsb => -bandwidth / 2.0,
            SsbMode::Dsb => 0.0,
        };
        self.xlator.set_offset(offset, self.sample_rate);
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let offset = match self.mode {
            SsbMode::Usb => self.bandwidth / 2.0,
            SsbMode::Lsb => -self.bandwidth / 2.0,
            SsbMode::Dsb => 0.0,
        };
        self.xlator.set_offset(offset, sample_rate);
    }

    pub fn reset(&mut self) {
        self.xlator.reset();
        self.agc.reset();
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut translated = vec![Complex32::default(); input.len()];
        self.xlator.process(input, &mut translated);
        let mut output: Vec<f32> = translated.iter().map(|c| c.re).collect();
        self.agc.process(&output.clone(), &mut output);
        output
    }

    pub fn process_stereo(&mut self, input: &[Complex32]) -> Vec<StereoSample> {
        self.process(input)
            .into_iter()
            .map(StereoSample::mono)
            .collect()
    }
}

// ============================================================================
// WFM / Broadcast FM Demodulator
// ============================================================================

pub struct WfmOutput {
    pub stereo: Vec<StereoSample>,
    pub rds: Option<Vec<Complex32>>,
}

pub struct WfmDemod {
    sample_rate: f32,
    deviation: f32,
    stereo: bool,
    low_pass: bool,
    rds_out: bool,

    quadrature: QuadratureDemod,
    pilot_fir: FirFilter<Complex32, Complex32>,
    pilot_pll: Pll,
    lpr_delay: DelayLine<f32>,
    lmr_delay: DelayLine<Complex32>,
    audio_fir_l: FirFilter<f32, f32>,
    audio_fir_r: FirFilter<f32, f32>,
    rds_xlator: FrequencyXlator,
    rds_resamp: Option<RationalResampler<Complex32>>,
}

impl WfmDemod {
    pub fn new(
        deviation: f32,
        sample_rate: f32,
        stereo: bool,
        low_pass: bool,
        rds_out: bool,
    ) -> Self {
        let pilot_taps = bandpass_taps_complex(18750.0, 19250.0, 3000.0, sample_rate);
        let pilot_group_delay = (pilot_taps.len() - 1) / 2 + 1;
        let audio_taps = lowpass_taps(15000.0, 4000.0, sample_rate);

        let quadrature = QuadratureDemod::new(deviation, sample_rate);
        let pilot_fir = FirFilter::new(&pilot_taps);
        let pilot_pll = Pll::new(
            25000.0 / sample_rate,
            0.0,
            hz_to_rads(19000.0, sample_rate),
            hz_to_rads(18750.0, sample_rate),
            hz_to_rads(19250.0, sample_rate),
        );
        let lpr_delay = DelayLine::new(pilot_group_delay);
        let lmr_delay = DelayLine::new(pilot_group_delay);
        let audio_fir_l = FirFilter::new(&audio_taps);
        let audio_fir_r = FirFilter::new(&audio_taps);
        let rds_xlator = FrequencyXlator::new(-57000.0, sample_rate);
        let rds_resamp = if rds_out {
            Some(RationalResampler::new(sample_rate, 5000.0))
        } else {
            None
        };

        Self {
            sample_rate,
            deviation,
            stereo,
            low_pass,
            rds_out,
            quadrature,
            pilot_fir,
            pilot_pll,
            lpr_delay,
            lmr_delay,
            audio_fir_l,
            audio_fir_r,
            rds_xlator,
            rds_resamp,
        }
    }

    pub fn set_deviation(&mut self, deviation: f32) {
        self.deviation = deviation;
        self.quadrature
            .set_deviation(deviation, self.sample_rate);
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.quadrature
            .set_deviation(self.deviation, sample_rate);

        let pilot_taps = bandpass_taps_complex(18750.0, 19250.0, 3000.0, sample_rate);
        let pilot_group_delay = (pilot_taps.len() - 1) / 2 + 1;
        self.pilot_fir = FirFilter::new(&pilot_taps);
        self.pilot_pll.set_freq_limits(
            hz_to_rads(18750.0, sample_rate),
            hz_to_rads(19250.0, sample_rate),
        );
        self.pilot_pll.init_freq = hz_to_rads(19000.0, sample_rate);
        self.lpr_delay.set_delay(pilot_group_delay);
        self.lmr_delay.set_delay(pilot_group_delay);

        let audio_taps = lowpass_taps(15000.0, 4000.0, sample_rate);
        self.audio_fir_l = FirFilter::new(&audio_taps);
        self.audio_fir_r = FirFilter::new(&audio_taps);

        self.rds_xlator.set_offset(-57000.0, sample_rate);
        if let Some(ref mut rds) = self.rds_resamp {
            *rds = RationalResampler::new(sample_rate, 5000.0);
        }
        self.reset();
    }

    pub fn set_stereo(&mut self, stereo: bool) {
        self.stereo = stereo;
        self.reset();
    }

    pub fn set_low_pass(&mut self, low_pass: bool) {
        self.low_pass = low_pass;
        self.reset();
    }

    pub fn set_rds_out(&mut self, rds_out: bool) {
        self.rds_out = rds_out;
        if rds_out && self.rds_resamp.is_none() {
            self.rds_resamp = Some(RationalResampler::new(self.sample_rate, 5000.0));
        } else if !rds_out {
            self.rds_resamp = None;
        }
        self.reset();
    }

    pub fn reset(&mut self) {
        self.quadrature.reset();
        self.pilot_fir.reset();
        self.pilot_pll.reset();
        self.lpr_delay.reset();
        self.lmr_delay.reset();
        self.audio_fir_l.reset();
        self.audio_fir_r.reset();
        if let Some(ref mut rds) = self.rds_resamp {
            rds.reset();
        }
    }

    pub fn process(&mut self, input: &[Complex32]) -> WfmOutput {
        let count = input.len();
        let mut mpx = vec![0.0f32; count];
        self.quadrature.process(input, &mut mpx);

        let mut rds = None;

        let stereo = if self.stereo {
            // 1. MPX -> complex
            let mpx_c: Vec<Complex32> = mpx.iter().map(|&r| Complex32::new(r, 0.0)).collect();

            // RDS path is taken from the (undelayed) complex MPX
            if self.rds_out {
                let mut rds_base = vec![Complex32::default(); count];
                self.rds_xlator.process(&mpx_c, &mut rds_base);
                if let Some(ref mut resamp) = self.rds_resamp {
                    rds = Some(resamp.process(&rds_base));
                }
            }

            // 2. Extract pilot with complex bandpass FIR
            let mut pilot_filtered = vec![Complex32::default(); count];
            self.pilot_fir.process(&mpx_c, &mut pilot_filtered);

            // 3. Lock PLL to pilot
            let mut pll_out = vec![Complex32::default(); count];
            self.pilot_pll.process(&pilot_filtered, &mut pll_out);

            // 4. Delay LPR and LMR to match pilot filter group delay
            let mut lpr_delayed = vec![0.0f32; count];
            self.lpr_delay.process(&mpx, &mut lpr_delayed);

            let mut lmr_delayed = vec![Complex32::default(); count];
            self.lmr_delay.process(&mpx_c, &mut lmr_delayed);

            // 5. Downconvert L-R: multiply by conj(PLL)^2 (38 kHz)
            for i in 0..count {
                let conj_pll = pll_out[i].conj();
                let mixer = conj_pll * conj_pll;
                lmr_delayed[i] = lmr_delayed[i] * mixer;
            }

            // 6. Complex -> real and amplify by 2x
            let mut lmr = vec![0.0f32; count];
            for i in 0..count {
                lmr[i] = lmr_delayed[i].re * 2.0;
            }

            // 7. Matrix: L = (L+R) + (L-R), R = (L+R) - (L-R)
            let mut l = vec![0.0f32; count];
            let mut r = vec![0.0f32; count];
            for i in 0..count {
                l[i] = lpr_delayed[i] + lmr[i];
                r[i] = lpr_delayed[i] - lmr[i];
            }

            // 8. Optional 15 kHz audio lowpass
            if self.low_pass {
                let mut l_filt = l.clone();
                let mut r_filt = r.clone();
                self.audio_fir_l.process(&l, &mut l_filt);
                self.audio_fir_r.process(&r, &mut r_filt);
                l = l_filt;
                r = r_filt;
            }

            l.into_iter()
                .zip(r)
                .map(|(left, right)| StereoSample { l: left, r: right })
                .collect()
        } else {
            // Mono / raw MPX path
            if self.rds_out {
                let mpx_c: Vec<Complex32> = mpx.iter().map(|&r| Complex32::new(r, 0.0)).collect();
                let mut rds_base = vec![Complex32::default(); count];
                self.rds_xlator.process(&mpx_c, &mut rds_base);
                if let Some(ref mut resamp) = self.rds_resamp {
                    rds = Some(resamp.process(&rds_base));
                }
            }

            let mut out = mpx.clone();
            if self.low_pass {
                self.audio_fir_l.process(&mpx, &mut out);
            }
            out.into_iter().map(StereoSample::mono).collect()
        };

        WfmOutput { stereo, rds }
    }
}
