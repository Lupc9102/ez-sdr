use rumqttc::{Client, MqttOptions, QoS};
use std::time::Duration;

use crate::adsb_panel::AircraftEntry;
use crate::tle_engine::PassInfo;

pub struct MqttPublisher {
    pub enabled: bool,
    pub broker: String,
    pub port: u16,
    pub topic_prefix: String,
    client: Option<Client>,
}

impl MqttPublisher {
    pub fn new() -> Self {
        Self {
            enabled: false,
            broker: "localhost".to_string(),
            port: 1883,
            topic_prefix: "ezsdr".to_string(),
            client: None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, broker: String, topic_prefix: String) {
        self.enabled = enabled;
        self.broker = broker;
        self.topic_prefix = topic_prefix;
        if enabled {
            self.connect();
        } else {
            self.disconnect();
        }
    }

    pub fn connect(&mut self) {
        if !self.enabled || self.client.is_some() { return; }
        let mut opts = MqttOptions::new("ez-sdr", &self.broker, self.port);
        opts.set_keep_alive(Duration::from_secs(10));
        let (client, mut connection) = Client::new(opts, 128);

        std::thread::spawn(move || {
            for notification in connection.iter() {
                if notification.is_err() {
                    break;
                }
            }
        });

        self.client = Some(client);
    }

    pub fn disconnect(&mut self) {
        self.client = None;
    }

    pub fn is_connected(&self) -> bool {
        self.enabled && self.client.is_some()
    }

    pub fn publish(&mut self, subtopic: &str, payload: &str) {
        if let Some(client) = &mut self.client {
            if !self.enabled { return; }
            let topic = format!("{}/{}", self.topic_prefix, subtopic);
            let _ = client.publish(topic, QoS::AtLeastOnce, false, payload.as_bytes());
        }
    }

    pub fn tick(&mut self, freq_hz: u64, gain_db: f64) {
        let json = serde_json::json!({
            "frequency_hz": freq_hz,
            "frequency_mhz": freq_hz as f64 / 1e6,
            "gain_db": gain_db,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        self.publish("sdr/state", &json.to_string());
    }

    pub fn publish_signal(&mut self, freq_hz: u64, signal_db: f32, noise_db: f32, demod: &str, recording: bool) {
        let json = serde_json::json!({
            "frequency_hz": freq_hz,
            "frequency_mhz": freq_hz as f64 / 1e6,
            "signal_db": signal_db,
            "noise_floor_db": noise_db,
            "snr_db": signal_db - noise_db,
            "demod_mode": demod,
            "recording": recording,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        self.publish("sdr/signal", &json.to_string());
    }

    pub fn publish_scanner_hit(&mut self, freq_hz: u64, strength_db: f32) {
        let json = serde_json::json!({
            "frequency_hz": freq_hz,
            "frequency_mhz": freq_hz as f64 / 1e6,
            "strength_db": strength_db,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        self.publish("scanner/hit", &json.to_string());
    }

    pub fn publish_aircraft(&mut self, aircraft: &[AircraftEntry]) {
        for ac in aircraft {
            let json = serde_json::json!({
                "icao": format!("{:06X}", ac.icao),
                "callsign": ac.callsign,
                "lat": ac.lat,
                "lon": ac.lon,
                "altitude": ac.altitude,
                "speed": ac.speed,
                "heading": ac.heading,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            self.publish("adsb/aircraft", &json.to_string());
        }
    }

    pub fn publish_passes(&mut self, passes: &[PassInfo]) {
        let json = serde_json::json!({
            "passes": passes.iter().map(|p| serde_json::json!({
                "satellite": p.satellite,
                "aos": p.aos,
                "los": p.los,
                "max_elevation": p.max_elevation,
                "frequency_hz": p.frequency_hz,
            })).collect::<Vec<_>>(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        self.publish("satellite/passes", &json.to_string());
    }
}
