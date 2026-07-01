use std::sync::{Arc, Mutex};
use crate::app::SharedState;

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
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Antenna setup checklist gate
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

        // User level check for adaptive UI
        let user_level = self.shared.try_lock()
            .map(|s| crate::user_level::UserLevel::from_str(&s.config.user_level))
            .unwrap_or(crate::user_level::UserLevel::Beginner);
        if user_level.simplify_layout() {
            ui.add_space(4.0);
            if ui.add(egui::Button::new(egui::RichText::new("🤖 Ask AI to track a satellite").size(14.0))
                .min_size(egui::vec2(220.0, 28.0))
                .fill(egui::Color32::from_rgb(40, 60, 120)))
                .on_hover_text("Let the AI assistant set up satellite tracking for you. Just tell it which satellite to track.")
                .clicked()
            {
                self.pending_ai_prompt = Some("Help me track a satellite. What satellites are currently active and how do I set up tracking?".to_string());
            }
            ui.add_space(4.0);
        }

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
            let changed_lat = ui.add(egui::Slider::new(&mut self.observer_lat, -90.0..=90.0).text("Latitude")).changed();
            let changed_lon = ui.add(egui::Slider::new(&mut self.observer_lon, -180.0..=180.0).text("Longitude")).changed();
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
            let color = if norm > 0.5 { egui::Color32::GREEN } else if norm > 0.2 { egui::Color32::YELLOW } else { egui::Color32::RED };
            ui.add(egui::ProgressBar::new(norm as f32).fill(color).text(format!("{:.1} dB", self.signal_strength)));
        });

        {
            let doppler_color = if self.doppler_hz.abs() > 5000.0 { egui::Color32::from_rgb(255, 120, 60) }
                else if self.doppler_hz.abs() > 1000.0 { egui::Color32::from_rgb(255, 220, 80) }
                else { egui::Color32::from_rgb(120, 220, 120) };
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
            if ui.button("Start Recording").clicked() { self.recording = true; }
            if ui.button("Stop Recording").clicked() { self.recording = false; }
            ui.label(if self.recording { "● RECORDING" } else { "" });
        });

        ui.separator();

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
            .map(|d| d.as_secs_f64())
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
                    let countdown_text;
                    let countdown_color;
                    if is_active {
                        let remaining = secs_until_los.max(0.0) as u64;
                        let m = remaining / 60;
                        let s = remaining % 60;
                        countdown_text = format!("▶ {:02}:{:02}", m, s);
                        countdown_color = egui::Color32::from_rgb(50, 255, 100);
                    } else if secs_until_aos < 0.0 {
                        countdown_text = "past".to_string();
                        countdown_color = egui::Color32::GRAY;
                    } else if secs_until_aos < 600.0 {
                        let m = secs_until_aos as u64 / 60;
                        let s = secs_until_aos as u64 % 60;
                        countdown_text = format!("{:02}:{:02}", m, s);
                        countdown_color = egui::Color32::YELLOW;
                    } else {
                        let h = secs_until_aos as u64 / 3600;
                        let m = (secs_until_aos as u64 % 3600) / 60;
                        countdown_text = format!("{}h {:02}m", h, m);
                        countdown_color = egui::Color32::GRAY;
                    }
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
}
