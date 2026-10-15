use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use std::future::IntoFuture;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::{broadcast, watch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenerState {
    Stopped,
    Starting,
    Listening(String),
    Stopping,
    Failed(String),
}

impl std::fmt::Display for ListenerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => f.write_str("Stopped"),
            Self::Starting => f.write_str("Starting"),
            Self::Listening(endpoint) => write!(f, "Listening on {endpoint}"),
            Self::Stopping => f.write_str("Stopping"),
            Self::Failed(error) => write!(f, "Failed: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerSnapshot {
    pub generation: u64,
    pub state: ListenerState,
}

impl Default for ListenerSnapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            state: ListenerState::Stopped,
        }
    }
}

pub(crate) fn publish_listener_state(
    status: &Arc<Mutex<ListenerSnapshot>>,
    generation: u64,
    state: ListenerState,
) {
    let mut snapshot = status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if snapshot.generation == generation {
        snapshot.state = state;
    }
}

pub enum RemoteCommand {
    Tune { freq_hz: u64 },
    SetGain { gain_db: f64 },
    SetDemod { mode: String },
    SetSquelch { db: f32 },
    SetVolume { level: f32 },
    StartRecord,
    StopRecord,
    StartScan,
    StopScan,
}

pub struct WebRemote {
    pub enabled: bool,
    pub port: u16,
    pub tx: Option<broadcast::Sender<String>>,
    pub cmd_rx: Option<mpsc::Receiver<RemoteCommand>>,
    // Active and retired workers are reaped only after is_finished() so service
    // changes never join a network worker on the GUI thread.
    join: Option<thread::JoinHandle<()>>,
    shutdown: Option<watch::Sender<bool>>,
    retired: Vec<thread::JoinHandle<()>>,
    status: Arc<Mutex<ListenerSnapshot>>,
    generation: u64,
}

pub struct StreamState<'a> {
    pub freq_hz: u64,
    pub gain_db: f64,
    pub demod_mode: &'a str,
    pub aircraft_count: usize,
    pub passes: &'a [crate::tle_engine::PassInfo],
    pub squelch: f32,
    pub squelch_pending: Option<f32>,
    pub squelch_error: Option<&'a str>,
    pub squelch_applied: bool,
    pub volume: f32,
    pub recording: bool,
    pub scanner_active: bool,
    pub snr_db: f32,
}

impl<'a> StreamState<'a> {
    pub(crate) fn from_receiver(
        state: &'a crate::app::SharedState,
        aircraft_count: usize,
        passes: &'a [crate::tle_engine::PassInfo],
        scanner_active: bool,
        snr_db: f32,
    ) -> Self {
        Self {
            freq_hz: state.source.frequency_hz,
            gain_db: state.source.gain_db,
            demod_mode: state.demod_mode.label(),
            aircraft_count,
            passes,
            squelch: state.squelch,
            squelch_pending: state.source.daemon_squelch_pending_db(),
            squelch_error: state.source.daemon_squelch_error.as_deref(),
            squelch_applied: state.source.source_mode != crate::source_manager::SourceMode::Daemon
                || state.source.daemon_squelch_applied().is_some(),
            volume: state.volume,
            recording: state.recording,
            scanner_active,
            snr_db,
        }
    }
}

/// Whether a WebSocket `Origin` header value is allowed to drive the remote.
///
/// Only loopback origins are accepted, matched on exact host (with optional
/// `:port`) — never by prefix, so `http://localhost.evil.com` does not pass.
/// `null` (sandboxed iframe / `file://`) is rejected: this is an
/// unauthenticated control plane (tune/record/scan), and any foreign page
/// that can reach it is a cross-site hijack. Non-browser clients send no
/// `Origin` at all and are unaffected (see `ws_handler`).
fn origin_allowed(origin: &str) -> bool {
    for scheme in ["http://", "https://"] {
        if let Some(rest) = origin.strip_prefix(scheme) {
            let authority = rest.split('/').next().unwrap_or("");
            // Strip possible `user@` info so `http://localhost@evil.com`
            // cannot smuggle a foreign host past the check.
            let host = authority.split('@').next_back().unwrap_or("");
            if host == "localhost" || host == "127.0.0.1" || host == "[::1]" {
                return true;
            }
            if let Some((h, p)) = host.rsplit_once(':') {
                if (h == "localhost" || h == "127.0.0.1" || h == "[::1]")
                    && p.parse::<u16>().is_ok()
                {
                    return true;
                }
            }
            return false;
        }
    }
    false
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    State(state): State<(broadcast::Sender<String>, mpsc::Sender<RemoteCommand>)>,
) -> axum::response::Response {
    // Mitigate cross-site WebSocket hijacking: browsers send `Origin` on WS
    // upgrades. Reject non-loopback upgrades so a malicious web page can't
    // drive the SDR (tune/record/etc.) via the victim's browser. Non-browser
    // clients send no Origin and are allowed.
    if let Some(origin) = headers.get(axum::http::header::ORIGIN) {
        if let Ok(origin_str) = origin.to_str() {
            if !origin_allowed(origin_str) {
                return (axum::http::StatusCode::FORBIDDEN, "Cross-origin rejected")
                    .into_response();
            }
        }
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
}

async fn handle_socket(
    mut socket: WebSocket,
    state: (broadcast::Sender<String>, mpsc::Sender<RemoteCommand>),
) {
    let (tx, cmd_tx) = state;
    let mut rx = tx.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(data) => {
                        if socket.send(Message::Text(data.into())).await.is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
            Some(Ok(msg)) = socket.recv() => {
                if let Message::Text(text) = msg {
                    if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text) {
                        let action = cmd.get("action").and_then(|v| v.as_str()).unwrap_or("");
                        match action {
                            "tune" => {
                                if let Some(hz) = cmd.get("hz").and_then(serde_json::Value::as_u64) {
                                    let _ = cmd_tx.send(RemoteCommand::Tune { freq_hz: hz });
                                }
                            }
                            "set_gain" => {
                                if let Some(db) = cmd.get("db").and_then(serde_json::Value::as_f64) {
                                    let _ = cmd_tx.send(RemoteCommand::SetGain { gain_db: db });
                                }
                            }
                            "set_demod" => {
                                if let Some(mode) = cmd.get("mode").and_then(|v| v.as_str()) {
                                    let _ = cmd_tx.send(RemoteCommand::SetDemod { mode: mode.to_string() });
                                }
                            }
                            "set_squelch" => {
                                if let Some(db) = cmd.get("db").and_then(serde_json::Value::as_f64) {
                                    let _ = cmd_tx.send(RemoteCommand::SetSquelch { db: db as f32 });
                                }
                            }
                            "set_volume" => {
                                if let Some(level) = cmd.get("level").and_then(serde_json::Value::as_f64) {
                                    let _ = cmd_tx.send(RemoteCommand::SetVolume { level: level as f32 });
                                }
                            }
                            "start_record" => { let _ = cmd_tx.send(RemoteCommand::StartRecord); }
                            "stop_record" => { let _ = cmd_tx.send(RemoteCommand::StopRecord); }
                            "start_scan" => { let _ = cmd_tx.send(RemoteCommand::StartScan); }
                            "stop_scan" => { let _ = cmd_tx.send(RemoteCommand::StopScan); }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

async fn index_handler() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("web_remote.html"))
}

impl Default for WebRemote {
    fn default() -> Self {
        Self::new()
    }
}

impl WebRemote {
    pub fn new() -> Self {
        Self {
            enabled: false,
            port: 5259,
            tx: None,
            cmd_rx: None,
            join: None,
            shutdown: None,
            retired: Vec::new(),
            status: Arc::new(Mutex::new(ListenerSnapshot::default())),
            generation: 0,
        }
    }

    pub fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.retire_active();
        let state = if self.retired.is_empty() {
            ListenerState::Stopped
        } else {
            ListenerState::Stopping
        };
        self.set_state(generation, state);
        self.reap_finished();
    }

    fn retire_active(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(true);
        }
        self.tx = None;
        self.cmd_rx = None;
        if let Some(handle) = self.join.take() {
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                self.retired.push(handle);
            }
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, port: u16) {
        self.reap_finished();
        let unchanged = self.enabled == enabled && self.port == port;
        if unchanged && (!enabled || self.current_worker_healthy()) {
            return;
        }
        self.enabled = enabled;
        self.port = port;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.retire_active();
        if enabled {
            self.set_state(generation, ListenerState::Starting);
            self.start_generation(generation);
        } else {
            let state = if self.retired.is_empty() {
                ListenerState::Stopped
            } else {
                ListenerState::Stopping
            };
            self.set_state(generation, state);
        }
    }

    pub fn start(&mut self) {
        if self.enabled {
            self.set_enabled(true, self.port);
        }
    }

    fn current_worker_healthy(&self) -> bool {
        let worker_alive = self.join.as_ref().is_some_and(|join| !join.is_finished());
        let state = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .state
            .clone();
        worker_alive && matches!(state, ListenerState::Starting | ListenerState::Listening(_))
    }

    fn set_state(&self, generation: u64, state: ListenerState) {
        let mut snapshot = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot.generation = generation;
        snapshot.state = state;
    }

    fn reap_finished(&mut self) {
        let mut pending = Vec::with_capacity(self.retired.len());
        let mut retired_panicked = false;
        for handle in self.retired.drain(..) {
            if handle.is_finished() {
                retired_panicked |= handle.join().is_err();
            } else {
                pending.push(handle);
            }
        }
        self.retired = pending;
        if self.retired.is_empty() {
            let mut snapshot = self
                .status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if snapshot.generation == self.generation && snapshot.state == ListenerState::Stopping {
                snapshot.state = if retired_panicked {
                    ListenerState::Failed("listener worker stopped unexpectedly".into())
                } else {
                    ListenerState::Stopped
                };
            }
        }
        if self
            .join
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            let handle = self.join.take().expect("finished worker handle");
            if handle.join().is_err() {
                let state = self
                    .status
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .state
                    .clone();
                if matches!(state, ListenerState::Starting | ListenerState::Listening(_)) {
                    self.set_state(
                        self.generation,
                        ListenerState::Failed("listener worker stopped unexpectedly".into()),
                    );
                }
            }
        }
    }

    pub fn status_snapshot(&mut self) -> ListenerSnapshot {
        self.reap_finished();
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn start_generation(&mut self, generation: u64) {
        let (tx, _rx) = broadcast::channel(128);
        let tx_worker = tx.clone();
        self.tx = Some(tx);
        let (cmd_tx, cmd_rx) = mpsc::channel::<RemoteCommand>();
        self.cmd_rx = Some(cmd_rx);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.shutdown = Some(shutdown_tx);
        let status = Arc::clone(&self.status);
        let port = self.port;
        let handle = thread::Builder::new()
            .name("ez-sdr-web-remote".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(error) => {
                        publish_listener_state(
                            &status,
                            generation,
                            ListenerState::Failed(format!(
                                "runtime initialization failed: {error}"
                            )),
                        );
                        return;
                    }
                };
                rt.block_on(async move {
                    let app = Router::new()
                        .route("/ws", get(ws_handler))
                        .route("/", get(index_handler))
                        .with_state((tx_worker, cmd_tx));

                    // Bind loopback only: exposing unauthenticated tune/record
                    // control on 0.0.0.0 is RCE-adjacent on hostile LANs.
                    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                        Ok(listener) => listener,
                        Err(error) => {
                            publish_listener_state(
                                &status,
                                generation,
                                ListenerState::Failed(format!("127.0.0.1:{port}: {error}")),
                            );
                            return;
                        }
                    };
                    let endpoint = listener
                        .local_addr()
                        .map(|address| address.to_string())
                        .unwrap_or_else(|_| format!("127.0.0.1:{port}"));
                    publish_listener_state(&status, generation, ListenerState::Listening(endpoint));
                    let graceful_shutdown_rx = shutdown_rx.clone();
                    let server = axum::serve(listener, app)
                        .with_graceful_shutdown(async move {
                            let mut rx = graceful_shutdown_rx;
                            loop {
                                if *rx.borrow() {
                                    break;
                                }
                                if rx.changed().await.is_err() {
                                    break;
                                }
                            }
                        })
                        .into_future();
                    tokio::pin!(server);
                    let mut shutdown_timeout_rx = shutdown_rx;
                    let result = tokio::select! {
                        result = &mut server => Some(result),
                        _ = async {
                            loop {
                                if *shutdown_timeout_rx.borrow() {
                                    break;
                                }
                                if shutdown_timeout_rx.changed().await.is_err() {
                                    break;
                                }
                            }
                        } => tokio::time::timeout(
                            std::time::Duration::from_millis(500),
                            &mut server,
                        ).await.ok(),
                    };
                    match result {
                        Some(Ok(())) | None => {
                            publish_listener_state(&status, generation, ListenerState::Stopped);
                        }
                        Some(Err(error)) => {
                            publish_listener_state(
                                &status,
                                generation,
                                ListenerState::Failed(format!("server error: {error}")),
                            );
                        }
                    }
                });
            });
        match handle {
            Ok(handle) => self.join = Some(handle),
            Err(error) => {
                self.tx = None;
                self.cmd_rx = None;
                self.shutdown = None;
                self.set_state(
                    generation,
                    ListenerState::Failed(format!("worker spawn failed: {error}")),
                );
            }
        }
    }

    pub fn poll_commands(&mut self) -> Vec<RemoteCommand> {
        self.reap_finished();
        let mut cmds = vec![];
        if let Some(rx) = &self.cmd_rx {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        cmds
    }

    pub fn broadcast_command_error(&self, action: &str, message: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(
                serde_json::json!({"type":"command_error", "action":action, "message":message})
                    .to_string(),
            );
        }
    }

    pub fn broadcast_state(&mut self, state: &StreamState<'_>) {
        self.reap_finished();
        let tx = match &self.tx {
            Some(t) if t.receiver_count() > 0 => t,
            _ => return,
        };
        let json = serde_json::json!({
            "frequency_hz": state.freq_hz,
            "gain_db": state.gain_db,
            "demod_mode": state.demod_mode,
            "aircraft_count": state.aircraft_count,
            "squelch_db": state.squelch,
            "squelch_pending_db": state.squelch_pending,
            "squelch_error": state.squelch_error,
            "squelch_applied": state.squelch_applied,
            "volume": state.volume,
            "recording": state.recording,
            "scanner_active": state.scanner_active,
            "snr_db": state.snr_db,
            "upcoming_passes": state.passes.iter().map(|p| serde_json::json!({
                "satellite": p.satellite,
                "aos": p.aos,
                "los": p.los,
                "max_elevation": p.max_elevation,
            })).collect::<Vec<_>>(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        let _ = tx.send(json.to_string());
    }
}

impl Drop for WebRemote {
    fn drop(&mut self) {
        // Best-effort cleanup on shutdown if stop() wasn't called explicitly.
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[test]
    fn origin_allows_loopback_with_any_port() {
        assert!(origin_allowed("http://localhost:5259"));
        assert!(origin_allowed("http://127.0.0.1:5259"));
        assert!(origin_allowed("https://localhost"));
        assert!(origin_allowed("http://[::1]:8080"));
        assert!(origin_allowed("http://localhost/app/index.html"));
    }

    #[test]
    fn origin_rejects_lookalikes_and_foreign() {
        // Prefix-match bypasses that `starts_with("http://localhost")` missed.
        assert!(!origin_allowed("http://localhost.evil.com"));
        assert!(!origin_allowed("http://localhost.evil.com:80"));
        assert!(!origin_allowed("http://127.0.0.1.evil.com"));
        assert!(!origin_allowed("http://localhost@evil.com"));
        assert!(!origin_allowed("https://example.com"));
        assert!(!origin_allowed("null"));
        assert!(!origin_allowed(""));
        assert!(!origin_allowed("file://localhost"));
        assert!(!origin_allowed("http://localhost:99999"));
        assert!(!origin_allowed("http://localhost:notaport"));
    }

    #[test]
    fn new_creates_disabled_instance() {
        let wr = WebRemote::new();
        assert!(!wr.enabled);
        assert_eq!(wr.port, 5259);
        assert!(wr.tx.is_none());
        assert!(wr.cmd_rx.is_none());
    }

    #[test]
    fn stop_clears_channels() {
        let mut wr = WebRemote::new();
        wr.tx = Some(broadcast::channel(8).0);
        let (_tx, rx) = mpsc::channel();
        wr.cmd_rx = Some(rx);
        wr.stop();
        assert!(wr.tx.is_none());
        assert!(wr.cmd_rx.is_none());
    }

    #[test]
    fn set_enabled_false_stops() {
        let mut wr = WebRemote::new();
        wr.set_enabled(false, 5259);
        assert!(!wr.enabled);
        assert_eq!(wr.port, 5259);
        assert!(wr.tx.is_none());
    }

    #[test]
    fn no_crash_poll_without_channel() {
        let mut wr = WebRemote::new();
        assert!(wr.poll_commands().is_empty());
    }

    #[test]
    fn no_crash_broadcast_without_listeners() {
        let mut wr = WebRemote::new();
        let s = StreamState {
            freq_hz: 1090000000,
            gain_db: 40.0,
            demod_mode: "RAW",
            aircraft_count: 5,
            passes: &[],
            squelch: 0.0,
            squelch_pending: None,
            squelch_error: None,
            squelch_applied: true,
            volume: 0.8,
            recording: false,
            scanner_active: false,
            snr_db: 12.0,
        };
        wr.broadcast_state(&s);
        // No listeners — should silently return without panicking.
    }

    #[test]
    fn set_enabled_true_sets_fields() {
        let mut wr = WebRemote::new();
        wr.set_enabled(true, 0);
        assert!(wr.enabled);
        assert_eq!(wr.port, 0);
        // tx and cmd_rx may or may not be set depending on thread success
    }

    #[test]
    fn set_enabled_toggle_re_enables() {
        let mut wr = WebRemote::new();
        wr.set_enabled(true, 0);
        wr.set_enabled(false, 0);
        assert!(!wr.enabled);
        assert!(wr.tx.is_none());
        assert!(wr.cmd_rx.is_none());
        wr.set_enabled(true, 0);
        assert!(wr.enabled);
        assert_eq!(wr.port, 0);
    }

    #[test]
    fn remote_command_tune_variant() {
        let cmd = RemoteCommand::Tune {
            freq_hz: 1090000000,
        };
        assert!(matches!(cmd, RemoteCommand::Tune { freq_hz } if freq_hz == 1090000000));
    }

    #[test]
    fn remote_command_set_gain_variant() {
        let cmd = RemoteCommand::SetGain { gain_db: 42.5 };
        assert!(
            matches!(cmd, RemoteCommand::SetGain { gain_db } if (gain_db - 42.5).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn remote_command_set_demod_variant() {
        let cmd = RemoteCommand::SetDemod { mode: "AM".into() };
        assert!(matches!(cmd, RemoteCommand::SetDemod { ref mode } if mode == "AM"));
    }

    #[test]
    fn remote_command_set_squelch_variant() {
        let cmd = RemoteCommand::SetSquelch { db: -60.0 };
        assert!(
            matches!(cmd, RemoteCommand::SetSquelch { db } if (db - -60.0).abs() < f32::EPSILON)
        );
    }

    #[test]
    fn remote_command_set_volume_variant() {
        let cmd = RemoteCommand::SetVolume { level: 0.75 };
        assert!(
            matches!(cmd, RemoteCommand::SetVolume { level } if (level - 0.75).abs() < f32::EPSILON)
        );
    }

    #[test]
    fn remote_command_unit_variants() {
        assert!(matches!(
            RemoteCommand::StartRecord,
            RemoteCommand::StartRecord
        ));
        assert!(matches!(
            RemoteCommand::StopRecord,
            RemoteCommand::StopRecord
        ));
        assert!(matches!(RemoteCommand::StartScan, RemoteCommand::StartScan));
        assert!(matches!(RemoteCommand::StopScan, RemoteCommand::StopScan));
    }

    #[test]
    fn broadcast_state_with_listener_no_crash() {
        let mut wr = WebRemote::new();
        let (tx, rx) = broadcast::channel(128);
        let _rx = rx; // keep rx alive so sender has receivers
        wr.tx = Some(tx);
        let s = StreamState {
            freq_hz: 1090000000,
            gain_db: 40.0,
            demod_mode: "RAW",
            aircraft_count: 5,
            passes: &[],
            squelch: 0.0,
            squelch_pending: None,
            squelch_error: None,
            squelch_applied: true,
            volume: 0.8,
            recording: false,
            scanner_active: false,
            snr_db: 12.0,
        };
        wr.broadcast_state(&s);
        drop(_rx);
    }

    #[test]
    fn poll_commands_after_stop_is_empty() {
        let mut wr = WebRemote::new();
        wr.set_enabled(true, 0);
        wr.set_enabled(false, 0);
        assert!(wr.poll_commands().is_empty());
    }

    /// Bind to a random OS-assigned port, hand the port to the WebRemote,
    /// and wait briefly for the server thread to start.
    fn start_on_random_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("test server should bind to random port");
        let port = listener
            .local_addr()
            .expect("bound listener should report its port")
            .port();
        drop(listener); // race window is negligible in practice
        port
    }

    fn wait_for_state(
        server: &mut WebRemote,
        predicate: impl Fn(&ListenerState) -> bool,
    ) -> ListenerSnapshot {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let snapshot = server.status_snapshot();
            if predicate(&snapshot.state) {
                return snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "listener state timed out: {snapshot:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn test_stream_state() -> StreamState<'static> {
        StreamState {
            freq_hz: 118_700_000,
            gain_db: 18.0,
            demod_mode: "AM",
            aircraft_count: 0,
            passes: &[],
            squelch: 0.0,
            squelch_pending: None,
            squelch_error: None,
            squelch_applied: true,
            volume: 0.5,
            recording: false,
            scanner_active: false,
            snr_db: 0.0,
        }
    }

    #[tokio::test]
    async fn occupied_port_recovers_and_healthy_apply_preserves_websocket() {
        let blocker = std::net::TcpListener::bind("127.0.0.1:0").expect("occupy loopback port");
        let port = blocker.local_addr().unwrap().port();
        let mut wr = WebRemote::new();
        wr.set_enabled(true, port);
        let failed = wait_for_state(&mut wr, |state| matches!(state, ListenerState::Failed(_)));
        assert!(failed.state.to_string().contains(&port.to_string()));

        drop(blocker);
        wr.set_enabled(true, port);
        let listening = wait_for_state(&mut wr, |state| {
            matches!(state, ListenerState::Listening(_))
        });
        assert!(listening.state.to_string().contains(&port.to_string()));
        let endpoint = format!("127.0.0.1:{port}");
        assert_eq!(listening.state, ListenerState::Listening(endpoint));

        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
            .await
            .expect("connect WebSocket client");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while wr.tx.as_ref().is_none_or(|tx| tx.receiver_count() == 0)
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(wr.tx.as_ref().is_some_and(|tx| tx.receiver_count() > 0));
        // A prior implementation wrapped the entire server future in the
        // shutdown timeout and silently stopped listening after 500 ms.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert!(matches!(
            wr.status_snapshot().state,
            ListenerState::Listening(_)
        ));
        let generation = wr.status_snapshot().generation;
        wr.set_enabled(true, port);
        assert_eq!(
            wr.status_snapshot().generation,
            generation,
            "healthy apply must not replace worker"
        );
        wr.broadcast_state(&test_stream_state());
        let message = tokio::time::timeout(std::time::Duration::from_secs(1), socket.next())
            .await
            .expect("state should reach existing WebSocket")
            .expect("WebSocket should remain open")
            .expect("read WebSocket state");
        assert!(message.into_text().unwrap().contains("118700000"));

        // Keep the client open and stop reading. Broadcast remains bounded by
        // the existing 128-message channel, and shutdown has a finite grace.
        for _ in 0..256 {
            wr.broadcast_state(&test_stream_state());
        }
        let started = std::time::Instant::now();
        wr.stop();
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        wait_for_state(&mut wr, |state| *state == ListenerState::Stopped);
        drop(socket);
    }

    #[test]
    fn changed_port_releases_previous_listener() {
        let first_port = start_on_random_port();
        let second_port = start_on_random_port();
        let mut wr = WebRemote::new();
        wr.set_enabled(true, first_port);
        wait_for_state(&mut wr, |state| {
            matches!(state, ListenerState::Listening(_))
        });
        wr.set_enabled(true, second_port);
        let snapshot = wait_for_state(
            &mut wr,
            |state| matches!(state, ListenerState::Listening(endpoint) if endpoint.ends_with(&second_port.to_string())),
        );
        assert!(snapshot
            .state
            .to_string()
            .contains(&second_port.to_string()));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::net::TcpListener::bind(("127.0.0.1", first_port)).is_err()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(std::net::TcpListener::bind(("127.0.0.1", first_port)).is_ok());
        wr.stop();
        wait_for_state(&mut wr, |state| *state == ListenerState::Stopped);
    }

    #[test]
    fn repeated_enable_disable_releases_each_listener() {
        let mut wr = WebRemote::new();
        for _ in 0..4 {
            wr.set_enabled(true, 0);
            let snapshot = wait_for_state(&mut wr, |state| {
                matches!(state, ListenerState::Listening(_))
            });
            let ListenerState::Listening(endpoint) = snapshot.state else {
                unreachable!("wait returned a listening state")
            };
            let port = endpoint.rsplit(':').next().unwrap().parse::<u16>().unwrap();
            wr.set_enabled(false, 0);
            wait_for_state(&mut wr, |state| *state == ListenerState::Stopped);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
        }
    }

    #[test]
    fn serve_index_via_http() {
        let port = start_on_random_port();
        let mut wr = WebRemote::new();
        wr.set_enabled(true, port);
        // give the async server thread time to bind
        std::thread::sleep(std::time::Duration::from_millis(200));

        let url = format!("http://127.0.0.1:{port}/");
        let resp = reqwest::blocking::get(&url).expect("HTTP request should succeed");
        assert_eq!(resp.status(), 200);
        let body = resp.text().expect("HTTP response body should be readable");
        assert!(body.contains("SDR"), "response should contain page content");
        drop(wr);
    }

    #[test]
    fn serve_index_rejects_bad_path() {
        let port = start_on_random_port();
        let mut wr = WebRemote::new();
        wr.set_enabled(true, port);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let url = format!("http://127.0.0.1:{port}/nonexistent");
        let resp = reqwest::blocking::get(&url).expect("HTTP request should succeed");
        assert_eq!(resp.status(), 404);
        drop(wr);
    }
}
