#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct SatPosition {
    pub azimuth: f64,
    pub elevation: f64,
    pub distance_km: f64,
    pub lat: f64,
    pub lon: f64,
    #[allow(dead_code)]
    pub timestamp: f64,
}

#[derive(Debug, Clone)]
pub struct SatelliteCatalogEntry {
    pub name: String,
    pub tle_name: String,
    pub frequency_hz: u64,
    pub mode: &'static str,
    pub description: &'static str,
    pub is_active_pass: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TrajectoryPoint {
    pub geo: GeoPoint,
    pub segment: TrajectorySegment,
    pub timestamp: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrajectorySegment {
    PreAOS,
    InPass,
    PostLOS,
}

impl TrajectorySegment {
    pub fn label(&self) -> &'static str {
        match self {
            Self::PreAOS => "Approaching",
            Self::InPass => "Active",
            Self::PostLOS => "Past",
        }
    }
}
