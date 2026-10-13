//! Async TCP front door: accepts client connections, performs the `Hello`/`Welcome`
//! handshake, and routes each connection's [`ClientCommand`]s into [`DaemonState`] while
//! fanning its subscribed pipeline output back out as [`ServerEvent`]s.
//!
//! Per connection, three concurrently-running pieces cooperate over plain channels (no
//! shared trait, no generic connection-handler framework):
//! - the command loop (`command_loop`), driven by `tokio::select!` over incoming commands
//!   and a periodic hardware-status tick;
//! - one writer task (`run_writer`) that owns the socket's write half and drains an
//!   a prioritized event mailbox — the single point every event funnels through, so
//!   commands, hardware-status ticks, and pipeline forwarders never fight over the socket;
//! - one plain OS thread per active subscription (`forward`), bridging a blocking
//!   [`BroadcasterHandle::recv_timeout`] pull into bounded/coalesced stream slots,
//!   independent of controls — deliberately a thread, since it just blocks on a
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

use crate::outbound::{EventReceiver, EventSender};
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
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
        perform_handshake(&mut reader, &mut writer, &state),
    )
    .await
    {
        Ok(Ok(Some(name))) => name,
        Ok(Ok(None)) => return Ok(()),
        Ok(Err(e)) => return Err(e),
        Err(_) => anyhow::bail!("handshake timed out"),
    };
    tracing::info!(%peer, client_name, "client attached");

    let client_channels = state.allocate_client_channels()?;
    writer
        .send(ServerEvent::Welcome {
            client_channels,
            protocol_version: PROTOCOL_VERSION,
            active_channels: state.active_channels(),
        })
        .await?;
    writer
        .send(ServerEvent::Hardware(state.hardware_status()))
        .await?;

    let (event_tx, event_rx) = crate::outbound::channel(EVENT_CHANNEL_CAPACITY);
    let writer_task = tokio::spawn(run_writer(writer, event_rx));

    let mut forwarders: HashMap<ChannelId, Arc<AtomicBool>> = HashMap::new();
    let result = command_loop(
        &mut reader,
        &state,
        &running,
        &event_tx,
        &mut forwarders,
        client_channels,
    )
    .await;

    for stop in forwarders.values() {
        stop.store(false, Ordering::Relaxed);
    }
    state.release_client_channels(client_channels);
    drop(event_tx);
    writer_task.abort();
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
    state: &DaemonState,
) -> Result<Option<String>> {
    let first = reader.next().await;
    let hello = if state.authentication_required() {
        match first {
            Some(Ok(ClientCommand::Authenticate { token }))
                if state.authenticate(token.as_str()) =>
            {
                reader.next().await
            }
            Some(Ok(_)) => {
                let _ = writer
                    .send(ServerEvent::Error {
                        message: "authentication required or invalid".to_string(),
                    })
                    .await;
                return Ok(None);
            }
            Some(Err(error)) => return Err(error.into()),
            None => return Ok(None),
        }
    } else {
        first
    };

    match hello {
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
                    message: "expected Hello after authentication".to_string(),
                })
                .await;
            Ok(None)
        }
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

async fn run_writer<W: tokio::io::AsyncWrite + Unpin>(
    mut writer: FramedWrite<W, MessageCodec<ServerEvent>>,
    mut events: EventReceiver,
) {
    while let Some(event) = events.recv().await {
        if !matches!(
            tokio::time::timeout(Duration::from_secs(5), writer.send(event)).await,
            Ok(Ok(()))
        ) {
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
    event_tx: &EventSender,
    forwarders: &mut HashMap<ChannelId, Arc<AtomicBool>>,
    client_channels: ez_proto::ClientChannels,
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
                        if !apply_command(cmd, state, event_tx, forwarders, client_channels).await {
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
                report_removed_channels(state, event_tx, forwarders).await;
                for status in state.recording_statuses().into_iter().filter(|status| {
                    status.channel_id < ez_proto::CLIENT_CHANNEL_START
                        || client_channels.contains(status.channel_id)
                        || forwarders.contains_key(&status.channel_id)
                }) {
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
    event_tx: &EventSender,
    forwarders: &mut HashMap<ChannelId, Arc<AtomicBool>>,
    client_channels: ez_proto::ClientChannels,
) -> bool {
    let controlled_channel = match &cmd {
        ClientCommand::SetDemodMode { channel_id, .. }
        | ClientCommand::SetVolume { channel_id, .. }
        | ClientCommand::SetSquelch { channel_id, .. }
        | ClientCommand::Retune { channel_id, .. }
        | ClientCommand::RemoveChannel { channel_id }
        | ClientCommand::FinalizeTelemetryProduct { channel_id }
        | ClientCommand::StartRecording { channel_id, .. }
        | ClientCommand::StopRecording { channel_id }
        | ClientCommand::StartTelemetryProduct { channel_id }
        | ClientCommand::BeginSpectrumObservation { channel_id, .. } => Some(*channel_id),
        _ => None,
    };
    if controlled_channel
        .is_some_and(|id| id >= ez_proto::CLIENT_CHANNEL_START && !client_channels.contains(id))
    {
        let message = "channel is owned by another connection".to_string();
        if let ClientCommand::SetSquelch {
            channel_id,
            request_id,
            ..
        } = cmd
        {
            let _ = event_tx
                .send(ServerEvent::Squelch {
                    channel_id,
                    request_id,
                    result: Err(message),
                })
                .await;
        } else {
            send_error(event_tx, message).await;
        }
        return true;
    }
    match cmd {
        ClientCommand::Authenticate { .. } => {
            send_error(
                event_tx,
                "Authenticate is only valid before Hello".to_string(),
            )
            .await;
        }
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
            let subscription = if channel_id >= ez_proto::CLIENT_CHANNEL_START
                && !client_channels.contains(channel_id)
            {
                state
                    .subscribe_existing(channel_id, channel.kind)
                    .map(|sub| (sub, false))
            } else {
                state.subscribe(channel)
            };
            match subscription {
                Ok((sub, _)) => spawn_forwarder(channel_id, sub, event_tx.clone(), forwarders),
                Err(e) => send_error(event_tx, e.to_string()).await,
            }
        }
        ClientCommand::Unsubscribe { channel_id } => {
            if let Some(stop) = forwarders.remove(&channel_id) {
                stop.store(false, Ordering::Relaxed);
            }
            event_tx.clear(channel_id);
        }
        ClientCommand::SetDemodMode {
            audio,
            channel_id,
            mode,
        } => {
            if let Err(e) = state.configure_audio(channel_id, mode, audio) {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::SetVolume { channel_id, level } => {
            if let Err(e) = state.set_volume(channel_id, level) {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::SetSquelch {
            channel_id,
            db,
            request_id,
        } => {
            let result = state
                .set_squelch(channel_id, db)
                .map_err(|error| error.to_string());
            let _ = event_tx
                .send(ServerEvent::Squelch {
                    channel_id,
                    request_id,
                    result,
                })
                .await;
        }
        ClientCommand::Retune {
            audio,
            channel_id,
            center_offset_hz,
            channel_sample_rate_hz,
        } => {
            if let Err(e) =
                state.retune_configured(channel_id, center_offset_hz, channel_sample_rate_hz, audio)
            {
                send_error(event_tx, e.to_string()).await;
            }
        }
        ClientCommand::RemoveChannel { channel_id } => match state.remove_channel(channel_id) {
            Ok(()) => {
                if let Some(stop) = forwarders.remove(&channel_id) {
                    stop.store(false, Ordering::Relaxed);
                }
                let _ = event_tx
                    .send(ServerEvent::ChannelRemoved { channel_id })
                    .await;
            }
            Err(e) => send_error(event_tx, e.to_string()).await,
        },
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
        ClientCommand::StartTelemetryProduct { channel_id } => {
            if let Err(error) = state.start_telemetry_product(channel_id) {
                send_error(event_tx, error.to_string()).await;
            }
        }
        ClientCommand::FinalizeTelemetryProduct { channel_id } => {
            if let Err(error) = state.finalize_telemetry_product(channel_id) {
                send_error(event_tx, error.to_string()).await;
            }
        }
        ClientCommand::BeginSpectrumObservation {
            channel_id,
            revision,
        } => {
            if let Err(error) = state.begin_spectrum_observation(channel_id, revision) {
                send_error(event_tx, error.to_string()).await;
            }
        }
        ClientCommand::Ping { nonce } => {
            let _ = event_tx.send(ServerEvent::Pong { nonce }).await;
        }
        ClientCommand::Detach => return false,
    }
    true
}

pub(crate) async fn report_removed_channels(
    state: &DaemonState,
    event_tx: &EventSender,
    forwarders: &mut HashMap<ChannelId, Arc<AtomicBool>>,
) {
    let removed: Vec<_> = forwarders
        .keys()
        .copied()
        .filter(|id| !state.channel_exists(*id))
        .collect();
    for channel_id in removed {
        if let Some(stop) = forwarders.remove(&channel_id) {
            stop.store(false, Ordering::Relaxed);
        }
        let _ = event_tx
            .send(ServerEvent::ChannelRemoved { channel_id })
            .await;
    }
}

async fn send_error(event_tx: &EventSender, message: String) {
    let _ = event_tx.send(ServerEvent::Error { message }).await;
}

/// Starts (or replaces) `channel_id`'s forwarder thread for this connection. Replacing
/// rather than leaking is what keeps a duplicate `Subscribe` for an id already active on
/// this same connection safe: the previous thread is told to stop before the new one
/// starts, instead of orphaning it with no reachable stop flag.
///
/// `pub(crate)` so `crate::web`'s data-plane WebSocket handler can spin up the same kind of
/// per-subscription forwarder against the same bounded/coalesced event mailbox.
pub(crate) fn spawn_forwarder(
    channel_id: ChannelId,
    sub: ChannelSubscription,
    event_tx: EventSender,
    forwarders: &mut HashMap<ChannelId, Arc<AtomicBool>>,
) {
    if let Some(previous) = forwarders.remove(&channel_id) {
        previous.store(false, Ordering::Relaxed);
    }
    let keep_running = match sub {
        ChannelSubscription::Spectrum(h) => {
            spawn_event_forwarder(channel_id, h, event_tx, ServerEvent::Spectrum)
        }
        ChannelSubscription::Audio(h) => {
            spawn_event_forwarder(channel_id, h, event_tx, ServerEvent::Audio)
        }
        ChannelSubscription::AdsbPackets(h) => {
            spawn_event_forwarder(channel_id, h, event_tx, move |mut snapshot| {
                snapshot.channel_id = channel_id;
                ServerEvent::Aircraft(snapshot)
            })
        }
        ChannelSubscription::LrptTelemetry(h) => {
            spawn_event_forwarder(channel_id, h, event_tx, ServerEvent::Telemetry)
        }
    };
    forwarders.insert(channel_id, keep_running);
}

/// Stream forwarders never wait for the socket or the control queue. The
/// mailbox replaces old snapshots, bounds audio latency, and reports losses.
fn spawn_event_forwarder<T: Send + 'static>(
    channel_id: ChannelId,
    handle: BroadcasterHandle<T>,
    tx: EventSender,
    wrap: impl Fn(T) -> ServerEvent + Send + 'static,
) -> Arc<AtomicBool> {
    let keep_running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&keep_running);
    std::thread::Builder::new()
        .name(format!("ez-daemon-fwd-{channel_id}"))
        .spawn(move || {
            let mut previous_drops = 0;
            while thread_running.load(Ordering::Relaxed) && !tx.is_closed() {
                if let Some(item) = handle.recv_timeout(FORWARD_POLL) {
                    if !thread_running.load(Ordering::Relaxed) || tx.is_closed() {
                        break;
                    }
                    let drops = handle.dropped_count();
                    tx.note_gap(channel_id, drops.saturating_sub(previous_drops));
                    previous_drops = drops;
                    tx.stream(wrap(item));
                }
            }
        })
        .expect("spawning forwarder thread");
    keep_running
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
        let (addr, running, _) = spawn_test_server_with_auth(None).await;
        (addr, running)
    }

    async fn spawn_test_server_with_auth(
        auth_token: Option<String>,
    ) -> (SocketAddr, Arc<AtomicBool>, Arc<DaemonState>) {
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
        state.set_auth_token(auth_token).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        let server_state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = serve(listener, server_state, server_running).await;
        });
        (addr, running, state)
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
            audio: None,
            meteor_profile: None,
            id,
            center_offset_hz: 0,
            channel_sample_rate_hz: 2_000_000,
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
    async fn configured_auth_rejects_missing_and_wrong_tokens_before_welcome_or_controls() {
        let token = "a test token longer than a password";
        let (addr, running, state) = spawn_test_server_with_auth(Some(token.to_string())).await;
        let initial_frequency = state.hardware_status().frequency_hz;

        for first in [
            ClientCommand::Hello {
                client_name: "missing-auth".into(),
                protocol_version: PROTOCOL_VERSION,
            },
            ClientCommand::Authenticate {
                token: ez_proto::AuthToken::new("incorrect token"),
            },
        ] {
            let (mut writer, mut reader) = connect(addr).await;
            writer.send(first).await.unwrap();
            let event = reader.next().await.unwrap().unwrap();
            assert!(matches!(event, ServerEvent::Error { .. }));
            assert!(reader.next().await.is_none());
        }
        assert_eq!(state.hardware_status().frequency_hz, initial_frequency);

        let (mut writer, mut reader) = connect(addr).await;
        writer
            .send(ClientCommand::Authenticate {
                token: ez_proto::AuthToken::new(token),
            })
            .await
            .unwrap();
        hello(&mut writer, "authenticated-client").await;
        assert!(matches!(
            reader.next().await.unwrap().unwrap(),
            ServerEvent::Welcome { .. }
        ));
        assert!(matches!(
            reader.next().await.unwrap().unwrap(),
            ServerEvent::Hardware(_)
        ));
        writer
            .send(ClientCommand::SetFrequency { hz: 104_000_000 })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.hardware_status().frequency_hz != 104_000_000 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("authenticated command should reach hardware");
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
    async fn authenticated_live_lrpt_stop_delivers_completion_before_unsubscribe() {
        let token = "tcp-live-lrpt-secret";
        let (addr, running, _) = spawn_test_server_with_auth(Some(token.into())).await;
        let stream = TcpStream::connect(addr).await.unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut writer =
            FramedWrite::new(write_half, MessageCodec::<ClientCommand>::for_commands());
        let mut reader = FramedRead::new(read_half, MessageCodec::<ServerEvent>::for_data());
        writer
            .send(ClientCommand::Authenticate {
                token: ez_proto::AuthToken::new(token),
            })
            .await
            .unwrap();
        hello(&mut writer, "lrpt-stop-client").await;
        let _ = reader.next().await.unwrap().unwrap();
        let _ = reader.next().await.unwrap().unwrap();
        let channel_id = 42;
        writer
            .send(ClientCommand::Subscribe {
                channel: ChannelSpec {
                    audio: None,
                    meteor_profile: Some(ez_proto::MeteorProfile::Oqpsk72),
                    id: channel_id,
                    center_offset_hz: 0,
                    channel_sample_rate_hz: 288_000,
                    kind: PipelineKind::LrptTelemetry,
                    demod_mode: None,
                },
            })
            .await
            .unwrap();
        writer
            .send(ClientCommand::FinalizeTelemetryProduct { channel_id })
            .await
            .unwrap();
        let completion = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let ServerEvent::Telemetry(frame) = reader.next().await.unwrap().unwrap() {
                    if frame.channel_id == channel_id && frame.product_complete {
                        return frame;
                    }
                }
            }
        })
        .await
        .expect("finalized LRPT product must arrive while still subscribed");
        assert_eq!((completion.width, completion.height), (0, 0));
        assert!(!completion.final_product);
        writer
            .send(ClientCommand::Unsubscribe { channel_id })
            .await
            .unwrap();
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

    #[tokio::test]
    async fn mixed_stream_saturation_keeps_control_delivery_independent() {
        use crate::broadcast::{Broadcaster, OverflowPolicy};
        let bus = SampleBus::new();
        let ingest_running = Arc::new(AtomicBool::new(true));
        let (hardware, ingest_thread) = ingest::spawn(
            Box::new(SyntheticSource::default()),
            bus.clone(),
            Arc::clone(&ingest_running),
        )
        .unwrap();
        let dir = unique_temp_dir("mixed-saturation");
        let state = Arc::new(DaemonState::new(
            bus,
            hardware,
            100_000_000,
            2_000_000,
            dir.clone(),
        ));
        let ids = state.allocate_client_channels().unwrap();
        let audio = Broadcaster::new();
        let spectrum = Broadcaster::new();
        let aircraft = Broadcaster::new();
        let (tx, mut rx) = crate::outbound::channel(8);
        let mut forwarders = HashMap::new();
        spawn_forwarder(
            ids.audio,
            ChannelSubscription::Audio(audio.subscribe(64, OverflowPolicy::DropOldest)),
            tx.clone(),
            &mut forwarders,
        );
        spawn_forwarder(
            ids.spectrum,
            ChannelSubscription::Spectrum(spectrum.subscribe(64, OverflowPolicy::DropOldest)),
            tx.clone(),
            &mut forwarders,
        );
        spawn_forwarder(
            ids.adsb,
            ChannelSubscription::AdsbPackets(aircraft.subscribe(64, OverflowPolicy::DropOldest)),
            tx.clone(),
            &mut forwarders,
        );
        for n in 0..1000 {
            audio.publish(ez_proto::AudioFrame {
                audio_clock: ez_proto::CaptureClock::new(1, n * 16, 48_000, None),
                clock: ez_proto::CaptureClock::new(1, n * 16, 48_000, None),
                sequence: n + 1,
                channel_id: ids.audio,
                sample_rate_hz: 48_000,
                channels: 1,
                samples: vec![0.2; 16],
                squelch: ez_proto::SquelchState {
                    db: -120.0,
                    revision: 0,
                },
            });
            spectrum.publish(ez_proto::SpectrumFrame {
                channel_id: ids.spectrum,
                center_hz: 100_000_000,
                sample_rate_hz: 2_000_000,
                bins: vec![n as f32; 16],
                timestamp_ms: n,
                observation_revision: 0,
                measurement_bins: None,
            });
            aircraft.publish(ez_proto::AircraftSnapshot {
                channel_id: ids.adsb,
                receiver: ez_proto::ReceiverSnapshot {
                    total_messages: n,
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tx.queued_frames() < 10 {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            apply_command(
                ClientCommand::Ping { nonce: 99 },
                &state,
                &tx,
                &mut forwarders,
                ids,
            ),
        )
        .await;
        assert!(
            result.is_ok(),
            "mixed data queues blocked Ping delivery without a data drain"
        );
        assert!(matches!(
            rx.recv().await,
            Some(ServerEvent::Pong { nonce: 99 })
        ));
        let mut last = [false; 3];
        let mut retained = 0;
        let mut skipped = 0;
        while !last.iter().all(|seen| *seen) {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                ServerEvent::Audio(frame) => {
                    retained += 1;
                    last[0] |= frame.sequence == 1000;
                }
                ServerEvent::Spectrum(frame) => {
                    retained += 1;
                    last[1] |= frame.timestamp_ms == 999;
                }
                ServerEvent::Aircraft(frame) => {
                    retained += 1;
                    last[2] |= frame.receiver.total_messages == 999;
                }
                ServerEvent::StreamGap { dropped_frames, .. } => skipped += dropped_frames,
                _ => unreachable!(),
            }
        }
        assert_eq!(retained + skipped, 3000);
        for stop in forwarders.values() {
            stop.store(false, Ordering::Relaxed);
        }
        ingest_running.store(false, Ordering::Relaxed);
        ingest_thread.join().unwrap();
        drop(state);
        std::fs::remove_dir_all(dir).ok();
        println!(
            "Mixed forwarders: 3000 updates = {retained} retained + {skipped} skipped; Ping independent of stalled data"
        );
    }

    #[tokio::test]
    async fn mixed_stream_wire_writer_prioritizes_controls_after_one_inflight_frame() {
        let (write, read) = tokio::io::duplex(64);
        let (tx, rx) = crate::outbound::channel(8);
        let writer = tokio::spawn(run_writer(
            FramedWrite::new(write, MessageCodec::for_data()),
            rx,
        ));
        let audio = |n: u64, count: usize| ez_proto::AudioFrame {
            audio_clock: ez_proto::CaptureClock::new(1, n * 16, 48_000, None),
            clock: ez_proto::CaptureClock::new(1, n * 16, 48_000, None),
            sequence: n + 1,
            channel_id: 1,
            sample_rate_hz: 48_000,
            channels: 1,
            samples: vec![0.25; count],
            squelch: ez_proto::SquelchState {
                db: -120.0,
                revision: 0,
            },
        };
        tx.stream(ServerEvent::Audio(audio(0, 4096)));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tx.queued_frames() != 0 {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        // The 16 KiB first frame cannot finish writing into a 64-byte peer.
        for n in 1..=1000 {
            tx.stream(ServerEvent::Audio(audio(n, 16)));
        }
        tx.send(ServerEvent::Pong { nonce: 17 }).await.unwrap();
        let mut reader = FramedRead::new(read, MessageCodec::<ServerEvent>::for_data());
        let first = tokio::time::timeout(Duration::from_secs(5), reader.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(first, ServerEvent::Audio(frame) if frame.sequence == 1 && frame.samples.len() == 4096)
        );
        assert!(matches!(
            reader.next().await.unwrap().unwrap(),
            ServerEvent::Pong { nonce: 17 }
        ));
        assert!(matches!(
            reader.next().await.unwrap().unwrap(),
            ServerEvent::StreamGap {
                channel_id: 1,
                dropped_frames: 992
            }
        ));
        for sequence in 994..=1001 {
            assert!(
                matches!(reader.next().await.unwrap().unwrap(), ServerEvent::Audio(frame) if frame.sequence == sequence)
            );
        }
        writer.abort();
        let _ = writer.await;
        println!(
            "Bincode writer: 64-byte stalled peer, one 16 KiB in-flight frame, then Pong before 992-frame gap and fresh eight-frame tail"
        );
    }

    #[tokio::test]
    async fn mixed_stream_stalled_writer_has_a_bounded_disconnect() {
        let (write, _read) = tokio::io::duplex(64);
        let (tx, rx) = crate::outbound::channel(8);
        let writer = tokio::spawn(run_writer(
            FramedWrite::new(write, MessageCodec::for_data()),
            rx,
        ));
        tx.stream(ServerEvent::Telemetry(ez_proto::TelemetryFrame {
            product_id: 1,
            channel_id: 1,
            apid: 0,
            width: 1024,
            height: 1,
            pixels: vec![0; 1024],
            line_times_unix_seconds: Vec::new(),
            final_product: false,
            product_complete: false,
            rs_ok: 0,
            rs_failed: 0,
            costas_locked: false,
            frame_locked: false,
        }));
        tokio::time::timeout(Duration::from_secs(7), writer)
            .await
            .expect("stalled socket must not retain its writer indefinitely")
            .unwrap();
        assert!(tx.is_closed());
        println!("Stalled socket writer closes after the five-second write deadline");
    }
}
