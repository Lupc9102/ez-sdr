use serde::{Deserialize, Serialize};

/// A serializable RGBA color (0–255). Convertible to/from `egui::Color32`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 255)
    }
    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(r, g, b, a)
    }
    pub fn to_egui(self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(self.0, self.1, self.2, self.3)
    }
    pub fn with_alpha(&self, a: u8) -> Self {
        Self(self.0, self.1, self.2, a)
    }

    fn from_gray(v: u8) -> Self {
        Self(v, v, v, 255)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_from_rgb_sets_alpha_255() {
        let c = Rgba::from_rgb(10, 20, 30);
        assert_eq!(c, Rgba(10, 20, 30, 255));
    }

    #[test]
    fn rgba_from_rgba_preserves_alpha() {
        let c = Rgba::from_rgba(100, 150, 200, 80);
        assert_eq!(c, Rgba(100, 150, 200, 80));
    }

    #[test]
    fn rgba_to_egui_maps_correctly() {
        let c = Rgba(64, 128, 192, 255);
        let egui_c = c.to_egui();
        assert_eq!(egui_c.r(), 64);
        assert_eq!(egui_c.g(), 128);
        assert_eq!(egui_c.b(), 192);
        assert_eq!(egui_c.a(), 255);
    }

    #[test]
    fn rgba_with_alpha_changes_only_alpha() {
        let c = Rgba(10, 20, 30, 255);
        let c2 = c.with_alpha(128);
        assert_eq!(c2, Rgba(10, 20, 30, 128));
    }

    #[test]
    fn rgba_partial_eq() {
        assert_eq!(Rgba(1, 2, 3, 4), Rgba(1, 2, 3, 4));
        assert_ne!(Rgba(1, 2, 3, 4), Rgba(5, 2, 3, 4));
    }

    #[test]
    fn theme_dark_preset_has_correct_name() {
        let t = ThemeConfig::dark();
        assert_eq!(t.preset, "dark");
        assert_eq!(t.accent, Rgba::from_rgb(52, 152, 219));
        assert_eq!(t.bg, Rgba::from_rgb(20, 22, 28));
    }

    #[test]
    fn theme_light_preset_overrides_dark() {
        let t = ThemeConfig::light();
        assert_eq!(t.preset, "light");
        assert_eq!(t.bg, Rgba::from_rgb(245, 245, 245));
        assert_eq!(t.surface, Rgba::from_rgb(255, 255, 255));
        assert_eq!(t.text_normal, Rgba::from_rgb(40, 40, 50));
    }

    #[test]
    fn theme_high_contrast_preset() {
        let t = ThemeConfig::high_contrast();
        assert_eq!(t.preset, "high_contrast");
        assert_eq!(t.bg, Rgba::from_rgb(0, 0, 0));
        assert_eq!(t.text_normal, Rgba::from_rgb(255, 255, 255));
        assert_eq!(t.accent, Rgba::from_rgb(0, 200, 255));
    }

    #[test]
    fn theme_solarized_dark_preset() {
        let t = ThemeConfig::solarized_dark();
        assert_eq!(t.preset, "solarized_dark");
        assert_eq!(t.bg, Rgba::from_rgb(0, 43, 54));
        assert_eq!(t.accent, Rgba::from_rgb(38, 139, 210));
    }

    #[test]
    fn theme_nord_preset() {
        let t = ThemeConfig::nord();
        assert_eq!(t.preset, "nord");
        assert_eq!(t.bg, Rgba::from_rgb(46, 52, 64));
        assert_eq!(t.surface, Rgba::from_rgb(59, 66, 82));
        assert_eq!(t.accent, Rgba::from_rgb(136, 192, 208));
    }

    #[test]
    fn theme_default_is_dark() {
        let t = ThemeConfig::default();
        assert_eq!(t.preset, "dark");
        assert_eq!(t, ThemeConfig::dark());
    }

    #[test]
    fn theme_bg_luminance_dark_below_threshold() {
        let dark_bg = Rgba::from_rgb(20, 22, 28);
        let luma = bg_luminance(&dark_bg);
        assert!(luma < 0.5);
    }

    #[test]
    fn theme_bg_luminance_light_above_threshold() {
        let light_bg = Rgba::from_rgb(245, 245, 245);
        let luma = bg_luminance(&light_bg);
        assert!(luma > 0.5);
    }

    #[test]
    fn theme_mix_color_clamps_t() {
        let a = Rgba::from_rgb(0, 0, 0);
        let b = Rgba::from_rgb(100, 100, 100);
        let mixed = mix_color(&a, &b, 0.5);
        assert_eq!(mixed, Rgba(50, 50, 50, 255));
        let clamped = mix_color(&a, &b, 1.5);
        assert_eq!(clamped, b);
    }

    #[test]
    fn theme_each_preset_has_unique_name() {
        let names = vec![
            ThemeConfig::dark().preset,
            ThemeConfig::light().preset,
            ThemeConfig::high_contrast().preset,
            ThemeConfig::solarized_dark().preset,
            ThemeConfig::nord().preset,
        ];
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "All preset names must be unique");
    }

    #[test]
    fn test_theme_apply_to_ctx_no_crash() {
        for theme in &[
            ThemeConfig::dark(),
            ThemeConfig::light(),
            ThemeConfig::high_contrast(),
            ThemeConfig::solarized_dark(),
            ThemeConfig::nord(),
        ] {
            let ctx = egui::Context::default();
            theme.apply_to_ctx(&ctx);
        }
    }

    #[test]
    fn test_theme_serialize_deserialize() {
        for theme in &[
            ThemeConfig::dark(),
            ThemeConfig::light(),
            ThemeConfig::nord(),
        ] {
            let json = serde_json::to_string(theme).expect("ThemeConfig should serialize to JSON");
            let back: ThemeConfig = serde_json::from_str(&json)
                .expect("ThemeConfig JSON should deserialize round-trip");
            assert_eq!(*theme, back);
        }
    }

    #[test]
    fn test_rgba_serde_roundtrip() {
        let c = Rgba(100, 150, 200, 255);
        let json = serde_json::to_string(&c).expect("Rgba should serialize to JSON");
        let back: Rgba =
            serde_json::from_str(&json).expect("Rgba JSON should deserialize round-trip");
        assert_eq!(c, back);

        let zero = Rgba(0, 0, 0, 0);
        let json = serde_json::to_string(&zero).expect("zero Rgba should serialize to JSON");
        let back: Rgba =
            serde_json::from_str(&json).expect("zero Rgba JSON should deserialize round-trip");
        assert_eq!(zero, back);
    }

    #[test]
    fn test_bg_luminance_extremes() {
        assert_eq!(bg_luminance(&Rgba::from_rgb(0, 0, 0)), 0.0);
        assert_eq!(bg_luminance(&Rgba::from_rgb(255, 255, 255)), 1.0);
        let mid = bg_luminance(&Rgba::from_rgb(128, 128, 128));
        assert!((mid - 0.5).abs() < 0.01, "expected ~0.5, got {mid}");
    }

    #[test]
    fn test_mix_color_boundaries() {
        let a = Rgba(10, 20, 30, 255);
        let b = Rgba(100, 200, 50, 128);
        assert_eq!(mix_color(&a, &b, 0.0), a);
        assert_eq!(mix_color(&a, &b, 1.0), b);
        assert_eq!(mix_color(&a, &b, -0.5), a);
        assert_eq!(mix_color(&a, &b, 1.5), b);
    }

    #[test]
    fn test_apply_to_ctx_all_presets() {
        let presets = [
            ThemeConfig::dark(),
            ThemeConfig::light(),
            ThemeConfig::high_contrast(),
            ThemeConfig::solarized_dark(),
            ThemeConfig::nord(),
        ];
        for theme in &presets {
            let ctx = egui::Context::default();
            theme.apply_to_ctx(&ctx);
            let is_dark = bg_luminance(&theme.bg) < 0.5;
            let egui_theme = egui::Theme::from_dark_mode(is_dark);
            let style = ctx.style_of(egui_theme);
            assert_ne!(
                style.visuals.window_fill,
                egui::Color32::default(),
                "theme {} did not modify visuals",
                theme.preset,
            );
        }
    }

    #[test]
    fn gradient_sample_interpolates_between_stops() {
        let g = Gradient::two(Rgba::from_rgb(0, 0, 0), Rgba::from_rgb(200, 100, 50));
        assert_eq!(g.sample(0.0), Rgba::from_rgb(0, 0, 0));
        assert_eq!(g.sample(1.0), Rgba::from_rgb(200, 100, 50));
        let mid = g.sample(0.5);
        assert_eq!(mid, Rgba::from_rgb(100, 50, 25));
    }

    #[test]
    fn gradient_sample_clamps_out_of_range_t() {
        let g = Gradient::two(Rgba::from_rgb(10, 10, 10), Rgba::from_rgb(20, 20, 20));
        assert_eq!(g.sample(-1.0), g.sample(0.0));
        assert_eq!(g.sample(2.0), g.sample(1.0));
    }

    #[test]
    fn gradient_sample_empty_stops_returns_black() {
        let g = Gradient { stops: vec![] };
        assert_eq!(g.sample(0.5), Rgba::from_rgb(0, 0, 0));
    }

    #[test]
    fn gradient_default_matches_two_stop_shape() {
        let g = Gradient::default();
        assert_eq!(g.stops.len(), 2);
    }

    #[test]
    fn glow_config_default_is_disabled() {
        let glow = GlowConfig::default();
        assert!(!glow.enabled);
    }

    #[test]
    fn all_presets_have_unique_names_and_apply_cleanly() {
        for (name, make) in ThemeConfig::all_presets() {
            let theme = make();
            let ctx = egui::Context::default();
            theme.apply_to_ctx(&ctx);
            assert_eq!(
                &theme.preset, name,
                "preset field should match registry name"
            );
        }
    }

    #[test]
    fn named_theme_roundtrips_through_json() {
        let named = NamedTheme {
            id: "abc123".to_string(),
            name: "My Theme".to_string(),
            theme: ThemeConfig::dark(),
        };
        let json = serde_json::to_string(&named).expect("NamedTheme should serialize");
        let back: NamedTheme =
            serde_json::from_str(&json).expect("NamedTheme JSON should deserialize round-trip");
        assert_eq!(named.id, back.id);
        assert_eq!(named.name, back.name);
    }
}

type PresetFn = fn() -> ThemeConfig;

// ─── Gradients ─────────────────────────────────────────────────────────────

/// A multi-stop linear gradient, sampled top→bottom. `stops` are
/// `(position 0..1, color)` pairs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub stops: Vec<(f32, Rgba)>,
}

impl Gradient {
    pub fn two(top: Rgba, bottom: Rgba) -> Self {
        Self {
            stops: vec![(0.0, top), (1.0, bottom)],
        }
    }

    /// Sample the interpolated color at position `t` (clamped to `[0, 1]`).
    pub fn sample(&self, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        if self.stops.is_empty() {
            return Rgba::from_rgb(0, 0, 0);
        }
        let mut sorted = self.stops.clone();
        sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if sorted.len() == 1 {
            return sorted[0].1;
        }
        if t <= sorted[0].0 {
            return sorted[0].1;
        }
        for w in sorted.windows(2) {
            let (p0, c0) = w[0];
            let (p1, c1) = w[1];
            if t >= p0 && t <= p1 {
                let span = (p1 - p0).max(1e-6);
                return mix_color(&c0, &c1, (t - p0) / span);
            }
        }
        sorted.last().expect("checked non-empty above").1
    }
}

impl Default for Gradient {
    fn default() -> Self {
        Self::two(
            Rgba::from_rgba(30, 120, 200, 100),
            Rgba::from_rgba(10, 30, 60, 20),
        )
    }
}

// ─── Glow ──────────────────────────────────────────────────────────────────

/// Soft glow effect applied to active/highlighted elements (nav tab, status
/// indicators). Rendered as concentric fading strokes since egui has no
/// native blur.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GlowConfig {
    pub enabled: bool,
    pub color: Rgba,
    pub radius: f32,
    pub intensity: f32,
}

impl Default for GlowConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            color: Rgba::from_rgb(0, 168, 255),
            radius: 8.0,
            intensity: 0.5,
        }
    }
}

// ─── Corner roundness ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CornerStyle {
    pub buttons: f32,
    pub panels: f32,
    pub windows: f32,
}

impl Default for CornerStyle {
    fn default() -> Self {
        Self {
            buttons: 4.0,
            panels: 6.0,
            windows: 6.0,
        }
    }
}

// ─── Typography ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FontFamily {
    /// egui's built-in proportional + monospace fonts.
    #[default]
    EguiDefault,
    /// Bundled DejaVu Sans / DejaVu Sans Mono.
    DejaVu,
}

impl FontFamily {
    pub const ALL: [FontFamily; 2] = [FontFamily::EguiDefault, FontFamily::DejaVu];

    pub fn label(self) -> &'static str {
        match self {
            FontFamily::EguiDefault => "Default",
            FontFamily::DejaVu => "DejaVu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TypographyConfig {
    pub family: FontFamily,
    pub heading_size: f32,
    pub body_size: f32,
    pub small_size: f32,
    pub monospace_size: f32,
    pub button_size: f32,
}

impl Default for TypographyConfig {
    fn default() -> Self {
        Self {
            family: FontFamily::EguiDefault,
            heading_size: 18.0,
            body_size: 13.0,
            small_size: 10.0,
            monospace_size: 13.0,
            button_size: 13.0,
        }
    }
}

// ─── Spacing / density ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Spacious,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Comfortable, Density::Spacious];

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Comfortable => "Comfortable",
            Density::Spacious => "Spacious",
        }
    }

    pub fn item_spacing(self) -> egui::Vec2 {
        match self {
            Density::Compact => egui::vec2(3.0, 2.0),
            Density::Comfortable => egui::vec2(4.0, 3.0),
            Density::Spacious => egui::vec2(8.0, 6.0),
        }
    }

    pub fn button_padding(self) -> egui::Vec2 {
        match self {
            Density::Compact => egui::vec2(6.0, 1.0),
            Density::Comfortable => egui::vec2(8.0, 2.0),
            Density::Spacious => egui::vec2(12.0, 5.0),
        }
    }

    pub fn indent(self) -> f32 {
        match self {
            Density::Compact => 12.0,
            Density::Comfortable => 16.0,
            Density::Spacious => 22.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpacingConfig {
    pub density: Density,
}

impl Default for SpacingConfig {
    fn default() -> Self {
        Self {
            density: Density::Comfortable,
        }
    }
}

// ─── Named theme gallery ────────────────────────────────────────────────────

/// A user-saved theme in the customization gallery, distinct from the
/// built-in presets returned by [`ThemeConfig::all_presets`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedTheme {
    pub id: String,
    pub name: String,
    pub theme: ThemeConfig,
}

// ─── Theme Config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub bandplan_ham: Rgba,
    pub bandplan_broadcast: Rgba,
    pub bandplan_aviation: Rgba,
    pub bandplan_marine: Rgba,
    pub bandplan_weather: Rgba,
    pub bandplan_satellite: Rgba,
    pub bandplan_mobile: Rgba,
    pub bandplan_ism: Rgba,
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
    /// Corner roundness for buttons/panels/windows.
    #[serde(default)]
    pub corner: CornerStyle,
    /// Glow effect applied to active nav tab + status indicators.
    #[serde(default)]
    pub glow: GlowConfig,
    /// Gradient used for the spectrum fill-under-curve.
    #[serde(default)]
    pub spectrum_gradient: Gradient,
    /// Font family + text sizes.
    #[serde(default)]
    pub typography: TypographyConfig,
    /// Widget spacing density.
    #[serde(default)]
    pub spacing: SpacingConfig,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self::dark()
    }
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
            bandplan_ham: Rgba::from_rgba(80, 200, 80, 28),
            bandplan_broadcast: Rgba::from_rgba(255, 140, 50, 28),
            bandplan_aviation: Rgba::from_rgba(80, 160, 255, 28),
            bandplan_marine: Rgba::from_rgba(0, 200, 180, 28),
            bandplan_weather: Rgba::from_rgba(100, 255, 120, 28),
            bandplan_satellite: Rgba::from_rgba(180, 100, 255, 28),
            bandplan_mobile: Rgba::from_rgba(200, 180, 60, 22),
            bandplan_ism: Rgba::from_rgba(255, 80, 80, 25),
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
            corner: CornerStyle::default(),
            glow: GlowConfig::default(),
            spectrum_gradient: Gradient::two(
                Rgba::from_rgba(30, 120, 200, 100),
                Rgba::from_rgba(10, 30, 60, 20),
            ),
            typography: TypographyConfig::default(),
            spacing: SpacingConfig::default(),
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

    /// Neon-on-black preset with glow enabled and sharp corners.
    pub fn cyberpunk() -> Self {
        let mut t = Self::dark();
        t.preset = "cyberpunk".into();
        t.bg = Rgba::from_rgb(8, 4, 18);
        t.surface = Rgba::from_rgb(18, 8, 32);
        t.text_normal = Rgba::from_rgb(220, 240, 255);
        t.text_heading = Rgba::from_rgb(255, 40, 200);
        t.text_dim = Rgba::from_rgb(140, 100, 180);
        t.accent = Rgba::from_rgb(255, 40, 200);
        t.success = Rgba::from_rgb(60, 255, 180);
        t.warning = Rgba::from_rgb(255, 220, 40);
        t.error = Rgba::from_rgb(255, 60, 90);
        t.spectrum_line = Rgba::from_rgb(0, 240, 255);
        t.spectrum_fill_top = Rgba::from_rgba(255, 40, 200, 100);
        t.spectrum_fill_bot = Rgba::from_rgba(0, 240, 255, 10);
        t.spectrum_grid = Rgba::from_rgba(120, 40, 200, 90);
        t.waterfall_bg = Rgba::from_rgb(4, 0, 10);
        t.corner = CornerStyle {
            buttons: 0.0,
            panels: 2.0,
            windows: 2.0,
        };
        t.glow = GlowConfig {
            enabled: true,
            color: Rgba::from_rgb(255, 40, 200),
            radius: 10.0,
            intensity: 0.8,
        };
        t.spectrum_gradient = Gradient::two(t.spectrum_fill_top, t.spectrum_fill_bot);
        t
    }

    /// Pure-black AMOLED preset — minimizes lit pixels, subtle cyan glow.
    pub fn amoled() -> Self {
        let mut t = Self::dark();
        t.preset = "amoled".into();
        t.bg = Rgba::from_rgb(0, 0, 0);
        t.surface = Rgba::from_rgb(6, 6, 8);
        t.text_normal = Rgba::from_rgb(200, 200, 205);
        t.text_dim = Rgba::from_rgb(90, 90, 95);
        t.accent = Rgba::from_rgb(0, 200, 200);
        t.spectrum_line = Rgba::from_rgb(0, 220, 220);
        t.spectrum_fill_top = Rgba::from_rgba(0, 200, 200, 70);
        t.spectrum_fill_bot = Rgba::from_rgba(0, 0, 0, 0);
        t.waterfall_bg = Rgba::from_rgb(0, 0, 0);
        t.corner = CornerStyle {
            buttons: 8.0,
            panels: 10.0,
            windows: 10.0,
        };
        t.glow = GlowConfig {
            enabled: true,
            color: Rgba::from_rgb(0, 200, 200),
            radius: 6.0,
            intensity: 0.4,
        };
        t.spectrum_gradient = Gradient::two(t.spectrum_fill_top, t.spectrum_fill_bot);
        t
    }

    /// Warm gradient-heavy preset inspired by sunset skies.
    pub fn sunset() -> Self {
        let mut t = Self::dark();
        t.preset = "sunset".into();
        t.bg = Rgba::from_rgb(30, 15, 25);
        t.surface = Rgba::from_rgb(45, 22, 35);
        t.text_normal = Rgba::from_rgb(250, 230, 220);
        t.text_heading = Rgba::from_rgb(255, 200, 140);
        t.text_dim = Rgba::from_rgb(180, 130, 130);
        t.accent = Rgba::from_rgb(255, 130, 80);
        t.success = Rgba::from_rgb(150, 220, 130);
        t.warning = Rgba::from_rgb(255, 200, 60);
        t.error = Rgba::from_rgb(230, 70, 90);
        t.spectrum_line = Rgba::from_rgb(255, 150, 90);
        t.spectrum_fill_top = Rgba::from_rgba(255, 100, 60, 130);
        t.spectrum_fill_bot = Rgba::from_rgba(120, 20, 90, 20);
        t.spectrum_grid = Rgba::from_rgba(180, 100, 100, 90);
        t.corner = CornerStyle {
            buttons: 10.0,
            panels: 12.0,
            windows: 12.0,
        };
        t.spectrum_gradient = Gradient {
            stops: vec![
                (0.0, Rgba::from_rgba(255, 220, 100, 160)),
                (0.5, Rgba::from_rgba(255, 100, 60, 100)),
                (1.0, Rgba::from_rgba(90, 20, 90, 10)),
            ],
        };
        t
    }

    /// All built-in presets, in display order.
    pub fn all_presets() -> &'static [(&'static str, PresetFn)] {
        &[
            ("dark", ThemeConfig::dark as PresetFn),
            ("light", ThemeConfig::light),
            ("high_contrast", ThemeConfig::high_contrast),
            ("solarized_dark", ThemeConfig::solarized_dark),
            ("nord", ThemeConfig::nord),
            ("cyberpunk", ThemeConfig::cyberpunk),
            ("amoled", ThemeConfig::amoled),
            ("sunset", ThemeConfig::sunset),
        ]
    }

    /// Apply this theme to the egui context.
    pub fn apply_to_ctx(&self, ctx: &egui::Context) {
        crate::fonts::install(ctx, self.typography.family);

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
        let cr = egui::CornerRadius::same(self.corner.buttons as u8);

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

        let panel_cr = egui::CornerRadius::same(self.corner.panels.clamp(0.0, 255.0) as u8);
        let window_cr = egui::CornerRadius::same(self.corner.windows.clamp(0.0, 255.0) as u8);
        visuals.window_corner_radius = window_cr;
        visuals.menu_corner_radius = panel_cr;

        ctx.set_visuals(visuals);

        let mut style = (*ctx.style_of(theme)).clone();
        style.spacing.item_spacing = self.spacing.density.item_spacing();
        style.spacing.button_padding = self.spacing.density.button_padding();
        style.spacing.indent = self.spacing.density.indent();
        style.spacing.slider_width = 120.0;
        style.animation_time = 0.05;

        use egui::{FontFamily as EguiFamily, FontId, TextStyle};
        let prop = EguiFamily::Proportional;
        let mono = EguiFamily::Monospace;
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(self.typography.heading_size, prop.clone()),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(self.typography.body_size, prop.clone()),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(self.typography.button_size, prop.clone()),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(self.typography.small_size, prop),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(self.typography.monospace_size, mono),
        );

        ctx.set_style_of(theme, style);
    }

    // ─── UI: theme editor ──────────────────────────────────

    pub fn ui_editor(&mut self, ui: &mut egui::Ui, config_theme: &mut String) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Preset:");
            for (name, preset_fn) in Self::all_presets() {
                if ui.selectable_label(self.preset == *name, *name).clicked() {
                    *self = preset_fn();
                    *config_theme = (*name).to_string();
                }
            }
        });

        ui.add_space(4.0);

        #[allow(clippy::type_complexity)]
        let groups: &[(&str, &[(&str, fn(&mut Self) -> &mut Rgba)])] = &[
            (
                "Base & Surfaces",
                &[
                    ("Accent", |t| &mut t.accent),
                    ("Background", |t| &mut t.bg),
                    ("Surface", |t| &mut t.surface),
                ],
            ),
            (
                "Text",
                &[
                    ("Text Normal", |t| &mut t.text_normal),
                    ("Text Heading", |t| &mut t.text_heading),
                    ("Text Dim", |t| &mut t.text_dim),
                ],
            ),
            (
                "Semantic",
                &[
                    ("Success", |t| &mut t.success),
                    ("Warning", |t| &mut t.warning),
                    ("Error", |t| &mut t.error),
                ],
            ),
            (
                "Spectrum & Waterfall",
                &[
                    ("Spectrum Line", |t| &mut t.spectrum_line),
                    ("Spectrum Fill Top", |t| &mut t.spectrum_fill_top),
                    ("Spectrum Fill Bot", |t| &mut t.spectrum_fill_bot),
                    ("Spectrum Grid", |t| &mut t.spectrum_grid),
                    ("Noise Floor", |t| &mut t.noise_floor_line),
                    ("Waterfall BG", |t| &mut t.waterfall_bg),
                ],
            ),
            (
                "Bandplan",
                &[
                    ("HAM", |t| &mut t.bandplan_ham),
                    ("Broadcast", |t| &mut t.bandplan_broadcast),
                    ("Aviation", |t| &mut t.bandplan_aviation),
                    ("Marine", |t| &mut t.bandplan_marine),
                    ("Weather", |t| &mut t.bandplan_weather),
                    ("Satellite", |t| &mut t.bandplan_satellite),
                    ("Mobile", |t| &mut t.bandplan_mobile),
                    ("ISM", |t| &mut t.bandplan_ism),
                ],
            ),
            (
                "Bookmark Categories",
                &[
                    ("Aviation", |t| &mut t.bm_aviation),
                    ("Weather", |t| &mut t.bm_weather),
                    ("Marine", |t| &mut t.bm_marine),
                    ("Amateur", |t| &mut t.bm_amateur),
                    ("Broadcast", |t| &mut t.bm_broadcast),
                    ("Scanner", |t| &mut t.bm_scanner),
                    ("Default", |t| &mut t.bm_default),
                ],
            ),
            (
                "S-Meter",
                &[
                    ("Low", |t| &mut t.smeter_low),
                    ("Mid", |t| &mut t.smeter_mid),
                    ("High", |t| &mut t.smeter_high),
                    ("Background", |t| &mut t.smeter_bg),
                    ("Border", |t| &mut t.smeter_border),
                ],
            ),
            (
                "VFO & Status",
                &[
                    ("VFO A", |t| &mut t.vfo_a_color),
                    ("VFO B", |t| &mut t.vfo_b_color),
                    ("Status Signal", |t| &mut t.status_signal),
                    ("Status Recording", |t| &mut t.status_recording),
                ],
            ),
        ];

        for (group_name, fields) in groups {
            ui.collapsing(*group_name, |ui| {
                egui::Grid::new(format!("theme_colors_{group_name}"))
                    .num_columns(2)
                    .striped(true)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        for (label, getter) in *fields {
                            color_row(ui, label, getter(self));
                        }
                    });
            });
        }
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

// ─── Helpers ───────────────────────────────────────────────────────────────

fn bg_luminance(c: &Rgba) -> f32 {
    0.299 * f32::from(c.0) / 255.0 + 0.587 * f32::from(c.1) / 255.0 + 0.114 * f32::from(c.2) / 255.0
}

fn mix_color(a: &Rgba, b: &Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba(
        (f32::from(a.0) + (f32::from(b.0) - f32::from(a.0)) * t) as u8,
        (f32::from(a.1) + (f32::from(b.1) - f32::from(a.1)) * t) as u8,
        (f32::from(a.2) + (f32::from(b.2) - f32::from(a.2)) * t) as u8,
        (f32::from(a.3) + (f32::from(b.3) - f32::from(a.3)) * t) as u8,
    )
}
