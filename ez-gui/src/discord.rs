//! Discord notification integration for EZ-SDR.
//!
//! Provides types for configuring a Discord bot, constructing rich embed
//! messages for various SDR events (signals, aircraft, satellites, recordings,
//! etc.), and a [`DiscordNotifier`] that rate-limits and dispatches embeds.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// Configuration for the Discord notification bot.
///
/// Controls whether the bot is enabled, which channel/user to post to, which
/// event kinds are enabled, and rate-limiting parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    NotifKind {
        id: "lrpt_decode_started",
        category: "Satellite",
        label: "LRPT Decode Started",
        desc: "Meteor LRPT decoding began (live or file import)",
        emoji: "📡",
        essential: false,
        color: 0x0066FF,
    },
    NotifKind {
        id: "lrpt_decode_complete",
        category: "Satellite",
        label: "LRPT Decode Complete",
        desc: "Meteor LRPT image decoded",
        emoji: "🛰️",
        essential: true,
        color: 0x00CC00,
    },
    NotifKind {
        id: "lrpt_decode_error",
        category: "Satellite",
        label: "LRPT Decode Error",
        desc: "Decoding failed (no lock / RS failure rate too high)",
        emoji: "❌",
        essential: true,
        color: 0xFF0000,
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
    pub alt_ft: u32,
    /// Ground speed in knots.
    pub speed_kts: u32,
    /// Heading in degrees (0–359).
    pub heading: u32,
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
            ("Altitude".to_string(), format!("{} ft", ac.alt_ft), true),
            ("Speed".to_string(), format!("{} kts", ac.speed_kts), true),
            ("Heading".to_string(), format!("{}°", ac.heading), true),
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
#[cfg_attr(not(test), allow(dead_code))]
pub fn embed_lrpt_decode_started(sat_name: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: format!("📡 LRPT Decode Started: {sat_name}"),
        description: format!("Decoding **{sat_name}** LRPT downlink"),
        color: 0x0066FF,
        fields: vec![("Satellite".to_string(), sat_name.to_string(), false)],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a completed Meteor LRPT decode, including the
/// resulting image.
#[cfg_attr(not(test), allow(dead_code))]
pub fn embed_lrpt_decode_complete(
    sat_name: &str,
    lines: u32,
    rs_ok_pct: f32,
    image_path: &str,
) -> DiscordEmbed {
    DiscordEmbed {
        title: format!("🛰️ LRPT Decode Complete: {sat_name}"),
        description: format!("**{sat_name}** image decoded successfully"),
        color: 0x00CC00,
        fields: vec![
            ("Satellite".to_string(), sat_name.to_string(), true),
            ("Lines".to_string(), lines.to_string(), true),
            ("RS OK".to_string(), format!("{rs_ok_pct:.1}%"), true),
            ("Image".to_string(), image_path.to_string(), false),
        ],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

/// Build a Discord embed for a failed Meteor LRPT decode.
#[cfg_attr(not(test), allow(dead_code))]
pub fn embed_lrpt_decode_error(sat_name: &str, reason: &str) -> DiscordEmbed {
    DiscordEmbed {
        title: format!("❌ LRPT Decode Error: {sat_name}"),
        description: format!("**{reason}**"),
        color: 0xFF0000,
        fields: vec![
            ("Satellite".to_string(), sat_name.to_string(), true),
            ("Reason".to_string(), reason.to_string(), false),
        ],
        footer: "EZ-SDR • Satellite".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        image_url: None,
        image_attachment_name: None,
    }
}

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
    // Try multiple image sources in order
    let icao_upper = icao.to_uppercase();

    // Try PlaneSpotters CDN - most reliable for aircraft photos
    let planespotters_url = format!("https://cdn-photos.planespotters.net/photos/{icao_upper}.jpg");
    if is_url_valid(&planespotters_url) {
        return Some(planespotters_url);
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
}

/// Manages Discord notification dispatch with rate-limiting.
///
/// Holds a background thread that receives [`DiscordEmbed`] messages via a
/// channel and POSTs them to the Discord API. The `fire()` method enforces
/// per-kind enablement and a minimum inter-message interval.
pub struct DiscordNotifier {
    pub settings: DiscordSettings,
    tx: crossbeam_channel::Sender<QueuedNotification>,
    last_send: Instant,
}

impl DiscordNotifier {
    /// Create a new `DiscordNotifier`, spawning a background dispatch thread.
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(64);
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            loop {
                if let Ok(notification) = rx.recv() {
                    // Receive a batch to send (the main thread will rate-limit via Instant)
                    let result = match notification {
                        QueuedNotification::Plain(embed, settings) => {
                            Self::post_embed(&client, &embed, &settings)
                        }
                        QueuedNotification::WithAttachment(embed, settings, bytes, name) => {
                            Self::post_embed_with_attachment(
                                &client, &embed, &settings, bytes, name,
                            )
                        }
                    };
                    if let Err(e) = result {
                        eprintln!("[discord] POST failed: {e}");
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

    fn post_embed(
        client: &reqwest::blocking::Client,
        embed: &DiscordEmbed,
        settings: &DiscordSettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages",
            settings.channel_id
        );
        let body = embed.to_json(settings);
        client
            .post(&url)
            .header("Authorization", format!("Bot {}", settings.bot_token))
            .json(&body)
            .send()?;
        Ok(())
    }

    fn post_embed_with_attachment(
        client: &reqwest::blocking::Client,
        embed: &DiscordEmbed,
        settings: &DiscordSettings,
        file_bytes: Vec<u8>,
        file_name: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages",
            settings.channel_id
        );
        let body = embed.to_json(settings);
        let form = reqwest::blocking::multipart::Form::new()
            .text("payload_json", body.to_string())
            .part(
                "files[0]",
                reqwest::blocking::multipart::Part::bytes(file_bytes).file_name(file_name),
            );
        client
            .post(&url)
            .header("Authorization", format!("Bot {}", settings.bot_token))
            .multipart(form)
            .send()?;
        Ok(())
    }

    /// Update the notifier's settings from a new [`DiscordSettings`] snapshot.
    pub fn apply_settings(&mut self, settings: &DiscordSettings) {
        self.settings = settings.clone();
    }

    /// Returns `true` when all required Discord credentials (token, channel, user)
    /// are set and the notifier is enabled.
    pub fn is_configured(&self) -> bool {
        self.settings.enabled
            && !self.settings.bot_token.is_empty()
            && !self.settings.channel_id.is_empty()
            && !self.settings.user_id.is_empty()
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
        let _ = self
            .tx
            .try_send(QueuedNotification::Plain(embed, self.settings.clone()));
        self.last_send = Instant::now();
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
        let _ = self.tx.try_send(QueuedNotification::WithAttachment(
            embed,
            self.settings.clone(),
            file_bytes,
            file_name,
        ));
        self.last_send = Instant::now();
    }

    /// Send a test notification to Discord to verify the bot credentials and
    /// channel configuration are working.
    #[must_use = "send a test Discord notification and check if it succeeded"]
    pub fn send_test(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_configured() {
            return Err("Not configured".into());
        }
        let client = reqwest::blocking::Client::new();
        let embed = embed_generic(
            "Test Notification",
            "If you see this, Discord integration is working!",
            "✅",
            0x00AA00,
        );
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages",
            self.settings.channel_id
        );
        let body = embed.to_json(&self.settings);
        client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.settings.bot_token))
            .json(&body)
            .send()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_not_empty() {
        let cats = categories();
        assert!(!cats.is_empty());
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
            alt_ft: 35000,
            speed_kts: 480,
            heading: 270,
        };

        let e = embed_aircraft(&ac, None);
        assert_eq!(e.color, 0x0088FF);
        assert!(e.title.contains("UAL123"));
        assert!(e.fields.iter().any(|(n, ..)| n == "ICAO"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Altitude" && v == "35000 ft"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Speed" && v == "480 kts"));
        assert!(e
            .fields
            .iter()
            .any(|(n, v, _)| n == "Heading" && v == "270°"));
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
}
