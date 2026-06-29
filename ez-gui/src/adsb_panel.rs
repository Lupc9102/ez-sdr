use std::sync::{Arc, Mutex};
use crate::app::SharedState;

pub struct AdsBPanel {
    shared: Arc<Mutex<SharedState>>,
    pub aircraft: Vec<AircraftEntry>,
    pub selected_icao: Option<u32>,
    pub show_map: bool,
    pub total_messages: u64,
    pub start_time: Option<std::time::Instant>,
    pub aircraft_info: std::collections::HashMap<u32, AircraftInfo>,
    info_rx: std::sync::mpsc::Receiver<(u32, AircraftInfo)>,
    info_tx: std::sync::mpsc::Sender<(u32, AircraftInfo)>,
    pub min_altitude_ft: u32,
    pub max_altitude_ft: u32,
    pub altitude_filter_enabled: bool,
    pub max_age_secs: u64,
    pub callsign_filter: String,
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

#[derive(Debug, Clone, Default)]
pub struct AircraftInfo {
    pub model: String,
    pub operator: String,
    pub registration: String,
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
        let (info_tx, info_rx) = std::sync::mpsc::channel();
        Self {
            shared,
            aircraft: vec![],
            selected_icao: None,
            show_map: true,
            total_messages: 0,
            start_time: None,
            aircraft_info: std::collections::HashMap::new(),
            info_rx,
            info_tx,
            min_altitude_ft: 0,
            max_altitude_ft: 60_000,
            altitude_filter_enabled: false,
            max_age_secs: 60,
            callsign_filter: String::new(),
        }
    }

    fn fetch_aircraft_info(&mut self, icao: u32) {
        if self.aircraft_info.contains_key(&icao) {
            return;
        }
        self.aircraft_info.insert(icao, AircraftInfo {
            model: "Loading...".to_string(),
            operator: "Loading...".to_string(),
            registration: "Loading...".to_string(),
        });

        let icao_hex = format!("{:06X}", icao);
        let tx = self.info_tx.clone();

        std::thread::spawn(move || {
            let url = format!("https://api.planespotters.net/pub/photos/hex/{}", icao_hex);
            if let Ok(resp) = ureq::get(&url).call() {
                if let Ok(json) = resp.into_json::<serde_json::Value>() {
                    let model = json["aircraft"]["model"].as_str().unwrap_or("Unknown").to_string();
                    let operator = json["aircraft"]["operator"].as_str().unwrap_or("Unknown").to_string();
                    let registration = json["aircraft"]["registration"].as_str().unwrap_or("Unknown").to_string();
                    let _ = tx.send((icao, AircraftInfo { model, operator, registration }));
                }
            }
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Poll for aircraft info updates
        while let Ok((icao, info)) = self.info_rx.try_recv() {
            self.aircraft_info.insert(icao, info);
        }

        ui.heading("ADS-B / Mode S (1090 MHz)");

        // Stats bar
        ui.horizontal(|ui| {
            let msg_rate = if let Some(start) = self.start_time {
                self.total_messages as f64 / start.elapsed().as_secs_f64()
            } else {
                0.0
            };
            ui.label(format!("Aircraft: {} | Messages: {} ({:.0}/s)", self.aircraft.len(), self.total_messages, msg_rate))
                .on_hover_text("Live count of tracked aircraft and Mode S/ADS-B messages received. Message rate shows decode throughput.");
            if self.start_time.is_some() {
                if ui.button("Stop").on_hover_text("Stop the ADS-B decoder and SDR.").clicked() {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.adsb_running = false;
                    }
                    self.start_time = None;
                }
            } else if ui.button("Start ADS-B").on_hover_text("Tune to 1090 MHz, set sample rate 2.048 MS/s, and start decoding Mode S transponder messages from aircraft overhead.").clicked() {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.source.frequency_hz = 1_090_000_000;
                    state.source.sample_rate_hz = 2_048_000;
                    if state.source.status != crate::source_manager::SourceStatus::Running {
                        state.source.start();
                    }
                    state.adsb_running = true;
                }
                self.start_time = Some(std::time::Instant::now());
            }
        });

        ui.separator();
        // Filters
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.altitude_filter_enabled, "Alt filter")
                .on_hover_text("Only show aircraft within the altitude range below.");
            if self.altitude_filter_enabled {
                ui.add(egui::DragValue::new(&mut self.min_altitude_ft).speed(500.0).range(0..=60_000).suffix(" ft min"))
                    .on_hover_text("Minimum altitude to display (feet MSL).");
                ui.add(egui::DragValue::new(&mut self.max_altitude_ft).speed(500.0).range(0..=100_000).suffix(" ft max"))
                    .on_hover_text("Maximum altitude to display (feet MSL).");
            }
            ui.separator();
            ui.label("Max age:").on_hover_text("Remove aircraft not heard for more than this many seconds.");
            ui.add(egui::DragValue::new(&mut self.max_age_secs).speed(5.0).range(10..=600).suffix("s"))
                .on_hover_text("Stale aircraft timeout in seconds. Default 60 s.");
            ui.separator();
            ui.label("Search:").on_hover_text("Filter table by callsign or ICAO hex. Case-insensitive.");
            ui.add(egui::TextEdit::singleline(&mut self.callsign_filter).desired_width(100.0).hint_text("callsign/ICAO"));
            if !self.callsign_filter.is_empty() && ui.small_button("✕").clicked() {
                self.callsign_filter.clear();
            }
        });

        ui.separator();
        ui.checkbox(&mut self.show_map, "Show map")
            .on_hover_text("Toggle the geographic map view. Aircraft are plotted as green dots using their GPS-reported latitude/longitude from ADS-B position messages.");

        if self.show_map {
            // Pseudo-map: render aircraft as dots
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 200.0), egui::Sense::click());
            let response = response.on_hover_text("World map showing aircraft positions. Click a plane dot to look up its model, operator, and registration via Planespotters.net.");
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(15, 25, 15));

            // Handle clicks
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let mut closest_icao = None;
                    let mut closest_dist = f32::INFINITY;
                    for ac in &self.aircraft {
                        let x = rect.left() + ((ac.lon + 180.0) / 360.0) as f32 * rect.width();
                        let y = rect.top() + ((90.0 - ac.lat) / 180.0) as f32 * rect.height();
                        let dist = ((pos.x - x).powi(2) + (pos.y - y).powi(2)).sqrt();
                        if dist < 10.0 && dist < closest_dist {
                            closest_dist = dist;
                            closest_icao = Some(ac.icao);
                        }
                    }
                    if let Some(icao) = closest_icao {
                        self.selected_icao = Some(icao);
                        self.fetch_aircraft_info(icao);
                    }
                }
            }

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
            let map_now = std::time::Instant::now();
            for ac in &self.aircraft {
                let age = map_now.duration_since(ac.seen).as_secs();
                if age > self.max_age_secs { continue; }
                if self.altitude_filter_enabled && (ac.altitude < self.min_altitude_ft || ac.altitude > self.max_altitude_ft) { continue; }
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

        // Show selected aircraft info
        if let Some(icao) = self.selected_icao {
            if let Some(info) = self.aircraft_info.get(&icao) {
                ui.group(|ui| {
                    ui.heading(format!("Aircraft {:06X}", icao));
                    ui.label(format!("Model: {}", info.model));
                    ui.label(format!("Operator: {}", info.operator));
                    ui.label(format!("Registration: {}", info.registration));
                });
                ui.separator();
            }
        }

        // Aircraft table
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("adsb_grid").num_columns(8).striped(true).show(ui, |ui| {
                ui.label("ICAO").on_hover_text("24-bit ICAO Mode S address — unique to each aircraft worldwide. Like a tail number but in hex.");
                ui.label("Callsign").on_hover_text("Flight or aircraft callsign broadcast by the aircraft. May be a flight number (UAL123) or registration (N12345).");
                ui.label("Alt (ft)").on_hover_text("Barometric altitude in feet above mean sea level (MSL), from the aircraft's Mode C altitude encoder.");
                ui.label("Spd (kt)").on_hover_text("Ground speed in knots from ADS-B velocity message (1090 ES Type 19). Not airspeed.");
                ui.label("HDG").on_hover_text("Track angle (degrees true from north), not magnetic heading. Derived from ADS-B velocity message.");
                ui.label("Lat").on_hover_text("GPS latitude in decimal degrees from ADS-B surface/airborne position message (Type 9-18). Accuracy typically ±10m.");
                ui.label("Lon").on_hover_text("GPS longitude in decimal degrees from ADS-B surface/airborne position message.");
                ui.label("Age").on_hover_text("Seconds since the last ADS-B message was received from this aircraft. Aircraft not heard for >60 s may have moved out of range.");
                ui.end_row();

                let now = std::time::Instant::now();
                let max_age = self.max_age_secs;
                let alt_filter = self.altitude_filter_enabled;
                let min_alt = self.min_altitude_ft;
                let max_alt = self.max_altitude_ft;
                for ac in &self.aircraft {
                    let age = now.duration_since(ac.seen).as_secs();
                    if age > max_age { continue; }
                    if alt_filter && (ac.altitude < min_alt || ac.altitude > max_alt) { continue; }
                    let cs_filter = self.callsign_filter.to_lowercase();
                    if !cs_filter.is_empty() {
                        let icao_str = format!("{:06x}", ac.icao);
                        if !ac.callsign.to_lowercase().contains(&cs_filter) && !icao_str.contains(&cs_filter) { continue; }
                    }
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
