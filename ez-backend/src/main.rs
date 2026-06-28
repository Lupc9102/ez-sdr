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
    let action = cmd.get("cmd").or(cmd.get("action")).and_then(|v| v.as_str()).unwrap_or("");
    let mut s = state.lock().unwrap();

    match action {
        "tune" | "set_source" => {
            if let Some(hz) = cmd.get("hz").or(cmd.get("frequency_hz")).and_then(|v| v.as_u64()) {
                s.source.frequency_hz = hz;
            }
            if let Some(rate) = cmd.get("sample_rate_hz").and_then(|v| v.as_u64()) {
                s.source.sample_rate_hz = rate as u32;
            }
            if let Some(db) = cmd.get("db").or(cmd.get("gain_db")).and_then(|v| v.as_f64()) {
                s.source.gain_db = db;
            }
            if let Some(bias) = cmd.get("bias_tee").and_then(|v| v.as_bool()) {
                s.source.bias_tee = bias;
            }
            if let Some(ppm) = cmd.get("ppm_correction").and_then(|v| v.as_i64()) {
                s.source.ppm_correction = ppm as i32;
            }
            if let Some(ds) = cmd.get("direct_sampling").and_then(|v| v.as_bool()) {
                s.source.direct_sampling = ds;
            }
        }
        "set_gain" => {
            if let Some(db) = cmd.get("db").or(cmd.get("gain_db")).and_then(|v| v.as_f64()) {
                s.source.gain_db = db;
            }
        }
        "set_sample_rate" => {
            if let Some(rate) = cmd.get("rate").or(cmd.get("sample_rate_hz")).and_then(|v| v.as_u64()) {
                s.source.sample_rate_hz = rate as u32;
            }
        }
        "start_source" => {
            s.source.start();
        }
        "stop_source" => {
            s.source.stop();
        }
        "set_demod" => {
            if let Some(mode) = cmd.get("mode").and_then(|v| v.as_str()) {
                s.demod_mode = mode.to_string();
            }
        }
        "set_filter_bw" => {
            if let Some(bw) = cmd.get("bw").and_then(|v| v.as_u64()) {
                s.filter_bw = bw as u32;
            }
        }
        "set_squelch" => {
            if let Some(level) = cmd.get("level").and_then(|v| v.as_f64()) {
                s.squelch = level as f32;
            }
        }
        "toggle_bias_tee" => {
            if let Some(on) = cmd.get("on").or(cmd.get("bias_tee")).and_then(|v| v.as_bool()) {
                s.source.bias_tee = on;
            }
        }
        "tune_bookmark" => {
            if let Some(hz) = cmd.get("hz").or(cmd.get("frequency_hz")).and_then(|v| v.as_u64()) {
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
        "set_auto_record" => {
            if let Some(v) = cmd.get("value").and_then(|v| v.as_bool()) {
                s.auto_record = v;
            }
        }
        "set_auto_tune" => {
            if let Some(v) = cmd.get("value").and_then(|v| v.as_bool()) {
                s.auto_tune = v;
            }
        }
        "set_live_decode" => {
            if let Some(v) = cmd.get("value").and_then(|v| v.as_bool()) {
                s.live_decode = v;
            }
        }
        "set_observer" => {
            if let Some(lat) = cmd.get("lat").and_then(|v| v.as_f64()) {
                s.observer_lat = lat;
            }
            if let Some(lon) = cmd.get("lon").and_then(|v| v.as_f64()) {
                s.observer_lon = lon;
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
        "refresh_tles" | "refresh_bookmarks" => {
            s.refresh_bookmarks();
        }
        "reset_config" => {
            *s = SharedState::new();
        }
        "ai_query" => {
            println!("[cmd] AI query received (not implemented yet)");
        }
        _ => {
            println!("[cmd] unknown: {action}");
        }
    }
}
