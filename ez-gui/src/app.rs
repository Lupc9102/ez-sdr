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
}

pub struct CentralApp {
    dock_state: DockState<Tab>,
    shared: Arc<Mutex<SharedState>>,
    sdr_panel: SdrPanel,
    satellite_panel: SatellitePanel,
    adsb_panel: AdsBPanel,
    recorder_panel: RecorderPanel,
    ai_panel: AiPanel,
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
            bookmarks: BookmarkDb::default(),
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
        }

        let mut dock_state = DockState::new(vec![
            Tab::Spectrum,
            Tab::Sdr,
            Tab::Satellite,
            Tab::AdsB,
            Tab::Recorder,
            Tab::AiAgent,
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
                let _ = self.audio_tx.try_send(audio);
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
                RemoteCommand::StartRecord => self.recorder_panel.start_recording(),
                RemoteCommand::StopRecord => self.recorder_panel.stop_recording(),
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
            }

            // Frequency scanner tick (runs every frame, rate-limited by dwell_ms)
            {
                let peak = if let Ok(state) = self.shared.try_lock() {
                    state.spectrum.peak_level()
                } else { -120.0 };
                self.scanner.tick(peak);
                if let Some(freq) = self.scanner.tune_request_hz.take() {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.source.frequency_hz = freq;
                    }
                }
            }
        }

        // Apply config changes triggered from Settings tab
        if let Ok(mut state) = self.shared.try_lock() {
            if state.config.needs_apply {
                state.config.needs_apply = false;
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
            self.web_remote.broadcast_state(freq, gain, &mode, ac_count, &passes);
            if self.last_scheduler_update.elapsed().as_secs() < 1 {
                self.mqtt.tick(freq, gain);
                self.mqtt.publish_passes(&passes);
                if !self.adsb_panel.aircraft.is_empty() {
                    self.mqtt.publish_aircraft(&self.adsb_panel.aircraft);
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
                bookmark_filter: &mut self.bookmark_filter,
            });

        // Status bar
        ui.separator();
        ui.horizontal(|ui| {
            if let Ok(state) = self.shared.try_lock() {
                let running = state.source.status == crate::source_manager::SourceStatus::Running;
                let status_color = if running { egui::Color32::GREEN } else { egui::Color32::GRAY };
                ui.colored_label(status_color, "●");
                ui.small(if running { "Running" } else { "Stopped" });
                ui.separator();
                ui.monospace(format!("{:.3} MHz", state.source.frequency_hz as f64 / 1e6));
                ui.separator();
                ui.small(format!("{} · {:.1} MSps", state.demod_mode.label(), state.source.sample_rate_hz as f64 / 1e6));
                ui.separator();
                ui.small(format!("Gain: {:.1} dB", state.source.gain_db));
                ui.separator();
                if state.recording {
                    ui.colored_label(egui::Color32::RED, "● REC");
                }
                if self.audio.is_running() {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 255), "🔊 Audio");
                }
            }
            // Volume slider
            if let Ok(mut state) = self.shared.try_lock() {
                ui.separator();
                ui.small("Vol:");
                ui.add(egui::Slider::new(&mut state.volume, 0.0..=1.0).text(""));
                ui.separator();
                ui.small("Squelch:");
                ui.add(egui::Slider::new(&mut state.squelch, -120.0..=0.0).text("dB"));
            }
            if let Ok(state) = self.shared.try_lock() {
                let peak = state.spectrum.peak_level();
                let noise_floor = state.spectrum.noise_floor();
                let peak_color = if peak > -20.0 { egui::Color32::GREEN }
                    else if peak > -60.0 { egui::Color32::YELLOW }
                    else { egui::Color32::GRAY };
                ui.colored_label(peak_color, format!("Peak: {:.1}dB", peak));
                ui.colored_label(egui::Color32::DARK_GRAY, format!("Floor: {:.1}dB", noise_floor));
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
                        ui.monospace("Space"); ui.label("Start/Stop source"); ui.end_row();
                        ui.monospace("↑ / ↓"); ui.label("Tune ±1 MHz"); ui.end_row();
                        ui.monospace("← / →"); ui.label("Tune ±100 kHz"); ui.end_row();
                        ui.monospace("?"); ui.label("Toggle this help"); ui.end_row();
                        ui.monospace("Left-click"); ui.label("Tune to frequency on spectrum"); ui.end_row();
                        ui.monospace("Right-click"); ui.label("Reset zoom"); ui.end_row();
                        ui.monospace("Middle-click"); ui.label("Add frequency marker"); ui.end_row();
                        ui.monospace("Scroll"); ui.label("Zoom in/out on spectrum"); ui.end_row();
                        ui.monospace("Shift+Scroll"); ui.label("Pan spectrum left/right"); ui.end_row();
                        ui.monospace("Mid-drag"); ui.label("Pan spectrum view"); ui.end_row();
                    });
                });
        }
    }
}

struct SharedSnapshot {
    bookmarks: Vec<crate::bookmarks::Bookmark>,
    jobs: Vec<crate::scheduler::ScheduledJob>,
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
    bookmark_filter: &'a mut String,
}

impl<'a> egui_dock::TabViewer for TabViewer<'a> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Tab::Sdr => "SDR".into(),
            Tab::Spectrum => "Spectrum".into(),
            Tab::Satellite => "Satellite".into(),
            Tab::AdsB => "ADS-B".into(),
            Tab::Recorder => "Recorder".into(),
            Tab::Scanner => "Scanner".into(),
            Tab::AiAgent => "AI Agent".into(),
            Tab::Bookmarks => "Bookmarks".into(),
            Tab::Scheduler => "Scheduler".into(),
            Tab::Settings => "Settings".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Tab::Sdr => self.sdr.ui(ui),
            Tab::Spectrum => {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.spectrum.ui(ui);
                    if let Some(freq) = state.spectrum.clicked_tune_freq.take() {
                        state.source.frequency_hz = freq;
                    }
                }
            }
            Tab::Satellite => self.satellite.ui(ui),
            Tab::AdsB => self.adsb.ui(ui),
            Tab::Recorder => self.recorder.ui(ui),
            Tab::Scanner => self.scanner.ui(ui),
            Tab::AiAgent => self.ai.ui(ui),
            Tab::Bookmarks => {
                let snapshot = match &self.snapshot {
                    Some(s) => s,
                    None => return,
                };
                ui.heading("Frequency Bookmarks");
                ui.horizontal(|ui| {
                    ui.label(format!("{} bookmarks", snapshot.bookmarks.len()));
                    ui.add(egui::TextEdit::singleline(self.bookmark_filter).hint_text("Filter...").desired_width(150.0));
                });
                ui.separator();

                let filter_lower = self.bookmark_filter.to_lowercase();
                let filtered: Vec<_> = snapshot.bookmarks.iter()
                    .filter(|b| filter_lower.is_empty()
                        || b.name.to_lowercase().contains(&filter_lower)
                        || b.category.to_lowercase().contains(&filter_lower)
                        || b.mode.to_lowercase().contains(&filter_lower)
                        || b.freq_display().contains(&filter_lower))
                    .collect();

                let mut categories: Vec<_> = filtered.iter().map(|b| b.category.clone()).collect();
                categories.sort();
                categories.dedup();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for cat in categories {
                        ui.collapsing(&cat, |ui| {
                            for bm in filtered.iter().filter(|b| b.category == cat) {
                                ui.horizontal(|ui| {
                                    ui.label(&bm.name);
                                    ui.monospace(format!("{}", bm.freq_display()));
                                    ui.small(&bm.mode);
                                    if ui.small_button("Tune").clicked() {
                                        if let Ok(mut state) = self.shared.try_lock() {
                                            state.source.frequency_hz = bm.frequency_hz;
                                            if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&bm.mode) {
                                                state.demod_mode = mode;
                                            }
                                        }
                                    }
                                });
                            }
                        });
                    }
                });
            }
            Tab::Scheduler => {
                let jobs = match &self.snapshot {
                    Some(s) => &s.jobs,
                    None => return,
                };
                ui.heading("Scheduler");
                ui.label("Upcoming auto-record passes");
                ui.separator();
                if jobs.is_empty() {
                    ui.label("No upcoming passes scheduled.");
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::Grid::new("sched_grid").num_columns(5).striped(true).show(ui, |ui| {
                            ui.label("Satellite");
                            ui.label("AOS");
                            ui.label("LOS");
                            ui.label("Freq");
                            ui.label("Action");
                            ui.end_row();
                            for job in jobs {
                                ui.label(&job.satellite);
                                ui.label(&job.aos);
                                ui.label(&job.los);
                                ui.monospace(format!("{:.3} MHz", job.frequency_hz as f64 / 1e6));
                                if ui.small_button("Tune").clicked() {
                                    if let Ok(mut state) = self.shared.try_lock() {
                                        state.source.frequency_hz = job.frequency_hz;
                                    }
                                }
                                ui.end_row();
                            }
                        });
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
