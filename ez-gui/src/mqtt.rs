use arc_swap::{ArcSwap, ArcSwapOption};
use rumqttc::{AsyncClient, ConnectionError, Event, MqttOptions, Packet, QoS};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::adsb_panel::AircraftEntry;
use crate::tle_engine::PassInfo;

const TELEMETRY_INTERVAL: Duration = Duration::from_secs(5);
const MAX_PENDING_AGE: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const RETRY_DELAY: Duration = Duration::from_secs(10);
const CONNECT_GRACE: Duration = Duration::from_secs(5);
const MAX_AIRCRAFT: usize = 4096;
const MAX_PASSES: usize = 128;
const MAX_EVENTS: usize = 64;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Default)]
pub struct MqttStats {
    pub telemetry_sets: u64,
    pub coalesced: u64,
    pub backpressure: u64,
    pub expired: u64,
    pub dropped_events: u64,
    pub rejected: u64,
    /// Publications handed to MQTT; QoS acknowledgement still controls progress.
    pub submitted: u64,
    pub pending: usize,
}

#[derive(Default)]
struct Counters {
    telemetry_sets: AtomicU64,
    coalesced: AtomicU64,
    backpressure: AtomicU64,
    expired: AtomicU64,
    dropped_events: AtomicU64,
    rejected: AtomicU64,
    submitted: AtomicU64,
    pending: AtomicUsize,
}

#[derive(Clone)]
struct WorkerConfig {
    epoch: u64,
    enabled: bool,
    host: String,
    port: u16,
    prefix: String,
    error: Option<String>,
}

#[derive(Default)]
struct WorkerStatus {
    epoch: u64,
    connected_endpoint: Option<String>,
    retry_at: Option<Instant>,
    error: Option<String>,
}

struct Mailbox {
    config: ArcSwap<WorkerConfig>,
    status: ArcSwap<WorkerStatus>,
    latest: [ArcSwapOption<Update>; 4],
    events_tx: crossbeam_channel::Sender<Update>,
    events_rx: crossbeam_channel::Receiver<Update>,
    wake_tx: crossbeam_channel::Sender<()>,
    wake_rx: crossbeam_channel::Receiver<()>,
    stop: AtomicBool,
    counters: Counters,
}

#[derive(Clone)]
enum Payload {
    State {
        frequency: u64,
        gain: f64,
    },
    Signal {
        frequency: u64,
        signal: f32,
        noise: f32,
        mode: String,
        recording: bool,
    },
    Aircraft(AircraftEntry),
    AircraftSet(Vec<AircraftEntry>),
    Passes(Vec<PassInfo>),
    Scanner {
        frequency: u64,
        strength: f32,
    },
    Raw {
        subtopic: String,
        payload: String,
    },
}

struct Update {
    epoch: u64,
    prefix: String,
    created: Instant,
    payload: Payload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    State,
    Signal,
    Passes,
    Aircraft(u32),
    Event(u64),
}

#[derive(Default)]
struct Pending {
    values: HashMap<Key, Update>,
    order: VecDeque<Key>,
    next_event: u64,
    event_count: usize,
}

impl Pending {
    fn insert(&mut self, key: Key, update: Update, counters: &Counters) {
        if self.values.insert(key, update).is_some() {
            counters.coalesced.fetch_add(1, Ordering::Relaxed);
        } else {
            self.order.push_back(key);
        }
    }

    fn accept(&mut self, update: Update, counters: &Counters) {
        if let Payload::AircraftSet(aircraft) = update.payload {
            let keys: HashSet<_> = aircraft.iter().map(|ac| Key::Aircraft(ac.icao)).collect();
            self.values
                .retain(|key, _| !matches!(key, Key::Aircraft(_)) || keys.contains(key));
            self.order.retain(|key| self.values.contains_key(key));
            for ac in aircraft {
                self.insert(
                    Key::Aircraft(ac.icao),
                    Update {
                        epoch: update.epoch,
                        prefix: update.prefix.clone(),
                        created: update.created,
                        payload: Payload::Aircraft(ac),
                    },
                    counters,
                );
            }
            return;
        }
        let key = match update.payload {
            Payload::State { .. } => Key::State,
            Payload::Signal { .. } => Key::Signal,
            Payload::Passes(_) => Key::Passes,
            _ => {
                if self.event_count >= MAX_EVENTS {
                    counters.dropped_events.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                self.next_event = self.next_event.wrapping_add(1);
                self.event_count += 1;
                Key::Event(self.next_event)
            }
        };
        self.insert(key, update, counters);
    }

    fn remove(&mut self, key: Key) {
        self.values.remove(&key);
        self.order.pop_front();
        if matches!(key, Key::Event(_)) {
            self.event_count -= 1;
        }
    }

    fn expire(&mut self, now: Instant, epoch: u64, counters: &Counters) {
        self.values.retain(|_, value| {
            let keep = value.epoch == epoch
                && now.saturating_duration_since(value.created) <= MAX_PENDING_AGE;
            if !keep {
                counters.expired.fetch_add(1, Ordering::Relaxed);
            }
            keep
        });
        self.order.retain(|key| self.values.contains_key(key));
        self.event_count = self
            .values
            .keys()
            .filter(|key| matches!(key, Key::Event(_)))
            .count();
    }
}

struct Transport {
    client: AsyncClient,
    events: tokio::sync::mpsc::Receiver<Result<Event, ConnectionError>>,
    network: tokio::task::JoinHandle<()>,
    started: Instant,
    connected: bool,
    outstanding_since: Option<Instant>,
}

/// UI facade: publication and configuration never wait for a socket, worker or
/// broker queue. One worker owns serialization, MQTT and reconnect deadlines.
pub struct MqttPublisher {
    pub enabled: bool,
    pub broker: String,
    pub port: u16,
    pub topic_prefix: String,
    mailbox: Arc<Mailbox>,
    worker: Option<JoinHandle<()>>,
    next_telemetry: Option<Instant>,
}

pub struct TelemetryView<'a> {
    pub frequency_hz: u64,
    pub gain_db: f64,
    pub signal_db: f32,
    pub noise_db: f32,
    pub mode: &'a str,
    pub recording: bool,
    pub passes: &'a [PassInfo],
    pub aircraft: &'a [AircraftEntry],
}

impl Default for MqttPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttPublisher {
    pub fn new() -> Self {
        let (events_tx, events_rx) = crossbeam_channel::bounded(MAX_EVENTS);
        let (wake_tx, wake_rx) = crossbeam_channel::bounded(1);
        Self {
            enabled: false,
            broker: "localhost".into(),
            port: 1883,
            topic_prefix: "ezsdr".into(),
            mailbox: Arc::new(Mailbox {
                config: ArcSwap::from_pointee(WorkerConfig {
                    epoch: 0,
                    enabled: false,
                    host: "localhost".into(),
                    port: 1883,
                    prefix: "ezsdr".into(),
                    error: None,
                }),
                status: ArcSwap::from_pointee(WorkerStatus::default()),
                latest: std::array::from_fn(|_| ArcSwapOption::empty()),
                events_tx,
                events_rx,
                wake_tx,
                wake_rx,
                stop: AtomicBool::new(false),
                counters: Counters::default(),
            }),
            worker: None,
            next_telemetry: None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, broker: String, topic_prefix: String) {
        let was_enabled = self.enabled;
        self.enabled = enabled;
        self.broker = broker;
        self.topic_prefix = topic_prefix;
        if !enabled || !was_enabled {
            self.next_telemetry = None;
        }
        self.configure();
    }

    fn configure(&mut self) {
        let (host, port) = split_host_port(&self.broker, self.port);
        let problem = if host.is_empty() || host.len() > 253 || port == 0 {
            Some("MQTT requires a valid broker host and port".into())
        } else if self.topic_prefix.is_empty()
            || self.topic_prefix.len() > 256
            || self.topic_prefix.contains(['+', '#', '\0'])
        {
            Some("MQTT topic prefix must be 1–256 bytes without +, # or NUL".into())
        } else {
            None
        };
        let previous = self.mailbox.config.load_full();
        let connection_changed =
            previous.enabled != self.enabled || previous.host != host || previous.port != port;
        if connection_changed || previous.prefix != self.topic_prefix || previous.error != problem {
            self.mailbox.config.store(Arc::new(WorkerConfig {
                epoch: previous.epoch + u64::from(connection_changed),
                enabled: self.enabled,
                host: host.chars().take(253).collect(),
                port,
                prefix: self.topic_prefix.chars().take(256).collect(),
                error: problem,
            }));
        }
        if self.enabled && self.worker.is_none() {
            let mailbox = self.mailbox.clone();
            self.worker = Some(std::thread::spawn(move || run_worker(mailbox)));
        }
        let _ = self.mailbox.wake_tx.try_send(());
    }

    pub fn connect(&mut self) {
        self.configure();
    }
    pub fn disconnect(&mut self) {
        self.enabled = false;
        self.next_telemetry = None;
        self.configure();
    }
    /// The worker reconnects independently, including while the GUI is hidden.
    pub fn tick_reconnect(&mut self) {}

    pub fn is_connected(&self) -> bool {
        let status = self.mailbox.status.load();
        self.enabled
            && status.epoch == self.mailbox.config.load().epoch
            && status.connected_endpoint.is_some()
    }

    pub fn connected_endpoint(&self) -> Option<String> {
        let status = self.mailbox.status.load();
        (self.enabled && status.epoch == self.mailbox.config.load().epoch)
            .then(|| status.connected_endpoint.clone())
            .flatten()
    }

    pub fn reconnect_in_secs(&self) -> Option<u64> {
        let status = self.mailbox.status.load();
        (self.enabled && status.epoch == self.mailbox.config.load().epoch)
            .then_some(status.retry_at)
            .flatten()
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
    }

    pub fn last_error(&self) -> Option<String> {
        self.mailbox
            .config
            .load()
            .error
            .clone()
            .or_else(|| self.mailbox.status.load().error.clone())
    }

    pub fn stats(&self) -> MqttStats {
        let c = &self.mailbox.counters;
        MqttStats {
            telemetry_sets: c.telemetry_sets.load(Ordering::Relaxed),
            coalesced: c.coalesced.load(Ordering::Relaxed),
            backpressure: c.backpressure.load(Ordering::Relaxed),
            expired: c.expired.load(Ordering::Relaxed),
            dropped_events: c.dropped_events.load(Ordering::Relaxed),
            rejected: c.rejected.load(Ordering::Relaxed),
            submitted: c.submitted.load(Ordering::Relaxed),
            pending: c.pending.load(Ordering::Relaxed),
        }
    }

    fn update(&self, payload: Payload) -> Update {
        let config = self.mailbox.config.load();
        Update {
            epoch: config.epoch,
            prefix: config.prefix.clone(),
            created: Instant::now(),
            payload,
        }
    }

    fn latest(&self, slot: usize, payload: Payload) {
        if !self.enabled || self.mailbox.config.load().error.is_some() {
            return;
        }
        if self.mailbox.latest[slot]
            .swap(Some(Arc::new(self.update(payload))))
            .is_some()
        {
            self.mailbox
                .counters
                .coalesced
                .fetch_add(1, Ordering::Relaxed);
        }
        let _ = self.mailbox.wake_tx.try_send(());
    }

    fn event(&self, payload: Payload) {
        if !self.enabled || self.mailbox.config.load().error.is_some() {
            return;
        }
        if self
            .mailbox
            .events_tx
            .try_send(self.update(payload))
            .is_err()
        {
            self.mailbox
                .counters
                .dropped_events
                .fetch_add(1, Ordering::Relaxed);
        }
        let _ = self.mailbox.wake_tx.try_send(());
    }

    pub fn publish(&mut self, subtopic: &str, payload: &str) {
        if !self.enabled {
            return;
        }
        if payload.len() > MAX_PAYLOAD_BYTES || subtopic.len() > 256 {
            self.mailbox
                .counters
                .rejected
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.event(Payload::Raw {
            subtopic: subtopic.into(),
            payload: payload.into(),
        });
    }

    pub fn tick(&mut self, frequency: u64, gain: f64) {
        self.latest(0, Payload::State { frequency, gain });
    }
    pub fn publish_signal(
        &mut self,
        frequency: u64,
        signal: f32,
        noise: f32,
        mode: &str,
        recording: bool,
    ) {
        if !self.enabled {
            return;
        }
        if mode.len() > 256 {
            self.mailbox
                .counters
                .rejected
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.latest(
            1,
            Payload::Signal {
                frequency,
                signal,
                noise,
                mode: mode.into(),
                recording,
            },
        );
    }
    pub fn publish_passes(&mut self, passes: &[PassInfo]) {
        if !self.enabled {
            return;
        }
        if passes.len() > MAX_PASSES {
            self.mailbox
                .counters
                .rejected
                .fetch_add((passes.len() - MAX_PASSES) as u64, Ordering::Relaxed);
        }
        self.latest(
            2,
            Payload::Passes(passes.iter().take(MAX_PASSES).cloned().collect()),
        );
    }
    pub fn publish_aircraft(&mut self, aircraft: &[AircraftEntry]) {
        if !self.enabled {
            return;
        }
        if aircraft.len() > MAX_AIRCRAFT {
            self.mailbox
                .counters
                .rejected
                .fetch_add((aircraft.len() - MAX_AIRCRAFT) as u64, Ordering::Relaxed);
        }
        self.latest(
            3,
            Payload::AircraftSet(aircraft.iter().take(MAX_AIRCRAFT).cloned().collect()),
        );
    }
    pub fn publish_scanner_hit(&mut self, frequency: u64, strength: f32) {
        self.event(Payload::Scanner {
            frequency,
            strength,
        });
    }

    /// At most one set per dedicated deadline, without catch-up bursts. The
    /// caller provides its monotonic frame clock; receiver scheduling is unrelated.
    pub fn publish_telemetry_if_due(&mut self, now: Instant, view: TelemetryView<'_>) -> bool {
        if !self.enabled || self.next_telemetry.is_some_and(|deadline| now < deadline) {
            return false;
        }
        self.next_telemetry = Some(now + TELEMETRY_INTERVAL);
        self.mailbox
            .counters
            .telemetry_sets
            .fetch_add(1, Ordering::Relaxed);
        self.tick(view.frequency_hz, view.gain_db);
        self.publish_signal(
            view.frequency_hz,
            view.signal_db,
            view.noise_db,
            view.mode,
            view.recording,
        );
        self.publish_passes(view.passes);
        self.publish_aircraft(view.aircraft);
        true
    }
}

impl Drop for MqttPublisher {
    fn drop(&mut self) {
        self.mailbox.stop.store(true, Ordering::Release);
        let _ = self.mailbox.wake_tx.try_send(());
        // The worker owns the socket and wakes immediately for cleanup.
        // Dropping the handle detaches cleanup rather than joining on the UI.
        self.worker.take();
    }
}

fn payload_to_wire(payload: &Payload) -> (&str, String) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let (topic, value) = match payload {
        Payload::State { frequency, gain } => (
            "sdr/state",
            serde_json::json!({"frequency_hz":frequency,"frequency_mhz":*frequency as f64 / 1e6,"gain_db":gain,"timestamp":timestamp}),
        ),
        Payload::Signal {
            frequency,
            signal,
            noise,
            mode,
            recording,
        } => (
            "sdr/signal",
            serde_json::json!({"frequency_hz":frequency,"frequency_mhz":*frequency as f64 / 1e6,"signal_db":signal,"noise_floor_db":noise,"snr_db":signal-noise,"demod_mode":mode,"recording":recording,"timestamp":timestamp}),
        ),
        Payload::Passes(passes) => (
            "satellite/passes",
            serde_json::json!({"passes":passes.iter().map(|p| serde_json::json!({"satellite":p.satellite,"aos":p.aos,"los":p.los,"max_elevation":p.max_elevation,"frequency_hz":p.frequency_hz})).collect::<Vec<_>>(),"timestamp":timestamp}),
        ),
        Payload::Aircraft(aircraft) => ("adsb/aircraft", aircraft_payload(aircraft)),
        Payload::Scanner {
            frequency,
            strength,
        } => (
            "scanner/hit",
            serde_json::json!({"frequency_hz":frequency,"frequency_mhz":*frequency as f64 / 1e6,"strength_db":strength,"timestamp":timestamp}),
        ),
        Payload::Raw { subtopic, payload } => return (subtopic, payload.clone()),
        Payload::AircraftSet(_) => unreachable!("aircraft set is expanded before serialization"),
    };
    (topic, value.to_string())
}

// Cancellation completes on the worker, so a replacement connection cannot
// publish while an obsolete endpoint still owns its socket. The GUI never joins.
fn close_transport(transport: &mut Option<Transport>, runtime: &tokio::runtime::Runtime) {
    if let Some(old) = transport.take() {
        old.network.abort();
        let _ = runtime.block_on(old.network);
    }
}

fn run_worker(mailbox: Arc<Mailbox>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(problem) => {
            mailbox.status.store(Arc::new(WorkerStatus {
                error: Some(problem.to_string()),
                ..Default::default()
            }));
            return;
        }
    };
    let mut epoch = None;
    let mut transport: Option<Transport> = None;
    let mut pending = Pending::default();
    let mut retry_at = None;
    let mut error = None;
    while !mailbox.stop.load(Ordering::Acquire) {
        while mailbox.wake_rx.try_recv().is_ok() {}
        for slot in &mailbox.latest {
            if let Some(update) = slot.swap(None) {
                pending.accept(
                    Update {
                        epoch: update.epoch,
                        prefix: update.prefix.clone(),
                        created: update.created,
                        payload: update.payload.clone(),
                    },
                    &mailbox.counters,
                );
            }
        }
        // Limit work per iteration even when a producer continuously sends.
        for update in mailbox.events_rx.try_iter().take(MAX_EVENTS) {
            pending.accept(update, &mailbox.counters);
        }
        // Load configuration AFTER consuming updates: an update stamped with a
        // new generation must not be discarded using an older configuration.
        let config = mailbox.config.load_full();
        if epoch != Some(config.epoch) {
            close_transport(&mut transport, &runtime);
            retry_at = None;
            error = None;
            epoch = Some(config.epoch);
        }
        let now = Instant::now();
        pending.expire(now, config.epoch, &mailbox.counters);
        if !config.enabled || config.error.is_some() {
            close_transport(&mut transport, &runtime);
            pending = Pending::default();
            retry_at = None;
            error = config.error.clone();
        } else if transport.is_none() && retry_at.is_none_or(|deadline| now >= deadline) {
            let id = format!(
                "ez-sdr-{}-{}",
                std::process::id(),
                NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed)
            );
            let mut options = MqttOptions::new(id, config.host.clone(), config.port);
            options.set_keep_alive(Duration::from_secs(10));
            options.set_inflight(1);
            let (client, mut eventloop) = AsyncClient::new(options, 1);
            let (events_tx, events) = tokio::sync::mpsc::channel(8);
            let wake = mailbox.wake_tx.clone();
            let network = runtime.spawn(async move {
                loop {
                    // This future lives until completion: slow DNS/TCP/CONNACK
                    // is never restarted by the mailbox polling interval.
                    let event = eventloop.poll().await;
                    let failed = event.is_err();
                    if events_tx.send(event).await.is_err() {
                        break;
                    }
                    let _ = wake.try_send(());
                    if failed {
                        break;
                    }
                }
            });
            transport = Some(Transport {
                client,
                events,
                network,
                started: now,
                connected: false,
                outstanding_since: None,
            });
            retry_at = None;
        }
        let mut failed = None;
        if let Some(current) = &mut transport {
            while let Ok(event) = current.events.try_recv() {
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        current.connected = true;
                        error = None;
                    }
                    Ok(Event::Incoming(Packet::PubAck(_))) => current.outstanding_since = None,
                    Ok(_) => {}
                    Err(problem) => {
                        failed = Some(problem.to_string());
                        break;
                    }
                }
            }
            if failed.is_none()
                && ((!current.connected
                    && now.saturating_duration_since(current.started) >= CONNECT_GRACE)
                    || current
                        .outstanding_since
                        .is_some_and(|sent| now.saturating_duration_since(sent) > MAX_PENDING_AGE))
            {
                failed = Some("MQTT acknowledgement timed out".into());
                if current.outstanding_since.is_some() {
                    mailbox.counters.expired.fetch_add(1, Ordering::Relaxed);
                }
            }
            if failed.is_none() && current.connected && mailbox.config.load().epoch == config.epoch
            {
                if current.outstanding_since.is_some() {
                    if !pending.values.is_empty() {
                        mailbox
                            .counters
                            .backpressure
                            .fetch_add(1, Ordering::Relaxed);
                    }
                } else if let Some(key) = pending.order.front().copied() {
                    let update = &pending.values[&key];
                    let (subtopic, payload) = payload_to_wire(&update.payload);
                    let topic = format!("{}/{subtopic}", update.prefix);
                    if payload.len() > MAX_PAYLOAD_BYTES
                        || topic.len() > 1024
                        || topic.contains(['+', '#', '\0'])
                    {
                        mailbox.counters.rejected.fetch_add(1, Ordering::Relaxed);
                        pending.remove(key);
                    } else if current
                        .client
                        .try_publish(topic, QoS::AtLeastOnce, false, payload.as_bytes())
                        .is_ok()
                    {
                        current.outstanding_since = Some(now);
                        mailbox.counters.submitted.fetch_add(1, Ordering::Relaxed);
                        pending.remove(key);
                    } else {
                        mailbox
                            .counters
                            .backpressure
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        if let Some(problem) = failed {
            close_transport(&mut transport, &runtime);
            error = Some(problem);
            retry_at = Some(Instant::now() + RETRY_DELAY);
        }
        let connected_endpoint = transport
            .as_ref()
            .filter(|transport| transport.connected)
            .map(|_| format!("{}:{}", config.host, config.port));
        mailbox.status.store(Arc::new(WorkerStatus {
            epoch: config.epoch,
            connected_endpoint,
            retry_at,
            error: error.clone(),
        }));
        mailbox
            .counters
            .pending
            .store(pending.values.len(), Ordering::Relaxed);
        // MQTT events and UI updates both wake this wait; idle connections do
        // not spin, and an unavailable broker cannot block a producer.
        let wait = if !config.enabled || config.error.is_some() {
            Duration::from_millis(250)
        } else {
            POLL_INTERVAL
        };
        let _ = mailbox.wake_rx.recv_timeout(wait);
    }
    close_transport(&mut transport, &runtime);
    mailbox.status.store(Arc::new(WorkerStatus::default()));
    mailbox.counters.pending.store(0, Ordering::Relaxed);
    runtime.shutdown_background();
}

fn split_host_port(broker: &str, default_port: u16) -> (&str, u16) {
    match broker.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => match p.parse::<u16>() {
            Ok(p) => (h, p),
            Err(_) => (broker, default_port),
        },
        _ => (broker, default_port),
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

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeBroker {
        endpoint: String,
        publications: crossbeam_channel::Receiver<(String, String)>,
        closed: crossbeam_channel::Receiver<()>,
        running: Arc<AtomicBool>,
        ack: Arc<AtomicBool>,
        connack: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl FakeBroker {
        fn start() -> Self {
            Self::with_options(true, true)
        }
        fn with_options(acknowledge: bool, accept: bool) -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = listener.local_addr().unwrap().to_string();
            let (publication_tx, publications) = crossbeam_channel::bounded(8192);
            let (closed_tx, closed) = crossbeam_channel::bounded(1);
            let running = Arc::new(AtomicBool::new(true));
            let active = running.clone();
            let ack = Arc::new(AtomicBool::new(acknowledge));
            let broker_ack = ack.clone();
            let connack = Arc::new(AtomicBool::new(accept));
            let broker_connack = connack.clone();
            let worker = std::thread::spawn(move || {
                let mut socket = loop {
                    if !active.load(Ordering::Relaxed) {
                        return;
                    }
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("loopback MQTT accept failed: {error}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_millis(20)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut pending_ack: Vec<[u8; 2]> = Vec::new();
                let mut awaiting_connack = false;
                while active.load(Ordering::Relaxed) {
                    if awaiting_connack && broker_connack.load(Ordering::Acquire) {
                        if socket.write_all(&[0x20, 2, 0, 0]).is_err() {
                            break;
                        }
                        awaiting_connack = false;
                    }
                    if broker_ack.load(Ordering::Acquire) {
                        for id in pending_ack.drain(..) {
                            if socket.write_all(&[0x40, 2, id[0], id[1]]).is_err() {
                                return;
                            }
                        }
                    }
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
                        assert!(length <= MAX_PAYLOAD_BYTES + 2048);
                        if byte[0] & 0x80 == 0 {
                            break;
                        }
                        multiplier *= 128;
                        assert!(multiplier <= 128 * 128 * 128);
                    }
                    let mut body = vec![0; length];
                    socket.read_exact(&mut body).unwrap();
                    match header[0] >> 4 {
                        1 => awaiting_connack = true,
                        3 => {
                            let topic_len = usize::from(u16::from_be_bytes([body[0], body[1]]));
                            let topic = String::from_utf8(body[2..2 + topic_len].to_vec()).unwrap();
                            let mut payload_offset = 2 + topic_len;
                            if header[0] & 0x06 == 0x02 {
                                let id = [body[payload_offset], body[payload_offset + 1]];
                                if broker_ack.load(Ordering::Acquire) {
                                    socket.write_all(&[0x40, 2, id[0], id[1]]).unwrap();
                                } else {
                                    pending_ack.push(id);
                                }
                                payload_offset += 2;
                            }
                            let payload =
                                String::from_utf8(body[payload_offset..].to_vec()).unwrap();
                            publication_tx.try_send((topic, payload)).unwrap();
                        }
                        12 => socket.write_all(&[0xd0, 0]).unwrap(),
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
                ack,
                connack,
                worker: Some(worker),
            }
        }
    }

    fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !condition() {
            assert!(Instant::now() < deadline, "condition never became true");
            std::thread::sleep(Duration::from_millis(5));
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
        let old = FakeBroker::with_options(false, true);
        let new = FakeBroker::start();
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, old.endpoint.clone(), "old".into());
        wait_connected(&publisher);
        assert_eq!(publisher.connected_endpoint(), Some(old.endpoint.clone()));
        publisher.publish("test", "before change");
        assert_eq!(
            old.publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            ("old/test".into(), "before change".into())
        );
        publisher.tick(111, 1.0);
        let started = Instant::now();
        publisher.set_enabled(true, new.endpoint.clone(), "new".into());
        assert!(started.elapsed() < Duration::from_millis(50));
        assert!(!publisher.is_connected());
        publisher.publish("test", "after change");
        old.closed.recv_timeout(Duration::from_secs(3)).unwrap();
        wait_connected(&publisher);
        assert_eq!(publisher.connected_endpoint(), Some(new.endpoint.clone()));
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
        let original_worker = publisher.worker.as_ref().unwrap().thread().id();
        publisher.set_enabled(true, broker.endpoint.clone(), "two".into());
        assert!(publisher.is_connected());
        assert_eq!(
            original_worker,
            publisher.worker.as_ref().unwrap().thread().id()
        );
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
        assert!(mqtt.worker.is_none());
    }

    #[test]
    fn mqtt_disconnect_no_client() {
        let mut mqtt = MqttPublisher::new();
        mqtt.disconnect();
        assert!(mqtt.worker.is_none());
        assert!(!mqtt.is_connected());
    }

    #[test]
    fn mqtt_tick_reconnect_noop_when_disabled() {
        let mut mqtt = MqttPublisher::new();
        mqtt.enabled = false;
        mqtt.tick_reconnect();
        assert!(mqtt.worker.is_none());
    }

    #[test]
    fn telemetry_has_one_complete_wire_set_per_five_seconds_at_60_hz() {
        let broker = FakeBroker::start();
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "cadence".into());
        wait_connected(&publisher);
        let start = Instant::now();
        let aircraft = [AircraftEntry {
            icao: 0x0100_0123,
            callsign: "NONICAO".into(),
            ..Default::default()
        }];
        let mut due = Vec::new();
        for frame in 0..900_u64 {
            let now = start + Duration::from_secs_f64(frame as f64 / 60.0);
            if publisher.publish_telemetry_if_due(
                now,
                TelemetryView {
                    frequency_hz: 100_000_000 + frame,
                    gain_db: 40.0,
                    signal_db: -50.0,
                    noise_db: -90.0,
                    mode: "WFM",
                    recording: false,
                    passes: &[],
                    aircraft: &aircraft,
                },
            ) {
                due.push(frame);
                let mut topics = HashSet::new();
                for _ in 0..4 {
                    let (topic, payload) = broker
                        .publications
                        .recv_timeout(Duration::from_secs(3))
                        .unwrap();
                    let json: serde_json::Value = serde_json::from_str(&payload).unwrap();
                    match topic.as_str() {
                        "cadence/sdr/state" => {
                            assert_eq!(json["frequency_hz"], 100_000_000 + frame);
                            assert_eq!(json["gain_db"], 40.0);
                        }
                        "cadence/sdr/signal" => {
                            assert_eq!(json["snr_db"], 40.0);
                            assert_eq!(json["demod_mode"], "WFM");
                            assert_eq!(json["recording"], false);
                        }
                        "cadence/satellite/passes" => {
                            assert_eq!(json["passes"], serde_json::json!([]))
                        }
                        "cadence/adsb/aircraft" => {
                            assert_eq!(json["track_key"], aircraft[0].icao);
                            assert!(json["icao"].is_null());
                        }
                        other => panic!("unexpected topic {other}"),
                    }
                    assert!(topics.insert(topic), "duplicate topic in one set");
                }
            }
        }
        assert_eq!(due, [0, 300, 600]);
        assert_eq!(publisher.stats().telemetry_sets, 3);
        assert!(broker
            .publications
            .recv_timeout(Duration::from_millis(50))
            .is_err());
    }

    #[test]
    fn stalled_broker_bounds_publication_latency_and_coalesces_latest_telemetry() {
        let broker = FakeBroker::with_options(false, true);
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "pressure".into());
        wait_connected(&publisher);
        publisher.publish("test", "unacknowledged");
        assert_eq!(
            broker
                .publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .1,
            "unacknowledged"
        );
        let mut aircraft: Vec<_> = (0..256)
            .map(|id| AircraftEntry {
                icao: 0x0100_0000 + id,
                callsign: "INITIAL".into(),
                ..Default::default()
            })
            .collect();
        let mut latency = Vec::new();
        for frame in 0..2000 {
            let started = Instant::now();
            publisher.tick(frame, 40.0);
            publisher.publish_signal(frame, -50.0, -90.0, "WFM", false);
            publisher.publish_passes(&[]);
            publisher.publish_aircraft(&aircraft);
            publisher.publish_scanner_hit(frame, -45.0);
            latency.push(started.elapsed());
        }
        for ac in &mut aircraft {
            ac.callsign = "LATEST".into();
        }
        publisher.tick(999_999, 32.8);
        publisher.publish_signal(999_999, -30.0, -90.0, "AM", true);
        publisher.publish_aircraft(&aircraft);
        wait_until(Duration::from_secs(3), || {
            publisher
                .mailbox
                .latest
                .iter()
                .all(|slot| slot.load().is_none())
                && publisher.stats().pending >= aircraft.len() + 3
        });
        let stats = publisher.stats();
        assert!(stats.pending <= MAX_AIRCRAFT + 3 + MAX_EVENTS);
        assert!(stats.coalesced > 0);
        assert!(stats.backpressure > 0);
        assert!(stats.dropped_events > 0);
        assert!(
            broker.publications.try_recv().is_err(),
            "broker received another publication without acknowledging the first"
        );
        latency.sort_unstable();
        println!("MQTT saturated GUI publication latency: p50={:?}, p95={:?}, p99={:?}, max={:?}; pending={}, coalesced={}, backpressure={}, dropped={}", latency[1000], latency[1900], latency[1980], latency.last().unwrap(), stats.pending, stats.coalesced, stats.backpressure, stats.dropped_events);
        // The full workspace runs CPU-heavy DSP tests concurrently. Keep a
        // percentile bound there and run the strict maximum wall-time gate in
        // isolation with EZ_SDR_MQTT_STRICT_LATENCY=1 and --test-threads=1.
        assert!(latency[1980] < Duration::from_millis(50));
        let maximum = if std::env::var_os("EZ_SDR_MQTT_STRICT_LATENCY").is_some() {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(1)
        };
        assert!(
            *latency.last().unwrap() < maximum,
            "publication waited for broker progress"
        );
        broker.ack.store(true, Ordering::Release);
        let mut seen_aircraft = HashSet::new();
        let mut state = false;
        let mut signal = false;
        let mut passes = false;
        while seen_aircraft.len() < aircraft.len() || !state || !signal || !passes {
            let (topic, payload) = broker
                .publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            let json: serde_json::Value = serde_json::from_str(&payload).unwrap();
            match topic.as_str() {
                "pressure/sdr/state" => {
                    assert!(!state);
                    state = true;
                    assert_eq!(json["frequency_hz"], 999_999);
                    assert_eq!(json["gain_db"], 32.8);
                }
                "pressure/sdr/signal" => {
                    assert!(!signal);
                    signal = true;
                    assert_eq!(json["frequency_hz"], 999_999);
                    assert_eq!(json["demod_mode"], "AM");
                    assert_eq!(json["recording"], true);
                }
                "pressure/satellite/passes" => {
                    assert!(!passes);
                    passes = true;
                }
                "pressure/adsb/aircraft" => {
                    assert_eq!(json["callsign"], "LATEST");
                    assert!(seen_aircraft.insert(json["track_key"].as_u64().unwrap()));
                }
                "pressure/scanner/hit" => assert_eq!(json["strength_db"], -45.0),
                other => panic!("unexpected topic {other}"),
            }
        }
        let started = Instant::now();
        publisher.disconnect();
        assert!(started.elapsed() < Duration::from_millis(50));
        broker.closed.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn worker_expires_stale_updates_and_enforces_aircraft_and_payload_bounds() {
        let broker = FakeBroker::with_options(false, true);
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "bounded".into());
        wait_connected(&publisher);
        publisher.publish("test", "held");
        broker
            .publications
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let oversized: Vec<_> = (0..MAX_AIRCRAFT + 1)
            .map(|id| AircraftEntry {
                icao: id as u32,
                ..Default::default()
            })
            .collect();
        publisher.publish_aircraft(&oversized);
        wait_until(Duration::from_secs(3), || {
            publisher.stats().pending == MAX_AIRCRAFT
        });
        assert_eq!(publisher.stats().rejected, 1);
        // A replacement snapshot removes pending tracks no longer present.
        publisher.publish_aircraft(&[]);
        let mut stale = publisher.update(Payload::State {
            frequency: 1,
            gain: 0.0,
        });
        stale.created -= MAX_PENDING_AGE + Duration::from_millis(1);
        publisher.mailbox.latest[0].store(Some(Arc::new(stale)));
        let mut stale_event = publisher.update(Payload::Scanner {
            frequency: 1,
            strength: -90.0,
        });
        stale_event.created -= MAX_PENDING_AGE + Duration::from_millis(1);
        publisher.mailbox.events_tx.try_send(stale_event).unwrap();
        let _ = publisher.mailbox.wake_tx.try_send(());
        wait_until(Duration::from_secs(3), || {
            publisher.stats().expired >= 2 && publisher.stats().pending == 0
        });
        publisher.publish("oversized", &"x".repeat(MAX_PAYLOAD_BYTES + 1));
        publisher.publish("invalid/+", "bad");
        publisher.publish_signal(123, -40.0, -90.0, "FM", false);
        broker.ack.store(true, Ordering::Release);
        let (topic, payload) = broker
            .publications
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(topic, "bounded/sdr/signal");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&payload).unwrap()["frequency_hz"],
            123
        );
        wait_until(Duration::from_secs(3), || publisher.stats().rejected >= 3);
        assert!(broker
            .publications
            .recv_timeout(Duration::from_millis(50))
            .is_err());
    }

    #[test]
    fn stalled_qos_ack_expires_socket_and_schedules_retry_without_gui_ticks() {
        let broker = FakeBroker::with_options(false, true);
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "expire".into());
        wait_connected(&publisher);
        publisher.publish("test", "in flight");
        broker
            .publications
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        publisher.tick(2, 10.0);
        wait_until(MAX_PENDING_AGE + Duration::from_secs(3), || {
            publisher.reconnect_in_secs().is_some()
        });
        assert!(!publisher.is_connected());
        wait_until(Duration::from_secs(1), || publisher.stats().expired >= 2);
        broker.closed.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn shutdown_and_configuration_do_not_join_a_stalled_broker_on_the_gui() {
        for drop_publisher in [false, true] {
            let broker = FakeBroker::with_options(false, true);
            let mut publisher = MqttPublisher::new();
            publisher.set_enabled(true, broker.endpoint.clone(), "shutdown".into());
            wait_connected(&publisher);
            publisher.publish("test", "held");
            broker
                .publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            for frequency in 0..100 {
                publisher.tick(frequency, 1.0);
            }
            let started = Instant::now();
            if drop_publisher {
                drop(publisher);
            } else {
                publisher.disconnect();
            }
            assert!(started.elapsed() < Duration::from_millis(50));
            broker.closed.recv_timeout(Duration::from_secs(3)).unwrap();
        }
    }

    #[test]
    fn invalid_prefix_reports_error_and_never_connects_or_enqueues() {
        let broker = FakeBroker::start();
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "bad/#".into());
        publisher.tick(1, 1.0);
        assert!(publisher.last_error().unwrap().contains("topic prefix"));
        assert!(!publisher.is_connected());
        assert!(publisher.mailbox.latest[0].load().is_none());
        publisher.set_enabled(true, broker.endpoint.clone(), "valid".into());
        wait_connected(&publisher);
        publisher.tick(2, 1.0);
        assert_eq!(
            broker
                .publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .0,
            "valid/sdr/state"
        );
    }

    #[test]
    fn tick_reconnect_keeps_one_slow_handshake_within_grace() {
        let broker = FakeBroker::with_options(true, false);
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "slow".into());
        let until = Instant::now() + Duration::from_millis(150);
        while Instant::now() < until {
            publisher.tick_reconnect();
            assert!(publisher.reconnect_in_secs().is_none());
            std::thread::sleep(Duration::from_millis(5));
        }
        broker.connack.store(true, Ordering::Release);
        wait_connected(&publisher);
        publisher.publish("test", "slow handshake completed");
        assert_eq!(
            broker
                .publications
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .1,
            "slow handshake completed"
        );
        publisher.disconnect();
        broker.closed.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn worker_drops_unacknowledged_handshake_after_grace_without_gui_ticks() {
        let broker = FakeBroker::with_options(true, false);
        let mut publisher = MqttPublisher::new();
        publisher.set_enabled(true, broker.endpoint.clone(), "timeout".into());
        wait_until(Duration::from_secs(7), || {
            publisher.reconnect_in_secs().is_some()
        });
        assert!(!publisher.is_connected());
        let error = publisher.last_error().unwrap();
        assert!(
            error.contains("timed out") || error.contains("timeout"),
            "{error}"
        );
        broker.closed.recv_timeout(Duration::from_secs(3)).unwrap();
        publisher.disconnect();
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
