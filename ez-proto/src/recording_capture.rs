//! Immutable source-clock contract for one IQ recording file.
use crate::CaptureClock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingCapture {
    pub first_clock: CaptureClock,
    pub center_frequency_hz: u64,
    pub sample_rate_hz: u32,
    pub samples: u64,
    pub next_source_sample_index: u64,
}

impl RecordingCapture {
    /// Check before accepting bytes. A rejected block must never reach the file.
    pub fn accept(
        previous: Option<&Self>,
        clock: CaptureClock,
        center_frequency_hz: u64,
        sample_rate_hz: u32,
        samples: u64,
    ) -> Result<Self, String> {
        if clock.sample_stride == 0
            || sample_rate_hz == 0
            || clock.source_sample_rate_hz / clock.sample_stride != sample_rate_hz
        {
            return Err("Recording stopped: invalid acquisition sample clock".into());
        }
        let next = clock
            .offset_samples(samples)
            .ok_or("Recording stopped: acquisition sample index overflow")?;
        if let Some(previous) = previous {
            if previous.first_clock.epoch != clock.epoch {
                return Err(format!(
                    "Recording stopped: capture epoch changed from {} to {}",
                    previous.first_clock.epoch, clock.epoch
                ));
            }
            if previous.center_frequency_hz != center_frequency_hz
                || previous.sample_rate_hz != sample_rate_hz
                || previous.first_clock.source_sample_rate_hz != clock.source_sample_rate_hz
                || previous.first_clock.sample_stride != clock.sample_stride
                || previous.first_clock.origin_unix_ns != clock.origin_unix_ns
            {
                return Err("Recording stopped: capture tuning or sample clock changed".into());
            }
            if clock.discontinuous || previous.next_source_sample_index != clock.source_sample_index
            {
                return Err(format!("Recording stopped: samples lost or rewound (expected source sample {}, received {})", previous.next_source_sample_index, clock.source_sample_index));
            }
        }
        Ok(Self {
            first_clock: previous.map_or(clock, |previous| previous.first_clock),
            center_frequency_hz,
            sample_rate_hz,
            samples: previous
                .map_or(0, |previous| previous.samples)
                .checked_add(samples)
                .ok_or("Recording stopped: recorded sample count overflow")?,
            next_source_sample_index: next.source_sample_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_recording_accepts_only_contiguous_same_epoch_geometry() {
        let mut first = CaptureClock::new(1, 700, 16_800_003, None);
        first.sample_stride = 7;
        first.discontinuous = true; // starting a file at a boundary is valid
        let span = RecordingCapture::accept(None, first, 1090, 2_400_000, 10).unwrap();
        let mut next = first.offset_samples(10).unwrap();
        next.discontinuous = false;
        let continued = RecordingCapture::accept(Some(&span), next, 1090, 2_400_000, 10).unwrap();
        assert_eq!(continued.samples, 20);
        assert_eq!(continued.next_source_sample_index, 840);
        assert_eq!(continued.first_clock, first);
        for mutation in 0..7 {
            let mut bad = next;
            let mut center = 1090;
            let mut rate = 2_400_000;
            match mutation {
                0 => bad.epoch += 1,
                1 => center += 1,
                2 => {
                    bad.source_sample_rate_hz = 14_000_000;
                    rate = 2_000_000;
                }
                3 => bad.source_sample_index += 7,
                4 => bad.discontinuous = true,
                5 => bad.origin_unix_ns = Some(10),
                _ => bad.source_sample_index -= 7,
            }
            assert!(RecordingCapture::accept(Some(&span), bad, center, rate, 10).is_err());
        }
    }

    #[test]
    fn invalid_recording_clocks_fail_without_wrapping() {
        assert!(
            RecordingCapture::accept(None, CaptureClock::new(1, u64::MAX, 1, None), 1, 1, 1)
                .is_err()
        );
        let mut clock = CaptureClock::new(1, 0, 1, None);
        clock.sample_stride = 0;
        assert!(RecordingCapture::accept(None, clock, 1, 1, 1).is_err());
        assert!(RecordingCapture::accept(None, CaptureClock::new(1, 0, 2, None), 1, 1, 1).is_err());
    }
}
