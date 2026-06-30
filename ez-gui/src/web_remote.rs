use tokio::sync::broadcast;
use std::sync::mpsc;
use std::thread;

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
                    Err(e) => { eprintln!("[web_remote] failed to create tokio runtime: {e}"); return; }
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
                                            if socket.send(Message::Text(data.into())).await.is_err() { break; }
                                        }
                                        Err(_) => break,
                                    }
                                }
                                Some(Ok(msg)) = socket.recv() => {
                                    match msg {
                                        Message::Text(text) => {
                                            if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text) {
                                                let action = cmd.get("action").and_then(|v| v.as_str()).unwrap_or("");
                                                match action {
                                                    "tune" => {
                                                        if let Some(hz) = cmd.get("hz").and_then(|v| v.as_u64()) {
                                                            let _ = cmd_tx.send(RemoteCommand::Tune { freq_hz: hz });
                                                        }
                                                    }
                                                    "set_gain" => {
                                                        if let Some(db) = cmd.get("db").and_then(|v| v.as_f64()) {
                                                            let _ = cmd_tx.send(RemoteCommand::SetGain { gain_db: db });
                                                        }
                                                    }
                                                    "set_demod" => {
                                                        if let Some(mode) = cmd.get("mode").and_then(|v| v.as_str()) {
                                                            let _ = cmd_tx.send(RemoteCommand::SetDemod { mode: mode.to_string() });
                                                        }
                                                    }
                                                    "set_squelch" => {
                                                        if let Some(db) = cmd.get("db").and_then(|v| v.as_f64()) {
                                                            let _ = cmd_tx.send(RemoteCommand::SetSquelch { db: db as f32 });
                                                        }
                                                    }
                                                    "set_volume" => {
                                                        if let Some(level) = cmd.get("level").and_then(|v| v.as_f64()) {
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
                                        _ => {}
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

                    let addr = format!("0.0.0.0:{}", port);
                    println!("[web_remote] listening on http://{}", addr);
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

    #[allow(clippy::too_many_arguments)]
    pub fn broadcast_state(
        &mut self,
        freq_hz: u64,
        gain_db: f64,
        demod_mode: &str,
        aircraft_count: usize,
        passes: &[crate::tle_engine::PassInfo],
        squelch: f32,
        volume: f32,
        recording: bool,
        scanner_active: bool,
        snr_db: f32,
    ) {
        let tx = match &self.tx {
            Some(t) if t.receiver_count() > 0 => t,
            _ => return,
        };
        let state = serde_json::json!({
            "frequency_hz": freq_hz,
            "gain_db": gain_db,
            "demod_mode": demod_mode,
            "aircraft_count": aircraft_count,
            "squelch_db": squelch,
            "volume": volume,
            "recording": recording,
            "scanner_active": scanner_active,
            "snr_db": snr_db,
            "upcoming_passes": passes.iter().map(|p| serde_json::json!({
                "satellite": p.satellite,
                "aos": p.aos,
                "los": p.los,
                "max_elevation": p.max_elevation,
            })).collect::<Vec<_>>(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        let _ = tx.send(state.to_string());
    }
}
