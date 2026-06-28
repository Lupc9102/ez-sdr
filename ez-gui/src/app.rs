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
    AiAgent,
    Bookmarks,
    Scheduler,
    Settings,
}

pub struct SharedState {
    pub source: SourceManager,
    pub spectrum: SpectrumAnalyzer,
    pub config: AppConfig,
    pub db: Database,
    pub bookmarks: BookmarkDb,
    pub scheduler: Scheduler,
    pub tle: TleEngine,
    pub demod_mode: crate::sdr_panel::DemodMode,
    pub recording: bool,
    pub adsb_running: bool,
    pub selected_satellite: Option<String>,
    pub iq_tx: Option<crossbeam_channel::Sender<Vec<f32>>>,
    pub audio_tx: Option<crossbeam_channel::Sender<Vec<f32>>>,
    pub audio_running: bool,
    pub volume: f32,
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
            state.source.start();
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
                        sample_batch.push(samples);
                    } else {
                        break;
                    }
                }
            }
        }

        // Process samples outside lock
        for samples in &sample_batch {
            let (freq, rate, demod_mode, audio_running, volume, adsb_running) = {
                if let Ok(state) = self.shared.try_lock() {
                    (state.source.frequency_hz, state.source.sample_rate_hz,
                     state.demod_mode, state.audio_running, state.volume, state.adsb_running)
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
                let audio = self.demod.demodulate(samples, demod_mode);
                let audio: Vec<f32> = audio.into_iter().map(|s| s * volume).collect();
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

        // Only repaint at ~30fps max
        ctx.request_repaint_after(Duration::from_millis(33));

        // Start/stop audio based on state
        {
            if let Ok(state) = self.shared.try_lock() {
                if state.audio_running && !self.audio.is_running() {
                    let rx = self.audio_rx.clone();
                    if let Err(e) = self.audio.start(rx) {
                        eprintln!("Failed to start audio: {}", e);
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

        // Scheduler tick: check upcoming passes
        {
            if let Ok(mut state) = self.shared.try_lock() {
                let passes = state.tle.upcoming_passes().to_vec();
                state.scheduler.update_from_passes(&passes);
            }
        }

        // Broadcast state + MQTT tick
        if let Ok(mut state) = self.shared.try_lock() {
            let mode = format!("{:?}", state.demod_mode);
            let freq = state.source.frequency_hz;
            let gain = state.source.gain_db;
            let passes = state.tle.upcoming_passes().to_vec();
            let ac_count = self.adsb_panel.aircraft.len();
            self.web_remote.broadcast_state(freq, gain, &mode, ac_count, &passes);
            self.mqtt.tick(freq, gain);
            self.mqtt.publish_passes(&passes);
            if !self.adsb_panel.aircraft.is_empty() {
                self.mqtt.publish_aircraft(&self.adsb_panel.aircraft);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Take a snapshot of the shared state once per frame
        let snapshot = {
            if let Ok(state) = self.shared.try_lock() {
                Some(SharedSnapshot {
                    frequency_hz: state.source.frequency_hz,
                    sample_rate_hz: state.source.sample_rate_hz,
                    gain_db: state.source.gain_db,
                    demod_mode: format!("{}", state.demod_mode.label()),
                    recording: state.recording,
                    source_running: state.source.status == crate::source_manager::SourceStatus::Running,
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
                ai: &mut self.ai_panel,
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
                ui.separator();
                let peak = state.spectrum.peak_level();
                let peak_color = if peak > -20.0 { egui::Color32::GREEN }
                    else if peak > -60.0 { egui::Color32::YELLOW }
                    else { egui::Color32::GRAY };
                ui.colored_label(peak_color, format!("Peak: {:.1} dB", peak));
            }
        });
    }
}

struct SharedSnapshot {
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
    demod_mode: String,
    recording: bool,
    source_running: bool,
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
    ai: &'a mut AiPanel,
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
                }
            }
            Tab::Satellite => self.satellite.ui(ui),
            Tab::AdsB => self.adsb.ui(ui),
            Tab::Recorder => self.recorder.ui(ui),
            Tab::AiAgent => self.ai.ui(ui),
            Tab::Bookmarks => {
                let snapshot = match &self.snapshot {
                    Some(s) => s,
                    None => return,
                };
                ui.heading("Frequency Bookmarks");
                ui.label(format!("{} bookmarks", snapshot.bookmarks.len()));
                ui.separator();

                let mut categories: Vec<_> = snapshot.bookmarks.iter().map(|b| b.category.clone()).collect();
                categories.sort();
                categories.dedup();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for cat in categories {
                        ui.collapsing(&cat, |ui| {
                            for bm in snapshot.bookmarks.iter().filter(|b| b.category == cat) {
                                ui.horizontal(|ui| {
                                    ui.label(&bm.name);
                                    ui.monospace(format!("{}", bm.freq_display()));
                                    ui.small(&bm.mode);
                                    if ui.small_button("Tune").clicked() {
                                        if let Ok(mut state) = self.shared.try_lock() {
                                            state.source.frequency_hz = bm.frequency_hz;
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
