use serde::{Deserialize, Serialize};

pub const DEFAULT_AI_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
pub const DEFAULT_AI_MODEL: &str = "anthropic/claude-3-haiku";

pub struct ProviderPreset {
    pub name: &'static str,
    pub endpoint: &'static str,
    pub default_model: &'static str,
    pub needs_key: bool,
    pub note: &'static str,
}

pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset { name: "OpenRouter",    endpoint: "https://openrouter.ai/api/v1/chat/completions",   default_model: "anthropic/claude-3-5-haiku",         needs_key: true,  note: "Access 100+ models with one key. Free tier available." },
    ProviderPreset { name: "Anthropic",     endpoint: "https://api.anthropic.com/v1/messages",           default_model: "claude-3-5-haiku-20241022",           needs_key: true,  note: "Direct Anthropic API. Uses x-api-key header." },
    ProviderPreset { name: "OpenAI",        endpoint: "https://api.openai.com/v1/chat/completions",      default_model: "gpt-4o-mini",                         needs_key: true,  note: "Direct OpenAI API." },
    ProviderPreset { name: "Groq",          endpoint: "https://api.groq.com/openai/v1/chat/completions", default_model: "llama-3.1-8b-instant",                needs_key: true,  note: "Very fast inference. Free tier available." },
    ProviderPreset { name: "Mistral",       endpoint: "https://api.mistral.ai/v1/chat/completions",      default_model: "mistral-small-latest",                needs_key: true,  note: "European provider, strong multilingual." },
    ProviderPreset { name: "Ollama (local)",endpoint: "http://localhost:11434/v1/chat/completions",      default_model: "llama3.2",                            needs_key: false, note: "Fully local, no key needed. Install Ollama first." },
    ProviderPreset { name: "Custom",        endpoint: "",                                                 default_model: "",                                    needs_key: true,  note: "Set endpoint and model manually." },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: String,
    pub default_freq_hz: u64,
    pub default_sample_rate: u32,
    pub default_gain: f64,
    pub output_directory: String,
    pub theme: String,
    pub ai_api_key: String,
    pub ai_endpoint: String,
    pub ai_model: String,
    pub ai_max_tokens: u32,
    pub ai_temperature: f64,
    pub ai_system_prompt: String,
    pub ai_provider: String,
    pub mqtt_broker: String,
    pub mqtt_topic_prefix: String,
    pub web_remote_enabled: bool,
    pub web_remote_port: u16,
    pub observer_lat: f64,
    pub observer_lon: f64,
    pub font_scale: f64,
    pub needs_apply: bool,
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
            mqtt_broker: "localhost:1883".to_string(),
            mqtt_topic_prefix: "ezsdr".to_string(),
            web_remote_enabled: false,
            web_remote_port: 5259,
            observer_lat: 51.5,
            observer_lon: -0.1,
            font_scale: 1.0,
            needs_apply: false,
        }
    }
}

impl AppConfig {
    pub fn load_or_default() -> Self {
        std::fs::read_to_string("ez_sdr_config.json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write("ez_sdr_config.json", json);
        }
    }

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
                    .custom_formatter(|v, _| format!("{:.1} dB", v)))
                    .on_hover_text("RF amplification. Higher is not always better — too much gain causes overload and phantom signals. Typical sweet spot: 30–45 dB.");
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
                ui.add(egui::Slider::new(&mut self.ai_max_tokens, 256u32..=16384u32)
                    .step_by(256.0).text("Max tokens"))
                    .on_hover_text("Maximum response length in tokens (~4 chars each). 2048 is plenty for most tasks.");
                ui.collapsing("System prompt", |ui| {
                    ui.label("Leave empty for default (tool-enabled assistant).");
                    ui.add_sized([400.0, 120.0],
                        egui::TextEdit::multiline(&mut self.ai_system_prompt));
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

            ui.collapsing("Appearance", |ui| {
                ui.label("Theme:").on_hover_text("Switch between dark and light UI themes.");
                ui.horizontal(|ui| {
                    for t in &["dark", "light"] {
                        if ui.selectable_label(&self.theme == *t, *t).clicked() {
                            self.theme = t.to_string();
                            self.needs_apply = true;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label("Font scale:").on_hover_text("Scale all UI text. 1.0 is default. Increase for high-DPI displays or if text is too small.");
                let resp = ui.add(egui::Slider::new(&mut self.font_scale, 0.6..=2.0)
                    .step_by(0.05)
                    .text("")
                    .custom_formatter(|v, _| format!("{:.2}x", v)));
                if resp.changed() {
                    self.needs_apply = true;
                }
                ui.horizontal(|ui| {
                    for (label, scale) in [("Small", 0.8f64), ("Normal", 1.0), ("Large", 1.3), ("XL", 1.6)] {
                        if ui.small_button(label).clicked() {
                            self.font_scale = scale;
                            self.needs_apply = true;
                        }
                    }
                });
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
                            let _ = std::fs::write(&path, json);
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
                "Settings are saved to ez_sdr_config.json in the current directory.",
            );
        });
    }
}
