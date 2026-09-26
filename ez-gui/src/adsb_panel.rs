use crate::app::SharedState;
use crate::uat_receiver::{address_label, TimedReport, UatReceiver, UatStatus};
use crate::ui_kit::module_card;
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
    tile_download_tx: std::sync::mpsc::Sender<((u32, u32, u32), Result<egui::ColorImage, String>)>,
    tile_download_rx:
        std::sync::mpsc::Receiver<((u32, u32, u32), Result<egui::ColorImage, String>)>,
    tile_failures: std::collections::HashMap<(u32, u32, u32), (std::time::Instant, u32)>,
    tile_last_error: Option<String>,
    tile_zoom: u32,
    tile_cx: f64,
    tile_cy: f64,
    zoom_accum: f64,
    tile_last_used: std::collections::HashMap<(u32, u32, u32), u64>,
    tile_frame_counter: u64,
    tile_inflight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    last_zoom_change: std::time::Instant,
    geo_rx: Option<std::sync::mpsc::Receiver<(f64, f64)>>,
    plane_texture: Option<egui::TextureHandle>,
    pub region: AdsbRegion,
    pub uat_address: String,
    uat_receiver: UatReceiver,
    uat_position_seen: std::collections::HashMap<u32, std::time::Instant>,
}

/// Mode S is decoded locally; UAT reports come from dump978-fa's JSON TCP feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdsbRegion {
    ModeS1090,
    Uat978,
}

impl AdsbRegion {
    pub fn label(self) -> &'static str {
        match self {
            Self::ModeS1090 => "1090 MHz · Mode S",
            Self::Uat978 => "978 MHz · UAT",
        }
    }

    pub fn frequency_hz(self) -> u64 {
        match self {
            Self::ModeS1090 => 1_090_000_000,
            Self::Uat978 => 978_000_000,
        }
    }

    pub fn sample_rate_hz(self) -> u32 {
        match self {
            Self::ModeS1090 => 2_400_000,
            // FlightAware dump978's receive rate (external decoder owns tuning).
            Self::Uat978 => 2_083_333,
        }
    }
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
        let (observer_lat, observer_lon) = shared
            .try_lock()
            .map(|state| (state.config.observer_lat, state.config.observer_lon))
            .unwrap_or((51.5, -0.1));
        let init_cx = Self::lon_to_tile_x(observer_lon, 8);
        let init_cy = Self::lat_to_tile_y(observer_lat, 8);
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
            observer_lat,
            observer_lon,
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
            tile_failures: std::collections::HashMap::new(),
            tile_last_error: None,
            tile_zoom: 8,
            tile_cx: init_cx,
            tile_cy: init_cy,
            zoom_accum: 0.0,
            last_zoom_change: std::time::Instant::now(),
            tile_last_used: std::collections::HashMap::new(),
            tile_frame_counter: 0,
            tile_inflight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            geo_rx: None,
            plane_texture: None,
            region: AdsbRegion::ModeS1090,
            uat_address: crate::uat_receiver::DEFAULT_ADDRESS.into(),
            uat_receiver: UatReceiver::default(),
            uat_position_seen: std::collections::HashMap::new(),
        }
    }

    /// Auto-configure the SDR for ADS-B and start receiving. Idempotent: if the
    /// receiver is already running it does nothing (so re-entering the Planes
    /// mode doesn't reset stats). Called on entry to the Planes mode.
    pub fn begin(&mut self) {
        if self.region == AdsbRegion::Uat978 {
            if !self.uat_receiver.is_active() {
                self.connect_uat();
            }
            return;
        }
        let shared = self.shared.clone();
        let Ok(mut state) = shared.try_lock() else {
            self.pending_status_flash = Some("Receiver busy; try Start again".into());
            return;
        };
        if state.adsb_running {
            return;
        }
        state.audio_running = false;
        state
            .source
            .tune_and_restart(self.region.frequency_hz(), self.region.sample_rate_hz());
        if state.source.source_mode == crate::source_manager::SourceMode::Daemon {
            state.source.start();
        }
        state.adsb_running = true;
        self.start_time = Some(std::time::Instant::now());
    }

    /// Change source ownership before receiving: dump978-fa needs exclusive SDR
    /// access for 978 MHz; returning to 1090 restores the normal local source.
    pub fn set_region(&mut self, region: AdsbRegion) {
        if self.region == region {
            return;
        }
        let shared = self.shared.clone();
        let Ok(mut state) = shared.try_lock() else {
            self.pending_status_flash = Some("Receiver busy; try the band switch again".into());
            return;
        };
        self.stop_uat();
        self.region = region;
        self.clear_tracks();
        state.audio_running = false;
        if region == AdsbRegion::Uat978 {
            state.adsb_running = false;
            state.source.stop();
        } else {
            state
                .source
                .tune_and_restart(region.frequency_hz(), region.sample_rate_hz());
            if state.source.source_mode == crate::source_manager::SourceMode::Daemon {
                state.source.start();
            }
            state.adsb_running = true;
        }
        drop(state);
        if region == AdsbRegion::Uat978 {
            self.start_uat_connection();
            return;
        }
        self.start_time = Some(std::time::Instant::now());
    }

    pub(crate) fn clear_tracks(&mut self) {
        self.aircraft.clear();
        self.aircraft_trails.clear();
        self.uat_position_seen.clear();
        self.aircraft_info.clear();
        self.known_icao.clear();
        self.selected_icao = None;
        self.total_messages = 0;
        self.decode_stats = (0, 0, 0);
        self.notifications.clear();
        self.pending_ai_prompt = None;
        self.pending_status_flash = None;
        // In-flight enrichment belongs to the previous capture. Retire its
        // channel so late replies cannot repopulate the freshly cleared UI.
        (self.info_tx, self.info_rx) = std::sync::mpsc::channel();
    }

    fn connect_uat(&mut self) {
        let Ok(mut state) = self.shared.try_lock() else {
            self.pending_status_flash = Some("Receiver busy; try UAT Connect again".into());
            return;
        };
        state.audio_running = false;
        state.adsb_running = false;
        state.source.stop();
        drop(state);
        self.clear_tracks();
        self.start_uat_connection();
    }

    fn start_uat_connection(&mut self) {
        match self.uat_receiver.start(&self.uat_address) {
            Ok(()) => self.start_time = Some(std::time::Instant::now()),
            Err(message) => {
                self.pending_status_flash = Some(message);
                self.start_time = None;
            }
        }
    }

    pub fn stop_uat(&mut self) {
        self.uat_receiver.stop();
        if self.region == AdsbRegion::Uat978 {
            self.start_time = None;
        }
    }

    fn stop_receiving(&mut self) {
        let shared = self.shared.clone();
        let Ok(mut state) = shared.try_lock() else {
            self.pending_status_flash = Some("Receiver busy; try Stop again".into());
            return;
        };
        state.adsb_running = false;
        state.source.stop();
        drop(state);
        self.stop_uat();
        self.start_time = None;
    }

    pub fn receiver_status(&self) -> String {
        if self.region == AdsbRegion::ModeS1090 {
            return "Mode S · local decoder".into();
        }
        match self.uat_receiver.status() {
            UatStatus::Stopped => "UAT · disconnected".into(),
            UatStatus::Connecting => "UAT · connecting to dump978-fa".into(),
            UatStatus::Connected => {
                format!("UAT · connected · {} reports", self.uat_receiver.counts().0)
            }
            UatStatus::Reconnecting(_) => "UAT · reconnecting in 2 s".into(),
            UatStatus::Error(_) => "UAT · connection error".into(),
        }
    }

    /// Drain bounded report batches without blocking egui. Partial messages
    /// merge into the same track; missing positions never invent map markers.
    pub fn poll_uat(&mut self) {
        if self.region != AdsbRegion::Uat978 {
            return;
        }
        for _ in 0..512 {
            let Some(report) = self.uat_receiver.try_recv() else {
                break;
            };
            self.ingest_uat_report(report);
        }
        let now = std::time::Instant::now();
        let retention_secs = self.max_age_secs.max(120);
        self.aircraft
            .retain(|ac| now.duration_since(ac.seen).as_secs() <= retention_secs);
        let live: std::collections::HashSet<u32> = self.aircraft.iter().map(|ac| ac.icao).collect();
        self.uat_position_seen.retain(|key, seen| {
            live.contains(key) && now.duration_since(*seen).as_secs() <= self.max_age_secs
        });
        self.aircraft_trails
            .retain(|key, _| self.uat_position_seen.contains_key(key));
        self.aircraft_info.retain(|key, _| live.contains(key));
        self.known_icao.retain(|key| live.contains(key));
        let (received, rejected, dropped) = self.uat_receiver.counts();
        self.total_messages = received;
        self.decode_stats = (received + rejected, received, rejected + dropped);
    }

    fn ingest_uat_report(&mut self, timed: TimedReport) {
        let report = timed.report;
        let index = if let Some(index) = self.aircraft.iter().position(|ac| ac.icao == report.key) {
            index
        } else {
            // Even a hostile or misconfigured feed cannot grow tracking forever.
            if self.aircraft.len() >= 4096 {
                let oldest = self
                    .aircraft
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, ac)| ac.seen)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let removed = self.aircraft.swap_remove(oldest).icao;
                self.uat_position_seen.remove(&removed);
                self.aircraft_trails.remove(&removed);
                self.aircraft_info.remove(&removed);
            }
            self.aircraft.push(AircraftEntry {
                icao: report.key,
                seen: timed.received,
                ..Default::default()
            });
            self.aircraft.len() - 1
        };
        let aircraft = &mut self.aircraft[index];
        aircraft.seen = timed.received;
        if let Some(callsign) = report.callsign {
            aircraft.callsign = callsign;
        }
        if let Some((lat, lon)) = report.position {
            aircraft.lat = lat;
            aircraft.lon = lon;
            self.uat_position_seen.insert(report.key, timed.received);
        }
        if let Some(altitude) = report.altitude_ft {
            aircraft.altitude = altitude.max(0) as u32;
        }
        if let Some(speed) = report.ground_speed_kt {
            aircraft.speed = speed.round() as u32;
        }
        if let Some(heading) = report.heading_deg {
            aircraft.heading = heading.round() as u32 % 360;
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_uat_fixture(&mut self, report: crate::uat_receiver::UatReport) {
        self.ingest_uat_report(TimedReport {
            report,
            received: std::time::Instant::now(),
        });
    }

    fn has_position(&self, aircraft: &AircraftEntry) -> bool {
        if !aircraft.lat.is_finite()
            || !aircraft.lon.is_finite()
            || !(-90.0..=90.0).contains(&aircraft.lat)
            || !(-180.0..=180.0).contains(&aircraft.lon)
        {
            return false;
        }
        if self.region == AdsbRegion::Uat978 {
            self.uat_position_seen
                .get(&aircraft.icao)
                .is_some_and(|seen| seen.elapsed().as_secs() <= self.max_age_secs)
        } else {
            aircraft.lat.is_finite()
                && aircraft.lon.is_finite()
                && (aircraft.lat != 0.0 || aircraft.lon != 0.0)
        }
    }

    fn matches_filters(&self, aircraft: &AircraftEntry) -> bool {
        if aircraft.seen.elapsed().as_secs() > self.max_age_secs
            || (self.altitude_filter_enabled
                && (aircraft.altitude < self.min_altitude_ft
                    || aircraft.altitude > self.max_altitude_ft))
        {
            return false;
        }
        let query = self.callsign_filter.trim().to_ascii_lowercase();
        query.is_empty()
            || aircraft.callsign.to_ascii_lowercase().contains(&query)
            || format!("{:06x}", aircraft.icao & 0x00ff_ffff).contains(&query)
    }

    /// Compact band selector shared by both real receive paths.
    pub fn ui_region_picker(&mut self, ui: &mut egui::Ui) -> bool {
        let mut selected = self.region;
        let mut changed = false;
        for region in [AdsbRegion::ModeS1090, AdsbRegion::Uat978] {
            if ui
                .selectable_value(&mut selected, region, region.label())
                .clicked()
            {
                let before = self.region;
                self.set_region(region);
                changed = before != self.region;
            }
        }
        changed
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

        if !adsb_running && !(self.region == AdsbRegion::Uat978 && self.uat_receiver.is_active()) {
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

            let has_pos = self.has_position(ac);
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
                address_label(ac.icao)
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
                    let hover = if n.distance_km.is_some() {
                        format!(
                            "Address {} · {:.4}, {:.4} · {} kt",
                            address_label(n.icao),
                            n.lat,
                            n.lon,
                            n.speed
                        )
                    } else {
                        format!("Address {} · {} kt", address_label(n.icao), n.speed)
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
        // UAT anonymous/trackfile/ground addresses are not ICAO registrations.
        if icao > 0x00ff_ffff {
            return;
        }
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
                        json["photos"][0]["plane"]["model"]
                            .as_str()
                            .unwrap_or("Unknown")
                            .to_string(),
                        json["photos"][0]["airline"]["name"]
                            .as_str()
                            .unwrap_or("Unknown")
                            .to_string(),
                        json["photos"][0]["registration"]
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
    const MAX_TILE_BYTES: usize = 1024 * 1024;

    fn lon_to_tile_x(lon: f64, zoom: u32) -> f64 {
        let n = (1u64 << zoom) as f64;
        (lon + 180.0) / 360.0 * n
    }

    fn lat_to_tile_y(lat: f64, zoom: u32) -> f64 {
        let n = (1u64 << zoom) as f64;
        let lat_rad = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
        (1.0 - (lat_rad.tan().asinh() / std::f64::consts::PI)) / 2.0 * n
    }

    fn tile_x_delta(&self, x: f64) -> f64 {
        let width = (1_u64 << self.tile_zoom) as f64;
        (x - self.tile_cx + width / 2.0).rem_euclid(width) - width / 2.0
    }

    fn clamp_map_center(&mut self) {
        let width = (1_u64 << self.tile_zoom) as f64;
        self.tile_cx = self.tile_cx.rem_euclid(width);
        // Web Mercator wraps only horizontally. Beyond the poles is blank.
        self.tile_cy = self.tile_cy.clamp(0.0, width);
    }

    fn plane_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.plane_texture.is_none() {
            if let Ok(image) = egui_extras::image::load_svg_bytes(
                include_bytes!("../assets/plane.svg"),
                &Default::default(),
            ) {
                self.plane_texture =
                    Some(ctx.load_texture("adsb-plane-svg", image, egui::TextureOptions::LINEAR));
            }
        }
        self.plane_texture.clone()
    }

    fn recenter_on_observer(&mut self) {
        self.tile_cx = Self::lon_to_tile_x(self.observer_lon, self.tile_zoom);
        self.tile_cy = Self::lat_to_tile_y(self.observer_lat, self.tile_zoom);
        self.zoom_accum = 0.0;
    }

    fn tile_disk_path(z: u32, x: u32, y: u32) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("tile_cache/{z}/{x}/{y}.png"))
    }

    fn read_tile_bytes(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        reader
            .take(Self::MAX_TILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > Self::MAX_TILE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Map tile exceeds 1 MiB",
            ));
        }
        Ok(bytes)
    }

    fn decode_tile(bytes: &[u8]) -> Result<egui::ColorImage, String> {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(256);
        limits.max_image_height = Some(256);
        limits.max_alloc = Some(4 * 1024 * 1024);
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
        reader.limits(limits);
        let img = reader
            .decode()
            .map_err(|e| format!("Invalid map tile: {e}"))?;
        if img.width() != 256 || img.height() != 256 {
            return Err("Map tile has unexpected dimensions".into());
        }
        Ok(egui::ColorImage::from_rgba_unmultiplied(
            [256, 256],
            img.to_rgba8().as_raw(),
        ))
    }

    fn load_or_fetch_tile(z: u32, x: u32, y: u32) -> Result<egui::ColorImage, String> {
        let path = Self::tile_disk_path(z, x, y);
        if let Ok(mut file) = std::fs::File::open(&path) {
            if let Ok(cached) = Self::read_tile_bytes(&mut file) {
                if let Ok(image) = Self::decode_tile(&cached) {
                    return Ok(image);
                }
            }
        }
        let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");
        let response = ureq::get(&url)
            .header(
                "User-Agent",
                "ez-sdr/0.1 (+https://github.com/Lupc9102/ez-sdr)",
            )
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(8)))
            .build()
            .call()
            .map_err(|e| format!("Map download failed: {e}"))?;
        let bytes = Self::read_tile_bytes(&mut response.into_body().into_reader())
            .map_err(|e| e.to_string())?;
        // Validate before caching; truncated/error responses must not poison the cache.
        let image = Self::decode_tile(&bytes)?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &bytes);
        Ok(image)
    }

    fn tile_retry_delay(attempt: u32) -> std::time::Duration {
        std::time::Duration::from_secs((5_u64 << attempt.saturating_sub(1).min(4)).min(60))
    }

    fn request_tile(&mut self, z: u32, x: u32, y: u32) {
        if self.tile_pending.contains(&(z, x, y)) {
            return;
        }
        if self
            .tile_failures
            .get(&(z, x, y))
            .is_some_and(|(retry_at, _)| *retry_at > std::time::Instant::now())
        {
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
            let image = Self::load_or_fetch_tile(z, x, y);
            let _ = tx.send(((z, x, y), image));
            inflight.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }

    fn process_tile_downloads(&mut self, ctx: &egui::Context) {
        while let Ok(((z, x, y), result)) = self.tile_download_rx.try_recv() {
            self.tile_pending.remove(&(z, x, y));
            match result {
                Ok(color_image) => {
                    self.tile_failures.remove(&(z, x, y));
                    let name = format!("tile_{z}_{x}_{y}");
                    let handle = ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR);
                    self.tile_cache.insert((z, x, y), handle);
                    self.tile_last_used
                        .insert((z, x, y), self.tile_frame_counter);
                }
                Err(error) => {
                    let attempt = self
                        .tile_failures
                        .get(&(z, x, y))
                        .map_or(1, |(_, n)| n.saturating_add(1));
                    self.tile_failures.insert(
                        (z, x, y),
                        (
                            std::time::Instant::now() + Self::tile_retry_delay(attempt),
                            attempt,
                        ),
                    );
                    self.tile_last_error = Some(error);
                }
            }
        }
        while self.tile_failures.len() > 512 {
            if let Some(key) = self
                .tile_failures
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| *key)
            {
                self.tile_failures.remove(&key);
            }
        }
    }

    fn prefetch_adjacent_zoom(&mut self, rect: egui::Rect) {
        let zoom = self.tile_zoom;
        let tile_px = 256.0_f64;
        let half_w = f64::from(rect.width() / 2.0);
        let half_h = f64::from(rect.height() / 2.0);
        for &z2 in &[zoom.wrapping_sub(1), zoom + 1] {
            if !(2..=18).contains(&z2) || z2 == zoom {
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
                    if !(0..n as i64).contains(&ty) {
                        continue;
                    }
                    let wt = tx.rem_euclid(n as i64) as u32;
                    let wu = ty as u32;
                    if !self.tile_cache.contains_key(&(z2, wt, wu)) {
                        self.request_tile(z2, wt, wu);
                    }
                }
            }
        }
    }

    /// Renders the standalone ADS-B receive/antenna setup guide — used as an
    /// on-page instructions banner on the ADS-B tab.
    pub fn ui_antenna_guide(&mut self, ui: &mut egui::Ui) {
        let theme = self
            .shared
            .try_lock()
            .map(|s| s.config.theme_config.clone())
            .unwrap_or_default();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("📡 ADS-B Antenna Setup Guide")
                .size(16.0)
                .strong(),
        );
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(theme.success.to_egui(), "TIP");
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
                ui.colored_label(theme.warning.to_egui(), "NOTE");
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
                ui.colored_label(theme.success.to_egui(), "TIP");
                ui.separator();
                ui.label("Buy the filter FIRST. An LNA amplifies signal AND noise equally — filtering addresses the real problem. Many combo filtered-LNA products (SAWbird+) simplify this.");
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(theme.error.to_egui(), "AVOID");
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
                ui.colored_label(theme.success.to_egui(), "TIP");
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
        let theme = self
            .shared
            .try_lock()
            .map(|s| s.config.theme_config.clone())
            .unwrap_or_default();
        let mut observer_changed = false;
        if let Ok(state) = self.shared.try_lock() {
            if (state.config.observer_lat - self.observer_lat).abs() > 0.001
                || (state.config.observer_lon - self.observer_lon).abs() > 0.001
            {
                self.observer_lat = state.config.observer_lat;
                self.observer_lon = state.config.observer_lon;
                observer_changed = true;
            }
        }
        if observer_changed {
            self.recenter_on_observer();
        }

        // First-time tile init: geolocate, then center on observer
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
        let painter = ui.painter().with_clip_rect(rect);

        // Background fill (shows behind tiles during loading)
        painter.rect_filled(rect, 0.0, theme.bg.to_egui());

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
        self.clamp_map_center();

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
        let dark_map = ui.visuals().dark_mode;
        let tile_tint = if dark_map {
            egui::Color32::from_rgb(144, 156, 168)
        } else {
            egui::Color32::WHITE
        };
        let mut missing_tiles = 0_usize;
        let mut failed_tiles = 0_usize;

        for tx in tx_s..tx_e {
            for ty in ty_s..ty_e {
                if !(0..n).contains(&ty) {
                    continue;
                }
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
                let wu = ty as u32;
                let key = (zoom, wt, wu);
                if let Some(handle) = self.tile_cache.get(&key) {
                    self.tile_last_used.insert(key, self.tile_frame_counter);
                    painter.image(
                        handle.id(),
                        tile_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        tile_tint,
                    );
                } else {
                    missing_tiles += 1;
                    if self.tile_failures.contains_key(&key) {
                        failed_tiles += 1;
                    }
                    painter.rect_filled(tile_rect, 0.0, theme.surface.to_egui());
                    let grid = theme.text_dim.with_alpha(30).to_egui();
                    for division in 0..4 {
                        let offset = division as f32 * 64.0;
                        painter.line_segment(
                            [
                                tile_rect.left_top() + egui::vec2(offset, 0.0),
                                tile_rect.left_bottom() + egui::vec2(offset, 0.0),
                            ],
                            egui::Stroke::new(0.5, grid),
                        );
                        painter.line_segment(
                            [
                                tile_rect.left_top() + egui::vec2(0.0, offset),
                                tile_rect.right_top() + egui::vec2(0.0, offset),
                            ],
                            egui::Stroke::new(0.5, grid),
                        );
                    }
                    if should_fetch_tiles {
                        self.request_tile(zoom, wt, wu);
                    }
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
        if !zoom_debouncing && missing_tiles == 0 {
            self.prefetch_adjacent_zoom(rect);
        }

        if missing_tiles > 0 {
            let status = if failed_tiles > 0 && self.tile_cache.is_empty() {
                "Map unavailable · retrying automatically"
            } else if failed_tiles > 0 {
                "Some map tiles unavailable · retrying"
            } else {
                "Loading map tiles…"
            };
            let galley = painter.layout_no_wrap(
                status.into(),
                egui::FontId::proportional(11.0),
                theme.text_normal.to_egui(),
            );
            let status_rect = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(8.0, 8.0),
                galley.size() + egui::vec2(12.0, 8.0),
            );
            painter.rect_filled(status_rect, 3.0, theme.bg.with_alpha(235).to_egui());
            painter.galley(
                status_rect.left_top() + egui::vec2(6.0, 4.0),
                galley,
                theme.text_normal.to_egui(),
            );
            if failed_tiles > 0 {
                ui.interact(
                    status_rect,
                    ui.id().with("map_tile_status"),
                    egui::Sense::hover(),
                )
                .on_hover_text(
                    self.tile_last_error
                        .as_deref()
                        .unwrap_or("Map tiles are unavailable. Aircraft tracking continues."),
                );
            }
        }

        // Click handler (select aircraft) — use coordinate from tile projection
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let mut closest_icao = None;
                let mut closest_dist = f32::INFINITY;
                let cx_f32 = rect.center().x;
                let cy_f32 = rect.center().y;
                for ac in &self.aircraft {
                    if !self.has_position(ac) || !self.matches_filters(ac) {
                        continue;
                    }
                    let tx_f = Self::lon_to_tile_x(ac.lon, self.tile_zoom);
                    let ty_f = Self::lat_to_tile_y(ac.lat, self.tile_zoom);
                    let x = cx_f32 + self.tile_x_delta(tx_f) as f32 * 256.0;
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
        let obs_x = (f64::from(rect.center().x) + self.tile_x_delta(obs_tx) * 256.0) as f32;
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
                    let rx = (f64::from(rect.center().x) + self.tile_x_delta(t2x) * 256.0) as f32;
                    let ry = (f64::from(rect.center().y) + (t2y - self.tile_cy) * 256.0) as f32;
                    ring_points.push(egui::pos2(rx, ry));
                }
                for w in ring_points.windows(2) {
                    painter.line_segment(
                        [w[0], w[1]],
                        egui::Stroke::new(0.5, theme.success.with_alpha(alpha).to_egui()),
                    );
                }
                if let Some(first) = ring_points.first() {
                    painter.text(
                        *first + egui::vec2(3.0, -3.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{}km", dist_km as u32),
                        egui::FontId::proportional(9.0),
                        theme.success.with_alpha(alpha.saturating_add(20)).to_egui(),
                    );
                }
            }
            painter.circle_stroke(
                egui::pos2(obs_x, obs_y),
                6.0,
                egui::Stroke::new(1.5, theme.warning.to_egui()),
            );
            painter.circle_filled(egui::pos2(obs_x, obs_y), 2.5, theme.warning.to_egui());
        }

        // Trails
        if self.show_trails {
            let visible: std::collections::HashSet<_> = self
                .aircraft
                .iter()
                .filter(|ac| self.has_position(ac) && self.matches_filters(ac))
                .map(|ac| ac.icao)
                .collect();
            for (icao, trail) in &self.aircraft_trails {
                if !visible.contains(icao) {
                    continue;
                }
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
                            (f64::from(rect.center().x) + self.tile_x_delta(tix) * 256.0) as f32,
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
        let plane_texture = self.plane_texture(ui.ctx());
        for ac in &self.aircraft {
            if !self.matches_filters(ac) {
                continue;
            }
            if !self.has_position(ac) {
                continue;
            }
            let tix = Self::lon_to_tile_x(ac.lon, self.tile_zoom);
            let tiy = Self::lat_to_tile_y(ac.lat, self.tile_zoom);
            let x = (f64::from(rect.center().x) + self.tile_x_delta(tix) * 256.0) as f32;
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
            // Use a real embedded SVG marker for the common fixed-wing case.
            // The painter fallback remains for helicopters and unusual models.
            if !matches!(category, AcCategory::Helicopter) && plane_texture.is_some() {
                if let Some(texture) = &plane_texture {
                    let size = egui::vec2(model_scale * 2.6, model_scale * 2.6);
                    let marker_rect = egui::Rect::from_center_size(egui::pos2(x, y), size);
                    let mut mesh = egui::epaint::Mesh::with_texture(texture.id());
                    mesh.add_rect_with_uv(
                        marker_rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        color,
                    );
                    mesh.rotate(
                        egui::emath::Rot2::from_angle((ac.heading as f32).to_radians()),
                        marker_rect.center(),
                    );
                    // Clip the marker to the map, including near toolbar edges.
                    painter.add(mesh);
                }
            } else {
                draw_plane_model(
                    &painter,
                    egui::pos2(x, y),
                    ac.heading as f32,
                    &category,
                    color,
                    model_scale,
                );
            }
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
            theme.text_dim.to_egui(),
        );
        painter.text(
            egui::pos2(legend_x + 5.0, legend_top + legend_h),
            egui::Align2::CENTER_BOTTOM,
            "0 ft",
            egui::FontId::proportional(7.0),
            theme.text_dim.to_egui(),
        );
        painter.text(
            egui::pos2(rect.left() + 6.0, rect.bottom() - 5.0),
            egui::Align2::LEFT_BOTTOM,
            "© OpenStreetMap contributors",
            egui::FontId::proportional(9.0),
            theme.text_dim.to_egui(),
        );
    }

    fn ui_uat_source(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("978 MHz source").strong());
        ui.label("dump978-fa · JSON TCP feed");
        ui.horizontal(|ui| {
            ui.label("Address");
            ui.add_enabled(
                !self.uat_receiver.is_active(),
                egui::TextEdit::singleline(&mut self.uat_address)
                    .desired_width(ui.available_width())
                    .hint_text(crate::uat_receiver::DEFAULT_ADDRESS),
            );
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.uat_receiver.is_active(), egui::Button::new("Connect"))
                .clicked()
            {
                self.clear_tracks();
                self.connect_uat();
            }
            if ui
                .add_enabled(
                    self.uat_receiver.is_active(),
                    egui::Button::new("Disconnect"),
                )
                .clicked()
            {
                self.stop_uat();
            }
        });
        ui.label(self.receiver_status());
        if let UatStatus::Error(message) | UatStatus::Reconnecting(message) =
            self.uat_receiver.status()
        {
            ui.label(
                egui::RichText::new(message)
                    .small()
                    .color(egui::Color32::from_rgb(222, 172, 97)),
            );
        }
        let (received, rejected, dropped) = self.uat_receiver.counts();
        ui.label(
            egui::RichText::new(format!(
                "{received} reports · {rejected} invalid · {dropped} dropped"
            ))
            .small(),
        );
        ui.collapsing("Receiver setup", |ui| {
            ui.label("Start FlightAware dump978-fa on the receiver computer, then connect here. UAT broadcasts are primarily available in the US.");
            let mut command = crate::uat_receiver::SETUP_COMMAND.to_owned();
            ui.add(egui::TextEdit::multiline(&mut command).font(egui::TextStyle::Monospace).desired_width(ui.available_width()).desired_rows(3).interactive(false));
            if ui.small_button("Copy command").clicked() {
                ui.ctx().copy_text(crate::uat_receiver::SETUP_COMMAND.into());
            }
            ui.label("Install dump978-fa and your SoapySDR device driver first. Selecting 978 releases ez-sdr's receiver so dump978-fa can own the SDR. Stop dump978-fa before returning to 1090 on the same dongle. For a remote feed, enter its IP:port.");
            ui.hyperlink_to("dump978-fa documentation", "https://github.com/flightaware/dump978");
        });
        ui.separator();
    }

    /// Renders the aircraft list and controls — for the ADS-B tab right sidebar.
    pub fn ui_list(&mut self, ui: &mut egui::Ui) {
        if self.region == AdsbRegion::Uat978 {
            self.ui_uat_source(ui);
        }
        if self.region == AdsbRegion::ModeS1090 {
            ui.collapsing("Antenna setup", |ui| {
                self.checklist.ui(ui);
            });
        }
        if let Some(msg) = self.checklist.pending_status.take() {
            self.pending_status_flash = Some(msg);
        }

        let theme = self
            .shared
            .try_lock()
            .map(|s| s.config.theme_config.clone())
            .unwrap_or_default();

        while let Ok((icao, info)) = self.info_rx.try_recv() {
            if self.aircraft.iter().any(|ac| ac.icao == icao) {
                self.aircraft_info.insert(icao, info);
            }
        }

        if let Ok(state) = self.shared.try_lock() {
            if (state.config.observer_lat - self.observer_lat).abs() > 0.001
                || (state.config.observer_lon - self.observer_lon).abs() > 0.001
            {
                self.observer_lat = state.config.observer_lat;
                self.observer_lon = state.config.observer_lon;
            }
        }

        // Update trails here so they're ready when ui_map() renders. Kept
        // outside the collapsible cards below so trail history keeps building
        // even while a card is collapsed (module_card skips contents when closed).
        let live: std::collections::HashSet<_> = self.aircraft.iter().map(|ac| ac.icao).collect();
        let positioned: std::collections::HashSet<_> = self
            .aircraft
            .iter()
            .filter(|ac| self.has_position(ac) && ac.seen.elapsed().as_secs() <= self.max_age_secs)
            .map(|ac| ac.icao)
            .collect();
        self.aircraft_trails
            .retain(|key, _| positioned.contains(key));
        self.aircraft_info.retain(|key, _| live.contains(key));
        if self.selected_icao.is_some_and(|key| !live.contains(&key)) {
            self.selected_icao = None;
        }
        for ac in &self.aircraft {
            if !self.has_position(ac) {
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

        // Stats summary — always visible above the cards, like SDR++'s status line.
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
                    && self.has_position(ac)
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

        module_card(
            ui,
            &theme,
            "planes.controls",
            "🎛",
            "Controls & Filters",
            true,
            |ui| {
                ui.horizontal(|ui| {
                    if self.start_time.is_some() {
                        if ui.button("■ Stop").clicked() {
                            self.stop_receiving();
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
                            .hint_text("search callsign/address"),
                    );
                    if !self.callsign_filter.is_empty() && ui.small_button("✕").clicked() {
                        self.callsign_filter.clear();
                    }
                });
            },
        );

        // Selected aircraft detail
        if let Some(icao) = self.selected_icao {
            if let Some(info) = self.aircraft_info.get(&icao) {
                ui.separator();
                egui::Frame::new()
                    .fill(theme.surface.to_egui())
                    .stroke(egui::Stroke::new(
                        1.0,
                        theme.accent.with_alpha(120).to_egui(),
                    ))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("{icao:06X}"))
                                .strong()
                                .color(theme.accent.to_egui()),
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
                                    .color(theme.text_dim.to_egui()),
                                );
                            }
                        }
                    });
            }
        }

        ui.separator();
        module_card(ui, &theme, "planes.list", "✈", "Aircraft", true, |ui| {
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
                    if !self.matches_filters(ac) { continue; }
                    let is_selected = self.selected_icao == Some(ac.icao);
                    let age_frac = (age as f32 / max_age as f32).clamp(0.0, 1.0);
                    // Age fade: blend text_normal → text_dim as the contact goes stale.
                    let row_col = if is_selected {
                        theme.accent.to_egui()
                    } else {
                        let n = theme.text_normal;
                        let d = theme.text_dim;
                        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * age_frac * 0.85) as u8;
                        egui::Color32::from_rgb(lerp(n.0, d.0), lerp(n.1, d.1), lerp(n.2, d.2))
                    };
                    let label = if ac.callsign.is_empty() { address_label(ac.icao) } else { ac.callsign.clone() };
                    if ui.label(egui::RichText::new(&label).color(row_col).small()).clicked() {
                        self.selected_icao = Some(ac.icao);
                        fetch_icao = Some(ac.icao);
                    }
                    ui.label(egui::RichText::new(format!("{}ft", ac.altitude)).color(row_col).small());
                    ui.label(egui::RichText::new(format!("{}kt", ac.speed)).color(row_col).small());
                    let has_position = self.has_position(ac);
                    let dist = self.haversine_distance(self.observer_lat, self.observer_lon, ac.lat, ac.lon);
                    let distance_label = if has_position { format!("{dist:.0}km") } else { "—".into() };
                    ui.label(egui::RichText::new(distance_label).color(row_col).small());
                    let age_color = if age < 10 { theme.success.to_egui() } else if age < 30 { theme.warning.to_egui() } else { theme.text_dim.to_egui() };
                    ui.label(egui::RichText::new(format!("{age}s")).color(age_color).small());
                    if ui.add_enabled(has_position, egui::Button::new("🤖").small()).clicked() {
                        let bearing = self.bearing(self.observer_lat, self.observer_lon, ac.lat, ac.lon);
                        self.pending_ai_prompt = Some(format!(
                            "Aircraft on ADS-B:\nCallsign: {}\nAddress: {}\nAlt: {} ft, Speed: {} kt, Heading: {}°\nPos: {:.4}°N, {:.4}°E\nDist: {:.1} km, Bearing: {:.0}°",
                            label, address_label(ac.icao), ac.altitude, ac.speed, ac.heading, ac.lat, ac.lon, dist, bearing
                        ));
                    }
                    ui.end_row();
                }
                if let Some(icao) = fetch_icao {
                    self.fetch_aircraft_info(icao);
                }
            });
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
        assert!((panel.observer_lat - 51.5).abs() < f64::EPSILON);
        assert!((panel.observer_lon + 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn adsb_reset_retires_late_enrichment_and_old_selection() {
        let mut panel = AdsBPanel::new(make_shared_state());
        let old_sender = panel.info_tx.clone();
        panel.aircraft.push(AircraftEntry {
            icao: 1,
            ..Default::default()
        });
        panel.selected_icao = Some(1);
        panel.pending_ai_prompt = Some("old aircraft".into());
        panel.clear_tracks();
        assert!(old_sender.send((1, AircraftInfo::default())).is_err());
        assert!(panel.aircraft.is_empty());
        assert!(panel.selected_icao.is_none());
        assert!(panel.pending_ai_prompt.is_none());
        assert!(panel.info_rx.try_recv().is_err());
    }

    #[test]
    fn adsb_map_wraps_date_line_but_never_poles_and_rejects_invalid_positions() {
        let mut panel = AdsBPanel::new(make_shared_state());
        panel.tile_cx = AdsBPanel::lon_to_tile_x(179.9, panel.tile_zoom);
        let east = panel.tile_x_delta(AdsBPanel::lon_to_tile_x(-179.9, panel.tile_zoom));
        assert!(
            east > 0.0 && east < 0.2,
            "nearby dateline aircraft should be visible"
        );
        panel.tile_cy = -100.0;
        panel.clamp_map_center();
        assert_eq!(panel.tile_cy, 0.0);
        panel.tile_cy = 1000.0;
        panel.clamp_map_center();
        assert_eq!(panel.tile_cy, 256.0);
        assert!(!panel.has_position(&AircraftEntry {
            lat: 91.0,
            lon: 1.0,
            ..Default::default()
        }));
        assert!(!panel.has_position(&AircraftEntry {
            lat: 40.0,
            lon: f64::NAN,
            ..Default::default()
        }));
    }

    #[test]
    fn adsb_map_filters_match_search_and_altitude_and_busy_switch_is_atomic() {
        let shared = make_shared_state();
        let mut panel = AdsBPanel::new(shared.clone());
        let ac = AircraftEntry {
            icao: 0x123abc,
            callsign: "TEST123".into(),
            altitude: 10000,
            ..Default::default()
        };
        panel.callsign_filter = "test".into();
        assert!(panel.matches_filters(&ac));
        panel.callsign_filter = "missing".into();
        assert!(!panel.matches_filters(&ac));
        panel.callsign_filter = "123ABC".into();
        panel.altitude_filter_enabled = true;
        panel.min_altitude_ft = 12000;
        assert!(!panel.matches_filters(&ac));
        let _guard = shared.lock().unwrap();
        panel.set_region(AdsbRegion::Uat978);
        assert_eq!(panel.region, AdsbRegion::ModeS1090);
        assert!(panel
            .pending_status_flash
            .as_deref()
            .unwrap()
            .contains("busy"));
    }

    #[test]
    fn map_recenter_uses_observer_coordinates() {
        let mut panel = AdsBPanel::new(make_shared_state());
        panel.observer_lat = 40.7128;
        panel.observer_lon = -74.0060;
        panel.recenter_on_observer();
        assert!((panel.tile_cx - AdsBPanel::lon_to_tile_x(-74.0060, panel.tile_zoom)).abs() < 1e-9);
        assert!((panel.tile_cy - AdsBPanel::lat_to_tile_y(40.7128, panel.tile_zoom)).abs() < 1e-9);
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

    #[test]
    fn region_picker_frequencies_are_standard() {
        assert_eq!(AdsbRegion::ModeS1090.frequency_hz(), 1_090_000_000);
        assert_eq!(AdsbRegion::ModeS1090.sample_rate_hz(), 2_400_000);
        assert_eq!(AdsbRegion::Uat978.frequency_hz(), 978_000_000);
        assert_eq!(AdsbRegion::Uat978.sample_rate_hz(), 2_083_333);
        assert_ne!(AdsbRegion::ModeS1090, AdsbRegion::Uat978);
    }

    #[test]
    fn uat_reports_merge_into_table_and_map_without_fabricating_positions() {
        let mut panel = AdsBPanel::new(make_shared_state());
        panel.region = AdsbRegion::Uat978;
        let update = |line: &[u8]| TimedReport {
            report: crate::uat_receiver::parse_report(line).unwrap(),
            received: std::time::Instant::now(),
        };
        panel.ingest_uat_report(update(
            br#"{"address":"a1b2c3","address_qualifier":"adsb_icao","callsign":"N123AB"}"#,
        ));
        assert_eq!(panel.aircraft.len(), 1);
        assert_eq!(panel.aircraft[0].callsign, "N123AB");
        assert!(!panel.has_position(&panel.aircraft[0]));
        panel.ingest_uat_report(update(br#"{"address":"a1b2c3","address_qualifier":"adsb_icao","position":{"lat":40.25,"lon":-75.5},"pressure_altitude":12000,"ground_speed":140,"true_track":90}"#));
        assert_eq!(panel.aircraft.len(), 1);
        let aircraft = &panel.aircraft[0];
        assert!(panel.has_position(aircraft));
        assert_eq!((aircraft.lat, aircraft.lon), (40.25, -75.5));
        assert_eq!(
            (aircraft.altitude, aircraft.speed, aircraft.heading),
            (12000, 140, 90)
        );
        assert_eq!(aircraft.callsign, "N123AB");
        panel.ingest_uat_report(update(
            br#"{"address":"a1b2c3","address_qualifier":"tisb_icao","ground_speed":142}"#,
        ));
        assert_eq!(panel.aircraft.len(), 1);
        assert_eq!(panel.aircraft[0].speed, 142);
        assert!(panel.has_position(&panel.aircraft[0]));
    }

    #[test]
    fn uat_anonymous_tracks_are_distinct_and_never_enriched_as_icao() {
        let mut panel = AdsBPanel::new(make_shared_state());
        panel.region = AdsbRegion::Uat978;
        for qualifier in ["adsb_icao", "adsb_other", "tisb_trackfile"] {
            let json = format!(r#"{{"address":"a1b2c3","address_qualifier":"{qualifier}"}}"#);
            panel.ingest_uat_report(TimedReport {
                report: crate::uat_receiver::parse_report(json.as_bytes()).unwrap(),
                received: std::time::Instant::now(),
            });
        }
        assert_eq!(panel.aircraft.len(), 3);
        let anonymous = panel.aircraft[1].icao;
        panel.fetch_aircraft_info(anonymous);
        assert!(!panel.aircraft_info.contains_key(&anonymous));
    }

    #[test]
    fn uat_positions_expire_even_when_nonposition_reports_keep_arriving() {
        let mut panel = AdsBPanel::new(make_shared_state());
        panel.region = AdsbRegion::Uat978;
        let report = crate::uat_receiver::parse_report(
            br#"{"address":"a1b2c3","address_qualifier":"adsb_icao","position":{"lat":0,"lon":0}}"#,
        )
        .unwrap();
        panel.ingest_uat_report(TimedReport {
            report,
            received: std::time::Instant::now(),
        });
        assert!(
            panel.has_position(&panel.aircraft[0]),
            "real zero coordinates are a valid position"
        );
        panel.uat_position_seen.insert(
            0xa1b2c3,
            std::time::Instant::now() - std::time::Duration::from_secs(61),
        );
        panel.poll_uat();
        assert_eq!(panel.aircraft.len(), 1);
        assert!(
            !panel.has_position(&panel.aircraft[0]),
            "stale coordinates must leave the map"
        );
        panel.aircraft[0].seen = std::time::Instant::now() - std::time::Duration::from_secs(121);
        panel.poll_uat();
        assert!(panel.aircraft.is_empty());
        assert!(panel.uat_position_seen.is_empty());
    }

    #[test]
    fn uat_selection_releases_local_receiver_and_disables_modes_decoder() {
        let shared = make_shared_state();
        let mut panel = AdsBPanel::new(shared.clone());
        panel.uat_address = "invalid endpoint".into();
        {
            let mut state = shared.lock().unwrap();
            state.adsb_running = true;
            state.audio_running = true;
        }
        panel.set_region(AdsbRegion::Uat978);
        assert_eq!(panel.region, AdsbRegion::Uat978);
        let state = shared.lock().unwrap();
        assert!(!state.adsb_running);
        assert!(!state.audio_running);
        assert_eq!(
            state.source.status,
            crate::source_manager::SourceStatus::Idle
        );
        assert!(panel.receiver_status().contains("error"));
    }

    #[test]
    fn modes_begin_disables_radio_audio_and_starts_idle_daemon() {
        let shared = make_shared_state();
        {
            let mut state = shared.lock().unwrap();
            state.audio_running = true;
            state.source.source_mode = crate::source_manager::SourceMode::Daemon;
            state.source.daemon_addr = "invalid endpoint".into();
        }
        let mut panel = AdsBPanel::new(shared.clone());
        panel.begin();
        let state = shared.lock().unwrap();
        assert!(!state.audio_running);
        assert!(state.adsb_running);
        assert_eq!(state.source.frequency_hz, 1_090_000_000);
        assert!(
            matches!(
                state.source.status,
                crate::source_manager::SourceStatus::Error(_)
            ),
            "idle daemon must attempt a connection, exposing the invalid endpoint"
        );
    }

    #[test]
    fn generic_stop_cancels_uat_backend() {
        let mut panel = AdsBPanel::new(make_shared_state());
        panel.region = AdsbRegion::Uat978;
        panel.uat_receiver.start("127.0.0.1:9").unwrap();
        panel.start_time = Some(std::time::Instant::now());
        panel.stop_receiving();
        assert!(!panel.uat_receiver.is_active());
        assert_eq!(panel.uat_receiver.status(), UatStatus::Stopped);
        assert!(panel.start_time.is_none());
    }

    #[test]
    fn map_tile_input_is_bounded_and_dimensions_validated() {
        let oversized = vec![0_u8; AdsBPanel::MAX_TILE_BYTES + 1];
        assert!(AdsBPanel::read_tile_bytes(&mut std::io::Cursor::new(oversized)).is_err());
        for (width, height, valid) in [(256, 256, true), (1, 1, false), (257, 256, false)] {
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::new(width, height))
                .write_to(&mut encoded, image::ImageFormat::Png)
                .unwrap();
            let decoded = AdsBPanel::decode_tile(encoded.get_ref());
            assert_eq!(decoded.is_ok(), valid);
        }
        assert!(AdsBPanel::decode_tile(b"not a PNG").is_err());
    }

    #[test]
    fn failed_map_tiles_back_off_instead_of_restarting_each_frame() {
        let mut panel = AdsBPanel::new(make_shared_state());
        let context = egui::Context::default();
        let key = (8, 100, 100);
        panel
            .tile_download_tx
            .send((key, Err("offline".into())))
            .unwrap();
        panel.process_tile_downloads(&context);
        assert_eq!(panel.tile_failures[&key].1, 1);
        for _ in 0..60 {
            panel.request_tile(key.0, key.1, key.2);
        }
        assert!(panel.tile_pending.is_empty());
        assert_eq!(
            panel
                .tile_inflight
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(AdsBPanel::tile_retry_delay(1).as_secs(), 5);
        assert_eq!(AdsBPanel::tile_retry_delay(2).as_secs(), 10);
        assert_eq!(AdsBPanel::tile_retry_delay(u32::MAX).as_secs(), 60);
    }
}
