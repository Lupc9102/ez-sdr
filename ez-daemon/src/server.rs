//! Async TCP front door: accepts client connections, performs the `Hello`/`Welcome`
//! handshake, and routes each connection's [`ClientCommand`]s into [`DaemonState`] while
//! fanning its subscribed pipeline output back out as [`ServerEvent`]s.
//!
//! Per connection, three concurrently-running pieces cooperate over plain channels (no
//! shared trait, no generic connection-handler framework):
//! - the command loop (`command_loop`), driven by `tokio::select!` over incoming commands
//!   and a periodic hardware-status tick;
//! - one writer task (`run_writer`) that owns the socket's write half and drains an
//!   `mpsc::Receiver<ServerEvent>` — the single point every event funnels through, so
//!   commands, hardware-status ticks, and pipeline forwarders never fight over the socket;
//! - one plain OS thread per active subscription (`forward`), bridging a blocking
//!   [`BroadcasterHandle::recv_timeout`] pull to a `blocking_send` on the writer's mpsc
//!   channel — deliberately a thread, not `spawn_blocking`, since it just blocks on a
//!   condvar-backed channel rather than doing CPU or filesystem work.
//!
//! A client closing its socket or sending `Detach` both end the connection task the same
//! way: the command loop returns, every forwarder is told to stop, and the writer task
//! drains out once the event channel closes.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite};

use ez_proto::{ChannelId, ClientCommand, MessageCodec, ServerEvent, PROTOCOL_VERSION};

use crate::broadcast::BroadcasterHandle;
use crate::state::{ChannelSubscription, DaemonState};

const ACCEPT_POLL: Duration = Duration::from_millis(100);
const CONNECTION_TICK: Duration = Duration::from_millis(250);
const FORWARD_POLL: Duration = Duration::from_millis(200);
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Accepts connections on `listener` until `running` clears, spawning one task per
/// connection. Takes an already-bound [`TcpListener`] (rather than a [`SocketAddr`]) so
/// callers — and tests — can observe the resolved local address (useful for `:0` ephemeral
/// ports) before handing it off.
pub async fn serve(
    listener: TcpListener,
    state: Arc<DaemonState>,
    running: Arc<AtomicBool>,
) -> Result<()> {
    let local_addr = listener.local_addr().ok();
    tracing::info!(?local_addr, "ez-daemon listening");
    let sem = Arc::new(tokio::sync::Semaphore::new(64));

    while running.load(Ordering::Relaxed) {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        let permit = match sem.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::warn!(%peer, "connection limit reached, dropping connection");
                                continue;
                            }
                        };
                        let state = Arc::clone(&state);
                        let conn_running = Arc::clone(&running);
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, peer, state, conn_running).await {
                                tracing::warn!(%peer, error = %e, "connection ended with error");
                            }
                            drop(permit);
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "accept failed"),
                }
            }
            () = tokio::time::sleep(ACCEPT_POLL) => {}
        }
    }
    tracing::info!("ez-daemon TCP server shutting down");
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    state: Arc<DaemonState>,
    running: Arc<AtomicBool>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = FramedRead::new(read_half, MessageCodec::<ClientCommand>::for_commands());
    let mut writer = FramedWrite::new(write_half, MessageCodec::<ServerEvent>::for_data());

    let client_name = match tokio::time::timeout(
        Duration::from_secs(5),
        perform_handshake(&mut reader, &mut writer),
    )
    .await
    {
        Ok(Ok(Some(name))) => name,
        Ok(Ok(None)) => return Ok(()),
        Ok(Err(e)) => return Err(e),
        Err(_) => anyhow::bail!("handshake timed out"),
    };
    tracing::info!(%peer, client_name, "client attached");

    writer
        .send(ServerEvent::Welcome {
            protocol_version: PROTOCOL_VERSION,
            active_channels: state.active_channels(),
        })
        .await?;
    writer
        .send(ServerEvent::Hardware(state.hardware_status()))
        .await?;

    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let writer_task = tokio::spawn(run_writer(writer, event_rx));

    let mut forwarders: HashMap<ChannelId, Arc<AtomicBool>> = HashMap::new();
    let result = command_loop(&mut reader, &state, &running, &event_tx, &mut forwarders).await;

    for stop in forwarders.values() {
        stop.store(false, Ordering::Relaxed);
    }
    drop(event_tx);
    let _ = writer_task.await;
    tracing::info!(%peer, "client detached");
    result
}

/// Reads the connection's first message and validates it's a version-matched `Hello`,
/// sending `ServerEvent::Error` and returning `Ok(None)` for any handshake failure (bad
/// first message, version mismatch, or the client closing before sending one) — the caller
/// treats `Ok(None)` as "close this connection", not an error, since a rejected handshake is
/// expected client behavior, not a daemon fault.
async fn perform_handshake(
    reader: &mut FramedRead<OwnedReadHalf, MessageCodec<ClientCommand>>,
    writer: &mut FramedWrite<OwnedWriteHalf, MessageCodec<ServerEvent>>,
) -> Result<Option<String>> {
    match reader.next().await {
        Some(Ok(ClientCommand::Hello {
            client_name,
            protocol_version,
        })) => {
            if protocol_version != PROTOCOL_VERSION {
                let _ = writer
                    .send(ServerEvent::Error {
                        message: format!(
                            "unsupported protocol version {protocol_version}, daemon speaks {PROTOCOL_VERSION}"
                        ),
                    })
                    .await;
                return Ok(None);
            }
            Ok(Some(client_name))
        }
        Some(Ok(_)) => {
            let _ = writer
                .send(ServerEvent::Error {
                    message: "first message must be Hello".to_string(),
                })
                .await;
            Ok(None)
        }
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

async fn run_writer(
    mut writer: FramedWrite<OwnedWriteHalf, MessageCodec<ServerEvent>>,
    mut events: mpsc::Receiver<ServerEvent>,
) {
    while let Some(event) = events.recv().await {
        if writer.send(event).await.is_err() {
            break;
        }
    }
}

/// Drives one connection after a successful handshake: routes incoming commands and pushes
/// a periodic [`ServerEvent::Hardware`] snapshot, until the client disconnects, sends
/// `Detach`, or `daemon_running` clears (checked once per loop iteration so a daemon
/// shutdown doesn't leave connection tasks blocked forever on an idle client's socket).
async fn command_loop(
    reader: &mut FramedRead<OwnedReadHalf, MessageCodec<ClientCommand>>,
    state: &Arc<DaemonState>,
    daemon_running: &Arc<AtomicBool>,
    event_tx: &mpsc::Sender<ServerEvent>,
    forwarders: &mut HashMap<ChannelId, Arc<AtomicBool>>,
) -> Result<()> {
    let mut hw_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + CONNECTION_TICK,
        CONNECTION_TICK,
    );

    while daemon_running.load(Ordering::Relaxed) {
        tokio::select! {
            msg = reader.next() => {
                match msg {
                    Some(Ok(cmd)) => {
                        if !apply_command(cmd, state, event_tx, forwarders).await {
                            return Ok(());
                        }
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()),
                }
            }
            _ = hw_tick.tick() => {
                if event_tx.send(ServerEvent::Hardware(state.hardware_status())).await.is_err() {
                    return Ok(());
                }
                for status in state.recording_statuses() {
                    if event_tx.send(ServerEvent::Recording(status)).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

/// Applies one command. Returns `false` only for `Detach`, signaling `command_loop` to end
/// the connection.
///
/// `pub(crate)` so `crate::web`'s control WebSocket (JSON-over-WS instead of bincode-over-TCP)
/// can drive the exact same command semantics rather than re-implementing this match — the
/// two transports differ only in framing, never in what a given [`ClientCommand`] does.
pub(crate) async fn apply_command(
    cmd: ClientCommand,
    state: &Arc<DaemonState>,
    event_tx: &mpsc::Sender<ServerEvent>,
    forwarders: &mut HashMap<ChannelId, Arc<AtomicBool>>,
) -> bool {
    match cmd {
        ClientCommand::Hello { .. } => {
            send_error(event_tx, "unexpected Hello after handshake".to_string()).await;
        }
        ClientCommand::SetFrequency { hz } => {
            let state = Arc::clone(state);
            match tokio::task::spawn_blocking(move || state.set_frequency(hz)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => send_error(event_tx, error.to_string()).await,
                Err(error) => send_error(event_tx, format!("hardware task failed: {error}")).await,
            }
        }
        ClientCommand::SetSampleRate { hz } => {
            let state = Arc::clone(state);
            match tokio::task::spawn_blocking(move || state.set_sample_rate(hz)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => send_error(event_tx, error.to_string()).await,
                Err(error) => send_error(event_tx, format!("hardware task failed: {error}")).await,
            }
        }
        ClientCommand::SetGain { db } => {
            let state = Arc::clone(state);
            match tokio::task::spawn_blocking(move || state.set_gain(db)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => send_error(event_tx, error.to_string()).await,
                Err(error) => send_error(event_tx, format!("hardware task failed: {error}")).await,
            }
        }
        ClientCommand::Subscribe { channel } => {
            let channel_id = channel.id;
            match state.subscribe(channel) {
                Ok((sub, _)) => spawn_forwarder(channel_id, sub, event_tx.clone(), forwarders),
                Err(e) => send_error(event_tx, e.to_string()).await,
            }
        }
        ClientCommand::Unsubscribe { channel_id } => {
            if let Some(stop) = forwarders.remove(&channel_id) {
                stop.store(false, Ordering::Relaxed);
            }
        }
        ClientCommand::SetDemodMode { channel_id, mode } => {
            if let Err(e) = state.set_demod_mode(channel_id, mode) {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::SetVolume { channel_id, level } => {
            if let Err(e) = state.set_volume(channel_id, level) {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::SetSquelch { channel_id, db } => {
            if let Err(e) = state.set_squelch(channel_id, db) {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::Retune {
            channel_id,
            center_offset_hz,
            bandwidth_hz,
        } => {
            if let Err(e) = state.retune(channel_id, center_offset_hz, bandwidth_hz) {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::RemoveChannel { channel_id } => {
            if let Err(e) = state.remove_channel(channel_id) {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::StartRecording { channel_id, format } => {
            match state.start_recording(channel_id, format) {
                Ok(status) => {
                    let _ = event_tx.send(ServerEvent::Recording(status)).await;
                }
                Err(e) => send_error(event_tx, e.to_string()).await,
            }
        }
        ClientCommand::StopRecording { channel_id } => match state.stop_recording(channel_id) {
            Ok(status) => {
                let _ = event_tx.send(ServerEvent::Recording(status)).await;
            }
            Err(e) => send_error(event_tx, e.to_string()).await,
        },
        ClientCommand::Ping { nonce } => {
            let _ = event_tx.send(ServerEvent::Pong { nonce }).await;
        }
        ClientCommand::Detach => return false,
    }
    true
}

async fn send_error(event_tx: &mpsc::Sender<ServerEvent>, message: String) {
    let _ = event_tx.send(ServerEvent::Error { message }).await;
}

/// Starts (or replaces) `channel_id`'s forwarder thread for this connection. Replacing
/// rather than leaking is what keeps a duplicate `Subscribe` for an id already active on
/// this same connection safe: the previous thread is told to stop before the new one
/// starts, instead of orphaning it with no reachable stop flag.
///
/// `pub(crate)` so `crate::web`'s data-plane WebSocket handler can spin up the same kind of
/// per-subscription forwarder thread against its own `mpsc::Sender<axum::extract::ws::Message>`
/// instead of duplicating the `ChannelSubscription` match.
pub(crate) fn spawn_forwarder(
    channel_id: ChannelId,
    sub: ChannelSubscription,
    event_tx: mpsc::Sender<ServerEvent>,
    forwarders: &mut HashMap<ChannelId, Arc<AtomicBool>>,
) {
    if let Some(previous) = forwarders.remove(&channel_id) {
        previous.store(false, Ordering::Relaxed);
    }
    let keep_running = match sub {
        ChannelSubscription::Spectrum(h) => {
            spawn_forward_thread(channel_id, h, event_tx, ServerEvent::Spectrum)
        }
        ChannelSubscription::Audio(h) => {
            spawn_forward_thread(channel_id, h, event_tx, ServerEvent::Audio)
        }
        ChannelSubscription::AdsbPackets(h) => {
            spawn_forward_thread(channel_id, h, event_tx, ServerEvent::Aircraft)
        }
        ChannelSubscription::LrptTelemetry(h) => {
            spawn_forward_thread(channel_id, h, event_tx, ServerEvent::Telemetry)
        }
    };
    forwarders.insert(channel_id, keep_running);
}

/// Generic over the outbound message type `M` (not just [`ServerEvent`]) so `crate::web`'s
/// WebSocket data plane can reuse this exact spawn-and-bridge logic with
/// `M = axum::extract::ws::Message` — the wrapping closure is the only thing that differs
/// between "wrap for bincode-over-TCP" and "wrap for a WS frame".
pub(crate) fn spawn_forward_thread<T: Send + 'static, M: Send + 'static>(
    channel_id: ChannelId,
    handle: BroadcasterHandle<T>,
    tx: mpsc::Sender<M>,
    wrap: impl Fn(T) -> M + Send + 'static,
) -> Arc<AtomicBool> {
    let keep_running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&keep_running);
    std::thread::Builder::new()
        .name(format!("ez-daemon-fwd-{channel_id}"))
        .spawn(move || forward(handle, tx, thread_running, wrap))
        .expect("spawning forwarder thread");
    keep_running
}

/// Bridges a blocking [`BroadcasterHandle`] pull to an async `mpsc` channel via
/// `blocking_send` — valid and intended here specifically because this runs on a plain OS
/// thread (see [`spawn_forward_thread`]), never a tokio worker thread. Polls `running` every
/// `FORWARD_POLL` rather than blocking on `recv` forever so `Unsubscribe`/disconnect is
/// noticed promptly without needing a second wakeup channel.
///
/// Generic over `M` for the same reason as [`spawn_forward_thread`]: the TCP writer wants
/// `ServerEvent`, the WS data plane wants a WS frame, and this loop doesn't need to know which.
pub(crate) fn forward<T, M>(
    handle: BroadcasterHandle<T>,
    tx: mpsc::Sender<M>,
    running: Arc<AtomicBool>,
    wrap: impl Fn(T) -> M,
) {
    while running.load(Ordering::Relaxed) {
        if let Some(item) = handle.recv_timeout(FORWARD_POLL) {
            let mut msg = wrap(item);
            let mut attempts = 0;
            while running.load(Ordering::Relaxed) {
                match tx.try_send(msg) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                    Err(mpsc::error::TrySendError::Full(returned_msg)) => {
                        attempts += 1;
                        if attempts >= 50 {
                            // Relieve backpressure: drop frame rather than blocking indefinitely
                            break;
                        }
                        msg = returned_msg;
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SampleBus;
    use crate::hardware::synthetic::SyntheticSource;
    use crate::hardware::IqSource;
    use crate::ingest;
    use ez_proto::{ChannelSpec, PipelineKind};
    use std::sync::atomic::AtomicU64;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct RejectFrequencySource(SyntheticSource);

    impl IqSource for RejectFrequencySource {
        fn start(&mut self) -> anyhow::Result<()> {
            self.0.start()
        }
        fn stop(&mut self) {
            self.0.stop();
        }
        fn set_frequency(&mut self, _hz: u64) -> anyhow::Result<()> {
            anyhow::bail!("frequency rejected by TCP test source")
        }
        fn set_sample_rate(&mut self, hz: u32) -> anyhow::Result<()> {
            self.0.set_sample_rate(hz)
        }
        fn set_gain(&mut self, db: f64) -> anyhow::Result<()> {
            self.0.set_gain(db)
        }
        fn read_iq(&mut self, buf: &mut [num_complex::Complex32]) -> anyhow::Result<usize> {
            self.0.read_iq(buf)
        }
        fn frequency_hz(&self) -> u64 {
            self.0.frequency_hz()
        }
        fn sample_rate_hz(&self) -> u32 {
            self.0.sample_rate_hz()
        }
        fn gain_db(&self) -> f64 {
            self.0.gain_db()
        }
        fn kind(&self) -> &'static str {
            "reject-frequency-test"
        }
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("ez-daemon-{label}-{}-{n}", std::process::id()))
    }

    async fn spawn_test_server() -> (SocketAddr, Arc<AtomicBool>) {
        let bus = SampleBus::new();
        let ingest_running = Arc::new(AtomicBool::new(true));
        let (hardware, _ingest_thread) = ingest::spawn(
            Box::new(SyntheticSource::default()),
            bus.clone(),
            ingest_running,
        )
        .unwrap();
        let state = Arc::new(DaemonState::new(
            bus,
            hardware,
            100_000_000,
            2_000_000,
            unique_temp_dir("server"),
        ));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        tokio::spawn(async move {
            let _ = serve(listener, state, server_running).await;
        });
        (addr, running)
    }

    async fn spawn_rejecting_test_server() -> (SocketAddr, Arc<AtomicBool>) {
        let bus = SampleBus::new();
        let ingest_running = Arc::new(AtomicBool::new(true));
        let (hardware, _ingest_thread) = ingest::spawn(
            Box::new(RejectFrequencySource(SyntheticSource::default())),
            bus.clone(),
            ingest_running,
        )
        .unwrap();
        let state = Arc::new(DaemonState::new(
            bus,
            hardware,
            100_000_000,
            2_000_000,
            unique_temp_dir("server-reject"),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        tokio::spawn(async move {
            let _ = serve(listener, state, server_running).await;
        });
        (addr, running)
    }

    async fn connect(
        addr: SocketAddr,
    ) -> (
        FramedWrite<OwnedWriteHalf, MessageCodec<ClientCommand>>,
        FramedRead<OwnedReadHalf, MessageCodec<ServerEvent>>,
    ) {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (r, w) = stream.into_split();
        (
            FramedWrite::new(w, MessageCodec::<ClientCommand>::for_commands()),
            FramedRead::new(r, MessageCodec::<ServerEvent>::for_data()),
        )
    }

    async fn hello(
        writer: &mut FramedWrite<OwnedWriteHalf, MessageCodec<ClientCommand>>,
        name: &str,
    ) {
        writer
            .send(ClientCommand::Hello {
                client_name: name.to_string(),
                protocol_version: PROTOCOL_VERSION,
            })
            .await
            .unwrap();
    }

    async fn wait_for_spectrum_frame(
        reader: &mut FramedRead<OwnedReadHalf, MessageCodec<ServerEvent>>,
    ) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let ServerEvent::Spectrum(_) = reader.next().await.unwrap().unwrap() {
                    return;
                }
            }
        })
        .await
        .expect("expected a spectrum frame");
    }

    fn spectrum_spec(id: ChannelId) -> ChannelSpec {
        ChannelSpec {
            id,
            center_offset_hz: 0,
            bandwidth_hz: 2_000_000,
            kind: PipelineKind::Spectrum,
            demod_mode: None,
        }
    }

    #[tokio::test]
    async fn handshake_succeeds_and_yields_welcome_then_hardware() {
        let (addr, running) = spawn_test_server().await;
        let (mut writer, mut reader) = connect(addr).await;
        hello(&mut writer, "test-client").await;

        let welcome = reader.next().await.unwrap().unwrap();
        assert!(
            matches!(welcome, ServerEvent::Welcome { protocol_version, .. } if protocol_version == PROTOCOL_VERSION)
        );

        let hw = reader.next().await.unwrap().unwrap();
        assert!(matches!(hw, ServerEvent::Hardware(_)));

        running.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn rejected_hardware_frequency_is_reported_to_tcp_client() {
        let (addr, running) = spawn_rejecting_test_server().await;
        let (mut writer, mut reader) = connect(addr).await;
        hello(&mut writer, "hardware-error-client").await;
        let _ = reader.next().await;
        let _ = reader.next().await;

        writer
            .send(ClientCommand::SetFrequency { hz: 105_000_000 })
            .await
            .unwrap();
        let error = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let ServerEvent::Error { message } = reader.next().await.unwrap().unwrap() {
                    return message;
                }
            }
        })
        .await
        .expect("expected backend rejection over TCP");
        assert!(error.contains("frequency rejected by TCP test source"));
        running.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn non_hello_first_message_is_rejected_and_connection_closes() {
        let (addr, running) = spawn_test_server().await;
        let (mut writer, mut reader) = connect(addr).await;
        writer.send(ClientCommand::Ping { nonce: 1 }).await.unwrap();

        let event = reader.next().await.unwrap().unwrap();
        assert!(matches!(event, ServerEvent::Error { .. }));
        assert!(
            reader.next().await.is_none(),
            "connection should close after handshake failure"
        );

        running.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn mismatched_protocol_version_is_rejected() {
        let (addr, running) = spawn_test_server().await;
        let (mut writer, mut reader) = connect(addr).await;
        writer
            .send(ClientCommand::Hello {
                client_name: "old".into(),
                protocol_version: PROTOCOL_VERSION + 1,
            })
            .await
            .unwrap();

        let event = reader.next().await.unwrap().unwrap();
        assert!(matches!(event, ServerEvent::Error { .. }));

        running.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn subscribing_to_spectrum_streams_frames() {
        let (addr, running) = spawn_test_server().await;
        let (mut writer, mut reader) = connect(addr).await;
        hello(&mut writer, "spectrum-watcher").await;
        let _ = reader.next().await;
        let _ = reader.next().await;

        writer
            .send(ClientCommand::Subscribe {
                channel: spectrum_spec(1),
            })
            .await
            .unwrap();
        wait_for_spectrum_frame(&mut reader).await;

        running.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn detach_closes_the_connection() {
        let (addr, running) = spawn_test_server().await;
        let (mut writer, mut reader) = connect(addr).await;
        hello(&mut writer, "detacher").await;
        let _ = reader.next().await;
        let _ = reader.next().await;

        writer.send(ClientCommand::Detach).await.unwrap();

        let result = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if reader.next().await.is_none() {
                    break;
                }
            }
        })
        .await;
        assert!(result.is_ok(), "connection never closed after Detach");

        running.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn unsubscribe_stops_frames_without_closing_the_connection() {
        let (addr, running) = spawn_test_server().await;
        let (mut writer, mut reader) = connect(addr).await;
        hello(&mut writer, "unsubscriber").await;
        let _ = reader.next().await;
        let _ = reader.next().await;

        writer
            .send(ClientCommand::Subscribe {
                channel: spectrum_spec(1),
            })
            .await
            .unwrap();
        wait_for_spectrum_frame(&mut reader).await;

        writer
            .send(ClientCommand::Unsubscribe { channel_id: 1 })
            .await
            .unwrap();
        writer
            .send(ClientCommand::Ping { nonce: 42 })
            .await
            .unwrap();

        let pong = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let ServerEvent::Pong { nonce } = reader.next().await.unwrap().unwrap() {
                    return nonce;
                }
            }
        })
        .await
        .expect("expected a pong after unsubscribe");
        assert_eq!(pong, 42);

        running.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn two_clients_can_attach_to_the_same_channel_independently() {
        let (addr, running) = spawn_test_server().await;

        let (mut w1, mut r1) = connect(addr).await;
        hello(&mut w1, "client-1").await;
        let _ = r1.next().await;
        let _ = r1.next().await;
        w1.send(ClientCommand::Subscribe {
            channel: spectrum_spec(1),
        })
        .await
        .unwrap();
        wait_for_spectrum_frame(&mut r1).await;

        let (mut w2, mut r2) = connect(addr).await;
        hello(&mut w2, "client-2").await;
        let welcome2 = r2.next().await.unwrap().unwrap();
        match welcome2 {
            ServerEvent::Welcome {
                active_channels, ..
            } => {
                assert_eq!(
                    active_channels.len(),
                    1,
                    "client 2 should see the channel client 1 already created"
                );
            }
            other => panic!("expected Welcome, got {other:?}"),
        }
        let _ = r2.next().await;
        w2.send(ClientCommand::Subscribe {
            channel: spectrum_spec(1),
        })
        .await
        .unwrap();

        wait_for_spectrum_frame(&mut r1).await;
        wait_for_spectrum_frame(&mut r2).await;

        running.store(false, Ordering::Relaxed);
    }

    #[test]
    fn forward_exits_promptly_when_running_clears_under_backpressure() {
        // Issue 20: forward thread must not hang in blocking_send when channel is full and running is cleared.
        use crate::broadcast::{Broadcaster, OverflowPolicy};
        let b = Broadcaster::new();
        let handle = b.subscribe(4, OverflowPolicy::DropOldest);
        let (tx, _rx) = mpsc::channel(1);

        // Fill channel completely
        tx.try_send(100).expect("fill channel");

        // Now publish item to broadcaster so forward will try to send
        b.publish(200);

        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);

        let thread = std::thread::spawn(move || {
            forward(handle, tx, thread_running, |x| x);
        });

        // Let forward start and enter the full-channel retry loop
        std::thread::sleep(Duration::from_millis(30));

        // Signal stop
        running.store(false, Ordering::Relaxed);

        // Thread must join promptly without hanging
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = thread.join();
            let _ = done_tx.send(());
        });

        assert!(
            done_rx.recv_timeout(Duration::from_millis(500)).is_ok(),
            "forward thread must exit when running is cleared under backpressure"
        );
    }
}
