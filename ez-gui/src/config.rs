use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: String,
    pub default_freq_hz: u64,
    pub default_sample_rate: u32,
    pub default_gain: f64,
    pub output_directory: String,
    pub theme: String,
    pub openrouter_api_key: String,
    pub mqtt_broker: String,
    pub mqtt_topic_prefix: String,
    pub web_remote_enabled: bool,
    pub web_remote_port: u16,
    pub observer_lat: f64,
    pub observer_lon: f64,
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
            openrouter_api_key: String::new(),
            mqtt_broker: "localhost:1883".to_string(),
            mqtt_topic_prefix: "ezsdr".to_string(),
            web_remote_enabled: false,
            web_remote_port: 5259,
            observer_lat: 51.5,
            observer_lon: -0.1,
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
                    .custom_formatter(|v, _| format!("{:.3} MHz", v / 1e6)));
                ui.add(egui::Slider::new(&mut self.default_sample_rate, 225_001..=3_200_000)
                    .text("Sample rate")
                    .custom_formatter(|v, _| format!("{:.3} MSps", v / 1e6)));
                ui.add(egui::Slider::new(&mut self.default_gain, 0.0..=49.6)
                    .step_by(0.1)
                    .text("Gain (dB)")
                    .custom_formatter(|v, _| format!("{:.1} dB", v)));
            });

            ui.collapsing("Recording", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Output directory:");
                    ui.add(egui::TextEdit::singleline(&mut self.output_directory).desired_width(200.0));
                });
            });

            ui.collapsing("AI Agent", |ui| {
                ui.horizontal(|ui| {
                    ui.label("OpenRouter API Key:");
                    ui.add(egui::TextEdit::singleline(&mut self.openrouter_api_key).password(true).desired_width(200.0));
                });
            });

            ui.collapsing("MQTT", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Broker:");
                    ui.add(egui::TextEdit::singleline(&mut self.mqtt_broker).desired_width(200.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Topic prefix:");
                    ui.add(egui::TextEdit::singleline(&mut self.mqtt_topic_prefix).desired_width(200.0));
                });
            });

            ui.collapsing("Web Remote", |ui| {
                ui.checkbox(&mut self.web_remote_enabled, "Enable web remote");
                ui.add(egui::Slider::new(&mut self.web_remote_port, 1024..=65535).text("Port"));
            });

            ui.collapsing("Satellite Observer Location", |ui| {
                ui.add(egui::Slider::new(&mut self.observer_lat, -90.0..=90.0).text("Latitude"));
                ui.add(egui::Slider::new(&mut self.observer_lon, -180.0..=180.0).text("Longitude"));
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    self.save();
                    self.needs_apply = true;
                }
                if ui.button("Reset to defaults").clicked() {
                    *self = Self::default();
                    self.needs_apply = true;
                }
            });
        });
    }
}
