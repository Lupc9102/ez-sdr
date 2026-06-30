//! SDR abstraction - translated from sdr.c

pub mod rtlsdr;
pub mod hackrf;
pub mod soapy;
pub mod ifile;

use anyhow::Result;

/// Common interface to different SDR inputs.
pub trait SdrSource: Send {
    /// Start the SDR stream / open the file.
    fn start(&mut self) -> Result<()>;

    /// Stop streaming.
    fn stop(&mut self);

    /// Set center frequency in Hz.
    fn set_frequency(&mut self, freq: u64) -> Result<()>;

    /// Set sample rate in Hz.
    fn set_sample_rate(&mut self, rate: u32) -> Result<()>;

    /// Set gain in dB.
    fn set_gain(&mut self, gain: f64) -> Result<()>;

    /// Read magnitude samples into `buf`. Returns number of samples placed.
    fn read_samples(&mut self, buf: &mut [u16]) -> Result<usize>;
}
