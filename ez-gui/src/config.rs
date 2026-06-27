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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: "0.1.0".to_string(),
            default_freq_hz: 1_090_000_000,
            default_sample_rate: 2_048_000,
            default_gain: 40.0,
            output_directory: "./recordings".to_string(),
            theme: "dark".to_string(),
            openrouter_api_key: String::new(),
            mqtt_broker: "localhost:1883".to_string(),
            mqtt_topic_prefix: "ezsdr".to_string(),
            web_remote_enabled: false,
            web_remote_port: 5259,
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
        let _ = std::fs::write("ez_sdr_config.json", serde_json::to_string_pretty(self).unwrap());
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.add(egui::Slider::new(&mut self.default_freq_hz, 0..=1_770_000_000).text("Default freq (Hz)"));
        ui.text_edit_singleline(&mut self.output_directory);
        ui.text_edit_singleline(&mut self.mqtt_broker);
        ui.text_edit_singleline(&mut self.mqtt_topic_prefix);
        ui.checkbox(&mut self.web_remote_enabled, "Web remote");
        if ui.button("Save").clicked() {
            self.save();
        }
    }
}
