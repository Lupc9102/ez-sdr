use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct FreqMemEntry {
    pub freq_hz: u64,
    pub label: String,
}

use crate::adsb_decoder::AdsBDecoder;
use crate::adsb_panel::AdsBPanel;
use crate::ai_panel::AiPanel;
use crate::audio_output::AudioOutput;
use crate::bookmarks::BookmarkDb;
use crate::config::AppConfig;
use crate::constellation::ConstellationDisplay;
use crate::decoding_panel::DecodingPanel;
use crate::demod::{DemodConfig, DemodWorker};
use crate::discord::DiscordNotifier;
use crate::discord_panel::DiscordPanel;
use crate::howto_panel::HowToPanel;
use crate::mqtt::MqttPublisher;
use crate::recorder_panel::RecorderPanel;
use crate::rigctl::{RigctlRequest, RigctlServer};
use crate::satellite_panel::SatellitePanel;
use crate::scheduler::Scheduler;
use crate::source_manager::SourceManager;
use crate::spectrum::SpectrumAnalyzer;
use crate::tle_engine::TleEngine;
use crate::web_remote::{RemoteCommand, WebRemote};

pub use crate::mode_bar::{AppTab, SecondaryTool, ALL_SECONDARY_TOOLS, MAIN_TABS};

/// Scanner configuration request produced outside the scanner itself (the AI
/// panel's `configure_scanner` tool call). Fields left `None` keep the
/// scanner's current value; the app loop drains and applies it every frame.
#[derive(Debug, Clone, Default)]
pub struct ScannerCommand {
    pub start_hz: Option<u64>,
    pub stop_hz: Option<u64>,
    pub step_hz: Option<u64>,
    pub dwell_ms: Option<u64>,
    pub threshold_db: Option<f32>,
    /// `Some(true)` starts the sweep after applying settings, `Some(false)` stops it.
    pub run: Option<bool>,
}

pub struct SharedState {
    pub source: SourceManager,
    pub spectrum: SpectrumAnalyzer,
    pub config: AppConfig,
    pub bookmarks: BookmarkDb,
    pub scheduler: Scheduler,
    pub tle: TleEngine,
    pub demod_mode: crate::sdr_panel::DemodMode,
    pub recording: bool,
    pub adsb_running: bool,
    pub selected_satellite: Option<String>,
    pub audio_running: bool,
    pub volume: f32,
    pub squelch: f32,
    pub lpf_cutoff: f32,
    pub fm_deviation_hz: f32,
    pub audio_peak: f32,
    pub freq_history: VecDeque<u64>,
    pub vfo_b: u64,
    pub freq_memory: [FreqMemEntry; 9],
    pub tune_step_fine_hz: u64,
    pub tune_step_coarse_hz: u64,
    pub lo_offset_hz: i64,
    pub mqtt_connected: bool,
    pub mqtt_enabled: bool,
    pub bookmarks_modified: bool,
    pub scanner_command: Option<ScannerCommand>,
}

pub struct CentralApp {
    shared: Arc<Mutex<SharedState>>,
    event_bus: crate::events::EventBus,
    keyboard_handler: crate::keyboard::KeyboardHandler,
    status_bar: crate::status_bar::StatusBar,
    frequency_history: crate::frequency_history::FrequencyHistory,
    radio_ui: crate::radio_ui::RadioUi,
    satellite_panel: SatellitePanel,
    adsb_panel: AdsBPanel,
    recorder_panel: RecorderPanel,
    constellation: ConstellationDisplay,
    decoding_panel: DecodingPanel,
    meteor_decoder: DecodingPanel,
    ai_panel: AiPanel,
    howto_panel: HowToPanel,
    web_remote: WebRemote,
    rigctl: RigctlServer,
    mqtt: MqttPublisher,
    demod_worker: DemodWorker,
    radio_iq: crate::radio_iq::RadioIqProcessor,
    vfo_mixer: crate::radio_iq::VfoMixer,
    last_radio_capture: Option<(u64, crate::source_manager::SourceMode, u64)>,
    last_radio_profile: Option<(crate::sdr_panel::DemodMode, crate::config::RadioModeProfile)>,
    ctcss: crate::radio_squelch::CtcssSquelch,
    rds: crate::radio_rds::RdsDecoder,
    daemon_audio: crate::audio_resampler::AudioResampler,
    audio: AudioOutput,
    adsb_decoder: AdsBDecoder,
    last_adsb_capture: Option<(u64, u32, crate::source_manager::SourceMode, u64, bool)>,
    scanner: crate::scanner::FrequencyScanner,
    last_scheduler_update: std::time::Instant,
    last_source_status: crate::source_manager::SourceStatus,
    last_auto_tuned_satellite: String,
    bookmark_panel: crate::bookmark_manager::BookmarkPanel,
    quick_start: crate::quick_start::QuickStartWizard,
    show_keyboard_help: bool,
    last_history_freq: u64,
    freq_history_idx: Option<usize>,
    recording_start: Option<std::time::Instant>,
    bm_last_len: usize,
    bm_dirty_since: Option<std::time::Instant>,
    scheduler_panel: crate::scheduler::SchedulerPanel,
    // Frequency jump dialog
    show_freq_jump: bool,
    freq_jump_input: String,
    freq_jump_matches: Vec<(String, u64)>,
    // Session notes
    session_notes: String,
    // SDR glossary popup
    show_glossary: bool,
    // First strong signal celebration
    first_strong_signal_seen: bool,
    // Track demod mode changes to reset demodulator and avoid clicks
    last_demod_mode: crate::sdr_panel::DemodMode,
    last_audio_source: Option<(
        u64,
        u64,
        crate::radio_iq::RadioIqConfig,
        crate::source_manager::SourceMode,
        u64,
    )>,
    theme_applied: bool,
    discord: DiscordNotifier,
    discord_panel: DiscordPanel,
    last_recording: bool,
    last_adsb_running: bool,
    last_scanner_enabled: bool,
    last_mqtt_connected: bool,
    seen_aircraft: std::collections::HashSet<u32>,
    last_active_pass_sat: String,
    discord_summary_last: std::time::Instant,
    current_tab: AppTab,
    last_traffic_bucket: usize,
    last_manual_tune_time: std::time::Instant,
    active_secondary_tool: Option<SecondaryTool>,
    /// 🤖 Ask — ambient AI slide-over, openable from every mode's bar button.
    ai_ask_open: bool,
    /// ⚙ More — whether the tool-picker menu (deck of hidden tools) is showing.
    #[allow(dead_code)]
    more_menu_open: bool,
    adsb_instructions_open: bool,
    satellite_subtab: crate::satellite_panel::SatelliteSubTab,
    customize_panel: crate::customize_panel::CustomizePanel,
}

impl CentralApp {
    /// Non-blocking acquisition of SharedState with poison recovery and event publishing.
    #[inline]
    pub fn try_lock_shared(
        &self,
        context: &'static str,
    ) -> Option<std::sync::MutexGuard<'_, SharedState>> {
        match self.shared.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::WouldBlock) => None,
            Err(std::sync::TryLockError::Poisoned(p)) => {
                eprintln!(
                    "[WARN] SharedState mutex recovered from poison in {}",
                    context
                );
                self.event_bus.publish(crate::events::AppEvent::Error {
                    context: context.to_string(),
                    message: "SharedState mutex recovered from poison".to_string(),
                });
                Some(p.into_inner())
            }
        }
    }

    /// Blocking acquisition of SharedState with error propagation.
    #[inline]
    pub fn lock_shared(
        &self,
        context: &'static str,
    ) -> crate::error::AppResult<std::sync::MutexGuard<'_, SharedState>> {
        self.shared
            .lock()
            .map_err(|_| crate::error::AppError::MutexPoisoned {
                context: context.to_string(),
            })
    }

    /// Process pending events from panels and event bus
    fn handle_events(&mut self) {
        let events = self.event_bus.drain();
        for event in events {
            match event {
                crate::events::AppEvent::FrequencyChanged { hz } => {
                    self.frequency_history.add(hz, None);
                    if let Ok(mut state) = self.shared.try_lock() {
                        if state.source.frequency_hz != hz {
                            crate::radio_ui::tune(&mut state, hz);
                        }
                    }
                }
                crate::events::AppEvent::RecordingStateChanged { recording } => {
                    if recording {
                        self.status_bar.info("⏺ Recording started".to_string());
                    } else {
                        self.status_bar.info("⏹ Recording stopped".to_string());
                    }
                }
                crate::events::AppEvent::ScannerStateChanged { enabled } => {
                    if enabled {
                        self.status_bar.info("🔍 Scanner: ON".to_string());
                    } else {
                        self.status_bar.info("🔍 Scanner: OFF".to_string());
                    }
                }
                crate::events::AppEvent::SpectrumRangeChanged { min_db, max_db } => {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.spectrum.set_display_range(min_db, max_db);
                    }
                }
                crate::events::AppEvent::DemodChanged { mode } => {
                    self.status_bar.info(format!("Mode: {}", mode));
                }
                crate::events::AppEvent::StatusMessage { text, severity } => match severity {
                    crate::events::MessageSeverity::Info => self.status_bar.info(text),
                    crate::events::MessageSeverity::Warning => self.status_bar.warning(text),
                    crate::events::MessageSeverity::Error => self.status_bar.error(text),
                },
                crate::events::AppEvent::Error { context, message } => {
                    self.status_bar
                        .error(format!("Error in {}: {}", context, message));
                }
                _ => {} // Other events handled by panels directly
            }
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_config(cc, AppConfig::load_or_default())
    }

    fn new_with_config(_cc: &eframe::CreationContext<'_>, mut config: AppConfig) -> Self {
        config.normalize();
        let mut spectrum = SpectrumAnalyzer::new();
        spectrum.load_signal_history();
        let shared = Arc::new(Mutex::new(SharedState {
            source: SourceManager::new(),
            spectrum,
            config,
            bookmarks: BookmarkDb::load_or_default(),
            scheduler: Scheduler::new(),
            tle: TleEngine::new(),
            demod_mode: crate::sdr_panel::DemodMode::Fm,
            recording: false,
            adsb_running: false,
            selected_satellite: None,
            audio_running: false,
            volume: 0.5,
            squelch: -100.0,
            lpf_cutoff: 15000.0,
            fm_deviation_hz: 0.0,
            audio_peak: 0.0,
            freq_history: VecDeque::with_capacity(20),
            vfo_b: 0,
            freq_memory: std::array::from_fn(|_| FreqMemEntry::default()),
            tune_step_fine_hz: 100_000,
            tune_step_coarse_hz: 1_000_000,
            lo_offset_hz: 0,
            mqtt_connected: false,
            mqtt_enabled: false,
            bookmarks_modified: true,
            scanner_command: None,
        }));

        let mut web_remote = WebRemote::new();
        let mut rigctl = RigctlServer::new();
        let mut mqtt = MqttPublisher::new();
        let mut discord = DiscordNotifier::new();
        {
            let state = shared.lock().expect("shared state mutex poisoned");
            if state.config.web_remote_enabled {
                web_remote.set_enabled(true, state.config.web_remote_port);
            }
            if state.config.rigctl_enabled {
                rigctl.set_enabled(true, state.config.rigctl_port);
            }
            if !state.config.mqtt_broker.is_empty() {
                mqtt.set_enabled(
                    true,
                    state.config.mqtt_broker.clone(),
                    state.config.mqtt_topic_prefix.clone(),
                );
            }
            discord.apply_settings(&state.config.discord);
        }

        // Restore controls without opening hardware or playing demo audio on launch.
        {
            let mut state = shared.lock().expect("shared state mutex poisoned");
            let preferences = state.config.source_preferences.clone();
            preferences.restore(&mut state.source);
            // Restore last session freq/gain/demod if available, else fall back to config defaults
            state.source.frequency_hz = if state.config.last_session_freq_hz > 0 {
                state.config.last_session_freq_hz
            } else {
                state.config.default_freq_hz
            };
            state.source.sample_rate_hz = state.config.default_sample_rate;
            state.source.center_frequency_hz = state
                .config
                .last_session_center_hz
                .or(Some(state.source.frequency_hz));
            state.source.gain_db = if state.config.last_session_gain_db >= 0.0 {
                state.config.last_session_gain_db
            } else {
                state.config.default_gain
            };
            if !state.config.last_session_demod.is_empty() {
                if let Some(mode) =
                    crate::sdr_panel::DemodMode::from_label(&state.config.last_session_demod)
                {
                    state.demod_mode = mode;
                }
            }
            state.source.ppm_correction = state.config.ppm_correction;
            state.lo_offset_hz = state.config.lo_offset_hz;
            state.source.frequency_offset_hz = state.lo_offset_hz;
            state.vfo_b = if state.config.vfo_b_hz > 0 {
                state.config.vfo_b_hz
            } else {
                state.config.default_freq_hz
            };
            // Restore frequency memory labels from config
            let saved_hz = state.config.freq_memory_hz.clone();
            let saved_labels = state.config.freq_memory_labels.clone();
            for (i, mem) in state.freq_memory.iter_mut().enumerate() {
                if let Some(&hz) = saved_hz.get(i) {
                    mem.freq_hz = hz;
                }
                if let Some(label) = saved_labels.get(i) {
                    if !label.is_empty() {
                        mem.label = label.clone();
                    }
                }
            }
            state.tle.observer_lat = state.config.observer_lat;
            state.tle.observer_lon = state.config.observer_lon;
            // Restore recent frequency history from config
            let saved_recent: Vec<u64> = state.config.recent_frequencies.clone();
            for hz in saved_recent {
                if hz > 0 {
                    state.freq_history.push_back(hz);
                }
            }
            // Restore saved spectrum dB range (non-default check avoids stomping on first run)
            if state.config.spectrum_min_db != 0.0 || state.config.spectrum_max_db != 0.0 {
                let min = state.config.spectrum_min_db;
                let max = state.config.spectrum_max_db;
                state.spectrum.set_display_range(min, max);
            }
            // Restore waterfall color range
            if state.config.wf_min_db != 0.0 || state.config.wf_max_db != 0.0 {
                state.spectrum.wf_min_db = state.config.wf_min_db;
                state.spectrum.wf_max_db = state.config.wf_max_db;
            }
            // Restore waterfall colormap
            if !state.config.color_map.is_empty() {
                state.spectrum.color_map = match state.config.color_map.as_str() {
                    "Viridis" => crate::spectrum::ColorMap::Viridis,
                    "Plasma" => crate::spectrum::ColorMap::Plasma,
                    "Magma" => crate::spectrum::ColorMap::Magma,
                    "Inferno" => crate::spectrum::ColorMap::Inferno,
                    "Turbo" => crate::spectrum::ColorMap::Turbo,
                    "Grayscale" => crate::spectrum::ColorMap::Grayscale,
                    "Hot" => crate::spectrum::ColorMap::Hot,
                    _ => crate::spectrum::ColorMap::Classic,
                };
            }
            let init_freq = state.source.frequency_hz;
            if state.freq_history.is_empty() || state.freq_history.back() != Some(&init_freq) {
                state.freq_history.push_back(init_freq);
            }
        }

        let mut app = Self {
            shared: shared.clone(),
            event_bus: crate::events::EventBus::new(),
            keyboard_handler: crate::keyboard::KeyboardHandler::new(),
            status_bar: crate::status_bar::StatusBar::new(),
            frequency_history: crate::frequency_history::FrequencyHistory::new(),
            radio_ui: crate::radio_ui::RadioUi::new(shared.clone()),
            satellite_panel: SatellitePanel::new(shared.clone()),
            adsb_panel: AdsBPanel::new(shared.clone()),
            recorder_panel: RecorderPanel::new(shared.clone()),
            constellation: ConstellationDisplay::new(),
            decoding_panel: DecodingPanel::default(),
            meteor_decoder: DecodingPanel::default(),
            ai_panel: AiPanel::new(shared.clone()),
            howto_panel: HowToPanel::new(),
            web_remote,
            rigctl,
            mqtt,
            discord,
            discord_panel: DiscordPanel::new(),
            radio_iq: crate::radio_iq::RadioIqProcessor::new(Default::default()),
            vfo_mixer: crate::radio_iq::VfoMixer::new(2_048_000.0),
            last_radio_capture: None,
            last_radio_profile: None,
            ctcss: crate::radio_squelch::CtcssSquelch::new(),
            rds: crate::radio_rds::RdsDecoder::new(),
            daemon_audio: crate::audio_resampler::AudioResampler::default(),
            audio: AudioOutput::new(),
            #[cfg(test)]
            demod_worker: crate::demod::DemodWorker::new_uninitialized(4),
            #[cfg(not(test))]
            // Keep several source blocks queued so a late GUI repaint does not
            // turn directly into a dropped audio frame.
            demod_worker: crate::demod::DemodWorker::new(16),
            adsb_decoder: AdsBDecoder::new(),
            last_adsb_capture: None,
            scanner: crate::scanner::FrequencyScanner::new(shared.clone()),
            last_scheduler_update: std::time::Instant::now(),
            last_source_status: crate::source_manager::SourceStatus::Idle,
            last_auto_tuned_satellite: String::new(),
            bookmark_panel: crate::bookmark_manager::BookmarkPanel::new(),
            quick_start: crate::quick_start::QuickStartWizard::new(),
            show_keyboard_help: false,
            last_history_freq: {
                let state = shared.lock().expect("shared state mutex poisoned");
                state.source.frequency_hz
            },
            freq_history_idx: None,
            scheduler_panel: crate::scheduler::SchedulerPanel::new(),
            show_freq_jump: false,
            freq_jump_input: String::new(),
            freq_jump_matches: Vec::new(),
            session_notes: String::new(),
            show_glossary: false,
            first_strong_signal_seen: false,
            last_demod_mode: crate::sdr_panel::DemodMode::Fm,
            last_audio_source: None,
            theme_applied: false,
            recording_start: None,
            bm_last_len: 0,
            bm_dirty_since: None,
            last_recording: false,
            last_adsb_running: false,
            last_scanner_enabled: false,
            last_mqtt_connected: false,
            seen_aircraft: std::collections::HashSet::new(),
            last_active_pass_sat: String::new(),
            current_tab: AppTab::Listen,
            discord_summary_last: std::time::Instant::now(),
            last_traffic_bucket: 0,
            last_manual_tune_time: std::time::Instant::now(),
            active_secondary_tool: None,
            ai_ask_open: false,
            more_menu_open: false,
            adsb_instructions_open: false,
            satellite_subtab: crate::satellite_panel::SatelliteSubTab::Track,
            customize_panel: crate::customize_panel::CustomizePanel::default(),
        };
        // Apply saved advanced settings onto the live engines so a fresh launch
        // reproduces the user's last profile.
        app.apply_advanced();
        app
    }
}

impl CentralApp {
    /// Push the saved advanced settings onto the demod worker.
    pub fn apply_advanced(&mut self) {
        let shared = self.shared.lock().expect("shared state mutex poisoned");
        let mode = shared.demod_mode.resolve(shared.source.frequency_hz);
        self.demod_worker
            .configure_advanced(&shared.config.advanced, mode);
    }

    /// Forward the current Advanced-panel DSP settings to the demod worker.
    /// `state` must be a live [`SharedState`] guard.
    fn sync_demod_advanced(&self, state: &SharedState) {
        let mode = state.demod_mode.resolve(state.source.frequency_hz);
        self.demod_worker
            .configure_advanced(&state.config.advanced, mode);
    }

    fn process_rigctl_requests(&mut self) {
        let requests = self.rigctl.poll_requests();
        for request in requests {
            match request {
                RigctlRequest::GetFrequency { reply } => {
                    let response = self
                        .shared
                        .try_lock()
                        .map(|state| format!("{}\n", state.source.frequency_hz))
                        .unwrap_or_else(|_| crate::rigctl::error_response(-5));
                    let _ = reply.send(response);
                }
                RigctlRequest::SetFrequency { hz, reply } => {
                    let response = if let Ok(mut state) = self.shared.try_lock() {
                        crate::radio_ui::tune(&mut state, hz);
                        crate::rigctl::ok_response()
                    } else {
                        crate::rigctl::error_response(-5)
                    };
                    let _ = reply.send(response);
                }
                RigctlRequest::GetMode { reply } => {
                    let response = self
                        .shared
                        .try_lock()
                        .map(|state| {
                            format!(
                                "{} {}\n",
                                state.demod_mode.label(),
                                crate::radio_ui::channel_bandwidth(&state).round() as u32
                            )
                        })
                        .unwrap_or_else(|_| crate::rigctl::error_response(-5));
                    let _ = reply.send(response);
                }
                RigctlRequest::SetMode {
                    mode,
                    bandwidth_hz,
                    reply,
                } => {
                    let response = if let Ok(mut state) = self.shared.try_lock() {
                        if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&mode) {
                            crate::radio_ui::select_demod(&mut state, mode);
                            if let Some(width) = bandwidth_hz {
                                state.config.advanced.radio_bandwidth_hz = width as f32;
                            }
                            crate::rigctl::ok_response()
                        } else {
                            crate::rigctl::error_response(-1)
                        }
                    } else {
                        crate::rigctl::error_response(-5)
                    };
                    let _ = reply.send(response);
                }
                RigctlRequest::GetVolume { reply } => {
                    let response = self
                        .shared
                        .try_lock()
                        .map(|state| format!("{:.1}\n", state.volume.clamp(0.0, 1.0) * 100.0))
                        .unwrap_or_else(|_| crate::rigctl::error_response(-5));
                    let _ = reply.send(response);
                }
                RigctlRequest::SetVolume { percent, reply } => {
                    let response = if let Ok(mut state) = self.shared.try_lock() {
                        state.volume = (percent / 100.0).clamp(0.0, 1.0);
                        crate::rigctl::ok_response()
                    } else {
                        crate::rigctl::error_response(-5)
                    };
                    let _ = reply.send(response);
                }
            }
        }
    }

    fn sync_adsb_capture(&mut self) {
        let capture = self.shared.try_lock().ok().map(|state| {
            (
                state.source.capture_center_frequency_hz(),
                state.source.sample_rate_hz,
                state.source.source_mode.clone(),
                state.source.stream_generation(),
                state.adsb_running,
            )
        });
        if let Some(capture) = capture {
            if self.last_adsb_capture.as_ref() != Some(&capture) {
                self.adsb_decoder = AdsBDecoder::new();
                self.seen_aircraft.clear();
                if self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090 {
                    self.adsb_panel.clear_tracks();
                }
                self.last_adsb_capture = Some(capture);
            }
        }
    }

    /// Drain queued daemon events (only produces events in `SourceMode::Daemon`)
    /// and apply them: spectrum frames feed the analyser, errors surface as a
    /// status flash. Hardware shadow fields are applied inside
    /// `SourceManager::recv_daemon_event`; audio and aircraft are routed here into the
    /// desktop's existing playback and tracking state.
    fn map_daemon_aircraft(
        item: ez_proto::AircraftTelemetry,
    ) -> Option<crate::adsb_panel::AircraftEntry> {
        let (lat, lon) = (item.lat?, item.lon?);
        if !lat.is_finite()
            || !lon.is_finite()
            || !(-90.0..=90.0).contains(&lat)
            || !(-180.0..=180.0).contains(&lon)
        {
            return None;
        }
        Some(crate::adsb_panel::AircraftEntry {
            icao: item.icao,
            callsign: item.callsign.unwrap_or_default(),
            lat,
            lon,
            altitude: item.altitude_ft.unwrap_or_default().max(0) as u32,
            speed: item.ground_speed_kt.unwrap_or_default().max(0.0) as u32,
            heading: item.track_deg.unwrap_or_default().rem_euclid(360.0) as u32,
            seen: std::time::Instant::now(),
        })
    }

    fn drain_daemon_events(&mut self) {
        let Ok(mut state) = self.shared.try_lock() else {
            return;
        };
        state.source.sync_daemon_controls();
        let audio_running = state.audio_running;
        let adsb_running = state.adsb_running;
        let demod_mode = state.demod_mode;
        state
            .source
            .sync_daemon_workflows(audio_running, adsb_running, false, demod_mode);
        for _ in 0..8 {
            match state.source.recv_daemon_event() {
                Some(ez_proto::ServerEvent::Spectrum(frame)) => {
                    // The Radio plot is the only consumer of daemon FFT
                    // frames. Drop hidden-tab frames so switching to ADS-B or
                    // Meteor does not keep doing display work in the
                    // background; the daemon will send a fresh frame when
                    // Radio becomes visible again.
                    if self.current_tab == AppTab::Listen {
                        state.spectrum.push_spectrum_frame(&frame);
                    }
                }
                Some(ez_proto::ServerEvent::Error { message }) => {
                    self.status_bar.warning(format!("⚠ {message}"));
                }
                Some(ez_proto::ServerEvent::Audio(frame)) => {
                    if state.audio_running {
                        let samples = self.daemon_audio.process(
                            &frame.samples,
                            frame.sample_rate_hz,
                            self.audio.sample_rate(),
                            state.volume,
                        );
                        let _ = self.audio.push_audio(samples);
                    }
                }
                Some(ez_proto::ServerEvent::Aircraft(aircraft))
                    if adsb_running
                        && self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090 =>
                {
                    self.adsb_panel.aircraft = aircraft
                        .into_iter()
                        .filter_map(Self::map_daemon_aircraft)
                        .collect();
                }
                Some(ez_proto::ServerEvent::Telemetry(frame)) => {
                    if let Err(message) = self.decoding_panel.ingest_daemon_telemetry(frame) {
                        self.status_bar.warning(format!("⚠ {message}"));
                    }
                }
                Some(ez_proto::ServerEvent::Recording(status)) => {
                    self.recorder_panel.handle_daemon_recording(status);
                }
                Some(_) => {}
                None => break,
            }
        }
    }
}

impl eframe::App for CentralApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.recorder_panel
            .set_audio_sample_rate(self.audio.sample_rate());
        if let Ok(mut state) = self.shared.try_lock() {
            crate::radio_ui::reconcile_tuning(&mut state);
        }
        let desired_audio = self.shared.try_lock().ok().map(|state| {
            let channels = if crate::radio_ui::stereo_audio_requested(&state) {
                2
            } else {
                1
            };
            (
                channels,
                (
                    state.source.frequency_hz,
                    state.source.capture_center_frequency_hz(),
                    crate::radio_ui::radio_iq_config(&state),
                    state.source.source_mode.clone(),
                    state.source.stream_generation(),
                ),
            )
        });
        let reset_audio = desired_audio.as_ref().is_some_and(|(channels, source)| {
            self.last_audio_source.as_ref() != Some(source)
                || (self.audio.is_running() && *channels != self.audio.input_channels())
        });
        if let Some((_, source)) = desired_audio {
            self.last_audio_source = Some(source);
        }
        if reset_audio {
            self.audio.shutdown();
            self.daemon_audio.reset();
            self.demod_worker.request_reset();
            self.ctcss.reset();
            self.rds.reset();
            self.radio_ui.rds = None;
            self.radio_ui.received_tone = None;
        }
        // In SourceMode::Daemon the local worker threads are idle; spectrum, hardware and
        // status all arrive over the network instead. Drain and apply those first (a no-op in
        // every other mode) so the frame below renders on the freshest daemon state.
        self.sync_adsb_capture();
        self.drain_daemon_events();

        // Process pending events from panels and event bus
        self.handle_events();
        self.status_bar.update();

        // Mode profiles also change via keyboard/bookmarks/remote commands.
        // Apply only when their signal-processing settings actually change.
        let profile_shared = Arc::clone(&self.shared);
        if let Ok(mut state) = profile_shared.try_lock() {
            crate::radio_ui::sync_demod_settings(&mut state);
            let profile = (
                state.demod_mode.resolve(state.source.frequency_hz),
                crate::config::RadioModeProfile::capture(&state.config.advanced),
            );
            if self.last_radio_profile.as_ref() != Some(&profile) {
                self.sync_demod_advanced(&state);
                self.last_radio_profile = Some(profile);
            }
        }

        // Drain samples from source and capture parameters in a single lock
        let mut sample_batch: Vec<Vec<u8>> = Vec::new();
        let source_params = {
            if let Some(mut state) = self.try_lock_shared("drain_samples") {
                crate::radio_ui::sync_demod_settings(&mut state);
                crate::radio_ui::reconcile_tuning(&mut state);
                state.spectrum.vfo_freq_hz = Some(state.source.frequency_hz);
                state.spectrum.vfo_bw_hz = crate::radio_ui::channel_bandwidth(&state) as u32;
                state.spectrum.demod_mode = state
                    .demod_mode
                    .resolve(state.source.frequency_hz)
                    .label()
                    .into();
                // RTL-SDR delivers an 8 ms block at the default 2.048 MS/s.
                // Drain ahead of the repaint cadence and allow a delayed frame
                // to catch up instead of dropping input and producing a click.
                for _ in 0..8 {
                    if let Some(samples) = state.source.recv_samples() {
                        sample_batch.push(samples);
                    } else {
                        break;
                    }
                }
                Some((
                    state.source.frequency_hz,
                    state.source.capture_center_frequency_hz(),
                    state.source.sample_rate_hz,
                    crate::radio_ui::radio_iq_config(&state),
                    state.source.source_mode.clone(),
                    state.source.stream_generation(),
                    state.demod_mode,
                    matches!(
                        state.source.status,
                        crate::source_manager::SourceStatus::Running
                            | crate::source_manager::SourceStatus::Opening
                    ),
                    state.audio_running,
                    state.volume,
                    state.squelch,
                    state.config.advanced.squelch_mode,
                    state.config.advanced.ctcss_tone_hz,
                    (
                        state.config.advanced.rds_enabled
                            && state.source.source_mode
                                != crate::source_manager::SourceMode::Daemon,
                        state.config.advanced.rds_incremental,
                        state.config.advanced.rds_region,
                    ),
                    state.lpf_cutoff,
                    crate::radio_ui::channel_bandwidth(&state),
                    crate::radio_ui::stereo_audio_requested(&state),
                    state.adsb_running,
                    state.spectrum.vfo_signal_level(),
                ))
            } else {
                None
            }
        };

        if let Ok(state) = self.shared.try_lock() {
            // A replay may have been captured anywhere. Its viewer location is not
            // verified capture metadata and must not disambiguate surface CPR.
            if state.source.source_mode == crate::source_manager::SourceMode::Hardware {
                self.adsb_decoder
                    .set_receiver_position(state.config.observer_lat, state.config.observer_lon);
            } else {
                self.adsb_decoder.set_receiver_position(f64::NAN, f64::NAN);
            }
        }

        // Process samples outside lock
        if let Some((
            freq,
            center,
            rate,
            iq_config,
            source_mode,
            stream_generation,
            demod_mode,
            receiving,
            audio_running,
            volume,
            squelch_db,
            squelch_mode,
            ctcss_tone_hz,
            rds_settings,
            lpf_cutoff,
            channel_bandwidth,
            stereo_audio,
            adsb_running,
            signal_level,
        )) = source_params
        {
            let mut all_audio_samples: Vec<f32> = Vec::new();
            let mut spectrum_batch: Vec<Vec<num_complex::Complex32>> = Vec::new();
            let capture = (center, source_mode, stream_generation);
            let capture_changed = self.last_radio_capture.as_ref() != Some(&capture);
            if capture_changed {
                self.radio_iq.reset();
                self.vfo_mixer.reset();
                self.demod_worker.request_reset();
                self.ctcss.reset();
                self.rds.reset();
                self.radio_ui.rds = None;
                self.radio_ui.received_tone = None;
                self.last_radio_capture = Some(capture);
            }
            let iq_changed = self.radio_iq.configure(iq_config);
            if iq_changed {
                self.vfo_mixer.reset();
                self.demod_worker.request_reset();
                self.ctcss.reset();
                self.rds.reset();
                self.radio_ui.rds = None;
            }
            let radio_rate = self.radio_iq.output_rate();
            let offset_hz = (i128::from(freq) - i128::from(center)) as f64;
            self.vfo_mixer.configure(radio_rate, offset_hz);
            let resolved_mode = demod_mode.resolve(freq);
            let has_capture = receiving || !sample_batch.is_empty();
            let rds_active =
                has_capture && rds_settings.0 && resolved_mode == crate::sdr_panel::DemodMode::Wfm;
            let ctcss_active = has_capture
                && squelch_mode.uses_ctcss()
                && resolved_mode == crate::sdr_panel::DemodMode::Fm;
            self.rds.set_incremental(rds_settings.1);
            self.rds.set_region(rds_settings.2);
            // Per-tick playback gate (squelch), applied to drained frames below.
            let gate: f32 = if !crate::radio_ui::mode_has_squelch(resolved_mode)
                || squelch_mode != crate::radio_squelch::SquelchMode::Power
                || signal_level > squelch_db
            {
                1.0
            } else {
                0.0
            };

            for samples in &sample_batch {
                self.recorder_panel.write_samples(samples);
                self.satellite_panel.feed_recording(samples);
                self.constellation.push_iq_samples(samples);
                let wideband_iq = self.radio_iq.process(samples);

                // Queue demodulation on the worker thread. The channelizer and
                // VFO mixer stay on the UI thread; only the heavy demodulation
                // and its audio output run off-thread.
                if audio_running || rds_active || ctcss_active {
                    if resolved_mode != self.last_demod_mode {
                        self.demod_worker.request_reset();
                        self.ctcss.reset();
                        self.rds.reset();
                        self.radio_ui.rds = None;
                        self.radio_ui.received_tone = None;
                        self.last_demod_mode = resolved_mode;
                    }
                    let channel_iq = self.vfo_mixer.process(&wideband_iq);
                    self.demod_worker.configure(DemodConfig {
                        mode: resolved_mode,
                        input_rate: radio_rate,
                        audio_rate: self.audio.sample_rate(),
                        lpf_cutoff,
                        rf_bandwidth: channel_bandwidth,
                        stereo: stereo_audio,
                    });
                    self.demod_worker.push_iq(channel_iq);
                }

                // Feed to ADS-B decoder when tuned to 1090 MHz
                if adsb_running
                    && self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090
                    && (center == 1_090_000_000
                        || (center > 1_088_000_000 && center < 1_092_000_000))
                {
                    self.adsb_decoder.feed_iq(samples, rate);
                    let ac = self.adsb_decoder.get_aircraft();
                    self.adsb_panel.aircraft = ac;
                    self.adsb_panel.total_messages = self.adsb_decoder.total_messages;
                    self.adsb_panel.decode_stats = self.adsb_decoder.stats();
                }
                if self.current_tab == AppTab::Listen {
                    spectrum_batch.push(wideband_iq);
                }
            }

            // Drain finished demod frames (non-blocking; the UI thread never
            // waits on the worker). Frames still queued from before demod was
            // paused are discarded so stale audio never plays on resume.
            let mut last_demod_metrics: Option<(f32, f32)> = None;
            let demod_active = audio_running || rds_active || ctcss_active;
            while let Some(frame) = self.demod_worker.try_recv_frame() {
                if !demod_active {
                    continue;
                }
                if frame.stereo {
                    self.radio_ui.stereo_locked = frame.stereo_locked;
                    let mut interleaved = frame.audio;
                    let mut mono = Vec::with_capacity(interleaved.len() / 2);
                    for pair in interleaved.chunks_exact_mut(2) {
                        let left = pair[0] * volume * gate;
                        let right = pair[1] * volume * gate;
                        pair[0] = left;
                        pair[1] = right;
                        mono.push((left + right) * 0.5);
                    }
                    if rds_active {
                        self.rds
                            .process_multiplex(&frame.wfm_multiplex, frame.wfm_multiplex_rate);
                    }
                    if audio_running {
                        self.recorder_panel.write_audio_samples(&mono);
                    }
                    all_audio_samples.extend_from_slice(&mono);
                    if audio_running {
                        let _ = self.audio.push_audio(interleaved);
                    }
                } else {
                    self.radio_ui.stereo_locked = false;
                    let mut audio = frame.audio;
                    if frame.mode == crate::sdr_panel::DemodMode::Fm && squelch_mode.uses_ctcss() {
                        let gains = self.ctcss.process(
                            &frame.nfm_subaudible,
                            self.audio.sample_rate(),
                            ctcss_tone_hz,
                        );
                        self.radio_ui.received_tone = self.ctcss.status().detected_tone_hz;
                        if squelch_mode == crate::radio_squelch::SquelchMode::CtcssMute {
                            for (index, sample) in audio.iter_mut().enumerate() {
                                *sample *= gains.get(index).copied().unwrap_or(0.0);
                            }
                        }
                    } else {
                        self.ctcss.reset();
                        self.radio_ui.received_tone = None;
                    }
                    for sample in &mut audio {
                        *sample *= volume * gate;
                    }
                    if rds_active {
                        self.rds
                            .process_multiplex(&frame.wfm_multiplex, frame.wfm_multiplex_rate);
                    }
                    if audio_running {
                        self.recorder_panel.write_audio_samples(&audio);
                    }
                    all_audio_samples.extend_from_slice(&audio);
                    if audio_running {
                        let _ = self.audio.push_audio(audio);
                    }
                }
                last_demod_metrics = Some((frame.fm_deviation_hz, frame.audio_peak));
            }
            if !ctcss_active {
                self.ctcss.reset();
                self.radio_ui.received_tone = None;
            }
            if rds_active {
                self.radio_ui.rds = Some(self.rds.snapshot());
            } else if self.radio_ui.rds.take().is_some() {
                self.rds.reset();
            }

            // Update spectrum, waveform, and demod metrics in a single consolidated lock
            if capture_changed
                || iq_changed
                || !sample_batch.is_empty()
                || !all_audio_samples.is_empty()
            {
                if let Some(mut state) = self.try_lock_shared("update_spectrum_and_audio") {
                    if capture_changed || iq_changed {
                        state.spectrum.reset_stream();
                    }
                    state.spectrum.update_params_exact(center, radio_rate);
                    state.spectrum.vfo_freq_hz = Some(freq);
                    for samples in &spectrum_batch {
                        state.spectrum.push_complex_samples(samples);
                    }
                    state.spectrum.try_recv_spectrum();
                    if !all_audio_samples.is_empty() {
                        let wf = &mut state.spectrum.audio_waveform;
                        if wf.capacity() < 2048 {
                            wf.reserve(2048);
                        }
                        let step = (all_audio_samples.len() / 800).max(1);
                        for &s in all_audio_samples.iter().step_by(step) {
                            if wf.len() >= 2048 {
                                wf.pop_front();
                            }
                            wf.push_back(s);
                        }
                        if let Some((deviation, peak)) = last_demod_metrics {
                            state.fm_deviation_hz = deviation;
                            state.audio_peak = peak;
                        }
                    }
                }
            }
        }

        // Drain an asynchronously completed FFT even on a frame without new IQ,
        // such as the final window arriving as the receiver pauses.
        if self.current_tab == AppTab::Listen {
            if let Ok(mut state) = self.shared.try_lock() {
                state.spectrum.try_recv_spectrum();
            }
        }

        // Passive ADS-B: detect newly-arrived aircraft and fire alert toasts + Discord notifications.
        self.adsb_panel.poll_uat();
        self.adsb_panel.check_for_new_aircraft();
        if let Some(msg) = self.adsb_panel.pending_status_flash.take() {
            self.status_bar.info(msg);
        }
        if let Some(msg) = self.satellite_panel.pending_status.take() {
            self.status_bar.info(msg);
        }
        if let Some(req) = self.satellite_panel.pending_decode_request.take() {
            self.current_tab = AppTab::Satellites;
            self.satellite_subtab = crate::satellite_panel::SatelliteSubTab::Decode;
            self.active_secondary_tool = None;
            self.decoding_panel.request_from_satellite(req);
        }
        if let Some(msg) = self.decoding_panel.pending_status.take() {
            self.status_bar.info(msg);
        }
        // Poll satellite real-time position
        self.satellite_panel.tick_realtime();
        self.decoding_panel.tick_decode();
        self.meteor_decoder.tick_decode();
        if let Some(message) = self.meteor_decoder.pending_status.take() {
            self.status_bar.info(message);
        }
        // Fire Discord notifications and publish event for new aircraft
        for ac in self
            .adsb_panel
            .aircraft
            .iter()
            .filter(|ac| ac.icao <= 0x00FF_FFFF)
        {
            if self.seen_aircraft.insert(ac.icao) {
                self.event_bus
                    .publish(crate::events::AppEvent::AircraftDetected {
                        icao: ac.icao,
                        callsign: if ac.callsign.is_empty() {
                            None
                        } else {
                            Some(ac.callsign.clone())
                        },
                    });
                let icao_str = format!("{:06X}", ac.icao);
                // Offload to the Discord background thread: fetch_aircraft_image
                // does a blocking HEAD request and must never run in the GUI loop.
                self.discord.fire_aircraft(crate::discord::AircraftData {
                    icao: icao_str,
                    callsign: ac.callsign.clone(),
                    lat: ac.lat,
                    lon: ac.lon,
                    alt_ft: ac.altitude,
                    speed_kts: ac.speed,
                    heading: ac.heading,
                });
            }
        }
        // Check traffic milestone (every 10 aircraft)
        let current_bucket = self.adsb_panel.aircraft.len() / 10;
        if current_bucket > self.last_traffic_bucket && current_bucket > 0 {
            let milestone = current_bucket * 10;
            let embed = crate::discord::embed_generic(
                &format!("Traffic Milestone: {milestone} Aircraft"),
                &format!("You're now tracking **{milestone}** aircraft!"),
                "📈",
                0xFF8800,
            );
            self.discord.fire("traffic_milestone", embed);
            self.last_traffic_bucket = current_bucket;
        }

        // Keyboard shortcuts (extracted to KeyboardHandler)
        let outcome = if let Ok(mut state) = self.shared.try_lock() {
            self.keyboard_handler.handle_input(
                ctx,
                &mut state,
                &mut self.status_bar,
                self.scanner.enabled,
                &mut self.freq_history_idx,
                &mut self.last_history_freq,
            )
        } else {
            let mut fallback = crate::keyboard::KeyboardOutcome::default();
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Questionmark) {
                    fallback.toggle_help = true;
                }
            });
            fallback
        };

        if outcome.toggle_help {
            self.show_keyboard_help = !self.show_keyboard_help;
        }
        if outcome.freq_changed {
            self.last_manual_tune_time = std::time::Instant::now();
            if let Ok(mut state) = self.shared.try_lock() {
                let frequency = state.source.frequency_hz;
                crate::radio_ui::tune(&mut state, frequency);
                self.event_bus
                    .publish(crate::events::AppEvent::FrequencyChanged {
                        hz: state.source.frequency_hz,
                    });
            }
        }
        if outcome.toggle_scanner {
            if self.scanner.enabled {
                self.scanner.stop();
            } else {
                self.scanner.start();
            }
        }
        if outcome.toggle_recording && self.current_tab == AppTab::Listen {
            if self.recorder_panel.recording {
                self.recorder_panel.stop();
                self.event_bus
                    .publish(crate::events::AppEvent::RecordingStopped { duration_secs: 0 });
            } else {
                self.recorder_panel.start();
                self.event_bus
                    .publish(crate::events::AppEvent::RecordingStarted {
                        filename: "recording.wav".to_string(),
                    });
            }
        }

        // Keep up with local IQ and daemon audio even when no local sample batch
        // arrived. The daemon emits audio faster than an idle UI refresh rate.
        let receiver_active = self.shared.try_lock().is_ok_and(|state| {
            matches!(
                state.source.status,
                crate::source_manager::SourceStatus::Running
                    | crate::source_manager::SourceStatus::Opening
            )
        });
        let background_active = self.meteor_decoder.running
            || self.decoding_panel.running
            || self.adsb_panel.region == crate::adsb_panel::AdsbRegion::Uat978;
        ctx.request_repaint_after(Duration::from_millis(
            if receiver_active || !sample_batch.is_empty() {
                16
            } else if background_active {
                100
            } else {
                500
            },
        ));

        // Track source status transitions, dynamic window title, and audio lifecycle (consolidated lock)
        let (audio_action, title, new_source_status) = {
            if let Some(mut state) = self.try_lock_shared("status_lifecycle_title") {
                let was_running = matches!(
                    self.last_source_status,
                    crate::source_manager::SourceStatus::Running
                );
                let is_running =
                    state.source.status == crate::source_manager::SourceStatus::Running;
                if was_running && !is_running {
                    state.adsb_running = false;
                    self.event_bus.publish(crate::events::AppEvent::SdrStopped);
                } else if !was_running && is_running {
                    self.event_bus.publish(crate::events::AppEvent::SdrStarted);
                }

                let freq_mhz = state.source.frequency_hz as f64 / 1e6;
                let mode = state.demod_mode.label();
                let title = if is_running {
                    format!("EZ-SDR — {freq_mhz:.3} MHz {mode} ▶")
                } else {
                    format!("EZ-SDR — {freq_mhz:.3} MHz {mode} ■")
                };

                let audio_running = state.audio_running
                    && matches!(
                        state.source.status,
                        crate::source_manager::SourceStatus::Running
                            | crate::source_manager::SourceStatus::Opening
                    );
                (
                    Some((
                        audio_running,
                        state.config.advanced.audio_output.clone(),
                        if crate::radio_ui::stereo_audio_requested(&state) {
                            2
                        } else {
                            1
                        },
                    )),
                    Some(title),
                    Some(state.source.status.clone()),
                )
            } else {
                (None, None, None)
            }
        };

        if let Some(status) = new_source_status {
            self.last_source_status = status;
        }
        if let Some(title) = title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        if let Some(error) = self.audio.take_error() {
            self.status_bar.warning(format!(
                "Audio: {error}. Stop and restart the receiver to retry."
            ));
        }
        if let Some((audio_running, selection, input_channels)) = audio_action {
            if audio_running && !self.audio.is_running() {
                if !self.audio.has_failed() {
                    match self.audio.start_with_selection_channels(
                        crossbeam_channel::never::<Vec<f32>>(),
                        &selection,
                        input_channels,
                    ) {
                        Ok(()) => {
                            self.recorder_panel
                                .set_audio_sample_rate(self.audio.sample_rate());
                            self.demod_worker.request_reset();
                        }
                        Err(error) => {
                            self.audio.mark_failed();
                            let _ = self.audio.take_error();
                            self.status_bar.warning(format!(
                                "Audio: {error}. Stop and restart the receiver to retry."
                            ));
                        }
                    }
                }
            } else if !audio_running && (self.audio.is_running() || self.audio.has_failed()) {
                self.audio.stop();
                self.daemon_audio.reset();
            }
        }

        // Process web remote commands in a single lock
        let web_cmds = self.web_remote.poll_commands();
        if !web_cmds.is_empty() {
            let mut actions = Vec::new();
            if let Some(mut state) = self.try_lock_shared("web_remote_commands") {
                for cmd in web_cmds {
                    match cmd {
                        RemoteCommand::Tune { freq_hz } => {
                            crate::radio_ui::tune(&mut state, freq_hz);
                        }
                        RemoteCommand::SetGain { gain_db } => {
                            state.source.gain_db = gain_db;
                            crate::radio_ui::restart_if_running(&mut state.source);
                        }
                        RemoteCommand::SetDemod { mode } => {
                            use crate::sdr_panel::DemodMode;
                            if let Some(dm) = DemodMode::from_label(&mode) {
                                state.demod_mode = dm;
                            }
                        }
                        RemoteCommand::SetSquelch { db } => {
                            crate::radio_ui::set_power_squelch_level(&mut state, db);
                        }
                        RemoteCommand::SetVolume { level } => {
                            state.volume = level.clamp(0.0, 1.0);
                        }
                        cmd => {
                            actions.push(cmd);
                        }
                    }
                }
            }
            for cmd in actions {
                match cmd {
                    RemoteCommand::StartRecord => self.recorder_panel.start(),
                    RemoteCommand::StopRecord => self.recorder_panel.stop(),
                    RemoteCommand::StartScan => self.scanner.start(),
                    RemoteCommand::StopScan => self.scanner.stop(),
                    _ => {}
                }
            }
        }

        // Process loopback Hamlib/rigctld commands on the app thread so all
        // tuning and demodulation changes share the same live radio state.
        self.process_rigctl_requests();

        // Scheduler tick: check upcoming passes (rate-limited to every 5s)
        {
            let now = std::time::Instant::now();
            let needs_update = now.duration_since(self.last_scheduler_update).as_secs() >= 5;
            if let Ok(mut state) = self.shared.try_lock() {
                if needs_update {
                    let passes = state.tle.upcoming_passes().to_vec();
                    state.scheduler.update_from_passes(&passes);
                    self.last_scheduler_update = now;
                }
                // Auto-tune to the first active pass (with cooldown after manual tuning)
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                let tune_to = state
                    .scheduler
                    .active_radio_job(now_unix)
                    .map(|j| (j.satellite.clone(), j.frequency_hz));
                if let Some((sat, freq)) = tune_to {
                    // Satellite AOS detection (new pass)
                    if sat != self.last_active_pass_sat {
                        let passes = state.tle.upcoming_passes();
                        self.event_bus
                            .publish(crate::events::AppEvent::SatellitePassStarting {
                                name: sat.clone(),
                                aos: "Pass active".to_string(),
                            });
                        if let Some(pass) = passes.iter().find(|p| p.satellite == sat) {
                            let embed =
                                crate::discord::embed_sat_aos(&sat, freq, pass.max_elevation);
                            self.discord.fire("sat_aos", embed);
                        }
                        self.last_active_pass_sat = sat.clone();
                    }
                    // Only auto-tune if user hasn't manually changed frequency recently (15s cooldown)
                    let manual_tune_recent = self.last_manual_tune_time.elapsed().as_secs() < 15;
                    if !manual_tune_recent && sat != self.last_auto_tuned_satellite {
                        crate::radio_ui::tune(
                            &mut state,
                            crate::satellite_panel::pass_center_frequency(freq, 0.0),
                        );
                        self.last_auto_tuned_satellite = sat.clone();
                        self.event_bus
                            .publish(crate::events::AppEvent::FrequencyChanged { hz: freq });
                        self.status_bar.info(format!(
                            "🛰 Auto-tuned to {} ({:.3} MHz)",
                            sat,
                            freq as f64 / 1e6
                        ));
                    }
                    // Report Doppler continuously while leaving the receiver centered at
                    // the nominal downlink. Repeated tuner jumps disrupt LRPT carrier and
                    // timing recovery; the decoder's Costas loop follows the residual.
                    let doppler = state.tle.doppler_shift_for_sat(&sat, freq as f64, now_unix);
                    self.satellite_panel.doppler_hz = doppler;
                } else if !self.last_active_pass_sat.is_empty() {
                    // Satellite LOS detection (pass ended)
                    let embed = crate::discord::embed_sat_los(&self.last_active_pass_sat);
                    self.discord.fire("sat_los", embed);
                    self.last_active_pass_sat.clear();
                }
                // Poll custom scheduled tasks
                if let Some((label, freq)) = state.scheduler.poll_custom_tasks(now_unix) {
                    crate::radio_ui::tune(&mut state, freq);
                    eprintln!(
                        "[scheduler] fired custom task '{}' → {:.3} MHz",
                        label,
                        freq as f64 / 1e6
                    );
                    let embed = crate::discord::embed_task_fired(&label, freq);
                    self.discord.fire("task_fired", embed);
                }
            }

            // Squelch-triggered recording tick
            {
                let (signal_db, squelch_db, freq_hz, mode_label) =
                    if let Ok(state) = self.shared.try_lock() {
                        (
                            state.spectrum.vfo_signal_level(),
                            state.squelch,
                            state.source.frequency_hz,
                            state.demod_mode.label().to_string(),
                        )
                    } else {
                        (-120.0, -50.0, 0u64, "NFM".to_string())
                    };
                self.recorder_panel.tick_squelch_record(
                    signal_db,
                    squelch_db,
                    freq_hz,
                    &mode_label,
                );
            }

            // Frequency scanner tick (runs every frame, rate-limited by dwell_ms)
            {
                let (peak, scanner_cmd) = if let Ok(mut state) = self.shared.try_lock() {
                    (state.spectrum.peak_level(), state.scanner_command.take())
                } else {
                    (-120.0, None)
                };
                // Apply any AI-issued scanner configuration before ticking so
                // the sweep runs with the requested settings this frame.
                if let Some(cmd) = scanner_cmd {
                    self.scanner.apply_command(&cmd);
                }
                let prev_hits = self.scanner.hits.len();
                self.scanner.tick(peak);
                // Publish any new hits to MQTT + Discord
                if self.scanner.hits.len() > prev_hits {
                    for hit in &self.scanner.hits[prev_hits..] {
                        self.mqtt.publish_scanner_hit(hit.freq_hz, hit.strength_db);
                        let embed = crate::discord::embed_scanner_hit(hit.freq_hz, hit.strength_db);
                        self.discord.fire("scanner_hit", embed);
                    }
                }
                if let Some(freq) = self.scanner.tune_request_hz.take() {
                    if let Ok(mut state) = self.shared.try_lock() {
                        crate::radio_ui::tune(&mut state, freq);
                        if let Some(mode_str) = self.scanner.mode_request.take() {
                            if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&mode_str) {
                                state.demod_mode = mode;
                            }
                        }
                    }
                } else {
                    let _ = self.scanner.mode_request.take();
                }
            }
        }

        // Apply theme on first frame (retry until the lock is available)
        if !self.theme_applied {
            if let Ok(state) = self.shared.try_lock() {
                state.config.theme_config.apply_to_ctx(ctx);
                let scale = state.config.font_scale as f32;
                ctx.set_pixels_per_point(scale);
                self.theme_applied = true;
            }
        }

        // Apply config changes triggered from Settings tab
        if let Ok(mut state) = self.shared.try_lock() {
            if state.config.needs_apply {
                state.config.normalize();
                state.config.needs_apply = false;
                // Apply imported advanced controls as well as theme/source settings.
                self.sync_demod_advanced(&state);
                // Apply theme
                state.config.theme_config.apply_to_ctx(ctx);
                // Apply font scale
                let scale = state.config.font_scale as f32;
                ctx.set_pixels_per_point(scale);
                let source_settings_changed = state.source.sample_rate_hz
                    != state.config.default_sample_rate
                    || state.source.gain_db != state.config.default_gain;
                let old_center = state.source.capture_center_frequency_hz();
                state.source.sample_rate_hz = state.config.default_sample_rate;
                state.source.gain_db = state.config.default_gain;
                let frequency = state.config.default_freq_hz;
                crate::radio_ui::tune(&mut state, frequency);
                if source_settings_changed
                    && old_center == state.source.capture_center_frequency_hz()
                {
                    crate::radio_ui::restart_if_running(&mut state.source);
                }
                state.tle.observer_lat = state.config.observer_lat;
                state.tle.observer_lon = state.config.observer_lon;
                self.web_remote.set_enabled(
                    state.config.web_remote_enabled,
                    state.config.web_remote_port,
                );
                self.rigctl
                    .set_enabled(state.config.rigctl_enabled, state.config.rigctl_port);
                self.mqtt.set_enabled(
                    !state.config.mqtt_broker.is_empty(),
                    state.config.mqtt_broker.clone(),
                    state.config.mqtt_topic_prefix.clone(),
                );
                self.discord.apply_settings(&state.config.discord);
            }
        }

        // Broadcast state + MQTT tick (rate-limited to every 5s)
        if let Ok(mut state) = self.shared.try_lock() {
            let mode = state.demod_mode.label();
            let freq = state.source.frequency_hz;
            let gain = state.source.gain_db;
            let ac_count = self.adsb_panel.aircraft.len();
            let passes = state.tle.upcoming_passes().to_vec();
            let squelch = state.squelch;
            let volume = state.volume;
            let recording = state.recording;
            let scanner_active = self.scanner.enabled;
            let peak = state.spectrum.peak_level();
            let noise = state.spectrum.noise_floor();
            let snr = peak - noise;

            // State transitions for Discord notifications
            if recording && !self.last_recording {
                self.recording_start = Some(std::time::Instant::now());
                let embed = crate::discord::embed_recording_started(
                    freq,
                    mode,
                    self.recorder_panel.record_iq,
                    self.recorder_panel.record_audio,
                );
                self.discord.fire("rec_started", embed);
            } else if !recording && self.last_recording {
                let duration = self.recording_start.map_or(0, |t| t.elapsed().as_secs());
                self.recording_start = None;
                let embed = crate::discord::embed_recording_stopped(
                    freq,
                    mode,
                    duration,
                    self.recorder_panel.bytes_written,
                );
                self.discord.fire("rec_stopped", embed);
            }
            if state.adsb_running && !self.last_adsb_running {
                let embed = crate::discord::embed_generic(
                    "ADS-B Started",
                    "ADS-B decoder activated",
                    "📡",
                    0x00AA00,
                );
                self.discord.fire("adsb_started", embed);
            } else if !state.adsb_running && self.last_adsb_running {
                let embed = crate::discord::embed_generic(
                    "ADS-B Stopped",
                    "ADS-B decoder stopped",
                    "🔌",
                    0xCC0000,
                );
                self.discord.fire("adsb_stopped", embed);
            }
            if self.scanner.enabled && !self.last_scanner_enabled {
                let embed = crate::discord::embed_generic(
                    "Scanner Started",
                    "Frequency scanner activated",
                    "▶️",
                    0x00AA00,
                );
                self.discord.fire("scanner_started", embed);
            } else if !self.scanner.enabled && self.last_scanner_enabled {
                let embed = crate::discord::embed_generic(
                    "Scanner Stopped",
                    "Frequency scanner stopped",
                    "⏹",
                    0xCC0000,
                );
                self.discord.fire("scanner_stopped", embed);
            }
            if self.mqtt.is_connected() && !self.last_mqtt_connected {
                let embed = crate::discord::embed_generic(
                    "MQTT Connected",
                    &format!("Connected to {}", self.mqtt.broker),
                    "🔗",
                    0x00AA00,
                );
                self.discord.fire("mqtt_connected", embed);
            } else if !self.mqtt.is_connected() && self.last_mqtt_connected {
                let embed = crate::discord::embed_generic(
                    "MQTT Disconnected",
                    "MQTT broker disconnected",
                    "🔌",
                    0xCC0000,
                );
                self.discord.fire("mqtt_disconnected", embed);
            }
            self.last_recording = recording;
            self.last_adsb_running = state.adsb_running;
            self.last_scanner_enabled = self.scanner.enabled;
            self.last_mqtt_connected = self.mqtt.is_connected();

            // First strong signal celebration
            if !self.first_strong_signal_seen && snr > 20.0 {
                self.first_strong_signal_seen = true;
                self.event_bus
                    .publish(crate::events::AppEvent::SignalDetected {
                        freq_hz: freq,
                        strength_db: snr,
                    });
                self.status_bar.info(format!(
                    "🎉 First signal! {:.3} MHz — SNR {:.1} dB — great reception!",
                    freq as f64 / 1e6,
                    snr
                ));
                let embed = crate::discord::embed_strong_signal(freq, snr);
                self.discord.fire("first_signal", embed);
            }
            // Any strong signal
            if snr > 20.0 && (self.discord_summary_last.elapsed().as_secs() > 5) {
                self.event_bus
                    .publish(crate::events::AppEvent::SignalDetected {
                        freq_hz: freq,
                        strength_db: snr,
                    });
                let embed = crate::discord::embed_strong_signal(freq, snr);
                self.discord.fire("strong_signal", embed);
            }
            self.web_remote
                .broadcast_state(&crate::web_remote::StreamState {
                    freq_hz: freq,
                    gain_db: gain,
                    demod_mode: mode,
                    aircraft_count: ac_count,
                    passes: &passes,
                    squelch,
                    volume,
                    recording,
                    scanner_active,
                    snr_db: snr,
                });
            self.mqtt.tick_reconnect();
            if self.last_scheduler_update.elapsed().as_secs() < 1 {
                self.mqtt.tick(freq, gain);
                self.mqtt.publish_signal(freq, peak, noise, mode, recording);
                self.mqtt.publish_passes(&passes);
                if !self.adsb_panel.aircraft.is_empty() {
                    let icao_aircraft: Vec<_> = self
                        .adsb_panel
                        .aircraft
                        .iter()
                        .filter(|ac| ac.icao <= 0x00FF_FFFF)
                        .cloned()
                        .collect();
                    self.mqtt.publish_aircraft(&icao_aircraft);
                }
            }

            // Periodic session summary report
            if self.discord.settings.summary_enabled
                && self.discord_summary_last.elapsed().as_secs()
                    >= (u64::from(self.discord.settings.summary_interval_min) * 60)
            {
                let uptime = self
                    .recording_start
                    .as_ref()
                    .map_or(0, |t| t.elapsed().as_secs());
                let embed = crate::discord::embed_session_summary(
                    uptime,
                    freq as f64 / 1e6,
                    mode,
                    ac_count,
                    self.scanner.hits.len(),
                    0, // recordings count - would need to track
                    passes.len(),
                );
                self.discord.fire("session_summary", embed);
                self.discord_summary_last = std::time::Instant::now();
            }

            // Recording error detection
            if !self.recorder_panel.last_error.is_empty() {
                let embed = crate::discord::embed_recording_error(&self.recorder_panel.last_error);
                self.discord.fire("rec_error", embed);
                // Note: would need to track whether we've already sent this error
            }
        }

        // Auto-save bookmarks when modified (15-second debounce)
        if let Ok(state) = self.shared.try_lock() {
            let cur_len = state.bookmarks.bookmarks.len();
            if cur_len != self.bm_last_len {
                self.bm_last_len = cur_len;
                self.bm_dirty_since = Some(std::time::Instant::now());
            }
            if let Some(dirty_since) = self.bm_dirty_since {
                if dirty_since.elapsed().as_secs() >= 15 {
                    state.bookmarks.save();
                    self.bm_dirty_since = None;
                }
            }
        }

        // Track frequency changes for history
        if let Ok(mut state) = self.shared.try_lock() {
            let freq = state.source.frequency_hz;
            if freq != self.last_history_freq {
                self.last_history_freq = freq;
                if state.freq_history.back() != Some(&freq) {
                    // Manual tune: truncate forward history and append
                    if let Some(idx) = self.freq_history_idx {
                        let len = state.freq_history.len();
                        let excess = len.saturating_sub(idx + 1);
                        for _ in 0..excess {
                            state.freq_history.pop_back();
                        }
                        self.freq_history_idx = None;
                    }
                    state.freq_history.push_back(freq);
                    if state.freq_history.len() > 50 {
                        state.freq_history.pop_front();
                    }
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.quick_start.is_active() {
            self.quick_start.ui(ui.ctx(), &self.shared);
            return;
        }

        // Take a snapshot of the shared state once per frame
        let snapshot = {
            if let Ok(state) = self.shared.try_lock() {
                Some(SharedSnapshot {
                    jobs: state.scheduler.jobs.clone(),
                    custom_tasks: state.scheduler.custom_tasks.clone(),
                    auto_tune_enabled: state.scheduler.auto_tune_enabled,
                })
            } else {
                return;
            }
        };

        // Top mode bar: 3 task modes + ambient 🤖 Ask + ⚙ More.
        egui::Panel::top("mode_bar")
            .exact_size(30.0)
            .show(ui, |ui| self.render_mode_bar(ui));

        // ⚙ More — the deck of hidden tools (Bookmarks/Scanner/Settings/…) as a
        // left slide-over, reusing the existing secondary-tool machinery.
        if let Some(tool) = self.active_secondary_tool {
            egui::Panel::left("secondary_panel")
                .resizable(true)
                .default_size(360.0)
                .show(ui, |ui| self.render_secondary_panel(ui, tool, &snapshot));
        }

        // 🤖 Ask — ambient AI slide-over, available in every mode.
        if self.ai_ask_open {
            egui::Panel::right("ai_ask_panel")
                .resizable(true)
                .default_size(340.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("🤖 Ask").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text("Close").clicked() {
                                self.ai_ask_open = false;
                            }
                        });
                    });
                    ui.separator();
                    self.ai_panel.ui(ui);
                });
        }

        // Humanized status strip: frequency, mode, plain-language signal, REC.
        // Declared before the central mode content (egui requires side panels
        // to precede the CentralPanel).
        if self.current_tab != AppTab::Listen {
            egui::Panel::bottom("status_strip")
                .exact_size(26.0)
                .show(ui, |ui| self.render_status_strip(ui));
        }

        match self.current_tab {
            AppTab::Listen => self.render_sdr_tab(ui, &snapshot),
            AppTab::Planes => self.render_adsb_tab(ui),
            AppTab::Satellites => self.render_satellite_tab(ui, &snapshot),
            AppTab::Meteor => self.render_meteor_tab(ui),
        }

        // Frequency jump dialog (J key)
        if self.show_freq_jump {
            let mut close = false;
            let mut tune_to: Option<u64> = None;
            egui::Window::new("⤵ Jump to Frequency")
                .id(egui::Id::new("freq_jump_dialog"))
                .default_size([380.0, 400.0])
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("Enter a frequency (MHz) or band name:");
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.freq_jump_input)
                            .desired_width(340.0)
                            .hint_text("e.g. '145.5', '88.0', 'aviation', 'weather', 'noaa'"),
                    );
                    if edit.changed() {
                        self.freq_jump_matches.clear();
                        let q = self.freq_jump_input.trim().to_lowercase();
                        if !q.is_empty() {
                            // Try numeric parse first
                            if let Ok(mhz) = q.parse::<f64>() {
                                self.freq_jump_matches
                                    .push((format!("{mhz:.3} MHz"), (mhz * 1e6) as u64));
                            } else {
                                // Search built-in frequency database
                                for preset in crate::frequency_db::FrequencyDatabase::search(&q) {
                                    self.freq_jump_matches.push((
                                        format!(
                                            "{} {} ({:.3} MHz)",
                                            preset.category.icon(),
                                            preset.name,
                                            preset.frequency_hz as f64 / 1e6
                                        ),
                                        preset.frequency_hz,
                                    ));
                                }
                            }
                            // Also search bookmarks
                            if let Ok(state) = self.shared.try_lock() {
                                for bm in &state.bookmarks.bookmarks {
                                    if bm.name.to_lowercase().contains(&q)
                                        || bm.category.to_lowercase().contains(&q)
                                    {
                                        self.freq_jump_matches.push((
                                            format!(
                                                "⭐ {} ({:.3} MHz)",
                                                bm.name,
                                                bm.frequency_hz as f64 / 1e6
                                            ),
                                            bm.frequency_hz,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    // Focus the text field when dialog opens
                    if edit.hovered() || self.freq_jump_input.is_empty() {
                        edit.request_focus();
                    }

                    // Show matches or suggestions if empty
                    let has_input = !self.freq_jump_input.trim().is_empty();
                    let should_show_suggestions = self.freq_jump_matches.is_empty() && !has_input;

                    if !self.freq_jump_matches.is_empty() || should_show_suggestions {
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                // Show search matches if any
                                for (label, freq) in self.freq_jump_matches.clone() {
                                    if ui.selectable_label(false, &label).clicked() {
                                        tune_to = Some(freq);
                                        close = true;
                                    }
                                }

                                // Show suggestions (presets, recent freqs + bookmarks) when input is empty
                                if should_show_suggestions {
                                    ui.label(egui::RichText::new("Preset Bands:").italics().small());
                                    ui.horizontal_wrapped(|ui| {
                                        for cat in crate::frequency_db::PresetCategory::all() {
                                            if ui.small_button(format!("{} {}", cat.icon(), cat.label())).clicked() {
                                                self.freq_jump_matches = crate::frequency_db::FrequencyDatabase::by_category(*cat)
                                                    .into_iter()
                                                    .map(|p| (format!("{} {} ({:.3} MHz)", cat.icon(), p.name, p.frequency_hz as f64 / 1e6), p.frequency_hz))
                                                    .collect();
                                            }
                                        }
                                    });

                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new("Recent:").italics().small());
                                    for entry in self.frequency_history.recent(5) {
                                        let label = if let Some(ref l) = entry.label {
                                            format!("  {:.3} MHz ({})", entry.freq_hz as f64 / 1e6, l)
                                        } else {
                                            format!("  {:.3} MHz", entry.freq_hz as f64 / 1e6)
                                        };
                                        if ui.selectable_label(false, &label).clicked() {
                                            tune_to = Some(entry.freq_hz);
                                            close = true;
                                        }
                                    }

                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new("Bookmarks:").italics().small());
                                    if let Ok(state) = self.shared.try_lock() {
                                        let bookmarks_to_show =
                                            state.bookmarks.bookmarks.iter().take(5);
                                        for bm in bookmarks_to_show {
                                            let label = format!(
                                                "  ⭐ {} ({:.3} MHz)",
                                                bm.name,
                                                bm.frequency_hz as f64 / 1e6
                                            );
                                            if ui.selectable_label(false, &label).clicked() {
                                                tune_to = Some(bm.frequency_hz);
                                                close = true;
                                            }
                                        }
                                    }
                                }
                            });
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        // Enter key tunes to first match or numeric entry
                        if ui.button("Tune").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            if let Some((_, freq)) = self.freq_jump_matches.first() {
                                tune_to = Some(*freq);
                            } else if let Ok(mhz) = self.freq_jump_input.trim().parse::<f64>() {
                                tune_to = Some((mhz * 1e6) as u64);
                            }
                            close = true;
                        }
                        if ui.button("Cancel").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            close = true;
                        }
                    });
                });
            if close {
                self.show_freq_jump = false;
            }
            if let Some(freq) = tune_to {
                if let Ok(mut state) = self.shared.try_lock() {
                    crate::radio_ui::tune(&mut state, freq);
                    self.last_manual_tune_time = std::time::Instant::now();
                    self.status_bar
                        .info(format!("⤵ {:.3} MHz", freq as f64 / 1e6));
                }
            }
        }

        // Quick Start Wizard overlay
        self.quick_start.ui(ui.ctx(), &self.shared);

        // Keyboard shortcuts help overlay
        if self.show_keyboard_help {
            egui::Window::new("Keyboard Shortcuts (?)")
                .id(egui::Id::new("keyboard_help"))
                .default_size([400.0, 500.0])
                .movable(true)
                .show(ui.ctx(), |ui| {
                    egui::Grid::new("shortcuts_grid").num_columns(2).striped(true).show(ui, |ui| {
                        ui.monospace("Space"); ui.label("Start/Stop SDR source"); ui.end_row();
                        ui.monospace("↑ / ↓"); ui.label("Tune by coarse step (default 1 MHz, set via step row)"); ui.end_row();
                        ui.monospace("← / →"); ui.label("Tune by fine step (default 100 kHz, set via step row)"); ui.end_row();
                        ui.monospace("Shift+Arrow"); ui.label("Tune by 10× the current step"); ui.end_row();
                        ui.monospace("[ / ] or Alt+←/→"); ui.label("Frequency history back/forward"); ui.end_row();
                        ui.monospace("F1 / Alt+R"); ui.label("Demod: RAW"); ui.end_row();
                        ui.monospace("F2 / Alt+A"); ui.label("Demod: AM"); ui.end_row();
                        ui.monospace("F3 / Alt+F"); ui.label("Demod: NFM"); ui.end_row();
                        ui.monospace("F4 / Alt+W"); ui.label("Demod: WFM"); ui.end_row();
                        ui.monospace("F5 / Alt+L"); ui.label("Demod: LSB"); ui.end_row();
                        ui.monospace("F6 / Alt+U"); ui.label("Demod: USB"); ui.end_row();
                        ui.monospace("M"); ui.label("Toggle audio mute on/off"); ui.end_row();
                        ui.monospace("F"); ui.label("Freeze / unfreeze spectrum display"); ui.end_row();
                        ui.monospace("C"); ui.label("Cycle waterfall colormap (Classic→Viridis→Plasma→…)"); ui.end_row();
                        ui.monospace("V"); ui.label("Swap VFO A ↔ VFO B (quick frequency toggle)"); ui.end_row();
                        ui.monospace("B"); ui.label("Tune to nearest bookmark from current frequency"); ui.end_row();
                        ui.monospace("T"); ui.label("Tune to the strongest signal in the visible spectrum"); ui.end_row();
                        ui.monospace("S"); ui.label("Toggle scanner on/off (frequency sweep mode)"); ui.end_row();
                        ui.monospace("R"); ui.label("Reset spectrum dB range to default (-120 to 0 dB)"); ui.end_row();
                        ui.monospace("J"); ui.label("Open frequency jump dialog — type MHz or band name to jump"); ui.end_row();
                        ui.monospace("Ctrl++"); ui.label("Zoom in on spectrum (×1.5 per press)"); ui.end_row();
                        ui.monospace("Ctrl+-"); ui.label("Zoom out on spectrum"); ui.end_row();
                        ui.monospace("Ctrl+0"); ui.label("Reset spectrum zoom to 1× (full span)"); ui.end_row();
                        ui.monospace("P"); ui.label("Toggle spectrum peak hold on/off"); ui.end_row();
                        ui.monospace("1–9"); ui.label("Tune to bookmark #1–#9 instantly"); ui.end_row();
                        ui.monospace("Alt+1–9"); ui.label("Recall frequency memory M1–M9 (empty slots do nothing)"); ui.end_row();
                        ui.monospace("Alt+Shift+1–9"); ui.label("Save current frequency to memory M1–M9"); ui.end_row();
                        ui.monospace("G / Shift+G"); ui.label("Gain +5 dB / −5 dB step"); ui.end_row();
                        ui.monospace("Ctrl+↑ / Ctrl+↓"); ui.label("Volume +10% / −10%"); ui.end_row();
                        ui.monospace("Ctrl+B"); ui.label("Quick-bookmark current frequency"); ui.end_row();
                        ui.monospace("Ctrl+R"); ui.label("Start / stop recording (toggle)"); ui.end_row();
                        ui.monospace("Ctrl+S"); ui.label("Save config + recent frequencies + spectrum dB range + VFO B + waterfall range"); ui.end_row();
                        ui.monospace("?"); ui.label("Toggle this shortcut reference"); ui.end_row();
                        ui.separator(); ui.separator(); ui.end_row();
                        ui.label(egui::RichText::new("Spectrum / Waterfall").italics()); ui.label(""); ui.end_row();
                        ui.monospace("Left-click"); ui.label("Tune to clicked frequency"); ui.end_row();
                        ui.monospace("Left-drag (waterfall)"); ui.label("Pan zoom window left/right"); ui.end_row();
                        ui.monospace("Right-click"); ui.label("Context menu: Tune · Bookmark · Copy freq · Set squelch"); ui.end_row();
                        ui.monospace("Middle-click"); ui.label("Drop a frequency marker"); ui.end_row();
                        ui.monospace("Scroll"); ui.label("Zoom in/out on spectrum and waterfall"); ui.end_row();
                        ui.monospace("Shift+Scroll"); ui.label("Pan spectrum left/right"); ui.end_row();
                        ui.monospace("Mid-drag"); ui.label("Pan spectrum view"); ui.end_row();
                        ui.separator(); ui.separator(); ui.end_row();
                        ui.label(egui::RichText::new("Status bar").italics()); ui.label(""); ui.end_row();
                        ui.monospace("Click frequency"); ui.label("Copy frequency value to clipboard"); ui.end_row();
                        ui.monospace("◀ ▶ buttons"); ui.label("Navigate frequency history"); ui.end_row();
                        ui.monospace("⟳ Layout"); ui.label("Reset panel layout to default"); ui.end_row();
                        ui.monospace("❓ Glossary"); ui.label("Open/close the SDR term glossary"); ui.end_row();
                    });
                });
        }

        // SDR Glossary popup
        if self.show_glossary {
            let mut open = true;
            egui::Window::new("SDR Glossary")
                .id(egui::Id::new("sdr_glossary"))
                .default_size([480.0, 600.0])
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    ui.label(egui::RichText::new("Common SDR terms and what they mean:").italics());
                    ui.add_space(4.0);
                    egui::Grid::new("glossary_grid").num_columns(2).striped(true).min_col_width(90.0).show(ui, |ui| {
                        let h = |ui: &mut egui::Ui, term: &str| {
                            ui.label(egui::RichText::new(term).strong().color(egui::Color32::from_rgb(100, 200, 255)));
                        };
                        h(ui, "dBFS"); ui.label("Decibels relative to Full Scale. 0 dBFS = maximum possible signal; −120 dBFS ≈ noise floor. Negative is normal."); ui.end_row();
                        h(ui, "SNR"); ui.label("Signal-to-Noise Ratio. How much stronger your signal is vs background noise. >20 dB = great, 8–20 dB = weak but readable, <8 dB = noise."); ui.end_row();
                        h(ui, "MSps"); ui.label("Megasamples per second — the SDR's sample rate. Higher = wider spectrum view. RTL-SDR supports 0.25–3.2 MSps."); ui.end_row();
                        h(ui, "LPF"); ui.label("Low-Pass Filter. Removes high-frequency audio hiss above a set cutoff (kHz). Lower cutoff = cleaner audio, narrower bandwidth."); ui.end_row();
                        h(ui, "PPM"); ui.label("Parts Per Million — crystal frequency error correction. If signals appear off-frequency, adjust PPM to shift the whole spectrum. Calibrate using a known signal (e.g. local FM station)."); ui.end_row();
                        h(ui, "VFO"); ui.label("Variable Frequency Oscillator — your tuned frequency. VFO A is the main frequency; VFO B is a saved alternate you can swap to with 'V'."); ui.end_row();
                        h(ui, "BW"); ui.label("Bandwidth — the frequency range occupied by a signal. WFM stations are ~200 kHz wide; NFM (voice) is ~12.5 kHz; AM ~10 kHz."); ui.end_row();
                        h(ui, "Squelch"); ui.label("A gate that silences audio when signal strength drops below a threshold. Prevents constant static between transmissions. Set 5 dB above noise floor."); ui.end_row();
                        h(ui, "LO"); ui.label("Local Oscillator — the hardware frequency the SDR chip tunes to. The actual receive frequency = LO ± baseband offset (shown if different from VFO)."); ui.end_row();
                        h(ui, "Gain"); ui.label("RF amplification in dB. Higher = more sensitive but increases noise and overload risk. Start at 30–40 dB and adjust for best SNR."); ui.end_row();
                        h(ui, "IQ / I-Q"); ui.label("In-phase and Quadrature — two channels the SDR captures to preserve both amplitude and phase. Together they describe the complex baseband signal."); ui.end_row();
                        h(ui, "FFT"); ui.label("Fast Fourier Transform — converts the raw IQ time-domain data into the frequency-domain spectrum display you see."); ui.end_row();
                        h(ui, "Waterfall"); ui.label("A time-frequency plot: frequencies on the X axis, time scrolling down. Bright spots = signals. Great for spotting intermittent transmissions."); ui.end_row();
                        h(ui, "WFM"); ui.label("Wideband FM — used for broadcast FM radio stations (~88–108 MHz). Requires ≥200 kHz bandwidth."); ui.end_row();
                        h(ui, "NFM"); ui.label("Narrowband FM — used for VHF/UHF voice (police, aircraft, amateur). ~12.5 kHz bandwidth."); ui.end_row();
                        h(ui, "AM"); ui.label("Amplitude Modulation — used for shortwave/HF broadcasts and aircraft voice (108–137 MHz). Envelope of the carrier carries audio."); ui.end_row();
                        h(ui, "SSB/USB/LSB"); ui.label("Single Sideband — used for amateur radio HF voice. USB = Upper Sideband (>10 MHz), LSB = Lower Sideband (<10 MHz). Very efficient."); ui.end_row();
                        h(ui, "Bias Tee"); ui.label("Passes DC voltage (4.5V) through the antenna port to power an external LNA (low-noise amplifier). Only on compatible hardware (RTL-SDR Blog V3+)."); ui.end_row();
                        h(ui, "ADS-B"); ui.label("Automatic Dependent Surveillance-Broadcast — 1090 MHz signals from aircraft reporting position, altitude, speed. Received by the ADS-B tab."); ui.end_row();
                        h(ui, "S-meter"); ui.label("Signal strength meter using the IARU S-unit scale: S1 ≈ −121 dBm, each S-unit = 6 dB. S9 ≈ −73 dBm. 'S9+20dB' means 20 dB above S9."); ui.end_row();
                    });
                    ui.add_space(4.0);
                    if ui.button("Close").clicked() {
                        self.show_glossary = false;
                    }
                });
            if !open {
                self.show_glossary = false;
            }
        }

        // Passive ADS-B alert toasts — drawn over any tab.
        self.adsb_panel.render_toasts(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Auto-save session state on clean exit so next launch resumes where we left off.
        if let Ok(state) = self.shared.try_lock() {
            let mut cfg = state.config.clone();
            cfg.last_session_freq_hz = state.source.frequency_hz;
            cfg.last_session_center_hz = state.source.center_frequency_hz;
            cfg.last_session_gain_db = state.source.gain_db;
            cfg.default_sample_rate = state.source.sample_rate_hz;
            cfg.last_session_demod = state.demod_mode.label().to_string();
            cfg.source_preferences = crate::config::SourcePreferences::capture(&state.source);
            cfg.radio_profiles.insert(
                state.demod_mode.label().to_string(),
                crate::config::RadioModeProfile::capture(&state.config.advanced),
            );
            cfg.recent_frequencies = state.freq_history.iter().copied().collect();
            let (min_db, max_db) = state.spectrum.display_range();
            cfg.spectrum_min_db = min_db;
            cfg.spectrum_max_db = max_db;
            cfg.ppm_correction = state.source.ppm_correction;
            cfg.vfo_b_hz = state.vfo_b;
            cfg.wf_min_db = state.spectrum.wf_min_db;
            cfg.wf_max_db = state.spectrum.wf_max_db;
            cfg.lo_offset_hz = state.lo_offset_hz;
            cfg.color_map = state.spectrum.color_map.name().to_string();
            cfg.freq_memory_hz = state.freq_memory.iter().map(|m| m.freq_hz).collect();
            cfg.freq_memory_labels = state.freq_memory.iter().map(|m| m.label.clone()).collect();
            cfg.save();
            state.spectrum.save_signal_history();
        }
    }
}

pub struct SharedSnapshot {
    pub jobs: Vec<crate::scheduler::ScheduledJob>,
    pub custom_tasks: Vec<crate::scheduler::CustomTask>,
    pub auto_tune_enabled: bool,
}

// ── Main tab render methods ───────────────────────────────────────────────────

impl CentralApp {
    /// Top mode bar: the entire top-level navigation. Left = the 3 task modes
    /// (icon + word); right = the ambient `🤖 Ask` slide-over toggle and the
    /// `⚙ More` deck of hidden tools (opened as a menu, revealed by user level).
    fn render_mode_bar(&mut self, ui: &mut egui::Ui) {
        crate::mode_bar::render_mode_bar(
            ui,
            &self.shared,
            &mut self.current_tab,
            &mut self.active_secondary_tool,
            &mut self.adsb_panel,
            &mut self.quick_start,
            &mut self.show_keyboard_help,
            &mut self.ai_ask_open,
        );
    }

    /// Bottom status strip: plain-language summary of what's happening —
    /// frequency, demod mode, signal strength word, and the REC indicator,
    /// ending with the latest transient status flash.
    fn render_status_strip(&mut self, ui: &mut egui::Ui) {
        match self.current_tab {
            AppTab::Meteor => ui.horizontal(|ui| {
                ui.label(if self.meteor_decoder.running {
                    "Decoding recording"
                } else {
                    "Meteor · offline decoder"
                });
                if let Some(message) = self.status_bar.current() {
                    ui.separator();
                    ui.label(&message.text);
                }
            }),
            AppTab::Planes => ui.horizontal(|ui| {
                ui.label(self.adsb_panel.receiver_status());
                ui.separator();
                ui.label(format!("{} aircraft", self.adsb_panel.aircraft.len()));
            }),
            _ => return self.status_bar.render_strip(ui, &self.shared),
        };
    }

    fn render_secondary_panel(
        &mut self,
        ui: &mut egui::Ui,
        tool: SecondaryTool,
        snapshot: &Option<SharedSnapshot>,
    ) {
        self.recorder_panel
            .set_audio_sample_rate(self.audio.sample_rate());
        let mut ctx = crate::secondary_panel::SecondaryPanelContext {
            shared: &self.shared,
            active_secondary_tool: &mut self.active_secondary_tool,
            bookmark_panel: &mut self.bookmark_panel,
            scheduler_panel: &mut self.scheduler_panel,
            demod: &mut self.demod_worker,
            scanner: &mut self.scanner,
            ai_panel: &mut self.ai_panel,
            ai_ask_open: &mut self.ai_ask_open,
            status_bar: &mut self.status_bar,
            recorder_panel: &mut self.recorder_panel,
            radio_ui: &mut self.radio_ui,
            howto_panel: &mut self.howto_panel,
            discord_panel: &mut self.discord_panel,
            discord: &mut self.discord,
            mqtt: &mut self.mqtt,
            web_remote: &mut self.web_remote,
            rigctl: &mut self.rigctl,
            customize_panel: &mut self.customize_panel,
            last_manual_tune_time: &mut self.last_manual_tune_time,
            session_notes: &mut self.session_notes,
        };
        crate::secondary_panel::render_secondary_panel(&mut ctx, ui, tool, snapshot);
    }

    fn render_sdr_tab(&mut self, ui: &mut egui::Ui, snapshot: &Option<SharedSnapshot>) {
        let _ = snapshot;
        egui::Panel::top("radio_transport")
            .exact_size(crate::radio_ui::TOOLBAR_HEIGHT)
            .show(ui, |ui| self.radio_ui.toolbar(ui));
        if self.radio_ui.show_sidebar {
            egui::Panel::left("sdr_modules")
                .resizable(true)
                .default_size(crate::radio_ui::SIDEBAR_WIDTH)
                .size_range(200.0..=400.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.radio_ui.sidebar(ui));
                });
            // Module shortcuts are painted inside RadioUi, then applied here
            // after the sidebar closure releases its mutable borrow.
            if let Some(tool) = self.radio_ui.take_requested_tool() {
                self.active_secondary_tool = Some(tool);
            }
        }
        egui::Panel::right("radio_spectrum_rail")
            .exact_size(64.0)
            .show(ui, |ui| {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.spectrum.ui_radio_rail(ui);
                }
            });
        if std::mem::take(&mut self.radio_ui.dsp_changed) {
            if let Ok(state) = self.shared.try_lock() {
                self.sync_demod_advanced(&state);
            }
        }
        if std::mem::take(&mut self.radio_ui.audio_changed) {
            self.audio.shutdown();
            self.daemon_audio.reset();
            self.demod_worker.request_reset();
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if let Ok(mut state) = self.shared.try_lock() {
                if state.spectrum.bookmark_freqs_dirty {
                    state.spectrum.bookmark_freqs = state.bookmarks.bookmarks.iter()
                        .map(|b| (b.frequency_hz, b.name.clone(), b.category.clone())).collect();
                    state.spectrum.bookmark_freqs_dirty = false;
                }
                if state.source.source_mode != crate::source_manager::SourceMode::Daemon {
                    let center = state.source.capture_center_frequency_hz();
                    let rate = crate::radio_ui::effective_radio_rate(&state);
                    state.spectrum.update_params_exact(center, rate);
                }
                state.spectrum.vfo_bw_hz = crate::radio_ui::channel_bandwidth(&state) as u32;
                state.spectrum.vfo_freq_hz = Some(state.source.frequency_hz);
                state.spectrum.vfo_b_freq = state.vfo_b;
                state.spectrum.demod_mode = state.demod_mode.resolve(state.source.frequency_hz).label().to_string();
                state.spectrum.scan_marker = if self.scanner.enabled && !self.scanner.paused {
                    Some(self.scanner.current_freq_hz)
                } else { None };
                state.spectrum.squelch_db = state.squelch;
                state.spectrum.source_running = state.source.status == crate::source_manager::SourceStatus::Running;
                state.spectrum.fill_top = state.config.theme_config.spectrum_gradient.sample(0.0).to_egui();
                state.spectrum.fill_bot = state.config.theme_config.spectrum_gradient.sample(1.0).to_egui();
                state.spectrum.signal_glow = state.config.theme_config.glow;
                state.spectrum.plot_bg = state.config.theme_config.waterfall_bg.to_egui();
                state.spectrum.grid_color = state.config.theme_config.spectrum_grid.to_egui();
                state.spectrum.curve_color = state.config.theme_config.spectrum_line.to_egui();
                state.spectrum.noise_floor_color = state.config.theme_config.noise_floor_line.to_egui();
                state.spectrum.color_success = state.config.theme_config.success.to_egui();
                state.spectrum.color_warning = state.config.theme_config.warning.to_egui();
                state.spectrum.color_error = state.config.theme_config.error.to_egui();
                {
                    let translucent = |c: crate::theme::Rgba| {
                        egui::Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
                    };
                    let (ham, bcast, air, mar, wx, sat, mob, ism) = {
                        let bp = &state.config.theme_config;
                        (
                            bp.bandplan_ham,
                            bp.bandplan_broadcast,
                            bp.bandplan_aviation,
                            bp.bandplan_marine,
                            bp.bandplan_weather,
                            bp.bandplan_satellite,
                            bp.bandplan_mobile,
                            bp.bandplan_ism,
                        )
                    };
                    state.spectrum.bandplan_ham = translucent(ham);
                    state.spectrum.bandplan_broadcast = translucent(bcast);
                    state.spectrum.bandplan_aviation = translucent(air);
                    state.spectrum.bandplan_marine = translucent(mar);
                    state.spectrum.bandplan_weather = translucent(wx);
                    state.spectrum.bandplan_satellite = translucent(sat);
                    state.spectrum.bandplan_mobile = translucent(mob);
                    state.spectrum.bandplan_ism = translucent(ism);
                }
                let sq_active = state.squelch > -90.0 && state.spectrum.vfo_signal_level() > state.squelch;
                state.spectrum.signal_active = sq_active;
                if sq_active {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                    state.spectrum.last_signal_unix = Some(now);
                }
                let previous_bandwidth = state.spectrum.vfo_bw_hz;
                state.spectrum.fft_controls_enabled = state.source.source_mode != crate::source_manager::SourceMode::Daemon;
                state.spectrum.ui_radio_workspace(ui);
                crate::radio_ui::persist_display_settings(&mut state);
                if state.source.source_mode != crate::source_manager::SourceMode::Daemon
                    && state.spectrum.vfo_bw_hz != previous_bandwidth {
                    state.config.advanced.radio_bandwidth_hz = state.spectrum.vfo_bw_hz as f32;
                }
                if let Some(freq) = state.spectrum.clicked_tune_freq.take() {
                    crate::radio_ui::tune(&mut state, freq);
                    self.last_manual_tune_time = std::time::Instant::now();
                }
                if let Some(freq) = state.spectrum.pending_vfo_b_freq.take() {
                    state.vfo_b = freq;
                    self.status_bar.info(format!("🔷 VFO B → {:.4} MHz", freq as f64 / 1e6));
                }
                if let Some(freq) = state.spectrum.pending_bookmark_freq.take() {
                    let mode = state.demod_mode.label().to_string();
                    state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                        name: format!("{:.4} MHz {}", freq as f64 / 1e6, mode),
                        frequency_hz: freq, mode, bandwidth_hz: 12_500,
                        category: "Quick".to_string(), notes: String::new(), starred: false,
                    });
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
                if let Some(sq) = state.spectrum.pending_squelch_db.take() { crate::radio_ui::set_power_squelch_level(&mut state, sq); }
                let mut scan_range_changed = false;
                if let Some(hz) = state.spectrum.pending_scan_start.take() {
                    self.scanner.start_hz = hz;
                    scan_range_changed = true;
                }
                if let Some(hz) = state.spectrum.pending_scan_stop.take() {
                    self.scanner.stop_hz = hz;
                    scan_range_changed = true;
                }
                if scan_range_changed {
                    self.scanner.ensure_current_in_range();
                }
                if let Some(mode_str) = state.spectrum.pending_demod_mode.take() {
                    if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&mode_str) { state.demod_mode = mode; }
                }
                if state.spectrum.pending_start_source {
                    state.spectrum.pending_start_source = false;
                    self.radio_ui.start_receiver(&mut state);
                }
                if let Some(freq) = state.spectrum.pending_ai_freq.take() {
                    let freq_mhz = freq as f64 / 1e6;
                    self.ai_panel.input = format!("I'm looking at {freq_mhz:.4} MHz on the spectrum. What signals might be here? What demod mode?");
                    self.status_bar.info(format!("🤖 AI prompt for {freq_mhz:.3} MHz"));
                }
            }
        });
    }

    fn render_adsb_tab(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("aircraft_list")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                self.adsb_panel.ui_list(ui);
                if let Some(prompt) = self.adsb_panel.pending_ai_prompt.take() {
                    self.ai_panel.input = prompt;
                    self.status_bar
                        .info("🤖 Aircraft details sent to AI".to_string());
                }
            });
        // Map dominates the tab; on-page receive instructions live in a collapsed
        // banner so the map stays the primary focus by default.
        egui::Panel::top("adsb_instructions")
            .resizable(false)
            .exact_size(if self.adsb_instructions_open {
                260.0
            } else {
                24.0
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let arrow = if self.adsb_instructions_open {
                        "▼"
                    } else {
                        "▶"
                    };
                    if ui
                        .small_button(format!(
                            "{arrow} 📡 How to receive ADS-B (antenna, 1090 MHz setup)"
                        ))
                        .on_hover_text(
                            "Show/hide setup instructions for receiving live aircraft data",
                        )
                        .clicked()
                    {
                        self.adsb_instructions_open = !self.adsb_instructions_open;
                    }
                });
                if self.adsb_instructions_open {
                    egui::ScrollArea::vertical()
                        .id_salt("adsb_instructions_scroll")
                        .show(ui, |ui| {
                            self.adsb_panel.ui_antenna_guide(ui);
                        });
                }
            });
        egui::Panel::top("adsb_band_picker").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Region / band");
                if self.adsb_panel.ui_region_picker(ui) {
                    self.adsb_decoder = AdsBDecoder::new();
                    self.seen_aircraft.clear();
                }
                ui.separator();
                ui.label(
                    egui::RichText::new(self.adsb_panel.receiver_status())
                        .small()
                        .weak(),
                );
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.adsb_panel.ui_map(ui);
        });
    }

    /// Imported recordings have their own decoder state, independent of live sources.
    fn render_meteor_tab(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.heading("Meteor LRPT decoder");
                ui.label(
                    egui::RichText::new("· offline .cs8 / .cf32 import").color(egui::Color32::GRAY),
                );
            });
            ui.label(
                egui::RichText::new(
                    "Decode Meteor-M2 recordings without touching the live SDR source.",
                )
                .small()
                .color(egui::Color32::GRAY),
            );
            ui.add_space(8.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.meteor_decoder.ui_offline(ui);
                    });
            });
        });
    }

    fn render_satellite_tab(&mut self, ui: &mut egui::Ui, snapshot: &Option<SharedSnapshot>) {
        let _ = snapshot;
        crate::satellite_tab::render_satellite_tab(
            ui,
            &self.shared,
            &mut self.satellite_panel,
            &mut self.satellite_subtab,
            &mut self.constellation,
            &mut self.decoding_panel,
            &mut self.ai_panel,
            &mut self.status_bar,
        );
    }
}

#[cfg(test)]
#[path = "app_ui_tests.rs"]
mod app_ui_tests;
