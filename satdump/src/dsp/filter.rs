use num_complex::Complex32;
use rustfft::{num_complex::Complex, FftDirection, FftPlanner as RustFftPlanner};
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Direct-form FIR block
// ---------------------------------------------------------------------------

pub struct FirBlock<T> {
    taps: Vec<T>,
    buffer: Vec<T>,
    ntaps: usize,
}

impl<T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<Output = T>> FirBlock<T> {
    pub fn new(taps: Vec<T>) -> Self {
        let ntaps = taps.len();
        Self {
            taps,
            buffer: Vec::new(),
            ntaps,
        }
    }

    pub fn set_taps(&mut self, taps: Vec<T>) {
        self.taps = taps;
        self.ntaps = self.taps.len();
        self.buffer.clear();
    }

    ///_trait method: process `nsamples` from `input` into `output`.
    /// Returns number of samples produced (<= nsamples - ntaps + 1 typically).
    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        // Append input to buffer
        self.buffer.extend_from_slice(input);

        let max_output = self.buffer.len().saturating_sub(self.ntaps - 1);
        let to_produce = max_output.min(output.len());

        for i in 0..to_produce {
            let mut sum = T::default();
            for (j, &tap) in self.taps.iter().enumerate() {
                sum = sum + self.buffer[i + j] * tap;
            }
            output[i] = sum;
        }

        // Shift unconsumed samples to front
        let consumed = to_produce;
        self.buffer.drain(0..consumed);

        to_produce
    }
}

// ---------------------------------------------------------------------------
// Decimating FIR block
// ---------------------------------------------------------------------------

pub struct DecimatingFirBlock<T> {
    taps: Vec<T>,
    buffer: Vec<T>,
    ntaps: usize,
    decimation: usize,
    decim_pos: usize,
}

impl<T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<Output = T>> DecimatingFirBlock<T> {
    pub fn new(taps: Vec<T>, decimation: usize) -> Self {
        let ntaps = taps.len();
        Self {
            taps,
            buffer: Vec::new(),
            ntaps,
            decimation: decimation.max(1),
            decim_pos: 1,
        }
    }

    pub fn set_taps(&mut self, taps: Vec<T>) {
        self.taps = taps;
        self.ntaps = self.taps.len();
        self.buffer.clear();
        self.decim_pos = 1;
    }

    pub fn set_decimation(&mut self, decimation: usize) {
        self.decimation = decimation.max(1);
    }

    pub fn process(&mut self, input: &[T], output: &mut [T]) -> usize {
        self.buffer.extend_from_slice(input);

        let max_output = self.buffer.len().saturating_sub(self.ntaps - 1);
        let mut produced = 0;

        for i in 0..max_output {
            if self.decim_pos >= self.decimation {
                if produced >= output.len() {
                    break;
                }
                let mut sum = T::default();
                for (j, &tap) in self.taps.iter().enumerate() {
                    sum = sum + self.buffer[i + j] * tap;
                }
                output[produced] = sum;
                produced += 1;
                self.decim_pos = 1;
            } else {
                self.decim_pos += 1;
            }
        }

        // Shift unconsumed samples; account for samples consumed by the filter
        let filter_consumed = max_output;
        self.buffer.drain(0..filter_consumed);

        produced
    }
}

// ---------------------------------------------------------------------------
// FFT-based filter block (overlap-add / block FFT convolution)
// ---------------------------------------------------------------------------

pub struct FftFilterBlock {
    ntaps: usize,
    fftsize: usize,
    nsamples: usize,
    fft_taps_buffer: Vec<Complex32>,
    tail: Vec<Complex32>,
    buffer: Vec<Complex32>,
    fft_fwd_in: Vec<Complex32>,
    fft_fwd_out: Vec<Complex32>,
    fft_inv_in: Vec<Complex32>,
    fft_inv_out: Vec<Complex32>,
    fft_fwd: std::sync::Arc<dyn rustfft::Fft<f32>>,
    fft_inv: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl FftFilterBlock {
    pub fn new(taps: &[f32]) -> Self {
        let ntaps = taps.len();
        let fftsize = next_power_of_two(ntaps * 2 - 1);
        let nsamples = fftsize - ntaps + 1;

        let mut planner = RustFftPlanner::new();
        let fft_fwd = planner.plan_fft(fftsize, FftDirection::Forward);
        let fft_inv = planner.plan_fft(fftsize, FftDirection::Inverse);

        // Precompute FFT of taps (padded to fftsize)
        let mut fft_taps_buffer = vec![Complex::new(0.0, 0.0); fftsize];
        for (i, &tap) in taps.iter().enumerate() {
            fft_taps_buffer[i] = Complex::new(tap, 0.0);
        }

        fft_fwd.process(&mut fft_taps_buffer);

        Self {
            ntaps,
            fftsize,
            nsamples,
            fft_taps_buffer,
            tail: vec![Complex::new(0.0, 0.0); ntaps - 1],
            buffer: Vec::new(),
            fft_fwd_in: vec![Complex::new(0.0, 0.0); fftsize],
            fft_fwd_out: vec![Complex::new(0.0, 0.0); fftsize],
            fft_inv_in: vec![Complex::new(0.0, 0.0); fftsize],
            fft_inv_out: vec![Complex::new(0.0, 0.0); fftsize],
            fft_fwd,
            fft_inv,
        }
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut [Complex32]) -> usize {
        self.buffer.extend_from_slice(input);

        let mut produced = 0;
        let nsamples = self.nsamples;
        let fftsize = self.fftsize;

        while self.buffer.len() >= nsamples {
            if produced + nsamples > output.len() {
                break;
            }

            // Copy nsamples to forward FFT input, zero-pad rest
            self.fft_fwd_in[..nsamples]
                .iter_mut()
                .zip(self.buffer[..nsamples].iter())
                .for_each(|(dst, &src)| *dst = src);
            for val in &mut self.fft_fwd_in[nsamples..] {
                *val = Complex::new(0.0, 0.0);
            }

            self.fft_fwd.process(&mut self.fft_fwd_in);

            // Multiply in frequency domain
            for (i, tap) in self.fft_taps_buffer.iter().enumerate() {
                self.fft_inv_in[i] = self.fft_fwd_in[i] * tap;
            }

            self.fft_inv.process(&mut self.fft_inv_in);

            // Add tail from previous block and save new tail
            for j in 0..(self.ntaps - 1) {
                self.fft_inv_in[j] = self.fft_inv_in[j] + self.tail[j];
            }

            // Copy nsamples to output
            output[produced..produced + nsamples]
                .copy_from_slice(&self.fft_inv_in[..nsamples]);

            // Copy tail for next block
            self.tail.copy_from_slice(
                &self.fft_inv_in[nsamples..nsamples + self.ntaps - 1],
            );

            produced += nsamples;
            self.buffer.drain(0..nsamples);
        }

        produced
    }
}

fn next_power_of_two(n: usize) -> usize {
    let mut p = 1usize;
    while p < n {
        p <<= 1;
    }
    p
}
