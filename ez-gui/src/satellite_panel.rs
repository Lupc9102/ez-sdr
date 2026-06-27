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
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Satellite Tracking");

        // Observer location
        ui.collapsing("Observer Location", |ui| {
            ui.add(egui::Slider::new(&mut self.observer_lat, -90.0..=90.0).text("Latitude"));
            ui.add(egui::Slider::new(&mut self.observer_lon, -180.0..=180.0).text("Longitude"));
        });

        ui.separator();

        // Controls
        ui.checkbox(&mut self.auto_record, "Auto-record on pass");
        ui.checkbox(&mut self.auto_tune, "Auto-tune to downlink + Doppler");
        ui.checkbox(&mut self.live_decode, "Live decode (LRPT/APT)");

        ui.separator();

        // Signal strength bar
        ui.horizontal(|ui| {
            ui.label("Signal Strength:");
            let norm = ((self.signal_strength + 120.0) / 120.0).clamp(0.0, 1.0);
            let color = if norm > 0.5 { egui::Color32::GREEN } else if norm > 0.2 { egui::Color32::YELLOW } else { egui::Color32::RED };
            ui.add(egui::ProgressBar::new(norm as f32).fill(color).text(format!("{:.1} dB", self.signal_strength)));
        });

        ui.label(format!("Doppler shift: {:.1} Hz", self.doppler_hz));

        ui.horizontal(|ui| {
            if ui.button("Start Recording").clicked() { self.recording = true; }
            if ui.button("Stop Recording").clicked() { self.recording = false; }
            ui.label(if self.recording { "● RECORDING" } else { "" });
        });

        ui.separator();

        // Pass list
        ui.heading("Upcoming Passes");
        egui::ScrollArea::vertical().show(ui, |ui| {
            let passes = {
                let state = self.shared.lock().unwrap();
                state.tle.upcoming_passes()
            };

            egui::Grid::new("pass_grid").num_columns(5).striped(true).show(ui, |ui| {
                ui.label("Satellite");
                ui.label("AOS");
                ui.label("LOS");
                ui.label("MaxEl");
                ui.label("Action");
                ui.end_row();

                for pass in &passes {
                    let selected = self.selected_sat.as_deref() == Some(&pass.satellite);
                    ui.colored_label(
                        if selected { egui::Color32::from_rgb(0, 255, 255) } else { egui::Color32::WHITE },
                        &pass.satellite,
                    );
                    ui.label(&pass.aos);
                    ui.label(&pass.los);
                    ui.label(format!("{:.0}°", pass.max_elevation));
                    if ui.button(if selected { "Selected" } else { "Select" }).clicked() {
                        self.selected_sat = Some(pass.satellite.clone());
                        if self.auto_tune {
                            let mut state = self.shared.lock().unwrap();
                            state.source.frequency_hz = pass.frequency_hz;
                        }
                    }
                    ui.end_row();
                }
            });
        });
    }
}
