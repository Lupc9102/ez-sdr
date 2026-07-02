use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct FreqMemEntry {
    pub freq_hz: u64,
    pub label: String,
}

impl Default for FreqMemEntry {
    fn default() -> Self { Self { freq_hz: 0, label: String::new() } }
}

use crate::adsb_decoder::AdsBDecoder;
use crate::adsb_panel::AdsBPanel;
use crate::ai_panel::AiPanel;
use crate::audio_output::AudioOutput;
use crate::bookmarks::BookmarkDb;
use crate::config::AppConfig;
use crate::demod::Demodulator;
use crate::mqtt::MqttPublisher;
use crate::discord::DiscordNotifier;
use crate::discord_panel::DiscordPanel;
use crate::recorder_panel::RecorderPanel;
use crate::satellite_panel::SatellitePanel;
use crate::scheduler::Scheduler;
use crate::sdr_panel::SdrPanel;
use crate::source_manager::SourceManager;
use crate::spectrum::SpectrumAnalyzer;
use crate::tle_engine::TleEngine;
use crate::howto_panel::HowToPanel;
use crate::theme::ThemeColors;
use crate::tutorial;
use crate::user_level::{TutorialState, UserLevel};
use crate::web_remote::{RemoteCommand, WebRemote};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Tab {
    Sdr,
    Spectrum,
    Satellite,
    AdsB,
    Recorder,
    Scanner,
    AiAgent,
    Bookmarks,
    Scheduler,
    Settings,
    HowTo,
    Discord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppTab {
    Sdr,
    AdsB,
    Satellite,
    Ai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryTool {
    Bookmarks,
    Scanner,
    Recorder,
    Scheduler,
    Settings,
    HowTo,
    Discord,
}

impl SecondaryTool {
    fn icon(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "⭐",
            SecondaryTool::Scanner => "🔍",
            SecondaryTool::Recorder => "⏺",
            SecondaryTool::Scheduler => "🗓",
            SecondaryTool::Settings => "⚙",
            SecondaryTool::HowTo => "❓",
            SecondaryTool::Discord => "💬",
        }
    }
    fn label(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "Bookmarks",
            SecondaryTool::Scanner => "Scanner",
            SecondaryTool::Recorder => "Recorder",
            SecondaryTool::Scheduler => "Scheduler",
            SecondaryTool::Settings => "Settings",
            SecondaryTool::HowTo => "How To",
            SecondaryTool::Discord => "Discord",
        }
    }
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
    pub theme_colors: ThemeColors,
    pub bookmarks_modified: bool,
}

pub struct CentralApp {
    shared: Arc<Mutex<SharedState>>,
    sdr_panel: SdrPanel,
    satellite_panel: SatellitePanel,
    adsb_panel: AdsBPanel,
    recorder_panel: RecorderPanel,
    ai_panel: AiPanel,
    howto_panel: HowToPanel,
    web_remote: WebRemote,
    mqtt: MqttPublisher,
    demod: Demodulator,
    audio: AudioOutput,
    audio_rx: Arc<Mutex<crossbeam_channel::Receiver<Vec<f32>>>>,
    audio_tx: crossbeam_channel::Sender<Vec<f32>>,
    adsb_decoder: AdsBDecoder,
    scanner: crate::scanner::FrequencyScanner,
    last_scheduler_update: std::time::Instant,
    last_source_status: crate::source_manager::SourceStatus,
    last_auto_tuned_satellite: String,
    bookmark_filter: String,
    show_keyboard_help: bool,
    last_history_freq: u64,
    freq_history_idx: Option<usize>,
    status_flash: Option<(String, std::time::Instant)>,
    recording_start: Option<std::time::Instant>,
    tutorial: TutorialState,
    bm_last_len: usize,
    bm_dirty_since: Option<std::time::Instant>,
    // New-bookmark form state
    new_bm_name: String,
    new_bm_freq_mhz: String,
    new_bm_mode: String,
    new_bm_category: String,
    new_bm_notes: String,
    new_bm_error: String,
    show_add_bm: bool,
    bm_import_msg: String,
    // Bookmark inline edit
    edit_bm_idx: Option<usize>,
    edit_bm_name: String,
    edit_bm_freq_mhz: String,
    edit_bm_mode: String,
    edit_bm_category: String,
    edit_bm_notes: String,
    // Custom task form
    new_task_label: String,
    new_task_freq_mhz: String,
    new_task_time: String,
    new_task_error: String,
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
    theme_applied: bool,
    show_starred_only: bool,
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
    sdr_ai_panel_open: bool,
    adsb_instructions_open: bool,
    satellite_advanced: bool,
}

impl CentralApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (audio_tx, audio_rx) = crossbeam_channel::bounded(64);
        let audio_rx = Arc::new(Mutex::new(audio_rx));

        let config = AppConfig::load_or_default();
        let theme_colors = ThemeColors::from(&config.theme_config);
        let shared = Arc::new(Mutex::new(SharedState {
            source: SourceManager::new(),
            spectrum: SpectrumAnalyzer::new(),
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
            theme_colors,
            bookmarks_modified: true,
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
                if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&state.config.last_session_demod) {
                    state.demod_mode = mode;
                }
            }
            state.source.ppm_correction = state.config.ppm_correction;
            state.lo_offset_hz = state.config.lo_offset_hz;
            state.vfo_b = if state.config.vfo_b_hz > 0 { state.config.vfo_b_hz } else { state.config.default_freq_hz };
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

        Self {
            shared: shared.clone(),
            sdr_panel: SdrPanel::new(shared.clone()),
            satellite_panel: SatellitePanel::new(shared.clone()),
            adsb_panel: AdsBPanel::new(shared.clone()),
            recorder_panel: RecorderPanel::new(shared.clone()),
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
            bookmark_filter: String::new(),
            show_keyboard_help: false,
            last_history_freq: {
                let state = shared.lock().expect("shared state mutex poisoned");
                state.source.frequency_hz
            },
            freq_history_idx: None,
            status_flash: None,
            new_bm_name: String::new(),
            new_bm_freq_mhz: String::new(),
            new_bm_mode: "NFM".to_string(),
            new_bm_category: "Custom".to_string(),
            new_bm_notes: String::new(),
            new_bm_error: String::new(),
            show_add_bm: false,
            bm_import_msg: String::new(),
            edit_bm_idx: None,
            edit_bm_name: String::new(),
            edit_bm_freq_mhz: String::new(),
            edit_bm_mode: String::new(),
            edit_bm_category: String::new(),
            edit_bm_notes: String::new(),
            new_task_label: String::new(),
            new_task_freq_mhz: String::new(),
            new_task_time: String::new(),
            new_task_error: String::new(),
            show_freq_jump: false,
            freq_jump_input: String::new(),
            freq_jump_matches: Vec::new(),
            session_notes: String::new(),
            show_glossary: false,
            first_strong_signal_seen: false,
            last_demod_mode: crate::sdr_panel::DemodMode::Fm,
            theme_applied: false,
            recording_start: None,
            bm_last_len: 0,
            bm_dirty_since: None,
            // Tutorial: first boot or resume incomplete tutorial
            tutorial: {
                let state = shared.lock().expect("shared state mutex poisoned");
                let mut t = TutorialState::new();
                if !state.config.tutorial_seen {
                    // First boot — show level selector
                    t.active = true;
                    t.level = UserLevel::from_str(&state.config.user_level);
                    t.level_chosen = false;
                    // Check if we have a saved step to resume
                    if state.config.tutorial_step > 0 {
                        t.step = state.config.tutorial_step;
                        t.asked_resume = true;
                    }
                } else {
                    t.active = false;
                }
                t
            },
            show_starred_only: false,
            last_recording: false,
            last_adsb_running: false,
            last_scanner_enabled: false,
            last_mqtt_connected: false,
            seen_aircraft: std::collections::HashSet::new(),
            last_active_pass_sat: String::new(),
            current_tab: AppTab::Sdr,
            discord_summary_last: std::time::Instant::now(),
            last_traffic_bucket: 0,
            last_manual_tune_time: std::time::Instant::now(),
            active_secondary_tool: None,
            sdr_ai_panel_open: false,
            adsb_instructions_open: false,
            satellite_advanced: false,
        }
    }
}

impl CentralApp {
    /// Programmatically focus a tab (used by tutorial navigation).
    fn focus_tab(&mut self, tab: &Tab) {
        match tab {
            Tab::AdsB => { self.current_tab = AppTab::AdsB; self.active_secondary_tool = None; }
            Tab::Satellite => { self.current_tab = AppTab::Satellite; self.active_secondary_tool = None; }
            Tab::AiAgent => { self.current_tab = AppTab::Ai; self.active_secondary_tool = None; }
            Tab::Bookmarks => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = Some(SecondaryTool::Bookmarks); }
            Tab::Scheduler => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = Some(SecondaryTool::Scheduler); }
            Tab::Settings => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = Some(SecondaryTool::Settings); }
            Tab::Scanner => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = Some(SecondaryTool::Scanner); }
            Tab::Recorder => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = Some(SecondaryTool::Recorder); }
            Tab::HowTo => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = Some(SecondaryTool::HowTo); }
            Tab::Discord => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = Some(SecondaryTool::Discord); }
            Tab::Sdr | Tab::Spectrum => { self.current_tab = AppTab::Sdr; self.active_secondary_tool = None; }
        }
    }
}

impl eframe::App for CentralApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain samples from source — single lock, fast
        let mut sample_batch: Vec<Vec<u8>> = Vec::new();
        {
            if let Ok(mut state) = self.shared.try_lock() {
                for _ in 0..4 {
                    if let Some(samples) = state.source.recv_samples() {
                        if samples == b"ERROR" {
                            state.source.status = crate::source_manager::SourceStatus::Error("Device open failed".to_string());
                            break;
                        }
                        sample_batch.push(samples);
                    } else {
                        break;
                    }
                }
            }
        }

        // Process samples outside lock
        for samples in &sample_batch {
            let (freq, rate, demod_mode, audio_running, volume, squelch_db, lpf_cutoff, adsb_running) = {
                if let Ok(state) = self.shared.try_lock() {
                    (state.source.frequency_hz, state.source.sample_rate_hz,
                     state.demod_mode, state.audio_running, state.volume, state.squelch, state.lpf_cutoff, state.adsb_running)
                } else {
                    continue;
                }
            };
            {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.spectrum.update_params(freq, rate);
                    state.spectrum.push_iq_samples(samples);
                }
            }
            self.recorder_panel.write_samples(samples);

            // Demodulate and send audio
            if audio_running {
                // Reset demodulator state when mode changes to avoid audio clicks
                if demod_mode != self.last_demod_mode {
                    self.demod.reset();
                    self.last_demod_mode = demod_mode;
                }
                self.demod.set_sample_rates(rate, self.audio.sample_rate());
                self.demod.set_lpf_cutoff(lpf_cutoff);
                let audio = self.demod.demodulate(samples, demod_mode);
                let gate: f32 = if squelch_db < -80.0 { 1.0 } else {
                    let signal_level = if let Ok(state) = self.shared.try_lock() {
                        state.spectrum.signal_level()
                    } else { -120.0 };
                    if signal_level > squelch_db { 1.0 } else { 0.0 }
                };
                let audio: Vec<f32> = audio.into_iter().map(|s| s * volume * gate).collect();
                self.recorder_panel.write_audio_samples(&audio);
                // Push audio to waveform display (decimate to ~1000 samples for visualization)
                if let Ok(mut state) = self.shared.try_lock() {
                    let wf = &mut state.spectrum.audio_waveform;
                    if wf.capacity() < 2048 { wf.reserve(2048); }
                    for &s in audio.iter().step_by((audio.len() / 800).max(1)) {
                        if wf.len() >= 2048 { wf.pop_front(); }
                        wf.push_back(s);
                    }
                }
                let _ = self.audio_tx.try_send(audio);
                // Update demod metrics in shared state
                if let Ok(mut state) = self.shared.try_lock() {
                    state.fm_deviation_hz = self.demod.last_fm_deviation_hz;
                    state.audio_peak = self.demod.last_audio_peak;
                }
            }

            // Feed to ADS-B decoder when tuned to 1090 MHz
            if adsb_running && (freq == 1_090_000_000 || (freq > 1_088_000_000 && freq < 1_092_000_000)) {
                self.adsb_decoder.feed_iq(samples, rate);
                let ac = self.adsb_decoder.get_aircraft();
                self.adsb_panel.aircraft = ac;
                self.adsb_panel.total_messages = self.adsb_decoder.total_messages;
            }
        }

        // Passive ADS-B: detect newly-arrived aircraft and fire alert toasts + Discord notifications.
        self.adsb_panel.check_for_new_aircraft();
        if let Some(msg) = self.adsb_panel.pending_status_flash.take() {
            self.status_flash = Some((msg, std::time::Instant::now()));
        }
        if let Some(msg) = self.sdr_panel.pending_status.take() {
            self.status_flash = Some((msg, std::time::Instant::now()));
        }
        if let Some(msg) = self.satellite_panel.pending_status.take() {
            self.status_flash = Some((msg, std::time::Instant::now()));
        }
        // Fire Discord notifications for new aircraft
        for ac in &self.adsb_panel.aircraft {
            if self.seen_aircraft.insert(ac.icao) {
                let icao_str = format!("{:06X}", ac.icao);
                // Try to fetch aircraft image (non-blocking, returns None if not found)
                let image_url = crate::discord::fetch_aircraft_image(&icao_str);
                let embed = crate::discord::embed_aircraft(
                    &icao_str,
                    &ac.callsign,
                    ac.lat,
                    ac.lon,
                    ac.altitude,
                    ac.speed,
                    ac.heading,
                    image_url,
                );
                self.discord.fire("aircraft_new", embed);
            }
        }
        // Check traffic milestone (every 10 aircraft)
        let current_bucket = self.adsb_panel.aircraft.len() / 10;
        if current_bucket > self.last_traffic_bucket && current_bucket > 0 {
            let milestone = current_bucket * 10;
            let embed = crate::discord::embed_generic(
                &format!("Traffic Milestone: {} Aircraft", milestone),
                &format!("You're now tracking **{}** aircraft!", milestone),
                "📈",
                0xFF8800,
            );
            self.discord.fire("traffic_milestone", embed);
            self.last_traffic_bucket = current_bucket;
        }

        // Keyboard shortcuts
        let mut ctrl_r_pressed = false;
        let mut freq_changed = false;
        ctx.input(|i| {
            // ? : toggle keyboard help
            if i.key_pressed(egui::Key::Questionmark) {
                self.show_keyboard_help = !self.show_keyboard_help;
            }
            if let Ok(mut state) = self.shared.try_lock() {
                // Space: toggle start/stop
                if i.key_pressed(egui::Key::Space) {
                    if state.source.status == crate::source_manager::SourceStatus::Running {
                        state.source.stop();
                    } else {
                        state.source.start();
                    }
                }
                // Arrow keys: tune up/down/left/right (coarse = up/down, fine = left/right)
                // Shift modifier doubles the step
                let fine = state.tune_step_fine_hz * if i.modifiers.shift { 10 } else { 1 };
                let coarse = state.tune_step_coarse_hz * if i.modifiers.shift { 10 } else { 1 };
                if i.key_pressed(egui::Key::ArrowUp) && !i.modifiers.alt {
                    state.source.frequency_hz = (state.source.frequency_hz + coarse).min(1_770_000_000);
                    freq_changed = true;
                }
                if i.key_pressed(egui::Key::ArrowDown) && !i.modifiers.alt {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(coarse).max(500_000);
                    freq_changed = true;
                }
                if i.key_pressed(egui::Key::ArrowRight) && !i.modifiers.alt {
                    state.source.frequency_hz = (state.source.frequency_hz + fine).min(1_770_000_000);
                    freq_changed = true;
                }
                if i.key_pressed(egui::Key::ArrowLeft) && !i.modifiers.alt {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(fine).max(500_000);
                    freq_changed = true;
                }
                // Alt+Left/Right: frequency history back/forward
                if i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft) {
                    let hist: Vec<u64> = state.freq_history.iter().cloned().collect();
                    if !hist.is_empty() {
                        let cur_idx = self.freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                        if cur_idx > 0 {
                            let new_idx = cur_idx - 1;
                            self.freq_history_idx = Some(new_idx);
                            state.source.frequency_hz = hist[new_idx];
                            self.last_history_freq = hist[new_idx];
                            freq_changed = true;
                        }
                    }
                }
                if i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight) {
                    let hist: Vec<u64> = state.freq_history.iter().cloned().collect();
                    if !hist.is_empty() {
                        let cur_idx = self.freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                        if cur_idx + 1 < hist.len() {
                            let new_idx = cur_idx + 1;
                            self.freq_history_idx = Some(new_idx);
                            state.source.frequency_hz = hist[new_idx];
                            self.last_history_freq = hist[new_idx];
                            freq_changed = true;
                        }
                    }
                }
                // F1-F6: select demod mode
                if i.key_pressed(egui::Key::F1) { state.demod_mode = crate::sdr_panel::DemodMode::Raw; }
                if i.key_pressed(egui::Key::F2) { state.demod_mode = crate::sdr_panel::DemodMode::Am; }
                if i.key_pressed(egui::Key::F3) { state.demod_mode = crate::sdr_panel::DemodMode::Fm; }
                if i.key_pressed(egui::Key::F4) { state.demod_mode = crate::sdr_panel::DemodMode::Wfm; }
                if i.key_pressed(egui::Key::F5) { state.demod_mode = crate::sdr_panel::DemodMode::Lsb; }
                if i.key_pressed(egui::Key::F6) { state.demod_mode = crate::sdr_panel::DemodMode::Usb; }
                // Alt+letter: quick demod mode shortcuts
                if i.modifiers.alt && i.key_pressed(egui::Key::F) {
                    state.demod_mode = crate::sdr_panel::DemodMode::Fm;
                    self.status_flash = Some(("NFM".to_string(), std::time::Instant::now()));
                }
                if i.modifiers.alt && i.key_pressed(egui::Key::W) {
                    state.demod_mode = crate::sdr_panel::DemodMode::Wfm;
                    self.status_flash = Some(("WFM".to_string(), std::time::Instant::now()));
                }
                if i.modifiers.alt && i.key_pressed(egui::Key::A) {
                    state.demod_mode = crate::sdr_panel::DemodMode::Am;
                    self.status_flash = Some(("AM".to_string(), std::time::Instant::now()));
                }
                if i.modifiers.alt && i.key_pressed(egui::Key::U) {
                    state.demod_mode = crate::sdr_panel::DemodMode::Usb;
                    self.status_flash = Some(("USB".to_string(), std::time::Instant::now()));
                }
                if i.modifiers.alt && i.key_pressed(egui::Key::L) {
                    state.demod_mode = crate::sdr_panel::DemodMode::Lsb;
                    self.status_flash = Some(("LSB".to_string(), std::time::Instant::now()));
                }
                if i.modifiers.alt && i.key_pressed(egui::Key::R) {
                    state.demod_mode = crate::sdr_panel::DemodMode::Raw;
                    self.status_flash = Some(("RAW".to_string(), std::time::Instant::now()));
                }
                // Ctrl+R: toggle recording
                if i.modifiers.ctrl && i.key_pressed(egui::Key::R) {
                    ctrl_r_pressed = true;
                }
                // M: toggle audio mute
                if i.key_pressed(egui::Key::M) {
                    state.audio_running = !state.audio_running;
                }
                // Ctrl+S: save config (also persists recent frequencies + spectrum range + PPM + session state)
                if i.modifiers.ctrl && i.key_pressed(egui::Key::S) {
                    let recent: Vec<u64> = state.freq_history.iter().cloned().collect();
                    state.config.recent_frequencies = recent;
                    let (min_db, max_db) = state.spectrum.display_range();
                    state.config.spectrum_min_db = min_db;
                    state.config.spectrum_max_db = max_db;
                    state.config.ppm_correction = state.source.ppm_correction;
                    state.config.vfo_b_hz = state.vfo_b;
                    state.config.wf_min_db = state.spectrum.wf_min_db;
                    state.config.wf_max_db = state.spectrum.wf_max_db;
                    state.config.lo_offset_hz = state.lo_offset_hz;
                    state.config.color_map = state.spectrum.color_map.name().to_string();
                    state.config.freq_memory_hz = state.freq_memory.iter().map(|m| m.freq_hz).collect();
                    state.config.freq_memory_labels = state.freq_memory.iter().map(|m| m.label.clone()).collect();
                    // Save current session state so next launch resumes here
                    state.config.last_session_freq_hz = state.source.frequency_hz;
                    state.config.last_session_gain_db = state.source.gain_db;
                    state.config.last_session_demod = state.demod_mode.label().to_string();
                    state.config.save();
                    if let Some(flash) = &mut self.status_flash {
                        flash.0 = "💾 Config saved (freq, gain, demod, spectrum range)".to_string();
                        flash.1 = std::time::Instant::now();
                    } else {
                        self.status_flash = Some(("💾 Config saved".to_string(), std::time::Instant::now()));
                    }
                }
                // F: freeze/unfreeze spectrum
                if i.key_pressed(egui::Key::F) && !i.modifiers.ctrl && !i.modifiers.alt {
                    state.spectrum.frozen = !state.spectrum.frozen;
                }
                // C: cycle waterfall colormap
                if i.key_pressed(egui::Key::C) && !i.modifiers.ctrl && !i.modifiers.alt {
                    state.spectrum.cycle_colormap();
                }
                // Ctrl++ / Ctrl+- / Ctrl+0: zoom in/out/reset
                if (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) && i.modifiers.ctrl {
                    state.spectrum.zoom_in();
                }
                if i.key_pressed(egui::Key::Minus) && i.modifiers.ctrl && !i.modifiers.alt {
                    state.spectrum.zoom_out();
                }
                if i.key_pressed(egui::Key::Num0) && i.modifiers.ctrl {
                    state.spectrum.zoom_reset();
                }
                // J: open frequency jump dialog
                if i.key_pressed(egui::Key::J) && !i.modifiers.ctrl && !i.modifiers.alt {
                    self.show_freq_jump = !self.show_freq_jump;
                    if self.show_freq_jump {
                        self.freq_jump_input.clear();
                        self.freq_jump_matches.clear();
                    }
                }
                // P: toggle peak hold
                if i.key_pressed(egui::Key::P) && !i.modifiers.ctrl && !i.modifiers.alt {
                    let on = state.spectrum.toggle_peak_hold();
                    self.status_flash = Some((
                        if on { "Peak Hold ON".to_string() } else { "Peak Hold OFF".to_string() },
                        std::time::Instant::now()
                    ));
                }
                // V: swap VFO A/B
                if i.key_pressed(egui::Key::V) && !i.modifiers.ctrl && !i.modifiers.alt {
                    let tmp = state.source.frequency_hz;
                    state.source.frequency_hz = state.vfo_b;
                    state.vfo_b = tmp;
                    freq_changed = true;
                }
                // 1-9: tune to bookmark #N (no modifiers)
                // Alt+Shift+1-9: save frequency memory M1-M9
                // Alt+1-9: recall frequency memory M1-M9
                {
                    let mem_keys = [
                        (egui::Key::Num1, 0usize), (egui::Key::Num2, 1), (egui::Key::Num3, 2),
                        (egui::Key::Num4, 3), (egui::Key::Num5, 4), (egui::Key::Num6, 5),
                        (egui::Key::Num7, 6), (egui::Key::Num8, 7), (egui::Key::Num9, 8),
                    ];
                    for (key, idx) in mem_keys {
                        if i.key_pressed(key) && !i.modifiers.ctrl {
                            if i.modifiers.alt && i.modifiers.shift {
                                // Save memory
                                state.freq_memory[idx].freq_hz = state.source.frequency_hz;
                                if state.freq_memory[idx].label.is_empty() {
                                    state.freq_memory[idx].label = format!("{:.4} MHz", state.source.frequency_hz as f64 / 1e6);
                                }
                                self.status_flash = Some((
                                    format!("💾 M{} saved: {:.4} MHz", idx + 1, state.source.frequency_hz as f64 / 1e6),
                                    std::time::Instant::now()
                                ));
                                break;
                            } else if i.modifiers.alt {
                                // Recall memory
                                if state.freq_memory[idx].freq_hz > 0 {
                                    state.source.frequency_hz = state.freq_memory[idx].freq_hz;
                                    freq_changed = true;
                                    self.status_flash = Some((
                                        format!("🔁 M{} recalled: {:.4} MHz", idx + 1, state.freq_memory[idx].freq_hz as f64 / 1e6),
                                        std::time::Instant::now()
                                    ));
                                }
                                break;
                            } else if !i.modifiers.shift {
                                // Bookmark 1-9 (no modifiers)
                                if let Some(bm) = state.bookmarks.bookmarks.get(idx) {
                                    state.source.frequency_hz = bm.frequency_hz;
                                    freq_changed = true;
                                }
                                break;
                            }
                        }
                    }
                }
                // B: tune to nearest bookmark
                if i.key_pressed(egui::Key::B) && !i.modifiers.ctrl && !i.modifiers.alt {
                    let cur = state.source.frequency_hz;
                    let nearest = state.bookmarks.bookmarks.iter()
                        .min_by_key(|b| (b.frequency_hz as i64 - cur as i64).unsigned_abs())
                        .map(|bm| (bm.frequency_hz, bm.name.clone()));
                    if let Some((freq, name)) = nearest {
                        state.source.frequency_hz = freq;
                        freq_changed = true;
                        self.status_flash = Some((format!("⭐ {}", name), std::time::Instant::now()));
                    }
                }
                // [ / ] : frequency history back/forward
                if i.key_pressed(egui::Key::OpenBracket) && !i.modifiers.ctrl && !i.modifiers.alt {
                    let hist: Vec<u64> = state.freq_history.iter().cloned().collect();
                    if !hist.is_empty() {
                        let cur_idx = self.freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                        if cur_idx > 0 {
                            let new_idx = cur_idx - 1;
                            self.freq_history_idx = Some(new_idx);
                            state.source.frequency_hz = hist[new_idx];
                            self.last_history_freq = hist[new_idx];
                            freq_changed = true;
                        }
                    }
                }
                if i.key_pressed(egui::Key::CloseBracket) && !i.modifiers.ctrl && !i.modifiers.alt {
                    let hist: Vec<u64> = state.freq_history.iter().cloned().collect();
                    if !hist.is_empty() {
                        let cur_idx = self.freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                        if cur_idx + 1 < hist.len() {
                            let new_idx = cur_idx + 1;
                            self.freq_history_idx = Some(new_idx);
                            state.source.frequency_hz = hist[new_idx];
                            self.last_history_freq = hist[new_idx];
                            freq_changed = true;
                        }
                    }
                }

                // G / Shift+G: gain step up / down by 5 dB
                if i.key_pressed(egui::Key::G) && !i.modifiers.ctrl && !i.modifiers.alt {
                    if i.modifiers.shift {
                        state.source.gain_db = (state.source.gain_db - 5.0).max(0.0);
                        self.status_flash = Some((format!("Gain: {:.0} dB", state.source.gain_db), std::time::Instant::now()));
                    } else {
                        state.source.gain_db = (state.source.gain_db + 5.0).min(49.0);
                        self.status_flash = Some((format!("Gain: {:.0} dB", state.source.gain_db), std::time::Instant::now()));
                    }
                }
                // Ctrl+Up/Down: volume up / down by 10%
                if i.modifiers.ctrl && i.key_pressed(egui::Key::ArrowUp) {
                    state.volume = (state.volume + 0.1).min(1.0);
                    self.status_flash = Some((format!("Volume: {:.0}%", state.volume * 100.0), std::time::Instant::now()));
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::ArrowDown) {
                    state.volume = (state.volume - 0.1).max(0.0);
                    self.status_flash = Some((format!("Volume: {:.0}%", state.volume * 100.0), std::time::Instant::now()));
                }
                // Ctrl+B: quick bookmark current frequency
                if i.modifiers.ctrl && i.key_pressed(egui::Key::B) {
                    let freq = state.source.frequency_hz;
                    let mode = state.demod_mode.label().to_string();
                    let name = format!("{:.3} MHz", freq as f64 / 1e6);
                    let already = state.bookmarks.bookmarks.iter().any(|b| b.frequency_hz == freq);
                    if !already {
                        state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                            name: "Quick".to_string(),
                            frequency_hz: freq,
                            mode: mode.clone(),
                            bandwidth_hz: 12_500,
                            category: "Quick".to_string(),
                            notes: String::new(),
                            starred: false,
                        });
                        state.bookmarks_modified = true;
                        state.spectrum.bookmark_freqs_dirty = true;
                        self.status_flash = Some((format!("🔖 Bookmarked {}", name), std::time::Instant::now()));
                    } else {
                        self.status_flash = Some((format!("⭐ Already bookmarked"), std::time::Instant::now()));
                    }
                }
                // T: tune to spectrum peak frequency
                if i.key_pressed(egui::Key::T) && !i.modifiers.ctrl && !i.modifiers.alt {
                    let peak_hz = state.spectrum.peak_freq_hz();
                    if peak_hz > 0 {
                        state.source.frequency_hz = peak_hz;
                        freq_changed = true;
                        self.status_flash = Some((format!("📡 Peak: {:.3} MHz", peak_hz as f64 / 1e6), std::time::Instant::now()));
                    }
                }
                // S key: toggle scanner on/off
                if i.key_pressed(egui::Key::S) && !i.modifiers.ctrl && !i.modifiers.alt {
                    if self.scanner.enabled {
                        self.scanner.stop();
                        self.status_flash = Some((format!("🔍 Scanner: OFF"), std::time::Instant::now()));
                    } else {
                        self.scanner.start();
                        self.status_flash = Some((format!("🔍 Scanner: ON"), std::time::Instant::now()));
                    }
                }
                // R key: reset spectrum dB range to default (-120 to 0)
                if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl && !i.modifiers.alt {
                    state.spectrum.set_display_range(-120.0, 0.0);
                    self.status_flash = Some((format!("📊 dB range reset to -120…0"), std::time::Instant::now()));
                }
            }
        });
        if freq_changed {
            self.last_manual_tune_time = std::time::Instant::now();
        }

        // Ctrl+R: toggle recording (must be outside ctx.input to access recorder_panel)
        if ctrl_r_pressed {
            if self.recorder_panel.recording {
                self.recorder_panel.stop_recording();
            } else {
                self.recorder_panel.start_recording();
            }
        }

        // Track source status transitions
        if let Ok(mut state) = self.shared.try_lock() {
            let was_running = matches!(self.last_source_status, crate::source_manager::SourceStatus::Running);
            let is_running = state.source.status == crate::source_manager::SourceStatus::Running;
            if was_running && !is_running {
                state.adsb_running = false;
            }
            self.last_source_status = state.source.status.clone();
        }

        // Repaint rate: 30fps when active, 1fps when idle (saves CPU on battery)
        ctx.request_repaint_after(Duration::from_millis(if sample_batch.is_empty() { 1000 } else { 33 }));

        // Dynamic window title: show current frequency so it's visible in taskbar
        {
            if let Ok(state) = self.shared.try_lock() {
                let freq_mhz = state.source.frequency_hz as f64 / 1e6;
                let mode = state.demod_mode.label();
                let running = state.source.status == crate::source_manager::SourceStatus::Running;
                let title = if running {
                    format!("EZ-SDR — {:.3} MHz {} ▶", freq_mhz, mode)
                } else {
                    format!("EZ-SDR — {:.3} MHz {} ■", freq_mhz, mode)
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            }
        }

        // Start/stop audio based on state (once only, no retry storm)
        {
            if let Ok(state) = self.shared.try_lock() {
                if state.audio_running && !self.audio.is_running() {
                    if !self.audio.has_failed() {
                        let rx = self.audio_rx.clone();
                        if self.audio.start(rx).is_ok() {
                            self.demod.reset();
                        } else {
                            self.audio.mark_failed();
                        }
                    }
                } else if !state.audio_running && self.audio.is_running() {
                    self.audio.stop();
                }
            }
        }

        // Process web remote commands
        for cmd in self.web_remote.poll_commands() {
            match cmd {
                RemoteCommand::Tune { freq_hz } => {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.source.frequency_hz = freq_hz;
                    }
                }
                RemoteCommand::SetGain { gain_db } => {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.source.gain_db = gain_db;
                    }
                }
                RemoteCommand::SetDemod { mode } => {
                    if let Ok(mut state) = self.shared.try_lock() {
                        use crate::sdr_panel::DemodMode;
                        if let Some(dm) = DemodMode::from_label(&mode) {
                            state.demod_mode = dm;
                        }
                    }
                }
                RemoteCommand::SetSquelch { db } => {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.squelch = db;
                    }
                }
                RemoteCommand::SetVolume { level } => {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.volume = level.clamp(0.0, 1.0);
                    }
                }
                RemoteCommand::StartRecord => self.recorder_panel.start_recording(),
                RemoteCommand::StopRecord => self.recorder_panel.stop_recording(),
                RemoteCommand::StartScan => self.scanner.start(),
                RemoteCommand::StopScan => self.scanner.stop(),
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
                let tune_to = state.scheduler.active_job(now_unix).map(|j| (j.satellite.clone(), j.frequency_hz));
                if let Some((sat, freq)) = tune_to {
                    // Satellite AOS detection (new pass)
                    if sat != self.last_active_pass_sat {
                        let passes = state.tle.upcoming_passes();
                        if let Some(pass) = passes.iter().find(|p| p.satellite == sat) {
                            let embed = crate::discord::embed_sat_aos(&sat, freq, pass.max_elevation);
                            self.discord.fire("sat_aos", embed);
                        }
                        self.last_active_pass_sat = sat.clone();
                    }
                    // Only auto-tune if user hasn't manually changed frequency recently (15s cooldown)
                    let manual_tune_recent = self.last_manual_tune_time.elapsed().as_secs() < 15;
                    if !manual_tune_recent && sat != self.last_auto_tuned_satellite {
                        state.source.frequency_hz = freq;
                        self.last_auto_tuned_satellite = sat.clone();
                        self.status_flash = Some((format!("🛰 Auto-tuned to {} ({:.3} MHz)", sat, freq as f64 / 1e6), std::time::Instant::now()));
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
                    eprintln!("[scheduler] fired custom task '{}' → {:.3} MHz", label, freq as f64 / 1e6);
                    let embed = crate::discord::embed_task_fired(&label, freq);
                    self.discord.fire("task_fired", embed);
                }
            }

            // Squelch-triggered recording tick
            {
                let (signal_db, squelch_db, freq_hz, mode_label) = if let Ok(state) = self.shared.try_lock() {
                    (state.spectrum.signal_level(), state.squelch, state.source.frequency_hz, state.demod_mode.label().to_string())
                } else { (-120.0, -50.0, 0u64, "NFM".to_string()) };
                self.recorder_panel.tick_squelch_record(signal_db, squelch_db, freq_hz, &mode_label);
            }

            // Frequency scanner tick (runs every frame, rate-limited by dwell_ms)
            {
                let peak = if let Ok(state) = self.shared.try_lock() {
                    state.spectrum.peak_level()
                } else { -120.0 };
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
                let name = format!("{:.3} MHz {}", freq_mhz, mode);
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
                "I'm currently tuned to {:.4} MHz in {} mode (SNR: {:.1} dB). \
                 What signals should I expect here? What demod mode and settings would you recommend?",
                freq_mhz, mode, snr
            );
            self.status_flash = Some((
                format!("🤖 AI prompt ready for {:.3} MHz — switch to AI Agent tab", freq_mhz),
                std::time::Instant::now(),
            ));
        }

        // Apply theme on first frame (retry until the lock is available)
        if !self.theme_applied {
            if let Ok(mut state) = self.shared.try_lock() {
                state.theme_colors = ThemeColors::from(&state.config.theme_config);
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
                state.theme_colors = ThemeColors::from(&state.config.theme_config);
                state.config.theme_config.apply_to_ctx(ctx);
                // Apply font scale
                let scale = state.config.font_scale as f32;
                ctx.set_pixels_per_point(scale);
                state.source.frequency_hz = state.config.default_freq_hz;
                state.source.sample_rate_hz = state.config.default_sample_rate;
                state.source.gain_db = state.config.default_gain;
                state.tle.observer_lat = state.config.observer_lat;
                state.tle.observer_lon = state.config.observer_lon;
                self.web_remote.set_enabled(state.config.web_remote_enabled, state.config.web_remote_port);
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
                let embed = crate::discord::embed_recording_started(freq, &mode, self.recorder_panel.record_iq, self.recorder_panel.record_audio);
                self.discord.fire("rec_started", embed);
            } else if !recording && self.last_recording {
                let duration = self.recording_start.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                let embed = crate::discord::embed_recording_stopped(freq, &mode, duration, self.recorder_panel.bytes_written);
                self.discord.fire("rec_stopped", embed);
            }
            if state.adsb_running && !self.last_adsb_running {
                let embed = crate::discord::embed_generic("ADS-B Started", "ADS-B decoder activated", "📡", 0x00AA00);
                self.discord.fire("adsb_started", embed);
            } else if !state.adsb_running && self.last_adsb_running {
                let embed = crate::discord::embed_generic("ADS-B Stopped", "ADS-B decoder stopped", "🔌", 0xCC0000);
                self.discord.fire("adsb_stopped", embed);
            }
            if self.scanner.enabled && !self.last_scanner_enabled {
                let embed = crate::discord::embed_generic("Scanner Started", "Frequency scanner activated", "▶️", 0x00AA00);
                self.discord.fire("scanner_started", embed);
            } else if !self.scanner.enabled && self.last_scanner_enabled {
                let embed = crate::discord::embed_generic("Scanner Stopped", "Frequency scanner stopped", "⏹", 0xCC0000);
                self.discord.fire("scanner_stopped", embed);
            }
            if self.mqtt.is_connected() && !self.last_mqtt_connected {
                let embed = crate::discord::embed_generic("MQTT Connected", &format!("Connected to {}", self.mqtt.broker), "🔗", 0x00AA00);
                self.discord.fire("mqtt_connected", embed);
            } else if !self.mqtt.is_connected() && self.last_mqtt_connected {
                let embed = crate::discord::embed_generic("MQTT Disconnected", "MQTT broker disconnected", "🔌", 0xCC0000);
                self.discord.fire("mqtt_disconnected", embed);
            }
            self.last_recording = recording;
            self.last_adsb_running = state.adsb_running;
            self.last_scanner_enabled = self.scanner.enabled;
            self.last_mqtt_connected = self.mqtt.is_connected();

            // First strong signal celebration
            if !self.first_strong_signal_seen && snr > 20.0 {
                self.first_strong_signal_seen = true;
                self.status_flash = Some((
                    format!("🎉 First signal! {:.3} MHz — SNR {:.1} dB — great reception!", freq as f64 / 1e6, snr),
                    std::time::Instant::now(),
                ));
                let embed = crate::discord::embed_strong_signal(freq, snr);
                self.discord.fire("first_signal", embed);
            }
            // Any strong signal
            if snr > 20.0 && (self.discord_summary_last.elapsed().as_secs() > 5) {
                let embed = crate::discord::embed_strong_signal(freq, snr);
                self.discord.fire("strong_signal", embed);
            }
            self.web_remote.broadcast_state(freq, gain, &mode, ac_count, &passes, squelch, volume, recording, scanner_active, snr);
            self.mqtt.tick_reconnect();
            if self.last_scheduler_update.elapsed().as_secs() < 1 {
                self.mqtt.tick(freq, gain);
                self.mqtt.publish_signal(freq, peak, noise, &mode, recording);
                self.mqtt.publish_passes(&passes);
                if !self.adsb_panel.aircraft.is_empty() {
                    self.mqtt.publish_aircraft(&self.adsb_panel.aircraft);
                }
            }

            // Periodic session summary report
            if self.discord.settings.summary_enabled && self.discord_summary_last.elapsed().as_secs() >= (self.discord.settings.summary_interval_min as u64 * 60) {
                let uptime = self.recording_start.as_ref()
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                let embed = crate::discord::embed_session_summary(
                    uptime,
                    freq as f64 / 1e6,
                    &mode,
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

        egui::Panel::top("main_tab_bar")
            .exact_size(44.0)
            .show_inside(ui, |ui| self.render_tab_bar(ui));

        egui::Panel::left("secondary_rail")
            .exact_size(44.0)
            .show_inside(ui, |ui| self.render_secondary_rail(ui));

        if let Some(tool) = self.active_secondary_tool {
            egui::Panel::left("secondary_panel")
                .resizable(true)
                .default_size(360.0)
                .show_inside(ui, |ui| self.render_secondary_panel(ui, tool, &snapshot));
        }

        match self.current_tab {
            AppTab::Sdr => self.render_sdr_tab(ui, &snapshot),
            AppTab::AdsB => self.render_adsb_tab(ui),
            AppTab::Satellite => self.render_satellite_tab(ui, &snapshot),
            AppTab::Ai => self.render_ai_tab(ui),
        }

        // Tutorial / first-run onboarding
        if self.tutorial.active {
            let dismissed = tutorial::render_tutorial(
                &mut self.tutorial,
                &self.shared,
                ui,
            );
            if dismissed && !self.tutorial.active {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.config.tutorial_seen = true;
                    state.config.user_level = self.tutorial.level.to_str().to_string();
                    state.config.tutorial_step = 0;
                    state.config.save();
                }
            }
            return; // Don't render main UI during tutorial
        }

        // Handle tab navigation from tutorial
        if let Some(tab) = self.tutorial.tab_to_open.take() {
            // Focus the tab by selecting it in the dock
            self.focus_tab(&tab);
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
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.freq_jump_input)
                        .desired_width(340.0)
                        .hint_text("e.g. '145.5', '88.0', 'aviation', 'weather', 'noaa'"));
                    if edit.changed() {
                        self.freq_jump_matches.clear();
                        let q = self.freq_jump_input.trim().to_lowercase();
                        if !q.is_empty() {
                            // Try numeric parse first
                            if let Ok(mhz) = q.parse::<f64>() {
                                self.freq_jump_matches.push((format!("{:.3} MHz", mhz), (mhz * 1e6) as u64));
                            } else {
                                // Search known band allocations
                                let bands: &[(&str, u64)] = &[
                                    ("AM Broadcast (530 kHz)", 530_000),
                                    ("Shortwave 49m (5.9 MHz)", 5_900_000),
                                    ("Shortwave 31m (9.5 MHz)", 9_500_000),
                                    ("Shortwave 25m (11.7 MHz)", 11_700_000),
                                    ("FM Broadcast (88 MHz)", 88_000_000),
                                    ("Aviation VHF (118 MHz)", 118_000_000),
                                    ("NOAA APT satellites (137.6 MHz)", 137_620_000),
                                    ("NOAA Weather Radio (162.4 MHz)", 162_400_000),
                                    ("Marine VHF Ch16 distress (156.8 MHz)", 156_800_000),
                                    ("APRS North America (144.39 MHz)", 144_390_000),
                                    ("Amateur 2m simplex (146.52 MHz)", 146_520_000),
                                    ("Amateur 70cm simplex (446 MHz)", 446_000_000),
                                    ("PMR446 (446.006 MHz)", 446_006_250),
                                    ("ISM 433 MHz band", 433_920_000),
                                    ("ISM 868 MHz band", 868_000_000),
                                    ("ADS-B aircraft transponder (1090 MHz)", 1_090_000_000),
                                    ("GPS L1 (1575.42 MHz)", 1_575_420_000),
                                    ("GOES satellite downlink (1691 MHz)", 1_691_000_000),
                                ];
                                for (name, freq) in bands {
                                    if name.to_lowercase().contains(&q) {
                                        self.freq_jump_matches.push((name.to_string(), *freq));
                                    }
                                }
                                // Also search bookmarks
                                if let Ok(state) = self.shared.try_lock() {
                                    for bm in &state.bookmarks.bookmarks {
                                        if bm.name.to_lowercase().contains(&q) || bm.category.to_lowercase().contains(&q) {
                                            self.freq_jump_matches.push((
                                                format!("⭐ {} ({:.3} MHz)", bm.name, bm.frequency_hz as f64 / 1e6),
                                                bm.frequency_hz
                                            ));
                                        }
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
                        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            // Show search matches if any
                            for (label, freq) in self.freq_jump_matches.clone() {
                                if ui.selectable_label(false, &label).clicked() {
                                    tune_to = Some(freq);
                                    close = true;
                                }
                            }

                            // Show suggestions (recent freqs + bookmarks) when input is empty
                            if should_show_suggestions {
                                ui.label(egui::RichText::new("Recent:").italics().small());
                                if let Ok(state) = self.shared.try_lock() {
                                    let recent: Vec<u64> = state.freq_history.iter().cloned().rev().take(5).collect();
                                    for freq in recent {
                                        let label = format!("  {:.3} MHz", freq as f64 / 1e6);
                                        if ui.selectable_label(false, &label).clicked() {
                                            tune_to = Some(freq);
                                            close = true;
                                        }
                                    }
                                }

                                ui.label(egui::RichText::new("Bookmarks:").italics().small());
                                if let Ok(state) = self.shared.try_lock() {
                                    let bookmarks_to_show = state.bookmarks.bookmarks.iter().take(5);
                                    for bm in bookmarks_to_show {
                                        let label = format!("  ⭐ {} ({:.3} MHz)", &bm.name, bm.frequency_hz as f64 / 1e6);
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
                        if ui.button("Tune").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            if let Some((_, freq)) = self.freq_jump_matches.first() {
                                tune_to = Some(*freq);
                            } else if let Ok(mhz) = self.freq_jump_input.trim().parse::<f64>() {
                                tune_to = Some((mhz * 1e6) as u64);
                            }
                            close = true;
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            close = true;
                        }
                    });
                });
            if close { self.show_freq_jump = false; }
            if let Some(freq) = tune_to {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.source.frequency_hz = freq;
                    self.last_manual_tune_time = std::time::Instant::now();
                    self.status_flash = Some((format!("⤵ {:.3} MHz", freq as f64 / 1e6), std::time::Instant::now()));
                }
            }
        }

        // Status bar
        ui.separator();
        ui.horizontal(|ui| {
            // Frequency history nav
            let (hist_len, hist_idx) = if let Ok(state) = self.shared.try_lock() {
                (state.freq_history.len(), self.freq_history_idx.unwrap_or(state.freq_history.len().saturating_sub(1)))
            } else { (0, 0) };
            let can_back = hist_idx > 0 && hist_len > 1;
            let can_fwd = self.freq_history_idx.is_some() && hist_idx + 1 < hist_len;
            if ui.add_enabled(can_back, egui::Button::new("◀")).on_hover_text("Go back to previous frequency (Alt+←)").clicked() {
                if let Ok(mut state) = self.shared.try_lock() {
                    let hist: Vec<u64> = state.freq_history.iter().cloned().collect();
                    if hist_idx > 0 {
                        let new_idx = hist_idx - 1;
                        self.freq_history_idx = Some(new_idx);
                        state.source.frequency_hz = hist[new_idx];
                        self.last_history_freq = hist[new_idx];
                        self.last_manual_tune_time = std::time::Instant::now();
                    }
                }
            }
            if ui.add_enabled(can_fwd, egui::Button::new("▶")).on_hover_text("Go forward in frequency history (Alt+→)").clicked() {
                if let Ok(mut state) = self.shared.try_lock() {
                    let hist: Vec<u64> = state.freq_history.iter().cloned().collect();
                    if hist_idx + 1 < hist_len {
                        let new_idx = hist_idx + 1;
                        self.freq_history_idx = Some(new_idx);
                        state.source.frequency_hz = hist[new_idx];
                        self.last_history_freq = hist[new_idx];
                        self.last_manual_tune_time = std::time::Instant::now();
                    }
                }
            }
            ui.separator();
            if let Ok(state) = self.shared.try_lock() {
                let running = state.source.status == crate::source_manager::SourceStatus::Running;
                let status_color = if running { egui::Color32::GREEN } else { egui::Color32::GRAY };
                ui.colored_label(status_color, "●")
                    .on_hover_text(if running { "SDR source is active and streaming samples." } else { "SDR source is stopped. Press Start or Space to begin." });

                ui.separator();
                let true_hz = (state.source.frequency_hz as i64 + state.lo_offset_hz).max(0) as u64;
                let freq_str = if state.lo_offset_hz != 0 {
                    format!("{:.3} MHz (+{:.0}M)", true_hz as f64 / 1e6, state.lo_offset_hz as f64 / 1e6)
                } else {
                    format!("{:.3} MHz", state.source.frequency_hz as f64 / 1e6)
                };
                let mut freq_hint = if state.lo_offset_hz != 0 {
                    format!("True frequency: {:.6} MHz (tuned {:.6} MHz + {} MHz LO offset).", true_hz as f64/1e6, state.source.frequency_hz as f64/1e6, state.lo_offset_hz/1_000_000)
                } else {
                    format!("Tuned frequency: {:.6} MHz.", state.source.frequency_hz as f64/1e6)
                };
                if state.source.ppm_correction != 0 {
                    freq_hint.push_str(&format!(" PPM correction: {} PPM active.", state.source.ppm_correction));
                }
                freq_hint.push_str(" Click to copy. Use arrow keys or SDR panel to change. RTL-SDR range: 24–1766 MHz.");

                let freq_resp = ui.add(egui::Label::new(egui::RichText::new(&freq_str).monospace()
                    .color(if state.lo_offset_hz != 0 || state.source.ppm_correction != 0 { egui::Color32::from_rgb(255, 200, 80) } else { egui::Color32::WHITE }))
                    .sense(egui::Sense::click()))
                    .on_hover_text(freq_hint);
                if freq_resp.clicked() {
                    ui.ctx().copy_text(format!("{:.6}", true_hz as f64 / 1e6));
                }
                ui.separator();
                let sps_mhz = state.source.sample_rate_hz as f64 / 1e6;
                ui.small(format!("{} · {:.1} MSps", state.demod_mode.label(), sps_mhz))
                    .on_hover_text(format!("Demod mode: {}. Sample rate: {:.1} MSps (spectrum width: ±{:.1} MHz). Higher rates = wider view, more CPU.",
                        state.demod_mode.label(), sps_mhz, sps_mhz / 2.0));
                ui.separator();
                ui.small(format!("Gain: {:.1} dB", state.source.gain_db))
                    .on_hover_text("RF gain in dB. Higher = more sensitive, more noise. 30–40 dB typical.");
                if state.recording {
                    if self.recording_start.is_none() {
                        self.recording_start = Some(std::time::Instant::now());
                    }
                    let elapsed = self.recording_start.map(|s| s.elapsed().as_secs()).unwrap_or(0);
                    let rec_label = if elapsed < 60 {
                        format!("● REC {:02}s", elapsed)
                    } else {
                        format!("● REC {:02}:{:02}", elapsed / 60, elapsed % 60)
                    };
                    let rec_type = if self.recorder_panel.record_iq { "IQ" } else { "WAV" };
                    ui.colored_label(egui::Color32::RED, format!("{} [{}]", rec_label, rec_type))
                        .on_hover_text(format!("Recording {} format in progress. Go to the Recorder tab to stop.", rec_type));
                } else {
                    self.recording_start = None;
                }
                if self.audio.has_failed() {
                    ui.colored_label(egui::Color32::RED, "❌ Audio Failed")
                        .on_hover_text("Audio device not found or failed to initialize. Check your audio settings. Press M to retry.");
                } else if self.audio.is_running() {
                    let audio_peak = state.audio_peak;
                    let is_muted = state.volume < 0.01;
                    let (audio_color, audio_label) = if is_muted {
                        (egui::Color32::from_rgb(120, 120, 120), "🔇 Muted")
                    } else if audio_peak > 0.95 {
                        (egui::Color32::from_rgb(231, 76, 60), "🔊 CLIP")
                    } else {
                        (egui::Color32::from_rgb(100, 200, 255), "🔊 Audio")
                    };
                    ui.colored_label(audio_color, audio_label)
                        .on_hover_text(format!(
                            "Audio at {:.0}% level. {}{}",
                            (audio_peak * 100.0).min(100.0),
                            if is_muted { "Press M to unmute audio." } else if audio_peak > 0.95 { "⚠ Clipping — reduce volume in the SDR panel." } else { "Use Vol slider in the SDR panel to adjust." },
                            if is_muted { "" } else { "" }
                        ));
                    // FM deviation indicator (for FM/NFM/WFM modes)
                    if matches!(state.demod_mode, crate::sdr_panel::DemodMode::Fm | crate::sdr_panel::DemodMode::Wfm) {
                        let dev_khz = state.fm_deviation_hz / 1000.0;
                        ui.label(format!("±{:.1}kHz", dev_khz.abs()))
                            .on_hover_text(format!("FM deviation: ±{:.1} kHz. Indicates the frequency span of the modulated signal.", dev_khz.abs()));
                    }
                }
                // Squelch-blocked indicator
                {
                    let signal = state.spectrum.signal_level();
                    let squelch = state.squelch;
                    if signal < squelch && squelch > -90.0 {
                        ui.colored_label(egui::Color32::from_rgb(160, 130, 60), "🔒 SQ")
                            .on_hover_text(format!("Squelch is blocking audio — signal ({:.0} dB) is below squelch threshold ({:.0} dB). Reduce squelch or wait for a stronger signal.", signal, squelch));
                    }
                }
                // S-meter bargraph (signal strength)
                {
                    let signal_db = state.spectrum.signal_level();
                    // Map -120..0 dB to 0..1, clamp
                    let fill = ((signal_db + 120.0) / 120.0).clamp(0.0, 1.0);
                    let bar_w = 60.0f32;
                    let bar_h = 10.0f32;
                    let (bar_rect, bar_resp) = ui.allocate_exact_size(
                        egui::vec2(bar_w, bar_h),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter();
                    // Background
                    painter.rect_filled(bar_rect, 2.0, egui::Color32::from_rgb(30, 30, 40));
                    // Fill bar: red → yellow → green based on level
                    let bar_color = if fill > 0.75 {
                        egui::Color32::from_rgb(46, 204, 113)
                    } else if fill > 0.4 {
                        egui::Color32::from_rgb(241, 196, 15)
                    } else {
                        egui::Color32::from_rgb(231, 76, 60)
                    };
                    let filled = egui::Rect::from_min_size(bar_rect.min, egui::vec2(bar_w * fill, bar_h));
                    painter.rect_filled(filled, 2.0, bar_color);
                    // Border
                    painter.rect_stroke(bar_rect, 2.0, egui::Stroke::new(0.5, egui::Color32::from_gray(80)), egui::StrokeKind::Middle);
                    // S-unit label overlay
                    let s_unit = ((signal_db + 127.0) / 6.0).clamp(0.0, 9.0) as u8;
                    let label = if signal_db > -73.0 { format!("S9+{:.0}", signal_db + 73.0) }
                        else { format!("S{}", s_unit) };
                    painter.text(
                        bar_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        &label,
                        egui::FontId::proportional(7.5),
                        egui::Color32::from_rgba_premultiplied(255, 255, 255, 200),
                    );
                    bar_resp.on_hover_text(format!(
                        "Signal strength: {:.1} dBFS ({}). S-units follow the IARU standard: S1 = -121 dBm, each S-unit is 6 dB.",
                        signal_db, label
                    ));
                }
                // RF clipping detection (spectrum saturation warning)
                {
                    let peak = state.spectrum.peak_level();
                    if peak > -5.0 {
                        ui.separator();
                        ui.colored_label(egui::Color32::from_rgb(220, 100, 80), "⚠️ RF CLIP")
                            .on_hover_text(format!("RF signal saturating! Peak at {:.1} dB — reduce gain or antenna signal level to prevent distortion.", peak));
                    }
                }
                // Source mode badge
                if state.source.source_mode == crate::source_manager::SourceMode::Simulated {
                    let (label, tooltip) = if cfg!(feature = "rtlsdr") {
                        ("RTL-SDR", "Receiving live signals from the connected RTL-SDR device.")
                    } else {
                        ("⚠ DEMO", "Running in simulated (demo) mode — no real SDR device connected. The spectrum shows synthetic test signals. Connect an RTL-SDR and rebuild with the 'rtlsdr' feature, or use File Replay mode.")
                    };
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 180, 50),
                        label,
                    ).on_hover_text(tooltip);
                }
                // MQTT badge
                {
                    let mqtt_connected = self.mqtt.is_connected();
                    let mqtt_enabled = self.mqtt.enabled;
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.mqtt_connected = mqtt_connected;
                        state.mqtt_enabled = mqtt_enabled;
                    }
                }
                if self.mqtt.enabled {
                    ui.separator();
                    if self.mqtt.is_connected() {
                        ui.colored_label(egui::Color32::from_rgb(46, 204, 113), "●")
                            .on_hover_text("Connected to MQTT broker");
                        ui.colored_label(egui::Color32::from_rgb(46, 204, 113), "MQTT")
                            .on_hover_text(format!("Publishing to broker at {}:{} — Topics: {}/signal, {}/scanner, {}/adsb/aircraft, {}/satellite/passes",
                                self.mqtt.broker, self.mqtt.port, self.mqtt.topic_prefix, self.mqtt.topic_prefix, self.mqtt.topic_prefix, self.mqtt.topic_prefix));
                    } else {
                        let secs = self.mqtt.reconnect_in_secs().unwrap_or(0);
                        ui.colored_label(egui::Color32::from_rgb(231, 76, 60), "●")
                            .on_hover_text("Disconnected from MQTT broker");
                        ui.colored_label(egui::Color32::from_rgb(200, 150, 50), format!("MQTT ⏳{}s", secs))
                            .on_hover_text(format!("Connection to {}:{} lost. Auto-reconnect in {}s.", self.mqtt.broker, self.mqtt.port, secs));
                    }
                }
            }
            // Doppler correction status badge (outside the lock, uses satellite_panel data)
            let doppler_hz = self.satellite_panel.doppler_hz;
            let sat_selected = self.satellite_panel.selected_sat.is_some();
            let auto_tune = self.satellite_panel.auto_tune;
            if sat_selected && doppler_hz.abs() > 1.0 {
                ui.separator();
                let doppler_str = if doppler_hz.abs() >= 1000.0 {
                    format!("🛰 {:+.1}kHz", doppler_hz / 1000.0)
                } else {
                    format!("🛰 {:+.0}Hz", doppler_hz)
                };
                let dop_color = if auto_tune {
                    egui::Color32::from_rgb(80, 230, 130)
                } else {
                    egui::Color32::from_rgb(200, 200, 80)
                };
                ui.colored_label(dop_color, &doppler_str)
                    .on_hover_text(if auto_tune {
                        format!("Doppler correction ACTIVE: {:+.1} Hz applied to compensate for satellite motion. Frequency is continuously adjusted. Disable 'Auto-tune' in Satellite panel to stop.", doppler_hz)
                    } else {
                        format!("Doppler shift: {:+.1} Hz — not correcting (auto-tune off). Enable 'Auto-tune to downlink + Doppler' in Satellite panel.", doppler_hz)
                    });
            }
            // Volume slider
            if let Ok(mut state) = self.shared.try_lock() {
                ui.separator();
                ui.small("Vol:").on_hover_text("Quick volume control for audio output.");
                ui.add(egui::Slider::new(&mut state.volume, 0.0..=1.0).text(""))
                    .on_hover_text("Audio output volume. Does not affect RF gain.");
                ui.separator();
                ui.small("Squelch:").on_hover_text("Squelch threshold. Audio mutes when signal drops below this level — silences static during quiet periods.");
                ui.add(egui::Slider::new(&mut state.squelch, -120.0..=0.0).text("dB"))
                    .on_hover_text("Squelch level in dBFS. Set ~5 dB above your noise floor to gate out background hiss between transmissions.");
            }
            if let Ok(state) = self.shared.try_lock() {
                let peak = state.spectrum.peak_level();
                let noise_floor = state.spectrum.noise_floor();
                let snr = peak - noise_floor;
                let (badge, badge_color, badge_tip) = if snr > 20.0 {
                    ("🟢 Signal", egui::Color32::GREEN,    "Strong signal (SNR > 20 dB). Good reception.")
                } else if snr > 8.0 {
                    ("🟡 Weak",   egui::Color32::YELLOW,   "Weak signal (SNR 8–20 dB). May be readable.")
                } else {
                    ("⚫ Quiet",  egui::Color32::DARK_GRAY, "No signal (SNR < 8 dB). Try a different frequency or increase gain.")
                };
                ui.colored_label(badge_color, badge)
                    .on_hover_text(format!("{} Peak: {:.0} dBFS · Floor: {:.0} dBFS · SNR: {:.0} dB", badge_tip, peak, noise_floor, snr));
            }
            // Status flash (short-lived messages, e.g. "⭐ Bookmark name")
            if let Some((msg, since)) = &self.status_flash {
                let duration = if msg.starts_with("🎉") { 8.0f32 } else { 3.0f32 };
                if since.elapsed().as_secs_f32() < duration {
                    ui.separator();
                    let alpha = ((duration - since.elapsed().as_secs_f32()) / duration * 255.0) as u8;
                    let color = if msg.starts_with("🎉") {
                        egui::Color32::from_rgba_unmultiplied(100, 255, 150, alpha)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(220, 200, 80, alpha)
                    };
                    ui.colored_label(color, msg);
                } else {
                    self.status_flash = None;
                }
            }
        });

        // Glossary button in status bar trailing area
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("❓ Glossary").on_hover_text("SDR term glossary — click to open/close definitions for dBFS, SNR, MSps, LPF, PPM, VFO, BW, squelch, and more.").clicked() {
                self.show_glossary = !self.show_glossary;
            }
        });

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
            if !open { self.show_glossary = false; }
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
            cfg.recent_frequencies = state.freq_history.iter().cloned().collect();
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
            // Save tutorial state for resume support
            if self.tutorial.active {
                cfg.tutorial_step = if self.tutorial.level_chosen { self.tutorial.step } else { 0 };
                if self.tutorial.level_chosen {
                    cfg.user_level = self.tutorial.level.to_str().to_string();
                }
            }
            cfg.save();
        }
    }
}

struct SharedSnapshot {
    jobs: Vec<crate::scheduler::ScheduledJob>,
    custom_tasks: Vec<crate::scheduler::CustomTask>,
    auto_tune_enabled: bool,
}


// ── New 3-tab render methods ──────────────────────────────────────────────────

impl CentralApp {
    fn render_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(13, 17, 23));
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            for (tab, label, tip) in [
                (AppTab::Sdr,       "📻 SDR",       "Spectrum, tuning, demodulation"),
                (AppTab::AdsB,      "✈ ADS-B",      "Aircraft tracking — 1090 MHz decoder"),
                (AppTab::Satellite, "🛰 Satellite",  "Pass predictions, Doppler correction"),
                (AppTab::Ai,        "🤖 AI",         "AI assistant — ask questions, run tools"),
            ] {
                let is_active = self.current_tab == tab;
                let fg = if is_active { egui::Color32::from_rgb(0, 168, 255) } else { egui::Color32::from_rgb(155, 165, 175) };
                let bg = if is_active { egui::Color32::from_rgb(20, 26, 34) } else { egui::Color32::TRANSPARENT };
                let btn = egui::Button::new(egui::RichText::new(label).color(fg).size(13.5))
                    .fill(bg)
                    .min_size(egui::vec2(112.0, 36.0));
                let resp = ui.add(btn).on_hover_text(tip);
                if is_active {
                    let r = resp.rect;
                    ui.painter().line_segment(
                        [egui::pos2(r.left() + 6.0, r.bottom()), egui::pos2(r.right() - 6.0, r.bottom())],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 168, 255)),
                    );
                }
                if resp.clicked() { self.current_tab = tab; }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                if let Ok(state) = self.shared.try_lock() {
                    let (dot, dot_color) = match &state.source.status {
                        crate::source_manager::SourceStatus::Running => ("●", egui::Color32::from_rgb(80, 220, 80)),
                        crate::source_manager::SourceStatus::Error(_) => ("●", egui::Color32::RED),
                        crate::source_manager::SourceStatus::Opening => ("●", egui::Color32::YELLOW),
                        _ => ("○", egui::Color32::DARK_GRAY),
                    };
                    ui.colored_label(dot_color, dot);
                    if state.recording {
                        ui.separator();
                        ui.colored_label(egui::Color32::RED, "● REC");
                    }
                    if self.current_tab == AppTab::Sdr {
                        ui.separator();
                        let freq_mhz = state.source.frequency_hz as f64 / 1e6;
                        ui.monospace(egui::RichText::new(format!("{:.4} MHz", freq_mhz))
                            .size(15.0).color(egui::Color32::from_rgb(0, 168, 255)));
                    }
                }
            });
        });
    }

    fn render_secondary_rail(&mut self, ui: &mut egui::Ui) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(10, 13, 18));
        ui.vertical_centered(|ui| {
            ui.add_space(6.0);
            for tool in [
                SecondaryTool::Bookmarks,
                SecondaryTool::Scanner,
                SecondaryTool::Recorder,
                SecondaryTool::Scheduler,
                SecondaryTool::Discord,
                SecondaryTool::HowTo,
                SecondaryTool::Settings,
            ] {
                let is_active = self.active_secondary_tool == Some(tool);
                let fg = if is_active { egui::Color32::from_rgb(0, 168, 255) } else { egui::Color32::from_rgb(140, 150, 160) };
                let bg = if is_active { egui::Color32::from_rgb(20, 26, 34) } else { egui::Color32::TRANSPARENT };
                let btn = egui::Button::new(egui::RichText::new(tool.icon()).color(fg).size(17.0))
                    .fill(bg)
                    .min_size(egui::vec2(36.0, 36.0));
                if ui.add(btn).on_hover_text(tool.label()).clicked() {
                    self.active_secondary_tool = if is_active { None } else { Some(tool) };
                }
                ui.add_space(4.0);
            }
        });
    }

    fn render_secondary_panel(&mut self, ui: &mut egui::Ui, tool: SecondaryTool, snapshot: &Option<SharedSnapshot>) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} {}", tool.icon(), tool.label())).strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("Close panel").clicked() {
                    self.active_secondary_tool = None;
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("secondary_panel_scroll").show(ui, |ui| {
            match tool {
                SecondaryTool::Bookmarks => self.render_bookmarks_full(ui),
                SecondaryTool::Scheduler => self.render_scheduler_full(ui, snapshot),
                SecondaryTool::Settings => {
                    if let Ok(mut state) = self.shared.try_lock() { state.config.ui(ui); }
                }
                SecondaryTool::Scanner => {
                    if let Ok(state) = self.shared.try_lock() {
                        self.scanner.spectrum_visible_range = Some((state.spectrum.visible_left_hz, state.spectrum.visible_right_hz));
                    }
                    self.scanner.ui(ui);
                    if let Some(prompt) = self.scanner.pending_ai_prompt.take() {
                        self.ai_panel.input = prompt;
                    }
                }
                SecondaryTool::Recorder => self.recorder_panel.ui(ui),
                SecondaryTool::HowTo => self.howto_panel.ui(ui),
                SecondaryTool::Discord => self.discord_panel.ui(ui, &mut self.discord, &self.shared),
            }
        });
    }

    fn render_ai_tab(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.ai_panel.ui(ui);
        });
    }

    fn render_sdr_tab(&mut self, ui: &mut egui::Ui, snapshot: &Option<SharedSnapshot>) {
        egui::Panel::bottom("sdr_status_bar")
            .exact_size(32.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Ok(state) = self.shared.try_lock() {
                        let sig = state.spectrum.signal_level();
                        let sig_color = if sig > -60.0 { egui::Color32::GREEN } else if sig > -100.0 { egui::Color32::YELLOW } else { egui::Color32::RED };
                        ui.colored_label(sig_color, format!("📶 {:.1} dB", sig));
                        ui.separator();
                        let status = match &state.source.status {
                            crate::source_manager::SourceStatus::Idle => "Idle".to_string(),
                            crate::source_manager::SourceStatus::Opening => "Opening…".to_string(),
                            crate::source_manager::SourceStatus::Running => "🟢 Running".to_string(),
                            crate::source_manager::SourceStatus::Error(e) => format!("⚠ {}", e.chars().take(28).collect::<String>()),
                        };
                        ui.label(status);

                        // Satellite countdown
                        let now_unix = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                        if let Some(job) = state.scheduler.jobs.iter().find(|j| now_unix >= j.aos_dt && now_unix <= j.los_dt) {
                            ui.separator();
                            let rem = (job.los_dt - now_unix).max(0.0) as u64;
                            ui.colored_label(egui::Color32::from_rgb(50, 255, 100),
                                format!("🛰 {} IN PASS {:02}:{:02}", job.satellite, rem / 60, rem % 60));
                        } else if let Some(job) = state.scheduler.jobs.iter().filter(|j| j.aos_dt > now_unix)
                            .min_by(|a, b| a.aos_dt.partial_cmp(&b.aos_dt).unwrap_or(std::cmp::Ordering::Equal))
                        {
                            let secs = (job.aos_dt - now_unix) as u64;
                            ui.separator();
                            ui.colored_label(egui::Color32::GRAY, format!("🛰 {} in {}m", job.satellite, secs / 60));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ai_fg = if self.sdr_ai_panel_open { egui::Color32::from_rgb(0, 168, 255) } else { egui::Color32::GRAY };
                        if ui.add(egui::Button::new(egui::RichText::new("🤖 AI").color(ai_fg))).on_hover_text("Toggle AI assistant panel").clicked() {
                            self.sdr_ai_panel_open = !self.sdr_ai_panel_open;
                        }
                        ui.separator();
                        if ui.small_button("▶").clicked() {
                            if let Ok(state) = self.shared.try_lock() {
                                let len = state.freq_history.len();
                                if let Some(idx) = self.freq_history_idx {
                                    let next = if idx + 1 < len { Some(idx + 1) } else { None };
                                    let target = next.unwrap_or(len.saturating_sub(1));
                                    let freq = state.freq_history.iter().nth(target).copied();
                                    drop(state);
                                    self.freq_history_idx = next;
                                    if let (Some(f), Ok(mut s)) = (freq, self.shared.try_lock()) { s.source.frequency_hz = f; }
                                }
                            }
                        }
                        if ui.small_button("◀").clicked() {
                            if let Ok(state) = self.shared.try_lock() {
                                let len = state.freq_history.len();
                                if len > 1 {
                                    let new_idx = self.freq_history_idx.map(|i| i.saturating_sub(1)).unwrap_or(len.saturating_sub(2));
                                    let freq = state.freq_history.iter().nth(new_idx).copied();
                                    drop(state);
                                    self.freq_history_idx = Some(new_idx);
                                    if let (Some(f), Ok(mut s)) = (freq, self.shared.try_lock()) { s.source.frequency_hz = f; }
                                }
                            }
                        }
                    });
                });
            });

        let _ = snapshot;

        if self.sdr_ai_panel_open {
            egui::Panel::right("sdr_ai_panel")
                .resizable(true)
                .default_size(320.0)
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("🤖 AI Assistant").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").clicked() { self.sdr_ai_panel_open = false; }
                        });
                    });
                    ui.separator();
                    self.ai_panel.ui(ui);
                });
        }

        egui::Panel::left("sdr_modules")
            .resizable(true)
            .default_size(280.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.sdr_panel.ui_source(ui);
                    if let Some(freq) = self.sdr_panel.tune_request.take() {
                        if let Ok(mut state) = self.shared.try_lock() {
                            state.source.frequency_hz = freq;
                            self.last_manual_tune_time = std::time::Instant::now();
                        }
                    }
                    if let Some(msg) = self.sdr_panel.pending_status.take() {
                        self.status_flash = Some((msg, std::time::Instant::now()));
                    }
                });
            });

        egui::Panel::right("sdr_demod")
            .resizable(true)
            .default_size(260.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.sdr_panel.ui_demod(ui);
                    if let Some(msg) = self.sdr_panel.pending_status.take() {
                        self.status_flash = Some((msg, std::time::Instant::now()));
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
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
                    self.status_flash = Some((format!("🔷 VFO B → {:.4} MHz", freq as f64 / 1e6), std::time::Instant::now()));
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
                if let Some(hz) = state.spectrum.pending_scan_start.take() { self.scanner.start_hz = hz; }
                if let Some(hz) = state.spectrum.pending_scan_stop.take() { self.scanner.stop_hz = hz; }
                if let Some(mode_str) = state.spectrum.pending_demod_mode.take() {
                    if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&mode_str) { state.demod_mode = mode; }
                }
                if state.spectrum.pending_start_source { state.spectrum.pending_start_source = false; state.source.start(); }
                if let Some(freq) = state.spectrum.pending_ai_freq.take() {
                    let freq_mhz = freq as f64 / 1e6;
                    self.ai_panel.input = format!("I'm looking at {:.4} MHz on the spectrum. What signals might be here? What demod mode?", freq_mhz);
                    self.status_flash = Some((format!("🤖 AI prompt for {:.3} MHz", freq_mhz), std::time::Instant::now()));
                }
            }
        });

        if let Some(freq) = self.sdr_panel.pending_ai_freq.take() {
            if let Ok(state) = self.shared.try_lock() {
                let snr = state.spectrum.peak_level() - state.spectrum.noise_floor();
                self.ai_panel.input = format!(
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
            .show_inside(ui, |ui| {
                self.adsb_panel.ui_list(ui);
                if let Some(prompt) = self.adsb_panel.pending_ai_prompt.take() {
                    self.ai_panel.input = prompt;
                    self.status_flash = Some(("🤖 Aircraft details sent to AI".to_string(), std::time::Instant::now()));
                }
            });
        // Map dominates the tab; on-page receive instructions live in a collapsed
        // banner so the map stays the primary focus by default.
        egui::Panel::top("adsb_instructions")
            .resizable(false)
            .exact_size(if self.adsb_instructions_open { 260.0 } else { 24.0 })
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let arrow = if self.adsb_instructions_open { "▼" } else { "▶" };
                    if ui.small_button(format!("{arrow} 📡 How to receive ADS-B (antenna, 1090 MHz setup)"))
                        .on_hover_text("Show/hide setup instructions for receiving live aircraft data")
                        .clicked()
                    {
                        self.adsb_instructions_open = !self.adsb_instructions_open;
                    }
                });
                if self.adsb_instructions_open {
                    egui::ScrollArea::vertical().id_salt("adsb_instructions_scroll").show(ui, |ui| {
                        self.adsb_panel.ui_antenna_guide(ui);
                    });
                }
            });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.adsb_panel.ui_map(ui);
        });
    }

    fn render_satellite_tab(&mut self, ui: &mut egui::Ui, snapshot: &Option<SharedSnapshot>) {
        let _ = snapshot;
        egui::Panel::top("sat_subtabs")
            .exact_size(36.0)
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    for (advanced, label) in [(false, "🛰 Track"), (true, "⚙ Advanced")] {
                        let is_active = self.satellite_advanced == advanced;
                        let fg = if is_active { egui::Color32::from_rgb(0, 168, 255) } else { egui::Color32::GRAY };
                        if ui.add(egui::Button::new(egui::RichText::new(label).color(fg)).fill(egui::Color32::TRANSPARENT)).clicked() {
                            self.satellite_advanced = advanced;
                        }
                    }
                });
            });

        egui::Panel::bottom("sat_status")
            .exact_size(48.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let doppler = self.satellite_panel.doppler_hz;
                    let dop_color = if doppler.abs() > 5000.0 { egui::Color32::from_rgb(255, 120, 60) }
                        else if doppler.abs() > 1000.0 { egui::Color32::YELLOW }
                        else { egui::Color32::from_rgb(120, 220, 120) };
                    ui.colored_label(dop_color, format!("Doppler: {:+.2} kHz", doppler / 1000.0));
                    ui.separator();
                    ui.label(format!("Observer: {:.4}°N  {:.4}°E",
                        self.satellite_panel.observer_lat, self.satellite_panel.observer_lon));
                    if self.satellite_panel.auto_tune {
                        ui.separator();
                        ui.colored_label(egui::Color32::GREEN, "✓ Auto-tune");
                    }
                    if self.satellite_panel.recording {
                        ui.separator();
                        ui.colored_label(egui::Color32::RED, "● REC");
                    }
                });
            });

        egui::Panel::right("sat_pipeline")
            .resizable(true)
            .default_size(280.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.satellite_advanced {
                        self.satellite_panel.ui_advanced(ui);
                    } else {
                        self.satellite_panel.ui_simple(ui);
                    }
                    if let Some(prompt) = self.satellite_panel.pending_ai_prompt.take() {
                        self.ai_panel.input = prompt;
                        self.status_flash = Some(("🤖 Satellite details sent to AI".to_string(), std::time::Instant::now()));
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_satellite_world_map(ui);
        });
    }

    fn render_satellite_world_map(&mut self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(8, 14, 24));

        for lat_step in 0..=6i32 {
            let lat = -90.0 + lat_step as f32 * 30.0;
            let y = rect.top() + ((90.0 - lat) / 180.0) * rect.height();
            painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(0.4, egui::Color32::from_rgb(28, 42, 60)));
        }
        for lon_step in 0..=12i32 {
            let lon = -180.0 + lon_step as f32 * 30.0;
            let x = rect.left() + ((lon + 180.0) / 360.0) * rect.width();
            painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(0.4, egui::Color32::from_rgb(28, 42, 60)));
        }
        for lat in [-60i32, -30, 0, 30, 60] {
            let y = rect.top() + ((90.0 - lat as f32) / 180.0) * rect.height();
            painter.text(egui::pos2(rect.left() + 5.0, y), egui::Align2::LEFT_CENTER,
                format!("{}°", lat), egui::FontId::proportional(8.0), egui::Color32::from_gray(50));
        }

        let obs_x = rect.left() + ((self.satellite_panel.observer_lon as f32 + 180.0) / 360.0) * rect.width();
        let obs_y = rect.top() + ((90.0 - self.satellite_panel.observer_lat as f32) / 180.0) * rect.height();
        if rect.contains(egui::pos2(obs_x, obs_y)) {
            painter.line_segment([egui::pos2(obs_x - 9.0, obs_y), egui::pos2(obs_x + 9.0, obs_y)],
                egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 240, 80)));
            painter.line_segment([egui::pos2(obs_x, obs_y - 9.0), egui::pos2(obs_x, obs_y + 9.0)],
                egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 240, 80)));
            painter.circle_stroke(egui::pos2(obs_x, obs_y), 5.0,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 240, 80)));
            painter.text(egui::pos2(obs_x + 11.0, obs_y - 7.0), egui::Align2::LEFT_CENTER,
                "Observer", egui::FontId::proportional(9.0), egui::Color32::from_rgb(210, 200, 70));
        }

        painter.text(egui::pos2(rect.left() + 8.0, rect.bottom() - 8.0), egui::Align2::LEFT_BOTTOM,
            "Select a satellite in the panel to track its ground path",
            egui::FontId::proportional(9.0), egui::Color32::from_gray(45));
    }

    fn render_bookmarks_full(&mut self, ui: &mut egui::Ui) {
        let bm_count = if let Ok(state) = self.shared.try_lock() { state.bookmarks.bookmarks.len() } else { 0 };
        ui.heading("Frequency Bookmarks");
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{} bookmarks", bm_count));
            ui.add(egui::TextEdit::singleline(&mut self.bookmark_filter).hint_text("Filter...").desired_width(150.0));
            ui.toggle_value(&mut self.show_starred_only, "⭐ Starred").on_hover_text("Show only starred (favorite) bookmarks");
            if ui.button("💾 Save").on_hover_text("Save all bookmarks to ez_sdr_bookmarks.json in the current directory.").clicked() {
                if let Ok(state) = self.shared.try_lock() {
                    state.bookmarks.save();
                }
            }
            if ui.button("📂 Load").on_hover_text("Reload bookmarks from ez_sdr_bookmarks.json, replacing the current list.").clicked() {
                if let Ok(mut state) = self.shared.try_lock() {
                    if let Some(loaded) = crate::bookmarks::BookmarkDb::load_saved() {
                        state.bookmarks.bookmarks = loaded;
                    }
                }
            }
            if ui.button("📥 Import CSV").on_hover_text("Import bookmarks from a CSV file (columns: name,frequency_hz,mode,category). Appends to current list.").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).pick_file() {
                    if let Some(path_str) = path.to_str() {
                        if let Ok(mut state) = self.shared.try_lock() {
                            let (count, err) = state.bookmarks.import_csv(path_str);
                            if err.is_empty() {
                                self.bm_import_msg = format!("Imported {} bookmarks.", count);
                            } else {
                                self.bm_import_msg = err;
                            }
                        }
                    }
                }
            }
            if ui.button("📤 Export CSV").on_hover_text("Export all bookmarks to a timestamped CSV file in the current directory.").clicked() {
                if let Ok(state) = self.shared.try_lock() {
                    let (path, err) = state.bookmarks.export_csv();
                    if err.is_empty() {
                        self.bm_import_msg = format!("Exported to {}", path);
                    } else {
                        self.bm_import_msg = err;
                    }
                }
            }
            if ui.small_button("A→Z").on_hover_text("Sort all bookmarks alphabetically by name within each category.").clicked() {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.bookmarks.bookmarks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
            if ui.small_button("Hz↑").on_hover_text("Sort all bookmarks by frequency (lowest first) within each category.").clicked() {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.bookmarks.bookmarks.sort_by_key(|b| b.frequency_hz);
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
            if ui.button(if self.show_add_bm { "✕ Cancel" } else { "+ Add" })
                .on_hover_text("Add a new bookmark for the current or any frequency.")
                .clicked()
            {
                self.show_add_bm = !self.show_add_bm;
                self.new_bm_error.clear();
                if self.show_add_bm {
                    if let Ok(state) = self.shared.try_lock() {
                        self.new_bm_freq_mhz = format!("{:.4}", state.source.frequency_hz as f64 / 1e6);
                        self.new_bm_mode = state.demod_mode.label().to_string();
                    }
                }
            }
        });

        // Add bookmark form
        if self.show_add_bm {
            ui.group(|ui| {
                ui.label(egui::RichText::new("New Bookmark").strong());
                egui::Grid::new("add_bm_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Name:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_bm_name).desired_width(200.0).hint_text("e.g. Local Police"));
                    ui.end_row();
                    ui.label("Freq (MHz):");
                    ui.add(egui::TextEdit::singleline(&mut self.new_bm_freq_mhz).desired_width(120.0).hint_text("145.5"));
                    ui.end_row();
                    ui.label("Mode:");
                    egui::ComboBox::from_id_salt("bm_mode_combo")
                        .selected_text(self.new_bm_mode.as_str())
                        .width(ui.available_width().min(100.0))
                        .show_ui(ui, |ui| {
                            for m in ["NFM", "WFM", "AM", "USB", "LSB", "RAW"] {
                                ui.selectable_value(&mut self.new_bm_mode, m.to_string(), m);
                            }
                        });
                    ui.end_row();
                    ui.label("Category:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_bm_category).desired_width(150.0).hint_text("Custom"));
                    ui.end_row();
                    ui.label("Notes:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_bm_notes).desired_width(250.0).hint_text("Optional notes about this signal"));
                    ui.end_row();
                });
                if !self.new_bm_error.is_empty() {
                    ui.colored_label(egui::Color32::RED, self.new_bm_error.as_str());
                }
                if ui.button("Save Bookmark").clicked() {
                    let name = self.new_bm_name.trim().to_string();
                    let freq_str = self.new_bm_freq_mhz.trim().to_string();
                    match freq_str.parse::<f64>() {
                        Ok(mhz) if mhz > 0.0 && !name.is_empty() => {
                            let bm = crate::bookmarks::Bookmark {
                                name,
                                frequency_hz: (mhz * 1e6) as u64,
                                mode: self.new_bm_mode.clone(),
                                bandwidth_hz: 12_500,
                                category: if self.new_bm_category.trim().is_empty() { "Custom".to_string() } else { self.new_bm_category.trim().to_string() },
                                notes: self.new_bm_notes.trim().to_string(),
                                starred: false,
                            };
                            if let Ok(mut state) = self.shared.try_lock() {
                                state.bookmarks.bookmarks.push(bm);
                                state.bookmarks_modified = true;
                                state.spectrum.bookmark_freqs_dirty = true;
                            }
                            self.new_bm_name.clear();
                            self.new_bm_notes.clear();
                            self.new_bm_error.clear();
                            self.show_add_bm = false;
                        }
                        Ok(_) => self.new_bm_error = "Frequency must be > 0 MHz".to_string(),
                        Err(_) if name.is_empty() => self.new_bm_error = "Name cannot be empty".to_string(),
                        Err(_) => self.new_bm_error = "Invalid frequency — enter a number like 145.5".to_string(),
                    }
                }
            });
        }

        if !self.bm_import_msg.is_empty() {
            ui.colored_label(egui::Color32::from_rgb(100, 220, 100), self.bm_import_msg.as_str());
        }

        ui.separator();

        let filter_lower = self.bookmark_filter.to_lowercase();
        let (bookmarks_snapshot, total) = if let Ok(state) = self.shared.try_lock() {
            (state.bookmarks.bookmarks.clone(), state.bookmarks.bookmarks.len())
        } else {
            return;
        };
        let _ = total;

        // Category quick-filter chips
        {
            let mut all_cats: Vec<String> = bookmarks_snapshot.iter().map(|b| b.category.clone()).collect();
            all_cats.sort();
            all_cats.dedup();
            if all_cats.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    ui.small("Filter by:");
                    for cat in &all_cats {
                        let is_active = filter_lower == cat.to_lowercase();
                        let btn = ui.add(egui::Button::new(
                            egui::RichText::new(cat.as_str()).small()
                                .color(if is_active { egui::Color32::BLACK } else { egui::Color32::from_rgb(180, 200, 240) })
                        ).fill(if is_active { egui::Color32::from_rgb(80, 160, 255) } else { egui::Color32::from_rgba_premultiplied(40, 60, 100, 80) })
                        .small())
                        .on_hover_text(format!("Click to filter by '{}' category. Click again to clear.", cat));
                        if btn.clicked() {
                            if is_active {
                                self.bookmark_filter.clear();
                            } else {
                                self.bookmark_filter = cat.clone();
                            }
                        }
                    }
                    if !filter_lower.is_empty() {
                        if ui.small_button("✕ Clear").clicked() {
                            self.bookmark_filter.clear();
                        }
                    }
                });
            }
        }

        let filtered: Vec<(usize, &crate::bookmarks::Bookmark)> = bookmarks_snapshot.iter()
            .enumerate()
            .filter(|(_, b)| {
                let matches_starred = !self.show_starred_only || b.starred;
                let matches_text = filter_lower.is_empty()
                    || b.name.to_lowercase().contains(&filter_lower)
                    || b.category.to_lowercase().contains(&filter_lower)
                    || b.mode.to_lowercase().contains(&filter_lower)
                    || b.freq_display().contains(&filter_lower);
                matches_starred && matches_text
            })
            .collect();

        let mut categories: Vec<String> = filtered.iter().map(|(_, b)| b.category.clone()).collect();
        categories.sort();
        categories.dedup();

        let mut delete_idx: Option<usize> = None;
        let mut duplicate_idx: Option<usize> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for cat in &categories {
                let cat_count = filtered.iter().filter(|(_, b)| &b.category == cat).count();
                let cat_header = format!("{} ({})", cat, cat_count);
                ui.collapsing(cat_header, |ui| {
                    for (orig_idx, bm) in filtered.iter().filter(|(_, b)| &b.category == cat) {
                        let is_editing = self.edit_bm_idx == Some(*orig_idx);
                        let row_response = ui.horizontal(|ui| {
                            if is_editing {
                                // Inline edit row
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_name).desired_width(120.0).hint_text("Name"));
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_freq_mhz).desired_width(70.0).hint_text("MHz"));
                                egui::ComboBox::from_id_salt(format!("edit_mode_{}", orig_idx))
                                    .selected_text(self.edit_bm_mode.as_str())
                                    .width(ui.available_width().min(80.0))
                                    .show_ui(ui, |ui| {
                                        for m in ["NFM", "WFM", "AM", "USB", "LSB", "RAW"] {
                                            ui.selectable_value(&mut self.edit_bm_mode, m.to_string(), m);
                                        }
                                    });
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_category).desired_width(80.0).hint_text("Category"));
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_notes).desired_width(120.0).hint_text("Notes"));
                                if ui.small_button("✓").on_hover_text("Save changes").clicked() {
                                    if let Ok(freq_mhz) = self.edit_bm_freq_mhz.trim().parse::<f64>() {
                                        if let Ok(mut state) = self.shared.try_lock() {
                                            if let Some(bm) = state.bookmarks.bookmarks.get_mut(*orig_idx) {
                                                bm.name = self.edit_bm_name.trim().to_string();
                                                bm.frequency_hz = (freq_mhz * 1e6) as u64;
                                                bm.mode = self.edit_bm_mode.clone();
                                                bm.category = if self.edit_bm_category.trim().is_empty() { "Custom".into() } else { self.edit_bm_category.trim().to_string() };
                                                bm.notes = self.edit_bm_notes.trim().to_string();
                                            }
                                        }
                                    }
                                    self.edit_bm_idx = None;
                                }
                                if ui.small_button("✕").on_hover_text("Cancel edit").clicked() {
                                    self.edit_bm_idx = None;
                                }
                            } else {
                                if *orig_idx < 9 {
                                    ui.colored_label(egui::Color32::from_rgb(100, 180, 255), format!("[{}]", orig_idx + 1))
                                        .on_hover_text(format!("Press {} to tune here instantly", orig_idx + 1));
                                }
                                ui.label(&bm.name);
                                ui.monospace(bm.freq_display());
                                ui.small(&bm.mode);
                                let tune_tip = if bm.notes.is_empty() {
                                    format!("Double-click or click 'Tune' to tune to {} in {} mode", bm.freq_display(), bm.mode)
                                } else {
                                    format!("Double-click or click 'Tune' to tune to {} in {} mode\n{}", bm.freq_display(), bm.mode, bm.notes)
                                };
                                if ui.small_button("Tune")
                                    .on_hover_text(&tune_tip)
                                    .clicked()
                                {
                                    if let Ok(mut state) = self.shared.try_lock() {
                                        state.source.frequency_hz = bm.frequency_hz;
                                        self.last_manual_tune_time = std::time::Instant::now();
                                        if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&bm.mode) {
                                            state.demod_mode = mode;
                                        }
                                    }
                                }
                                if ui.small_button("✏")
                                    .on_hover_text("Edit this bookmark")
                                    .clicked()
                                {
                                    self.edit_bm_idx = Some(*orig_idx);
                                    self.edit_bm_name = bm.name.clone();
                                    self.edit_bm_freq_mhz = format!("{:.4}", bm.frequency_hz as f64 / 1e6);
                                    self.edit_bm_mode = bm.mode.clone();
                                    self.edit_bm_category = bm.category.clone();
                                    self.edit_bm_notes = bm.notes.clone();
                                }
                                let star_icon = if bm.starred { "⭐" } else { "☆" };
                                if ui.small_button(star_icon)
                                    .on_hover_text(if bm.starred { "Remove from favorites" } else { "Add to favorites" })
                                    .clicked()
                                {
                                    if let Ok(mut state) = self.shared.try_lock() {
                                        if let Some(bookmark) = state.bookmarks.bookmarks.get_mut(*orig_idx) {
                                            bookmark.starred = !bookmark.starred;
                                        }
                                    }
                                }
                                if ui.small_button("📋")
                                    .on_hover_text(format!("Copy {} to clipboard", bm.freq_display()))
                                    .clicked()
                                {
                                    ui.ctx().copy_text(bm.freq_display());
                                }
                                if ui.small_button("⧉")
                                    .on_hover_text("Duplicate this bookmark")
                                    .clicked()
                                {
                                    duplicate_idx = Some(*orig_idx);
                                }
                                if ui.small_button("🗑")
                                    .on_hover_text("Delete this bookmark")
                                    .clicked()
                                {
                                    delete_idx = Some(*orig_idx);
                                }
                                let bm_freq_mhz = bm.frequency_hz as f64 / 1e6;
                                if ui.small_button("🤖")
                                    .on_hover_text(format!("Ask AI about {}", bm.freq_display()))
                                    .clicked()
                                {
                                    self.ai_panel.input = format!(
                                        "Tell me about the bookmark \"{}\": {:.4} MHz ({} mode). \
                                         What signals should I expect here, and what are the best settings?",
                                        bm.name, bm_freq_mhz, bm.mode
                                    );
                                    self.status_flash = Some((
                                        format!("🤖 AI prompt ready for {} — switch to the AI tab", bm.freq_display()),
                                        std::time::Instant::now(),
                                    ));
                                }
                            }
                        });
                        // Double-click to tune
                        if !is_editing && row_response.response.double_clicked() {
                            if let Ok(mut state) = self.shared.try_lock() {
                                state.source.frequency_hz = bm.frequency_hz;
                                if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&bm.mode) {
                                    state.demod_mode = mode;
                                }
                            }
                        }
                    }
                });
            }
        });
        if let Some(idx) = delete_idx {
            if let Ok(mut state) = self.shared.try_lock() {
                if idx < state.bookmarks.bookmarks.len() {
                    state.bookmarks.bookmarks.remove(idx);
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
        }
        if let Some(idx) = duplicate_idx {
            if let Ok(mut state) = self.shared.try_lock() {
                if let Some(bm) = state.bookmarks.bookmarks.get(idx).cloned() {
                    let mut copy = bm;
                    copy.name = format!("{} (copy)", copy.name);
                    state.bookmarks.bookmarks.push(copy);
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
        }
    }

    fn render_scheduler_full(&mut self, ui: &mut egui::Ui, snapshot: &Option<SharedSnapshot>) {
        let (jobs, custom_tasks, auto_tune) = match snapshot {
            Some(s) => (s.jobs.clone(), s.custom_tasks.clone(), s.auto_tune_enabled),
            None => return,
        };
        ui.heading("Scheduler");

        // Next event countdown summary
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let next_pass = jobs.first();
        let next_task = custom_tasks.iter().filter(|t| !t.fired && t.at_unix > now_unix)
            .min_by(|a, b| a.at_unix.partial_cmp(&b.at_unix).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(job) = next_pass {
            ui.colored_label(
                egui::Color32::from_rgb(100, 180, 255),
                format!("Next pass: {} at {} ({})", job.satellite, job.aos, job.los),
            ).on_hover_text("Next satellite pass scheduled. Enable Auto-tune below to tune automatically.");
        } else if next_task.is_none() {
            ui.colored_label(egui::Color32::GRAY, "No upcoming events. Add a custom task below or update TLE data in the Satellite tab.");
        }
        if let Some(task) = next_task {
            let secs = (task.at_unix - now_unix).max(0.0) as u64;
            let countdown = if secs < 60 { format!("{}s", secs) } else { format!("{}m {}s", secs / 60, secs % 60) };
            ui.colored_label(
                egui::Color32::from_rgb(241, 196, 15),
                format!("Next task: '{}' at {:.3} MHz — fires in {}", task.label, task.frequency_hz as f64 / 1e6, countdown),
            );
        }
        ui.add_space(4.0);

        // Auto-tune toggle
        let mut auto = auto_tune;
        if ui.checkbox(&mut auto, "Auto-tune to satellite passes")
            .on_hover_text("When enabled, the SDR automatically tunes to the frequency of any satellite currently overhead.")
            .changed()
        {
            if let Ok(mut state) = self.shared.try_lock() {
                state.scheduler.auto_tune_enabled = auto;
            }
        }

        ui.separator();
        ui.label(egui::RichText::new("Upcoming Satellite Passes").strong());

        // Visual timeline: 24-hour bar with pass blocks
        {
            let tl_h = 28.0;
            let (tl_rect, tl_resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), tl_h),
                egui::Sense::hover(),
            );
            let painter = ui.painter();
            painter.rect_filled(tl_rect, 2.0, egui::Color32::from_rgb(8, 8, 18));

            let today_start = (now_unix as u64).saturating_sub((now_unix as u64) % 86400) as f64;
            let day_span = 86400.0f64;

            // Hour marks
            for h in 0..=24 {
                let frac = h as f32 / 24.0;
                let x = tl_rect.left() + frac * tl_rect.width();
                painter.line_segment(
                    [egui::pos2(x, tl_rect.top()), egui::pos2(x, tl_rect.bottom())],
                    egui::Stroke::new(0.4, egui::Color32::from_rgba_premultiplied(60, 60, 80, 100)),
                );
                if h % 4 == 0 {
                    painter.text(
                        egui::pos2(x + 2.0, tl_rect.top() + 1.0),
                        egui::Align2::LEFT_TOP,
                        format!("{:02}", h),
                        egui::FontId::proportional(7.0),
                        egui::Color32::from_gray(90),
                    );
                }
            }

            // Pass blocks (color-coded by satellite index)
            let pass_colors = [
                egui::Color32::from_rgb(52, 152, 219),
                egui::Color32::from_rgb(46, 204, 113),
                egui::Color32::from_rgb(241, 196, 15),
                egui::Color32::from_rgb(155, 89, 182),
                egui::Color32::from_rgb(231, 76, 60),
            ];
            let hover_x = tl_resp.hover_pos().map(|p| p.x);
            let mut hovered_job: Option<&crate::scheduler::ScheduledJob> = None;
            for (i, job) in jobs.iter().enumerate() {
                let aos_frac = ((job.aos_dt - today_start) / day_span).clamp(0.0, 1.0) as f32;
                let los_frac = ((job.los_dt - today_start) / day_span).clamp(0.0, 1.0) as f32;
                if los_frac <= aos_frac { continue; }
                let x1 = tl_rect.left() + aos_frac * tl_rect.width();
                let x2 = tl_rect.left() + los_frac * tl_rect.width();
                let col = pass_colors[i % pass_colors.len()];
                let block = egui::Rect::from_x_y_ranges(x1..=x2, (tl_rect.top() + 4.0)..=(tl_rect.bottom() - 4.0));
                painter.rect_filled(block, 1.0, col.linear_multiply(0.7));
                painter.rect_filled(egui::Rect::from_x_y_ranges(x1..=(x1 + 1.0), (tl_rect.top() + 4.0)..=(tl_rect.bottom() - 4.0)), 0.0, col);
                painter.rect_filled(egui::Rect::from_x_y_ranges((x2 - 1.0)..=x2, (tl_rect.top() + 4.0)..=(tl_rect.bottom() - 4.0)), 0.0, col);
                if x2 - x1 > 16.0 {
                    painter.text(
                        egui::pos2((x1 + x2) / 2.0, tl_rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        &job.satellite,
                        egui::FontId::proportional(7.0),
                        egui::Color32::WHITE,
                    );
                }
                if let Some(hx) = hover_x {
                    if hx >= x1 && hx <= x2 {
                        hovered_job = Some(job);
                    }
                }
            }

            // Current time marker
            let now_frac = ((now_unix - today_start) / day_span).clamp(0.0, 1.0) as f32;
            let now_x = tl_rect.left() + now_frac * tl_rect.width();
            painter.line_segment(
                [egui::pos2(now_x, tl_rect.top()), egui::pos2(now_x, tl_rect.bottom())],
                egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 80, 80)),
            );

            // Tooltip for hovered pass
            if let Some(job) = hovered_job {
                let tip = format!("{}\nAOS: {}  LOS: {}\n{:.3} MHz", job.satellite, job.aos, job.los, job.frequency_hz as f64 / 1e6);
                tl_resp.on_hover_text(tip);
            }
        }

        if jobs.is_empty() {
            ui.label(egui::RichText::new("No upcoming passes (update TLE data in Satellite tab).").color(egui::Color32::GRAY));
        } else {
            egui::ScrollArea::vertical().max_height(180.0).id_salt("sched_sat_scroll").show(ui, |ui| {
                egui::Grid::new("sched_grid").num_columns(5).striped(true).show(ui, |ui| {
                    ui.label(egui::RichText::new("Satellite").strong()).on_hover_text("Satellite name from TLE catalogue.");
                    ui.label(egui::RichText::new("AOS").strong()).on_hover_text("Acquisition of Signal — time the satellite rises above the horizon.");
                    ui.label(egui::RichText::new("LOS").strong()).on_hover_text("Loss of Signal — time the satellite drops below the horizon.");
                    ui.label(egui::RichText::new("Freq").strong()).on_hover_text("Downlink frequency to tune to.");
                    ui.label(egui::RichText::new("Tune").strong());
                    ui.end_row();
                    for job in &jobs {
                        ui.label(&job.satellite);
                        ui.label(&job.aos);
                        ui.label(&job.los);
                        ui.monospace(format!("{:.3} MHz", job.frequency_hz as f64 / 1e6));
                        if ui.small_button("📡 Tune").on_hover_text("Tune SDR to this satellite's frequency now.").clicked() {
                            if let Ok(mut state) = self.shared.try_lock() {
                                state.source.frequency_hz = job.frequency_hz;
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        }

        ui.separator();
        ui.label(egui::RichText::new("Custom Timed Tasks").strong())
            .on_hover_text("Schedule a one-shot frequency tune at a specific time (HH:MM:SS today).");

        // Add task form
        ui.group(|ui| {
            egui::Grid::new("task_form_grid").num_columns(2).show(ui, |ui| {
                ui.label("Label:");
                ui.add(egui::TextEdit::singleline(&mut self.new_task_label).desired_width(150.0).hint_text("e.g. NOAA pass"));
                ui.end_row();
                ui.label("Freq (MHz):");
                ui.add(egui::TextEdit::singleline(&mut self.new_task_freq_mhz).desired_width(100.0).hint_text("137.620"));
                ui.end_row();
                ui.label("Time (HH:MM):");
                ui.add(egui::TextEdit::singleline(&mut self.new_task_time).desired_width(80.0).hint_text("14:30"));
                ui.end_row();
            });
            if !self.new_task_error.is_empty() {
                ui.colored_label(egui::Color32::RED, self.new_task_error.as_str());
            }
            if ui.button("+ Add Task").clicked() {
                let label = self.new_task_label.trim().to_string();
                let freq_res = self.new_task_freq_mhz.trim().parse::<f64>();
                let time_res = parse_hhmm_today(self.new_task_time.trim());
                match (freq_res, time_res) {
                    (Ok(mhz), Some(at_unix)) if mhz > 0.0 => {
                        if let Ok(mut state) = self.shared.try_lock() {
                            state.scheduler.custom_tasks.push(crate::scheduler::CustomTask {
                                label: if label.is_empty() { format!("{:.3} MHz", mhz) } else { label },
                                frequency_hz: (mhz * 1e6) as u64,
                                at_unix,
                                fired: false,
                            });
                        }
                        self.new_task_label.clear();
                        self.new_task_freq_mhz.clear();
                        self.new_task_time.clear();
                        self.new_task_error.clear();
                    }
                    (Err(_), _) => self.new_task_error = "Invalid frequency.".to_string(),
                    (_, None) => self.new_task_error = "Invalid time — use HH:MM format.".to_string(),
                    _ => self.new_task_error = "Frequency must be > 0.".to_string(),
                }
            }
        });

        if !custom_tasks.is_empty() {
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            egui::Grid::new("custom_tasks_grid").num_columns(4).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Label").strong());
                ui.label(egui::RichText::new("Freq").strong());
                ui.label(egui::RichText::new("In").strong());
                ui.label(egui::RichText::new("Del").strong());
                ui.end_row();
                let mut remove_idx = None;
                for (i, task) in custom_tasks.iter().enumerate() {
                    let remaining = task.at_unix - now_unix;
                    let color = if task.fired { egui::Color32::GRAY }
                        else if remaining < 0.0 { egui::Color32::RED }
                        else { egui::Color32::WHITE };
                    ui.colored_label(color, &task.label);
                    ui.monospace(format!("{:.3} MHz", task.frequency_hz as f64 / 1e6));
                    let in_str = if task.fired { "fired".to_string() }
                        else if remaining < 0.0 { "overdue".to_string() }
                        else if remaining < 60.0 { format!("{:.0}s", remaining) }
                        else { format!("{:.0}m", remaining / 60.0) };
                    ui.label(in_str);
                    if ui.small_button("✕").clicked() { remove_idx = Some(i); }
                    ui.end_row();
                }
                if let Some(idx) = remove_idx {
                    if let Ok(mut state) = self.shared.try_lock() {
                        if idx < state.scheduler.custom_tasks.len() {
                            state.scheduler.custom_tasks.remove(idx);
                        }
                    }
                }
            });
        }

        // Session notes — lightweight text scratchpad
        ui.separator();
        ui.collapsing("📝 Session Notes", |ui| {
            ui.label(egui::RichText::new("Jot down frequencies, signal notes, or observations for this session.").small().color(egui::Color32::GRAY));
            ui.add(egui::TextEdit::multiline(&mut self.session_notes)
                .desired_rows(6)
                .desired_width(f32::INFINITY)
                .hint_text("e.g. 'Strong signal at 145.500 MHz — probably a local repeater. Heard voice at 156.800 MHz marine ch16.'"));
            ui.horizontal(|ui| {
                if ui.small_button("💾 Save to file").on_hover_text("Save session notes to a text file.").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name("sdr_session_notes.txt")
                        .add_filter("Text", &["txt"])
                        .save_file()
                    {
                        let _ = std::fs::write(&path, &self.session_notes);
                    }
                }
                if ui.small_button("Clear").on_hover_text("Clear all session notes.").clicked() {
                    self.session_notes.clear();
                }
            });
        });
    }
}

/// Read process RSS memory from /proc/self/status (Linux).
fn proc_memory_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(val) = line.strip_prefix("VmRSS:") {
            let kb: u64 = val.trim().trim_end_matches(" kB").parse().ok()?;
            return Some(kb);
        }
    }
    None
}

/// Parse "HH:MM" or "HH:MM:SS" as a unix timestamp for today in local time.
fn parse_hhmm_today(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 { return None; }
    let h: u64 = parts[0].parse().ok()?;
    let m: u64 = parts[1].parse().ok()?;
    let sec: u64 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    if h > 23 || m > 59 || sec > 59 { return None; }
    // Get start of today in UTC via SystemTime
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let today_start = now - (now % 86400);
    Some((today_start + h * 3600 + m * 60 + sec) as f64)
}
