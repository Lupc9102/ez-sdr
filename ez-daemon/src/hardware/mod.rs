//! Hardware abstraction: a single trait, [`IqSource`], that every wideband sample
//! producer implements — synthetic generator, file replay, or a real SDR. The daemon's
//! ingestion thread (`crate::ingest`) is generic over `Box<dyn IqSource>`, so swapping
//! hardware never touches downstream code (bus, channelizer, pipelines).

pub mod replay;
pub mod synthetic;

#[cfg(feature = "rtlsdr")]
pub mod rtlsdr;

#[cfg(feature = "hackrf")]
pub mod hackrf;

#[cfg(feature = "soapy")]
pub mod soapy;

use anyhow::Result;
use num_complex::Complex32;

/// A source of wideband complex IQ samples.
///
/// Implementations own whatever hardware/file/synthetic state they need. `read_iq` is a
/// blocking pull call made from a dedicated ingestion thread (never the async runtime),
/// mirroring `dump1090::sdr::SdrSource`'s pull model but yielding raw `Complex32` I/Q
/// (phase-preserving) rather than pre-computed magnitude, since the spectrum and LRPT
/// pipelines both need phase information the magnitude-only trait discards.
pub trait IqSource: Send {
    fn start(&mut self) -> Result<()>;
    fn stop(&mut self);

    fn set_frequency(&mut self, hz: u64) -> Result<()>;
    fn set_sample_rate(&mut self, hz: u32) -> Result<()>;
    fn set_gain(&mut self, db: f64) -> Result<()>;

    /// Fills as much of `buf` as available and returns the number of samples written.
    /// A return of `0` means end-of-stream (only expected from finite sources such as a
    /// non-looping file replay) — live/synthetic sources always block until they have at
    /// least one sample rather than returning `0`.
    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize>;

    fn frequency_hz(&self) -> u64;
    fn sample_rate_hz(&self) -> u32;
    fn gain_db(&self) -> f64;

    /// Short, human-readable identifier for status reporting (`"synthetic"`, `"replay"`,
    /// `"rtlsdr"`, ...).
    fn kind(&self) -> &'static str;
}
