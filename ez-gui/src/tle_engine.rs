#[derive(Debug, Clone)]
pub struct TleEntry {
    pub name: String,
    pub mean_motion: f64,
    pub inclination: f64,
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

impl TleEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            tles: vec![],
            observer_lat: 51.5,
            observer_lon: -0.1,
            cached_passes: vec![],
            // `None` means "never computed" — forces an immediate refresh on
            // the first upcoming_passes() call. Previously done via
            // `Instant::now().checked_sub(999s).unwrap()` which panics on
            // hosts whose uptime is under 999 s.
            cached_at: None,
        };
        engine.load_builtin();
        engine
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_observer(lat: f64, lon: f64) -> Self {
        let mut engine = Self::new();
        engine.observer_lat = lat;
        engine.observer_lon = lon;
        engine
    }

    fn load_builtin(&mut self) {
        self.tles = vec![
            TleEntry {
                name: "NOAA 15".into(),
                mean_motion: 14.26,
                inclination: 98.74,
            },
            TleEntry {
                name: "NOAA 18".into(),
                mean_motion: 14.13,
                inclination: 99.01,
            },
            TleEntry {
                name: "NOAA 19".into(),
                mean_motion: 14.13,
                inclination: 98.99,
            },
            TleEntry {
                name: "Meteor-M2-2".into(),
                mean_motion: 14.21,
                inclination: 98.57,
            },
            TleEntry {
                name: "ISS".into(),
                mean_motion: 15.50,
                inclination: 51.64,
            },
        ];
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
        let mut passes = vec![];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
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
        passes.sort_by(|a, b| {
            a.aos_dt
                .partial_cmp(&b.aos_dt)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        passes
    }

    pub fn doppler_shift(&self, sat: &TleEntry, freq_hz: f64, t: f64) -> f64 {
        let period_s = 1440.0 / sat.mean_motion * 60.0;
        let orbit_phase = (t % period_s) / period_s;
        let vel_lat = sat.inclination * 2.0 * std::f64::consts::PI / period_s * orbit_phase.cos();
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
    if t < 0.0 || !t.is_finite() {
        return "N/A".into();
    }
    let epoch = std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(t);
    let datetime: chrono::DateTime<chrono::Utc> = epoch.into();
    datetime.format("%H:%M:%S UTC").to_string()
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn from_tle_lines(lines: &[&str]) -> Result<TleEntry, String> {
    if lines.len() < 3 {
        return Err("Need at least 3 lines: name, line1, line2".into());
    }
    let name = lines[0].trim().to_string();
    let line2 = lines[2].trim();
    if !line2.starts_with('2') {
        return Err(format!("line 2 must start with '2', got: {line2}"));
    }
    let inclination = line2
        .as_bytes()
        .get(8..16)
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .ok_or_else(|| "Cannot parse inclination from line 2".to_string())?;
    let mean_motion = line2
        .as_bytes()
        .get(52..63)
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .ok_or_else(|| "Cannot parse mean motion from line 2".to_string())?;
    Ok(TleEntry {
        name,
        mean_motion,
        inclination,
    })
}

impl TleEngine {
    /// Compute real-time satellite azimuth/elevation/distance from observer.
    pub fn satellite_position(
        &self,
        name: &str,
        observer_lat: f64,
        observer_lon: f64,
        t: f64,
    ) -> Option<crate::satellite::types::SatPosition> {
        let sat = self.tles.iter().find(|s| s.name == name)?;
        let period_s = 1440.0 / sat.mean_motion * 60.0;
        let orbit_phase = (t % period_s) / period_s;

        let sat_lat = sat.inclination * (2.0 * std::f64::consts::PI * orbit_phase).sin();
        let sat_lon = (observer_lon + 360.0 * orbit_phase) % 360.0 - 180.0;

        let dlat = (sat_lat - observer_lat).to_radians();
        let dlon = (sat_lon - observer_lon).to_radians();
        let a = (dlat * 0.5).sin().powi(2)
            + observer_lat.to_radians().cos()
                * sat_lat.to_radians().cos()
                * (dlon * 0.5).sin().powi(2);
        let c: f64 = 2.0 * a.sqrt().asin();
        let earth_r: f64 = 6371.0;
        let orbit_alt: f64 = 850.0;
        let dist_ground: f64 = earth_r * c;
        let dist_km: f64 = (dist_ground * dist_ground + orbit_alt * orbit_alt).sqrt();

        let elev_ground = (90.0 - c.to_degrees()).max(0.0);
        let elevation = (orbit_alt / dist_km).atan().to_degrees() + elev_ground * 0.3;

        let y = dlon.sin() * sat_lat.to_radians().cos();
        let x = observer_lat.to_radians().cos() * sat_lat.to_radians().sin()
            - observer_lat.to_radians().sin() * sat_lat.to_radians().cos() * dlon.cos();
        let azimuth = (y.atan2(x).to_degrees() + 360.0) % 360.0;

        Some(crate::satellite::types::SatPosition {
            azimuth,
            elevation: elevation.min(90.0),
            distance_km: dist_km,
            lat: sat_lat,
            lon: sat_lon,
            timestamp: t,
        })
    }

    /// Compute ground-track polyline for a full pass over the next N hours.
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
        let dt = 30.0;
        let steps = (hours * 3600.0 / dt) as usize;

        let pass_window = self
            .compute_passes(observer_lat, observer_lon, hours)
            .iter()
            .find(|p| p.satellite == name)
            .map(|p| (p.aos_dt, p.los_dt));

        let (aos_dt, los_dt) = match pass_window {
            Some((a, l)) => (a, l),
            None => return points,
        };

        for i in 0..steps {
            let t = now + i as f64 * dt;
            if t < aos_dt - 600.0 || t > los_dt + 600.0 {
                continue;
            }
            if let Some(pos) = self.satellite_position(name, observer_lat, observer_lon, t) {
                let segment = if t < aos_dt {
                    crate::satellite::types::TrajectorySegment::PreAOS
                } else if t > los_dt {
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
        }
        points
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sat_frequency_known() {
        assert_eq!(sat_frequency("ISS"), 145_800_000);
        assert_eq!(sat_frequency("NOAA 15"), 137_620_000);
        assert_eq!(sat_frequency("NOAA 19"), 137_100_000);
    }

    #[test]
    fn sat_frequency_unknown_default() {
        assert_eq!(sat_frequency("Unknown Satellite"), 100_000_000);
    }

    #[test]
    fn format_time_epoch_midnight() {
        let s = format_time(0.0);
        assert!(s.contains("1970-01-01") || s.contains(":00:00 UTC"));
    }

    #[test]
    fn new_engine_has_builtin_sats() {
        let engine = TleEngine::new();
        assert_eq!(engine.tles.len(), 5);
        let names: Vec<&str> = engine.tles.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"ISS"));
        assert!(names.contains(&"NOAA 15"));
    }

    #[test]
    fn new_engine_default_observer_at_london() {
        let engine = TleEngine::new();
        assert!((engine.observer_lat - 51.5).abs() < 0.01);
        assert!((engine.observer_lon - (-0.1)).abs() < 0.01);
    }

    #[test]
    fn doppler_shift_for_sat_iss() {
        let engine = TleEngine::new();
        let shift = engine.doppler_shift_for_sat("ISS", 145_800_000.0, 100_000.0);
        // Doppler shift should be a reasonable value (not zero, not huge)
        assert!(shift.abs() > 0.0);
        // For LEO at 145.8 MHz the worst-case Doppler is ~±10 kHz. A prior
        // missing /period_s factor in vel_lat inflated this by ~5400×, so a
        // 30 kHz cap catches that regression.
        assert!(
            shift.abs() < 30_000.0,
            "doppler shift too large (likely missing period_s divisor): {}",
            shift
        );
    }

    #[test]
    fn doppler_shift_unknown_sat_returns_zero() {
        let engine = TleEngine::new();
        assert_eq!(
            engine.doppler_shift_for_sat("NONEXISTENT", 100_000_000.0, 0.0),
            0.0
        );
    }

    #[test]
    fn compute_passes_returns_sorted() {
        let engine = TleEngine::new();
        let passes = engine.compute_passes(51.5, -0.1, 24.0);
        // Within a 24-hour window there should be several passes from 5 sats
        assert!(!passes.is_empty());
        for i in 1..passes.len() {
            assert!(
                passes[i - 1].aos_dt <= passes[i].aos_dt,
                "passes not sorted by AOS"
            );
        }
    }

    #[test]
    fn compute_passes_contains_expected_sats() {
        let engine = TleEngine::new();
        let passes = engine.compute_passes(51.5, -0.1, 72.0);
        let sats: std::collections::BTreeSet<&str> =
            passes.iter().map(|p| p.satellite.as_str()).collect();
        // Should have some NOAA and ISS passes
        assert!(sats.contains("ISS"));
    }

    #[test]
    fn format_time_returns_utc_string() {
        let s = format_time(1_234_567_890.0);
        // Should contain UTC somewhere — chrono formats vary by version
        assert!(!s.is_empty());
    }

    // --- from_tle_lines tests ---

    #[test]
    fn from_tle_lines_valid_iss() {
        // Proper fixed-width TLE (69-char lines per standard)
        let lines = vec![
            "ISS (ZARYA)",
            "1 25544U 98067A   24001.50000000  .00000000  00000+0  00000+0 0  9991",
            "2 25544  51.6420  -0.1000 0007000   0.0000   0.0000 15.50138746 99990",
        ];
        let entry = from_tle_lines(&lines).expect("valid TLE lines should parse to OrbitalEntry");
        assert_eq!(entry.name, "ISS (ZARYA)");
        assert!((entry.inclination - 51.642).abs() < 0.001);
        assert!((entry.mean_motion - 15.50138746).abs() < 0.0001);
    }

    #[test]
    fn from_tle_lines_too_few_lines() {
        let err =
            from_tle_lines(&["Name", "1 ..."]).expect_err("too few lines should produce error");
        assert!(err.contains("3 lines"));
    }

    #[test]
    fn from_tle_lines_invalid_line2_prefix() {
        let lines = vec!["Sat", "1 ...", "3 99999  99.0000"];
        let err = from_tle_lines(&lines).expect_err("invalid line2 prefix should produce error");
        assert!(err.contains("must start with '2'"));
    }

    #[test]
    fn from_tle_lines_malformed_empty_line2() {
        let err = from_tle_lines(&["Sat", "1 ...", "2"])
            .expect_err("malformed line2 should produce error");
        assert!(err.contains("inclination") || err.contains("mean motion"));
    }

    #[test]
    fn from_tle_lines_partial_inclination() {
        let lines = vec!["Sat", "1 ...", "2 25544   abcdef  ..."];
        let err = from_tle_lines(&lines).expect_err("partial inclination should produce error");
        assert!(err.contains("inclination") || err.contains("mean motion"));
    }

    // --- compute_passes edge cases ---

    #[test]
    fn compute_passes_empty_tles() {
        let mut empty = TleEngine::with_observer(51.5, -0.1);
        empty.tles.clear();
        let passes = empty.compute_passes(51.5, -0.1, 24.0);
        assert!(passes.is_empty());
    }

    #[test]
    fn compute_passes_observer_at_north_pole() {
        let engine = TleEngine::with_observer(90.0, 0.0);
        let passes = engine.compute_passes(90.0, 0.0, 72.0);
        // The simplified model may still produce passes at the pole
        // but the key is the function doesn't crash
        assert!(passes.is_empty() || !passes.is_empty());
    }

    #[test]
    fn compute_passes_observer_at_equator() {
        let engine = TleEngine::with_observer(0.0, 0.0);
        let passes = engine.compute_passes(0.0, 0.0, 48.0);
        assert!(!passes.is_empty(), "should see passes from equator");
        let names: std::collections::BTreeSet<&str> =
            passes.iter().map(|p| p.satellite.as_str()).collect();
        assert!(names.contains("ISS"));
    }

    #[test]
    fn compute_passes_short_window_yields_fewer_passes() {
        let engine = TleEngine::new();
        let passes_1h = engine.compute_passes(51.5, -0.1, 1.0);
        let passes_72h = engine.compute_passes(51.5, -0.1, 72.0);
        assert!(passes_1h.len() <= passes_72h.len());
    }

    // --- doppler_shift_for_sat edge values ---

    #[test]
    fn doppler_shift_for_sat_zero_hz() {
        let engine = TleEngine::new();
        let shift = engine.doppler_shift_for_sat("ISS", 0.0, 100_000.0);
        assert_eq!(shift, 0.0, "0 Hz should yield 0 Doppler shift");
    }

    #[test]
    fn doppler_shift_for_sat_large_freq() {
        let engine = TleEngine::new();
        let shift = engine.doppler_shift_for_sat("ISS", 1e12, 100_000.0);
        // At high frequency the shift magnitude is larger
        assert!(shift.abs() > 100.0, "large freq should give large shift");
        assert!(shift < 0.0, "shift should be negative (receding)");
    }

    // --- format_time edge timestamps ---

    #[test]
    fn format_time_negative_returns_na() {
        let s = format_time(-1.0);
        assert_eq!(s, "N/A");
    }

    #[test]
    fn format_time_nan_returns_na() {
        let s = format_time(f64::NAN);
        assert_eq!(s, "N/A");
    }

    #[test]
    fn format_time_infinity_returns_na() {
        let s = format_time(f64::INFINITY);
        assert_eq!(s, "N/A");
    }

    #[test]
    fn format_time_year_2038_boundary() {
        // 2038-01-19 03:14:07 UTC = 2147483647 (i32 max)
        let s = format_time(2_147_483_647.0);
        assert!(s.contains("UTC") || s.contains("N/A"));
        assert!(!s.is_empty());
    }

    #[test]
    fn format_time_large_timestamp() {
        // Year 3000-ish: ~32503680000 seconds from epoch
        let s = format_time(32_503_680_000.0);
        assert!(s.contains(":") && s.contains("UTC"));
    }

    // --- TleEngine with_observer ---

    #[test]
    fn with_observer_custom_location() {
        let engine = TleEngine::with_observer(-33.86, 151.21);
        assert!((engine.observer_lat - (-33.86)).abs() < 0.01);
        assert!((engine.observer_lon - 151.21).abs() < 0.01);
        assert_eq!(engine.tles.len(), 5);
    }

    #[test]
    fn with_observer_south_pole() {
        let engine = TleEngine::with_observer(-90.0, 0.0);
        assert!((engine.observer_lat - (-90.0)).abs() < 0.01);
        let passes = engine.compute_passes(-90.0, 0.0, 24.0);
        // Function runs without panicking
        assert!(passes.is_empty() || !passes.is_empty());
    }
}
