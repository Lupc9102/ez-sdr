//! Persistent application configuration.
//!
//! Provides [`AppConfig`] for serializing/deserializing all SDR, UI, theme,
//! Discord, and AI settings to/from `ez_sdr_config.json`, including theme
//! configuration ([`ThemeConfig`]) and Discord integration settings.

use crate::discord::DiscordSettings;
use crate::theme::{NamedTheme, ThemeConfig};
use serde::{Deserialize, Serialize};

/// Advanced / experimental user-tunable parameters exposed via the
/// `⚙ More → Advanced` drawer. All fields are persisted to `ez_sdr_config.json`
/// (every field carries `#[serde(default)]` so a missing key never breaks load).
///
/// Conventions used to keep sliders "real but off by default":
/// * Frequencies/amounts of `0.0` mean *disabled* (no DSP stage runs).
/// * Booleans default to the existing app behaviour so the panel is a strict
///   superset — turning everything to its default reproduces stock EZ-SDR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvancedConfig {
    // ---------- Audio / DSP ----------
    /// RF channel width in Hz. Zero chooses the selected mode's default.
    #[serde(default)]
    pub radio_bandwidth_hz: f32,
    /// Zero chooses a tuning step appropriate to the selected demodulator.
    #[serde(default)]
    pub radio_snap_hz: u64,
    /// Audio low-pass cutoff in Hz. Zero chooses the selected mode's default.
    #[serde(default)]
    pub audio_cutoff_hz: f32,
    /// CW beat-frequency oscillator tone in Hz.
    #[serde(default = "default_cw_tone")]
    pub cw_tone_hz: f32,
    #[serde(default)]
    pub audio_output: crate::audio_output::AudioOutputSelection,
    #[serde(default)]
    pub wfm_stereo: bool,
    #[serde(default)]
    pub rds_enabled: bool,
    #[serde(default = "default_enabled")]
    pub rds_incremental: bool,
    #[serde(default)]
    pub rds_info: bool,
    #[serde(default)]
    pub rds_region: crate::radio_rds::RdsRegion,
    #[serde(default = "default_enabled")]
    pub fm_lowpass: bool,
    #[serde(default)]
    pub carrier_agc: bool,
    #[serde(default = "default_agc_attack_rate")]
    pub agc_attack_rate: f32,
    #[serde(default = "default_agc_decay_rate")]
    pub agc_decay_rate: f32,
    #[serde(default)]
    pub fm_if_nr: bool,
    #[serde(default)]
    pub fm_if_preset: crate::demod::FmIfPreset,
    #[serde(default)]
    pub squelch_mode: crate::radio_squelch::SquelchMode,
    #[serde(default = "default_squelch_level")]
    pub squelch_level_db: f32,
    #[serde(default)]
    pub ctcss_tone_hz: Option<f32>,
    /// Audio high-pass cutoff (Hz). 0 = disabled.
    #[serde(default)]
    pub audio_hpf_hz: f32,
    /// DC blocker blend 0..1 (0 = off, 1 = full removal).
    #[serde(default)]
    pub dc_blocker: f32,
    /// Audio AGC enabled.
    #[serde(default)]
    pub agc_enabled: bool,
    /// AGC target RMS level (0..1).
    #[serde(default)]
    pub agc_target: f32,
    /// AGC attack coefficient (higher = faster gain reduction).
    #[serde(default)]
    pub agc_attack: f32,
    /// AGC decay coefficient (higher = faster gain recovery).
    #[serde(default)]
    pub agc_decay: f32,
    /// FM de-emphasis time constant (microseconds). 50 = EU, 75 = US.
    #[serde(default)]
    pub deemph_tau_us: f32,
    /// Bass shelf gain (dB), -24..+24.
    #[serde(default)]
    pub bass_db: f32,
    /// Treble shelf gain (dB), -24..+24.
    #[serde(default)]
    pub treble_db: f32,
    /// Audio notch centre frequency (Hz). 0 = disabled.
    #[serde(default)]
    pub notch_hz: f32,
    /// Audio notch bandwidth (Hz).
    #[serde(default)]
    pub notch_width_hz: f32,
    /// Noise blanker strength 0..1 (0 = off).
    #[serde(default)]
    pub noise_blanker: f32,
    /// Pitch shift in octaves (-2..+2, 0 = none).
    #[serde(default)]
    pub pitch_octaves: f32,
    /// Extra audio gain multiplier (1.0 = unity).
    #[serde(default)]
    pub audio_gain: f32,
    /// DSB demodulator sideband selection.
    #[serde(default)]
    pub dsb_sideband: crate::demod::DsbSideband,
    /// CW BFO offset in Hz (added to the CW tone frequency).
    #[serde(default)]
    pub cw_offset_hz: f32,
    /// CW output volume multiplier (0.0 = mute, 1.0 = unity).
    #[serde(default = "default_cw_volume")]
    pub cw_volume: f32,
    /// CW squelch enabled (mutes audio when carrier drops below threshold).
    #[serde(default)]
    pub cw_squelch_enabled: bool,
    /// CW squelch threshold in dB (audio muted below this level).
    #[serde(default = "default_cw_squelch_level")]
    pub cw_squelch_level_db: f32,

    // ---------- Display / Spectrum ----------
    /// FFT size (power of two, 256..65536).
    #[serde(default)]
    pub fft_size: usize,
    #[serde(default = "default_fft_rate")]
    pub fft_rate: u32,
    #[serde(default = "default_enabled")]
    pub waterfall_visible: bool,
    #[serde(default = "default_enabled")]
    pub snr_smoothing: bool,
    #[serde(default = "default_snr_smoothing_secs")]
    pub snr_smoothing_secs: f32,
    /// FFT window name: "Hann" | "Hamming" | "Blackman" | "FlatTop".
    #[serde(default)]
    pub window: String,
    /// Waterfall scroll interval (push every N frames). 1 = fastest.
    #[serde(default)]
    pub wf_speed: u32,
    /// Waterfall history depth (rows).
    #[serde(default)]
    pub wf_depth: usize,
    /// Draw spectrum/waterfall grid lines.
    #[serde(default)]
    pub grid: bool,
    /// Peak-hold decay time (seconds). Larger = peaks linger longer.
    #[serde(default)]
    pub peak_hold_time: f32,
    /// Spectrum trace new-sample weight (0..1, lower = smoother).
    #[serde(default)]
    pub avg_alpha: f32,
    /// Spectrum persistence / afterglow 0..1 (0 = off).
    #[serde(default)]
    pub persistence: f32,
    /// Gradient fill under the spectrum line.
    #[serde(default)]
    pub gradient_fill: bool,
    /// Spectrum display floor (dBFS).
    #[serde(default)]
    pub db_min: f32,
    /// Spectrum display ceiling (dBFS).
    #[serde(default)]
    pub db_max: f32,

    // ---------- RF / Source ----------
    /// Tuner (hardware) AGC mode.
    #[serde(default)]
    pub tuner_agc: bool,
    /// RTL AGC mode.
    #[serde(default)]
    pub rtl_agc: bool,
    /// Direct sampling (RTL-SDR zero-IF mode).
    #[serde(default)]
    pub direct_sampling: bool,
    #[serde(default)]
    pub direct_sampling_branch: crate::source_manager::DirectSamplingBranch,
    #[serde(default)]
    pub rtl_device: crate::source_manager::RtlDeviceSelection,
    #[serde(default)]
    pub offset_tuning: bool,
    /// RF IQ decimation factor (1 = off, 2/4/8 downsample before demod).
    #[serde(default)]
    pub rf_decim: u32,
    /// Remove DC offset from raw IQ.
    #[serde(default)]
    pub rf_dc_remove: bool,
    #[serde(default)]
    pub invert_iq: bool,
    #[serde(default)]
    pub full_waterfall_update: bool,
    /// RF band-reject notch (software).
    #[serde(default)]
    pub rf_notch: bool,
    /// RF impulse noise blanker (software).
    #[serde(default)]
    pub rf_noise_blanker: bool,
    #[serde(default = "default_blanker_level")]
    pub rf_noise_blanker_level: f32,
    /// Bias-T enabled.
    #[serde(default)]
    pub bias_tee: bool,
    /// RF notch centre frequency (Hz).
    #[serde(default)]
    pub rf_notch_hz: f32,

    // ---------- Scan / Record / AI / Satellite extras ----------
    /// Scanner sweep direction: "up" | "down".
    #[serde(default)]
    pub scan_direction: String,
    /// Recorder file format: "wav" | "raw".
    #[serde(default)]
    pub record_format: String,
    /// Recorder auto-split size in MB (0 = no split).
    #[serde(default)]
    pub record_split_mb: u32,
    /// AI context window cap in tokens (0 = unlimited).
    #[serde(default)]
    pub ai_context_window: u32,
    /// Satellite elevation offset (degrees).
    #[serde(default)]
    pub sat_elevation_offset: f32,
    /// Satellite azimuth offset (degrees).
    #[serde(default)]
    pub sat_azimuth_offset: f32,
    /// Map zoom multiplier (0.5..4).
    #[serde(default)]
    pub map_zoom: f32,
    /// Satellite pass prediction lead time (minutes).
    #[serde(default)]
    pub pass_lead_min: u32,
}

fn default_cw_tone() -> f32 {
    800.0
}
fn default_fft_rate() -> u32 {
    20
}
fn default_enabled() -> bool {
    true
}
fn default_squelch_level() -> f32 {
    -100.0
}
fn default_agc_attack_rate() -> f32 {
    50.0
}
fn default_agc_decay_rate() -> f32 {
    5.0
}
fn default_blanker_level() -> f32 {
    1.0
}
fn default_snr_smoothing_secs() -> f32 {
    0.5
}
fn default_cw_volume() -> f32 {
    1.0
}
fn default_cw_squelch_level() -> f32 {
    -60.0
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            radio_bandwidth_hz: 0.0,
            radio_snap_hz: 0,
            audio_cutoff_hz: 0.0,
            cw_tone_hz: default_cw_tone(),
            audio_output: crate::audio_output::AudioOutputSelection::default(),
            wfm_stereo: false,
            rds_enabled: false,
            rds_incremental: true,
            rds_info: false,
            rds_region: crate::radio_rds::RdsRegion::Europe,
            fm_lowpass: true,
            carrier_agc: false,
            agc_attack_rate: default_agc_attack_rate(),
            agc_decay_rate: default_agc_decay_rate(),
            fm_if_nr: false,
            fm_if_preset: crate::demod::FmIfPreset::Voice,
            squelch_mode: crate::radio_squelch::SquelchMode::Off,
            squelch_level_db: default_squelch_level(),
            ctcss_tone_hz: None,
            audio_hpf_hz: 0.0,
            dc_blocker: 0.0,
            agc_enabled: true,
            agc_target: 0.25,
            agc_attack: 0.01,
            agc_decay: 0.0001,
            deemph_tau_us: 50.0,
            bass_db: 0.0,
            treble_db: 0.0,
            notch_hz: 0.0,
            notch_width_hz: 100.0,
            noise_blanker: 0.0,
            pitch_octaves: 0.0,
            audio_gain: 1.0,
            dsb_sideband: crate::demod::DsbSideband::Both,
            cw_offset_hz: 0.0,
            cw_volume: default_cw_volume(),
            cw_squelch_enabled: false,
            cw_squelch_level_db: default_cw_squelch_level(),

            fft_size: 2048,
            fft_rate: default_fft_rate(),
            waterfall_visible: true,
            snr_smoothing: true,
            snr_smoothing_secs: default_snr_smoothing_secs(),
            window: "Hann".to_string(),
            wf_speed: 2,
            wf_depth: 256,
            grid: true,
            peak_hold_time: 1.0,
            avg_alpha: 0.3,
            persistence: 0.0,
            gradient_fill: true,
            db_min: -120.0,
            db_max: 0.0,

            tuner_agc: false,
            rtl_agc: false,
            direct_sampling: false,
            direct_sampling_branch: crate::source_manager::DirectSamplingBranch::default(),
            rtl_device: crate::source_manager::RtlDeviceSelection::default(),
            offset_tuning: false,
            rf_decim: 1,
            rf_dc_remove: false,
            invert_iq: false,
            full_waterfall_update: false,
            rf_notch: false,
            rf_noise_blanker: false,
            rf_noise_blanker_level: default_blanker_level(),
            bias_tee: false,
            rf_notch_hz: 10_000.0,

            scan_direction: "up".to_string(),
            record_format: "wav".to_string(),
            record_split_mb: 0,
            ai_context_window: 0,
            sat_elevation_offset: 0.0,
            sat_azimuth_offset: 0.0,
            map_zoom: 1.0,
            pass_lead_min: 5,
        }
    }
}

/// A single togglable/reorderable entry in the sidebar layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutItem {
    pub id: String,
    pub visible: bool,
}

/// Which main tabs and secondary tools appear in the sidebar, and in what
/// order. Driven entirely by user customization — see the Layout sub-tab of
/// the Customize panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub main_tabs: Vec<LayoutItem>,
    pub secondary_tools: Vec<LayoutItem>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        fn items(ids: &[&str]) -> Vec<LayoutItem> {
            ids.iter()
                .map(|id| LayoutItem {
                    id: (*id).to_string(),
                    visible: true,
                })
                .collect()
        }
        Self {
            main_tabs: items(&["listen", "planes", "meteor"]),
            secondary_tools: items(&[
                "bookmarks",
                "scanner",
                "recorder",
                "scheduler",
                "discord",
                "howto",
                "settings",
            ]),
        }
    }
}

/// Default AI provider API endpoint (`OpenRouter`).
pub const DEFAULT_AI_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
/// Default AI model identifier.
pub const DEFAULT_AI_MODEL: &str = "anthropic/claude-haiku-latest";

/// A known AI provider preset with its endpoint, default model, and notes.
pub struct ProviderPreset {
    /// Display name of the provider.
    pub name: &'static str,
    /// Base URL for the provider's chat completions API.
    pub endpoint: &'static str,
    /// Recommended default model ID for this provider.
    pub default_model: &'static str,
    /// Whether this provider requires an API key.
    pub needs_key: bool,
    /// Short hint about the provider (e.g. "Free tier available").
    pub note: &'static str,
}

pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "OpenRouter",
        endpoint: "https://openrouter.ai/api/v1/chat/completions",
        default_model: "anthropic/claude-haiku-latest",
        needs_key: true,
        note: "Access 100+ models with one key. Free tier available.",
    },
    ProviderPreset {
        name: "Anthropic",
        endpoint: "https://api.anthropic.com/v1/messages",
        default_model: "claude-haiku-4-5-20251001",
        needs_key: true,
        note: "Direct Anthropic API. Uses x-api-key header.",
    },
    ProviderPreset {
        name: "OpenAI",
        endpoint: "https://api.openai.com/v1/chat/completions",
        default_model: "gpt-4o-mini",
        needs_key: true,
        note: "Direct OpenAI API.",
    },
    ProviderPreset {
        name: "Groq",
        endpoint: "https://api.groq.com/openai/v1/chat/completions",
        default_model: "llama-3.1-8b-instant",
        needs_key: true,
        note: "Very fast inference. Free tier available.",
    },
    ProviderPreset {
        name: "Mistral",
        endpoint: "https://api.mistral.ai/v1/chat/completions",
        default_model: "mistral-small-latest",
        needs_key: true,
        note: "European provider, strong multilingual.",
    },
    ProviderPreset {
        name: "Ollama (local)",
        endpoint: "http://localhost:11434/v1/chat/completions",
        default_model: "llama3.2",
        needs_key: false,
        note: "Fully local, no key needed. Install Ollama first.",
    },
    ProviderPreset {
        name: "Custom",
        endpoint: "",
        default_model: "",
        needs_key: true,
        note: "Set endpoint and model manually.",
    },
];

// Keep the subset copied for a mode switch explicit. Source, output-device,
// display and recording settings stay global and never enter a mode profile.
macro_rules! radio_mode_profile {
    ($($field:ident: $ty:ty),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct RadioModeProfile { $(pub $field: $ty),+ }

        impl Default for RadioModeProfile {
            fn default() -> Self { Self::capture(&AdvancedConfig::default()) }
        }
        impl RadioModeProfile {
            pub fn capture(settings: &AdvancedConfig) -> Self {
                Self { $($field: settings.$field.clone()),+ }
            }
            pub fn apply(&self, settings: &mut AdvancedConfig) {
                $(settings.$field = self.$field.clone();)+
            }
        }
    };
}
radio_mode_profile! {
    radio_bandwidth_hz: f32,
    radio_snap_hz: u64,
    audio_cutoff_hz: f32,
    cw_tone_hz: f32,
    wfm_stereo: bool,
    rds_enabled: bool,
    rds_incremental: bool,
    rds_info: bool,
    rds_region: crate::radio_rds::RdsRegion,
    fm_lowpass: bool,
    carrier_agc: bool,
    agc_enabled: bool,
    agc_target: f32,
    agc_attack_rate: f32,
    agc_decay_rate: f32,
    fm_if_nr: bool,
    fm_if_preset: crate::demod::FmIfPreset,
    squelch_mode: crate::radio_squelch::SquelchMode,
    squelch_level_db: f32,
    ctcss_tone_hz: Option<f32>,
    deemph_tau_us: f32,
    audio_hpf_hz: f32,
    rf_noise_blanker: bool,
    rf_noise_blanker_level: f32,
}

/// Connection and replay choices are restored without starting a receiver.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SourcePreferences {
    pub mode: String,
    pub daemon_address: String,
    pub replay_file: Option<String>,
    pub replay_loop: bool,
    pub replay_speed: f32,
}

impl Default for SourcePreferences {
    fn default() -> Self {
        Self {
            mode: "demo".into(),
            daemon_address: "127.0.0.1:7890".into(),
            replay_file: None,
            replay_loop: false,
            replay_speed: 1.0,
        }
    }
}

impl SourcePreferences {
    pub fn capture(source: &crate::source_manager::SourceManager) -> Self {
        use crate::source_manager::SourceMode;
        Self {
            mode: match source.source_mode {
                SourceMode::Simulated => "demo",
                SourceMode::Hardware => "rtl-sdr",
                SourceMode::Replay => "file",
                SourceMode::Daemon => "daemon",
            }
            .into(),
            daemon_address: source.daemon_addr.clone(),
            replay_file: source.replay_file.clone(),
            replay_loop: source.replay_loop,
            replay_speed: source.replay_speed,
        }
    }

    pub fn restore(&self, source: &mut crate::source_manager::SourceManager) {
        use crate::source_manager::SourceMode;
        source.source_mode = match self.mode.as_str() {
            "rtl-sdr" => SourceMode::Hardware,
            "file" => SourceMode::Replay,
            "daemon" => SourceMode::Daemon,
            _ => SourceMode::Simulated,
        };
        source.daemon_addr = self.daemon_address.clone();
        source.replay_file = self.replay_file.clone();
        source.replay_loop = self.replay_loop;
        source.replay_speed = if self.replay_speed.is_finite() && self.replay_speed > 0.0 {
            self.replay_speed
        } else {
            1.0
        };
    }
}

/// Top-level application configuration persisted to `ez_sdr_config.json`.
///
/// Contains all SDR, UI, theme, AI, MQTT, web remote, satellite, and Discord
/// settings. Serialised/deserialised with serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Schema version string.
    pub version: String,
    /// Default centre frequency (Hz) on startup.
    pub default_freq_hz: u64,
    /// Default sample rate (samples/second).
    pub default_sample_rate: u32,
    /// Default RF gain (dB).
    pub default_gain: f64,
    /// Directory for saving recorded I/Q and audio files.
    pub output_directory: String,
    /// UI theme name ("dark" / "light").
    pub theme: String,
    /// AI provider API key.
    pub ai_api_key: String,
    /// AI provider API endpoint URL.
    pub ai_endpoint: String,
    /// AI model identifier.
    pub ai_model: String,
    /// Maximum tokens per AI response.
    pub ai_max_tokens: u32,
    /// AI temperature (0.0 – 2.0).
    pub ai_temperature: f64,
    /// Custom system prompt for the AI agent.
    pub ai_system_prompt: String,
    /// AI provider name (matches a [`ProviderPreset`] entry).
    pub ai_provider: String,
    /// Reasoning effort level ("off", "low", "medium", "high").
    pub ai_reasoning_effort: String,
    /// Whether the AI agent has web-search capability enabled.
    pub ai_web_search: bool,
    /// MQTT broker address (host:port).
    pub mqtt_broker: String,
    /// MQTT topic prefix for all published messages.
    pub mqtt_topic_prefix: String,
    /// Whether the web remote control server is enabled.
    pub web_remote_enabled: bool,
    /// TCP port for the web remote server.
    pub web_remote_port: u16,
    /// Whether the loopback Hamlib/rigctld-compatible server is enabled.
    pub rigctl_enabled: bool,
    /// TCP port for the loopback rigctl server.
    pub rigctl_port: u16,
    /// Observer latitude (decimal degrees, north positive).
    pub observer_lat: f64,
    /// Observer longitude (decimal degrees, east positive).
    pub observer_lon: f64,
    /// UI font scale multiplier.
    pub font_scale: f64,
    /// Flag indicating settings have changed and need to be applied.
    pub needs_apply: bool,
    /// Recently tuned frequencies (for quick-access menu).
    pub recent_frequencies: Vec<u64>,
    /// Spectrum display minimum (dBFS).
    pub spectrum_min_db: f32,
    /// Spectrum display maximum (dBFS).
    pub spectrum_max_db: f32,
    /// Frequency correction in parts-per-million.
    pub ppm_correction: i32,
    /// VFO B frequency (Hz).
    pub vfo_b_hz: u64,
    /// Waterfall colour range minimum (dBFS).
    pub wf_min_db: f32,
    /// Waterfall colour range maximum (dBFS).
    pub wf_max_db: f32,
    /// Local oscillator offset (Hz) for upconverter / downconverter.
    pub lo_offset_hz: i64,
    /// Keep the tuned VFO at the capture center; false enables digital tuning.
    pub center_tuning: bool,
    pub last_session_center_hz: Option<u64>,
    /// Last-used frequency from previous session.
    pub last_session_freq_hz: u64,
    /// Last-used gain from previous session.
    pub last_session_gain_db: f64,
    /// Last-used demodulation mode from previous session.
    pub last_session_demod: String,
    pub source_preferences: SourcePreferences,
    /// Waterfall colour map name.
    pub color_map: String,
    /// Frequency memory slot values (Hz).
    pub freq_memory_hz: Vec<u64>,
    /// Frequency memory slot labels.
    pub freq_memory_labels: Vec<String>,
    /// Theme configuration (colours, presets).
    pub theme_config: ThemeConfig,
    /// Discord notification settings.
    pub discord: DiscordSettings,
    /// Whether to skip the antenna-setup checklist on startup.
    pub skip_antenna_checklists: bool,
    /// User experience level string (e.g. "beginner", "advanced").
    pub user_level: String,
    /// User-saved named themes (the Customize tab's theme gallery), distinct
    /// from the built-in presets in [`ThemeConfig::all_presets`].
    pub custom_themes: Vec<NamedTheme>,
    /// Sidebar tab/tool visibility and ordering.
    pub layout: LayoutConfig,
    /// Advanced / experimental tunables (⚙ More → Advanced drawer).
    pub advanced: AdvancedConfig,
    /// Each demodulator keeps its own radio module controls, like SDR++.
    pub radio_profiles: std::collections::BTreeMap<String, RadioModeProfile>,
    /// Whether the initial quick start wizard has been completed.
    pub quick_start_completed: bool,
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
            ai_reasoning_effort: "off".to_string(),
            ai_web_search: false,
            mqtt_broker: "localhost:1883".to_string(),
            mqtt_topic_prefix: "ezsdr".to_string(),
            web_remote_enabled: false,
            web_remote_port: 5259,
            rigctl_enabled: false,
            rigctl_port: 4532,
            observer_lat: 51.5,
            observer_lon: -0.1,
            font_scale: 1.0,
            needs_apply: false,
            recent_frequencies: Vec::new(),
            spectrum_min_db: -120.0,
            spectrum_max_db: 0.0,
            ppm_correction: 0,
            vfo_b_hz: 0,
            wf_min_db: -120.0,
            wf_max_db: -20.0,
            lo_offset_hz: 0,
            center_tuning: false,
            last_session_center_hz: None,
            last_session_freq_hz: 0,
            last_session_gain_db: -1.0,
            last_session_demod: String::new(),
            source_preferences: SourcePreferences::default(),
            color_map: "Classic".to_string(),
            freq_memory_hz: Vec::new(),
            freq_memory_labels: Vec::new(),
            theme_config: ThemeConfig::default(),
            discord: DiscordSettings::default(),
            skip_antenna_checklists: false,
            user_level: "beginner".to_string(),
            custom_themes: Vec::new(),
            layout: LayoutConfig::default(),
            advanced: AdvancedConfig::default(),
            radio_profiles: std::collections::BTreeMap::new(),
            quick_start_completed: false,
        }
    }
}

impl AppConfig {
    /// Load configuration from `ez_sdr_config.json`, or return defaults if the file does not exist or cannot be parsed.
    pub fn load_or_default() -> Self {
        let mut cfg: AppConfig = std::fs::read_to_string("ez_sdr_config.json")
            .ok()
            .and_then(|s| serde_json::from_str::<AppConfig>(&s).ok())
            .unwrap_or_default();
        cfg.normalize();
        cfg
    }

    /// Normalize settings from older files before they reach UI or hardware APIs.
    pub fn normalize(&mut self) {
        let defaults = Self::default();
        if !self.font_scale.is_finite() || self.font_scale <= 0.0 {
            self.font_scale = defaults.font_scale;
        }
        self.font_scale = self.font_scale.clamp(0.5, 3.0);
        if self.default_sample_rate == 0 {
            self.default_sample_rate = defaults.default_sample_rate;
        }
        if !self.default_gain.is_finite() {
            self.default_gain = defaults.default_gain;
        }
        self.default_gain = self.default_gain.clamp(0.0, 49.6);
        if !self.last_session_gain_db.is_finite() {
            self.last_session_gain_db = -1.0;
        }
        if !self.observer_lat.is_finite() || !self.observer_lon.is_finite() {
            self.observer_lat = defaults.observer_lat;
            self.observer_lon = defaults.observer_lon;
        }
        self.observer_lat = self.observer_lat.clamp(-90.0, 90.0);
        self.observer_lon = self.observer_lon.clamp(-180.0, 180.0);
        let cfg = self;
        // Migration: the old 6-tab layout (sdr/adsb/satellite/ai/decoding/…)
        // collapses to the current task tabs. Any config with invalid tab IDs
        // is normalized to the new mode set, preserving the
        // "keep plumbing + migration pattern" contract.
        let valid_tab_ids = ["listen", "planes", "satellites", "meteor"];
        if cfg
            .layout
            .main_tabs
            .iter()
            .all(|i| valid_tab_ids.contains(&i.id.as_str()))
            && !cfg.layout.main_tabs.iter().any(|i| i.id == "meteor")
        {
            let meteor = LayoutItem {
                id: "meteor".to_string(),
                visible: true,
            };
            let index = cfg
                .layout
                .main_tabs
                .iter()
                .position(|item| item.id == "satellites")
                .unwrap_or(cfg.layout.main_tabs.len());
            cfg.layout.main_tabs.insert(index, meteor);
        }
        if !cfg
            .layout
            .main_tabs
            .iter()
            .all(|i| valid_tab_ids.contains(&i.id.as_str()))
        {
            cfg.layout.main_tabs = LayoutConfig::default().main_tabs;
        }
    }

    /// Serialise and write the configuration atomically to `ez_sdr_config.json`.
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(e) =
                crate::bookmarks::atomic_write_file("ez_sdr_config.json", json.as_bytes())
            {
                eprintln!("[config] failed to write config file: {}", e);
            }
        }
    }
}

#[derive(Clone)]
struct ConfigFileJob(
    std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Result<Option<AppConfig>, String>>>>,
);

fn begin_config_file_job(
    ctx: &egui::Context,
    work: impl FnOnce() -> Result<Option<AppConfig>, String> + Send + 'static,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let id = egui::Id::new("config.file_job");
    ctx.data_mut(|data| {
        data.insert_temp(
            id,
            ConfigFileJob(std::sync::Arc::new(std::sync::Mutex::new(rx))),
        )
    });
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err("File dialog stopped unexpectedly".into()));
        let _ = tx.send(result);
        repaint.request_repaint();
    });
}

impl AppConfig {
    /// Render the egui-based settings panel UI.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        let job_id = egui::Id::new("config.file_job");
        let message_id = egui::Id::new("config.file_message");
        let job = ui
            .ctx()
            .data_mut(|data| data.get_temp::<ConfigFileJob>(job_id));
        if let Some(job) = &job {
            let result = job.0.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
            let message = match result {
                Ok(Ok(Some(mut loaded))) => {
                    loaded.normalize();
                    *self = loaded;
                    self.needs_apply = true;
                    Some("Settings imported".to_string())
                }
                Ok(Ok(None)) => Some("File action finished".to_string()),
                Ok(Err(error)) => Some(error),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some("File action stopped unexpectedly".into())
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(message) = message {
                ui.ctx().data_mut(|data| {
                    data.remove::<ConfigFileJob>(job_id);
                    data.insert_temp(message_id, message);
                });
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if let Some(message) = ui
            .ctx()
            .data_mut(|data| data.get_temp::<String>(message_id))
        {
            ui.label(message);
        }

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
                    .custom_formatter(|v, _| format!("{v:.1} dB")))
                    .on_hover_text("RF amplification. Higher is not always better — too much gain causes overload and phantom signals. Typical sweet spot: 30–45 dB.");
            });

            ui.collapsing("Frequency Calibration", |ui| {
                ui.label("PPM Correction (parts per million):")
                    .on_hover_text("Crystal frequency error compensation. Positive = crystal runs slow, negative = runs fast. Typical RTL-SDR: ±20 ppm.");
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.ppm_correction, -100..=100)
                        .text("PPM")
                        .custom_formatter(|v, _| format!("{v:+.0} ppm")));
                });
                ui.label("Quick adjust:").on_hover_text("Click to adjust PPM by preset amounts.");
                ui.horizontal_wrapped(|ui| {
                    for offset in &[-50, -20, -10, -5, 0, 5, 10, 20, 50] {
                        let label = if *offset == 0 { "Reset".to_string() } else { format!("{offset:+}") };
                        if ui.small_button(label).clicked() {
                            self.ppm_correction = *offset;
                        }
                    }
                });
                ui.label(format!("Current: {} ppm", self.ppm_correction))
                    .on_hover_text("Tune a known frequency and adjust this value until it matches exactly.");
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
                            for m in &["anthropic/claude-haiku-latest", "anthropic/claude-sonnet-latest", "google/gemini-flash-1.5", "meta-llama/llama-3.1-8b-instruct:free", "mistralai/mistral-7b-instruct:free"] {
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
                            for m in &["claude-haiku-4-5-20251001", "claude-3-5-sonnet-20241022", "claude-3-opus-20240229"] {
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

                // Reasoning effort
                ui.horizontal(|ui| {
                    ui.label("Reasoning:").on_hover_text("Controls how hard the model thinks. Higher = slower but more thorough. 'Off' uses default behavior.");
                    let current = self.ai_reasoning_effort.clone();
                    egui::ComboBox::from_id_salt("ai_reasoning_effort")
                        .selected_text(&current)
                        .show_ui(ui, |ui| {
                            for level in &["off", "low", "medium", "high"] {
                                if ui.selectable_label(current == *level, *level).clicked() {
                                    self.ai_reasoning_effort = level.to_string();
                                }
                            }
                        });
                });

                // Web search toggle
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.ai_web_search, " Enable web search")
                        .on_hover_text("Ask the selected AI provider to use its native web-search tool when supported. Search terms are sent to that provider; EZ-SDR does not scrape search-engine HTML.");
                });

                ui.add(egui::Slider::new(&mut self.ai_max_tokens, 256u32..=16384u32)
                    .step_by(256.0).text("Max tokens"))
                    .on_hover_text("Maximum response length in tokens (~4 chars each). 2048 is plenty for most tasks.");
                ui.collapsing("System prompt", |ui| {
                    ui.label("Leave empty for default (tool-enabled assistant).");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.ai_system_prompt)
                            .desired_width(f32::INFINITY));
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

            ui.collapsing("Rigctl Server", |ui| {
                ui.checkbox(&mut self.rigctl_enabled, "Enable loopback rigctl")
                    .on_hover_text("Expose the Hamlib/rigctld-compatible control socket on localhost only.");
                ui.add(egui::Slider::new(&mut self.rigctl_port, 1024..=65535).text("Port"))
                    .on_hover_text("TCP port for rigctld clients. Default 4532.");
            });

            ui.colored_label(
                egui::Color32::GRAY,
                "🎨 Theme, fonts, effects, and layout have moved to the Customize tab.",
            );

        ui.collapsing("User Experience", |ui| {
            ui.label(egui::RichText::new("Knowledge Level").strong())
                .on_hover_text("Controls which UI controls and features are visible. Beginners see simplified panels with more hints; Experts see everything.");
            let user_level = crate::user_level::UserLevel::from_name(&self.user_level);
            let mut level_idx = user_level as usize;
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut level_idx, 0..=3).step_by(1.0).text("Level"))
                    .on_hover_text("Drag to change your experience level.");
            });
            // Show the name + description below the slider
            if let Some(level) = [crate::user_level::UserLevel::Beginner,
                                  crate::user_level::UserLevel::Intermediate,
                                  crate::user_level::UserLevel::Advanced,
                                  crate::user_level::UserLevel::ClerkMaxwell].get(level_idx) {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 255),
                    format!("{} — {}", level.label(), level.description()));
                self.user_level = level.to_str().to_string();
            }
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
                if ui.add_enabled(job.is_none(), egui::Button::new("📤 Export…")).on_hover_text("Export config to a custom file path via file dialog.").clicked() {
                    let config = self.clone();
                    begin_config_file_job(ui.ctx(), move || {
                        let Some(path) = rfd::FileDialog::new()
                            .set_file_name("ez_sdr_config_backup.json")
                            .add_filter("JSON", &["json"])
                            .save_file() else { return Ok(None); };
                        let json = serde_json::to_vec_pretty(&config).map_err(|e| format!("Cannot export settings: {e}"))?;
                        crate::bookmarks::atomic_write_file(&path, &json)
                            .map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
                        Ok(None)
                    });
                }
                if ui.add_enabled(job.is_none(), egui::Button::new("📥 Import…")).on_hover_text("Load config from a previously exported JSON file.").clicked() {
                    begin_config_file_job(ui.ctx(), || {
                        let Some(path) = rfd::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .pick_file() else { return Ok(None); };
                        let data = std::fs::read_to_string(&path)
                            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
                        serde_json::from_str::<AppConfig>(&data).map(Some)
                            .map_err(|e| format!("Invalid settings in {}: {e}", path.display()))
                    });
                }
            });
            ui.colored_label(
                egui::Color32::GRAY,
                "Settings are saved to ez_sdr_config.json in the current directory. Spectrum dB range is saved with Ctrl+S.",
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_zero_scale_and_sample_rate_are_repaired_before_apply() {
        let mut config: AppConfig = serde_json::from_str(
            r#"{"font_scale":0,"default_sample_rate":0,"observer_lat":300,"observer_lon":-400}"#,
        )
        .unwrap();
        config.normalize();
        assert_eq!(config.font_scale, 1.0);
        assert_eq!(config.default_sample_rate, 2_048_000);
        assert_eq!(config.observer_lat, 90.0);
        assert_eq!(config.observer_lon, -180.0);
        assert!(config.layout.main_tabs.iter().any(|tab| tab.id == "meteor"));
    }

    #[test]
    fn receiver_parity_settings_upgrade_and_roundtrip() {
        let old: AdvancedConfig = serde_json::from_str(r#"{"fft_size":4096}"#).unwrap();
        assert_eq!(old.fft_rate, 20);
        assert!(old.waterfall_visible);
        assert!(old.snr_smoothing);
        assert_eq!(old.snr_smoothing_secs, 0.5);
        assert_eq!(old.cw_tone_hz, 800.0);
        assert!(old.fm_lowpass && old.rds_incremental);
        assert!(!old.rds_enabled && !old.fm_if_nr && !old.carrier_agc);
        assert_eq!(old.squelch_mode, crate::radio_squelch::SquelchMode::Off);
        assert_eq!(old.radio_bandwidth_hz, 0.0);
        let configured = AdvancedConfig {
            fft_size: 65_536,
            fft_rate: 30,
            waterfall_visible: false,
            radio_bandwidth_hz: 800.0,
            audio_cutoff_hz: 2_000.0,
            cw_tone_hz: 850.0,
            audio_output: crate::audio_output::AudioOutputSelection {
                device_id: Some("alsa:device-test".into()),
                sample_rate: 44_100,
            },
            rtl_device: crate::source_manager::RtlDeviceSelection {
                index: 2,
                serial: Some("RECEIVER-B".into()),
            },
            offset_tuning: true,
            ..Default::default()
        };
        let saved = serde_json::to_string(&configured).unwrap();
        assert_eq!(
            serde_json::from_str::<AdvancedConfig>(&saved).unwrap(),
            configured
        );
    }

    // `std::env::current_dir`/`set_current_dir` are process-global, so any test
    // that changes cwd must serialize with other cwd-changing tests or they'll
    // race and clobber each other's "restore" step under parallel test execution.
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn config_default_has_reasonable_freq() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.default_freq_hz, 100_000_000);
    }

    #[test]
    fn config_default_sample_rate() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.default_sample_rate, 2_048_000);
    }

    #[test]
    fn config_default_theme_is_dark() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.theme, "dark");
    }

    #[test]
    fn config_default_observer_at_london() {
        let cfg = AppConfig::default();
        assert!((cfg.observer_lat - 51.5).abs() < f64::EPSILON);
        assert!((cfg.observer_lon - (-0.1)).abs() < f64::EPSILON);
    }

    #[test]
    fn config_default_has_theme_and_discord() {
        let cfg = AppConfig::default();
        // theme_config should have the default preset
        assert_eq!(cfg.theme_config.preset, "dark");
        // discord should default to disabled
        assert!(!cfg.discord.enabled);
        assert!(!cfg.rigctl_enabled);
        assert_eq!(cfg.rigctl_port, 4532);
    }

    #[test]
    fn config_serde_roundtrip_preserves_fields() {
        let cfg = AppConfig::default();
        let json = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.default_freq_hz, cfg.default_freq_hz);
        assert_eq!(deserialized.theme, cfg.theme);
        assert_eq!(deserialized.ai_model, cfg.ai_model);
        assert_eq!(deserialized.rigctl_enabled, cfg.rigctl_enabled);
        assert_eq!(deserialized.rigctl_port, cfg.rigctl_port);
    }

    #[test]
    fn provider_presets_have_names() {
        for p in PROVIDER_PRESETS {
            assert!(!p.name.is_empty());
        }
        assert!(PROVIDER_PRESETS.iter().any(|p| p.name == "Custom"));
    }

    #[test]
    fn config_load_or_default_nonexistent_returns_default() {
        let cfg = AppConfig::load_or_default();
        assert_eq!(cfg.default_freq_hz, 100_000_000);
        assert_eq!(cfg.theme, "dark");
    }

    #[test]
    fn config_save_and_load_roundtrip() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ez_sdr_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let original_dir = std::env::current_dir().expect("test should have a current directory");
        std::env::set_current_dir(&dir).expect("test should cd into temp dir");

        let cfg = AppConfig {
            default_freq_hz: 433_000_000,
            theme: "light".to_string(),
            ai_model: "custom-model".to_string(),
            output_directory: "/tmp/recordings".to_string(),
            observer_lat: 40.7128,
            observer_lon: -74.0060,
            ..Default::default()
        };
        cfg.save();

        let loaded = AppConfig::load_or_default();
        assert_eq!(loaded.default_freq_hz, 433_000_000);
        assert_eq!(loaded.theme, "light");
        assert_eq!(loaded.ai_model, "custom-model");
        assert_eq!(loaded.output_directory, "/tmp/recordings");
        assert!((loaded.observer_lat - 40.7128).abs() < f64::EPSILON);
        assert!((loaded.observer_lon - (-74.0060)).abs() < f64::EPSILON);

        std::env::set_current_dir(original_dir).expect("test should restore original directory");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_partial_json_missing_fields_get_defaults() {
        let json = r#"{"default_freq_hz": 144000000}"#;
        let cfg: AppConfig = serde_json::from_str(json).expect("partial deserialize");
        assert_eq!(cfg.default_freq_hz, 144_000_000);
        // Omitted settings use usable application defaults, including UI scale.
        assert_eq!(cfg.theme, "dark");
        assert_eq!(cfg.default_sample_rate, 2_048_000);
        assert_eq!(cfg.font_scale, 1.0);
        assert!(!cfg.discord.enabled);
    }

    #[test]
    fn layout_config_default_lists_all_current_tabs_and_tools_visible() {
        let layout = LayoutConfig::default();
        assert_eq!(
            layout
                .main_tabs
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            vec!["listen", "planes", "meteor"]
        );
        assert_eq!(
            layout
                .secondary_tools
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "bookmarks",
                "scanner",
                "recorder",
                "scheduler",
                "discord",
                "howto",
                "settings"
            ]
        );
        assert!(layout.main_tabs.iter().all(|i| i.visible));
        assert!(layout.secondary_tools.iter().all(|i| i.visible));
    }

    #[test]
    fn config_default_has_empty_custom_theme_gallery() {
        let cfg = AppConfig::default();
        assert!(cfg.custom_themes.is_empty());
    }

    #[test]
    fn layout_migration_maps_legacy_tabs_to_current_modes() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ez_sdr_test_migrate_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let original_dir = std::env::current_dir().expect("test should have a current directory");
        std::env::set_current_dir(&dir).expect("test should cd into temp dir");

        // Simulate a config saved under the old 6-tab layout.
        let old_layout = LayoutConfig {
            main_tabs: vec![
                LayoutItem {
                    id: "sdr".into(),
                    visible: true,
                },
                LayoutItem {
                    id: "adsb".into(),
                    visible: true,
                },
                LayoutItem {
                    id: "satellite".into(),
                    visible: true,
                },
                LayoutItem {
                    id: "ai".into(),
                    visible: true,
                },
                LayoutItem {
                    id: "decoding".into(),
                    visible: true,
                },
            ],
            ..Default::default()
        };
        let old_cfg = AppConfig {
            layout: old_layout,
            ..Default::default()
        };
        old_cfg.save();

        let loaded = AppConfig::load_or_default();
        let ids: Vec<&str> = loaded
            .layout
            .main_tabs
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, vec!["listen", "planes", "meteor"]);
        assert!(
            loaded
                .layout
                .main_tabs
                .iter()
                .find(|i| i.id == "listen")
                .expect("listen entry should be present")
                .visible
        );
        assert!(!loaded.layout.main_tabs.iter().any(|i| i.id == "decoding"));

        std::env::set_current_dir(original_dir).expect("test should restore original directory");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_missing_layout_and_custom_themes_fall_back_to_defaults() {
        let json = r#"{"default_freq_hz": 144000000}"#;
        let cfg: AppConfig = serde_json::from_str(json).expect("partial deserialize");
        assert_eq!(cfg.layout, LayoutConfig::default());
        assert!(cfg.custom_themes.is_empty());
    }

    #[test]
    fn config_custom_themes_roundtrip_through_json() {
        let mut cfg = AppConfig::default();
        cfg.custom_themes.push(NamedTheme {
            id: "t1".to_string(),
            name: "My Custom".to_string(),
            theme: ThemeConfig::cyberpunk(),
        });
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.custom_themes.len(), 1);
        assert_eq!(back.custom_themes[0].name, "My Custom");
        assert_eq!(back.custom_themes[0].theme.preset, "cyberpunk");
    }

    #[test]
    fn config_partial_json_theme_and_discord() {
        let json = r#"{"theme": "light", "discord": {"enabled": true}}"#;
        let cfg: AppConfig = serde_json::from_str(json)
            .expect("config partial JSON should deserialize with defaults");
        assert_eq!(cfg.theme, "light");
        assert!(cfg.discord.enabled);
        assert_eq!(cfg.default_freq_hz, 100_000_000);
    }

    #[test]
    fn config_edge_values_zero_freq() {
        let json = r#"{"version": "0.1.0", "default_freq_hz": 0}"#;
        let cfg: AppConfig =
            serde_json::from_str(json).expect("config edge values JSON should deserialize");
        assert_eq!(cfg.default_freq_hz, 0);
    }

    #[test]
    fn config_edge_values_max_freq() {
        let json = format!(r#"{{"version": "0.1.0", "default_freq_hz": {}}}"#, u64::MAX);
        let cfg: AppConfig =
            serde_json::from_str(&json).expect("config max freq JSON should deserialize");
        assert_eq!(cfg.default_freq_hz, u64::MAX);
    }

    #[test]
    fn config_edge_values_empty_strings() {
        let json = r#"{"version": "0.1.0", "theme": "", "output_directory": "", "ai_model": ""}"#;
        let cfg: AppConfig =
            serde_json::from_str(json).expect("config empty strings JSON should deserialize");
        assert_eq!(cfg.theme, "");
        assert_eq!(cfg.output_directory, "");
        assert_eq!(cfg.ai_model, "");
    }
}
