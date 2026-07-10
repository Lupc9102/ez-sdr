//! WebSocket transports: a JSON **control** channel at `/ws/control` mirroring the TCP
//! control plane's command/event semantics one-for-one, and a binary **data-plane** channel
//! at `/ws/stream/{kind}/{id}` for high-rate spectrum/audio/telemetry frames.
//!
//! Both reuse `crate::server`'s existing primitives rather than re-implementing connection
//! handling: `apply_command` drives `ClientCommand`s identically to the TCP path, and
//! `spawn_forward_thread` bridges a pipeline's `BroadcasterHandle` to this connection's
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
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use ez_proto::{ChannelId, ClientCommand, PipelineKind, ServerEvent, PROTOCOL_VERSION};

use crate::server::{apply_command, spawn_forward_thread};
use crate::state::{ChannelSubscription, DaemonState};
use crate::web::wire;

const CONTROL_EVENT_CHANNEL_CAPACITY: usize = 256;
const CONTROL_HW_TICK: Duration = Duration::from_millis(250);
const DATA_CHANNEL_CAPACITY: usize = 64;

/// Assembles the `/ws/*` routes. `Router<Arc<DaemonState>>`, same reasoning as
/// `crate::web::api::router` — left unattached so `crate::web::serve` can merge this with
/// `api::router()` before a single `.with_state(...)`.
pub fn router() -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/ws/control", get(control_ws))
        .route("/ws/stream/{kind}/{id}", get(data_plane_ws))
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
    Path((kind, id)): Path<(String, ChannelId)>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(kind) = parse_pipeline_kind(&kind) else {
        return (StatusCode::BAD_REQUEST, format!("unknown pipeline kind {kind:?}")).into_response();
    };
    match state.subscribe_existing(id, kind) {
        Ok(sub) => ws.on_upgrade(move |socket| stream_channel(socket, id, sub)),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}

/// Streams one channel's pipeline output as binary (or, for `AdsbPackets`, JSON text) frames
/// until the client closes the socket. One forwarder OS thread bridges the pipeline's
/// `BroadcasterHandle` into an mpsc channel a plain async task drains into the WS sink —
/// identical shape to the TCP path's `spawn_forwarder`/`run_writer` split, just one
/// subscription per connection instead of a table of them (this route names exactly one
/// channel in its URL).
async fn stream_channel(socket: WebSocket, channel_id: ChannelId, sub: ChannelSubscription) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Message>(DATA_CHANNEL_CAPACITY);

    let keep_running = match sub {
        ChannelSubscription::Spectrum(h) => spawn_forward_thread(channel_id, h, tx.clone(), |f| {
            Message::Binary(wire::encode_spectrum(&f).into())
        }),
        ChannelSubscription::Audio(h) => spawn_forward_thread(channel_id, h, tx.clone(), |f| {
            Message::Binary(wire::encode_audio(&f).into())
        }),
        ChannelSubscription::AdsbPackets(h) => spawn_forward_thread(channel_id, h, tx.clone(), |f| {
            Message::Text(serde_json::to_string(&f).unwrap_or_default().into())
        }),
        ChannelSubscription::LrptTelemetry(h) => spawn_forward_thread(channel_id, h, tx.clone(), |f| {
            Message::Binary(wire::encode_telemetry(&f).into())
        }),
    };
    drop(tx);

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Close(_)) | Err(_) => break,
            // Data plane is one-way (server -> browser); pings/pongs are handled by axum
            // itself, and any other client-sent frame is simply not part of this protocol.
            Ok(_) => {}
        }
    }

    keep_running.store(false, Ordering::Relaxed);
    let _ = writer.await;
}

/// `GET /ws/control`: JSON-over-WebSocket mirror of the TCP control connection
/// (`crate::server::handle_connection`), minus the `Hello`/`Welcome` version handshake — the
/// HTTP upgrade already establishes the connection, so a `Welcome` event is sent immediately
/// instead of waiting on a first client message.
async fn control_ws(State(state): State<Arc<DaemonState>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| control_connection(socket, state))
}

async fn control_connection(socket: WebSocket, state: Arc<DaemonState>) {
    let (mut sink, mut stream) = socket.split();
    let (event_tx, mut event_rx) = mpsc::channel::<ServerEvent>(CONTROL_EVENT_CHANNEL_CAPACITY);

    if event_tx
        .send(ServerEvent::Welcome {
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
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    let mut forwarders: HashMap<ChannelId, Arc<AtomicBool>> = HashMap::new();
    let mut hw_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + CONTROL_HW_TICK,
        CONTROL_HW_TICK,
    );

    loop {
        tokio::select! {
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientCommand>(text.as_str()) {
                            Ok(cmd) => {
                                if !apply_command(cmd, &state, &event_tx, &mut forwarders).await {
                                    break;
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
            }
        }
    }

    for stop in forwarders.values() {
        stop.store(false, Ordering::Relaxed);
    }
    drop(event_tx);
    let _ = writer.await;
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
        std::env::temp_dir().join(format!("ez-daemon-ws-test-{label}-{}-{n}", std::process::id()))
    }

    async fn spawn_test_ws() -> (String, Arc<DaemonState>) {
        let bus = SampleBus::new();
        let running = Arc::new(StdAtomicBool::new(true));
        let (hardware, _ingest_thread) =
            ingest::spawn(Box::new(SyntheticSource::default()), bus.clone(), running).unwrap();
        let state = Arc::new(DaemonState::new(
            bus,
            hardware,
            100_000_000,
            2_000_000,
            unique_temp_dir("state"),
        ));

        let app = crate::web::api::router()
            .merge(router())
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service()).await.unwrap();
        });
        (format!("ws://{addr}"), state)
    }

    #[tokio::test]
    async fn control_ws_sends_welcome_then_hardware_and_answers_ping() {
        let (base, _state) = spawn_test_ws().await;
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
        ws.send(TMessage::Text(serde_json::to_string(&cmd).unwrap().into()))
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
        let (base, state) = spawn_test_ws().await;
        let (mut ws, _resp) = connect_async(format!("{base}/ws/control")).await.unwrap();
        let _welcome = ws.next().await.unwrap().unwrap();
        let _hardware = ws.next().await.unwrap().unwrap();

        let cmd = ClientCommand::SetFrequency { hz: 105_500_000 };
        ws.send(TMessage::Text(serde_json::to_string(&cmd).unwrap().into()))
            .await
            .unwrap();

        // apply_command runs synchronously against DaemonState before returning, but there's
        // no ack event for SetFrequency, so poll the state directly rather than the socket.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if state.hardware_status().frequency_hz == 105_500_000 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "frequency never updated");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn data_plane_ws_rejects_unknown_channel_without_upgrading() {
        let (base, _state) = spawn_test_ws().await;
        let url = format!("{base}/ws/stream/spectrum/999");
        let result = connect_async(url).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn data_plane_ws_rejects_unknown_kind() {
        let (base, state) = spawn_test_ws().await;
        state
            .subscribe(ChannelSpec {
                id: 1,
                center_offset_hz: 0,
                bandwidth_hz: 2_000_000,
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
        let (base, state) = spawn_test_ws().await;
        state
            .subscribe(ChannelSpec {
                id: 1,
                center_offset_hz: 0,
                bandwidth_hz: 2_000_000,
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
        let (base, state) = spawn_test_ws().await;
        state
            .subscribe(ChannelSpec {
                id: 5,
                center_offset_hz: 10_000,
                bandwidth_hz: 200_000,
                kind: PipelineKind::Audio,
                demod_mode: Some(ez_proto::DemodMode::Fm),
            })
            .unwrap();

        let url = format!("{base}/ws/stream/spectrum/5");
        let result = connect_async(url).await;
        assert!(result.is_err());
    }
}
