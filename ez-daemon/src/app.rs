//! Top-level daemon wiring: builds the configured [`IqSource`], the wideband [`SampleBus`],
//! the ingestion thread, [`DaemonState`], and the TCP server into one running daemon. This
//! is the one module that knows every concrete piece exists and how they connect —
//! everything downstream only knows the piece it directly depends on (the channelizer
//! never hears about `IqSource`, pipelines never hear about the TCP layer, and so on).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::bus::SampleBus;
use crate::hardware::replay::{FileReplaySource, ReplayFormat};
use crate::hardware::synthetic::SyntheticSource;
use crate::hardware::tcp_iq::TcpIqSource;
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
        amplitude: Option<lrpt_decode::Cf32Amplitude>,
    },
    /// Raw Uc8 IQ from a network rtl_tcp-compatible source.
    TcpIq {
        address: SocketAddr,
        rtl_tcp_header: bool,
    },
    /// First available RTL-SDR, or the device whose serial/name matches `device`.
    #[cfg(feature = "rtlsdr")]
    RtlSdr { device: Option<String> },
    /// First available HackRF device.
    #[cfg(feature = "hackrf")]
    HackRf,
    /// SoapySDR device selected by its driver argument string.
    #[cfg(feature = "soapy")]
    Soapy { device: Option<String> },
    #[cfg(test)]
    LifecycleTest {
        probe: Arc<LifecycleTestProbe>,
        fail_start: bool,
    },
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct LifecycleTestProbe {
    starts: std::sync::atomic::AtomicUsize,
    stops: std::sync::atomic::AtomicUsize,
    reads: std::sync::atomic::AtomicUsize,
    active: AtomicBool,
}

#[cfg(test)]
struct LifecycleTestSource {
    probe: Arc<LifecycleTestProbe>,
    fail_start: bool,
    running: bool,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
}

#[cfg(test)]
impl IqSource for LifecycleTestSource {
    fn start(&mut self) -> Result<()> {
        self.probe
            .starts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.fail_start {
            anyhow::bail!("injected source startup failure");
        }
        self.running = true;
        self.probe.active.store(true, Ordering::Release);
        Ok(())
    }

    fn stop(&mut self) {
        self.running = false;
        self.probe.active.store(false, Ordering::Release);
        self.probe
            .stops
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        self.frequency_hz = hz;
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        self.sample_rate_hz = hz;
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        self.gain_db = db;
        Ok(())
    }

    fn read_iq(&mut self, samples: &mut [num_complex::Complex32]) -> Result<usize> {
        if !self.running {
            return Ok(0);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        self.probe
            .reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        samples.fill(num_complex::Complex32::new(0.25, -0.25));
        Ok(samples.len())
    }

    fn frequency_hz(&self) -> u64 {
        self.frequency_hz
    }

    fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    fn gain_db(&self) -> f64 {
        self.gain_db
    }

    fn kind(&self) -> &'static str {
        "lifecycle-test"
    }
}

#[derive(Clone)]
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
    /// Optional bearer token shared by REST, browser WebSocket, and binary TCP clients.
    /// `None` preserves unauthenticated loopback development behavior.
    pub auth_token: Option<String>,
    pub source: SourceConfig,
    /// Coordinates of the actual capture site, used only to resolve surface CPR.
    pub receiver_position: Option<(f64, f64)>,
}

impl std::fmt::Debug for DaemonConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonConfig")
            .field("listen_addr", &self.listen_addr)
            .field("web_listen_addr", &self.web_listen_addr)
            .field("initial_freq_hz", &self.initial_freq_hz)
            .field("initial_sample_rate_hz", &self.initial_sample_rate_hz)
            .field("recording_dir", &self.recording_dir)
            .field("web_static_dir", &self.web_static_dir)
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("source", &self.source)
            .field("receiver_position", &self.receiver_position)
            .finish()
    }
}

fn build_source(config: &SourceConfig, initial_sample_rate_hz: u32) -> Result<Box<dyn IqSource>> {
    let source: Box<dyn IqSource> = match config {
        SourceConfig::Synthetic => Box::new(SyntheticSource::default()),
        SourceConfig::Replay {
            path,
            format,
            looping,
            speed,
            amplitude,
        } => Box::new(FileReplaySource::open_with_amplitude(
            path,
            *format,
            initial_sample_rate_hz,
            *looping,
            *speed,
            *amplitude,
        )?),
        SourceConfig::TcpIq {
            address,
            rtl_tcp_header,
        } => Box::new(TcpIqSource::new(*address, *rtl_tcp_header)),
        #[cfg(feature = "rtlsdr")]
        SourceConfig::RtlSdr { device } => {
            Box::new(crate::hardware::rtlsdr::RtlSdrSource::new(device.clone()))
        }
        #[cfg(feature = "hackrf")]
        SourceConfig::HackRf => Box::new(crate::hardware::hackrf::HackRfSource::new()),
        #[cfg(feature = "soapy")]
        SourceConfig::Soapy { device } => {
            Box::new(crate::hardware::soapy::SoapySource::new(device.clone()))
        }
        #[cfg(test)]
        SourceConfig::LifecycleTest { probe, fail_start } => Box::new(LifecycleTestSource {
            probe: Arc::clone(probe),
            fail_start: *fail_start,
            running: false,
            frequency_hz: 0,
            sample_rate_hz: initial_sample_rate_hz,
            gain_db: 0.0,
        }),
    };
    Ok(source)
}

const SERVER_STOP_TIMEOUT: Duration = Duration::from_secs(10);

struct DaemonLifecycle {
    running: Arc<AtomicBool>,
    runtime: tokio::runtime::Handle,
    tcp_task: Option<tokio::task::JoinHandle<Result<()>>>,
    web_task: Option<tokio::task::JoinHandle<Result<()>>>,
    ingest_thread: Option<std::thread::JoinHandle<()>>,
}

impl DaemonLifecycle {
    fn new(running: Arc<AtomicBool>, ingest_thread: std::thread::JoinHandle<()>) -> Self {
        Self {
            running,
            runtime: tokio::runtime::Handle::current(),
            tcp_task: None,
            web_task: None,
            ingest_thread: Some(ingest_thread),
        }
    }
}

impl Drop for DaemonLifecycle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        let tcp_task = self.tcp_task.take();
        let web_task = self.web_task.take();
        if let Some(task) = &tcp_task {
            task.abort();
        }
        if let Some(task) = &web_task {
            task.abort();
        }
        if tcp_task.is_some() || web_task.is_some() {
            self.runtime.spawn(async move {
                if let Some(task) = tcp_task {
                    let _ = task.await;
                }
                if let Some(task) = web_task {
                    let _ = task.await;
                }
            });
        }
        if let Some(ingest_thread) = self.ingest_thread.take() {
            self.runtime.spawn_blocking(move || {
                let _ = ingest_thread.join();
            });
        }
    }
}

fn validate_config(config: &DaemonConfig) -> Result<()> {
    if config.auth_token.as_deref() == Some("") {
        anyhow::bail!("authentication token must not be empty");
    }
    if config.receiver_position.is_some_and(|(lat, lon)| {
        !lat.is_finite()
            || !lon.is_finite()
            || !(-90.0..=90.0).contains(&lat)
            || !(-180.0..=180.0).contains(&lon)
    }) {
        anyhow::bail!("invalid capture-site coordinates");
    }
    Ok(())
}

fn flatten_server_task(
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
) -> Result<()> {
    result.context("daemon server task panicked")?
}

async fn wait_for_first_server_exit(
    tcp_task: &mut tokio::task::JoinHandle<Result<()>>,
    web_task: &mut tokio::task::JoinHandle<Result<()>>,
) -> (bool, Result<()>) {
    tokio::select! {
        result = tcp_task => (true, flatten_server_task(result)),
        result = web_task => (false, flatten_server_task(result)),
    }
}

async fn stop_server_task(
    mut task: tokio::task::JoinHandle<Result<()>>,
    name: &'static str,
) -> Result<()> {
    match tokio::time::timeout(SERVER_STOP_TIMEOUT, &mut task).await {
        Ok(result) => flatten_server_task(result),
        Err(_) => {
            task.abort();
            let _ = task.await;
            anyhow::bail!("{name} server did not stop within {SERVER_STOP_TIMEOUT:?}");
        }
    }
}

async fn join_ingest_thread(thread: std::thread::JoinHandle<()>) -> Result<()> {
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .context("ingestion join task failed")?
        .map_err(|_| anyhow::anyhow!("ingestion thread panicked"))
}

/// Runs the daemon until `running` clears. Both listeners are bound before the IQ source
/// starts. Normal shutdown joins both server tasks and the ingestion worker; dropping this
/// future signals the source, aborts its listener tasks and schedules the ingestion join off
/// the async executor.
pub async fn run(config: DaemonConfig, running: Arc<AtomicBool>) -> Result<()> {
    validate_config(&config)?;

    let tcp_listener = TcpListener::bind(config.listen_addr)
        .await
        .with_context(|| format!("bind TCP listener {}", config.listen_addr))?;
    let web_listener = TcpListener::bind(config.web_listen_addr)
        .await
        .with_context(|| format!("bind web listener {}", config.web_listen_addr))?;

    let mut source = build_source(&config.source, config.initial_sample_rate_hz)?;
    if let Err(error) = source.set_frequency(config.initial_freq_hz) {
        source.stop();
        return Err(error).context("configure initial receiver frequency");
    }
    if let Err(error) = source.set_sample_rate(config.initial_sample_rate_hz) {
        source.stop();
        return Err(error).context("configure initial receiver sample rate");
    }

    let bus = SampleBus::new();
    let (hardware, ingest_thread) = ingest::spawn(source, bus.clone(), Arc::clone(&running))?;
    let mut lifecycle = DaemonLifecycle::new(Arc::clone(&running), ingest_thread);

    let state = Arc::new(DaemonState::new(
        bus,
        hardware,
        config.initial_freq_hz,
        config.initial_sample_rate_hz,
        config.recording_dir.clone(),
    ));
    state.set_auth_token(config.auth_token.clone())?;

    state.set_receiver_position(config.receiver_position)?;

    let tcp_state = Arc::clone(&state);
    let tcp_running = Arc::clone(&running);
    lifecycle.tcp_task = Some(tokio::spawn(async move {
        server::serve(tcp_listener, tcp_state, tcp_running).await
    }));

    let web_state = Arc::clone(&state);
    let web_running = Arc::clone(&running);
    let web_static_dir = config.web_static_dir.clone();
    lifecycle.web_task = Some(tokio::spawn(async move {
        crate::web::serve(web_listener, web_state, web_running, web_static_dir).await
    }));

    let (tcp_finished_first, first_result) = wait_for_first_server_exit(
        lifecycle.tcp_task.as_mut().expect("TCP server task owner"),
        lifecycle.web_task.as_mut().expect("web server task owner"),
    )
    .await;
    running.store(false, Ordering::Release);

    let second_result = if tcp_finished_first {
        lifecycle.tcp_task.take();
        stop_server_task(
            lifecycle.web_task.take().expect("web server task owner"),
            "web",
        )
        .await
    } else {
        lifecycle.web_task.take();
        stop_server_task(
            lifecycle.tcp_task.take().expect("TCP server task owner"),
            "TCP",
        )
        .await
    };
    let ingest_result = join_ingest_thread(
        lifecycle
            .ingest_thread
            .take()
            .expect("ingestion thread owner"),
    )
    .await;
    first_result.and(second_result)?;
    ingest_result
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

    fn lifecycle_config(
        listen_addr: SocketAddr,
        web_listen_addr: SocketAddr,
        probe: Arc<LifecycleTestProbe>,
        fail_start: bool,
    ) -> DaemonConfig {
        DaemonConfig {
            listen_addr,
            web_listen_addr,
            initial_freq_hz: 100_000_000,
            initial_sample_rate_hz: 2_000_000,
            recording_dir: std::env::temp_dir().join("ez-daemon-lifecycle-tests"),
            web_static_dir: std::env::temp_dir().join("ez-daemon-lifecycle-nonexistent-static"),
            auth_token: None,
            receiver_position: None,
            source: SourceConfig::LifecycleTest { probe, fail_start },
        }
    }

    async fn wait_for_reads(probe: &LifecycleTestProbe) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while probe.reads.load(Ordering::Acquire) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fake source did not produce samples");
    }

    async fn wait_for_listener_release(addr: SocketAddr) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(listener) = TcpListener::bind(addr).await {
                    drop(listener);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("listener at {addr} was not released"));
    }

    async fn assert_source_stays_stopped(probe: &LifecycleTestProbe) {
        assert!(!probe.active.load(Ordering::Acquire));
        let reads_after_stop = probe.reads.load(Ordering::Acquire);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(probe.reads.load(Ordering::Acquire), reads_after_stop);
    }

    async fn connect_control_websocket(
        web_addr: SocketAddr,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>> {
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("ws://{web_addr}/ws/control"))
                .await
                .expect("control WebSocket connection failed");
        let welcome = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("control WebSocket did not send Welcome")
            .expect("control WebSocket closed before Welcome")
            .expect("control WebSocket Welcome failed");
        assert!(matches!(
            welcome,
            tokio_tungstenite::tungstenite::Message::Text(_)
        ));
        socket
    }

    #[tokio::test]
    async fn occupied_tcp_port_does_not_start_source() {
        let blocker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let blocked_addr = blocker.local_addr().unwrap();
        let web_addr = free_addr().await;
        let probe = Arc::new(LifecycleTestProbe::default());

        let error = run(
            lifecycle_config(blocked_addr, web_addr, Arc::clone(&probe), false),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("bind TCP listener"));
        assert_eq!(probe.starts.load(Ordering::Acquire), 0);
        assert_eq!(probe.stops.load(Ordering::Acquire), 0);
        wait_for_listener_release(web_addr).await;
    }

    #[tokio::test]
    async fn occupied_web_port_releases_tcp_listener_without_starting_source() {
        let tcp_addr = free_addr().await;
        let blocker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let blocked_web_addr = blocker.local_addr().unwrap();
        let probe = Arc::new(LifecycleTestProbe::default());

        let error = run(
            lifecycle_config(tcp_addr, blocked_web_addr, Arc::clone(&probe), false),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("bind web listener"));
        assert_eq!(probe.starts.load(Ordering::Acquire), 0);
        assert_eq!(probe.stops.load(Ordering::Acquire), 0);
        wait_for_listener_release(tcp_addr).await;
    }

    #[tokio::test]
    async fn source_start_failure_stops_source_and_releases_both_listeners() {
        let tcp_addr = free_addr().await;
        let web_addr = free_addr().await;
        let probe = Arc::new(LifecycleTestProbe::default());

        let error = run(
            lifecycle_config(tcp_addr, web_addr, Arc::clone(&probe), true),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("injected source startup failure"));
        assert_eq!(probe.starts.load(Ordering::Acquire), 1);
        assert_eq!(probe.stops.load(Ordering::Acquire), 1);
        assert_eq!(probe.reads.load(Ordering::Acquire), 0);
        wait_for_listener_release(tcp_addr).await;
        wait_for_listener_release(web_addr).await;
    }

    #[tokio::test]
    async fn running_flag_shutdown_closes_websocket_and_joins_ingestion() {
        let tcp_addr = free_addr().await;
        let web_addr = free_addr().await;
        let probe = Arc::new(LifecycleTestProbe::default());
        let running = Arc::new(AtomicBool::new(true));
        let daemon = tokio::spawn(run(
            lifecycle_config(tcp_addr, web_addr, Arc::clone(&probe), false),
            Arc::clone(&running),
        ));
        wait_for_reads(&probe).await;
        let _socket = connect_control_websocket(web_addr).await;

        running.store(false, Ordering::Release);
        let result = tokio::time::timeout(Duration::from_secs(5), daemon)
            .await
            .expect("daemon did not finish flag-triggered shutdown");
        assert!(result.unwrap().is_ok());
        assert_eq!(probe.stops.load(Ordering::Acquire), 1);
        assert_source_stays_stopped(&probe).await;
        wait_for_listener_release(tcp_addr).await;
        wait_for_listener_release(web_addr).await;
    }

    #[tokio::test]
    async fn aborting_run_stops_source_and_releases_both_listeners() {
        let tcp_addr = free_addr().await;
        let web_addr = free_addr().await;
        let probe = Arc::new(LifecycleTestProbe::default());
        let daemon = tokio::spawn(run(
            lifecycle_config(tcp_addr, web_addr, Arc::clone(&probe), false),
            Arc::new(AtomicBool::new(true)),
        ));
        expect_welcome(tcp_addr, "lifecycle-cancel-test").await;
        wait_for_reads(&probe).await;

        daemon.abort();
        assert!(daemon.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), async {
            while probe.stops.load(Ordering::Acquire) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("cancelled daemon did not stop its source");
        assert_eq!(probe.stops.load(Ordering::Acquire), 1);
        assert_source_stays_stopped(&probe).await;
        wait_for_listener_release(tcp_addr).await;
        wait_for_listener_release(web_addr).await;
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
            auth_token: None,
            receiver_position: None,
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
            auth_token: None,
            receiver_position: None,
            source: SourceConfig::Replay {
                path: fixture.to_str().unwrap().to_string(),
                format: ReplayFormat::Cf32Le,
                looping: true,
                speed: 1000.0,
                amplitude: None,
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
