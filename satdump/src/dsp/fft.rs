use num_complex::Complex32;
use rustfft::{num_complex::Complex, FftDirection, FftPlanner as RustFftPlanner};
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// FFT helper (RAII wrapper around rustfft)
// ---------------------------------------------------------------------------

pub struct FftHelper {
    pub input: Vec<Complex<f32>>,
    pub output: Vec<Complex<f32>>,
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl FftHelper {
    pub fn new(size: usize, forward: bool) -> Self {
        let mut planner = RustFftPlanner::new();
        let fft = planner.plan_fft(
            size,
            if forward {
                FftDirection::Forward
            } else {
                FftDirection::Inverse
            },
        );
        Self {
            input: vec![Complex::new(0.0, 0.0); size],
            output: vec![Complex::new(0.0, 0.0); size],
            fft,
        }
    }

    pub fn execute(&mut self) {
        self.output.copy_from_slice(&self.input);
        self.fft.process(&mut self.output);
    }
}

// ---------------------------------------------------------------------------
// FFT Pan Block
// ---------------------------------------------------------------------------

pub struct FftPanBlock {
    fft_size: usize,
    sample_rate: u64,
    rate: i32,
    rbuffer_rate: usize,
    rbuffer_size: usize,
    rbuffer_skip: usize,
    fft_taps: Vec<f32>,
    fft_helper: FftHelper,
    fft_output_buffer: Vec<f32>,
    reshape_buffer: Vec<Complex32>,
    in_reshape_buffer: usize,
    reshape_buffer_size: usize,
    input_buffer: Vec<Complex32>,
    pub avg_num: f32,
    pub output_fft_buff: Vec<f32>,
}

impl FftPanBlock {
    pub fn new()
        -> Self
    {
        let mut block = Self {
            fft_size: 65536,
            sample_rate: 10_000_000,
            rate: 20,
            rbuffer_rate: 0,
            rbuffer_size: 0,
            rbuffer_skip: 0,
            fft_taps: Vec::new(),
            fft_helper: FftHelper::new(1, true), // placeholder
            fft_output_buffer: Vec::new(),
            reshape_buffer: Vec::new(),
            in_reshape_buffer: 0,
            reshape_buffer_size: 0,
            input_buffer: Vec::new(),
            avg_num: 10.0,
            output_fft_buff: Vec::new(),
        };
        block.init();
        block
    }

    pub fn set_fft_settings(&mut self, size: usize, sample_rate: u64, rate: i32) {
        self.fft_size = size;
        self.sample_rate = sample_rate;
        self.rate = rate.max(1);

        self.rbuffer_rate = (self.sample_rate / self.rate as u64) as usize;
        self.rbuffer_size = self.rbuffer_rate.min(self.fft_size);
        self.rbuffer_skip = self.rbuffer_rate - self.rbuffer_size;

        // Nuttall window with alternating sign
        self.fft_taps = (0..self.rbuffer_size)
            .map(|i| {
                nuttall(i, self.rbuffer_size - 1)
                    * if i % 2 == 0 { -1.0 } else { 1.0 }
            })
            .collect();

        self.fft_helper = FftHelper::new(self.fft_size, true);
        self.fft_output_buffer = vec![0.0; self.fft_size];
        self.input_buffer = vec![Complex::new(0.0, 0.0); self.fft_size];
        self.output_fft_buff = vec![0.0; self.fft_size];

        self.reshape_buffer_size = (dsp::STREAM_BUFFER_SIZE as usize)
            .max(self.rbuffer_rate * 10);
        self.reshape_buffer =
            vec![Complex::new(0.0, 0.0); self.reshape_buffer_size];
        self.in_reshape_buffer = 0;
    }

    fn init(&mut self) {
        self.set_fft_settings(self.fft_size, self.sample_rate, self.rate);
    }

    pub fn work(&mut self, input: &[Complex32], on_fft: &mut dyn FnMut(&[f32], usize)) {
        // Append to reshape buffer if there's space
        let nsamples = input.len();
        if self.in_reshape_buffer + nsamples < self.reshape_buffer_size {
            self.reshape_buffer[self.in_reshape_buffer
                ..self.in_reshape_buffer + nsamples]
                .copy_from_slice(&input[..nsamples]);
            self.in_reshape_buffer += nsamples;
        }

        if self.in_reshape_buffer > self.rbuffer_rate {
            let mut pos_in_buffer = 0;
            while self.in_reshape_buffer - pos_in_buffer > self.rbuffer_rate {
                // Copy rbuffer_size samples to input buffer
                self.input_buffer[..self.rbuffer_size].copy_from_slice(
                    &self.reshape_buffer[pos_in_buffer..pos_in_buffer + self.rbuffer_size],
                );
                pos_in_buffer += self.rbuffer_rate;

                // Apply window / alternating sign
                for (i, sample) in self.input_buffer[..self.rbuffer_size].iter_mut().enumerate() {
                    *sample *= self.fft_taps[i];
                }
                // Zero pad the rest
                for sample in &mut self.input_buffer[self.rbuffer_size..] {
                    *sample = Complex::new(0.0, 0.0);
                }

                // Perform FFT
                self.fft_helper.input.copy_from_slice(&self.input_buffer);
                self.fft_helper.execute();

                // Power spectrum
                for (i, &c) in self.fft_helper.output.iter().enumerate() {
                    self.fft_output_buffer[i] = c.norm();
                }

                // Average
                let avg_rate = 1.0 / self.avg_num.max(1.0);
                for i in 0..self.fft_size {
                    self.output_fft_buff[i] = self.output_fft_buff[i] * (1.0 - avg_rate)
                        + self.fft_output_buffer[i] * avg_rate;
                }

                on_fft(&self.output_fft_buff, self.fft_size);
            }

            // Move remaining samples to front
            if pos_in_buffer < self.in_reshape_buffer {
                let remaining = self.in_reshape_buffer - pos_in_buffer;
                self.reshape_buffer.copy_within(pos_in_buffer..pos_in_buffer + remaining, 0);
                self.in_reshape_buffer = remaining;
            } else {
                self.in_reshape_buffer = 0;
            }
        }
    }
}

fn nuttall(n: usize, m: usize) -> f32 {
    if m == 0 {
        return 1.0;
    }
    let x = (n as f32) / (m as f32);
    0.3635819
        - 0.4891775 * (2.0 * PI * x).cos()
        + 0.1365995 * (4.0 * PI * x).cos()
        - 0.0106411 * (6.0 * PI * x).cos()
}

// ---------------------------------------------------------------------------
// Filter tap designs (RRC, LPF) – direct transliteration of firdes
// ---------------------------------------------------------------------------

pub mod dsp {
    pub const STREAM_BUFFER_SIZE: usize = 8192;
}

pub fn root_raised_cosine(gain: f64, sample_rate: f64, symbol_rate: f64, alpha: f64, ntaps: usize) -> Vec<f32> {
    let mut taps = vec![0.0f32; ntaps];
    let gain = gain as f32;
    let sample_rate = sample_rate as f32;
    let symbol_rate = symbol_rate as f32;
    let alpha = alpha as f32;

    for i in 0..ntaps {
        let x = ((i as f32) - ((ntaps as f32) / 2.0)) / (sample_rate / symbol_rate);
        let y1 = 1.0 - (4.0 * alpha * alpha * x * x);
        let mut y2 = PI * x * (1.0 - 4.0 * alpha * alpha * x * x).sqrt();
        if x.abs() < 1e-6 {
            taps[i] = gain * (1.0 + alpha * (4.0 / PI - 1.0));
        } else {
            taps[i] = gain * (y1 * (PI * x * alpha).sin().cos() + (PI * x * alpha).sin() / alpha)
                / (PI * y2.abs());
        }
    }
    taps
}

pub fn low_pass(gain: f64, sample_rate: f64, cutoff: f64, transition_width: f64) -> Vec<f32> {
    let alpha = 0.0; // unused in transliteration, kept for signature compatibility
    let ntaps = (3.3 / (transition_width / sample_rate)).ceil() as usize | 1;
    let mut taps = vec![0.0f32; ntaps];
    let sample_rate = sample_rate as f32;
    let cutoff = cutoff as f32;
    let gain = gain as f32;

    for i in 0..ntaps {
        let n = (i as f32) - ((ntaps - 1) as f32) / 2.0;
        let fc = cutoff / sample_rate;
        let h = if n == 0.0 {
            2.0 * fc
        } else {
            (2.0 * fc * PI * n).sin() / (PI * n)
        };
        let w = 0.54 - 0.46 * (2.0 * PI * (i as f32) / ((ntaps - 1) as f32)).cos();
        taps[i] = gain * h * w;
    }
    taps
}
