use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct FreqMemEntry {
    pub freq_hz: u64,
    pub label: String,
}

#[cfg(test)]
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
use crate::howto_panel::HowToPanel;
use crate::mqtt::{MqttPublisher, TelemetryView};
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

#[path = "app_runtime_actions.rs"]
mod runtime_actions;
pub(crate) use runtime_actions::{
    PendingRuntimeAction, RuntimeAction, RuntimeActionRequest, RUNTIME_ACTION_CAPACITY,
};

pub struct SharedState {
    pub source: SourceManager,
    pub spectrum: SpectrumAnalyzer,
    pub config: AppConfig,
    /// IQ sample format produced by the current source.
    pub input_format: dump1090::demod::InputFormat,
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
    /// Every manual tuning route uses the same monotonic scheduler cooldown.
    pub last_manual_tune: Option<Instant>,
    pub(crate) runtime_actions: VecDeque<RuntimeActionRequest>,
}

impl SharedState {
    /// Set the IQ sample format produced by the current source.
    pub fn set_input_format(&mut self, format: dump1090::demod::InputFormat) {
        self.input_format = format;
    }
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
    quick_start: crate::quick_start::QuickStartWizard,
    web_remote: WebRemote,
    rigctl: RigctlServer,
    mqtt: MqttPublisher,
    demod_worker: DemodWorker,
    last_radio_boundary: Option<(u64, Option<u64>, u64)>,
    last_radio_lost_samples: u64,
    last_radio_profile: Option<(crate::sdr_panel::DemodMode, crate::config::RadioModeProfile)>,
    daemon_audio: crate::audio_resampler::AudioResampler,
    audio: AudioOutput,
    local_adsb: crate::local_adsb::LocalAdsbWorker,
    secondary_vfo: crate::secondary_vfo::SecondaryVfo,
    georef_jobs: Vec<(
        bool,
        std::sync::mpsc::Receiver<Result<lrpt_decode::geolocation::GeoreferenceProducts, String>>,
    )>,
    last_local_adsb_boundary: Option<(u64, Option<u64>, u64)>,
    #[cfg(test)]
    adsb_decoder: AdsBDecoder,
    last_local_adsb_epoch: Option<u64>,
    last_adsb_capture: Option<(u64, u32, crate::source_manager::SourceMode, u64, bool)>,
    scanner: crate::scanner::FrequencyScanner,
    last_scheduler_update: std::time::Instant,
    runtime_action: Option<PendingRuntimeAction>,
    runtime_error_serial: u64,
    runtime_last_error: String,
    last_source_status: crate::source_manager::SourceStatus,
    last_auto_tuned_job: Option<(u64, u64, u64, u64)>,
    last_history_freq: u64,
    freq_history_idx: Option<usize>,
    recording_start: Option<std::time::Instant>,
    bm_last_observed: Vec<crate::bookmarks::Bookmark>,
    bookmark_path: std::path::PathBuf,
    bm_dirty_since: Option<std::time::Instant>,
    // Frequency jump dialog
    show_freq_jump: bool,
    freq_jump_input: String,
    freq_jump_matches: Vec<(String, u64)>,
    // SDR glossary popup
    show_glossary: bool,
    // First strong signal celebration
    first_strong_signal_seen: bool,
    last_audio_source: Option<(
        u64,
        u64,
        crate::radio_iq::RadioIqConfig,
        crate::source_manager::SourceMode,
        u64,
    )>,
    theme_applied: bool,
    discord: DiscordNotifier,
    last_recording: bool,
    last_adsb_running: bool,
    last_scanner_enabled: bool,
    last_mqtt_connected: bool,
    seen_aircraft: std::collections::HashSet<u32>,
    notified_aircraft: std::collections::HashSet<u32>,
    last_active_pass_job: Option<(u64, String)>,
    discord_summary_last: std::time::Instant,
    current_tab: AppTab,
    last_traffic_bucket: usize,
    active_secondary_tool: Option<SecondaryTool>,
    /// ⚙ More — whether the tool-picker menu (deck of hidden tools) is showing.
    #[allow(dead_code)]
    more_menu_open: bool,
    satellite_subtab: crate::satellite_panel::SatelliteSubTab,
    close_pending: bool,
}

impl CentralApp {
    /// Keep the event loop alive while owned file finalizers finish. A blocked
    /// driver cannot freeze close handling or imply a completed recording.
    fn tick_close_request(&mut self, ctx: &egui::Context) -> bool {
        if !self.close_pending && !ctx.input(|input| input.viewport().close_requested()) {
            return false;
        }
        if !self.close_pending {
            self.close_pending = true;
            self.recorder_panel.squelch_record = false;
            self.recorder_panel.stop_recording();
            self.satellite_panel.stop_cf32_recording();
            self.audio.shutdown();
            self.secondary_vfo.stop();
            self.decoding_panel.stop_live();
            self.meteor_decoder.stop_live();
        }
        self.decoding_panel.tick_live();
        self.meteor_decoder.tick_live();
        let source_cancelled = if let Ok(mut state) = self.shared.try_lock() {
            state.source.stop();
            state.adsb_running = false;
            true
        } else {
            false
        };
        self.recorder_panel.tick();
        self.satellite_panel.poll_cf32_finalization();
        if !source_cancelled
            || self.recorder_panel.is_finalizing()
            || self.satellite_panel.is_cf32_finalizing()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status_bar
                .info("Finalizing recordings before closing…".into());
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        true
    }

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
                    // This reports an already-applied change. Replaying it as
                    // a command would overwrite a newer manual frequency.
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
            input_format: dump1090::demod::InputFormat::Uc8,
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
            last_manual_tune: None,
            runtime_actions: VecDeque::new(),
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
            let observer = (state.config.observer_lat, state.config.observer_lon);
            state.tle.set_observer(observer.0, observer.1);
            // Restore recent frequency history from config
            let saved_recent: Vec<u64> = state.config.recent_frequencies.clone();
            for hz in saved_recent {
                if hz > 0 {
                    state.freq_history.push_back(hz);
                }
            }
            // Preserve the loaded DSP profile before RadioUi's mode-sync runs.
            state.config.last_session_demod = state.demod_mode.label().into();
            let init_freq = state.source.frequency_hz;
            if state.freq_history.is_empty() || state.freq_history.back() != Some(&init_freq) {
                state.freq_history.push_back(init_freq);
            }
        }

        let capture_stream = shared
            .lock()
            .expect("shared state mutex poisoned")
            .source
            .capture_stream();
        let demod_worker = crate::demod::DemodWorker::new_capture(capture_stream.clone());
        let mut recorder_panel = RecorderPanel::new(shared.clone());
        recorder_panel.set_audio_stream(demod_worker.audio_stream());
        let mut adsb_panel = AdsBPanel::new(shared.clone());
        adsb_panel.attach_native_capture(capture_stream.clone());
        let secondary_vfo = crate::secondary_vfo::SecondaryVfo::new(capture_stream.clone());
        let mut app = Self {
            shared: shared.clone(),
            event_bus: crate::events::EventBus::new(),
            keyboard_handler: crate::keyboard::KeyboardHandler::new(),
            status_bar: crate::status_bar::StatusBar::new(),
            frequency_history: crate::frequency_history::FrequencyHistory::new(),
            radio_ui: crate::radio_ui::RadioUi::new(shared.clone()),
            satellite_panel: SatellitePanel::new(shared.clone()),
            adsb_panel,
            recorder_panel,
            constellation: ConstellationDisplay::new(),
            decoding_panel: DecodingPanel::default(),
            meteor_decoder: DecodingPanel::default(),
            ai_panel: AiPanel::new(shared.clone()),
            howto_panel: HowToPanel::new(),
            quick_start: crate::quick_start::QuickStartWizard::new(),
            web_remote,
            rigctl,
            mqtt,
            discord,
            last_radio_profile: None,
            daemon_audio: crate::audio_resampler::AudioResampler::default(),
            audio: AudioOutput::new(),
            demod_worker,
            last_radio_boundary: None,
            last_radio_lost_samples: 0,
            local_adsb: crate::local_adsb::LocalAdsbWorker::new(capture_stream),
            secondary_vfo,
            georef_jobs: Vec::new(),
            last_local_adsb_boundary: None,
            #[cfg(test)]
            adsb_decoder: AdsBDecoder::new(),
            last_local_adsb_epoch: None,
            last_adsb_capture: None,
            scanner: crate::scanner::FrequencyScanner::new(shared.clone()),
            runtime_action: None,
            close_pending: false,
            runtime_error_serial: 0,
            runtime_last_error: String::new(),
            last_scheduler_update: std::time::Instant::now(),
            last_source_status: crate::source_manager::SourceStatus::Idle,
            last_auto_tuned_job: None,
            last_history_freq: {
                let state = shared.lock().expect("shared state mutex poisoned");
                state.source.frequency_hz
            },
            freq_history_idx: None,
            show_freq_jump: false,
            freq_jump_input: String::new(),
            freq_jump_matches: Vec::new(),
            show_glossary: false,
            first_strong_signal_seen: false,
            last_audio_source: None,
            theme_applied: false,
            recording_start: None,
            bm_last_observed: Vec::new(),
            bookmark_path: "ez_sdr_bookmarks.json".into(),
            bm_dirty_since: None,
            last_recording: false,
            last_adsb_running: false,
            last_scanner_enabled: false,
            last_mqtt_connected: false,
            seen_aircraft: std::collections::HashSet::new(),
            notified_aircraft: std::collections::HashSet::new(),
            last_active_pass_job: None,
            current_tab: AppTab::Listen,
            discord_summary_last: std::time::Instant::now(),
            last_traffic_bucket: 0,
            active_secondary_tool: None,
            more_menu_open: false,
            satellite_subtab: crate::satellite_panel::SatelliteSubTab::Track,
        };
        // Apply saved advanced settings onto the live engines so a fresh launch
        // reproduces the user's last profile.
        app.apply_advanced();
        app
    }
}

impl CentralApp {
    /// Apply saved receiver, display and DSP settings before capture starts.
    pub fn apply_advanced(&mut self) {
        let mut shared = self.shared.lock().expect("shared state mutex poisoned");
        crate::advanced_panel::push_advanced(&mut self.demod_worker, &mut shared);
    }

    /// Forward the current Advanced-panel DSP settings to the demod worker.
    /// `state` must be a live [`SharedState`] guard.
    fn sync_demod_advanced(&self, state: &SharedState) -> bool {
        let mode = state.demod_mode.resolve(state.source.frequency_hz);
        self.demod_worker
            .configure_advanced(&state.config.advanced, mode)
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

    /// Scheduler lifecycle runs regardless of the selected/rendered tab.
    fn tick_satellite_jobs(&mut self, now_unix: f64, now: Instant) {
        let shared = Arc::clone(&self.shared);
        let Ok(mut state) = shared.try_lock() else {
            return;
        };
        let job = state.scheduler.active_radio_job(now_unix).cloned();
        if let Some(job) = job {
            if self
                .last_active_pass_job
                .as_ref()
                .is_none_or(|(id, _)| *id != job.id)
            {
                self.event_bus
                    .publish(crate::events::AppEvent::SatellitePassStarting {
                        name: job.satellite.clone(),
                        aos: job.aos.clone(),
                    });
                if let Some(pass) = state
                    .tle
                    .upcoming_passes()
                    .iter()
                    .find(|pass| pass.satellite == job.satellite)
                {
                    self.discord.fire(
                        "sat_aos",
                        crate::discord::embed_sat_aos(
                            &job.satellite,
                            job.frequency_hz,
                            pass.max_elevation,
                        ),
                    );
                }
                self.last_active_pass_job = Some((job.id, job.satellite.clone()));
            }
            let manual_recent = state
                .last_manual_tune
                .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(15));
            if self
                .last_auto_tuned_job
                .is_some_and(|(id, _, _, _)| id == job.id)
            {
                self.satellite_panel.auto_tune_status =
                    Some(format!("Pass tune completed for {}", job.satellite));
            } else if manual_recent {
                self.satellite_panel.auto_tune_status = Some("Waiting after manual tune".into());
            } else {
                let frequency =
                    crate::satellite_panel::pass_center_frequency(job.frequency_hz, 0.0);
                // Reconcile even an equal VFO: a replay/source transition can
                // change which frequencies the current capture actually covers.
                // Receiver control synchronization coalesces unchanged requests.
                crate::radio_ui::tune_scheduled(&mut state, frequency);
                match &state.source.status {
                    crate::source_manager::SourceStatus::Opening => {
                        self.satellite_panel.auto_tune_status = Some("Waiting for receiver".into());
                    }
                    crate::source_manager::SourceStatus::Error(error) => {
                        self.satellite_panel.auto_tune_status =
                            Some(format!("Tune failed: {error}"));
                    }
                    _ if state.source.frequency_hz != frequency => {
                        self.satellite_panel.auto_tune_status =
                            Some("Scheduled frequency is outside the available capture".into());
                    }
                    _ if !state.source.capture_tuning_applied() => {
                        self.satellite_panel.auto_tune_status =
                            Some("Waiting for capture acknowledgement".into());
                    }
                    _ => {
                        self.last_auto_tuned_job = Some((
                            job.id,
                            state.source.frequency_hz,
                            state.source.capture_center_frequency_hz(),
                            state.source.stream_generation(),
                        ));
                        self.event_bus
                            .publish(crate::events::AppEvent::FrequencyChanged { hz: frequency });
                        let action = if state.source.status
                            == crate::source_manager::SourceStatus::Running
                        {
                            "Auto-tuned"
                        } else {
                            "Prepared VFO"
                        };
                        self.satellite_panel.auto_tune_status =
                            Some(format!("{action} for {}", job.satellite));
                        self.status_bar.info(format!(
                            "{action} to {} ({:.3} MHz)",
                            job.satellite,
                            frequency as f64 / 1e6
                        ));
                    }
                }
            }
            self.satellite_panel.doppler_hz =
                state
                    .tle
                    .doppler_shift_for_sat(&job.satellite, job.frequency_hz as f64, now_unix);
        } else {
            if let Some((_, satellite)) = self.last_active_pass_job.take() {
                self.discord
                    .fire("sat_los", crate::discord::embed_sat_los(&satellite));
            }
            self.last_auto_tuned_job = None;
            self.satellite_panel.auto_tune_status = None;
        }
        if let Some((label, frequency)) = state.scheduler.poll_custom_tasks(now_unix) {
            crate::radio_ui::tune_scheduled(&mut state, frequency);
            self.discord.fire(
                "task_fired",
                crate::discord::embed_task_fired(&label, frequency),
            );
        }
    }

    fn apply_scanner_tune(&mut self) {
        if let Ok(mut state) = self.shared.try_lock() {
            self.scanner.apply_tune_request(&mut state);
        }
    }

    fn sync_adsb_capture(&mut self) {
        let capture = self.shared.try_lock().ok().map(|state| {
            self.local_adsb.configure(
                state.adsb_running
                    && self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090
                    && state.source.source_mode != crate::source_manager::SourceMode::Daemon,
                Some((state.config.observer_lat, state.config.observer_lon)),
            );
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
                #[cfg(test)]
                {
                    self.adsb_decoder = AdsBDecoder::new();
                }
                self.last_local_adsb_epoch = None;
                self.last_local_adsb_boundary = None;
                self.seen_aircraft.clear();
                self.notified_aircraft.clear();
                if self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090 {
                    self.adsb_panel.clear_tracks();
                }
                self.last_adsb_capture = Some(capture);
            }
        }
    }

    #[cfg(test)]
    fn feed_local_adsb_capture(
        &mut self,
        samples: &[u8],
        clock: ez_proto::CaptureClock,
        format: dump1090::demod::InputFormat,
    ) {
        if self.last_local_adsb_epoch != Some(clock.epoch) {
            // Rewinds can occur inside the same source worker. Receiver state,
            // trails and pending enrichment belong to the new acquisition epoch.
            self.adsb_decoder = AdsBDecoder::new();
            self.adsb_panel.clear_tracks();
            self.seen_aircraft.clear();
            self.notified_aircraft.clear();
            self.last_local_adsb_epoch = Some(clock.epoch);
        } else if clock.discontinuous {
            self.adsb_panel.clear_tracks();
        }
        self.adsb_decoder.feed_iq_with_clock(samples, clock, format);
        if self.adsb_decoder.sample_rate_mismatch() {
            self.status_bar
                .warning("⚠ ADS-B: sample rate mismatch (expected 2.4 MHz)".to_string());
        }
        self.adsb_panel.aircraft = self.adsb_decoder.get_aircraft();
        self.adsb_panel
            .apply_receiver_snapshot(self.adsb_decoder.receiver_snapshot());
    }

    fn apply_local_adsb_snapshot(&mut self) {
        let Some((generation, enabled)) = self.shared.try_lock().ok().map(|state| {
            (
                state.source.stream_generation(),
                state.adsb_running
                    && state.source.source_mode != crate::source_manager::SourceMode::Daemon
                    && self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090,
            )
        }) else {
            return;
        };
        if !enabled {
            return;
        }
        let Some(snapshot) = self.local_adsb.snapshot() else {
            return;
        };
        if generation == 0 || snapshot.generation != generation {
            return;
        }
        let boundary = (
            snapshot.generation,
            snapshot.epoch,
            snapshot.discontinuity_revision,
        );
        if self.last_local_adsb_boundary != Some(boundary) {
            self.adsb_panel.clear_tracks();
            if self.last_local_adsb_epoch != snapshot.epoch {
                self.seen_aircraft.clear();
                self.notified_aircraft.clear();
            }
            self.last_local_adsb_epoch = snapshot.epoch;
            self.last_local_adsb_boundary = Some(boundary);
            if snapshot.total_lost_samples > 0 {
                self.status_bar.warning(format!(
                    "ADS-B input overflow: {} samples lost in {} blocks",
                    snapshot.total_lost_samples, snapshot.dropped_blocks,
                ));
            }
        }
        if snapshot.sample_rate_mismatch {
            self.status_bar
                .warning("⚠ ADS-B: sample rate mismatch (expected 2.4 MHz)".into());
        }
        self.adsb_panel.aircraft.clone_from(&snapshot.aircraft);
        self.adsb_panel
            .apply_receiver_snapshot(snapshot.receiver.clone());
    }

    /// Drain queued daemon events (only produces events in `SourceMode::Daemon`)
    /// and apply them: spectrum frames feed the analyser, errors surface as a
    /// status flash. Hardware shadow fields are applied inside
    /// `SourceManager::recv_daemon_event`; audio and aircraft are routed here into the
    /// desktop's existing playback and tracking state.
    fn map_daemon_aircraft(
        item: ez_proto::AircraftTelemetry,
    ) -> Option<crate::adsb_panel::AircraftEntry> {
        let now = std::time::Instant::now();
        let wall_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let observation_time = |timestamp: u64| {
            now.checked_sub(std::time::Duration::from_millis(
                wall_ms.saturating_sub(timestamp),
            ))
            .unwrap_or(now - std::time::Duration::from_secs(600))
        };
        let position = item.lat.zip(item.lon).filter(|&(lat, lon)| {
            lat.is_finite()
                && lon.is_finite()
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lon)
        });
        let position_seen = item.position_seen_ms.map(observation_time);
        let (lat, lon) = position.unwrap_or((0.0, 0.0));
        let track = item.track_deg.filter(|track| track.is_finite());
        let quality = item.position_quality;
        Some(crate::adsb_panel::AircraftEntry {
            icao: item.icao,
            address_type: if item.address_type.is_empty() {
                "unknown".into()
            } else {
                item.address_type
            },
            source: if item.source.is_empty() {
                "unknown".into()
            } else {
                item.source
            },
            callsign: item.callsign.unwrap_or_default(),
            lat,
            lon,
            altitude: item.altitude_ft,
            altitude_type: if item.altitude_geometric {
                crate::adsb_panel::AltitudeType::Geometric
            } else {
                crate::adsb_panel::AltitudeType::Barometric
            },
            speed: item
                .ground_speed_kt
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| v.round() as u32),
            airspeed: item
                .airspeed_kt
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| v.round() as u32),
            airspeed_true: item.airspeed_true,
            on_ground: item.on_ground,
            heading_true: item.heading_true,
            ground_track: track.map(|v| v.round() as u32 % 360),
            heading: item
                .heading_deg
                .filter(|v| v.is_finite())
                .map(|v| v.rem_euclid(360.0).round() as u32 % 360),
            squawk: item.squawk,
            vertical_rate: item.vertical_rate_fpm,
            emergency: item.emergency,
            surface_movement: None,
            surface_heading: item
                .surface_direction_deg
                .filter(|v| v.is_finite())
                .map(|v| v.rem_euclid(360.0).round() as u32 % 360),
            has_position: position.is_some()
                && position_seen
                    .is_some_and(|seen| seen.elapsed() <= std::time::Duration::from_secs(60)),
            has_heading: track.is_some()
                || item.heading_deg.is_some_and(f64::is_finite)
                || item.surface_direction_deg.is_some_and(f64::is_finite),
            position_seen,
            position_source: item.position_source,
            seen: observation_time(item.last_seen_ms),
            reliable: quality.as_ref().is_some_and(|quality| quality.reliable),
            quality_known: quality.is_some(),
            correctedbits: quality.as_ref().map_or(0, |quality| quality.corrected_bits),
            score: quality.as_ref().map_or(0.0, |quality| quality.score),
        })
    }

    fn save_bookmarks_locked(&mut self, state: &mut SharedState) {
        state.bookmarks_modified = true;
        match state.bookmarks.save_to(&self.bookmark_path) {
            Ok(()) => {
                state.bookmarks_modified = false;
                self.bm_last_observed = state.bookmarks.bookmarks.clone();
                self.bm_dirty_since = None;
            }
            Err(error) => {
                self.status_bar
                    .error(format!("Could not save bookmarks: {error}"));
                // Keep dirty state and retry after the debounce, avoiding an I/O
                // failure on every frame when the destination remains unavailable.
                self.bm_dirty_since = Some(Instant::now());
            }
        }
    }

    fn tick_bookmark_autosave(&mut self) {
        let shared = Arc::clone(&self.shared);
        let Ok(mut state) = shared.try_lock() else {
            return;
        };
        if state.bookmarks.bookmarks != self.bm_last_observed {
            self.bm_last_observed = state.bookmarks.bookmarks.clone();
            state.bookmarks_modified = true;
            self.bm_dirty_since = Some(Instant::now());
        }
        if state.bookmarks_modified && self.bm_dirty_since.is_none() {
            self.bm_dirty_since = Some(Instant::now());
        }
        if self
            .bm_dirty_since
            .is_some_and(|since| since.elapsed() >= Duration::from_secs(15))
        {
            self.save_bookmarks_locked(&mut state);
        }
    }

    /// Clean exit bypasses debounce and waits for the shared-state guard.
    fn flush_bookmarks(&mut self) {
        let shared = Arc::clone(&self.shared);
        let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
        if state.bookmarks_modified
            || self.bm_dirty_since.is_some()
            || state.bookmarks.bookmarks != self.bm_last_observed
        {
            self.save_bookmarks_locked(&mut state);
        }
    }

    fn apply_daemon_recording(
        &mut self,
        status: ez_proto::RecordingStatus,
        state: &mut SharedState,
    ) {
        if status.channel_id != state.source.daemon_channel_ids().spectrum {
            return;
        }
        state.recording = status.active;
        self.recorder_panel
            .handle_daemon_recording(status, state.source.stream_generation());
    }

    fn process_web_remote_commands(&mut self) {
        let web_cmds = self.web_remote.poll_commands();
        if !web_cmds.is_empty() {
            let mut actions = Vec::new();
            let mut errors = Vec::new();
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
                            if let Err(error) =
                                crate::radio_ui::set_power_squelch_level(&mut state, db)
                            {
                                errors.push(error);
                            }
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
            for error in errors {
                self.status_bar.warning(format!("Squelch: {error}"));
                self.web_remote
                    .broadcast_command_error("set_squelch", &error);
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
    }

    fn tick_meteor_workflows(&mut self) {
        self.decoding_panel.tick_live();
        self.meteor_decoder.tick_live();
        let shared = Arc::clone(&self.shared);
        for meteor in [false, true] {
            let other_live = if meteor {
                self.decoding_panel.live_active
            } else {
                self.meteor_decoder.live_active
            };
            let panel = if meteor {
                &mut self.meteor_decoder
            } else {
                &mut self.decoding_panel
            };
            if panel.live_active {
                if let Ok(state) = shared.try_lock() {
                    let finalize_error = panel
                        .is_daemon_live()
                        .then(|| state.source.daemon_channel_error())
                        .flatten();
                    if let Some(error) = finalize_error
                        .filter(|_| panel.is_daemon_live() && !panel.daemon_workflow_active())
                    {
                        panel.abort_daemon_live(&format!("Daemon finalization failed: {error}"));
                    } else if let crate::source_manager::SourceStatus::Error(error) =
                        &state.source.status
                    {
                        if panel.is_daemon_live() {
                            panel.abort_daemon_live(&format!("Receiver failed: {error}"));
                        } else {
                            panel.stop_live();
                            panel.error = Some(format!("Live capture failed: {error}"));
                        }
                    } else if state.source.status == crate::source_manager::SourceStatus::Idle {
                        if panel.is_daemon_live() {
                            panel.abort_daemon_live(
                                "Receiver stopped before LRPT finalization completed.",
                            );
                        } else {
                            panel.stop_live();
                        }
                    }
                }
            }
            if panel.take_live_stop_request() {
                if panel.is_daemon_live() {
                    panel.request_daemon_live_stop();
                } else {
                    panel.stop_live();
                }
            }
            if panel.take_live_start_request() {
                if let Ok(mut state) = shared.try_lock() {
                    if other_live {
                        panel.error = Some("Stop the other live Meteor decoder first.".into());
                        continue;
                    }
                    let satellite = state
                        .selected_satellite
                        .clone()
                        .or_else(|| panel.satellite_name.clone());
                    state.source.start();
                    if let crate::source_manager::SourceStatus::Error(error) = &state.source.status
                    {
                        panel.error = Some(format!("Receiver could not start: {error}"));
                        continue;
                    }
                    if state.source.source_mode == crate::source_manager::SourceMode::Daemon {
                        panel.start_daemon_live(panel.mission_profile, satellite);
                    } else {
                        let epoch = panel.cds_epoch();
                        if let Err(error) = panel.start_live(
                            state.source.capture_stream(),
                            state.source.sample_rate_hz,
                            panel.mission_profile,
                            satellite,
                            epoch,
                        ) {
                            panel.error = Some(error);
                        }
                    }
                } else {
                    panel.error = Some("Receiver is busy; try live decode again.".into());
                }
            }
            if self.georef_jobs.iter().any(|(target, _)| *target == meteor) {
                continue;
            }
            if let Some(request) = panel.take_georeference_request() {
                let inputs = panel.georeference_inputs(request.apid);
                let model = shared
                    .try_lock()
                    .ok()
                    .and_then(|state| state.tle.lrpt_geolocation_model(&request.satellite_name));
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let result = match (inputs, model) {
                        (Some((image, times)), Some(model)) => model.georeference_image(&image, &times, lrpt_decode::geolocation::ScanGeometry {
                            first_pixel_angle_rad: -request.scan_edge_angle_rad,
                            last_pixel_angle_rad: request.scan_edge_angle_rad,
                        }, 100).map_err(|error| error.to_string()),
                        _ => Err("Complete packet timestamps and a loaded matching satellite TLE are required.".into()),
                    };
                    let _ = tx.send(result);
                });
                self.georef_jobs.push((meteor, rx));
            }
        }
        let mut index = 0;
        while index < self.georef_jobs.len() {
            match self.georef_jobs[index].1.try_recv() {
                Ok(result) => {
                    let (meteor, _) = self.georef_jobs.swap_remove(index);
                    let panel = if meteor {
                        &mut self.meteor_decoder
                    } else {
                        &mut self.decoding_panel
                    };
                    panel.complete_georef(result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => index += 1,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let (meteor, _) = self.georef_jobs.swap_remove(index);
                    let panel = if meteor {
                        &mut self.meteor_decoder
                    } else {
                        &mut self.decoding_panel
                    };
                    panel.complete_georef(Err("Geolocation worker stopped unexpectedly.".into()));
                }
            }
        }
    }

    fn drain_daemon_events(&mut self) {
        let shared = Arc::clone(&self.shared);
        let Ok(mut state) = shared.try_lock() else {
            return;
        };
        state.source.sync_daemon_controls();
        let audio_running = state.audio_running;
        let monitor_power = (self.recorder_panel.squelch_record
            || self.recorder_panel.signal_monitor)
            && state.config.advanced.squelch_mode == crate::radio_squelch::SquelchMode::Power;
        let adsb_running = state.adsb_running;
        let demod_mode = state.demod_mode;
        let stereo = state.config.advanced.wfm_stereo;
        let cw_tone = state.config.advanced.cw_tone_hz;
        state.source.set_daemon_radio_audio_options(stereo, cw_tone);
        let meteor_profile = if self.meteor_decoder.live_active {
            self.meteor_decoder.mission_profile
        } else {
            self.decoding_panel.mission_profile
        };
        state.source.set_daemon_meteor_profile(meteor_profile);
        state.source.sync_daemon_workflows(
            audio_running || monitor_power,
            adsb_running,
            self.decoding_panel.daemon_workflow_active()
                || self.meteor_decoder.daemon_workflow_active(),
            demod_mode,
        );
        crate::radio_ui::sync_squelch_level(&mut state);
        for _ in 0..8 {
            let previous_squelch = state.source.daemon_squelch_applied();
            let event = state.source.recv_daemon_event();
            if state.source.daemon_squelch_applied() != previous_squelch {
                self.daemon_audio.reset();
            }
            match event {
                Some(ez_proto::ServerEvent::Spectrum(frame)) => {
                    // Scanner measurements continue independently of tab rendering.
                    if !self.scanner.enabled && !monitor_power {
                        state.spectrum.require_daemon_observation(None);
                    }
                    if self.current_tab == AppTab::Listen || self.scanner.enabled || monitor_power {
                        let observed_at = state
                            .source
                            .daemon_spectrum_received_at()
                            .unwrap_or_else(std::time::Instant::now);
                        state.spectrum.push_spectrum_frame_at(&frame, observed_at);
                    }
                }
                Some(ez_proto::ServerEvent::Error { message }) => {
                    self.runtime_error_serial = self.runtime_error_serial.wrapping_add(1);
                    self.runtime_last_error.clone_from(&message);
                    self.status_bar.warning(format!("⚠ {message}"));
                }
                Some(ez_proto::ServerEvent::Squelch { .. }) => {
                    crate::radio_ui::sync_squelch_level(&mut state);
                    if let Some(error) = &state.source.daemon_squelch_error {
                        self.status_bar.warning(format!("Squelch: {error}"));
                    }
                }
                Some(ez_proto::ServerEvent::Audio(frame)) => {
                    crate::radio_ui::sync_squelch_level(&mut state);
                    if state.audio_running && state.source.accept_daemon_audio(&frame) {
                        let samples = self.daemon_audio.process_frame(
                            &frame,
                            self.audio.sample_rate(),
                            state.volume,
                        );
                        let _ = self.audio.push_audio(samples);
                    }
                }
                Some(ez_proto::ServerEvent::Aircraft(snapshot))
                    if adsb_running
                        && self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090 =>
                {
                    self.adsb_panel.apply_receiver_snapshot(snapshot.receiver);
                    self.adsb_panel.aircraft = snapshot
                        .aircraft
                        .into_iter()
                        .filter_map(Self::map_daemon_aircraft)
                        .collect();
                }
                Some(ez_proto::ServerEvent::Welcome { .. }) => {
                    self.decoding_panel.clear_daemon_telemetry();
                }
                Some(ez_proto::ServerEvent::Telemetry(frame)) => {
                    let panel = if self.meteor_decoder.live_active {
                        &mut self.meteor_decoder
                    } else {
                        &mut self.decoding_panel
                    };
                    if let Err(message) = panel.ingest_daemon_telemetry(frame) {
                        self.status_bar.warning(format!("⚠ {message}"));
                    }
                }
                Some(ez_proto::ServerEvent::StreamGap {
                    channel_id,
                    dropped_frames,
                }) => {
                    if channel_id == state.source.daemon_channel_ids().audio {
                        self.daemon_audio.reset();
                    }
                    self.status_bar.warning(format!(
                        "Daemon channel {channel_id}: {dropped_frames} stream updates skipped"
                    ));
                }
                Some(ez_proto::ServerEvent::Recording(status)) => {
                    self.apply_daemon_recording(status, &mut state);
                }
                Some(_) => {}
                None => break,
            }
        }
        // Drain the prioritized control queue before checking the squelch
        // deadline. A reply may already be queued while the GUI was stalled;
        // timing it out first would discard that valid correlated ACK.
        let desired_squelch = crate::radio_ui::requested_power_squelch(&state);
        state.source.sync_daemon_squelch(desired_squelch);
    }
}

impl eframe::App for CentralApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.tick_close_request(ctx) {
            return;
        }
        // Apply config changes triggered from Settings tab
        if let Ok(mut state) = self.shared.try_lock() {
            if state.config.needs_apply {
                state.config.normalize();
                state.config.needs_apply = false;
                // Apply runtime appearance, DSP and integrations. Source defaults
                // are startup preferences; live tuning stays in receiver controls.
                crate::advanced_panel::push_advanced(&mut self.demod_worker, &mut state);
                // Apply theme
                state.config.theme_config.apply_to_ctx(ctx);
                // Apply font scale
                let scale = state.config.font_scale as f32;
                ctx.set_pixels_per_point(scale);
                let observer = (state.config.observer_lat, state.config.observer_lon);
                state.tle.set_observer(observer.0, observer.1);
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

        self.ai_panel.tick();
        self.tick_runtime_actions();
        self.recorder_panel.tick();
        self.recorder_panel
            .set_audio_sample_rate(self.audio.sample_rate());
        if let Ok(mut state) = self.shared.try_lock() {
            crate::radio_ui::reconcile_tuning(&mut state);
        }
        let desired_audio = self.shared.try_lock().ok().map(|state| {
            let channels = crate::radio_ui::playback_channels(&state);
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
            if !self.demod_worker.is_native() {
                self.demod_worker.request_reset();
            }
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
            if self.last_radio_profile.as_ref() != Some(&profile)
                && self.sync_demod_advanced(&state)
            {
                self.last_radio_profile = Some(profile);
            }
        }

        // Drain samples from source and capture parameters in a single lock
        let mut sample_batch: Vec<crate::source_manager::CaptureBlock> = Vec::new();
        let source_params = {
            if let Some(mut state) = self.try_lock_shared("drain_samples") {
                crate::radio_ui::sync_demod_settings(&mut state);
                crate::radio_ui::reconcile_tuning(&mut state);
                state.spectrum.vfo_freq_hz = Some(state.source.frequency_hz);
                state.spectrum.vfo_bw_hz = crate::radio_ui::channel_bandwidth(&state) as u32;
                state.spectrum.dsb_sideband = state.config.advanced.dsb_sideband;
                state.spectrum.demod_mode = state
                    .demod_mode
                    .resolve(state.source.frequency_hz)
                    .label()
                    .into();
                // This is only a bounded constellation preview. Native radio,
                // aircraft and recording consumers independently receive RF.
                for _ in 0..2 {
                    if let Some(samples) = state.source.recv_capture_block() {
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
                ))
            } else {
                None
            }
        };

        #[cfg(test)]
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
        )) = source_params
        {
            self.constellation
                .observe_capture(center, rate, &source_mode, stream_generation);
            if self.demod_worker.is_native() {
                let normalized = iq_config.normalized();
                let radio_rate =
                    f64::from(normalized.input_rate) / f64::from(normalized.decimation);
                self.demod_worker.configure_radio(
                    crate::demod::RadioConfig {
                        enabled: source_mode != crate::source_manager::SourceMode::Daemon,
                        preview: self.current_tab == AppTab::Listen || self.scanner.enabled,
                        measure_power: (self.recorder_panel.squelch_record
                            || self.recorder_panel.signal_monitor)
                            && squelch_mode == crate::radio_squelch::SquelchMode::Power,
                        frequency_hz: freq,
                        center_hz: center,
                        generation: stream_generation,
                        iq: normalized,
                        audio_running,
                        volume,
                        squelch_db,
                        squelch_mode,
                        ctcss_tone_hz,
                        rds_enabled: rds_settings.0,
                        rds_incremental: rds_settings.1,
                        rds_region: rds_settings.2,
                        sink: self.audio.sink(),
                    },
                    DemodConfig {
                        mode: demod_mode.resolve(freq),
                        input_rate: radio_rate,
                        audio_rate: self.audio.sample_rate(),
                        lpf_cutoff,
                        rf_bandwidth: channel_bandwidth,
                        stereo: stereo_audio,
                    },
                );
                for block in &sample_batch {
                    self.constellation.push_iq_samples(&block.samples);
                }
                let previews = self.demod_worker.radio_previews();
                let snapshot = self
                    .demod_worker
                    .radio_snapshot()
                    .expect("native radio snapshot");
                let current = snapshot.control_version == self.demod_worker.requested_version();
                if snapshot.lost_samples > self.last_radio_lost_samples {
                    self.status_bar.warning(format!(
                        "Radio capture lost {} samples ({} dropped blocks); DSP state reset.",
                        snapshot.lost_samples, snapshot.dropped_blocks
                    ));
                    self.last_radio_lost_samples = snapshot.lost_samples;
                }
                self.radio_ui.received_tone = current.then_some(snapshot.received_tone).flatten();
                self.radio_ui.rds = (current && receiving)
                    .then(|| snapshot.rds.clone())
                    .flatten();
                self.radio_ui.stereo_locked = current && snapshot.stereo_locked;
                let display_shared = Arc::clone(&self.shared);
                if let Ok(mut state) = display_shared.try_lock() {
                    let boundary = (
                        snapshot.generation,
                        snapshot.epoch,
                        snapshot.discontinuity_revision,
                    );
                    if self.last_radio_boundary != Some(boundary) {
                        state.spectrum.reset_capture_stream();
                        self.last_radio_boundary = Some(boundary);
                    }
                    state.spectrum.update_params_exact(center, radio_rate);
                    state.spectrum.vfo_freq_hz = Some(freq);
                    if self.current_tab == AppTab::Listen || self.scanner.enabled {
                        for preview in &previews {
                            if preview.center_hz == center
                                && preview.rate == radio_rate
                                && preview.control_version == self.demod_worker.requested_version()
                            {
                                state.spectrum.push_capture_samples(
                                    &preview.samples,
                                    preview.clock,
                                    preview.observed_at,
                                    preview.input_samples,
                                );
                            }
                        }
                        state.spectrum.try_recv_spectrum();
                    }
                    if current {
                        state.spectrum.audio_waveform = snapshot.waveform.iter().copied().collect();
                        state.fm_deviation_hz = snapshot.fm_deviation_hz;
                        state.audio_peak = snapshot.audio_peak;
                    }
                };
            }
        }

        // Drain an asynchronously completed FFT even on a frame without new IQ,
        // such as the final window arriving as the receiver pauses.
        if self.current_tab == AppTab::Listen || self.scanner.enabled {
            if let Ok(mut state) = self.shared.try_lock() {
                state.spectrum.try_recv_spectrum();
            }
        }

        // Passive ADS-B: detect newly-arrived aircraft and fire alert toasts + Discord notifications.
        self.adsb_panel.poll_uat();
        self.apply_local_adsb_snapshot();
        #[cfg(test)]
        if self.adsb_panel.region == crate::adsb_panel::AdsbRegion::ModeS1090
            && self.shared.try_lock().ok().is_some_and(|state| {
                state.source.source_mode != crate::source_manager::SourceMode::Daemon
                    && state.source.stream_generation() == 0
            })
        {
            self.adsb_panel
                .apply_receiver_snapshot(self.adsb_decoder.receiver_snapshot());
        }
        self.adsb_panel.tick_model();
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
        self.tick_meteor_workflows();
        if let Ok(mut state) = self.shared.try_lock() {
            state.config.secondary_vfo.frequency_hz = state.vfo_b;
            self.secondary_vfo.tick(
                &state.source,
                &state.config.secondary_vfo,
                &state.config.advanced.audio_output,
            );
        }
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
            }
            if let Some(notification) =
                crate::discord::positioned_aircraft_notification(ac, &mut self.notified_aircraft)
            {
                self.discord.fire_aircraft(notification);
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
            crate::keyboard::KeyboardOutcome::default()
        };

        if outcome.freq_changed {
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
                if was_running
                    && matches!(
                        state.source.status,
                        crate::source_manager::SourceStatus::Idle
                            | crate::source_manager::SourceStatus::Error(_)
                    )
                {
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
                        crate::radio_ui::playback_channels(&state),
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

        self.process_web_remote_commands();

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
            }
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            self.tick_satellite_jobs(now_unix, now);

            // Squelch-triggered recording tick
            {
                let input = if let Ok(mut state) = self.shared.try_lock() {
                    let native = self.demod_worker.radio_snapshot();
                    self.recorder_panel.prepare_squelch_trigger(
                        &mut state,
                        native.as_deref(),
                        self.demod_worker.requested_version(),
                        self.scanner.enabled,
                        now,
                    )
                } else {
                    crate::recorder_panel::AutomaticTrigger {
                        signal_db: None,
                        threshold_db: 0.0,
                        enabled: false,
                        frequency_hz: 0,
                        mode: String::new(),
                    }
                };
                self.recorder_panel.tick_squelch_record(
                    input.signal_db,
                    input.threshold_db,
                    input.enabled,
                    input.frequency_hz,
                    &input.mode,
                );
            }

            // Frequency scanner tick (runs every frame, rate-limited by dwell_ms)
            {
                let prev_hits = self.scanner.hits.len();
                let shared = Arc::clone(&self.shared);
                if let Ok(mut state) = shared.try_lock() {
                    if let Some(cmd) = state.scanner_command.take() {
                        self.scanner.apply_command(&cmd);
                    }
                    self.scanner.validate_fixed_capture(&state);
                    let expected_rate =
                        if state.source.source_mode == crate::source_manager::SourceMode::Daemon {
                            f64::from(state.source.sample_rate_hz)
                        } else {
                            let iq = crate::radio_ui::radio_iq_config(&state).normalized();
                            f64::from(iq.input_rate) / f64::from(iq.decimation)
                        };
                    let ready = state.spectrum.capture_geometry()
                        == (state.source.capture_center_frequency_hz(), expected_rate)
                        && state.source.status == crate::source_manager::SourceStatus::Running
                        && state.source.capture_tuning_applied();
                    self.scanner.daemon_mode =
                        state.source.source_mode == crate::source_manager::SourceMode::Daemon;
                    self.scanner.tick_spectrum(&mut state.spectrum, ready);
                    if let Some(revision) = self.scanner.daemon_observation_request {
                        if state.source.begin_daemon_spectrum_observation(revision) {
                            self.scanner.daemon_observation_request = None;
                        }
                    }
                }
                // Publish any new hits to MQTT + Discord
                if self.scanner.hits.len() > prev_hits {
                    for hit in &self.scanner.hits[prev_hits..] {
                        self.mqtt.publish_scanner_hit(hit.freq_hz, hit.strength_db);
                        let embed = crate::discord::embed_scanner_hit(hit.freq_hz, hit.strength_db);
                        self.discord.fire("scanner_hit", embed);
                    }
                }
                self.apply_scanner_tune();
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

        // Broadcast state + MQTT tick (rate-limited to every 5s)
        if let Ok(mut state) = self.shared.try_lock() {
            let mode = state.demod_mode.label();
            let freq = state.source.frequency_hz;
            let gain = state.source.gain_db;
            let ac_count = self.adsb_panel.aircraft.len();
            let passes = state.tle.upcoming_passes().to_vec();
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
                    &format!(
                        "Connected to {}",
                        self.mqtt
                            .connected_endpoint()
                            .as_deref()
                            .unwrap_or("unknown endpoint")
                    ),
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
                .broadcast_state(&crate::web_remote::StreamState::from_receiver(
                    &state,
                    ac_count,
                    &passes,
                    scanner_active,
                    snr,
                ));
            self.mqtt.publish_telemetry_if_due(
                std::time::Instant::now(),
                TelemetryView {
                    frequency_hz: freq,
                    gain_db: gain,
                    signal_db: peak,
                    noise_db: noise,
                    mode,
                    recording,
                    passes: &passes,
                    aircraft: &self.adsb_panel.aircraft,
                },
            );

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

        self.tick_bookmark_autosave();

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

        // Secondary tools are utility windows. They stay above the receiver
        // workspace so opening a tool never resizes the spectrum or controls.
        if let Some(tool) = self.active_secondary_tool {
            let mut open = true;
            egui::Window::new(format!("{} {}", tool.icon(), tool.label()))
                .id(egui::Id::new(("secondary_tool_window", tool.id())))
                .open(&mut open)
                .default_size([430.0, 560.0])
                .min_size([320.0, 240.0])
                .max_size([680.0, 820.0])
                .resizable(true)
                .collapsible(false)
                .show(ui.ctx(), |ui| self.render_secondary_panel(ui, tool));
            if !open {
                self.active_secondary_tool = None;
            }
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

                    self.status_bar
                        .info(format!("⤵ {:.3} MHz", freq as f64 / 1e6));
                }
            }
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
                        h(ui, "VFO"); ui.label("Variable Frequency Oscillator — your tuned frequency. VFO A is the main frequency; VFO B can receive simultaneously inside the same capture. Enable it in Independent B receiver, or swap frequencies with 'V'."); ui.end_row();
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
        self.flush_bookmarks();
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
            cfg.secondary_vfo = state.config.secondary_vfo.clone();
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
    /// Top mode bar: the entire top-level navigation. It exposes the task
    /// modes, curated utility menu, and compact AI popup toggle.
    fn render_mode_bar(&mut self, ui: &mut egui::Ui) {
        crate::mode_bar::render_mode_bar(
            ui,
            &self.shared,
            &mut self.current_tab,
            &mut self.active_secondary_tool,
            &mut self.adsb_panel,
        );
    }

    /// Bottom status strip: plain-language summary of what's happening —
    /// frequency, demod mode, signal strength word, and the REC indicator,
    /// ending with the latest transient status flash.
    fn render_status_strip(&mut self, ui: &mut egui::Ui) {
        match self.current_tab {
            AppTab::Meteor => {
                ui.horizontal(|ui| {
                    ui.label(if self.meteor_decoder.running {
                        "Decoding recording"
                    } else {
                        "Meteor · live and recording decoder"
                    });
                    if let Some(message) = self.status_bar.current() {
                        ui.separator();
                        ui.label(&message.text);
                    }
                });
            }
            AppTab::Planes => {
                ui.horizontal(|ui| {
                    ui.label(self.adsb_panel.receiver_status());
                    ui.separator();
                    ui.label(format!("{} aircraft", self.adsb_panel.aircraft.len()));
                });
            }
            _ => self.status_bar.render_strip(ui, &self.shared),
        }
    }

    fn render_secondary_panel(&mut self, ui: &mut egui::Ui, tool: SecondaryTool) {
        self.recorder_panel
            .set_audio_sample_rate(self.audio.sample_rate());
        let mut ctx = crate::secondary_panel::SecondaryPanelContext {
            shared: &self.shared,
            active_secondary_tool: &mut self.active_secondary_tool,
            scanner: &mut self.scanner,
            howto_panel: &mut self.howto_panel,
            quick_start: &mut self.quick_start,
            status_bar: &mut self.status_bar,
            ai_panel: &mut self.ai_panel,
            recorder_panel: &mut self.recorder_panel,
            mqtt: &self.mqtt,
        };
        crate::secondary_panel::render_secondary_panel(&mut ctx, ui, tool);
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
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.radio_ui.sidebar(ui);
                        ui.collapsing("Independent B receiver", |ui| {
                            if let Ok(mut state) = self.shared.try_lock() {
                                let mut settings = state.config.secondary_vfo.clone();
                                settings.frequency_hz = state.vfo_b;
                                self.secondary_vfo.ui(
                                    ui,
                                    &mut settings,
                                    &state.source,
                                    &state.config.advanced.audio_output,
                                );
                                state.vfo_b = settings.frequency_hz;
                                state.config.secondary_vfo = settings;
                            }
                        });
                    });
                });
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
                state.spectrum.dsb_sideband = state.config.advanced.dsb_sideband;
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

                }
                if let Some(freq) = state.spectrum.pending_vfo_b_freq.take() {
                    state.vfo_b = freq;
                    state.config.secondary_vfo.frequency_hz = freq;
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
                if let Some(sq) = state.spectrum.pending_squelch_db.take() { if let Err(error) = crate::radio_ui::set_power_squelch_level(&mut state, sq) { self.status_bar.warning(format!("Squelch: {error}")); } }
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
                    self.active_secondary_tool = Some(SecondaryTool::Ai);
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
                    self.active_secondary_tool = Some(SecondaryTool::Ai);
                    self.status_bar
                        .info("🤖 Aircraft details sent to AI".to_string());
                }
            });
        // Setup guidance is available from the How To popup; keep the map and
        // aircraft list unobstructed in the tracking workspace.
        egui::Panel::top("adsb_band_picker").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Region / band");
                if self.adsb_panel.ui_region_picker(ui) {
                    self.local_adsb.reset();
                    #[cfg(test)]
                    {
                        self.adsb_decoder = AdsBDecoder::new();
                    }
                    self.seen_aircraft.clear();
                    self.notified_aircraft.clear();
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
                egui::RichText::new("Decode a live Meteor pass or import an IQ recording.")
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

#[cfg(test)]
#[path = "app_performance_tests.rs"]
mod app_performance_tests;

#[cfg(test)]
#[path = "../tests/support/software_renderer.rs"]
mod software_renderer;
