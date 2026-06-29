use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui_dock::{DockArea, DockState, NodeIndex, Style, SurfaceIndex};

use crate::adsb_decoder::AdsBDecoder;
use crate::adsb_panel::AdsBPanel;
use crate::ai_panel::AiPanel;
use crate::audio_output::AudioOutput;
use crate::bookmarks::BookmarkDb;
use crate::config::AppConfig;
use crate::database::Database;
use crate::demod::Demodulator;
use crate::mqtt::MqttPublisher;
use crate::recorder_panel::RecorderPanel;
use crate::satellite_panel::SatellitePanel;
use crate::scheduler::Scheduler;
use crate::sdr_panel::SdrPanel;
use crate::source_manager::SourceManager;
use crate::spectrum::SpectrumAnalyzer;
use crate::tle_engine::TleEngine;
use crate::howto_panel::HowToPanel;
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
}

pub struct SharedState {
    pub source: SourceManager,
    pub spectrum: SpectrumAnalyzer,
    pub config: AppConfig,
    #[allow(dead_code)]
    pub db: Database,
    pub bookmarks: BookmarkDb,
    pub scheduler: Scheduler,
    pub tle: TleEngine,
    pub demod_mode: crate::sdr_panel::DemodMode,
    pub recording: bool,
    pub adsb_running: bool,
    pub selected_satellite: Option<String>,
    #[allow(dead_code)]
    pub iq_tx: Option<crossbeam_channel::Sender<Vec<f32>>>,
    #[allow(dead_code)]
    pub audio_tx: Option<crossbeam_channel::Sender<Vec<f32>>>,
    pub audio_running: bool,
    pub volume: f32,
    pub squelch: f32,
    pub lpf_cutoff: f32,
    pub fm_deviation_hz: f32,
    pub audio_peak: f32,
    pub freq_history: VecDeque<u64>,
}

pub struct CentralApp {
    dock_state: DockState<Tab>,
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
    recording_start: Option<std::time::Instant>,
    show_welcome: bool,
    bm_last_len: usize,
    bm_dirty_since: Option<std::time::Instant>,
    // New-bookmark form state
    new_bm_name: String,
    new_bm_freq_mhz: String,
    new_bm_mode: String,
    new_bm_category: String,
    new_bm_error: String,
    show_add_bm: bool,
    bm_import_msg: String,
    // Bookmark inline edit
    edit_bm_idx: Option<usize>,
    edit_bm_name: String,
    edit_bm_freq_mhz: String,
    edit_bm_mode: String,
    edit_bm_category: String,
    // Custom task form
    new_task_label: String,
    new_task_freq_mhz: String,
    new_task_time: String,
    new_task_error: String,
}

impl CentralApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (audio_tx, audio_rx) = crossbeam_channel::bounded(64);
        let audio_rx = Arc::new(Mutex::new(audio_rx));

        let shared = Arc::new(Mutex::new(SharedState {
            source: SourceManager::new(),
            spectrum: SpectrumAnalyzer::new(),
            config: AppConfig::load_or_default(),
            db: Database::open_or_create().unwrap(),
            bookmarks: BookmarkDb::load_or_default(),
            scheduler: Scheduler::new(),
            tle: TleEngine::new(),
            demod_mode: crate::sdr_panel::DemodMode::Fm,
            recording: false,
            adsb_running: false,
            selected_satellite: None,
            iq_tx: None,
            audio_tx: None,
            audio_running: false,
            volume: 0.5,
            squelch: -50.0,
            lpf_cutoff: 15000.0,
            fm_deviation_hz: 0.0,
            audio_peak: 0.0,
            freq_history: VecDeque::with_capacity(20),
        }));

        let mut web_remote = WebRemote::new();
        let mut mqtt = MqttPublisher::new();
        {
            let state = shared.lock().unwrap();
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
        }

        // Start demo source immediately
        {
            let mut state = shared.lock().unwrap();
            state.source.frequency_hz = state.config.default_freq_hz;
            state.source.sample_rate_hz = state.config.default_sample_rate;
            state.source.gain_db = state.config.default_gain;
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
            let init_freq = state.source.frequency_hz;
            if state.freq_history.is_empty() || state.freq_history.back() != Some(&init_freq) {
                state.freq_history.push_back(init_freq);
            }
        }

        let mut dock_state = DockState::new(vec![
            Tab::Spectrum,
            Tab::Sdr,
            Tab::Satellite,
            Tab::AdsB,
            Tab::Recorder,
            Tab::Scanner,
            Tab::AiAgent,
            Tab::HowTo,
        ]);

        let surface = SurfaceIndex::main();
        dock_state[surface].split_left(NodeIndex::root(), 0.25, vec![Tab::Bookmarks, Tab::Scheduler, Tab::Settings]);

        Self {
            dock_state,
            shared: shared.clone(),
            sdr_panel: SdrPanel::new(shared.clone()),
            satellite_panel: SatellitePanel::new(shared.clone()),
            adsb_panel: AdsBPanel::new(shared.clone()),
            recorder_panel: RecorderPanel::new(shared.clone()),
            ai_panel: AiPanel::new(shared.clone()),
            howto_panel: HowToPanel::new(),
            web_remote,
            mqtt,
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
                let state = shared.lock().unwrap();
                state.source.frequency_hz
            },
            freq_history_idx: None,
            new_bm_name: String::new(),
            new_bm_freq_mhz: String::new(),
            new_bm_mode: "NFM".to_string(),
            new_bm_category: "Custom".to_string(),
            new_bm_error: String::new(),
            show_add_bm: false,
            bm_import_msg: String::new(),
            edit_bm_idx: None,
            edit_bm_name: String::new(),
            edit_bm_freq_mhz: String::new(),
            edit_bm_mode: String::new(),
            edit_bm_category: String::new(),
            new_task_label: String::new(),
            new_task_freq_mhz: String::new(),
            new_task_time: String::new(),
            new_task_error: String::new(),
            recording_start: None,
            bm_last_len: 0,
            bm_dirty_since: None,
            // Show welcome if config is fresh (default frequency = 100 MHz means unconfigured)
            show_welcome: {
                let state = shared.lock().unwrap();
                state.config.ai_api_key.is_empty() && state.config.mqtt_broker == "localhost:1883"
            },
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

        // Keyboard shortcuts
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
                // Arrow keys: tune up/down by 1 MHz
                if i.key_pressed(egui::Key::ArrowUp) {
                    state.source.frequency_hz = (state.source.frequency_hz + 1_000_000).min(1_770_000_000);
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(1_000_000).max(500_000);
                }
                // Arrow left/right: tune by 100 kHz
                if i.key_pressed(egui::Key::ArrowRight) {
                    state.source.frequency_hz = (state.source.frequency_hz + 100_000).min(1_770_000_000);
                }
                if i.key_pressed(egui::Key::ArrowLeft) {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(100_000).max(500_000);
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
                // R: toggle recording
                if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl {
                    if state.recording {
                        state.recording = false;
                    }
                    // Note: actual start_recording() is handled by recorder_panel via state.recording
                }
                // M: toggle audio mute
                if i.key_pressed(egui::Key::M) {
                    state.audio_running = !state.audio_running;
                }
                // Ctrl+S: save config (also persists recent frequencies)
                if i.modifiers.ctrl && i.key_pressed(egui::Key::S) {
                    let recent: Vec<u64> = state.freq_history.iter().cloned().collect();
                    state.config.recent_frequencies = recent;
                    state.config.save();
                }
                // F: freeze/unfreeze spectrum
                if i.key_pressed(egui::Key::F) && !i.modifiers.ctrl && !i.modifiers.alt {
                    state.spectrum.frozen = !state.spectrum.frozen;
                }
            }
        });

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
                            eprintln!("Failed to start audio");
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
                // Auto-tune to the first active pass
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                let tune_to = state.scheduler.active_job(now_unix).map(|j| (j.satellite.clone(), j.frequency_hz));
                if let Some((sat, freq)) = tune_to {
                    if sat != self.last_auto_tuned_satellite {
                        state.source.frequency_hz = freq;
                        self.last_auto_tuned_satellite = sat.clone();
                    }
                    // Apply doppler correction
                    let doppler = state.tle.doppler_shift_for_sat(&sat, freq as f64, now_unix);
                    if doppler.abs() > 1.0 {
                        let corrected = (freq as f64 + doppler) as u64;
                        if corrected != state.source.frequency_hz {
                            state.source.frequency_hz = corrected;
                        }
                    }
                }
                // Poll custom scheduled tasks
                if let Some((label, freq)) = state.scheduler.poll_custom_tasks(now_unix) {
                    state.source.frequency_hz = freq;
                    eprintln!("[scheduler] fired custom task '{}' → {:.3} MHz", label, freq as f64 / 1e6);
                }
            }

            // Frequency scanner tick (runs every frame, rate-limited by dwell_ms)
            {
                let peak = if let Ok(state) = self.shared.try_lock() {
                    state.spectrum.peak_level()
                } else { -120.0 };
                let prev_hits = self.scanner.hits.len();
                self.scanner.tick(peak);
                // Publish any new hits to MQTT
                if self.scanner.hits.len() > prev_hits {
                    for hit in &self.scanner.hits[prev_hits..] {
                        self.mqtt.publish_scanner_hit(hit.freq_hz, hit.strength_db);
                    }
                }
                if let Some(freq) = self.scanner.tune_request_hz.take() {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.source.frequency_hz = freq;
                    }
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
                });
            }
        }

        // Apply config changes triggered from Settings tab
        if let Ok(mut state) = self.shared.try_lock() {
            if state.config.needs_apply {
                state.config.needs_apply = false;
                // Apply theme
                if state.config.theme == "light" {
                    ctx.set_visuals(egui::Visuals::light());
                } else {
                    ctx.set_visuals(egui::Visuals::dark());
                }
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
            self.web_remote.broadcast_state(freq, gain, &mode, ac_count, &passes, squelch, volume, recording, scanner_active, snr);
            if self.last_scheduler_update.elapsed().as_secs() < 1 {
                self.mqtt.tick(freq, gain);
                self.mqtt.publish_signal(freq, peak, noise, &mode, recording);
                self.mqtt.publish_passes(&passes);
                if !self.adsb_panel.aircraft.is_empty() {
                    self.mqtt.publish_aircraft(&self.adsb_panel.aircraft);
                }
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
                    bookmarks: state.bookmarks.bookmarks.clone(),
                    jobs: state.scheduler.jobs.clone(),
                    custom_tasks: state.scheduler.custom_tasks.clone(),
                    auto_tune_enabled: state.scheduler.auto_tune_enabled,
                })
            } else {
                return;
            }
        };

        let style = Style::from_egui(ui.style().as_ref());
        DockArea::new(&mut self.dock_state)
            .style(style)
            .show_inside(ui, &mut TabViewer {
                snapshot,
                shared: self.shared.clone(),
                sdr: &mut self.sdr_panel,
                satellite: &mut self.satellite_panel,
                adsb: &mut self.adsb_panel,
                recorder: &mut self.recorder_panel,
                scanner: &mut self.scanner,
                ai: &mut self.ai_panel,
                howto: &mut self.howto_panel,
                bookmark_filter: &mut self.bookmark_filter,
                new_bm_name: &mut self.new_bm_name,
                new_bm_freq_mhz: &mut self.new_bm_freq_mhz,
                new_bm_mode: &mut self.new_bm_mode,
                new_bm_category: &mut self.new_bm_category,
                new_bm_error: &mut self.new_bm_error,
                show_add_bm: &mut self.show_add_bm,
                bm_import_msg: &mut self.bm_import_msg,
                edit_bm_idx: &mut self.edit_bm_idx,
                edit_bm_name: &mut self.edit_bm_name,
                edit_bm_freq_mhz: &mut self.edit_bm_freq_mhz,
                edit_bm_mode: &mut self.edit_bm_mode,
                edit_bm_category: &mut self.edit_bm_category,
                new_task_label: &mut self.new_task_label,
                new_task_freq_mhz: &mut self.new_task_freq_mhz,
                new_task_time: &mut self.new_task_time,
                new_task_error: &mut self.new_task_error,
            });

        // First-run welcome banner
        if self.show_welcome {
            egui::Window::new("👋 Welcome to EZ-SDR!")
                .id(egui::Id::new("welcome_banner"))
                .default_width(480.0)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(egui::RichText::new("You're running in DEMO mode — no real SDR hardware required to explore!").strong());
                    ui.add_space(4.0);
                    egui::Grid::new("welcome_tips").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                        ui.label("📡"); ui.label("Connect an RTL-SDR dongle and rebuild with '--features rtlsdr' for live reception."); ui.end_row();
                        ui.label("📻"); ui.label("Click a band in 'Bands:' row (SDR tab) to jump to common frequencies."); ui.end_row();
                        ui.label("📊"); ui.label("Left-click the spectrum to tune. Scroll to zoom. Right-click for more options."); ui.end_row();
                        ui.label("🔊"); ui.label("Press Start Audio or M key to toggle audio output."); ui.end_row();
                        ui.label("❓"); ui.label("Open the How To tab for a full beginner guide."); ui.end_row();
                        ui.label("?");  ui.label("Press ? anywhere to show keyboard shortcuts."); ui.end_row();
                    });
                    ui.add_space(6.0);
                    if ui.button("Got it — dismiss").clicked() {
                        self.show_welcome = false;
                    }
                });
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
                    }
                }
            }
            ui.separator();
            if let Ok(state) = self.shared.try_lock() {
                let running = state.source.status == crate::source_manager::SourceStatus::Running;
                let status_color = if running { egui::Color32::GREEN } else { egui::Color32::GRAY };
                ui.colored_label(status_color, "●")
                    .on_hover_text(if running { "SDR source is active and streaming samples." } else { "SDR source is stopped. Press Start or Space to begin." });
                ui.small(if running { "Running" } else { "Stopped" })
                    .on_hover_text("SDR source status indicator.");
                ui.separator();
                let freq_str = format!("{:.3} MHz", state.source.frequency_hz as f64 / 1e6);
                let freq_resp = ui.add(egui::Label::new(egui::RichText::new(&freq_str).monospace())
                    .sense(egui::Sense::click()))
                    .on_hover_text("Click to copy frequency to clipboard. Use arrow keys or SDR panel to change. RTL-SDR range: 24–1766 MHz.");
                if freq_resp.clicked() {
                    ui.ctx().copy_text(format!("{:.6}", state.source.frequency_hz as f64 / 1e6));
                }
                ui.separator();
                ui.small(format!("{} · {:.1} MSps", state.demod_mode.label(), state.source.sample_rate_hz as f64 / 1e6))
                    .on_hover_text("Active demodulation mode and sample rate in megasamples per second. Higher sample rates show wider spectrum but use more CPU.");
                ui.separator();
                ui.small(format!("Gain: {:.1} dB", state.source.gain_db))
                    .on_hover_text("RF gain in dB. Higher = more sensitive but more noise and risk of overload. 30–40 dB is typical for outdoor signals.");
                ui.separator();
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
                    ui.colored_label(egui::Color32::RED, rec_label)
                        .on_hover_text("Recording in progress. Go to the Recorder tab to stop.");
                } else {
                    self.recording_start = None;
                }
                if self.audio.is_running() {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 255), "🔊 Audio")
                        .on_hover_text("Audio is playing through your speakers/headphones. Use Vol slider or Stop Audio in the SDR panel.");
                }
                // Demo mode badge — helps beginners know they are in simulation
                if state.source.source_mode == crate::source_manager::SourceMode::Simulated {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 180, 50),
                        "⚠ DEMO",
                    ).on_hover_text("Running in simulated (demo) mode — no real SDR device connected. The spectrum shows synthetic test signals. Connect an RTL-SDR and rebuild with the 'rtlsdr' feature, or use File Replay mode.");
                }
                // MQTT connected badge
                if self.mqtt.is_connected() {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(46, 204, 113), "📡 MQTT")
                        .on_hover_text(format!("Publishing to MQTT broker at {}. Topics: {}/signal, {}/scanner, etc.", self.mqtt.broker, self.mqtt.topic_prefix, self.mqtt.topic_prefix));
                }
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
                let peak_color = if peak > -20.0 { egui::Color32::GREEN }
                    else if peak > -60.0 { egui::Color32::YELLOW }
                    else { egui::Color32::GRAY };
                ui.colored_label(peak_color, format!("Peak: {:.1}dB", peak))
                    .on_hover_text("Strongest signal in the current spectrum view (dBFS). Green = strong signal present, yellow = moderate, grey = weak/none.");
                ui.colored_label(egui::Color32::DARK_GRAY, format!("Floor: {:.1}dB", noise_floor))
                    .on_hover_text("Estimated noise floor — the average background noise level. The gap between floor and peak is SNR (signal-to-noise ratio).");
                let snr = peak - noise_floor;
                let (badge, badge_color, badge_tip) = if snr > 20.0 {
                    ("🟢 Signal", egui::Color32::GREEN,     "Strong signal detected (SNR > 20 dB). Good reception.")
                } else if snr > 8.0 {
                    ("🟡 Weak",   egui::Color32::YELLOW,    "Weak signal detected (SNR 8–20 dB). May be readable.")
                } else {
                    ("⚫ Quiet",  egui::Color32::DARK_GRAY,  "No significant signal at current frequency (SNR < 8 dB). Try a different frequency or increase gain.")
                };
                ui.colored_label(badge_color, badge).on_hover_text(badge_tip);
            }
        });

        // Keyboard shortcuts help overlay
        if self.show_keyboard_help {
            egui::Window::new("Keyboard Shortcuts (?)")
                .id(egui::Id::new("keyboard_help"))
                .default_width(350.0)
                .movable(true)
                .show(ui.ctx(), |ui| {
                    egui::Grid::new("shortcuts_grid").num_columns(2).striped(true).show(ui, |ui| {
                        ui.monospace("Space"); ui.label("Start/Stop SDR source"); ui.end_row();
                        ui.monospace("↑ / ↓"); ui.label("Tune ±1 MHz"); ui.end_row();
                        ui.monospace("← / →"); ui.label("Tune ±100 kHz"); ui.end_row();
                        ui.monospace("Alt+← / Alt+→"); ui.label("Frequency history back/forward"); ui.end_row();
                        ui.monospace("F1"); ui.label("Demod: RAW"); ui.end_row();
                        ui.monospace("F2"); ui.label("Demod: AM"); ui.end_row();
                        ui.monospace("F3"); ui.label("Demod: NFM"); ui.end_row();
                        ui.monospace("F4"); ui.label("Demod: WFM"); ui.end_row();
                        ui.monospace("F5"); ui.label("Demod: LSB"); ui.end_row();
                        ui.monospace("F6"); ui.label("Demod: USB"); ui.end_row();
                        ui.monospace("M"); ui.label("Toggle audio on/off"); ui.end_row();
                        ui.monospace("F"); ui.label("Freeze / unfreeze spectrum display"); ui.end_row();
                        ui.monospace("Ctrl+S"); ui.label("Save settings"); ui.end_row();
                        ui.monospace("?"); ui.label("Toggle this help"); ui.end_row();
                        ui.monospace("Left-click (spectrum/wfall)"); ui.label("Tune to clicked frequency"); ui.end_row();
                        ui.monospace("Right-click (spectrum)"); ui.label("Context menu (bookmark, marker, zoom reset…)"); ui.end_row();
                        ui.monospace("Middle-click (spectrum)"); ui.label("Add frequency marker"); ui.end_row();
                        ui.monospace("Scroll"); ui.label("Zoom in/out on spectrum"); ui.end_row();
                        ui.monospace("Shift+Scroll"); ui.label("Pan spectrum left/right"); ui.end_row();
                        ui.monospace("Mid-drag"); ui.label("Pan spectrum view"); ui.end_row();
                    });
                });
        }
    }
}

struct SharedSnapshot {
    #[allow(dead_code)]
    bookmarks: Vec<crate::bookmarks::Bookmark>,
    jobs: Vec<crate::scheduler::ScheduledJob>,
    custom_tasks: Vec<crate::scheduler::CustomTask>,
    auto_tune_enabled: bool,
}

struct TabViewer<'a> {
    snapshot: Option<SharedSnapshot>,
    shared: Arc<Mutex<SharedState>>,
    sdr: &'a mut SdrPanel,
    satellite: &'a mut SatellitePanel,
    adsb: &'a mut AdsBPanel,
    recorder: &'a mut RecorderPanel,
    scanner: &'a mut crate::scanner::FrequencyScanner,
    ai: &'a mut AiPanel,
    howto: &'a mut HowToPanel,
    bookmark_filter: &'a mut String,
    new_bm_name: &'a mut String,
    new_bm_freq_mhz: &'a mut String,
    new_bm_mode: &'a mut String,
    new_bm_category: &'a mut String,
    new_bm_error: &'a mut String,
    show_add_bm: &'a mut bool,
    bm_import_msg: &'a mut String,
    edit_bm_idx: &'a mut Option<usize>,
    edit_bm_name: &'a mut String,
    edit_bm_freq_mhz: &'a mut String,
    edit_bm_mode: &'a mut String,
    edit_bm_category: &'a mut String,
    new_task_label: &'a mut String,
    new_task_freq_mhz: &'a mut String,
    new_task_time: &'a mut String,
    new_task_error: &'a mut String,
}

impl<'a> egui_dock::TabViewer for TabViewer<'a> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Tab::Sdr => "📻 SDR".into(),
            Tab::Spectrum => "📊 Spectrum".into(),
            Tab::Satellite => "🛸 Satellite".into(),
            Tab::AdsB => "✈ ADS-B".into(),
            Tab::Recorder => "⏺ Recorder".into(),
            Tab::Scanner => "🔍 Scanner".into(),
            Tab::AiAgent => "🤖 AI Agent".into(),
            Tab::Bookmarks => "⭐ Bookmarks".into(),
            Tab::Scheduler => "🗓 Scheduler".into(),
            Tab::Settings => "⚙ Settings".into(),
            Tab::HowTo => "❓ How To".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Tab::Sdr => self.sdr.ui(ui),
            Tab::Spectrum => {
                if let Ok(mut state) = self.shared.try_lock() {
                    // Sync bookmarks and VFO BW to spectrum for overlays
                    state.spectrum.bookmark_freqs = state.bookmarks.bookmarks.iter()
                        .map(|b| (b.frequency_hz, b.name.clone()))
                        .collect();
                    state.spectrum.vfo_bw_hz = state.lpf_cutoff as u32 * 2;
                    state.spectrum.ui(ui);
                    if let Some(freq) = state.spectrum.clicked_tune_freq.take() {
                        state.source.frequency_hz = freq;
                    }
                    if let Some(freq) = state.spectrum.pending_bookmark_freq.take() {
                        let freq_mhz = freq as f64 / 1e6;
                        let mode = state.demod_mode.label().to_string();
                        let name = format!("{:.4} MHz {}", freq_mhz, mode);
                        state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                            name,
                            frequency_hz: freq,
                            mode,
                            bandwidth_hz: 12_500,
                            category: "Quick".to_string(),
                            notes: String::new(),
                        });
                    }
                }
            }
            Tab::Satellite => self.satellite.ui(ui),
            Tab::AdsB => self.adsb.ui(ui),
            Tab::Recorder => self.recorder.ui(ui),
            Tab::Scanner => self.scanner.ui(ui),
            Tab::AiAgent => self.ai.ui(ui),
            Tab::HowTo => self.howto.ui(ui),
            Tab::Bookmarks => {
                let bm_count = if let Ok(state) = self.shared.try_lock() { state.bookmarks.bookmarks.len() } else { 0 };
                ui.heading("Frequency Bookmarks");
                ui.horizontal(|ui| {
                    ui.label(format!("{} bookmarks", bm_count));
                    ui.add(egui::TextEdit::singleline(self.bookmark_filter).hint_text("Filter...").desired_width(150.0));
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
                                        *self.bm_import_msg = format!("Imported {} bookmarks.", count);
                                    } else {
                                        *self.bm_import_msg = err;
                                    }
                                }
                            }
                        }
                    }
                    if ui.button("📤 Export CSV").on_hover_text("Export all bookmarks to a timestamped CSV file in the current directory.").clicked() {
                        if let Ok(state) = self.shared.try_lock() {
                            let (path, err) = state.bookmarks.export_csv();
                            if err.is_empty() {
                                *self.bm_import_msg = format!("Exported to {}", path);
                            } else {
                                *self.bm_import_msg = err;
                            }
                        }
                    }
                    if ui.button(if *self.show_add_bm { "✕ Cancel" } else { "+ Add" })
                        .on_hover_text("Add a new bookmark for the current or any frequency.")
                        .clicked()
                    {
                        *self.show_add_bm = !*self.show_add_bm;
                        self.new_bm_error.clear();
                        // Pre-fill frequency from SDR
                        if *self.show_add_bm {
                            if let Ok(state) = self.shared.try_lock() {
                                *self.new_bm_freq_mhz = format!("{:.4}", state.source.frequency_hz as f64 / 1e6);
                                *self.new_bm_mode = state.demod_mode.label().to_string();
                            }
                        }
                    }
                });

                // Add bookmark form
                if *self.show_add_bm {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("New Bookmark").strong());
                        egui::Grid::new("add_bm_grid").num_columns(2).show(ui, |ui| {
                            ui.label("Name:");
                            ui.add(egui::TextEdit::singleline(self.new_bm_name).desired_width(200.0).hint_text("e.g. Local Police"));
                            ui.end_row();
                            ui.label("Freq (MHz):");
                            ui.add(egui::TextEdit::singleline(self.new_bm_freq_mhz).desired_width(120.0).hint_text("145.5"));
                            ui.end_row();
                            ui.label("Mode:");
                            egui::ComboBox::from_id_salt("bm_mode_combo")
                                .selected_text(self.new_bm_mode.as_str())
                                .show_ui(ui, |ui| {
                                    for m in ["NFM", "WFM", "AM", "USB", "LSB", "RAW"] {
                                        ui.selectable_value(self.new_bm_mode, m.to_string(), m);
                                    }
                                });
                            ui.end_row();
                            ui.label("Category:");
                            ui.add(egui::TextEdit::singleline(self.new_bm_category).desired_width(150.0).hint_text("Custom"));
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
                                        notes: String::new(),
                                    };
                                    if let Ok(mut state) = self.shared.try_lock() {
                                        state.bookmarks.bookmarks.push(bm);
                                    }
                                    self.new_bm_name.clear();
                                    self.new_bm_error.clear();
                                    *self.show_add_bm = false;
                                }
                                Ok(_) => *self.new_bm_error = "Frequency must be > 0 MHz".to_string(),
                                Err(_) if name.is_empty() => *self.new_bm_error = "Name cannot be empty".to_string(),
                                Err(_) => *self.new_bm_error = "Invalid frequency — enter a number like 145.5".to_string(),
                            }
                        }
                    });
                }

                if !self.bm_import_msg.is_empty() {
                    ui.colored_label(egui::Color32::from_rgb(100, 220, 100), self.bm_import_msg.as_str());
                }

                ui.separator();

                let filter_lower = self.bookmark_filter.to_lowercase();
                // Collect bookmarks while holding lock briefly
                let (bookmarks_snapshot, total) = if let Ok(state) = self.shared.try_lock() {
                    (state.bookmarks.bookmarks.clone(), state.bookmarks.bookmarks.len())
                } else {
                    return;
                };
                let _ = total;

                let filtered: Vec<(usize, &crate::bookmarks::Bookmark)> = bookmarks_snapshot.iter()
                    .enumerate()
                    .filter(|(_, b)| filter_lower.is_empty()
                        || b.name.to_lowercase().contains(&filter_lower)
                        || b.category.to_lowercase().contains(&filter_lower)
                        || b.mode.to_lowercase().contains(&filter_lower)
                        || b.freq_display().contains(&filter_lower))
                    .collect();

                let mut categories: Vec<String> = filtered.iter().map(|(_, b)| b.category.clone()).collect();
                categories.sort();
                categories.dedup();

                let mut delete_idx: Option<usize> = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for cat in &categories {
                        let cat_count = filtered.iter().filter(|(_, b)| &b.category == cat).count();
                        let cat_header = format!("{} ({})", cat, cat_count);
                        ui.collapsing(cat_header, |ui| {
                            for (orig_idx, bm) in filtered.iter().filter(|(_, b)| &b.category == cat) {
                                ui.horizontal(|ui| {
                                    let is_editing = *self.edit_bm_idx == Some(*orig_idx);
                                    if is_editing {
                                        // Inline edit row
                                        ui.add(egui::TextEdit::singleline(self.edit_bm_name).desired_width(120.0).hint_text("Name"));
                                        ui.add(egui::TextEdit::singleline(self.edit_bm_freq_mhz).desired_width(70.0).hint_text("MHz"));
                                        egui::ComboBox::from_id_salt(format!("edit_mode_{}", orig_idx))
                                            .selected_text(self.edit_bm_mode.as_str())
                                            .show_ui(ui, |ui| {
                                                for m in ["NFM", "WFM", "AM", "USB", "LSB", "RAW"] {
                                                    ui.selectable_value(self.edit_bm_mode, m.to_string(), m);
                                                }
                                            });
                                        ui.add(egui::TextEdit::singleline(self.edit_bm_category).desired_width(80.0).hint_text("Category"));
                                        if ui.small_button("✓").on_hover_text("Save changes").clicked() {
                                            if let Ok(freq_mhz) = self.edit_bm_freq_mhz.trim().parse::<f64>() {
                                                if let Ok(mut state) = self.shared.try_lock() {
                                                    if let Some(bm) = state.bookmarks.bookmarks.get_mut(*orig_idx) {
                                                        bm.name = self.edit_bm_name.trim().to_string();
                                                        bm.frequency_hz = (freq_mhz * 1e6) as u64;
                                                        bm.mode = self.edit_bm_mode.clone();
                                                        bm.category = if self.edit_bm_category.trim().is_empty() { "Custom".into() } else { self.edit_bm_category.trim().to_string() };
                                                    }
                                                }
                                            }
                                            *self.edit_bm_idx = None;
                                        }
                                        if ui.small_button("✕").on_hover_text("Cancel edit").clicked() {
                                            *self.edit_bm_idx = None;
                                        }
                                    } else {
                                        ui.label(&bm.name);
                                        ui.monospace(bm.freq_display());
                                        ui.small(&bm.mode);
                                        let tune_tip = if bm.notes.is_empty() {
                                            format!("Tune to {} and switch to {} mode", bm.freq_display(), bm.mode)
                                        } else {
                                            format!("Tune to {} and switch to {} mode\n{}", bm.freq_display(), bm.mode, bm.notes)
                                        };
                                        if ui.small_button("Tune")
                                            .on_hover_text(tune_tip)
                                            .clicked()
                                        {
                                            if let Ok(mut state) = self.shared.try_lock() {
                                                state.source.frequency_hz = bm.frequency_hz;
                                                if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&bm.mode) {
                                                    state.demod_mode = mode;
                                                }
                                            }
                                        }
                                        if ui.small_button("✏")
                                            .on_hover_text("Edit this bookmark")
                                            .clicked()
                                        {
                                            *self.edit_bm_idx = Some(*orig_idx);
                                            *self.edit_bm_name = bm.name.clone();
                                            *self.edit_bm_freq_mhz = format!("{:.4}", bm.frequency_hz as f64 / 1e6);
                                            *self.edit_bm_mode = bm.mode.clone();
                                            *self.edit_bm_category = bm.category.clone();
                                        }
                                        if ui.small_button("🗑")
                                            .on_hover_text("Delete this bookmark")
                                            .clicked()
                                        {
                                            delete_idx = Some(*orig_idx);
                                        }
                                    }
                                });
                            }
                        });
                    }
                });
                if let Some(idx) = delete_idx {
                    if let Ok(mut state) = self.shared.try_lock() {
                        if idx < state.bookmarks.bookmarks.len() {
                            state.bookmarks.bookmarks.remove(idx);
                        }
                    }
                }
            }
            Tab::Scheduler => {
                let (jobs, custom_tasks, auto_tune) = match &self.snapshot {
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
                        // Border via two filled rects (left/right edges)
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
                        ui.add(egui::TextEdit::singleline(self.new_task_label).desired_width(150.0).hint_text("e.g. NOAA pass"));
                        ui.end_row();
                        ui.label("Freq (MHz):");
                        ui.add(egui::TextEdit::singleline(self.new_task_freq_mhz).desired_width(100.0).hint_text("137.620"));
                        ui.end_row();
                        ui.label("Time (HH:MM):");
                        ui.add(egui::TextEdit::singleline(self.new_task_time).desired_width(80.0).hint_text("14:30"));
                        ui.end_row();
                    });
                    if !self.new_task_error.is_empty() {
                        ui.colored_label(egui::Color32::RED, self.new_task_error.as_str());
                    }
                    if ui.button("+ Add Task").clicked() {
                        let label = self.new_task_label.trim().to_string();
                        let freq_res = self.new_task_freq_mhz.trim().parse::<f64>();
                        // Parse HH:MM or HH:MM:SS as today's unix time
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
                                *self.new_task_error = String::new();
                            }
                            (Err(_), _) => *self.new_task_error = "Invalid frequency.".to_string(),
                            (_, None) => *self.new_task_error = "Invalid time — use HH:MM format.".to_string(),
                            _ => *self.new_task_error = "Frequency must be > 0.".to_string(),
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
            }
            Tab::Settings => {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.config.ui(ui);
                }
            }
        }
    }
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
