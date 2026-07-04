use crate::app::SharedState;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ── Decode sub-tab types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatelliteSubTab {
    Track,
    Decode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodePreset {
    MeteorM2_2,
    MeteorM2_3,
    MeteorM2_4,
    Custom,
}

impl DecodePreset {
    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::MeteorM2_2 | Self::MeteorM2_3 | Self::MeteorM2_4 => 2_048_000,
            Self::Custom => 2_048_000,
        }
    }

    pub fn symbol_rate(&self) -> u32 {
        match self {
            Self::MeteorM2_2 | Self::MeteorM2_3 | Self::MeteorM2_4 => 72_000,
            Self::Custom => 72_000,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::MeteorM2_2 => "Meteor-M2-2 (137.1 MHz, LRPT)",
            Self::MeteorM2_3 => "Meteor-M2-3 (137.9 MHz, LRPT)",
            Self::MeteorM2_4 => "Meteor-M2-4 (137.1 MHz, LRPT)",
            Self::Custom => "Custom",
        }
    }

    pub fn all() -> &'static [DecodePreset] {
        &[
            Self::MeteorM2_2,
            Self::MeteorM2_3,
            Self::MeteorM2_4,
            Self::Custom,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct DecodeResultDisplay {
    pub satellite: String,
    pub lines_decoded: u32,
    pub rs_ok: u32,
    pub rs_failed: u32,
    pub image_paths: Vec<String>,
    pub elapsed_ms: u64,
}

/// Internal message sent from decode thread when finished.
struct DecodeThreadResult {
    result: Option<DecodeResultDisplay>,
    error: Option<String>,
    /// Decoded channel images keyed by APID (64=ch1..69=ch6).
    channels: Vec<(u16, image::GrayImage)>,
}

pub struct SatellitePanel {
    shared: Arc<Mutex<SharedState>>,
    pub selected_sat: Option<String>,
    pub auto_record: bool,
    pub signal_strength: f32,
    pub doppler_hz: f64,
    pub recording: bool,
    pub live_decode: bool,
    pub observer_lat: f64,
    pub observer_lon: f64,
    pub auto_tune: bool,
    cached_passes: Vec<crate::tle_engine::PassInfo>,
    pass_cache_at: std::time::Instant,
    pub pending_ai_prompt: Option<String>,
    checklist: crate::antenna_checklist::AntennaChecklist,
    pub pending_status: Option<String>,
    // Decode pipeline state
    pub decode_file_path: String,
    pub decode_running: bool,
    pub decode_progress: Option<lrpt_decode::DecodeProgress>,
    pub decode_result: Option<DecodeResultDisplay>,
    pub decode_satellite_preset: DecodePreset,
    pub decode_sample_rate: u32,
    pub decode_symbol_rate: u32,
    pub decode_output_dir: String,
    pub decode_error: Option<String>,
    decode_rx: Option<crossbeam_channel::Receiver<lrpt_decode::DecodeProgress>>,
    decode_done_rx: Option<crossbeam_channel::Receiver<DecodeThreadResult>>,
    decode_handle: Option<std::thread::JoinHandle<()>>,
    /// Channels from the most recent decode, ready for the editor.
    pub decoded_channels: Vec<(u16, image::GrayImage)>,
    pub pending_decode_tab: bool,
}

impl SatellitePanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared: shared.clone(),
            selected_sat: None,
            auto_record: true,
            signal_strength: -120.0,
            doppler_hz: 0.0,
            recording: false,
            live_decode: false,
            observer_lat: 51.5,
            observer_lon: -0.1,
            auto_tune: true,
            cached_passes: vec![],
            pass_cache_at: std::time::Instant::now(),
            pending_ai_prompt: None,
            checklist: crate::antenna_checklist::AntennaChecklist::for_satellite(shared),
            pending_status: None,
            // Decode pipeline
            decode_file_path: String::new(),
            decode_running: false,
            decode_progress: None,
            decode_result: None,
            decode_satellite_preset: DecodePreset::MeteorM2_2,
            decode_sample_rate: 2_048_000,
            decode_symbol_rate: 72_000,
            decode_output_dir: "./decoded".to_string(),
            decode_error: None,
            decode_rx: None,
            decode_done_rx: None,
            decode_handle: None,
            decoded_channels: Vec::new(),
            pending_decode_tab: false,
        }
    }

    /// SatDump-style simplified view: preset picker + pass tracking, front and center.
    pub fn ui_simple(&mut self, ui: &mut egui::Ui) {
        if !self.checklist.ui(ui) {
            if let Some(msg) = self.checklist.pending_status.take() {
                self.pending_status = Some(msg);
            }
            return;
        }
        if let Some(msg) = self.checklist.pending_status.take() {
            self.pending_status = Some(msg);
        }

        ui.heading("Satellite Tracking");
        ui.add_space(4.0);

        // Satellite preset picker — one-click track/tune
        ui.label(egui::RichText::new("Satellites").strong());
        let presets: &[(&str, u64, &str)] = &[
            ("NOAA 15", 137_620_000, "APT weather imagery"),
            ("NOAA 18", 137_912_500, "APT weather imagery"),
            ("NOAA 19", 137_100_000, "APT weather imagery"),
            ("Meteor-M2-2", 137_100_000, "LRPT weather imagery"),
            ("ISS", 145_800_000, "Voice / APRS / SSTV"),
        ];
        egui::Grid::new("sat_preset_grid")
            .num_columns(1)
            .spacing([0.0, 4.0])
            .show(ui, |ui| {
                for (name, freq_hz, desc) in presets {
                    let selected = self.selected_sat.as_deref() == Some(*name);
                    let fg = if selected {
                        egui::Color32::BLACK
                    } else {
                        egui::Color32::from_rgb(210, 220, 235)
                    };
                    let bg = if selected {
                        egui::Color32::from_rgb(0, 168, 255)
                    } else {
                        egui::Color32::from_rgb(24, 30, 40)
                    };
                    let btn = egui::Button::new(
                        egui::RichText::new(format!(
                            "🛰 {}\n{:.3} MHz — {}",
                            name,
                            *freq_hz as f64 / 1e6,
                            desc
                        ))
                        .color(fg)
                        .size(13.0),
                    )
                    .fill(bg)
                    .min_size(egui::vec2(ui.available_width(), 40.0));
                    if ui
                        .add(btn)
                        .on_hover_text(format!("Track {name} and tune to its downlink frequency"))
                        .clicked()
                    {
                        self.selected_sat = Some(name.to_string());
                        if let Ok(mut state) = self.shared.try_lock() {
                            state.source.frequency_hz = *freq_hz;
                        }
                        self.auto_tune = true;
                    }
                    ui.end_row();
                }
            });

        if self.selected_sat.as_deref() == Some("Meteor-M2-2") {
            ui.add_space(2.0);
            if ui
                .button("📡 Decode LRPT from Meteor-M2-2")
                .on_hover_text("Jump to the Decode tab with the Meteor-M2-2 LRPT preset pre-filled")
                .clicked()
            {
                self.request_decode_tab(DecodePreset::MeteorM2_2);
            }
        }

        ui.add_space(8.0);
        ui.separator();

        // Pass tracking, front and center
        if self.pass_cache_at.elapsed() > std::time::Duration::from_secs(5) {
            if let Ok(mut state) = self.shared.try_lock() {
                self.cached_passes = state.tle.upcoming_passes().to_vec();
                self.pass_cache_at = std::time::Instant::now();
            }
        }
        let passes = self.cached_passes.clone();
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_secs_f64())
            .unwrap_or(0.0);

        if let Some(active) = passes
            .iter()
            .find(|pas| pas.aos_dt <= now_unix && pas.los_dt > now_unix)
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
                if self.auto_tune {
                    ui.colored_label(
                        egui::Color32::from_rgb(80, 200, 120),
                        "✓ Auto-tune & Doppler correction active",
                    );
                }
            });
        } else if let Some(next) =
            passes
                .iter()
                .filter(|pas| pas.aos_dt > now_unix)
                .min_by(|pa, pb| {
                    pa.aos_dt
                        .partial_cmp(&pb.aos_dt)
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
                if ui
                    .button("▶ Track this pass")
                    .on_hover_text(
                        "Select this satellite and enable auto-tune for the upcoming pass",
                    )
                    .clicked()
                {
                    self.selected_sat = Some(next.satellite.clone());
                    self.auto_tune = true;
                }
            });
        } else {
            ui.colored_label(
                egui::Color32::GRAY,
                "No upcoming passes — update TLE data or check observer location in Advanced.",
            );
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.auto_tune, "Auto-tune + Doppler");
            ui.checkbox(&mut self.auto_record, "Auto-record");
        });

        ui.add_space(4.0);
        if ui.button("🤖 Ask AI to track a satellite").clicked() {
            self.pending_ai_prompt = Some("Help me track a satellite. What satellites are currently active and how do I set up tracking?".to_string());
        }

        // Advanced settings (observer location, TLE-driven pass table, Doppler
        // detail, manual frequency override) folded into the Track panel now
        // that the dedicated Advanced sub-tab has been removed.
        ui.add_space(8.0);
        ui.separator();
        egui::CollapsingHeader::new("⚙ Advanced (observer, TLE, pass table)")
            .default_open(false)
            .show(ui, |ui| self.ui_advanced_inline(ui));
    }

    /// Advanced settings inlined at the bottom of `ui_simple`: Doppler detail,
    /// manual observer/frequency overrides, full pass table.
    fn ui_advanced_inline(&mut self, ui: &mut egui::Ui) {
        // Sync observer location from shared state (e.g., when Settings → Save applies config values)
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
        ui.checkbox(&mut self.live_decode, "Live decode (LRPT/APT)");

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
                    .on_hover_text("Real-time Doppler shift applied to the receive frequency. Positive = satellite approaching. Negative = receding. Automatically corrected when auto-tune is on.");
                if self.auto_tune {
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 120),
                        egui::RichText::new("✓ Corrected").small())
                        .on_hover_text("Doppler correction is active. The SDR frequency is continuously adjusted to compensate.");
                }
            });
        }

        ui.horizontal(|ui| {
            if ui.button("Start Recording").clicked() {
                self.recording = true;
            }
            if ui.button("Stop Recording").clicked() {
                self.recording = false;
            }
            ui.label(if self.recording { "● RECORDING" } else { "" });
        });

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
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_secs_f64())
            .unwrap_or(0.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("pass_grid").num_columns(7).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Satellite").strong())
                    .on_hover_text("Satellite name from TLE catalog");
                ui.label(egui::RichText::new("AOS").strong())
                    .on_hover_text("Acquisition of Signal — when the satellite rises above your horizon");
                ui.label(egui::RichText::new("LOS").strong())
                    .on_hover_text("Loss of Signal — when the satellite sets below your horizon");
                ui.label(egui::RichText::new("MaxEl").strong())
                    .on_hover_text("Maximum elevation above horizon during the pass. >20° = good pass. >45° = excellent.");
                ui.label(egui::RichText::new("In").strong())
                    .on_hover_text("Time remaining until AOS. Green = pass in progress. Yellow = within 10 minutes.");
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
                    ui.label(&pass.aos).on_hover_text("Local time of AOS");
                    ui.label(&pass.los).on_hover_text("Local time of LOS");
                    ui.label(format!("{:.0}°", pass.max_elevation))
                        .on_hover_text(if pass.max_elevation > 45.0 { "Excellent pass — overhead!" } else if pass.max_elevation > 20.0 { "Good pass" } else { "Low pass — horizon obstructions may affect signal" });

                    // Countdown
                    let (countdown_text, countdown_color) = if is_active {
                        let remaining = secs_until_los.max(0.0) as u64;
                        let mins = remaining / 60;
                        let secs = remaining % 60;
                        (format!("▶ {mins:02}:{secs:02}"), egui::Color32::from_rgb(50, 255, 100))
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
                    ui.colored_label(countdown_color, countdown_text)
                        .on_hover_text(if is_active { "Pass in progress!" } else { "Time until AOS" });

                    if ui.button(if is_active { "▶ Tune" } else if selected { "✓ Sel" } else { "Select" })
                        .on_hover_text(format!("Tune to {:.3} MHz for this satellite", pass.frequency_hz as f64 / 1e6))
                        .clicked()
                    {
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
                    if ui.small_button("🤖")
                        .on_hover_text(format!("Ask AI about {} pass", pass.satellite))
                        .clicked()
                    {
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

    // ── Decode pipeline methods ─────────────────────────────────────────────

    /// Called from Track sub-tab to jump to Decode with a preset pre-filled.
    pub fn request_decode_tab(&mut self, preset: DecodePreset) {
        self.decode_satellite_preset = preset;
        self.pending_decode_tab = true;
    }

    fn start_decode(&mut self) {
        let path = PathBuf::from(&self.decode_file_path);
        let sample_rate = self.decode_sample_rate;
        let symbol_rate = self.decode_symbol_rate;
        let output_dir = PathBuf::from(&self.decode_output_dir);
        let preset_label = self.decode_satellite_preset.label().to_string();

        self.decode_running = true;
        self.decode_progress = None;
        self.decode_result = None;
        self.decode_error = None;

        let (progress_tx, progress_rx) = crossbeam_channel::unbounded();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        self.decode_rx = Some(progress_rx);
        self.decode_done_rx = Some(done_rx);

        let handle = std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&output_dir);
            let start = std::time::Instant::now();

            let result = lrpt_decode::decode_file(&path, sample_rate, symbol_rate, progress_tx);

            let thread_result = match result {
                Ok(decode_result) => {
                    let mut image_paths = Vec::new();
                    let mut channels = Vec::new();
                    for (apid, img) in &decode_result.images {
                        let filename = format!(
                            "{}_apid{}.png",
                            path.file_stem()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            apid
                        );
                        let img_path = output_dir.join(&filename);
                        let _ = img.save(&img_path);
                        image_paths.push(img_path.display().to_string());
                        channels.push((*apid, img.clone()));
                    }

                    let elapsed = start.elapsed().as_millis() as u64;
                    DecodeThreadResult {
                        result: Some(DecodeResultDisplay {
                            satellite: preset_label,
                            lines_decoded: decode_result
                                .images
                                .iter()
                                .map(|(_, img)| img.height())
                                .max()
                                .unwrap_or(0),
                            rs_ok: decode_result.rs_ok,
                            rs_failed: decode_result.rs_failed,
                            image_paths,
                            elapsed_ms: elapsed,
                        }),
                        error: None,
                        channels,
                    }
                }
                Err(e) => DecodeThreadResult {
                    result: None,
                    error: Some(format!("{e}")),
                    channels: Vec::new(),
                },
            };

            let _ = done_tx.send(thread_result);
        });

        self.decode_handle = Some(handle);
    }

    /// Poll decode thread for progress/completion. Call every UI frame.
    pub fn tick_decode(&mut self) {
        if !self.decode_running {
            return;
        }

        // Drain progress updates
        if let Some(ref rx) = self.decode_rx {
            while let Ok(progress) = rx.try_recv() {
                self.decode_progress = Some(progress);
            }
        }

        // Check for completion
        if let Some(ref rx) = self.decode_done_rx {
            if let Ok(thread_result) = rx.try_recv() {
                self.decode_running = false;
                self.decode_rx = None;
                self.decode_done_rx = None;
                if let Some(h) = self.decode_handle.take() {
                    let _ = h.join();
                }

                if let Some(result) = thread_result.result {
                    let total = result.rs_ok + result.rs_failed;
                    let pct = if total > 0 {
                        result.rs_ok as f32 / total as f32 * 100.0
                    } else {
                        0.0
                    };
                    self.pending_status = Some(format!(
                        "✅ Decoded {} — {} lines, RS {:.1}%",
                        result.satellite, result.lines_decoded, pct
                    ));
                    self.decoded_channels = thread_result.channels;
                    self.decode_result = Some(result);
                } else if let Some(err) = thread_result.error {
                    self.pending_status = Some(format!("❌ Decode failed: {err}"));
                    self.decode_error = Some(err);
                }
            }
        }
    }

    pub fn ui_decode(&mut self, ui: &mut egui::Ui) {
        ui.heading("Satellite Signal Decode");
        ui.add_space(4.0);

        // ── Preset Picker ──
        ui.label(egui::RichText::new("Preset").strong());
        egui::ComboBox::from_id_salt("decode_preset")
            .selected_text(self.decode_satellite_preset.label())
            .show_ui(ui, |ui| {
                for preset in DecodePreset::all() {
                    ui.selectable_value(
                        &mut self.decode_satellite_preset,
                        *preset,
                        preset.label(),
                    );
                }
            });

        ui.add_space(8.0);
        ui.separator();

        // ── File Import ──
        ui.label(egui::RichText::new("Import Recording").strong());
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.decode_file_path)
                .on_hover_text("Path to .iq recording file");
            if ui.button("Browse...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("IQ Recording", &["iq"])
                    .add_filter("All Files", &["*"])
                    .pick_file()
                {
                    self.decode_file_path = path.display().to_string();
                    if let Some(sidecar) = lrpt_decode::load_sidecar(&path) {
                        self.decode_sample_rate = sidecar.sample_rate_hz;
                        match sidecar.frequency_hz {
                            137_100_000..=137_110_000 => {
                                self.decode_satellite_preset = DecodePreset::MeteorM2_2;
                            }
                            137_900_000..=137_920_000 => {
                                self.decode_satellite_preset = DecodePreset::MeteorM2_3;
                            }
                            _ => {
                                self.decode_satellite_preset = DecodePreset::Custom;
                            }
                        }
                    }
                }
            }
        });

        // File info
        if !self.decode_file_path.is_empty() {
            let path = std::path::Path::new(&self.decode_file_path);
            if path.exists() {
                let size_mb = std::fs::metadata(path)
                    .map(|m| m.len() as f64 / 1e6)
                    .unwrap_or(0.0);
                ui.label(format!(
                    "  {} ({:.1} MB)",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    size_mb
                ));
            } else {
                ui.colored_label(egui::Color32::RED, "  File not found");
            }
        }

        ui.add_space(8.0);
        ui.separator();

        // ── Decode Parameters (collapsible) ──
        ui.collapsing("Decode Parameters", |ui| {
            egui::Grid::new("decode_params")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Sample Rate:");
                    ui.add(
                        egui::DragValue::new(&mut self.decode_sample_rate).suffix(" Hz"),
                    );
                    ui.end_row();
                    ui.label("Symbol Rate:");
                    ui.add(
                        egui::DragValue::new(&mut self.decode_symbol_rate).suffix(" Hz"),
                    );
                    ui.end_row();
                });
        });

        ui.add_space(8.0);

        // ── Output Directory ──
        ui.horizontal(|ui| {
            ui.label("Output:");
            ui.text_edit_singleline(&mut self.decode_output_dir);
            if ui
                .button("📁")
                .on_hover_text("Select output directory")
                .clicked()
            {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.decode_output_dir = dir.display().to_string();
                }
            }
        });

        ui.add_space(8.0);

        // ── Decode Button ──
        let can_decode = !self.decode_file_path.is_empty()
            && std::path::Path::new(&self.decode_file_path).exists()
            && !self.decode_running;

        if can_decode {
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("▶ Decode")
                            .size(15.0)
                            .strong(),
                    )
                    .min_size(egui::vec2(ui.available_width(), 32.0)),
                )
                .clicked()
            {
                self.start_decode();
            }
        } else if self.decode_running {
            ui.colored_label(egui::Color32::YELLOW, "⏳ Decoding...");
        }

        // ── Progress ──
        if let Some(ref progress) = self.decode_progress {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Decode Progress").strong());
                let total = progress.rs_ok + progress.rs_failed;
                let pct = if total > 0 {
                    progress.rs_ok as f32 / total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(pct).text(format!(
                        "RS OK: {} / {} ({:.1}%)",
                        progress.rs_ok,
                        total,
                        pct * 100.0
                    )),
                );
                ui.label(format!("Lines decoded: {}", progress.lines_decoded));
                ui.horizontal(|ui| {
                    ui.label(if progress.costas_locked {
                        "🟢 Costas locked"
                    } else {
                        "🔴 Costas unlocked"
                    });
                    ui.separator();
                    ui.label(if progress.frame_locked {
                        "🟢 Frame sync"
                    } else {
                        "🔴 Frame unlock"
                    });
                });

                for (apid, img) in &progress.preview {
                    ui.add_space(4.0);
                    ui.label(format!(
                        "APID {} — {}×{} px, {} lines",
                        apid,
                        img.width(),
                        img.height(),
                        img.height()
                    ));
                }
            });
        }

        // ── Result ──
        if let Some(ref result) = self.decode_result {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.colored_label(
                    egui::Color32::from_rgb(50, 255, 100),
                    egui::RichText::new("✅ Decode Complete").strong(),
                );
                ui.label(format!("Satellite: {}", result.satellite));
                ui.label(format!(
                    "Lines: {} · RS OK: {} · RS Failed: {}",
                    result.lines_decoded, result.rs_ok, result.rs_failed
                ));
                ui.label(format!("Time: {} ms", result.elapsed_ms));
                if !result.image_paths.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Output Images:").strong());
                    for path in &result.image_paths {
                        ui.label(format!("  📷 {path}"));
                    }
                    if ui.button("📂 Open Output Folder").clicked() {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(&self.decode_output_dir)
                            .spawn();
                    }
                }
            });
        }

        // ── Error ──
        if let Some(ref err) = self.decode_error {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.colored_label(
                    egui::Color32::RED,
                    egui::RichText::new("❌ Decode Error").strong(),
                );
                ui.label(err.as_str());
            });
        }

        ui.add_space(8.0);
        ui.separator();

        // ── Recent Recordings ──
        ui.collapsing("Recent Recordings", |ui| {
            let recordings_dir = std::path::Path::new("./recordings");
            if recordings_dir.exists() {
                let mut files: Vec<_> = std::fs::read_dir(recordings_dir)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map(|ext| ext == "iq")
                            .unwrap_or(false)
                    })
                    .collect();
                files.sort_by(|a, b| {
                    b.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH)
                        .cmp(
                            &a.metadata()
                                .and_then(|m| m.modified())
                                .unwrap_or(std::time::UNIX_EPOCH),
                        )
                });

                for entry in files.iter().take(10) {
                    let path = entry.path();
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let size = entry
                        .metadata()
                        .map(|m| m.len() as f64 / 1e6)
                        .unwrap_or(0.0);
                    if ui
                        .selectable_label(false, format!("{name} ({size:.1} MB)"))
                        .clicked()
                    {
                        self.decode_file_path = path.display().to_string();
                        if let Some(sidecar) = lrpt_decode::load_sidecar(&path) {
                            self.decode_sample_rate = sidecar.sample_rate_hz;
                            match sidecar.frequency_hz {
                                137_100_000..=137_110_000 => {
                                    self.decode_satellite_preset = DecodePreset::MeteorM2_2;
                                }
                                137_900_000..=137_920_000 => {
                                    self.decode_satellite_preset = DecodePreset::MeteorM2_3;
                                }
                                _ => {
                                    self.decode_satellite_preset = DecodePreset::Custom;
                                }
                            }
                        }
                    }
                }
            } else {
                ui.label("No recordings directory found");
            }
        });
    }
}
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
        assert!(!panel.live_decode);
        assert_eq!(panel.observer_lat, 51.5);
        assert_eq!(panel.observer_lon, -0.1);
        assert!(panel.auto_tune);
        assert!(panel.pending_ai_prompt.is_none());
        assert!(panel.pending_status.is_none());
        // Decode fields
        assert!(!panel.decode_running);
        assert!(panel.decode_file_path.is_empty());
        assert_eq!(panel.decode_satellite_preset, DecodePreset::MeteorM2_2);
        assert_eq!(panel.decode_sample_rate, 2_048_000);
        assert_eq!(panel.decode_symbol_rate, 72_000);
        assert!(!panel.pending_decode_tab);
        assert!(panel.decode_result.is_none());
        assert!(panel.decode_error.is_none());
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
    fn test_auto_record_toggle() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert!(panel.auto_record);
        panel.auto_record = false;
        assert!(!panel.auto_record);
        panel.auto_record = true;
        assert!(panel.auto_record);
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
        panel.signal_strength = 0.0;
        assert_eq!(panel.signal_strength, 0.0);
    }

    #[test]
    fn test_ui_simple_no_crash() {
        let mut panel = SatellitePanel::new(make_shared_state());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new(egui::Id::new("test")).show(ctx, |ui| {
                panel.ui_simple(ui);
            });
        });
    }

    #[test]
    fn test_ui_advanced_no_crash() {
        let mut panel = SatellitePanel::new(make_shared_state());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after UNIX epoch")
            .as_secs_f64();
        panel.cached_passes = vec![crate::tle_engine::PassInfo {
            satellite: "NOAA 19".into(),
            aos: "12:00:00".into(),
            los: "12:10:00".into(),
            max_elevation: 45.0,
            frequency_hz: 137_100_000,
            aos_dt: now,
            los_dt: now + 600.0,
        }];
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new(egui::Id::new("test")).show(ctx, |ui| {
                panel.ui_simple(ui);
            });
        });
    }

    #[test]
    fn test_pending_status_flow() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert!(panel.pending_status.is_none());

        panel.pending_status = Some("Test status message".into());
        assert_eq!(panel.pending_status.as_deref(), Some("Test status message"));

        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new(egui::Id::new("test")).show(ctx, |ui| {
                panel.ui_simple(ui);
            });
        });

        // ui_simple does not drain panel.pending_status, so it remains Some
        assert_eq!(panel.pending_status.as_deref(), Some("Test status message"));
    }

    #[test]
    fn test_decode_preset_values() {
        assert_eq!(DecodePreset::MeteorM2_2.sample_rate(), 2_048_000);
        assert_eq!(DecodePreset::MeteorM2_2.symbol_rate(), 72_000);
        assert_eq!(DecodePreset::MeteorM2_3.sample_rate(), 2_048_000);
        assert_eq!(DecodePreset::MeteorM2_4.sample_rate(), 2_048_000);
        assert_eq!(DecodePreset::Custom.symbol_rate(), 72_000);
        assert_eq!(DecodePreset::all().len(), 4);
    }

    #[test]
    fn test_tick_decode_no_op_when_not_running() {
        let mut panel = SatellitePanel::new(make_shared_state());
        panel.tick_decode(); // should not panic
        assert!(panel.decode_result.is_none());
        assert!(panel.decode_error.is_none());
    }

    #[test]
    fn test_request_decode_tab() {
        let mut panel = SatellitePanel::new(make_shared_state());
        assert!(!panel.pending_decode_tab);
        panel.request_decode_tab(DecodePreset::MeteorM2_3);
        assert!(panel.pending_decode_tab);
        assert_eq!(panel.decode_satellite_preset, DecodePreset::MeteorM2_3);
    }

    #[test]
    fn test_ui_decode_no_crash() {
        let mut panel = SatellitePanel::new(make_shared_state());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new(egui::Id::new("test_decode")).show(ctx, |ui| {
                panel.ui_decode(ui);
            });
        });
    }
}
