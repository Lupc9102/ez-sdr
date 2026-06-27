//! Clock recovery - translated from src-core/dsp/clock_recovery/

use num_complex::Complex32;
use std::f32::consts::PI;

pub struct MmClockRecovery<T> {
    pub omega: f32,
    pub omega_gain: f32,
    pub mu: f32,
    pub mu_gain: f32,
    pub omega_limit: f32,
    pub nfilt: usize,
    pub ntaps: usize,
    buffer: Vec<T>,
    taps: Vec<Vec<f32>>,
    mu_state: f32,
    omega_state: f32,
    omega_mid: f32,
    omega_limit_state: f32,
    sample: f32,
    last_sample: f32,
    p_2t: Complex32,
    p_1t: Complex32,
    p_0t: Complex32,
    c_2t: Complex32,
    c_1t: Complex32,
    c_0t: Complex32,
    phase_error: f32,
}

fn clip_value(x: f32, clip: f32) -> f32 {
    0.5 * ((x + clip).abs() - (x - clip).abs())
}

impl<T: Copy + Default + std::ops::Add<Output = T> + std::ops::Mul<f32, Output = T>>
    MmClockRecovery<T>
{
    pub fn new(
        omega: f32,
        omega_gain: f32,
        mu: f32,
        mu_gain: f32,
        omega_limit: f32,
        nfilt: usize,
        ntaps: usize,
    ) -> Self {
        let total_taps = nfilt * ntaps;
        let mut all_taps = vec![0.0f32; total_taps];
        let fc = 0.5 / nfilt as f32;
        let center = (total_taps - 1) as f32 / 2.0;
        for i in 0..total_taps {
            let x = i as f32 - center;
            let sinc = if x.abs() < 1e-9 {
                2.0 * fc
            } else {
                (2.0 * PI * fc * x).sin() / (PI * x)
            };
            let window = 0.54 - 0.46 * (2.0 * PI * i as f32 / (total_taps - 1) as f32).cos();
            all_taps[i] = sinc * window;
        }
        let mut taps = vec![vec![0.0f32; ntaps]; nfilt];
        for i in 0..total_taps {
            let phase = (nfilt - 1) - (i % nfilt);
            let tap_idx = i / nfilt;
            if tap_idx < ntaps {
                taps[phase][tap_idx] = all_taps[i];
            }
        }
        Self {
            omega,
            omega_gain,
            mu,
            mu_gain,
            omega_limit,
            nfilt,
            ntaps,
            buffer: Vec::with_capacity(8192),
            taps,
            mu_state: mu,
            omega_state: omega,
            omega_mid: omega,
            omega_limit_state: omega_limit * omega,
            sample: 0.0,
            last_sample: 0.0,
            p_2t: Complex32::new(0.0, 0.0),
            p_1t: Complex32::new(0.0, 0.0),
            p_0t: Complex32::new(0.0, 0.0),
            c_2t: Complex32::new(0.0, 0.0),
            c_1t: Complex32::new(0.0, 0.0),
            c_0t: Complex32::new(0.0, 0.0),
            phase_error: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.mu_state = self.mu;
        self.omega_state = self.omega;
        self.omega_mid = self.omega;
        self.omega_limit_state = self.omega_limit * self.omega;
        self.sample = 0.0;
        self.last_sample = 0.0;
        self.p_2t = Complex32::new(0.0, 0.0);
        self.p_1t = Complex32::new(0.0, 0.0);
        self.p_0t = Complex32::new(0.0, 0.0);
        self.c_2t = Complex32::new(0.0, 0.0);
        self.c_1t = Complex32::new(0.0, 0.0);
        self.c_0t = Complex32::new(0.0, 0.0);
        self.phase_error = 0.0;
        self.buffer.clear();
    }
}

impl MmClockRecovery<f32> {
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        self.buffer.extend_from_slice(input);
        let mut produced = 0usize;
        while produced < output.len() {
            let imu = (self.mu_state * self.nfilt as f32).round() as usize;
            let imu = imu.min(self.nfilt - 1);
            if self.ntaps > self.buffer.len() {
                break;
            }
            let branch = &self.taps[imu];
            let mut acc = 0.0f32;
            for (k, &tap) in branch.iter().enumerate() {
                acc += self.buffer[k] * tap;
            }
            self.sample = acc;
            self.phase_error = (if self.last_sample < 0.0 { -1.0 } else { 1.0 }) * self.sample
                - (if self.sample < 0.0 { -1.0 } else { 1.0 }) * self.last_sample;
            self.phase_error = clip_value(self.phase_error, 1.0);
            self.last_sample = self.sample;
            output[produced] = self.sample;
            produced += 1;
            self.omega_state += self.omega_gain * self.phase_error;
            self.omega_state =
                self.omega_mid + clip_value(self.omega_state - self.omega_mid, self.omega_limit_state);
            self.mu_state += self.omega_state + self.mu_gain * self.phase_error;
            let advance = self.mu_state.floor() as usize;
            self.mu_state -= self.mu_state.floor();
            if advance > 0 {
                if advance >= self.buffer.len() {
                    self.buffer.clear();
                    break;
                }
                self.buffer.drain(0..advance);
            }
        }
        produced
    }
}

impl MmClockRecovery<Complex32> {
    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) -> usize {
        self.buffer.extend_from_slice(input);
        let mut produced = 0usize;
        while produced < output.len() {
            let imu = (self.mu_state * self.nfilt as f32).round() as usize;
            let imu = imu.min(self.nfilt - 1);
            if self.ntaps > self.buffer.len() {
                break;
            }
            self.p_2t = self.p_1t;
            self.p_1t = self.p_0t;
            self.c_2t = self.c_1t;
            self.c_1t = self.c_0t;
            let branch = &self.taps[imu];
            let mut acc = Complex32::new(0.0, 0.0);
            for (k, &tap) in branch.iter().enumerate() {
                acc += self.buffer[k] * tap;
            }
            self.p_0t = acc;
            self.c_0t = Complex32::new(
                if self.p_0t.re > 0.0 { 1.0 } else { 0.0 },
                if self.p_0t.im > 0.0 { 1.0 } else { 0.0 },
            );
            self.phase_error = (((self.p_0t - self.p_2t) * self.c_1t.conj())
                - ((self.c_0t - self.c_2t) * self.p_1t.conj()))
                .re;
            self.phase_error = clip_value(self.phase_error, 1.0);
            output[produced] = self.p_0t;
            produced += 1;
            self.omega_state += self.omega_gain * self.phase_error;
            self.omega_state =
                self.omega_mid + clip_value(self.omega_state - self.omega_mid, self.omega_limit_state);
            self.mu_state += self.omega_state + self.mu_gain * self.phase_error;
            let advance = self.mu_state.floor() as usize;
            self.mu_state -= self.mu_state.floor();
            if advance > 0 {
                if advance >= self.buffer.len() {
                    self.buffer.clear();
                    break;
                }
                self.buffer.drain(0..advance);
            }
        }
        produced
    }
}
