//! Dedicated hardware-ingestion thread: owns a `Box<dyn IqSource>` exclusively and pulls IQ
//! into the wideband [`SampleBus`]. Frequency/sample-rate/gain changes arrive over a small
//! non-blocking control channel polled once per read, rather than a `Mutex` guarding the
//! source itself — the hot path (`read_iq` + `publish`) never locks, matching the daemon's
//! zero-copy/lock-free ingestion requirement exactly as [`crate::broadcast`] does for fan-out.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender};
use num_complex::Complex32;

use ez_proto::HardwareStatus;

use crate::bus::{SampleBlock, SampleBus};
use crate::hardware::IqSource;

/// Samples pulled per `read_iq` call. Not a hard limit — sources may return fewer.
const READ_CHUNK: usize = 8_192;

/// Control-channel capacity. Only the latest frequency/rate/gain matters, so a
/// flood of stale commands is coalesced (oldest dropped) instead of growing
/// without bound — an unauthenticated client could otherwise OOM the daemon
/// faster than the blocking `read_iq` loop drains.
const COMMAND_CAPACITY: usize = 64;

enum HardwareCommand {
    Frequency(u64),
    SampleRate(u32),
    Gain(f64),
}

/// Cheap-to-clone handle for sending rare control commands to the ingestion thread and
/// reading its latest published [`HardwareStatus`] snapshot, without ever touching the
/// source itself (only the ingestion thread ever does that).
#[derive(Clone)]
pub struct IngestHandle {
    commands: Sender<HardwareCommand>,
    // A second handle on the same queue, used only to drop the oldest queued
    // command when a flood fills it (see `send_coalescing`). The ingestion
    // thread's own receiver is unaffected — both receivers share one queue.
    drain: Receiver<HardwareCommand>,
    status: Arc<ArcSwap<HardwareStatus>>,
}

impl IngestHandle {
    /// Requests a frequency change. Fire-and-forget: applied on the ingestion thread's next
    /// loop iteration, reflected shortly after in [`Self::status`].
    pub fn set_frequency(&self, hz: u64) {
        self.send_coalescing(HardwareCommand::Frequency(hz));
    }

    pub fn set_sample_rate(&self, hz: u32) {
        self.send_coalescing(HardwareCommand::SampleRate(hz));
    }

    pub fn set_gain(&self, db: f64) {
        self.send_coalescing(HardwareCommand::Gain(db));
    }

    /// Enqueue a command, dropping the oldest queued command when full. Control
    /// state is last-writer-wins (only the latest frequency/rate/gain matters),
    /// so coalescing is semantically lossless while bounding memory.
    fn send_coalescing(&self, cmd: HardwareCommand) {
        use crossbeam_channel::TrySendError;
        match self.commands.try_send(cmd) {
            Ok(()) => {}
            Err(TrySendError::Full(cmd)) => {
                let _ = self.drain.try_recv();
                let _ = self.commands.try_send(cmd);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    #[must_use]
    pub fn status(&self) -> HardwareStatus {
        (**self.status.load()).clone()
    }
}

/// Spawns the ingestion thread, which runs until `running` clears. Each loop iteration
/// applies any queued `HardwareCommand`s (non-blocking), then does one blocking `read_iq`
/// pull and publishes the result to `bus`. Returns once `source.start()` succeeds; the
/// thread itself calls `source.stop()` on exit (clean shutdown or a source error/EOF).
pub fn spawn(
    mut source: Box<dyn IqSource>,
    bus: SampleBus,
    running: Arc<AtomicBool>,
) -> anyhow::Result<(IngestHandle, JoinHandle<()>)> {
    let (tx, rx) = bounded(COMMAND_CAPACITY);
    source.start()?;
    let status = Arc::new(ArcSwap::from_pointee(status_snapshot(
        source.as_ref(),
        true,
        None,
    )));
    let handle = IngestHandle {
        commands: tx,
        drain: rx.clone(),
        status: Arc::clone(&status),
    };

    let thread = std::thread::Builder::new()
        .name("ez-daemon-ingest".to_string())
        .spawn(move || run(source.as_mut(), &bus, &rx, &running, &status))?;

    Ok((handle, thread))
}

fn run(
    source: &mut dyn IqSource,
    bus: &SampleBus,
    commands: &Receiver<HardwareCommand>,
    running: &AtomicBool,
    status: &ArcSwap<HardwareStatus>,
) {
    let mut buf = vec![Complex32::new(0.0, 0.0); READ_CHUNK];
    let mut sample_counter: u64 = 0;

    while running.load(Ordering::Relaxed) {
        let mut applied_command = false;
        while let Ok(cmd) = commands.try_recv() {
            applied_command = true;
            let result = match cmd {
                HardwareCommand::Frequency(hz) => source.set_frequency(hz),
                HardwareCommand::SampleRate(hz) => source.set_sample_rate(hz),
                HardwareCommand::Gain(db) => source.set_gain(db),
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "hardware command failed");
                status.store(Arc::new(status_snapshot(source, true, Some(e.to_string()))));
            }
        }
        if applied_command {
            status.store(Arc::new(status_snapshot(source, true, None)));
        }

        match source.read_iq(&mut buf) {
            Ok(0) => {
                // Finite source exhausted (e.g. non-looping replay reaching EOF): stop
                // cleanly rather than spin on an EOF that will never produce more samples.
                tracing::info!("ingestion source reached end of stream");
                status.store(Arc::new(status_snapshot(source, false, None)));
                break;
            }
            Ok(n) => {
                let block = SampleBlock {
                    start_sample: sample_counter,
                    sample_rate_hz: source.sample_rate_hz(),
                    center_freq_hz: source.frequency_hz(),
                    samples: Arc::from(&buf[..n]),
                };
                sample_counter += n as u64;
                bus.publish(block);
            }
            Err(e) => {
                tracing::error!(error = %e, "ingestion source read failed");
                status.store(Arc::new(status_snapshot(
                    source,
                    false,
                    Some(e.to_string()),
                )));
                break;
            }
        }
    }
    source.stop();
}

fn status_snapshot(
    source: &dyn IqSource,
    connected: bool,
    error: Option<String>,
) -> HardwareStatus {
    HardwareStatus {
        connected,
        source_kind: source.kind().to_string(),
        frequency_hz: source.frequency_hz(),
        sample_rate_hz: source.sample_rate_hz(),
        gain_db: source.gain_db(),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::OverflowPolicy;
    use crate::hardware::synthetic::SyntheticSource;
    use std::time::Duration;

    #[test]
    fn spawn_publishes_blocks_and_reports_connected_status() {
        let bus = SampleBus::new();
        let sub = bus.subscribe(8, OverflowPolicy::DropOldest);
        let running = Arc::new(AtomicBool::new(true));

        let (handle, thread) = spawn(
            Box::new(SyntheticSource::default()),
            bus,
            Arc::clone(&running),
        )
        .unwrap();

        let block = sub
            .recv_timeout(Duration::from_secs(2))
            .expect("expected a published block");
        assert!(!block.samples.is_empty());

        let status = handle.status();
        assert!(status.connected);
        assert_eq!(status.source_kind, "synthetic");

        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn set_frequency_is_applied_and_reflected_in_status() {
        let bus = SampleBus::new();
        let _sub = bus.subscribe(8, OverflowPolicy::DropOldest);
        let running = Arc::new(AtomicBool::new(true));

        let (handle, thread) = spawn(
            Box::new(SyntheticSource::default()),
            bus,
            Arc::clone(&running),
        )
        .unwrap();

        handle.set_frequency(101_000_000);

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if handle.status().frequency_hz == 101_000_000 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "frequency change never observed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn command_flood_is_coalesced_not_unbounded() {
        // A client flooding SetFrequency faster than the blocking read_iq
        // loop drains must not grow the queue without bound: only the
        // latest command matters, so the queue stays at capacity.
        let bus = SampleBus::new();
        let running = Arc::new(AtomicBool::new(true));
        let (handle, thread) = spawn(
            Box::new(SyntheticSource::default()),
            bus,
            Arc::clone(&running),
        )
        .unwrap();
        for hz in 0..10_000u64 {
            handle.set_frequency(100_000_000 + hz);
        }
        assert!(
            handle.commands.len() <= COMMAND_CAPACITY,
            "queue must stay bounded, len = {}",
            handle.commands.len()
        );
        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn stops_promptly_when_running_flag_clears() {
        let bus = SampleBus::new();
        let _sub = bus.subscribe(8, OverflowPolicy::DropOldest);
        let running = Arc::new(AtomicBool::new(true));

        let (_handle, thread) = spawn(
            Box::new(SyntheticSource::default()),
            bus,
            Arc::clone(&running),
        )
        .unwrap();
        running.store(false, Ordering::Relaxed);
        thread
            .join()
            .expect("ingestion thread should join promptly");
    }
}
