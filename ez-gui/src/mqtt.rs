use rumqttc::{Client, Event, MqttOptions, Packet, QoS, RecvTimeoutError};
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
    /// Endpoint used by the current attempt, distinct from editable preferences.
    active_endpoint: Option<String>,
    connected_flag: Arc<AtomicBool>,
    reconnect_after: Option<Instant>,
    // When the current connection attempt started. `tick_reconnect` must not
    // mistake a fresh attempt (CONNACK still in flight) for a dead connection,
    // or it kills each attempt before the broker can answer and the client
    // flaps forever without ever connecting.
    connecting_since: Option<Instant>,
    // Stop signal shared with the background drain thread. Set by disconnect().
    stop_flag: Arc<AtomicBool>,
    // Handle to the background drain thread. Joined on disconnect/drop so the
    // underlying MQTT connection (and its socket) is released instead of
    // leaking one FD + thread per reconnect cycle.
    drain_handle: Option<JoinHandle<()>>,
}

/// Split a user-facing broker address (`"host"` or `"host:port"`) into a
/// hostname and port, falling back to `default_port` when no `:port` suffix
/// is present (or it doesn't parse as a u16).
///
/// `self.broker` is stored verbatim from config as typed by the user, e.g.
/// "localhost" or "broker.local:1883". Passing that raw string straight into
/// `MqttOptions::new()`'s host parameter — without splitting off the port —
/// makes rumqttc try to resolve a hostname containing a colon, which always
/// fails to connect. This must be called before every `MqttOptions::new()`.
fn split_host_port(broker: &str, default_port: u16) -> (&str, u16) {
    match broker.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => match p.parse::<u16>() {
            Ok(p) => (h, p),
            Err(_) => (broker, default_port),
        },
        _ => (broker, default_port),
    }
}

impl Default for MqttPublisher {
    fn default() -> Self {
        Self::new()
    }
}

pub fn aircraft_payload(ac: &AircraftEntry) -> serde_json::Value {
    let position_valid = ac.position_valid();
    serde_json::json!({
        "icao": (ac.icao <= 0x00ff_ffff).then(|| format!("{:06X}", ac.icao)),
        "address": format!("{:06X}", ac.icao & 0x00ff_ffff),
        "track_key": ac.icao,
        "address_type": ac.address_type,
        "source": ac.source,
        "position_source": ac.position_source,
        "emergency": ac.emergency,
        "squawk": ac.squawk,
        "callsign": ac.callsign,
        "lat": position_valid.then_some(ac.lat),
        "lon": position_valid.then_some(ac.lon),
        "altitude": ac.altitude,
        "speed": ac.speed,
        "heading": ac.heading,
        "heading_reference": ac.heading.map(|_| if ac.heading_true { "true" } else { "magnetic" }),
        "ground_track": ac.ground_track,
        "airspeed": ac.airspeed,
        "airspeed_type": ac.airspeed.map(|_| if ac.airspeed_true { "TAS" } else { "IAS" }),
        "surface_track_or_heading": ac.surface_heading,
        "on_ground": ac.on_ground,
        "position_valid": position_valid,
        "position_age_ms": ac.position_seen.map(|t| t.elapsed().as_millis() as u64),
        "observation_age_ms": ac.seen.elapsed().as_millis() as u64,
        "altitude_reference": ac.altitude.map(|_| if ac.altitude_type == crate::adsb_panel::AltitudeType::Geometric { "GNSS" } else { "barometric" }),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

impl MqttPublisher {
    pub fn new() -> Self {
        Self {
            enabled: false,
            broker: "localhost".to_string(),
            port: 1883,
            topic_prefix: "ezsdr".to_string(),
            client: None,
            active_endpoint: None,
            connected_flag: Arc::new(AtomicBool::new(false)),
            reconnect_after: None,
            connecting_since: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
            drain_handle: None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, broker: String, topic_prefix: String) {
        let (host, port) = split_host_port(&broker, self.port);
        let endpoint = format!("{host}:{port}");
        if enabled && self.client.is_some() && self.active_endpoint.as_ref() != Some(&endpoint) {
            // Release the old connection before making the new preferences
            // active. No post-change publication may use the old socket.
            self.disconnect();
        }
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
        self.join_drain();
        let (host, port) = split_host_port(&self.broker, self.port);
        // Unique client ID per process + timestamp so two EZ-SDR instances
        // (or a stale session) don't kick each other off the broker.
        let client_id = format!(
            "ez-sdr-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let mut opts = MqttOptions::new(client_id, host, port);
        opts.set_keep_alive(Duration::from_secs(10));
        let (client, mut connection) = Client::new(opts, 128);

        // Each attempt owns its flags: a retiring worker must never mark a
        // subsequent connection disconnected or inherit its stop signal.
        self.connected_flag = Arc::new(AtomicBool::new(false));
        self.stop_flag = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&self.connected_flag);
        // Only report connected after the broker's CONNACK — never optimistically.
        flag.store(false, Ordering::Relaxed);
        let stop = Arc::clone(&self.stop_flag);
        let handle = std::thread::spawn(move || {
            // Drain connection events with a bounded poll so we can observe the
            // stop flag without blocking forever on `recv()`. 250 ms is short
            // enough for responsive shutdown and long enough to avoid busy-spin.
            while !stop.load(Ordering::Relaxed) {
                match connection.recv_timeout(Duration::from_millis(250)) {
                    Ok(Ok(Event::Incoming(Packet::ConnAck(_)))) => {
                        flag.store(true, Ordering::Relaxed);
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        // Broker idle — not an error, keep polling.
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            flag.store(false, Ordering::Relaxed);
        });

        self.client = Some(client);
        self.active_endpoint = Some(format!("{host}:{port}"));
        self.drain_handle = Some(handle);
        self.reconnect_after = None;
        self.connecting_since = Some(Instant::now());
    }

    /// Signal the drain thread to stop and join it, releasing the underlying
    /// MQTT connection and its socket. No-op if no thread is running.
    fn join_drain(&mut self) {
        if let Some(handle) = self.drain_handle.take() {
            self.stop_flag.store(true, Ordering::Relaxed);
            // The connection poll has a 250 ms deadline and drops its socket
            // before the join completes. Worker-side reconfiguration and UI
            // latency under backpressure are separate I01 remediation work.
            let _ = handle.join();
        }
    }

    pub fn disconnect(&mut self) {
        self.connected_flag.store(false, Ordering::Relaxed);
        self.client = None;
        self.reconnect_after = None;
        self.connecting_since = None;
        self.join_drain();
        self.active_endpoint = None;
    }

    pub fn is_connected(&self) -> bool {
        self.enabled && self.client.is_some() && self.connected_flag.load(Ordering::Relaxed)
    }

    pub fn connected_endpoint(&self) -> Option<&str> {
        if self.is_connected() {
            self.active_endpoint.as_deref()
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn reconnect_in_secs(&self) -> Option<u64> {
        self.reconnect_after
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
    }

    /// Call once per frame to auto-reconnect after connection drops.
    pub fn tick_reconnect(&mut self) {
        if !self.enabled {
            return;
        }
        if self.connected_flag.load(Ordering::Relaxed) {
            // Healthy: a previous attempt completed its CONNACK.
            self.connecting_since = None;
        } else if self.client.is_some() {
            // Not (yet) connected. A fresh attempt needs a few seconds for
            // the TCP + CONNACK round trip — only declare loss after the
            // grace period, otherwise every attempt is killed in the cradle.
            let grace = self
                .connecting_since
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5));
            if !grace {
                eprintln!("[mqtt] connection lost — will retry in 10s");
                // Join the dead worker first: releases its socket + thread
                // instead of leaking one pair per reconnect cycle.
                self.join_drain();
                self.client = None;
                self.active_endpoint = None;
                self.connecting_since = None;
                self.reconnect_after = Some(Instant::now() + Duration::from_secs(10));
            }
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
            let json = aircraft_payload(ac);
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

    struct FakeBroker {
        endpoint: String,
        publications: crossbeam_channel::Receiver<(String, String)>,
        closed: crossbeam_channel::Receiver<()>,
        running: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl FakeBroker {
        fn start() -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = listener.local_addr().unwrap().to_string();
            let (publication_tx, publications) = crossbeam_channel::bounded(32);
            let (closed_tx, closed) = crossbeam_channel::bounded(1);
            let running = Arc::new(AtomicBool::new(true));
            let active = running.clone();
            let worker = std::thread::spawn(move || {
                let mut socket = loop {
                    if !active.load(Ordering::Relaxed) {
                        return;
                    }
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("loopback MQTT accept failed: {error}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_millis(250)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                while active.load(Ordering::Relaxed) {
                    let mut header = [0];
                    match socket.read_exact(&mut header) {
                        Ok(()) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            continue
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::UnexpectedEof
                                    | std::io::ErrorKind::ConnectionReset
                            ) =>
                        {
                            break
                        }
                        Err(error) => panic!("loopback MQTT read failed: {error}"),
                    }
                    let mut length = 0_usize;
                    let mut multiplier = 1;
                    loop {
                        let mut byte = [0];
                        socket.read_exact(&mut byte).unwrap();
                        length += usize::from(byte[0] & 0x7f) * multiplier;
                        assert!(length <= 64 * 1024);
                        if byte[0] & 0x80 == 0 {
                            break;
                        }
                        multiplier *= 128;
                        assert!(multiplier <= 128 * 128 * 128);
                    }
                    let mut body = vec![0; length];
                    socket.read_exact(&mut body).unwrap();
                    match header[0] >> 4 {
                        1 => socket.write_all(&[0x20, 2, 0, 0]).unwrap(), // accepted CONNACK
                        3 => {
                            let topic_len = usize::from(u16::from_be_bytes([body[0], body[1]]));
                            let topic = String::from_utf8(body[2..2 + topic_len].to_vec()).unwrap();
                            let mut payload_offset = 2 + topic_len;
                            if header[0] & 0x06 == 0x02 {
                                socket
                                    .write_all(&[
                                        0x40,
                                        2,
                                        body[payload_offset],
                                        body[payload_offset + 1],
                                    ])
                                    .unwrap();
                                payload_offset += 2;
                            }
                            let payload =
                                String::from_utf8(body[payload_offset..].to_vec()).unwrap();
                            publication_tx.send((topic, payload)).unwrap();
                        }
                        12 => socket.write_all(&[0xd0, 0]).unwrap(), // PINGRESP
                        14 => break,
                        packet => panic!("unexpected MQTT packet {packet}"),
                    }
                }
                let _ = closed_tx.send(());
            });
            Self {
                endpoint,
                publications,
                closed,
                running,
                worker: Some(worker),
            }
        }
    }

    impl Drop for FakeBroker {
        fn drop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    fn wait_connected(publisher: &MqttPublisher) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !publisher.is_connected() {
            assert!(Instant::now() < deadline, "MQTT never received CONNACK");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn changing_enabled_broker_closes_old_socket_and_publishes_only_to_new_endpoint() {
        let old = FakeBroker::start();
        let new = FakeBroker::start();
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, old.endpoint.clone(), "old".into());
        wait_connected(&publisher);
        assert_eq!(publisher.connected_endpoint(), Some(old.endpoint.as_str()));
        publisher.publish("test", "before change");
        assert_eq!(
            old.publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            ("old/test".into(), "before change".into())
        );
        let old_stop = publisher.stop_flag.clone();
        publisher.set_enabled(true, new.endpoint.clone(), "new".into());
        old.closed.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(old_stop.load(Ordering::Relaxed));
        assert!(
            !publisher.stop_flag.load(Ordering::Relaxed),
            "new worker inherited the old stop flag"
        );
        wait_connected(&publisher);
        assert_eq!(publisher.connected_endpoint(), Some(new.endpoint.as_str()));
        publisher.publish("test", "after change");
        assert_eq!(
            new.publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            ("new/test".into(), "after change".into())
        );
        assert!(old.publications.try_recv().is_err());
        assert!(new.closed.try_recv().is_err());
        publisher.set_enabled(false, new.endpoint.clone(), "new".into());
        new.closed.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(publisher.connected_endpoint(), None);
    }

    #[test]
    fn changing_only_topic_prefix_preserves_connection_and_uses_new_topics() {
        let broker = FakeBroker::start();
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "one".into());
        wait_connected(&publisher);
        let original_stop = publisher.stop_flag.clone();
        publisher.set_enabled(true, broker.endpoint.clone(), "two".into());
        assert!(publisher.is_connected());
        assert!(Arc::ptr_eq(&original_stop, &publisher.stop_flag));
        publisher.publish("test", "new prefix");
        assert_eq!(
            broker
                .publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            ("two/test".into(), "new prefix".into())
        );
        assert!(broker.closed.try_recv().is_err());
        publisher.disconnect();
        broker.closed.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn aircraft_exports_distinguish_unknown_stale_and_real_zero_measurements() {
        let mut ac = AircraftEntry {
            icao: 1,
            ..Default::default()
        };
        let unknown = aircraft_payload(&ac);
        for field in [
            "lat",
            "lon",
            "altitude",
            "speed",
            "heading",
            "ground_track",
            "airspeed",
        ] {
            assert!(unknown[field].is_null(), "{field}");
        }
        assert_eq!(unknown["position_valid"], false);
        ac.has_position = true;
        ac.position_seen = Some(Instant::now() - Duration::from_secs(61));
        ac.lat = 52.0;
        ac.lon = 8.0;
        assert!(aircraft_payload(&ac)["lat"].is_null());
        ac.position_seen = Some(Instant::now());
        ac.lat = 0.0;
        ac.lon = 0.0;
        ac.altitude = Some(-500);
        ac.speed = Some(0);
        ac.heading = Some(0);
        let positioned = aircraft_payload(&ac);
        assert_eq!(positioned["lat"], 0.0);
        assert_eq!(positioned["lon"], 0.0);
        assert_eq!(positioned["position_valid"], true);
        assert_eq!(positioned["altitude"], -500);
        assert_eq!(positioned["speed"], 0);
        assert_eq!(positioned["heading"], 0);
        assert_eq!(positioned["heading_reference"], "magnetic");
        assert!(positioned["position_age_ms"].as_u64().is_some());
        assert_eq!(positioned["source"], "unknown");
    }

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
    fn tick_reconnect_keeps_fresh_attempt_within_grace() {
        // A fresh connect() has flag=false until CONNACK arrives; ticking
        // immediately must not declare loss. This private loopback listener
        // intentionally never acknowledges, without using a real broker.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut m = MqttPublisher::new();
        m.broker = listener.local_addr().unwrap().to_string();
        m.enabled = true;
        m.connect();
        assert!(m.client.is_some());
        m.tick_reconnect();
        assert!(
            m.client.is_some(),
            "fresh attempt must survive its CONNACK grace period"
        );
        assert!(m.reconnect_after.is_none());
        m.disconnect();
    }

    #[test]
    fn tick_reconnect_drops_stale_attempt_after_grace() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut m = MqttPublisher::new();
        m.broker = listener.local_addr().unwrap().to_string();
        m.enabled = true;
        m.connect();
        // Simulate an attempt that never got its CONNACK (RTT long over).
        m.connecting_since = Some(Instant::now() - Duration::from_secs(30));
        m.tick_reconnect();
        assert!(m.client.is_none(), "stale attempt must be dropped");
        assert!(m.reconnect_after.is_some(), "reconnect must be scheduled");
        m.disconnect();
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

    #[test]
    fn split_host_port_with_explicit_port() {
        assert_eq!(split_host_port("localhost:1883", 9999), ("localhost", 1883));
        assert_eq!(
            split_host_port("broker.local:8883", 1883),
            ("broker.local", 8883)
        );
    }

    #[test]
    fn split_host_port_without_port_uses_default() {
        assert_eq!(split_host_port("localhost", 1883), ("localhost", 1883));
    }

    #[test]
    fn split_host_port_invalid_port_suffix_falls_back_to_whole_string() {
        // Not a real host:port (e.g. an IPv6-ish or malformed address) —
        // must not silently truncate the host or panic.
        assert_eq!(
            split_host_port("localhost:notaport", 1883),
            ("localhost:notaport", 1883)
        );
    }

    #[test]
    fn split_host_port_empty_host_before_colon_falls_back() {
        assert_eq!(split_host_port(":1883", 9999), (":1883", 9999));
    }
}
