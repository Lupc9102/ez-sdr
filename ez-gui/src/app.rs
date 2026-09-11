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
use crate::advanced_panel::push_advanced;
use crate::ai_panel::AiPanel;
use crate::audio_output::AudioOutput;
use crate::bookmarks::BookmarkDb;
use crate::config::AppConfig;
use crate::constellation::ConstellationDisplay;
use crate::decoding_panel::DecodingPanel;
use crate::demod::Demodulator;
use crate::discord::DiscordNotifier;
use crate::discord_panel::DiscordPanel;
use crate::howto_panel::HowToPanel;
use crate::mqtt::MqttPublisher;
use crate::recorder_panel::RecorderPanel;
use crate::satellite_panel::SatellitePanel;
use crate::scheduler::Scheduler;
use crate::sdr_panel::SdrPanel;
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
    sdr_panel: SdrPanel,
    satellite_panel: SatellitePanel,
    adsb_panel: AdsBPanel,
    recorder_panel: RecorderPanel,
    constellation: ConstellationDisplay,
    decoding_panel: DecodingPanel,
    ai_panel: AiPanel,
    howto_panel: HowToPanel,
    web_remote: WebRemote,
    mqtt: MqttPublisher,
    demod: Demodulator,
    audio: AudioOutput,
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    audio_tx: crossbeam_channel::Sender<Vec<f32>>,
    adsb_decoder: AdsBDecoder,
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
    // Listen mode smart-tune box
    listen_tune_input: String,
    // Session notes
    session_notes: String,
    // SDR glossary popup
    show_glossary: bool,
    // First strong signal celebration
    first_strong_signal_seen: bool,
    // Track demod mode changes to reset demodulator and avoid clicks
    last_demod_mode: crate::sdr_panel::DemodMode,
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
                        state.source.frequency_hz = hz;
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

    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (audio_tx, audio_rx) = crossbeam_channel::bounded(64);

        let config = AppConfig::load_or_default();
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
            squelch: -50.0,
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
        let mut mqtt = MqttPublisher::new();
        let mut discord = DiscordNotifier::new();
        {
            let state = shared.lock().expect("shared state mutex poisoned");
            if state.config.web_remote_enabled {
                web_remote.set_enabled(true, state.config.web_remote_port);
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

        // Start demo source immediately
        {
            let mut state = shared.lock().expect("shared state mutex poisoned");
            // Restore last session freq/gain/demod if available, else fall back to config defaults
            state.source.frequency_hz = if state.config.last_session_freq_hz > 0 {
                state.config.last_session_freq_hz
            } else {
                state.config.default_freq_hz
            };
            state.source.sample_rate_hz = state.config.default_sample_rate;
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
            state.source.start();
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
            sdr_panel: SdrPanel::new(shared.clone()),
            satellite_panel: SatellitePanel::new(shared.clone()),
            adsb_panel: AdsBPanel::new(shared.clone()),
            recorder_panel: RecorderPanel::new(shared.clone()),
            constellation: ConstellationDisplay::new(),
            decoding_panel: DecodingPanel::default(),
            ai_panel: AiPanel::new(shared.clone()),
            howto_panel: HowToPanel::new(),
            web_remote,
            mqtt,
            discord,
            discord_panel: DiscordPanel::new(),
            demod: Demodulator::new(),
            audio: AudioOutput::new(),
            audio_rx,
            audio_tx,
            adsb_decoder: AdsBDecoder::new(),
            scanner: crate::scanner::FrequencyScanner::new(shared.clone()),
            last_scheduler_update: std::time::Instant::now(),
            last_source_status: crate::source_manager::SourceStatus::Idle,
            last_auto_tuned_satellite: String::new(),
            bookmark_panel: crate::bookmark_manager::BookmarkPanel::new(),
            quick_start: {
                let mut qs = crate::quick_start::QuickStartWizard::new();
                if let Ok(s) = shared.try_lock() {
                    if !s.config.quick_start_completed {
                        qs.start();
                    }
                }
                qs
            },
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
            listen_tune_input: String::new(),
            session_notes: String::new(),
            show_glossary: false,
            first_strong_signal_seen: false,
            last_demod_mode: crate::sdr_panel::DemodMode::Fm,
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
    /// Push the saved advanced settings onto the live engines.
    pub fn apply_advanced(&mut self) {
        let mut shared = self.shared.lock().expect("shared state mutex poisoned");
        push_advanced(&mut self.demod, &mut shared);
    }

    /// Drain queued daemon events (only produces events in `SourceMode::Daemon`)
    /// and apply them: spectrum frames feed the analyser, errors surface as a
    /// status flash. Hardware/audio/aircraft/telemetry/recording/pong events are
    /// already applied inside `SourceManager::recv_daemon_event` (e.g. the
    /// hardware shadow-field sync) and need no further handling here.
    fn drain_daemon_events(&mut self) {
        let Ok(mut state) = self.shared.try_lock() else {
            return;
        };
        state.source.sync_daemon_controls();
        for _ in 0..8 {
            match state.source.recv_daemon_event() {
                Some(ez_proto::ServerEvent::Spectrum(frame)) => {
                    state.spectrum.push_spectrum_frame(&frame);
                }
                Some(ez_proto::ServerEvent::Error { message }) => {
                    self.status_bar.warning(format!("⚠ {message}"));
                }
                Some(_) => {}
                None => break,
            }
        }
    }
}

impl eframe::App for CentralApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // In SourceMode::Daemon the local worker threads are idle; spectrum, hardware and
        // status all arrive over the network instead. Drain and apply those first (a no-op in
        // every other mode) so the frame below renders on the freshest daemon state.
        self.drain_daemon_events();

        // Process pending events from panels and event bus
        self.handle_events();
        self.status_bar.update();

        // Drain samples from source and capture parameters in a single lock
        let mut sample_batch: Vec<Vec<u8>> = Vec::new();
        let source_params = {
            if let Some(mut state) = self.try_lock_shared("drain_samples") {
                for _ in 0..4 {
                    if let Some(samples) = state.source.recv_samples() {
                        if samples == b"ERROR" {
                            state.source.status = crate::source_manager::SourceStatus::Error(
                                "Device open failed".to_string(),
                            );
                            break;
                        }
                        sample_batch.push(samples);
                    } else {
                        break;
                    }
                }
                Some((
                    state.source.frequency_hz,
                    state.source.sample_rate_hz,
                    state.demod_mode,
                    state.audio_running,
                    state.volume,
                    state.squelch,
                    state.lpf_cutoff,
                    state.adsb_running,
                    state.spectrum.signal_level(),
                ))
            } else {
                None
            }
        };

        // Process samples outside lock
        if let Some((
            freq,
            rate,
            demod_mode,
            audio_running,
            volume,
            squelch_db,
            lpf_cutoff,
            adsb_running,
            signal_level,
        )) = source_params
        {
            let mut all_audio_samples: Vec<f32> = Vec::new();

            for samples in &sample_batch {
                self.recorder_panel.write_samples(samples);
                self.satellite_panel.feed_recording(samples);
                self.constellation.push_iq_samples(samples);

                // Demodulate and send audio
                if audio_running {
                    let effective_mode = demod_mode.resolve(freq);
                    if effective_mode != self.last_demod_mode {
                        self.demod.reset();
                        self.last_demod_mode = effective_mode;
                    }
                    self.demod.set_sample_rates(rate, self.audio.sample_rate());
                    self.demod.set_lpf_cutoff(lpf_cutoff);
                    let audio = self.demod.demodulate(samples, effective_mode);
                    let gate: f32 = if squelch_db < -80.0 || signal_level > squelch_db {
                        1.0
                    } else {
                        0.0
                    };
                    let audio: Vec<f32> = audio.into_iter().map(|s| s * volume * gate).collect();
                    self.recorder_panel.write_audio_samples(&audio);
                    all_audio_samples.extend_from_slice(&audio);
                    let _ = self.audio_tx.try_send(audio);
                }

                // Feed to ADS-B decoder when tuned to 1090 MHz
                if adsb_running
                    && (freq == 1_090_000_000 || (freq > 1_088_000_000 && freq < 1_092_000_000))
                {
                    self.adsb_decoder.feed_iq(samples, rate);
                    let ac = self.adsb_decoder.get_aircraft();
                    self.adsb_panel.aircraft = ac;
                    self.adsb_panel.total_messages = self.adsb_decoder.total_messages;
                    self.adsb_panel.decode_stats = self.adsb_decoder.stats();
                }
            }

            // Update spectrum, waveform, and demod metrics in a single consolidated lock
            if !sample_batch.is_empty() || !all_audio_samples.is_empty() {
                if let Some(mut state) = self.try_lock_shared("update_spectrum_and_audio") {
                    state.spectrum.update_params(freq, rate);
                    for samples in &sample_batch {
                        state.spectrum.push_iq_samples(samples);
                    }
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
                        state.fm_deviation_hz = self.demod.last_fm_deviation_hz;
                        state.audio_peak = self.demod.last_audio_peak;
                    }
                }
            }
        }

        // Passive ADS-B: detect newly-arrived aircraft and fire alert toasts + Discord notifications.
        self.adsb_panel.check_for_new_aircraft();
        if let Some(msg) = self.adsb_panel.pending_status_flash.take() {
            self.status_bar.info(msg);
        }
        if let Some(msg) = self.sdr_panel.pending_status.take() {
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
        // Fire Discord notifications and publish event for new aircraft
        for ac in &self.adsb_panel.aircraft {
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
            if let Ok(state) = self.shared.try_lock() {
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
        if outcome.toggle_recording {
            if self.recorder_panel.recording {
                self.recorder_panel.stop_recording();
                self.event_bus
                    .publish(crate::events::AppEvent::RecordingStopped { duration_secs: 0 });
            } else {
                self.recorder_panel.start_recording();
                self.event_bus
                    .publish(crate::events::AppEvent::RecordingStarted {
                        filename: "recording.wav".to_string(),
                    });
            }
        }

        // Repaint rate: 30fps when active, 1fps when idle (saves CPU on battery)
        ctx.request_repaint_after(Duration::from_millis(if sample_batch.is_empty() {
            1000
        } else {
            33
        }));

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

                let audio_running = state.audio_running;
                (
                    Some(audio_running),
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
        if let Some(audio_running) = audio_action {
            if audio_running && !self.audio.is_running() {
                if !self.audio.has_failed() {
                    let rx = self.audio_rx.clone();
                    if self.audio.start(rx).is_ok() {
                        self.demod.reset();
                    } else {
                        self.audio.mark_failed();
                    }
                }
            } else if !audio_running && self.audio.is_running() {
                self.audio.stop();
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
                            state.source.frequency_hz = freq_hz;
                        }
                        RemoteCommand::SetGain { gain_db } => {
                            state.source.gain_db = gain_db;
                        }
                        RemoteCommand::SetDemod { mode } => {
                            use crate::sdr_panel::DemodMode;
                            if let Some(dm) = DemodMode::from_label(&mode) {
                                state.demod_mode = dm;
                            }
                        }
                        RemoteCommand::SetSquelch { db } => {
                            state.squelch = db;
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
                    RemoteCommand::StartRecord => self.recorder_panel.start_recording(),
                    RemoteCommand::StopRecord => self.recorder_panel.stop_recording(),
                    RemoteCommand::StartScan => self.scanner.start(),
                    RemoteCommand::StopScan => self.scanner.stop(),
                    _ => {}
                }
            }
        }

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
                    .active_job(now_unix)
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
                        state.source.frequency_hz = freq;
                        self.last_auto_tuned_satellite = sat.clone();
                        self.event_bus
                            .publish(crate::events::AppEvent::FrequencyChanged { hz: freq });
                        self.status_bar.info(format!(
                            "🛰 Auto-tuned to {} ({:.3} MHz)",
                            sat,
                            freq as f64 / 1e6
                        ));
                    }
                    // Apply doppler correction (round to avoid jitter)
                    let doppler = state.tle.doppler_shift_for_sat(&sat, freq as f64, now_unix);
                    self.satellite_panel.doppler_hz = doppler;
                    if self.satellite_panel.auto_tune && doppler.abs() > 1000.0 {
                        let corrected_hz = ((freq as f64 + doppler) / 1000.0).round() * 1000.0;
                        let corrected = corrected_hz.max(0.0) as u64;
                        if corrected != state.source.frequency_hz {
                            state.source.frequency_hz = corrected;
                        }
                    }
                } else if !self.last_active_pass_sat.is_empty() {
                    // Satellite LOS detection (pass ended)
                    let embed = crate::discord::embed_sat_los(&self.last_active_pass_sat);
                    self.discord.fire("sat_los", embed);
                    self.last_active_pass_sat.clear();
                }
                // Poll custom scheduled tasks
                if let Some((label, freq)) = state.scheduler.poll_custom_tasks(now_unix) {
                    state.source.frequency_hz = freq;
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
                            state.spectrum.signal_level(),
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
                        state.source.frequency_hz = freq;
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

        // Process quick-bookmark request from SDR panel
        if let Some((freq, mode)) = self.sdr_panel.bookmark_request.take() {
            if let Ok(mut state) = self.shared.try_lock() {
                let freq_mhz = freq as f64 / 1e6;
                let name = format!("{freq_mhz:.3} MHz {mode}");
                state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                    name,
                    frequency_hz: freq,
                    mode,
                    bandwidth_hz: 12_500,
                    category: "Quick".to_string(),
                    notes: String::new(),
                    starred: false,
                });
                state.bookmarks_modified = true;
                state.spectrum.bookmark_freqs_dirty = true;
            }
        }

        if let Some(freq) = self.sdr_panel.pending_ai_freq.take() {
            let freq_mhz = freq as f64 / 1e6;
            let (snr, mode) = if let Ok(state) = self.shared.try_lock() {
                let snr = state.spectrum.peak_level() - state.spectrum.noise_floor();
                let mode = state.demod_mode.label().to_string();
                (snr, mode)
            } else {
                (0.0, "unknown".to_string())
            };
            self.ai_panel.input = format!(
                "I'm currently tuned to {freq_mhz:.4} MHz in {mode} mode (SNR: {snr:.1} dB). \
                 What signals should I expect here? What demod mode and settings would you recommend?"
            );
            self.status_bar.info(format!(
                "🤖 AI prompt ready for {freq_mhz:.3} MHz — switch to AI Agent tab"
            ));
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
                state.config.needs_apply = false;
                // Apply theme
                state.config.theme_config.apply_to_ctx(ctx);
                // Apply font scale
                let scale = state.config.font_scale as f32;
                ctx.set_pixels_per_point(scale);
                state.source.frequency_hz = state.config.default_freq_hz;
                state.source.sample_rate_hz = state.config.default_sample_rate;
                state.source.gain_db = state.config.default_gain;
                state.tle.observer_lat = state.config.observer_lat;
                state.tle.observer_lon = state.config.observer_lon;
                self.web_remote.set_enabled(
                    state.config.web_remote_enabled,
                    state.config.web_remote_port,
                );
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
                    self.mqtt.publish_aircraft(&self.adsb_panel.aircraft);
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
            .exact_size(44.0)
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
        egui::Panel::bottom("status_strip")
            .exact_size(26.0)
            .show(ui, |ui| self.render_status_strip(ui));

        match self.current_tab {
            AppTab::Listen => self.render_sdr_tab(ui, &snapshot),
            AppTab::Planes => self.render_adsb_tab(ui),
            AppTab::Satellites => self.render_satellite_tab(ui, &snapshot),
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
                    state.source.frequency_hz = freq;
                    self.last_manual_tune_time = std::time::Instant::now();
                    self.status_bar
                        .info(format!("⤵ {:.3} MHz", freq as f64 / 1e6));
                }
            }
        }

        // Minimal status bar
        ui.separator();
        ui.horizontal(|ui| {
            if let Ok(state) = self.shared.try_lock() {
                let running = state.source.status == crate::source_manager::SourceStatus::Running;
                let status_color = if running {
                    egui::Color32::GREEN
                } else {
                    egui::Color32::GRAY
                };
                ui.colored_label(status_color, "●")
                    .on_hover_text(if running {
                        "SDR running"
                    } else {
                        "SDR stopped — Space to start"
                    });

                let true_hz = (state.source.frequency_hz as i64 + state.lo_offset_hz).max(0) as u64;
                let freq_str = if state.lo_offset_hz != 0 {
                    format!(
                        "{:.3} MHz (+{:.0}M)",
                        true_hz as f64 / 1e6,
                        state.lo_offset_hz as f64 / 1e6
                    )
                } else {
                    format!("{:.3} MHz", state.source.frequency_hz as f64 / 1e6)
                };
                let freq_resp = ui.add(
                    egui::Label::new(egui::RichText::new(&freq_str).monospace().size(13.0).color(
                        if state.lo_offset_hz != 0 || state.source.ppm_correction != 0 {
                            egui::Color32::from_rgb(255, 200, 80)
                        } else {
                            egui::Color32::WHITE
                        },
                    ))
                    .sense(egui::Sense::click()),
                );
                if freq_resp.clicked() {
                    ui.ctx().copy_text(format!("{:.6}", true_hz as f64 / 1e6));
                }

                ui.separator();
                ui.small(state.demod_mode.label().to_string());

                ui.separator();
                let signal_db = state.spectrum.signal_level();
                let sig_color = if signal_db > -40.0 {
                    egui::Color32::GREEN
                } else if signal_db > -80.0 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::DARK_GRAY
                };
                ui.colored_label(sig_color, format!("{signal_db:.0} dB"));

                if state.recording {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, "● REC");
                }

                // Status flash
                if let Some(msg) = self.status_bar.current() {
                    let elapsed = msg.created_at.elapsed().as_secs_f32();
                    let total = msg.duration.as_secs_f32();
                    if elapsed < total {
                        let alpha = ((1.0 - (elapsed / total)).max(0.0) * 255.0) as u8;
                        ui.separator();
                        ui.colored_label(
                            egui::Color32::from_rgba_unmultiplied(220, 200, 80, alpha),
                            &msg.text,
                        );
                    }
                }
            }
        });

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
            cfg.last_session_gain_db = state.source.gain_db;
            cfg.last_session_demod = state.demod_mode.label().to_string();
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

// ── New 3-tab render methods ──────────────────────────────────────────────────

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
        self.status_bar.render_strip(ui, &self.shared);
    }

    fn render_secondary_panel(
        &mut self,
        ui: &mut egui::Ui,
        tool: SecondaryTool,
        snapshot: &Option<SharedSnapshot>,
    ) {
        let mut ctx = crate::secondary_panel::SecondaryPanelContext {
            shared: &self.shared,
            active_secondary_tool: &mut self.active_secondary_tool,
            bookmark_panel: &mut self.bookmark_panel,
            scheduler_panel: &mut self.scheduler_panel,
            demod: &mut self.demod,
            scanner: &mut self.scanner,
            ai_panel: &mut self.ai_panel,
            ai_ask_open: &mut self.ai_ask_open,
            status_bar: &mut self.status_bar,
            recorder_panel: &mut self.recorder_panel,
            howto_panel: &mut self.howto_panel,
            discord_panel: &mut self.discord_panel,
            discord: &mut self.discord,
            mqtt: &mut self.mqtt,
            web_remote: &mut self.web_remote,
            customize_panel: &mut self.customize_panel,
            last_manual_tune_time: &mut self.last_manual_tune_time,
            session_notes: &mut self.session_notes,
        };
        crate::secondary_panel::render_secondary_panel(&mut ctx, ui, tool, snapshot);
    }

    /// Listen-mode header: one-tap preset tiles, a plain-language smart-tune
    /// box, the ambient Auto demod chip, and a humanized signal meter — the
    /// "tune → listen" path that needs zero radio knowledge.
    fn render_listen_header(&mut self, ui: &mut egui::Ui) {
        crate::listen_header::render_listen_header(
            ui,
            &self.shared,
            &mut self.listen_tune_input,
            &mut self.last_manual_tune_time,
            &mut self.active_secondary_tool,
        );
    }

    fn render_sdr_tab(&mut self, ui: &mut egui::Ui, snapshot: &Option<SharedSnapshot>) {
        let _ = snapshot;
        let theme = self
            .shared
            .try_lock()
            .map(|s| s.config.theme_config.clone())
            .unwrap_or_default();

        egui::Panel::left("sdr_modules")
            .resizable(true)
            .default_size(320.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.sdr_panel.ui_source(ui);
                    ui.collapsing("IQ Constellation", |ui| {
                        self.constellation.ui(ui, &theme);
                    });
                    if let Some(freq) = self.sdr_panel.tune_request.take() {
                        if let Ok(mut state) = self.shared.try_lock() {
                            state.source.frequency_hz = freq;
                            self.last_manual_tune_time = std::time::Instant::now();
                        }
                    }
                    if let Some(msg) = self.sdr_panel.pending_status.take() {
                        self.status_bar.info(msg);
                    }
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            self.render_listen_header(ui);
            if let Ok(mut state) = self.shared.try_lock() {
                if state.spectrum.bookmark_freqs_dirty {
                    state.spectrum.bookmark_freqs = state.bookmarks.bookmarks.iter()
                        .map(|b| (b.frequency_hz, b.name.clone(), b.category.clone())).collect();
                    state.spectrum.bookmark_freqs_dirty = false;
                }
                state.spectrum.vfo_bw_hz = state.lpf_cutoff as u32 * 2;
                state.spectrum.vfo_b_freq = state.vfo_b;
                state.spectrum.demod_mode = state.demod_mode.label().to_string();
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
                    let premul = |c: crate::theme::Rgba| {
                        egui::Color32::from_rgba_premultiplied(c.0, c.1, c.2, c.3)
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
                    state.spectrum.bandplan_ham = premul(ham);
                    state.spectrum.bandplan_broadcast = premul(bcast);
                    state.spectrum.bandplan_aviation = premul(air);
                    state.spectrum.bandplan_marine = premul(mar);
                    state.spectrum.bandplan_weather = premul(wx);
                    state.spectrum.bandplan_satellite = premul(sat);
                    state.spectrum.bandplan_mobile = premul(mob);
                    state.spectrum.bandplan_ism = premul(ism);
                }
                let sq_active = state.squelch > -90.0 && state.spectrum.signal_level() > state.squelch;
                state.spectrum.signal_active = sq_active;
                if sq_active {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                    state.spectrum.last_signal_unix = Some(now);
                }
                state.spectrum.ui(ui);
                if let Some(freq) = state.spectrum.clicked_tune_freq.take() { state.source.frequency_hz = freq; }
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
                if let Some(sq) = state.spectrum.pending_squelch_db.take() { state.squelch = sq; }
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
                if state.spectrum.pending_start_source { state.spectrum.pending_start_source = false; state.source.start(); }
                if let Some(freq) = state.spectrum.pending_ai_freq.take() {
                    let freq_mhz = freq as f64 / 1e6;
                    self.ai_panel.input = format!("I'm looking at {freq_mhz:.4} MHz on the spectrum. What signals might be here? What demod mode?");
                    self.status_bar.info(format!("🤖 AI prompt for {freq_mhz:.3} MHz"));
                }
            }
        });

        if let Some(freq) = self.sdr_panel.pending_ai_freq.take() {
            if let Ok(state) = self.shared.try_lock() {
                let snr = state.spectrum.peak_level() - state.spectrum.noise_floor();
                self.ai_panel.input =
                    format!(
                    "Tuned to {:.4} MHz in {} mode (SNR: {:.1} dB). What signals? Best settings?",
                    freq as f64 / 1e6, state.demod_mode.label(), snr
                );
            }
        }
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
        egui::CentralPanel::default().show(ui, |ui| {
            self.adsb_panel.ui_map(ui);
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
