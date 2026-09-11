//! Top-level daemon wiring: builds the configured [`IqSource`], the wideband [`SampleBus`],
//! the ingestion thread, [`DaemonState`], and the TCP server into one running daemon. This
//! is the one module that knows every concrete piece exists and how they connect —
//! everything downstream only knows the piece it directly depends on (the channelizer
//! never hears about `IqSource`, pipelines never hear about the TCP layer, and so on).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;

use crate::bus::SampleBus;
use crate::hardware::replay::{FileReplaySource, ReplayFormat};
use crate::hardware::synthetic::SyntheticSource;
use crate::hardware::IqSource;
use crate::ingest;
use crate::server;
use crate::state::DaemonState;

/// Which [`IqSource`] the daemon should ingest from.
#[derive(Debug, Clone)]
pub enum SourceConfig {
    /// Deterministic generated signal; always available, no hardware or files needed.
    Synthetic,
    /// Plays back a previously captured IQ file — see `crate::hardware::replay`.
    Replay {
        path: String,
        format: ReplayFormat,
        looping: bool,
        speed: f32,
    },
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub listen_addr: SocketAddr,
    /// Address the web UI/API (`crate::web`) listens on — a separate socket from
    /// `listen_addr` so the legacy TCP control/data path and the HTTP/WebSocket path can
    /// each be bound, saturated, or shut down independently.
    pub web_listen_addr: SocketAddr,
    pub initial_freq_hz: u64,
    pub initial_sample_rate_hz: u32,
    pub recording_dir: PathBuf,
    /// Directory containing the compiled frontend's static assets (`index.html`, JS/CSS
    /// bundles). Served at `/` by `crate::web::serve`; see that function's doc comment for
    /// behavior when the directory doesn't exist.
    pub web_static_dir: PathBuf,
    pub source: SourceConfig,
}

fn build_source(config: &SourceConfig, initial_sample_rate_hz: u32) -> Result<Box<dyn IqSource>> {
    let source: Box<dyn IqSource> = match config {
        SourceConfig::Synthetic => Box::new(SyntheticSource::default()),
        SourceConfig::Replay {
            path,
            format,
            looping,
            speed,
        } => Box::new(FileReplaySource::open(
            path,
            *format,
            initial_sample_rate_hz,
            *looping,
            *speed,
        )?),
    };
    Ok(source)
}

/// Runs the daemon until `running` clears: binds both the legacy TCP listener and the web
/// UI/API listener, wires hardware through to both, then runs `server::serve` and
/// `web::serve` concurrently on this same runtime until either returns (both observe the
/// same `running` flag, so a clean shutdown stops both together; an early error from either
/// still tears down the other via `running` clearing on drop of this future). Returns once
/// shutdown is complete — including the ingestion thread, whose blocking join is offloaded
/// to a blocking-pool thread so a slow-to-stop hardware source can't stall the async runtime
/// during shutdown.
pub async fn run(config: DaemonConfig, running: Arc<AtomicBool>) -> Result<()> {
    let mut source = build_source(&config.source, config.initial_sample_rate_hz)?;
    source.set_frequency(config.initial_freq_hz)?;
    source.set_sample_rate(config.initial_sample_rate_hz)?;

    let bus = SampleBus::new();
    let (hardware, ingest_thread) = ingest::spawn(source, bus.clone(), Arc::clone(&running))?;

    let state = Arc::new(DaemonState::new(
        bus,
        hardware,
        config.initial_freq_hz,
        config.initial_sample_rate_hz,
        config.recording_dir.clone(),
    ));

    let tcp_listener = TcpListener::bind(config.listen_addr).await?;
    let web_listener = TcpListener::bind(config.web_listen_addr).await?;

    let tcp_state = Arc::clone(&state);
    let tcp_running = Arc::clone(&running);
    let tcp_task =
        tokio::spawn(async move { server::serve(tcp_listener, tcp_state, tcp_running).await });

    let web_state = Arc::clone(&state);
    let web_running = Arc::clone(&running);
    let web_static_dir = config.web_static_dir.clone();
    let web_task = tokio::spawn(async move {
        crate::web::serve(web_listener, web_state, web_running, web_static_dir).await
    });

    let (tcp_result, web_result) = tokio::join!(tcp_task, web_task);
    let result = tcp_result
        .map_err(anyhow::Error::from)
        .and_then(|r| r)
        .and(web_result.map_err(anyhow::Error::from).and_then(|r| r));

    let _ = tokio::task::spawn_blocking(move || ingest_thread.join()).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ez_proto::{ClientCommand, MessageCodec, ServerEvent, PROTOCOL_VERSION};
    use futures_util::{SinkExt, StreamExt};
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tokio::net::TcpStream;
    use tokio_util::codec::{FramedRead, FramedWrite};

    /// Grabs an OS-assigned free port by binding then immediately dropping a listener.
    /// There's a theoretical race if something else grabs the port before `run` rebinds
    /// it, but for a listen-only (never-connected) socket on Linux this is not subject to
    /// `TIME_WAIT` and is a standard, effectively-reliable pattern for this kind of test.
    async fn free_addr() -> SocketAddr {
        TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
    }

    async fn connect_with_retry(addr: SocketAddr) -> TcpStream {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match TcpStream::connect(addr).await {
                Ok(s) => return s,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => panic!("could not connect to daemon: {e}"),
            }
        }
    }

    async fn expect_welcome(addr: SocketAddr, client_name: &str) {
        let stream = connect_with_retry(addr).await;
        let (r, w) = stream.into_split();
        let mut writer = FramedWrite::new(w, MessageCodec::<ClientCommand>::for_commands());
        let mut reader = FramedRead::new(r, MessageCodec::<ServerEvent>::for_data());
        writer
            .send(ClientCommand::Hello {
                client_name: client_name.to_string(),
                protocol_version: PROTOCOL_VERSION,
            })
            .await
            .unwrap();
        let welcome = reader.next().await.unwrap().unwrap();
        assert!(matches!(welcome, ServerEvent::Welcome { .. }));
    }

    #[tokio::test]
    async fn runs_end_to_end_with_synthetic_source_and_shuts_down_on_signal() {
        let addr = free_addr().await;
        let web_addr = free_addr().await;
        let dir = std::env::temp_dir().join(format!(
            "ez-daemon-app-tests-synthetic-{}",
            std::process::id()
        ));
        let config = DaemonConfig {
            listen_addr: addr,
            web_listen_addr: web_addr,
            initial_freq_hz: 100_000_000,
            initial_sample_rate_hz: 2_000_000,
            recording_dir: dir,
            web_static_dir: std::env::temp_dir().join("ez-daemon-app-tests-nonexistent-static"),
            source: SourceConfig::Synthetic,
        };
        let running = Arc::new(AtomicBool::new(true));
        let run_running = Arc::clone(&running);
        let daemon = tokio::spawn(async move { run(config, run_running).await });

        expect_welcome(addr, "app-test-synthetic").await;

        running.store(false, Ordering::Relaxed);
        let result = tokio::time::timeout(Duration::from_secs(5), daemon)
            .await
            .expect("daemon did not shut down promptly");
        assert!(result.unwrap().is_ok());
    }

    #[tokio::test]
    async fn runs_end_to_end_with_replay_source() {
        let addr = free_addr().await;
        let web_addr = free_addr().await;
        let fixture = std::env::temp_dir().join(format!(
            "ez-daemon-app-replay-fixture-{}.cf32",
            std::process::id()
        ));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&fixture).unwrap();
            for i in 0..64 {
                let iq = (i as f32 * 0.01, -(i as f32) * 0.01);
                f.write_all(&iq.0.to_le_bytes()).unwrap();
                f.write_all(&iq.1.to_le_bytes()).unwrap();
            }
        }
        let dir =
            std::env::temp_dir().join(format!("ez-daemon-app-tests-replay-{}", std::process::id()));
        let config = DaemonConfig {
            listen_addr: addr,
            web_listen_addr: web_addr,
            initial_freq_hz: 100_000_000,
            initial_sample_rate_hz: 1_000_000,
            recording_dir: dir,
            web_static_dir: std::env::temp_dir().join("ez-daemon-app-tests-nonexistent-static"),
            source: SourceConfig::Replay {
                path: fixture.to_str().unwrap().to_string(),
                format: ReplayFormat::Cf32Le,
                looping: true,
                speed: 1000.0,
            },
        };
        let running = Arc::new(AtomicBool::new(true));
        let run_running = Arc::clone(&running);
        let daemon = tokio::spawn(async move { run(config, run_running).await });

        expect_welcome(addr, "app-test-replay").await;

        running.store(false, Ordering::Relaxed);
        let result = tokio::time::timeout(Duration::from_secs(5), daemon)
            .await
            .expect("daemon did not shut down promptly");
        assert!(result.unwrap().is_ok());

        std::fs::remove_file(&fixture).ok();
    }
}
