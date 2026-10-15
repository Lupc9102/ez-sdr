//! Bounded callback IQ bridge with acquisition indices assigned before queue admission.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::ReadPending;
use anyhow::Result;
use ez_proto::CaptureClock;
use num_complex::Complex32;

struct Transfer {
    first_sample: u64,
    bytes: Vec<u8>,
}
#[derive(Default)]
struct Acquisition {
    next_sample: u64,
    pending_component: Option<u8>,
}
pub(crate) struct CallbackProducer {
    tx: SyncSender<Transfer>,
    acquisition: Mutex<Acquisition>,
    dropped_samples: AtomicU64,
    signed: bool,
}
impl CallbackProducer {
    /// False means the consumer disappeared or the acquisition index overflowed.
    /// A full queue drops complete sample pairs and advances the acquisition clock.
    pub(crate) fn push(&self, bytes: &[u8]) -> bool {
        let Ok(mut state) = self.acquisition.lock() else {
            return false;
        };
        let mut data = Vec::with_capacity(bytes.len() + 1);
        if let Some(component) = state.pending_component.take() {
            data.push(component);
        }
        data.extend_from_slice(bytes);
        if !data.len().is_multiple_of(2) {
            state.pending_component = data.pop();
        }
        if data.is_empty() {
            return true;
        }
        let samples = (data.len() / 2) as u64;
        let first_sample = state.next_sample;
        let Some(next_sample) = first_sample.checked_add(samples) else {
            return false;
        };
        state.next_sample = next_sample;
        if self.signed {
            for byte in &mut data {
                *byte ^= 0x80;
            }
        }
        match self.tx.try_send(Transfer {
            first_sample,
            bytes: data,
        }) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.dropped_samples.fetch_add(samples, Ordering::Relaxed);
                true
            }
            Err(TrySendError::Disconnected(_)) => {
                self.dropped_samples.fetch_add(samples, Ordering::Relaxed);
                false
            }
        }
    }
    pub(crate) fn dropped_samples(&self) -> u64 {
        self.dropped_samples.load(Ordering::Relaxed)
    }
}

pub(crate) struct CallbackReader {
    rx: Receiver<Transfer>,
    current: Option<Transfer>,
    consumed: usize,
    expected_sample: Option<u64>,
    epoch: u64,
    last_clock: Option<CaptureClock>,
}
impl CallbackReader {
    pub(crate) fn read(&mut self, output: &mut [Complex32], rate: u32) -> Result<usize> {
        self.read_with(output.len(), rate, |bytes| {
            let samples = lrpt_decode::uc8_bytes_to_normalized(bytes);
            output[..samples.len()].copy_from_slice(&samples);
        })
    }
    #[cfg(any(feature = "rtl-async", test))]
    pub(crate) fn read_uc8(&mut self, output: &mut [u8], rate: u32) -> Result<usize> {
        self.read_with(output.len() / 2, rate, |bytes| {
            output[..bytes.len()].copy_from_slice(bytes)
        })
        .map(|n| n * 2)
    }
    fn read_with(
        &mut self,
        capacity: usize,
        rate: u32,
        consume: impl FnOnce(&[u8]),
    ) -> Result<usize> {
        if self.current.is_none() {
            match self.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(transfer) => {
                    self.current = Some(transfer);
                    self.consumed = 0;
                }
                Err(RecvTimeoutError::Timeout) => return Err(ReadPending::Timeout.into()),
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let current = self.current.as_ref().unwrap();
        let first_sample = current.first_sample + (self.consumed / 2) as u64;
        let count = capacity.min((current.bytes.len() - self.consumed) / 2);
        let bytes = &current.bytes[self.consumed..self.consumed + count * 2];
        // One returned block belongs to one transfer. Never stitch a gap into it.
        consume(bytes);
        let mut clock = CaptureClock::new(self.epoch, first_sample, rate, None);
        clock.discontinuous = self.expected_sample != Some(first_sample);
        self.expected_sample = Some(first_sample + count as u64);
        self.last_clock = Some(clock);
        self.consumed += count * 2;
        if self.consumed == current.bytes.len() {
            self.current = None;
        }
        Ok(count)
    }
    pub(crate) fn capture_clock(&self) -> Option<CaptureClock> {
        self.last_clock
    }
}

pub(crate) fn channel(capacity: usize, signed: bool) -> (Arc<CallbackProducer>, CallbackReader) {
    let (tx, rx) = sync_channel(capacity);
    (
        Arc::new(CallbackProducer {
            tx,
            acquisition: Mutex::new(Acquisition::default()),
            dropped_samples: AtomicU64::new(0),
            signed,
        }),
        CallbackReader {
            rx,
            current: None,
            consumed: 0,
            expected_sample: None,
            epoch: ez_proto::new_capture_epoch(),
            last_clock: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsigned_raw_bridge_keeps_bytes_and_acquisition_clock_across_partial_reads() {
        let (tx, mut rx) = channel(1, false);
        let bytes = (0..=255).collect::<Vec<u8>>();
        assert!(tx.push(&bytes));
        assert!(tx.push(&[0, 1, 2, 3]));
        let mut out = [0; 255]; // Keep a complete sample boundary in an odd-sized output.
        assert_eq!(rx.read_uc8(&mut out, 2_400_000).unwrap(), 254);
        assert_eq!(&out[..254], &bytes[..254]);
        assert_eq!(rx.capture_clock().unwrap().source_sample_index, 0);
        assert_eq!(rx.read_uc8(&mut out, 2_400_000).unwrap(), 2);
        assert_eq!(&out[..2], &bytes[254..]);
        assert!(tx.push(&[22, 33]));
        assert_eq!(rx.read_uc8(&mut out, 2_400_000).unwrap(), 2);
        assert_eq!(&out[..2], &[22, 33]);
        assert_eq!(rx.capture_clock().unwrap().source_sample_index, 130);
        assert!(rx.capture_clock().unwrap().discontinuous);
    }

    #[test]
    fn overflow_counts_acquired_samples_and_never_stitches_retained_transfers() {
        let (tx, mut rx) = channel(1, true);
        assert!(tx.push(&[0, 1, 2, 3, 4, 5]));
        assert!(tx.push(&[6, 7, 8, 9])); // Full: loses two complete samples.
        assert_eq!(tx.dropped_samples(), 2);
        let mut out = [Complex32::default(); 8];
        assert_eq!(rx.read(&mut out[..1], 1_000_000).unwrap(), 1);
        let clock = rx.capture_clock().unwrap();
        assert_eq!(clock.source_sample_index, 0);
        assert!(clock.discontinuous);
        assert_eq!(rx.read(&mut out, 1_000_000).unwrap(), 2); // Transfer tail only.
        assert_eq!(rx.capture_clock().unwrap().source_sample_index, 1);
        assert!(!rx.capture_clock().unwrap().discontinuous);
        assert!(tx.push(&[10, 11, 12, 13]));
        assert_eq!(rx.read(&mut out, 1_000_000).unwrap(), 2);
        let after = rx.capture_clock().unwrap();
        assert_eq!(after.epoch, clock.epoch);
        assert_eq!(after.source_sample_index, 5);
        assert!(after.discontinuous);
        assert_eq!(
            &out[..2],
            &lrpt_decode::uc8_bytes_to_normalized(&[138, 139, 140, 141])
        );
        drop(tx);
        assert_eq!(rx.read(&mut out, 1_000_000).unwrap(), 0);
    }
    #[test]
    fn odd_callback_partitions_preserve_sample_pairs_and_signedness() {
        let bytes = (0..=255).collect::<Vec<u8>>();
        for width in [1, 3, 5, 17, 256] {
            let (tx, mut rx) = channel(512, true);
            for chunk in bytes.chunks(width) {
                assert!(tx.push(chunk));
            }
            drop(tx);
            let mut actual = Vec::new();
            let mut out = [Complex32::default(); 7];
            loop {
                let n = rx.read(&mut out, 1_000_000).unwrap();
                if n == 0 {
                    break;
                }
                assert_eq!(
                    rx.capture_clock().unwrap().source_sample_index,
                    actual.len() as u64
                );
                assert_eq!(rx.capture_clock().unwrap().discontinuous, actual.is_empty());
                actual.extend_from_slice(&out[..n]);
            }
            let expected = bytes.iter().map(|byte| byte ^ 0x80).collect::<Vec<_>>();
            assert_eq!(actual, lrpt_decode::uc8_bytes_to_normalized(&expected));
        }
    }
    #[test]
    fn silent_callback_queue_returns_to_cancellation_within_one_deadline() {
        let (_tx, mut rx) = channel(1, true);
        let start = std::time::Instant::now();
        let error = rx
            .read(&mut [Complex32::default(); 8], 1_000_000)
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ReadPending>(),
            Some(ReadPending::Timeout)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(rx.capture_clock().is_none());
    }
}
