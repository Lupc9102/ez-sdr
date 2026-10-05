//! Acquisition clocks retained across buffering, replay and integer decimation.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique within the running process; zero is reserved for unannotated input.
pub fn new_capture_epoch() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureClock {
    pub epoch: u64,
    /// Index of this block's first sample in the original acquisition clock.
    pub source_sample_index: u64,
    pub source_sample_rate_hz: u32,
    /// Original acquisition samples between successive samples in this block.
    pub sample_stride: u32,
    /// Acquisition sample zero's Unix time, when known. Replay never invents it.
    pub origin_unix_ns: Option<u64>,
    pub discontinuous: bool,
}

impl CaptureClock {
    pub fn new(
        epoch: u64,
        source_sample_index: u64,
        source_sample_rate_hz: u32,
        origin_unix_ns: Option<u64>,
    ) -> Self {
        Self {
            epoch,
            source_sample_index,
            source_sample_rate_hz,
            sample_stride: 1,
            origin_unix_ns,
            discontinuous: false,
        }
    }

    /// Time since original sample zero, unaffected by replay speed or rounding
    /// the decimated sample rate to an integer number of Hz.
    pub fn relative_time_ns(&self, sample_offset: u64) -> Option<u64> {
        if self.source_sample_rate_hz == 0 || self.sample_stride == 0 {
            return None;
        }
        let index = u128::from(self.source_sample_index)
            + u128::from(sample_offset) * u128::from(self.sample_stride);
        u64::try_from(index * 1_000_000_000 / u128::from(self.source_sample_rate_hz)).ok()
    }

    pub fn unix_time_ns(&self, sample_offset: u64) -> Option<u64> {
        self.origin_unix_ns?
            .checked_add(self.relative_time_ns(sample_offset)?)
    }

    pub fn exact_sample_rate_hz(&self) -> f64 {
        f64::from(self.source_sample_rate_hz) / f64::from(self.sample_stride.max(1))
    }

    pub fn offset_samples(&self, samples: u64) -> Option<Self> {
        let mut clock = *self;
        clock.source_sample_index = clock
            .source_sample_index
            .checked_add(samples.checked_mul(u64::from(clock.sample_stride))?)?;
        Some(clock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimated_clock_retains_exact_acquisition_times_and_unknown_utc() {
        let mut clock = CaptureClock::new(1, 700, 2_048_003, Some(1_000_000));
        clock.sample_stride = 7;
        assert_eq!(clock.exact_sample_rate_hz(), 2_048_003.0 / 7.0);
        assert_eq!(
            clock.relative_time_ns(100),
            Some(1400 * 1_000_000_000 / 2_048_003)
        );
        assert_eq!(clock.offset_samples(100).unwrap().source_sample_index, 1400);
        assert_eq!(
            clock.unix_time_ns(100),
            Some(1_000_000 + 1400 * 1_000_000_000 / 2_048_003)
        );
        clock.origin_unix_ns = None;
        assert_eq!(clock.unix_time_ns(100), None);
        clock.source_sample_rate_hz = 0;
        assert_eq!(clock.relative_time_ns(0), None);
    }
}
