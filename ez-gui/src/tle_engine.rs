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
    tles: Vec<TleEntry>,
    pub observer_lat: f64,
    pub observer_lon: f64,
    cached_passes: Vec<PassInfo>,
    cached_at: Option<std::time::Instant>,
    cached_key: Option<PassCacheKey>,
    catalog_revision: u64,
}

/// Owned, immutable TLE snapshot for background LRPT georeferencing. Capturing
/// this value lets the projection worker run without holding SharedState's lock.
#[derive(Debug, Clone)]
pub struct LrptGeolocationModel {
    satellite: TleEntry,
}

impl LrptGeolocationModel {
    pub fn satellite_name(&self) -> &str {
        &self.satellite.name
    }

    pub fn state_at(
        &self,
        unix_seconds: f64,
    ) -> Option<lrpt_decode::geolocation::EarthFixedOrbitState> {
        let state = propagate_earth_fixed(&self.satellite, unix_seconds)?;
        Some(lrpt_decode::geolocation::EarthFixedOrbitState {
            position_km: state.position_km,
            velocity_km_s: state.velocity_km_s,
        })
    }

    pub fn geolocate_lines(
        &self,
        width: u32,
        line_times_unix_seconds: &[Option<f64>],
        geometry: lrpt_decode::geolocation::ScanGeometry,
        control_point_stride: u32,
    ) -> Result<lrpt_decode::geolocation::GeoReferencedImage, String> {
        let mut times = Vec::with_capacity(line_times_unix_seconds.len());
        let mut states = Vec::with_capacity(line_times_unix_seconds.len());
        for time in line_times_unix_seconds {
            let time = time.ok_or_else(|| "LRPT image has rows without absolute CDS time; georeferencing is unavailable for this product.".to_string())?;
            let state = self.state_at(time).ok_or_else(|| {
                format!(
                    "The loaded TLE for {} cannot propagate at image time {time:.3} UTC.",
                    self.satellite.name
                )
            })?;
            times.push(time);
            states.push(state);
        }
        lrpt_decode::geolocation::geolocate_scanlines(
            width,
            &times,
            &states,
            geometry,
            control_point_stride,
        )
        .map_err(|error| error.to_string())
    }

    /// Build sparse source-pixel GCP/GeoJSON geometry and a nearest-neighbor
    /// WGS84 projected raster from the same actual packet times and orbit
    /// model. Each source row is propagated independently from the TLE.
    pub fn georeference_image(
        &self,
        image: &image::GrayImage,
        line_times_unix_seconds: &[Option<f64>],
        geometry: lrpt_decode::geolocation::ScanGeometry,
        control_point_stride: u32,
    ) -> Result<lrpt_decode::geolocation::GeoreferenceProducts, String> {
        if line_times_unix_seconds.len() != image.height() as usize {
            return Err("LRPT image rows do not match packet CDS timestamps.".into());
        }
        let mut times = Vec::with_capacity(line_times_unix_seconds.len());
        let mut states = Vec::with_capacity(line_times_unix_seconds.len());
        for time in line_times_unix_seconds {
            let time = time.ok_or_else(|| {
                "LRPT image has rows without absolute CDS time; georeferencing is unavailable for this product.".to_string()
            })?;
            let state = self.state_at(time).ok_or_else(|| {
                format!(
                    "The loaded TLE for {} cannot propagate at image time {time:.3} UTC.",
                    self.satellite.name
                )
            })?;
            times.push(time);
            states.push(state);
        }
        let (vectors, projected) = lrpt_decode::geolocation::project_equirectangular_nearest(
            image,
            &times,
            &states,
            geometry,
            control_point_stride,
        )
        .map_err(|error| error.to_string())?;
        Ok(lrpt_decode::geolocation::GeoreferenceProducts { vectors, projected })
    }
}

/// Identity of every observer-dependent orbital product. The catalog is private;
/// validated imports are its only mutation path and advance the revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OrbitIdentity {
    latitude_bits: u64,
    longitude_bits: u64,
    catalog_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PassCacheKey {
    orbit: OrbitIdentity,
    start_minute: i64,
    horizon_bits: u64,
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
            cached_key: None,
            catalog_revision: 1,
        };
        engine.load_builtin();
        engine
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_observer(lat: f64, lon: f64) -> Self {
        let mut engine = Self::new();
        engine.set_observer(lat, lon);
        engine
    }

    pub fn catalog(&self) -> &[TleEntry] {
        &self.tles
    }

    /// Apply a finite observer position and invalidate immediately. Cache keys
    /// also detect legacy direct field assignments before returning cached data.
    pub fn set_observer(&mut self, lat: f64, lon: f64) -> bool {
        if !lat.is_finite() || !lon.is_finite() {
            return false;
        }
        let lat = lat.clamp(-90.0, 90.0);
        let lon = normalize_longitude(lon);
        if self.observer_lat == lat && self.observer_lon == lon {
            return false;
        }
        self.observer_lat = lat;
        self.observer_lon = lon;
        self.cached_at = None;
        self.cached_key = None;
        self.cached_passes.clear();
        true
    }

    pub(crate) fn orbit_identity(&self) -> OrbitIdentity {
        OrbitIdentity {
            latitude_bits: self.observer_lat.to_bits(),
            longitude_bits: self.observer_lon.to_bits(),
            catalog_revision: self.catalog_revision,
        }
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
        self.catalog_revision = self.catalog_revision.wrapping_add(1);
        self.cached_passes.clear();
        self.cached_at = None;
        self.cached_key = None;
        Ok(replacement_count)
    }

    pub fn upcoming_passes(&mut self) -> &[PassInfo] {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.upcoming_passes_at(now, 72.0, std::time::Instant::now())
    }

    pub(crate) fn upcoming_passes_at(
        &mut self,
        start: f64,
        hours: f64,
        now: std::time::Instant,
    ) -> &[PassInfo] {
        let key = PassCacheKey {
            orbit: self.orbit_identity(),
            start_minute: (start / 60.0).floor() as i64,
            horizon_bits: hours.to_bits(),
        };
        let stale = self.cached_key != Some(key)
            || self.cached_at.is_none_or(|at| {
                now.saturating_duration_since(at) >= std::time::Duration::from_secs(60)
            });
        if stale {
            self.cached_passes =
                self.compute_passes_from(self.observer_lat, self.observer_lon, start, hours);
            self.cached_at = Some(now);
            self.cached_key = Some(key);
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

    /// Return the Earth-fixed position and velocity propagated from the
    /// selected satellite's loaded TLE. These vectors are the orbit input used
    /// by LRPT scan-ray/ellipsoid intersection; no location is synthesized when
    /// a satellite name or propagation instant is missing.
    pub fn lrpt_geolocation_state(
        &self,
        name: &str,
        unix_seconds: f64,
    ) -> Option<lrpt_decode::geolocation::EarthFixedOrbitState> {
        let sat = self.tles.iter().find(|sat| sat.name == name)?;
        let state = propagate_earth_fixed(sat, unix_seconds)?;
        Some(lrpt_decode::geolocation::EarthFixedOrbitState {
            position_km: state.position_km,
            velocity_km_s: state.velocity_km_s,
        })
    }

    pub fn lrpt_geolocation_model(&self, name: &str) -> Option<LrptGeolocationModel> {
        Some(LrptGeolocationModel {
            satellite: self.tles.iter().find(|sat| sat.name == name)?.clone(),
        })
    }

    /// Georeference an LRPT image using actual packet-derived line times and
    /// the selected loaded TLE. The caller must supply instrument scan edges;
    /// incomplete CDS timestamps or stale/unavailable TLE propagation are
    /// rejected instead of filled with a receive-time approximation.
    pub fn geolocate_lrpt_lines(
        &self,
        name: &str,
        width: u32,
        line_times_unix_seconds: &[Option<f64>],
        geometry: lrpt_decode::geolocation::ScanGeometry,
        control_point_stride: u32,
    ) -> Result<lrpt_decode::geolocation::GeoReferencedImage, String> {
        self.lrpt_geolocation_model(name)
            .ok_or_else(|| format!("No loaded TLE matches {name}."))?
            .geolocate_lines(
                width,
                line_times_unix_seconds,
                geometry,
                control_point_stride,
            )
    }

    /// Compute a ground-track polyline around the next pass in the requested window.
    pub fn pass_trajectory(
        &self,
        name: &str,
        observer_lat: f64,
        observer_lon: f64,
        hours: f64,
    ) -> Vec<crate::satellite::types::TrajectoryPoint> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.pass_trajectory_from(name, observer_lat, observer_lon, now, hours)
    }

    pub(crate) fn pass_trajectory_from(
        &self,
        name: &str,
        observer_lat: f64,
        observer_lon: f64,
        now: f64,
        hours: f64,
    ) -> Vec<crate::satellite::types::TrajectoryPoint> {
        let mut points = Vec::new();
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
    if (-180.0..180.0).contains(&longitude) {
        longitude
    } else {
        (longitude + 180.0).rem_euclid(360.0) - 180.0
    }
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

    fn pass_signature(passes: &[PassInfo]) -> Vec<(String, f64, f64)> {
        passes
            .iter()
            .map(|pass| (pass.satellite.clone(), pass.aos_dt, pass.los_dt))
            .collect()
    }

    #[test]
    fn fresh_pass_cache_tracks_observer_catalog_horizon_and_time_window() {
        let mut engine = TleEngine::new();
        let time = engine.tles[2].elements.datetime.and_utc().timestamp() as f64;
        let now = std::time::Instant::now();
        let original = pass_signature(engine.upcoming_passes_at(time, 24.0, now));
        assert!(!original.is_empty());
        assert!(engine.set_observer(-33.8688, 151.2093));
        let relocated = pass_signature(engine.upcoming_passes_at(time, 24.0, now));
        assert_eq!(
            relocated,
            pass_signature(&engine.compute_passes_from(-33.8688, 151.2093, time, 24.0))
        );
        assert_ne!(original, relocated);
        // Legacy assignments cannot read a cache computed for another location.
        engine.observer_lat = 40.7128;
        engine.observer_lon = -74.0060;
        let direct = pass_signature(engine.upcoming_passes_at(time, 24.0, now));
        assert_eq!(
            direct,
            pass_signature(&engine.compute_passes_from(40.7128, -74.0060, time, 24.0))
        );
        assert_ne!(direct, relocated);
        let short = pass_signature(engine.upcoming_passes_at(time, 6.0, now));
        assert_eq!(
            short,
            pass_signature(&engine.compute_passes_from(40.7128, -74.0060, time, 6.0))
        );
        assert_ne!(short, direct);
        let later = pass_signature(engine.upcoming_passes_at(time + 3600.0, 6.0, now));
        assert_eq!(
            later,
            pass_signature(&engine.compute_passes_from(40.7128, -74.0060, time + 3600.0, 6.0))
        );
        assert_ne!(later, short);
        let old_identity = engine.orbit_identity();
        engine
            .update_tles_from_text(&REFERENCE_ISS.join("\n"))
            .unwrap();
        assert_ne!(engine.orbit_identity(), old_identity);
        let imported = pass_signature(engine.upcoming_passes_at(time + 3600.0, 6.0, now));
        assert_eq!(
            imported,
            pass_signature(&engine.compute_passes_from(40.7128, -74.0060, time + 3600.0, 6.0))
        );
        assert_ne!(imported, later);
    }

    #[test]
    fn unchanged_observer_reuses_cache_and_invalid_coordinates_preserve_it() {
        let mut engine = TleEngine::new();
        let time = engine.tles[2].elements.datetime.and_utc().timestamp() as f64;
        let now = std::time::Instant::now();
        engine.upcoming_passes_at(time, 72.0, now);
        let key = engine.cached_key;
        let at = engine.cached_at;
        assert!(!engine.set_observer(51.5, -0.1));
        assert!(!engine.set_observer(f64::NAN, 1.0));
        assert!(!engine.set_observer(1.0, f64::INFINITY));
        engine.upcoming_passes_at(time, 72.0, now + std::time::Duration::from_secs(1));
        assert_eq!(engine.cached_key, key);
        assert_eq!(engine.cached_at, at);
        assert_eq!((engine.observer_lat, engine.observer_lon), (51.5, -0.1));
    }

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
    fn lrpt_geolocation_uses_real_tle_vectors_and_packet_times() {
        let sat = from_tle_lines(&REFERENCE_ISS).unwrap();
        let epoch = sat.elements.datetime.and_utc().timestamp() as f64;
        let expected = propagate_earth_fixed(&sat, epoch).unwrap();
        let engine = TleEngine {
            tles: vec![sat],
            observer_lat: 0.0,
            observer_lon: 0.0,
            cached_passes: vec![],
            cached_at: None,
            cached_key: None,
            catalog_revision: 1,
        };
        let footprint = engine
            .geolocate_lrpt_lines(
                "ISS (ZARYA)",
                3,
                &[Some(epoch); 1],
                lrpt_decode::geolocation::ScanGeometry {
                    first_pixel_angle_rad: 0.0,
                    last_pixel_angle_rad: 0.0,
                },
                1,
            )
            .unwrap();
        let ground = footprint.scanlines[0][1].unwrap();
        assert!((ground.latitude_deg - expected.latitude_deg).abs() < 0.1);
        assert!((normalize_longitude(ground.longitude_deg - expected.longitude_deg)).abs() < 0.1);
        let model = engine.lrpt_geolocation_model("ISS (ZARYA)").unwrap();
        let source = image::GrayImage::from_pixel(5, 1, image::Luma([42]));
        let products = model
            .georeference_image(
                &source,
                &[Some(epoch)],
                lrpt_decode::geolocation::ScanGeometry {
                    first_pixel_angle_rad: -0.1,
                    last_pixel_angle_rad: 0.1,
                },
                1,
            )
            .unwrap();
        assert_eq!(products.projected.image.dimensions(), source.dimensions());
        assert!(products
            .projected
            .image
            .pixels()
            .any(|pixel| pixel[3] == 255 && pixel[0] == 42));
        assert!(!products.vectors.ground_control_points.is_empty());
        let error = engine
            .geolocate_lrpt_lines(
                "ISS (ZARYA)",
                1,
                &[None],
                lrpt_decode::geolocation::ScanGeometry {
                    first_pixel_angle_rad: 0.0,
                    last_pixel_angle_rad: 0.0,
                },
                1,
            )
            .unwrap_err();
        assert!(error.contains("without absolute CDS time"));
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
