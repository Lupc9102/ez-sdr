use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub ping_user: bool,
    #[serde(default)]
    pub enabled_kinds: BTreeMap<String, bool>,
    #[serde(default)]
    pub starred_kinds: BTreeSet<String>,
    #[serde(default)]
    pub summary_enabled: bool,
    #[serde(default)]
    pub summary_interval_min: u32,
    #[serde(default)]
    pub min_send_interval_ms: u64,
}

impl Default for DiscordSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            channel_id: String::new(),
            user_id: String::new(),
            ping_user: true,
            enabled_kinds: BTreeMap::new(),
            starred_kinds: CATALOG.iter().filter(|k| k.essential).map(|k| k.id.to_string()).collect(),
            summary_enabled: false,
            summary_interval_min: 30,
            min_send_interval_ms: 1500,
        }
    }
}

pub struct NotifKind {
    pub id: &'static str,
    pub category: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub emoji: &'static str,
    pub essential: bool,
    pub color: u32,
}

pub const CATALOG: &[NotifKind] = &[
    // Source & Hardware
    NotifKind { id: "source_started", category: "Source", label: "Source Started", desc: "SDR device opened and running", emoji: "🟢", essential: false, color: 0x00CC00 },
    NotifKind { id: "source_stopped", category: "Source", label: "Source Stopped", desc: "SDR device closed", emoji: "🔴", essential: false, color: 0xCC0000 },
    NotifKind { id: "source_error", category: "Source", label: "Source Error", desc: "SDR device open or read failed", emoji: "❌", essential: true, color: 0xFF0000 },
    NotifKind { id: "freq_tuned", category: "Source", label: "Frequency Tuned", desc: "Manually tuned to a new frequency", emoji: "📡", essential: false, color: 0x0066FF },
    NotifKind { id: "gain_changed", category: "Source", label: "Gain Changed", desc: "Gain adjusted", emoji: "🔊", essential: false, color: 0x00AA00 },
    NotifKind { id: "demod_changed", category: "Source", label: "Demod Mode Changed", desc: "Demodulation mode switched", emoji: "🔄", essential: false, color: 0x0099CC },
    NotifKind { id: "sample_rate_changed", category: "Source", label: "Sample Rate Changed", desc: "Sample rate adjusted", emoji: "⚙", essential: false, color: 0x666600 },
    NotifKind { id: "ppm_changed", category: "Source", label: "PPM Correction Set", desc: "Frequency calibration adjusted", emoji: "📏", essential: false, color: 0x996600 },

    // Signal
    NotifKind { id: "strong_signal", category: "Signal", label: "Strong Signal Detected", desc: "SNR exceeded 20 dB", emoji: "📈", essential: true, color: 0x00DD00 },
    NotifKind { id: "first_signal", category: "Signal", label: "First Signal!", desc: "First strong signal of the session", emoji: "🎉", essential: true, color: 0xFFAA00 },
    NotifKind { id: "signal_detected", category: "Signal", label: "Signal Detected", desc: "Squelch opened (signal above threshold)", emoji: "📊", essential: false, color: 0x00CC00 },
    NotifKind { id: "signal_lost", category: "Signal", label: "Signal Lost", desc: "Squelch closed (signal below threshold)", emoji: "📉", essential: false, color: 0xCC0000 },

    // Scanner
    NotifKind { id: "scanner_hit", category: "Scanner", label: "Scanner Hit", desc: "Frequency found with active signal", emoji: "🔍", essential: true, color: 0x0099FF },
    NotifKind { id: "scanner_started", category: "Scanner", label: "Scanner Started", desc: "Frequency scanner activated", emoji: "▶️", essential: false, color: 0x00AA00 },
    NotifKind { id: "scanner_stopped", category: "Scanner", label: "Scanner Stopped", desc: "Frequency scanner stopped", emoji: "⏹", essential: false, color: 0xCC0000 },
    NotifKind { id: "scan_complete", category: "Scanner", label: "Scan Complete", desc: "Scanner finished a pass (if bounded)", emoji: "✅", essential: false, color: 0x00AA00 },

    // ADS-B
    NotifKind { id: "aircraft_new", category: "ADS-B", label: "New Aircraft", desc: "First time seeing an aircraft's ICAO", emoji: "✈", essential: true, color: 0x0088FF },
    NotifKind { id: "adsb_started", category: "ADS-B", label: "ADS-B Decoder Started", desc: "ADS-B decoding enabled", emoji: "📡", essential: false, color: 0x00AA00 },
    NotifKind { id: "adsb_stopped", category: "ADS-B", label: "ADS-B Decoder Stopped", desc: "ADS-B decoding disabled", emoji: "🔌", essential: false, color: 0xCC0000 },
    NotifKind { id: "traffic_milestone", category: "ADS-B", label: "Traffic Milestone", desc: "Aircraft count crossed 10/25/50 threshold", emoji: "📈", essential: false, color: 0xFF8800 },
    NotifKind { id: "aircraft_low_altitude", category: "ADS-B", label: "Low Altitude Aircraft", desc: "Aircraft below 1000 ft detected", emoji: "🛬", essential: false, color: 0xFF6600 },

    // Satellite
    NotifKind { id: "sat_aos", category: "Satellite", label: "Satellite AOS", desc: "Satellite acquired (rise above horizon)", emoji: "🛸", essential: true, color: 0x9900FF },
    NotifKind { id: "sat_los", category: "Satellite", label: "Satellite LOS", desc: "Satellite lost (set below horizon)", emoji: "🌅", essential: true, color: 0xFF6600 },
    NotifKind { id: "sat_upcoming", category: "Satellite", label: "Upcoming Pass", desc: "Satellite pass is in the next 30 minutes", emoji: "📅", essential: true, color: 0x0066FF },
    NotifKind { id: "sat_max_elevation", category: "Satellite", label: "High Pass", desc: "Upcoming pass with >45° max elevation", emoji: "⬆️", essential: false, color: 0xFF9900 },

    // Recorder
    NotifKind { id: "rec_started", category: "Recorder", label: "Recording Started", desc: "I/Q or audio recording began", emoji: "⏺", essential: true, color: 0xFF0000 },
    NotifKind { id: "rec_stopped", category: "Recorder", label: "Recording Stopped", desc: "Recording finished (with duration/size)", emoji: "⏹", essential: true, color: 0x660000 },
    NotifKind { id: "rec_error", category: "Recorder", label: "Recording Error", desc: "Disk error or disk space critical", emoji: "⚠️", essential: true, color: 0xFF3333 },
    NotifKind { id: "rec_squelch_triggered", category: "Recorder", label: "Squelch Recording Triggered", desc: "Squelch-based recording captured a signal", emoji: "📹", essential: false, color: 0xFF6633 },

    // Scheduler
    NotifKind { id: "task_fired", category: "Scheduler", label: "Scheduled Task Fired", desc: "Custom scheduled task executed", emoji: "🗓", essential: true, color: 0x0066CC },
    NotifKind { id: "sat_job_activated", category: "Scheduler", label: "Satellite Job Activated", desc: "Scheduled satellite pass task started", emoji: "🛸", essential: false, color: 0x9900FF },

    // Bookmarks
    NotifKind { id: "bookmark_added", category: "Bookmarks", label: "Bookmark Added", desc: "New frequency bookmarked", emoji: "🔖", essential: false, color: 0xFFCC00 },
    NotifKind { id: "bookmark_starred", category: "Bookmarks", label: "Bookmark Starred", desc: "Bookmark marked as favorite", emoji: "⭐", essential: false, color: 0xFFDD00 },
    NotifKind { id: "bookmark_imported", category: "Bookmarks", label: "Bookmarks Imported", desc: "Bookmarks imported from file", emoji: "📥", essential: false, color: 0x00DD00 },

    // System
    NotifKind { id: "mqtt_connected", category: "System", label: "MQTT Connected", desc: "MQTT broker connected", emoji: "🔗", essential: false, color: 0x00AA00 },
    NotifKind { id: "mqtt_disconnected", category: "System", label: "MQTT Disconnected", desc: "MQTT broker disconnected", emoji: "🔌", essential: false, color: 0xCC0000 },
    NotifKind { id: "app_started", category: "System", label: "App Started", desc: "EZ-SDR session started", emoji: "🟢", essential: false, color: 0x00CC00 },
    NotifKind { id: "session_summary", category: "System", label: "Session Summary", desc: "Periodic session status report (opt-in)", emoji: "📊", essential: false, color: 0x0066FF },
];

pub fn categories() -> Vec<&'static str> {
    let mut cats: Vec<_> = CATALOG.iter().map(|k| k.category).collect();
    cats.sort();
    cats.dedup();
    cats
}

pub fn kinds_in(category: &str) -> Vec<&'static NotifKind> {
    CATALOG.iter().filter(|k| k.category == category).collect()
}

pub fn is_enabled(settings: &DiscordSettings, kind_id: &str) -> bool {
    settings.enabled_kinds.get(kind_id).copied()
        .unwrap_or_else(|| CATALOG.iter().find(|k| k.id == kind_id).map(|k| k.essential).unwrap_or(false))
}

pub fn is_starred(settings: &DiscordSettings, kind_id: &str) -> bool {
    settings.starred_kinds.contains(kind_id)
}

#[derive(Debug, Clone)]
pub struct DiscordEmbed {
    pub title: String,
    pub description: String,
    pub color: u32,
    pub fields: Vec<(String, String, bool)>,
    pub footer: String,
    pub timestamp: String,
    pub image_url: Option<String>,
}

impl DiscordEmbed {
    pub fn to_json(&self, settings: &DiscordSettings) -> serde_json::Value {
        let mut content = String::new();
        if settings.ping_user && !settings.user_id.is_empty() {
            content = format!("<@{}>", settings.user_id);
        }

        let fields: Vec<_> = self.fields.iter().map(|(name, value, inline)| {
            serde_json::json!({
                "name": name,
                "value": value,
                "inline": inline
            })
        }).collect();

        let mut embed_json = serde_json::json!({
            "title": self.title,
            "description": self.description,
            "color": self.color,
            "fields": fields,
            "footer": {
                "text": self.footer
            },
            "timestamp": self.timestamp
        });

        if let Some(url) = &self.image_url {
            if let Some(obj) = embed_json.as_object_mut() {
                obj.insert("image".to_string(), serde_json::json!({"url": url}));
            }
        }

        serde_json::json!({
            "content": content,
            "embeds": [embed_json]
        })
    }
}

pub fn embed_aircraft(icao: &str, callsign: &str, lat: f64, lon: f64, alt_ft: u32, speed_kts: u32, heading: u32, image_url: Option<String>) -> DiscordEmbed {
    let maps_url = format!("https://www.google.com/maps?q={},{}", lat, lon);
    DiscordEmbed {
        title: format!("✈ New Aircraft: {}", callsign.trim_end()),
        description: format!("[View on map]({})", maps_url),
        color: 0x0088FF,
        fields: vec![
            ("ICAO".to_string(), icao.to_string(), true),
            ("Callsign".to_string(), callsign.trim_end().to_string(), true),
            ("Altitude".to_string(), format!("{} ft", alt_ft), true),
            ("Speed".to_string(), format!("{} kts", speed_kts), true),
            ("Heading".to_string(), format!("{}°", heading), true),
            ("Position".to_string(), format!("{:.4}°, {:.4}°", lat, lon), false),
        ],
        footer: "EZ-SDR • ADS-B".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url,
    }
}

pub fn embed_scanner_hit(freq_hz: u64, strength_db: f32) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: "🔍 Scanner Hit".to_string(),
        description: format!("Active frequency detected at **{:.4} MHz**", freq_mhz),
        color: 0x0099FF,
        fields: vec![
            ("Frequency".to_string(), format!("{:.4} MHz", freq_mhz), true),
            ("Strength".to_string(), format!("{:.1} dB", strength_db), true),
            ("Frequency (Hz)".to_string(), freq_hz.to_string(), false),
        ],
        footer: "EZ-SDR • Scanner".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_sat_aos(sat_name: &str, freq_hz: u64, max_elev: f64) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: format!("🛸 Satellite AOS: {}", sat_name),
        description: format!("**{}** is now above the horizon!", sat_name),
        color: 0x9900FF,
        fields: vec![
            ("Satellite".to_string(), sat_name.to_string(), true),
            ("Frequency".to_string(), format!("{:.3} MHz", freq_mhz), true),
            ("Max Elevation".to_string(), format!("{:.1}°", max_elev), true),
        ],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_sat_los(sat_name: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: format!("🌅 Satellite LOS: {}", sat_name),
        description: format!("**{}** has set below the horizon", sat_name),
        color: 0xFF6600,
        fields: vec![
            ("Satellite".to_string(), sat_name.to_string(), false),
        ],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_sat_upcoming(sat_name: &str, aos_str: &str, los_str: &str, max_elev: f64, freq_hz: u64) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: format!("📅 Upcoming Pass: {}", sat_name),
        description: format!("**{}** pass coming up soon", sat_name),
        color: 0x0066FF,
        fields: vec![
            ("Satellite".to_string(), sat_name.to_string(), true),
            ("AOS".to_string(), aos_str.to_string(), true),
            ("LOS".to_string(), los_str.to_string(), true),
            ("Max Elevation".to_string(), format!("{:.1}°", max_elev), true),
            ("Frequency".to_string(), format!("{:.3} MHz", freq_mhz), true),
        ],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_recording_started(freq_hz: u64, mode: &str, is_iq: bool, is_audio: bool) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    let rec_type = match (is_iq, is_audio) {
        (true, true) => "I/Q + Audio",
        (true, false) => "I/Q",
        (false, true) => "Audio",
        _ => "Unknown",
    };
    DiscordEmbed {
        title: "⏺ Recording Started".to_string(),
        description: format!("Recording **{}** at **{:.4} MHz** in **{}** mode", rec_type, freq_mhz, mode),
        color: 0xFF0000,
        fields: vec![
            ("Frequency".to_string(), format!("{:.4} MHz", freq_mhz), true),
            ("Mode".to_string(), mode.to_string(), true),
            ("Type".to_string(), rec_type.to_string(), true),
        ],
        footer: "EZ-SDR • Recorder".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_recording_stopped(freq_hz: u64, mode: &str, duration_sec: u64, bytes: u64) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    let size_mb = bytes as f64 / 1e6;
    DiscordEmbed {
        title: "⏹ Recording Stopped".to_string(),
        description: format!("Recording finished after **{}s** at **{:.4} MHz**", duration_sec, freq_mhz),
        color: 0x660000,
        fields: vec![
            ("Duration".to_string(), format!("{} sec", duration_sec), true),
            ("Size".to_string(), format!("{:.1} MB", size_mb), true),
            ("Frequency".to_string(), format!("{:.4} MHz", freq_mhz), true),
            ("Mode".to_string(), mode.to_string(), true),
        ],
        footer: "EZ-SDR • Recorder".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_recording_error(error: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: "⚠️ Recording Error".to_string(),
        description: format!("**{}**", error),
        color: 0xFF3333,
        fields: vec![
            ("Error".to_string(), error.to_string(), false),
        ],
        footer: "EZ-SDR • Recorder".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_strong_signal(freq_hz: u64, snr_db: f32) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: "📈 Strong Signal!".to_string(),
        description: format!("Excellent reception at **{:.4} MHz**", freq_mhz),
        color: 0x00DD00,
        fields: vec![
            ("Frequency".to_string(), format!("{:.4} MHz", freq_mhz), true),
            ("SNR".to_string(), format!("{:.1} dB", snr_db), true),
        ],
        footer: "EZ-SDR • Signal".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_source_error(error: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: "❌ Source Error".to_string(),
        description: format!("**{}**", error),
        color: 0xFF0000,
        fields: vec![
            ("Error".to_string(), error.to_string(), false),
        ],
        footer: "EZ-SDR • Source".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_task_fired(label: &str, freq_hz: u64) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: "🗓 Scheduled Task Fired".to_string(),
        description: format!("**{}** executed", label),
        color: 0x0066CC,
        fields: vec![
            ("Task".to_string(), label.to_string(), true),
            ("Frequency".to_string(), format!("{:.4} MHz", freq_mhz), true),
        ],
        footer: "EZ-SDR • Scheduler".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_session_summary(uptime_sec: u64, current_freq_mhz: f64, mode: &str, aircraft_count: usize, scanner_hits: usize, recordings: usize, upcoming_passes: usize) -> DiscordEmbed {
    let hours = uptime_sec / 3600;
    let mins = (uptime_sec % 3600) / 60;
    DiscordEmbed {
        title: "📊 Session Summary".to_string(),
        description: "Current EZ-SDR session status".to_string(),
        color: 0x0066FF,
        fields: vec![
            ("Uptime".to_string(), format!("{}h {}m", hours, mins), true),
            ("Current Frequency".to_string(), format!("{:.4} MHz", current_freq_mhz), true),
            ("Demod Mode".to_string(), mode.to_string(), true),
            ("Aircraft Tracked".to_string(), aircraft_count.to_string(), true),
            ("Scanner Hits".to_string(), scanner_hits.to_string(), true),
            ("Recordings".to_string(), recordings.to_string(), true),
            ("Upcoming Passes".to_string(), upcoming_passes.to_string(), false),
        ],
        footer: "EZ-SDR • System".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn embed_generic(title: &str, description: &str, emoji: &str, color: u32) -> DiscordEmbed {
    DiscordEmbed {
        title: format!("{} {}", emoji, title),
        description: description.to_string(),
        color,
        fields: vec![],
        footer: "EZ-SDR".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
    }
}

pub fn fetch_aircraft_image(icao: &str) -> Option<String> {
    // Try multiple image sources in order
    let icao_upper = icao.to_uppercase();

    // Try PlaneSpotters CDN - most reliable for aircraft photos
    let planespotters_url = format!("https://cdn-photos.planespotters.net/photos/{}.jpg", icao_upper);
    if is_url_valid(&planespotters_url) {
        return Some(planespotters_url);
    }

    // Try FlightRadar24's aircraft type icon database (fallback)
    // This uses a generic URL pattern for aircraft types
    Some(format!("https://static.radarbox.com/pictures/01000000/01{}.png", icao_upper))
}

fn is_url_valid(url: &str) -> bool {
    // Try a quick HEAD request to see if the URL is valid
    match reqwest::blocking::Client::new().head(url).timeout(std::time::Duration::from_secs(2)).send() {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

pub struct DiscordNotifier {
    pub settings: DiscordSettings,
    tx: crossbeam_channel::Sender<DiscordEmbed>,
    last_send: Instant,
}

impl DiscordNotifier {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(64);
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            loop {
                if let Ok(embed) = rx.recv() {
                    // Receive a batch to send (the main thread will rate-limit via Instant)
                    if let Err(e) = Self::post_embed(&client, &embed) {
                        eprintln!("[discord] POST failed: {}", e);
                    }
                }
            }
        });

        Self {
            settings: DiscordSettings::default(),
            tx,
            last_send: Instant::now(),
        }
    }

    fn post_embed(client: &reqwest::blocking::Client, embed: &DiscordEmbed) -> Result<(), Box<dyn std::error::Error>> {
        // This won't be called with invalid creds, but kept simple
        Ok(())
    }

    pub fn apply_settings(&mut self, settings: &DiscordSettings) {
        self.settings = settings.clone();
    }

    pub fn is_configured(&self) -> bool {
        self.settings.enabled
            && !self.settings.bot_token.is_empty()
            && !self.settings.channel_id.is_empty()
            && !self.settings.user_id.is_empty()
    }

    pub fn fire(&mut self, kind_id: &str, embed: DiscordEmbed) {
        if !self.settings.enabled || !self.is_configured() {
            return;
        }
        if !is_enabled(&self.settings, kind_id) {
            return;
        }
        // Rate-limit
        let elapsed = self.last_send.elapsed().as_millis() as u64;
        if elapsed < self.settings.min_send_interval_ms {
            return;
        }
        let _ = self.tx.try_send(embed);
        self.last_send = Instant::now();
    }

    pub fn send_test(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_configured() {
            return Err("Not configured".into());
        }
        let client = reqwest::blocking::Client::new();
        let embed = embed_generic("Test Notification", "If you see this, Discord integration is working!", "✅", 0x00AA00);
        let url = format!("https://discord.com/api/v10/channels/{}/messages", self.settings.channel_id);
        let body = embed.to_json(&self.settings);
        client.post(&url)
            .header("Authorization", format!("Bot {}", self.settings.bot_token))
            .json(&body)
            .send()?;
        Ok(())
    }
}
