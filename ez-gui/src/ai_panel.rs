use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};

use crate::app::SharedState;
use crate::config::{DEFAULT_AI_MODEL, PROVIDER_PRESETS};

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub streaming: bool,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

const DEFAULT_SYSTEM: &str = "You are EZ-SDR AI Agent, an expert radio and satellite assistant. \
You can control the SDR by responding with a JSON tool call block.

Available tools:
- tune_frequency(hz: u64) — Set center frequency
- set_gain(db: f64) — Set tuner gain (0–100)
- set_demod(mode: string) — AM, FM, WFM, LSB, USB, RAW
- set_sample_rate(rate: u32) — Sample rate in Hz
- toggle_bias_tee(on: bool) — Enable/disable bias tee
- start_recording() / stop_recording()
- select_satellite(name: string) — NOAA 15/18/19, Meteor-M2, ISS
- set_squelch(db: f64) — Set squelch threshold in dB (e.g. -60.0)
- set_volume(level: f64) — Set audio volume 0.0–1.0
- start_adsb() / stop_adsb()
- get_status() — Return full JSON of current SDR state

When you want to call a tool respond with exactly:
{\"tool\": \"name\", \"args\": {}}
You may call multiple tools sequentially. Always explain what you are doing.";

// Streaming state sent from worker thread
enum StreamEvent {
    Chunk(String),
    ToolCallDetected { tool: String, args: serde_json::Value },
    Done(#[allow(dead_code)] String),
    Error(String),
}

pub struct AiPanel {
    shared: Arc<Mutex<SharedState>>,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub thinking: bool,
    temperature: f64,
    pending_rx: Option<crossbeam_channel::Receiver<StreamEvent>>,
}

impl AiPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            messages: Vec::new(),
            input: String::new(),
            thinking: false,
            temperature: 0.7,
            pending_rx: None,
        }
    }

    /// Build the messages JSON array for the API call, injecting system prompt.
    fn build_api_messages(&self) -> (Vec<serde_json::Value>, String, String, String, String, u32, f64, String) {
        let (endpoint, model, api_key, provider, max_tokens, temperature, system_prompt, freq, rate, gain, mode, recording, sat, adsb, noise_floor, peak_db, squelch) = {
            if let Ok(state) = self.shared.try_lock() {
                let cfg = &state.config;
                let mode_label = state.demod_mode.label().to_string();
                (
                    cfg.ai_endpoint.clone(),
                    cfg.ai_model.clone(),
                    cfg.ai_api_key.clone(),
                    cfg.ai_provider.clone(),
                    cfg.ai_max_tokens,
                    cfg.ai_temperature,
                    cfg.ai_system_prompt.clone(),
                    state.source.frequency_hz,
                    state.source.sample_rate_hz,
                    state.source.gain_db,
                    mode_label,
                    state.recording,
                    state.selected_satellite.clone(),
                    state.adsb_running,
                    state.spectrum.noise_floor(),
                    state.spectrum.peak_level(),
                    state.squelch,
                )
            } else {
                return (vec![], String::new(), String::new(), String::new(), String::new(), 0, 0.0, String::new());
            }
        };
        let snr = peak_db - noise_floor;

        let mut msgs: Vec<serde_json::Value> = Vec::new();

        // System prompt
        let sys = if system_prompt.is_empty() {
            format!(
                "{}\n\nCurrent SDR state:\n\
                 - Frequency: {:.4} MHz\n\
                 - Sample rate: {:.3} MSps\n\
                 - Gain: {:.1} dB\n\
                 - Demod mode: {}\n\
                 - Noise floor: {:.1} dB\n\
                 - Peak signal: {:.1} dB\n\
                 - SNR: {:.1} dB\n\
                 - Squelch: {:.1} dB\n\
                 - Recording: {}\n\
                 - Satellite: {}\n\
                 - ADS-B: {}",
                DEFAULT_SYSTEM,
                freq as f64 / 1e6,
                rate as f64 / 1e6,
                gain,
                mode,
                noise_floor,
                peak_db,
                snr,
                squelch,
                if recording { "yes" } else { "no" },
                sat.as_deref().unwrap_or("none"),
                if adsb { "running" } else { "stopped" },
            )
        } else {
            system_prompt
        };
        msgs.push(serde_json::json!({"role": "system", "content": sys}));

        // Conversation history (skip old system messages we already injected)
        for m in &self.messages {
            if m.role == "system" {
                continue;
            }
            let mut obj = serde_json::json!({
                "role": m.role,
                "content": m.content,
            });
            if let Some(ref calls) = m.tool_calls {
                let tools: Vec<serde_json::Value> = calls.iter().map(|tc| {
                    serde_json::json!({
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments.to_string(),
                        }
                    })
                }).collect();
                obj["tool_calls"] = serde_json::json!(tools);
            }
            msgs.push(obj);
        }

        (msgs, endpoint, model, api_key, provider, max_tokens, temperature, sys)
    }

    pub fn send_message(&mut self) {
        if self.input.is_empty() || self.thinking {
            return;
        }
        let user_msg = self.input.clone();
        self.input.clear();
        self.messages.push(ChatMessage {
            role: "user".to_string(),
            content: user_msg,
            tool_calls: None,
            streaming: false,
        });

        // Start assistant message placeholder
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: None,
            streaming: true,
        });
        self.thinking = true;

        let (api_messages, endpoint, model, api_key, provider, max_tokens, temperature, system_prompt) = self.build_api_messages();

        let needs_key = PROVIDER_PRESETS.iter()
            .find(|p| p.name == provider)
            .map(|p| p.needs_key)
            .unwrap_or(true);

        if needs_key && api_key.is_empty() {
            if let Some(last) = self.messages.last_mut() {
                last.streaming = false;
                last.content = format!(
                    "⚠ No API key set for {}. Go to Settings → AI Agent to add one.",
                    provider
                );
            }
            self.thinking = false;
            return;
        }

        let (evt_tx, evt_rx) = crossbeam_channel::bounded::<StreamEvent>(256);
        let is_anthropic = provider == "Anthropic";

        std::thread::spawn(move || {
            let client = match reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = evt_tx.send(StreamEvent::Error(format!("Client error: {}", e)));
                    return;
                }
            };

            if is_anthropic {
                Self::stream_anthropic(&client, &evt_tx, &endpoint, &api_key, &model, &system_prompt, &api_messages, max_tokens, temperature);
            } else {
                Self::stream_openai_compat(&client, &evt_tx, &endpoint, &api_key, &model, &api_messages, max_tokens, temperature);
            }
        });

        self.pending_rx = Some(evt_rx);
    }

    fn stream_openai_compat(
        client: &reqwest::blocking::Client,
        evt_tx: &crossbeam_channel::Sender<StreamEvent>,
        endpoint: &str,
        api_key: &str,
        model: &str,
        api_messages: &[serde_json::Value],
        max_tokens: u32,
        temperature: f64,
    ) {
        let body = serde_json::json!({
            "model": model,
            "messages": api_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": true,
        });

        let mut req = client.post(endpoint)
            .header("Content-Type", "application/json")
            .json(&body);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                let _ = evt_tx.send(StreamEvent::Error(format!("HTTP error: {}", e)));
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            let _ = evt_tx.send(StreamEvent::Error(format!("HTTP {}: {}", status, text)));
            return;
        }

        let mut full_text = String::new();
        let reader = BufReader::new(resp);
        for line in reader.lines() {
            let line = match line { Ok(l) => l, Err(_) => break };
            if !line.starts_with("data: ") { continue; }
            let data = &line[6..];
            if data == "[DONE]" { break; }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                let content = json["choices"][0]["delta"]["content"].as_str().unwrap_or("");
                if !content.is_empty() {
                    full_text.push_str(content);
                    let _ = evt_tx.send(StreamEvent::Chunk(content.to_string()));
                }
            }
        }

        Self::check_tool_calls(evt_tx, &full_text);
        let _ = evt_tx.send(StreamEvent::Done(full_text));
    }

    fn stream_anthropic(
        client: &reqwest::blocking::Client,
        evt_tx: &crossbeam_channel::Sender<StreamEvent>,
        endpoint: &str,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        api_messages: &[serde_json::Value],
        max_tokens: u32,
        temperature: f64,
    ) {
        // Anthropic format: system is a top-level field, messages exclude system
        let non_system: Vec<&serde_json::Value> = api_messages.iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .collect();

        let body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "system": system_prompt,
            "messages": non_system,
            "stream": true,
        });

        let resp = match client.post(endpoint)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                let _ = evt_tx.send(StreamEvent::Error(format!("HTTP error: {}", e)));
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            let _ = evt_tx.send(StreamEvent::Error(format!("Anthropic HTTP {}: {}", status, text)));
            return;
        }

        let mut full_text = String::new();
        let reader = BufReader::new(resp);
        for line in reader.lines() {
            let line = match line { Ok(l) => l, Err(_) => break };
            if !line.starts_with("data: ") { continue; }
            let data = &line[6..];
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                match json["type"].as_str() {
                    Some("content_block_delta") => {
                        let text = json["delta"]["text"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            full_text.push_str(text);
                            let _ = evt_tx.send(StreamEvent::Chunk(text.to_string()));
                        }
                    }
                    Some("message_stop") => break,
                    _ => {}
                }
            }
        }

        Self::check_tool_calls(evt_tx, &full_text);
        let _ = evt_tx.send(StreamEvent::Done(full_text));
    }

    fn check_tool_calls(evt_tx: &crossbeam_channel::Sender<StreamEvent>, full_text: &str) {
        let tool_info = serde_json::from_str::<serde_json::Value>(full_text)
            .ok()
            .and_then(|json| {
                let name = json.get("tool").and_then(|v| v.as_str())?.to_string();
                let args = json.get("args").cloned().unwrap_or(serde_json::json!({}));
                Some((name, args))
            });
        if let Some((ref tool_name, ref args)) = tool_info {
            let _ = evt_tx.send(StreamEvent::ToolCallDetected {
                tool: tool_name.clone(),
                args: args.clone(),
            });
        }
    }

    fn poll_stream(&mut self) {
        let rx = match self.pending_rx.take() {
            Some(rx) => rx,
            None => return,
        };

        let mut keep = true;
        loop {
            match rx.try_recv() {
                Ok(StreamEvent::Chunk(text)) => {
                    if let Some(last) = self.messages.last_mut() {
                        last.content.push_str(&text);
                    }
                }
                Ok(StreamEvent::ToolCallDetected { tool, args }) => {
                    let result = self.execute_tool_call(&tool, &args);
                    if let Some(last) = self.messages.last_mut() {
                        let tc = ToolCall {
                            name: tool.clone(),
                            arguments: args.clone(),
                        };
                        let mut calls = last.tool_calls.take().unwrap_or_default();
                        calls.push(tc);
                        last.tool_calls = Some(calls);
                        last.content.push_str(&format!("\n▶ {} → {}", tool, result));
                    }
                }
                Ok(StreamEvent::Done(_)) => {
                    if let Some(last) = self.messages.last_mut() {
                        last.streaming = false;
                    }
                    keep = false;
                    break;
                }
                Ok(StreamEvent::Error(err)) => {
                    self.messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: format!("⚠ {}", err),
                        tool_calls: None,
                        streaming: false,
                    });
                    keep = false;
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    if let Some(last) = self.messages.last_mut() {
                        last.streaming = false;
                    }
                    keep = false;
                    break;
                }
            }
        }

        if keep {
            self.pending_rx = Some(rx);
        }
        self.thinking = keep;
    }

    fn execute_tool_call(&mut self, name: &str, args: &serde_json::Value) -> String {
        if let Ok(mut state) = self.shared.try_lock() {
            match name {
                "tune_frequency" => {
                    if let Some(hz) = args["hz"].as_u64() {
                        state.source.frequency_hz = hz;
                        return format!("Tuned to {:.3} MHz", hz as f64 / 1e6);
                    }
                }
                "set_gain" => {
                    if let Some(db) = args["db"].as_f64() {
                        state.source.gain_db = db;
                        return format!("Gain set to {:.1} dB", db);
                    }
                }
                "set_sample_rate" => {
                    if let Some(rate) = args["rate"].as_u64() {
                        state.source.sample_rate_hz = rate as u32;
                        return format!("Sample rate set to {} Hz", rate);
                    }
                }
                "toggle_bias_tee" => {
                    if let Some(on) = args["on"].as_bool() {
                        state.source.bias_tee = on;
                        return format!("Bias tee {}", if on { "ON" } else { "OFF" });
                    }
                }
                "set_demod" => {
                    if let Some(mode) = args["mode"].as_str() {
                        let demod = match mode.to_uppercase().as_str() {
                            "RAW" => crate::sdr_panel::DemodMode::Raw,
                            "AM" => crate::sdr_panel::DemodMode::Am,
                            "FM" | "NFM" => crate::sdr_panel::DemodMode::Fm,
                            "WFM" => crate::sdr_panel::DemodMode::Wfm,
                            "LSB" => crate::sdr_panel::DemodMode::Lsb,
                            "USB" => crate::sdr_panel::DemodMode::Usb,
                            _ => crate::sdr_panel::DemodMode::Fm,
                        };
                        state.demod_mode = demod;
                        return format!("Demod mode set to {}", mode.to_uppercase());
                    }
                }
                "start_recording" => {
                    state.recording = true;
                    return "Recording started".to_string();
                }
                "stop_recording" => {
                    state.recording = false;
                    return "Recording stopped".to_string();
                }
                "select_satellite" => {
                    if let Some(name) = args["name"].as_str() {
                        state.selected_satellite = Some(name.to_string());
                        return format!("Satellite '{}' selected", name);
                    }
                }
                "start_adsb" => {
                    state.adsb_running = true;
                    return "ADS-B tracking started at 1090 MHz".to_string();
                }
                "stop_adsb" => {
                    state.adsb_running = false;
                    return "ADS-B tracking stopped".to_string();
                }
                "set_squelch" => {
                    if let Some(db) = args["db"].as_f64() {
                        state.squelch = db as f32;
                        return format!("Squelch set to {:.1} dB", db);
                    }
                }
                "set_volume" => {
                    if let Some(level) = args["level"].as_f64() {
                        state.volume = (level as f32).clamp(0.0, 1.0);
                        return format!("Volume set to {:.0}%", level * 100.0);
                    }
                }
                "get_status" => {
                    return serde_json::json!({
                        "frequency_mhz": state.source.frequency_hz as f64 / 1e6,
                        "gain_db": state.source.gain_db,
                        "demod": state.demod_mode.label(),
                        "sample_rate_msps": state.source.sample_rate_hz as f64 / 1e6,
                        "squelch_db": state.squelch,
                        "volume": state.volume,
                        "recording": state.recording,
                        "adsb_running": state.adsb_running,
                        "peak_db": state.spectrum.peak_level(),
                        "noise_floor_db": state.spectrum.noise_floor(),
                        "snr_db": state.spectrum.peak_level() - state.spectrum.noise_floor(),
                    }).to_string();
                }
                _ => return format!("Unknown tool: {}", name),
            }
        }
        "Error: could not access SDR state".to_string()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll_stream();

        // Header with model info
        let (model, provider, has_key, temp) = {
            if let Ok(state) = self.shared.try_lock() {
                let needs_key = PROVIDER_PRESETS.iter()
                    .find(|p| p.name == state.config.ai_provider)
                    .map(|p| p.needs_key)
                    .unwrap_or(true);
                (
                    state.config.ai_model.clone(),
                    state.config.ai_provider.clone(),
                    !needs_key || !state.config.ai_api_key.is_empty(),
                    state.config.ai_temperature,
                )
            } else {
                (DEFAULT_AI_MODEL.to_string(), "?".to_string(), false, 0.7)
            }
        };
        self.temperature = temp;

        ui.horizontal(|ui| {
            ui.heading("AI Agent");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.monospace(format!("{} · {}", provider, model));
                if !has_key {
                    ui.colored_label(egui::Color32::YELLOW, "⚠ no key")
                        .on_hover_text("Go to Settings → AI Agent to add an API key.");
                }
            });
        });
        ui.separator();

        // Collapsing tools reference
        ui.collapsing("Available Tools", |ui| {
            egui::Grid::new("ai_tools_grid").num_columns(2).striped(true).show(ui, |ui| {
                let tools = [
                    ("tune_frequency(hz)", "Set center frequency in Hz"),
                    ("set_gain(db)", "RF gain 0–49.6 dB"),
                    ("set_demod(mode)", "AM / NFM / WFM / USB / LSB / RAW"),
                    ("set_sample_rate(rate)", "Sample rate in Hz (e.g. 2048000)"),
                    ("set_squelch(db)", "Squelch threshold in dB"),
                    ("set_volume(level)", "Audio volume 0.0–1.0"),
                    ("toggle_bias_tee(on)", "Bias tee power for LNAs"),
                    ("start_recording()", "Begin IQ/WAV recording"),
                    ("stop_recording()", "Stop recording"),
                    ("select_satellite(name)", "Auto-track a satellite"),
                    ("start_adsb()", "Start ADS-B decoder at 1090 MHz"),
                    ("stop_adsb()", "Stop ADS-B decoder"),
                    ("get_status()", "Return full SDR state as JSON"),
                ];
                for (name, desc) in &tools {
                    ui.monospace(*name);
                    ui.label(*desc);
                    ui.end_row();
                }
            });
        });
        ui.separator();

        // Quick prompt buttons
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Quick:").weak());
            let quick_prompts = [
                ("📻 FM Radio",      "Tune to 100.1 MHz and set mode to WFM"),
                ("✈ ADS-B",          "Start ADS-B tracking"),
                ("🛰 NOAA 19",       "Track NOAA 19 weather satellite"),
                ("📡 Scan VHF",      "Scan 145 to 165 MHz for active signals"),
                ("🔊 Max audio",     "Set gain to 40 and volume to maximum"),
                ("📋 Status",        "Show me the current SDR status"),
                ("❓ What can I hear?","What signals can I find near 400 MHz?"),
                ("🔧 Diagnose",      "Get the current SDR status and tell me if anything looks wrong or could be improved. Give me beginner-friendly advice."),
                ("🔇 No audio?",     "I can't hear any audio. Diagnose why — check the current SDR state, demod mode, squelch, and audio settings and suggest fixes."),
            ];
            for (label, prompt) in &quick_prompts {
                if ui.small_button(*label).on_hover_text(*prompt).clicked() && !self.thinking {
                    self.input = prompt.to_string();
                    self.send_message();
                }
            }
        });
        ui.separator();

        // Chat area
        egui::ScrollArea::vertical()
            .id_salt("ai_chat_scroll")
            .max_height(ui.available_height() - 90.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for msg in &self.messages {
                    if msg.role == "system" {
                        continue;
                    }
                    let (color, label) = match msg.role.as_str() {
                        "user"      => (egui::Color32::from_rgb(100, 180, 255), "You"),
                        "assistant" => (egui::Color32::from_rgb(100, 255, 130), "AI"),
                        _           => (egui::Color32::from_gray(180), "?"),
                    };

                    let is_error = msg.content.starts_with('⚠');
                    let content_color = if is_error {
                        egui::Color32::LIGHT_RED
                    } else {
                        ui.style().visuals.text_color()
                    };

                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color, format!("{}:", label));
                        if msg.content.contains("```") {
                            for (i, block) in msg.content.split("```").enumerate() {
                                if i % 2 == 0 {
                                    ui.label(egui::RichText::new(block.to_string()).color(content_color));
                                } else {
                                    let code = if let Some(idx) = block.find('\n') { &block[idx + 1..] } else { block };
                                    ui.monospace(egui::RichText::new(code.to_string()).color(egui::Color32::from_rgb(200, 200, 100)));
                                }
                            }
                        } else {
                            ui.label(egui::RichText::new(&msg.content).color(content_color));
                        }
                    });

                    if let Some(ref calls) = msg.tool_calls {
                        for tc in calls {
                            ui.horizontal(|ui| {
                                ui.colored_label(egui::Color32::from_rgb(255, 200, 50), ">>>");
                                ui.monospace(format!("{}({})", tc.name, tc.arguments));
                            });
                        }
                    }

                    if msg.streaming {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.small("streaming...");
                        });
                    }

                    ui.add_space(4.0);
                }

                let rough_tokens: usize = self.messages.iter().map(|m| m.content.len() / 4).sum();
                if rough_tokens > 3000 {
                    ui.colored_label(egui::Color32::YELLOW,
                        format!("⚠ ~{}k context tokens — consider clearing chat if responses seem confused.", rough_tokens / 1000));
                }
            });

        ui.separator();

        // Input bar
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .desired_width(f32::INFINITY)
                    .hint_text("Ask to tune, record, scan, track satellites…")
                    .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Enter))),
            );
            let send_clicked = ui.button("Send").clicked()
                || resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if send_clicked && !self.input.is_empty() && !self.thinking {
                self.send_message();
            }
            if ui.button("Clear")
                .on_hover_text("Clear conversation history. Useful when the model seems confused due to long context.")
                .clicked() && !self.thinking
            {
                self.messages.clear();
            }
        });

        if self.thinking && self.pending_rx.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Connecting…");
            });
        }
    }
}

#[allow(dead_code)]
fn execute_tool_sync(name: &str, args: &serde_json::Value) -> String {
    match name {
        "tune_frequency" => {
            args["hz"].as_u64()
                .map(|hz| format!("Tuned to {:.3} MHz", hz as f64 / 1e6))
                .unwrap_or_else(|| "Error: missing hz".to_string())
        }
        "set_gain" => {
            args["db"].as_f64()
                .map(|db| format!("Gain set to {:.1} dB", db))
                .unwrap_or_else(|| "Error: missing db".to_string())
        }
        "set_sample_rate" => {
            args["rate"].as_u64()
                .map(|rate| format!("Sample rate set to {} Hz", rate))
                .unwrap_or_else(|| "Error: missing rate".to_string())
        }
        "toggle_bias_tee" => {
            args["on"].as_bool()
                .map(|on| format!("Bias tee {}", if on { "ON" } else { "OFF" }))
                .unwrap_or_else(|| "Error: missing on".to_string())
        }
        "set_demod" => {
            args["mode"].as_str()
                .map(|mode| format!("Demod mode set to {}", mode.to_uppercase()))
                .unwrap_or_else(|| "Error: missing mode".to_string())
        }
        "start_recording" => "Recording started".to_string(),
        "stop_recording" => "Recording stopped".to_string(),
        "select_satellite" => {
            args["name"].as_str()
                .map(|name| format!("Satellite '{}' selected", name))
                .unwrap_or_else(|| "Error: missing name".to_string())
        }
        "start_adsb" => "ADS-B tracking started at 1090 MHz".to_string(),
        "stop_adsb" => "ADS-B tracking stopped".to_string(),
        _ => format!("Unknown tool: {}", name),
    }
}
