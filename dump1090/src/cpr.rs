//! Compact Position Reporting — translated from cpr.c

use std::collections::HashMap;

/// CPR encoding type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CprType {
    #[default]
    Surface,
    Airborne,
    Coarse,
}

/// A single CPR frame extracted from a Mode S message.
#[derive(Debug, Clone, Copy)]
pub struct CprFrame {
    pub cpr_type: CprType,
    pub odd: bool,
    pub lat: u32,
    pub lon: u32,
}

/// Always-positive modulo for integers.
fn cpr_mod(a: i32, b: i32) -> i32 {
    let mut res = a % b;
    if res < 0 {
        res += b;
    }
    res
}

/// Always-positive modulo for doubles.
fn cpr_mod_double(a: f64, b: f64) -> f64 {
    let mut res = a % b;
    if res < 0.0 {
        res += b;
    }
    res
}

/// NL function using the pre-computed lookup table from 1090-WP-9-14.
///
/// Lookup boundaries are in ascending order from equator to pole.
fn cpr_nl_function(lat: f64) -> i32 {
    let lat = lat.abs();
    const BOUNDARIES: &[(f64, i32)] = &[
        (10.47047130, 59),
        (14.82817437, 58),
        (18.18626357, 57),
        (21.02939493, 56),
        (23.54504487, 55),
        (25.82924707, 54),
        (27.93898710, 53),
        (29.91135686, 52),
        (31.77209708, 51),
        (33.53993436, 50),
        (35.22899598, 49),
        (36.85025108, 48),
        (38.41241892, 47),
        (39.92256684, 46),
        (41.38651832, 45),
        (42.80914012, 44),
        (44.19454951, 43),
        (45.54626723, 42),
        (46.86733252, 41),
        (48.16039128, 40),
        (49.42776439, 39),
        (50.67150166, 38),
        (51.89342469, 37),
        (53.09516153, 36),
        (54.27817472, 35),
        (55.44378444, 34),
        (56.59318756, 33),
        (57.72747354, 32),
        (58.84763776, 31),
        (59.95459277, 30),
        (61.04917774, 29),
        (62.13216659, 28),
        (63.20427479, 27),
        (64.26616523, 26),
        (65.31845310, 25),
        (66.36171008, 24),
        (67.39646774, 23),
        (68.42322022, 22),
        (69.44242631, 21),
        (70.45451075, 20),
        (71.45986473, 19),
        (72.45884545, 18),
        (73.45177442, 17),
        (74.43893416, 16),
        (75.42056257, 15),
        (76.39684391, 14),
        (77.36789461, 13),
        (78.33374083, 12),
        (79.29428225, 11),
        (80.24923213, 10),
        (81.19801349, 9),
        (82.13956981, 8),
        (83.07199445, 7),
        (83.99173563, 6),
        (84.89166191, 5),
        (85.75541621, 4),
        (86.53536998, 3),
        (87.00000000, 2),
    ];

    for &(boundary, nl) in BOUNDARIES {
        if lat < boundary {
            return nl;
        }
    }
    1
}

fn cpr_n_function(lat: f64, fflag: bool) -> i32 {
    let mut nl = cpr_nl_function(lat) - i32::from(fflag);
    if nl < 1 {
        nl = 1;
    }
    nl
}

fn cpr_dlon_function(lat: f64, fflag: bool, surface: bool) -> f64 {
    (if surface { 90.0 } else { 360.0 }) / f64::from(cpr_n_function(lat, fflag))
}

/// Decode a pair of airborne CPR frames.
///
/// `fflag` is `false` for even, `true` for odd — it selects which frame to
/// use as the authoritative latitude.
#[must_use]
pub fn decode_cpr_airborne(
    even_lat: u32,
    even_lon: u32,
    odd_lat: u32,
    odd_lon: u32,
    fflag: bool,
) -> Option<(f64, f64)> {
    let air_dlat0 = 360.0 / 60.0;
    let air_dlat1 = 360.0 / 59.0;

    let lat0 = f64::from(even_lat);
    let lat1 = f64::from(odd_lat);
    let lon0 = f64::from(even_lon);
    let lon1 = f64::from(odd_lon);

    let j = ((59.0 * lat0 - 60.0 * lat1) / 131072.0 + 0.5).floor() as i32;
    let mut rlat0 = air_dlat0 * (f64::from(cpr_mod(j, 60)) + lat0 / 131072.0);
    let mut rlat1 = air_dlat1 * (f64::from(cpr_mod(j, 59)) + lat1 / 131072.0);

    if rlat0 >= 270.0 {
        rlat0 -= 360.0;
    }
    if rlat1 >= 270.0 {
        rlat1 -= 360.0;
    }

    if !(-90.0..=90.0).contains(&rlat0) || !(-90.0..=90.0).contains(&rlat1) {
        return None;
    }

    if cpr_nl_function(rlat0) != cpr_nl_function(rlat1) {
        return None;
    }

    let (rlat, rlon) = if fflag {
        let ni = cpr_n_function(rlat1, true);
        let nl = cpr_nl_function(rlat1);
        let m =
            (((lon0 * f64::from(nl - 1)) - (lon1 * f64::from(nl))) / 131072.0 + 0.5).floor() as i32;
        let rlon =
            cpr_dlon_function(rlat1, true, false) * (f64::from(cpr_mod(m, ni)) + lon1 / 131072.0);
        (rlat1, rlon)
    } else {
        let ni = cpr_n_function(rlat0, false);
        let nl = cpr_nl_function(rlat0);
        let m =
            (((lon0 * f64::from(nl - 1)) - (lon1 * f64::from(nl))) / 131072.0 + 0.5).floor() as i32;
        let rlon =
            cpr_dlon_function(rlat0, false, false) * (f64::from(cpr_mod(m, ni)) + lon0 / 131072.0);
        (rlat0, rlon)
    };

    let rlon = rlon - ((rlon + 180.0) / 360.0).floor() * 360.0;
    Some((rlat, rlon))
}

/// Decode a pair of surface CPR frames given a reference position.
#[must_use]
pub fn decode_cpr_surface(
    reflat: f64,
    reflon: f64,
    even_lat: u32,
    even_lon: u32,
    odd_lat: u32,
    odd_lon: u32,
    fflag: bool,
) -> Option<(f64, f64)> {
    let air_dlat0 = 90.0 / 60.0;
    let air_dlat1 = 90.0 / 59.0;

    let lat0 = f64::from(even_lat);
    let lat1 = f64::from(odd_lat);
    let lon0 = f64::from(even_lon);
    let lon1 = f64::from(odd_lon);

    let j = ((59.0 * lat0 - 60.0 * lat1) / 131072.0 + 0.5).floor() as i32;
    let mut rlat0 = air_dlat0 * (f64::from(cpr_mod(j, 60)) + lat0 / 131072.0);
    let mut rlat1 = air_dlat1 * (f64::from(cpr_mod(j, 59)) + lat1 / 131072.0);

    if rlat0 == 0.0 {
        if reflat < -45.0 {
            rlat0 = -90.0;
        } else if reflat > 45.0 {
            rlat0 = 90.0;
        }
    } else if rlat0 - reflat > 45.0 {
        rlat0 -= 90.0;
    }

    if rlat1 == 0.0 {
        if reflat < -45.0 {
            rlat1 = -90.0;
        } else if reflat > 45.0 {
            rlat1 = 90.0;
        }
    } else if rlat1 - reflat > 45.0 {
        rlat1 -= 90.0;
    }

    if !(-90.0..=90.0).contains(&rlat0) || !(-90.0..=90.0).contains(&rlat1) {
        return None;
    }

    if cpr_nl_function(rlat0) != cpr_nl_function(rlat1) {
        return None;
    }

    let (rlat, rlon) = if fflag {
        let ni = cpr_n_function(rlat1, true);
        let nl = cpr_nl_function(rlat1);
        let m =
            (((lon0 * f64::from(nl - 1)) - (lon1 * f64::from(nl))) / 131072.0 + 0.5).floor() as i32;
        let rlon =
            cpr_dlon_function(rlat1, true, true) * (f64::from(cpr_mod(m, ni)) + lon1 / 131072.0);
        (rlat1, rlon)
    } else {
        let ni = cpr_n_function(rlat0, false);
        let nl = cpr_nl_function(rlat0);
        let m =
            (((lon0 * f64::from(nl - 1)) - (lon1 * f64::from(nl))) / 131072.0 + 0.5).floor() as i32;
        let rlon =
            cpr_dlon_function(rlat0, false, true) * (f64::from(cpr_mod(m, ni)) + lon0 / 131072.0);
        (rlat0, rlon)
    };

    let mut rlon = rlon + ((reflon - rlon + 45.0) / 90.0).floor() * 90.0;
    rlon = rlon - ((rlon + 180.0) / 360.0).floor() * 360.0;
    Some((rlat, rlon))
}

/// Decode a single CPR frame given a nearby reference position.
#[must_use]
pub fn decode_cpr_relative(
    reflat: f64,
    reflon: f64,
    cprlat: u32,
    cprlon: u32,
    fflag: bool,
    surface: bool,
) -> Option<(f64, f64)> {
    let fractional_lat = f64::from(cprlat) / 131072.0;
    let fractional_lon = f64::from(cprlon) / 131072.0;

    let air_dlat = (if surface { 90.0 } else { 360.0 }) / (if fflag { 59.0 } else { 60.0 });

    let j = (reflat / air_dlat).floor()
        + (0.5 + cpr_mod_double(reflat, air_dlat) / air_dlat - fractional_lat).floor();
    let mut rlat = air_dlat * (j + fractional_lat);
    if rlat >= 270.0 {
        rlat -= 360.0;
    }

    if !(-90.0..=90.0).contains(&rlat) {
        return None;
    }
    if (rlat - reflat).abs() > air_dlat / 2.0 {
        return None;
    }

    let air_dlon = cpr_dlon_function(rlat, fflag, surface);
    let m = (reflon / air_dlon).floor()
        + (0.5 + cpr_mod_double(reflon, air_dlon) / air_dlon - fractional_lon).floor();
    let mut rlon = air_dlon * (m + fractional_lon);
    if rlon > 180.0 {
        rlon -= 360.0;
    }

    if (rlon - reflon).abs() > air_dlon / 2.0 {
        return None;
    }

    Some((rlat, rlon))
}

/// Per-aircraft CPR cache entry.
#[derive(Debug, Clone, Copy, Default)]
struct CprCacheEntry {
    even: Option<CprFrame>,
    odd: Option<CprFrame>,
}

/// Stateful CPR decoder that caches recent frames per ICAO address.
#[derive(Debug, Clone, Default)]
pub struct CprDecoder {
    cache: HashMap<u32, CprCacheEntry>,
}

impl CprDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Submit a new CPR frame for an aircraft.
    ///
    /// Returns a decoded `(lat, lon)` if a matching even/odd pair is available.
    #[must_use]
    pub fn submit(&mut self, icao: u32, frame: CprFrame) -> Option<(f64, f64)> {
        let entry = self.cache.entry(icao).or_default();

        match frame.cpr_type {
            CprType::Airborne => {
                if frame.odd {
                    entry.odd = Some(frame);
                    let even = entry.even?;
                    decode_cpr_airborne(even.lat, even.lon, frame.lat, frame.lon, true)
                } else {
                    entry.even = Some(frame);
                    let odd = entry.odd?;
                    decode_cpr_airborne(frame.lat, frame.lon, odd.lat, odd.lon, false)
                }
            }
            CprType::Surface => {
                // Surface decoding requires a reference position, which we don't have here.
                // Store the frame but do not decode globally.
                if frame.odd {
                    entry.odd = Some(frame);
                } else {
                    entry.even = Some(frame);
                }
                None
            }
            CprType::Coarse => {
                // Store similarly to surface.
                if frame.odd {
                    entry.odd = Some(frame);
                } else {
                    entry.even = Some(frame);
                }
                None
            }
        }
    }

    /// Attempt relative decoding for a surface frame when a reference position is known.
    #[must_use]
    pub fn decode_surface_relative(
        &mut self,
        icao: u32,
        reflat: f64,
        reflon: f64,
    ) -> Option<(f64, f64)> {
        let entry = self.cache.get(&icao)?;
        let frame = entry.even.or(entry.odd)?;
        decode_cpr_relative(reflat, reflon, frame.lat, frame.lon, frame.odd, true)
    }

    /// Clear all cached frames.
    pub fn clear(&mut self) {
        self.cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpr_airborne_decode() {
        // Example from dump1090 test suite / known frames.
        let even_lat = 93000;
        let even_lon = 113609;
        let odd_lat = 74158;
        let odd_lon = 108994;
        let (lat, lon) = decode_cpr_airborne(even_lat, even_lon, odd_lat, odd_lon, false)
            .expect("valid airborne decode");
        // Verified against an independent reference implementation of the
        // dump1090 CPR algorithm; both lat and lon match to well within tolerance.
        assert!((lat - 52.2572).abs() < 0.001);
        assert!((lon - 8.6676).abs() < 0.001);
    }

    #[test]
    fn cpr_mod_positive() {
        assert_eq!(cpr_mod(5, 3), 2);
        assert_eq!(cpr_mod(10, 5), 0);
    }

    #[test]
    fn cpr_mod_negative_yields_positive() {
        assert_eq!(cpr_mod(-1, 360), 359);
        assert_eq!(cpr_mod(-5, 3), 1);
    }

    #[test]
    fn cpr_mod_zero() {
        assert_eq!(cpr_mod(0, 360), 0);
    }

    #[test]
    fn cpr_mod_double_positive() {
        let r = cpr_mod_double(5.5, 3.0);
        assert!((r - 2.5).abs() < 1e-9);
    }

    #[test]
    fn cpr_mod_double_negative() {
        let r = cpr_mod_double(-1.0, 360.0);
        assert!((r - 359.0).abs() < 1e-9);
    }

    #[test]
    fn cpr_nl_function_equator() {
        assert_eq!(cpr_nl_function(0.0), 59);
    }

    #[test]
    fn cpr_nl_function_boundaries() {
        // Below first boundary = 59, just above = 58
        assert_eq!(cpr_nl_function(10.0), 59);
        assert_eq!(cpr_nl_function(10.5), 58);
    }

    #[test]
    fn cpr_nl_function_pole() {
        // At 87°, lat < 87.000 is false, falls through to 1
        assert_eq!(cpr_nl_function(87.0), 1);
        assert_eq!(cpr_nl_function(88.0), 1);
        assert_eq!(cpr_nl_function(90.0), 1);
    }

    #[test]
    fn cpr_nl_function_just_below_87() {
        // At 86.9, lat < 87.0 = true, returns 2
        assert_eq!(cpr_nl_function(86.9), 2);
    }

    #[test]
    fn cpr_nl_function_negative_lat_same_as_positive() {
        assert_eq!(cpr_nl_function(-10.0), cpr_nl_function(10.0));
        assert_eq!(cpr_nl_function(-45.0), cpr_nl_function(45.0));
    }

    #[test]
    fn cpr_nl_function_at_45_degrees() {
        // At 45°, lat < 45.54626723 = true, returns 42
        assert_eq!(cpr_nl_function(45.0), 42);
    }
}
