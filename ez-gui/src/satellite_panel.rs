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
}

impl SatellitePanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
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
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Satellite Tracking");

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

        ui.add_space(16.0);
        ui.separator();
        // ── Satellite Antenna Setup Tutorial ──────────────────────────────
        ui.add_space(4.0);
        ui.label(egui::RichText::new("📡 Satellite Antenna Setup Guide").size(16.0).strong());
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
            ui.separator();
            ui.label("NOAA APT satellites transmit at only 4 W from 800+ km away. A tuned antenna with a clear horizon is essential for good images.");
        });

        ui.add_space(4.0);
        ui.collapsing("V-Dipole — the best starter antenna for NOAA APT (137 MHz)", |ui| {
            ui.add_space(4.0);
            ui.label("The 9A4QV V-dipole is the most popular DIY antenna for 137 MHz weather satellites. It's cheap, easy to build, and outperforms most store-bought options.");
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Build specs:").strong());
            ui.add_space(2.0);
            ui.label("  Each arm length: 53.4 cm (quarter-wave at 137.5 MHz)");
            ui.label("  Angle between arms: 120° (V-shape)");
            ui.label("  Material: Any conductive rod/wire (coat hanger, brass, aluminium)");
            ui.label("  Feed: One arm to coax centre conductor, other to shield");
            ui.label("  Mount: HORIZONTAL, arms pointing North–South");
            ui.label("  Coax: 50 Ω (RG-58 or better), keep under 10 m if possible");
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
                ui.separator();
                ui.label("The V-dipole's horizontal polarisation rejects vertically-polarised terrestrial signals (FM broadcast, land mobile) by ~20 dB — this significantly improves satellite SNR in urban areas.");
            });
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 180, 0), "NOTE");
                ui.separator();
                ui.label("A dipole made from an old metal coat hanger works. Don't overthink the materials. Focus on getting it outside with a clear view of the sky.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Turnstile / QFH — circular polarisation for better performance", |ui| {
            ui.add_space(4.0);
            ui.label("These antennas use circular polarisation (RHCP), which matches the satellite's transmission. They provide more consistent signal strength during a pass compared to a linearly-polarised V-dipole.");
            ui.add_space(4.0);
            egui::Grid::new("sat_ant_grid").num_columns(2).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Type").strong());
                ui.label(egui::RichText::new("Trade-off").strong());
                ui.end_row();
                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "Turnstile (cross-dipole)");
                ui.label("Two crossed dipoles at 90°, fed 90° out of phase. Good circular polarisation. Harder to build than V-dipole but better null-filling overhead.");
                ui.end_row();
                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "QFH (Quadrifilar Helix)");
                ui.label("Best performance for LEO satellites. True hemispherical pattern — receives from horizon to horizon. Complex to build but the gold standard for NOAA/Meteor.");
                ui.end_row();
            });
        });

        ui.add_space(4.0);
        ui.collapsing("GOES geostationary satellites — a different challenge (1.7 GHz)", |ui| {
            ui.add_space(4.0);
            ui.label("GOES-16/18 are in geostationary orbit (35,786 km). Signals are much weaker than LEO NOAA sats. You will need a directional antenna:");
            ui.add_space(4.0);
            ui.label("  1. Grid dish (60–120 cm, repurposed Ku-band TV dish + L-band feed) — $0–30");
            ui.label("  2. Helical antenna (7–12 turns, DIY ~$15) — 15–18 dBic gain");
            ui.label("  3. Nooelec SAWbird+ GOES LNA ($35) with bandpass filter for 1694 MHz");
            ui.label("  Sample rate: ≥2.4 MHz, frequency: 1694.1 MHz, polarisation: RHCP");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 180, 0), "NOTE");
                ui.separator();
                ui.label("GOES requires precise antenna pointing. Use a satellite pointing calculator and fine-tune by watching the signal strength in the spectrum display. Budget: ~$80–150 for a working setup.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Pass checklist — before each satellite pass", |ui| {
            ui.add_space(4.0);
            ui.label("1.  Check pass time on heavens-above.com, N2YO.com, or the pass table above");
            ui.label("2.  Best passes: ≥30° max elevation. Overhead (90°) gives 12+ minutes of signal.");
            ui.label("3.  Set mode to WFM, bandwidth 34–40 kHz on the SDR panel");
            ui.label("4.  Enable auto-tune & auto-record in the satellite panel");
            ui.label("5.  Start recording 2 minutes BEFORE AOS (satellite rises)");
            ui.label("6.  After the pass (LOS), decode the WAV file with SatDump or WXtoIMG");
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Active NOAA APT frequencies:").strong());
            ui.label("  NOAA 15: 137.620 MHz  |  NOAA 18: 137.9125 MHz  |  NOAA 19: 137.100 MHz");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
                ui.separator();
                ui.label("NOAA APT uses analog FM — you can hear the distinctive 'chirping' image data as you tune in. If you hear it, you're close! Fine-tune until it sounds clearest.");
            });
        });
    }
}
