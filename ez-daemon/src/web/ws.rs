//! WebSocket transports: a JSON **control** channel at `/ws/control` mirroring the TCP
//! control plane's command/event semantics one-for-one, and a binary **data-plane** channel
//! at `/ws/stream/{kind}/{id}` for high-rate spectrum/audio/telemetry frames.
//!
//! Both reuse `crate::server`'s existing primitives rather than re-implementing connection
//! handling: `apply_command` drives `ClientCommand`s identically to the TCP path, and
//! `spawn_forwarder` bridges a pipeline's `BroadcasterHandle` to this connection's
//! outbound channel exactly like the TCP forwarder does, just wrapping into a WS
//! [`Message`] instead of a bincode-framed [`ServerEvent`]. Keeping data-plane traffic on
//! its own route (rather than letting a client `Subscribe` over `/ws/control` the way a TCP
//! client can) is what keeps a browser's high-throughput spectrum/audio stream from ever
//! sharing a socket — or a slow-consumer stall — with its control commands.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use ez_proto::{ChannelId, ClientCommand, PipelineKind, ServerEvent, PROTOCOL_VERSION};

use crate::server::{apply_command, spawn_forwarder};
use crate::state::{ChannelSubscription, DaemonState};
use crate::web::browser_origin_allowed;
use crate::web::wire;

const CONTROL_EVENT_CHANNEL_CAPACITY: usize = 256;
const CONTROL_COMMAND_CHANNEL_CAPACITY: usize = 256;
const CONTROL_HW_TICK: Duration = Duration::from_millis(250);

/// Assembles the `/ws/*` routes. `Router<Arc<DaemonState>>`, same reasoning as
/// `crate::web::api::router` — left unattached so `crate::web::serve` can merge this with
/// `api::router()` before a single `.with_state(...)`.
pub fn router(running: Arc<AtomicBool>) -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/ws/control", get(control_ws))
        .route("/ws/stream/{kind}/{id}", get(data_plane_ws))
        .layer(Extension(running))
}

fn parse_pipeline_kind(s: &str) -> Option<PipelineKind> {
    match s {
        "spectrum" => Some(PipelineKind::Spectrum),
        "audio" => Some(PipelineKind::Audio),
        "adsb-packets" => Some(PipelineKind::AdsbPackets),
        "lrpt-telemetry" => Some(PipelineKind::LrptTelemetry),
        _ => None,
    }
}

/// `GET /ws/stream/{kind}/{id}`: binary data plane for an already-created channel. Validates
/// the kind/id pair *before* upgrading (via `subscribe_existing`, which errors rather than
/// creating) so a bad request gets a plain HTTP error instead of an upgrade the server would
/// immediately have to close.
async fn data_plane_ws(
    State(state): State<Arc<DaemonState>>,
    Extension(running): Extension<Arc<AtomicBool>>,
    Path((kind, id)): Path<(String, ChannelId)>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !browser_origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-origin WebSocket rejected").into_response();
    }
    let selected_protocol = match authorize_websocket(&state, &headers) {
        Ok(protocol) => protocol,
        Err(status) => return (status, "Authentication required").into_response(),
    };
    let Some(kind) = parse_pipeline_kind(&kind) else {
        return (
            StatusCode::BAD_REQUEST,
            format!("unknown pipeline kind {kind:?}"),
        )
            .into_response();
    };
    match state.subscribe_existing(id, kind) {
        Ok(sub) => {
            let ws = match selected_protocol {
                Some(protocol) => ws.protocols([protocol]),
                None => ws,
            };
            ws.on_upgrade(move |socket| stream_channel(socket, id, sub, state, running))
        }
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}

/// Streams one channel's pipeline output as binary (or, for `AdsbPackets`, JSON text) frames
/// until the client closes the socket. One forwarder OS thread bridges the pipeline's
/// `BroadcasterHandle` into bounded/coalesced stream slots drained into the WS sink —
/// identical shape to the TCP path's `spawn_forwarder`/`run_writer` split, just one
/// subscription per connection instead of a table of them (this route names exactly one
/// channel in its URL).
async fn stream_channel(
    socket: WebSocket,
    channel_id: ChannelId,
    sub: ChannelSubscription,
    state: Arc<DaemonState>,
    running: Arc<AtomicBool>,
) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = crate::outbound::channel(CONTROL_EVENT_CHANNEL_CAPACITY);
    let mut forwarders = HashMap::new();
    spawn_forwarder(channel_id, sub, tx.clone(), &mut forwarders);
    let keep_running = forwarders.remove(&channel_id).unwrap();
    drop(tx);

    let writer_running = Arc::clone(&running);
    let mut writer = tokio::spawn(async move {
        let mut lifecycle = tokio::time::interval(CONTROL_HW_TICK);
        loop {
            tokio::select! {
                _ = wait_for_stop(Arc::clone(&writer_running)) => break,
                _ = lifecycle.tick() => {
                    if !state.channel_exists(channel_id) {
                        let removed = ServerEvent::ChannelRemoved { channel_id };
                        let _ = sink.send(Message::Text(serde_json::to_string(&removed).unwrap().into())).await;
                        let _ = sink.send(Message::Close(None)).await;
                        break;
                    }
                }
                msg = rx.recv() => {
                    let Some(msg) = msg else { break; };
                    if !matches!(tokio::time::timeout(Duration::from_secs(5), sink.send(stream_message(msg))).await, Ok(Ok(()))) { break; }
                }
            }
        }
    });

    loop {
        let msg = tokio::select! {
            _ = &mut writer => { keep_running.store(false, Ordering::Relaxed); return; }
            _ = wait_for_stop(Arc::clone(&running)) => break,
            msg = stream.next() => { let Some(msg) = msg else { break; }; msg }
        };
        match msg {
            Ok(Message::Close(_)) | Err(_) => break,
            // Data plane is one-way (server -> browser); pings/pongs are handled by axum
            // itself, and any other client-sent frame is simply not part of this protocol.
            Ok(_) => {}
        }
    }

    keep_running.store(false, Ordering::Relaxed);
    writer.abort();
    let _ = writer.await;
}

/// StreamGap is a JSON text notice alongside the unchanged binary data layouts.
/// Consumers can display omissions and reset playback when audio was skipped.
fn stream_message(event: ServerEvent) -> Message {
    match event {
        ServerEvent::Spectrum(frame) => Message::Binary(wire::encode_spectrum(&frame).into()),
        ServerEvent::Audio(frame) => Message::Binary(wire::encode_audio(&frame).into()),
        ServerEvent::Aircraft(frame) => {
            Message::Text(serde_json::to_string(&frame).unwrap().into())
        }
        ServerEvent::Telemetry(frame) => Message::Binary(wire::encode_telemetry(&frame).into()),
        event => Message::Text(serde_json::to_string(&event).unwrap().into()),
    }
}

/// `GET /ws/control`: JSON-over-WebSocket mirror of the TCP control connection
/// (`crate::server::handle_connection`), minus the `Hello`/`Welcome` version handshake — the
/// HTTP upgrade already establishes the connection, so a `Welcome` event is sent immediately
/// instead of waiting on a first client message.
async fn control_ws(
    State(state): State<Arc<DaemonState>>,
    Extension(running): Extension<Arc<AtomicBool>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !browser_origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-origin WebSocket rejected").into_response();
    }
    let selected_protocol = match authorize_websocket(&state, &headers) {
        Ok(protocol) => protocol,
        Err(status) => return (status, "Authentication required").into_response(),
    };
    let ws = match selected_protocol {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.on_upgrade(move |socket| control_connection(socket, state, running))
}

async fn wait_for_stop(running: Arc<AtomicBool>) {
    while running.load(Ordering::Acquire) {
        tokio::time::sleep(CONTROL_HW_TICK).await;
    }
}

/// Browser WebSocket APIs cannot set `Authorization`, so configured credentials may be
/// offered as the `ez-sdr-auth.<lowercase-hex-token>` subprotocol. Hex encoding keeps
/// arbitrary UTF-8 tokens inside RFC WebSocket subprotocol syntax. The successful protocol
/// is echoed by the handshake, while arbitrary client-selected protocols are never accepted.
fn authorize_websocket(
    state: &DaemonState,
    headers: &HeaderMap,
) -> Result<Option<String>, StatusCode> {
    if !state.authentication_required() {
        return Ok(None);
    }
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, token)| scheme.eq_ignore_ascii_case("bearer") && !token.is_empty())
        .map(|(_, token)| token)
    {
        if state.authenticate(token) {
            return Ok(None);
        }
    }

    let protocols = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim);
    for protocol in protocols {
        if let Some(encoded) = protocol.strip_prefix("ez-sdr-auth.") {
            if let Some(token) = decode_hex(encoded) {
                let Ok(token) = std::str::from_utf8(&token) else {
                    continue;
                };
                if !state.authenticate(token) {
                    continue;
                }
                return Ok(Some(protocol.to_string()));
            }
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            Some(nibble(pair[0])? * 16 + nibble(pair[1])?)
        })
        .collect()
}

async fn control_connection(socket: WebSocket, state: Arc<DaemonState>, running: Arc<AtomicBool>) {
    let Ok(client_channels) = state.allocate_client_channels() else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let (event_tx, mut event_rx) = crate::outbound::channel(CONTROL_EVENT_CHANNEL_CAPACITY);

    if event_tx
        .send(ServerEvent::Welcome {
            client_channels,
            protocol_version: PROTOCOL_VERSION,
            active_channels: state.active_channels(),
        })
        .await
        .is_err()
    {
        return;
    }
    if event_tx
        .send(ServerEvent::Hardware(state.hardware_status()))
        .await
        .is_err()
    {
        return;
    }

    let writer = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let Ok(text) = serde_json::to_string(&event) else {
                continue;
            };
            if !matches!(
                tokio::time::timeout(
                    Duration::from_secs(5),
                    sink.send(Message::Text(text.into()))
                )
                .await,
                Ok(Ok(()))
            ) {
                break;
            }
        }
    });

    let (command_tx, mut command_rx) =
        mpsc::channel::<ClientCommand>(CONTROL_COMMAND_CHANNEL_CAPACITY);
    let command_state = Arc::clone(&state);
    let command_events = event_tx.clone();
    let command_worker = tokio::spawn(async move {
        let mut forwarders: HashMap<ChannelId, Arc<AtomicBool>> = HashMap::new();
        let mut deferred = None;

        loop {
            let mut command = match deferred.take() {
                Some(command) => command,
                None => match tokio::time::timeout(CONTROL_HW_TICK, command_rx.recv()).await {
                    Err(_) => {
                        crate::server::report_removed_channels(
                            &command_state,
                            &command_events,
                            &mut forwarders,
                        )
                        .await;
                        continue;
                    }
                    Ok(command) => match command {
                        Some(command) => command,
                        None => break,
                    },
                },
            };

            while let Ok(next) = command_rx.try_recv() {
                if let Some(next) = coalesce_hardware_command(&mut command, next) {
                    deferred = Some(next);
                    break;
                }
            }

            if !apply_command(
                command,
                &command_state,
                &command_events,
                &mut forwarders,
                client_channels,
            )
            .await
            {
                break;
            }
        }

        for stop in forwarders.values() {
            stop.store(false, Ordering::Relaxed);
        }
        command_state.release_client_channels(client_channels);
    });

    let mut hw_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + CONTROL_HW_TICK,
        CONTROL_HW_TICK,
    );

    loop {
        tokio::select! {
            _ = wait_for_stop(Arc::clone(&running)) => break,
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientCommand>(text.as_str()) {
                            Ok(ClientCommand::Ping { nonce }) => {
                                if event_tx.send(ServerEvent::Pong { nonce }).await.is_err() {
                                    break;
                                }
                            }
                            Ok(ClientCommand::Detach) => break,
                            Ok(cmd) => {
                                if let Err(error) = command_tx.try_send(cmd) {
                                    let message = match error {
                                        mpsc::error::TrySendError::Full(_) => {
                                            "control command queue is full; try again".to_string()
                                        }
                                        mpsc::error::TrySendError::Closed(_) => break,
                                    };
                                    if event_tx.send(ServerEvent::Error { message }).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = event_tx
                                    .send(ServerEvent::Error {
                                        message: format!("invalid command: {e}"),
                                    })
                                    .await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            _ = hw_tick.tick() => {
                if event_tx.send(ServerEvent::Hardware(state.hardware_status())).await.is_err() {
                    break;
                }
                let mut disconnected = false;
                for status in state.recording_statuses().into_iter().filter(|status| {
                    status.channel_id < ez_proto::CLIENT_CHANNEL_START || client_channels.contains(status.channel_id)
                }) {
                    if event_tx.send(ServerEvent::Recording(status)).await.is_err() {
                        disconnected = true;
                        break;
                    }
                }
                if disconnected {
                    break;
                }
            }
        }
    }

    drop(command_tx);
    let _ = command_worker.await;
    drop(event_tx);
    writer.abort();
    let _ = writer.await;
}

fn coalesce_hardware_command(
    current: &mut ClientCommand,
    next: ClientCommand,
) -> Option<ClientCommand> {
    let same_control = matches!(
        (&*current, &next),
        (
            ClientCommand::SetFrequency { .. },
            ClientCommand::SetFrequency { .. }
        ) | (
            ClientCommand::SetSampleRate { .. },
            ClientCommand::SetSampleRate { .. }
        ) | (ClientCommand::SetGain { .. }, ClientCommand::SetGain { .. })
    );
    if same_control {
        *current = next;
        None
    } else {
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SampleBus;
    use crate::hardware::synthetic::SyntheticSource;
    use crate::ingest;
    use ez_proto::ChannelSpec;
    use std::sync::atomic::{AtomicBool as StdAtomicBool, AtomicU64, Ordering as StdOrdering};
    use tokio::net::TcpListener;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as TMessage;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let n = TEST_DIR_COUNTER.fetch_add(1, StdOrdering::Relaxed);
        std::env::temp_dir().join(format!(
            "ez-daemon-ws-test-{label}-{}-{n}",
            std::process::id()
        ))
    }

    struct TestWsGuard {
        running: Arc<StdAtomicBool>,
        server: Option<tokio::task::JoinHandle<()>>,
        ingest_thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for TestWsGuard {
        fn drop(&mut self) {
            self.running.store(false, StdOrdering::Release);
            if let Some(server) = self.server.take() {
                server.abort();
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn(async move {
                        let _ = server.await;
                    });
                }
            }
            if let Some(ingest_thread) = self.ingest_thread.take() {
                match tokio::runtime::Handle::try_current() {
                    Ok(runtime) => {
                        runtime.spawn_blocking(move || {
                            let _ = ingest_thread.join();
                        });
                    }
                    Err(_) => {
                        let _ = std::thread::Builder::new()
                            .name("ez-daemon-ws-test-ingest-reaper".into())
                            .spawn(move || {
                                let _ = ingest_thread.join();
                            });
                    }
                }
            }
        }
    }

    async fn spawn_test_ws() -> (String, Arc<DaemonState>, TestWsGuard) {
        let bus = SampleBus::new();
        let running = Arc::new(StdAtomicBool::new(true));
        let (hardware, ingest_thread) = ingest::spawn(
            Box::new(SyntheticSource::default()),
            bus.clone(),
            Arc::clone(&running),
        )
        .unwrap();
        let state = Arc::new(DaemonState::new(
            bus,
            hardware,
            100_000_000,
            2_000_000,
            unique_temp_dir("state"),
        ));
        let app = crate::web::api::router()
            .merge(router(Arc::clone(&running)))
            .layer(axum::middleware::from_fn_with_state(
                Arc::clone(&state),
                crate::web::transport_request_guard,
            ))
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_running = Arc::clone(&running);
        let server = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(wait_for_stop(server_running))
                .await
                .unwrap();
        });
        (
            format!("ws://{addr}"),
            state,
            TestWsGuard {
                running,
                server: Some(server),
                ingest_thread: Some(ingest_thread),
            },
        )
    }

    #[test]
    fn browser_origins_require_loopback_or_same_numeric_endpoint() {
        use axum::http::header::{HOST, ORIGIN};
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "192.0.2.10:7891".parse().unwrap());
        assert!(browser_origin_allowed(&headers)); // Native client.
        for origin in [
            "http://localhost:9000",
            "https://127.0.0.1",
            "http://[::1]:7891",
            "http://192.0.2.10:7891",
        ] {
            headers.insert(ORIGIN, origin.parse().unwrap());
            assert!(browser_origin_allowed(&headers), "{origin}");
        }
        for origin in [
            "https://foreign.example",
            "http://localhost.evil.example",
            "http://localhost@foreign.example",
            "http://192.0.2.10:9000",
            "http://localhost:99999",
            "null",
            "file://localhost",
            "http://localhost/path",
            "http://0.0.0.0",
        ] {
            headers.insert(ORIGIN, origin.parse().unwrap());
            assert!(!browser_origin_allowed(&headers), "{origin}");
        }
    }

    #[tokio::test]
    async fn foreign_browser_cannot_upgrade_control_or_data_websocket() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let (base, state, _server_guard) = spawn_test_ws().await;
        let initial = state.hardware_status().frequency_hz;
        for path in ["/ws/control", "/ws/stream/spectrum/77"] {
            let mut request = format!("{base}{path}").into_client_request().unwrap();
            request
                .headers_mut()
                .insert("Origin", "https://foreign.example".parse().unwrap());
            let error = connect_async(request).await.unwrap_err();
            let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
                panic!("expected HTTP rejection: {error}");
            };
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        assert_eq!(state.hardware_status().frequency_hz, initial);
        let mut request = format!("{base}/ws/control").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", "http://localhost:9000".parse().unwrap());
        let (mut socket, _) = connect_async(request).await.unwrap();
        socket.close(None).await.unwrap();
    }

    #[tokio::test]
    async fn configured_auth_protects_rest_and_both_websocket_planes() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let (base, state, _server_guard) = spawn_test_ws().await;
        state
            .set_auth_token(Some("test-token-which-is-long-enough".into()))
            .unwrap();
        let initial_frequency = state.hardware_status().frequency_hz;
        let initial_channels = state.channel_metrics().len();

        let response = reqwest::get(format!("{}/api/status", base.replace("ws://", "http://")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/api/status", base.replace("ws://", "http://")))
            .bearer_auth("incorrect token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = client
            .post(format!(
                "{}/api/hardware/frequency",
                base.replace("ws://", "http://")
            ))
            .json(&serde_json::json!({"hz": 109_000_000}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        for path in ["/ws/control", "/ws/stream/spectrum/77"] {
            let request = format!("{base}{path}").into_client_request().unwrap();
            let error = connect_async(request).await.unwrap_err();
            let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
                panic!("expected HTTP rejection: {error}");
            };
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let mut request = format!("{base}/ws/control").into_client_request().unwrap();
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            "ez-sdr-auth.696e636f727265637420746f6b656e"
                .parse()
                .unwrap(),
        );
        let error = connect_async(request).await.unwrap_err();
        let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
            panic!("expected HTTP rejection for wrong token: {error}");
        };
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(state.hardware_status().frequency_hz, initial_frequency);
        assert_eq!(state.channel_metrics().len(), initial_channels);

        let mut request = format!("{base}/ws/control").into_client_request().unwrap();
        let auth_protocol = format!(
            "ez-sdr-auth.{}",
            "test-token-which-is-long-enough"
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", auth_protocol.parse().unwrap());
        let (mut socket, response) = connect_async(request).await.unwrap();
        assert_eq!(response.headers()["Sec-WebSocket-Protocol"], auth_protocol);
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            TMessage::Text(_)
        ));
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            TMessage::Text(_)
        ));

        let response = client
            .post(format!(
                "{}/api/hardware/frequency",
                base.replace("ws://", "http://")
            ))
            .bearer_auth("test-token-which-is-long-enough")
            .json(&serde_json::json!({"hz":109_000_000}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(state.hardware_status().frequency_hz, 109_000_000);
        socket.close(None).await.unwrap();

        state
            .subscribe(ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 77,
                center_offset_hz: 0,
                channel_sample_rate_hz: 2_000_000,
                kind: PipelineKind::Spectrum,
                demod_mode: None,
            })
            .unwrap();
        let mut request = format!("{base}/ws/stream/spectrum/77")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", auth_protocol.parse().unwrap());
        let (mut data, _) = connect_async(request).await.unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(Ok(TMessage::Binary(frame))) = data.next().await {
                    break frame;
                }
            }
        })
        .await
        .expect("authenticated data websocket should stream frames");
        assert!(frame.len() >= 24);
        data.close(None).await.unwrap();
    }

    #[test]
    fn hardware_command_coalescing_keeps_latest_matching_value() {
        let mut current = ClientCommand::SetFrequency { hz: 100_000_000 };
        assert!(coalesce_hardware_command(
            &mut current,
            ClientCommand::SetFrequency { hz: 101_000_000 }
        )
        .is_none());
        assert!(matches!(
            current,
            ClientCommand::SetFrequency { hz: 101_000_000 }
        ));

        let deferred = coalesce_hardware_command(&mut current, ClientCommand::SetGain { db: 12.0 });
        assert!(matches!(
            deferred,
            Some(ClientCommand::SetGain { db: 12.0 })
        ));
    }

    #[tokio::test]
    async fn control_ws_sends_welcome_then_hardware_and_answers_ping() {
        let (base, _state, _server_guard) = spawn_test_ws().await;
        let (mut ws, _resp) = connect_async(format!("{base}/ws/control")).await.unwrap();

        let welcome = ws.next().await.unwrap().unwrap();
        let TMessage::Text(text) = welcome else {
            panic!("expected text frame")
        };
        let event: ServerEvent = serde_json::from_str(&text).unwrap();
        assert!(matches!(event, ServerEvent::Welcome { .. }));

        let hardware = ws.next().await.unwrap().unwrap();
        let TMessage::Text(text) = hardware else {
            panic!("expected text frame")
        };
        let event: ServerEvent = serde_json::from_str(&text).unwrap();
        assert!(matches!(event, ServerEvent::Hardware(_)));

        let cmd = ClientCommand::Ping { nonce: 42 };
        ws.send(TMessage::Text(serde_json::to_string(&cmd).unwrap()))
            .await
            .unwrap();

        loop {
            let msg = ws.next().await.unwrap().unwrap();
            let TMessage::Text(text) = msg else { continue };
            let event: ServerEvent = serde_json::from_str(&text).unwrap();
            if let ServerEvent::Pong { nonce } = event {
                assert_eq!(nonce, 42);
                break;
            }
        }
    }

    #[tokio::test]
    async fn control_ws_set_frequency_is_reflected_in_hardware_status() {
        let (base, state, _server_guard) = spawn_test_ws().await;
        let (mut ws, _resp) = connect_async(format!("{base}/ws/control")).await.unwrap();
        let _welcome = ws.next().await.unwrap().unwrap();
        let _hardware = ws.next().await.unwrap().unwrap();

        let cmd = ClientCommand::SetFrequency { hz: 105_500_000 };
        ws.send(TMessage::Text(serde_json::to_string(&cmd).unwrap()))
            .await
            .unwrap();

        // apply_command runs synchronously against DaemonState before returning, but there's
        // no ack event for SetFrequency, so poll the state directly rather than the socket.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if state.hardware_status().frequency_hz == 105_500_000 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "frequency never updated"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn data_plane_ws_rejects_unknown_channel_without_upgrading() {
        let (base, _state, _server_guard) = spawn_test_ws().await;
        let url = format!("{base}/ws/stream/spectrum/999");
        let result = connect_async(url).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn data_plane_ws_rejects_unknown_kind() {
        let (base, state, _server_guard) = spawn_test_ws().await;
        state
            .subscribe(ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 1,
                center_offset_hz: 0,
                channel_sample_rate_hz: 2_000_000,
                kind: PipelineKind::Spectrum,
                demod_mode: None,
            })
            .unwrap();
        let url = format!("{base}/ws/stream/not-a-kind/1");
        let result = connect_async(url).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn data_plane_ws_streams_binary_spectrum_frames_for_an_existing_channel() {
        let (base, state, _server_guard) = spawn_test_ws().await;
        state
            .subscribe(ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 1,
                center_offset_hz: 0,
                channel_sample_rate_hz: 2_000_000,
                kind: PipelineKind::Spectrum,
                demod_mode: None,
            })
            .unwrap();

        let url = format!("{base}/ws/stream/spectrum/1");
        let (mut ws, _resp) = connect_async(url).await.unwrap();

        loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("timed out waiting for a spectrum frame")
                .unwrap()
                .unwrap();
            if let TMessage::Binary(bytes) = msg {
                assert!(bytes.len() >= 24, "spectrum frame header truncated");
                // Spectrum frames report the wideband source's own center frequency (the
                // synthetic source's fixed default), not `DaemonState::new`'s
                // `wideband_center_hz` argument (which only seeds the channelizer's initial
                // NCO reference) — mirrors the same distinction `crate::server`'s own
                // `subscribing_to_spectrum_streams_frames` test relies on.
                let center_hz = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                assert!(center_hz > 0, "expected a nonzero center frequency");
                break;
            }
        }
    }

    #[tokio::test]
    async fn data_plane_ws_mismatched_kind_for_existing_channel_is_rejected() {
        let (base, state, _server_guard) = spawn_test_ws().await;
        state
            .subscribe(ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: 5,
                center_offset_hz: 10_000,
                channel_sample_rate_hz: 200_000,
                kind: PipelineKind::Audio,
                demod_mode: Some(ez_proto::DemodMode::Fm),
            })
            .unwrap();

        let url = format!("{base}/ws/stream/spectrum/5");
        let result = connect_async(url).await;
        assert!(result.is_err());
    }
}
