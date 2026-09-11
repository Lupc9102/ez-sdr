//! Integration tests for daemon connection resilience under simulated high-throughput
//! processing load (Definition of Done #5).
//!
//! Unlike the in-crate unit tests (which exercise `serve` against a hand-built
//! [`DaemonState`]), these drive a *fully wired* daemon through its public entry point
//! [`ez_daemon::run`] — real ingestion thread, real channelizer, real spectrum pipeline,
//! real async TCP server — over loopback TCP with the same `ez-proto` wire codec a GUI
//! client uses. They assert the property the headless architecture exists to guarantee:
//! **client connection volatility never stalls or corrupts daemon-side processing.**
//!
//! Each test spins the daemon on the test's own multi-threaded runtime so many connections
//! are genuinely concurrent, hammers it (attach/detach churn, a deliberately stalled
//! reader, a control-command flood), and verifies a well-behaved client keeps getting a
//! live spectrum stream throughout, then that the daemon shuts down promptly and cleanly.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{FramedRead, FramedWrite};

use ez_daemon::{run, DaemonConfig, SourceConfig};
use ez_proto::{
    ChannelId, ChannelSpec, ClientCommand, MessageCodec, PipelineKind, ServerEvent,
    PROTOCOL_VERSION,
};

type ClientWriter = FramedWrite<OwnedWriteHalf, MessageCodec<ClientCommand>>;
type ClientReader = FramedRead<OwnedReadHalf, MessageCodec<ServerEvent>>;

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A running daemon under test, plus the address to reach it and the flag to stop it.
struct Harness {
    addr: SocketAddr,
    #[allow(dead_code)]
    web_addr: SocketAddr,
    running: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Harness {
    /// Boots a synthetic-source daemon on an ephemeral loopback port. Synthetic ingestion
    /// is deterministic and hardware-free, so the spectrum pipeline always produces frames —
    /// exactly the steady processing load these tests need to run *underneath* client churn.
    async fn spawn(label: &str) -> Self {
        let addr = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let web_addr = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let recording_dir =
            std::env::temp_dir().join(format!("ez-daemon-it-{label}-{}-{n}", std::process::id()));
        let config = DaemonConfig {
            listen_addr: addr,
            web_listen_addr: web_addr,
            initial_freq_hz: 100_000_000,
            initial_sample_rate_hz: 2_000_000,
            recording_dir,
            web_static_dir: std::env::temp_dir().join("ez-daemon-it-nonexistent-static"),
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

    /// Signals shutdown and asserts the daemon's top-level future returns `Ok` promptly —
    /// i.e. no connection task, forwarder thread, or ingestion join left it wedged.
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

/// Connects with a bounded retry loop against `ConnectionRefused`. `Harness::spawn` returns
/// as soon as the daemon task is *spawned*, not once it's actually listening — `run()` does
/// real setup (build the source, spawn the ingestion thread, construct `DaemonState`) before
/// it binds the TCP listener, so the very first `attach()` in a test can otherwise race that
/// startup window and lose. Mirrors `connect_with_retry` in `ez-daemon/src/app.rs`'s own
/// tests, duplicated here because that helper lives in a `#[cfg(test)]` module private to the
/// crate and isn't visible to this separate integration-test binary.
async fn connect_with_retry(addr: SocketAddr) -> TcpStream {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect(addr).await {
            Ok(s) => return s,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(e) => panic!("connect to daemon: {e}"),
        }
    }
}

/// Opens a TCP connection and completes the `Hello`/`Welcome` handshake, draining the
/// initial `Welcome` + `Hardware` events. Returns the framed halves ready for use.
async fn attach(addr: SocketAddr, name: &str) -> (ClientWriter, ClientReader) {
    let stream = connect_with_retry(addr).await;
    let _ = stream.set_nodelay(true);
    let (rh, wh) = stream.into_split();
    let mut writer = FramedWrite::new(wh, MessageCodec::<ClientCommand>::for_commands());
    let mut reader = FramedRead::new(rh, MessageCodec::<ServerEvent>::for_data());

    writer
        .send(ClientCommand::Hello {
            client_name: name.to_string(),
            protocol_version: PROTOCOL_VERSION,
        })
        .await
        .expect("send Hello");

    let welcome = next_event(&mut reader, "Welcome").await;
    assert!(
        matches!(welcome, ServerEvent::Welcome { protocol_version, .. } if protocol_version == PROTOCOL_VERSION),
        "expected Welcome, got {welcome:?}"
    );
    // The daemon always follows Welcome with an initial Hardware snapshot.
    let _ = next_event(&mut reader, "initial Hardware").await;

    (writer, reader)
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

/// Reads the next decoded event or panics with context on timeout / stream close.
async fn next_event(reader: &mut ClientReader, what: &str) -> ServerEvent {
    tokio::time::timeout(Duration::from_secs(10), reader.next())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
        .unwrap_or_else(|| panic!("connection closed while waiting for {what}"))
        .unwrap_or_else(|e| panic!("decode error waiting for {what}: {e}"))
}

/// Pulls events until a `Spectrum` frame arrives (skipping periodic `Hardware` ticks and
/// anything else), returning the number of bins so callers can sanity-check the payload.
async fn wait_for_spectrum(reader: &mut ClientReader, what: &str) -> usize {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or_default();
        assert!(
            !remaining.is_zero(),
            "timed out waiting for a spectrum frame while {what}"
        );
        match tokio::time::timeout(remaining, reader.next()).await {
            Ok(Some(Ok(ServerEvent::Spectrum(frame)))) => return frame.bins.len(),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("decode error while {what}: {e}"),
            Ok(None) => panic!("connection closed while {what}"),
            Err(_) => panic!("timed out waiting for a spectrum frame while {what}"),
        }
    }
}

/// A persistent, well-behaved subscriber must keep receiving spectrum frames while dozens
/// of other clients rapidly attach, subscribe, read a frame, and detach. This is the core
/// resilience guarantee: connection churn on other sockets never stalls the processing
/// pipeline feeding an established client.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn persistent_subscriber_survives_rapid_attach_detach_churn() {
    let daemon = Harness::spawn("churn").await;

    let (mut keeper_w, mut keeper_r) = attach(daemon.addr, "keeper").await;
    keeper_w
        .send(ClientCommand::Subscribe {
            channel: spectrum_spec(1),
        })
        .await
        .unwrap();
    let bins_before = wait_for_spectrum(&mut keeper_r, "establishing the keeper stream").await;
    assert!(bins_before > 0, "spectrum frame should carry FFT bins");

    // Churn: many short-lived clients, each doing a full attach → subscribe → read → detach.
    for i in 0..30 {
        let (mut w, mut r) = attach(daemon.addr, &format!("ephemeral-{i}")).await;
        w.send(ClientCommand::Subscribe {
            channel: spectrum_spec(100 + i),
        })
        .await
        .unwrap();
        wait_for_spectrum(&mut r, "an ephemeral client's stream").await;
        // Explicit clean detach on half, hard socket drop on the other half — the daemon
        // must treat both the same and neither must wedge the keeper.
        if i % 2 == 0 {
            w.send(ClientCommand::Detach).await.unwrap();
        }
        drop(w);
        drop(r);
    }

    // The keeper must still be getting fresh frames after all that churn.
    let bins_after = wait_for_spectrum(&mut keeper_r, "verifying the keeper survived churn").await;
    assert_eq!(
        bins_before, bins_after,
        "spectrum width should be stable across churn"
    );

    drop((keeper_w, keeper_r));
    daemon.shutdown().await;
}

/// A stalled client that subscribes but never reads its socket must not throttle a healthy
/// client sharing the same daemon. Per-connection writer tasks + independent broadcast
/// queues mean backpressure on one socket is isolated to that socket.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_reader_does_not_starve_a_healthy_client() {
    let daemon = Harness::spawn("stall").await;

    // Deliberately antisocial client: subscribe, then never read again. Its OS socket buffer
    // and the daemon's per-connection event channel will fill and back up — on its side only.
    let (mut slow_w, slow_r) = attach(daemon.addr, "stalled").await;
    slow_w
        .send(ClientCommand::Subscribe {
            channel: spectrum_spec(1),
        })
        .await
        .unwrap();
    // Keep the handles alive (connection stays open) but never poll the reader.
    let _slow_guard = (slow_w, slow_r);

    // Healthy client should sail through, receiving many frames back to back.
    let (mut fast_w, mut fast_r) = attach(daemon.addr, "healthy").await;
    fast_w
        .send(ClientCommand::Subscribe {
            channel: spectrum_spec(2),
        })
        .await
        .unwrap();

    for _ in 0..15 {
        let bins =
            wait_for_spectrum(&mut fast_r, "healthy client streaming past a stalled peer").await;
        assert!(bins > 0);
    }

    daemon.shutdown().await;
}

/// A single connection flooding the control plane (frequency retunes + pings) at high rate
/// must not deadlock its own data plane: pongs keep coming back and spectrum keeps flowing,
/// because commands and pipeline frames funnel through the one per-connection writer without
/// fighting over the socket.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_command_flood_keeps_data_plane_live() {
    let daemon = Harness::spawn("flood").await;

    let (mut w, mut r) = attach(daemon.addr, "flooder").await;
    w.send(ClientCommand::Subscribe {
        channel: spectrum_spec(1),
    })
    .await
    .unwrap();
    wait_for_spectrum(&mut r, "establishing the flooder stream").await;

    // Blast a burst of control commands, interleaving pings we can correlate.
    for nonce in 0..200u64 {
        w.send(ClientCommand::SetFrequency {
            hz: 100_000_000 + nonce * 1_000,
        })
        .await
        .unwrap();
        if nonce % 20 == 0 {
            w.send(ClientCommand::Ping { nonce }).await.unwrap();
        }
    }

    // Both a Pong (control-plane liveness) and a Spectrum frame (data-plane liveness) must
    // still arrive despite the flood — prove the connection didn't wedge either direction.
    let mut saw_pong = false;
    let mut saw_spectrum = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while (!saw_pong || !saw_spectrum) && tokio::time::Instant::now() < deadline {
        match next_event(&mut r, "pong or spectrum after the flood").await {
            ServerEvent::Pong { .. } => saw_pong = true,
            ServerEvent::Spectrum(_) => saw_spectrum = true,
            _ => {}
        }
    }
    assert!(
        saw_pong,
        "control plane wedged: never got a Pong after the command flood"
    );
    assert!(
        saw_spectrum,
        "data plane wedged: spectrum stopped during the command flood"
    );

    drop((w, r));
    daemon.shutdown().await;
}

/// Many clients attaching at once and streaming concurrently must all make progress, and the
/// daemon must still shut down cleanly afterward. Exercises the accept loop and per-connection
/// task fan-out under simultaneous (not just sequential) load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_concurrent_clients_all_receive_frames() {
    let daemon = Harness::spawn("concurrent").await;
    let addr = daemon.addr;

    let mut tasks = Vec::new();
    for i in 0..16u32 {
        tasks.push(tokio::spawn(async move {
            let (mut w, mut r) = attach(addr, &format!("concurrent-{i}")).await;
            w.send(ClientCommand::Subscribe {
                channel: spectrum_spec(i + 1),
            })
            .await
            .unwrap();
            // Each independent client must pull several frames of its own.
            for _ in 0..5 {
                wait_for_spectrum(&mut r, "a concurrent client's stream").await;
            }
        }));
    }

    for (i, t) in tasks.into_iter().enumerate() {
        tokio::time::timeout(Duration::from_secs(15), t)
            .await
            .unwrap_or_else(|_| panic!("concurrent client {i} did not finish streaming in time"))
            .unwrap_or_else(|e| panic!("concurrent client {i} panicked: {e}"));
    }

    daemon.shutdown().await;
}

/// Reconnecting after a hard drop (no `Detach`) must work: the daemon treats a dropped TCP
/// connection as an implicit detach, frees the channel, and serves a brand-new client
/// normally. Verifies lifecycle state isn't corrupted by ungraceful disconnects.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_after_hard_drop_streams_again() {
    let daemon = Harness::spawn("reconnect").await;

    {
        let (mut w, mut r) = attach(daemon.addr, "first").await;
        w.send(ClientCommand::Subscribe {
            channel: spectrum_spec(1),
        })
        .await
        .unwrap();
        wait_for_spectrum(&mut r, "first connection stream").await;
        // Drop both halves without sending Detach: an abrupt disconnect.
    }

    // A fresh client reusing the same channel id must still be served.
    let (mut w2, mut r2) = attach(daemon.addr, "second").await;
    w2.send(ClientCommand::Subscribe {
        channel: spectrum_spec(1),
    })
    .await
    .unwrap();
    wait_for_spectrum(&mut r2, "reconnected client stream").await;

    drop((w2, r2));
    daemon.shutdown().await;
}
