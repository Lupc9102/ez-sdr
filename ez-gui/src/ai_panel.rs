use std::io::{BufRead, BufReader};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use crate::app::SharedState;
use crate::config::{DEFAULT_AI_MODEL, PROVIDER_PRESETS};

#[derive(Clone, Copy)]
struct StreamParams<'a> {
    client: &'a reqwest::blocking::Client,
    evt_tx: &'a crossbeam_channel::Sender<StreamEvent>,
    endpoint: &'a str,
    api_key: &'a str,
    model: &'a str,
    api_messages: &'a [serde_json::Value],
    max_tokens: u32,
    temperature: f64,
    abort_flag: &'a Arc<AtomicBool>,
    reasoning_effort: &'a str,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub image_url: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub streaming: bool,
    pub timestamp_secs: u64,
}

impl ChatMessage {
    fn new(role: &str, content: &str) -> Self {
        let timestamp_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            role: role.to_string(),
            content: content.to_string(),
            image_url: None,
            tool_calls: None,
            streaming: false,
            timestamp_secs,
        }
    }

    /// Create a message with an image attachment for vision models.
    #[allow(dead_code)]
    fn with_image(role: &str, content: &str, image_url: String) -> Self {
        let mut msg = Self::new(role, content);
        msg.image_url = Some(image_url);
        msg
    }

    fn format_time(&self) -> String {
        let secs = self.timestamp_secs % 86400;
        format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
    }
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
- get_freq_history() — Return the list of recently tuned frequencies
- add_bookmark(name: string, hz: u64, mode: string, notes: string) — Save a frequency as a bookmark
- set_lpf_cutoff(hz: f64) — Set audio low-pass filter cutoff in Hz (e.g. 3000 for voice, 15000 for FM)
- set_ppm(ppm: i32) — Set frequency correction in parts-per-million (corrects oscillator drift)
- configure_scanner(start_mhz: f64, stop_mhz: f64, step_khz: f64, dwell_ms: u64, threshold_db: f64, run: bool) — Configure the frequency scanner. All args optional; only the ones you pass are changed. run=true starts the sweep after applying, run=false stops it. Example: {\"tool\": \"configure_scanner\", \"args\": {\"start_mhz\": 118.0, \"stop_mhz\": 137.0, \"step_khz\": 25.0, \"dwell_ms\": 300, \"threshold_db\": -60, \"run\": true}}
- web_search(query: string) — Search the web for external information (current events, technical specs, frequency databases, regulatory info, etc.)

When you want to call a tool respond with exactly:
{\"tool\": \"name\", \"args\": {}}
You may call multiple tools sequentially — include one JSON block per tool call in your response.
Always explain what you are doing before each tool call.
Use web_search when the user asks about external information not available in your training data (current events, specific product specs, local frequency allocations, etc.).";

// Streaming state sent from worker thread
enum StreamEvent {
    Chunk(String),
    ToolCallDetected {
        tool: String,
        args: serde_json::Value,
    },
    Done(()),
    Error(String),
}

pub struct AiPanel {
    shared: Arc<Mutex<SharedState>>,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub thinking: bool,
    temperature: f64,
    pending_rx: Option<crossbeam_channel::Receiver<StreamEvent>>,
    abort_flag: Arc<AtomicBool>,
    stream_start: Option<std::time::Instant>,
    reasoning_effort: String,
    web_search_enabled: bool,
    /// State-changing tool calls awaiting user approval. The model runs
    /// untrusted text through `extract_tool_calls`, so executing mutations
    /// (tune/record/gain/…) sight-unseen lets a confused or injected model
    /// drive the radio. Read-only tools bypass the queue (see
    /// `is_read_only_tool`); everything else needs an explicit Approve click
    /// unless `auto_approve_tools` is on.
    pending_tool_calls: Vec<(String, serde_json::Value)>,
    /// Opt-in escape hatch restoring immediate execution (default off).
    auto_approve_tools: bool,
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
            abort_flag: Arc::new(AtomicBool::new(false)),
            stream_start: None,
            reasoning_effort: "off".to_string(),
            web_search_enabled: false,
            pending_tool_calls: Vec::new(),
            auto_approve_tools: false,
        }
    }

    /// Build the messages JSON array for the API call, injecting system prompt.
    fn build_api_messages(
        &self,
    ) -> (
        Vec<serde_json::Value>,
        String,
        String,
        String,
        String,
        u32,
        f64,
        String,
        String,
        bool,
    ) {
        let state_snapshot = {
            if let Ok(state) = self.shared.try_lock() {
                let cfg = &state.config;
                let mode_label = state.demod_mode.label().to_string();
                Some((
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
                    state.volume,
                    state.lpf_cutoff,
                    state.audio_peak,
                    state.vfo_b,
                    state.bookmarks.bookmarks.len(),
                    state.lo_offset_hz,
                    state.source.ppm_correction,
                    !state.freq_history.is_empty(),
                    cfg.ai_reasoning_effort.clone(),
                    cfg.ai_web_search,
                ))
            } else {
                None
            }
        };

        let (
            endpoint,
            model,
            api_key,
            provider,
            max_tokens,
            temperature,
            system_prompt,
            freq,
            rate,
            gain,
            mode,
            recording,
            sat,
            adsb,
            noise_floor,
            peak_db,
            squelch,
            volume,
            lpf_cutoff,
            audio_peak,
            vfo_b,
            bookmark_count,
            lo_offset_hz,
            ppm,
            has_history,
            reasoning_effort,
            web_search_enabled,
        ) = match state_snapshot {
            Some(s) => s,
            None => {
                return (
                    vec![],
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    0,
                    0.0,
                    String::new(),
                    String::new(),
                    false,
                )
            }
        };

        let snr = peak_db - noise_floor;

        let mut msgs: Vec<serde_json::Value> = Vec::new();

        let sys = if system_prompt.is_empty() {
            let vfo_b_info = if vfo_b > 0 {
                format!(
                    "\n - VFO-B: {:.4} MHz (offset {:.3} MHz from VFO-A)",
                    vfo_b as f64 / 1e6,
                    (vfo_b as f64 - freq as f64) / 1e6
                )
            } else {
                String::new()
            };
            let lo_info = if lo_offset_hz != 0 {
                format!("\n - LO offset: {lo_offset_hz:+} Hz")
            } else {
                String::new()
            };
            let ppm_info = if ppm != 0 {
                format!("\n - PPM correction: {ppm:+} ppm")
            } else {
                String::new()
            };
            format!(
                "{}\n\nCurrent SDR state:\
                 \n - Frequency: {:.4} MHz\
                 \n - Sample rate: {:.3} MSps\
                 \n - Gain: {:.1} dB\
                 \n - Demod mode: {}\
                 \n - Noise floor: {:.1} dB\
                 \n - Peak signal: {:.1} dB\
                 \n - SNR: {:.1} dB\
                 \n - Squelch: {:.1} dB\
                 \n - Volume: {:.0}%\
                 \n - Audio LPF: {:.0} Hz\
                 \n - Audio peak: {:.1} dB\
                 \n - Recording: {}\
                 \n - Satellite: {}\
                 \n - ADS-B: {}\
                 \n - Bookmarks: {}{}{}{}\
                 \n - Recent frequency history: {}",
                DEFAULT_SYSTEM,
                freq as f64 / 1e6,
                f64::from(rate) / 1e6,
                gain,
                mode,
                noise_floor,
                peak_db,
                snr,
                squelch,
                volume * 100.0,
                lpf_cutoff,
                audio_peak,
                if recording { "yes" } else { "no" },
                sat.as_deref().unwrap_or("none"),
                if adsb { "running" } else { "stopped" },
                bookmark_count,
                vfo_b_info,
                lo_info,
                ppm_info,
                if has_history { "available" } else { "empty" },
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
            // Vision messages use array content format for OpenAI/Anthropic compatibility
            let mut obj = if let Some(ref url) = m.image_url {
                serde_json::json!({
                    "role": m.role,
                    "content": [
                        {"type": "text", "text": m.content},
                        {"type": "image_url", "image_url": {"url": url}},
                    ],
                })
            } else {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            };
            if let Some(ref calls) = m.tool_calls {
                let tools: Vec<serde_json::Value> = calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                obj["tool_calls"] = serde_json::json!(tools);
            }
            msgs.push(obj);
        }

        (
            msgs,
            endpoint,
            model,
            api_key,
            provider,
            max_tokens,
            temperature,
            sys,
            reasoning_effort,
            web_search_enabled,
        )
    }

    pub fn send_message(&mut self) {
        if self.input.is_empty() || self.thinking {
            return;
        }
        let user_msg = self.input.trim().to_string();
        if user_msg.is_empty() {
            return;
        }
        self.input.clear();
        self.messages.push({
            let mut m = ChatMessage::new("user", &user_msg);
            m.streaming = false;
            m
        });

        // Start assistant message placeholder
        self.messages.push({
            let mut m = ChatMessage::new("assistant", "");
            m.streaming = true;
            m
        });
        self.thinking = true;
        self.stream_start = Some(std::time::Instant::now());

        let (
            api_messages,
            endpoint,
            model,
            api_key,
            provider,
            max_tokens,
            temperature,
            system_prompt,
            reasoning_effort,
            web_search_enabled,
        ) = self.build_api_messages();

        let needs_key = PROVIDER_PRESETS
            .iter()
            .find(|p| p.name == provider)
            .is_none_or(|p| p.needs_key);

        if needs_key && api_key.is_empty() {
            if let Some(last) = self.messages.last_mut() {
                last.streaming = false;
                last.content = format!(
                    "⚠ No API key set for {provider}. Go to Settings → AI Agent to add one."
                );
            }
            self.thinking = false;
            return;
        }

        // Reset abort flag before spawning new thread
        self.abort_flag.store(false, Ordering::Relaxed);
        let abort_flag = self.abort_flag.clone();

        let (evt_tx, evt_rx) = crossbeam_channel::bounded::<StreamEvent>(256);
        let is_anthropic = provider == "Anthropic";

        std::thread::spawn(move || {
            let client = match reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = evt_tx.send(StreamEvent::Error(format!("Client error: {e}")));
                    return;
                }
            };

            let params = StreamParams {
                client: &client,
                evt_tx: &evt_tx,
                endpoint: &endpoint,
                api_key: &api_key,
                model: &model,
                api_messages: &api_messages,
                max_tokens,
                temperature,
                abort_flag: &abort_flag,
                reasoning_effort: &reasoning_effort,
            };
            if is_anthropic {
                Self::stream_anthropic(&params, &system_prompt);
            } else {
                Self::stream_openai_compat(&params, web_search_enabled);
            }
        });

        self.pending_rx = Some(evt_rx);
    }

    fn stream_openai_compat(params: &StreamParams, web_search_enabled: bool) {
        let StreamParams {
            client,
            evt_tx,
            endpoint,
            api_key,
            model,
            api_messages,
            max_tokens,
            temperature,
            abort_flag,
            reasoning_effort,
        } = *params;

        let mut body = serde_json::json!({
            "model": model,
            "messages": api_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": true,
        });

        if reasoning_effort != "off" {
            body["reasoning_effort"] = serde_json::json!(reasoning_effort);
        }

        if web_search_enabled {
            body["tools"] = serde_json::json!([
                {"type": "web_search"}
            ]);
        }

        let mut req = client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .json(&body);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {api_key}"));
        }

        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                let _ = evt_tx.send(StreamEvent::Error(format!("HTTP error: {e}")));
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            let _ = evt_tx.send(StreamEvent::Error(format!("HTTP {status}: {text}")));
            return;
        }

        let mut full_text = String::new();
        let reader = BufReader::new(resp);
        for line in reader.lines() {
            if abort_flag.load(Ordering::Relaxed) {
                let _ = evt_tx.send(StreamEvent::Done(()));
                return;
            }
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

        Self::dispatch_tool_calls(evt_tx, &full_text);
        let _ = evt_tx.send(StreamEvent::Done(()));
    }

    fn stream_anthropic(params: &StreamParams, system_prompt: &str) {
        let StreamParams {
            client,
            evt_tx,
            endpoint,
            api_key,
            model,
            api_messages,
            max_tokens,
            temperature,
            abort_flag,
            reasoning_effort,
        } = *params;
        let non_system: Vec<&serde_json::Value> = api_messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .collect();

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "system": system_prompt,
            "messages": non_system,
            "stream": true,
        });

        if reasoning_effort != "off" {
            // Anthropic uses thinking.budget_tokens instead of reasoning_effort string
            let budget = match reasoning_effort {
                "low" => 2000,
                "medium" => 5000,
                "high" => 10000,
                _ => 0,
            };
            if budget > 0 {
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget,
                });
                // When thinking is enabled, temperature must be 1.0 for Anthropic
                body["temperature"] = serde_json::json!(1.0);
            }
        }

        let resp = match client
            .post(endpoint)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                let _ = evt_tx.send(StreamEvent::Error(format!("HTTP error: {e}")));
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            let _ = evt_tx.send(StreamEvent::Error(format!(
                "Anthropic HTTP {status}: {text}"
            )));
            return;
        }

        let mut full_text = String::new();
        let reader = BufReader::new(resp);
        for line in reader.lines() {
            if abort_flag.load(Ordering::Relaxed) {
                let _ = evt_tx.send(StreamEvent::Done(()));
                return;
            }
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if !line.starts_with("data: ") {
                continue;
            }
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

        Self::dispatch_tool_calls(evt_tx, &full_text);
        let _ = evt_tx.send(StreamEvent::Done(()));
    }

    /// Tools that never mutate radio/app state and are safe to run the moment
    /// the model emits them. Everything else (tune/gain/record/bookmarks/…)
    /// waits in `pending_tool_calls` for user approval. Unknown names fall
    /// through to the `_` arm of `execute_tool_call`, which mutates nothing,
    /// so they execute immediately (returning an "Unknown tool" message).
    fn is_read_only_tool(name: &str) -> bool {
        matches!(name, "get_status" | "get_freq_history" | "web_search")
    }

    /// Approve a queued tool call by index, executing it now. Returns the
    /// result string, or `None` when `idx` is out of range.
    fn approve_pending(&mut self, idx: usize) -> Option<String> {
        if idx >= self.pending_tool_calls.len() {
            return None;
        }
        let (tool, args) = self.pending_tool_calls.remove(idx);
        Some(self.execute_tool_call(&tool, &args))
    }

    /// Drop a queued tool call by index without executing it.
    fn deny_pending(&mut self, idx: usize) -> bool {
        if idx >= self.pending_tool_calls.len() {
            return false;
        }
        self.pending_tool_calls.remove(idx);
        true
    }

    /// Find all {"tool": ..., "args": ...} JSON objects in text and emit a `ToolCallDetected` for each.
    fn dispatch_tool_calls(evt_tx: &crossbeam_channel::Sender<StreamEvent>, text: &str) {
        for (tool, args) in Self::extract_tool_calls(text) {
            let _ = evt_tx.send(StreamEvent::ToolCallDetected { tool, args });
        }
    }

    /// Scan text for all JSON objects matching {"tool": ..., "args": ...} using brace counting.
    fn extract_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
        let mut results = Vec::new();
        let mut search_from = 0;

        while search_from < text.len() {
            // Find next candidate
            let Some(rel_start) = text[search_from..].find("{\"tool\"") else {
                break;
            };
            let abs_start = search_from + rel_start;
            let slice = &text[abs_start..];

            // Count braces to extract the full JSON object
            let mut depth = 0i32;
            let mut end = 0;
            for (i, ch) in slice.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if end == 0 {
                break;
            }

            let candidate = &slice[..end];
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(candidate) {
                if let Some(name) = json.get("tool").and_then(|v| v.as_str()) {
                    let args = json.get("args").cloned().unwrap_or(serde_json::json!({}));
                    results.push((name.to_string(), args));
                }
            }

            search_from = abs_start + end;
        }

        results
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
                    // Untrusted model output must not drive the radio
                    // sight-unseen: mutating tools wait for approval.
                    if self.auto_approve_tools || Self::is_read_only_tool(&tool) {
                        let result = self.execute_tool_call(&tool, &args);
                        if let Some(last) = self.messages.last_mut() {
                            let tc = ToolCall {
                                name: tool.clone(),
                                arguments: args.clone(),
                            };
                            let mut calls = last.tool_calls.take().unwrap_or_default();
                            calls.push(tc);
                            last.tool_calls = Some(calls);
                            last.content
                                .push_str(&format!("\n\u{25b6} {tool} \u{2192} {result}"));
                        }
                    } else {
                        self.pending_tool_calls.push((tool.clone(), args.clone()));
                        if let Some(last) = self.messages.last_mut() {
                            last.content
                                .push_str(&format!("\n\u{23f3} `{tool}` awaiting approval below."));
                        }
                    }
                }
                Ok(StreamEvent::Done(())) => {
                    if let Some(last) = self.messages.last_mut() {
                        last.streaming = false;
                    }
                    self.stream_start = None;
                    keep = false;
                    break;
                }
                Ok(StreamEvent::Error(err)) => {
                    // Replace the placeholder with the error message
                    if let Some(last) = self.messages.last_mut() {
                        last.streaming = false;
                        last.content = format!("⚠ {err}");
                    }
                    self.stream_start = None;
                    keep = false;
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    if let Some(last) = self.messages.last_mut() {
                        last.streaming = false;
                    }
                    self.stream_start = None;
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
                        // Clamp to the radio tuning range: an unclamped model
                        // value (or MHz/Hz confusion) must not park the SDR
                        // at 0 Hz or 18 EHz.
                        let hz = hz.clamp(500_000, 1_770_000_000);
                        state.source.frequency_hz = hz;
                        return format!("Tuned to {:.3} MHz", hz as f64 / 1e6);
                    }
                    return "Error: missing hz argument".to_string();
                }
                "set_gain" => {
                    if let Some(db) = args["db"].as_f64() {
                        if !db.is_finite() {
                            return "Error: gain must be finite".to_string();
                        }
                        state.source.gain_db = db;
                        return format!("Gain set to {db:.1} dB");
                    }
                    return "Error: missing db argument".to_string();
                }
                "set_sample_rate" => {
                    if let Some(rate) = args["rate"].as_u64() {
                        if rate == 0 {
                            return "Error: sample rate must be non-zero".to_string();
                        }
                        // Saturate instead of `as u32` truncation past 4 GHz.
                        let rate = rate.min(u64::from(u32::MAX)) as u32;
                        state.source.sample_rate_hz = rate;
                        return format!("Sample rate set to {:.3} MSps", rate as f64 / 1e6);
                    }
                    return "Error: missing rate argument".to_string();
                }
                "toggle_bias_tee" => {
                    if let Some(on) = args["on"].as_bool() {
                        state.source.bias_tee = on;
                        return format!("Bias tee {}", if on { "ON" } else { "OFF" });
                    }
                    return "Error: missing on argument".to_string();
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
                    return "Error: missing mode argument".to_string();
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
                    if let Some(sat) = args["name"].as_str() {
                        state.selected_satellite = Some(sat.to_string());
                        return format!("Satellite '{sat}' selected");
                    }
                    return "Error: missing name argument".to_string();
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
                        if !db.is_finite() {
                            return "Error: squelch must be finite".to_string();
                        }
                        state.squelch = db as f32;
                        return format!("Squelch set to {db:.1} dB");
                    }
                    return "Error: missing db argument".to_string();
                }
                "set_volume" => {
                    if let Some(level) = args["level"].as_f64() {
                        state.volume = (level as f32).clamp(0.0, 1.0);
                        return format!("Volume set to {:.0}%", level * 100.0);
                    }
                    return "Error: missing level argument".to_string();
                }
                "get_status" => {
                    return serde_json::json!({
                        "frequency_mhz": state.source.frequency_hz as f64 / 1e6,
                        "gain_db": state.source.gain_db,
                        "demod": state.demod_mode.label(),
                        "sample_rate_msps": f64::from(state.source.sample_rate_hz) / 1e6,
                        "squelch_db": state.squelch,
                        "volume": state.volume,
                        "lpf_cutoff_hz": state.lpf_cutoff,
                        "recording": state.recording,
                        "adsb_running": state.adsb_running,
                        "peak_db": state.spectrum.peak_level(),
                        "noise_floor_db": state.spectrum.noise_floor(),
                        "snr_db": state.spectrum.peak_level() - state.spectrum.noise_floor(),
                        "bookmark_count": state.bookmarks.bookmarks.len(),
                        "lo_offset_hz": state.lo_offset_hz,
                        "ppm_correction": state.source.ppm_correction,
                    })
                    .to_string();
                }
                "get_freq_history" => {
                    let history: Vec<String> = state
                        .freq_history
                        .iter()
                        .map(|&hz| format!("{:.4} MHz", hz as f64 / 1e6))
                        .collect();
                    if history.is_empty() {
                        return "No frequency history yet".to_string();
                    }
                    return format!("Recent frequencies (oldest→newest): {}", history.join(", "));
                }
                "add_bookmark" => {
                    let name = args["name"].as_str().unwrap_or("AI Bookmark").to_string();
                    let hz = args["hz"]
                        .as_u64()
                        .unwrap_or(state.source.frequency_hz)
                        .clamp(500_000, 1_770_000_000);
                    let mode = args["mode"]
                        .as_str()
                        .unwrap_or(state.demod_mode.label())
                        .to_string();
                    let notes = args["notes"].as_str().unwrap_or("").to_string();
                    let freq_mhz = hz as f64 / 1e6;
                    state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                        name: name.clone(),
                        frequency_hz: hz,
                        mode,
                        bandwidth_hz: 12_500,
                        category: "AI".to_string(),
                        notes,
                        starred: false,
                    });
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                    return format!("Bookmark '{name}' saved at {freq_mhz:.4} MHz");
                }
                "set_lpf_cutoff" => {
                    if let Some(hz) = args["hz"].as_f64() {
                        if !hz.is_finite() || hz <= 0.0 {
                            return "Error: LPF cutoff must be positive".to_string();
                        }
                        let hz = hz.clamp(50.0, 24_000.0);
                        state.lpf_cutoff = hz as f32;
                        return format!("Audio LPF cutoff set to {hz:.0} Hz");
                    }
                    return "Error: missing hz argument".to_string();
                }
                "set_ppm" => {
                    if let Some(ppm) = args["ppm"].as_i64() {
                        // Real TCXOs are tens of ppm; clamp generously so a
                        // hallucinated value can't push tuning math absurd.
                        let ppm = ppm.clamp(-10_000, 10_000);
                        state.source.ppm_correction = ppm as i32;
                        return format!("PPM correction set to {ppm} ppm");
                    }
                    return "Error: missing ppm argument".to_string();
                }
                "configure_scanner" => {
                    // Merge into any command still pending from an earlier tool
                    // call this frame so sequential calls don't clobber each other.
                    let mut cmd = state.scanner_command.take().unwrap_or_default();
                    let mut applied: Vec<String> = Vec::new();
                    if let Some(mhz) = args["start_mhz"].as_f64() {
                        let mhz = mhz.clamp(0.5, 1770.0);
                        cmd.start_hz = Some((mhz * 1e6) as u64);
                        applied.push(format!("start {mhz:.3} MHz"));
                    }
                    if let Some(mhz) = args["stop_mhz"].as_f64() {
                        let mhz = mhz.clamp(0.5, 1770.0);
                        cmd.stop_hz = Some((mhz * 1e6) as u64);
                        applied.push(format!("stop {mhz:.3} MHz"));
                    }
                    if let Some(khz) = args["step_khz"].as_f64() {
                        cmd.step_hz = Some((khz * 1e3) as u64);
                        applied.push(format!("step {khz:.1} kHz"));
                    }
                    if let Some(ms) = args["dwell_ms"].as_u64() {
                        cmd.dwell_ms = Some(ms);
                        applied.push(format!("dwell {ms} ms"));
                    }
                    if let Some(db) = args["threshold_db"].as_f64() {
                        cmd.threshold_db = Some(db as f32);
                        applied.push(format!("threshold {db:.0} dB"));
                    }
                    if let Some(run) = args["run"].as_bool() {
                        cmd.run = Some(run);
                        applied.push(if run {
                            "start sweep".to_string()
                        } else {
                            "stop sweep".to_string()
                        });
                    }
                    if applied.is_empty() {
                        return "Error: configure_scanner needs at least one of start_mhz, stop_mhz, step_khz, dwell_ms, threshold_db, run".to_string();
                    }
                    state.scanner_command = Some(cmd);
                    return format!("Scanner configured: {}", applied.join(", "));
                }
                "web_search" => {
                    if let Some(q) = args["query"].as_str() {
                        drop(state); // Release lock before network call
                        return Self::web_search(q);
                    }
                    return "Error: missing query argument".to_string();
                }
                _ => return format!("Unknown tool: {name}"),
            }
        }
        "Error: could not access SDR state".to_string()
    }

    /// Search the web using `DuckDuckGo` Lite. Returns formatted results.
    fn web_search(query: &str) -> String {
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (compatible; EZ-SDR/1.0)")
            .build()
        {
            Ok(c) => c,
            Err(e) => return format!("Search client error: {e}"),
        };

        let url = format!(
            "https://lite.duckduckgo.com/lite/?q={}",
            urlencoding::encode(query)
        );
        let resp = match client.get(&url).send() {
            Ok(r) => r,
            Err(e) => return format!("Search request failed: {e}"),
        };

        let html = match resp.text() {
            Ok(t) => t,
            Err(e) => return format!("Failed to read search response: {e}"),
        };

        // Parse DuckDuckGo Lite HTML for result snippets
        // The layout has <td class="result-snippet"> for snippets and <a class="result-link"> for URLs
        let mut results = Vec::new();
        let mut remaining = html.as_str();

        // Extract result snippets
        while let Some(start) = remaining.find("class=\"result-snippet\"") {
            let after_tag = &remaining[start + 22..];
            if let Some(end) = after_tag.find("</td>") {
                let snippet_html = &after_tag[..end];
                // Strip HTML tags from snippet
                let mut clean = String::new();
                let mut in_tag = false;
                for ch in snippet_html.chars() {
                    match ch {
                        '<' => in_tag = true,
                        '>' => in_tag = false,
                        '&' => { /* skip HTML entities */ }
                        _ if !in_tag => clean.push(ch),
                        _ => {}
                    }
                }
                let clean = clean.trim().to_string();
                if !clean.is_empty() {
                    results.push(clean);
                }
                remaining = &after_tag[end..];
            } else {
                break;
            }
        }

        if results.is_empty() {
            // Fallback: try to extract any text content between <body> tags
            if let Some(body_start) = html.find("<body>") {
                if let Some(body_end) = html.find("</body>") {
                    let body = &html[body_start + 6..body_end];
                    let mut text = String::new();
                    let mut in_tag = false;
                    for ch in body.chars() {
                        match ch {
                            '<' => in_tag = true,
                            '>' => in_tag = false,
                            _ if !in_tag && !ch.is_control() => text.push(ch),
                            _ => {}
                        }
                    }
                    let text = text.split_whitespace().collect::<Vec<&_>>().join(" ");
                    if !text.is_empty() {
                        return format!(
                            "Search results for '{}':\n{}",
                            query,
                            text.chars().take(2000).collect::<String>()
                        );
                    }
                }
            }
            return format!("No results found for '{query}'");
        }

        let mut output = format!(
            "Search results for '{}' ({} results):\n\n",
            query,
            results.len().min(5)
        );
        for (i, snippet) in results.iter().take(5).enumerate() {
            output.push_str(&format!("{}. {}\n\n", i + 1, snippet));
        }
        output
    }

    /// Scan `text` for the first frequency mention (e.g. "137.1 MHz", "1090 MHz", "433 kHz").
    /// Returns (`start_byte`, `end_byte`, hz) or None.
    fn find_next_freq(text: &str) -> Option<(usize, usize, u64)> {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() {
                let num_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'.' {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let num_end = i;
                let suffix = &text[num_end..];

                let (mult, slen): (f64, usize) = if suffix.starts_with(" MHz") {
                    (1e6, 4)
                } else if suffix.starts_with("MHz") {
                    (1e6, 3)
                } else if suffix.starts_with(" GHz") {
                    (1e9, 4)
                } else if suffix.starts_with("GHz") {
                    (1e9, 3)
                } else if suffix.starts_with(" kHz") {
                    (1e3, 4)
                } else if suffix.starts_with("kHz") {
                    (1e3, 3)
                } else {
                    (0.0, 0)
                };

                if mult > 0.0 {
                    if let Ok(val) = text[num_start..num_end].parse::<f64>() {
                        if val > 0.0 {
                            let hz = (val * mult) as u64;
                            return Some((num_start, num_end + slen, hz));
                        }
                    }
                }
                // Not a frequency — keep scanning from num_end
            } else {
                i += 1;
            }
        }
        None
    }

    /// Tokenize a line into (text, bold, italic, code, `freq_hz`) spans for inline rendering.
    fn tokenize_inline(text: &str) -> Vec<(String, bool, bool, bool, Option<u64>)> {
        enum Span {
            Plain(String),
            Bold(String),
            Italic(String),
            Code(String),
        }

        // Split on **bold**, *italic*, and `code` markers.
        let mut spans: Vec<Span> = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            if rest.starts_with("**") {
                if let Some(close) = rest[2..].find("**") {
                    spans.push(Span::Bold(rest[2..2 + close].to_string()));
                    rest = &rest[2 + close + 2..];
                    continue;
                }
                spans.push(Span::Plain("**".to_string()));
                rest = &rest[2..];
                continue;
            }
            if rest.starts_with('*') {
                if let Some(close) = rest[1..].find('*') {
                    if close > 0 {
                        spans.push(Span::Italic(rest[1..=close].to_string()));
                        rest = &rest[1 + close + 1..];
                        continue;
                    }
                }
                spans.push(Span::Plain("*".to_string()));
                rest = &rest[1..];
                continue;
            }
            if rest.starts_with('`') {
                if let Some(close) = rest[1..].find('`') {
                    spans.push(Span::Code(rest[1..=close].to_string()));
                    rest = &rest[1 + close + 1..];
                    continue;
                }
                spans.push(Span::Plain("`".to_string()));
                rest = &rest[1..];
                continue;
            }
            // Advance to the next marker
            let next = rest
                .find("**")
                .or_else(|| rest.find('*'))
                .or_else(|| rest.find('`'))
                .unwrap_or(rest.len());
            spans.push(Span::Plain(rest[..next].to_string()));
            rest = &rest[next..];
        }

        // Expand Plain spans on frequency mentions.
        let mut result: Vec<(String, bool, bool, bool, Option<u64>)> = Vec::new();
        for span in spans {
            match span {
                Span::Bold(s) => result.push((s, true, false, false, None)),
                Span::Italic(s) => result.push((s, false, true, false, None)),
                Span::Code(s) => result.push((s, false, false, true, None)),
                Span::Plain(s) => {
                    let mut sub = s.as_str();
                    while !sub.is_empty() {
                        if let Some((start, end, hz)) = Self::find_next_freq(sub) {
                            if start > 0 {
                                result.push((sub[..start].to_string(), false, false, false, None));
                            }
                            result.push((
                                sub[start..end].to_string(),
                                false,
                                false,
                                false,
                                Some(hz),
                            ));
                            sub = &sub[end..];
                        } else {
                            result.push((sub.to_string(), false, false, false, None));
                            break;
                        }
                    }
                }
            }
        }
        result
    }

    /// Render one line of prose with frequency mentions rendered as blue clickable links,
    /// and **bold** / *italic* markdown rendered inline.
    /// Returns Some(hz) if a link was clicked this frame.
    fn render_line_with_freqs(
        ui: &mut egui::Ui,
        line: &str,
        text_color: egui::Color32,
    ) -> Option<u64> {
        let mut clicked: Option<u64> = None;
        ui.horizontal_wrapped(|ui| {
            for (text, bold, italic, code, freq_hz) in Self::tokenize_inline(line) {
                if text.is_empty() {
                    continue;
                }
                if let Some(hz) = freq_hz {
                    let link_text = egui::RichText::new(&text)
                        .color(egui::Color32::from_rgb(80, 180, 255))
                        .underline();
                    let resp = ui
                        .add(egui::Label::new(link_text).sense(egui::Sense::click()))
                        .on_hover_text(format!("🎯 Tune to {:.4} MHz", hz as f64 / 1e6));
                    if resp.clicked() {
                        clicked = Some(hz);
                    }
                } else if code {
                    ui.monospace(
                        egui::RichText::new(&text)
                            .color(egui::Color32::from_rgb(200, 200, 100))
                            .background_color(egui::Color32::from_black_alpha(80)),
                    );
                } else {
                    let mut rich = egui::RichText::new(&text).color(text_color);
                    if bold {
                        rich = rich.strong();
                    }
                    if italic {
                        rich = rich.italics();
                    }
                    ui.label(rich);
                }
            }
        });
        clicked
    }

    /// Render a prose block (no code fences) line-by-line so \n shows as line breaks.
    /// Handles markdown headings (#, ##, ###), bullet lists (- item, * item, N. item),
    /// and clickable frequency links.
    fn render_prose(ui: &mut egui::Ui, text: &str, text_color: egui::Color32) -> Option<u64> {
        let mut clicked: Option<u64> = None;
        for line in text.split('\n') {
            if line.is_empty() {
                ui.add_space(2.0);
                continue;
            }

            // Heading: ### / ## / #
            if let Some(rest) = line.strip_prefix("### ") {
                ui.label(egui::RichText::new(rest).strong().color(text_color));
            } else if let Some(rest) = line.strip_prefix("## ") {
                ui.label(
                    egui::RichText::new(rest)
                        .strong()
                        .heading()
                        .color(text_color),
                );
            } else if let Some(rest) = line.strip_prefix("# ") {
                ui.label(
                    egui::RichText::new(rest)
                        .strong()
                        .heading()
                        .color(text_color),
                );
            // Horizontal rule
            } else if line == "---" || line == "***" || line == "___" {
                ui.separator();
            // Blockquote: "> text"
            } else if let Some(body) = line.strip_prefix("> ") {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Separator::default().vertical().spacing(6.0));
                    ui.label(
                        egui::RichText::new(body)
                            .color(egui::Color32::from_gray(180))
                            .italics(),
                    );
                });
            // Bullet: "- text" or "* text"
            } else if (line.starts_with("- ") || line.starts_with("* ")) && !line.starts_with("**")
            {
                let body = &line[2..];
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("•").color(text_color));
                    if let Some(hz) = Self::render_line_with_freqs(ui, body, text_color) {
                        clicked = Some(hz);
                    }
                });
            // Numbered list: "1. text", "2. text", etc.
            } else if line.len() > 2 && line.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                let dot_pos = line.find(". ");
                if let Some(dot) = dot_pos {
                    let num = &line[..dot];
                    let body = &line[dot + 2..];
                    if num.chars().all(|c| c.is_ascii_digit()) {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(format!("{num}.")).color(text_color));
                            if let Some(hz) = Self::render_line_with_freqs(ui, body, text_color) {
                                clicked = Some(hz);
                            }
                        });
                        continue;
                    }
                }
                if let Some(hz) = Self::render_line_with_freqs(ui, line, text_color) {
                    clicked = Some(hz);
                }
            } else if let Some(hz) = Self::render_line_with_freqs(ui, line, text_color) {
                clicked = Some(hz);
            }
        }
        clicked
    }

    /// Render message content: handles ``` code fences, newlines, and clickable frequencies.
    /// Returns Some(hz) if the user clicked a frequency link.
    fn render_message_content(
        ui: &mut egui::Ui,
        content: &str,
        text_color: egui::Color32,
    ) -> Option<u64> {
        let mut clicked: Option<u64> = None;
        if content.contains("```") {
            let mut in_code = false;
            for block in content.split("```") {
                if in_code {
                    let code = if let Some(nl) = block.find('\n') {
                        &block[nl + 1..]
                    } else {
                        block
                    };
                    let code = code.trim_end();
                    if !code.is_empty() {
                        ui.group(|ui| {
                            ui.monospace(
                                egui::RichText::new(code)
                                    .color(egui::Color32::from_rgb(200, 200, 100)),
                            );
                        });
                    }
                } else if !block.is_empty() {
                    if let Some(hz) = Self::render_prose(ui, block, text_color) {
                        clicked = Some(hz);
                    }
                }
                in_code = !in_code;
            }
        } else {
            clicked = Self::render_prose(ui, content, text_color);
        }
        clicked
    }

    /// Send a single-shot (non-streaming) vision request. Returns the full response text.
    /// Used by EditorPanel for AI recommend, not for the chat UI.
    #[allow(dead_code)]
    pub fn send_vision_request(
        endpoint: &str,
        api_key: &str,
        model: &str,
        provider: &str,
        messages: &[serde_json::Value],
    ) -> Result<String, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("Client error: {e}"))?;

        let is_anthropic = provider == "Anthropic";

        let body = if is_anthropic {
            // Anthropic format: separate system, content blocks with image
            let system = messages
                .iter()
                .find(|m| m["role"] == "system")
                .map(|m| m["content"].clone())
                .unwrap_or(serde_json::json!(""));
            let non_system: Vec<&serde_json::Value> = messages
                .iter()
                .filter(|m| m["role"].as_str() != Some("system"))
                .collect();
            serde_json::json!({
                "model": model,
                "max_tokens": 4096,
                "system": system,
                "messages": non_system,
            })
        } else {
            serde_json::json!({
                "model": model,
                "messages": messages,
                "max_tokens": 4096,
            })
        };

        let mut req = client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .json(&body);

        if is_anthropic {
            req = req
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        } else if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {api_key}"));
        }

        let resp = req.send().map_err(|e| format!("HTTP error: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("HTTP {status}: {text}"));
        }

        let json: serde_json::Value = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        if is_anthropic {
            // Anthropic: content[0].text
            json["content"][0]["text"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("Unexpected Anthropic response: {json}"))
        } else {
            // OpenAI: choices[0].message.content
            json["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("Unexpected response: {json}"))
        }
    }

    /// Encode raw bytes as a data URL with base64.
    #[allow(dead_code)]
    pub fn base64_encode(data: &[u8], media_type: &str) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            if chunk.len() > 2 {
                result.push(CHARS[(triple & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
        }
        format!("data:{media_type};base64,{result}")
    }

    fn export_chat(&self) {
        let mut text = String::new();
        for msg in &self.messages {
            if msg.role == "system" {
                continue;
            }
            let label = match msg.role.as_str() {
                "user" => "You",
                "assistant" => "AI",
                _ => &msg.role,
            };
            text.push_str(&format!(
                "[{}] {}: {}\n\n",
                msg.format_time(),
                label,
                msg.content
            ));
        }
        if text.is_empty() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("ez_sdr_ai_chat.txt")
            .add_filter("Text", &["txt"])
            .save_file()
        {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll_stream();

        // Header with model/provider info and context token estimate
        let (model, provider, has_key, temp, reasoning_effort, web_search) = {
            if let Ok(state) = self.shared.try_lock() {
                let needs_key = PROVIDER_PRESETS
                    .iter()
                    .find(|p| p.name == state.config.ai_provider)
                    .is_none_or(|p| p.needs_key);
                (
                    state.config.ai_model.clone(),
                    state.config.ai_provider.clone(),
                    !needs_key || !state.config.ai_api_key.is_empty(),
                    state.config.ai_temperature,
                    state.config.ai_reasoning_effort.clone(),
                    state.config.ai_web_search,
                )
            } else {
                (
                    DEFAULT_AI_MODEL.to_string(),
                    "?".to_string(),
                    false,
                    0.7,
                    "off".to_string(),
                    false,
                )
            }
        };
        self.temperature = temp;
        self.reasoning_effort = reasoning_effort;
        self.web_search_enabled = web_search;

        let rough_tokens: usize = self.messages.iter().map(|m| m.content.len() / 4).sum();

        ui.horizontal(|ui| {
            ui.heading("AI Agent");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("~{}k tok", rough_tokens / 1000 + 1));
                ui.separator();

                // Reasoning effort selector (compact)
                let re_label = match self.reasoning_effort.as_str() {
                    "low" => "🧠 Low",
                    "medium" => "🧠 Med",
                    "high" => "🧠 High",
                    _ => "🧠 Off",
                };
                egui::ComboBox::from_id_salt("ai_panel_reasoning")
                    .selected_text(re_label)
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for (val, label) in [("off", "Off"), ("low", "Low"), ("medium", "Medium"), ("high", "High")] {
                            if ui.selectable_label(self.reasoning_effort == val, label).clicked() {
                                if let Ok(mut state) = self.shared.try_lock() {
                                    state.config.ai_reasoning_effort = val.to_string();
                                    state.config.save();
                                }
                            }
                        }
                    });

                ui.label(egui::RichText::new("🧠").small())
                    .on_hover_text("Reasoning effort: controls how hard the model thinks. Higher = slower but more thorough.");

                // Web search toggle
                let mut ws = self.web_search_enabled;
                ui.checkbox(&mut ws, "🔍 Search")
                    .on_hover_text("Toggle web search: allow the AI to search the web for external info (DuckDuckGo, no key needed).");
                if ws != self.web_search_enabled {
                    self.web_search_enabled = ws;
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.config.ai_web_search = ws;
                        state.config.save();
                    }
                }

                ui.separator();
                ui.monospace(format!("{provider} · {model}"));
                if !has_key {
                    ui.colored_label(egui::Color32::YELLOW, "⚠ no key")
                        .on_hover_text("Go to Settings → AI Agent to add an API key.");
                }
            });
        });
        ui.separator();

        // Pending tool-call approvals: state-changing calls the model emitted
        // wait here until the user explicitly approves them.
        if !self.pending_tool_calls.is_empty() {
            ui.group(|ui| {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!(
                        "⏳ {} tool call{} awaiting approval",
                        self.pending_tool_calls.len(),
                        if self.pending_tool_calls.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                );
                let mut approve: Option<usize> = None;
                let mut deny: Option<usize> = None;
                for (i, (tool, args)) in self.pending_tool_calls.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.monospace(tool);
                        ui.weak(args.to_string());
                        if ui.small_button("✔ Approve").clicked() {
                            approve = Some(i);
                        }
                        if ui.small_button("✕ Deny").clicked() {
                            deny = Some(i);
                        }
                    });
                }
                if let Some(i) = approve {
                    if let Some(result) = self.approve_pending(i) {
                        if let Some(last) = self.messages.last_mut() {
                            last.content
                                .push_str(&format!("\n\u{25b6} approved \u{2192} {result}"));
                        }
                    }
                } else if let Some(i) = deny {
                    self.deny_pending(i);
                }
                if ui.small_button("Deny all").clicked() {
                    self.pending_tool_calls.clear();
                }
            });
            ui.separator();
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.auto_approve_tools, "Auto-approve tool calls")
                .on_hover_text("When off (recommended), tuning/recording/gain and other state-changing AI actions need one click each. Read-only queries always run immediately.");
        });

        // Collapsing tools reference
        ui.collapsing("Available Tools", |ui| {
            egui::Grid::new("ai_tools_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    let tools = [
                        ("tune_frequency(hz)", "Set center frequency in Hz"),
                        ("set_gain(db)", "RF gain 0–49.6 dB"),
                        ("set_demod(mode)", "AM / NFM / WFM / USB / LSB / RAW"),
                        ("set_sample_rate(rate)", "Sample rate in Hz (e.g. 2048000)"),
                        ("set_squelch(db)", "Squelch threshold in dB"),
                        ("set_volume(level)", "Audio volume 0.0–1.0"),
                        ("set_lpf_cutoff(hz)", "Audio low-pass filter cutoff in Hz"),
                        ("toggle_bias_tee(on)", "Bias tee power for LNAs"),
                        ("start_recording()", "Begin IQ/WAV recording"),
                        ("stop_recording()", "Stop recording"),
                        ("select_satellite(name)", "Auto-track a satellite"),
                        ("start_adsb()", "Start ADS-B decoder at 1090 MHz"),
                        ("stop_adsb()", "Stop ADS-B decoder"),
                        ("get_status()", "Return full SDR state as JSON"),
                        ("get_freq_history()", "Show recently tuned frequencies"),
                        (
                            "add_bookmark(name,hz,mode,notes)",
                            "Save a frequency as a bookmark",
                        ),
                        (
                            "set_ppm(ppm)",
                            "Frequency correction in PPM (oscillator drift)",
                        ),
                        (
                            "web_search(query)",
                            "Search the web for external information",
                        ),
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
                ("📍 Bookmark this", "Get the current SDR status, then add a bookmark for the current frequency with an appropriate name and useful notes about what signal it is."),
                ("📜 History",       "Get the frequency history and tell me what signals I've been looking at. Group similar frequencies together and comment on any patterns."),
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
        let mut retry = false;
        egui::ScrollArea::vertical()
            .id_salt("ai_chat_scroll")
            .max_height(ui.available_height() - 90.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (idx, msg) in self.messages.iter().enumerate() {
                    if msg.role == "system" {
                        continue;
                    }

                    let (role_color, label) = match msg.role.as_str() {
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

                    // Message header: role label + timestamp + copy button
                    let content_to_copy = msg.content.clone();
                    let time_str = msg.format_time();
                    ui.horizontal(|ui| {
                        ui.colored_label(role_color, format!("{label}:"));
                        ui.weak(&time_str);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(format!("📋##{idx}"))
                                .on_hover_text("Copy message to clipboard")
                                .clicked()
                            {
                                ui.ctx().copy_text(content_to_copy.clone());
                            }
                        });
                    });

                    // Message content: newlines, code fences, clickable frequency links
                    if let Some(hz) = Self::render_message_content(ui, &msg.content, content_color) {
                        if let Ok(mut state) = self.shared.try_lock() {
                            state.source.frequency_hz = hz;
                        }
                    }

                    if msg.streaming {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            if let Some(start) = self.stream_start {
                                let elapsed = start.elapsed().as_secs_f32();
                                ui.weak(format!("thinking… {elapsed:.1}s"));
                            } else {
                                ui.weak("streaming…");
                            }
                        });
                    }

                    if is_error && !self.thinking
                        && ui.small_button("↩ Retry").on_hover_text("Resend the last message").clicked() {
                            retry = true;
                        }

                    ui.add_space(6.0);
                }

                if rough_tokens > 3000 {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("⚠ ~{}k context tokens — consider clearing chat if responses seem confused.", rough_tokens / 1000),
                    );
                }
            });

        // Retry: remove the error + last user message, re-queue the user content
        if retry && !self.thinking {
            if self
                .messages
                .last()
                .is_some_and(|m| m.content.starts_with('⚠'))
            {
                self.messages.pop();
            }
            if let Some(pos) = self.messages.iter().rposition(|m| m.role == "user") {
                let user_content = self.messages[pos].content.clone();
                self.messages.truncate(pos);
                self.input = user_content;
                self.send_message();
            }
        }

        ui.separator();

        // Input bar — multiline: Enter sends, Shift+Enter inserts newline
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .hint_text("Ask to tune, record, scan… (Enter to send, Shift+Enter for newline)")
                    .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Enter))),
            );
            let send_clicked = ui.add_enabled(
                !self.thinking,
                egui::Button::new("Send"),
            ).clicked()
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.thinking);

            if send_clicked && !self.input.is_empty() {
                self.send_message();
            }

            // Stop button — only shown while streaming
            if self.thinking
                && ui.button("⬛ Stop")
                    .on_hover_text("Cancel the current request")
                    .clicked()
                {
                    self.abort_flag.store(true, Ordering::Relaxed);
                }

            if ui.add_enabled(!self.thinking, egui::Button::new("Clear"))
                .on_hover_text("Clear conversation history. Useful when the model seems confused due to long context.")
                .clicked()
            {
                self.messages.clear();
            }

            if !self.messages.is_empty()
                && ui.button("💾 Export")
                    .on_hover_text("Save this conversation to a text file")
                    .clicked()
                {
                    self.export_chat();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_new() {
        let msg = ChatMessage::new("user", "hello");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "hello");
        assert!(msg.tool_calls.is_none());
        assert!(!msg.streaming);
        assert!(msg.timestamp_secs > 0);
    }

    #[test]
    fn test_chat_message_format_time() {
        let mut msg = ChatMessage::new("user", "");
        msg.timestamp_secs = 0;
        assert_eq!(msg.format_time(), "00:00");
        msg.timestamp_secs = 3661;
        assert_eq!(msg.format_time(), "01:01");
        msg.timestamp_secs = 86399;
        assert_eq!(msg.format_time(), "23:59");
    }

    #[test]
    fn test_extract_tool_calls() {
        let result = AiPanel::extract_tool_calls("");
        assert!(result.is_empty());

        let text = r#"Some text {"tool": "tune_frequency", "args": {"hz": 100000000}} more text"#;
        let result = AiPanel::extract_tool_calls(text);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "tune_frequency");
        assert_eq!(result[0].1["hz"], 100000000);

        let text = r#"First {"tool": "set_gain", "args": {"db": 20.0}} then {"tool": "tune_frequency", "args": {"hz": 145800000}}"#;
        let result = AiPanel::extract_tool_calls(text);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "set_gain");
        assert_eq!(result[1].0, "tune_frequency");

        let text =
            r#"Some text {"tool": "tune_frequency", "args": {"hz": 100000000} more malformed"#;
        let result = AiPanel::extract_tool_calls(text);
        assert!(result.is_empty());

        let text = "Just some regular text with no tool calls";
        let result = AiPanel::extract_tool_calls(text);
        assert!(result.is_empty());
    }

    #[test]
    fn test_read_only_tool_classification() {
        for t in ["get_status", "get_freq_history", "web_search"] {
            assert!(AiPanel::is_read_only_tool(t), "{t} must be read-only");
        }
        for t in [
            "tune_frequency",
            "set_gain",
            "set_sample_rate",
            "toggle_bias_tee",
            "set_demod",
            "start_recording",
            "stop_recording",
            "select_satellite",
            "start_adsb",
            "stop_adsb",
            "set_squelch",
            "set_volume",
            "add_bookmark",
            "set_lpf_cutoff",
            "set_ppm",
            "configure_scanner",
        ] {
            assert!(!AiPanel::is_read_only_tool(t), "{t} must need approval");
        }
    }

    #[test]
    fn test_approve_deny_pending_tool_calls() {
        let shared = crate::test_helpers::make_shared_state();
        let mut panel = AiPanel::new(shared);
        panel
            .pending_tool_calls
            .push(("set_volume".to_string(), serde_json::json!({"level": 0.5})));
        panel.pending_tool_calls.push((
            "tune_frequency".to_string(),
            serde_json::json!({"hz": 100_000_000}),
        ));
        // Deny drops without executing.
        assert!(panel.deny_pending(0));
        assert_eq!(panel.pending_tool_calls.len(), 1);
        assert!(!panel.deny_pending(7));
        // Approve executes the remaining call.
        let result = panel.approve_pending(0).expect("approve must execute");
        assert!(result.contains("100.000"), "unexpected result: {result}");
        assert!(panel.pending_tool_calls.is_empty());
        assert!(panel.approve_pending(0).is_none());
    }

    #[test]
    fn test_tool_arg_clamps() {
        let shared = crate::test_helpers::make_shared_state();
        let mut panel = AiPanel::new(shared);
        // Absurd tune value clamps into the radio range instead of parking at 0/MAX.
        let result = panel.execute_tool_call("tune_frequency", &serde_json::json!({"hz": 1}));
        assert!(result.contains("0.500"), "unexpected result: {result}");
        // Zero sample rate is rejected, not truncated.
        let result = panel.execute_tool_call("set_sample_rate", &serde_json::json!({"rate": 0}));
        assert!(result.contains("non-zero"), "unexpected result: {result}");
        // Unknown tools execute immediately (they mutate nothing) with a message.
        let result = panel.execute_tool_call("frobnicate", &serde_json::json!({}));
        assert!(
            result.contains("Unknown tool"),
            "unexpected result: {result}"
        );
    }

    #[test]
    fn test_find_next_freq() {
        let result = AiPanel::find_next_freq("100.5 MHz");
        assert_eq!(result, Some((0, 9, 100_500_000)));

        let result = AiPanel::find_next_freq("145.800 MHz and 137.100 MHz");
        assert_eq!(result, Some((0, 11, 145_800_000)));

        let result = AiPanel::find_next_freq("no frequency here");
        assert_eq!(result, None);

        let result = AiPanel::find_next_freq("1000 Hz");
        assert_eq!(result, None);

        let result = AiPanel::find_next_freq("2.4 GHz");
        assert_eq!(result, Some((0, 7, 2_400_000_000)));
    }

    #[test]
    fn test_tokenize_inline() {
        let result = AiPanel::tokenize_inline("hello world");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "hello world");
        assert!(!result[0].1);
        assert!(!result[0].2);
        assert!(!result[0].3);
        assert!(result[0].4.is_none());

        let result = AiPanel::tokenize_inline("use `fn foo()` here");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "use ");
        assert_eq!(result[1].0, "fn foo()");
        assert!(result[1].3);
        assert_eq!(result[2].0, " here");

        let result = AiPanel::tokenize_inline("Tune to 100.5 MHz now");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "Tune to ");
        assert_eq!(result[1].0, "100.5 MHz");
        assert_eq!(result[1].4, Some(100_500_000));
        assert_eq!(result[2].0, " now");
    }

    #[test]
    fn test_ai_panel_new_defaults() {
        let shared = crate::test_helpers::make_shared_state();
        let panel = AiPanel::new(shared);
        assert!(panel.messages.is_empty());
        assert!(panel.input.is_empty());
        assert!(!panel.thinking);
        assert!(panel.pending_rx.is_none());
    }

    #[test]
    fn test_execute_tool_call_functions() {
        let shared = crate::test_helpers::make_shared_state();
        let mut panel = AiPanel::new(shared);

        let result = panel.execute_tool_call("get_status", &serde_json::json!({}));
        assert!(
            !result.is_empty(),
            "get_status should return a non-empty string"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("get_status should return valid JSON");
        assert!(parsed.is_object(), "get_status should return a JSON object");

        let result = panel.execute_tool_call("unknown_tool", &serde_json::json!({}));
        assert!(
            result.contains("Unknown tool"),
            "unknown tool should return error string"
        );
    }

    #[test]
    fn test_find_next_freq_various_patterns() {
        let result = AiPanel::find_next_freq("Tune to 100.5 MHz for the weather");
        assert_eq!(result, Some((8, 17, 100_500_000)));

        let result = AiPanel::find_next_freq("Try 145.800 MHz for ISS");
        assert_eq!(result, Some((4, 15, 145_800_000)));

        let result = AiPanel::find_next_freq("Listen on 2.4 GHz");
        assert_eq!(result, Some((10, 17, 2_400_000_000)));

        let result = AiPanel::find_next_freq("No frequency here");
        assert_eq!(result, None);

        let result = AiPanel::find_next_freq("");
        assert_eq!(result, None);

        let result = AiPanel::find_next_freq("137.100");
        assert_eq!(result, None);
    }

    #[test]
    fn test_tokenize_inline_variants() {
        let result = AiPanel::tokenize_inline("hello world");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "hello world");

        let result = AiPanel::tokenize_inline("use `code` here");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "use ");
        assert_eq!(result[1].0, "code");
        assert!(result[1].3);
        assert_eq!(result[2].0, " here");

        let result = AiPanel::tokenize_inline("Try 100.5 MHz now");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "Try ");
        assert_eq!(result[1].0, "100.5 MHz");
        assert_eq!(result[1].4, Some(100_500_000));
        assert_eq!(result[2].0, " now");

        let result = AiPanel::tokenize_inline("**bold** `code` 100.5 MHz");
        assert_eq!(result.len(), 5);
        assert_eq!(result[0].0, "bold");
        assert!(result[0].1);
        assert_eq!(result[1].0, " ");
        assert_eq!(result[2].0, "code");
        assert!(result[2].3);
        assert_eq!(result[3].0, " ");
        assert_eq!(result[4].0, "100.5 MHz");
        assert_eq!(result[4].4, Some(100_500_000));
    }

    #[test]
    fn test_chat_message_time_edges() {
        let mut msg = ChatMessage::new("user", "");

        msg.timestamp_secs = 0;
        assert_eq!(msg.format_time(), "00:00");

        msg.timestamp_secs = 3600;
        assert_eq!(msg.format_time(), "01:00");

        msg.timestamp_secs = 43200;
        assert_eq!(msg.format_time(), "12:00");

        msg.timestamp_secs = 86399;
        assert_eq!(msg.format_time(), "23:59");
    }

    #[test]
    fn test_ai_panel_abort_flag() {
        let shared = crate::test_helpers::make_shared_state();
        let panel = AiPanel::new(shared);
        assert!(
            !panel.abort_flag.load(Ordering::Relaxed),
            "abort_flag should be false initially"
        );
    }

    #[test]
    fn test_web_search_fn() {
        let result = AiPanel::web_search("");
        assert!(
            !result.is_empty(),
            "web_search should return a non-empty string"
        );
    }

    #[test]
    fn test_chat_message_with_image() {
        let msg = ChatMessage::with_image(
            "user",
            "Analyze this",
            "data:image/png;base64,abc123".to_string(),
        );
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "Analyze this");
        assert_eq!(
            msg.image_url.as_deref(),
            Some("data:image/png;base64,abc123")
        );
    }

    #[test]
    fn test_base64_encode() {
        let data = b"hello world";
        let result = AiPanel::base64_encode(data, "text/plain");
        assert!(result.starts_with("data:text/plain;base64,"));
        assert_eq!(&result[23..], "aGVsbG8gd29ybGQ=");
    }
}
