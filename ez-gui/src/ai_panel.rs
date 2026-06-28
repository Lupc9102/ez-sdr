use std::sync::{Arc, Mutex};

use crate::app::SharedState;

pub struct AiPanel {
    shared: Arc<Mutex<SharedState>>,
    pub api_key: String,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub thinking: bool,
    pending_response: Option<crossbeam_channel::Receiver<String>>,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

const SYSTEM_PROMPT: &str = "You are EZ-SDR AI Agent, an expert radio and satellite assistant. You can control the SDR application by responding with tool calls in your JSON output.

Available tools:
- tune_frequency(hz: u64) — Set the SDR center frequency
- set_gain(db: f64) — Set the tuner gain
- set_demod(mode: string) — Set demodulation mode: AM, FM, WFM, LSB, USB, RAW
- set_sample_rate(rate: u32) — Set sample rate in Hz
- toggle_bias_tee(on: bool) — Enable/disable bias tee
- start_recording() — Start recording to disk
- stop_recording() — Stop recording
- select_satellite(name: string) — Select satellite for tracking (NOAA 15, NOAA 18, NOAA 19, Meteor-M2, Meteor-M2-2, ISS)
- start_adsb() — Start ADS-B tracking at 1090 MHz
- stop_adsb() — Stop ADS-B tracking
- get_status() — Get current SDR status

When you want to call a tool, respond with a JSON block like:
{\"tool\": \"tune_frequency\", \"args\": {\"hz\": 137620000}}

Always be helpful. Explain what you are doing when you change settings.";

impl AiPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            api_key: String::new(),
            messages: vec![ChatMessage {
                role: "system".to_string(),
                content: SYSTEM_PROMPT.to_string(),
                tool_calls: None,
            }],
            input: String::new(),
            thinking: false,
            pending_response: None,
        }
    }

    fn send_message(&mut self) {
        if self.input.is_empty() { return; }
        let user_msg = self.input.clone();
        self.messages.push(ChatMessage {
            role: "user".to_string(),
            content: user_msg,
            tool_calls: None,
        });
        self.input.clear();
        self.thinking = true;

        let api_key = self.api_key.clone();
        let messages_json: Vec<serde_json::Value> = self.messages.iter().map(|m| {
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
            obj
        }).collect();

        // We need a way to get the response back to the UI.
        // Use crossbeam oneshot channel.
        let (resp_tx, resp_rx) = crossbeam_channel::bounded::<String>(1);

        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            let body = serde_json::json!({
                "model": "anthropic/claude-3-haiku",
                "messages": messages_json,
                "max_tokens": 2048,
            });
            let resp = client.post("https://openrouter.ai/api/v1/chat/completions")
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send();

            match resp {
                Ok(r) => {
                    let v: serde_json::Value = r.json().unwrap_or(serde_json::json!({"choices": []}));
                    let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("No response.").to_string();
                    let _ = resp_tx.send(content);
                }
                Err(e) => {
                    let _ = resp_tx.send(format!("Error: {}", e));
                }
            }
        });

        // Poll for response via crossbeam channel
        self.pending_response = Some(resp_rx);
    }

    fn check_response(&mut self) {
        if let Some(rx) = &self.pending_response {
            if let Ok(content) = rx.try_recv() {
                // Check for tool calls in the response
                let mut tool_calls = Vec::new();
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(tool) = json.get("tool") {
                        if let Some(name) = tool.as_str() {
                            let args = json.get("args").cloned().unwrap_or(serde_json::json!({}));
                            tool_calls.push(ToolCall { name: name.to_string(), arguments: args });
                        }
                    }
                }

                self.messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: content.clone(),
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls.clone()) },
                });

                // Execute any tool calls
                for tc in &tool_calls {
                    self.execute_tool_call(tc);
                }

                self.thinking = false;
                self.pending_response = None;
            }
        }
    }

    fn execute_tool_call(&mut self, call: &ToolCall) {
        if let Ok(mut state) = self.shared.try_lock() {
            match call.name.as_str() {
                "tune_frequency" => {
                    if let Some(hz) = call.arguments["hz"].as_u64() {
                        state.source.frequency_hz = hz;
                    }
                }
                "set_gain" => {
                    if let Some(db) = call.arguments["db"].as_f64() {
                        state.source.gain_db = db;
                    }
                }
                "set_sample_rate" => {
                    if let Some(rate) = call.arguments["rate"].as_u64() {
                        state.source.sample_rate_hz = rate as u32;
                    }
                }
                "toggle_bias_tee" => {
                    if let Some(on) = call.arguments["on"].as_bool() {
                        state.source.bias_tee = on;
                    }
                }
                "set_demod" => {
                    if let Some(mode) = call.arguments["mode"].as_str() {
                        state.demod_mode = match mode.to_uppercase().as_str() {
                            "RAW" => crate::sdr_panel::DemodMode::Raw,
                            "AM" => crate::sdr_panel::DemodMode::Am,
                            "FM" => crate::sdr_panel::DemodMode::Fm,
                            "WFM" => crate::sdr_panel::DemodMode::Wfm,
                            "LSB" => crate::sdr_panel::DemodMode::Lsb,
                            "USB" => crate::sdr_panel::DemodMode::Usb,
                            _ => crate::sdr_panel::DemodMode::Fm,
                        };
                    }
                }
                "start_recording" => {
                    state.recording = true;
                }
                "stop_recording" => {
                    state.recording = false;
                }
                "select_satellite" => {
                    if let Some(name) = call.arguments["name"].as_str() {
                        state.selected_satellite = Some(name.to_string());
                    }
                }
                "start_adsb" => {
                    state.adsb_running = true;
                }
                "stop_adsb" => {
                    state.adsb_running = false;
                }
                _ => {}
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.check_response();

        ui.heading("AI Agent — Haiku via OpenRouter");
        ui.horizontal(|ui| {
            ui.label("API Key:");
            ui.add(egui::TextEdit::singleline(&mut self.api_key).password(true).desired_width(250.0));
        });
        ui.separator();

        egui::ScrollArea::vertical().max_height(350.0).stick_to_bottom(true).show(ui, |ui| {
            for msg in &self.messages {
                if msg.role == "system" { continue; }
                let (color, label) = match msg.role.as_str() {
                    "user" => (egui::Color32::from_rgb(100, 180, 255), "You"),
                    "assistant" => (egui::Color32::from_rgb(100, 255, 130), "AI"),
                    _ => (egui::Color32::from_gray(180), "?"),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(color, format!("{}:", label));
                    ui.label(&msg.content);
                });
                if let Some(ref calls) = msg.tool_calls {
                    for tc in calls {
                        ui.horizontal(|ui| {
                            ui.colored_label(egui::Color32::from_rgb(255, 200, 50), ">>>");
                            ui.monospace(format!("{}({})", tc.name, tc.arguments));
                        });
                    }
                }
                ui.add_space(4.0);
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.input).desired_width(400.0).hint_text("Ask the AI anything..."));
            if ui.button("Send").clicked() && !self.input.is_empty() {
                self.send_message();
            }
        });
        if self.thinking {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Thinking...");
            });
        }
    }
}
