//! Deterministic synthetic IQ generator. Always compiled (no system SDR libraries
//! required), used as the daemon's default source and throughout the test suite where a
//! believable, reproducible wideband signal is needed without real hardware.

use std::f32::consts::PI;

use anyhow::Result;
use num_complex::Complex32;

use super::IqSource;

/// A tiny, dependency-free xorshift64* PRNG. Not cryptographic — just needs to be fast
/// and reproducible from a fixed seed so tests are deterministic.
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform float in `[-1.0, 1.0]`.
    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
    }
}

/// One synthetic carrier: a constant-power tone at `offset_hz` from center, with slow AM
/// and FM modulation so it exercises both spectrum-peak detection and audio demod paths.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticTone {
    pub offset_hz: f32,
    pub amplitude: f32,
    pub fm_deviation_hz: f32,
    pub fm_rate_hz: f32,
}

#[derive(Debug, Clone)]
pub struct SyntheticConfig {
    pub noise_amplitude: f32,
    pub tones: Vec<SyntheticTone>,
    pub seed: u64,
}

impl Default for SyntheticConfig {
    fn default() -> Self {
        Self {
            noise_amplitude: 0.02,
            tones: vec![
                // Strong, unmodulated carrier — a clean spectral peak for FFT tests.
                SyntheticTone {
                    offset_hz: 250_000.0,
                    amplitude: 0.6,
                    fm_deviation_hz: 0.0,
                    fm_rate_hz: 0.0,
                },
                // A narrowband FM "voice-like" tone for the audio/demod pipeline.
                SyntheticTone {
                    offset_hz: -400_000.0,
                    amplitude: 0.35,
                    fm_deviation_hz: 3_000.0,
                    fm_rate_hz: 400.0,
                },
            ],
            seed: 0xE255_5D00,
        }
    }
}

/// Free function so unit tests (e.g. the channelizer's tone-translation test) can
/// generate a known signal without spinning up a full [`SyntheticSource`].
pub fn generate_block(
    config: &SyntheticConfig,
    rng: &mut SynthesisState,
    sample_rate_hz: u32,
    start_sample_index: u64,
    out: &mut [Complex32],
) {
    let fs = sample_rate_hz as f32;
    for (n, sample) in out.iter_mut().enumerate() {
        let t = (start_sample_index + n as u64) as f32 / fs;
        let mut acc = Complex32::new(0.0, 0.0);
        for tone in &config.tones {
            let inst_freq = if tone.fm_deviation_hz > 0.0 {
                tone.offset_hz + tone.fm_deviation_hz * (2.0 * PI * tone.fm_rate_hz * t).sin()
            } else {
                tone.offset_hz
            };
            // Integrate instantaneous frequency into phase incrementally so FM stays
            // continuous across block boundaries (matches how a real oscillator behaves).
            let phase = rng.phase_accum(tone.offset_hz as i64, inst_freq, fs);
            acc += Complex32::new(tone.amplitude * phase.cos(), tone.amplitude * phase.sin());
        }
        acc += Complex32::new(
            config.noise_amplitude * rng.rng.next_f32(),
            config.noise_amplitude * rng.rng.next_f32(),
        );
        *sample = acc;
    }
}

/// Bundles the PRNG together with a per-tone running phase accumulator, keyed by the
/// tone's nominal offset, so FM stays phase-continuous across successive `read_iq` calls
/// without callers having to thread state through manually.
pub struct SynthesisState {
    rng: Xorshift64,
    phases: std::collections::HashMap<i64, f32>,
}

impl SynthesisState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Xorshift64::new(seed),
            phases: std::collections::HashMap::new(),
        }
    }

    fn phase_accum(&mut self, key: i64, inst_freq_hz: f32, sample_rate_hz: f32) -> f32 {
        let phase = self.phases.entry(key).or_insert(0.0);
        let next = *phase + 2.0 * PI * inst_freq_hz / sample_rate_hz;
        *phase = next.rem_euclid(2.0 * PI);
        next
    }
}

pub struct SyntheticSource {
    config: SyntheticConfig,
    rng: SynthesisState,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
    sample_index: u64,
    running: bool,
}

impl SyntheticSource {
    #[must_use]
    pub fn new(config: SyntheticConfig) -> Self {
        let seed = config.seed;
        Self {
            config,
            rng: SynthesisState::new(seed),
            frequency_hz: 433_000_000,
            sample_rate_hz: 2_048_000,
            gain_db: 20.0,
            sample_index: 0,
            running: false,
        }
    }
}

impl Default for SyntheticSource {
    fn default() -> Self {
        Self::new(SyntheticConfig::default())
    }
}

impl IqSource for SyntheticSource {
    fn start(&mut self) -> Result<()> {
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        self.frequency_hz = hz;
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        self.sample_rate_hz = hz;
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        self.gain_db = db;
        Ok(())
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        generate_block(
            &self.config,
            &mut self.rng,
            self.sample_rate_hz,
            self.sample_index,
            buf,
        );
        self.sample_index += buf.len() as u64;
        Ok(buf.len())
    }

    fn frequency_hz(&self) -> u64 {
        self.frequency_hz
    }

    fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    fn gain_db(&self) -> f64 {
        self.gain_db
    }

    fn kind(&self) -> &'static str {
        "synthetic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_requested_sample_count_every_call() {
        let mut src = SyntheticSource::default();
        src.start().unwrap();
        let mut buf = vec![Complex32::new(0.0, 0.0); 4096];
        for _ in 0..8 {
            let n = src.read_iq(&mut buf).unwrap();
            assert_eq!(n, buf.len());
        }
    }

    #[test]
    fn strong_tone_is_the_dominant_spectral_peak() {
        // Sanity check the generator itself: run an FFT over one block and confirm the
        // unmodulated 250kHz tone (amplitude 0.6, by far the strongest component) lands
        // in the expected bin. This is what the spectrum pipeline and channelizer tests
        // build on, so it needs to be trustworthy on its own.
        let sample_rate = 2_048_000u32;
        let fft_len = 8192usize;
        let mut src = SyntheticSource::new(SyntheticConfig {
            noise_amplitude: 0.0,
            tones: vec![SyntheticTone {
                offset_hz: 250_000.0,
                amplitude: 0.6,
                fm_deviation_hz: 0.0,
                fm_rate_hz: 0.0,
            }],
            seed: 42,
        });
        src.set_sample_rate(sample_rate).unwrap();
        src.start().unwrap();
        let mut buf = vec![Complex32::new(0.0, 0.0); fft_len];
        src.read_iq(&mut buf).unwrap();

        let mut planner = rustfft::FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_len);
        fft.process(&mut buf);

        let (peak_bin, _) = buf
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.norm().partial_cmp(&b.1.norm()).unwrap())
            .unwrap();
        // Bin ordering pre-fftshift: bin k corresponds to k * fs/N for k < N/2.
        let peak_freq = if peak_bin <= fft_len / 2 {
            peak_bin as f32 * sample_rate as f32 / fft_len as f32
        } else {
            (peak_bin as f32 - fft_len as f32) * sample_rate as f32 / fft_len as f32
        };
        assert!(
            (peak_freq - 250_000.0).abs() < sample_rate as f32 / fft_len as f32 * 2.0,
            "expected peak near 250kHz, got {peak_freq}Hz (bin {peak_bin})"
        );
    }
}
