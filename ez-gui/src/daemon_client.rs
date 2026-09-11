//! Background network client for `ez-daemon`: owns a dedicated OS thread running its own
//! single-threaded Tokio runtime, so the rest of `ez-gui` (a plain synchronous `egui` loop)
//! never needs to poll an async executor itself. Commands flow out over an unbounded
//! `tokio::sync::mpsc` channel — `UnboundedSender::send` is synchronous and non-blocking, so
//! it's callable directly from the egui thread with no `.await`. Events flow back over a
//! `crossbeam_channel`, matching the same non-blocking `try_recv()` polling shape
//! `SourceManager` already uses for its local worker threads (see `crate::source_manager`).
//!
//! Modeled directly on `ez_daemon::server`'s own connection handling (handshake sequence,
//! `tokio::select!` shutdown-polling pattern) so the two ends of the wire protocol stay in
//! lockstep by construction rather than by convention.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// Handle to a background-thread connection to a running `ez-daemon`. Single-owner by design
/// (not `Clone`) — `SourceManager` holds exactly one at a time. Dropping it signals the worker
/// to stop, best-effort sends `Detach`, and joins the thread, so no orphaned connection or
/// thread outlives the handle.
pub struct DaemonClient {
    status: Arc<Mutex<ConnectionStatus>>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<ClientCommand>,
    event_rx: crossbeam_channel::Receiver<ServerEvent>,
    running: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl DaemonClient {
    /// Spawns the background thread and starts connecting immediately; never blocks the
    /// caller. Poll [`Self::status`] to observe handshake progress or failure.
    #[must_use]
    pub fn connect(addr: SocketAddr, client_name: String) -> Self {
        let status = Arc::new(Mutex::new(ConnectionStatus::Connecting));
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
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
                    cmd_rx,
                    event_tx,
                    worker_running,
                ));
            })
            .expect("failed to spawn ez-gui-daemon-client thread");

        Self {
            status,
            cmd_tx,
            event_rx,
            running,
            worker: Some(worker),
        }
    }

    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        self.status
            .lock()
            .expect("daemon client status mutex poisoned")
            .clone()
    }

    /// Queues a command for delivery. Fire-and-forget, matching `ez_daemon::ingest::IngestHandle`'s
    /// style: the only failure mode is the worker thread having already exited, which
    /// `status()` already reports, so a dropped send here would be redundant to also surface.
    pub fn send(&self, cmd: ClientCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Non-blocking pull of one queued event, oldest first. Returns `None` if nothing is
    /// queued right now — callers poll this from their own render/update loop.
    #[must_use]
    pub fn try_recv_event(&self) -> Option<ServerEvent> {
        self.event_rx.try_recv().ok()
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

async fn connect_and_handshake(addr: SocketAddr, client_name: &str) -> Result<Halves, String> {
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

    match reader.next().await {
        Some(Ok(ServerEvent::Welcome {
            protocol_version, ..
        })) => {
            if protocol_version != PROTOCOL_VERSION {
                return Err(format!(
                    "protocol version mismatch: daemon speaks {protocol_version}, client speaks {PROTOCOL_VERSION}"
                ));
            }
        }
        Some(Ok(ServerEvent::Error { message })) => {
            return Err(format!("daemon rejected handshake: {message}"))
        }
        Some(Ok(other)) => return Err(format!("expected Welcome, got {other:?}")),
        Some(Err(e)) => return Err(format!("handshake decode error: {e}")),
        None => return Err("connection closed during handshake".to_string()),
    }

    Ok((reader, writer))
}

/// Drives one connection end to end: handshake, then a `tokio::select!` loop routing queued
/// outgoing commands to the socket and incoming frames to `event_tx`, until `running` clears,
/// the peer closes the connection, or a decode error occurs. Always leaves `status` in a
/// terminal state (`Disconnected` or `Error`) before returning, so `DaemonClient::status`
/// never gets stuck reporting a connection that's actually gone.
async fn run_connection(
    addr: SocketAddr,
    client_name: String,
    status: Arc<Mutex<ConnectionStatus>>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<ClientCommand>,
    event_tx: crossbeam_channel::Sender<ServerEvent>,
    running: Arc<AtomicBool>,
) {
    let handshake =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, connect_and_handshake(addr, &client_name)).await;
    let (mut reader, mut writer) = match handshake {
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
    *status.lock().expect("daemon client status mutex poisoned") = ConnectionStatus::Connected;

    let mut ticker = tokio::time::interval(SHUTDOWN_POLL_INTERVAL);
    while running.load(Ordering::Relaxed) {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ClientCommand::Detach) => {
                        let _ = tokio::time::timeout(DETACH_SEND_TIMEOUT, writer.send(ClientCommand::Detach)).await;
                        break;
                    }
                    Some(cmd) => {
                        if writer.send(cmd).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            frame = reader.next() => {
                match frame {
                    Some(Ok(event)) => {
                        let _ = event_tx.send(event);
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
    *status.lock().expect("daemon client status mutex poisoned") = ConnectionStatus::Disconnected;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ez_daemon::{run, DaemonConfig, SourceConfig};
    use ez_proto::{ChannelSpec, PipelineKind};
    use std::sync::atomic::AtomicU64;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn free_addr() -> SocketAddr {
        tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
    }

    struct TestDaemon {
        addr: SocketAddr,
        running: Arc<AtomicBool>,
        handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    }

    impl TestDaemon {
        async fn spawn(label: &str) -> Self {
            let addr = free_addr().await;
            let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "ez-gui-daemon-client-tests-{label}-{}-{n}",
                std::process::id()
            ));
            let config = DaemonConfig {
                listen_addr: addr,
                web_listen_addr: free_addr().await,
                initial_freq_hz: 100_000_000,
                initial_sample_rate_hz: 2_000_000,
                recording_dir: dir,
                web_static_dir: std::env::temp_dir()
                    .join("ez-gui-daemon-client-nonexistent-static"),
                source: SourceConfig::Synthetic,
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
                running,
                handle,
            }
        }

        async fn shutdown(self) {
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
                id: 1,
                center_offset_hz: 0,
                bandwidth_hz: 2_000_000,
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
}
