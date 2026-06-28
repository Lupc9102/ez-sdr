use std::sync::{Arc, Mutex};

use egui::Context;
use egui_dock::{DockArea, DockState, NodeIndex, Style, SurfaceIndex};

use crate::adsb_panel::AdsBPanel;
use crate::ai_panel::AiPanel;
use crate::bookmarks::BookmarkDb;
use crate::config::AppConfig;
use crate::database::Database;
use crate::mqtt::MqttPublisher;
use crate::recorder_panel::RecorderPanel;
use crate::satellite_panel::SatellitePanel;
use crate::scheduler::Scheduler;
use crate::sdr_panel::SdrPanel;
use crate::source_manager::SourceManager;
use crate::spectrum::SpectrumAnalyzer;
use crate::tle_engine::TleEngine;
use crate::web_remote::{RemoteCommand, WebRemote};

#[derive(Debug, Clone, PartialEq)]
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
}

impl CentralApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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

        let mut dock_state = DockState::new(vec![
            Tab::Spectrum,
            Tab::Sdr,
            Tab::Satellite,
            Tab::AdsB,
            Tab::Recorder,
            Tab::AiAgent,
        ]);

        let surface = SurfaceIndex::main();
        let _root = dock_state[surface].root_node().unwrap();
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
        }
    }
}

impl eframe::App for CentralApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Check if source is running
        let source_running = {
            if let Ok(state) = self.shared.try_lock() {
                state.source.status == crate::source_manager::SourceStatus::Running
            } else {
                false
            }
        };

        // Drain samples from source (fast, minimal lock hold)
        let mut sample_batch: Vec<Vec<u8>> = Vec::new();
        {
            let mut state = self.shared.lock().unwrap();
            state.source.poll();
            for _ in 0..4 {
                if let Some(samples) = state.source.recv_samples() {
                    sample_batch.push(samples);
                } else {
                    break;
                }
            }
        }

        // Process samples outside lock
        for samples in &sample_batch {
            if let Ok(mut state) = self.shared.try_lock() {
                state.spectrum.push_iq_samples(samples);
            }
            self.recorder_panel.write_samples(samples);
        }

        // Only repaint continuously when source is running
        if source_running {
            ctx.request_repaint();
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

        // Broadcast state + MQTT tick (cheap — passes are cached now)
        {
            if let Ok(mut state) = self.shared.try_lock() {
                let mode = format!("{:?}", state.demod_mode);
                let freq = state.source.frequency_hz;
                let gain = state.source.gain_db;
                let passes = state.tle.upcoming_passes().to_vec();
                let ac_count = self.adsb_panel.aircraft.len();
                self.web_remote.broadcast_state(freq, gain, &mode, ac_count, &passes);
                self.mqtt.tick(freq, gain);
                self.mqtt.publish_passes(&passes);
            }
        }
        if !self.adsb_panel.aircraft.is_empty() {
            self.mqtt.publish_aircraft(&self.adsb_panel.aircraft);
        }

        let style = Style::from_egui(ctx.style().as_ref());
        DockArea::new(&mut self.dock_state)
            .style(style)
            .show(ctx, &mut TabViewer {
                shared: self.shared.clone(),
                sdr: &mut self.sdr_panel,
                satellite: &mut self.satellite_panel,
                adsb: &mut self.adsb_panel,
                recorder: &mut self.recorder_panel,
                ai: &mut self.ai_panel,
            });
    }
}

struct TabViewer<'a> {
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
                let bookmarks_clone = {
                    if let Ok(state) = self.shared.try_lock() {
                        state.bookmarks.bookmarks.clone()
                    } else {
                        return;
                    }
                };
                ui.heading("Frequency Bookmarks");
                let categories: std::collections::HashSet<_> = bookmarks_clone.iter().map(|b| b.category.clone()).collect();
                for cat in categories {
                    ui.collapsing(&cat, |ui| {
                        for bm in bookmarks_clone.iter().filter(|b| b.category == cat) {
                            ui.horizontal(|ui| {
                                ui.label(&bm.name);
                                ui.monospace(format!("{:.3} MHz", bm.frequency_hz as f64 / 1e6));
                                if ui.button("Tune").clicked() {
                                    if let Ok(mut state) = self.shared.try_lock() {
                                        state.source.frequency_hz = bm.frequency_hz;
                                    }
                                }
                            });
                        }
                    });
                }
            }
            Tab::Scheduler => {
                let jobs_clone = {
                    if let Ok(state) = self.shared.try_lock() {
                        state.scheduler.jobs.clone()
                    } else {
                        return;
                    }
                };
                ui.heading("Scheduler");
                if ui.button("Refresh TLEs + compute passes").clicked() {
                    // TODO
                }
                for job in &jobs_clone {
                    ui.label(format!("{}  {} → {}", job.satellite, job.aos, job.los));
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
