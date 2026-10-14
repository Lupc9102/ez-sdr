//! Dedicated hardware-ingestion thread: owns a `Box<dyn IqSource>` exclusively and pulls IQ
//! into the wideband [`SampleBus`]. Frequency/sample-rate/gain changes arrive over a small
//! non-blocking control channel polled once per read, rather than a `Mutex` guarding the
//! source itself — the hot path (`read_iq` + `publish`) never locks, matching the daemon's
//! zero-copy/lock-free ingestion requirement exactly as [`crate::broadcast`] does for fan-out.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender};
use num_complex::Complex32;

use ez_proto::HardwareStatus;

use crate::bus::{SampleBlock, SampleBus};
use crate::hardware::{IqSource, ReadPending};

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

/// A command has one correlated lifecycle even if its original waiter expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommandPhase {
    Queued,
    Applying,
    Applied,
    Rejected,
    Cancelled,
}

#[derive(Debug)]
struct CommandReceipt {
    phase: AtomicU8,
    outcome: Mutex<Option<Result<(), String>>>,
}
impl CommandReceipt {
    fn new() -> Self {
        Self {
            phase: AtomicU8::new(CommandPhase::Queued as u8),
            outcome: Mutex::new(None),
        }
    }
    fn phase(&self) -> CommandPhase {
        match self.phase.load(Ordering::Acquire) {
            0 => CommandPhase::Queued,
            1 => CommandPhase::Applying,
            2 => CommandPhase::Applied,
            3 => CommandPhase::Rejected,
            _ => CommandPhase::Cancelled,
        }
    }
    fn begin(&self) -> bool {
        self.phase
            .compare_exchange(
                CommandPhase::Queued as u8,
                CommandPhase::Applying as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    fn cancel_queued(&self) -> bool {
        self.phase
            .compare_exchange(
                CommandPhase::Queued as u8,
                CommandPhase::Cancelled as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    fn finish(&self, result: Result<(), String>) {
        let phase = if result.is_ok() {
            CommandPhase::Applied
        } else {
            CommandPhase::Rejected
        };
        *self
            .outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
        self.phase.store(phase as u8, Ordering::Release);
    }
    fn outcome(&self) -> Option<Result<(), String>> {
        match self.phase() {
            CommandPhase::Applied | CommandPhase::Rejected => self
                .outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            CommandPhase::Cancelled => {
                Some(Err("hardware command cancelled before application".into()))
            }
            _ => None,
        }
    }
}

/// The device call already started when its acknowledgement deadline expired.
/// It cannot safely be undone or labelled cancelled. This receipt exposes its
/// eventual applied/rejected result, while ingestion retains hardware ownership.
#[derive(Debug, Clone, thiserror::Error)]
#[error("hardware command is still applying; outcome pending, check receiver status")]
pub struct PendingHardwareCommand {
    receipt: Arc<CommandReceipt>,
}
impl PendingHardwareCommand {
    #[must_use]
    pub fn phase(&self) -> CommandPhase {
        self.receipt.phase()
    }
    #[must_use]
    pub fn outcome(&self) -> Option<Result<(), String>> {
        self.receipt.outcome()
    }
}

struct QueuedCommand {
    command: HardwareCommand,
    deadline: Instant,
    receipt: Arc<CommandReceipt>,
}
impl Drop for QueuedCommand {
    fn drop(&mut self) {
        if !self.receipt.cancel_queued() && self.receipt.phase() == CommandPhase::Applying {
            self.receipt.finish(Err(
                "ingestion ended while applying: hardware outcome unknown".into(),
            ));
        }
    }
}

const COMMAND_CAPACITY: usize = 64;
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

struct StartedSource(Box<dyn IqSource>);

impl Drop for StartedSource {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Cheap-to-clone handle for sending rare, acknowledged control commands to the ingestion
/// thread and reading its latest published [`HardwareStatus`] snapshot, without ever touching
/// the source itself (only the ingestion thread ever does that).
#[derive(Clone)]
pub struct IngestHandle {
    commands: Sender<QueuedCommand>,
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
        self.request_with_timeout(make_command, COMMAND_TIMEOUT)
    }

    fn request_with_timeout(
        &self,
        make_command: impl FnOnce(Sender<anyhow::Result<()>>) -> HardwareCommand,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        let receipt = Arc::new(CommandReceipt::new());
        let (reply_tx, reply_rx) = bounded(1);
        self.commands
            .send_timeout(
                QueuedCommand {
                    command: make_command(reply_tx),
                    deadline,
                    receipt: Arc::clone(&receipt),
                },
                timeout,
            )
            .map_err(|error| anyhow::anyhow!("hardware command queue unavailable: {error}"))?;
        match reply_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => result,
            Err(_) => {
                // This CAS orders timeout against the worker's begin CAS: a
                // cancelled queued command can never enter the device setter.
                receipt.cancel_queued();
                if let Some(result) = receipt.outcome() {
                    result.map_err(anyhow::Error::msg)
                } else {
                    Err(PendingHardwareCommand { receipt }.into())
                }
            }
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
/// ingestion thread owns a stop guard so the source is stopped on every thread exit.
pub fn spawn(
    mut source: Box<dyn IqSource>,
    bus: SampleBus,
    running: Arc<AtomicBool>,
) -> anyhow::Result<(IngestHandle, JoinHandle<()>)> {
    let (commands, command_rx) = bounded(COMMAND_CAPACITY);
    if let Err(error) = source.start() {
        source.stop();
        return Err(error);
    }
    let status = Arc::new(ArcSwap::from_pointee(status_snapshot(
        source.as_ref(),
        true,
        None,
    )));
    let handle = IngestHandle {
        commands,
        status: Arc::clone(&status),
    };

    let source_slot = Arc::new(Mutex::new(Some(source)));
    let worker_source = Arc::clone(&source_slot);
    let thread = std::thread::Builder::new()
        .name("ez-daemon-ingest".to_string())
        .spawn(move || {
            let source = worker_source
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .expect("ingestion source owner");
            let mut source = StartedSource(source);
            run(source.0.as_mut(), &bus, &command_rx, &running, &status);
        });

    let thread = match thread {
        Ok(thread) => thread,
        Err(error) => {
            if let Some(mut source) = source_slot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                source.stop();
            }
            return Err(error.into());
        }
    };

    Ok((handle, thread))
}

fn run(
    source: &mut dyn IqSource,
    bus: &SampleBus,
    commands: &Receiver<QueuedCommand>,
    running: &AtomicBool,
    status: &ArcSwap<HardwareStatus>,
) {
    let mut buf = vec![Complex32::new(0.0, 0.0); READ_CHUNK];
    let mut sample_counter: u64 = 0;
    let mut capture_epoch = ez_proto::new_capture_epoch();
    let mut product_session = ez_proto::new_capture_epoch();
    let mut pending_discontinuity = true;
    let mut last_source_epoch = None;

    'acquisition: while running.load(Ordering::Relaxed) {
        while let Ok(command) = commands.try_recv() {
            if !running.load(Ordering::Relaxed) || Instant::now() >= command.deadline {
                command.receipt.cancel_queued();
            }
            if !command.receipt.begin() {
                continue;
            }
            let before = (
                source.frequency_hz(),
                source.sample_rate_hz(),
                source.gain_db(),
                source.gain_mode(),
            );
            let requested_gain = source.requested_gain_db().unwrap_or(before.2);
            let (result, reply) = match &command.command {
                HardwareCommand::Frequency { hz, reply } => (source.set_frequency(*hz), reply),
                HardwareCommand::SampleRate { hz, reply } => (source.set_sample_rate(*hz), reply),
                HardwareCommand::Gain { db, reply } => (source.set_gain(*db), reply),
            };
            match result {
                Ok(()) => {
                    if before
                        != (
                            source.frequency_hz(),
                            source.sample_rate_hz(),
                            source.gain_db(),
                            source.gain_mode(),
                        )
                    {
                        capture_epoch = ez_proto::new_capture_epoch();
                        sample_counter = 0;
                        pending_discontinuity = true;
                    }
                    if before.0 != source.frequency_hz() || before.1 != source.sample_rate_hz() {
                        product_session = ez_proto::new_capture_epoch();
                    }
                    // Setters can restart a native pump. Its next epoch is
                    // already accounted for above; gain changes keep the product.
                    last_source_epoch = None;
                    status.store(Arc::new(status_snapshot(source, true, None)));
                    command.receipt.finish(Ok(()));
                    let _ = reply.send(Ok(()));
                }
                Err(error) => {
                    let message = error.to_string();
                    tracing::warn!(error = %message, "hardware command failed");
                    // A fallible multi-stage hardware call may have changed the
                    // device even when its getter was left unchanged. Reopen at
                    // the last verified settings before publishing any more IQ.
                    let mut verified = (**status.load()).clone();
                    source.stop();
                    let recovery = source
                        .set_frequency(before.0)
                        .and_then(|()| source.set_sample_rate(before.1))
                        .and_then(|()| source.set_gain(requested_gain))
                        .and_then(|()| source.start());
                    match recovery {
                        Ok(()) => {
                            capture_epoch = ez_proto::new_capture_epoch();
                            sample_counter = 0;
                            last_source_epoch = None;
                            pending_discontinuity = true;
                            status.store(Arc::new(status_snapshot(
                                source,
                                true,
                                Some(message.clone()),
                            )));
                            command.receipt.finish(Err(message.clone()));
                            let _ = reply.send(Err(anyhow::anyhow!(message)));
                        }
                        Err(recovery_error) => {
                            let message = format!("{message}; recovery failed: {recovery_error}");
                            verified.connected = false;
                            verified.error = Some(message.clone());
                            status.store(Arc::new(verified));
                            command.receipt.finish(Err(message.clone()));
                            let _ = reply.send(Err(anyhow::anyhow!(message)));
                            break 'acquisition;
                        }
                    }
                }
            }
        }

        if !running.load(Ordering::Relaxed) {
            break;
        }
        let read = source.read_iq(&mut buf);
        if !running.load(Ordering::Relaxed) {
            break;
        }
        match read {
            Ok(0) => {
                // Finite source exhausted (e.g. non-looping replay reaching EOF): stop
                // cleanly rather than spin on an EOF that will never produce more samples.
                tracing::info!("ingestion source reached end of stream");
                status.store(Arc::new(status_snapshot(source, false, None)));
                break;
            }
            Ok(n) => {
                let mut clock = if let Some(clock) = source.capture_clock() {
                    if last_source_epoch != Some(clock.epoch) {
                        if last_source_epoch.is_some() {
                            product_session = ez_proto::new_capture_epoch();
                        }
                        capture_epoch = ez_proto::new_capture_epoch();
                        pending_discontinuity = true;
                        last_source_epoch = Some(clock.epoch);
                    }
                    sample_counter = clock.source_sample_index;
                    clock
                } else {
                    ez_proto::CaptureClock::new(
                        capture_epoch,
                        sample_counter,
                        source.sample_rate_hz(),
                        // A read-call wall clock is not a hardware acquisition
                        // timestamp. Sources with a real origin supply it above.
                        None,
                    )
                };
                clock.epoch = capture_epoch;
                clock.discontinuous |= pending_discontinuity;
                pending_discontinuity = false;
                let block = SampleBlock {
                    product_session,
                    observed_at: Some(std::time::Instant::now()),
                    capture: Some(clock),
                    start_sample: sample_counter,
                    sample_rate_hz: source.sample_rate_hz(),
                    center_freq_hz: source.frequency_hz(),
                    samples: Arc::from(&buf[..n]),
                };
                sample_counter += n as u64;
                bus.publish(block);
            }
            Err(e) if e.downcast_ref::<ReadPending>().is_some() => {
                if matches!(e.downcast_ref::<ReadPending>(), Some(ReadPending::Gap)) {
                    // The driver cannot count lost samples. Start a new domain
                    // rather than inventing a contiguous acquisition clock.
                    capture_epoch = ez_proto::new_capture_epoch();
                    sample_counter = 0;
                    last_source_epoch = None;
                    pending_discontinuity = true;
                    tracing::warn!("acquisition overflow: capture epoch retired");
                }
                // Some drivers report timeout immediately; avoid a busy loop.
                std::thread::sleep(std::time::Duration::from_millis(1));
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
    // Retire every queued command on EOF/error/shutdown; none may escape into
    // a replacement source. Dropping the envelope publishes Cancelled.
    while let Ok(command) = commands.try_recv() {
        drop(command);
    }
    // A deliberate shutdown is terminal too; retain an existing read failure.
    let mut terminal = (**status.load()).clone();
    terminal.connected = false;
    status.store(Arc::new(terminal));
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
        requested_gain_db: source.requested_gain_db(),
        gain_mode: source.gain_mode(),
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
    fn each_replay_loop_has_a_new_product_session() {
        use crate::hardware::replay::{FileReplaySource, ReplayFormat};
        let path = std::env::temp_dir().join(format!(
            "ez-s11-replay-{}.iq",
            ez_proto::new_capture_epoch()
        ));
        std::fs::write(&path, [127u8; 512]).unwrap();
        let source = FileReplaySource::open(
            path.to_str().unwrap(),
            ReplayFormat::RawU8,
            288_000,
            true,
            1.0,
        )
        .unwrap();
        let bus = SampleBus::new();
        let sub = bus.subscribe(8, OverflowPolicy::DropIncoming);
        let running = Arc::new(AtomicBool::new(true));
        let (_, thread) = spawn(Box::new(source), bus, Arc::clone(&running)).unwrap();
        let first = sub.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = sub.recv_timeout(Duration::from_secs(2)).unwrap();
        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
        assert_ne!(first.product_session, second.product_session);
        assert_eq!(first.start_sample, 0);
        assert_eq!(second.start_sample, 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn set_frequency_is_applied_and_reflected_in_status() {
        let bus = SampleBus::new();
        let sub = bus.subscribe(8, OverflowPolicy::DropOldest);
        let running = Arc::new(AtomicBool::new(true));

        let (handle, thread) = spawn(
            Box::new(SyntheticSource::default()),
            bus,
            Arc::clone(&running),
        )
        .unwrap();

        let before = sub.recv_timeout(Duration::from_secs(2)).unwrap();
        let before_clock = before.capture.unwrap();
        assert_eq!(before_clock.source_sample_index, before.start_sample);
        assert_eq!(before_clock.origin_unix_ns, None);

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

        loop {
            let after = sub.recv_timeout(Duration::from_secs(2)).unwrap();
            if after.center_freq_hz == 101_000_000 {
                let after_clock = after.capture.unwrap();
                assert_ne!(after.product_session, before.product_session);
                assert_ne!(after_clock.epoch, before_clock.epoch);
                assert_eq!(after_clock.source_sample_index, after.start_sample);
                if after.start_sample == 0 {
                    assert!(after_clock.discontinuous);
                }
                break;
            }
            assert!(std::time::Instant::now() < deadline);
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
    fn unrecoverable_control_errors_retire_source_and_preserve_verified_status() {
        for control in 0..3 {
            let bus = SampleBus::new();
            let running = Arc::new(AtomicBool::new(true));
            let (handle, thread) = spawn(Box::new(RejectingSource::new()), bus, running).unwrap();
            let initial = handle.status();
            let error = match control {
                0 => handle.set_frequency(initial.frequency_hz + 1),
                1 => handle.set_sample_rate(initial.sample_rate_hz + 1),
                _ => handle.set_gain(initial.gain_db + 1.0),
            }
            .unwrap_err();
            assert!(error.to_string().contains("rejected"));
            thread.join().unwrap();
            let status = handle.status();
            assert!(!status.connected);
            assert_eq!(status.frequency_hz, initial.frequency_hz);
            assert_eq!(status.sample_rate_hz, initial.sample_rate_hz);
            assert_eq!(status.gain_db, initial.gain_db);
            assert!(status.error.unwrap().contains("recovery failed"));
        }
    }

    struct MutatingFailureSource {
        inner: SyntheticSource,
        live: bool,
        reject_recovery: bool,
        reads: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl IqSource for MutatingFailureSource {
        fn start(&mut self) -> anyhow::Result<()> {
            self.live = true;
            self.inner.start()
        }
        fn stop(&mut self) {
            self.live = false;
            self.inner.stop();
        }
        fn set_frequency(&mut self, hz: u64) -> anyhow::Result<()> {
            self.inner.set_frequency(hz)?;
            if self.live || self.reject_recovery {
                anyhow::bail!("mutated before driver rejection");
            }
            Ok(())
        }
        fn set_sample_rate(&mut self, hz: u32) -> anyhow::Result<()> {
            self.inner.set_sample_rate(hz)
        }
        fn set_gain(&mut self, db: f64) -> anyhow::Result<()> {
            self.inner.set_gain(db)
        }
        fn read_iq(&mut self, buf: &mut [Complex32]) -> anyhow::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
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
            "mutating-failure"
        }
    }

    #[test]
    fn mutation_before_error_reopens_at_verified_geometry_or_stops_without_reads() {
        for reject_recovery in [false, true] {
            let bus = SampleBus::new();
            let sub = bus.subscribe(1024, OverflowPolicy::DropOldest);
            let running = Arc::new(AtomicBool::new(true));
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (handle, thread) = spawn(
                Box::new(MutatingFailureSource {
                    inner: SyntheticSource::default(),
                    live: false,
                    reject_recovery,
                    reads: Arc::clone(&reads),
                }),
                bus,
                Arc::clone(&running),
            )
            .unwrap();
            let before = sub.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(handle.set_frequency(before.center_freq_hz + 12345).is_err());
            let after_error_reads = reads.load(Ordering::Relaxed);
            if reject_recovery {
                thread.join().unwrap();
                assert!(!handle.status().connected);
                assert_eq!(reads.load(Ordering::Relaxed), after_error_reads);
            } else {
                loop {
                    let after = sub.recv_timeout(Duration::from_secs(2)).unwrap();
                    assert_eq!(after.center_freq_hz, before.center_freq_hz);
                    if after.capture_clock().epoch != before.capture_clock().epoch {
                        assert!(after.capture_clock().discontinuous);
                        assert_eq!(after.start_sample, 0);
                        break;
                    }
                }
                assert!(handle.status().connected);
                running.store(false, Ordering::Relaxed);
                thread.join().unwrap();
            }
            assert_eq!(handle.status().frequency_hz, before.center_freq_hz);
            while let Some(block) = sub.try_recv() {
                assert_eq!(block.center_freq_hz, before.center_freq_hz);
            }
        }
    }

    struct SoapyCodeSource {
        codes: std::collections::VecDeque<i32>,
        entered_pending: Arc<AtomicBool>,
        hold_pending: bool,
        bytes: Vec<u8>,
    }
    impl IqSource for SoapyCodeSource {
        fn start(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn stop(&mut self) {}
        fn set_frequency(&mut self, _: u64) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_sample_rate(&mut self, _: u32) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_gain(&mut self, _: f64) -> anyhow::Result<()> {
            Ok(())
        }
        fn read_iq(&mut self, buf: &mut [Complex32]) -> anyhow::Result<usize> {
            let code = self
                .codes
                .pop_front()
                .unwrap_or(if self.hold_pending { -1 } else { -2 });
            if code == -1 {
                self.entered_pending.store(true, Ordering::Release);
                std::thread::sleep(Duration::from_millis(5));
            }
            crate::hardware::soapy_read::read_once(buf, &mut self.bytes, |_, _| code)
        }
        fn frequency_hz(&self) -> u64 {
            100_000_000
        }
        fn sample_rate_hz(&self) -> u32 {
            2_000_000
        }
        fn gain_db(&self) -> f64 {
            0.0
        }
        fn kind(&self) -> &'static str {
            "injected-soapy-codes"
        }
    }

    #[test]
    fn soapy_timeout_retries_overflow_retires_clock_and_fatal_error_stops() {
        let bus = SampleBus::new();
        let sub = bus.subscribe(16, OverflowPolicy::DropOldest);
        let running = Arc::new(AtomicBool::new(true));
        let (handle, thread) = spawn(
            Box::new(SoapyCodeSource {
                codes: [3, -1, 0, 3, -4, -1, 3, -2].into(),
                entered_pending: Arc::new(AtomicBool::new(false)),
                hold_pending: false,
                bytes: vec![],
            }),
            bus,
            running,
        )
        .unwrap();
        thread.join().unwrap();
        let first = sub.try_recv().unwrap();
        let after_timeout = sub.try_recv().unwrap();
        let after_overflow = sub.try_recv().unwrap();
        assert_eq!(
            first.capture_clock().epoch,
            after_timeout.capture_clock().epoch
        );
        assert_eq!(first.product_session, after_timeout.product_session);
        assert_eq!(first.product_session, after_overflow.product_session);
        assert_eq!(after_timeout.start_sample, 3);
        assert!(!after_timeout.capture_clock().discontinuous);
        assert_ne!(
            first.capture_clock().epoch,
            after_overflow.capture_clock().epoch
        );
        assert_eq!(after_overflow.start_sample, 0);
        assert!(after_overflow.capture_clock().discontinuous);
        assert!(!handle.status().connected);
        assert!(handle.status().error.unwrap().contains("code -2"));
        assert!(sub.try_recv().is_none());
    }

    #[test]
    fn endless_timeouts_keep_controls_available_and_shutdown_bounded() {
        let bus = SampleBus::new();
        let running = Arc::new(AtomicBool::new(true));
        let entered = Arc::new(AtomicBool::new(false));
        let (handle, thread) = spawn(
            Box::new(SoapyCodeSource {
                codes: [].into(),
                entered_pending: Arc::clone(&entered),
                hold_pending: true,
                bytes: vec![],
            }),
            bus,
            Arc::clone(&running),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !entered.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        handle.set_gain(5.0).unwrap();
        assert!(handle.status().connected);
        let start = std::time::Instant::now();
        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(!handle.status().connected);
    }

    struct GatedSource {
        frequency: u64,
        first_read: bool,
        read_started: Sender<()>,
        read_release: Receiver<()>,
        tune_started: Sender<()>,
        tune_release: Option<Receiver<()>>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        stopped: Arc<AtomicBool>,
    }
    impl IqSource for GatedSource {
        fn start(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn stop(&mut self) {
            self.stopped.store(true, Ordering::Release);
        }
        fn set_frequency(&mut self, hz: u64) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let _ = self.tune_started.try_send(());
            if let Some(release) = self.tune_release.take() {
                release.recv().unwrap();
            }
            self.frequency = hz;
            Ok(())
        }
        fn set_sample_rate(&mut self, _: u32) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_gain(&mut self, _: f64) -> anyhow::Result<()> {
            Ok(())
        }
        fn read_iq(&mut self, _: &mut [Complex32]) -> anyhow::Result<usize> {
            if self.first_read {
                self.first_read = false;
                self.read_started.send(()).unwrap();
                self.read_release.recv().unwrap();
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(ReadPending::Timeout.into())
        }
        fn frequency_hz(&self) -> u64 {
            self.frequency
        }
        fn sample_rate_hz(&self) -> u32 {
            2_400_000
        }
        fn gain_db(&self) -> f64 {
            0.0
        }
        fn kind(&self) -> &'static str {
            "gated-lifecycle-test"
        }
    }

    #[test]
    fn queued_deadline_cancellation_prevents_late_tuning_after_a_stalled_read() {
        let (read_started_tx, read_started) = bounded(1);
        let (read_release, read_release_rx) = bounded(1);
        let (tune_started_tx, tune_started) = bounded(1);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let (handle, thread) = spawn(
            Box::new(GatedSource {
                frequency: 100_000_000,
                first_read: true,
                read_started: read_started_tx,
                read_release: read_release_rx,
                tune_started: tune_started_tx,
                tune_release: None,
                calls: Arc::clone(&calls),
                stopped: Arc::new(AtomicBool::new(false)),
            }),
            SampleBus::new(),
            Arc::clone(&running),
        )
        .unwrap();
        read_started.recv_timeout(Duration::from_secs(2)).unwrap();
        let error = handle
            .request_with_timeout(
                |reply| HardwareCommand::Frequency {
                    hz: 101_000_000,
                    reply,
                },
                Duration::from_millis(100),
            )
            .unwrap_err();
        assert!(error.to_string().contains("cancelled before application"));
        assert!(error.downcast_ref::<PendingHardwareCommand>().is_none());
        read_release.send(()).unwrap();
        assert!(tune_started
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(handle.status().frequency_hz, 100_000_000);
        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn applying_deadline_returns_correlated_pending_outcome_and_retains_ownership() {
        let (read_started_tx, read_started) = bounded(1);
        let (read_release, read_release_rx) = bounded(1);
        let (tune_started_tx, tune_started) = bounded(1);
        let (tune_release, tune_release_rx) = bounded(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let (handle, thread) = spawn(
            Box::new(GatedSource {
                frequency: 100_000_000,
                first_read: true,
                read_started: read_started_tx,
                read_release: read_release_rx,
                tune_started: tune_started_tx,
                tune_release: Some(tune_release_rx),
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                stopped: Arc::clone(&stopped),
            }),
            SampleBus::new(),
            Arc::clone(&running),
        )
        .unwrap();
        read_started.recv_timeout(Duration::from_secs(2)).unwrap();
        let client = handle.clone();
        let request = std::thread::spawn(move || {
            client.request_with_timeout(
                |reply| HardwareCommand::Frequency {
                    hz: 101_000_000,
                    reply,
                },
                Duration::from_millis(100),
            )
        });
        read_release.send(()).unwrap();
        tune_started.recv_timeout(Duration::from_secs(2)).unwrap();
        let pending = request
            .join()
            .unwrap()
            .unwrap_err()
            .downcast::<PendingHardwareCommand>()
            .unwrap();
        assert_eq!(pending.phase(), CommandPhase::Applying);
        assert!(pending.outcome().is_none());
        assert_eq!(handle.status().frequency_hz, 100_000_000);
        running.store(false, Ordering::Relaxed);
        assert!(!stopped.load(Ordering::Acquire));
        assert!(!thread.is_finished());
        tune_release.send(()).unwrap();
        thread.join().unwrap();
        assert!(stopped.load(Ordering::Acquire));
        assert_eq!(pending.phase(), CommandPhase::Applied);
        assert_eq!(pending.outcome(), Some(Ok(())));
        assert_eq!(handle.status().frequency_hz, 101_000_000);
        assert!(!handle.status().connected);
    }

    #[test]
    fn dropping_queued_command_on_shutdown_publishes_cancelled_receipt() {
        let receipt = Arc::new(CommandReceipt::new());
        let (reply, _) = bounded(1);
        drop(QueuedCommand {
            command: HardwareCommand::Gain { db: 1.0, reply },
            deadline: Instant::now() + Duration::from_secs(1),
            receipt: Arc::clone(&receipt),
        });
        assert_eq!(receipt.phase(), CommandPhase::Cancelled);
        assert!(receipt
            .outcome()
            .unwrap()
            .unwrap_err()
            .contains("cancelled"));
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
