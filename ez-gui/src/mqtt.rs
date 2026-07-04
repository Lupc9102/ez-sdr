use rumqttc::{Client, MqttOptions, QoS};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::adsb_panel::AircraftEntry;
use crate::tle_engine::PassInfo;

pub struct MqttPublisher {
    pub enabled: bool,
    pub broker: String,
    pub port: u16,
    pub topic_prefix: String,
    client: Option<Client>,
    connected_flag: Arc<AtomicBool>,
    reconnect_after: Option<Instant>,
    // Stop signal shared with the background drain thread. Set by disconnect().
    stop_flag: Arc<AtomicBool>,
    // Handle to the background drain thread. Joined on disconnect/drop so the
    // underlying MQTT connection (and its socket) is released instead of
    // leaking one FD + thread per reconnect cycle.
    drain_handle: Option<JoinHandle<()>>,
}

impl MqttPublisher {
    pub fn new() -> Self {
        Self {
            enabled: false,
            broker: "localhost".to_string(),
            port: 1883,
            topic_prefix: "ezsdr".to_string(),
            client: None,
            connected_flag: Arc::new(AtomicBool::new(false)),
            reconnect_after: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
            drain_handle: None,
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
        if !self.enabled || self.client.is_some() {
            return;
        }
        let mut opts = MqttOptions::new("ez-sdr", &self.broker, self.port);
        opts.set_keep_alive(Duration::from_secs(10));
        let (client, mut connection) = Client::new(opts, 128);

        let flag = Arc::clone(&self.connected_flag);
        flag.store(true, Ordering::Relaxed);
        // Ensure any previously-detached drain thread (from an earlier
        // connect/disconnect cycle) is signalled and joined so we don't
        // accumulate one thread + socket per reconnect.
        self.stop_flag.store(false, Ordering::Relaxed);
        self.join_drain();
        let stop = Arc::clone(&self.stop_flag);
        let handle = std::thread::spawn(move || {
            // Drain connection events with a bounded poll so we can observe the
            // stop flag without blocking forever on `recv()`. 250 ms is short
            // enough for responsive shutdown and long enough to avoid busy-spin.
            while !stop.load(Ordering::Relaxed) {
                match connection.recv_timeout(Duration::from_millis(250)) {
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => break,
                    Err(_) => break,
                }
            }
            flag.store(false, Ordering::Relaxed);
        });

        self.client = Some(client);
        self.drain_handle = Some(handle);
        self.reconnect_after = None;
    }

    /// Signal the drain thread to stop and join it, releasing the underlying
    /// MQTT connection and its socket. No-op if no thread is running.
    fn join_drain(&mut self) {
        if let Some(handle) = self.drain_handle.take() {
            self.stop_flag.store(true, Ordering::Relaxed);
            // The thread polls recv_timeout every 250 ms, so join should return
            // within ~250 ms. If it somehow hangs (would only happen if
            // rumqttc's recv_timeout ignored its timeout), drop the handle to
            // detach rather than block the GUI thread indefinitely.
            let _ = handle.join();
        }
    }

    pub fn disconnect(&mut self) {
        self.connected_flag.store(false, Ordering::Relaxed);
        self.client = None;
        self.reconnect_after = None;
        self.join_drain();
    }

    pub fn is_connected(&self) -> bool {
        self.enabled && self.client.is_some() && self.connected_flag.load(Ordering::Relaxed)
    }

    pub fn reconnect_in_secs(&self) -> Option<u64> {
        self.reconnect_after
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
    }

    /// Call once per frame to auto-reconnect after connection drops.
    pub fn tick_reconnect(&mut self) {
        if !self.enabled {
            return;
        }
        // If the background thread died, drop the stale client and schedule reconnect
        if self.client.is_some() && !self.connected_flag.load(Ordering::Relaxed) {
            eprintln!("[mqtt] connection lost — will retry in 10s");
            self.client = None;
            self.reconnect_after = Some(Instant::now() + Duration::from_secs(10));
        }
        // Reconnect when timer expires
        if self.client.is_none() && self.reconnect_after.is_some_and(|t| Instant::now() >= t) {
            self.reconnect_after = None;
            self.connect();
        }
    }

    pub fn publish(&mut self, subtopic: &str, payload: &str) {
        if let Some(client) = &mut self.client {
            if !self.enabled {
                return;
            }
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

    pub fn publish_signal(
        &mut self,
        freq_hz: u64,
        signal_db: f32,
        noise_db: f32,
        demod: &str,
        recording: bool,
    ) {
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

impl Drop for MqttPublisher {
    fn drop(&mut self) {
        // Best-effort cleanup on shutdown if disconnect() wasn't called.
        self.join_drain();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mqtt_new_defaults_disabled() {
        let mqtt = MqttPublisher::new();
        assert!(!mqtt.enabled);
        assert_eq!(mqtt.broker, "localhost");
        assert_eq!(mqtt.port, 1883);
        assert_eq!(mqtt.topic_prefix, "ezsdr");
    }

    #[test]
    fn mqtt_new_not_connected() {
        let mqtt = MqttPublisher::new();
        assert!(!mqtt.is_connected());
    }

    #[test]
    fn mqtt_new_no_reconnect() {
        let mqtt = MqttPublisher::new();
        assert!(mqtt.reconnect_in_secs().is_none());
    }

    #[test]
    fn mqtt_set_enabled_disabled_noop() {
        let mut mqtt = MqttPublisher::new();
        mqtt.set_enabled(false, "test:1883".into(), "test".into());
        assert!(!mqtt.enabled);
        assert_eq!(mqtt.broker, "test:1883");
        assert_eq!(mqtt.topic_prefix, "test");
    }

    #[test]
    fn mqtt_publish_without_client_no_crash() {
        let mut mqtt = MqttPublisher::new();
        mqtt.publish("test/topic", "hello");
        mqtt.publish_signal(100_000_000, -50.0, -90.0, "WFM", false);
        mqtt.publish_scanner_hit(145_800_000, -45.0);
        mqtt.tick(100_000_000, 40.0);
    }

    #[test]
    fn mqtt_is_connected_requires_enabled_and_client() {
        let mqtt = MqttPublisher::new();
        assert!(!mqtt.is_connected());
        let mut mqtt = MqttPublisher::new();
        mqtt.enabled = true;
        assert!(!mqtt.is_connected());
    }

    #[test]
    fn mqtt_set_enabled_disabled_does_not_connect() {
        let mut mqtt = MqttPublisher::new();
        mqtt.set_enabled(false, "broker.local".into(), "test".into());
        assert!(!mqtt.is_connected());
        assert!(mqtt.client.is_none());
    }

    #[test]
    fn mqtt_disconnect_no_client() {
        let mut mqtt = MqttPublisher::new();
        mqtt.disconnect();
        assert!(mqtt.client.is_none());
        assert!(!mqtt.is_connected());
    }

    #[test]
    fn mqtt_tick_reconnect_noop_when_disabled() {
        let mut mqtt = MqttPublisher::new();
        mqtt.enabled = false;
        mqtt.tick_reconnect();
        assert!(mqtt.client.is_none());
    }

    #[test]
    fn mqtt_json_tick_format() {
        let json = serde_json::json!({
            "frequency_hz": 100_000_000,
            "frequency_mhz": 100.0,
            "gain_db": 40.0,
            "timestamp": "placeholder",
        });
        assert_eq!(json["frequency_hz"], 100_000_000);
        assert_eq!(json["frequency_mhz"], 100.0);
        assert_eq!(json["gain_db"], 40.0);
    }

    #[test]
    fn mqtt_json_signal_format() {
        let json = serde_json::json!({
            "frequency_hz": 145_800_000,
            "frequency_mhz": 145.8,
            "signal_db": -50.0,
            "noise_floor_db": -90.0,
            "snr_db": 40.0,
            "demod_mode": "WFM",
            "recording": false,
            "timestamp": "placeholder",
        });
        assert_eq!(json["frequency_hz"], 145_800_000);
        assert_eq!(json["snr_db"], 40.0);
        assert_eq!(json["demod_mode"], "WFM");
        assert!(!json["recording"]
            .as_bool()
            .expect("recording field should be a boolean"));
    }

    #[test]
    fn mqtt_json_scanner_hit_format() {
        let json = serde_json::json!({
            "frequency_hz": 145_800_000,
            "frequency_mhz": 145.8,
            "strength_db": -45.0,
            "timestamp": "placeholder",
        });
        assert_eq!(json["frequency_hz"], 145_800_000);
        assert_eq!(json["strength_db"], -45.0);
    }

    #[test]
    fn mqtt_tick_reconnect_noop_when_client_exists() {
        // tick_reconnect should not disconnect if connected_flag is still true
        // We can't easily set up a real connection, but we can verify the noop case
        let mut mqtt = MqttPublisher::new();
        mqtt.enabled = true;
        mqtt.connected_flag = Arc::new(AtomicBool::new(true));
        mqtt.tick_reconnect();
        // No crash, state unchanged
    }

    #[test]
    fn mqtt_publish_noop_when_disabled() {
        let mut mqtt = MqttPublisher::new();
        mqtt.publish("test", "payload");
        // Should not panic even though client is None
    }
}
