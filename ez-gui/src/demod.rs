use crate::sdr_panel::DemodMode;
use num_complex::Complex32;
use std::borrow::Cow;

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

/// Sixth-order Butterworth channel filter. Double-precision state avoids the
/// pole-rounding instability of narrow CW filters at MHz input rates. Coefficients
/// are cached, so no trigonometry or divisions occur in the filter sample loop.
#[derive(Default)]
struct ComplexChannelLowpass {
    stages: [([f64; 3], [f64; 2], [[f64; 2]; 2]); 3],
    cutoff: f32,
    rate: f64,
}

impl ComplexChannelLowpass {
    fn process(&mut self, i: f32, q: f32, cutoff_hz: f32, sample_rate_hz: f64) -> (f32, f32) {
        let rate = sample_rate_hz.max(1.0);
        let cutoff = cutoff_hz.max(1.0).min(rate as f32 * 0.45);
        if self.cutoff != cutoff || self.rate != rate {
            let omega = std::f64::consts::TAU * f64::from(cutoff) / f64::from(rate);
            let (sin, cos) = omega.sin_cos();
            for (stage, quality) in self.stages.iter_mut().zip([
                0.517_638_090_205_041_5,
                std::f64::consts::FRAC_1_SQRT_2,
                1.931_851_652_578_136_6,
            ]) {
                let alpha = sin / (2.0 * quality);
                let a0 = 1.0 + alpha;
                stage.0 = [
                    (1.0 - cos) / (2.0 * a0),
                    (1.0 - cos) / a0,
                    (1.0 - cos) / (2.0 * a0),
                ];
                stage.1 = [-2.0 * cos / a0, (1.0 - alpha) / a0];
            }
            self.cutoff = cutoff;
            self.rate = rate;
        }
        let mut signal = [f64::from(i), f64::from(q)];
        for (b, a, history) in &mut self.stages {
            for (sample, state) in signal.iter_mut().zip(history) {
                let output = b[0] * *sample + state[0];
                state[0] = b[1] * *sample - a[0] * output + state[1];
                state[1] = b[2] * *sample - a[1] * output;
                *sample = output;
            }
        }
        (signal[0] as f32, signal[1] as f32)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Default)]
struct AudioResampler {
    previous: Option<f32>,
    input_index: u64,
    next_output: f64,
}

impl AudioResampler {
    fn process(&mut self, input: Vec<f32>, input_rate: f64, output_rate: u32) -> Vec<f32> {
        if (input_rate - f64::from(output_rate)).abs() < 1e-6 {
            return input;
        }
        let step = input_rate / f64::from(output_rate.max(1));
        let mut output = Vec::with_capacity((input.len() as f64 / step).ceil() as usize + 1);
        for sample in input {
            if let Some(previous) = self.previous {
                self.input_index += 1;
                while self.next_output <= self.input_index as f64 {
                    let fraction = (self.next_output - (self.input_index - 1) as f64) as f32;
                    output.push(previous + fraction * (sample - previous));
                    self.next_output += step;
                }
            } else {
                output.push(sample);
                self.next_output = step;
            }
            self.previous = Some(sample);
        }
        output
    }
}

/// Linear-phase low-pass with decimation. The duplicated ring avoids modulo in
/// the dot product, which is evaluated only for retained multiplex samples.
#[derive(Default)]
struct MultiplexDecimator {
    rate: f64,
    factor: usize,
    phase: usize,
    write: usize,
    taps: Vec<f32>,
    history: Vec<f32>,
}

impl MultiplexDecimator {
    fn configure(&mut self, rate: f64) {
        if self.rate == rate {
            return;
        }
        self.rate = rate;
        self.factor = (rate / 192_000.0).floor().max(1.0) as usize;
        self.phase = 0;
        self.write = 0;
        let count = (self.factor * 24 + 1).clamp(63, 513) | 1;
        let cutoff = (65_000.0 / f64::from(rate)).min(0.4 / self.factor as f64);
        let middle = (count - 1) as f64 * 0.5;
        self.taps = (0..count)
            .map(|index| {
                let offset = index as f64 - middle;
                let sinc = if offset == 0.0 {
                    2.0 * cutoff
                } else {
                    (std::f64::consts::TAU * cutoff * offset).sin()
                        / (std::f64::consts::PI * offset)
                };
                let window = 0.42
                    - 0.5 * (std::f64::consts::TAU * index as f64 / (count - 1) as f64).cos()
                    + 0.08
                        * (2.0 * std::f64::consts::TAU * index as f64 / (count - 1) as f64).cos();
                (sinc * window) as f32
            })
            .collect();
        let sum = self.taps.iter().sum::<f32>();
        for tap in &mut self.taps {
            *tap /= sum;
        }
        self.history = vec![0.0; count * 2];
    }

    fn push(&mut self, value: f32) -> Option<f32> {
        let length = self.taps.len();
        self.history[self.write] = value;
        self.history[self.write + length] = value;
        self.write = (self.write + 1) % length;
        self.phase += 1;
        if self.phase < self.factor {
            return None;
        }
        self.phase = 0;
        Some(
            self.history[self.write..self.write + length]
                .iter()
                .zip(&self.taps)
                .map(|(sample, tap)| sample * tap)
                .sum(),
        )
    }
}

#[derive(Default)]
struct StereoFmDecoder {
    multiplex: MultiplexDecimator,
    pilot_phase: f64,
    pilot_frequency: f64,
    pilot_i: [f64; 2],
    pilot_q: [f64; 2],
    pilot_amplitude: f64,
    pilot_error: f64,
    locked: bool,
    blend: f32,
    channels: ComplexChannelLowpass,
    deemphasis: [f32; 2],
    audio_phase: usize,
}

impl StereoFmDecoder {
    fn process(
        &mut self,
        input: &[f32],
        input_rate: f64,
        audio_rate: u32,
        tau_us: f32,
    ) -> (Vec<f32>, Vec<f32>, f64) {
        self.multiplex.configure(input_rate);
        let rate = f64::from(input_rate) / self.multiplex.factor as f64;
        let decimation = (rate / f64::from(audio_rate)).floor().max(1.0) as usize;
        let output_rate = rate / decimation as f64;
        let pilot_step = std::f64::consts::TAU * 19_000.0 / rate;
        let pilot_alpha = 1.0 - (-std::f64::consts::TAU * 300.0 / rate).exp();
        let observation_alpha = 1.0 - (-std::f64::consts::TAU * 10.0 / rate).exp();
        let omega = std::f64::consts::TAU * 20.0 / rate;
        let loop_p = std::f64::consts::SQRT_2 * omega;
        let loop_i = omega * omega;
        let maximum_error = std::f64::consts::TAU * 100.0 / rate;
        let blend_alpha = (1.0 - (-1.0 / (rate * 0.04)).exp()) as f32;
        let dt = 1.0 / rate as f32;
        let deemph_alpha = dt / (tau_us.max(0.0) * 1e-6 + dt);
        let mut left = Vec::with_capacity(input.len() / self.multiplex.factor / decimation + 1);
        let mut right = Vec::with_capacity(left.capacity());
        for &input in input {
            let Some(mpx) = self.multiplex.push(input) else {
                continue;
            };
            let (sin, cos) = self.pilot_phase.sin_cos();
            self.pilot_i[0] += pilot_alpha * (f64::from(mpx) * cos - self.pilot_i[0]);
            self.pilot_q[0] += pilot_alpha * (-f64::from(mpx) * sin - self.pilot_q[0]);
            self.pilot_i[1] += pilot_alpha * (self.pilot_i[0] - self.pilot_i[1]);
            self.pilot_q[1] += pilot_alpha * (self.pilot_q[0] - self.pilot_q[1]);
            let amplitude = self.pilot_i[1].hypot(self.pilot_q[1]);
            let error = self.pilot_q[1].atan2(self.pilot_i[1]);
            self.pilot_amplitude += observation_alpha * (amplitude - self.pilot_amplitude);
            self.pilot_error += observation_alpha * (error.abs() - self.pilot_error);
            self.locked = self.pilot_amplitude > 0.02 && self.pilot_error < 0.18;
            let target_blend = if self.locked { 1.0 } else { 0.0 };
            self.blend += blend_alpha * (target_blend - self.blend);
            let difference = mpx * (2.0 * (2.0 * self.pilot_phase).cos()) as f32;
            if amplitude > 0.005 {
                self.pilot_frequency =
                    (self.pilot_frequency + loop_i * error).clamp(-maximum_error, maximum_error);
                self.pilot_phase += pilot_step + self.pilot_frequency + loop_p * error;
            } else {
                self.pilot_phase += pilot_step + self.pilot_frequency;
            }
            self.pilot_phase = self.pilot_phase.rem_euclid(std::f64::consts::TAU);
            let (sum, difference) = self.channels.process(
                mpx,
                difference,
                15_000.0_f32.min(audio_rate as f32 * 0.4),
                rate,
            );
            let stereo = [sum + self.blend * difference, sum - self.blend * difference];
            for (state, sample) in self.deemphasis.iter_mut().zip(stereo) {
                *state += deemph_alpha * (sample - *state);
            }
            self.audio_phase += 1;
            if self.audio_phase >= decimation {
                self.audio_phase = 0;
                left.push(self.deemphasis[0]);
                right.push(self.deemphasis[1]);
            }
        }
        (left, right, output_rate)
    }
}

/// Installed SDR++ FM IF noise-reduction preset names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FmIfPreset {
    NoaaApt,
    #[default]
    Voice,
    NarrowBand,
}

impl FmIfPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoaaApt => "NOAA APT",
            Self::Voice => "Voice",
            Self::NarrowBand => "Narrow Band",
        }
    }
}

/// DSB demodulator sideband selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DsbSideband {
    Lower,
    Upper,
    #[default]
    Both,
}

impl DsbSideband {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lower => "Lower",
            Self::Upper => "Upper",
            Self::Both => "Both",
        }
    }
}

/// Sliding Nuttall-window strongest-bin reconstruction, matching the recovered
/// installed FMIF structure. With only one inverse bin, evaluating its center
/// sample analytically is exactly the inverse-DFT sum and avoids a second FFT.
struct FmIfNoiseReducer {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    scratch: Vec<Complex32>,
    window: Vec<f32>,
    center_phase: Vec<Complex32>,
    history: Vec<Complex32>,
    spectrum: Vec<Complex32>,
    write: usize,
}

impl FmIfNoiseReducer {
    fn new(bins: usize) -> Self {
        let mut planner = rustfft::FftPlanner::new();
        let fft = planner.plan_fft_forward(bins);
        let window = (0..bins)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / (bins - 1) as f64;
                (0.355768 - 0.487396 * angle.cos() + 0.144232 * (2.0 * angle).cos()
                    - 0.012604 * (3.0 * angle).cos()) as f32
            })
            .collect();
        let center_phase = (0..bins)
            .map(|bin| {
                Complex32::from_polar(
                    1.0,
                    (std::f64::consts::TAU * bin as f64 * (bins / 2) as f64 / bins as f64) as f32,
                )
            })
            .collect();
        Self {
            scratch: vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()],
            fft,
            window,
            center_phase,
            history: vec![Complex32::new(0.0, 0.0); bins * 2],
            spectrum: vec![Complex32::new(0.0, 0.0); bins],
            write: 0,
        }
    }

    fn push(&mut self, sample: Complex32) -> Complex32 {
        let bins = self.window.len();
        self.history[self.write] = sample;
        self.history[self.write + bins] = sample;
        self.write = (self.write + 1) % bins;
        for ((value, sample), window) in self
            .spectrum
            .iter_mut()
            .zip(&self.history[self.write..self.write + bins])
            .zip(&self.window)
        {
            *value = *sample * *window;
        }
        self.fft
            .process_with_scratch(&mut self.spectrum, &mut self.scratch);
        let mut best = 0;
        let mut power = self.spectrum[0].norm_sqr();
        for (index, value) in self.spectrum.iter().enumerate().skip(1) {
            let candidate = value.norm_sqr();
            if candidate > power {
                power = candidate;
                best = index;
            }
        }
        // FFTW's reference inverse is unnormalized. FM discrimination is
        // amplitude-independent, so preserve that scaling rather than clip IQ.
        self.spectrum[best] * self.center_phase[best]
    }
}

/// Bring the channel to its real IF clock before sliding FFT processing. A
/// persistent RF filter precedes fractional resampling; no bytes are produced.
struct FmIfStage {
    filter: ComplexChannelLowpass,
    reducer: FmIfNoiseReducer,
    rate: f64,
    previous: Option<Complex32>,
    input_index: u64,
    next_output: f64,
}

impl FmIfStage {
    fn new(mode: DemodMode, preset: FmIfPreset, input_rate: f64, bandwidth: f32) -> Self {
        let (bins, reference_rate): (usize, f64) = if mode == DemodMode::Wfm {
            (32, 250_000.0)
        } else {
            (
                match preset {
                    FmIfPreset::NoaaApt => 9,
                    FmIfPreset::Voice => 15,
                    FmIfPreset::NarrowBand => 31,
                },
                50_000.0,
            )
        };
        // Preserve unusually wide user channels; normal mode widths use the
        // installed reference's 50 kHz / 250 kHz IF clocks exactly.
        let rate = reference_rate
            .max(f64::from(bandwidth) * 1.25)
            .min(input_rate);
        Self {
            filter: ComplexChannelLowpass::default(),
            reducer: FmIfNoiseReducer::new(bins),
            rate,
            previous: None,
            input_index: 0,
            next_output: 0.0,
        }
    }

    fn process(
        &mut self,
        samples: &[Complex32],
        input_rate: f64,
        bandwidth: f32,
    ) -> Vec<Complex32> {
        let step = input_rate / self.rate;
        let cutoff = (bandwidth * 0.5).min(self.rate as f32 * 0.4);
        let mut output = Vec::with_capacity((samples.len() as f64 / step).ceil() as usize + 1);
        for sample in samples {
            let (i, q) = self
                .filter
                .process(sample.re, sample.im, cutoff, input_rate);
            let sample = Complex32::new(i, q);
            if let Some(previous) = self.previous {
                self.input_index += 1;
                while self.next_output <= self.input_index as f64 {
                    let fraction = (self.next_output - (self.input_index - 1) as f64) as f32;
                    output.push(self.reducer.push(previous + (sample - previous) * fraction));
                    self.next_output += step;
                }
            } else {
                output.push(self.reducer.push(sample));
                self.next_output = step;
            }
            self.previous = Some(sample);
        }
        output
    }
}

/// Rate-independent asymmetric envelope estimator. Unlike the legacy audio
/// gain servo, this observes signal magnitude before scaling it.
fn envelope_gain(
    magnitude: f32,
    envelope: &mut f64,
    rate: f64,
    rates: (f32, f32),
    target: f32,
) -> f32 {
    let speed = if f64::from(magnitude) > *envelope {
        rates.0
    } else {
        rates.1
    };
    let alpha = (f64::from(speed) / rate.max(1.0)).clamp(0.0, 1.0);
    *envelope += alpha * (f64::from(magnitude) - *envelope);
    (f64::from(target) / envelope.max(1e-9)).clamp(0.01, 40.0) as f32
}

pub struct Demodulator {
    prev_i: f32,
    prev_q: f32,
    prev_phase: f32,
    decimation: usize,
    decim_counter: usize,
    audio_sample_rate: u32,
    effective_output_rate: f64,
    lpf_cutoff: f32,
    lpf_state_l: f32,
    lpf_alpha: f32,
    rf_bandwidth_hz: f32,
    cw_tone_hz: f32,
    cw_osc_phase: f64,
    dsb_sideband: DsbSideband,
    cw_offset_hz: f32,
    cw_volume: f32,
    cw_squelch_enabled: bool,
    cw_squelch_level_db: f32,
    cw_squelch_envelope: f64,
    resampler: AudioResampler,
    stereo: Option<StereoFmDecoder>,
    stereo_right: Option<Box<Demodulator>>,
    pub last_stereo_locked: bool,
    pub last_fm_deviation_hz: f32,
    pub last_audio_peak: f32,
    source_rate: f64,
    input_rate: f64,
    // AGC state
    agc_gain: f32,
    pub agc_enabled: bool,
    agc_target: f32,
    agc_attack: f32,
    agc_decay: f32,
    agc_rates: Option<(f32, f32)>,
    agc_envelope: f64,
    carrier_agc_enabled: bool,
    carrier_envelope: f64,
    fm_lowpass_enabled: bool,
    fm_if_enabled: bool,
    fm_if_preset: FmIfPreset,
    fm_if_stage: Option<FmIfStage>,
    fm_if_settings: Option<(DemodMode, f64, f32)>,
    fm_audio_antialias: ComplexChannelLowpass,
    nfm_subaudible_audio: Vec<f32>,
    rds_tap_enabled: bool,
    wfm_multiplex: Vec<f32>,
    wfm_multiplex_rate: f64,
    nfm_subaudible_resampler: AudioResampler,
    // Persistent DSP state across demod chunks (avoids per-call transients).
    wfm_deemph_state: f32,
    ssb_osc_phase: f32,
    ssb_channel_lpf: ComplexChannelLowpass,
    am_channel_lpf: ComplexChannelLowpass,
    fm_channel_lpf: ComplexChannelLowpass,
    am_dc_prev_x: f32,
    am_dc_prev_y: f32,
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
    pitch_buffer: Vec<f32>,
    pitch_write_idx: usize,
    // ---- Advanced RF IQ pre-stage ----
    rf_dc_remove: bool,
    rf_noise_blanker: bool,
    rf_noise_blanker_level: f32,
    rf_notch: bool,
    rf_notch_hz: f32,
    rf_decim: u32,
    rf_decim_counter: u32,
    rf_decim_filter: ComplexChannelLowpass,
    rf_dc_estimate: [f64; 2],
    rf_nb_avg: f32,
    rf_notch_b: (f32, f32, f32, f32, f32),
    rf_notch_x1: (f32, f32),
    rf_notch_x2: (f32, f32),
    rf_notch_y1: (f32, f32),
    rf_notch_y2: (f32, f32),
}

impl Default for Demodulator {
    fn default() -> Self {
        Self::new()
    }
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
            effective_output_rate: 48000.0,
            lpf_cutoff: 15000.0,
            lpf_state_l: 0.0,
            lpf_alpha: 1.0,
            rf_bandwidth_hz: 0.0,
            cw_tone_hz: 700.0,
            cw_osc_phase: 0.0,
            dsb_sideband: DsbSideband::Both,
            cw_offset_hz: 0.0,
            cw_volume: 1.0,
            cw_squelch_enabled: false,
            cw_squelch_level_db: -60.0,
            cw_squelch_envelope: 0.0,
            resampler: AudioResampler::default(),
            stereo: None,
            stereo_right: None,
            last_stereo_locked: false,
            last_fm_deviation_hz: 0.0,
            last_audio_peak: 0.0,
            source_rate: 2_048_000.0,
            input_rate: 2_048_000.0,
            agc_gain: 1.0,
            agc_enabled: true,
            agc_target: 0.25,
            agc_attack: 0.01,
            agc_decay: 0.0001,
            agc_rates: None,
            agc_envelope: 0.25,
            carrier_agc_enabled: false,
            carrier_envelope: 1.0,
            fm_lowpass_enabled: true,
            fm_if_enabled: false,
            fm_if_preset: FmIfPreset::Voice,
            fm_if_stage: None,
            fm_if_settings: None,
            fm_audio_antialias: ComplexChannelLowpass::default(),
            nfm_subaudible_audio: Vec::new(),
            rds_tap_enabled: false,
            wfm_multiplex: Vec::new(),
            wfm_multiplex_rate: 0.0,
            nfm_subaudible_resampler: AudioResampler::default(),
            wfm_deemph_state: 0.0,
            ssb_osc_phase: 0.0,
            ssb_channel_lpf: ComplexChannelLowpass::default(),
            am_channel_lpf: ComplexChannelLowpass::default(),
            fm_channel_lpf: ComplexChannelLowpass::default(),
            am_dc_prev_x: 0.0,
            am_dc_prev_y: 0.0,
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
            pitch_buffer: vec![0.0; 2048],
            pitch_write_idx: 0,
            rf_dc_remove: false,
            rf_noise_blanker: false,
            rf_noise_blanker_level: 1.0,
            rf_notch: false,
            rf_notch_hz: 10_000.0,
            rf_decim: 1,
            rf_decim_counter: 0,
            rf_decim_filter: ComplexChannelLowpass::default(),
            rf_dc_estimate: [0.0; 2],
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
        let dt = 1.0 / self.effective_output_rate as f32;
        self.lpf_alpha = (dt / (rc + dt)).clamp(0.001, 1.0);
    }

    /// Set the RF channel's complete width in Hz, independently of the audio
    /// low-pass. Zero selects the current demodulation mode's default width.
    pub fn set_rf_bandwidth(&mut self, bandwidth_hz: f32) {
        self.rf_bandwidth_hz = if bandwidth_hz.is_finite() && bandwidth_hz > 0.0 {
            bandwidth_hz.clamp(50.0, 1_000_000.0)
        } else {
            0.0
        };
    }

    pub fn set_cw_tone(&mut self, tone_hz: f32) {
        if tone_hz.is_finite() {
            self.cw_tone_hz = tone_hz.clamp(100.0, 2_000.0);
        }
    }

    /// DSB sideband selection: Both (raw I), Upper (USB), or Lower (LSB).
    pub fn set_dsb_sideband(&mut self, sideband: DsbSideband) {
        if self.dsb_sideband != sideband {
            self.ssb_osc_phase = 0.0;
        }
        self.dsb_sideband = sideband;
    }

    /// CW beat frequency offset in Hz (added to the base tone).
    pub fn set_cw_offset(&mut self, offset: f32) {
        if offset.is_finite() {
            self.cw_offset_hz = offset.clamp(-2_000.0, 2_000.0);
        }
    }

    /// CW output volume scaling (0.0 to 1.0).
    pub fn set_cw_volume(&mut self, vol: f32) {
        self.cw_volume = vol.clamp(0.0, 1.0);
    }

    /// CW squelch: mute output when signal power drops below `level_db`.
    pub fn set_cw_squelch(&mut self, enabled: bool, level_db: f32) {
        self.cw_squelch_enabled = enabled;
        self.cw_squelch_level_db = level_db.clamp(-120.0, 0.0);
        self.cw_squelch_envelope = 0.0;
    }

    fn rf_bandwidth(&self, mode: DemodMode) -> f32 {
        if self.rf_bandwidth_hz > 0.0 {
            self.rf_bandwidth_hz
        } else {
            mode.default_rf_bandwidth_hz()
        }
    }

    pub fn set_sample_rates(&mut self, input_rate: u32, audio_rate: u32) {
        self.set_sample_rates_exact(f64::from(input_rate), audio_rate);
    }

    /// Preserve the actual clock after power-of-two source decimation, even
    /// when the source sample rate is not divisible by the decimation factor.
    pub fn set_sample_rates_exact(&mut self, input_rate: f64, audio_rate: u32) {
        let source_rate = if input_rate.is_finite() {
            input_rate.max(1.0)
        } else {
            1.0
        };
        let input_rate = (source_rate / f64::from(self.rf_decim)).max(1.0);
        let audio_rate = audio_rate.max(1);
        let decimation = (input_rate / f64::from(audio_rate)).floor().max(1.0) as usize;
        if self.source_rate != source_rate
            || self.input_rate != input_rate
            || self.audio_sample_rate != audio_rate
            || self.decimation != decimation
        {
            self.reset();
        }
        self.audio_sample_rate = audio_rate;
        self.source_rate = source_rate;
        self.input_rate = input_rate;
        self.decimation = decimation;
        self.effective_output_rate =
            source_rate / f64::from(self.rf_decim) / self.decimation as f64;
        self.set_lpf_cutoff(self.lpf_cutoff);
        self.recompute_audio_filters();
    }

    /// Recompute derived filter coefficients from the current user params.
    /// Call after changing sample rate or any filter setting.
    pub fn recompute_audio_filters(&mut self) {
        let fs = self.effective_output_rate as f32;
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
        let fs = self.source_rate as f32;
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
        self.agc_rates = None;
        self.agc_attack = v.clamp(0.0001, 1.0);
    }
    pub fn set_agc_decay(&mut self, v: f32) {
        self.agc_rates = None;
        self.agc_decay = v.clamp(0.00001, 1.0);
    }
    /// Select reference-style envelope AGC rates in inverse seconds, independent
    /// of the IQ/audio clock. Legacy coefficient setters select the older servo.
    pub fn set_agc_rates(&mut self, attack_per_second: f32, decay_per_second: f32) {
        if attack_per_second.is_finite() && decay_per_second.is_finite() {
            self.agc_rates = Some((
                attack_per_second.clamp(1.0, 200.0),
                decay_per_second.clamp(1.0, 20.0),
            ));
        }
    }

    /// AM only: choose gain from RF carrier magnitude before envelope detection
    /// instead of gain from the DC-blocked audio. `agc_enabled` remains the master.
    pub fn set_carrier_agc_enabled(&mut self, enabled: bool) {
        if self.carrier_agc_enabled != enabled {
            self.carrier_envelope = 1.0;
            self.agc_envelope = f64::from(self.agc_target);
            self.agc_gain = 1.0;
        }
        self.carrier_agc_enabled = enabled;
    }

    /// Bypass the user audio LPF in NFM/WFM, retaining compulsory antialiasing
    /// and the stereo multiplex/channel-separation filters.
    pub fn set_fm_lowpass_enabled(&mut self, enabled: bool) {
        self.fm_lowpass_enabled = enabled;
    }

    /// FM-only sliding FFT noise reduction. NFM exposes the three reference
    /// presets; WFM uses the reference's fixed 32-bin reconstruction.
    pub fn set_fm_if_noise_reduction(&mut self, enabled: bool, preset: FmIfPreset) {
        if self.fm_if_enabled != enabled || self.fm_if_preset != preset {
            self.fm_if_stage = None;
            self.fm_if_settings = None;
            self.fm_audio_antialias.reset();
            self.decim_counter = 0;
            self.resampler = AudioResampler::default();
            self.nfm_subaudible_resampler = AudioResampler::default();
            self.stereo = None;
            self.stereo_right = None;
        }
        self.fm_if_enabled = enabled;
        self.fm_if_preset = preset;
    }

    /// NFM discriminator audio before user processing, at the configured audio
    /// clock and with exactly the returned audible block's sample count. Each
    /// demodulation call replaces the tap; other modes and reset clear it.
    pub fn take_nfm_subaudible_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.nfm_subaudible_audio)
    }

    /// Collect the raw normalized WFM multiplex for an independent RDS decoder.
    /// Disabled taps allocate no multiplex output in the mono path.
    pub fn set_rds_tap_enabled(&mut self, enabled: bool) {
        self.rds_tap_enabled = enabled;
        if !enabled {
            self.wfm_multiplex.clear();
            self.wfm_multiplex_rate = 0.0;
        }
    }

    /// Multiplex before de-emphasis/audio filtering, normalized to 75 kHz peak
    /// deviation, plus its exact input clock. Empty for non-WFM or disabled tap.
    pub fn take_wfm_multiplex(&mut self) -> (Vec<f32>, f64) {
        (
            std::mem::take(&mut self.wfm_multiplex),
            self.wfm_multiplex_rate,
        )
    }

    /// FM de-emphasis in microseconds; zero is a real bypass (None).
    pub fn set_deemph_tau(&mut self, us: f32) {
        if us.is_finite() {
            self.deemph_tau_us = us.clamp(0.0, 200.0);
        }
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
        if self.rf_dc_remove != on {
            self.rf_dc_estimate = [0.0; 2];
        }
        self.rf_dc_remove = on;
    }
    pub fn set_rf_noise_blanker(&mut self, on: bool) {
        if self.rf_noise_blanker != on {
            self.rf_nb_avg = 0.0;
        }
        self.rf_noise_blanker = on;
    }
    /// Linear ratio to the streaming IF magnitude average, not calibrated dB.
    pub fn set_rf_noise_blanker_level(&mut self, level: f32) {
        if level.is_finite() {
            self.rf_noise_blanker_level = level.clamp(1.0, 10.0);
        }
    }
    pub fn set_rf_notch(&mut self, on: bool, hz: f32) {
        self.rf_notch = on;
        self.rf_notch_hz = hz.max(0.0);
        self.recompute_rf_notch();
    }
    pub fn set_rf_decim(&mut self, factor: u32) {
        let factor = factor.clamp(1, 64).min(self.source_rate.max(1.0) as u32);
        if self.rf_decim != factor {
            self.rf_decim = factor;
            self.reset();
            self.set_sample_rates_exact(self.source_rate, self.audio_sample_rate);
        }
    }

    // ---------------- RF IQ pre-stage ----------------

    fn bytes_to_complex(iq: &[u8]) -> Vec<Complex32> {
        iq.chunks_exact(2)
            .map(|pair| {
                Complex32::new(
                    (f32::from(pair[0]) - 127.4) / 128.0,
                    (f32::from(pair[1]) - 127.4) / 128.0,
                )
            })
            .collect()
    }

    fn iq_preprocess<'a>(&mut self, iq: &'a [Complex32]) -> Cow<'a, [Complex32]> {
        if !self.rf_dc_remove && !self.rf_noise_blanker && !self.rf_notch && self.rf_decim <= 1 {
            return Cow::Borrowed(iq);
        }
        let mut samples = iq.to_vec();
        if self.rf_dc_remove {
            // Compatibility path: continuous 5 Hz DC removal. The app disables
            // this when its source RadioIqProcessor already removes DC.
            let alpha = 1.0 - (-std::f64::consts::TAU * 5.0 / self.source_rate).exp();
            for sample in &mut samples {
                for (value, estimate) in [&mut sample.re, &mut sample.im]
                    .into_iter()
                    .zip(&mut self.rf_dc_estimate)
                {
                    *estimate += alpha * (f64::from(*value) - *estimate);
                    *value -= *estimate as f32;
                }
            }
        }
        if self.rf_noise_blanker {
            let alpha = (500.0 / self.source_rate).min(1.0) as f32;
            for sample in &mut samples {
                let magnitude = sample.norm();
                self.rf_nb_avg += alpha * (magnitude - self.rf_nb_avg);
                let ratio = magnitude / self.rf_nb_avg.max(1e-12);
                if ratio > self.rf_noise_blanker_level {
                    *sample /= ratio;
                }
            }
        }
        if self.rf_notch && self.rf_notch_b.0 > 0.0 {
            let (b0, b1, b2, a1, a2) = self.rf_notch_b;
            let (mut x1, mut x1q) = self.rf_notch_x1;
            let (mut x2, mut x2q) = self.rf_notch_x2;
            let (mut y1, mut y1q) = self.rf_notch_y1;
            let (mut y2, mut y2q) = self.rf_notch_y2;
            for sample in &mut samples {
                let xi = sample.re;
                let yi = b0 * xi + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                x2 = x1;
                x1 = xi;
                y2 = y1;
                y1 = yi;
                let xq = sample.im;
                let yq = b0 * xq + b1 * x1q + b2 * x2q - a1 * y1q - a2 * y2q;
                x2q = x1q;
                x1q = xq;
                y2q = y1q;
                y1q = yq;
                *sample = Complex32::new(yi, yq);
            }
            self.rf_notch_x1 = (x1, x1q);
            self.rf_notch_x2 = (x2, x2q);
            self.rf_notch_y1 = (y1, y1q);
            self.rf_notch_y2 = (y2, y2q);
        }
        if self.rf_decim > 1 {
            let mut output = Vec::with_capacity(samples.len() / self.rf_decim as usize + 1);
            let cutoff = self.source_rate as f32 / self.rf_decim as f32 * 0.35;
            for sample in samples {
                let (i, q) =
                    self.rf_decim_filter
                        .process(sample.re, sample.im, cutoff, self.source_rate);
                self.rf_decim_counter += 1;
                if self.rf_decim_counter >= self.rf_decim {
                    self.rf_decim_counter = 0;
                    output.push(Complex32::new(i, q));
                }
            }
            samples = output;
        }
        Cow::Owned(samples)
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

        // Pitch shift via dual-head crossfading delay line (preserves sample count & clock sync).
        if self.pitch_octaves.abs() > 1e-4 {
            let ratio = 2.0_f32.powf(self.pitch_octaves);
            let speed = 1.0 - ratio;
            let buf_len = self.pitch_buffer.len();
            let buf_len_f = buf_len as f32;
            let half_len = buf_len_f * 0.5;

            let mut out = Vec::with_capacity(samples.len());
            for &s in &samples {
                self.pitch_buffer[self.pitch_write_idx] = s;
                self.pitch_write_idx = (self.pitch_write_idx + 1) % buf_len;

                self.pitch_pos = (self.pitch_pos + speed).rem_euclid(buf_len_f);

                // Head 1
                let offset1 = self.pitch_pos;
                let frac1 = offset1 / buf_len_f;
                let w1 = if frac1 < 0.5 {
                    frac1 * 2.0
                } else {
                    (1.0 - frac1) * 2.0
                };

                // Head 2 (180 deg out of phase)
                let offset2 = (self.pitch_pos + half_len).rem_euclid(buf_len_f);
                let w2 = 1.0 - w1;

                let read_idx1 =
                    (self.pitch_write_idx as f32 + buf_len_f - offset1 - 1.0).rem_euclid(buf_len_f);
                let idx1_0 = read_idx1.floor() as usize % buf_len;
                let idx1_1 = (idx1_0 + 1) % buf_len;
                let frac_read1 = read_idx1.fract();
                let val1 = self.pitch_buffer[idx1_0]
                    + frac_read1 * (self.pitch_buffer[idx1_1] - self.pitch_buffer[idx1_0]);

                let read_idx2 =
                    (self.pitch_write_idx as f32 + buf_len_f - offset2 - 1.0).rem_euclid(buf_len_f);
                let idx2_0 = read_idx2.floor() as usize % buf_len;
                let idx2_1 = (idx2_0 + 1) % buf_len;
                let frac_read2 = read_idx2.fract();
                let val2 = self.pitch_buffer[idx2_0]
                    + frac_read2 * (self.pitch_buffer[idx2_1] - self.pitch_buffer[idx2_0]);

                out.push(w1 * val1 + w2 * val2);
            }
            samples = out;
        }

        samples
    }

    fn prepare_fm_if<'a>(
        &mut self,
        iq: Cow<'a, [Complex32]>,
        mode: DemodMode,
    ) -> (Cow<'a, [Complex32]>, Option<(f64, usize, f64)>) {
        if !self.fm_if_enabled || !matches!(mode, DemodMode::Fm | DemodMode::Wfm) {
            return (iq, None);
        }
        let bandwidth = self.rf_bandwidth(mode);
        let settings = (mode, self.input_rate, bandwidth);
        if self.fm_if_settings != Some(settings) {
            self.fm_if_stage = Some(FmIfStage::new(
                mode,
                self.fm_if_preset,
                self.input_rate,
                bandwidth,
            ));
            self.fm_if_settings = Some(settings);
            self.decim_counter = 0;
            self.resampler = AudioResampler::default();
            self.nfm_subaudible_resampler = AudioResampler::default();
            self.fm_channel_lpf.reset();
            self.fm_audio_antialias.reset();
            self.stereo = None;
            self.stereo_right = None;
        }
        let old_clock = (self.input_rate, self.decimation, self.effective_output_rate);
        let stage = self.fm_if_stage.as_mut().expect("FMIF configured above");
        let output = stage.process(&iq, self.input_rate, bandwidth);
        self.input_rate = stage.rate;
        self.decimation = (self.input_rate / f64::from(self.audio_sample_rate))
            .floor()
            .max(1.0) as usize;
        self.effective_output_rate = self.input_rate / self.decimation as f64;
        self.set_lpf_cutoff(self.lpf_cutoff);
        self.recompute_audio_filters();
        (Cow::Owned(output), Some(old_clock))
    }

    fn restore_fm_clock(&mut self, clock: Option<(f64, usize, f64)>) {
        if let Some((input_rate, decimation, effective_output_rate)) = clock {
            self.input_rate = input_rate;
            self.decimation = decimation;
            self.effective_output_rate = effective_output_rate;
        }
    }

    pub fn demodulate(&mut self, iq: &[u8], mode: DemodMode) -> Vec<f32> {
        self.demodulate_complex(&Self::bytes_to_complex(iq), mode)
    }

    /// Demodulate normalized floating-point I/Q without clipping or requantizing
    /// the output of source correction, decimation or digital VFO translation.
    pub fn demodulate_complex(&mut self, iq: &[Complex32], mode: DemodMode) -> Vec<f32> {
        self.last_stereo_locked = false;
        self.nfm_subaudible_audio.clear();
        self.wfm_multiplex.clear();
        self.wfm_multiplex_rate = 0.0;
        let iq = self.iq_preprocess(iq);
        let (iq, previous_clock) = self.prepare_fm_if(iq, mode);
        let mut samples = match mode {
            // Auto should be resolved to a concrete mode before reaching here
            // (see `DemodMode::resolve`); pass through if it ever slips by.
            DemodMode::Auto | DemodMode::Raw => self.demod_raw(&iq),
            DemodMode::Am => self.demod_am(&iq),
            DemodMode::Fm => self.demod_fm(&iq),
            DemodMode::Wfm => self.demod_wfm(&iq),
            DemodMode::Lsb => self.demod_ssb(&iq, false),
            DemodMode::Usb => self.demod_ssb(&iq, true),
            DemodMode::Dsb => self.demod_product(&iq, false),
            DemodMode::Cw => self.demod_product(&iq, true),
        };
        if let Some((source_clock, _, _)) = previous_clock {
            // Phase increments grow when IF is downsampled. Preserve the
            // established discriminator loudness when toggling FMIF.
            let scale = (self.input_rate / source_clock) as f32;
            for sample in samples.iter_mut().chain(&mut self.nfm_subaudible_audio) {
                *sample *= scale;
            }
        }
        let filtered = if matches!(mode, DemodMode::Fm | DemodMode::Wfm) && !self.fm_lowpass_enabled
        {
            samples
        } else {
            self.apply_lpf(samples)
        };
        let agc = if self.agc_enabled && !(mode == DemodMode::Am && self.carrier_agc_enabled) {
            self.apply_agc(filtered)
        } else {
            filtered
        };
        let processed = self.post_process(agc);
        let audio = if matches!(mode, DemodMode::Raw | DemodMode::Auto) {
            processed
        } else {
            self.resampler.process(
                processed,
                self.effective_output_rate,
                self.audio_sample_rate,
            )
        };
        self.restore_fm_clock(previous_clock);
        audio
    }

    /// Local broadcast-FM stereo, expressed as frames rather than interleaved
    /// samples. Other modes and sources too narrow to hold an FM multiplex
    /// return identical left/right channels through the existing mono path.
    pub fn demodulate_stereo(&mut self, iq: &[u8], mode: DemodMode) -> Vec<[f32; 2]> {
        self.demodulate_stereo_complex(&Self::bytes_to_complex(iq), mode)
    }

    /// Floating-point equivalent of `demodulate_stereo` with no byte conversion.
    pub fn demodulate_stereo_complex(
        &mut self,
        iq: &[Complex32],
        mode: DemodMode,
    ) -> Vec<[f32; 2]> {
        if mode != DemodMode::Wfm || self.input_rate < 120_000.0 {
            return self
                .demodulate_complex(iq, mode)
                .into_iter()
                .map(|sample| [sample, sample])
                .collect();
        }
        self.nfm_subaudible_audio.clear();
        self.wfm_multiplex.clear();
        self.wfm_multiplex_rate = 0.0;
        let iq = self.iq_preprocess(iq);
        let (iq, previous_clock) = self.prepare_fm_if(iq, mode);
        let cutoff = self.rf_bandwidth(DemodMode::Wfm) / 2.0;
        let discriminator_scale = self.input_rate as f32 / (std::f32::consts::TAU * 75_000.0);
        let mut multiplex = Vec::with_capacity(iq.len());
        for sample in iq.iter() {
            let i = sample.re;
            let q = sample.im;
            let (i, q) = self.fm_channel_lpf.process(i, q, cutoff, self.input_rate);
            let dot = i * self.prev_i + q * self.prev_q;
            let cross = q * self.prev_i - i * self.prev_q;
            multiplex.push(cross.atan2(dot) * discriminator_scale);
            self.prev_i = i;
            self.prev_q = q;
        }
        if self.rds_tap_enabled {
            self.wfm_multiplex.clone_from(&multiplex);
            self.wfm_multiplex_rate = self.input_rate;
        }
        let mut decoder = self.stereo.take().unwrap_or_default();
        let (mut left, mut right, rate) = decoder.process(
            &multiplex,
            self.input_rate,
            self.audio_sample_rate,
            self.deemph_tau_us,
        );
        self.last_stereo_locked = decoder.locked;
        self.stereo = Some(decoder);
        // Keep the established mono-WFM gain when the user toggles stereo.
        // MPX decoding uses normalized deviation for a consistent pilot lock
        // threshold, then restores the mono discriminator's output scale.
        let discriminator_clock = previous_clock.map_or(self.input_rate, |clock| clock.0);
        let mono_gain = std::f32::consts::TAU * 75_000.0 / discriminator_clock as f32 * 0.8;
        for sample in left.iter_mut().chain(&mut right) {
            *sample *= mono_gain;
        }
        self.effective_output_rate = rate;
        self.set_lpf_cutoff(self.lpf_cutoff);
        self.recompute_audio_filters();
        let mut right_chain = self
            .stereo_right
            .take()
            .unwrap_or_else(|| Box::new(Self::new()));
        right_chain.copy_audio_settings(self);
        let mut left = if self.fm_lowpass_enabled {
            self.apply_lpf(left)
        } else {
            left
        };
        let mut right = if self.fm_lowpass_enabled {
            right_chain.apply_lpf(right)
        } else {
            right
        };
        // Link gain between channels so the AGC cannot move the stereo image
        // or amplify the quiet channel's leakage independently.
        if self.agc_enabled {
            for (left, right) in left.iter_mut().zip(&mut right) {
                let peak = (left.abs().max(right.abs()) * self.agc_gain).max(1e-12);
                let gain = self.agc_gain;
                if peak > self.agc_target {
                    self.agc_gain *=
                        1.0 - self.agc_attack * (peak / self.agc_target - 1.0).min(1.0);
                } else {
                    self.agc_gain *= 1.0 + self.agc_decay;
                }
                self.agc_gain = self.agc_gain.clamp(0.1, 40.0);
                *left = (*left * gain).clamp(-1.0, 1.0);
                *right = (*right * gain).clamp(-1.0, 1.0);
            }
        }
        let left = self.post_process(left);
        let right = right_chain.post_process(right);
        let left = self
            .resampler
            .process(left, f64::from(rate), self.audio_sample_rate);
        let right = right_chain
            .resampler
            .process(right, f64::from(rate), self.audio_sample_rate);
        self.stereo_right = Some(right_chain);
        debug_assert_eq!(left.len(), right.len());
        self.last_audio_peak = left
            .iter()
            .chain(&right)
            .map(|sample| sample.abs())
            .fold(0.0, f32::max);
        self.restore_fm_clock(previous_clock);
        left.into_iter()
            .zip(right)
            .map(|(left, right)| [left, right])
            .collect()
    }

    fn copy_audio_settings(&mut self, source: &Self) {
        self.audio_sample_rate = source.audio_sample_rate;
        self.effective_output_rate = source.effective_output_rate;
        self.set_lpf_cutoff(source.lpf_cutoff);
        self.audio_hpf_hz = source.audio_hpf_hz;
        self.dc_blocker = source.dc_blocker;
        self.audio_gain = source.audio_gain;
        self.notch_hz = source.notch_hz;
        self.notch_width_hz = source.notch_width_hz;
        self.bass_db = source.bass_db;
        self.treble_db = source.treble_db;
        self.noise_blanker = source.noise_blanker;
        if self.pitch_octaves != source.pitch_octaves {
            self.set_pitch(source.pitch_octaves);
        }
        self.recompute_audio_filters();
    }

    fn apply_agc(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        // Soft-knee AGC with user-tunable target/attack/decay.
        const MAX_GAIN: f32 = 40.0;
        const MIN_GAIN: f32 = 0.1;
        if let Some(rates) = self.agc_rates {
            for sample in &mut samples {
                self.agc_gain = envelope_gain(
                    sample.abs(),
                    &mut self.agc_envelope,
                    self.effective_output_rate,
                    rates,
                    self.agc_target,
                );
                *sample = (*sample * self.agc_gain).clamp(-1.0, 1.0);
            }
            return samples;
        }

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

    fn demod_raw(&mut self, iq: &[Complex32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len());
        for sample in iq {
            let i = sample.re;
            let q = sample.im;
            out.push(i * 0.3);
            out.push(q * 0.3);
        }
        out
    }

    fn demod_am(&mut self, iq: &[Complex32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len() / self.decimation.max(1));
        let channel_cutoff = self.rf_bandwidth(DemodMode::Am) / 2.0;
        for sample in iq {
            let i = sample.re;
            let q = sample.im;
            // Filter complex RF before envelope detection and subsampling. Filtering only
            // the already-decimated audio cannot remove aliases folded into its passband.
            let (i, q) = self
                .am_channel_lpf
                .process(i, q, channel_cutoff, self.input_rate);
            let gain = if self.agc_enabled && self.carrier_agc_enabled {
                envelope_gain(
                    i.hypot(q),
                    &mut self.carrier_envelope,
                    self.input_rate,
                    self.agc_rates.unwrap_or((50.0, 5.0)),
                    1.0,
                )
            } else {
                1.0
            };
            let env = (i * gain).hypot(q * gain);
            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                // AM envelopes contain a large carrier/DC term. Block it by default so a
                // steady carrier settles to silence while modulation remains audible.
                let audio = env - self.am_dc_prev_x + 0.995 * self.am_dc_prev_y;
                self.am_dc_prev_x = env;
                self.am_dc_prev_y = audio;
                out.push(audio);
            }
        }
        out
    }

    fn demod_fm(&mut self, iq: &[Complex32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len() / self.decimation.max(1));
        let mut max_diff: f32 = 0.0;
        let mut subaudible = Vec::with_capacity(out.capacity());
        let cutoff = self.audio_sample_rate as f32 * 0.4;
        let dt = 1.0 / self.input_rate as f32;
        let deemph_alpha = dt / (self.deemph_tau_us * 1e-6 + dt);
        let channel_cutoff = self.rf_bandwidth(DemodMode::Fm) / 2.0;
        for sample in iq {
            let i = sample.re;
            let q = sample.im;

            // A complex channel filter must precede the non-linear discriminator and
            // decimator; an audio LPF after raw subsampling is too late to prevent aliasing.
            let (i, q) = self
                .fm_channel_lpf
                .process(i, q, channel_cutoff, self.input_rate);
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
            let filtered = self
                .fm_audio_antialias
                .process(diff, 0.0, cutoff, self.input_rate)
                .0;
            self.wfm_deemph_state += deemph_alpha * (filtered - self.wfm_deemph_state);
            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                subaudible.push(filtered * 0.5);
                out.push(self.wfm_deemph_state * 0.5);
            }
        }
        self.nfm_subaudible_audio = self.nfm_subaudible_resampler.process(
            subaudible,
            self.effective_output_rate,
            self.audio_sample_rate,
        );
        // FM deviation = max_phase_diff * sample_rate / (2π)
        if self.input_rate > 0.0 {
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

    fn demod_wfm(&mut self, iq: &[Complex32]) -> Vec<f32> {
        // Wide FM demodulation with correct de-emphasis. Time constant τ is
        // user-selectable (50 µs EU / 75 µs US). τ = 1/(2π·f_c).
        // Discrete IIR: alpha = dt/(τ + dt) where dt = 1/sample_rate
        let tau = self.deemph_tau_us * 1e-6_f32;
        // The de-emphasis IIR advances once per input IQ pair, so dt must use
        // the input sample rate (≈2.048 MHz), NOT the audio rate. Using the
        // audio rate here (≈48 kHz) made alpha ~0.294 instead of the correct
        // ≈0.00967, moving the 50 µs pole and leaving WFM audio
        // harsh with no bass restoration.
        let dt = 1.0 / self.input_rate as f32;
        let alpha = dt / (tau + dt);
        if self.rds_tap_enabled {
            self.wfm_multiplex.reserve(iq.len());
            self.wfm_multiplex_rate = self.input_rate;
        }
        let mut out = Vec::with_capacity(iq.len() / self.decimation.max(1));
        let channel_cutoff = self.rf_bandwidth(DemodMode::Wfm) / 2.0;

        for sample in iq {
            let i = sample.re;
            let q = sample.im;

            let (i, q) = self
                .fm_channel_lpf
                .process(i, q, channel_cutoff, self.input_rate);

            let phase = q.atan2(i);
            let mut diff = phase - self.prev_phase;
            while diff > std::f32::consts::PI {
                diff -= 2.0 * std::f32::consts::PI;
            }
            while diff < -std::f32::consts::PI {
                diff += 2.0 * std::f32::consts::PI;
            }

            self.prev_phase = phase;
            if self.rds_tap_enabled {
                self.wfm_multiplex
                    .push(diff * self.input_rate as f32 / (std::f32::consts::TAU * 75_000.0));
            }

            // 1st-order IIR low-pass de-emphasis: y[n] = y[n-1] + α*(x[n] - y[n-1]).
            // State persists across chunks (field) to avoid a settling transient
            // at every source-buffer boundary.
            self.wfm_deemph_state += alpha * (diff - self.wfm_deemph_state);
            let audio = self
                .fm_audio_antialias
                .process(
                    self.wfm_deemph_state,
                    0.0,
                    self.audio_sample_rate as f32 * 0.4,
                    self.input_rate,
                )
                .0;

            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(audio * 0.8);
            }
        }
        out
    }

    fn demod_ssb(&mut self, iq: &[Complex32], usb: bool) -> Vec<f32> {
        // Select [0, width] for USB or [-width, 0] for LSB by translating that
        // passband to DC, filtering, then translating it back. Restoring the
        // shift is essential: otherwise every received voice tone changes pitch.
        let mut out = Vec::with_capacity(iq.len() / self.decimation.max(1));
        let mode = if usb { DemodMode::Usb } else { DemodMode::Lsb };
        let shift_hz = self.rf_bandwidth(mode) / 2.0;
        let shift_rad = 2.0 * std::f32::consts::PI * shift_hz / self.input_rate.max(1.0) as f32;
        let sign = if usb { -1.0 } else { 1.0 };

        for sample in iq {
            let i = sample.re;
            let q = sample.im;

            let angle = sign * self.ssb_osc_phase;
            self.ssb_osc_phase += shift_rad;
            if self.ssb_osc_phase >= 2.0 * std::f32::consts::PI {
                self.ssb_osc_phase -= 2.0 * std::f32::consts::PI;
            }
            let (sin, cos) = angle.sin_cos();
            let i_shift = i * cos - q * sin;
            let q_shift = i * sin + q * cos;
            let (i_filtered, q_filtered) =
                self.ssb_channel_lpf
                    .process(i_shift, q_shift, shift_hz, self.input_rate);

            self.decim_counter += 1;
            if self.decim_counter >= self.decimation {
                self.decim_counter = 0;
                out.push(i_filtered * cos + q_filtered * sin);
            }
        }
        out
    }

    fn demod_product(&mut self, iq: &[Complex32], cw: bool) -> Vec<f32> {
        // DSB with a sideband selection delegates to the SSB demodulator.
        if !cw && self.dsb_sideband != DsbSideband::Both {
            let usb = self.dsb_sideband == DsbSideband::Upper;
            return self.demod_ssb(iq, usb);
        }
        let mode = if cw { DemodMode::Cw } else { DemodMode::Dsb };
        let cutoff = self.rf_bandwidth(mode) / 2.0;
        let mut out = Vec::with_capacity(iq.len() / self.decimation.max(1));
        let beat = self.cw_tone_hz + self.cw_offset_hz;
        let beat_step =
            std::f64::consts::TAU * f64::from(beat) / f64::from(self.effective_output_rate);
        for sample in iq.iter() {
            let i = sample.re;
            let q = sample.im;
            // Filter around the tuned RF carrier before adding the CW beat
            // frequency. A narrow 500 Hz RF channel can therefore emit 700 Hz
            // audio while rejecting a carrier a few kHz away.
            let (i, q) = self.am_channel_lpf.process(i, q, cutoff, self.input_rate);
            self.decim_counter += 1;
            if self.decim_counter < self.decimation {
                continue;
            }
            self.decim_counter = 0;
            let mut sample = if cw {
                let (sin, cos) = self.cw_osc_phase.sin_cos();
                self.cw_osc_phase =
                    (self.cw_osc_phase + beat_step).rem_euclid(std::f64::consts::TAU);
                i * cos as f32 - q * sin as f32
            } else {
                // Coherent DSB product detection retains the signed message;
                // taking its magnitude would rectify it and double tone pitch.
                i
            };
            if cw && self.cw_volume != 1.0 {
                sample *= self.cw_volume;
            }
            let mut audio = sample - self.am_dc_prev_x + 0.995 * self.am_dc_prev_y;
            self.am_dc_prev_x = sample;
            self.am_dc_prev_y = audio;
            if cw && self.cw_squelch_enabled {
                let power = f64::from(audio * audio);
                let alpha = 1.0 - (-std::f64::consts::TAU * 5.0 / self.effective_output_rate).exp();
                self.cw_squelch_envelope += alpha * (power - self.cw_squelch_envelope);
                let threshold = 10.0_f64.powf(f64::from(self.cw_squelch_level_db) / 10.0);
                if self.cw_squelch_envelope < threshold {
                    audio = 0.0;
                }
            }
            out.push(audio);
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
        self.cw_osc_phase = 0.0;
        self.cw_squelch_envelope = 0.0;
        self.resampler = AudioResampler::default();
        self.nfm_subaudible_resampler = AudioResampler::default();
        self.nfm_subaudible_audio.clear();
        self.wfm_multiplex.clear();
        self.wfm_multiplex_rate = 0.0;
        self.fm_audio_antialias.reset();
        self.fm_if_stage = None;
        self.fm_if_settings = None;
        self.agc_envelope = f64::from(self.agc_target);
        self.carrier_envelope = 1.0;
        self.agc_gain = 1.0;
        self.stereo = None;
        self.stereo_right = None;
        self.last_stereo_locked = false;
        self.ssb_channel_lpf.reset();
        self.am_channel_lpf.reset();
        self.fm_channel_lpf.reset();
        self.am_dc_prev_x = 0.0;
        self.am_dc_prev_y = 0.0;
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
        self.pitch_buffer.fill(0.0);
        self.pitch_write_idx = 0;
        self.rf_nb_avg = 0.0;
        self.rf_dc_estimate = [0.0; 2];
        self.rf_decim_counter = 0;
        self.rf_decim_filter.reset();
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

    fn complex_fm(rate: f64, count: usize, frequency: f64, deviation: f64) -> Vec<Complex32> {
        (0..count)
            .map(|n| {
                let phase = deviation / frequency
                    * (std::f64::consts::TAU * frequency * n as f64 / rate).sin();
                Complex32::new(phase.cos() as f32 * 0.6, phase.sin() as f32 * 0.6)
            })
            .collect()
    }

    fn tone_snr(audio: &[f32], frequency: f64, rate: u32) -> f64 {
        let signal = tone_amplitude(audio, frequency, rate).powi(2) * 0.5;
        let power = audio.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / audio.len() as f64;
        signal / (power - signal).max(1e-15)
    }

    #[test]
    fn carrier_agc_tracks_rf_strength_but_preserves_am_modulation_depth() {
        let measure = |carrier: f64, depth: f64, carrier_agc: bool| {
            let rate = 96_000;
            let iq: Vec<_> = (0..rate * 2)
                .map(|n| {
                    Complex32::new(
                        (carrier
                            * (1.0
                                + depth
                                    * (std::f64::consts::TAU * 1000.0 * f64::from(n)
                                        / f64::from(rate))
                                    .cos())) as f32,
                        0.0,
                    )
                })
                .collect();
            let mut demod = configured_demod(rate, 8000.0);
            demod.set_agc_enabled(true);
            demod.set_agc_rates(50.0, 5.0);
            demod.set_carrier_agc_enabled(carrier_agc);
            let audio = demod.demodulate_complex(&iq, DemodMode::Am);
            tone_amplitude(&audio[72_000..], 1000.0, 48_000)
        };
        let weak = measure(0.1, 0.2, true);
        let strong = measure(0.7, 0.2, true);
        assert!(
            (weak / strong - 1.0).abs() < 0.03,
            "carrier gain {weak}/{strong}"
        );
        let deep = measure(0.1, 0.7, true);
        assert!(
            deep > weak * 2.0,
            "carrier AGC erased modulation depth: {deep}/{weak}"
        );
        let audio_shallow = measure(0.1, 0.2, false);
        let audio_deep = measure(0.1, 0.7, false);
        assert!(
            (audio_shallow / audio_deep - 1.0).abs() < 0.05,
            "audio AGC must observe modulation, not carrier: {audio_shallow}/{audio_deep}"
        );
    }

    #[test]
    fn carrier_agc_is_rate_independent_and_preserves_stream_state() {
        let mut levels = Vec::new();
        for rate in [96_000, 384_000] {
            let source: Vec<_> = (0..rate)
                .map(|n| {
                    let t = f64::from(n) / f64::from(rate);
                    let carrier = if t < 0.3 { 0.6 } else { 0.15 };
                    Complex32::new(
                        (carrier * (1.0 + 0.4 * (std::f64::consts::TAU * 1000.0 * t).cos())) as f32,
                        0.0,
                    )
                })
                .collect();
            let create = || {
                let mut demod = configured_demod(rate, 8000.0);
                demod.set_agc_enabled(true);
                demod.set_agc_rates(50.0, 5.0);
                demod.set_carrier_agc_enabled(true);
                demod
            };
            let reference = create().demodulate_complex(&source, DemodMode::Am);
            let mut fragmented = create();
            let actual: Vec<_> = source
                .chunks(719)
                .flat_map(|chunk| fragmented.demodulate_complex(chunk, DemodMode::Am))
                .collect();
            assert_eq!(actual, reference);
            fragmented.reset();
            fragmented.set_sample_rates(rate, 48_000);
            assert_eq!(
                fragmented.demodulate_complex(&source, DemodMode::Am),
                reference
            );
            levels.push(tone_amplitude(&reference[36_000..], 1000.0, 48_000));
        }
        assert!(
            (levels[0] / levels[1] - 1.0).abs() < 0.01,
            "carrier time constants changed with clock: {levels:?}"
        );
    }

    #[test]
    fn fm_lowpass_bypass_is_real_and_keeps_antialias_protection() {
        let measure = |frequency: f64, enabled: bool| {
            let mut demod = configured_demod(384_000, 150_000.0);
            demod.set_lpf_cutoff(500.0);
            demod.set_fm_lowpass_enabled(enabled);
            demod.set_deemph_tau(0.0);
            let audio = demod.demodulate_complex(
                &complex_fm(384_000.0, 192_000, frequency, 1000.0),
                DemodMode::Wfm,
            );
            tone_amplitude(&audio[4800..], 8000.0, 48_000)
        };
        let bypass = measure(8000.0, false);
        let filtered = measure(8000.0, true);
        let alias = measure(40_000.0, false);
        assert!(
            bypass > filtered * 10.0,
            "LPF toggle has no real effect {bypass}/{filtered}"
        );
        assert!(
            alias < bypass * 0.02,
            "LPF bypass admitted aliased multiplex: {alias}/{bypass}"
        );
    }

    #[test]
    fn fm_deemphasis_supports_none_22_50_and_75_microseconds() {
        for mode in [DemodMode::Fm, DemodMode::Wfm] {
            let source = complex_fm(192_000.0, 96_000, 4000.0, 500.0);
            let measure = |tau| {
                let mut demod = configured_demod(192_000, 25_000.0);
                demod.set_fm_lowpass_enabled(false);
                demod.set_deemph_tau(tau);
                let audio = demod.demodulate_complex(&source, mode);
                tone_amplitude(&audio[4800..], 4000.0, 48_000)
            };
            let levels = [0.0, 22.0, 50.0, 75.0].map(measure);
            assert!(
                levels.windows(2).all(|pair| pair[0] > pair[1] * 1.08),
                "{mode:?}: {levels:?}"
            );
        }
    }

    #[test]
    fn fmif_uses_reference_presets_and_single_bin_inverse_center() {
        for (preset, bins) in [
            (FmIfPreset::NoaaApt, 9),
            (FmIfPreset::Voice, 15),
            (FmIfPreset::NarrowBand, 31),
        ] {
            assert_eq!(
                FmIfStage::new(DemodMode::Fm, preset, 192_000.0, 12_500.0)
                    .reducer
                    .window
                    .len(),
                bins
            );
            assert_eq!(
                FmIfStage::new(DemodMode::Wfm, preset, 384_000.0, 150_000.0)
                    .reducer
                    .window
                    .len(),
                32
            );
            let mut reducer = FmIfNoiseReducer::new(bins);
            let mut last = Complex32::new(0.0, 0.0);
            for n in 0..bins * 2 {
                last = reducer.push(Complex32::from_polar(
                    0.6,
                    std::f32::consts::TAU * n as f32 * 2.0 / bins as f32,
                ));
            }
            let expected = reducer.spectrum[2]
                * Complex32::from_polar(
                    1.0,
                    std::f32::consts::TAU * 2.0 * (bins / 2) as f32 / bins as f32,
                );
            assert!((last - expected).norm() < 2e-5);
            assert_eq!(reducer.window.len(), bins);
        }
    }

    #[test]
    fn fmif_improves_noisy_nfm_and_keeps_tone_gain_when_enabled() {
        let clean = complex_fm(192_000.0, 192_000, 1000.0, 1000.0);
        let mut random = 0x8173_9371_u32;
        let mut noise = || {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            (f64::from(random) / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * 0.45
        };
        let noisy: Vec<_> = clean
            .iter()
            .map(|sample| *sample + Complex32::new(noise(), noise()))
            .collect();
        let demodulate = |input: &[Complex32], enabled| {
            let mut demod = configured_demod(192_000, 12_500.0);
            demod.set_deemph_tau(0.0);
            demod.set_fm_if_noise_reduction(enabled, FmIfPreset::Voice);
            demod.demodulate_complex(input, DemodMode::Fm)
        };
        let raw = demodulate(&noisy, false);
        let reduced = demodulate(&noisy, true);
        let raw_snr = tone_snr(&raw[12_000..], 1000.0, 48_000);
        let reduced_snr = tone_snr(&reduced[12_000..], 1000.0, 48_000);
        assert!(
            reduced_snr > raw_snr * 1.25,
            "NR failed SNR {reduced_snr}/{raw_snr}"
        );
        let off = demodulate(&clean, false);
        let on = demodulate(&clean, true);
        let gain = tone_amplitude(&on[12_000..], 1000.0, 48_000)
            / tone_amplitude(&off[12_000..], 1000.0, 48_000);
        assert!(
            (0.8..1.1).contains(&gain),
            "IF clock changed loudness: {gain}"
        );
    }

    #[test]
    fn new_fm_controls_and_taps_preserve_chunk_counts_reset_and_precision() {
        let source = complex_fm(192_003.0, 96_002, 100.0, 400.0);
        for nr in [false, true] {
            let create = || {
                let mut demod = configured_demod(192_003, 12_500.0);
                demod.set_sample_rates(192_003, 44_100);
                demod.set_fm_if_noise_reduction(nr, FmIfPreset::NarrowBand);
                demod.set_audio_hpf(600.0);
                demod.set_dc_blocker(1.0);
                demod.set_notch(100.0, 20.0);
                demod.set_pitch(0.5);
                demod.set_audio_gain(0.0);
                demod
            };
            let mut whole = create();
            let reference = whole.demodulate_complex(&source, DemodMode::Fm);
            let reference_tap = whole.take_nfm_subaudible_audio();
            assert_eq!(reference.len(), reference_tap.len());
            assert!(reference.iter().all(|sample| *sample == 0.0));
            assert!(tone_amplitude(&reference_tap[4400..], 100.0, 44_100) > 0.004);
            let mut fragmented = create();
            let mut audio = Vec::new();
            let mut tap = Vec::new();
            for chunk in source.chunks(701) {
                fragmented.set_sample_rates(192_003, 44_100);
                let block = fragmented.demodulate_complex(chunk, DemodMode::Fm);
                let block_tap = fragmented.take_nfm_subaudible_audio();
                assert_eq!(block.len(), block_tap.len());
                audio.extend(block);
                tap.extend(block_tap);
            }
            assert_eq!(audio, reference);
            assert_eq!(tap, reference_tap);
            fragmented.reset();
            fragmented.set_sample_rates(192_003, 44_100);
            assert_eq!(
                fragmented.demodulate_complex(&source, DemodMode::Fm),
                reference
            );
            assert_eq!(fragmented.take_nfm_subaudible_audio(), reference_tap);
            fragmented.demodulate_complex(&source[..32], DemodMode::Am);
            assert!(fragmented.take_nfm_subaudible_audio().is_empty());
        }
    }

    #[test]
    fn rds_tap_preserves_57khz_before_audio_filters_and_reports_exact_clock() {
        let source = complex_fm(384_000.0, 153_600, 57_000.0, 3000.0);
        for stereo in [false, true] {
            let mut demod = configured_demod(384_000, 150_000.0);
            demod.set_rds_tap_enabled(true);
            demod.set_audio_gain(0.0);
            demod.set_lpf_cutoff(100.0);
            demod.set_deemph_tau(75.0);
            if stereo {
                demod.demodulate_stereo_complex(&source, DemodMode::Wfm);
            } else {
                demod.demodulate_complex(&source, DemodMode::Wfm);
            }
            let (tap, rate) = demod.take_wfm_multiplex();
            assert_eq!(rate, 384_000.0);
            assert_eq!(tap.len(), source.len());
            assert!(tone_amplitude(&tap[38_400..], 57_000.0, 384_000) > 0.03);
            demod.set_rds_tap_enabled(false);
            demod.demodulate_complex(&source[..100], DemodMode::Wfm);
            assert!(demod.take_wfm_multiplex().0.is_empty());
            demod.set_rds_tap_enabled(true);
            demod.demodulate_complex(&source[..100], DemodMode::Am);
            assert!(demod.take_wfm_multiplex().0.is_empty());
        }
    }

    #[test]
    fn wfm_stereo_if_reduction_preserves_clock_taps_and_chunk_state() {
        let bytes = synthetic_stereo_fm(384_000, 0.35, 19_000.0, 0.4, 0.1, 0.0);
        let source = Demodulator::bytes_to_complex(&bytes);
        let create = || {
            let mut demod = configured_demod(384_000, 150_000.0);
            demod.set_sample_rates(384_000, 44_100);
            demod.set_fm_if_noise_reduction(true, FmIfPreset::Voice);
            demod.set_fm_lowpass_enabled(false);
            demod.set_deemph_tau(22.0);
            demod.set_rds_tap_enabled(true);
            demod
        };
        let mut whole = create();
        let reference = whole.demodulate_stereo_complex(&source, DemodMode::Wfm);
        let (reference_tap, rate) = whole.take_wfm_multiplex();
        assert_eq!(rate, 250_000.0);
        assert!(reference.len().abs_diff(15_435) <= 1);
        assert!(reference.iter().flatten().all(|value| value.is_finite()));
        let mut fragmented = create();
        let mut actual = Vec::new();
        let mut taps = Vec::new();
        for chunk in source.chunks(683) {
            fragmented.set_sample_rates(384_000, 44_100);
            actual.extend(fragmented.demodulate_stereo_complex(chunk, DemodMode::Wfm));
            let (tap, rate) = fragmented.take_wfm_multiplex();
            assert_eq!(rate, 250_000.0);
            taps.extend(tap);
        }
        assert_eq!(actual, reference);
        assert_eq!(taps, reference_tap);
        fragmented.reset();
        fragmented.set_sample_rates(384_000, 44_100);
        assert_eq!(
            fragmented.demodulate_stereo_complex(&source, DemodMode::Wfm),
            reference
        );
        assert_eq!(fragmented.take_wfm_multiplex().0, reference_tap);
    }

    #[test]
    fn rf_blanker_level_controls_real_impulse_suppression_before_demodulation() {
        let mut source = vec![Complex32::new(0.2, 0.0); 48_000];
        source[24_000] = Complex32::new(1.5, 0.0);
        let measure = |level| {
            let mut demod = configured_demod(48_000, 20_000.0);
            demod.set_lpf_cutoff(20_000.0);
            demod.set_rf_noise_blanker(true);
            demod.set_rf_noise_blanker_level(level);
            let audio = demod.demodulate_complex(&source, DemodMode::Raw);
            audio[48_000..48_010]
                .iter()
                .map(|sample| sample.abs())
                .fold(0.0, f32::max)
        };
        let limited = measure(2.0);
        let passed = measure(10.0);
        assert!(
            limited < passed * 0.3,
            "blanker level ignored: {limited}/{passed}"
        );
    }

    #[test]
    fn complex_and_byte_apis_match_for_every_mode_and_stereo() {
        let rate = 384_000;
        let source = synth_iq(rate, 38_400, |time| {
            let phase = std::f64::consts::TAU * 1700.0 * time;
            (0.4 * phase.cos(), 0.4 * phase.sin())
        });
        let complex = Demodulator::bytes_to_complex(&source);
        for mode in [
            DemodMode::Auto,
            DemodMode::Raw,
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Wfm,
            DemodMode::Usb,
            DemodMode::Lsb,
            DemodMode::Dsb,
            DemodMode::Cw,
        ] {
            let create = || {
                let mut demod = configured_demod(rate, mode.default_rf_bandwidth_hz());
                demod.set_rf_dc_remove(true);
                demod.set_rf_notch(true, 12_000.0);
                demod.set_rf_noise_blanker(true);
                demod
            };
            assert_eq!(
                create().demodulate(&source, mode),
                create().demodulate_complex(&complex, mode),
                "{mode:?}"
            );
            assert_eq!(
                create().demodulate_stereo(&source, mode),
                create().demodulate_stereo_complex(&complex, mode),
                "stereo {mode:?}"
            );
        }
    }

    #[test]
    fn complex_input_preserves_signals_far_below_an_eight_bit_quantization_step() {
        let rate = 192_000;
        let source: Vec<_> = (0..48_000)
            .map(|n| {
                let angle = std::f64::consts::TAU * 1000.0 * n as f64 / f64::from(rate);
                Complex32::new(angle.cos() as f32 * 1e-5, angle.sin() as f32 * 1e-5)
            })
            .collect();
        let mut demod = configured_demod(rate, 8000.0);
        demod.set_rf_decim(2);
        let audio = demod.demodulate_complex(&source, DemodMode::Dsb);
        let amplitude = tone_amplitude(&audio[2400..], 1000.0, 48_000);
        assert!(
            amplitude > 9e-6 && amplitude < 1.1e-5,
            "lost float precision: {amplitude}"
        );
    }

    #[test]
    fn fractional_source_clock_preserves_exact_audio_counts_pitch_and_chunk_state() {
        let rate = 192_003.0 / 4.0;
        let source = vec![Complex32::new(0.6, 0.0); 192_003];
        let create = || {
            let mut demod = configured_demod(48_000, 500.0);
            demod.set_sample_rates_exact(rate, 44_100);
            demod
        };
        let reference = create().demodulate_complex(&source, DemodMode::Cw);
        let mut demod = create();
        let actual: Vec<_> = source
            .chunks(701)
            .flat_map(|chunk| {
                demod.set_sample_rates_exact(rate, 44_100);
                demod.demodulate_complex(chunk, DemodMode::Cw)
            })
            .collect();
        let expected_count = (((source.len() - 1) as f64 * 44_100.0 / rate).floor() as usize) + 1;
        assert_eq!(actual.len(), expected_count);
        assert_eq!(actual, reference);
        assert_eq!(actual.len(), 176_400);
        assert!(tone_amplitude(&actual[4400..], 700.0, 44_100) > 0.5);
    }

    fn make_iq_dc(i_val: u8, q_val: u8, count: usize) -> Vec<u8> {
        // Constant IQ = same byte pair repeated
        let mut v = Vec::with_capacity(count * 2);
        for _ in 0..count {
            v.push(i_val);
            v.push(q_val);
        }
        v
    }

    fn synth_iq(rate: u32, count: usize, signal: impl Fn(f64) -> (f64, f64)) -> Vec<u8> {
        (0..count)
            .flat_map(|index| {
                let (i, q) = signal(index as f64 / f64::from(rate));
                [i, q].map(|sample| (sample * 128.0 + 127.4).round().clamp(0.0, 255.0) as u8)
            })
            .collect()
    }

    fn tone_amplitude(audio: &[f32], frequency: f64, rate: u32) -> f64 {
        let mut real = 0.0;
        let mut imaginary = 0.0;
        for (index, sample) in audio.iter().enumerate() {
            let phase = std::f64::consts::TAU * frequency * index as f64 / f64::from(rate);
            real += f64::from(*sample) * phase.cos();
            imaginary += f64::from(*sample) * phase.sin();
        }
        2.0 * real.hypot(imaginary) / audio.len() as f64
    }

    fn configured_demod(rate: u32, bandwidth: f32) -> Demodulator {
        let mut demod = Demodulator::new();
        demod.set_sample_rates(rate, 48_000);
        demod.set_agc_enabled(false);
        demod.set_lpf_cutoff(15_000.0);
        demod.set_rf_bandwidth(bandwidth);
        demod
    }

    #[test]
    fn dsb_recovers_signed_modulation_without_envelope_frequency_doubling() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |time| {
            (0.6 * (std::f64::consts::TAU * 1_000.0 * time).cos(), 0.0)
        });
        let mut demod = configured_demod(rate, 8_000.0);
        let audio = demod.demodulate(&iq, DemodMode::Dsb);
        let settled = &audio[2_400..];
        let fundamental = tone_amplitude(settled, 1_000.0, 48_000);
        let harmonic = tone_amplitude(settled, 2_000.0, 48_000);
        assert!(
            fundamental > 0.5,
            "DSB lost signed modulation: {fundamental}"
        );
        assert!(
            harmonic < fundamental * 0.01,
            "DSB rectified the signal: {harmonic}"
        );
        assert!(settled.iter().any(|sample| *sample < -0.4));
        assert!(settled.iter().any(|sample| *sample > 0.4));
    }

    #[test]
    fn rf_bandwidth_and_audio_cutoff_independently_reject_tones() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |time| {
            (0.6 * (std::f64::consts::TAU * 4_000.0 * time).cos(), 0.0)
        });
        let wide = configured_demod(rate, 12_000.0);
        let narrow_rf = configured_demod(rate, 2_000.0);
        let mut narrow_audio = configured_demod(rate, 12_000.0);
        narrow_audio.set_lpf_cutoff(500.0);
        let measure = |mut demod: Demodulator| {
            let output = demod.demodulate(&iq, DemodMode::Dsb);
            tone_amplitude(&output[2_400..], 4_000.0, 48_000)
        };
        let pass = measure(wide);
        let rejected_rf = measure(narrow_rf);
        let rejected_audio = measure(narrow_audio);
        assert!(pass > 0.4, "wide channel should pass 4 kHz: {pass}");
        assert!(
            rejected_rf < pass * 0.01,
            "RF filter leaked adjacent signal: {rejected_rf}"
        );
        assert!(
            rejected_audio < pass * 0.2,
            "audio LPF did not act independently: {rejected_audio}"
        );
    }

    #[test]
    fn cw_tone_is_added_after_narrow_rf_filter_and_honors_setting() {
        let rate = 192_000;
        for (beat, carrier_offset) in [(700.0, 0.0), (1_200.0, 100.0), (500.0, -100.0)] {
            let iq = synth_iq(rate, rate as usize / 4, |time| {
                let phase = std::f64::consts::TAU * carrier_offset * time;
                (0.6 * phase.cos(), 0.6 * phase.sin())
            });
            let mut demod = configured_demod(rate, 500.0);
            demod.set_cw_tone(beat as f32);
            let audio = demod.demodulate(&iq, DemodMode::Cw);
            let tone = tone_amplitude(&audio[2_400..], beat + carrier_offset, 48_000);
            assert!(
                tone > 0.5,
                "CW tone {beat}+{carrier_offset} Hz missing: {tone}"
            );
        }
    }

    #[test]
    fn cw_rejects_adjacent_carriers_before_beat_mixing() {
        let rate = 192_000;
        let measure = |offset: f64| {
            let iq = synth_iq(rate, rate as usize / 4, |time| {
                let phase = std::f64::consts::TAU * offset * time;
                (0.6 * phase.cos(), 0.6 * phase.sin())
            });
            let mut demod = configured_demod(rate, 500.0);
            let audio = demod.demodulate(&iq, DemodMode::Cw);
            let settled = &audio[2_400..];
            (settled
                .iter()
                .map(|value| f64::from(*value).powi(2))
                .sum::<f64>()
                / settled.len() as f64)
                .sqrt()
        };
        let pass = measure(0.0);
        for offset in [-3_000.0, 3_000.0] {
            let rejected = measure(offset);
            assert!(
                rejected < pass * 0.01,
                "CW leaked {offset} Hz carrier: {rejected}/{pass}"
            );
        }
    }

    #[test]
    fn ssb_selects_requested_sideband_without_pitch_translation() {
        let rate = 192_000;
        for (mode, sign) in [(DemodMode::Usb, 1.0), (DemodMode::Lsb, -1.0)] {
            let iq = synth_iq(rate, rate as usize / 4, |time| {
                let wanted = std::f64::consts::TAU * sign * 900.0 * time;
                let unwanted = -std::f64::consts::TAU * sign * 1_800.0 * time;
                (
                    0.4 * wanted.cos() + 0.4 * unwanted.cos(),
                    0.4 * wanted.sin() + 0.4 * unwanted.sin(),
                )
            });
            let mut demod = configured_demod(rate, 2_400.0);
            let audio = demod.demodulate(&iq, mode);
            let wanted = tone_amplitude(&audio[2_400..], 900.0, 48_000);
            let unwanted = tone_amplitude(&audio[2_400..], 1_800.0, 48_000);
            assert!(wanted > 0.35, "{mode:?} changed desired pitch: {wanted}");
            assert!(
                unwanted < wanted * 0.02,
                "{mode:?} passed opposite sideband: {unwanted}"
            );
        }
    }

    #[test]
    fn product_modes_keep_exact_audio_clock_and_chunk_continuity_at_noninteger_rates() {
        let rate = 2_048_000;
        let count = rate as usize / 10;
        for mode in [DemodMode::Dsb, DemodMode::Cw] {
            let iq = synth_iq(rate, count, |time| {
                if mode == DemodMode::Cw {
                    (0.6, 0.0)
                } else {
                    (0.6 * (std::f64::consts::TAU * 1_000.0 * time).cos(), 0.0)
                }
            });
            let bandwidth = mode.default_rf_bandwidth_hz();
            let mut whole = configured_demod(rate, bandwidth);
            let reference = whole.demodulate(&iq, mode);
            let mut fragmented = configured_demod(rate, bandwidth);
            let mut actual = Vec::new();
            for chunk in iq.chunks(1_022) {
                // Application reapplies settings every batch. That must not
                // reset decimation, oscillators, filters, or resampling phase.
                fragmented.set_sample_rates(rate, 48_000);
                fragmented.set_rf_bandwidth(bandwidth);
                actual.extend(fragmented.demodulate(chunk, mode));
            }
            assert!(
                reference.len().abs_diff(4_800) <= 1,
                "wrong playback clock: {}",
                reference.len()
            );
            assert_eq!(actual.len(), reference.len());
            let error = actual
                .iter()
                .zip(&reference)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(
                error < 1e-6,
                "chunk boundaries changed {mode:?} samples: {error}"
            );
            let tone = if mode == DemodMode::Cw {
                700.0
            } else {
                1_000.0
            };
            assert!(tone_amplitude(&actual[2_400..], tone, 48_000) > 0.5);
        }
    }

    #[test]
    fn optional_rf_decimation_rejects_a_signal_that_would_alias_into_audio() {
        let rate = 192_000;
        let measure = |frequency: f64| {
            let iq = synth_iq(rate, rate as usize / 4, |time| {
                let phase = std::f64::consts::TAU * frequency * time;
                (0.6 * phase.cos(), 0.6 * phase.sin())
            });
            let mut demod = configured_demod(rate, 8_000.0);
            demod.set_rf_decim(4);
            let audio = demod.demodulate(&iq, DemodMode::Dsb);
            assert_eq!(audio.len(), 12_000);
            tone_amplitude(&audio[2_400..], 1_000.0, 48_000)
        };
        let wanted = measure(1_000.0);
        // 47 kHz aliases to -1 kHz at the reduced 48 kHz RF rate unless the
        // RF pre-stage filters it before discarding samples.
        let alias = measure(47_000.0);
        assert!(wanted > 0.5);
        assert!(
            alias < wanted * 0.01,
            "RF decimation folded an adjacent channel into audio: {alias}/{wanted}"
        );
    }

    #[test]
    fn rf_dc_notch_blanker_and_decimator_preserve_stream_state_across_chunks() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 8, |time| {
            let wanted = std::f64::consts::TAU * 700.0 * time;
            let unwanted = std::f64::consts::TAU * 12_000.0 * time;
            (
                0.2 + 0.4 * wanted.cos() + 0.1 * unwanted.cos(),
                -0.1 + 0.4 * wanted.sin() + 0.1 * unwanted.sin(),
            )
        });
        let create = || {
            let mut demod = configured_demod(rate, 8_000.0);
            demod.set_rf_decim(4);
            demod.set_rf_dc_remove(true);
            demod.set_rf_noise_blanker(true);
            demod.set_rf_notch(true, 12_000.0);
            demod
        };
        let iq = Demodulator::bytes_to_complex(&iq);
        let reference = create().iq_preprocess(&iq).into_owned();
        let mut fragmented = create();
        let actual: Vec<Complex32> = iq
            .chunks(511)
            .flat_map(|chunk| {
                fragmented.set_sample_rates(rate, 48_000);
                fragmented.iq_preprocess(chunk).into_owned()
            })
            .collect();
        assert_eq!(actual.len(), iq.len() / 4);
        assert_eq!(
            actual, reference,
            "optional RF processing changed at block boundaries"
        );
    }

    #[test]
    fn rf_decimation_retains_exact_audio_duration_and_cw_pitch() {
        let rate = 2_048_000;
        let iq = synth_iq(rate, rate as usize / 10, |_| (0.6, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_rf_decim(8);
        let mut audio = Vec::new();
        for chunk in iq.chunks(1_022) {
            demod.set_sample_rates(rate, 48_000);
            audio.extend(demod.demodulate(chunk, DemodMode::Cw));
        }
        assert!(
            audio.len().abs_diff(4_800) <= 1,
            "RF decimation changed playback clock: {}",
            audio.len()
        );
        assert!(tone_amplitude(&audio[2_400..], 700.0, 48_000) > 0.5);
    }

    fn synthetic_stereo_fm(
        rate: u32,
        seconds: f64,
        pilot_hz: f64,
        pilot_phase: f64,
        pilot_amplitude: f64,
        carrier_offset: f64,
    ) -> Vec<u8> {
        let mut phase = 0.73_f64;
        let mut iq = Vec::with_capacity((f64::from(rate) * seconds) as usize * 2);
        for index in 0..(f64::from(rate) * seconds) as usize {
            let time = index as f64 / f64::from(rate);
            let left = 0.4 * (std::f64::consts::TAU * 1_000.0 * time).cos();
            let right = 0.35 * (std::f64::consts::TAU * 2_000.0 * time).cos();
            let pilot = std::f64::consts::TAU * pilot_hz * time + pilot_phase;
            let mpx = 0.5 * (left + right)
                + 0.5 * (left - right) * (2.0 * pilot).cos()
                + pilot_amplitude * pilot.cos();
            phase = (phase
                + std::f64::consts::TAU * (carrier_offset + 75_000.0 * mpx) / f64::from(rate))
            .rem_euclid(std::f64::consts::TAU);
            iq.extend(
                [phase.cos(), phase.sin()]
                    .map(|value| (127.4 + 100.0 * value).round().clamp(0.0, 255.0) as u8),
            );
        }
        iq
    }

    fn stereo_tones(frames: &[[f32; 2]], frequency: f64) -> [f64; 2] {
        std::array::from_fn(|channel| {
            let samples: Vec<_> = frames.iter().map(|frame| frame[channel]).collect();
            tone_amplitude(&samples, frequency, 48_000)
        })
    }

    #[test]
    fn stereo_fm_recovers_left_and_right_with_pilot_phase_and_frequency_offsets() {
        for (pilot_offset, phase, carrier_offset) in [
            (0.0, 0.0, 0.0),
            (12.0, 1.7, 1_500.0),
            (-18.0, 4.8, -1_500.0),
        ] {
            let rate = 384_000;
            let iq = synthetic_stereo_fm(
                rate,
                0.65,
                19_000.0 + pilot_offset,
                phase,
                0.1,
                carrier_offset,
            );
            let mut demod = configured_demod(rate, 150_000.0);
            let frames = demod.demodulate_stereo(&iq, DemodMode::Wfm);
            assert!(
                demod.last_stereo_locked,
                "pilot failed to lock at offset={pilot_offset}, phase={phase}"
            );
            let settled = &frames[19_200..];
            let left_tone = stereo_tones(settled, 1_000.0);
            let right_tone = stereo_tones(settled, 2_000.0);
            assert!(left_tone[0] > 0.3, "left tone missing: {left_tone:?}");
            assert!(right_tone[1] > 0.2, "right tone missing: {right_tone:?}");
            assert!(
                left_tone[1] < left_tone[0] * 0.05,
                "left leaks right: {left_tone:?}, pilot {pilot_offset}/{phase}"
            );
            assert!(
                right_tone[0] < right_tone[1] * 0.05,
                "right leaks left: {right_tone:?}, pilot {pilot_offset}/{phase}"
            );
        }
    }

    #[test]
    fn stereo_fm_falls_back_to_identical_mono_without_pilot() {
        let rate = 384_000;
        let iq = synthetic_stereo_fm(rate, 0.35, 19_000.0, 2.0, 0.0, 0.0);
        let mut demod = configured_demod(rate, 150_000.0);
        let frames = demod.demodulate_stereo(&iq, DemodMode::Wfm);
        assert!(!demod.last_stereo_locked);
        assert!(frames.iter().all(|frame| frame[0] == frame[1]));
        assert!(stereo_tones(&frames[9_600..], 1_000.0)[0] > 0.15);
        assert!(stereo_tones(&frames[9_600..], 2_000.0)[0] > 0.1);
    }

    #[test]
    fn stereo_fm_chunk_boundaries_preserve_pll_filters_and_channel_clocks() {
        let rate = 384_000;
        let iq = synthetic_stereo_fm(rate, 0.35, 19_008.0, 1.2, 0.1, 0.0);
        let mut whole = configured_demod(rate, 150_000.0);
        whole.set_audio_hpf(60.0);
        whole.set_notch(4_000.0, 100.0);
        whole.set_bass(2.0);
        whole.set_treble(-2.0);
        let reference = whole.demodulate_stereo(&iq, DemodMode::Wfm);
        let mut fragmented = configured_demod(rate, 150_000.0);
        fragmented.set_audio_hpf(60.0);
        fragmented.set_notch(4_000.0, 100.0);
        fragmented.set_bass(2.0);
        fragmented.set_treble(-2.0);
        let mut actual = Vec::new();
        for chunk in iq.chunks(1_022) {
            fragmented.set_sample_rates(rate, 48_000);
            actual.extend(fragmented.demodulate_stereo(chunk, DemodMode::Wfm));
        }
        assert_eq!(actual.len(), reference.len());
        let error = actual
            .iter()
            .zip(&reference)
            .flat_map(|(a, b)| [(a[0] - b[0]).abs(), (a[1] - b[1]).abs()])
            .fold(0.0, f32::max);
        assert!(
            error < 1e-6,
            "stereo changed at a source block boundary: {error}"
        );
        assert_eq!(fragmented.last_stereo_locked, whole.last_stereo_locked);
    }

    #[test]
    fn stereo_fm_resamples_each_channel_to_exact_audio_clock() {
        let rate = 2_048_000;
        let iq = synthetic_stereo_fm(rate, 0.35, 19_000.0, 0.4, 0.1, 0.0);
        for audio_rate in [48_000, 44_100] {
            let mut demod = configured_demod(rate, 150_000.0);
            demod.set_sample_rates(rate, audio_rate);
            let frames = demod.demodulate_stereo(&iq, DemodMode::Wfm);
            let expected = (f64::from(audio_rate) * 0.35) as usize;
            assert!(
                frames.len().abs_diff(expected) <= 1,
                "wrong stereo playback duration: {}",
                frames.len()
            );
            assert!(demod.last_stereo_locked);
            let skip = audio_rate as usize / 5;
            let left: Vec<_> = frames[skip..].iter().map(|frame| frame[0]).collect();
            let right: Vec<_> = frames[skip..].iter().map(|frame| frame[1]).collect();
            let gain = std::f64::consts::TAU * 75_000.0 / f64::from(rate) * 0.8;
            assert!(tone_amplitude(&left, 1_000.0, audio_rate) > 0.3 * gain);
            assert!(tone_amplitude(&right, 2_000.0, audio_rate) > 0.2 * gain);
        }
    }

    #[test]
    fn stereo_fm_low_iq_rate_falls_back_without_pilot_alias_lock() {
        for (source_rate, decimation) in [(48_000, 1), (384_000, 8)] {
            let iq = synthetic_stereo_fm(source_rate, 0.1, 19_000.0, 0.4, 0.1, 0.0);
            let mut demod = configured_demod(source_rate, 150_000.0);
            demod.set_rf_decim(decimation);
            let stereo = demod.demodulate_stereo(&iq, DemodMode::Wfm);
            assert!(!demod.last_stereo_locked);
            assert!(!stereo.is_empty());
            assert!(stereo
                .iter()
                .all(|frame| frame[0].is_finite() && frame[0] == frame[1]));
        }
    }

    #[test]
    fn stereo_toggle_preserves_mono_downmix_loudness() {
        let rate = 2_048_000;
        let iq = synthetic_stereo_fm(rate, 0.35, 19_000.0, 0.4, 0.1, 0.0);
        let mut mono = configured_demod(rate, 150_000.0);
        let mono = mono.demodulate(&iq, DemodMode::Wfm);
        let mut stereo = configured_demod(rate, 150_000.0);
        let downmix: Vec<_> = stereo
            .demodulate_stereo(&iq, DemodMode::Wfm)
            .iter()
            .map(|frame| (frame[0] + frame[1]) * 0.5)
            .collect();
        for tone in [1_000.0, 2_000.0] {
            let ratio = tone_amplitude(&downmix[9_600..], tone, 48_000)
                / tone_amplitude(&mono[9_600..], tone, 48_000);
            assert!(
                (0.9..1.1).contains(&ratio),
                "stereo toggle changed {tone} Hz loudness: {ratio}"
            );
        }
    }

    #[test]
    fn stereo_fm_losing_pilot_blends_back_to_mono_and_rejects_weak_pilot() {
        let rate = 384_000;
        let mut demod = configured_demod(rate, 150_000.0);
        let strong = synthetic_stereo_fm(rate, 0.35, 19_000.0, 0.4, 0.1, 0.0);
        demod.demodulate_stereo(&strong, DemodMode::Wfm);
        assert!(demod.last_stereo_locked);
        let absent = synthetic_stereo_fm(rate, 0.5, 19_000.0, 0.4, 0.0, 0.0);
        let mono = demod.demodulate_stereo(&absent, DemodMode::Wfm);
        assert!(!demod.last_stereo_locked);
        let separation = mono[mono.len() - 2_400..]
            .iter()
            .map(|frame| (frame[0] - frame[1]).abs())
            .fold(0.0, f32::max);
        assert!(
            separation < 1e-4,
            "pilot loss left a stereo difference: {separation}"
        );
        let weak = synthetic_stereo_fm(rate, 0.35, 19_000.0, 0.4, 0.01, 0.0);
        let mut weak_demod = configured_demod(rate, 150_000.0);
        weak_demod.demodulate_stereo(&weak, DemodMode::Wfm);
        assert!(!weak_demod.last_stereo_locked);
    }

    #[test]
    fn stereo_fm_deemphasis_audio_lpf_and_gain_apply_to_both_channels() {
        let rate = 384_000;
        let iq = synthetic_stereo_fm(rate, 0.35, 19_000.0, 0.4, 0.1, 0.0);
        let mut baseline = configured_demod(rate, 150_000.0);
        let normal = baseline.demodulate_stereo(&iq, DemodMode::Wfm);
        let mut seventy_five = configured_demod(rate, 150_000.0);
        seventy_five.set_deemph_tau(75.0);
        let de_emphasized = seventy_five.demodulate_stereo(&iq, DemodMode::Wfm);
        let mut narrow = configured_demod(rate, 150_000.0);
        narrow.set_lpf_cutoff(500.0);
        let filtered = narrow.demodulate_stereo(&iq, DemodMode::Wfm);
        for (frequency, channel) in [(1_000.0, 0), (2_000.0, 1)] {
            let normal = stereo_tones(&normal[9_600..], frequency)[channel];
            let deemph = stereo_tones(&de_emphasized[9_600..], frequency)[channel];
            let filtered = stereo_tones(&filtered[9_600..], frequency)[channel];
            assert!(deemph < normal * 0.98 && deemph > normal * 0.7);
            assert!(
                filtered < normal * 0.5,
                "audio LPF did not affect channel {channel}"
            );
        }
        let mut silence = configured_demod(rate, 150_000.0);
        silence.set_audio_gain(0.0);
        assert!(silence
            .demodulate_stereo(&iq, DemodMode::Wfm)
            .iter()
            .all(|frame| *frame == [0.0; 2]));
    }

    #[test]
    fn stereo_fm_agc_links_channel_gain_without_changing_balance() {
        let rate = 384_000;
        let iq = synthetic_stereo_fm(rate, 0.35, 19_000.0, 0.4, 0.1, 0.0);
        let mut baseline = configured_demod(rate, 150_000.0);
        let normal = baseline.demodulate_stereo(&iq, DemodMode::Wfm);
        let mut agc = configured_demod(rate, 150_000.0);
        agc.set_agc_enabled(true);
        agc.set_agc_target(0.1);
        let controlled = agc.demodulate_stereo(&iq, DemodMode::Wfm);
        let mut compared = 0;
        for (normal, controlled) in normal[9_600..].iter().zip(&controlled[9_600..]) {
            if normal[0].abs().min(normal[1].abs()) > 0.02 {
                let left_gain = controlled[0] / normal[0];
                let right_gain = controlled[1] / normal[1];
                assert!((left_gain - right_gain).abs() < 1e-5);
                compared += 1;
            }
        }
        assert!(compared > 1_000);
        assert!(agc.agc_gain < 0.6);
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
    fn demod_am_constant_carrier_is_dc_blocked_by_default() {
        let mut d = Demodulator::new();
        d.agc_enabled = false;
        d.decimation = 1;
        let iq = make_iq_dc(255, 127, 5_000);
        let out = d.demodulate(&iq, DemodMode::Am);
        assert!(!out.is_empty());
        let tail = &out[out.len() - 500..];
        let mean: f32 = tail.iter().map(|x| x.abs()).sum::<f32>() / tail.len() as f32;
        assert!(
            mean < 0.01,
            "AM carrier/DC should settle near silence, got {mean}"
        );
    }

    #[test]
    fn am_and_nfm_channel_filters_reject_out_of_band_iq_before_decimation() {
        let sample_rate = 2_048_000u32;
        let mut am = ComplexChannelLowpass::default();
        let mut fm = ComplexChannelLowpass::default();
        let mut in_power = 0.0;
        let mut out_power = 0.0;
        for n in 0..40_000 {
            let in_phase = 2.0 * std::f32::consts::PI * 5_000.0 * n as f32 / sample_rate as f32;
            let out_phase = 2.0 * std::f32::consts::PI * 300_000.0 * n as f32 / sample_rate as f32;
            let (ii, iq) = am.process(
                in_phase.cos(),
                in_phase.sin(),
                15_000.0,
                f64::from(sample_rate),
            );
            let (oi, oq) = fm.process(
                out_phase.cos(),
                out_phase.sin(),
                15_000.0,
                f64::from(sample_rate),
            );
            if n > 20_000 {
                in_power += ii * ii + iq * iq;
                out_power += oi * oi + oq * oq;
            }
        }
        assert!(
            out_power < in_power * 0.01,
            "pre-decimation channel filter did not reject the alias source: in={in_power}, out={out_power}"
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

    /// A 50 µs de-emphasis pole attenuates a real 10 kHz modulation tone to
    /// about 30% of a nearly flat 1 µs reference. This distinguishes applying
    /// the pole at the input clock from using audio-rate coefficients there.
    #[test]
    fn demod_wfm_deemph_runs_at_input_rate() {
        let rate = 384_000;
        let iq = synth_iq(rate, rate as usize / 4, |time| {
            let phase = 0.5 * (std::f64::consts::TAU * 10_000.0 * time).sin();
            (0.7 * phase.cos(), 0.7 * phase.sin())
        });
        let measure = |tau| {
            let mut demod = configured_demod(rate, 150_000.0);
            demod.set_deemph_tau(tau);
            let audio = demod.demodulate(&iq, DemodMode::Wfm);
            tone_amplitude(&audio[2_400..], 10_000.0, 48_000)
        };
        let ratio = measure(50.0) / measure(1.0);
        assert!(
            (0.2..0.4).contains(&ratio),
            "wrong de-emphasis clock: amplitude ratio {ratio}"
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

    #[test]
    fn test_pitch_shift_preserves_sample_count() {
        let mut d = Demodulator::new();
        let iq = make_iq_dc(128, 128, 512);
        let out_baseline = d.demodulate(&iq, DemodMode::Am);

        d.set_pitch(1.0); // +1 octave up
        let out_up = d.demodulate(&iq, DemodMode::Am);
        assert_eq!(out_up.len(), out_baseline.len());

        d.set_pitch(-0.7); // pitch down
        let out_down = d.demodulate(&iq, DemodMode::Am);
        assert_eq!(out_down.len(), out_baseline.len());
    }

    #[test]
    fn test_pitch_shift_multiple_chunks_continuity() {
        let mut d = Demodulator::new();
        d.set_pitch(0.5);
        let iq = make_iq_dc(135, 120, 256);
        for _ in 0..10 {
            let out = d.demodulate(&iq, DemodMode::Fm);
            assert_eq!(out.len(), 256);
            for s in out {
                assert!(s.is_finite());
            }
        }
    }

    // --- New demod feature tests ---

    #[test]
    fn dsb_sideband_both_recovers_signed_modulation() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |time| {
            (0.6 * (std::f64::consts::TAU * 1_000.0 * time).cos(), 0.0)
        });
        let mut demod = configured_demod(rate, 8_000.0);
        demod.set_dsb_sideband(DsbSideband::Both);
        let audio = demod.demodulate(&iq, DemodMode::Dsb);
        let settled = &audio[2_400..];
        let fundamental = tone_amplitude(settled, 1_000.0, 48_000);
        let harmonic = tone_amplitude(settled, 2_000.0, 48_000);
        assert!(
            fundamental > 0.5,
            "DSB Both lost signed modulation: {fundamental}"
        );
        assert!(
            harmonic < fundamental * 0.01,
            "DSB Both rectified the signal: {harmonic}"
        );
        assert!(settled.iter().any(|sample| *sample < -0.4));
        assert!(settled.iter().any(|sample| *sample > 0.4));
    }

    #[test]
    fn dsb_sideband_upper_selects_upper_sideband() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |time| {
            let wanted = std::f64::consts::TAU * 900.0 * time;
            let unwanted = -std::f64::consts::TAU * 1_800.0 * time;
            (
                0.4 * wanted.cos() + 0.4 * unwanted.cos(),
                0.4 * wanted.sin() + 0.4 * unwanted.sin(),
            )
        });
        let mut demod = configured_demod(rate, 2_400.0);
        demod.set_dsb_sideband(DsbSideband::Upper);
        let audio = demod.demodulate(&iq, DemodMode::Dsb);
        let wanted = tone_amplitude(&audio[2_400..], 900.0, 48_000);
        let unwanted = tone_amplitude(&audio[2_400..], 1_800.0, 48_000);
        assert!(wanted > 0.35, "DSB Upper changed desired pitch: {wanted}");
        assert!(
            unwanted < wanted * 0.02,
            "DSB Upper passed opposite sideband: {unwanted}"
        );
    }

    #[test]
    fn dsb_sideband_lower_selects_lower_sideband() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |time| {
            let wanted = -std::f64::consts::TAU * 900.0 * time;
            let unwanted = std::f64::consts::TAU * 1_800.0 * time;
            (
                0.4 * wanted.cos() + 0.4 * unwanted.cos(),
                0.4 * wanted.sin() + 0.4 * unwanted.sin(),
            )
        });
        let mut demod = configured_demod(rate, 2_400.0);
        demod.set_dsb_sideband(DsbSideband::Lower);
        let audio = demod.demodulate(&iq, DemodMode::Dsb);
        let wanted = tone_amplitude(&audio[2_400..], 900.0, 48_000);
        let unwanted = tone_amplitude(&audio[2_400..], 1_800.0, 48_000);
        assert!(wanted > 0.35, "DSB Lower changed desired pitch: {wanted}");
        assert!(
            unwanted < wanted * 0.02,
            "DSB Lower passed opposite sideband: {unwanted}"
        );
    }

    #[test]
    fn cw_offset_zero_produces_tone_at_cw_tone_hz() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |_| (0.6, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_cw_tone(700.0);
        demod.set_cw_offset(0.0);
        let audio = demod.demodulate(&iq, DemodMode::Cw);
        let tone = tone_amplitude(&audio[2_400..], 700.0, 48_000);
        assert!(tone > 0.5, "CW offset=0 tone missing at 700 Hz: {tone}");
    }

    #[test]
    fn cw_offset_200_produces_tone_at_cw_tone_hz_plus_200() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |_| (0.6, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_cw_tone(700.0);
        demod.set_cw_offset(200.0);
        let audio = demod.demodulate(&iq, DemodMode::Cw);
        let tone = tone_amplitude(&audio[2_400..], 900.0, 48_000);
        assert!(tone > 0.5, "CW offset=200 tone missing at 900 Hz: {tone}");
    }

    #[test]
    fn cw_volume_full_produces_reference_amplitude() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |_| (0.6, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_cw_tone(700.0);
        demod.set_cw_volume(1.0);
        let audio = demod.demodulate(&iq, DemodMode::Cw);
        let reference = tone_amplitude(&audio[2_400..], 700.0, 48_000);
        assert!(
            reference > 0.5,
            "CW volume=1.0 baseline too low: {reference}"
        );
    }

    #[test]
    fn cw_volume_half_produces_half_amplitude() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |_| (0.6, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_cw_tone(700.0);
        demod.set_cw_volume(0.5);
        let audio = demod.demodulate(&iq, DemodMode::Cw);
        let amplitude = tone_amplitude(&audio[2_400..], 700.0, 48_000);
        // Half volume should produce roughly half the amplitude of full volume.
        // Use a loose bound to account for filter settling.
        assert!(
            amplitude > 0.15 && amplitude < 0.55,
            "CW volume=0.5 amplitude out of expected range: {amplitude}"
        );
    }

    #[test]
    fn cw_volume_zero_produces_silence() {
        let rate = 192_000;
        let iq = synth_iq(rate, rate as usize / 4, |_| (0.6, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_cw_tone(700.0);
        demod.set_cw_volume(0.0);
        let audio = demod.demodulate(&iq, DemodMode::Cw);
        let peak = audio
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        assert!(
            peak < 0.01,
            "CW volume=0.0 should be silent, got peak {peak}"
        );
    }

    #[test]
    fn cw_squelch_strong_signal_passes_audio() {
        let rate = 192_000;
        // Strong carrier well above the squelch threshold.
        let iq = synth_iq(rate, rate as usize / 4, |_| (0.6, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_cw_tone(700.0);
        demod.set_cw_squelch(true, -60.0);
        let audio = demod.demodulate(&iq, DemodMode::Cw);
        let tone = tone_amplitude(&audio[2_400..], 700.0, 48_000);
        assert!(tone > 0.3, "CW squelch blocked strong signal: {tone}");
    }

    #[test]
    fn cw_squelch_weak_signal_gates_to_silence() {
        let rate = 192_000;
        // Very weak carrier below the squelch threshold.
        let iq = synth_iq(rate, rate as usize / 4, |_| (0.001, 0.0));
        let mut demod = configured_demod(rate, 500.0);
        demod.set_cw_tone(700.0);
        demod.set_cw_squelch(true, -30.0);
        let audio = demod.demodulate(&iq, DemodMode::Cw);
        let peak = audio
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        assert!(peak < 0.01, "CW squelch passed weak signal: peak {peak}");
    }
}
