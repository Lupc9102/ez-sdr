use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};

use crate::app::SharedState;
use crate::config::DEFAULT_AI_MODEL;

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
- start_adsb() / stop_adsb()
- get_status() — Return current SDR state

When you want to call a tool respond with exactly:
{\"tool\": \"name\", \"args\": {}}
You may call multiple tools sequentially. Always explain what you are doing.";

// Streaming state sent from worker thread
enum StreamEvent {
    Chunk(String),
    ToolCallDetected { tool: String, args: serde_json::Value },
    Done(String),
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
    fn build_api_messages(&self) -> (Vec<serde_json::Value>, String, String, String, u32, f64) {
        let (endpoint, model, api_key, max_tokens, temperature, system_prompt, freq, rate, gain, mode, recording, sat, adsb) = {
            if let Ok(state) = self.shared.try_lock() {
                let cfg = &state.config;
                let mode_label = state.demod_mode.label().to_string();
                (
                    cfg.ai_endpoint.clone(),
                    cfg.ai_model.clone(),
                    cfg.ai_api_key.clone(),
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
                )
            } else {
                return (vec![], String::new(), String::new(), String::new(), 0, 0.0);
            }
        };

        let mut msgs: Vec<serde_json::Value> = Vec::new();

        // System prompt
        let sys = if system_prompt.is_empty() {
            format!(
                "{}\n\nCurrent SDR state:\n\
                 - Frequency: {} MHz\n\
                 - Sample rate: {} Hz\n\
                 - Gain: {:.1} dB\n\
                 - Demod mode: {}\n\
                 - Recording: {}\n\
                 - Satellite: {}\n\
                 - ADS-B: {}",
                DEFAULT_SYSTEM,
                freq as f64 / 1e6,
                rate,
                gain,
                mode,
                if recording { "yes" } else { "no" },
                sat.unwrap_or_default(),
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

        (msgs, endpoint, model, api_key, max_tokens, temperature)
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

        let (api_messages, endpoint, model, api_key, max_tokens, temperature) = self.build_api_messages();
        if api_key.is_empty() {
            self.messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: "⚠ API key not set. Configure it in Settings → AI Agent.".to_string(),
                tool_calls: None,
                streaming: false,
            });
            self.thinking = false;
            return;
        }

        let (evt_tx, evt_rx) = crossbeam_channel::bounded::<StreamEvent>(256);

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

            let body = serde_json::json!({
                "model": model,
                "messages": api_messages,
                "max_tokens": max_tokens,
                "temperature": temperature,
                "stream": true,
            });

            let resp = match client
                .post(&endpoint)
                .header("Authorization", format!("Bearer {}", api_key))
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
                let _ = evt_tx.send(StreamEvent::Error(format!("HTTP {}: {}", status, text)));
                return;
            }

            // Read SSE stream line by line
            let mut full_text = String::new();
            let reader = BufReader::new(resp);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if !line.starts_with("data: ") {
                    continue;
                }
                let data = &line[6..];
                if data == "[DONE]" {
                    break;
                }
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    let content = json["choices"][0]["delta"]["content"]
                        .as_str()
                        .unwrap_or("");
                    if !content.is_empty() {
                        full_text.push_str(content);
                        let _ = evt_tx.send(StreamEvent::Chunk(content.to_string()));
                    }
                }
            }

            // After stream completes, check for tool calls in the full text
            let tool_info = serde_json::from_str::<serde_json::Value>(&full_text)
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
            let _ = evt_tx.send(StreamEvent::Done(full_text.clone()));

            // Tool feedback loop: execute tool, send result back to LLM for friendly summary
            if let Some((ref tool_name, ref args)) = tool_info {
                let result = execute_tool_sync(tool_name, args);
                let feedback_body = serde_json::json!({
                    "model": model,
                    "messages": serde_json::json!([
                        {"role": "system", "content": "You are a helpful radio assistant. Report the result concisely."},
                        {"role": "user", "content": format!(
                            "I called the tool '{}' with args {} and got result: {}. \
                             Summarize what happened in a friendly sentence.",
                            tool_name, args, result
                        )}
                    ]),
                    "max_tokens": 512,
                    "temperature": 0.5,
                    "stream": true,
                });

                if let Ok(fb_resp) = client
                    .post(&endpoint)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("Content-Type", "application/json")
                    .json(&feedback_body)
                    .send()
                {
                    if fb_resp.status().is_success() {
                        let reader = BufReader::new(fb_resp);
                        for line in reader.lines() {
                            let line = match line {
                                Ok(l) => l,
                                Err(_) => break,
                            };
                            if !line.starts_with("data: ") { continue; }
                            let data = &line[6..];
                            if data == "[DONE]" { break; }
                            if let Ok(js) = serde_json::from_str::<serde_json::Value>(data) {
                                let c = js["choices"][0]["delta"]["content"]
                                    .as_str().unwrap_or("");
                                if !c.is_empty() {
                                    let _ = evt_tx.send(StreamEvent::Chunk(c.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        });

        self.pending_rx = Some(evt_rx);
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
                _ => return format!("Unknown tool: {}", name),
            }
        }
        "Error: could not access SDR state".to_string()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll_stream();

        // Header with model info
        let (model, has_key, temp) = {
            if let Ok(state) = self.shared.try_lock() {
                (
                    state.config.ai_model.clone(),
                    !state.config.ai_api_key.is_empty(),
                    state.config.ai_temperature,
                )
            } else {
                (DEFAULT_AI_MODEL.to_string(), false, 0.7)
            }
        };
        self.temperature = temp;

        ui.horizontal(|ui| {
            ui.heading("AI Agent");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.monospace(format!("model: {}", model));
                if !has_key {
                    ui.colored_label(egui::Color32::YELLOW, "⚠ no key");
                }
            });
        });
        ui.separator();

        // Chat area
        egui::ScrollArea::vertical()
            .id_salt("ai_chat_scroll")
            .max_height(320.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for msg in &self.messages {
                    if msg.role == "system" {
                        continue;
                    }
                    let (color, label) = match msg.role.as_str() {
                        "user" => (egui::Color32::from_rgb(100, 180, 255), "You"),
                        "assistant" => (egui::Color32::from_rgb(100, 255, 130), "AI"),
                        _ => (egui::Color32::from_gray(180), "?"),
                    };

                    let is_error = msg.content.starts_with('⚠');
                    let content_color = if is_error {
                        egui::Color32::LIGHT_RED
                    } else {
                        ui.style().visuals.text_color()
                    };

                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color, format!("{}:", label));
                        // Check if content contains a code block (``` ... ```)
                        if msg.content.contains("```") {
                            for (i, block) in msg.content.split("```").enumerate() {
                                if i % 2 == 0 {
                                    ui.label(egui::RichText::new(block.to_string()).color(content_color));
                                } else {
                                    let code = if let Some(idx) = block.find('\n') {
                                        &block[idx + 1..]
                                    } else {
                                        block
                                    };
                                    ui.monospace(egui::RichText::new(code.to_string()).color(egui::Color32::from_rgb(200, 200, 100)));
                                }
                            }
                        } else {
                            ui.label(egui::RichText::new(&msg.content).color(content_color));
                        }
                    });

                    // Tool calls
                    if let Some(ref calls) = msg.tool_calls {
                        for tc in calls {
                            ui.horizontal(|ui| {
                                ui.colored_label(egui::Color32::from_rgb(255, 200, 50), ">>>");
                                ui.monospace(format!("{}({})", tc.name, tc.arguments));
                            });
                        }
                    }

                    // Streaming indicator
                    if msg.streaming {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.small("streaming...");
                        });
                    }

                    ui.add_space(4.0);
                }

                // Token estimate warning
                let rough_tokens: usize = self.messages.iter()
                    .map(|m| m.content.len() / 4)
                    .sum();
                if rough_tokens > 3000 {
                    ui.colored_label(egui::Color32::YELLOW,
                        format!("⚠ ~{}k context tokens — consider clearing chat if the LLM seems confused.", rough_tokens / 1000));
                }
            });

        ui.separator();

        // Input bar
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .desired_width(f32::INFINITY)
                    .hint_text("Ask the AI to tune, record, track satellites…")
                    .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Enter))),
            );
            let send_clicked = ui.button("Send").clicked() || resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if send_clicked && !self.input.is_empty() && !self.thinking {
                self.send_message();
            }
            if ui.button("Clear").clicked() && !self.thinking {
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

/// Synchronous version of execute_tool_call for use in the worker thread.
/// Does NOT have access to SharedState, so we just build a result string.
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
