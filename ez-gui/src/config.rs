//! Persistent application configuration.
//!
//! Provides [`AppConfig`] for serializing/deserializing all SDR, UI, theme,
//! Discord, and AI settings to/from `ez_sdr_config.json`, including theme
//! configuration ([`ThemeConfig`]) and Discord integration settings.

use crate::discord::DiscordSettings;
use crate::theme::{NamedTheme, ThemeConfig};
use serde::{Deserialize, Serialize};

/// A single togglable/reorderable entry in the sidebar layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutItem {
    pub id: String,
    pub visible: bool,
}

/// Which main tabs and secondary tools appear in the sidebar, and in what
/// order. Driven entirely by user customization — see the Layout sub-tab of
/// the Customize panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub main_tabs: Vec<LayoutItem>,
    pub secondary_tools: Vec<LayoutItem>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        fn items(ids: &[&str]) -> Vec<LayoutItem> {
            ids.iter()
                .map(|id| LayoutItem {
                    id: (*id).to_string(),
                    visible: true,
                })
                .collect()
        }
        Self {
            main_tabs: items(&["sdr", "adsb", "satellite", "ai"]),
            secondary_tools: items(&[
                "bookmarks",
                "scanner",
                "recorder",
                "scheduler",
                "discord",
                "howto",
                "settings",
            ]),
        }
    }
}

/// Default AI provider API endpoint (`OpenRouter`).
pub const DEFAULT_AI_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
/// Default AI model identifier.
pub const DEFAULT_AI_MODEL: &str = "anthropic/claude-3-haiku";

/// A known AI provider preset with its endpoint, default model, and notes.
pub struct ProviderPreset {
    /// Display name of the provider.
    pub name: &'static str,
    /// Base URL for the provider's chat completions API.
    pub endpoint: &'static str,
    /// Recommended default model ID for this provider.
    pub default_model: &'static str,
    /// Whether this provider requires an API key.
    pub needs_key: bool,
    /// Short hint about the provider (e.g. "Free tier available").
    pub note: &'static str,
}

pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "OpenRouter",
        endpoint: "https://openrouter.ai/api/v1/chat/completions",
        default_model: "anthropic/claude-3-5-haiku",
        needs_key: true,
        note: "Access 100+ models with one key. Free tier available.",
    },
    ProviderPreset {
        name: "Anthropic",
        endpoint: "https://api.anthropic.com/v1/messages",
        default_model: "claude-3-5-haiku-20241022",
        needs_key: true,
        note: "Direct Anthropic API. Uses x-api-key header.",
    },
    ProviderPreset {
        name: "OpenAI",
        endpoint: "https://api.openai.com/v1/chat/completions",
        default_model: "gpt-4o-mini",
        needs_key: true,
        note: "Direct OpenAI API.",
    },
    ProviderPreset {
        name: "Groq",
        endpoint: "https://api.groq.com/openai/v1/chat/completions",
        default_model: "llama-3.1-8b-instant",
        needs_key: true,
        note: "Very fast inference. Free tier available.",
    },
    ProviderPreset {
        name: "Mistral",
        endpoint: "https://api.mistral.ai/v1/chat/completions",
        default_model: "mistral-small-latest",
        needs_key: true,
        note: "European provider, strong multilingual.",
    },
    ProviderPreset {
        name: "Ollama (local)",
        endpoint: "http://localhost:11434/v1/chat/completions",
        default_model: "llama3.2",
        needs_key: false,
        note: "Fully local, no key needed. Install Ollama first.",
    },
    ProviderPreset {
        name: "Custom",
        endpoint: "",
        default_model: "",
        needs_key: true,
        note: "Set endpoint and model manually.",
    },
];

/// Top-level application configuration persisted to `ez_sdr_config.json`.
///
/// Contains all SDR, UI, theme, AI, MQTT, web remote, satellite, and Discord
/// settings. Serialised/deserialised with serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Schema version string.
    #[serde(default)]
    pub version: String,
    /// Default centre frequency (Hz) on startup.
    #[serde(default)]
    pub default_freq_hz: u64,
    /// Default sample rate (samples/second).
    #[serde(default)]
    pub default_sample_rate: u32,
    /// Default RF gain (dB).
    #[serde(default)]
    pub default_gain: f64,
    /// Directory for saving recorded I/Q and audio files.
    #[serde(default)]
    pub output_directory: String,
    /// UI theme name ("dark" / "light").
    #[serde(default)]
    pub theme: String,
    /// AI provider API key.
    #[serde(default)]
    pub ai_api_key: String,
    /// AI provider API endpoint URL.
    #[serde(default)]
    pub ai_endpoint: String,
    /// AI model identifier.
    #[serde(default)]
    pub ai_model: String,
    /// Maximum tokens per AI response.
    #[serde(default)]
    pub ai_max_tokens: u32,
    /// AI temperature (0.0 – 2.0).
    #[serde(default)]
    pub ai_temperature: f64,
    /// Custom system prompt for the AI agent.
    #[serde(default)]
    pub ai_system_prompt: String,
    /// AI provider name (matches a [`ProviderPreset`] entry).
    #[serde(default)]
    pub ai_provider: String,
    /// Reasoning effort level ("off", "low", "medium", "high").
    #[serde(default)]
    pub ai_reasoning_effort: String,
    /// Whether the AI agent has web-search capability enabled.
    #[serde(default)]
    pub ai_web_search: bool,
    /// MQTT broker address (host:port).
    #[serde(default)]
    pub mqtt_broker: String,
    /// MQTT topic prefix for all published messages.
    #[serde(default)]
    pub mqtt_topic_prefix: String,
    /// Whether the web remote control server is enabled.
    #[serde(default)]
    pub web_remote_enabled: bool,
    /// TCP port for the web remote server.
    #[serde(default)]
    pub web_remote_port: u16,
    /// Observer latitude (decimal degrees, north positive).
    #[serde(default)]
    pub observer_lat: f64,
    /// Observer longitude (decimal degrees, east positive).
    #[serde(default)]
    pub observer_lon: f64,
    /// UI font scale multiplier.
    #[serde(default)]
    pub font_scale: f64,
    /// Flag indicating settings have changed and need to be applied.
    #[serde(default)]
    pub needs_apply: bool,
    /// Recently tuned frequencies (for quick-access menu).
    #[serde(default)]
    pub recent_frequencies: Vec<u64>,
    /// Spectrum display minimum (dBFS).
    #[serde(default)]
    pub spectrum_min_db: f32,
    /// Spectrum display maximum (dBFS).
    #[serde(default)]
    pub spectrum_max_db: f32,
    /// Frequency correction in parts-per-million.
    #[serde(default)]
    pub ppm_correction: i32,
    /// VFO B frequency (Hz).
    #[serde(default)]
    pub vfo_b_hz: u64,
    /// Waterfall colour range minimum (dBFS).
    #[serde(default)]
    pub wf_min_db: f32,
    /// Waterfall colour range maximum (dBFS).
    #[serde(default)]
    pub wf_max_db: f32,
    /// Local oscillator offset (Hz) for upconverter / downconverter.
    #[serde(default)]
    pub lo_offset_hz: i64,
    /// Whether the welcome dialog has been shown (migration flag).
    #[serde(default)]
    pub welcome_seen: bool,
    /// Last-used frequency from previous session.
    #[serde(default)]
    pub last_session_freq_hz: u64,
    /// Last-used gain from previous session.
    #[serde(default)]
    pub last_session_gain_db: f64,
    /// Last-used demodulation mode from previous session.
    #[serde(default)]
    pub last_session_demod: String,
    /// Waterfall colour map name.
    #[serde(default)]
    pub color_map: String,
    /// Frequency memory slot values (Hz).
    #[serde(default)]
    pub freq_memory_hz: Vec<u64>,
    /// Frequency memory slot labels.
    #[serde(default)]
    pub freq_memory_labels: Vec<String>,
    /// Theme configuration (colours, presets).
    #[serde(default)]
    pub theme_config: ThemeConfig,
    /// Discord notification settings.
    #[serde(default)]
    pub discord: DiscordSettings,
    /// Whether to skip the antenna-setup checklist on startup.
    #[serde(default)]
    pub skip_antenna_checklists: bool,
    /// User experience level string (e.g. "beginner", "advanced").
    #[serde(default)]
    pub user_level: String,
    /// Whether the interactive tutorial has been seen.
    #[serde(default)]
    pub tutorial_seen: bool,
    /// Current tutorial step index.
    #[serde(default)]
    pub tutorial_step: usize,
    /// User-saved named themes (the Customize tab's theme gallery), distinct
    /// from the built-in presets in [`ThemeConfig::all_presets`].
    #[serde(default)]
    pub custom_themes: Vec<NamedTheme>,
    /// Sidebar tab/tool visibility and ordering.
    #[serde(default)]
    pub layout: LayoutConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: "0.1.0".to_string(),
            default_freq_hz: 100_000_000,
            default_sample_rate: 2_048_000,
            default_gain: 40.0,
            output_directory: "./recordings".to_string(),
            theme: "dark".to_string(),
            ai_api_key: String::new(),
            ai_endpoint: DEFAULT_AI_ENDPOINT.to_string(),
            ai_model: DEFAULT_AI_MODEL.to_string(),
            ai_max_tokens: 2048,
            ai_temperature: 0.7,
            ai_system_prompt: String::new(),
            ai_provider: "OpenRouter".to_string(),
            ai_reasoning_effort: "off".to_string(),
            ai_web_search: false,
            mqtt_broker: "localhost:1883".to_string(),
            mqtt_topic_prefix: "ezsdr".to_string(),
            web_remote_enabled: false,
            web_remote_port: 5259,
            observer_lat: 51.5,
            observer_lon: -0.1,
            font_scale: 1.0,
            needs_apply: false,
            recent_frequencies: Vec::new(),
            spectrum_min_db: -120.0,
            spectrum_max_db: 0.0,
            ppm_correction: 0,
            vfo_b_hz: 0,
            wf_min_db: -120.0,
            wf_max_db: -20.0,
            lo_offset_hz: 0,
            welcome_seen: false,
            last_session_freq_hz: 0,
            last_session_gain_db: -1.0,
            last_session_demod: String::new(),
            color_map: "Classic".to_string(),
            freq_memory_hz: Vec::new(),
            freq_memory_labels: Vec::new(),
            theme_config: ThemeConfig::default(),
            discord: DiscordSettings::default(),
            skip_antenna_checklists: false,
            user_level: "beginner".to_string(),
            tutorial_seen: false,
            tutorial_step: 0,
            custom_themes: Vec::new(),
            layout: LayoutConfig::default(),
        }
    }
}

impl AppConfig {
    /// Load configuration from `ez_sdr_config.json`, or return defaults if the
    /// file does not exist or cannot be parsed. Also migrates the legacy
    /// `welcome_seen` flag to the `tutorial_seen` field.
    pub fn load_or_default() -> Self {
        let mut cfg = std::fs::read_to_string("ez_sdr_config.json")
            .ok()
            .and_then(|s| serde_json::from_str::<AppConfig>(&s).ok())
            .unwrap_or_default();
        // Migrate from old welcome_seen to tutorial_seen
        if cfg.welcome_seen && !cfg.tutorial_seen {
            cfg.tutorial_seen = true;
        }
        cfg
    }

    /// Serialise and write the configuration to `ez_sdr_config.json`.
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write("ez_sdr_config.json", &json) {
                eprintln!("[config] failed to write config file: {}", e);
            }
        }
    }
}

impl AppConfig {
    /// Render the egui-based settings panel UI.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.collapsing("Source", |ui| {
                ui.add(egui::Slider::new(&mut self.default_freq_hz, 500_000..=1_770_000_000)
                    .text("Default frequency")
                    .custom_formatter(|v, _| format!("{:.3} MHz", v / 1e6)))
                    .on_hover_text("The frequency the SDR tunes to when first started.");
                ui.add(egui::Slider::new(&mut self.default_sample_rate, 225_001..=3_200_000)
                    .text("Sample rate")
                    .custom_formatter(|v, _| format!("{:.3} MSps", v / 1e6)))
                    .on_hover_text("How many samples per second the ADC captures. Also sets the visible spectrum width. Max stable for RTL-SDR: 2.4 MSps.");
                ui.add(egui::Slider::new(&mut self.default_gain, 0.0..=49.6)
                    .step_by(0.1)
                    .text("Gain (dB)")
                    .custom_formatter(|v, _| format!("{v:.1} dB")))
                    .on_hover_text("RF amplification. Higher is not always better — too much gain causes overload and phantom signals. Typical sweet spot: 30–45 dB.");
            });

            ui.collapsing("Frequency Calibration", |ui| {
                ui.label("PPM Correction (parts per million):")
                    .on_hover_text("Crystal frequency error compensation. Positive = crystal runs slow, negative = runs fast. Typical RTL-SDR: ±20 ppm.");
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.ppm_correction, -100..=100)
                        .text("PPM")
                        .custom_formatter(|v, _| format!("{v:+.0} ppm")));
                });
                ui.label("Quick adjust:").on_hover_text("Click to adjust PPM by preset amounts.");
                ui.horizontal_wrapped(|ui| {
                    for offset in &[-50, -20, -10, -5, 0, 5, 10, 20, 50] {
                        let label = if *offset == 0 { "Reset".to_string() } else { format!("{offset:+}") };
                        if ui.small_button(label).clicked() {
                            self.ppm_correction = *offset;
                        }
                    }
                });
                ui.label(format!("Current: {} ppm", self.ppm_correction))
                    .on_hover_text("Tune a known frequency and adjust this value until it matches exactly.");
            });

            ui.collapsing("Recording", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Output directory:").on_hover_text("Where recorded I/Q and audio files are saved.");
                    ui.add(egui::TextEdit::singleline(&mut self.output_directory).desired_width(200.0));
                });
            });

            ui.collapsing("AI Agent", |ui| {
                // Provider picker
                ui.label(egui::RichText::new("Provider").strong());
                let current = self.ai_provider.clone();
                egui::ComboBox::from_id_salt("ai_provider_combo")
                    .selected_text(&current)
                    .show_ui(ui, |ui| {
                        for preset in PROVIDER_PRESETS {
                            if ui.selectable_label(current == preset.name, preset.name).clicked() {
                                self.ai_provider = preset.name.to_string();
                                if preset.name != "Custom" {
                                    self.ai_endpoint = preset.endpoint.to_string();
                                    self.ai_model = preset.default_model.to_string();
                                }
                            }
                        }
                    });

                if let Some(preset) = PROVIDER_PRESETS.iter().find(|p| p.name == self.ai_provider) {
                    ui.colored_label(egui::Color32::GRAY, preset.note);
                    if !preset.needs_key {
                        ui.colored_label(egui::Color32::from_rgb(100, 220, 100), "No API key required.");
                    }
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("API Key:").on_hover_text("Your provider API key. Stored locally in ez_sdr_config.json. Leave blank for Ollama.");
                    ui.add(egui::TextEdit::singleline(&mut self.ai_api_key).password(true).desired_width(260.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Endpoint:").on_hover_text("The full URL for the /chat/completions API. Auto-filled when you pick a provider.");
                    ui.add(egui::TextEdit::singleline(&mut self.ai_endpoint).desired_width(360.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Model:").on_hover_text("The model ID to request. Auto-filled from the provider preset, but you can override it.");
                    ui.add(egui::TextEdit::singleline(&mut self.ai_model).desired_width(260.0));
                });

                // Suggested models for current provider
                if let Some(preset) = PROVIDER_PRESETS.iter().find(|p| p.name == self.ai_provider) {
                    if preset.name == "OpenRouter" {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new("Popular models:").small());
                        ui.horizontal_wrapped(|ui| {
                            for m in &["anthropic/claude-3-5-haiku", "google/gemini-flash-1.5", "meta-llama/llama-3.1-8b-instruct:free", "mistralai/mistral-7b-instruct:free"] {
                                if ui.small_button(*m).clicked() {
                                    self.ai_model = m.to_string();
                                }
                            }
                        });
                    } else if preset.name == "Groq" {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new("Popular models:").small());
                        ui.horizontal_wrapped(|ui| {
                            for m in &["llama-3.1-8b-instant", "llama-3.3-70b-versatile", "mixtral-8x7b-32768", "gemma2-9b-it"] {
                                if ui.small_button(*m).clicked() {
                                    self.ai_model = m.to_string();
                                }
                            }
                        });
                    } else if preset.name == "Anthropic" {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new("Popular models:").small());
                        ui.horizontal_wrapped(|ui| {
                            for m in &["claude-3-5-haiku-20241022", "claude-3-5-sonnet-20241022", "claude-3-opus-20240229"] {
                                if ui.small_button(*m).clicked() {
                                    self.ai_model = m.to_string();
                                }
                            }
                        });
                    } else if preset.name == "OpenAI" {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new("Popular models:").small());
                        ui.horizontal_wrapped(|ui| {
                            for m in &["gpt-4o-mini", "gpt-4o", "gpt-3.5-turbo"] {
                                if ui.small_button(*m).clicked() {
                                    self.ai_model = m.to_string();
                                }
                            }
                        });
                    } else if preset.name == "Ollama (local)" {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new("Popular models (must be pulled first):").small());
                        ui.horizontal_wrapped(|ui| {
                            for m in &["llama3.2", "llama3.1", "mistral", "gemma2", "qwen2.5"] {
                                if ui.small_button(*m).clicked() {
                                    self.ai_model = m.to_string();
                                }
                            }
                        });
                    }
                }

                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut self.ai_temperature, 0.0..=2.0)
                    .step_by(0.05).text("Temperature"))
                    .on_hover_text("Randomness of responses. 0 = deterministic, 1 = balanced, 2 = very creative. 0.5–0.7 works well for radio control.");

                // Reasoning effort
                ui.horizontal(|ui| {
                    ui.label("Reasoning:").on_hover_text("Controls how hard the model thinks. Higher = slower but more thorough. 'Off' uses default behavior.");
                    let current = self.ai_reasoning_effort.clone();
                    egui::ComboBox::from_id_salt("ai_reasoning_effort")
                        .selected_text(&current)
                        .show_ui(ui, |ui| {
                            for level in &["off", "low", "medium", "high"] {
                                if ui.selectable_label(current == *level, *level).clicked() {
                                    self.ai_reasoning_effort = level.to_string();
                                }
                            }
                        });
                });

                // Web search toggle
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.ai_web_search, " Enable web search")
                        .on_hover_text("Allow the AI to search the web for external information (current events, specs, frequency databases, etc.). Uses DuckDuckGo — no API key needed.");
                });

                ui.add(egui::Slider::new(&mut self.ai_max_tokens, 256u32..=16384u32)
                    .step_by(256.0).text("Max tokens"))
                    .on_hover_text("Maximum response length in tokens (~4 chars each). 2048 is plenty for most tasks.");
                ui.collapsing("System prompt", |ui| {
                    ui.label("Leave empty for default (tool-enabled assistant).");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.ai_system_prompt)
                            .desired_width(f32::INFINITY));
                });
            });

            ui.collapsing("MQTT", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Broker:").on_hover_text("MQTT broker address and port, e.g. localhost:1883. MQTT lets other systems subscribe to SDR state.");
                    ui.add(egui::TextEdit::singleline(&mut self.mqtt_broker).desired_width(200.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Topic prefix:").on_hover_text("All MQTT topics will be prefixed with this. e.g. 'ezsdr' → 'ezsdr/frequency'.");
                    ui.add(egui::TextEdit::singleline(&mut self.mqtt_topic_prefix).desired_width(200.0));
                });
            });

            ui.collapsing("Web Remote", |ui| {
                ui.checkbox(&mut self.web_remote_enabled, "Enable web remote")
                    .on_hover_text("Starts a local HTTP server so you can control the SDR from a browser on your LAN.");
                ui.add(egui::Slider::new(&mut self.web_remote_port, 1024..=65535).text("Port"))
                    .on_hover_text("TCP port for the web remote. Default 5259. Open http://localhost:5259 in a browser.");
            });

            ui.colored_label(
                egui::Color32::GRAY,
                "🎨 Theme, fonts, effects, and layout have moved to the Customize tab.",
            );

        ui.collapsing("User Experience", |ui| {
            ui.label(egui::RichText::new("Knowledge Level").strong())
                .on_hover_text("Controls which UI controls and features are visible. Beginners see simplified panels with more hints; Experts see everything.");
            let user_level = crate::user_level::UserLevel::from_str(&self.user_level);
            let mut level_idx = user_level as usize;
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut level_idx, 0..=3).step_by(1.0).text("Level"))
                    .on_hover_text("Drag to change your experience level.");
            });
            // Show the name + description below the slider
            if let Some(level) = [crate::user_level::UserLevel::Beginner,
                                  crate::user_level::UserLevel::Intermediate,
                                  crate::user_level::UserLevel::Advanced,
                                  crate::user_level::UserLevel::ClerkMaxwell].get(level_idx) {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 255),
                    format!("{} — {}", level.label(), level.description()));
                self.user_level = level.to_str().to_string();
            }

            ui.add_space(4.0);
            if ui.button("🔁 Restart Tutorial").on_hover_text("Re-open the first-run tutorial on next launch.").clicked() {
                self.tutorial_seen = false;
                self.tutorial_step = 0;
                self.needs_apply = true;
            }
        });

            ui.collapsing("Satellite Observer Location", |ui| {
                ui.add(egui::Slider::new(&mut self.observer_lat, -90.0..=90.0).text("Latitude"))
                    .on_hover_text("Your latitude in decimal degrees. North positive. Used to predict satellite pass times.");
                ui.add(egui::Slider::new(&mut self.observer_lon, -180.0..=180.0).text("Longitude"))
                    .on_hover_text("Your longitude in decimal degrees. East positive. Used together with latitude to compute pass elevation angles.");
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("💾 Save & Apply").on_hover_text("Save settings to ez_sdr_config.json and apply them immediately.").clicked() {
                    self.save();
                    self.needs_apply = true;
                }
                if ui.button("Reset to defaults").on_hover_text("Restore all settings to factory defaults. Does not delete saved recordings.").clicked() {
                    *self = Self::default();
                    self.needs_apply = true;
                }
                if ui.button("📤 Export…").on_hover_text("Export config to a custom file path via file dialog.").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name("ez_sdr_config_backup.json")
                        .add_filter("JSON", &["json"])
                        .save_file()
                    {
                        if let Ok(json) = serde_json::to_string_pretty(self) {
                            if let Err(e) = std::fs::write(&path, json) {
                                eprintln!("[config] failed to export config to {}: {}", path.display(), e);
                            }
                        }
                    }
                }
                if ui.button("📥 Import…").on_hover_text("Load config from a previously exported JSON file.").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("JSON", &["json"])
                        .pick_file()
                    {
                        if let Ok(data) = std::fs::read_to_string(&path) {
                            if let Ok(loaded) = serde_json::from_str::<AppConfig>(&data) {
                                *self = loaded;
                                self.needs_apply = true;
                            }
                        }
                    }
                }
            });
            ui.colored_label(
                egui::Color32::GRAY,
                "Settings are saved to ez_sdr_config.json in the current directory. Spectrum dB range is saved with Ctrl+S.",
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_has_reasonable_freq() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.default_freq_hz, 100_000_000);
    }

    #[test]
    fn config_default_sample_rate() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.default_sample_rate, 2_048_000);
    }

    #[test]
    fn config_default_theme_is_dark() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.theme, "dark");
    }

    #[test]
    fn config_default_observer_at_london() {
        let cfg = AppConfig::default();
        assert!((cfg.observer_lat - 51.5).abs() < f64::EPSILON);
        assert!((cfg.observer_lon - (-0.1)).abs() < f64::EPSILON);
    }

    #[test]
    fn config_default_has_theme_and_discord() {
        let cfg = AppConfig::default();
        // theme_config should have the default preset
        assert_eq!(cfg.theme_config.preset, "dark");
        // discord should default to disabled
        assert!(!cfg.discord.enabled);
    }

    #[test]
    fn config_serde_roundtrip_preserves_fields() {
        let cfg = AppConfig::default();
        let json = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.default_freq_hz, cfg.default_freq_hz);
        assert_eq!(deserialized.theme, cfg.theme);
        assert_eq!(deserialized.ai_model, cfg.ai_model);
    }

    #[test]
    fn provider_presets_have_names() {
        for p in PROVIDER_PRESETS {
            assert!(!p.name.is_empty());
        }
        assert!(PROVIDER_PRESETS.iter().any(|p| p.name == "Custom"));
    }

    #[test]
    fn config_load_or_default_nonexistent_returns_default() {
        let cfg = AppConfig::load_or_default();
        assert_eq!(cfg.default_freq_hz, 100_000_000);
        assert_eq!(cfg.theme, "dark");
    }

    #[test]
    fn config_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ez_sdr_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let original_dir = std::env::current_dir().expect("test should have a current directory");
        std::env::set_current_dir(&dir).expect("test should cd into temp dir");

        let cfg = AppConfig {
            default_freq_hz: 433_000_000,
            theme: "light".to_string(),
            ai_model: "custom-model".to_string(),
            output_directory: "/tmp/recordings".to_string(),
            observer_lat: 40.7128,
            observer_lon: -74.0060,
            ..Default::default()
        };
        cfg.save();

        let loaded = AppConfig::load_or_default();
        assert_eq!(loaded.default_freq_hz, 433_000_000);
        assert_eq!(loaded.theme, "light");
        assert_eq!(loaded.ai_model, "custom-model");
        assert_eq!(loaded.output_directory, "/tmp/recordings");
        assert!((loaded.observer_lat - 40.7128).abs() < f64::EPSILON);
        assert!((loaded.observer_lon - (-74.0060)).abs() < f64::EPSILON);

        std::env::set_current_dir(original_dir).expect("test should restore original directory");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_partial_json_missing_fields_get_defaults() {
        let json = r#"{"default_freq_hz": 144000000}"#;
        let cfg: AppConfig = serde_json::from_str(json).expect("partial deserialize");
        assert_eq!(cfg.default_freq_hz, 144_000_000);
        // Fields not in JSON get Default::default() for their type
        assert_eq!(cfg.theme, "");
        assert_eq!(cfg.default_sample_rate, 0u32);
        assert!(!cfg.discord.enabled);
    }

    #[test]
    fn layout_config_default_lists_all_current_tabs_and_tools_visible() {
        let layout = LayoutConfig::default();
        assert_eq!(
            layout
                .main_tabs
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            vec!["sdr", "adsb", "satellite", "ai"]
        );
        assert_eq!(
            layout
                .secondary_tools
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "bookmarks",
                "scanner",
                "recorder",
                "scheduler",
                "discord",
                "howto",
                "settings"
            ]
        );
        assert!(layout.main_tabs.iter().all(|i| i.visible));
        assert!(layout.secondary_tools.iter().all(|i| i.visible));
    }

    #[test]
    fn config_default_has_empty_custom_theme_gallery() {
        let cfg = AppConfig::default();
        assert!(cfg.custom_themes.is_empty());
    }

    #[test]
    fn config_missing_layout_and_custom_themes_fall_back_to_defaults() {
        let json = r#"{"default_freq_hz": 144000000}"#;
        let cfg: AppConfig = serde_json::from_str(json).expect("partial deserialize");
        assert_eq!(cfg.layout, LayoutConfig::default());
        assert!(cfg.custom_themes.is_empty());
    }

    #[test]
    fn config_custom_themes_roundtrip_through_json() {
        let mut cfg = AppConfig::default();
        cfg.custom_themes.push(NamedTheme {
            id: "t1".to_string(),
            name: "My Custom".to_string(),
            theme: ThemeConfig::cyberpunk(),
        });
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.custom_themes.len(), 1);
        assert_eq!(back.custom_themes[0].name, "My Custom");
        assert_eq!(back.custom_themes[0].theme.preset, "cyberpunk");
    }

    #[test]
    fn config_partial_json_theme_and_discord() {
        let json = r#"{"theme": "light", "discord": {"enabled": true}}"#;
        let cfg: AppConfig = serde_json::from_str(json)
            .expect("config partial JSON should deserialize with defaults");
        assert_eq!(cfg.theme, "light");
        assert!(cfg.discord.enabled);
        assert_eq!(cfg.default_freq_hz, 0);
    }

    #[test]
    fn config_edge_values_zero_freq() {
        let json = r#"{"version": "0.1.0", "default_freq_hz": 0}"#;
        let cfg: AppConfig =
            serde_json::from_str(json).expect("config edge values JSON should deserialize");
        assert_eq!(cfg.default_freq_hz, 0);
    }

    #[test]
    fn config_edge_values_max_freq() {
        let json = format!(r#"{{"version": "0.1.0", "default_freq_hz": {}}}"#, u64::MAX);
        let cfg: AppConfig =
            serde_json::from_str(&json).expect("config max freq JSON should deserialize");
        assert_eq!(cfg.default_freq_hz, u64::MAX);
    }

    #[test]
    fn config_edge_values_empty_strings() {
        let json = r#"{"version": "0.1.0", "theme": "", "output_directory": "", "ai_model": ""}"#;
        let cfg: AppConfig =
            serde_json::from_str(json).expect("config empty strings JSON should deserialize");
        assert_eq!(cfg.theme, "");
        assert_eq!(cfg.output_directory, "");
        assert_eq!(cfg.ai_model, "");
    }
}
