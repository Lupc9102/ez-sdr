//! SoapySDR source - translated from plugins/sdr_sources/soapy_sdr_support

use super::Source;
use anyhow::{anyhow, Result};
use num_complex::Complex32;
use soapysdr::Direction;

/// SoapySDR source implementing the common `Source` trait.
pub struct SoapySdrSource {
    label: String,
    channel: usize,
    dev: Option<soapysdr::Device>,
    stream: Option<soapysdr::RxStream<Complex32>>,
    sample_rate: f64,
    frequency: f64,
    antenna: String,
    gains: Vec<(String, f64)>,
}

impl SoapySdrSource {
    /// Create a new SoapySDR source targeting the given device label.
    pub fn new(label: String) -> Self {
        Self {
            label,
            channel: 0,
            dev: None,
            stream: None,
            sample_rate: 1_024_000.0,
            frequency: 100e6,
            antenna: String::new(),
            gains: Vec::new(),
        }
    }

    fn apply_params(&mut self) -> Result<()> {
        let dev = self.dev.as_ref().ok_or_else(|| anyhow!("Device not open"))?;

        if !self.antenna.is_empty() {
            dev.set_antenna(Direction::Rx, self.channel, &self.antenna)
                .map_err(|e| anyhow!("set_antenna failed: {}", e))?;
        }

        dev.set_sample_rate(Direction::Rx, self.channel, self.sample_rate)
            .map_err(|e| anyhow!("set_sample_rate failed: {}", e))?;

        dev.set_frequency(Direction::Rx, self.channel, self.frequency)
            .map_err(|e| anyhow!("set_frequency failed: {}", e))?;

        for (name, val) in &self.gains {
            dev.set_gain_element(Direction::Rx, self.channel, name, *val)
                .map_err(|e| anyhow!("set_gain_element failed: {}", e))?;
        }

        Ok(())
    }
}

impl Source for SoapySdrSource {
    fn open(&mut self) -> Result<()> {
        if self.dev.is_some() {
            return Ok(());
        }

        let devices = soapysdr::enumerate("")
            .map_err(|e| anyhow!("SoapySDR enumerate failed: {}", e))?;

        let mut found = None;
        for args in devices {
            let label = args.get("label").unwrap_or("");
            let driver = args.get("driver").unwrap_or("");
            let name = if !label.is_empty() { label } else { driver };
            let display_name = format!("{} [Soapy]", name);

            if display_name == self.label {
                found = Some(args);
                break;
            }
        }

        let args = found.ok_or_else(|| anyhow!("SoapySDR device '{}' not found", self.label))?;
        let dev = soapysdr::Device::new(args)
            .map_err(|e| anyhow!("SoapySDR open failed: {}", e))?;

        let antennas = dev
            .antennas(Direction::Rx, self.channel)
            .map_err(|e| anyhow!("listAntennas failed: {}", e))?;
        if let Some(a) = antennas.first() {
            self.antenna.clone_from(a);
        }

        let gain_names = dev
            .list_gains(Direction::Rx, self.channel)
            .map_err(|e| anyhow!("listGains failed: {}", e))?;
        self.gains = gain_names.into_iter().map(|n| (n, 0.0)).collect();

        self.dev = Some(dev);
        self.apply_params()?;

        let dev = self.dev.as_ref().unwrap();
        let mut stream = dev
            .rx_stream::<Complex32>(&[self.channel])
            .map_err(|e| anyhow!("setupStream failed: {}", e))?;
        stream
            .activate(None)
            .map_err(|e| anyhow!("activateStream failed: {}", e))?;

        self.stream = Some(stream);
        Ok(())
    }

    fn set_freq(&mut self, freq: u64) -> Result<()> {
        self.frequency = freq as f64;
        if let Some(ref dev) = self.dev {
            dev.set_frequency(Direction::Rx, self.channel, self.frequency)
                .map_err(|e| anyhow!("set_frequency failed: {}", e))?;
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, sr: u32) -> Result<()> {
        self.sample_rate = sr as f64;
        if let Some(ref dev) = self.dev {
            dev.set_sample_rate(Direction::Rx, self.channel, self.sample_rate)
                .map_err(|e| anyhow!("set_sample_rate failed: {}", e))?;
        }
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow!("Stream not open"))?;
        let n = stream
            .read(std::slice::from_ref(&buf), 1_000_000)
            .map_err(|e| anyhow!("readStream failed: {}", e))?;
        Ok(n)
    }
}
