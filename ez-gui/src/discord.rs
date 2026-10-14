//! Discord notification integration for EZ-SDR.
//!
//! Provides types for configuring a Discord bot, constructing rich embed
//! messages for various SDR events (signals, aircraft, satellites, recordings,
//! etc.), and a [`DiscordNotifier`] that rate-limits and dispatches embeds.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Configuration for the Discord notification bot.
///
/// Controls whether the bot is enabled, which channel/user to post to, which
/// event kinds are enabled, and rate-limiting parameters.
#[derive(Clone, Serialize, Deserialize)]
pub struct DiscordSettings {
    /// Whether the Discord bot is active.
    #[serde(default)]
    pub enabled: bool,
    /// Discord bot token for authentication.
    #[serde(default)]
    pub bot_token: String,
    /// Discord channel ID where notifications are sent.
    #[serde(default)]
    pub channel_id: String,
    /// Discord user ID to ping in notifications.
    #[serde(default)]
    pub user_id: String,
    /// Whether to ping the configured user in each notification.
    #[serde(default)]
    pub ping_user: bool,
    /// Per-kind enablement map (`kind_id` -> enabled).
    #[serde(default)]
    pub enabled_kinds: BTreeMap<String, bool>,
    /// Set of starred (favorite) kind IDs for filtering.
    #[serde(default)]
    pub starred_kinds: BTreeSet<String>,
    /// Whether periodic session summaries are enabled.
    #[serde(default)]
    pub summary_enabled: bool,
    /// Interval (minutes) between automatic summary messages.
    #[serde(default)]
    pub summary_interval_min: u32,
    /// Minimum interval (ms) between successive sends (rate limiting).
    #[serde(default)]
    pub min_send_interval_ms: u64,
}

impl std::fmt::Debug for DiscordSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiscordSettings")
            .field("enabled", &self.enabled)
            .field("bot_token", &"[REDACTED]")
            .field("ping_user", &self.ping_user)
            .field("enabled_kinds", &self.enabled_kinds)
            .field("starred_kinds", &self.starred_kinds)
            .field("summary_enabled", &self.summary_enabled)
            .field("summary_interval_min", &self.summary_interval_min)
            .field("min_send_interval_ms", &self.min_send_interval_ms)
            .finish_non_exhaustive()
    }
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
            starred_kinds: CATALOG
                .iter()
                .filter(|k| k.essential)
                .map(|k| k.id.to_string())
                .collect(),
            summary_enabled: false,
            summary_interval_min: 30,
            min_send_interval_ms: 1500,
        }
    }
}

/// A notification kind descriptor.
///
/// Each variant defines the metadata (id, category, label, color, etc.) for a
/// type of event that can be sent to Discord.
pub struct NotifKind {
    /// Unique identifier string for this notification kind (e.g. "`source_error`").
    pub id: &'static str,
    /// Category grouping (e.g. "Source", "Signal", "ADS-B", "Satellite").
    pub category: &'static str,
    /// Human-readable display label.
    pub label: &'static str,
    /// Short description of the event.
    pub desc: &'static str,
    /// Emoji string for visual identification.
    pub emoji: &'static str,
    /// Whether this kind is essential (enabled by default).
    pub essential: bool,
    /// Discord embed color (hex RGB, e.g. 0x00CC00).
    pub color: u32,
}

/// The master list of all notification kinds supported by the Discord bot.
///
/// Each [`NotifKind`] entry defines the id, category, label, color, and default
/// enablement for one event type. The list is used to drive the settings UI and
/// to look up metadata when dispatching notifications.
pub const CATALOG: &[NotifKind] = &[
    // Source & Hardware
    NotifKind {
        id: "source_started",
        category: "Source",
        label: "Source Started",
        desc: "SDR device opened and running",
        emoji: "🟢",
        essential: false,
        color: 0x00CC00,
    },
    NotifKind {
        id: "source_stopped",
        category: "Source",
        label: "Source Stopped",
        desc: "SDR device closed",
        emoji: "🔴",
        essential: false,
        color: 0xCC0000,
    },
    NotifKind {
        id: "source_error",
        category: "Source",
        label: "Source Error",
        desc: "SDR device open or read failed",
        emoji: "❌",
        essential: true,
        color: 0xFF0000,
    },
    NotifKind {
        id: "freq_tuned",
        category: "Source",
        label: "Frequency Tuned",
        desc: "Manually tuned to a new frequency",
        emoji: "📡",
        essential: false,
        color: 0x0066FF,
    },
    NotifKind {
        id: "gain_changed",
        category: "Source",
        label: "Gain Changed",
        desc: "Gain adjusted",
        emoji: "🔊",
        essential: false,
        color: 0x00AA00,
    },
    NotifKind {
        id: "demod_changed",
        category: "Source",
        label: "Demod Mode Changed",
        desc: "Demodulation mode switched",
        emoji: "🔄",
        essential: false,
        color: 0x0099CC,
    },
    NotifKind {
        id: "sample_rate_changed",
        category: "Source",
        label: "Sample Rate Changed",
        desc: "Sample rate adjusted",
        emoji: "⚙",
        essential: false,
        color: 0x666600,
    },
    NotifKind {
        id: "ppm_changed",
        category: "Source",
        label: "PPM Correction Set",
        desc: "Frequency calibration adjusted",
        emoji: "📏",
        essential: false,
        color: 0x996600,
    },
    // Signal
    NotifKind {
        id: "strong_signal",
        category: "Signal",
        label: "Strong Signal Detected",
        desc: "SNR exceeded 20 dB",
        emoji: "📈",
        essential: true,
        color: 0x00DD00,
    },
    NotifKind {
        id: "first_signal",
        category: "Signal",
        label: "First Signal!",
        desc: "First strong signal of the session",
        emoji: "🎉",
        essential: true,
        color: 0xFFAA00,
    },
    NotifKind {
        id: "signal_detected",
        category: "Signal",
        label: "Signal Detected",
        desc: "Squelch opened (signal above threshold)",
        emoji: "📊",
        essential: false,
        color: 0x00CC00,
    },
    NotifKind {
        id: "signal_lost",
        category: "Signal",
        label: "Signal Lost",
        desc: "Squelch closed (signal below threshold)",
        emoji: "📉",
        essential: false,
        color: 0xCC0000,
    },
    // Scanner
    NotifKind {
        id: "scanner_hit",
        category: "Scanner",
        label: "Scanner Hit",
        desc: "Frequency found with active signal",
        emoji: "🔍",
        essential: true,
        color: 0x0099FF,
    },
    NotifKind {
        id: "scanner_started",
        category: "Scanner",
        label: "Scanner Started",
        desc: "Frequency scanner activated",
        emoji: "▶️",
        essential: false,
        color: 0x00AA00,
    },
    NotifKind {
        id: "scanner_stopped",
        category: "Scanner",
        label: "Scanner Stopped",
        desc: "Frequency scanner stopped",
        emoji: "⏹",
        essential: false,
        color: 0xCC0000,
    },
    NotifKind {
        id: "scan_complete",
        category: "Scanner",
        label: "Scan Complete",
        desc: "Scanner finished a pass (if bounded)",
        emoji: "✅",
        essential: false,
        color: 0x00AA00,
    },
    // ADS-B
    NotifKind {
        id: "aircraft_new",
        category: "ADS-B",
        label: "New Aircraft",
        desc: "First time seeing an aircraft's ICAO",
        emoji: "✈",
        essential: true,
        color: 0x0088FF,
    },
    NotifKind {
        id: "adsb_started",
        category: "ADS-B",
        label: "ADS-B Decoder Started",
        desc: "ADS-B decoding enabled",
        emoji: "📡",
        essential: false,
        color: 0x00AA00,
    },
    NotifKind {
        id: "adsb_stopped",
        category: "ADS-B",
        label: "ADS-B Decoder Stopped",
        desc: "ADS-B decoding disabled",
        emoji: "🔌",
        essential: false,
        color: 0xCC0000,
    },
    NotifKind {
        id: "traffic_milestone",
        category: "ADS-B",
        label: "Traffic Milestone",
        desc: "Aircraft count crossed 10/25/50 threshold",
        emoji: "📈",
        essential: false,
        color: 0xFF8800,
    },
    NotifKind {
        id: "aircraft_low_altitude",
        category: "ADS-B",
        label: "Low Altitude Aircraft",
        desc: "Aircraft below 1000 ft detected",
        emoji: "🛬",
        essential: false,
        color: 0xFF6600,
    },
    // Satellite
    NotifKind {
        id: "sat_aos",
        category: "Satellite",
        label: "Satellite AOS",
        desc: "Satellite acquired (rise above horizon)",
        emoji: "🛸",
        essential: true,
        color: 0x9900FF,
    },
    NotifKind {
        id: "sat_los",
        category: "Satellite",
        label: "Satellite LOS",
        desc: "Satellite lost (set below horizon)",
        emoji: "🌅",
        essential: true,
        color: 0xFF6600,
    },
    NotifKind {
        id: "sat_upcoming",
        category: "Satellite",
        label: "Upcoming Pass",
        desc: "Satellite pass is in the next 30 minutes",
        emoji: "📅",
        essential: true,
        color: 0x0066FF,
    },
    NotifKind {
        id: "sat_max_elevation",
        category: "Satellite",
        label: "High Pass",
        desc: "Upcoming pass with >45° max elevation",
        emoji: "⬆️",
        essential: false,
        color: 0xFF9900,
    },
    // Recorder
    NotifKind {
        id: "rec_started",
        category: "Recorder",
        label: "Recording Started",
        desc: "I/Q or audio recording began",
        emoji: "⏺",
        essential: true,
        color: 0xFF0000,
    },
    NotifKind {
        id: "rec_stopped",
        category: "Recorder",
        label: "Recording Stopped",
        desc: "Recording finished (with duration/size)",
        emoji: "⏹",
        essential: true,
        color: 0x660000,
    },
    NotifKind {
        id: "rec_error",
        category: "Recorder",
        label: "Recording Error",
        desc: "Disk error or disk space critical",
        emoji: "⚠️",
        essential: true,
        color: 0xFF3333,
    },
    NotifKind {
        id: "rec_squelch_triggered",
        category: "Recorder",
        label: "Squelch Recording Triggered",
        desc: "Squelch-based recording captured a signal",
        emoji: "📹",
        essential: false,
        color: 0xFF6633,
    },
    // Scheduler
    NotifKind {
        id: "task_fired",
        category: "Scheduler",
        label: "Scheduled Task Fired",
        desc: "Custom scheduled task executed",
        emoji: "🗓",
        essential: true,
        color: 0x0066CC,
    },
    NotifKind {
        id: "sat_job_activated",
        category: "Scheduler",
        label: "Satellite Job Activated",
        desc: "Scheduled satellite pass task started",
        emoji: "🛸",
        essential: false,
        color: 0x9900FF,
    },
    // Bookmarks
    NotifKind {
        id: "bookmark_added",
        category: "Bookmarks",
        label: "Bookmark Added",
        desc: "New frequency bookmarked",
        emoji: "🔖",
        essential: false,
        color: 0xFFCC00,
    },
    NotifKind {
        id: "bookmark_starred",
        category: "Bookmarks",
        label: "Bookmark Starred",
        desc: "Bookmark marked as favorite",
        emoji: "⭐",
        essential: false,
        color: 0xFFDD00,
    },
    NotifKind {
        id: "bookmark_imported",
        category: "Bookmarks",
        label: "Bookmarks Imported",
        desc: "Bookmarks imported from file",
        emoji: "📥",
        essential: false,
        color: 0x00DD00,
    },
    // System
    NotifKind {
        id: "mqtt_connected",
        category: "System",
        label: "MQTT Connected",
        desc: "MQTT broker connected",
        emoji: "🔗",
        essential: false,
        color: 0x00AA00,
    },
    NotifKind {
        id: "mqtt_disconnected",
        category: "System",
        label: "MQTT Disconnected",
        desc: "MQTT broker disconnected",
        emoji: "🔌",
        essential: false,
        color: 0xCC0000,
    },
    NotifKind {
        id: "app_started",
        category: "System",
        label: "App Started",
        desc: "EZ-SDR session started",
        emoji: "🟢",
        essential: false,
        color: 0x00CC00,
    },
    NotifKind {
        id: "session_summary",
        category: "System",
        label: "Session Summary",
        desc: "Periodic session status report (opt-in)",
        emoji: "📊",
        essential: false,
        color: 0x0066FF,
    },
];

/// Return a sorted, deduplicated list of category names from [`CATALOG`].
pub fn categories() -> Vec<&'static str> {
    let mut cats: Vec<_> = CATALOG.iter().map(|k| k.category).collect();
    cats.sort_unstable();
    cats.dedup();
    cats
}

/// Return all [`NotifKind`] entries belonging to the given category.
pub fn kinds_in(category: &str) -> Vec<&'static NotifKind> {
    CATALOG.iter().filter(|k| k.category == category).collect()
}

/// Check whether a notification kind is enabled in the given settings.
///
/// Returns the user's preference if set, otherwise falls back to the kind's
/// `essential` flag.
pub fn is_enabled(settings: &DiscordSettings, kind_id: &str) -> bool {
    settings
        .enabled_kinds
        .get(kind_id)
        .copied()
        .unwrap_or_else(|| {
            CATALOG
                .iter()
                .find(|k| k.id == kind_id)
                .is_some_and(|k| k.essential)
        })
}

/// Check whether a notification kind is starred (favorited) in the settings.
pub fn is_starred(settings: &DiscordSettings, kind_id: &str) -> bool {
    settings.starred_kinds.contains(kind_id)
}

/// A rich embed message payload for the Discord webhook API.
///
/// Contains the title, description, color, fields, footer, and optional image
/// that together compose a single Discord embed card.
#[derive(Debug, Clone)]
pub struct DiscordEmbed {
    /// Embed title (displayed as bold heading).
    pub title: String,
    /// Embed body text (Markdown supported).
    pub description: String,
    /// Color bar on the left edge of the embed (hex RGB).
    pub color: u32,
    /// Row fields: each tuple is (name, value, inline).
    pub fields: Vec<(String, String, bool)>,
    /// Footer text displayed at the bottom of the embed.
    pub footer: String,
    /// ISO-8601 timestamp string.
    pub timestamp: String,
    /// Optional image URL to embed in the card.
    pub image_url: Option<String>,
    /// Optional file name of a locally-attached image (sent via multipart
    /// upload), referenced in the embed JSON as `attachment://{name}`.
    pub image_attachment_name: Option<String>,
}

impl DiscordEmbed {
    /// Serialize this embed along with an optional user ping into a JSON value
    /// suitable for the Discord webhook API (channels/{id}/messages).
    pub fn to_json(&self, settings: &DiscordSettings) -> serde_json::Value {
        let mut content = String::new();
        if settings.ping_user && !settings.user_id.is_empty() {
            content = format!("<@{}>", settings.user_id);
        }

        let fields: Vec<_> = self
            .fields
            .iter()
            .map(|(name, value, inline)| {
                serde_json::json!({
                    "name": name,
                    "value": value,
                    "inline": inline
                })
            })
            .collect();

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

        if let Some(name) = &self.image_attachment_name {
            if let Some(obj) = embed_json.as_object_mut() {
                obj.insert(
                    "image".to_string(),
                    serde_json::json!({"url": format!("attachment://{name}")}),
                );
            }
        } else if let Some(url) = &self.image_url {
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

/// Data representing an aircraft tracked via ADS-B, used to build notification embeds.
#[derive(Clone, Default)]
pub struct AircraftData {
    /// ICAO 24-bit address (hex string).
    pub icao: String,
    /// Aircraft callsign / flight number.
    pub callsign: String,
    /// Latitude in decimal degrees.
    pub lat: f64,
    /// Longitude in decimal degrees.
    pub lon: f64,
    /// Altitude in feet.
    pub alt_ft: Option<i32>,
    /// Ground speed in knots.
    pub speed_kts: Option<u32>,
    /// Heading in degrees (0–359).
    pub heading: Option<u32>,
    pub heading_true: bool,
    pub ground_track: Option<u32>,
    pub airspeed: Option<u32>,
    pub airspeed_true: bool,
    pub altitude_geometric: bool,
}

/// Retry partial tracks until a current position exists. Deduplication is
/// independent of the app's first-identity event, so a callsign alone cannot
/// consume the position-dependent notification.
pub fn positioned_aircraft_notification(
    ac: &crate::adsb_panel::AircraftEntry,
    notified: &mut std::collections::HashSet<u32>,
) -> Option<AircraftData> {
    if ac.icao > 0x00ff_ffff || !ac.position_valid() || !notified.insert(ac.icao) {
        return None;
    }
    Some(AircraftData {
        icao: format!("{:06X}", ac.icao),
        callsign: ac.callsign.clone(),
        lat: ac.lat,
        lon: ac.lon,
        alt_ft: ac.altitude,
        speed_kts: ac.speed,
        heading: ac.heading,
        heading_true: ac.heading_true,
        ground_track: ac.ground_track,
        airspeed: ac.airspeed,
        airspeed_true: ac.airspeed_true,
        altitude_geometric: ac.altitude_type == crate::adsb_panel::AltitudeType::Geometric,
    })
}

/// Build a Discord embed for a new aircraft detection event.
pub fn embed_aircraft(ac: &AircraftData, image_url: Option<String>) -> DiscordEmbed {
    let maps_url = format!("https://www.google.com/maps?q={},{}", ac.lat, ac.lon);
    DiscordEmbed {
        title: format!("✈ New Aircraft: {}", ac.callsign.trim_end()),
        description: format!("[View on map]({maps_url})"),
        color: 0x0088FF,
        fields: vec![
            ("ICAO".to_string(), ac.icao.clone(), true),
            (
                "Callsign".to_string(),
                ac.callsign.trim_end().to_string(),
                true,
            ),
            (
                "Altitude".to_string(),
                ac.alt_ft.map_or("Unavailable".into(), |v| {
                    format!(
                        "{v} ft {}",
                        if ac.altitude_geometric {
                            "GNSS"
                        } else {
                            "baro"
                        }
                    )
                }),
                true,
            ),
            (
                "Speed".to_string(),
                ac.speed_kts
                    .map_or("Unavailable".into(), |v| format!("{v} kt GS")),
                true,
            ),
            (
                "Ground track".into(),
                ac.ground_track
                    .map_or("Unavailable".into(), |v| format!("{v}° true")),
                true,
            ),
            (
                "Airspeed".into(),
                ac.airspeed.map_or("Unavailable".into(), |v| {
                    format!("{v} kt {}", if ac.airspeed_true { "TAS" } else { "IAS" })
                }),
                true,
            ),
            (
                "Heading".to_string(),
                ac.heading.map_or("Unavailable".into(), |v| {
                    format!("{v}° {}", if ac.heading_true { "true" } else { "magnetic" })
                }),
                true,
            ),
            (
                "Position".to_string(),
                format!("{:.4}°, {:.4}°", ac.lat, ac.lon),
                false,
            ),
        ],
        footer: "EZ-SDR • ADS-B".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a frequency scanner hit event.
pub fn embed_scanner_hit(freq_hz: u64, strength_db: f32) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: "🔍 Scanner Hit".to_string(),
        description: format!("Active frequency detected at **{freq_mhz:.4} MHz**"),
        color: 0x0099FF,
        fields: vec![
            ("Frequency".to_string(), format!("{freq_mhz:.4} MHz"), true),
            ("Strength".to_string(), format!("{strength_db:.1} dB"), true),
            ("Frequency (Hz)".to_string(), freq_hz.to_string(), false),
        ],
        footer: "EZ-SDR • Scanner".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a satellite acquisition-of-signal (AOS) event.
pub fn embed_sat_aos(sat_name: &str, freq_hz: u64, max_elev: f64) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: format!("🛸 Satellite AOS: {sat_name}"),
        description: format!("**{sat_name}** is now above the horizon!"),
        color: 0x9900FF,
        fields: vec![
            ("Satellite".to_string(), sat_name.to_string(), true),
            ("Frequency".to_string(), format!("{freq_mhz:.3} MHz"), true),
            ("Max Elevation".to_string(), format!("{max_elev:.1}°"), true),
        ],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a satellite loss-of-signal (LOS) event.
pub fn embed_sat_los(sat_name: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: format!("🌅 Satellite LOS: {sat_name}"),
        description: format!("**{sat_name}** has set below the horizon"),
        color: 0xFF6600,
        fields: vec![("Satellite".to_string(), sat_name.to_string(), false)],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for an upcoming satellite pass notification.
pub fn embed_sat_upcoming(
    sat_name: &str,
    aos_str: &str,
    los_str: &str,
    max_elev: f64,
    freq_hz: u64,
) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: format!("📅 Upcoming Pass: {sat_name}"),
        description: format!("**{sat_name}** pass coming up soon"),
        color: 0x0066FF,
        fields: vec![
            ("Satellite".to_string(), sat_name.to_string(), true),
            ("AOS".to_string(), aos_str.to_string(), true),
            ("LOS".to_string(), los_str.to_string(), true),
            ("Max Elevation".to_string(), format!("{max_elev:.1}°"), true),
            ("Frequency".to_string(), format!("{freq_mhz:.3} MHz"), true),
        ],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for the start of a Meteor LRPT decode session.
/// Build a Discord embed for a recording-started event.
pub fn embed_recording_started(
    freq_hz: u64,
    mode: &str,
    is_iq: bool,
    is_audio: bool,
) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    let rec_type = match (is_iq, is_audio) {
        (true, true) => "I/Q + Audio",
        (true, false) => "I/Q",
        (false, true) => "Audio",
        _ => "Unknown",
    };
    DiscordEmbed {
        title: "⏺ Recording Started".to_string(),
        description: format!(
            "Recording **{rec_type}** at **{freq_mhz:.4} MHz** in **{mode}** mode"
        ),
        color: 0xFF0000,
        fields: vec![
            ("Frequency".to_string(), format!("{freq_mhz:.4} MHz"), true),
            ("Mode".to_string(), mode.to_string(), true),
            ("Type".to_string(), rec_type.to_string(), true),
        ],
        footer: "EZ-SDR • Recorder".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a recording-stopped event.
pub fn embed_recording_stopped(
    freq_hz: u64,
    mode: &str,
    duration_sec: u64,
    bytes: u64,
) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    let size_mb = bytes as f64 / 1e6;
    DiscordEmbed {
        title: "⏹ Recording Stopped".to_string(),
        description: format!(
            "Recording finished after **{duration_sec}s** at **{freq_mhz:.4} MHz**"
        ),
        color: 0x660000,
        fields: vec![
            ("Duration".to_string(), format!("{duration_sec} sec"), true),
            ("Size".to_string(), format!("{size_mb:.1} MB"), true),
            ("Frequency".to_string(), format!("{freq_mhz:.4} MHz"), true),
            ("Mode".to_string(), mode.to_string(), true),
        ],
        footer: "EZ-SDR • Recorder".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a recording error event.
pub fn embed_recording_error(error: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: "⚠️ Recording Error".to_string(),
        description: format!("**{error}**"),
        color: 0xFF3333,
        fields: vec![("Error".to_string(), error.to_string(), false)],
        footer: "EZ-SDR • Recorder".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a strong signal detection event (SNR > 20 dB).
pub fn embed_strong_signal(freq_hz: u64, snr_db: f32) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: "📈 Strong Signal!".to_string(),
        description: format!("Excellent reception at **{freq_mhz:.4} MHz**"),
        color: 0x00DD00,
        fields: vec![
            ("Frequency".to_string(), format!("{freq_mhz:.4} MHz"), true),
            ("SNR".to_string(), format!("{snr_db:.1} dB"), true),
        ],
        footer: "EZ-SDR • Signal".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for an SDR source error event.
pub fn embed_source_error(error: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: "❌ Source Error".to_string(),
        description: format!("**{error}**"),
        color: 0xFF0000,
        fields: vec![("Error".to_string(), error.to_string(), false)],
        footer: "EZ-SDR • Source".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a scheduled-task-fired event.
pub fn embed_task_fired(label: &str, freq_hz: u64) -> DiscordEmbed {
    let freq_mhz = freq_hz as f64 / 1e6;
    DiscordEmbed {
        title: "🗓 Scheduled Task Fired".to_string(),
        description: format!("**{label}** executed"),
        color: 0x0066CC,
        fields: vec![
            ("Task".to_string(), label.to_string(), true),
            ("Frequency".to_string(), format!("{freq_mhz:.4} MHz"), true),
        ],
        footer: "EZ-SDR • Scheduler".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a periodic session-summary report.
pub fn embed_session_summary(
    uptime_sec: u64,
    current_freq_mhz: f64,
    mode: &str,
    aircraft_count: usize,
    scanner_hits: usize,
    recordings: usize,
    upcoming_passes: usize,
) -> DiscordEmbed {
    let hours = uptime_sec / 3600;
    let mins = (uptime_sec % 3600) / 60;
    DiscordEmbed {
        title: "📊 Session Summary".to_string(),
        description: "Current EZ-SDR session status".to_string(),
        color: 0x0066FF,
        fields: vec![
            ("Uptime".to_string(), format!("{hours}h {mins}m"), true),
            (
                "Current Frequency".to_string(),
                format!("{current_freq_mhz:.4} MHz"),
                true,
            ),
            ("Demod Mode".to_string(), mode.to_string(), true),
            (
                "Aircraft Tracked".to_string(),
                aircraft_count.to_string(),
                true,
            ),
            ("Scanner Hits".to_string(), scanner_hits.to_string(), true),
            ("Recordings".to_string(), recordings.to_string(), true),
            (
                "Upcoming Passes".to_string(),
                upcoming_passes.to_string(),
                false,
            ),
        ],
        footer: "EZ-SDR • System".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a generic Discord embed with an emoji prefix and no additional fields.
pub fn embed_generic(title: &str, description: &str, emoji: &str, color: u32) -> DiscordEmbed {
    DiscordEmbed {
        title: format!("{emoji} {title}"),
        description: description.to_string(),
        color,
        fields: vec![],
        footer: "EZ-SDR".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Attempt to fetch an aircraft photo URL from `PlaneSpotters` or a fallback source.
///
/// Returns `None` if no valid image URL could be resolved within the timeout.
#[must_use]
pub fn fetch_aircraft_image(icao: &str) -> Option<String> {
    // PlaneSpotters photo URLs contain numeric photo IDs and are returned by
    // the public API; constructing a URL from the aircraft ICAO hex is invalid.
    let icao_upper = icao.to_uppercase();
    let endpoint = format!("https://api.planespotters.net/pub/photos/hex/{icao_upper}");
    if let Ok(resp) = ureq::get(&endpoint).call() {
        if let Ok(json) = resp.into_body().read_json::<serde_json::Value>() {
            let photo = &json["photos"][0];
            let image_url = photo["image"]["src"]
                .as_str()
                .or_else(|| photo["thumbnail"]["src"].as_str());
            if let Some(url) = image_url {
                if is_url_valid(url) {
                    return Some(url.to_string());
                }
            }
        }
    }

    // Try FlightRadar24's aircraft type icon database (fallback)
    // This uses a generic URL pattern for aircraft types
    Some(format!(
        "https://static.radarbox.com/pictures/01000000/01{icao_upper}.png"
    ))
}

fn is_url_valid(url: &str) -> bool {
    // Try a quick HEAD request to see if the URL is valid
    match reqwest::blocking::Client::new()
        .head(url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// A notification queued for dispatch by the background thread.
///
/// Carries a snapshot of the [`DiscordSettings`] in effect at the time
/// `fire()`/`fire_with_attachment()` was called, since settings may change
/// after the message is queued but before it is sent.
enum QueuedNotification {
    /// A plain embed with no file attachment.
    Plain(DiscordEmbed, DiscordSettings),
    /// An embed with an accompanying file to upload via multipart/form-data.
    #[cfg_attr(not(test), allow(dead_code))]
    WithAttachment(DiscordEmbed, DiscordSettings, Vec<u8>, String),
    /// New-aircraft notification: the blocking image lookup (`fetch_aircraft_image`
    /// does a HEAD request) runs on the background thread, never the GUI loop.
    Aircraft(AircraftData, DiscordSettings),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordDeliveryOutcome {
    Queued,
    Delivered,
    Failed(String),
    Dropped(String),
}

/// Bounded, lock-protected summary of Discord queue and delivery activity.
/// `sequence` changes whenever a new status is published, allowing the UI to
/// consume changes without an unbounded event queue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscordDeliverySnapshot {
    pub sequence: u64,
    pub submitted: u64,
    pub delivered: u64,
    pub failed: u64,
    pub dropped: u64,
    pub latest: Option<DiscordDeliveryOutcome>,
}

#[derive(Clone)]
struct DiscordTransport {
    client: reqwest::blocking::Client,
    api_base: String,
}

impl DiscordTransport {
    fn new(api_base: impl Into<String>, timeout: Duration) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .timeout(timeout)
                .build()?,
            api_base: api_base.into(),
        })
    }

    fn production() -> Result<Self, String> {
        Self::new("https://discord.com/api/v10", Duration::from_secs(10))
            .map_err(|_| "Could not initialize the Discord HTTP client".to_string())
    }

    fn message_url(&self, channel_id: &str) -> String {
        format!(
            "{}/channels/{channel_id}/messages",
            self.api_base.trim_end_matches('/')
        )
    }

    fn post_embed(&self, embed: &DiscordEmbed, settings: &DiscordSettings) -> Result<(), String> {
        let response = self
            .client
            .post(self.message_url(&settings.channel_id))
            .header("Authorization", format!("Bot {}", settings.bot_token))
            .json(&embed.to_json(settings))
            .send()
            .map_err(sanitize_request_error)?;
        check_discord_response(response.status())
    }

    fn post_embed_with_attachment(
        &self,
        embed: &DiscordEmbed,
        settings: &DiscordSettings,
        file_bytes: Vec<u8>,
        file_name: String,
    ) -> Result<(), String> {
        let body = embed.to_json(settings);
        let form = reqwest::blocking::multipart::Form::new()
            .text("payload_json", body.to_string())
            .part(
                "files[0]",
                reqwest::blocking::multipart::Part::bytes(file_bytes).file_name(file_name),
            );
        let response = self
            .client
            .post(self.message_url(&settings.channel_id))
            .header("Authorization", format!("Bot {}", settings.bot_token))
            .multipart(form)
            .send()
            .map_err(sanitize_request_error)?;
        check_discord_response(response.status())
    }
}

fn sanitize_request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Discord request timed out".to_string()
    } else if error.is_connect() {
        "Could not connect to Discord".to_string()
    } else {
        "Discord request failed due to a network or TLS error".to_string()
    }
}

fn check_discord_response(status: reqwest::StatusCode) -> Result<(), String> {
    if status.is_success() {
        Ok(())
    } else {
        let reason = status.canonical_reason().unwrap_or("request failed");
        Err(format!(
            "Discord returned HTTP {} ({reason})",
            status.as_u16()
        ))
    }
}

fn publish_delivery_result(
    snapshot: &Arc<Mutex<DiscordDeliverySnapshot>>,
    result: Result<(), String>,
) {
    let mut snapshot = snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    snapshot.sequence = snapshot.sequence.wrapping_add(1);
    match result {
        Ok(()) => {
            snapshot.delivered = snapshot.delivered.saturating_add(1);
            snapshot.latest = Some(DiscordDeliveryOutcome::Delivered);
        }
        Err(error) => {
            snapshot.failed = snapshot.failed.saturating_add(1);
            snapshot.latest = Some(DiscordDeliveryOutcome::Failed(error));
        }
    }
}

fn publish_submission(
    snapshot: &Arc<Mutex<DiscordDeliverySnapshot>>,
    result: Result<(), crossbeam_channel::TrySendError<QueuedNotification>>,
) -> bool {
    let mut snapshot = snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    snapshot.sequence = snapshot.sequence.wrapping_add(1);
    match result {
        Ok(()) => {
            snapshot.submitted = snapshot.submitted.saturating_add(1);
            snapshot.latest = Some(DiscordDeliveryOutcome::Queued);
            true
        }
        Err(crossbeam_channel::TrySendError::Full(_)) => {
            snapshot.dropped = snapshot.dropped.saturating_add(1);
            snapshot.latest = Some(DiscordDeliveryOutcome::Dropped(
                "Discord notification queue is full; notification dropped".into(),
            ));
            false
        }
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
            snapshot.dropped = snapshot.dropped.saturating_add(1);
            snapshot.latest = Some(DiscordDeliveryOutcome::Dropped(
                "Discord delivery worker is unavailable; notification dropped".into(),
            ));
            false
        }
    }
}

/// Manages Discord notification dispatch with rate-limiting.
///
/// Holds a background thread that receives [`DiscordEmbed`] messages via a
/// channel and POSTs them to the Discord API. The `fire()` method enforces
/// per-kind enablement and a minimum inter-message interval.
pub struct DiscordNotifier {
    pub settings: DiscordSettings,
    tx: crossbeam_channel::Sender<QueuedNotification>,
    _worker_done: crossbeam_channel::Receiver<()>,
    delivery: Arc<Mutex<DiscordDeliverySnapshot>>,
    transport: DiscordTransport,
    last_send: Instant,
}

impl Default for DiscordNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordNotifier {
    /// Create a new `DiscordNotifier`, spawning a background dispatch thread.
    pub fn new() -> Self {
        let transport = DiscordTransport::production().expect("valid Discord HTTP client");
        Self::with_transport(transport)
    }

    fn with_transport(transport: DiscordTransport) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(64);
        let (worker_done_tx, worker_done) = crossbeam_channel::bounded(1);
        let delivery = Arc::new(Mutex::new(DiscordDeliverySnapshot::default()));
        let worker_delivery = Arc::clone(&delivery);
        let worker_transport = transport.clone();
        std::thread::spawn(move || {
            while let Ok(notification) = rx.recv() {
                // Receive a batch to send (the main thread will rate-limit via Instant)
                let result = match notification {
                    QueuedNotification::Plain(embed, settings) => {
                        worker_transport.post_embed(&embed, &settings)
                    }
                    QueuedNotification::WithAttachment(embed, settings, bytes, name) => {
                        worker_transport.post_embed_with_attachment(&embed, &settings, bytes, name)
                    }
                    QueuedNotification::Aircraft(ac, settings) => {
                        // Blocking HEAD lookup happens here (background),
                        // not in the egui render loop.
                        let image_url = fetch_aircraft_image(&ac.icao);
                        let embed = embed_aircraft(&ac, image_url);
                        worker_transport.post_embed(&embed, &settings)
                    }
                };
                publish_delivery_result(&worker_delivery, result);
            }
            let _ = worker_done_tx.send(());
        });

        Self {
            settings: DiscordSettings::default(),
            tx,
            _worker_done: worker_done,
            delivery,
            transport,
            last_send: Instant::now(),
        }
    }

    #[cfg(test)]
    fn with_test_api(api_base: String, timeout: Duration) -> Self {
        Self::with_transport(DiscordTransport::new(api_base, timeout).unwrap())
    }

    /// Latest bounded delivery counters and result for the application status surface.
    pub fn delivery_snapshot(&self) -> DiscordDeliverySnapshot {
        self.delivery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Update the notifier's settings from a new [`DiscordSettings`] snapshot.
    pub fn apply_settings(&mut self, settings: &DiscordSettings) {
        self.settings = settings.clone();
    }

    /// Returns `true` when the credentials required by the selected ping behavior
    /// are set and the notifier is enabled.
    pub fn is_configured(&self) -> bool {
        self.settings.enabled
            && !self.settings.bot_token.is_empty()
            && !self.settings.channel_id.is_empty()
            && (!self.settings.ping_user || !self.settings.user_id.is_empty())
    }

    fn enqueue(&self, notification: QueuedNotification) -> bool {
        publish_submission(&self.delivery, self.tx.try_send(notification))
    }

    /// Enqueue a notification embed for dispatch.
    ///
    /// The notification is silently dropped if the notifier is not configured,
    /// the kind is disabled, or the rate-limit interval has not elapsed.
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
        if self.enqueue(QueuedNotification::Plain(embed, self.settings.clone())) {
            self.last_send = Instant::now();
        }
    }

    /// Enqueue a new-aircraft notification. The blocking image lookup runs on
    /// the Discord background thread — callers must NOT call
    /// `fetch_aircraft_image` themselves (it does a synchronous HEAD request
    /// that would freeze the GUI loop).
    pub fn fire_aircraft(&mut self, ac: AircraftData) {
        if !self.settings.enabled || !self.is_configured() {
            return;
        }
        if !is_enabled(&self.settings, "aircraft_new") {
            return;
        }
        let elapsed = self.last_send.elapsed().as_millis() as u64;
        if elapsed < self.settings.min_send_interval_ms {
            return;
        }
        if self.enqueue(QueuedNotification::Aircraft(ac, self.settings.clone())) {
            self.last_send = Instant::now();
        }
    }

    /// Enqueue a notification embed with a local file attachment for dispatch.
    ///
    /// The embed's image should reference `attachment://{file_name}` (e.g. via
    /// [`DiscordEmbed::image_attachment_name`]) so Discord displays the
    /// uploaded file inline. Subject to the same enablement and rate-limiting
    /// rules as `fire()`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fire_with_attachment(
        &mut self,
        kind_id: &str,
        embed: DiscordEmbed,
        file_bytes: Vec<u8>,
        file_name: String,
    ) {
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
        if self.enqueue(QueuedNotification::WithAttachment(
            embed,
            self.settings.clone(),
            file_bytes,
            file_name,
        )) {
            self.last_send = Instant::now();
        }
    }

    /// Send a test notification to Discord to verify the bot credentials and
    /// channel configuration are working.
    #[must_use = "send a test Discord notification and check if it succeeded"]
    pub fn send_test(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let embed = embed_generic(
            "Test Notification",
            "If you see this, Discord integration is working!",
            "✅",
            0x00AA00,
        );
        self.send_test_embed(embed)
    }

    /// Send a test notification with a specific embed to Discord.
    ///
    /// BLOCKS on network I/O — call from a background thread (e.g. via
    /// `send_embed_blocking`), never from the UI thread.
    pub fn send_test_embed(
        &mut self,
        embed: DiscordEmbed,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_configured() {
            return Err("Not configured".into());
        }
        self.transport
            .post_embed(&embed, &self.settings)
            .map_err(|e| e.into())
    }

    /// Post one embed, blocking the caller. Static so UI code can run it on a
    /// worker thread with a cloned settings snapshot; returns a display-ready
    /// string in both outcomes for direct use as button status text.
    pub fn send_embed_blocking(
        settings: &DiscordSettings,
        embed: &DiscordEmbed,
    ) -> Result<(), String> {
        let transport = DiscordTransport::production()?;
        Self::send_embed_with_transport(settings, embed, &transport)
    }

    fn send_embed_with_transport(
        settings: &DiscordSettings,
        embed: &DiscordEmbed,
        transport: &DiscordTransport,
    ) -> Result<(), String> {
        if settings.bot_token.is_empty()
            || settings.channel_id.is_empty()
            || (settings.ping_user && settings.user_id.is_empty())
        {
            return Err("Not configured: bot token and channel are required; user ID is required when pinging is enabled".into());
        }
        transport.post_embed(embed, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn test_settings(ping_user: bool) -> DiscordSettings {
        DiscordSettings {
            enabled: true,
            bot_token: "bot-token-sentinel".into(),
            channel_id: "channel-test".into(),
            user_id: String::new(),
            ping_user,
            min_send_interval_ms: 0,
            ..DiscordSettings::default()
        }
    }

    fn mock_discord_server(
        status: u16,
        response_delay: Duration,
    ) -> (
        String,
        crossbeam_channel::Receiver<String>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = crossbeam_channel::bounded(1);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let request = read_http_request(&mut stream);
            let _ = request_tx.send(request);
            std::thread::sleep(response_delay);
            let reason = match status {
                201 => "Created",
                204 => "No Content",
                401 => "Unauthorized",
                403 => "Forbidden",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                _ => "Response",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes());
        });
        (format!("http://{address}/api/v10"), request_rx, server)
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        while let Ok(count) = stream.read(&mut chunk) {
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if bytes.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn wait_for_delivery(
        notifier: &DiscordNotifier,
        predicate: impl Fn(&DiscordDeliverySnapshot) -> bool,
    ) -> DiscordDeliverySnapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = notifier.delivery_snapshot();
            if predicate(&snapshot) {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "delivery status was not published"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn stop_test_notifier(notifier: DiscordNotifier) {
        let done = notifier._worker_done.clone();
        drop(notifier);
        done.recv_timeout(Duration::from_secs(3))
            .expect("test notifier worker should terminate");
    }

    #[test]
    fn partial_and_stale_aircraft_do_not_consume_position_notification() {
        let mut notified = std::collections::HashSet::new();
        let mut ac = crate::adsb_panel::AircraftEntry {
            icao: 1,
            callsign: "TEST".into(),
            ..Default::default()
        };
        assert!(positioned_aircraft_notification(&ac, &mut notified).is_none());
        assert!(notified.is_empty());
        ac.has_position = true;
        ac.position_seen = Some(Instant::now() - std::time::Duration::from_secs(61));
        assert!(positioned_aircraft_notification(&ac, &mut notified).is_none());
        assert!(notified.is_empty());
        ac.position_seen = Some(Instant::now());
        let data = positioned_aircraft_notification(&ac, &mut notified).unwrap();
        assert_eq!((data.lat, data.lon), (0.0, 0.0)); // actual valid zero coordinates
        assert_eq!(data.alt_ft, None);
        assert_eq!(data.speed_kts, None);
        assert_eq!(data.heading, None);
        let embed = embed_aircraft(&data, None);
        assert!(embed
            .fields
            .iter()
            .any(|(name, value, _)| name == "Altitude" && value == "Unavailable"));
        assert!(positioned_aircraft_notification(&ac, &mut notified).is_none());
    }

    #[test]
    fn categories_not_empty() {
        let cats = categories();
        assert!(!cats.is_empty());
    }

    #[test]
    fn notifier_workers_stop_after_all_senders_are_dropped() {
        let mut worker_done_receivers = Vec::new();

        for _ in 0..16 {
            let notifier = DiscordNotifier::new();
            worker_done_receivers.push(notifier._worker_done.clone());
            drop(notifier);
        }

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        for worker_done in worker_done_receivers {
            worker_done
                .recv_deadline(deadline)
                .expect("notifier worker should stop after its sender is dropped");
        }
    }

    #[test]
    fn http_status_codes_are_checked_for_plain_and_test_sends() {
        let settings = test_settings(false);
        let embed = embed_generic("Loopback test", "local response", "🧪", 0x123456);
        for status in [201, 401, 403, 429, 500] {
            let (api_base, requests, server) = mock_discord_server(status, Duration::ZERO);
            let transport = DiscordTransport::new(api_base, Duration::from_secs(1)).unwrap();
            let result = DiscordNotifier::send_embed_with_transport(&settings, &embed, &transport);
            let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(request.starts_with("POST /api/v10/channels/channel-test/messages HTTP/1.1"));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bot bot-token-sentinel"));
            if status == 201 {
                assert!(result.is_ok());
            } else {
                let error = result.unwrap_err();
                assert!(error.contains(&format!("HTTP {status}")));
                assert!(!error.contains("bot-token-sentinel"));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn attachment_send_checks_http_failures_and_uses_multipart() {
        let (api_base, requests, server) = mock_discord_server(500, Duration::ZERO);
        let transport = DiscordTransport::new(api_base, Duration::from_secs(1)).unwrap();
        let error = transport
            .post_embed_with_attachment(
                &embed_generic("Attachment", "local image", "🖼️", 0x123456),
                &test_settings(false),
                vec![1, 2, 3, 4],
                "test.png".into(),
            )
            .unwrap_err();
        assert!(error.contains("HTTP 500"));
        let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.starts_with("POST /api/v10/channels/channel-test/messages HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("multipart/form-data"));
        server.join().unwrap();
    }

    #[test]
    fn test_send_path_surfaces_http_failure_from_loopback_server() {
        let (api_base, requests, server) = mock_discord_server(401, Duration::ZERO);
        let mut notifier = DiscordNotifier::with_test_api(api_base, Duration::from_secs(1));
        notifier.apply_settings(&test_settings(false));
        let error = notifier
            .send_test_embed(embed_generic("Test", "loopback", "🧪", 0x123456))
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTP 401"));
        assert!(!error.contains("bot-token-sentinel"));
        assert!(requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .contains("POST /api/v10/channels/channel-test/messages"));
        server.join().unwrap();
        stop_test_notifier(notifier);
    }

    #[test]
    fn request_timeout_is_finite_and_reported_without_echoing_secrets() {
        let (api_base, requests, server) = mock_discord_server(204, Duration::from_millis(200));
        let transport = DiscordTransport::new(api_base, Duration::from_millis(30)).unwrap();
        let result = transport.post_embed(
            &embed_generic("Timeout", "local timeout", "⏱️", 0x123456),
            &test_settings(false),
        );
        assert_eq!(result.unwrap_err(), "Discord request timed out");
        let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.contains("POST /api/v10/channels/channel-test/messages"));
        server.join().unwrap();
    }

    #[test]
    fn worker_reports_queued_then_delivered_from_bounded_snapshot() {
        let (api_base, requests, server) = mock_discord_server(204, Duration::ZERO);
        let mut notifier = DiscordNotifier::with_test_api(api_base, Duration::from_secs(1));
        notifier.apply_settings(&test_settings(false));
        notifier.fire(
            "source_error",
            embed_generic("Worker", "loopback", "📡", 0x123456),
        );
        let queued = notifier.delivery_snapshot();
        assert_eq!(queued.submitted, 1);
        assert_eq!(queued.latest, Some(DiscordDeliveryOutcome::Queued));
        assert!(requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .contains("POST /api/v10/channels/channel-test/messages"));
        let delivered = wait_for_delivery(&notifier, |snapshot| snapshot.delivered == 1);
        assert_eq!(delivered.failed, 0);
        assert_eq!(delivered.latest, Some(DiscordDeliveryOutcome::Delivered));
        server.join().unwrap();
        stop_test_notifier(notifier);
    }

    #[test]
    fn worker_reports_http_failures_in_delivery_snapshot() {
        let (api_base, requests, server) = mock_discord_server(403, Duration::ZERO);
        let mut notifier = DiscordNotifier::with_test_api(api_base, Duration::from_secs(1));
        notifier.apply_settings(&test_settings(false));
        notifier.fire(
            "source_error",
            embed_generic("Worker", "loopback", "📡", 0x123456),
        );
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
        let failed = wait_for_delivery(&notifier, |snapshot| snapshot.failed == 1);
        let Some(DiscordDeliveryOutcome::Failed(error)) = failed.latest else {
            panic!("expected failed delivery outcome")
        };
        assert!(error.contains("HTTP 403"));
        assert!(!error.contains("bot-token-sentinel"));
        server.join().unwrap();
        stop_test_notifier(notifier);
    }

    #[test]
    fn full_notification_queue_is_visible_as_a_drop() {
        let (api_base, requests, server) = mock_discord_server(429, Duration::from_millis(200));
        let mut notifier = DiscordNotifier::with_test_api(api_base, Duration::from_millis(30));
        notifier.apply_settings(&test_settings(false));
        let embed = embed_generic("Queue", "loopback", "📡", 0x123456);
        notifier.fire("source_error", embed.clone());
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
        for _ in 0..100 {
            notifier.fire("source_error", embed.clone());
        }
        let snapshot = notifier.delivery_snapshot();
        assert!(snapshot.dropped > 0);
        assert!(matches!(
            snapshot.latest,
            Some(DiscordDeliveryOutcome::Dropped(_))
        ));
        server.join().unwrap();
        stop_test_notifier(notifier);
    }

    #[test]
    fn user_id_is_required_only_when_pinging_is_enabled() {
        let mut notifier = DiscordNotifier::with_test_api(
            "http://127.0.0.1:9/api/v10".into(),
            Duration::from_millis(50),
        );
        notifier.apply_settings(&test_settings(false));
        assert!(notifier.is_configured());
        notifier.settings.ping_user = true;
        assert!(!notifier.is_configured());
        notifier.settings.user_id = "user-test".into();
        assert!(notifier.is_configured());
        stop_test_notifier(notifier);
    }

    #[test]
    fn discord_debug_output_redacts_bot_token() {
        let settings = DiscordSettings {
            bot_token: "discord-debug-secret".into(),
            ..DiscordSettings::default()
        };
        let debug = format!("{settings:?}");
        assert!(!debug.contains("discord-debug-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn categories_contains_expected() {
        let cats = categories();
        assert!(cats.contains(&"ADS-B"));
        assert!(cats.contains(&"Source"));
        assert!(cats.contains(&"Signal"));
        assert!(cats.contains(&"System"));
    }

    #[test]
    fn categories_sorted() {
        let cats = categories();
        let mut sorted = cats.clone();
        sorted.sort_unstable();
        assert_eq!(cats, sorted);
    }

    #[test]
    fn categories_no_duplicates() {
        let cats = categories();
        let mut deduped = cats.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(cats, deduped);
    }

    #[test]
    fn kinds_in_known_category() {
        let kinds = kinds_in("Source");
        assert!(!kinds.is_empty());
        assert!(kinds.iter().all(|k| k.category == "Source"));
    }

    #[test]
    fn kinds_in_unknown_returns_empty() {
        let kinds = kinds_in("NonExistentCategory");
        assert!(kinds.is_empty());
    }

    #[test]
    fn is_enabled_explicit_true() {
        let mut settings = DiscordSettings::default();
        settings.enabled_kinds.insert("source_error".into(), true);
        assert!(is_enabled(&settings, "source_error"));
    }

    #[test]
    fn is_enabled_explicit_false() {
        let mut settings = DiscordSettings::default();
        settings.enabled_kinds.insert("source_error".into(), false);
        assert!(!is_enabled(&settings, "source_error"));
    }

    #[test]
    fn is_enabled_unset_essential() {
        let settings = DiscordSettings::default();
        assert!(is_enabled(&settings, "source_error"));
    }

    #[test]
    fn is_enabled_unset_non_essential() {
        let settings = DiscordSettings::default();
        assert!(!is_enabled(&settings, "source_started"));
    }

    #[test]
    fn is_enabled_missing_kind_id() {
        let settings = DiscordSettings::default();
        assert!(!is_enabled(&settings, "nonexistent_kind"));
    }

    #[test]
    fn is_enabled_empty_kind_id() {
        let settings = DiscordSettings::default();
        assert!(!is_enabled(&settings, ""));
    }

    #[test]
    fn is_starred_starred() {
        let settings = DiscordSettings::default();
        assert!(is_starred(&settings, "source_error"));
    }

    #[test]
    fn is_starred_unstarred() {
        let settings = DiscordSettings::default();
        assert!(!is_starred(&settings, "source_started"));
    }

    #[test]
    fn is_starred_missing_kind_id() {
        let settings = DiscordSettings::default();
        assert!(!is_starred(&settings, "nonexistent"));
    }

    // ── Embed builder tests ──────────────────────────────────────────

    #[test]
    fn embed_generic_builds_correctly() {
        let e = embed_generic("Hello", "World desc", "🚀", 0xFF00FF);
        assert!(e.title.contains("🚀"));
        assert!(e.title.contains("Hello"));
        assert_eq!(e.description, "World desc");
        assert_eq!(e.color, 0xFF00FF);
        assert!(e.fields.is_empty());
    }

    #[test]
    fn embed_aircraft_fields_and_image() {
        let ac = AircraftData {
            icao: "ABCDEF".into(),
            callsign: "UAL123".into(),
            lat: 37.77,
            lon: -122.42,
            alt_ft: Some(35000),
            speed_kts: Some(480),
            heading: Some(270),
            ..Default::default()
        };

        let e = embed_aircraft(&ac, None);
        assert_eq!(e.color, 0x0088FF);
        assert!(e.title.contains("UAL123"));
        assert!(e.fields.iter().any(|(n, ..)| n == "ICAO"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Altitude" && v == "35000 ft baro"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Speed" && v == "480 kt GS"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Heading" && v == "270° magnetic"));
        assert!(e.image_url.is_none());

        let e2 = embed_aircraft(&ac, Some("https://example.com/photo.jpg".into()));
        assert_eq!(e2.image_url, Some("https://example.com/photo.jpg".into()));
    }

    #[test]
    fn embed_scanner_hit_builds_correctly() {
        let e = embed_scanner_hit(123_456_789, -45.3);
        assert!(e.title.contains("Scanner Hit"));
        let f_mhz = 123_456_789f64 / 1e6;
        assert!(e.description.contains(&format!("{:.4}", f_mhz)));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Strength" && v == "-45.3 dB"));
    }

    #[test]
    fn embed_recording_started_all_combos() {
        let base = |is_iq, is_audio| embed_recording_started(100_000_000, "NFM", is_iq, is_audio);

        let iq_audio = base(true, true);
        assert!(iq_audio.description.contains("I/Q + Audio"));

        let iq = base(true, false);
        assert!(iq.description.contains("I/Q") && !iq.description.contains("Audio"));

        let audio = base(false, true);
        assert!(audio.description.contains("Audio") && !audio.description.contains("I/Q"));

        let unknown = base(false, false);
        assert!(unknown.description.contains("Unknown"));

        // All have frequency in title
        assert!(iq_audio.title.contains("Recording Started"));
    }

    #[test]
    fn embed_recording_stopped_builds_correctly() {
        let e = embed_recording_stopped(100_000_000, "NFM", 12345, 6_000_000);
        assert!(e.title.contains("Recording Stopped"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Duration" && v == "12345 sec"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Size" && v == "6.0 MB"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Frequency" && v.contains("100.0000")));
    }

    #[test]
    fn embed_recording_error_builds_correctly() {
        let e = embed_recording_error("Disk full");
        assert!(e.description.contains("Disk full"));
        assert_eq!(e.color, 0xFF3333);
    }

    #[test]
    fn embed_strong_signal_builds_correctly() {
        let e = embed_strong_signal(100_000_000, 25.5);
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Frequency" && v.contains("100.0000")));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "SNR" && v == "25.5 dB"));
    }

    #[test]
    fn embed_source_error_builds_correctly() {
        let e = embed_source_error("No device found");
        assert!(e.description.contains("No device found"));
        assert_eq!(e.color, 0xFF0000);
    }

    #[test]
    fn embed_task_fired_builds_correctly() {
        let e = embed_task_fired("Daily log", 150_000_000);
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Task" && v == "Daily log"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Frequency" && v.contains("150.0000")));
    }

    #[test]
    fn embed_sat_aos_builds_correctly() {
        let e = embed_sat_aos("ISS", 437_800_000, 45.0);
        assert!(e.title.contains("ISS"));
        assert!(e.title.contains("AOS"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Max Elevation" && v == "45.0°"));
    }

    #[test]
    fn embed_sat_los_builds_correctly() {
        let e = embed_sat_los("ISS");
        assert!(e.title.contains("ISS"));
        assert!(e.title.contains("LOS"));
    }

    #[test]
    fn embed_sat_upcoming_builds_correctly() {
        let e = embed_sat_upcoming("ISS", "12:00", "12:15", 67.5, 437_800_000);
        assert!(e.title.contains("ISS"));
        assert!(e.fields.iter().any(|(n, v, _)| n == "AOS" && v == "12:00"));
        assert!(e.fields.iter().any(|(n, v, _)| n == "LOS" && v == "12:15"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Max Elevation" && v == "67.5°"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Frequency" && v.contains("437.800")));
    }

    #[test]
    fn embed_session_summary_builds_correctly() {
        let e = embed_session_summary(3661, 100.5, "USB", 5, 42, 7, 3);
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Uptime" && v == "1h 1m"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Current Frequency" && v.contains("100.5000")));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Aircraft Tracked" && v == "5"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Scanner Hits" && v == "42"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Recordings" && v == "7"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Upcoming Passes" && v == "3"));
    }

    #[test]
    fn fire_aircraft_unconfigured_is_noop() {
        // Issue 40: fire_aircraft must never block the GUI loop. When
        // unconfigured it must drop immediately without network I/O.
        let mut n = DiscordNotifier::new();
        assert!(!n.is_configured());
        let ac = AircraftData {
            icao: "ABCDEF".into(),
            callsign: "TEST123".into(),
            lat: 51.5,
            lon: -0.1,
            alt_ft: Some(35000),
            speed_kts: Some(450),
            heading: Some(90),
            ..Default::default()
        };
        let start = Instant::now();
        n.fire_aircraft(ac);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "fire_aircraft must not block"
        );
    }
}
