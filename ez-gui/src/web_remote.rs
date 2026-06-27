use tokio::sync::broadcast;
use std::sync::mpsc;
use std::thread;

pub enum RemoteCommand {
    Tune { freq_hz: u64 },
    SetGain { gain_db: f64 },
    StartRecord,
    StopRecord,
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

    pub fn set_enabled(&mut self, enabled: bool, port: u16) {
        self.enabled = enabled;
        self.port = port;
        if enabled {
            self.start();
        }
    }

    pub fn start(&mut self) {
        if self.enabled && self.tx.is_none() {
            let (tx, _rx) = broadcast::channel(128);
            self.tx = Some(tx.clone());
            let (_cmd_tx, cmd_rx) = mpsc::channel::<RemoteCommand>();
            self.cmd_rx = Some(cmd_rx);

            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    use axum::{routing::get, Router, extract::State, extract::ws::{WebSocket, WebSocketUpgrade, Message}, response::IntoResponse};

                    async fn ws_handler(ws: WebSocketUpgrade, State(tx): State<broadcast::Sender<String>>) -> impl IntoResponse {
                        ws.on_upgrade(move |socket| handle_socket(socket, tx))
                    }

                    async fn handle_socket(mut socket: WebSocket, tx: broadcast::Sender<String>) {
                        let mut rx = tx.subscribe();
                        loop {
                            tokio::select! {
                                msg = rx.recv() => {
                                    if let Ok(data) = msg {
                                        if socket.send(Message::Text(data.into())).await.is_err() { break; }
                                    }
                                }
                                Some(Ok(msg)) = socket.recv() => {
                                    match msg {
                                        Message::Text(_text) => {}
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }

                    async fn index_handler() -> &'static str {
                        "EZ-SDR Web Remote"
                    }

                    let app = Router::new()
                        .route("/ws", get(ws_handler))
                        .route("/", get(index_handler))
                        .with_state(tx);

                    let addr = format!("0.0.0.0:{}", 5259);
                    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
                    axum::serve(listener, app).await.unwrap();
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

    pub fn broadcast_state(&mut self, freq_hz: u64, gain_db: f64, demod_mode: &str, aircraft_count: usize, passes: &[crate::tle_engine::PassInfo]) {
        if let Some(tx) = &self.tx {
            let state = serde_json::json!({
                "frequency_hz": freq_hz,
                "gain_db": gain_db,
                "demod_mode": demod_mode,
                "aircraft_count": aircraft_count,
                "upcoming_passes": passes.len(),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            let _ = tx.send(state.to_string());
        }
    }
}
