//! SDR abstraction - translated from sdr.c

#[cfg(feature = "hackrf")]
pub mod hackrf;
pub mod ifile;
#[cfg(feature = "rtlsdr")]
pub mod rtlsdr;
#[cfg(feature = "soapy")]
pub mod soapy;

use anyhow::Result;

/// Common interface to different SDR inputs.
pub trait SdrSource: Send {
    /// Start the SDR stream / open the file.
    ///
    /// # Errors
    /// Returns an error if the device cannot be opened or the stream fails to start.
    fn start(&mut self) -> Result<()>;

    /// Stop streaming.
    fn stop(&mut self);

    /// Set center frequency in Hz.
    ///
    /// # Errors
    /// Returns an error if the device does not support the requested frequency.
    fn set_frequency(&mut self, freq: u64) -> Result<()>;

    /// Set sample rate in Hz.
    ///
    /// # Errors
    /// Returns an error if the device does not support the requested sample rate.
    fn set_sample_rate(&mut self, rate: u32) -> Result<()>;

    /// Set gain in dB.
    ///
    /// # Errors
    /// Returns an error if the device does not support the requested gain.
    fn set_gain(&mut self, gain: f64) -> Result<()>;

    /// Read magnitude samples into `buf`. Returns number of samples placed.
    ///
    /// # Errors
    /// Returns an error if the read operation fails.
    fn read_samples(&mut self, buf: &mut [u16]) -> Result<usize>;
}
