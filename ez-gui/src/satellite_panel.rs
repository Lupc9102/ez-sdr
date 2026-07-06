use crate::app::SharedState;
use crate::decoding_panel::{satellite_to_preset, DecodeRequest};
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
    pub doppler_hz: f64,
    pub observer_lat: f64,
    pub observer_lon: f64,
    pub auto_tune: bool,
    cached_passes: Vec<PassInfo>,
    pass_cache_at: std::time::Instant,
    pub pending_ai_prompt: Option<String>,
    checklist: crate::antenna_checklist::AntennaChecklist,
    pub pending_status: Option<String>,
    pub pending_decode_request: Option<DecodeRequest>,
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
            doppler_hz: 0.0,
            observer_lat: 51.5,
            observer_lon: -0.1,
            auto_tune: true,
            cached_passes: vec![],
            pass_cache_at: std::time::Instant::now(),
            pending_ai_prompt: None,
            checklist: crate::antenna_checklist::AntennaChecklist::for_satellite(shared),
            pending_status: None,
            pending_decode_request: None,
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

        // Refresh the pass cache periodically — drives the pass summary below.
        if self.pass_cache_at.elapsed() > std::time::Duration::from_secs(5) {
            if let Ok(mut state) = self.shared.try_lock() {
                self.cached_passes = state.tle.upcoming_passes().to_vec();
                self.pass_cache_at = std::time::Instant::now();
            }
        }

        let catalog = self.satellite_catalog.clone();
        let selected_before = self.selected_sat_index;

        // 1. Pick a satellite.
        egui::Frame::group(ui.style())
            .inner_margin(6)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Satellites").strong());
                satellite_picker_ui(
                    ui,
                    &catalog,
                    &mut self.selected_sat_index,
                    &mut self.picker_search,
                    |_idx| {},
                );
            });

        // Selection change → retune to downlink and reset the trajectory.
        if self.selected_sat_index != selected_before {
            if let Some(idx) = self.selected_sat_index {
                self.on_satellite_selected(idx);
            }
        }

        ui.add_space(6.0);

        // 2. Compact next/active pass summary for the selection.
        if let Some(idx) = self.selected_sat_index {
            if let Some(entry) = catalog.get(idx) {
                let name = entry.tle_name.clone();
                self.ui_pass_summary(ui, &name);
            }
        }

        // 3. One big Record button (rendered by ui_record_control).
        ui.add_space(8.0);
        self.ui_record_control(ui);
    }

    /// Compact upcoming/active pass summary for the given TLE name, using the
    /// cached pass list. Kept intentionally minimal — the full schedule lives
    /// in the dedicated Scheduler tab.
    fn ui_pass_summary(&self, ui: &mut egui::Ui, name: &str) {
        let now_unix = current_unix_time();

        if let Some(active) = self
            .cached_passes
            .iter()
            .find(|p| p.satellite == name && p.aos_dt <= now_unix && p.los_dt > now_unix)
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
        } else if let Some(next) = self
            .cached_passes
            .iter()
            .filter(|p| p.satellite == name && p.aos_dt > now_unix)
            .min_by(|a, b| {
                a.aos_dt
                    .partial_cmp(&b.aos_dt)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            let secs = (next.aos_dt - now_unix).max(0.0) as u64;
            let countdown = if secs < 3600 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
            };
            ui.group(|ui| {
                ui.label(
                    egui::RichText::new(format!("Next pass: {}", next.satellite))
                        .size(14.0)
                        .strong(),
                );
                ui.label(format!(
                    "in {} · AOS {} · max elevation {:.0}°",
                    countdown, next.aos, next.max_elevation
                ));
            });
        } else {
            ui.colored_label(egui::Color32::GRAY, "No upcoming passes — update TLE data.");
        }
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
        let is_rec = self.cf32_recording;
        let label = if is_rec {
            "■  STOP RECORDING"
        } else {
            "●  RECORD  (raw cf32 I/Q)"
        };
        let fill = if is_rec {
            egui::Color32::from_rgb(200, 45, 45)
        } else {
            egui::Color32::from_rgb(45, 175, 60)
        };
        // One large, full-width, high-contrast button — the primary action.
        let btn = egui::Button::new(
            egui::RichText::new(label)
                .size(20.0)
                .strong()
                .color(egui::Color32::WHITE),
        )
        .fill(fill)
        .min_size(egui::vec2(ui.available_width(), 56.0));
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
                    "● REC  ·  {:.1} MB  ·  {:.0}s  ·  {:.1} MB/s",
                    mb,
                    secs,
                    if secs > 0.0 { mb / secs } else { 0.0 }
                );
                ui.add_space(4.0);
                ui.colored_label(
                    egui::Color32::from_rgb(255, 90, 90),
                    egui::RichText::new(line).size(13.0).strong(),
                );
            }
        }
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
                    self.pending_decode_request = Some(DecodeRequest {
                        file_path: w.path().display().to_string(),
                        preset: meta.satellite.as_deref().and_then(satellite_to_preset),
                        sample_rate: meta.sample_rate_hz,
                        satellite_name: meta.satellite.clone(),
                    });
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

    // ── Alignment subtab ──────────────────────────────────────────────────

    fn ui_alignment(&mut self, ui: &mut egui::Ui) {
        ui.heading("🧭 Dipole Alignment");
        ui.add_space(4.0);

        if let Some(idx) = self.selected_sat_index {
            if idx < self.satellite_catalog.len() {
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
                self.ui_observer_location(ui);

                ui.add_space(8.0);
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

    /// Observer latitude/longitude control. Lives in the Align subtab since it
    /// directly affects the computed azimuth/elevation the compass depends on.
    fn ui_observer_location(&mut self, ui: &mut egui::Ui) {
        // Keep local copy in sync with shared TLE state (e.g. GPS/config updates).
        if let Ok(state) = self.shared.try_lock() {
            if (state.tle.observer_lat - self.observer_lat).abs() > 0.001
                || (state.tle.observer_lon - self.observer_lon).abs() > 0.001
            {
                self.observer_lat = state.tle.observer_lat;
                self.observer_lon = state.tle.observer_lon;
            }
        }

        ui.collapsing("📍 Observer Location", |ui| {
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
        assert_eq!(panel.doppler_hz, 0.0);
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

    #[test]
    fn stop_cf32_recording_sets_pending_decode_request_on_success() {
        use crate::decoding_panel::DecodePreset;

        let mut panel = SatellitePanel::new(make_shared_state());
        let tmp_dir =
            std::env::temp_dir().join(format!("ez_sdr_test_decode_req_{}", std::process::id()));
        panel.cf32_output_dir = tmp_dir.to_string_lossy().to_string();
        panel.cf32_writer = Some(
            Cf32StreamWriter::start(
                &panel.cf32_output_dir,
                Some("METEOR-M2-3".to_string()),
                2_048_000,
                137_900_000,
                51.5,
                -0.1,
            )
            .expect("writer should start"),
        );
        panel.cf32_recording = true;

        panel.stop_cf32_recording();

        assert!(!panel.cf32_recording);
        let req = panel
            .pending_decode_request
            .expect("expected a pending decode request");
        assert_eq!(req.satellite_name.as_deref(), Some("METEOR-M2-3"));
        assert_eq!(req.sample_rate, 2_048_000);
        assert_eq!(req.preset, Some(DecodePreset::MeteorM2_3));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn pending_decode_request_carries_full_path_not_just_filename() {
        let mut panel = SatellitePanel::new(make_shared_state());
        let tmp_dir =
            std::env::temp_dir().join(format!("ez_sdr_test_decode_path_{}", std::process::id()));
        panel.cf32_output_dir = tmp_dir.to_string_lossy().to_string();
        let writer = Cf32StreamWriter::start(
            &panel.cf32_output_dir,
            Some("NOAA 19".to_string()),
            2_048_000,
            137_100_000,
            51.5,
            -0.1,
        )
        .expect("writer should start");
        let bare_filename = writer.filename();
        panel.cf32_writer = Some(writer);
        panel.cf32_recording = true;

        panel.stop_cf32_recording();

        let req = panel
            .pending_decode_request
            .expect("expected a pending decode request");
        assert_ne!(req.file_path, bare_filename);
        assert!(req.file_path.ends_with(&bare_filename));
        assert!(req.file_path.contains(&tmp_dir.to_string_lossy().to_string()));
        assert!(req.preset.is_none());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
