use std::sync::mpsc;
use std::thread;
use tokio::sync::broadcast;

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
}

pub struct StreamState<'a> {
    pub freq_hz: u64,
    pub gain_db: f64,
    pub demod_mode: &'a str,
    pub aircraft_count: usize,
    pub passes: &'a [crate::tle_engine::PassInfo],
    pub squelch: f32,
    pub volume: f32,
    pub recording: bool,
    pub scanner_active: bool,
    pub snr_db: f32,
}

impl WebRemote {
    pub fn new() -> Self {
        Self {
            enabled: false,
            port: 5259,
            tx: None,
            cmd_rx: None,
        }
    }

    pub fn stop(&mut self) {
        self.tx = None;
        self.cmd_rx = None;
    }

    pub fn set_enabled(&mut self, enabled: bool, port: u16) {
        self.enabled = enabled;
        self.port = port;
        self.stop();
        if enabled {
            self.start();
        }
    }

    pub fn start(&mut self) {
        if self.enabled && self.tx.is_none() {
            let (tx, _rx) = broadcast::channel(128);
            self.tx = Some(tx.clone());
            let (cmd_tx, cmd_rx) = mpsc::channel::<RemoteCommand>();
            self.cmd_rx = Some(cmd_rx);

            let port = self.port;

            thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("[web_remote] failed to create tokio runtime: {e}");
                        return;
                    }
                };
                rt.block_on(async move {
                    use axum::{routing::get, Router, extract::State, extract::ws::{WebSocket, WebSocketUpgrade, Message}, response::IntoResponse};

                    async fn ws_handler(ws: WebSocketUpgrade, State(state): State<(broadcast::Sender<String>, mpsc::Sender<RemoteCommand>)>) -> impl IntoResponse {
                        ws.on_upgrade(move |socket| handle_socket(socket, state))
                    }

                    async fn handle_socket(mut socket: WebSocket, state: (broadcast::Sender<String>, mpsc::Sender<RemoteCommand>)) {
                        let (tx, cmd_tx) = state;
                        let mut rx = tx.subscribe();
                        loop {
                            tokio::select! {
                                msg = rx.recv() => {
                                    match msg {
                                        Ok(data) => {
                                            if socket.send(Message::Text(data)).await.is_err() { break; }
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

                    let app = Router::new()
                        .route("/ws", get(ws_handler))
                        .route("/", get(index_handler))
                        .with_state((tx, cmd_tx));

                    let addr = format!("0.0.0.0:{port}");
                    println!("[web_remote] listening on http://{addr}");
                    let listener = match tokio::net::TcpListener::bind(&addr).await {
                        Ok(l) => l,
                        Err(e) => { eprintln!("[web_remote] bind failed on {addr}: {e}"); return; }
                    };
                    if let Err(e) = axum::serve(listener, app).await {
                        eprintln!("[web_remote] server error: {e}");
                    }
                });
            });
        }
    }

    pub fn poll_commands(&mut self) -> Vec<RemoteCommand> {
        let mut cmds = vec![];
        if let Some(rx) = &self.cmd_rx {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        cmds
    }

    pub fn broadcast_state(&mut self, state: &StreamState<'_>) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        match (RemoteCommand::Tune {
            freq_hz: 1090000000,
        }) {
            RemoteCommand::Tune { freq_hz } => assert_eq!(freq_hz, 1090000000),
            _ => panic!("expected Tune variant"),
        }
    }

    #[test]
    fn remote_command_set_gain_variant() {
        match (RemoteCommand::SetGain { gain_db: 42.5 }) {
            RemoteCommand::SetGain { gain_db } => assert!((gain_db - 42.5).abs() < f64::EPSILON),
            _ => panic!("expected SetGain variant"),
        }
    }

    #[test]
    fn remote_command_set_demod_variant() {
        match (RemoteCommand::SetDemod { mode: "AM".into() }) {
            RemoteCommand::SetDemod { mode } => assert_eq!(mode, "AM"),
            _ => panic!("expected SetDemod variant"),
        }
    }

    #[test]
    fn remote_command_set_squelch_variant() {
        match (RemoteCommand::SetSquelch { db: -60.0 }) {
            RemoteCommand::SetSquelch { db } => assert!((db - -60.0).abs() < f32::EPSILON),
            _ => panic!("expected SetSquelch variant"),
        }
    }

    #[test]
    fn remote_command_set_volume_variant() {
        match (RemoteCommand::SetVolume { level: 0.75 }) {
            RemoteCommand::SetVolume { level } => assert!((level - 0.75).abs() < f32::EPSILON),
            _ => panic!("expected SetVolume variant"),
        }
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
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // race window is negligible in practice
        port
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
        let body = resp.text().unwrap();
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
