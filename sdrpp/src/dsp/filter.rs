use num_complex::Complex32;
use rustfft::FftPlanner;
use std::sync::Arc;

pub fn nuttall_window(n: f64, n_taps: f64) -> f64 {
    const COEFS: [f64; 4] = [0.355768, 0.487396, 0.144232, 0.012604];
    let mut win = 0.0;
    let mut sign = 1.0;
    for (i, &coef) in COEFS.iter().enumerate() {
        win += sign * coef * ((i as f64) * 2.0 * std::f64::consts::PI * n / n_taps).cos();
        sign = -sign;
    }
    win
}

fn estimate_tap_count(transition_hz: f32, sample_rate: f32) -> usize {
    (3.8 * sample_rate / transition_hz).max(1.0) as usize
}

pub fn lowpass_taps(cutoff_hz: f32, transition_hz: f32, sample_rate: f32) -> Vec<f32> {
    let count = estimate_tap_count(transition_hz, sample_rate);
    let half = count as f64 / 2.0;
    let omega = 2.0 * std::f64::consts::PI * (cutoff_hz as f64 / sample_rate as f64);
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

pub fn bandpass_taps(low_hz: f32, high_hz: f32, transition_hz: f32, sample_rate: f32) -> Vec<f32> {
    let mut count = estimate_tap_count(transition_hz, sample_rate);
    if count % 2 == 0 {
        count += 1;
    }
    let half = count as f64 / 2.0;
    let bw = (high_hz - low_hz) as f64;
    let omega = std::f64::consts::PI * (bw / sample_rate as f64);
    let offset = 2.0 * std::f64::consts::PI * ((low_hz + high_hz) as f64 / 2.0) / sample_rate as f64;
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
            (2.0 * (offset * t).cos() * sinc * win * corr) as f32
        })
        .collect()
}

pub fn bandpass_taps_complex(low_hz: f32, high_hz: f32, transition_hz: f32, sample_rate: f32) -> Vec<Complex32> {
    let mut count = estimate_tap_count(transition_hz, sample_rate);
    if count % 2 == 0 {
        count += 1;
    }
    let half = count as f64 / 2.0;
    let bw = (high_hz - low_hz) as f64;
    let omega = std::f64::consts::PI * (bw / sample_rate as f64);
    let offset = 2.0 * std::f64::consts::PI * ((low_hz + high_hz) as f64 / 2.0) / sample_rate as f64;
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
            let phase = -(offset * t) as f32;
            Complex32::new(base * phase.cos(), base * phase.sin())
        })
        .collect()
}

pub struct FirFilter<T, Tap = f32> {
    taps: Vec<Tap>,
    buffer: Vec<T>,
    buf_start: usize,
}

impl<T, Tap> FirFilter<T, Tap>
where
    T: Copy + Default + std::ops::Add<Output = T>,
    Tap: Copy,
    T: std::ops::Mul<Tap, Output = T>,
{
    pub fn new(taps: &[Tap]) -> Self {
        let n = taps.len();
        Self {
            taps: taps.to_vec(),
            buffer: vec![T::default(); n - 1 + 64000],
            buf_start: n - 1,
        }
    }

    pub fn set_taps(&mut self, taps: &[Tap]) {
        let old_n = self.taps.len();
        let new_n = taps.len();
        if new_n < old_n {
            let shift = old_n - new_n;
            self.buffer.copy_within(shift..shift + new_n - 1, 0);
        } else if new_n > old_n {
            let shift = new_n - old_n;
            self.buffer.copy_within(0..old_n - 1, shift);
            for i in 0..shift {
                self.buffer[i] = T::default();
            }
        }
        self.taps = taps.to_vec();
        self.buf_start = new_n - 1;
    }

    pub fn reset(&mut self) {
        self.buffer.fill(T::default());
    }
}

impl FirFilter<f32, f32> {
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let n = self.taps.len();
        let count = input.len();
        let buf = &mut self.buffer;
        let start = self.buf_start;
        buf[start..start + count].copy_from_slice(input);
        for i in 0..count {
            let mut acc = 0.0f32;
            for j in 0..n {
                acc += buf[i + j] * self.taps[j];
            }
            output[i] = acc;
        }
        let keep = n - 1;
        buf.copy_within(count..count + keep, 0);
    }
}

impl FirFilter<Complex32, f32> {
    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) {
        let n = self.taps.len();
        let count = input.len();
        let buf = &mut self.buffer;
        let start = self.buf_start;
        buf[start..start + count].copy_from_slice(input);
        for i in 0..count {
            let mut acc = Complex32::new(0.0, 0.0);
            for j in 0..n {
                acc += buf[i + j] * self.taps[j];
            }
            output[i] = acc;
        }
        let keep = n - 1;
        buf.copy_within(count..count + keep, 0);
    }
}

impl FirFilter<Complex32, Complex32> {
    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) {
        let n = self.taps.len();
        let count = input.len();
        let buf = &mut self.buffer;
        let start = self.buf_start;
        buf[start..start + count].copy_from_slice(input);
        for i in 0..count {
            let mut acc = Complex32::new(0.0, 0.0);
            for j in 0..n {
                acc += buf[i + j] * self.taps[j];
            }
            output[i] = acc;
        }
        let keep = n - 1;
        buf.copy_within(count..count + keep, 0);
    }
}

pub struct DecimatingFir<T, Tap = f32> {
    inner: FirFilter<T, Tap>,
    decimation: usize,
    offset: usize,
}

impl DecimatingFir<f32, f32> {
    pub fn new(taps: &[f32], decimation: usize) -> Self {
        Self {
            inner: FirFilter::new(taps),
            decimation: decimation.max(1),
            offset: 0,
        }
    }

    pub fn set_taps(&mut self, taps: &[f32]) {
        self.inner.set_taps(taps);
        self.offset = 0;
    }

    pub fn set_decimation(&mut self, decimation: usize) {
        self.decimation = decimation.max(1);
        self.offset = 0;
    }

    pub fn reset(&mut self) {
        self.inner.reset();
        self.offset = 0;
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        let n = self.inner.taps.len();
        let count = input.len();
        let buf = &mut self.inner.buffer;
        let start = self.inner.buf_start;
        buf[start..start + count].copy_from_slice(input);
        let mut out_count = 0;
        while self.offset < count {
            let mut acc = 0.0f32;
            for j in 0..n {
                acc += buf[self.offset + j] * self.inner.taps[j];
            }
            output[out_count] = acc;
            out_count += 1;
            self.offset += self.decimation;
        }
        self.offset -= count;
        let keep = n - 1;
        buf.copy_within(count..count + keep, 0);
        out_count
    }
}

impl DecimatingFir<Complex32, f32> {
    pub fn new(taps: &[f32], decimation: usize) -> Self {
        Self {
            inner: FirFilter::new(taps),
            decimation: decimation.max(1),
            offset: 0,
        }
    }

    pub fn set_taps(&mut self, taps: &[f32]) {
        self.inner.set_taps(taps);
        self.offset = 0;
    }

    pub fn set_decimation(&mut self, decimation: usize) {
        self.decimation = decimation.max(1);
        self.offset = 0;
    }

    pub fn reset(&mut self) {
        self.inner.reset();
        self.offset = 0;
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) -> usize {
        let n = self.inner.taps.len();
        let count = input.len();
        let buf = &mut self.inner.buffer;
        let start = self.inner.buf_start;
        buf[start..start + count].copy_from_slice(input);
        let mut out_count = 0;
        while self.offset < count {
            let mut acc = Complex32::new(0.0, 0.0);
            for j in 0..n {
                acc += buf[self.offset + j] * self.inner.taps[j];
            }
            output[out_count] = acc;
            out_count += 1;
            self.offset += self.decimation;
        }
        self.offset -= count;
        let keep = n - 1;
        buf.copy_within(count..count + keep, 0);
        out_count
    }
}

impl DecimatingFir<Complex32, Complex32> {
    pub fn new(taps: &[Complex32], decimation: usize) -> Self {
        Self {
            inner: FirFilter::new(taps),
            decimation: decimation.max(1),
            offset: 0,
        }
    }

    pub fn set_taps(&mut self, taps: &[Complex32]) {
        self.inner.set_taps(taps);
        self.offset = 0;
    }

    pub fn set_decimation(&mut self, decimation: usize) {
        self.decimation = decimation.max(1);
        self.offset = 0;
    }

    pub fn reset(&mut self) {
        self.inner.reset();
        self.offset = 0;
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) -> usize {
        let n = self.inner.taps.len();
        let count = input.len();
        let buf = &mut self.inner.buffer;
        let start = self.inner.buf_start;
        buf[start..start + count].copy_from_slice(input);
        let mut out_count = 0;
        while self.offset < count {
            let mut acc = Complex32::new(0.0, 0.0);
            for j in 0..n {
                acc += buf[self.offset + j] * self.inner.taps[j];
            }
            output[out_count] = acc;
            out_count += 1;
            self.offset += self.decimation;
        }
        self.offset -= count;
        let keep = n - 1;
        buf.copy_within(count..count + keep, 0);
        out_count
    }
}

pub struct FftFilter {
    fft_size: usize,
    forward: Arc<dyn rustfft::Fft<f32>>,
    inverse: Arc<dyn rustfft::Fft<f32>>,
    taps_fft: Vec<Complex32>,
    input_buf: Vec<Complex32>,
    output_buf: Vec<Complex32>,
    overlap: Vec<Complex32>,
    block_size: usize,
}

impl FftFilter {
    pub fn new(taps: &[f32]) -> Self {
        let tap_len = taps.len();
        let block_size = (tap_len * 4).max(256);
        let fft_size = (block_size + tap_len - 1).next_power_of_two();
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(fft_size);
        let inverse = planner.plan_fft_inverse(fft_size);
        let mut taps_padded = vec![Complex32::new(0.0, 0.0); fft_size];
        for (i, &t) in taps.iter().enumerate() {
            taps_padded[i] = Complex32::new(t, 0.0);
        }
        forward.process(&mut taps_padded);
        Self {
            fft_size,
            forward,
            inverse,
            taps_fft: taps_padded,
            input_buf: vec![Complex32::new(0.0, 0.0); fft_size],
            output_buf: vec![Complex32::new(0.0, 0.0); fft_size],
            overlap: vec![Complex32::new(0.0, 0.0); tap_len - 1],
            block_size,
        }
    }

    pub fn set_taps(&mut self, taps: &[f32]) {
        let tap_len = taps.len();
        self.block_size = (tap_len * 4).max(256);
        let fft_size = (self.block_size + tap_len - 1).next_power_of_two();
        if fft_size != self.fft_size {
            self.fft_size = fft_size;
            let mut planner = FftPlanner::<f32>::new();
            self.forward = planner.plan_fft_forward(fft_size);
            self.inverse = planner.plan_fft_inverse(fft_size);
            self.input_buf.resize(fft_size, Complex32::new(0.0, 0.0));
            self.output_buf.resize(fft_size, Complex32::new(0.0, 0.0));
        }
        self.overlap.resize(tap_len - 1, Complex32::new(0.0, 0.0));
        let mut taps_padded = vec![Complex32::new(0.0, 0.0); fft_size];
        for (i, &t) in taps.iter().enumerate() {
            taps_padded[i] = Complex32::new(t, 0.0);
        }
        self.forward.process(&mut taps_padded);
        self.taps_fft = taps_padded;
    }

    pub fn reset(&mut self) {
        self.overlap.fill(Complex32::new(0.0, 0.0));
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) -> usize {
        let tap_len = self.overlap.len() + 1;
        let bs = self.block_size;
        let fs = self.fft_size;
        let mut produced = 0;
        let mut pos = 0;
        while pos + bs <= input.len() {
            self.input_buf[..bs].copy_from_slice(&input[pos..pos + bs]);
            self.input_buf[bs..fs].fill(Complex32::new(0.0, 0.0));
            self.forward.process(&mut self.input_buf);
            for i in 0..fs {
                self.output_buf[i] = self.input_buf[i] * self.taps_fft[i];
            }
            self.inverse.process(&mut self.output_buf);
            let scale = 1.0 / fs as f32;
            for i in 0..bs {
                self.output_buf[i] *= scale;
            }
            for i in 0..tap_len - 1 {
                self.output_buf[i] += self.overlap[i];
            }
            output[produced..produced + bs].copy_from_slice(&self.output_buf[..bs]);
            produced += bs;
            self.overlap.copy_from_slice(&self.output_buf[bs..bs + tap_len - 1]);
            pos += bs;
        }
        produced
    }
}
