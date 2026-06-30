#[derive(Debug, Clone)]
pub struct TleEntry {
    pub name: String,
    #[allow(dead_code)]
    pub line1: String,
    #[allow(dead_code)]
    pub line2: String,
    pub mean_motion: f64,
    pub inclination: f64,
    #[allow(dead_code)]
    pub eccentricity: f64,
}

#[derive(Debug, Clone)]
pub struct PassInfo {
    pub satellite: String,
    pub aos: String,
    pub los: String,
    pub max_elevation: f64,
    pub frequency_hz: u64,
    pub aos_dt: f64,
    #[allow(dead_code)]
    pub los_dt: f64,
}

pub struct TleEngine {
    pub tles: Vec<TleEntry>,
    pub observer_lat: f64,
    pub observer_lon: f64,
    #[allow(dead_code)]
    pub observer_alt: f64,
    cached_passes: Vec<PassInfo>,
    cached_at: std::time::Instant,
}

impl TleEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            tles: vec![],
            observer_lat: 51.5,
            observer_lon: -0.1,
            observer_alt: 10.0,
            cached_passes: vec![],
            cached_at: std::time::Instant::now() - std::time::Duration::from_secs(999),
        };
        engine.load_builtin();
        engine
    }

    fn load_builtin(&mut self) {
        self.tles = vec![
            TleEntry { name: "NOAA 15".into(), line1: "1 25338U 98030A   25178.50000000  .00000000  00000-0  00000-0 0  9999".into(), line2: "2 25338  98.7400 180.0000 0011700 120.0000 240.0000 14.26000000    10".into(), mean_motion: 14.26, inclination: 98.74, eccentricity: 0.00117 },
            TleEntry { name: "NOAA 18".into(), line1: "1 28654U 05018A   25178.50000000  .00000000  00000-0  00000-0 0  9999".into(), line2: "2 28654  99.0100 180.0000 0012000 120.0000 240.0000 14.13000000    10".into(), mean_motion: 14.13, inclination: 99.01, eccentricity: 0.0012 },
            TleEntry { name: "NOAA 19".into(), line1: "1 33591U 09005A   25178.50000000  .00000000  00000-0  00000-0 0  9999".into(), line2: "2 33591  98.9900 180.0000 0011500 120.0000 240.0000 14.13000000    10".into(), mean_motion: 14.13, inclination: 98.99, eccentricity: 0.00115 },
            TleEntry { name: "Meteor-M2-2".into(), line1: "1 44387U 19030A   25178.50000000  .00000000  00000-0  00000-0 0  9999".into(), line2: "2 44387  98.5700 180.0000 0011000 120.0000 240.0000 14.21000000    10".into(), mean_motion: 14.21, inclination: 98.57, eccentricity: 0.0011 },
            TleEntry { name: "ISS".into(), line1: "1 25544U 98067A   25178.50000000  .00020000  00000-0  28000-3 0  9999".into(), line2: "2 25544  51.6400 180.0000 0006000 120.0000 240.0000 15.50000000    10".into(), mean_motion: 15.50, inclination: 51.64, eccentricity: 0.0006 },
        ];
    }

    pub fn upcoming_passes(&mut self) -> &[PassInfo] {
        if self.cached_at.elapsed() > std::time::Duration::from_secs(60) {
            self.cached_passes = self.compute_passes(self.observer_lat, self.observer_lon, 72.0);
            self.cached_at = std::time::Instant::now();
        }
        &self.cached_passes
    }

    pub fn compute_passes(&self, lat: f64, lon: f64, hours: f64) -> Vec<PassInfo> {
        let mut passes = vec![];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        let dt = 60.0;
        let steps = (hours * 3600.0 / dt) as usize;

        for sat in &self.tles {
            let period_min = 1440.0 / sat.mean_motion;
            let period_s = period_min * 60.0;
            let mut aos_time = 0.0;
            let mut max_el = 0.0;
            let mut _los_time = 0.0;
            let mut visible = false;

            for i in 0..steps {
                let t = now + i as f64 * dt;
                let orbit_phase = (t % period_s) / period_s;
                let lat_sat = sat.inclination * (2.0 * std::f64::consts::PI * orbit_phase).sin();
                let lon_sat = (lon + 360.0 * orbit_phase) % 360.0 - 180.0;

                let dlat = lat_sat - lat;
                let dlon = lon_sat - lon;
                let dist = (dlat * dlat + dlon * dlon).sqrt();
                let elev = 90.0 - dist * 0.9;

                if elev > 0.0 && !visible {
                    aos_time = t;
                    visible = true;
                    max_el = 0.0;
                }
                if visible && elev > max_el {
                    max_el = elev;
                }
                if visible && elev <= 0.0 {
                    _los_time = t;
                    visible = false;
                    if max_el > 5.0 {
                        passes.push(PassInfo {
                            satellite: sat.name.clone(),
                            aos: format_time(aos_time),
                            los: format_time(_los_time),
                            max_elevation: max_el,
                            frequency_hz: sat_frequency(&sat.name),
                            aos_dt: aos_time,
                            los_dt: _los_time,
                        });
                    }
                }
            }
            if visible && max_el > 5.0 {
                passes.push(PassInfo {
                    satellite: sat.name.clone(),
                    aos: format_time(aos_time),
                    los: "TBD".into(),
                    max_elevation: max_el,
                    frequency_hz: sat_frequency(&sat.name),
                    aos_dt: aos_time,
                    los_dt: 0.0,
                });
            }
        }
        passes.sort_by(|a, b| a.aos_dt.partial_cmp(&b.aos_dt).unwrap_or(std::cmp::Ordering::Equal));
        passes
    }

    #[allow(dead_code)]
    pub fn doppler_shift(&self, sat: &TleEntry, freq_hz: f64, t: f64) -> f64 {
        let period_s = 1440.0 / sat.mean_motion * 60.0;
        let orbit_phase = (t % period_s) / period_s;
        let vel_lat = sat.inclination * 2.0 * std::f64::consts::PI * orbit_phase.cos();
        let vel_lon = 2.0 * std::f64::consts::PI * 7000.0 / period_s;
        let range_rate = (vel_lat * vel_lat + vel_lon * vel_lon).sqrt() * 0.5;
        let c = 299_792_458.0;
        let v = range_rate * 1000.0;
        -v / c * freq_hz
    }

    pub fn doppler_shift_for_sat(&self, name: &str, freq_hz: f64, t: f64) -> f64 {
        for sat in &self.tles {
            if sat.name == name {
                return self.doppler_shift(sat, freq_hz, t);
            }
        }
        0.0
    }
}

fn format_time(t: f64) -> String {
    let epoch = std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(t);
    let datetime: chrono::DateTime<chrono::Utc> = epoch.into();
    datetime.format("%H:%M:%S UTC").to_string()
}

fn sat_frequency(name: &str) -> u64 {
    match name {
        "NOAA 15" => 137_620_000,
        "NOAA 18" => 137_912_500,
        "NOAA 19" => 137_100_000,
        "Meteor-M2" => 137_900_000,
        "Meteor-M2-2" => 137_100_000,
        "ISS" => 145_800_000,
        _ => 100_000_000,
    }
}
