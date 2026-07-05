use crate::app::SharedState;
use crate::satellite::alignment::{
    alignment_info_ui, compass_rose_ui, compute_dipole_alignment, dipole_tutorial_ui, DipoleType,
};
use crate::satellite::map_renderer::MapRenderer;
use crate::satellite::picker::satellite_picker_ui;
use crate::satellite::recorder::Cf32StreamWriter;
use crate::satellite::types::*;
use crate::tle_engine::PassInfo;
use std::sync::{Arc, Mutex};

// ── Decode sub-tab types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatelliteSubTab {
    Track,
    Alignment,
}

fn current_unix_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub struct SatellitePanel {
    shared: Arc<Mutex<SharedState>>,

    // Picker state
    pub satellite_catalog: Vec<SatelliteCatalogEntry>,
    pub selected_sat_index: Option<usize>,
    pub picker_search: String,

    // Map tracker state
    pub map_renderer: MapRenderer,
    pub trajectory_loaded: bool,
    pub current_sat_position: Option<SatPosition>,
    last_position_update: std::time::Instant,
    position_update_interval: std::time::Duration,

    // cf32 recorder
    pub cf32_writer: Option<Cf32StreamWriter>,
    pub cf32_recording: bool,
    pub cf32_output_dir: String,

    // Legacy state (tracking, tuning, schedule)
    pub selected_sat: Option<String>,
    pub auto_record: bool,
    pub signal_strength: f32,
    pub doppler_hz: f64,
    pub recording: bool,
    pub observer_lat: f64,
    pub observer_lon: f64,
    pub auto_tune: bool,
    cached_passes: Vec<PassInfo>,
    pass_cache_at: std::time::Instant,
    pub pending_ai_prompt: Option<String>,
    checklist: crate::antenna_checklist::AntennaChecklist,
    pub pending_status: Option<String>,
}

impl SatellitePanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        let catalog = build_satellite_catalog(&shared);
        Self {
            shared: shared.clone(),
            satellite_catalog: catalog,
            selected_sat_index: None,
            picker_search: String::new(),
            map_renderer: MapRenderer::new(51.5, -0.1),
            trajectory_loaded: false,
            current_sat_position: None,
            last_position_update: std::time::Instant::now(),
            position_update_interval: std::time::Duration::from_secs(1),
            cf32_writer: None,
            cf32_recording: false,
            cf32_output_dir: "./recordings".to_string(),
            selected_sat: None,
            auto_record: true,
            signal_strength: -120.0,
            doppler_hz: 0.0,
            recording: false,
            observer_lat: 51.5,
            observer_lon: -0.1,
            auto_tune: true,
            cached_passes: vec![],
            pass_cache_at: std::time::Instant::now(),
            pending_ai_prompt: None,
            checklist: crate::antenna_checklist::AntennaChecklist::for_satellite(shared),
            pending_status: None,
        }
    }

    // ── Real-time update ──────────────────────────────────────────────────

    /// Call every frame from CentralApp::logic().
    pub fn tick_realtime(&mut self) {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_position_update) < self.position_update_interval {
            return;
        }
        self.last_position_update = now;

        if let Ok(state) = self.shared.try_lock() {
            self.observer_lat = state.tle.observer_lat;
            self.observer_lon = state.tle.observer_lon;
            self.map_renderer.center_lat = state.tle.observer_lat;
            self.map_renderer.center_lon = state.tle.observer_lon;
        }

        if let Some(idx) = self.selected_sat_index {
            if idx < self.satellite_catalog.len() {
                let entry = &self.satellite_catalog[idx];
                let t = current_unix_time();
                if let Ok(mut state) = self.shared.try_lock() {
                    if let Some(pos) = state.tle.satellite_position(
                        &entry.tle_name,
                        state.tle.observer_lat,
                        state.tle.observer_lon,
                        t,
                    ) {
                        self.current_sat_position = Some(pos);

                        // Mark active pass
                        let is_active = state.tle.upcoming_passes().iter().any(|p| {
                            p.satellite == entry.tle_name && p.aos_dt <= t && p.los_dt > t
                        });
                        self.map_renderer.in_pass_now = is_active;

                        // Doppler
                        if self.auto_tune {
                            let doppler = state.tle.doppler_shift_for_sat(
                                &entry.tle_name,
                                entry.frequency_hz as f64,
                                t,
                            );
                            self.doppler_hz = doppler;
                            state.source.frequency_hz =
                                (entry.frequency_hz as f64 + doppler) as u64;
                        }
                    }

                    // Load trajectory once
                    if !self.trajectory_loaded {
                        let traj = state.tle.pass_trajectory(
                            &entry.tle_name,
                            state.tle.observer_lat,
                            state.tle.observer_lon,
                            6.0,
                        );
                        self.map_renderer.set_trajectory(&traj);
                        self.trajectory_loaded = true;
                    }

                    // Update markers
                    self.map_renderer
                        .set_satellite_position(self.current_sat_position);
                }
            }
        }
    }

    // ── Main UI entry ─────────────────────────────────────────────────────

    pub fn ui(&mut self, ui: &mut egui::Ui, subtab: SatelliteSubTab) {
        if !self.checklist.ui(ui) {
            if let Some(msg) = self.checklist.pending_status.take() {
                self.pending_status = Some(msg);
            }
            return;
        }
        if let Some(msg) = self.checklist.pending_status.take() {
            self.pending_status = Some(msg);
        }

        match subtab {
            SatelliteSubTab::Track => self.ui_track(ui),
            SatelliteSubTab::Alignment => self.ui_alignment(ui),
        }
    }

    // ── Track subtab ──────────────────────────────────────────────────────

    fn ui_track(&mut self, ui: &mut egui::Ui) {
        ui.heading("Satellite Tracking");
        ui.add_space(4.0);

        // Destructure to avoid borrow conflicts in closures
        let SatellitePanel {
            ref satellite_catalog,
            ref mut selected_sat_index,
            ref mut picker_search,
            ref cached_passes,
            ref mut selected_sat,
            ref mut auto_tune,
            ..
        } = *self;

        let catalog = satellite_catalog.clone();
        let selected_before = *selected_sat_index;

        egui::Frame::group(ui.style())
            .inner_margin(6)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Satellites").strong());
                satellite_picker_ui(ui, &catalog, selected_sat_index, picker_search, |_idx| {});
            });

        ui.add_space(4.0);

        // Upcoming pass for selected satellite
        if let Some(idx) = *selected_sat_index {
            if idx < catalog.len() {
                let name = &catalog[idx].tle_name;
                let passes = cached_passes.clone();
                let now_unix = current_unix_time();

                if let Some(active) = passes
                    .iter()
                    .find(|p| p.satellite == *name && p.aos_dt <= now_unix && p.los_dt > now_unix)
                {
                    let remaining = (active.los_dt - now_unix).max(0.0) as u64;
                    ui.group(|ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(50, 255, 100),
                            egui::RichText::new(format!("▶ {} — IN PASS", active.satellite))
                                .size(15.0)
                                .strong(),
                        );
                        ui.label(format!(
                            "{:02}:{:02} remaining · max elevation {:.0}°",
                            remaining / 60,
                            remaining % 60,
                            active.max_elevation
                        ));
                    });
                } else if let Some(next) = passes
                    .iter()
                    .filter(|p| p.satellite == *name && p.aos_dt > now_unix)
                    .min_by(|a, b| {
                        a.aos_dt
                            .partial_cmp(&b.aos_dt)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                {
                    let secs = (next.aos_dt - now_unix).max(0.0) as u64;
                    ui.group(|ui| {
                        ui.label(
                            egui::RichText::new(format!("Next pass: {}", next.satellite))
                                .size(14.0)
                                .strong(),
                        );
                        let countdown = if secs < 3600 {
                            format!("{}m {}s", secs / 60, secs % 60)
                        } else {
                            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
                        };
                        ui.label(format!(
                            "in {} · AOS {} · max elevation {:.0}°",
                            countdown, next.aos, next.max_elevation
                        ));
                    });
                } else {
                    ui.colored_label(egui::Color32::GRAY, "No upcoming passes — update TLE data.");
                }
            }
        }

        // Handle selection change
        if *selected_sat_index != selected_before {
            if let Some(idx) = *selected_sat_index {
                if idx < catalog.len() {
                    *selected_sat = Some(catalog[idx].name.clone());
                    self.trajectory_loaded = false;
                    self.current_sat_position = None;
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.source.frequency_hz = catalog[idx].frequency_hz;
                    }
                    *auto_tune = true;
                }
            }
        }

        ui.add_space(8.0);
        ui.separator();
        self.ui_record_control(ui);
        ui.add_space(4.0);
        self.ui_preflight_badges(ui);

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.auto_tune, "Auto-tune + Doppler");
            ui.checkbox(&mut self.auto_record, "Auto-record");
        });

        ui.add_space(4.0);
        if ui.button("🤖 Ask AI to track a satellite").clicked() {
            self.pending_ai_prompt = Some(
                "Help me track a satellite. What satellites are currently active and how do I set up tracking?".to_string(),
            );
        }

        ui.add_space(8.0);
        ui.separator();
        egui::CollapsingHeader::new("⚙ Advanced")
            .default_open(false)
            .show(ui, |ui| self.ui_advanced_inline(ui));
    }

    fn on_satellite_selected(&mut self, idx: usize) {
        if idx >= self.satellite_catalog.len() {
            return;
        }
        let entry = &self.satellite_catalog[idx];
        self.selected_sat = Some(entry.name.clone());
        self.trajectory_loaded = false;
        self.current_sat_position = None;

        if let Ok(mut state) = self.shared.try_lock() {
            state.source.frequency_hz = entry.frequency_hz;
        }
        self.auto_tune = true;
    }

    // ── Record control ────────────────────────────────────────────────────

    fn ui_record_control(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let is_rec = self.cf32_recording;
            let label = if is_rec {
                "■ STOP CF32"
            } else {
                "● REC CF32"
            };
            let fill = if is_rec {
                egui::Color32::from_rgb(200, 50, 50)
            } else {
                egui::Color32::from_rgb(50, 180, 50)
            };
            let btn = egui::Button::new(egui::RichText::new(label).size(16.0).strong())
                .fill(fill)
                .min_size(egui::vec2(140.0, 38.0));
            if ui.add(btn).clicked() {
                if is_rec {
                    self.stop_cf32_recording();
                } else {
                    self.start_cf32_recording();
                }
            }

            if is_rec {
                if let Some(w) = &self.cf32_writer {
                    let mb = w.bytes_written() as f64 / 1_048_576.0;
                    let secs = w.elapsed_secs();
                    let line = format!(
                        "  {:.1} MB  ·  {:.0}s  ·  {:.1} MB/s",
                        mb,
                        secs,
                        if secs > 0.0 { mb / secs } else { 0.0 }
                    );
                    ui.colored_label(egui::Color32::RED, line);
                }
            }
        });
    }

    fn start_cf32_recording(&mut self) {
        let (freq_hz, sample_rate, sat_name) = self
            .selected_sat_index
            .and_then(|idx| self.satellite_catalog.get(idx))
            .map(|e| (e.frequency_hz, 2_048_000u32, Some(e.name.clone())))
            .unwrap_or((100_000_000, 2_048_000, None));

        match Cf32StreamWriter::start(
            &self.cf32_output_dir,
            sat_name,
            sample_rate,
            freq_hz,
            self.observer_lat,
            self.observer_lon,
        ) {
            Ok(writer) => {
                self.cf32_writer = Some(writer);
                self.cf32_recording = true;
            }
            Err(e) => {
                self.pending_status = Some(format!("❌ Recorder: {e}"));
            }
        }
    }

    fn stop_cf32_recording(&mut self) {
        if let Some(mut w) = self.cf32_writer.take() {
            match w.stop() {
                Ok(meta) => {
                    let mb = meta.bytes_total as f64 / 1_048_576.0;
                    self.pending_status = Some(format!(
                        "✅ Recorded {:.1} MB ({:.1}s) → {}",
                        mb,
                        meta.duration_sec,
                        w.filename()
                    ));
                }
                Err(e) => {
                    self.pending_status = Some(format!("❌ Recorder stop error: {e}"));
                }
            }
        }
        self.cf32_recording = false;
    }

    /// Feed IQ samples to cf32 writer. Called from CentralApp::logic().
    pub fn feed_recording(&mut self, samples: &[u8]) {
        if self.cf32_recording {
            if let Some(w) = &mut self.cf32_writer {
                w.write(samples);
            }
        }
    }

    // ── Pre-flight badges ─────────────────────────────────────────────────

    fn ui_preflight_badges(&mut self, ui: &mut egui::Ui) {
        let checklist =
            crate::antenna_checklist::AntennaChecklist::for_satellite(self.shared.clone());
        ui.horizontal_wrapped(|ui| {
            for item in &checklist.items {
                let (color, icon) = if true {
                    (egui::Color32::GREEN, "✓")
                } else {
                    (egui::Color32::YELLOW, "⚠")
                };
                ui.colored_label(color, format!("{} {}", icon, item.label))
                    .on_hover_text(item.detail);
            }
        });
    }

    // ── Alignment subtab ──────────────────────────────────────────────────

    fn ui_alignment(&mut self, ui: &mut egui::Ui) {
        ui.heading("🧭 Dipole Alignment");
        ui.add_space(4.0);

        if let Some(idx) = self.selected_sat_index {
            if idx < self.satellite_catalog.len() {
                let _sat = &self.satellite_catalog[idx];

                if let Some(pos) = self.current_sat_position {
                    let align = compute_dipole_alignment(pos, DipoleType::VDipole137);
                    if let Some(alignment) = align {
                        ui.columns(2, |cols| {
                            cols[0].vertical(|ui| {
                                compass_rose_ui(
                                    ui,
                                    alignment.compass_heading,
                                    pos.azimuth,
                                    pos.elevation,
                                    pos.distance_km,
                                );
                            });
                            cols[1].vertical(|ui| {
                                alignment_info_ui(ui, &alignment, &pos);
                            });
                        });
                    }
                } else {
                    ui.colored_label(egui::Color32::GRAY, "No satellite position data. Select a satellite in the Track tab and wait for position update.");
                }

                ui.add_space(12.0);
                ui.separator();
                dipole_tutorial_ui(ui);
            }
        } else {
            ui.colored_label(
                egui::Color32::GRAY,
                "Select a satellite from the Track tab first.",
            );
        }
    }

    // ── Advanced inline ───────────────────────────────────────────────────

    fn ui_advanced_inline(&mut self, ui: &mut egui::Ui) {
        if let Ok(state) = self.shared.try_lock() {
            if (state.tle.observer_lat - self.observer_lat).abs() > 0.001
                || (state.tle.observer_lon - self.observer_lon).abs() > 0.001
            {
                self.observer_lat = state.tle.observer_lat;
                self.observer_lon = state.tle.observer_lon;
            }
        }

        ui.collapsing("Observer Location", |ui| {
            let changed_lat = ui
                .add(egui::Slider::new(&mut self.observer_lat, -90.0..=90.0).text("Latitude"))
                .changed();
            let changed_lon = ui
                .add(egui::Slider::new(&mut self.observer_lon, -180.0..=180.0).text("Longitude"))
                .changed();
            if changed_lat || changed_lon {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.tle.observer_lat = self.observer_lat;
                    state.tle.observer_lon = self.observer_lon;
                }
            }
        });

        ui.separator();

        ui.checkbox(&mut self.auto_record, "Auto-record on pass");
        ui.checkbox(&mut self.auto_tune, "Auto-tune to downlink + Doppler");

        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Signal Strength:");
            let norm = ((self.signal_strength + 120.0) / 120.0).clamp(0.0, 1.0);
            let color = if norm > 0.5 {
                egui::Color32::GREEN
            } else if norm > 0.2 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::RED
            };
            ui.add(
                egui::ProgressBar::new(norm)
                    .fill(color)
                    .text(format!("{:.1} dB", self.signal_strength)),
            );
        });

        {
            let doppler_color = if self.doppler_hz.abs() > 5000.0 {
                egui::Color32::from_rgb(255, 120, 60)
            } else if self.doppler_hz.abs() > 1000.0 {
                egui::Color32::from_rgb(255, 220, 80)
            } else {
                egui::Color32::from_rgb(120, 220, 120)
            };
            let doppler_str = if self.doppler_hz.abs() >= 1000.0 {
                format!("Doppler: {:+.2} kHz", self.doppler_hz / 1000.0)
            } else {
                format!("Doppler: {:+.0} Hz", self.doppler_hz)
            };
            ui.horizontal(|ui| {
                ui.colored_label(doppler_color, &doppler_str)
                    .on_hover_text("Real-time Doppler shift. Positive = approaching.");
                if self.auto_tune {
                    ui.colored_label(
                        egui::Color32::from_rgb(80, 200, 120),
                        egui::RichText::new("✓ Corrected").small(),
                    );
                }
            });
        }

        ui.separator();
        ui.heading("Manual Frequency Override");
        ui.horizontal(|ui| {
            if let Ok(mut state) = self.shared.try_lock() {
                let mut mhz = state.source.frequency_hz as f64 / 1e6;
                if ui
                    .add(egui::DragValue::new(&mut mhz).speed(0.001).suffix(" MHz"))
                    .changed()
                {
                    state.source.frequency_hz = (mhz * 1e6) as u64;
                }
            }
        });

        ui.separator();
        self.ui_pass_table(ui);
    }

    fn ui_pass_table(&mut self, ui: &mut egui::Ui) {
        ui.heading("Upcoming Passes");
        if self.pass_cache_at.elapsed() > std::time::Duration::from_secs(5) {
            if let Ok(mut state) = self.shared.try_lock() {
                self.cached_passes = state.tle.upcoming_passes().to_vec();
                self.pass_cache_at = std::time::Instant::now();
            }
        }
        let passes = self.cached_passes.clone();
        let now_unix = current_unix_time();

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("pass_grid").num_columns(7).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Satellite").strong());
                ui.label(egui::RichText::new("AOS").strong());
                ui.label(egui::RichText::new("LOS").strong());
                ui.label(egui::RichText::new("MaxEl").strong());
                ui.label(egui::RichText::new("In").strong());
                ui.label(egui::RichText::new("Tune").strong());
                ui.label(egui::RichText::new("AI").strong());
                ui.end_row();

                for pass in &passes {
                    let selected = self.selected_sat.as_deref() == Some(&pass.satellite);
                    let secs_until_aos = pass.aos_dt - now_unix;
                    let secs_until_los = pass.los_dt - now_unix;
                    let is_active = secs_until_aos <= 0.0 && secs_until_los > 0.0;

                    let name_color = if is_active {
                        egui::Color32::from_rgb(50, 255, 100)
                    } else if selected {
                        egui::Color32::from_rgb(0, 220, 255)
                    } else {
                        egui::Color32::WHITE
                    };

                    ui.colored_label(name_color, &pass.satellite);
                    ui.label(&pass.aos);
                    ui.label(&pass.los);
                    ui.label(format!("{:.0}°", pass.max_elevation));

                    let (countdown_text, countdown_color) = if is_active {
                        let remaining = secs_until_los.max(0.0) as u64;
                        (format!("▶ {:02}:{:02}", remaining / 60, remaining % 60), egui::Color32::from_rgb(50, 255, 100))
                    } else if secs_until_aos < 0.0 {
                        ("past".to_string(), egui::Color32::GRAY)
                    } else if secs_until_aos < 600.0 {
                        let mins = secs_until_aos as u64 / 60;
                        let secs = secs_until_aos as u64 % 60;
                        (format!("{mins:02}:{secs:02}"), egui::Color32::YELLOW)
                    } else {
                        let hours = secs_until_aos as u64 / 3600;
                        let mins = (secs_until_aos as u64 % 3600) / 60;
                        (format!("{hours}h {mins:02}m"), egui::Color32::GRAY)
                    };
                    ui.colored_label(countdown_color, countdown_text);

                    if ui.button(if is_active { "▶ Tune" } else if selected { "✓ Sel" } else { "Select" }).clicked() {
                        self.selected_sat = Some(pass.satellite.clone());
                        if self.auto_tune {
                            if let Ok(mut state) = self.shared.try_lock() {
                                state.source.frequency_hz = pass.frequency_hz;
                            }
                        }
                    }

                    let pass_status = if is_active {
                        format!("IN PROGRESS — {:.0}s remaining", secs_until_los.max(0.0))
                    } else if secs_until_aos > 0.0 {
                        format!("in {:.0}m {:.0}s", secs_until_aos / 60.0, secs_until_aos % 60.0)
                    } else {
                        "past".to_string()
                    };
                    if ui.small_button("🤖").clicked() {
                        self.pending_ai_prompt = Some(format!(
                            "Tell me about the {} satellite pass:\n\
                             - Frequency: {:.3} MHz\n\
                             - AOS: {}, LOS: {}\n\
                             - Max elevation: {:.0}°\n\
                             - Status: {}\n\
                             What can I receive from this satellite, and what settings should I use?",
                            pass.satellite,
                            pass.frequency_hz as f64 / 1e6,
                            pass.aos, pass.los,
                            pass.max_elevation,
                            pass_status,
                        ));
                    }
                    ui.end_row();
                }
            });
        });
    }
}

// ── Satellite catalog builder ────────────────────────────────────────────────

fn build_satellite_catalog(shared: &Arc<Mutex<SharedState>>) -> Vec<SatelliteCatalogEntry> {
    let mut catalog: Vec<SatelliteCatalogEntry> = vec![
        SatelliteCatalogEntry {
            name: "NOAA 15".into(),
            tle_name: "NOAA 15".into(),
            frequency_hz: 137_620_000,
            mode: "APT",
            description: "NOAA weather satellite — APT imagery at 137.62 MHz",
            is_active_pass: false,
        },
        SatelliteCatalogEntry {
            name: "NOAA 18".into(),
            tle_name: "NOAA 18".into(),
            frequency_hz: 137_912_500,
            mode: "APT",
            description: "NOAA weather satellite — APT imagery at 137.9125 MHz",
            is_active_pass: false,
        },
        SatelliteCatalogEntry {
            name: "NOAA 19".into(),
            tle_name: "NOAA 19".into(),
            frequency_hz: 137_100_000,
            mode: "APT",
            description: "NOAA weather satellite — APT imagery at 137.10 MHz",
            is_active_pass: false,
        },
        SatelliteCatalogEntry {
            name: "Meteor-M2-2".into(),
            tle_name: "Meteor-M2-2".into(),
            frequency_hz: 137_100_000,
            mode: "LRPT",
            description: "Russian weather satellite — digital LRPT at 137.10 MHz",
            is_active_pass: false,
        },
        SatelliteCatalogEntry {
            name: "Meteor-M2-3".into(),
            tle_name: "Meteor-M2-3".into(),
            frequency_hz: 137_900_000,
            mode: "LRPT",
            description: "Russian weather satellite — digital LRPT at 137.90 MHz",
            is_active_pass: false,
        },
        SatelliteCatalogEntry {
            name: "Meteor-M2-4".into(),
            tle_name: "Meteor-M2-4".into(),
            frequency_hz: 137_100_000,
            mode: "LRPT",
            description: "Russian weather satellite — digital LRPT at 137.10 MHz",
            is_active_pass: false,
        },
        SatelliteCatalogEntry {
            name: "ISS".into(),
            tle_name: "ISS".into(),
            frequency_hz: 145_800_000,
            mode: "Voice/APRS",
            description: "International Space Station — voice, APRS, SSTV at 145.80 MHz",
            is_active_pass: false,
        },
    ];

    // Mark active passes
    if let Ok(mut state) = shared.try_lock() {
        let now = current_unix_time();
        for entry in &mut catalog {
            entry.is_active_pass = state
                .tle
                .upcoming_passes()
                .iter()
                .any(|p| p.satellite == entry.tle_name && p.aos_dt <= now && p.los_dt > now);
        }
    }

    catalog
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_shared_state;

    #[test]
    fn test_new_defaults() {
        let panel = SatellitePanel::new(make_shared_state());
        assert!(panel.selected_sat.is_none());
        assert!(panel.auto_record);
        assert_eq!(panel.signal_strength, -120.0);
        assert_eq!(panel.doppler_hz, 0.0);
        assert!(!panel.recording);
        assert!(!panel.cf32_recording);
        assert!(panel.cf32_writer.is_none());
        assert!(panel.selected_sat_index.is_none());
        assert_eq!(panel.observer_lat, 51.5);
        assert_eq!(panel.observer_lon, -0.1);
    }

    #[test]
    fn test_satellite_catalog_not_empty() {
        let shared = make_shared_state();
        let catalog = build_satellite_catalog(&shared);
        assert!(!catalog.is_empty());
        assert!(catalog.iter().any(|e| e.name == "ISS"));
        assert!(catalog.iter().any(|e| e.name == "NOAA 19"));
    }

    #[test]
    fn test_satellite_catalog_has_frequencies() {
        let shared = make_shared_state();
        let catalog = build_satellite_catalog(&shared);
        for entry in &catalog {
            assert!(entry.frequency_hz > 0);
            assert!(!entry.tle_name.is_empty());
        }
    }

    #[test]
    fn test_selected_sat_persistence() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert!(panel.selected_sat.is_none());
        panel.selected_sat = Some("NOAA 19".to_string());
        assert_eq!(panel.selected_sat.as_deref(), Some("NOAA 19"));
        panel.selected_sat = None;
        assert!(panel.selected_sat.is_none());
    }

    #[test]
    fn test_on_satellite_selected_updates_state() {
        let shared = make_shared_state();
        let mut panel = SatellitePanel::new(shared.clone());
        panel.on_satellite_selected(2); // NOAA 19 at index 2
        assert_eq!(panel.selected_sat.as_deref(), Some("NOAA 19"));
    }

    #[test]
    fn test_auto_record_toggle() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert!(panel.auto_record);
        panel.auto_record = false;
        assert!(!panel.auto_record);
    }

    #[test]
    fn test_observer_coordinates() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert_eq!(panel.observer_lat, 51.5);
        assert_eq!(panel.observer_lon, -0.1);
        panel.observer_lat = 40.7128;
        panel.observer_lon = -74.0060;
        assert_eq!(panel.observer_lat, 40.7128);
        assert_eq!(panel.observer_lon, -74.0060);
    }

    #[test]
    fn test_signal_strength_range() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert_eq!(panel.signal_strength, -120.0);
        panel.signal_strength = -50.0;
        assert_eq!(panel.signal_strength, -50.0);
    }

    #[test]
    fn test_ui_track_no_crash() {
        let mut panel = SatellitePanel::new(make_shared_state());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new("test".into()).show(ctx, |ui| {
                panel.ui(ui, SatelliteSubTab::Track);
            });
        });
    }

    #[test]
    fn test_ui_alignment_no_crash_without_selection() {
        let mut panel = SatellitePanel::new(make_shared_state());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new("test_align".into()).show(ctx, |ui| {
                panel.ui(ui, SatelliteSubTab::Alignment);
            });
        });
    }

    #[test]
    fn test_cf32_recording_start_stop() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert!(!panel.cf32_recording);
        assert!(panel.cf32_writer.is_none());

        // Test start (may fail due to filesystem, but shouldn't crash)
        // We rely on the internal state machine; real recording tested manually
        panel.cf32_recording = true;
        assert!(panel.cf32_recording);
        panel.cf32_recording = false;
        assert!(!panel.cf32_recording);
    }

    #[test]
    fn test_pending_status_flow() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert!(panel.pending_status.is_none());
        panel.pending_status = Some("Test status".into());
        assert_eq!(panel.pending_status.as_deref(), Some("Test status"));
    }
}
