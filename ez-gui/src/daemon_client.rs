//! Background network client for `ez-daemon`: owns a dedicated OS thread running its own
//! single-threaded Tokio runtime, so the rest of `ez-gui` (a plain synchronous `egui` loop)
//! never needs to poll an async executor itself. Commands flow out over an unbounded
//! `tokio::sync::mpsc` channel — `UnboundedSender::send` is synchronous and non-blocking, so
//! it's callable directly from the egui thread with no `.await`. Events flow back over a
//! bounded control and coalesced stream mailboxes, retaining non-blocking polling
//! `SourceManager` already uses for its local worker threads (see `crate::source_manager`).
//!
//! Modeled directly on `ez_daemon::server`'s own connection handling (handshake sequence,
//! `tokio::select!` shutdown-polling pattern) so the two ends of the wire protocol stay in
//! lockstep by construction rather than by convention.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::{FramedRead, FramedWrite};

use ez_proto::{ClientCommand, MessageCodec, ServerEvent, PROTOCOL_VERSION};

/// How long to wait for the initial connect + Hello/Welcome handshake before giving up.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound on how long a connected-but-idle loop can delay reacting to shutdown — mirrors
/// the periodic-tick pattern `ez_daemon::server`'s own `select!` loops use for the same reason.
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Bounds how long `Drop` can be held up trying to deliver a clean `Detach` before it just
/// gives up and tears the socket down anyway — a dropped connection is treated by the daemon
/// as an implicit detach, so this is a nicety, not a requirement for correctness.
const DETACH_SEND_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
    Error(String),
}

#[derive(Default)]
struct ControlQueue {
    events: std::collections::VecDeque<ServerEvent>,
}

impl ControlQueue {
    fn push(&mut self, event: ServerEvent) -> bool {
        if let ServerEvent::Recording(next) = &event {
            if let Some(index) = self.events.iter().rposition(|previous| {
                matches!(previous, ServerEvent::Recording(previous) if previous.channel_id == next.channel_id)
            }) {
                if matches!(&self.events[index], ServerEvent::Recording(previous)
                    if previous.active == next.active && previous.path == next.path)
                {
                    // Coalesce progress within one recording phase, while
                    // preserving start/stop acknowledgements in FIFO order.
                    self.events.remove(index);
                }
            }
        }
        self.events.retain(|previous| {
            !matches!(
                (&event, previous),
                (ServerEvent::Hardware(_), ServerEvent::Hardware(_))
            )
        });
        if self.events.len() >= 64 {
            return false;
        }
        self.events.push_back(event);
        true
    }
}

/// Handle to a background-thread connection to a running `ez-daemon`. Single-owner by design
/// (not `Clone`) — `SourceManager` holds exactly one at a time. Dropping it signals the worker
/// to stop, best-effort sends `Detach`, and joins the thread, so no orphaned connection or
/// thread outlives the handle.
pub struct DaemonClient {
    status: Arc<Mutex<ConnectionStatus>>,
    client_channels: Arc<Mutex<Option<ez_proto::ClientChannels>>>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<ClientCommand>,
    control_event_rx: crossbeam_channel::Receiver<ServerEvent>,
    stream_event_rx: crossbeam_channel::Receiver<ServerEvent>,
    spectrum_event_rx: crossbeam_channel::Receiver<(ServerEvent, Instant)>,
    last_spectrum_received_at: Mutex<Option<Instant>>,
    control_queue: Arc<Mutex<ControlQueue>>,
    stream_queue: Arc<Mutex<ez_proto::stream_queue::StreamQueue>>,
    running: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl DaemonClient {
    /// Spawns the background thread and starts connecting immediately; never blocks the
    /// caller. Poll [`Self::status`] to observe handshake progress or failure.
    #[must_use]
    pub fn connect(addr: SocketAddr, client_name: String) -> Self {
        let status = Arc::new(Mutex::new(ConnectionStatus::Connecting));
        let client_channels = Arc::new(Mutex::new(None));
        let worker_channels = Arc::clone(&client_channels);
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let control_queue = Arc::new(Mutex::new(ControlQueue::default()));
        let stream_queue = Arc::new(Mutex::new(ez_proto::stream_queue::StreamQueue::default()));
        let worker_controls = Arc::clone(&control_queue);
        let worker_streams = Arc::clone(&stream_queue);
        let running = Arc::new(AtomicBool::new(true));

        let worker_status = Arc::clone(&status);
        let worker_running = Arc::clone(&running);
        let worker = std::thread::Builder::new()
            .name("ez-gui-daemon-client".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        *worker_status
                            .lock()
                            .expect("daemon client worker status mutex poisoned") =
                            ConnectionStatus::Error(format!("runtime init failed: {e}"));
                        return;
                    }
                };
                runtime.block_on(run_connection(
                    addr,
                    client_name,
                    worker_status,
                    worker_channels,
                    cmd_rx,
                    worker_controls,
                    worker_streams,
                    worker_running,
                ));
            })
            .expect("failed to spawn ez-gui-daemon-client thread");

        Self {
            status,
            client_channels,
            cmd_tx,
            control_event_rx: crossbeam_channel::never(),
            stream_event_rx: crossbeam_channel::never(),
            spectrum_event_rx: crossbeam_channel::never(),
            control_queue,
            stream_queue,
            last_spectrum_received_at: Mutex::new(None),
            running,
            worker: Some(worker),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_connection() -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<ClientCommand>,
        crossbeam_channel::Sender<ServerEvent>,
    ) {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        (
            Self {
                status: Arc::new(Mutex::new(ConnectionStatus::Connected)),
                client_channels: Arc::new(Mutex::new(None)),
                cmd_tx,
                control_event_rx: event_rx,
                control_queue: Arc::new(Mutex::new(ControlQueue::default())),
                stream_queue: Arc::new(Mutex::new(ez_proto::stream_queue::StreamQueue::default())),
                stream_event_rx: crossbeam_channel::never(),
                spectrum_event_rx: crossbeam_channel::never(),
                last_spectrum_received_at: Mutex::new(None),
                running: Arc::new(AtomicBool::new(false)),
                worker: None,
            },
            cmd_rx,
            event_tx,
        )
    }

    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        self.status
            .lock()
            .expect("daemon client status mutex poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn test_set_client_channels(&self, channels: ez_proto::ClientChannels) {
        *self.client_channels.lock().unwrap() = Some(channels);
    }

    pub fn client_channels(&self) -> Option<ez_proto::ClientChannels> {
        *self
            .client_channels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Queues a command for delivery. Fire-and-forget, matching `ez_daemon::ingest::IngestHandle`'s
    /// style: the only failure mode is the worker thread having already exited, which
    /// `status()` already reports, so a dropped send here would be redundant to also surface.
    pub fn send(&self, cmd: ClientCommand) {
        let _ = self.try_send(cmd);
    }

    /// Reports a closed command channel to controls that require acknowledgement.
    pub fn try_send(&self, cmd: ClientCommand) -> bool {
        self.cmd_tx.send(cmd).is_ok()
    }

    /// Non-blocking pull with control priority and fair, coalesced live streams. Returns `None` if nothing is
    /// queued right now — callers poll this from their own render/update loop.
    #[must_use]
    pub fn try_recv_event(&self) -> Option<ServerEvent> {
        self.control_queue
            .lock()
            .unwrap()
            .events
            .pop_front()
            .or_else(|| self.control_event_rx.try_recv().ok())
            .or_else(|| {
                let (event, observed_at) = self.stream_queue.lock().unwrap().pop()?;
                if matches!(event, ServerEvent::Spectrum(_)) {
                    *self.last_spectrum_received_at.lock().unwrap() = Some(observed_at);
                }
                Some(event)
            })
            .or_else(|| self.stream_event_rx.try_recv().ok())
            .or_else(|| {
                let (event, received_at) = self.spectrum_event_rx.try_recv().ok()?;
                *self.last_spectrum_received_at.lock().unwrap() = Some(received_at);
                Some(event)
            })
    }

    pub(crate) fn spectrum_received_at(&self) -> Option<Instant> {
        *self.last_spectrum_received_at.lock().unwrap()
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.cmd_tx.send(ClientCommand::Detach);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

type Halves = (
    FramedRead<OwnedReadHalf, MessageCodec<ServerEvent>>,
    FramedWrite<OwnedWriteHalf, MessageCodec<ClientCommand>>,
);

/// A pending connect, handshake, or socket write must not keep Drop's join alive.
async fn until_stopped<T>(
    running: &AtomicBool,
    operation: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        result = operation => Some(result),
        _ = async {
            while running.load(Ordering::Relaxed) {
                tokio::time::sleep(SHUTDOWN_POLL_INTERVAL).await;
            }
        } => None,
    }
}

async fn connect_and_handshake(
    addr: SocketAddr,
    client_name: &str,
) -> Result<(Halves, ez_proto::ClientChannels), String> {
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("connect to {addr} failed: {e}"))?;
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = FramedRead::new(read_half, MessageCodec::<ServerEvent>::for_data());
    let mut writer = FramedWrite::new(write_half, MessageCodec::<ClientCommand>::for_commands());

    writer
        .send(ClientCommand::Hello {
            client_name: client_name.to_string(),
            protocol_version: PROTOCOL_VERSION,
        })
        .await
        .map_err(|e| format!("failed to send Hello: {e}"))?;

    let client_channels = match reader.next().await {
        Some(Ok(ServerEvent::Welcome {
            protocol_version,
            client_channels,
            ..
        })) => {
            if protocol_version != PROTOCOL_VERSION {
                return Err(format!(
                    "protocol version mismatch: daemon speaks {protocol_version}, client speaks {PROTOCOL_VERSION}"
                ));
            }
            client_channels
        }
        Some(Ok(ServerEvent::Error { message })) => {
            return Err(format!("daemon rejected handshake: {message}"))
        }
        Some(Ok(other)) => return Err(format!("expected Welcome, got {other:?}")),
        Some(Err(e)) => return Err(format!("handshake decode error: {e}")),
        None => return Err("connection closed during handshake".to_string()),
    };

    Ok(((reader, writer), client_channels))
}

/// Drives one connection end to end: handshake, then a `tokio::select!` loop routing queued
/// outgoing commands to the socket and incoming frames to `event_tx`, until `running` clears,
/// the peer closes the connection, or a decode error occurs. Always leaves `status` in a
/// terminal state (`Disconnected` or `Error`) before returning, so `DaemonClient::status`
/// never gets stuck reporting a connection that's actually gone.
#[allow(clippy::too_many_arguments)]
async fn run_connection(
    addr: SocketAddr,
    client_name: String,
    status: Arc<Mutex<ConnectionStatus>>,
    client_channels: Arc<Mutex<Option<ez_proto::ClientChannels>>>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<ClientCommand>,
    control_queue: Arc<Mutex<ControlQueue>>,
    stream_queue: Arc<Mutex<ez_proto::stream_queue::StreamQueue>>,
    running: Arc<AtomicBool>,
) {
    let Some(handshake) = until_stopped(
        &running,
        tokio::time::timeout(HANDSHAKE_TIMEOUT, connect_and_handshake(addr, &client_name)),
    )
    .await
    else {
        *status.lock().expect("daemon client status mutex poisoned") =
            ConnectionStatus::Disconnected;
        return;
    };
    let ((mut reader, mut writer), allocated) = match handshake {
        Ok(Ok(halves)) => halves,
        Ok(Err(e)) => {
            *status.lock().expect("daemon client status mutex poisoned") =
                ConnectionStatus::Error(e);
            return;
        }
        Err(_) => {
            *status.lock().expect("daemon client status mutex poisoned") =
                ConnectionStatus::Error(format!("connect to {addr} timed out"));
            return;
        }
    };
    *client_channels.lock().unwrap_or_else(|p| p.into_inner()) = Some(allocated);
    *status.lock().expect("daemon client status mutex poisoned") = ConnectionStatus::Connected;

    let mut ticker = tokio::time::interval(SHUTDOWN_POLL_INTERVAL);
    let mut interrupted_recording_stop = None;
    while running.load(Ordering::Relaxed) {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ClientCommand::Detach) => {
                        let _ = tokio::time::timeout(DETACH_SEND_TIMEOUT, writer.send(ClientCommand::Detach)).await;
                        break;
                    }
                    Some(cmd) => {
                        if matches!(cmd, ClientCommand::StopRecording { .. }) {
                            interrupted_recording_stop = Some(cmd.clone());
                        }
                        match until_stopped(&running, tokio::time::timeout(HANDSHAKE_TIMEOUT, writer.send(cmd))).await {
                            Some(Ok(Ok(()))) => { interrupted_recording_stop = None; }
                            None => break,
                            Some(result) => {
                                let message = match result {
                                    Ok(Err(error)) => format!("command send failed: {error}"),
                                    Err(_) => "command send timed out".to_string(),
                                    Ok(Ok(())) => unreachable!(),
                                };
                                *status.lock().expect("daemon client status mutex poisoned") = ConnectionStatus::Error(message);
                                return;
                            }
                        }
                    }
                    None => break,
                }
            }
            frame = reader.next() => {
                match frame {
                    Some(Ok(event)) => {
                        if let ServerEvent::ChannelRemoved { channel_id } = &event {
                            stream_queue.lock().unwrap().clear(*channel_id);
                        }
                        if ez_proto::stream_queue::StreamQueue::is_stream(&event) {
                            if !stream_queue.lock().unwrap().push(event, Instant::now()) {
                                *status.lock().unwrap() = ConnectionStatus::Error("daemon stream channel queue limit exceeded; reconnect".into());
                                return;
                            }
                        } else if !control_queue.lock().unwrap().push(event) {
                            *status.lock().unwrap() = ConnectionStatus::Error("daemon control reply queue is full; reconnect after processing replies".into());
                            return;
                        }
                    }
                    Some(Err(e)) => {
                        *status.lock()
                .expect("daemon client status mutex poisoned") = ConnectionStatus::Error(format!("decode error: {e}"));
                        return;
                    }
                    None => break,
                }
            }
            _ = ticker.tick() => {}
        }
    }
    // SourceManager queues recording stop before dropping this client. Remote
    // recordings survive disconnect, so give those stops a bounded chance to
    // reach the daemon even when shutdown canceled another pending operation.
    let _ = tokio::time::timeout(DETACH_SEND_TIMEOUT, async {
        if let Some(command) = interrupted_recording_stop {
            let _ = writer.send(command).await;
        }
        while let Ok(command) = cmd_rx.try_recv() {
            if matches!(command, ClientCommand::StopRecording { .. }) {
                let _ = writer.send(command).await;
            }
        }
        let _ = writer.send(ClientCommand::Detach).await;
    })
    .await;
    *status.lock().expect("daemon client status mutex poisoned") = ConnectionStatus::Disconnected;
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn automatic_trigger_recording_acknowledgements_survive_progress_coalescing() {
        let mut queue = super::ControlQueue::default();
        for (active, path, bytes) in [
            (true, "first.cf32", 0),
            (true, "first.cf32", 8),
            (false, "first.cf32", 16),
            (true, "second.cf32", 0),
            (true, "second.cf32", 24),
        ] {
            assert!(queue.push(ez_proto::ServerEvent::Recording(
                ez_proto::RecordingStatus {
                    channel_id: 1,
                    active,
                    path: Some(path.into()),
                    bytes_written: bytes,
                    duration_sec: 0.0,
                    stop_reason: None,
                }
            )));
        }
        let phases: Vec<_> = queue
            .events
            .iter()
            .map(|event| {
                let ez_proto::ServerEvent::Recording(status) = event else {
                    panic!("unexpected event")
                };
                (
                    status.active,
                    status.path.as_deref().unwrap(),
                    status.bytes_written,
                )
            })
            .collect();
        assert_eq!(
            phases,
            vec![
                (true, "first.cf32", 8),
                (false, "first.cf32", 16),
                (true, "second.cf32", 24)
            ]
        );
    }
    use super::*;
    use ez_daemon::{run, DaemonConfig, SourceConfig};
    use ez_proto::{ChannelSpec, PipelineKind};
    use std::sync::atomic::AtomicU64;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn mixed_stream_loopback_client_coalesces_snapshots_preserves_gaps_and_controls() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (read, write) = socket.into_split();
            let mut reader = FramedRead::new(read, MessageCodec::<ClientCommand>::for_commands());
            let mut writer = FramedWrite::new(write, MessageCodec::<ServerEvent>::for_data());
            assert!(matches!(
                reader.next().await.unwrap().unwrap(),
                ClientCommand::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    ..
                }
            ));
            writer
                .send(ServerEvent::Welcome {
                    protocol_version: PROTOCOL_VERSION,
                    client_channels: ez_proto::ClientChannels {
                        spectrum: 1,
                        audio: 2,
                        adsb: 3,
                        lrpt: 4,
                    },
                    active_channels: vec![],
                })
                .await
                .unwrap();
            for n in 0..1000 {
                writer
                    .send(ServerEvent::Audio(ez_proto::AudioFrame {
                        audio_clock: ez_proto::CaptureClock::new(7, n * 16, 48_000, None),
                        clock: ez_proto::CaptureClock::new(7, n * 16, 48_000, None),
                        sequence: n + 1,
                        channel_id: 2,
                        sample_rate_hz: 48_000,
                        samples: vec![0.25; 16],
                        squelch: ez_proto::SquelchState {
                            db: -120.0,
                            revision: 0,
                        },
                    }))
                    .await
                    .unwrap();
                writer
                    .send(ServerEvent::Spectrum(ez_proto::SpectrumFrame {
                        channel_id: 1,
                        center_hz: 100_000_000,
                        sample_rate_hz: 2_000_000,
                        bins: vec![n as f32; 16],
                        timestamp_ms: n,
                        observation_revision: 42,
                        measurement_bins: None,
                    }))
                    .await
                    .unwrap();
                writer
                    .send(ServerEvent::Aircraft(ez_proto::AircraftSnapshot {
                        channel_id: 3,
                        receiver: ez_proto::ReceiverSnapshot {
                            total_messages: n,
                            ..Default::default()
                        },
                        ..Default::default()
                    }))
                    .await
                    .unwrap();
                writer
                    .send(ServerEvent::Hardware(ez_proto::HardwareStatus {
                        connected: true,
                        source_kind: "loopback-fixture".into(),
                        frequency_hz: 100_000_000 + n,
                        sample_rate_hz: 2_000_000,
                        gain_db: 0.0,
                        requested_gain_db: None,
                        gain_mode: None,
                        error: None,
                    }))
                    .await
                    .unwrap();
            }
            assert!(matches!(
                reader.next().await.unwrap().unwrap(),
                ClientCommand::Ping { nonce: 77 }
            ));
            writer.send(ServerEvent::Pong { nonce: 77 }).await.unwrap();
            assert!(matches!(
                reader.next().await.unwrap().unwrap(),
                ClientCommand::Detach
            ));
        });
        let client = DaemonClient::connect(addr, "mixed-loopback".into());
        client.send(ClientCommand::Ping { nonce: 77 });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let replied = client
                .control_queue
                .lock()
                .unwrap()
                .events
                .iter()
                .any(|event| matches!(event, ServerEvent::Pong { nonce: 77 }));
            if replied {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "loopback client did not receive control reply: {:?}",
                client.status()
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(client.stream_queue.lock().unwrap().queued_frames(), 10);
        assert_eq!(client.stream_queue.lock().unwrap().skipped_frames(), 2990);
        assert!(
            matches!(client.try_recv_event(), Some(ServerEvent::Hardware(status)) if status.frequency_hz == 100_000_999)
        );
        assert!(matches!(
            client.try_recv_event(),
            Some(ServerEvent::Pong { nonce: 77 })
        ));
        let mut losses = std::collections::BTreeMap::new();
        let mut audio = Vec::new();
        let mut fresh = 0;
        while let Some(event) = client.try_recv_event() {
            match event {
                ServerEvent::StreamGap {
                    channel_id,
                    dropped_frames,
                } => {
                    losses.insert(channel_id, dropped_frames);
                }
                ServerEvent::Audio(frame) => {
                    assert_eq!(frame.clock.source_sample_index, (frame.sequence - 1) * 16);
                    audio.push(frame.sequence);
                }
                ServerEvent::Spectrum(frame) => {
                    assert_eq!(frame.timestamp_ms, 999);
                    fresh += 1;
                    assert!(client.spectrum_received_at().is_some());
                }
                ServerEvent::Aircraft(snapshot) => {
                    assert_eq!(snapshot.receiver.total_messages, 999);
                    fresh += 1;
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(audio, (993..=1000).collect::<Vec<_>>());
        assert_eq!(fresh, 2);
        assert_eq!(
            losses,
            std::collections::BTreeMap::from([(1, 999), (2, 992), (3, 999)])
        );
        drop(client);
        tokio::time::timeout(Duration::from_secs(5), peer)
            .await
            .unwrap()
            .unwrap();
        println!("Actual loopback GUI client: 3000 stream updates, 1000 hardware snapshots, newest snapshots/eight-frame audio tail, 2990 explicit skipped updates, controls first");
    }

    #[test]
    fn spectrum_observation_keeps_network_receipt_time_across_ui_queue_delay() {
        let (mut client, _, _) = DaemonClient::test_connection();
        let (sender, receiver) = crossbeam_channel::bounded(2);
        client.spectrum_event_rx = receiver;
        let received_at = Instant::now() - Duration::from_secs(2);
        sender
            .send((
                ServerEvent::Spectrum(ez_proto::SpectrumFrame {
                    channel_id: 1,
                    center_hz: 100_000_000,
                    sample_rate_hz: 2_000_000,
                    bins: vec![-40.0; 256],
                    timestamp_ms: 123,
                    observation_revision: 42,
                    measurement_bins: None,
                }),
                received_at,
            ))
            .unwrap();
        assert!(matches!(
            client.try_recv_event(),
            Some(ServerEvent::Spectrum(_))
        ));
        assert_eq!(client.spectrum_received_at(), Some(received_at));
    }

    async fn free_addr() -> SocketAddr {
        tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
    }

    pub(crate) struct TestDaemon {
        pub(crate) addr: SocketAddr,
        pub(crate) web_addr: SocketAddr,
        running: Arc<AtomicBool>,
        handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    }

    impl TestDaemon {
        pub(crate) async fn spawn(label: &str) -> Self {
            Self::spawn_with_rate(label, 2_000_000).await
        }

        pub(crate) async fn spawn_with_rate(label: &str, rate: u32) -> Self {
            let web_addr = free_addr().await;
            let addr = free_addr().await;
            let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "ez-gui-daemon-client-tests-{label}-{}-{n}",
                std::process::id()
            ));
            let config = DaemonConfig {
                listen_addr: addr,
                web_listen_addr: web_addr,
                initial_freq_hz: 100_000_000,
                initial_sample_rate_hz: rate,
                recording_dir: dir,
                web_static_dir: std::env::temp_dir()
                    .join("ez-gui-daemon-client-nonexistent-static"),
                source: SourceConfig::Synthetic,
                receiver_position: None,
            };
            let running = Arc::new(AtomicBool::new(true));
            let run_running = Arc::clone(&running);
            let handle = tokio::spawn(async move { run(config, run_running).await });

            // `run()` does real setup (build the source, spawn the ingestion thread,
            // construct `DaemonState`) before it binds the TCP listener, so the task above
            // returning "spawned" doesn't mean "accepting connections" yet. Unlike the
            // resilience test suite, `DaemonClient::connect` below is a single one-shot
            // attempt with no retry of its own (a real GUI client should surface
            // `ConnectionStatus::Error` immediately rather than silently retry-loop), so the
            // harness must block here until the daemon is actually listening.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                match tokio::net::TcpStream::connect(addr).await {
                    Ok(probe) => {
                        drop(probe);
                        break;
                    }
                    Err(_) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(e) => panic!("daemon under test never started accepting: {e}"),
                }
            }

            Self {
                addr,
                web_addr,
                running,
                handle,
            }
        }

        pub(crate) async fn shutdown(self) {
            self.running.store(false, Ordering::Relaxed);
            let result = tokio::time::timeout(Duration::from_secs(5), self.handle)
                .await
                .expect("daemon did not shut down promptly");
            assert!(result.unwrap().is_ok());
        }
    }

    async fn wait_for_status(
        client: &DaemonClient,
        target: impl Fn(&ConnectionStatus) -> bool,
        what: &str,
    ) -> ConnectionStatus {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = client.status();
            if target(&status) {
                return status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}, last status: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_event(client: &DaemonClient, what: &str) -> ServerEvent {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = client.try_recv_event() {
                return event;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn connect_reaches_connected_status_and_receives_hardware() {
        let daemon = TestDaemon::spawn("connect").await;
        let client = DaemonClient::connect(daemon.addr, "gui-test".to_string());

        wait_for_status(
            &client,
            |s| *s == ConnectionStatus::Connected,
            "Connected status",
        )
        .await;
        let event = wait_for_event(&client, "an event after connecting").await;
        assert!(
            matches!(event, ServerEvent::Hardware(_)),
            "expected Hardware, got {event:?}"
        );

        drop(client);
        daemon.shutdown().await;
    }

    #[tokio::test]
    async fn subscribing_to_spectrum_streams_frames_into_the_client() {
        let daemon = TestDaemon::spawn("spectrum").await;
        let client = DaemonClient::connect(daemon.addr, "gui-spectrum-test".to_string());
        wait_for_status(
            &client,
            |s| *s == ConnectionStatus::Connected,
            "Connected status",
        )
        .await;

        client.send(ClientCommand::Subscribe {
            channel: ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 1,
                center_offset_hz: 0,
                channel_sample_rate_hz: 2_000_000,
                kind: PipelineKind::Spectrum,
                demod_mode: None,
            },
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(ServerEvent::Spectrum(frame)) = client.try_recv_event() {
                assert!(!frame.bins.is_empty());
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for a spectrum frame"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        drop(client);
        daemon.shutdown().await;
    }

    #[tokio::test]
    async fn spectrum_recording_reports_live_and_final_status() {
        let daemon = TestDaemon::spawn("recording").await;
        let client = DaemonClient::connect(daemon.addr, "gui-recording-test".to_string());
        wait_for_status(
            &client,
            |status| *status == ConnectionStatus::Connected,
            "Connected status",
        )
        .await;

        client.send(ClientCommand::Subscribe {
            channel: ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 1,
                center_offset_hz: 0,
                channel_sample_rate_hz: 2_000_000,
                kind: PipelineKind::Spectrum,
                demod_mode: None,
            },
        });
        client.send(ClientCommand::StartRecording {
            channel_id: 1,
            format: ez_proto::RecordingFormat::Cf32,
        });

        // The recording writer runs on its own thread and the synthetic source
        // can be CPU-starved when the full GUI suite is running in parallel.
        // Keep this integration check bounded, but allow that first block to
        // reach disk under normal CI contention.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let live = loop {
            if let Some(ServerEvent::Recording(status)) = client.try_recv_event() {
                if status.active && status.bytes_written > 0 {
                    break status;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for live daemon recording status"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(live
            .path
            .as_deref()
            .is_some_and(|path| path.ends_with(".cf32")));

        client.send(ClientCommand::StopRecording { channel_id: 1 });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(ServerEvent::Recording(status)) = client.try_recv_event() {
                if !status.active {
                    assert!(status.bytes_written >= live.bytes_written);
                    break;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for final daemon recording status"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        drop(client);
        daemon.shutdown().await;
    }

    #[tokio::test]
    async fn connect_failure_is_reported_as_error_status() {
        let dead_addr = free_addr().await; // bound then dropped: nothing is listening
        let client = DaemonClient::connect(dead_addr, "gui-fail-test".to_string());
        let status = wait_for_status(
            &client,
            |s| matches!(s, ConnectionStatus::Error(_)),
            "Error status",
        )
        .await;
        assert!(matches!(status, ConnectionStatus::Error(_)));
    }

    #[tokio::test]
    async fn drop_delivers_queued_recording_stop_before_detaching() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = DaemonClient::connect(listener.local_addr().unwrap(), "record-stop".into());
        let (peer, _) = listener.accept().await.unwrap();
        let (read, write) = peer.into_split();
        let mut reader = FramedRead::new(read, MessageCodec::<ClientCommand>::for_commands());
        let mut writer = FramedWrite::new(write, MessageCodec::<ServerEvent>::for_data());
        assert!(matches!(
            reader.next().await,
            Some(Ok(ClientCommand::Hello { .. }))
        ));
        writer
            .send(ServerEvent::Welcome {
                client_channels: ez_proto::ClientChannels {
                    spectrum: 1,
                    audio: 2,
                    adsb: 3,
                    lrpt: 4,
                },
                protocol_version: PROTOCOL_VERSION,
                active_channels: Vec::new(),
            })
            .await
            .unwrap();
        wait_for_status(
            &client,
            |status| *status == ConnectionStatus::Connected,
            "Connected",
        )
        .await;
        client.send(ClientCommand::StopRecording { channel_id: 17 });
        let shutdown = tokio::task::spawn_blocking(move || drop(client));
        let command = tokio::time::timeout(Duration::from_secs(2), reader.next())
            .await
            .unwrap();
        assert!(matches!(
            command,
            Some(Ok(ClientCommand::StopRecording { channel_id: 17 }))
        ));
        shutdown.await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_a_pending_network_operation_without_waiting_for_its_timeout() {
        let running = Arc::new(AtomicBool::new(true));
        let stop = Arc::clone(&running);
        let shutdown = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            stop.store(false, Ordering::Relaxed);
        });
        let result = tokio::time::timeout(
            Duration::from_millis(750),
            until_stopped(&running, std::future::pending::<()>()),
        )
        .await
        .expect("pending network work prevented shutdown");
        assert_eq!(result, None);
        shutdown.await.unwrap();
    }

    #[tokio::test]
    async fn drop_cancels_a_silent_handshake_promptly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = DaemonClient::connect(listener.local_addr().unwrap(), "silent-peer".into());
        let (_silent_peer, _) = listener.accept().await.unwrap();
        assert_eq!(client.status(), ConnectionStatus::Connecting);
        let started = std::time::Instant::now();
        tokio::task::spawn_blocking(move || drop(client))
            .await
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(750),
            "stop waited for the remote handshake: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn drop_completes_promptly_after_connecting() {
        let daemon = TestDaemon::spawn("drop").await;
        let client = DaemonClient::connect(daemon.addr, "gui-drop-test".to_string());
        wait_for_status(
            &client,
            |s| *s == ConnectionStatus::Connected,
            "Connected status",
        )
        .await;

        let start = tokio::time::Instant::now();
        // `Drop` blocks synchronously (joins the worker thread) — offload to the blocking
        // pool so it doesn't stall this test's own async executor thread while we time it.
        tokio::task::spawn_blocking(move || drop(client))
            .await
            .unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Drop took too long to tear down the connection"
        );

        daemon.shutdown().await;
    }
    #[tokio::test]
    async fn daemon_squelch_wire_replies_report_applied_revision_and_rejections() {
        let daemon = TestDaemon::spawn("squelch-wire").await;
        let client = DaemonClient::connect(daemon.addr, "squelch-wire".into());
        wait_for_status(&client, |s| *s == ConnectionStatus::Connected, "connection").await;
        client.send(ClientCommand::Subscribe {
            channel: ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 71,
                center_offset_hz: 0,
                channel_sample_rate_hz: 200_000,
                kind: PipelineKind::Audio,
                demod_mode: Some(ez_proto::DemodMode::Fm),
            },
        });
        client.send(ClientCommand::Subscribe {
            channel: ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 72,
                center_offset_hz: 0,
                channel_sample_rate_hz: 2_000_000,
                kind: PipelineKind::Spectrum,
                demod_mode: None,
            },
        });
        let mut revision = 0;
        for (request_id, channel_id, db, expected) in [
            (1, 71, -50.0, None),
            (2, 71, -50.0, None),
            (3, 71, -20.0, None),
            (4, 99, -50.0, Some("unknown channel")),
            (5, 72, -50.0, Some("not an audio channel")),
            (6, 71, f32::NAN, Some("finite")),
            (7, 71, 61.0, Some("between")),
            (8, 71, -241.0, Some("between")),
        ] {
            client.send(ClientCommand::SetSquelch {
                channel_id,
                db,
                request_id,
            });
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let event = wait_for_event(&client, "squelch reply").await;
                if let ServerEvent::Error { message } = &event {
                    panic!("daemon command error: {message}");
                }
                if let ServerEvent::Squelch {
                    channel_id: returned_channel,
                    request_id: returned_request,
                    result,
                } = event
                {
                    assert_eq!(
                        (returned_channel, returned_request),
                        (channel_id, request_id)
                    );
                    match expected {
                        Some(error) => assert!(result.unwrap_err().contains(error)),
                        None => {
                            let state = result.unwrap();
                            assert_eq!(state.db, db);
                            if request_id == 2 {
                                assert_eq!(state.revision, revision);
                            } else {
                                assert!(state.revision > revision);
                            }
                            revision = state.revision;
                        }
                    }
                    break;
                }
                assert!(tokio::time::Instant::now() < deadline);
            }
        }
        drop(client);
        daemon.shutdown().await;
    }
}
