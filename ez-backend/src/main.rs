use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::tungstenite::protocol::Message;

mod state;
mod source_worker;

use state::SharedState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("EZ_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5347);

    println!("[ez-backend] starting on ws://127.0.0.1:{port}");

    let state = Arc::new(Mutex::new(SharedState::new()));
    let (event_tx, _) = broadcast::channel::<String>(1024);

    // Spawn SDR worker thread (demo mode generates fake IQ)
    source_worker::start(state.clone(), event_tx.clone());

    // Spawn broadcast loop (every ~33ms = ~30Hz sends spectrum + state)
    {
        let state = state.clone();
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(33));
            loop {
                interval.tick().await;
                let snapshot = state.lock().unwrap().snapshot();
                let json = serde_json::to_string(&snapshot).unwrap();
                let _ = event_tx.send(json);
            }
        });
    }

    // WebSocket server
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("[ez-backend] listening");

    while let Ok((stream, addr)) = listener.accept().await {
        println!("[ez-backend] client connected: {addr}");
        let state = state.clone();
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut write, mut read) = ws_stream.split();
            let mut rx = event_tx.subscribe();

            // Reader: handle incoming commands
            let state_r = state.clone();
            let read_task = tokio::spawn(async move {
                while let Some(Ok(msg)) = read.next().await {
                    if let Message::Text(text) = msg {
                        if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text) {
                            handle_command(&state_r, &cmd);
                        }
                    }
                }
            });

            // Writer: push events to client
            let write_task = tokio::spawn(async move {
                while let Ok(payload) = rx.recv().await {
                    if write.send(Message::Text(payload)).await.is_err() {
                        break;
                    }
                }
            });

            let _ = tokio::join!(read_task, write_task);
        });
    }

    Ok(())
}

fn handle_command(state: &Arc<Mutex<SharedState>>, cmd: &serde_json::Value) {
    let action = cmd.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let mut s = state.lock().unwrap();

    match action {
        "tune" => {
            if let Some(hz) = cmd.get("hz").and_then(|v| v.as_u64()) {
                s.source.frequency_hz = hz;
                println!("[cmd] tune {hz} Hz");
            }
        }
        "set_gain" => {
            if let Some(db) = cmd.get("db").and_then(|v| v.as_f64()) {
                s.source.gain_db = db;
                println!("[cmd] gain {db} dB");
            }
        }
        "set_sample_rate" => {
            if let Some(rate) = cmd.get("rate").and_then(|v| v.as_u64()) {
                s.source.sample_rate_hz = rate as u32;
            }
        }
        "start_source" => {
            s.source.start();
            println!("[cmd] source started");
        }
        "stop_source" => {
            s.source.stop();
            println!("[cmd] source stopped");
        }
        "set_demod" => {
            if let Some(mode) = cmd.get("mode").and_then(|v| v.as_str()) {
                s.demod_mode = mode.to_string();
            }
        }
        "toggle_bias_tee" => {
            if let Some(on) = cmd.get("on").and_then(|v| v.as_bool()) {
                s.source.bias_tee = on;
            }
        }
        "tune_bookmark" => {
            if let Some(hz) = cmd.get("hz").and_then(|v| v.as_u64()) {
                s.source.frequency_hz = hz;
            }
        }
        "start_recording" => {
            s.start_recording();
        }
        "stop_recording" => {
            s.stop_recording();
        }
        "select_satellite" => {
            if let Some(name) = cmd.get("name").and_then(|v| v.as_str()) {
                s.selected_satellite = Some(name.to_string());
            }
        }
        "start_adsb" => {
            s.source.frequency_hz = 1_090_000_000;
            s.source.sample_rate_hz = 2_048_000;
            s.start_adsb();
        }
        "stop_adsb" => {
            s.stop_adsb();
        }
        "refresh_tles" => {
            s.refresh_bookmarks();
        }
        "set_observer" => {
            // Observer settings stored for future TLE propagation
        }
        _ => {
            println!("[cmd] unknown: {action}");
        }
    }
}
