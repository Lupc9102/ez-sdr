use serde::{Deserialize, Serialize};

/// A serializable RGBA color (0–255). Convertible to/from egui::Color32.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self { Self(r, g, b, 255) }
    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self { Self(r, g, b, a) }
    pub fn to_egui(&self) -> egui::Color32 { egui::Color32::from_rgba_unmultiplied(self.0, self.1, self.2, self.3) }
    pub fn with_alpha(&self, a: u8) -> Self { Self(self.0, self.1, self.2, a) }

    fn from_gray(v: u8) -> Self { Self(v, v, v, 255) }
}

// ─── Theme Config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub preset: String,
    pub accent: Rgba,
    pub bg: Rgba,
    pub surface: Rgba,
    pub text_normal: Rgba,
    pub text_heading: Rgba,
    pub text_dim: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub error: Rgba,
    pub spectrum_line: Rgba,
    pub spectrum_fill_top: Rgba,
    pub spectrum_fill_bot: Rgba,
    pub spectrum_grid: Rgba,
    pub noise_floor_line: Rgba,
    pub waterfall_bg: Rgba,
    pub bm_aviation: Rgba,
    pub bm_weather: Rgba,
    pub bm_marine: Rgba,
    pub bm_amateur: Rgba,
    pub bm_broadcast: Rgba,
    pub bm_scanner: Rgba,
    pub bm_default: Rgba,
    pub smeter_low: Rgba,
    pub smeter_mid: Rgba,
    pub smeter_high: Rgba,
    pub smeter_bg: Rgba,
    pub smeter_border: Rgba,
    pub vfo_a_color: Rgba,
    pub vfo_b_color: Rgba,
    pub status_signal: Rgba,
    pub status_recording: Rgba,
}

impl Default for ThemeConfig {
    fn default() -> Self { Self::dark() }
}

impl ThemeConfig {
    pub fn dark() -> Self {
        Self {
            preset: "dark".into(),
            accent: Rgba::from_rgb(52, 152, 219),
            bg: Rgba::from_rgb(20, 22, 28),
            surface: Rgba::from_rgb(28, 30, 38),
            text_normal: Rgba::from_rgb(220, 220, 230),
            text_heading: Rgba::from_rgb(255, 255, 255),
            text_dim: Rgba::from_rgb(130, 130, 140),
            success: Rgba::from_rgb(46, 204, 113),
            warning: Rgba::from_rgb(241, 196, 15),
            error: Rgba::from_rgb(231, 76, 60),
            spectrum_line: Rgba::from_rgb(52, 152, 219),
            spectrum_fill_top: Rgba::from_rgba(30, 120, 200, 100),
            spectrum_fill_bot: Rgba::from_rgba(10, 30, 60, 20),
            spectrum_grid: Rgba::from_rgba(60, 65, 80, 120),
            noise_floor_line: Rgba::from_rgba(100, 100, 200, 80),
            waterfall_bg: Rgba::from_rgb(0, 0, 5),
            bm_aviation: Rgba::from_rgba(100, 180, 255, 200),
            bm_weather: Rgba::from_rgba(80, 220, 80, 200),
            bm_marine: Rgba::from_rgba(0, 200, 200, 200),
            bm_amateur: Rgba::from_rgba(200, 100, 255, 200),
            bm_broadcast: Rgba::from_rgba(255, 140, 60, 200),
            bm_scanner: Rgba::from_rgba(255, 80, 80, 200),
            bm_default: Rgba::from_rgba(255, 215, 0, 200),
            smeter_low: Rgba::from_rgb(46, 204, 113),
            smeter_mid: Rgba::from_rgb(241, 196, 15),
            smeter_high: Rgba::from_rgb(231, 76, 60),
            smeter_bg: Rgba::from_rgb(30, 30, 40),
            smeter_border: Rgba::from_gray(80),
            vfo_a_color: Rgba::from_rgb(52, 200, 100),
            vfo_b_color: Rgba::from_rgb(100, 180, 255),
            status_signal: Rgba::from_rgb(46, 204, 113),
            status_recording: Rgba::from_rgb(231, 76, 60),
        }
    }

    pub fn light() -> Self {
        let mut t = Self::dark();
        t.preset = "light".into();
        t.bg = Rgba::from_rgb(245, 245, 245);
        t.surface = Rgba::from_rgb(255, 255, 255);
        t.text_normal = Rgba::from_rgb(40, 40, 50);
        t.text_heading = Rgba::from_rgb(10, 10, 20);
        t.text_dim = Rgba::from_rgb(130, 130, 140);
        t.accent = Rgba::from_rgb(41, 128, 185);
        t.spectrum_line = Rgba::from_rgb(41, 128, 185);
        t.spectrum_fill_top = Rgba::from_rgba(41, 128, 185, 80);
        t.spectrum_fill_bot = Rgba::from_rgba(41, 128, 185, 15);
        t.spectrum_grid = Rgba::from_rgba(160, 165, 180, 100);
        t.smeter_bg = Rgba::from_gray(220);
        t.smeter_border = Rgba::from_gray(180);
        t.waterfall_bg = Rgba::from_rgb(255, 255, 255);
        t
    }

    pub fn high_contrast() -> Self {
        let mut t = Self::dark();
        t.preset = "high_contrast".into();
        t.bg = Rgba::from_rgb(0, 0, 0);
        t.surface = Rgba::from_rgb(15, 15, 20);
        t.text_normal = Rgba::from_rgb(255, 255, 255);
        t.text_heading = Rgba::from_rgb(255, 255, 255);
        t.text_dim = Rgba::from_rgb(200, 200, 200);
        t.accent = Rgba::from_rgb(0, 200, 255);
        t.success = Rgba::from_rgb(0, 255, 100);
        t.warning = Rgba::from_rgb(255, 255, 0);
        t.error = Rgba::from_rgb(255, 50, 50);
        t.spectrum_line = Rgba::from_rgb(0, 240, 255);
        t.spectrum_fill_top = Rgba::from_rgba(0, 200, 255, 120);
        t.spectrum_fill_bot = Rgba::from_rgba(0, 100, 200, 30);
        t.bm_aviation = Rgba::from_rgb(0, 200, 255);
        t.bm_weather = Rgba::from_rgb(0, 255, 100);
        t.bm_marine = Rgba::from_rgb(0, 255, 255);
        t
    }

    pub fn solarized_dark() -> Self {
        let mut t = Self::dark();
        t.preset = "solarized_dark".into();
        t.bg = Rgba::from_rgb(0, 43, 54);
        t.surface = Rgba::from_rgb(7, 54, 66);
        t.text_normal = Rgba::from_rgb(131, 148, 150);
        t.text_heading = Rgba::from_rgb(238, 232, 213);
        t.text_dim = Rgba::from_rgb(88, 110, 117);
        t.accent = Rgba::from_rgb(38, 139, 210);
        t.success = Rgba::from_rgb(133, 153, 0);
        t.warning = Rgba::from_rgb(181, 137, 0);
        t.error = Rgba::from_rgb(220, 50, 47);
        t.spectrum_line = Rgba::from_rgb(38, 139, 210);
        t.spectrum_fill_top = Rgba::from_rgba(38, 139, 210, 90);
        t.spectrum_fill_bot = Rgba::from_rgba(38, 139, 210, 15);
        t.spectrum_grid = Rgba::from_rgba(88, 110, 117, 80);
        t.noise_floor_line = Rgba::from_rgba(181, 137, 0, 80);
        t
    }

    pub fn nord() -> Self {
        let mut t = Self::dark();
        t.preset = "nord".into();
        t.bg = Rgba::from_rgb(46, 52, 64);
        t.surface = Rgba::from_rgb(59, 66, 82);
        t.text_normal = Rgba::from_rgb(216, 222, 233);
        t.text_heading = Rgba::from_rgb(236, 239, 244);
        t.text_dim = Rgba::from_rgb(163, 170, 186);
        t.accent = Rgba::from_rgb(136, 192, 208);
        t.success = Rgba::from_rgb(163, 190, 140);
        t.warning = Rgba::from_rgb(235, 203, 139);
        t.error = Rgba::from_rgb(191, 97, 106);
        t.spectrum_line = Rgba::from_rgb(136, 192, 208);
        t.spectrum_fill_top = Rgba::from_rgba(136, 192, 208, 90);
        t.spectrum_fill_bot = Rgba::from_rgba(136, 192, 208, 15);
        t.spectrum_grid = Rgba::from_rgba(76, 86, 106, 120);
        t.bm_aviation = Rgba::from_rgb(136, 192, 208);
        t.bm_weather = Rgba::from_rgb(163, 190, 140);
        t.bm_marine = Rgba::from_rgb(143, 188, 187);
        t.bm_amateur = Rgba::from_rgb(180, 142, 173);
        t.bm_broadcast = Rgba::from_rgb(235, 203, 139);
        t.bm_scanner = Rgba::from_rgb(191, 97, 106);
        t.bm_default = Rgba::from_rgb(216, 222, 233);
        t
    }

    /// Apply this theme to the egui context.
    pub fn apply_to_ctx(&self, ctx: &egui::Context) {
        let is_dark = bg_luminance(&self.bg) < 0.5;
        let theme = egui::Theme::from_dark_mode(is_dark);
        let mut visuals = if is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        visuals.override_text_color = Some(self.text_normal.to_egui());
        visuals.hyperlink_color = self.accent.to_egui();
        visuals.selection.stroke = egui::Stroke::new(1.0, self.accent.to_egui());
        visuals.selection.bg_fill = self.accent.with_alpha(60).to_egui();

        visuals.window_fill = self.surface.to_egui();
        visuals.panel_fill = self.surface.to_egui();
        visuals.faint_bg_color = self.bg.to_egui();
        visuals.extreme_bg_color = self.bg.to_egui();
        visuals.code_bg_color = self.surface.to_egui();

        visuals.warn_fg_color = self.warning.to_egui();
        visuals.error_fg_color = self.error.to_egui();

        let w = &mut visuals.widgets;
        let cr = egui::CornerRadius::same(4);

        w.noninteractive.bg_fill = self.bg.to_egui();
        w.noninteractive.weak_bg_fill = self.surface.to_egui();
        w.noninteractive.fg_stroke = egui::Stroke::new(1.0, self.text_dim.to_egui());
        w.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.text_dim.with_alpha(80).to_egui());
        w.noninteractive.corner_radius = cr;

        w.inactive.bg_fill = self.surface.to_egui();
        w.inactive.weak_bg_fill = self.bg.to_egui();
        w.inactive.fg_stroke = egui::Stroke::new(1.0, self.text_normal.to_egui());
        w.inactive.bg_stroke = egui::Stroke::new(1.0, self.text_dim.with_alpha(100).to_egui());
        w.inactive.corner_radius = cr;
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = mix_color(&self.surface, &self.accent, 0.15).to_egui();
        w.hovered.weak_bg_fill = self.surface.to_egui();
        w.hovered.fg_stroke = egui::Stroke::new(1.5, self.text_normal.to_egui());
        w.hovered.bg_stroke = egui::Stroke::new(1.5, self.accent.to_egui());
        w.hovered.corner_radius = cr;
        w.hovered.expansion = 1.0;

        w.active.bg_fill = self.accent.to_egui();
        w.active.weak_bg_fill = self.surface.to_egui();
        w.active.fg_stroke = egui::Stroke::new(2.0, self.text_heading.to_egui());
        w.active.bg_stroke = egui::Stroke::new(2.0, self.accent.to_egui());
        w.active.corner_radius = cr;
        w.active.expansion = 0.0;

        w.open.bg_fill = self.accent.with_alpha(30).to_egui();
        w.open.weak_bg_fill = self.surface.to_egui();
        w.open.fg_stroke = egui::Stroke::new(1.0, self.accent.to_egui());
        w.open.bg_stroke = egui::Stroke::new(1.0, self.accent.to_egui());
        w.open.corner_radius = cr;

        visuals.window_corner_radius = egui::CornerRadius::same(6);
        visuals.menu_corner_radius = egui::CornerRadius::same(6);

        ctx.set_visuals(visuals);

        let mut style = (*ctx.style_of(theme)).clone();
        style.spacing.item_spacing = egui::vec2(4.0, 3.0);
        style.spacing.button_padding = egui::vec2(8.0, 2.0);
        style.spacing.indent = 16.0;
        style.spacing.slider_width = 120.0;
        style.animation_time = 0.05;
        ctx.set_style_of(theme, style);
    }

    // ─── UI: theme editor ──────────────────────────────────

    pub fn ui_editor(&mut self, ui: &mut egui::Ui, config_theme: &mut String) {
        ui.horizontal(|ui| {
            ui.label("Preset:");
            let presets: &[(&str, fn() -> ThemeConfig)] = &[
                ("dark", ThemeConfig::dark as fn() -> ThemeConfig),
                ("light", ThemeConfig::light),
                ("high_contrast", ThemeConfig::high_contrast),
                ("solarized_dark", ThemeConfig::solarized_dark),
                ("nord", ThemeConfig::nord),
            ];
            for (name, preset_fn) in presets {
                if ui.selectable_label(self.preset == *name, *name).clicked() {
                    *self = preset_fn();
                    *config_theme = name.to_string();
                }
            }
        });

        ui.add_space(4.0);

        egui::Grid::new("theme_colors")
            .num_columns(2)
            .striped(true)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                color_row(ui, "Accent", &mut self.accent);
                color_row(ui, "Background", &mut self.bg);
                color_row(ui, "Surface", &mut self.surface);
                color_row(ui, "Text Normal", &mut self.text_normal);
                color_row(ui, "Text Heading", &mut self.text_heading);
                color_row(ui, "Text Dim", &mut self.text_dim);
                color_row(ui, "Success", &mut self.success);
                color_row(ui, "Warning", &mut self.warning);
                color_row(ui, "Error", &mut self.error);
                color_row(ui, "Spectrum Line", &mut self.spectrum_line);
                color_row(ui, "Spectrum Fill Top", &mut self.spectrum_fill_top);
                color_row(ui, "Spectrum Fill Bot", &mut self.spectrum_fill_bot);
                color_row(ui, "Spectrum Grid", &mut self.spectrum_grid);
                color_row(ui, "Noise Floor", &mut self.noise_floor_line);
                color_row(ui, "Waterfall BG", &mut self.waterfall_bg);
                color_row(ui, "BM Aviation", &mut self.bm_aviation);
                color_row(ui, "BM Weather", &mut self.bm_weather);
                color_row(ui, "BM Marine", &mut self.bm_marine);
                color_row(ui, "BM Amateur", &mut self.bm_amateur);
                color_row(ui, "BM Broadcast", &mut self.bm_broadcast);
                color_row(ui, "BM Scanner", &mut self.bm_scanner);
                color_row(ui, "BM Default", &mut self.bm_default);
                color_row(ui, "S-Meter Low", &mut self.smeter_low);
                color_row(ui, "S-Meter Mid", &mut self.smeter_mid);
                color_row(ui, "S-Meter High", &mut self.smeter_high);
                color_row(ui, "S-Meter BG", &mut self.smeter_bg);
                color_row(ui, "S-Meter Border", &mut self.smeter_border);
                color_row(ui, "VFO A", &mut self.vfo_a_color);
                color_row(ui, "VFO B", &mut self.vfo_b_color);
                color_row(ui, "Status Signal", &mut self.status_signal);
                color_row(ui, "Status Recording", &mut self.status_recording);
            });
    }
}

// ─── UI helper ──────────────────────────────────────────────────────────────

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut Rgba) {
    ui.label(label);
    let mut egui_color: egui::Color32 = color.to_egui();
    if ui.color_edit_button_srgba(&mut egui_color).changed() {
        let [r, g, b, a] = egui_color.to_srgba_unmultiplied();
        *color = Rgba(r, g, b, a);
    }
    ui.end_row();
}

// ─── Cached theme colors for panels ────────────────────────────────────────

/// All theme colors pre-converted to egui::Color32 for direct use by panels.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ThemeColors {
    pub accent: egui::Color32,
    pub bg: egui::Color32,
    pub surface: egui::Color32,
    pub text_normal: egui::Color32,
    pub text_heading: egui::Color32,
    pub text_dim: egui::Color32,
    pub success: egui::Color32,
    pub warning: egui::Color32,
    pub error: egui::Color32,
    pub spectrum_line: egui::Color32,
    pub spectrum_fill_top: egui::Color32,
    pub spectrum_fill_bot: egui::Color32,
    pub spectrum_grid: egui::Color32,
    pub noise_floor_line: egui::Color32,
    pub waterfall_bg: egui::Color32,
    pub bm_aviation: egui::Color32,
    pub bm_weather: egui::Color32,
    pub bm_marine: egui::Color32,
    pub bm_amateur: egui::Color32,
    pub bm_broadcast: egui::Color32,
    pub bm_scanner: egui::Color32,
    pub bm_default: egui::Color32,
    pub smeter_low: egui::Color32,
    pub smeter_mid: egui::Color32,
    pub smeter_high: egui::Color32,
    pub smeter_bg: egui::Color32,
    pub smeter_border: egui::Color32,
    pub vfo_a: egui::Color32,
    pub vfo_b: egui::Color32,
    pub status_signal: egui::Color32,
    pub status_recording: egui::Color32,
}

impl From<&ThemeConfig> for ThemeColors {
    fn from(t: &ThemeConfig) -> Self {
        Self {
            accent: t.accent.to_egui(),
            bg: t.bg.to_egui(),
            surface: t.surface.to_egui(),
            text_normal: t.text_normal.to_egui(),
            text_heading: t.text_heading.to_egui(),
            text_dim: t.text_dim.to_egui(),
            success: t.success.to_egui(),
            warning: t.warning.to_egui(),
            error: t.error.to_egui(),
            spectrum_line: t.spectrum_line.to_egui(),
            spectrum_fill_top: t.spectrum_fill_top.to_egui(),
            spectrum_fill_bot: t.spectrum_fill_bot.to_egui(),
            spectrum_grid: t.spectrum_grid.to_egui(),
            noise_floor_line: t.noise_floor_line.to_egui(),
            waterfall_bg: t.waterfall_bg.to_egui(),
            bm_aviation: t.bm_aviation.to_egui(),
            bm_weather: t.bm_weather.to_egui(),
            bm_marine: t.bm_marine.to_egui(),
            bm_amateur: t.bm_amateur.to_egui(),
            bm_broadcast: t.bm_broadcast.to_egui(),
            bm_scanner: t.bm_scanner.to_egui(),
            bm_default: t.bm_default.to_egui(),
            smeter_low: t.smeter_low.to_egui(),
            smeter_mid: t.smeter_mid.to_egui(),
            smeter_high: t.smeter_high.to_egui(),
            smeter_bg: t.smeter_bg.to_egui(),
            smeter_border: t.smeter_border.to_egui(),
            vfo_a: t.vfo_a_color.to_egui(),
            vfo_b: t.vfo_b_color.to_egui(),
            status_signal: t.status_signal.to_egui(),
            status_recording: t.status_recording.to_egui(),
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn bg_luminance(c: &Rgba) -> f32 {
    0.299 * c.0 as f32 / 255.0 + 0.587 * c.1 as f32 / 255.0 + 0.114 * c.2 as f32 / 255.0
}

fn mix_color(a: &Rgba, b: &Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba(
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
        (a.3 as f32 + (b.3 as f32 - a.3 as f32) * t) as u8,
    )
}
