# AI Recommended Edit — Implementation Plan

## Scope
After decoding a satellite image, user clicks "AI Recommend" → image is sent to the configured AI provider with vision → AI analyzes the image and returns a full processing preset (all parameters) → preset appears in dropdown → user clicks to apply.

## Prerequisites
This plan assumes the Image Editor plan (image-editor.md) is also implemented. The AI recommend feature adds to it.

---

## Step 1: Add vision support to ChatMessage

**File:** `ez-gui/src/ai_panel.rs`

### 1a. Add image_url field to ChatMessage

```rust
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub image_url: Option<String>,  // NEW: base64 data URI for vision
    pub tool_calls: Option<Vec<ToolCall>>,
    pub streaming: bool,
    pub timestamp_secs: u64,
}
```

Update all `ChatMessage { ... }` constructors in the file to include `image_url: None`.

### 1b. Update `build_api_messages()` for OpenAI-compatible providers

In the message assembly loop, when `image_url` is present, use the vision content format:

```rust
// For messages with images, use multimodal content format
if let Some(ref url) = msg.image_url {
    json!({
        "role": msg.role,
        "content": [
            { "type": "text", "text": &msg.content },
            { "type": "image_url", "image_url": { "url": url, "detail": "low" } }
        ]
    })
} else {
    json!({
        "role": msg.role,
        "content": &msg.content
    })
}
```

### 1c. Update `build_api_messages()` for Anthropic

For Anthropic, use the native image format:

```rust
if let Some(ref url) = msg.image_url {
    // Extract base64 data and media type from data URI
    // Format: "data:image/png;base64,iVBOR..."
    let (media_type, data) = parse_data_uri(url);
    json!({
        "role": msg.role,
        "content": [
            { "type": "image", "source": { "type": "base64", "media_type": media_type, "data": data } },
            { "type": "text", "text": &msg.content }
        ]
    })
}
```

Add helper:
```rust
fn parse_data_uri(uri: &str) -> (String, String) {
    // "data:image/png;base64,iVBOR..." -> ("image/png", "iVBOR...")
    let parts: Vec<&str> = uri.splitn(2, ',').collect();
    let media_type = parts[0]
        .trim_start_matches("data:")
        .trim_end_matches(";base64")
        .to_string();
    (media_type, parts[1].to_string())
}
```

---

## Step 2: Add non-streaming vision request method

**File:** `ez-gui/src/ai_panel.rs`

Add a new public method for single-shot vision requests (no streaming, no chat history):

```rust
/// Send a single image + prompt to the AI provider and return the text response.
/// This is a blocking call — runs in a background thread, returns via channel.
pub fn send_vision_request(
    &self,
    image_rgba: &[u8],
    width: u32,
    height: u32,
    prompt: String,
) -> crossbeam_channel::Receiver<String> {
    let (tx, rx) = crossbeam_channel::bounded(1);

    // Encode image as base64 PNG
    let mut png_bytes = Vec::new();
    {
        let img = image::RgbaImage::from_raw(width, height, image_rgba.to_vec()).unwrap();
        let mut cursor = std::io::Cursor::new(&mut png_bytes);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
    }
    let b64 = base64_encode(&png_bytes);
    let data_uri = format!("data:image/png;base64,{b64}");

    // Clone config for the thread
    let endpoint = self.config_endpoint.clone();
    let api_key = self.config_api_key.clone();
    let model = self.config_model.clone();
    let provider = self.config_provider.clone();
    let max_tokens = 1024u32;
    let temperature = 0.3f64;

    std::thread::spawn(move || {
        let result = if provider == "Anthropic" {
            call_anthropic_vision(&endpoint, &api_key, &model, &data_uri, &prompt, max_tokens, temperature)
        } else {
            call_openai_vision(&endpoint, &api_key, &model, &data_uri, &prompt, max_tokens, temperature)
        };
        let _ = tx.send(result.unwrap_or_default());
    });

    rx
}
```

Add base64 encoding (minimal, no external crate needed):
```rust
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 { result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char); } else { result.push('='); }
        if chunk.len() > 2 { result.push(CHARS[(triple & 0x3F) as usize] as char); } else { result.push('='); }
    }
    result
}
```

Add helper functions for the vision API calls:
```rust
fn call_openai_vision(
    endpoint: &str, api_key: &str, model: &str,
    image_data_uri: &str, prompt: &str,
    max_tokens: u32, temperature: f64,
) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": image_data_uri, "detail": "low" } }
            ]
        }]
    });

    let resp = ureq::post(endpoint)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("HTTP error: {e}"))?;

    let json: serde_json::Value = resp.into_json().map_err(|e| format!("JSON error: {e}"))?;
    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    Ok(text.to_string())
}

fn call_anthropic_vision(
    endpoint: &str, api_key: &str, model: &str,
    image_data_uri: &str, prompt: &str,
    max_tokens: u32, temperature: f64,
) -> Result<String, String> {
    // Parse data URI for Anthropic format
    let parts: Vec<&str> = image_data_uri.splitn(2, ',').collect();
    let media_type = parts[0].trim_start_matches("data:").trim_end_matches(";base64").to_string();
    let b64_data = parts[1].to_string();

    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image", "source": { "type": "base64", "media_type": media_type, "data": b64_data } },
                { "type": "text", "text": prompt }
            ]
        }]
    });

    let resp = ureq::post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("HTTP error: {e}"))?;

    let json: serde_json::Value = resp.into_json().map_err(|e| format!("JSON error: {e}"))?;
    let text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    Ok(text.to_string())
}
```

---

## Step 3: Add AI preset recommendation struct and prompt

**File:** `ez-gui/src/editor_panel.rs` (from Image Editor plan)

### 3a. Recommendation struct

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AiRecommendation {
    pub name: String,
    pub description: String,
    pub equalize: bool,
    pub equalize_per_channel: bool,
    pub white_balance: bool,
    pub brightness: f32,
    pub contrast: f32,
    pub hue_shift: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub median_blur: bool,
    pub invert: bool,
}
```

### 3b. Vision prompt

```rust
const AI_RECOMMEND_PROMPT: &str = r#"You are a satellite image processing expert. Analyze this Meteor MSU-MR satellite image and recommend optimal processing settings.

The image has 6 channels (MSU-MR): ch1=visible red, ch2=NIR, ch3=SWIR, ch4=thermal IR, ch5=water vapor, ch6=thermal IR.
This is a composite image from channels you should identify based on visual characteristics.

Respond with ONLY a JSON object (no markdown, no explanation):
{
  "name": "Preset Name",
  "description": "Brief description of what this preset does",
  "equalize": false,
  "equalize_per_channel": false,
  "white_balance": false,
  "brightness": 0.0,
  "contrast": 1.0,
  "hue_shift": 0.0,
  "saturation": 1.0,
  "lightness": 0.0,
  "median_blur": false,
  "invert": false
}

Guidelines:
- If the image appears dark or low-contrast, set equalize=true or white_balance=true
- If channels are mixed VIS+IR, consider equalize_per_channel=true
- Brightness: -1.0 to 1.0 (0 = no change)
- Contrast: 0.0 to 2.0 (1.0 = no change)
- Hue shift: -180 to 180 degrees
- Saturation: 0.0 to 2.0 (1.0 = no change)
- Lightness: -1.0 to 1.0 (0 = no change)
- If thermal channels dominate and appear inverted, set invert=true
- For noisy images, set median_blur=true
- Keep settings minimal — only adjust what clearly needs fixing"#;
```

---

## Step 4: Add "AI Recommend" button and dropdown to editor UI

**File:** `ez-gui/src/editor_panel.rs`

### 4a. Add state fields

```rust
pub struct EditorPanel {
    // ... existing fields ...
    ai_recommended: Option<AiRecommendation>,
    ai_recommending: bool,
    ai_rx: Option<crossbeam_channel::Receiver<String>>,
    ai_error: Option<String>,
}
```

Initialize in `new()`:
```rust
ai_recommended: None,
ai_recommending: false,
ai_rx: None,
ai_error: None,
```

### 4b. Add method to request AI recommendation

```rust
pub fn request_ai_recommend(&mut self, ai_panel: &crate::ai_panel::AiPanel) {
    if self.output_rgba.is_empty() || self.channel_width == 0 || self.channel_height == 0 {
        self.ai_error = Some("No image to analyze".to_string());
        return;
    }

    self.ai_recommending = true;
    self.ai_error = None;
    self.ai_rx = Some(ai_panel.send_vision_request(
        &self.output_rgba,
        self.channel_width,
        self.channel_height,
        AI_RECOMMEND_PROMPT.to_string(),
    ));
}
```

### 4c. Add polling in tick method

```rust
pub fn tick(&mut self) {
    if let Some(ref rx) = self.ai_rx {
        if let Ok(response) = rx.try_recv() {
            self.ai_recommending = false;
            self.ai_rx = None;
            self.parse_ai_response(&response);
        }
    }
}

fn parse_ai_response(&mut self, response: &str) {
    // Try to extract JSON from the response (may be wrapped in markdown)
    let json_str = response
        .trim()
        .trim_start_matches("```json")
        .trim_end_matches("```")
        .trim();

    match serde_json::from_str::<AiRecommendation>(json_str) {
        Ok(rec) => {
            self.ai_recommended = Some(rec);
        }
        Err(e) => {
            self.ai_error = Some(format!("Failed to parse AI response: {e}"));
        }
    }
}
```

### 4d. Add method to apply AI recommendation

```rust
pub fn apply_ai_recommendation(&mut self) {
    if let Some(ref rec) = self.ai_recommended {
        self.pipeline.equalize = rec.equalize;
        self.pipeline.equalize_per_channel = rec.equalize_per_channel;
        self.pipeline.white_balance = rec.white_balance;
        self.pipeline.brightness = rec.brightness;
        self.pipeline.contrast = rec.contrast;
        self.pipeline.hue_shift = rec.hue_shift;
        self.pipeline.saturation = rec.saturation;
        self.pipeline.lightness = rec.lightness;
        self.pipeline.median_blur = rec.median_blur;
        self.pipeline.invert = rec.invert;
        self.dirty = true;
    }
}
```

### 4e. Add UI elements

In `ui_controls()`, add after the composite section:

```rust
// ── AI Recommended Edit ──
ui.separator();
ui.label(egui::RichText::new("AI Assistant").strong());

ui.horizontal(|ui| {
    if ui.button("🤖 AI Recommend")
        .on_hover_text("Send current image to AI for processing recommendations")
        .clicked()
    {
        self.request_ai_recommend(&self.ai_panel_ref);
    }
    if self.ai_recommending {
        ui.spinner();
        ui.label("Analyzing...");
    }
});

if let Some(ref rec) = self.ai_recommended {
    ui.group(|ui| {
        ui.label(egui::RichText::new(&rec.name).strong());
        ui.label(&rec.description);
        if ui.button("✅ Apply This Preset").clicked() {
            self.apply_ai_recommendation();
        }
    });
}

if let Some(ref err) = self.ai_error {
    ui.colored_label(egui::Color32::RED, err.as_str());
}
```

---

## Step 5: Wire AI panel reference into editor

**File:** `ez-gui/src/app.rs`

The editor needs access to the AI panel for sending vision requests. Options:

### Option A: Pass AI panel config to editor (preferred)
Instead of passing the whole `AiPanel`, pass just the config needed for API calls:

```rust
pub struct EditorPanel {
    // ... existing fields ...
    ai_endpoint: String,
    ai_api_key: String,
    ai_model: String,
    ai_provider: String,
}
```

In `app.rs`, when config changes or on init:
```rust
self.editor_panel.set_ai_config(
    &self.ai_panel.endpoint(),
    &self.ai_panel.api_key(),
    &self.ai_panel.model(),
    &self.ai_panel.provider(),
);
```

### Option B: Pass &AiPanel reference to tick()
```rust
// In app.rs logic():
self.editor_panel.tick();
// or
self.editor_panel.tick_with_ai(&self.ai_panel);
```

Option B is simpler but creates borrow conflicts. Option A is cleaner.

**Use Option A.** Add getter methods to AiPanel:
```rust
pub fn endpoint(&self) -> String { self.config_endpoint.clone() }
pub fn api_key(&self) -> String { self.config_api_key.clone() }
pub fn model(&self) -> String { self.config_model.clone() }
pub fn provider(&self) -> String { self.config_provider.clone() }
```

---

## Step 6: Call editor tick from app.rs

**File:** `ez-gui/src/app.rs`

In `logic()`, add:
```rust
// Poll AI vision response for editor
self.editor_panel.tick();
```

---

## Step 7: Auto-load decoded image into editor

**File:** `ez-gui/src/app.rs`

After decode completes and channels are stored in SharedState, load them into editor and render the initial composite:

```rust
// In logic(), after decode completion:
if !self.satellite_panel.decode_running && self.last_decode_running {
    if self.satellite_panel.decode_result.is_some() {
        if let Ok(state) = self.shared.try_lock() {
            if !state.decoded_channels.is_empty() {
                self.editor_panel.load_channels(&state.decoded_channels);
                self.editor_panel.set_ai_config(
                    &self.ai_panel.endpoint(),
                    &self.ai_panel.api_key(),
                    &self.ai_panel.model(),
                    &self.ai_panel.provider(),
                );
                self.satellite_subtab = SatelliteSubTab::Editor;
            }
        }
    }
}
```

---

## Step 8: Tests

### ai_panel.rs tests
```rust
#[test]
fn test_parse_data_uri() {
    let (mt, data) = parse_data_uri("data:image/png;base64,ABC123");
    assert_eq!(mt, "image/png");
    assert_eq!(data, "ABC123");
}

#[test]
fn test_base64_encode() {
    let encoded = base64_encode(b"Hello");
    assert_eq!(encoded, "SGVsbG8=");
}
```

### editor_panel.rs tests
```rust
#[test]
fn test_parse_ai_recommendation_valid() {
    let json = r#"{
        "name": "Enhanced Visible",
        "description": "Brightened and equalized",
        "equalize": true,
        "equalize_per_channel": false,
        "white_balance": true,
        "brightness": 0.1,
        "contrast": 1.2,
        "hue_shift": 0.0,
        "saturation": 1.1,
        "lightness": 0.0,
        "median_blur": false,
        "invert": false
    }"#;
    let rec: AiRecommendation = serde_json::from_str(json).unwrap();
    assert_eq!(rec.name, "Enhanced Visible");
    assert!(rec.equalize);
    assert!((rec.contrast - 1.2).abs() < 0.01);
}

#[test]
fn test_parse_ai_recommendation_with_markdown_wrapper() {
    let json = "```json\n{\"name\":\"Test\",\"description\":\"\",\"equalize\":false,\"equalize_per_channel\":false,\"white_balance\":false,\"brightness\":0.0,\"contrast\":1.0,\"hue_shift\":0.0,\"saturation\":1.0,\"lightness\":0.0,\"median_blur\":false,\"invert\":false}\n```";
    // The parse function strips markdown wrapper
    let trimmed = json.trim().trim_start_matches("```json").trim_end_matches("```").trim();
    let rec: AiRecommendation = serde_json::from_str(trimmed).unwrap();
    assert_eq!(rec.name, "Test");
}

#[test]
fn test_apply_ai_recommendation() {
    let mut editor = EditorPanel::new();
    editor.ai_recommended = Some(AiRecommendation {
        name: "Test".into(),
        description: "".into(),
        equalize: true,
        equalize_per_channel: false,
        white_balance: true,
        brightness: 0.2,
        contrast: 1.5,
        hue_shift: 10.0,
        saturation: 0.8,
        lightness: -0.1,
        median_blur: false,
        invert: true,
    });
    editor.apply_ai_recommendation();
    assert!(editor.pipeline.equalize);
    assert!(editor.pipeline.white_balance);
    assert!((editor.pipeline.brightness - 0.2).abs() < 0.01);
    assert!((editor.pipeline.contrast - 1.5).abs() < 0.01);
    assert!(editor.pipeline.invert);
}
```

---

## Step 9: Build verification

```bash
cargo build --release -p ez-gui
cargo test --workspace
cargo clippy --workspace
```

---

## File Change Summary

| File | Lines Changed | Description |
|------|--------------|-------------|
| `ez-gui/src/ai_panel.rs` | +~120 | Add image_url to ChatMessage, vision API format in build_api_messages, send_vision_request(), base64_encode(), call_openai_vision(), call_anthropic_vision(), parse_data_uri(), config getters |
| `ez-gui/src/editor_panel.rs` | +~100 | AiRecommendation struct, AI_RECOMMEND_PROMPT, request_ai_recommend(), tick(), parse_ai_response(), apply_ai_recommendation(), AI UI section |
| `ez-gui/src/app.rs` | +~20 | Wire AI config to editor, call editor.tick(), auto-load channels + config on decode |

**Total: ~240 new lines across 3 files** (on top of Image Editor plan)

---

## Key Design Decisions

1. **Non-streaming vision request** — AI recommend is a single-shot request, not a chat stream. Uses `ureq::post()` synchronously in a background thread.
2. **Config passed by value** — Editor stores its own copy of AI config (endpoint, key, model, provider). Avoids borrow conflicts with AiPanel.
3. **Base64 in-memory** — Image is encoded to PNG → base64 in the calling thread before spawning the API thread. Keeps the API thread simple.
4. **Low detail** — Uses `"detail": "low"` for OpenAI vision to minimize token usage and cost.
5. **JSON-only response** — Prompt explicitly asks for JSON only (no markdown). Parser also handles markdown-wrapped responses as fallback.
6. **Single recommendation** — AI returns ONE best-guess preset, not multiple options. Keeps UI simple; user can click "AI Recommend" again for a different suggestion.
7. **No chat context** — Vision request is stateless (no conversation history). Each request is independent.
