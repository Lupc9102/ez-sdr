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

/// A decoded message's exact source-sample coordinate. Demod2400 timestamps
/// are in fifths of an output sample, retained through integer decimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureTimestamp {
    pub epoch: u64,
    pub source_sample_index: u64,
    pub source_sample_phase_fifths: u8,
    pub source_sample_rate_hz: u32,
    pub origin_unix_ns: Option<u64>,
}

impl CaptureTimestamp {
    pub fn relative_time_ns(self) -> Option<u64> {
        if self.source_sample_rate_hz == 0 || self.source_sample_phase_fifths >= 5 {
            return None;
        }
        let fifths =
            u128::from(self.source_sample_index) * 5 + u128::from(self.source_sample_phase_fifths);
        u64::try_from(fifths * 1_000_000_000 / (u128::from(self.source_sample_rate_hz) * 5)).ok()
    }

    pub fn unix_time_ns(self) -> Option<u64> {
        self.origin_unix_ns?.checked_add(self.relative_time_ns()?)
    }
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

    /// Convert a Demod2400 message offset from this block's first sample.
    /// Negative offsets are legitimate for the previous block's overlap. The
    /// stride/rate ratio keeps fractional source clocks exact after decimation.
    pub fn message_timestamp_2400(self, offset_fifths: i128) -> Option<CaptureTimestamp> {
        if self.source_sample_rate_hz == 0 || self.sample_stride == 0 {
            return None;
        }
        let fifths = i128::from(self.source_sample_index)
            .checked_mul(5)?
            .checked_add(offset_fifths.checked_mul(i128::from(self.sample_stride))?)?;
        if fifths < 0 {
            return None;
        }
        Some(CaptureTimestamp {
            epoch: self.epoch,
            source_sample_index: u64::try_from(fifths / 5).ok()?,
            source_sample_phase_fifths: (fifths % 5) as u8,
            source_sample_rate_hz: self.source_sample_rate_hz,
            origin_unix_ns: self.origin_unix_ns,
        })
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
    fn message_capture_time_preserves_signed_overlap_phase_and_native_stride() {
        let mut clock = CaptureClock::new(11, 700, 16_800_003, Some(1_000_000));
        clock.sample_stride = 7;
        let stamp = clock.message_timestamp_2400(-499).unwrap();
        assert_eq!(stamp.epoch, 11);
        assert_eq!(stamp.source_sample_index, 1);
        assert_eq!(stamp.source_sample_phase_fifths, 2);
        assert_eq!(
            stamp.relative_time_ns(),
            Some(7 * 1_000_000_000 / (16_800_003 * 5))
        );
        assert_eq!(
            stamp.unix_time_ns(),
            Some(1_000_000 + stamp.relative_time_ns().unwrap())
        );
        let phase = clock.message_timestamp_2400(1234).unwrap();
        assert_eq!(phase.source_sample_index, 2427);
        assert_eq!(phase.source_sample_phase_fifths, 3);
        assert_eq!(
            phase.relative_time_ns(),
            Some(12_138 * 1_000_000_000 / (16_800_003 * 5))
        );
        clock.origin_unix_ns = None;
        assert_eq!(
            clock.message_timestamp_2400(1234).unwrap().unix_time_ns(),
            None
        );
        assert_eq!(clock.message_timestamp_2400(-501), None);
    }

    #[test]
    fn invalid_or_overflowing_message_clocks_fail_without_wrapping_or_inventing_utc() {
        let mut clock = CaptureClock::new(1, u64::MAX, 2_400_000, Some(u64::MAX));
        assert_eq!(clock.message_timestamp_2400(5), None);
        assert_eq!(clock.message_timestamp_2400(i128::MAX), None);
        let mut stamp = CaptureClock::new(1, 1, 1, Some(u64::MAX))
            .message_timestamp_2400(0)
            .unwrap();
        assert_eq!(stamp.unix_time_ns(), None);
        stamp.source_sample_phase_fifths = 5;
        assert_eq!(stamp.relative_time_ns(), None);
        clock.source_sample_rate_hz = 0;
        assert_eq!(clock.message_timestamp_2400(0), None);
        clock.source_sample_rate_hz = 2_400_000;
        clock.sample_stride = 0;
        assert_eq!(clock.message_timestamp_2400(0), None);
    }

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
