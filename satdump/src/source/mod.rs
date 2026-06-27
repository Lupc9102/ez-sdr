//! SDR sources - translated from plugins/sdr_sources/
pub mod file;

#[cfg(feature = "rtlsdr")]
pub mod rtlsdr;

#[cfg(feature = "soapy")]
pub mod soapy;

use anyhow::Result;
use num_complex::Complex32;

/// Common trait for SDR sample sources.
pub trait Source {
    /// Open the underlying device / stream.
    fn open(&mut self) -> Result<()>;

    /// Set centre frequency (Hz).
    fn set_freq(&mut self, freq: u64) -> Result<()>;

    /// Set sample rate (Hz).
    fn set_sample_rate(&mut self, sr: u32) -> Result<()>;

    /// Read complex F32 samples into `buf`.
    /// Returns the number of samples written.
    fn read_samples(&mut self, buf: &mut [Complex32]) -> Result<usize>;
}
