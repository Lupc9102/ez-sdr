use std::sync::{Arc, Mutex};
use crate::app::SharedState;

pub struct AdsBPanel {
    shared: Arc<Mutex<SharedState>>,
    pub aircraft: Vec<AircraftEntry>,
    pub selected_icao: Option<u32>,
    pub show_map: bool,
    pub total_messages: u64,
    pub start_time: Option<std::time::Instant>,
}

#[derive(Debug, Clone)]
pub struct AircraftEntry {
    pub icao: u32,
    pub callsign: String,
    pub lat: f64,
    pub lon: f64,
    pub altitude: u32,
    pub speed: u32,
    pub heading: u32,
    pub seen: std::time::Instant,
}

impl Default for AircraftEntry {
    fn default() -> Self {
        Self {
            icao: 0,
            callsign: String::new(),
            lat: 0.0,
            lon: 0.0,
            altitude: 0,
            speed: 0,
            heading: 0,
            seen: std::time::Instant::now(),
        }
    }
}

impl AdsBPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            aircraft: vec![],
            selected_icao: None,
            show_map: true,
            total_messages: 0,
            start_time: None,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("ADS-B / Mode S (1090 MHz)");

        // Stats bar
        ui.horizontal(|ui| {
            let msg_rate = if let Some(start) = self.start_time {
                self.total_messages as f64 / start.elapsed().as_secs_f64()
            } else {
                0.0
            };
            ui.label(format!("Aircraft: {} | Messages: {} ({:.0}/s)", self.aircraft.len(), self.total_messages, msg_rate));
            if self.start_time.is_some() {
                if ui.button("Stop").clicked() {
                    self.start_time = None;
                }
            } else if ui.button("Start ADS-B").clicked() {
                let mut state = self.shared.lock().unwrap();
                state.source.frequency_hz = 1_090_000_000;
                state.source.sample_rate_hz = 2_048_000;
                if state.source.status == crate::source_manager::SourceStatus::Idle {
                    state.source.start();
                }
                self.start_time = Some(std::time::Instant::now());
            }
        });

        ui.separator();
        ui.checkbox(&mut self.show_map, "Show map");

        if self.show_map {
            // Pseudo-map: render aircraft as dots
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 200.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(15, 25, 15));

            // Draw grid
            for i in 0..20 {
                let x = rect.left() + (i as f32 / 20.0) * rect.width();
                let y = rect.top() + (i as f32 / 20.0) * rect.height();
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(0.5, egui::Color32::from_rgb(30, 50, 30)),
                );
                painter.line_segment(
                    [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                    egui::Stroke::new(0.5, egui::Color32::from_rgb(30, 50, 30)),
                );
            }

            // Plot aircraft
            for ac in &self.aircraft {
                let x = rect.left() + ((ac.lon + 180.0) / 360.0) as f32 * rect.width();
                let y = rect.top() + ((90.0 - ac.lat) / 180.0) as f32 * rect.height();
                let color = if self.selected_icao == Some(ac.icao) { egui::Color32::from_rgb(0, 255, 255) } else { egui::Color32::GREEN };
                painter.circle_filled(egui::pos2(x, y), 3.0, color);
                painter.text(
                    egui::pos2(x + 5.0, y - 8.0),
                    egui::Align2::LEFT_CENTER,
                    &ac.callsign,
                    egui::FontId::proportional(9.0),
                    color,
                );
            }
        }

        ui.separator();

        // Aircraft table
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("adsb_grid").num_columns(8).striped(true).show(ui, |ui| {
                ui.label("ICAO");
                ui.label("Callsign");
                ui.label("Alt (ft)");
                ui.label("Spd (kt)");
                ui.label("HDG");
                ui.label("Lat");
                ui.label("Lon");
                ui.label("Age");
                ui.end_row();

                let now = std::time::Instant::now();
                for ac in &self.aircraft {
                    let age = now.duration_since(ac.seen).as_secs();
                    ui.label(format!("{:06X}", ac.icao));
                    ui.label(&ac.callsign);
                    ui.label(format!("{}", ac.altitude));
                    ui.label(format!("{}", ac.speed));
                    ui.label(format!("{}°", ac.heading));
                    ui.label(format!("{:.4}", ac.lat));
                    ui.label(format!("{:.4}", ac.lon));
                    ui.label(format!("{}s", age));
                    ui.end_row();
                }
            });
        });
    }
}
