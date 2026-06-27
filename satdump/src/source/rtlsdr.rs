//! RTL-SDR source - translated from plugins/sdr_sources/rtlsdr_sdr_support

use super::Source;
use anyhow::{anyhow, Result};
use num_complex::Complex32;
use std::time::Duration;

/// Hardcoded RTL-SDR sample rates from legacy backend.
const VALID_SAMPLERATES: [u32; 11] = [
    250_000,
    1_024_000,
    1_536_000,
    1_792_000,
    1_920_000,
    2_048_000,
    2_160_000,
    2_400_000,
    2_560_000,
    2_880_000,
    3_200_000,
];

/// RTL-SDR source implementing the common `Source` trait.
pub struct RtlSdrSource {
    serial: String,
    dev: Option<rtlsdr::RTLSDRDevice>,
    sample_rate: u32,
    frequency: u64,
    gain_db: f32,
    lna_agc: bool,
    tuner_agc: bool,
    bias: bool,
    ppm: i32,
    available_gains: Vec<i32>,
}

impl RtlSdrSource {
    /// Create a new RTL-SDR source targeting the given serial.
    pub fn new(serial: String) -> Self {
        Self {
            serial,
            dev: None,
            sample_rate: 2_048_000,
            frequency: 100_000_000,
            gain_db: 0.0,
            lna_agc: false,
            tuner_agc: false,
            bias: false,
            ppm: 0,
            available_gains: vec![0, 496],
        }
    }

    fn apply_gain(&mut self) -> Result<()> {
        let dev = self.dev.as_mut().ok_or_else(|| anyhow!("Device not open"))?;

        for _ in 0..20 {
            if dev.set_agc_mode(self.lna_agc).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        for _ in 0..20 {
            if dev.set_tuner_gain_mode(!self.tuner_agc).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        if !self.tuner_agc {
            let target = (self.gain_db * 10.0).round() as i32;
            let idx = self.available_gains.partition_point(|&g| g < target);
            let nearest = if idx == self.available_gains.len() {
                *self.available_gains.last().unwrap_or(&0)
            } else {
                self.available_gains[idx]
            };

            for _ in 0..20 {
                if dev.set_tuner_gain(nearest).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        Ok(())
    }

    fn apply_ppm(&mut self) -> Result<()> {
        if self.ppm == 0 {
            return Ok(());
        }
        let dev = self.dev.as_mut().ok_or_else(|| anyhow!("Device not open"))?;
        for _ in 0..20 {
            if dev.set_freq_correction(self.ppm).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    fn apply_bias(&mut self) -> Result<()> {
        // The `rtlsdr` crate (0.1) does not expose bias-tee control.
        // The raw librtlsdr function `rtlsdr_set_bias_tee` would need to be
        // called via rtlsdr_sys if that dependency is added in the future.
        let _ = self.dev.as_mut();
        Ok(())
    }
}

impl Source for RtlSdrSource {
    fn open(&mut self) -> Result<()> {
        if self.dev.is_some() {
            return Ok(());
        }

        let idx = rtlsdr::get_index_by_serial(self.serial.clone())
            .map_err(|e| anyhow!("Failed to find RTL-SDR by serial: {}", e))?;
        let mut dev = rtlsdr::open(idx).map_err(|e| anyhow!("Failed to open RTL-SDR: {}", e))?;

        match dev.get_tuner_gains() {
            Ok(mut gains) => {
                gains.sort();
                self.available_gains = gains;
            }
            Err(_) => {}
        }

        dev.set_sample_rate(self.sample_rate)
            .map_err(|e| anyhow!("Failed to set sample rate: {}", e))?;
        dev.reset_buffer()
            .map_err(|e| anyhow!("Failed to reset buffer: {}", e))?;

        self.dev = Some(dev);

        self.set_freq(self.frequency)?;
        self.apply_gain()?;
        self.apply_ppm()?;
        self.apply_bias()?;

        Ok(())
    }

    fn set_freq(&mut self, freq: u64) -> Result<()> {
        self.frequency = freq;
        if let Some(ref mut dev) = self.dev {
            let freq_u32 = freq as u32;
            let mut attempts = 0;
            while attempts < 20 && dev.set_center_freq(freq_u32).is_err() {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            if attempts == 20 {
                return Err(anyhow!("Unable to set RTL-SDR frequency"));
            }
            // PLL lock workaround for frequencies > 1 GHz
            if freq > 1_000_000_000 {
                let _ = dev.set_center_freq((freq - 1_000_000_000) as u32);
                let _ = dev.set_center_freq(freq_u32);
            }
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, sr: u32) -> Result<()> {
        if !VALID_SAMPLERATES.contains(&sr) {
            return Err(anyhow!("Unsupported samplerate: {}", sr));
        }
        self.sample_rate = sr;
        if let Some(ref mut dev) = self.dev {
            dev.set_sample_rate(sr)
                .map_err(|e| anyhow!("Failed to set sample rate: {}", e))?;
        }
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        let dev = self.dev.as_mut().ok_or_else(|| anyhow!("Device not open"))?;
        let byte_count = buf.len() * 2;
        let data = dev
            .read_sync(byte_count)
            .map_err(|e| anyhow!("read_sync failed: {}", e))?;

        let nsamples = data.len() / 2;
        for i in 0..nsamples {
            let re = (data[i * 2] as f32 - 127.4) / 128.0;
            let im = (data[i * 2 + 1] as f32 - 127.4) / 128.0;
            buf[i] = Complex32::new(re, im);
        }
        Ok(nsamples)
    }
}
