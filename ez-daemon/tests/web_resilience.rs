//! Integration tests for the web transport's connection resilience under simulated
//! high-throughput processing load (Definition of Done #3/#4) — the HTTP/WebSocket analogue
//! of `tests/resilience.rs`.
//!
//! These drive a fully wired daemon through its public entry point [`ez_daemon::run`] over
//! loopback HTTP/WebSocket, exactly as a browser client would: `POST /api/channels` to create
//! a pipeline, `GET /ws/stream/{kind}/{id}` for the binary data plane, `GET /ws/control` for
//! the JSON control plane. They assert the same property the headless architecture exists to
//! guarantee, now for the web surface: **browser connection volatility (churn, stalled
//! readers, command floods, hard drops, mass concurrency) never stalls or corrupts daemon-side
//! processing.**

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as TMessage;

use ez_daemon::{run, DaemonConfig, SourceConfig};
use ez_proto::{ClientCommand, ServerEvent};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A running daemon under test, plus the addresses to reach it and the flag to stop it.
struct Harness {
    #[allow(dead_code)]
    addr: SocketAddr,
    web_addr: SocketAddr,
    running: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Harness {
    /// Boots a synthetic-source daemon on ephemeral loopback ports. Synthetic ingestion is
    /// deterministic and hardware-free, so the spectrum pipeline always produces frames —
    /// exactly the steady processing load these tests need to run *underneath* browser churn.
    async fn spawn(label: &str) -> Self {
        let addr = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let web_addr = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let recording_dir = std::env::temp_dir().join(format!(
            "ez-daemon-web-it-{label}-{}-{n}",
            std::process::id()
        ));
        let config = DaemonConfig {
            listen_addr: addr,
            web_listen_addr: web_addr,
            initial_freq_hz: 100_000_000,
            initial_sample_rate_hz: 2_000_000,
            recording_dir,
            web_static_dir: std::env::temp_dir().join("ez-daemon-web-it-nonexistent-static"),
            source: SourceConfig::Synthetic,
        };
        let running = Arc::new(AtomicBool::new(true));
        let run_running = Arc::clone(&running);
        let handle = tokio::spawn(async move { run(config, run_running).await });
        Self {
            addr,
            web_addr,
            running,
            handle,
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.web_addr)
    }

    fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{path}", self.web_addr)
    }

    /// Signals shutdown and asserts the daemon's top-level future returns `Ok` promptly —
    /// i.e. no WS connection task, forwarder thread, or ingestion join left it wedged.
    async fn shutdown(self) {
        self.running.store(false, Ordering::Relaxed);
        let result = tokio::time::timeout(Duration::from_secs(10), self.handle)
            .await
            .expect("daemon did not shut down within 10s of the stop signal");
        assert!(
            result.unwrap().is_ok(),
            "daemon returned an error on shutdown"
        );
    }
}

/// Retries `GET {base}/api/status` until it succeeds. `Harness::spawn` returns as soon as the
/// daemon task is *spawned*, not once the web listener is actually accepting — `run()` does
/// real setup (build the source, spawn ingestion, construct `DaemonState`, bind two listeners)
/// before either server can accept a connection, so the very first request in a test can
/// otherwise race that startup window and lose.
async fn wait_until_web_ready(base: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if reqwest::get(format!("{base}/api/status")).await.is_ok() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "web server never became ready"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn spectrum_spec(id: u32) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "center_offset_hz": 0,
        "bandwidth_hz": 2_000_000,
        "kind": "Spectrum",
        "demod_mode": null,
    })
}

/// Creates a spectrum channel over the REST API, mirroring how a browser dashboard would
/// before opening the corresponding data-plane WebSocket.
async fn create_spectrum_channel(base: &str, id: u32) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/channels"))
        .json(&spectrum_spec(id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "failed to create channel {id}");
}

/// Reads WS frames until a binary spectrum frame arrives (skipping anything else — the data
/// plane is spectrum-only binary frames in these tests, but being liberal here keeps the
/// helper robust), returning the frame's byte length so callers can sanity-check the payload.
async fn wait_for_binary_frame(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    what: &str,
) -> usize {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or_default();
        assert!(!remaining.is_zero(), "timed out waiting for {what}");
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(TMessage::Binary(bytes)))) => return bytes.len(),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("ws error while {what}: {e}"),
            Ok(None) => panic!("ws closed while {what}"),
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

/// A persistent, well-behaved data-plane subscriber must keep receiving spectrum frames while
/// dozens of other browser-style clients rapidly create a channel, open its WS stream, read a
/// frame, and disconnect. Core resilience guarantee for the web transport, mirroring
/// `resilience.rs`'s `persistent_subscriber_survives_rapid_attach_detach_churn` for TCP.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn persistent_ws_subscriber_survives_rapid_client_churn() {
    let daemon = Harness::spawn("web-churn").await;
    let base = daemon.base_url();
    wait_until_web_ready(&base).await;

    create_spectrum_channel(&base, 1).await;
    let (mut keeper, _resp) = connect_async(daemon.ws_url("/ws/stream/spectrum/1"))
        .await
        .unwrap();
    let bins_before = wait_for_binary_frame(&mut keeper, "establishing the keeper stream").await;
    assert!(bins_before > 24, "spectrum frame should carry FFT bins");

    for i in 0..30u32 {
        let id = 100 + i;
        create_spectrum_channel(&base, id).await;
        let (mut ws, _resp) = connect_async(daemon.ws_url(&format!("/ws/stream/spectrum/{id}")))
            .await
            .unwrap();
        wait_for_binary_frame(&mut ws, "an ephemeral client's stream").await;
        // Explicit close on half, hard drop on the other — the daemon must treat both the
        // same and neither must wedge the keeper.
        if i % 2 == 0 {
            let _ = ws.close(None).await;
        }
        drop(ws);
    }

    let bins_after =
        wait_for_binary_frame(&mut keeper, "verifying the keeper survived churn").await;
    assert_eq!(
        bins_before, bins_after,
        "spectrum frame size should be stable across churn"
    );

    drop(keeper);
    daemon.shutdown().await;
}

/// A stalled data-plane client that opens the WS but never reads it must not throttle a
/// healthy client streaming a different channel. Per-connection writer tasks + independent
/// broadcast queues mean backpressure on one socket is isolated to that socket.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_ws_reader_does_not_starve_a_healthy_client() {
    let daemon = Harness::spawn("web-stall").await;
    let base = daemon.base_url();
    wait_until_web_ready(&base).await;

    create_spectrum_channel(&base, 1).await;
    create_spectrum_channel(&base, 2).await;

    // Deliberately antisocial client: connect, then never read again. Its OS socket buffer
    // and the daemon's per-connection mpsc channel will fill and back up — on its side only.
    let (slow_ws, _resp) = connect_async(daemon.ws_url("/ws/stream/spectrum/1"))
        .await
        .unwrap();
    let _slow_guard = slow_ws;

    let (mut fast_ws, _resp) = connect_async(daemon.ws_url("/ws/stream/spectrum/2"))
        .await
        .unwrap();
    for _ in 0..15 {
        let len =
            wait_for_binary_frame(&mut fast_ws, "healthy client streaming past a stalled peer")
                .await;
        assert!(len > 24);
    }

    daemon.shutdown().await;
}

/// A control-plane connection flooding `/ws/control` with frequency retunes + pings at high
/// rate must not deadlock the data plane on a separate connection: spectrum keeps flowing on
/// its own socket because the control and data planes are fully independent WS routes/tasks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_ws_flood_keeps_data_plane_ws_live() {
    let daemon = Harness::spawn("web-flood").await;
    let base = daemon.base_url();
    wait_until_web_ready(&base).await;

    create_spectrum_channel(&base, 1).await;
    let (mut data_ws, _resp) = connect_async(daemon.ws_url("/ws/stream/spectrum/1"))
        .await
        .unwrap();
    wait_for_binary_frame(&mut data_ws, "establishing the data-plane stream").await;

    let (mut control_ws, _resp) = connect_async(daemon.ws_url("/ws/control")).await.unwrap();
    // Drain the initial Welcome + Hardware events.
    let _ = control_ws.next().await.unwrap().unwrap();
    let _ = control_ws.next().await.unwrap().unwrap();

    for nonce in 0..200u64 {
        let cmd = ClientCommand::SetFrequency {
            hz: 100_000_000u64 + nonce * 1_000,
        };
        control_ws
            .send(TMessage::Text(serde_json::to_string(&cmd).unwrap()))
            .await
            .unwrap();
        if nonce % 20 == 0 {
            let ping = ClientCommand::Ping { nonce };
            control_ws
                .send(TMessage::Text(serde_json::to_string(&ping).unwrap()))
                .await
                .unwrap();
        }
    }

    // A Pong (control-plane liveness) must arrive on the control socket...
    let mut saw_pong = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !saw_pong && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), control_ws.next()).await {
            Ok(Some(Ok(TMessage::Text(text)))) => {
                if let Ok(event) = serde_json::from_str::<ServerEvent>(&text) {
                    if matches!(event, ServerEvent::Pong { .. }) {
                        saw_pong = true;
                    }
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => panic!("control ws error while awaiting Pong: {e}"),
            Ok(None) => panic!("control ws closed while awaiting Pong"),
            // Per-message timeout, not the overall deadline — keep polling.
            Err(_) => {}
        }
    }
    assert!(
        saw_pong,
        "control plane wedged: never got a Pong after the command flood"
    );

    // ...and spectrum frames must never have stopped arriving on the independent data socket.
    for _ in 0..5 {
        let len = wait_for_binary_frame(&mut data_ws, "data plane after the control flood").await;
        assert!(len > 24);
    }

    drop((control_ws, data_ws));
    daemon.shutdown().await;
}

/// Many browser-style clients attaching at once and streaming concurrently must all make
/// progress, and the daemon must still shut down cleanly afterward. Exercises the axum accept
/// loop and per-connection task fan-out under simultaneous (not just sequential) web load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_concurrent_ws_clients_all_receive_frames() {
    let daemon = Harness::spawn("web-concurrent").await;
    let base = daemon.base_url();
    wait_until_web_ready(&base).await;

    for i in 0..16u32 {
        create_spectrum_channel(&base, i + 1).await;
    }

    let mut tasks = Vec::new();
    for i in 0..16u32 {
        let url = daemon.ws_url(&format!("/ws/stream/spectrum/{}", i + 1));
        tasks.push(tokio::spawn(async move {
            let (mut ws, _resp) = connect_async(url).await.unwrap();
            for _ in 0..5 {
                wait_for_binary_frame(&mut ws, "a concurrent client's stream").await;
            }
        }));
    }

    for (i, t) in tasks.into_iter().enumerate() {
        tokio::time::timeout(Duration::from_secs(15), t)
            .await
            .unwrap_or_else(|_| {
                panic!("concurrent web client {i} did not finish streaming in time")
            })
            .unwrap_or_else(|e| panic!("concurrent web client {i} panicked: {e}"));
    }

    daemon.shutdown().await;
}

/// Reconnecting a WS data-plane client after a hard drop (no close handshake) must work: the
/// daemon treats an abrupt socket loss as an implicit unsubscribe, frees the forwarder thread,
/// and serves a brand-new connection to the same channel normally.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_after_hard_ws_drop_streams_again() {
    let daemon = Harness::spawn("web-reconnect").await;
    let base = daemon.base_url();
    wait_until_web_ready(&base).await;

    create_spectrum_channel(&base, 1).await;

    {
        let (mut ws, _resp) = connect_async(daemon.ws_url("/ws/stream/spectrum/1"))
            .await
            .unwrap();
        wait_for_binary_frame(&mut ws, "first connection stream").await;
        // Drop without a close handshake: an abrupt disconnect.
    }

    let (mut ws2, _resp) = connect_async(daemon.ws_url("/ws/stream/spectrum/1"))
        .await
        .unwrap();
    wait_for_binary_frame(&mut ws2, "reconnected client stream").await;

    drop(ws2);
    daemon.shutdown().await;
}
