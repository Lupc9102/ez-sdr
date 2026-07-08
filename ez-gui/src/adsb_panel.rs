use crate::app::SharedState;
use std::io::Read;
use std::sync::{Arc, Mutex};

pub struct AdsBPanel {
    shared: Arc<Mutex<SharedState>>,
    pub aircraft: Vec<AircraftEntry>,
    pub selected_icao: Option<u32>,
    pub total_messages: u64,
    /// (preambles, accepted, rejected) from `AdsBDecoder::stats()`.
    pub decode_stats: (u64, u64, u64),
    pub start_time: Option<std::time::Instant>,
    pub aircraft_info: std::collections::HashMap<u32, AircraftInfo>,
    info_rx: std::sync::mpsc::Receiver<(u32, AircraftInfo)>,
    info_tx: std::sync::mpsc::Sender<(u32, AircraftInfo)>,
    pub min_altitude_ft: u32,
    pub max_altitude_ft: u32,
    pub altitude_filter_enabled: bool,
    pub max_age_secs: u64,
    pub callsign_filter: String,
    pub show_trails: bool,
    aircraft_trails: std::collections::HashMap<u32, std::collections::VecDeque<(f64, f64)>>,
    pub pending_ai_prompt: Option<String>,
    pub observer_lat: f64,
    pub observer_lon: f64,
    pub alert_enabled: bool,
    pub alert_range_km: f64,
    pub desktop_notifications: bool,
    known_icao: std::collections::HashSet<u32>,
    pub notifications: std::collections::VecDeque<AdsBNotification>,
    pub pending_status_flash: Option<String>,
    notif_counter: u64,
    checklist: crate::antenna_checklist::AntennaChecklist,
    tile_cache: std::collections::HashMap<(u32, u32, u32), egui::TextureHandle>,
    tile_pending: std::collections::HashSet<(u32, u32, u32)>,
    tile_download_tx: std::sync::mpsc::Sender<((u32, u32, u32), Vec<u8>)>,
    tile_download_rx: std::sync::mpsc::Receiver<((u32, u32, u32), Vec<u8>)>,
    tile_zoom: u32,
    tile_cx: f64,
    tile_cy: f64,
    zoom_accum: f64,
    tile_last_used: std::collections::HashMap<(u32, u32, u32), u64>,
    tile_frame_counter: u64,
    tile_inflight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    last_zoom_change: std::time::Instant,
    geo_rx: Option<std::sync::mpsc::Receiver<(f64, f64)>>,
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

/// A "passive ADS-B" alert — fired when a new aircraft comes into range.
#[derive(Debug, Clone)]
pub struct AdsBNotification {
    pub id: u64,
    pub icao: u32,
    pub callsign: String,
    pub lat: f64,
    pub lon: f64,
    pub altitude: u32,
    pub speed: u32,
    pub distance_km: Option<f64>,
    pub bearing_deg: Option<f64>,
    pub timestamp: std::time::Instant,
    pub dismissed: bool,
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

/// Aircraft category used to select the 3D icon shape.
#[derive(Debug, PartialEq)]
enum AcCategory {
    WideBody,
    NarrowBody,
    Regional,
    BizJet,
    Helicopter,
    Generic,
}

fn classify_aircraft(model: &str) -> AcCategory {
    let m = model.to_ascii_lowercase();
    if m.contains("helicopter")
        || m.contains("s-76")
        || m.contains("s-92")
        || m.contains("h-60")
        || m.contains("r44")
        || m.contains("r66")
        || m.contains("aw139")
        || m.contains("as350")
        || m.contains("sikorsky")
    {
        return AcCategory::Helicopter;
    }
    if m.contains("747")
        || m.contains("777")
        || m.contains("787")
        || m.contains("767")
        || m.contains("a380")
        || m.contains("a350")
        || m.contains("a330")
        || m.contains("a340")
        || m.contains("a300")
        || m.contains("a310")
        || m.contains("md-11")
        || m.contains("dc-10")
        || m.contains("l-1011")
    {
        return AcCategory::WideBody;
    }
    if m.contains("737")
        || m.contains("757")
        || m.contains("a320")
        || m.contains("a321")
        || m.contains("a319")
        || m.contains("a318")
        || m.contains("a220")
        || m.contains("717")
        || m.contains("md-8")
        || m.contains("md-9")
    {
        return AcCategory::NarrowBody;
    }
    if m.contains("crj")
        || m.contains("erj")
        || m.contains("e170")
        || m.contains("e175")
        || m.contains("e190")
        || m.contains("e195")
        || m.contains("atr")
        || m.contains("q400")
        || m.contains("dh8")
        || m.contains("dash 8")
        || m.contains("embraer 1")
        || m.contains("sf34")
        || m.contains("cessna 208")
    {
        return AcCategory::Regional;
    }
    if m.contains("citation")
        || m.contains("gulfstream")
        || m.contains("learjet")
        || m.contains("falcon")
        || m.contains("global ")
        || m.contains("challenger")
        || m.contains("phenom")
        || m.contains("pc-12")
        || m.contains("king air")
    {
        return AcCategory::BizJet;
    }
    AcCategory::Generic
}

/// Draw a simplified top-down aircraft silhouette at screen position `pos`.
///
/// `heading_deg` is a compass bearing: 0 = North (nose points up on screen),
/// 90 = East, etc. `scale` is pixels per normalized unit (7–12 works well).
fn draw_plane_model(
    painter: &egui::Painter,
    pos: egui::Pos2,
    heading_deg: f32,
    category: &AcCategory,
    color: egui::Color32,
    scale: f32,
) {
    let rad = heading_deg.to_radians();
    let (s, c) = rad.sin_cos();
    let fill = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 85);
    let stroke = egui::Stroke::new(1.0, color);

    // Rotate normalized shape coords and translate to screen space.
    // x' = x·cosθ − y·sinθ,  y' = x·sinθ + y·cosθ  → clockwise compass heading, Y-down screen.
    let xf = |pts: Vec<(f32, f32)>| -> Vec<egui::Pos2> {
        pts.into_iter()
            .map(|(x, y)| {
                egui::pos2(
                    pos.x + (x * c - y * s) * scale,
                    pos.y + (x * s + y * c) * scale,
                )
            })
            .collect()
    };

    if matches!(category, AcCategory::Helicopter) {
        // Oval fuselage
        painter.add(egui::Shape::convex_polygon(
            xf(vec![
                (-0.25, -0.45),
                (0.25, -0.45),
                (0.32, 0.08),
                (0.22, 0.32),
                (-0.22, 0.32),
                (-0.32, 0.08),
            ]),
            fill,
            stroke,
        ));
        // Tail boom
        painter.add(egui::Shape::convex_polygon(
            xf(vec![
                (-0.07, 0.30),
                (0.07, 0.30),
                (0.07, 1.05),
                (-0.07, 1.05),
            ]),
            fill,
            stroke,
        ));
        // Main rotor drawn in screen-space (not heading-rotated)
        let r = scale * 0.90;
        painter.line_segment(
            [egui::pos2(pos.x - r, pos.y), egui::pos2(pos.x + r, pos.y)],
            stroke,
        );
        painter.line_segment(
            [egui::pos2(pos.x, pos.y - r), egui::pos2(pos.x, pos.y + r)],
            stroke,
        );
        return;
    }

    // Fixed-wing shape parameters: (fuselage_half_w, wing_root_x, wing_y_forward, wing_tip_x, stab_tip_x)
    let (fw, wx, wy, wt, st) = match category {
        AcCategory::WideBody => (0.11_f32, 0.12, -0.18, 1.10, 0.48),
        AcCategory::NarrowBody => (0.08_f32, 0.09, -0.10, 0.85, 0.38),
        AcCategory::Regional => (0.07_f32, 0.08, -0.04, 0.70, 0.30),
        AcCategory::BizJet => (0.05_f32, 0.06, -0.26, 0.82, 0.32),
        _ => (0.08_f32, 0.09, -0.10, 0.78, 0.36),
    };
    let wy2 = wy + 0.40; // swept wing trailing-edge Y

    // Fuselage (5-point convex hull with pointed nose)
    painter.add(egui::Shape::convex_polygon(
        xf(vec![
            (0.0, -1.22),
            (fw, -0.90),
            (fw * 1.3, 0.80),
            (-fw * 1.3, 0.80),
            (-fw, -0.90),
        ]),
        fill,
        stroke,
    ));
    // Left wing (swept-back triangle)
    painter.add(egui::Shape::convex_polygon(
        xf(vec![(-wx, wy), (-wt, wy2), (-wx, wy2 + 0.06)]),
        fill,
        stroke,
    ));
    // Right wing
    painter.add(egui::Shape::convex_polygon(
        xf(vec![(wx, wy), (wt, wy2), (wx, wy2 + 0.06)]),
        fill,
        stroke,
    ));
    // Left horizontal stabilizer
    painter.add(egui::Shape::convex_polygon(
        xf(vec![(-0.09_f32, 0.58), (-st, 0.82), (-0.09_f32, 0.92)]),
        fill,
        stroke,
    ));
    // Right horizontal stabilizer
    painter.add(egui::Shape::convex_polygon(
        xf(vec![(0.09_f32, 0.58), (st, 0.82), (0.09_f32, 0.92)]),
        fill,
        stroke,
    ));
}

impl AdsBPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        let (info_tx, info_rx) = std::sync::mpsc::channel();
        let (tile_download_tx, tile_download_rx) = std::sync::mpsc::channel();
        let n = (1u64 << 8) as f64;
        let init_cx = (-0.1 + 180.0) / 360.0 * n;
        let init_cy =
            (1.0 - (51.5_f64.to_radians().tan().asinh() / std::f64::consts::PI)) / 2.0 * n;
        Self {
            shared: shared.clone(),
            aircraft: vec![],
            selected_icao: None,
            total_messages: 0,
            decode_stats: (0, 0, 0),
            start_time: None,
            aircraft_info: std::collections::HashMap::new(),
            info_rx,
            info_tx,
            min_altitude_ft: 0,
            max_altitude_ft: 60_000,
            altitude_filter_enabled: false,
            max_age_secs: 60,
            callsign_filter: String::new(),
            show_trails: true,
            aircraft_trails: std::collections::HashMap::new(),
            pending_ai_prompt: None,
            observer_lat: 51.5,
            observer_lon: -0.1,
            alert_enabled: true,
            alert_range_km: 0.0,
            desktop_notifications: false,
            known_icao: std::collections::HashSet::new(),
            notifications: std::collections::VecDeque::new(),
            pending_status_flash: None,
            notif_counter: 0,
            checklist: crate::antenna_checklist::AntennaChecklist::for_adsb(shared),
            tile_cache: std::collections::HashMap::new(),
            tile_pending: std::collections::HashSet::new(),
            tile_download_tx,
            tile_download_rx,
            tile_zoom: 8,
            tile_cx: init_cx,
            tile_cy: init_cy,
            zoom_accum: 0.0,
            last_zoom_change: std::time::Instant::now(),
            tile_last_used: std::collections::HashMap::new(),
            tile_frame_counter: 0,
            tile_inflight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            geo_rx: None,
        }
    }

    /// Auto-configure the SDR for ADS-B and start receiving. Idempotent: if the
    /// receiver is already running it does nothing (so re-entering the Planes
    /// mode doesn't reset stats). Called on entry to the Planes mode.
    pub fn begin(&mut self) {
        {
            if let Ok(state) = self.shared.try_lock() {
                if state.adsb_running {
                    return;
                }
            }
        }
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

    /// Passive ADS-B: scan the current aircraft list for newcomers and fire a
    /// notification for each one we haven't seen yet (and that satisfies the
    /// range filter). Called every frame from the main app loop.
    pub fn check_for_new_aircraft(&mut self) {
        // Sync observer location + read the ADS-B run flag from shared state.
        let adsb_running = {
            if let Ok(state) = self.shared.try_lock() {
                if (state.config.observer_lat - self.observer_lat).abs() > 0.001
                    || (state.config.observer_lon - self.observer_lon).abs() > 0.001
                {
                    self.observer_lat = state.config.observer_lat;
                    self.observer_lon = state.config.observer_lon;
                }
                state.adsb_running
            } else {
                return;
            }
        };

        if !adsb_running {
            // Reset tracking so the next start is fresh.
            self.known_icao.clear();
            return;
        }
        if !self.alert_enabled {
            return;
        }

        let now = std::time::Instant::now();
        let current_icaos: std::collections::HashSet<u32> =
            self.aircraft.iter().map(|a| a.icao).collect();

        for ac in &self.aircraft {
            if self.known_icao.contains(&ac.icao) {
                continue;
            }

            let has_pos = ac.lat != 0.0 || ac.lon != 0.0;
            let (dist, bearing) = if has_pos {
                let d =
                    self.haversine_distance(self.observer_lat, self.observer_lon, ac.lat, ac.lon);
                let b = self.bearing(self.observer_lat, self.observer_lon, ac.lat, ac.lon);
                (Some(d), Some(b))
            } else {
                (None, None)
            };

            // Range filter: when set, require a valid position within range.
            // If the aircraft has no position yet, skip it this frame and retry
            // once a position arrives — that's the moment it truly "comes into range".
            if self.alert_range_km > 0.0 {
                match dist {
                    Some(d) if d <= self.alert_range_km => {}
                    _ => continue,
                }
            }

            let callsign = if ac.callsign.is_empty() {
                format!("{:06X}", ac.icao)
            } else {
                ac.callsign.clone()
            };

            self.notif_counter += 1;
            let notif = AdsBNotification {
                id: self.notif_counter,
                icao: ac.icao,
                callsign: callsign.clone(),
                lat: ac.lat,
                lon: ac.lon,
                altitude: ac.altitude,
                speed: ac.speed,
                distance_km: dist,
                bearing_deg: bearing,
                timestamp: now,
                dismissed: false,
            };
            self.notifications.push_back(notif.clone());
            while self.notifications.len() > 100 {
                self.notifications.pop_front();
            }

            // Short status-bar flash (picked up by the main loop).
            let msg = match (dist, ac.altitude) {
                (Some(d), alt) if alt > 0 => {
                    format!("✈ {callsign} spotted — {d:.1} km, {alt} ft")
                }
                (Some(d), _) => format!("✈ {callsign} spotted — {d:.1} km"),
                _ => format!("✈ {callsign} in range"),
            };
            self.pending_status_flash = Some(msg.clone());

            // Optional desktop notification (Linux notify-send).
            if self.desktop_notifications {
                let cs = callsign.clone();
                let body = match (dist, ac.altitude) {
                    (Some(d), alt) if alt > 0 => format!("{d:.1} km away, {alt} ft"),
                    (Some(d), _) => format!("{d:.1} km away"),
                    _ => "Signal detected".to_string(),
                };
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("notify-send")
                        .arg("--icon")
                        .arg("airplane")
                        .arg("--expire-time")
                        .arg("8000")
                        .arg(format!("Aircraft in range: {cs}"))
                        .arg(body)
                        .status();
                });
            }

            self.known_icao.insert(ac.icao);
        }

        // Drop departed aircraft from the known set so they re-trigger on return.
        self.known_icao.retain(|icao| current_icaos.contains(icao));
    }

    /// Render floating toast popups for active alerts. Drawn over any tab so the
    /// user sees them even when not looking at the ADS-B panel.
    pub fn render_toasts(&mut self, ctx: &egui::Context) {
        if self.notifications.is_empty() {
            return;
        }
        let screen = ctx.input(egui::InputState::viewport_rect);
        let toast_w = 300.0;
        let toast_h = 84.0;
        let gap = 8.0;
        let right_margin = 12.0;
        let bottom_margin = 12.0;
        let max_toasts = 5;
        let toast_ttl = 8.0f32;

        let now = std::time::Instant::now();
        // Active toasts, oldest→newest; we render the newest few, newest at the bottom.
        let active: Vec<usize> = self
            .notifications
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                !n.dismissed && now.duration_since(n.timestamp).as_secs_f32() < toast_ttl
            })
            .map(|(i, _)| i)
            .collect();
        let count = active.len().min(max_toasts);
        let newest_first: Vec<usize> = active.iter().rev().take(count).copied().collect();

        for (stack_idx, &i) in newest_first.iter().enumerate() {
            let n = self.notifications[i].clone();
            let x = screen.right() - right_margin - toast_w;
            let y = screen.bottom() - bottom_margin - (toast_h + gap) * (stack_idx as f32 + 1.0);
            let id = egui::Id::new(("adsb_toast", n.id));
            let resp = egui::Window::new("adsb_toast")
                .id(id)
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .movable(false)
                .current_pos(egui::pos2(x, y))
                .fixed_size(egui::vec2(toast_w, toast_h))
                .frame({
                    egui::Frame {
                        fill: egui::Color32::from_rgba_unmultiplied(20, 35, 22, 245),
                        stroke: egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 200, 110)),
                        inner_margin: egui::Margin::same(8),
                        corner_radius: 6.0.into(),
                        ..Default::default()
                    }
                })
                .show(ctx, |ui| {
                    let hover = if n.lat != 0.0 || n.lon != 0.0 {
                        format!(
                            "ICAO {:06X} · {:.4}, {:.4} · {} kt",
                            n.icao, n.lat, n.lon, n.speed
                        )
                    } else {
                        format!("ICAO {:06X} · {} kt", n.icao, n.speed)
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("✈")
                                .size(24.0)
                                .color(egui::Color32::from_rgb(100, 255, 150)),
                        )
                        .on_hover_text(hover.clone());
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&n.callsign)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .on_hover_text(hover);
                            let mut detail = String::new();
                            if let Some(d) = n.distance_km {
                                detail.push_str(&format!("{d:.1} km"));
                                if let Some(b) = n.bearing_deg {
                                    detail.push_str(&format!(" · {b:.0}°"));
                                }
                                if n.altitude > 0 {
                                    detail.push_str(&format!(" · {} ft", n.altitude));
                                }
                            } else {
                                detail.push_str("Signal detected");
                                if n.altitude > 0 {
                                    detail.push_str(&format!(" · {} ft", n.altitude));
                                }
                            }
                            if n.speed > 0 {
                                detail.push_str(&format!(" · {} kt", n.speed));
                            }
                            ui.label(
                                egui::RichText::new(detail)
                                    .color(egui::Color32::from_rgb(150, 220, 160))
                                    .small(),
                            );
                            ui.label(
                                egui::RichText::new("click to dismiss")
                                    .color(egui::Color32::from_rgb(120, 120, 120))
                                    .small(),
                            );
                        });
                    });
                });
            if let Some(r) = resp {
                if r.response.clicked() {
                    self.notifications[i].dismissed = true;
                }
            }
        }

        // Prune dismissed / stale entries to bound memory.
        self.notifications
            .retain(|n| !n.dismissed && n.timestamp.elapsed().as_secs() < 600);
    }

    fn fetch_aircraft_info(&mut self, icao: u32) {
        if self.aircraft_info.contains_key(&icao) {
            return;
        }
        self.aircraft_info.insert(
            icao,
            AircraftInfo {
                model: "Loading...".to_string(),
                operator: "Loading...".to_string(),
                registration: "Loading...".to_string(),
            },
        );

        let icao_hex = format!("{icao:06X}");
        let tx = self.info_tx.clone();

        std::thread::spawn(move || {
            let url = format!("https://api.planespotters.net/pub/photos/hex/{icao_hex}");
            // Send on EVERY path: a fetch failure must still overwrite the
            // "Loading..." placeholder so it stops blocking retries, and must
            // surface "Unknown" so the user sees the lookup failed. The
            // previous code dropped both error paths silently, leaving the
            // aircraft info panel stuck at "Loading..." forever after one
            // transient network blip (the contains_key guard at the top then
            // prevented any retry for the rest of the session).
            let (model, operator, registration) = match ureq::get(&url).call() {
                Ok(resp) => match resp.into_body().read_json::<serde_json::Value>() {
                    Ok(json) => (
                        json["aircraft"]["model"]
                            .as_str()
                            .unwrap_or("Unknown")
                            .to_string(),
                        json["aircraft"]["operator"]
                            .as_str()
                            .unwrap_or("Unknown")
                            .to_string(),
                        json["aircraft"]["registration"]
                            .as_str()
                            .unwrap_or("Unknown")
                            .to_string(),
                    ),
                    Err(_) => (
                        "Unknown".to_string(),
                        "Unknown".to_string(),
                        "Unknown".to_string(),
                    ),
                },
                Err(_) => (
                    "Unknown".to_string(),
                    "Unknown".to_string(),
                    "Unknown".to_string(),
                ),
            };
            let _ = tx.send((
                icao,
                AircraftInfo {
                    model,
                    operator,
                    registration,
                },
            ));
        });
    }

    fn haversine_distance(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        const EARTH_RADIUS_KM: f64 = 6371.0;
        let lat1_rad = lat1.to_radians();
        let lat2_rad = lat2.to_radians();
        let delta_lat = (lat2 - lat1).to_radians();
        let delta_lon = (lon2 - lon1).to_radians();
        let a = (delta_lat / 2.0).sin().powi(2)
            + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
        EARTH_RADIUS_KM * c
    }

    fn bearing(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        let lat1_rad = lat1.to_radians();
        let lat2_rad = lat2.to_radians();
        let delta_lon = (lon2 - lon1).to_radians();
        let y = delta_lon.sin() * lat2_rad.cos();
        let x = lat1_rad.cos() * lat2_rad.sin() - lat1_rad.sin() * lat2_rad.cos() * delta_lon.cos();
        let bearing_rad = y.atan2(x);
        (bearing_rad.to_degrees() + 360.0) % 360.0
    }

    // --- OSM tile helpers ---
    const SCROLL_POINTS_PER_ZOOM_LEVEL: f64 = 250.0;
    const ZOOM_DEBOUNCE_MS: u64 = 300;
    const MAX_CACHED_TILES: usize = 400;
    const MAX_CONCURRENT_TILE_DOWNLOADS: usize = 8;

    fn lon_to_tile_x(lon: f64, zoom: u32) -> f64 {
        let n = (1u64 << zoom) as f64;
        (lon + 180.0) / 360.0 * n
    }

    fn lat_to_tile_y(lat: f64, zoom: u32) -> f64 {
        let n = (1u64 << zoom) as f64;
        let lat_rad = lat.to_radians();
        (1.0 - (lat_rad.tan().asinh() / std::f64::consts::PI)) / 2.0 * n
    }

    fn tile_disk_path(z: u32, x: u32, y: u32) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("tile_cache/{z}/{x}/{y}.png"))
    }

    fn load_or_fetch_tile(z: u32, x: u32, y: u32) -> Vec<u8> {
        let path = Self::tile_disk_path(z, x, y);
        if let Ok(cached) = std::fs::read(&path) {
            return cached;
        }
        let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");
        let fetched = match ureq::get(&url).header("User-Agent", "ez-sdr/0.1").call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                match resp.into_body().into_reader().read_to_end(&mut buf) {
                    Ok(_) => buf,
                    Err(_) => Vec::new(),
                }
            }
            Err(_) => Vec::new(),
        };
        if !fetched.is_empty() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, &fetched);
        }
        fetched
    }

    fn request_tile(&mut self, z: u32, x: u32, y: u32) {
        if self.tile_pending.contains(&(z, x, y)) {
            return;
        }
        if self
            .tile_inflight
            .load(std::sync::atomic::Ordering::Relaxed)
            >= Self::MAX_CONCURRENT_TILE_DOWNLOADS
        {
            return;
        }
        self.tile_pending.insert((z, x, y));
        self.tile_inflight
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tx = self.tile_download_tx.clone();
        let inflight = std::sync::Arc::clone(&self.tile_inflight);
        std::thread::spawn(move || {
            let bytes = Self::load_or_fetch_tile(z, x, y);
            let _ = tx.send(((z, x, y), bytes));
            inflight.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }

    fn process_tile_downloads(&mut self, ctx: &egui::Context) {
        while let Ok(((z, x, y), bytes)) = self.tile_download_rx.try_recv() {
            self.tile_pending.remove(&(z, x, y));
            if let Ok(img) = image::load_from_memory(&bytes) {
                let rgba = img.to_rgba8();
                let pixels = rgba.into_raw();
                let color_image = egui::ColorImage::from_rgba_unmultiplied([256, 256], &pixels);
                let name = format!("tile_{z}_{x}_{y}");
                let handle = ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR);
                self.tile_cache.insert((z, x, y), handle);
            }
        }
    }

    fn prefetch_adjacent_zoom(&mut self, rect: egui::Rect) {
        let zoom = self.tile_zoom;
        let tile_px = 256.0_f64;
        let half_w = f64::from(rect.width() / 2.0);
        let half_h = f64::from(rect.height() / 2.0);
        for &z2 in &[zoom.wrapping_sub(1), zoom + 1] {
            if z2 < 2 || z2 > 18 || z2 == zoom {
                continue;
            }
            let scale = 2.0_f64.powi(z2 as i32 - zoom as i32);
            let cx2 = self.tile_cx * scale;
            let cy2 = self.tile_cy * scale;
            let n = 1u64 << z2;
            let tx_s = (cx2 - half_w / tile_px).floor() as i64;
            let tx_e = (cx2 + half_w / tile_px).ceil() as i64;
            let ty_s = (cy2 - half_h / tile_px).floor() as i64;
            let ty_e = (cy2 + half_h / tile_px).ceil() as i64;
            for tx in tx_s..tx_e {
                for ty in ty_s..ty_e {
                    let wt = tx.rem_euclid(n as i64) as u32;
                    let wu = ty.rem_euclid(n as i64) as u32;
                    if !self.tile_cache.contains_key(&(z2, wt, wu)) {
                        self.request_tile(z2, wt, wu);
                    }
                }
            }
        }
    }

    fn maybe_geolocate(&mut self) {
        if self.geo_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.geo_rx = Some(rx);
        std::thread::spawn(move || {
            if let Ok(resp) = ureq::get("http://ip-api.com/json/")
                .header("User-Agent", "ez-sdr/0.1")
                .call()
            {
                if let Ok(json) = resp.into_body().read_json::<serde_json::Value>() {
                    if let (Some(lat), Some(lon)) = (json["lat"].as_f64(), json["lon"].as_f64()) {
                        let _ = tx.send((lat, lon));
                    }
                }
            }
        });
    }

    /// Renders the standalone ADS-B receive/antenna setup guide — used as an
    /// on-page instructions banner on the ADS-B tab.
    pub fn ui_antenna_guide(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("📡 ADS-B Antenna Setup Guide")
                .size(16.0)
                .strong(),
        );
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
            ui.separator();
            ui.label("The antenna is the #1 factor in ADS-B range. A well-placed $15 antenna beats a $200 SDR with a poor antenna every time.");
        });

        ui.add_space(4.0);
        ui.collapsing("Which antenna should I use?", |ui| {
            ui.add_space(4.0);
            egui::Grid::new("adsb_ant_table").num_columns(2).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Type").strong());
                ui.label(egui::RichText::new("Description").strong());
                ui.end_row();

                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "Quarter-wave ground plane");
                ui.label("Simplest DIY antenna: a 6.9 cm vertical element on a metal ground plane (≥15 cm square). Needs clear sky view. Cost: ~$5–15. Range: 100–250 km.");
                ui.end_row();

                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "Coaxial collinear (Co-Co)");
                ui.label("DIY from coax segments. 3–6 dB gain over a monopole. Longer vertical reach, narrower beam. Popular with feeders. Cost: ~$10–20.");
                ui.end_row();

                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "Commercial collinear");
                ui.label("FlightAware 26-inch or similar tuned 1090 MHz antenna. Pre-tuned, weatherproof. Best off-the-shelf choice for permanent outdoor install. Cost: ~$40–60.");
                ui.end_row();

                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "Stock SDR whip");
                ui.label("Works, but poorly. Expect 30–80 km range. Not tuned for 1090 MHz. Replace as soon as possible.");
                ui.end_row();
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Coax cable — don't lose your signal before it reaches the SDR", |ui| {
            ui.add_space(4.0);
            ui.label("At 1090 MHz, coax loss is severe. Every 3 dB of cable loss = roughly 30% less range.");
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Typical loss at 1090 MHz per 10 meters:").strong());
            ui.label("  RG-58/RG-316:    ~9 dB — avoid for any run over 2 m");
            ui.label("  LMR-240/RFC240:  ~5 dB — acceptable for short runs (≤5 m)");
            ui.label("  LMR-400/RFC400:  ~3 dB — good for runs up to 15 m");
            ui.label("  LMR-600:         ~1.8 dB — best for long runs");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 180, 0), "NOTE");
                ui.separator();
                ui.label("If you must run >10 m of coax, mount the RTL-SDR + Pi near the antenna and use Ethernet backhaul instead.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Filtering & LNA — the #1 upgrade for urban setups", |ui| {
            ui.add_space(4.0);
            ui.label("Cellular towers (LTE/5G at 700–900 MHz and 1800–2100 MHz) can desensitise your SDR's front-end, making it 'deaf' to weak ADS-B signals at 1090 MHz.");
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Recommended chain (in order from antenna):").strong());
            ui.label("  1. Antenna");
            ui.label("  2. 1090 MHz SAW filter (e.g. Uputronics, FlightAware, Nooelec SAWbird)");
            ui.label("  3. LNA with <1 dB noise figure (often integrated into the filter)");
            ui.label("  4. Coax cable to SDR");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
                ui.separator();
                ui.label("Buy the filter FIRST. An LNA amplifies signal AND noise equally — filtering addresses the real problem. Many combo filtered-LNA products (SAWbird+) simplify this.");
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 80, 80), "AVOID");
                ui.separator();
                ui.label("Don't buy a wideband LNA without a 1090 MHz filter. It will amplify nearby cellular interference and make things worse.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Placement — height is everything", |ui| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Approximate range by antenna location:").strong());
            ui.add_space(2.0);
            ui.label("  Indoor windowsill:    30–80 km  (worst — walls, roof, and window glass all absorb 1090 MHz)");
            ui.label("  Attic:                80–150 km (better, but roofing materials still attenuate)");
            ui.label("  Outdoor roofline:     150–300 km (dramatic improvement — clear horizon)");
            ui.label("  Mast 5–10 m high:     300–450 km (best — above obstructions, line-of-sight to horizon)");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
                ui.separator();
                ui.label("The single biggest improvement you can make: move the antenna from indoors to outdoors. This alone can triple your aircraft count.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Polarisation & antenna gain — the trade-offs", |ui| {
            ui.add_space(4.0);
            ui.label("ADS-B transponders transmit with vertical polarisation. Your antenna must also be vertically polarised (elements vertical). A horizontal antenna loses ~20 dB.");
            ui.add_space(4.0);
            ui.label("Higher-gain antennas (6–9 dBi) have a narrow vertical beam. They reach aircraft at cruise altitude (35,000 ft) further, but may miss nearby low-altitude traffic. A 2–3 dBi omnidirectional antenna gives more consistent total aircraft counts.");
        });
    }

    /// Renders just the aircraft map filling all available space — for the ADS-B tab central panel.
    pub fn ui_map(&mut self, ui: &mut egui::Ui) {
        if let Ok(state) = self.shared.try_lock() {
            if (state.config.observer_lat - self.observer_lat).abs() > 0.001
                || (state.config.observer_lon - self.observer_lon).abs() > 0.001
            {
                self.observer_lat = state.config.observer_lat;
                self.observer_lon = state.config.observer_lon;
            }
        }

        // First-time tile init: geolocate, then center on observer
        if self.geo_rx.is_none() {
            self.maybe_geolocate();
        }
        if let Some(rx) = &self.geo_rx {
            if let Ok((lat, lon)) = rx.try_recv() {
                self.observer_lat = lat;
                self.observer_lon = lon;
                self.tile_cx = Self::lon_to_tile_x(lon, self.tile_zoom);
                self.tile_cy = Self::lat_to_tile_y(lat, self.tile_zoom);
                self.geo_rx = None;
            }
        }

        // Process incoming tile downloads
        self.process_tile_downloads(ui.ctx());
        self.tile_frame_counter += 1;

        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let response = response.on_hover_text(
            "OSM map — drag to pan, scroll to zoom, click an aircraft dot to select",
        );
        let painter = ui.painter();

        // Background fill (shows behind tiles during loading)
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(28, 40, 51));

        // Scroll-to-zoom with throttled accumulator
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let zoom_delta = ui.input(|i| i.zoom_delta());
            if scroll != 0.0 {
                self.zoom_accum += f64::from(scroll) / Self::SCROLL_POINTS_PER_ZOOM_LEVEL;
            }
            if zoom_delta != 1.0 {
                self.zoom_accum += f64::from(zoom_delta).ln() / std::f64::consts::LN_2;
            }
            while self.zoom_accum.abs() >= 1.0 {
                let dz = if self.zoom_accum > 0.0 { 1i32 } else { -1i32 };
                let new_zoom = (self.tile_zoom as i32 + dz).clamp(2, 18) as u32;
                if new_zoom == self.tile_zoom {
                    self.zoom_accum = 0.0;
                    break;
                }
                let factor = 2.0_f64.powi(if dz > 0 { 1 } else { -1 });
                if let Some(mouse) = response.hover_pos() {
                    let mx = f64::from(mouse.x) - f64::from(rect.center().x);
                    let my = f64::from(mouse.y) - f64::from(rect.center().y);
                    let tile_mx = mx / 256.0 + self.tile_cx;
                    let tile_my = my / 256.0 + self.tile_cy;
                    self.tile_cx = tile_mx * factor - mx / 256.0;
                    self.tile_cy = tile_my * factor - my / 256.0;
                }
                self.tile_zoom = new_zoom;
                self.zoom_accum -= dz as f64;
                self.last_zoom_change = std::time::Instant::now();
            }
        }

        // Drag-to-pan
        if response.dragged() {
            let delta = response.drag_delta();
            self.tile_cx -= f64::from(delta.x) / 256.0;
            self.tile_cy -= f64::from(delta.y) / 256.0;
        }

        // Render OSM tiles
        let cx = self.tile_cx;
        let cy = self.tile_cy;
        let tile_px = 256.0_f64;
        let half_w = f64::from(rect.width() / 2.0);
        let half_h = f64::from(rect.height() / 2.0);
        let n = (1u64 << self.tile_zoom) as i64;
        let center_x = f64::from(rect.center().x);
        let center_y = f64::from(rect.center().y);
        let zoom = self.tile_zoom;
        let zoom_debouncing = self.last_zoom_change.elapsed()
            < std::time::Duration::from_millis(Self::ZOOM_DEBOUNCE_MS);

        let should_fetch_tiles = !zoom_debouncing || self.tile_frame_counter & 3 == 0;

        let tx_s = (cx - half_w / tile_px).floor() as i64;
        let tx_e = (cx + half_w / tile_px).ceil() as i64;
        let ty_s = (cy - half_h / tile_px).floor() as i64;
        let ty_e = (cy + half_h / tile_px).ceil() as i64;

        for tx in tx_s..tx_e {
            for ty in ty_s..ty_e {
                let sx = center_x + (tx as f64 - cx) * tile_px;
                let sy = center_y + (ty as f64 - cy) * tile_px;
                let tile_rect = egui::Rect::from_min_size(
                    egui::pos2(sx as f32, sy as f32),
                    egui::vec2(tile_px as f32, tile_px as f32),
                );
                if !tile_rect.intersects(rect) {
                    continue;
                }
                let wt = tx.rem_euclid(n) as u32;
                let wu = ty.rem_euclid(n) as u32;
                let key = (zoom, wt, wu);
                if let Some(handle) = self.tile_cache.get(&key) {
                    self.tile_last_used.insert(key, self.tile_frame_counter);
                    painter.image(
                        handle.id(),
                        tile_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                } else if should_fetch_tiles {
                    painter.rect_filled(tile_rect, 0.0, egui::Color32::from_rgb(40, 55, 70));
                    self.request_tile(zoom, wt, wu);
                }
            }
        }

        // Evict least-recently-used tiles if over the cap
        if self.tile_cache.len() > Self::MAX_CACHED_TILES {
            let mut by_age: Vec<((u32, u32, u32), u64)> =
                self.tile_last_used.iter().map(|(k, v)| (*k, *v)).collect();
            by_age.sort_by_key(|(_, frame)| *frame);
            let excess = self.tile_cache.len() - Self::MAX_CACHED_TILES;
            for (key, _) in by_age.into_iter().take(excess) {
                self.tile_cache.remove(&key);
                self.tile_last_used.remove(&key);
            }
        }

        // Prefetch adjacent zoom levels (debounced)
        if !zoom_debouncing {
            self.prefetch_adjacent_zoom(rect);
        }

        // Click handler (select aircraft) — use coordinate from tile projection
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let mut closest_icao = None;
                let mut closest_dist = f32::INFINITY;
                let cx_f32 = rect.center().x;
                let cy_f32 = rect.center().y;
                for ac in &self.aircraft {
                    if ac.lat == 0.0 && ac.lon == 0.0 {
                        continue;
                    }
                    let tx_f = Self::lon_to_tile_x(ac.lon, self.tile_zoom);
                    let ty_f = Self::lat_to_tile_y(ac.lat, self.tile_zoom);
                    let x = cx_f32 + (tx_f - self.tile_cx) as f32 * 256.0;
                    let y = cy_f32 + (ty_f - self.tile_cy) as f32 * 256.0;
                    let dist = ((pos.x - x).powi(2) + (pos.y - y).powi(2)).sqrt();
                    if dist < 14.0 && dist < closest_dist {
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

        // Observer + range rings (projected via tile coords)
        let obs_tx = Self::lon_to_tile_x(self.observer_lon, self.tile_zoom);
        let obs_ty = Self::lat_to_tile_y(self.observer_lat, self.tile_zoom);
        let obs_x = (f64::from(rect.center().x) + (obs_tx - self.tile_cx) * 256.0) as f32;
        let obs_y = (f64::from(rect.center().y) + (obs_ty - self.tile_cy) * 256.0) as f32;
        if rect.contains(egui::pos2(obs_x, obs_y)) {
            for (dist_km, alpha) in [
                (50.0_f64, 40u8),
                (100.0_f64, 30),
                (200.0_f64, 20),
                (400.0_f64, 12),
            ] {
                let ang_dist = dist_km / 6371.0;
                let mut ring_points: Vec<egui::Pos2> = Vec::with_capacity(72);
                for deg in (0..360).step_by(5) {
                    let brng = f64::from(deg).to_radians();
                    let lat2 = (self.observer_lat.to_radians().sin() * ang_dist.cos()
                        + self.observer_lat.to_radians().cos() * ang_dist.sin() * brng.cos())
                    .asin();
                    let lon2 = self.observer_lon.to_radians()
                        + (brng.sin() * ang_dist.sin() * self.observer_lat.to_radians().cos())
                            .atan2(
                                ang_dist.cos() - self.observer_lat.to_radians().sin() * lat2.sin(),
                            );
                    let t2x = Self::lon_to_tile_x(lon2.to_degrees(), self.tile_zoom);
                    let t2y = Self::lat_to_tile_y(lat2.to_degrees(), self.tile_zoom);
                    let rx = (f64::from(rect.center().x) + (t2x - self.tile_cx) * 256.0) as f32;
                    let ry = (f64::from(rect.center().y) + (t2y - self.tile_cy) * 256.0) as f32;
                    ring_points.push(egui::pos2(rx, ry));
                }
                for w in ring_points.windows(2) {
                    painter.line_segment(
                        [w[0], w[1]],
                        egui::Stroke::new(
                            0.5,
                            egui::Color32::from_rgba_unmultiplied(100, 200, 100, alpha),
                        ),
                    );
                }
                if let Some(first) = ring_points.first() {
                    painter.text(
                        *first + egui::vec2(3.0, -3.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{}km", dist_km as u32),
                        egui::FontId::proportional(9.0),
                        egui::Color32::from_rgba_unmultiplied(120, 220, 120, alpha),
                    );
                }
            }
            painter.circle_stroke(
                egui::pos2(obs_x, obs_y),
                6.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 240, 80)),
            );
            painter.circle_filled(
                egui::pos2(obs_x, obs_y),
                2.5,
                egui::Color32::from_rgb(255, 240, 80),
            );
        }

        // Trails
        if self.show_trails {
            for (icao, trail) in &self.aircraft_trails {
                if trail.len() < 2 {
                    continue;
                }
                let trail_color = if self.selected_icao == Some(*icao) {
                    egui::Color32::from_rgba_unmultiplied(0, 200, 255, 130)
                } else {
                    egui::Color32::from_rgba_unmultiplied(50, 200, 50, 80)
                };
                let pts: Vec<egui::Pos2> = trail
                    .iter()
                    .map(|&(lat, lon)| {
                        let tix = Self::lon_to_tile_x(lon, self.tile_zoom);
                        let tiy = Self::lat_to_tile_y(lat, self.tile_zoom);
                        egui::pos2(
                            (f64::from(rect.center().x) + (tix - self.tile_cx) * 256.0) as f32,
                            (f64::from(rect.center().y) + (tiy - self.tile_cy) * 256.0) as f32,
                        )
                    })
                    .collect();
                for w in pts.windows(2) {
                    painter.line_segment([w[0], w[1]], egui::Stroke::new(1.2, trail_color));
                }
            }
        }

        // Aircraft
        let map_now = std::time::Instant::now();
        for ac in &self.aircraft {
            let age = map_now.duration_since(ac.seen).as_secs();
            if age > self.max_age_secs {
                continue;
            }
            if self.altitude_filter_enabled
                && (ac.altitude < self.min_altitude_ft || ac.altitude > self.max_altitude_ft)
            {
                continue;
            }
            if ac.lat == 0.0 && ac.lon == 0.0 {
                continue;
            }
            let tix = Self::lon_to_tile_x(ac.lon, self.tile_zoom);
            let tiy = Self::lat_to_tile_y(ac.lat, self.tile_zoom);
            let x = (f64::from(rect.center().x) + (tix - self.tile_cx) * 256.0) as f32;
            let y = (f64::from(rect.center().y) + (tiy - self.tile_cy) * 256.0) as f32;
            let color = if self.selected_icao == Some(ac.icao) {
                egui::Color32::from_rgb(0, 255, 255)
            } else {
                let t = (ac.altitude as f32 / 40_000.0).clamp(0.0, 1.0);
                if t < 0.5 {
                    let u = t * 2.0;
                    egui::Color32::from_rgb(
                        (u * 50.0) as u8,
                        (100.0 + u * 155.0) as u8,
                        (200.0 - u * 200.0) as u8,
                    )
                } else {
                    let u = (t - 0.5) * 2.0;
                    egui::Color32::from_rgb((50.0 + u * 205.0) as u8, (255.0 - u * 205.0) as u8, 0)
                }
            };
            let model_scale = if self.selected_icao == Some(ac.icao) {
                10.0_f32
            } else {
                6.5_f32
            };
            let model_str = self
                .aircraft_info
                .get(&ac.icao)
                .map_or("", |i| i.model.as_str());
            let category = classify_aircraft(model_str);
            draw_plane_model(
                painter,
                egui::pos2(x, y),
                ac.heading as f32,
                &category,
                color,
                model_scale,
            );
            if !ac.callsign.is_empty() {
                painter.text(
                    egui::pos2(x + 6.0, y - 9.0),
                    egui::Align2::LEFT_CENTER,
                    &ac.callsign,
                    egui::FontId::proportional(10.0),
                    color,
                );
            }
        }

        // Altitude color legend
        let legend_x = rect.right() - 28.0;
        let legend_top = rect.top() + 8.0;
        let legend_h = (rect.height() - 16.0).min(200.0);
        for i in 0..=40 {
            let t = i as f32 / 40.0;
            let ly = legend_top + legend_h * (1.0 - t);
            let color = if t < 0.5 {
                let u = t * 2.0;
                egui::Color32::from_rgb(
                    (u * 50.0) as u8,
                    (100.0 + u * 155.0) as u8,
                    (200.0 - u * 200.0) as u8,
                )
            } else {
                let u = (t - 0.5) * 2.0;
                egui::Color32::from_rgb((50.0 + u * 205.0) as u8, (255.0 - u * 205.0) as u8, 0)
            };
            painter.line_segment(
                [egui::pos2(legend_x, ly), egui::pos2(legend_x + 10.0, ly)],
                egui::Stroke::new(2.0, color),
            );
        }
        painter.text(
            egui::pos2(legend_x + 5.0, legend_top),
            egui::Align2::CENTER_TOP,
            "40k ft",
            egui::FontId::proportional(7.0),
            egui::Color32::GRAY,
        );
        painter.text(
            egui::pos2(legend_x + 5.0, legend_top + legend_h),
            egui::Align2::CENTER_BOTTOM,
            "0 ft",
            egui::FontId::proportional(7.0),
            egui::Color32::GRAY,
        );
    }

    /// Renders the aircraft list and controls — for the ADS-B tab right sidebar.
    pub fn ui_list(&mut self, ui: &mut egui::Ui) {
        if !self.checklist.ui(ui) {
            if let Some(msg) = self.checklist.pending_status.take() {
                self.pending_status_flash = Some(msg);
            }
            return;
        }
        if let Some(msg) = self.checklist.pending_status.take() {
            self.pending_status_flash = Some(msg);
        }

        while let Ok((icao, info)) = self.info_rx.try_recv() {
            self.aircraft_info.insert(icao, info);
        }

        if let Ok(state) = self.shared.try_lock() {
            if (state.config.observer_lat - self.observer_lat).abs() > 0.001
                || (state.config.observer_lon - self.observer_lon).abs() > 0.001
            {
                self.observer_lat = state.config.observer_lat;
                self.observer_lon = state.config.observer_lon;
            }
        }

        // Stats + start/stop
        let now_inst = std::time::Instant::now();
        let active_count = self
            .aircraft
            .iter()
            .filter(|ac| now_inst.duration_since(ac.seen).as_secs() <= self.max_age_secs)
            .count();
        let with_pos = self
            .aircraft
            .iter()
            .filter(|ac| {
                now_inst.duration_since(ac.seen).as_secs() <= self.max_age_secs
                    && (ac.lat != 0.0 || ac.lon != 0.0)
            })
            .count();
        let msg_rate = if let Some(start) = self.start_time {
            self.total_messages as f64 / start.elapsed().as_secs_f64().max(0.001)
        } else {
            0.0
        };
        ui.label(format!(
            "✈ {active_count} aircraft  ({with_pos} w/pos)  {msg_rate:.0} msg/s"
        ));

        ui.horizontal(|ui| {
            if self.start_time.is_some() {
                if ui.button("■ Stop").clicked() {
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.adsb_running = false;
                    }
                    self.start_time = None;
                }
            } else if ui.button("▶ Start ADS-B").clicked() {
                self.begin();
            }
            ui.checkbox(&mut self.alert_enabled, "🔔");
            if self.alert_enabled {
                ui.add(
                    egui::DragValue::new(&mut self.alert_range_km)
                        .speed(5.0)
                        .range(0..=1000)
                        .suffix("km"),
                );
                ui.label("(0=any)");
            }
        });

        // Update trails here so they're ready when ui_map() renders
        for ac in &self.aircraft {
            if ac.lat == 0.0 && ac.lon == 0.0 {
                continue;
            }
            let trail = self.aircraft_trails.entry(ac.icao).or_default();
            if trail.back().is_none_or(|&(lat, lon)| {
                (lat - ac.lat).abs() > 0.001 || (lon - ac.lon).abs() > 0.001
            }) {
                trail.push_back((ac.lat, ac.lon));
                if trail.len() > 30 {
                    trail.pop_front();
                }
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.altitude_filter_enabled, "Alt");
            if self.altitude_filter_enabled {
                ui.add(
                    egui::DragValue::new(&mut self.min_altitude_ft)
                        .speed(500.0)
                        .range(0..=60_000)
                        .suffix("↑"),
                );
                ui.add(
                    egui::DragValue::new(&mut self.max_altitude_ft)
                        .speed(500.0)
                        .range(0..=100_000)
                        .suffix("↑max"),
                );
            }
        });
        ui.horizontal(|ui| {
            ui.label("Age:");
            ui.add(
                egui::DragValue::new(&mut self.max_age_secs)
                    .speed(5.0)
                    .range(10..=600)
                    .suffix("s"),
            );
            ui.checkbox(&mut self.show_trails, "Trails");
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.callsign_filter)
                    .desired_width(100.0)
                    .hint_text("search callsign/ICAO"),
            );
            if !self.callsign_filter.is_empty() && ui.small_button("✕").clicked() {
                self.callsign_filter.clear();
            }
        });

        // Selected aircraft detail
        if let Some(icao) = self.selected_icao {
            if let Some(info) = self.aircraft_info.get(&icao) {
                ui.separator();
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(15, 25, 20))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("{icao:06X}"))
                                .strong()
                                .color(egui::Color32::from_rgb(0, 220, 255)),
                        );
                        if info.model != "Loading..." && info.model != "Unknown" {
                            ui.label(egui::RichText::new(&info.model).small());
                            if !info.operator.is_empty() && info.operator != "Unknown" {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} | {}",
                                        info.operator, info.registration
                                    ))
                                    .small()
                                    .color(egui::Color32::GRAY),
                                );
                            }
                        }
                    });
            }
        }

        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("adsb_list_compact").num_columns(6).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Flight").small().strong());
                ui.label(egui::RichText::new("Alt").small().strong());
                ui.label(egui::RichText::new("Spd").small().strong());
                ui.label(egui::RichText::new("Dist").small().strong());
                ui.label(egui::RichText::new("Age").small().strong());
                ui.label(egui::RichText::new("").small());
                ui.end_row();

                let now = std::time::Instant::now();
                let max_age = self.max_age_secs;
                let mut fetch_icao: Option<u32> = None;
                for ac in &self.aircraft {
                    let age = now.duration_since(ac.seen).as_secs();
                    if age > max_age { continue; }
                    if self.altitude_filter_enabled && (ac.altitude < self.min_altitude_ft || ac.altitude > self.max_altitude_ft) { continue; }
                    if !self.callsign_filter.is_empty() {
                        let q = self.callsign_filter.to_lowercase();
                        let icao_str = format!("{:06x}", ac.icao);
                        if !ac.callsign.to_lowercase().contains(&q) && !icao_str.contains(&q) { continue; }
                    }
                    let is_selected = self.selected_icao == Some(ac.icao);
                    let age_frac = (age as f32 / max_age as f32).clamp(0.0, 1.0);
                    let brightness = (255.0 * (1.0 - age_frac * 0.65)) as u8;
                    let row_col = if is_selected { egui::Color32::from_rgb(0, 255, 255) } else { egui::Color32::from_rgb(brightness, brightness, brightness) };
                    let label = if ac.callsign.is_empty() { format!("{:06X}", ac.icao) } else { ac.callsign.clone() };
                    if ui.label(egui::RichText::new(&label).color(row_col).small()).clicked() {
                        self.selected_icao = Some(ac.icao);
                        fetch_icao = Some(ac.icao);
                    }
                    ui.label(egui::RichText::new(format!("{}ft", ac.altitude)).color(row_col).small());
                    ui.label(egui::RichText::new(format!("{}kt", ac.speed)).color(row_col).small());
                    let dist = self.haversine_distance(self.observer_lat, self.observer_lon, ac.lat, ac.lon);
                    ui.label(egui::RichText::new(format!("{dist:.0}km")).color(row_col).small());
                    let age_color = if age < 10 { egui::Color32::GREEN } else if age < 30 { egui::Color32::YELLOW } else { egui::Color32::GRAY };
                    ui.label(egui::RichText::new(format!("{age}s")).color(age_color).small());
                    if ui.small_button("🤖").clicked() {
                        let bearing = self.bearing(self.observer_lat, self.observer_lon, ac.lat, ac.lon);
                        self.pending_ai_prompt = Some(format!(
                            "Aircraft on ADS-B:\nCallsign: {}\nICAO: {:06X}\nAlt: {} ft, Speed: {} kt, Heading: {}°\nPos: {:.4}°N, {:.4}°E\nDist: {:.1} km, Bearing: {:.0}°",
                            label, ac.icao, ac.altitude, ac.speed, ac.heading, ac.lat, ac.lon, dist, bearing
                        ));
                    }
                    ui.end_row();
                }
                if let Some(icao) = fetch_icao {
                    self.fetch_aircraft_info(icao);
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_shared_state;

    #[test]
    fn test_new_defaults() {
        let panel = AdsBPanel::new(make_shared_state());
        assert!(panel.aircraft.is_empty());
        assert!(panel.selected_icao.is_none());
        assert_eq!(panel.total_messages, 0);
        assert_eq!(panel.decode_stats, (0, 0, 0));
        assert!(panel.start_time.is_none());
        assert_eq!(panel.min_altitude_ft, 0);
        assert_eq!(panel.max_altitude_ft, 60_000);
        assert!(!panel.altitude_filter_enabled);
        assert!(panel.max_age_secs > 0);
        assert!(panel.callsign_filter.is_empty());
        assert!(panel.show_trails);
        assert!(panel.pending_ai_prompt.is_none());
        assert!(panel.alert_enabled);
        assert!(!panel.desktop_notifications);
    }

    #[test]
    fn test_classify_aircraft() {
        assert_eq!(classify_aircraft("Boeing 737"), AcCategory::NarrowBody);
        assert_eq!(classify_aircraft("Airbus A320"), AcCategory::NarrowBody);
        assert_eq!(classify_aircraft("Bombardier CRJ900"), AcCategory::Regional);
        assert_eq!(classify_aircraft("Cessna 172"), AcCategory::Generic);
        assert_eq!(classify_aircraft("Boeing 777"), AcCategory::WideBody);
        assert_eq!(classify_aircraft(""), AcCategory::Generic);
        assert_eq!(classify_aircraft("boeing 737"), AcCategory::NarrowBody);
        assert_eq!(classify_aircraft("xyzzy"), AcCategory::Generic);
    }

    #[test]
    fn test_lon_to_tile_x() {
        let eps = 1e-12;
        assert!((AdsBPanel::lon_to_tile_x(0.0, 0) - 0.5).abs() < eps);
        assert!((AdsBPanel::lon_to_tile_x(-180.0, 0) - 0.0).abs() < eps);
        assert!((AdsBPanel::lon_to_tile_x(180.0, 0) - 1.0).abs() < eps);
        assert!((AdsBPanel::lon_to_tile_x(0.0, 10) - 512.0).abs() < eps);
        assert!((AdsBPanel::lon_to_tile_x(-90.0, 10) - 256.0).abs() < eps);
        assert!((AdsBPanel::lon_to_tile_x(90.0, 10) - 768.0).abs() < eps);
    }

    #[test]
    fn test_lat_to_tile_y() {
        let eps = 1e-6;
        assert!((AdsBPanel::lat_to_tile_y(0.0, 0) - 0.5).abs() < eps);
        assert!((AdsBPanel::lat_to_tile_y(85.0511, 0) - 0.0).abs() < 0.01);
        assert!((AdsBPanel::lat_to_tile_y(-85.0511, 0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_haversine_distance() {
        let panel = AdsBPanel::new(make_shared_state());
        let d = panel.haversine_distance(51.5, -0.1, 51.5, -0.1);
        assert!(d.abs() < 1e-6);
        let meridian_1deg = panel.haversine_distance(0.0, 0.0, 1.0, 0.0);
        assert!((meridian_1deg - 111.195).abs() < 0.1);
    }

    #[test]
    fn test_bearing() {
        let panel = AdsBPanel::new(make_shared_state());
        let north = panel.bearing(0.0, 0.0, 10.0, 0.0);
        assert!((north - 0.0).abs() < 1.0);
        let east = panel.bearing(0.0, 0.0, 0.0, 10.0);
        assert!((east - 90.0).abs() < 1.0);
        let south = panel.bearing(10.0, 0.0, 0.0, 0.0);
        assert!((south - 180.0).abs() < 1.0);
    }

    #[test]
    fn test_check_for_new_aircraft_adds_new() {
        let mut panel = AdsBPanel::new(make_shared_state());
        panel.check_for_new_aircraft();
        panel.aircraft.push(AircraftEntry {
            icao: 123456,
            callsign: "TEST".into(),
            lat: 51.5,
            lon: -0.1,
            altitude: 35000,
            speed: 450,
            heading: 270,
            seen: std::time::Instant::now(),
        });
        panel.check_for_new_aircraft();
    }

    #[test]
    fn test_render_toasts() {
        let ctx = egui::Context::default();
        let mut panel = AdsBPanel::new(make_shared_state());
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            panel.render_toasts(ctx);
        });
        panel.notifications.push_back(AdsBNotification {
            id: 1,
            icao: 123456,
            callsign: "TEST".into(),
            lat: 51.5,
            lon: -0.1,
            altitude: 35000,
            speed: 450,
            distance_km: Some(10.0),
            bearing_deg: Some(45.0),
            timestamp: std::time::Instant::now(),
            dismissed: false,
        });
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            panel.render_toasts(ctx);
        });
    }

    #[test]
    fn test_ui_methods_no_crash() {
        let mut panel = AdsBPanel::new(make_shared_state());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new(egui::Id::new("test_antenna")).show(ctx, |ui| {
                panel.ui_antenna_guide(ui);
            });
        });
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new(egui::Id::new("test_list")).show(ctx, |ui| {
                panel.ui_list(ui);
            });
        });
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new(egui::Id::new("test_map")).show(ctx, |ui| {
                panel.ui_map(ui);
            });
        });
    }

    #[test]
    fn test_haversine_distance_known_values() {
        let panel = AdsBPanel::new(make_shared_state());
        let d = panel.haversine_distance(51.5074, -0.1278, 48.8566, 2.3522);
        assert!(
            (d - 344.0).abs() < 10.0,
            "London-Paris distance {d} not ≈344 km"
        );
        let d = panel.haversine_distance(51.5074, -0.1278, 40.7128, -74.0060);
        assert!(
            (d - 5570.0).abs() < 50.0,
            "London-NYC distance {d} not ≈5570 km"
        );
        let d = panel.haversine_distance(51.5, -0.1, 51.5, -0.1);
        assert!(d.abs() < 1e-6);
    }

    #[test]
    fn test_bearing_known_values() {
        let panel = AdsBPanel::new(make_shared_state());
        let b = panel.bearing(0.0, 0.0, 10.0, 0.0);
        assert!((b - 0.0).abs() < 1.0, "north bearing {b} not ≈0°");
        let b = panel.bearing(0.0, 0.0, 0.0, 10.0);
        assert!((b - 90.0).abs() < 1.0, "east bearing {b} not ≈90°");
    }
}
