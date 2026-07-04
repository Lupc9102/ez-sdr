//! Satellite image editor panel — composites, processing filters, histogram,
//! zoom/pan viewer, AI-recommended edits, and PNG export.

use crate::composite::{self, CompositePreset, Expr, METEOR_COMPOSITES};
use crate::image_processing::{self, ProcessingPipeline};
use std::collections::HashMap;

// ── AI Recommendation ────────────────────────────────────────────────────────

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

// ── View mode ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Composite,
    SingleChannel,
}

// ── EditorPanel ──────────────────────────────────────────────────────────────

pub struct EditorPanel {
    /// Decoded channels: APID (64..69) -> grayscale pixels as f32 (0..1).
    channels: HashMap<u8, Vec<f32>>,
    channel_width: u32,
    channel_height: u32,

    /// Composite selection
    selected_preset: usize,
    custom_r: String,
    custom_g: String,
    custom_b: String,
    use_custom: bool,
    parsed_r: Option<Expr>,
    parsed_g: Option<Expr>,
    parsed_b: Option<Expr>,
    parse_error: Option<String>,

    /// Processing
    pipeline: ProcessingPipeline,

    /// Histogram
    histogram_r: [u32; 256],
    histogram_g: [u32; 256],
    histogram_b: [u32; 256],
    show_histogram: bool,

    /// Viewer
    zoom: f32,
    pan_x: f32,
    pan_y: f32,
    output_texture: Option<egui::TextureHandle>,
    output_rgba: Vec<u8>,

    /// Channel view
    view_mode: ViewMode,
    selected_channel: u8,

    /// Dirty flag: recompute composite when true
    dirty: bool,

    /// AI recommend state
    ai_recommended: Option<AiRecommendation>,
    ai_recommending: bool,
    ai_rx: Option<crossbeam_channel::Receiver<String>>,
    ai_error: Option<String>,

    /// AI config (passed from app)
    ai_endpoint: String,
    ai_api_key: String,
    ai_model: String,
    ai_provider: String,
}

impl EditorPanel {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
            channel_width: 0,
            channel_height: 0,
            selected_preset: 0,
            custom_r: "ch2".to_string(),
            custom_g: "ch2".to_string(),
            custom_b: "ch1".to_string(),
            use_custom: false,
            parsed_r: None,
            parsed_g: None,
            parsed_b: None,
            parse_error: None,
            pipeline: ProcessingPipeline::default(),
            histogram_r: [0; 256],
            histogram_g: [0; 256],
            histogram_b: [0; 256],
            show_histogram: true,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            output_texture: None,
            output_rgba: Vec::new(),
            view_mode: ViewMode::Composite,
            selected_channel: 64,
            dirty: true,
            ai_recommended: None,
            ai_recommending: false,
            ai_rx: None,
            ai_error: None,
            ai_endpoint: String::new(),
            ai_api_key: String::new(),
            ai_model: String::new(),
            ai_provider: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Set AI provider config for vision requests.
    pub fn set_ai_config(&mut self, endpoint: &str, api_key: &str, model: &str, provider: &str) {
        self.ai_endpoint = endpoint.to_string();
        self.ai_api_key = api_key.to_string();
        self.ai_model = model.to_string();
        self.ai_provider = provider.to_string();
    }

    /// Load decoded channels from decode result.
    pub fn load_channels(&mut self, channels: Vec<(u16, image::GrayImage)>) {
        self.channels.clear();
        for (apid, img) in &channels {
            if *apid >= 64 && *apid <= 69 {
                let pixels: Vec<f32> = img.pixels().map(|p| p.0[0] as f32 / 255.0).collect();
                self.channels.insert(*apid as u8, pixels);
                self.channel_width = img.width();
                self.channel_height = img.height();
            }
        }
        self.dirty = true;
    }

    /// Poll AI vision response. Call every frame.
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
        let json_str = response
            .trim()
            .trim_start_matches("```json")
            .trim_end_matches("```")
            .trim();

        match serde_json::from_str::<AiRecommendation>(json_str) {
            Ok(rec) => {
                self.ai_recommended = Some(rec);
                self.ai_error = None;
            }
            Err(e) => {
                self.ai_error = Some(format!("Failed to parse AI response: {e}"));
            }
        }
    }

    /// Apply the AI-recommended preset to the processing pipeline.
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

    /// Request an AI recommendation for the current image.
    pub fn request_ai_recommend(&mut self) {
        if self.output_rgba.is_empty() || self.channel_width == 0 || self.channel_height == 0 {
            self.ai_error = Some("No image to analyze".to_string());
            return;
        }
        if self.ai_api_key.is_empty() && self.ai_provider != "Ollama (local)" {
            self.ai_error = Some("No AI API key configured. Set one in Settings.".to_string());
            return;
        }

        self.ai_recommending = true;
        self.ai_error = None;

        // Encode current output as PNG
        let rgba_img = image::RgbaImage::from_raw(
            self.channel_width,
            self.channel_height,
            self.output_rgba.clone(),
        );
        let rgba_img = match rgba_img {
            Some(img) => img,
            None => {
                self.ai_error = Some("Failed to encode image".to_string());
                self.ai_recommending = false;
                return;
            }
        };

        let mut png_bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut png_bytes);
        if rgba_img.write_to(&mut cursor, image::ImageFormat::Png).is_err() {
            self.ai_error = Some("Failed to encode PNG".to_string());
            self.ai_recommending = false;
            return;
        }

        let b64 = base64_encode(&png_bytes);
        let data_uri = format!("data:image/png;base64,{b64}");

        let endpoint = self.ai_endpoint.clone();
        let api_key = self.ai_api_key.clone();
        let model = self.ai_model.clone();
        let provider = self.ai_provider.clone();
        let prompt = AI_RECOMMEND_PROMPT.to_string();

        let (tx, rx) = crossbeam_channel::bounded(1);
        self.ai_rx = Some(rx);

        std::thread::spawn(move || {
            let result = if provider == "Anthropic" {
                call_anthropic_vision(&endpoint, &api_key, &model, &data_uri, &prompt)
            } else {
                call_openai_vision(&endpoint, &api_key, &model, &data_uri, &prompt)
            };
            let _ = tx.send(result.unwrap_or_else(|e| format!("Error: {e}")));
        });
    }

    /// Recomposite: evaluate expressions for each pixel, apply pipeline, update texture.
    fn recomposite(&mut self) {
        if self.channels.is_empty() || self.channel_width == 0 || self.channel_height == 0 {
            return;
        }

        let w = self.channel_width as usize;
        let h = self.channel_height as usize;
        let n = w * h;

        // Get or compile expressions
        let (r_expr, g_expr, b_expr) = if self.use_custom {
            let r = composite::parse_expression(&self.custom_r);
            let g = composite::parse_expression(&self.custom_g);
            let b = composite::parse_expression(&self.custom_b);
            match (r, g, b) {
                (Ok(re), Ok(ge), Ok(be)) => {
                    self.parsed_r = Some(re.clone());
                    self.parsed_g = Some(ge.clone());
                    self.parsed_b = Some(be.clone());
                    self.parse_error = None;
                    (re, ge, be)
                }
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                    self.parse_error = Some(e);
                    return;
                }
            }
        } else {
            let preset = &METEOR_COMPOSITES[self.selected_preset];
            match (
                composite::parse_expression(preset.r_expr),
                composite::parse_expression(preset.g_expr),
                composite::parse_expression(preset.b_expr),
            ) {
                (Ok(re), Ok(ge), Ok(be)) => {
                    self.parse_error = None;
                    (re, ge, be)
                }
                _ => return,
            }
        };

        // Get channel data as f32 arrays for pixel evaluation
        let ch_data: Vec<Option<&Vec<f32>>> = (64..=69).map(|apid| self.channels.get(&apid)).collect();

        // Build per-pixel channel values
        let mut r_buf = vec![0.0f32; n];
        let mut g_buf = vec![0.0f32; n];
        let mut b_buf = vec![0.0f32; n];

        for idx in 0..n {
            let mut channels = [0.0f32; 6];
            for (i, ch) in ch_data.iter().enumerate() {
                if let Some(data) = ch {
                    if idx < data.len() {
                        channels[i] = data[idx];
                    }
                }
            }
            r_buf[idx] = r_expr.eval(&channels);
            g_buf[idx] = g_expr.eval(&channels);
            b_buf[idx] = b_expr.eval(&channels);
        }

        // Apply processing pipeline
        self.pipeline
            .apply(self.channel_width, self.channel_height, &mut r_buf, &mut g_buf, &mut b_buf);

        // Compute histogram
        self.histogram_r = image_processing::histogram(&r_buf);
        self.histogram_g = image_processing::histogram(&g_buf);
        self.histogram_b = image_processing::histogram(&b_buf);

        // Convert to RGBA u8
        self.output_rgba = Vec::with_capacity(n * 4);
        for i in 0..n {
            self.output_rgba.push((r_buf[i] * 255.0).round().clamp(0.0, 255.0) as u8);
            self.output_rgba.push((g_buf[i] * 255.0).round().clamp(0.0, 255.0) as u8);
            self.output_rgba.push((b_buf[i] * 255.0).round().clamp(0.0, 255.0) as u8);
            self.output_rgba.push(255); // alpha
        }

        // Upload texture
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [w, h],
            &self.output_rgba,
        );
        let tex_key = "editor_output";
        self.output_texture = Some(
            ui_ctx().load_texture(tex_key, color_image, egui::TextureOptions::default()),
        );

        self.dirty = false;
    }

    // ── UI renderers ────────────────────────────────────────────────────────

    /// Render the right-side controls panel.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if self.channels.is_empty() {
            ui.heading("Image Editor");
            ui.add_space(8.0);
            ui.label("No decoded image loaded.");
            ui.label("Decode a satellite recording first, then switch to this tab.");
            return;
        }

        ui.heading("Image Editor");
        ui.add_space(4.0);

        // Composite section
        self.ui_composite(ui);
        ui.separator();

        // Processing section
        self.ui_processing(ui);
        ui.separator();

        // Histogram
        self.ui_histogram_section(ui);
        ui.separator();

        // Channel thumbnails
        self.ui_channels(ui);
        ui.separator();

        // AI Recommend
        self.ui_ai_recommend(ui);
        ui.separator();

        // Export
        self.ui_export(ui);
    }

    fn ui_composite(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Composite").strong());

        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.use_custom, false, "Presets");
            ui.selectable_value(&mut self.use_custom, true, "Custom");
        });

        if !self.use_custom {
            egui::ComboBox::from_id_salt("composite_preset")
                .selected_text(METEOR_COMPOSITES[self.selected_preset].name)
                .show_ui(ui, |ui| {
                    for (i, preset) in METEOR_COMPOSITES.iter().enumerate() {
                        let label = format!("{} — {}", preset.name, preset.description);
                        if ui.selectable_value(&mut self.selected_preset, i, label).clicked() {
                            self.dirty = true;
                        }
                    }
                });
        } else {
            ui.label("R:");
            if ui.text_edit_singleline(&mut self.custom_r).changed() {
                self.dirty = true;
            }
            ui.label("G:");
            if ui.text_edit_singleline(&mut self.custom_g).changed() {
                self.dirty = true;
            }
            ui.label("B:");
            if ui.text_edit_singleline(&mut self.custom_b).changed() {
                self.dirty = true;
            }
            if let Some(ref err) = self.parse_error {
                ui.colored_label(egui::Color32::RED, err.as_str());
            }
        }
    }

    fn ui_processing(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Processing").strong());

        if ui.checkbox(&mut self.pipeline.equalize, "Equalize").changed() {
            self.dirty = true;
        }
        if self.pipeline.equalize {
            if ui.checkbox(&mut self.pipeline.equalize_per_channel, "Per Channel").changed() {
                self.dirty = true;
            }
        }
        if ui.checkbox(&mut self.pipeline.white_balance, "White Balance").changed() {
            self.dirty = true;
        }

        ui.add_space(4.0);

        let mut changed = false;
        ui.label("Brightness:");
        if ui.add(egui::Slider::new(&mut self.pipeline.brightness, -1.0..=1.0)).changed() {
            changed = true;
        }
        ui.label("Contrast:");
        if ui.add(egui::Slider::new(&mut self.pipeline.contrast, 0.0..=2.0)).changed() {
            changed = true;
        }
        ui.label("Hue:");
        if ui.add(egui::Slider::new(&mut self.pipeline.hue_shift, -180.0..=180.0).suffix("°")).changed() {
            changed = true;
        }
        ui.label("Saturation:");
        if ui.add(egui::Slider::new(&mut self.pipeline.saturation, 0.0..=2.0)).changed() {
            changed = true;
        }
        ui.label("Lightness:");
        if ui.add(egui::Slider::new(&mut self.pipeline.lightness, -1.0..=1.0)).changed() {
            changed = true;
        }

        if ui.checkbox(&mut self.pipeline.median_blur, "Median Blur").changed() {
            self.dirty = true;
        }
        if ui.checkbox(&mut self.pipeline.invert, "Invert").changed() {
            self.dirty = true;
        }

        if ui.button("↺ Reset All").clicked() {
            self.pipeline = ProcessingPipeline::default();
            self.dirty = true;
        }

        if changed {
            self.dirty = true;
        }
    }

    fn ui_histogram_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Histogram").strong());
            ui.toggle_value(&mut self.show_histogram, "Show");
        });

        if self.show_histogram && !self.output_rgba.is_empty() {
            let desired_size = egui::vec2(ui.available_width(), 80.0);
            let (response, painter) = ui.allocate_painter(desired_size, egui::Sense::hover());
            let rect = response.rect;

            let max_val = self
                .histogram_r
                .iter()
                .chain(self.histogram_g.iter())
                .chain(self.histogram_b.iter())
                .max()
                .copied()
                .unwrap_or(1) as f32;

            let bar_width = (rect.width() / 256.0).max(1.0);

            for i in 0..256 {
                let x = rect.left() + i as f32 * bar_width;

                let h_r = (self.histogram_r[i] as f32 / max_val) * rect.height();
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, rect.bottom() - h_r),
                        egui::vec2(bar_width, h_r),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(255, 50, 50, 90),
                );

                let h_g = (self.histogram_g[i] as f32 / max_val) * rect.height();
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, rect.bottom() - h_g),
                        egui::vec2(bar_width, h_g),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(50, 255, 50, 90),
                );

                let h_b = (self.histogram_b[i] as f32 / max_val) * rect.height();
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, rect.bottom() - h_b),
                        egui::vec2(bar_width, h_b),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(50, 50, 255, 90),
                );
            }

            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, egui::Color32::GRAY), egui::StrokeKind::Outside);
        }
    }

    fn ui_channels(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Channels").strong());

        let channel_names = [
            (64, "Ch1 VIS"),
            (65, "Ch2 NIR"),
            (66, "Ch3 SWIR"),
            (67, "Ch4 IR"),
            (68, "Ch5 WV"),
            (69, "Ch6 IR"),
        ];

        ui.horizontal_wrapped(|ui| {
            for (apid, label) in channel_names {
                if self.channels.contains_key(&apid) {
                    let is_selected = self.view_mode == ViewMode::SingleChannel
                        && self.selected_channel == apid;
                    if ui.selectable_label(is_selected, label).clicked() {
                        if is_selected {
                            self.view_mode = ViewMode::Composite;
                            self.dirty = true;
                        } else {
                            self.view_mode = ViewMode::SingleChannel;
                            self.selected_channel = apid;
                            self.dirty = true;
                        }
                    }
                }
            }
        });
    }

    fn ui_ai_recommend(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("AI Assistant").strong());

        ui.horizontal(|ui| {
            if ui
                .button("🤖 AI Recommend")
                .on_hover_text("Send current image to AI for processing recommendations")
                .clicked()
            {
                self.request_ai_recommend();
            }
            if self.ai_recommending {
                ui.spinner();
                ui.label("Analyzing...");
            }
        });

        if let Some(ref rec) = self.ai_recommended {
            let rec_name = rec.name.clone();
            let rec_desc = rec.description.clone();
            let mut apply = false;
            let mut dismiss = false;
            ui.group(|ui| {
                ui.label(egui::RichText::new(&rec_name).strong());
                ui.label(&rec_desc);
                ui.horizontal(|ui| {
                    if ui.button("✅ Apply Preset").clicked() {
                        apply = true;
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });
            if apply {
                self.apply_ai_recommendation();
            }
            if dismiss {
                self.ai_recommended = None;
            }
        }

        if let Some(ref err) = self.ai_error {
            ui.colored_label(egui::Color32::RED, err.as_str());
        }
    }

    fn ui_export(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("💾 Export PNG")
            .on_hover_text("Save the current processed image as a PNG file")
            .clicked()
        {
            self.export_png();
        }
    }

    fn export_png(&self) {
        if self.output_rgba.is_empty() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG Image", &["png"])
            .save_file()
        {
            if let Some(img) = image::RgbaImage::from_raw(
                self.channel_width,
                self.channel_height,
                self.output_rgba.clone(),
            ) {
                let _ = img.save(&path);
            }
        }
    }

    /// Render the image viewer (called from the central panel).
    pub fn ui_viewer(&mut self, ui: &mut egui::Ui) {
        if self.channels.is_empty() {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(8, 14, 24));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "🖼 Import a decoded satellite image to start editing",
                egui::FontId::proportional(14.0),
                egui::Color32::from_rgb(100, 110, 120),
            );
            return;
        }

        // Recomposite if dirty
        if self.dirty {
            if self.view_mode == ViewMode::SingleChannel {
                self.recomposite_single_channel();
            } else {
                self.recomposite();
            }
        }

        let response = ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::drag());
        let painter = ui.painter();
        painter.rect_filled(response.rect, 0.0, egui::Color32::from_rgb(8, 14, 24));

        // Zoom with scroll wheel
        let mut pending_zoom = None;
        ui.input(|i| {
            let scroll = i.smooth_scroll_delta.y;
            if scroll.abs() > 0.1 {
                pending_zoom = Some(if scroll > 0.0 { 1.1 } else { 0.9 });
            }
        });
        if let Some(factor) = pending_zoom {
            self.zoom *= factor;
            self.zoom = self.zoom.clamp(0.1, 10.0);
        }

        // Pan with left mouse drag
        if response.dragged() {
            self.pan_x += response.drag_delta().x;
            self.pan_y += response.drag_delta().y;
        }

        // Scroll to zoom
        ui.input(|i| {
            for event in &i.events {
                if let egui::Event::Zoom(factor) = event {
                    self.zoom = (self.zoom * factor).clamp(0.05, 50.0);
                }
            }
        });

        // Pixel info on hover
        if let Some(hover_pos) = response.hover_pos() {
            let img_x = ((hover_pos.x - response.rect.left() - self.pan_x) / self.zoom) as i32;
            let img_y = ((hover_pos.y - response.rect.top() - self.pan_y) / self.zoom) as i32;

            if img_x >= 0
                && img_y >= 0
                && (img_x as u32) < self.channel_width
                && (img_y as u32) < self.channel_height
            {
                let idx =
                    (img_y as u32 * self.channel_width + img_x as u32) as usize * 4;
                if idx + 3 < self.output_rgba.len() {
                    let r = self.output_rgba[idx];
                    let g = self.output_rgba[idx + 1];
                    let b = self.output_rgba[idx + 2];
                    painter.text(
                        hover_pos + egui::vec2(12.0, -12.0),
                        egui::Align2::LEFT_CENTER,
                        format!("({img_x}, {img_y})  R:{r} G:{g} B:{b}"),
                        egui::FontId::monospace(11.0),
                        egui::Color32::WHITE,
                    );
                }
            }
        }

        // Draw image with zoom/pan transform
        if let Some(ref tex) = self.output_texture {
            let img_rect = egui::Rect::from_min_size(
                response.rect.min + egui::vec2(self.pan_x, self.pan_y),
                egui::vec2(
                    self.channel_width as f32 * self.zoom,
                    self.channel_height as f32 * self.zoom,
                ),
            );
            painter.image(
                tex.id(),
                img_rect,
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Zoom indicator
        painter.text(
            response.rect.right_center() + egui::vec2(-80.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            format!("{:.1}x", self.zoom),
            egui::FontId::proportional(11.0),
            egui::Color32::from_rgb(120, 130, 140),
        );
    }

    fn recomposite_single_channel(&mut self) {
        if let Some(ch_data) = self.channels.get(&self.selected_channel) {
            let w = self.channel_width as usize;
            let h = self.channel_height as usize;
            let n = w * h;

            let mut r_buf = ch_data.clone();
            let mut g_buf = ch_data.clone();
            let mut b_buf = ch_data.clone();

            self.pipeline
                .apply(self.channel_width, self.channel_height, &mut r_buf, &mut g_buf, &mut b_buf);

            self.histogram_r = image_processing::histogram(&r_buf);
            self.histogram_g = image_processing::histogram(&g_buf);
            self.histogram_b = image_processing::histogram(&b_buf);

            self.output_rgba = Vec::with_capacity(n * 4);
            for i in 0..n {
                self.output_rgba.push((r_buf[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                self.output_rgba.push((g_buf[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                self.output_rgba.push((b_buf[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                self.output_rgba.push(255);
            }

            let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &self.output_rgba);
            let tex_key = "editor_output";
            self.output_texture = Some(
                ui_ctx().load_texture(tex_key, color_image, egui::TextureOptions::default()),
            );

            self.dirty = false;
        }
    }
}

impl Default for EditorPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn ui_ctx() -> egui::Context {
    egui::Context::default()
}

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
    result
}

fn call_openai_vision(
    endpoint: &str,
    api_key: &str,
    model: &str,
    image_data_uri: &str,
    prompt: &str,
) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "temperature": 0.3,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": image_data_uri, "detail": "low" } }
            ]
        }]
    });

    let mut resp = ureq::post(endpoint)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("HTTP error: {e}"))?;

    let resp_text = resp.body_mut().read_to_string().map_err(|e| format!("Read error: {e}"))?;
    let json: serde_json::Value = serde_json::from_str(&resp_text).map_err(|e| format!("JSON error: {e}"))?;
    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    Ok(text.to_string())
}

fn call_anthropic_vision(
    endpoint: &str,
    api_key: &str,
    model: &str,
    image_data_uri: &str,
    prompt: &str,
) -> Result<String, String> {
    let parts: Vec<&str> = image_data_uri.splitn(2, ',').collect();
    if parts.len() < 2 {
        return Err("Invalid data URI".to_string());
    }
    let media_type = parts[0]
        .trim_start_matches("data:")
        .trim_end_matches(";base64")
        .to_string();
    let b64_data = parts[1].to_string();

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "temperature": 0.3,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image", "source": { "type": "base64", "media_type": media_type, "data": b64_data } },
                { "type": "text", "text": prompt }
            ]
        }]
    });

    let mut resp = ureq::post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("HTTP error: {e}"))?;

    let resp_text = resp.body_mut().read_to_string().map_err(|e| format!("Read error: {e}"))?;
    let json: serde_json::Value = serde_json::from_str(&resp_text).map_err(|e| format!("JSON error: {e}"))?;
    let text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_editor_empty() {
        let editor = EditorPanel::new();
        assert!(editor.is_empty());
        assert_eq!(editor.zoom, 1.0);
    }

    #[test]
    fn test_load_channels() {
        let mut editor = EditorPanel::new();
        let img = image::GrayImage::from_raw(4, 2, vec![128; 8]).unwrap();
        let channels = vec![(64u16, img)];
        editor.load_channels(channels);
        assert!(!editor.is_empty());
        assert_eq!(editor.channel_width, 4);
        assert_eq!(editor.channel_height, 2);
        assert!(editor.channels.contains_key(&64));
    }

    #[test]
    fn test_load_channels_skips_invalid_apids() {
        let mut editor = EditorPanel::new();
        let img = image::GrayImage::from_raw(4, 2, vec![128; 8]).unwrap();
        let img2 = img.clone();
        let channels = vec![(70u16, img), (64u16, img2)];
        editor.load_channels(channels);
        assert_eq!(editor.channels.len(), 1);
        assert!(editor.channels.contains_key(&64));
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
        assert!(!editor.pipeline.equalize_per_channel);
        assert!(editor.pipeline.white_balance);
        assert!((editor.pipeline.brightness - 0.2).abs() < 0.01);
        assert!((editor.pipeline.contrast - 1.5).abs() < 0.01);
        assert!((editor.pipeline.hue_shift - 10.0).abs() < 0.01);
        assert!((editor.pipeline.saturation - 0.8).abs() < 0.01);
        assert!((editor.pipeline.lightness - (-0.1)).abs() < 0.01);
        assert!(editor.pipeline.invert);
    }

    #[test]
    fn test_parse_ai_response_valid() {
        let mut editor = EditorPanel::new();
        let json = r#"{
            "name": "Enhanced",
            "description": "test",
            "equalize": true,
            "equalize_per_channel": false,
            "white_balance": false,
            "brightness": 0.0,
            "contrast": 1.0,
            "hue_shift": 0.0,
            "saturation": 1.0,
            "lightness": 0.0,
            "median_blur": false,
            "invert": false
        }"#;
        editor.parse_ai_response(json);
        assert!(editor.ai_recommended.is_some());
        assert!(editor.ai_error.is_none());
    }

    #[test]
    fn test_parse_ai_response_with_markdown() {
        let mut editor = EditorPanel::new();
        let json = "```json\n{\"name\":\"Test\",\"description\":\"\",\"equalize\":false,\"equalize_per_channel\":false,\"white_balance\":false,\"brightness\":0.0,\"contrast\":1.0,\"hue_shift\":0.0,\"saturation\":1.0,\"lightness\":0.0,\"median_blur\":false,\"invert\":false}\n```";
        editor.parse_ai_response(json);
        assert!(editor.ai_recommended.is_some());
    }

    #[test]
    fn test_parse_ai_response_invalid() {
        let mut editor = EditorPanel::new();
        editor.parse_ai_response("not json at all");
        assert!(editor.ai_recommended.is_none());
        assert!(editor.ai_error.is_some());
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"A"), "QQ==");
        assert_eq!(base64_encode(b"AB"), "QUI=");
        assert_eq!(base64_encode(b"ABC"), "QUJD");
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
    }

    #[test]
    fn test_set_ai_config() {
        let mut editor = EditorPanel::new();
        editor.set_ai_config("http://api.test.com", "sk-test", "gpt-4o", "OpenAI");
        assert_eq!(editor.ai_endpoint, "http://api.test.com");
        assert_eq!(editor.ai_api_key, "sk-test");
        assert_eq!(editor.ai_model, "gpt-4o");
        assert_eq!(editor.ai_provider, "OpenAI");
    }
}
