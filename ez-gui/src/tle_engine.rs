use chrono::{DateTime, Utc};

const SPEED_OF_LIGHT_M_S: f64 = 299_792_458.0;
const EARTH_ROTATION_RAD_S: f64 = 7.292_115_0e-5;
const WGS84_A_KM: f64 = 6_378.137;
const WGS84_E2: f64 = 6.694_379_990_14e-3;

#[derive(Debug, Clone)]
pub struct TleEntry {
    pub name: String,
    pub mean_motion: f64,
    pub inclination: f64,
    elements: sgp4::Elements,
    constants: sgp4::Constants,
}

#[derive(Debug, Clone)]
pub struct PassInfo {
    pub satellite: String,
    pub aos: String,
    pub los: String,
    pub max_elevation: f64,
    pub frequency_hz: u64,
    pub aos_dt: f64,
    pub los_dt: f64,
}

pub struct TleEngine {
    pub tles: Vec<TleEntry>,
    pub observer_lat: f64,
    pub observer_lon: f64,
    cached_passes: Vec<PassInfo>,
    cached_at: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy)]
struct EarthFixedState {
    position_km: [f64; 3],
    velocity_km_s: [f64; 3],
    latitude_deg: f64,
    longitude_deg: f64,
}

#[derive(Debug, Clone, Copy)]
struct LookAngle {
    azimuth_deg: f64,
    elevation_deg: f64,
    distance_km: f64,
    range_rate_km_s: f64,
}

impl Default for TleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TleEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            tles: vec![],
            observer_lat: 51.5,
            observer_lon: -0.1,
            cached_passes: vec![],
            cached_at: None,
        };
        engine.load_builtin();
        engine
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_observer(lat: f64, lon: f64) -> Self {
        let mut engine = Self::new();
        engine.observer_lat = lat.clamp(-90.0, 90.0);
        engine.observer_lon = normalize_longitude(lon);
        engine
    }

    fn load_builtin(&mut self) {
        // Celestrak GP data retrieved 2026-09-20. These are offline fallbacks;
        // pass accuracy degrades as TLEs age, so imported current TLEs should
        // replace them when precise pass timing is required.
        const BUILTIN_TLES: [[&str; 3]; 3] = [
            [
                "Meteor-M2-3",
                "1 57166U 23091A   26263.23336345 -.00000030  00000+0  57873-5 0  9996",
                "2 57166  98.5987 316.2201 0002874 246.8980 113.1896 14.24052725168046",
            ],
            [
                "Meteor-M2-4",
                "1 59051U 24039A   26263.14480590  .00000012  00000+0  25071-4 0  9999",
                "2 59051  98.7125 221.2953 0005928 253.7523 106.3003 14.22437870132769",
            ],
            [
                "ISS",
                "1 25544U 98067A   26263.14255447  .00007470  00000+0  14267-3 0  9991",
                "2 25544  51.6307 190.1401 0004820 160.6694 199.4478 15.49188396586472",
            ],
        ];

        self.tles = BUILTIN_TLES
            .iter()
            .map(|lines| {
                from_tle_lines(lines).expect("bundled TLE data must remain syntactically valid")
            })
            .collect();
    }

    /// Replace known catalog satellites from a standard 2LE/3LE text file.
    /// Entries are matched by NORAD catalog ID, so Celestrak display-name
    /// variations do not break the desktop catalog's stable names.
    pub fn update_tles_from_text(&mut self, text: &str) -> Result<usize, String> {
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let mut parsed_entries = Vec::new();
        let mut index = 0;

        while index < lines.len() {
            let (name, line1_index) = if lines[index].starts_with("1 ") {
                ("Imported TLE", index)
            } else {
                (lines[index].trim_start_matches("0 ").trim(), index + 1)
            };
            if line1_index + 1 >= lines.len()
                || !lines[line1_index].starts_with("1 ")
                || !lines[line1_index + 1].starts_with("2 ")
            {
                return Err(format!(
                    "Malformed TLE near input line {}: expected line 1 followed by line 2",
                    index + 1
                ));
            }
            parsed_entries.push(from_tle_lines(&[
                name,
                lines[line1_index],
                lines[line1_index + 1],
            ])?);
            index = line1_index + 2;
        }

        if parsed_entries.is_empty() {
            return Err("The selected file contains no TLE entries".into());
        }

        let mut replacements = Vec::new();
        for mut imported in parsed_entries {
            if let Some((catalog_index, current)) = self
                .tles
                .iter()
                .enumerate()
                .find(|(_, current)| current.elements.norad_id == imported.elements.norad_id)
            {
                imported.name = current.name.clone();
                imported.elements.object_name = Some(imported.name.clone());
                replacements.push((catalog_index, imported));
            }
        }
        if replacements.is_empty() {
            return Err(
                "No entries matched the built-in Meteor-M2-3, Meteor-M2-4, or ISS catalog IDs"
                    .into(),
            );
        }

        let replacement_count = replacements.len();
        for (catalog_index, imported) in replacements {
            self.tles[catalog_index] = imported;
        }
        self.cached_passes.clear();
        self.cached_at = None;
        Ok(replacement_count)
    }

    pub fn upcoming_passes(&mut self) -> &[PassInfo] {
        let stale = match self.cached_at {
            Some(t) => t.elapsed() > std::time::Duration::from_secs(60),
            None => true,
        };
        if stale {
            self.cached_passes = self.compute_passes(self.observer_lat, self.observer_lon, 72.0);
            self.cached_at = Some(std::time::Instant::now());
        }
        &self.cached_passes
    }

    pub fn compute_passes(&self, lat: f64, lon: f64, hours: f64) -> Vec<PassInfo> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.compute_passes_from(lat, lon, now, hours)
    }

    fn compute_passes_from(&self, lat: f64, lon: f64, start: f64, hours: f64) -> Vec<PassInfo> {
        if !start.is_finite() || !hours.is_finite() || hours <= 0.0 {
            return Vec::new();
        }

        let lat = lat.clamp(-90.0, 90.0);
        let lon = normalize_longitude(lon);
        let end = start + hours * 3_600.0;
        let step_seconds = 60.0;
        let mut passes = Vec::new();

        for sat in &self.tles {
            let mut previous_time = start;
            let mut previous_elevation = self.elevation_at(sat, lat, lon, start);
            let mut visible = previous_elevation.is_some_and(|elevation| elevation > 0.0);
            let mut aos_time = if visible { start } else { 0.0 };
            let mut max_elevation = previous_elevation.unwrap_or(f64::NEG_INFINITY);
            let mut time = (start + step_seconds).min(end);

            while time <= end {
                let elevation = self.elevation_at(sat, lat, lon, time);
                if let (Some(previous), Some(current)) = (previous_elevation, elevation) {
                    if !visible && previous <= 0.0 && current > 0.0 {
                        aos_time = self.refine_horizon_crossing(sat, lat, lon, previous_time, time);
                        visible = true;
                        max_elevation = current;
                    } else if visible && previous > 0.0 && current <= 0.0 {
                        let los_time =
                            self.refine_horizon_crossing(sat, lat, lon, previous_time, time);
                        if max_elevation > 5.0 {
                            passes.push(make_pass(sat, aos_time, los_time, max_elevation));
                        }
                        visible = false;
                        max_elevation = f64::NEG_INFINITY;
                    } else if visible {
                        max_elevation = max_elevation.max(current);
                    }
                }

                previous_time = time;
                previous_elevation = elevation;
                if time >= end {
                    break;
                }
                time = (time + step_seconds).min(end);
            }

            if visible && max_elevation > 5.0 {
                passes.push(make_pass(sat, aos_time, end, max_elevation));
            }
        }

        passes.sort_by(|a, b| a.aos_dt.total_cmp(&b.aos_dt));
        passes
    }

    fn elevation_at(&self, sat: &TleEntry, lat: f64, lon: f64, t: f64) -> Option<f64> {
        let state = propagate_earth_fixed(sat, t)?;
        Some(look_angle(state, lat, lon).elevation_deg)
    }

    fn refine_horizon_crossing(
        &self,
        sat: &TleEntry,
        lat: f64,
        lon: f64,
        mut low: f64,
        mut high: f64,
    ) -> f64 {
        let low_is_visible = self
            .elevation_at(sat, lat, lon, low)
            .is_some_and(|elevation| elevation > 0.0);
        for _ in 0..12 {
            let middle = (low + high) * 0.5;
            let middle_is_visible = self
                .elevation_at(sat, lat, lon, middle)
                .is_some_and(|elevation| elevation > 0.0);
            if middle_is_visible == low_is_visible {
                low = middle;
            } else {
                high = middle;
            }
        }
        (low + high) * 0.5
    }

    pub fn doppler_shift(&self, sat: &TleEntry, freq_hz: f64, t: f64) -> f64 {
        if !freq_hz.is_finite() || freq_hz == 0.0 {
            return 0.0;
        }
        let Some(state) = propagate_earth_fixed(sat, t) else {
            return 0.0;
        };
        let look = look_angle(state, self.observer_lat, self.observer_lon);
        -(look.range_rate_km_s * 1_000.0 / SPEED_OF_LIGHT_M_S) * freq_hz
    }

    pub fn doppler_shift_for_sat(&self, name: &str, freq_hz: f64, t: f64) -> f64 {
        self.tles
            .iter()
            .find(|sat| sat.name == name)
            .map_or(0.0, |sat| self.doppler_shift(sat, freq_hz, t))
    }

    /// Compute satellite azimuth, elevation, slant range, and sub-satellite point.
    pub fn satellite_position(
        &self,
        name: &str,
        observer_lat: f64,
        observer_lon: f64,
        t: f64,
    ) -> Option<crate::satellite::types::SatPosition> {
        let sat = self.tles.iter().find(|sat| sat.name == name)?;
        let state = propagate_earth_fixed(sat, t)?;
        let look = look_angle(state, observer_lat, observer_lon);
        Some(crate::satellite::types::SatPosition {
            azimuth: look.azimuth_deg,
            elevation: look.elevation_deg,
            distance_km: look.distance_km,
            lat: state.latitude_deg,
            lon: state.longitude_deg,
            timestamp: t,
        })
    }

    /// Compute a ground-track polyline around the next pass in the requested window.
    pub fn pass_trajectory(
        &self,
        name: &str,
        observer_lat: f64,
        observer_lon: f64,
        hours: f64,
    ) -> Vec<crate::satellite::types::TrajectoryPoint> {
        let mut points = Vec::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let Some(pass) = self
            .compute_passes_from(observer_lat, observer_lon, now, hours)
            .into_iter()
            .find(|pass| pass.satellite == name)
        else {
            return points;
        };

        let start = (pass.aos_dt - 600.0).max(now);
        let end = pass.los_dt + 600.0;
        let mut t = start;
        while t <= end {
            if let Some(pos) = self.satellite_position(name, observer_lat, observer_lon, t) {
                let segment = if t < pass.aos_dt {
                    crate::satellite::types::TrajectorySegment::PreAOS
                } else if t > pass.los_dt {
                    crate::satellite::types::TrajectorySegment::PostLOS
                } else {
                    crate::satellite::types::TrajectorySegment::InPass
                };
                points.push(crate::satellite::types::TrajectoryPoint {
                    geo: crate::satellite::types::GeoPoint {
                        lat: pos.lat,
                        lon: pos.lon,
                    },
                    segment,
                    timestamp: t,
                });
            }
            t += 30.0;
        }
        points
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn from_tle_lines(lines: &[&str]) -> Result<TleEntry, String> {
    if lines.len() < 3 {
        return Err("Need at least 3 lines: name, line1, line2".into());
    }
    let name = lines[0].trim().to_string();
    if name.is_empty() {
        return Err("TLE name cannot be empty".into());
    }
    let line1 = lines[1].trim();
    let line2 = lines[2].trim();
    let elements = sgp4::Elements::from_tle(Some(name.clone()), line1.as_bytes(), line2.as_bytes())
        .map_err(|error| format!("Cannot parse TLE for {name}: {error}"))?;
    let constants = sgp4::Constants::from_elements(&elements)
        .map_err(|error| format!("Cannot initialize SGP4 for {name}: {error}"))?;

    Ok(TleEntry {
        name,
        mean_motion: elements.mean_motion,
        inclination: elements.inclination,
        elements,
        constants,
    })
}

fn propagate_earth_fixed(sat: &TleEntry, unix_seconds: f64) -> Option<EarthFixedState> {
    let datetime = unix_to_datetime(unix_seconds)?;
    let minutes = sat
        .elements
        .datetime_to_minutes_since_epoch(&datetime.naive_utc())
        .ok()?;
    let prediction = sat.constants.propagate(minutes).ok()?;
    let theta = greenwich_sidereal_angle(unix_seconds);
    let (sin_theta, cos_theta) = theta.sin_cos();
    let [x, y, z] = prediction.position;
    let [vx, vy, vz] = prediction.velocity;

    // TEME to a rotating Earth-fixed frame. Polar motion and the small TEME
    // equation-of-equinox correction are intentionally omitted at this UI
    // precision; Earth rotation is included in velocity for range-rate/Doppler.
    let position_km = [
        cos_theta * x + sin_theta * y,
        -sin_theta * x + cos_theta * y,
        z,
    ];
    let rotated_velocity = [
        cos_theta * vx + sin_theta * vy,
        -sin_theta * vx + cos_theta * vy,
        vz,
    ];
    let velocity_km_s = [
        rotated_velocity[0] + EARTH_ROTATION_RAD_S * position_km[1],
        rotated_velocity[1] - EARTH_ROTATION_RAD_S * position_km[0],
        rotated_velocity[2],
    ];
    let (latitude_deg, longitude_deg) = ecef_to_geodetic(position_km);

    Some(EarthFixedState {
        position_km,
        velocity_km_s,
        latitude_deg,
        longitude_deg,
    })
}

fn look_angle(state: EarthFixedState, observer_lat: f64, observer_lon: f64) -> LookAngle {
    let lat = observer_lat.clamp(-90.0, 90.0).to_radians();
    let lon = normalize_longitude(observer_lon).to_radians();
    let observer = geodetic_to_ecef(lat, lon);
    let relative = [
        state.position_km[0] - observer[0],
        state.position_km[1] - observer[1],
        state.position_km[2] - observer[2],
    ];
    let distance_km = vector_norm(relative).max(f64::MIN_POSITIVE);
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let east = -sin_lon * relative[0] + cos_lon * relative[1];
    let north =
        -sin_lat * cos_lon * relative[0] - sin_lat * sin_lon * relative[1] + cos_lat * relative[2];
    let up =
        cos_lat * cos_lon * relative[0] + cos_lat * sin_lon * relative[1] + sin_lat * relative[2];
    let azimuth_deg = east.atan2(north).to_degrees().rem_euclid(360.0);
    let elevation_deg = (up / distance_km).clamp(-1.0, 1.0).asin().to_degrees();
    let range_rate_km_s = dot(relative, state.velocity_km_s) / distance_km;

    LookAngle {
        azimuth_deg,
        elevation_deg,
        distance_km,
        range_rate_km_s,
    }
}

fn geodetic_to_ecef(lat_rad: f64, lon_rad: f64) -> [f64; 3] {
    let (sin_lat, cos_lat) = lat_rad.sin_cos();
    let (sin_lon, cos_lon) = lon_rad.sin_cos();
    let prime_vertical = WGS84_A_KM / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    [
        prime_vertical * cos_lat * cos_lon,
        prime_vertical * cos_lat * sin_lon,
        prime_vertical * (1.0 - WGS84_E2) * sin_lat,
    ]
}

fn ecef_to_geodetic(position_km: [f64; 3]) -> (f64, f64) {
    let [x, y, z] = position_km;
    let longitude = y.atan2(x);
    let horizontal = x.hypot(y);
    let semi_minor = WGS84_A_KM * (1.0 - WGS84_E2).sqrt();
    let second_eccentricity =
        (WGS84_A_KM * WGS84_A_KM - semi_minor * semi_minor) / (semi_minor * semi_minor);
    let auxiliary = (z * WGS84_A_KM).atan2(horizontal * semi_minor);
    let (sin_auxiliary, cos_auxiliary) = auxiliary.sin_cos();
    let latitude = (z + second_eccentricity * semi_minor * sin_auxiliary.powi(3))
        .atan2(horizontal - WGS84_E2 * WGS84_A_KM * cos_auxiliary.powi(3));
    (
        latitude.to_degrees().clamp(-90.0, 90.0),
        normalize_longitude(longitude.to_degrees()),
    )
}

fn greenwich_sidereal_angle(unix_seconds: f64) -> f64 {
    let julian_date = unix_seconds / 86_400.0 + 2_440_587.5;
    let days_since_j2000 = julian_date - 2_451_545.0;
    let centuries = days_since_j2000 / 36_525.0;
    let degrees = 280.460_618_37
        + 360.985_647_366_29 * days_since_j2000
        + 0.000_387_933 * centuries * centuries
        - centuries * centuries * centuries / 38_710_000.0;
    degrees.rem_euclid(360.0).to_radians()
}

fn unix_to_datetime(unix_seconds: f64) -> Option<DateTime<Utc>> {
    if !unix_seconds.is_finite() {
        return None;
    }
    let mut seconds = unix_seconds.floor();
    let mut nanoseconds = ((unix_seconds - seconds) * 1_000_000_000.0).round();
    if nanoseconds >= 1_000_000_000.0 {
        seconds += 1.0;
        nanoseconds = 0.0;
    }
    if seconds < i64::MIN as f64 || seconds > i64::MAX as f64 {
        return None;
    }
    DateTime::from_timestamp(seconds as i64, nanoseconds as u32)
}

fn make_pass(sat: &TleEntry, aos_time: f64, los_time: f64, max_elevation: f64) -> PassInfo {
    PassInfo {
        satellite: sat.name.clone(),
        aos: format_time(aos_time),
        los: format_time(los_time),
        max_elevation: max_elevation.clamp(0.0, 90.0),
        frequency_hz: sat_frequency(&sat.name),
        aos_dt: aos_time,
        los_dt: los_time,
    }
}

fn vector_norm(vector: [f64; 3]) -> f64 {
    dot(vector, vector).sqrt()
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn normalize_longitude(longitude: f64) -> f64 {
    (longitude + 180.0).rem_euclid(360.0) - 180.0
}

fn format_time(t: f64) -> String {
    unix_to_datetime(t)
        .map(|datetime| datetime.format("%H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "N/A".into())
}

fn sat_frequency(name: &str) -> u64 {
    match name {
        "Meteor-M2-3" => 137_900_000,
        "Meteor-M2-4" => 137_100_000,
        "ISS" => 145_800_000,
        _ => 100_000_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE_ISS: [&str; 3] = [
        "ISS (ZARYA)",
        "1 25544U 98067A   20194.88612269 -.00002218  00000-0 -31515-4 0  9992",
        "2 25544  51.6461 221.2784 0001413  89.1723 280.4612 15.49507896236008",
    ];

    #[test]
    fn bundled_tles_are_real_sgp4_elements() {
        let engine = TleEngine::new();
        assert_eq!(engine.tles.len(), 3);
        for sat in &engine.tles {
            assert!(sat.elements.norad_id > 0);
            assert!(sat.mean_motion > 14.0);
            assert!((0.0..=180.0).contains(&sat.inclination));
            assert!(sat
                .constants
                .propagate(sgp4::MinutesSinceEpoch(0.0))
                .is_ok());
        }
    }

    #[test]
    fn parses_complete_tle_and_rejects_malformed_input() {
        let entry = from_tle_lines(&REFERENCE_ISS).expect("reference TLE should parse");
        assert_eq!(entry.name, "ISS (ZARYA)");
        assert_eq!(entry.elements.norad_id, 25_544);
        assert!((entry.inclination - 51.6461).abs() < 1.0e-6);
        assert!((entry.mean_motion - 15.495_078_96).abs() < 1.0e-8);
        assert!(from_tle_lines(&["Name", "1 ..."]).is_err());
        assert!(from_tle_lines(&["Sat", "1 ...", "2 ..."]).is_err());
    }

    #[test]
    fn imported_tles_replace_known_catalog_ids_atomically() {
        let mut engine = TleEngine::new();
        let previous_mean_motion = engine.tles[2].mean_motion;
        let imported = REFERENCE_ISS.join("\n");
        assert_eq!(engine.update_tles_from_text(&imported).unwrap(), 1);
        let iss = engine.tles.iter().find(|sat| sat.name == "ISS").unwrap();
        assert_eq!(iss.elements.norad_id, 25_544);
        assert_ne!(iss.mean_motion, previous_mean_motion);

        let snapshot = iss.mean_motion;
        assert!(engine
            .update_tles_from_text("ISS\n1 malformed\n2 malformed")
            .is_err());
        assert_eq!(
            engine
                .tles
                .iter()
                .find(|sat| sat.name == "ISS")
                .unwrap()
                .mean_motion,
            snapshot
        );
    }

    #[test]
    fn sgp4_epoch_state_matches_reference_vector() {
        let sat = from_tle_lines(&REFERENCE_ISS).unwrap();
        let prediction = sat
            .constants
            .propagate(sgp4::MinutesSinceEpoch(0.0))
            .unwrap();
        let radius = vector_norm(prediction.position);
        let speed = vector_norm(prediction.velocity);
        assert!((6_750.0..6_850.0).contains(&radius), "radius={radius}");
        assert!((7.5..7.8).contains(&speed), "speed={speed}");
    }

    #[test]
    fn earth_fixed_position_is_physical_at_tle_epoch() {
        let sat = from_tle_lines(&REFERENCE_ISS).unwrap();
        let epoch = sat.elements.datetime.and_utc().timestamp() as f64;
        let state = propagate_earth_fixed(&sat, epoch).unwrap();
        assert!((-90.0..=90.0).contains(&state.latitude_deg));
        assert!((-180.0..180.0).contains(&state.longitude_deg));
        assert!((6_750.0..6_850.0).contains(&vector_norm(state.position_km)));
        assert!((7.0..7.8).contains(&vector_norm(state.velocity_km_s)));
    }

    #[test]
    fn observer_geometry_handles_antimeridian_and_below_horizon() {
        let engine = TleEngine::new();
        let epoch = engine.tles[2].elements.datetime.and_utc().timestamp() as f64;
        let position = engine
            .satellite_position("ISS", 0.0, 179.5, epoch)
            .expect("position should propagate");
        assert!((0.0..360.0).contains(&position.azimuth));
        assert!((-90.0..=90.0).contains(&position.elevation));
        assert!((100.0..20_000.0).contains(&position.distance_km));
        assert!((-180.0..180.0).contains(&position.lon));
    }

    #[test]
    fn deterministic_passes_are_sorted_and_physical() {
        let engine = TleEngine::new();
        let start = engine.tles[2].elements.datetime.and_utc().timestamp() as f64;
        let passes = engine.compute_passes_from(51.5, -0.1, start, 24.0);
        assert!(!passes.is_empty());
        for pair in passes.windows(2) {
            assert!(pair[0].aos_dt <= pair[1].aos_dt);
        }
        for pass in passes {
            assert!(pass.los_dt > pass.aos_dt);
            assert!((5.0..=90.0).contains(&pass.max_elevation));
        }
    }

    #[test]
    fn doppler_uses_observer_relative_range_rate() {
        let engine = TleEngine::new();
        let start = engine.tles[2].elements.datetime.and_utc().timestamp() as f64;
        let pass = engine
            .compute_passes_from(engine.observer_lat, engine.observer_lon, start, 24.0)
            .into_iter()
            .find(|pass| pass.satellite == "ISS")
            .expect("ISS pass should exist in 24 hours");
        let near_aos = engine.doppler_shift_for_sat("ISS", 145_800_000.0, pass.aos_dt + 30.0);
        let near_los = engine.doppler_shift_for_sat("ISS", 145_800_000.0, pass.los_dt - 30.0);
        assert!(near_aos > 0.0, "approaching shift={near_aos}");
        assert!(near_los < 0.0, "receding shift={near_los}");
        assert!(near_aos.abs() < 10_000.0);
        assert!(near_los.abs() < 10_000.0);
        assert_eq!(engine.doppler_shift_for_sat("ISS", 0.0, start), 0.0);
        assert_eq!(
            engine.doppler_shift_for_sat("NONEXISTENT", 145_800_000.0, start),
            0.0
        );
    }

    #[test]
    fn invalid_windows_and_empty_catalog_return_no_passes() {
        let mut engine = TleEngine::new();
        assert!(engine.compute_passes_from(0.0, 0.0, 0.0, 0.0).is_empty());
        engine.tles.clear();
        assert!(engine.compute_passes_from(0.0, 0.0, 0.0, 24.0).is_empty());
    }

    #[test]
    fn helpers_bound_longitudes_and_time() {
        assert_eq!(normalize_longitude(540.0), -180.0);
        assert_eq!(format_time(f64::NAN), "N/A");
        assert_eq!(format_time(-1.0), "23:59:59 UTC");
        assert_eq!(sat_frequency("Meteor-M2-3"), 137_900_000);
        assert_eq!(sat_frequency("Meteor-M2-4"), 137_100_000);
        assert_eq!(sat_frequency("ISS"), 145_800_000);
    }
}
