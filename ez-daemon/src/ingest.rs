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

enum HardwareCommand {
    Frequency {
        hz: u64,
        reply: Sender<anyhow::Result<()>>,
    },
    SampleRate {
        hz: u32,
        reply: Sender<anyhow::Result<()>>,
    },
    Gain {
        db: f64,
        reply: Sender<anyhow::Result<()>>,
    },
}

const COMMAND_CAPACITY: usize = 64;
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Cheap-to-clone handle for sending rare, acknowledged control commands to the ingestion
/// thread and reading its latest published [`HardwareStatus`] snapshot, without ever touching
/// the source itself (only the ingestion thread ever does that).
#[derive(Clone)]
pub struct IngestHandle {
    commands: Sender<HardwareCommand>,
    status: Arc<ArcSwap<HardwareStatus>>,
}

impl IngestHandle {
    /// Requests a frequency change and waits until the source accepts or rejects it.
    pub fn set_frequency(&self, hz: u64) -> anyhow::Result<()> {
        self.request(|reply| HardwareCommand::Frequency { hz, reply })
    }

    pub fn set_sample_rate(&self, hz: u32) -> anyhow::Result<()> {
        self.request(|reply| HardwareCommand::SampleRate { hz, reply })
    }

    pub fn set_gain(&self, db: f64) -> anyhow::Result<()> {
        self.request(|reply| HardwareCommand::Gain { db, reply })
    }

    fn request(
        &self,
        make_command: impl FnOnce(Sender<anyhow::Result<()>>) -> HardwareCommand,
    ) -> anyhow::Result<()> {
        let (reply_tx, reply_rx) = bounded(1);
        self.commands
            .send_timeout(make_command(reply_tx), COMMAND_TIMEOUT)
            .map_err(|error| anyhow::anyhow!("hardware command queue unavailable: {error}"))?;
        reply_rx
            .recv_timeout(COMMAND_TIMEOUT)
            .map_err(|error| anyhow::anyhow!("hardware command timed out: {error}"))?
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
    let (commands, command_rx) = bounded(COMMAND_CAPACITY);
    source.start()?;
    let status = Arc::new(ArcSwap::from_pointee(status_snapshot(
        source.as_ref(),
        true,
        None,
    )));
    let handle = IngestHandle {
        commands,
        status: Arc::clone(&status),
    };

    let thread = std::thread::Builder::new()
        .name("ez-daemon-ingest".to_string())
        .spawn(move || run(source.as_mut(), &bus, &command_rx, &running, &status))?;

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
        while let Ok(command) = commands.try_recv() {
            let (result, reply) = match command {
                HardwareCommand::Frequency { hz, reply } => (source.set_frequency(hz), reply),
                HardwareCommand::SampleRate { hz, reply } => (source.set_sample_rate(hz), reply),
                HardwareCommand::Gain { db, reply } => (source.set_gain(db), reply),
            };
            match result {
                Ok(()) => {
                    status.store(Arc::new(status_snapshot(source, true, None)));
                    let _ = reply.send(Ok(()));
                }
                Err(error) => {
                    let message = error.to_string();
                    tracing::warn!(error = %message, "hardware command failed");
                    status.store(Arc::new(status_snapshot(
                        source,
                        true,
                        Some(message.clone()),
                    )));
                    let _ = reply.send(Err(anyhow::anyhow!(message)));
                }
            }
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
    // A deliberate shutdown is terminal too; retain an existing read failure.
    let error = status.load().error.clone();
    status.store(Arc::new(status_snapshot(source, false, error)));
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

    struct RejectingSource {
        inner: SyntheticSource,
    }

    impl RejectingSource {
        fn new() -> Self {
            Self {
                inner: SyntheticSource::default(),
            }
        }
    }

    impl IqSource for RejectingSource {
        fn start(&mut self) -> anyhow::Result<()> {
            self.inner.start()
        }

        fn stop(&mut self) {
            self.inner.stop();
        }

        fn set_frequency(&mut self, _hz: u64) -> anyhow::Result<()> {
            anyhow::bail!("frequency rejected by test source")
        }

        fn set_sample_rate(&mut self, _hz: u32) -> anyhow::Result<()> {
            anyhow::bail!("sample rate rejected by test source")
        }

        fn set_gain(&mut self, _db: f64) -> anyhow::Result<()> {
            anyhow::bail!("gain rejected by test source")
        }

        fn read_iq(&mut self, buf: &mut [num_complex::Complex32]) -> anyhow::Result<usize> {
            self.inner.read_iq(buf)
        }

        fn frequency_hz(&self) -> u64 {
            self.inner.frequency_hz()
        }

        fn sample_rate_hz(&self) -> u32 {
            self.inner.sample_rate_hz()
        }

        fn gain_db(&self) -> f64 {
            self.inner.gain_db()
        }

        fn kind(&self) -> &'static str {
            "rejecting-test-source"
        }
    }

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

        handle.set_frequency(101_000_000).unwrap();

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
    fn command_requests_are_acknowledged_without_growing_unbounded() {
        let bus = SampleBus::new();
        let running = Arc::new(AtomicBool::new(true));
        let (handle, thread) = spawn(
            Box::new(SyntheticSource::default()),
            bus,
            Arc::clone(&running),
        )
        .unwrap();
        for hz in 0..100u64 {
            handle.set_frequency(100_000_000 + hz).unwrap();
        }
        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn rejected_hardware_commands_return_backend_errors_and_preserve_status() {
        let bus = SampleBus::new();
        let running = Arc::new(AtomicBool::new(true));
        let (handle, thread) =
            spawn(Box::new(RejectingSource::new()), bus, Arc::clone(&running)).unwrap();
        let initial = handle.status();

        assert!(handle
            .set_frequency(initial.frequency_hz + 1)
            .unwrap_err()
            .to_string()
            .contains("frequency rejected"));
        assert!(handle
            .set_sample_rate(initial.sample_rate_hz + 1)
            .unwrap_err()
            .to_string()
            .contains("sample rate rejected"));
        assert!(handle
            .set_gain(initial.gain_db + 1.0)
            .unwrap_err()
            .to_string()
            .contains("gain rejected"));

        let status = handle.status();
        assert_eq!(status.frequency_hz, initial.frequency_hz);
        assert_eq!(status.sample_rate_hz, initial.sample_rate_hz);
        assert_eq!(status.gain_db, initial.gain_db);
        assert!(status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("gain rejected")));

        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn stops_promptly_when_running_flag_clears() {
        let bus = SampleBus::new();
        let _sub = bus.subscribe(8, OverflowPolicy::DropOldest);
        let running = Arc::new(AtomicBool::new(true));

        let (handle, thread) = spawn(
            Box::new(SyntheticSource::default()),
            bus,
            Arc::clone(&running),
        )
        .unwrap();
        running.store(false, Ordering::Relaxed);
        thread
            .join()
            .expect("ingestion thread should join promptly");
        assert!(
            !handle.status().connected,
            "stopped hardware still reported connected"
        );
    }
}
