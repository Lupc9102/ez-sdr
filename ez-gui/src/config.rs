//! Persistent application configuration.
//!
//! Provides [`AppConfig`] for serializing/deserializing SDR, UI, theme,
//! application settings to/from `ez_sdr_config.json`.

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
    /// FFT peak-hold decay time in seconds (0 = off).
    #[serde(default = "default_fft_hold")]
    pub fft_hold: f32,
    /// Explicit smoothing on/off (mirrors avg_alpha but independently toggleable).
    #[serde(default = "default_smoothing_enabled")]
    pub smoothing_enabled: bool,
    /// Smoothing speed 0..1 (higher = faster response).
    #[serde(default = "default_smoothing_speed")]
    pub smoothing_speed: f32,
    /// Fast FFT: skip overlap windows for lower latency.
    #[serde(default)]
    pub fast_fft: bool,
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

    // ---------- Scan / Record / Satellite extras ----------
    /// Scanner sweep direction: "up" | "down".
    #[serde(default)]
    pub scan_direction: String,
    /// Recorder file format: "wav" | "raw".
    #[serde(default)]
    pub record_format: String,
    /// Recorder auto-split size in MB (0 = no split).
    #[serde(default)]
    pub record_split_mb: u32,
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
fn default_fft_hold() -> f32 {
    0.5
}
fn default_smoothing_enabled() -> bool {
    true
}
fn default_smoothing_speed() -> f32 {
    0.3
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
            fft_hold: default_fft_hold(),
            smoothing_enabled: default_smoothing_enabled(),
            smoothing_speed: default_smoothing_speed(),
            fast_fft: false,
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
            sat_elevation_offset: 0.0,
            sat_azimuth_offset: 0.0,
            map_zoom: 1.0,
            pass_lead_min: 5,
        }
    }
}

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
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SourcePreferences {
    pub mode: String,
    pub daemon_address: String,
    /// Daemon credential; password UI and Debug output redact it.
    pub daemon_token: String,
    pub replay_file: Option<String>,
    pub replay_loop: bool,
    pub replay_speed: f32,
    pub replay_cf32_amplitude: Option<lrpt_decode::Cf32Amplitude>,
}

impl std::fmt::Debug for SourcePreferences {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourcePreferences")
            .field("mode", &self.mode)
            .field("daemon_address", &self.daemon_address)
            .field("daemon_token", &"[REDACTED]")
            .field("replay_file", &self.replay_file)
            .field("replay_loop", &self.replay_loop)
            .field("replay_speed", &self.replay_speed)
            .field("replay_cf32_amplitude", &self.replay_cf32_amplitude)
            .finish()
    }
}

impl Default for SourcePreferences {
    fn default() -> Self {
        Self {
            mode: "demo".into(),
            daemon_address: "127.0.0.1:7890".into(),
            daemon_token: String::new(),
            replay_file: None,
            replay_loop: false,
            replay_speed: 1.0,
            replay_cf32_amplitude: None,
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
            daemon_token: source.daemon_token.clone(),
            replay_file: source.replay_file.clone(),
            replay_loop: source.replay_loop,
            replay_speed: source.replay_speed,
            replay_cf32_amplitude: source.replay_cf32_amplitude,
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
        source.daemon_token = self.daemon_token.clone();
        source.replay_file = self.replay_file.clone();
        source.replay_loop = self.replay_loop;
        source.replay_cf32_amplitude = self.replay_cf32_amplitude;
        source.replay_speed = if self.replay_speed.is_finite() && self.replay_speed > 0.0 {
            self.replay_speed
        } else {
            1.0
        };
    }
}

/// Top-level application configuration persisted to `ez_sdr_config.json`.
///
/// Contains SDR and integration settings plus legacy theme fields preserved
/// for existing JSON files.
#[derive(Clone, Serialize, Deserialize)]
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
    /// Legacy UI theme label retained for older settings files.
    pub theme: String,
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
    #[serde(skip)]
    pub needs_apply: bool,
    /// Persistence errors are visible for this session, never saved as settings.
    #[serde(skip)]
    pub save_error: Option<String>,
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
    pub secondary_vfo: crate::secondary_vfo::SecondaryVfoSettings,
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
    /// Whether to skip the antenna-setup checklist on startup.
    pub skip_antenna_checklists: bool,
    /// Legacy custom theme records retained when reading older configurations.
    pub custom_themes: Vec<NamedTheme>,
    /// Advanced / experimental tunables applied to receiver engines.
    pub advanced: AdvancedConfig,
    /// Each demodulator keeps its own radio module controls, like SDR++.
    pub radio_profiles: std::collections::BTreeMap<String, RadioModeProfile>,
    /// Whether the initial quick start wizard has been completed.
    pub quick_start_completed: bool,
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field("version", &self.version)
            .field("default_freq_hz", &self.default_freq_hz)
            .field("default_sample_rate", &self.default_sample_rate)
            .field("theme", &self.theme)
            .field("source_preferences", &self.source_preferences)
            .finish_non_exhaustive()
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: "0.1.0".to_string(),
            default_freq_hz: 100_000_000,
            default_sample_rate: 2_048_000,
            default_gain: 40.0,
            output_directory: "./recordings".to_string(),
            save_error: None,
            theme: "dark".to_string(),
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
            secondary_vfo: crate::secondary_vfo::SecondaryVfoSettings::default(),
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
            skip_antenna_checklists: false,
            custom_themes: Vec::new(),
            advanced: AdvancedConfig::default(),
            radio_profiles: std::collections::BTreeMap::new(),
            quick_start_completed: false,
        }
    }
}

impl AppConfig {
    /// Serialize settings for export. The daemon credential is omitted by default
    /// so importing a redacted file can preserve the value stored on this device.
    pub(crate) fn export_json(
        &self,
        include_credentials: bool,
    ) -> Result<Vec<u8>, serde_json::Error> {
        let mut value = serde_json::to_value(self)?;
        if !include_credentials {
            if let Some(object) = value.as_object_mut() {
                if let Some(source) = object
                    .get_mut("source_preferences")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    source.remove("daemon_token");
                }
            }
        }
        serde_json::to_vec_pretty(&value)
    }

    pub(crate) fn export_to(
        &self,
        path: impl AsRef<std::path::Path>,
        include_credentials: bool,
    ) -> std::io::Result<()> {
        let json = self
            .export_json(include_credentials)
            .map_err(std::io::Error::other)?;
        if include_credentials {
            crate::bookmarks::atomic_write_private_file(path, &json)
        } else {
            crate::bookmarks::atomic_write_file(path, &json)
        }
    }

    /// Import settings while keeping the daemon credential when its key is omitted.
    /// An explicit empty string remains a deliberate request to clear it.
    pub(crate) fn import_json_preserving_credentials(
        json: &str,
        current: &Self,
    ) -> Result<Self, serde_json::Error> {
        let mut value: serde_json::Value = serde_json::from_str(json)?;
        if let Some(object) = value.as_object_mut() {
            let source = object
                .entry("source_preferences")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(source) = source.as_object_mut() {
                source.entry("daemon_token").or_insert_with(|| {
                    serde_json::Value::String(current.source_preferences.daemon_token.clone())
                });
            }
        }
        serde_json::from_value(value)
    }

    /// Load configuration from `ez_sdr_config.json`, or return defaults if the file does not exist or cannot be parsed.
    pub fn load_or_default() -> Self {
        Self::load_from("ez_sdr_config.json")
    }

    pub(crate) fn load_from(path: impl AsRef<std::path::Path>) -> Self {
        let mut cfg: AppConfig = std::fs::read_to_string(path)
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
        self.ppm_correction = self.ppm_correction.clamp(-100, 100);
        // Older profiles have duplicate display fields. Nondefault advanced
        // values are authoritative; otherwise migrate the older aliases once.
        let a = &mut self.advanced;
        let d = &defaults.advanced;
        if (a.db_min, a.db_max) == (d.db_min, d.db_max)
            && (self.spectrum_min_db, self.spectrum_max_db)
                != (defaults.spectrum_min_db, defaults.spectrum_max_db)
        {
            a.db_min = self.spectrum_min_db;
            a.db_max = self.spectrum_max_db;
        }
        if a.peak_hold_time == d.peak_hold_time && a.fft_hold != d.fft_hold {
            a.peak_hold_time = a.fft_hold;
        }
        if a.avg_alpha == d.avg_alpha && a.smoothing_speed != d.smoothing_speed {
            a.avg_alpha = 1.0 - a.smoothing_speed;
        }
        fn finite(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            }
        }
        if !crate::spectrum::SpectrumAnalyzer::valid_fft_size(a.fft_size) {
            a.fft_size = d.fft_size;
        }
        a.fft_rate = a.fft_rate.clamp(1, 120);
        a.wf_depth = a.wf_depth.clamp(32, 4096);
        a.wf_speed = a.wf_speed.max(1);
        a.window = crate::advanced_panel::window_from_str(&a.window)
            .name()
            .into();
        self.color_map = crate::advanced_panel::color_from_str(&self.color_map)
            .name()
            .into();
        a.snr_smoothing_secs = finite(a.snr_smoothing_secs, d.snr_smoothing_secs, 0.01, 10.0);
        a.peak_hold_time = finite(a.peak_hold_time, d.peak_hold_time, 0.0, 60.0);
        a.fft_hold = a.peak_hold_time;
        a.avg_alpha = finite(a.avg_alpha, d.avg_alpha, 0.0, 1.0);
        a.smoothing_speed = 1.0 - a.avg_alpha;
        a.persistence = finite(a.persistence, d.persistence, 0.0, 0.98);
        a.db_min = finite(a.db_min, d.db_min, -240.0, 10.0);
        a.db_max = finite(a.db_max, d.db_max, a.db_min + 10.0, 60.0);
        self.spectrum_min_db = a.db_min;
        self.spectrum_max_db = a.db_max;
        self.wf_min_db = finite(self.wf_min_db, defaults.wf_min_db, -240.0, 10.0);
        self.wf_max_db = finite(
            self.wf_max_db,
            defaults.wf_max_db,
            self.wf_min_db + 1.0,
            60.0,
        );
    }

    /// Serialise and write the configuration atomically to `ez_sdr_config.json`.
    /// Return whether persistence succeeded and retain failures for the GUI.
    pub fn save(&mut self) -> bool {
        self.save_with_feedback("ez_sdr_config.json")
    }

    pub(crate) fn save_with_feedback(&mut self, path: impl AsRef<std::path::Path>) -> bool {
        let path = path.as_ref();
        self.save_error = self.save_to(path).err().map(|error| {
            let location = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            };
            format!(
                "Could not save settings to {}: {error}. Changes are only kept for this session.",
                location.display()
            )
        });
        if let Some(error) = &self.save_error {
            eprintln!("[config] {error}");
        }
        self.save_error.is_none()
    }

    pub(crate) fn save_to(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::bookmarks::atomic_write_private_file(path, json.as_bytes())
    }
}

type ConfigJobReceiver =
    std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Result<Option<AppConfig>, String>>>>;

#[cfg(test)]
pub(crate) const SETTINGS_SOURCE_HEADER_TEST_ID: &str = "settings.source.header_test_id";

#[derive(Clone)]
struct ConfigFileJob(ConfigJobReceiver);

fn config_file_job_id(session_id: u64) -> egui::Id {
    egui::Id::new(("config.file_job", session_id))
}

fn config_file_message_id(session_id: u64) -> egui::Id {
    egui::Id::new(("config.file_message", session_id))
}

fn config_export_private_id(session_id: u64) -> egui::Id {
    egui::Id::new(("config.export.include_credentials", session_id))
}

fn begin_config_file_job(
    ctx: &egui::Context,
    session_id: u64,
    work: impl FnOnce() -> Result<Option<AppConfig>, String> + Send + 'static,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let id = config_file_job_id(session_id);
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

/// Ephemeral state for the Settings popup. Drafts and file-dialog jobs are kept
/// outside `SharedState::config`, so rendering Settings cannot replace the live
/// configuration halfway through an application frame.
#[derive(Default)]
pub struct SettingsEditor {
    draft: Option<AppConfig>,
    session_id: u64,
}

impl SettingsEditor {
    pub(crate) fn ensure_draft(&mut self, current: &AppConfig) {
        if self.draft.is_some() {
            return;
        }
        self.session_id = self.session_id.wrapping_add(1).max(1);
        let mut draft = current.clone();
        draft.needs_apply = false;
        draft.save_error = None;
        draft.normalize();
        self.draft = Some(draft);
    }

    pub(crate) fn draft(&self) -> Option<&AppConfig> {
        self.draft.as_ref()
    }

    pub(crate) fn draft_mut(&mut self) -> Option<&mut AppConfig> {
        self.draft.as_mut()
    }

    pub(crate) fn session_id(&self) -> u64 {
        self.session_id
    }

    pub(crate) fn discard(&mut self, ctx: &egui::Context) {
        ctx.data_mut(|data| {
            data.remove::<ConfigFileJob>(config_file_job_id(self.session_id));
            data.remove::<String>(config_file_message_id(self.session_id));
            data.remove::<bool>(config_export_private_id(self.session_id));
        });
        self.draft = None;
        self.session_id = self.session_id.wrapping_add(1).max(1);
    }

    #[cfg(test)]
    pub(crate) fn inject_import_result(
        &self,
        ctx: &egui::Context,
        result: Result<Option<AppConfig>, String>,
    ) {
        inject_config_file_result(ctx, self.session_id, result);
    }

    #[cfg(test)]
    pub(crate) fn inject_import_result_for_session(
        &self,
        ctx: &egui::Context,
        session_id: u64,
        result: Result<Option<AppConfig>, String>,
    ) {
        inject_config_file_result(ctx, session_id, result);
    }
}

#[cfg(test)]
fn inject_config_file_result(
    ctx: &egui::Context,
    session_id: u64,
    result: Result<Option<AppConfig>, String>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(result).expect("test receiver is alive");
    ctx.data_mut(|data| {
        data.insert_temp(
            config_file_job_id(session_id),
            ConfigFileJob(std::sync::Arc::new(std::sync::Mutex::new(rx))),
        )
    });
}

impl AppConfig {
    pub(crate) fn save_and_apply(&mut self, path: impl AsRef<std::path::Path>) -> bool {
        self.normalize();
        let saved = self.save_with_feedback(path);
        self.needs_apply = true;
        saved
    }

    pub(crate) fn reset_to_defaults(&mut self) {
        *self = Self::default();
        self.needs_apply = true;
    }

    /// Render the egui-based settings panel UI.
    pub fn ui(&mut self, ui: &mut egui::Ui, session_id: u64, save_path: &std::path::Path) -> bool {
        ui.heading("Settings");
        if let Some(error) = &self.save_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let job_id = config_file_job_id(session_id);
        let message_id = config_file_message_id(session_id);
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
            let _source_section = ui.collapsing("Source", |ui| {
                ui.add(egui::Slider::new(&mut self.default_freq_hz, 500_000..=1_770_000_000)
                    .text("Default frequency")
                    .custom_formatter(|v, _| format!("{:.3} MHz", v / 1e6)))
                    .on_hover_text("Startup frequency when no saved session is available. Save & Apply preserves the active receiver; use the Radio controls to tune it.");
                ui.add(egui::Slider::new(&mut self.default_sample_rate, 225_001..=3_200_000)
                    .text("Default sample rate")
                    .custom_formatter(|v, _| format!("{:.3} MSps", v / 1e6)))
                    .on_hover_text("Startup sample rate. Use the Radio source controls to change the active capture rate.");
                ui.add(egui::Slider::new(&mut self.default_gain, 0.0..=49.6)
                    .step_by(0.1)
                    .text("Default gain (dB)")
                    .custom_formatter(|v, _| format!("{v:.1} dB")))
                    .on_hover_text("Startup gain when no saved session is available. Use the Radio source controls to change live RF amplification.");
            });
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(SETTINGS_SOURCE_HEADER_TEST_ID),
                    _source_section.header_response.id,
                );
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
                    .on_hover_text("Starts a local HTTP server on this computer only (127.0.0.1). It is not available to other devices on your LAN.");
                ui.add(egui::Slider::new(&mut self.web_remote_port, 1024..=65535).text("Port"))
                    .on_hover_text("TCP port for the web remote. Default 5259. Open http://localhost:5259 in a browser.");
            });

            ui.collapsing("Rigctl Server", |ui| {
                ui.checkbox(&mut self.rigctl_enabled, "Enable loopback rigctl")
                    .on_hover_text("Expose the Hamlib/rigctld-compatible control socket on localhost only.");
                ui.add(egui::Slider::new(&mut self.rigctl_port, 1024..=65535).text("Port"))
                    .on_hover_text("TCP port for rigctld clients. Default 4532.");
            });

            ui.collapsing("Satellite Observer Location", |ui| {
                ui.add(egui::Slider::new(&mut self.observer_lat, -90.0..=90.0).text("Latitude"))
                    .on_hover_text("Your latitude in decimal degrees. North positive. Used to predict satellite pass times.");
                ui.add(egui::Slider::new(&mut self.observer_lon, -180.0..=180.0).text("Longitude"))
                    .on_hover_text("Your longitude in decimal degrees. East positive. Used together with latitude to compute pass elevation angles.");
            });

            ui.add_space(8.0);
            let include_credentials_id = config_export_private_id(session_id);
            let mut include_credentials = ui
                .ctx()
                .data_mut(|data| data.get_temp::<bool>(include_credentials_id))
                .unwrap_or(false);
            ui.checkbox(&mut include_credentials, "Include credentials in private backup")
                .on_hover_text("Includes API keys and tokens. The exported file will be owner-only on Unix; store it securely.");
            ui.ctx().data_mut(|data| {
                data.insert_temp(include_credentials_id, include_credentials);
            });
            ui.horizontal(|ui| {
                if ui.button("💾 Save & Apply").on_hover_text("Save settings to ez_sdr_config.json and apply them immediately.").clicked() {
                    self.save_and_apply(save_path);
                }
                if ui.button("Reset to defaults").on_hover_text("Restore all settings to factory defaults. Does not delete saved recordings.").clicked() {
                    self.reset_to_defaults();
                }
                let export_label = if include_credentials {
                    "📤 Export private backup…"
                } else {
                    "📤 Export redacted…"
                };
                if ui.add_enabled(job.is_none(), egui::Button::new(export_label)).on_hover_text("Choose a destination for a redacted settings export, or opt in above to include credentials in a private backup.").clicked() {
                    let config = self.clone();
                    let export_private = include_credentials;
                    begin_config_file_job(ui.ctx(), session_id, move || {
                        let Some(path) = rfd::FileDialog::new()
                            .set_file_name(if export_private { "ez_sdr_private_config_backup.json" } else { "ez_sdr_config_backup.json" })
                            .add_filter("JSON", &["json"])
                            .save_file() else { return Ok(None); };
                        config.export_to(&path, export_private)
                            .map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
                        Ok(None)
                    });
                }
                if ui.add_enabled(job.is_none(), egui::Button::new("📥 Import…")).on_hover_text("Load config from a previously exported JSON file.").clicked() {
                    let current = self.clone();
                    begin_config_file_job(ui.ctx(), session_id, move || {
                        let Some(path) = rfd::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .pick_file() else { return Ok(None); };
                        let data = std::fs::read_to_string(&path)
                            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
                        AppConfig::import_json_preserving_credentials(&data, &current).map(Some)
                            .map_err(|e| format!("Invalid settings in {}: {e}", path.display()))
                    });
                }
            });
            ui.colored_label(
                egui::Color32::GRAY,
                format!("Settings file: {}. Spectrum dB range is saved with Ctrl+S.", save_path.display()),
            );
        });
        let apply = self.needs_apply;
        self.needs_apply = false;
        apply
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn new_receiver_settings_roundtrip_and_legacy_defaults_are_safe() {
        let mut config = super::AppConfig::default();
        config.source_preferences.daemon_token = "private-test-token".into();
        config.secondary_vfo.enabled = true;
        config.secondary_vfo.frequency_hz = 101_100_000;
        let serialized = serde_json::to_string(&config).unwrap();
        let restored: super::AppConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.secondary_vfo, config.secondary_vfo);
        assert_eq!(
            restored.source_preferences.daemon_token,
            "private-test-token"
        );
        assert!(!format!("{:?}", restored).contains("private-test-token"));
        let mut legacy = serde_json::to_value(&config).unwrap();
        legacy.as_object_mut().unwrap().remove("secondary_vfo");
        legacy["source_preferences"]
            .as_object_mut()
            .unwrap()
            .remove("daemon_token");
        let restored: super::AppConfig = serde_json::from_value(legacy).unwrap();
        assert!(!restored.secondary_vfo.enabled);
        assert!(restored.source_preferences.daemon_token.is_empty());
    }

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
    fn config_default_has_theme_and_service_defaults() {
        let cfg = AppConfig::default();
        // theme_config should have the default preset
        assert_eq!(cfg.theme_config.preset, "dark");
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
        assert_eq!(deserialized.rigctl_enabled, cfg.rigctl_enabled);
        assert_eq!(deserialized.rigctl_port, cfg.rigctl_port);
    }

    #[test]
    fn queued_apply_flag_is_transient_and_never_serialized() {
        let config = AppConfig {
            needs_apply: true,
            ..AppConfig::default()
        };
        let encoded = serde_json::to_value(&config).unwrap();
        assert!(encoded.get("needs_apply").is_none());
        let decoded: AppConfig = serde_json::from_value(encoded).unwrap();
        assert!(!decoded.needs_apply);
    }

    #[test]
    fn config_load_or_default_nonexistent_returns_default() {
        let directory =
            std::env::temp_dir().join(format!("ez_sdr_missing_config_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let cfg = AppConfig::load_from(directory.join("does-not-exist.json"));
        assert_eq!(cfg.default_freq_hz, 100_000_000);
        assert_eq!(cfg.theme, "dark");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn config_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ez_sdr_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ez_sdr_config.json");

        let cfg = AppConfig {
            default_freq_hz: 433_000_000,
            theme: "light".to_string(),
            output_directory: "/tmp/recordings".to_string(),
            observer_lat: 40.7128,
            observer_lon: -74.0060,
            ..Default::default()
        };
        cfg.save_to(&path).unwrap();

        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.default_freq_hz, 433_000_000);
        assert_eq!(loaded.theme, "light");
        assert_eq!(loaded.output_directory, "/tmp/recordings");
        assert!((loaded.observer_lat - 40.7128).abs() < f64::EPSILON);
        assert!((loaded.observer_lon - (-74.0060)).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn redacted_exports_omit_credentials_and_import_keeps_existing_values() {
        let current = AppConfig {
            source_preferences: SourcePreferences {
                daemon_token: "daemon-token-sentinel".into(),
                ..SourcePreferences::default()
            },
            theme: "light".into(),
            ..AppConfig::default()
        };

        let exported = current.export_json(false).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&exported).unwrap();
        assert!(value["source_preferences"].get("daemon_token").is_none());
        let contents = String::from_utf8(exported).unwrap();
        assert!(!contents.contains("daemon-token-sentinel"));

        let imported = AppConfig::import_json_preserving_credentials(&contents, &current).unwrap();
        assert_eq!(imported.theme, "light");
        assert_eq!(
            imported.source_preferences.daemon_token,
            current.source_preferences.daemon_token
        );
    }

    #[test]
    fn private_backup_roundtrips_credentials_and_debug_redacts_them() {
        let current = AppConfig {
            source_preferences: SourcePreferences {
                daemon_token: "daemon-token-sentinel".into(),
                ..SourcePreferences::default()
            },
            ..AppConfig::default()
        };
        let exported = current.export_json(true).unwrap();
        let contents = String::from_utf8(exported).unwrap();
        let imported =
            AppConfig::import_json_preserving_credentials(&contents, &AppConfig::default())
                .unwrap();
        assert_eq!(
            imported.source_preferences.daemon_token,
            current.source_preferences.daemon_token
        );

        let debug = format!("{current:?}");
        assert!(!debug.contains("daemon-token-sentinel"));
    }

    #[test]
    fn import_with_explicit_empty_credentials_clears_them() {
        let current = AppConfig {
            source_preferences: SourcePreferences {
                daemon_token: "daemon-token-sentinel".into(),
                ..SourcePreferences::default()
            },
            ..AppConfig::default()
        };
        let imported = AppConfig::import_json_preserving_credentials(
            r#"{"source_preferences":{"daemon_token":""}}"#,
            &current,
        )
        .unwrap();
        assert!(imported.source_preferences.daemon_token.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn saved_config_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "ez-settings-private-{}-{}",
            std::process::id(),
            ez_proto::new_capture_epoch()
        ));
        let path = directory.join("settings.json");
        AppConfig::default().save_to(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_config_export_is_owner_only_and_keeps_credentials() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "ez-private-export-{}-{}",
            std::process::id(),
            ez_proto::new_capture_epoch()
        ));
        let path = directory.join("private-backup.json");
        let config = AppConfig {
            source_preferences: SourcePreferences {
                daemon_token: "private-daemon-token".into(),
                ..SourcePreferences::default()
            },
            ..AppConfig::default()
        };
        config.export_to(&path, true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("private-daemon-token"));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_save_is_visible_and_a_successful_retry_clears_it() {
        let directory = std::env::temp_dir().join(format!(
            "ez-settings-save-error-{}-{}",
            std::process::id(),
            ez_proto::new_capture_epoch()
        ));
        let path = directory.join("settings.json");
        // A directory at the destination forces a real atomic rename failure,
        // independently of root privileges or platform permission emulation.
        std::fs::create_dir_all(&path).unwrap();
        let mut config = AppConfig {
            theme: "light".into(),
            ..Default::default()
        };
        assert!(!config.save_with_feedback(&path));
        let message = config.save_error.as_ref().unwrap();
        assert!(message.contains("Could not save settings"));
        assert!(message.contains("settings.json"));
        assert!(message.contains("only kept for this session"));
        assert!(serde_json::to_value(&config)
            .unwrap()
            .get("save_error")
            .is_none());

        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            config.ui(ui, 1, &path);
        });
        fn contains_error(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text().contains("Could not save settings"),
                egui::Shape::Vec(shapes) => shapes.iter().any(contains_error),
                _ => false,
            }
        }
        assert!(output
            .shapes
            .iter()
            .any(|shape| contains_error(&shape.shape)));
        std::fs::remove_dir(&path).unwrap();
        assert!(config.save_with_feedback(&path));
        assert!(config.save_error.is_none());
        assert_eq!(AppConfig::load_from(&path).theme, "light");
        std::fs::remove_dir_all(directory).unwrap();
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
    }

    #[test]
    fn config_default_has_empty_custom_theme_gallery() {
        let cfg = AppConfig::default();
        assert!(cfg.custom_themes.is_empty());
    }

    #[test]
    fn config_missing_custom_themes_fall_back_to_defaults() {
        let json = r#"{"default_freq_hz": 144000000}"#;
        let cfg: AppConfig = serde_json::from_str(json).expect("partial deserialize");
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
    fn config_keeps_legacy_theme_fields_readable() {
        let theme = serde_json::to_value(ThemeConfig::light()).unwrap();
        let legacy_json = serde_json::json!({
            "ai_api_key": "ignored-legacy-key",
            "custom_themes": [{
                "id": "legacy-theme",
                "name": "Legacy Theme",
                "theme": theme
            }]
        });
        let loaded: AppConfig = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(loaded.custom_themes[0].name, "Legacy Theme");
        assert!(serde_json::to_value(&loaded)
            .unwrap()
            .get("ai_api_key")
            .is_none());
        let roundtrip: AppConfig =
            serde_json::from_value(serde_json::to_value(loaded).unwrap()).unwrap();
        assert_eq!(roundtrip.custom_themes[0].theme.preset, "light");
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
        let json = r#"{"version": "0.1.0", "theme": "", "output_directory": ""}"#;
        let cfg: AppConfig =
            serde_json::from_str(json).expect("config empty strings JSON should deserialize");
        assert_eq!(cfg.theme, "");
        assert_eq!(cfg.output_directory, "");
    }
    #[test]
    fn saved_display_normalization_migrates_aliases_and_rejects_invalid_values() {
        let mut legacy = AppConfig {
            spectrum_min_db: -93.0,
            spectrum_max_db: -3.0,
            ..Default::default()
        };
        legacy.advanced.fft_hold = 3.0;
        legacy.advanced.smoothing_speed = 0.8;
        legacy.normalize();
        assert_eq!(
            (legacy.advanced.db_min, legacy.advanced.db_max),
            (-93.0, -3.0)
        );
        assert_eq!(legacy.advanced.peak_hold_time, 3.0);
        assert!((legacy.advanced.avg_alpha - 0.2).abs() < 1e-6);
        let before = legacy.advanced.clone();
        legacy.normalize();
        assert_eq!(legacy.advanced, before);

        let mut invalid = AppConfig {
            ppm_correction: 10_000,
            color_map: "missing".into(),
            wf_min_db: f32::NAN,
            wf_max_db: f32::INFINITY,
            ..Default::default()
        };
        let a = &mut invalid.advanced;
        a.fft_size = usize::MAX;
        a.fft_rate = 0;
        a.wf_depth = usize::MAX;
        a.wf_speed = 0;
        a.window = "missing".into();
        a.snr_smoothing_secs = f32::NAN;
        a.peak_hold_time = f32::NAN;
        a.avg_alpha = f32::NAN;
        a.persistence = f32::INFINITY;
        a.db_min = f32::NAN;
        a.db_max = f32::NEG_INFINITY;
        invalid.normalize();
        let a = &invalid.advanced;
        assert_eq!(
            (a.fft_size, a.fft_rate, a.wf_depth, a.wf_speed),
            (2048, 1, 4096, 1)
        );
        assert_eq!(
            (
                a.window.as_str(),
                invalid.color_map.as_str(),
                invalid.ppm_correction
            ),
            ("Hann", "Classic", 100)
        );
        for value in [
            a.snr_smoothing_secs,
            a.peak_hold_time,
            a.avg_alpha,
            a.persistence,
            a.db_min,
            a.db_max,
            invalid.wf_min_db,
            invalid.wf_max_db,
        ] {
            assert!(value.is_finite());
        }
        assert!(a.db_max >= a.db_min + 10.0);
        let encoded = serde_json::to_vec(&invalid).unwrap();
        let mut decoded: AppConfig = serde_json::from_slice(&encoded).unwrap();
        decoded.normalize();
        assert_eq!(decoded.advanced, invalid.advanced);
    }
}
