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
fn cpr_nl_function(lat: f64) -> i32 {
    let lat = lat.abs();
    if lat < 10.47047130 {
        return 59;
    }
    if lat < 14.82817437 {
        return 58;
    }
    if lat < 18.18626357 {
        return 57;
    }
    if lat < 21.02939493 {
        return 56;
    }
    if lat < 23.54504487 {
        return 55;
    }
    if lat < 25.82924707 {
        return 54;
    }
    if lat < 27.93898710 {
        return 53;
    }
    if lat < 29.91135686 {
        return 52;
    }
    if lat < 31.77209708 {
        return 51;
    }
    if lat < 33.53993436 {
        return 50;
    }
    if lat < 35.22899598 {
        return 49;
    }
    if lat < 36.85025108 {
        return 48;
    }
    if lat < 38.41241892 {
        return 47;
    }
    if lat < 39.92256684 {
        return 46;
    }
    if lat < 41.38651832 {
        return 45;
    }
    if lat < 42.80914012 {
        return 44;
    }
    if lat < 44.19454951 {
        return 43;
    }
    if lat < 45.54626723 {
        return 42;
    }
    if lat < 46.86733252 {
        return 41;
    }
    if lat < 48.16039128 {
        return 40;
    }
    if lat < 49.42776439 {
        return 39;
    }
    if lat < 50.67150166 {
        return 38;
    }
    if lat < 51.89342469 {
        return 37;
    }
    if lat < 53.09516153 {
        return 36;
    }
    if lat < 54.27817472 {
        return 35;
    }
    if lat < 55.44378444 {
        return 34;
    }
    if lat < 56.59318756 {
        return 33;
    }
    if lat < 57.72747354 {
        return 32;
    }
    if lat < 58.84763776 {
        return 31;
    }
    if lat < 59.95459277 {
        return 30;
    }
    if lat < 61.04917774 {
        return 29;
    }
    if lat < 62.13216659 {
        return 28;
    }
    if lat < 63.20427479 {
        return 27;
    }
    if lat < 64.26616523 {
        return 26;
    }
    if lat < 65.31845310 {
        return 25;
    }
    if lat < 66.36171008 {
        return 24;
    }
    if lat < 67.39646774 {
        return 23;
    }
    if lat < 68.42322022 {
        return 22;
    }
    if lat < 69.44242631 {
        return 21;
    }
    if lat < 70.45451075 {
        return 20;
    }
    if lat < 71.45986473 {
        return 19;
    }
    if lat < 72.45884545 {
        return 18;
    }
    if lat < 73.45177442 {
        return 17;
    }
    if lat < 74.43893416 {
        return 16;
    }
    if lat < 75.42056257 {
        return 15;
    }
    if lat < 76.39684391 {
        return 14;
    }
    if lat < 77.36789461 {
        return 13;
    }
    if lat < 78.33374083 {
        return 12;
    }
    if lat < 79.29428225 {
        return 11;
    }
    if lat < 80.24923213 {
        return 10;
    }
    if lat < 81.19801349 {
        return 9;
    }
    if lat < 82.13956981 {
        return 8;
    }
    if lat < 83.07199445 {
        return 7;
    }
    if lat < 83.99173563 {
        return 6;
    }
    if lat < 84.89166191 {
        return 5;
    }
    if lat < 85.75541621 {
        return 4;
    }
    if lat < 86.53536998 {
        return 3;
    }
    if lat < 87.00000000 {
        return 2;
    }
    1
}

fn cpr_n_function(lat: f64, fflag: bool) -> i32 {
    let mut nl = cpr_nl_function(lat) - if fflag { 1 } else { 0 };
    if nl < 1 {
        nl = 1;
    }
    nl
}

fn cpr_dlon_function(lat: f64, fflag: bool, surface: bool) -> f64 {
    (if surface { 90.0 } else { 360.0 }) / cpr_n_function(lat, fflag) as f64
}

/// Decode a pair of airborne CPR frames.
///
/// `fflag` is `false` for even, `true` for odd — it selects which frame to
/// use as the authoritative latitude.
pub fn decode_cpr_airborne(
    even_lat: u32,
    even_lon: u32,
    odd_lat: u32,
    odd_lon: u32,
    fflag: bool,
) -> Option<(f64, f64)> {
    let air_dlat0 = 360.0 / 60.0;
    let air_dlat1 = 360.0 / 59.0;

    let lat0 = even_lat as f64;
    let lat1 = odd_lat as f64;
    let lon0 = even_lon as f64;
    let lon1 = odd_lon as f64;

    let j = ((59.0 * lat0 - 60.0 * lat1) / 131072.0 + 0.5).floor() as i32;
    let mut rlat0 = air_dlat0 * (cpr_mod(j, 60) as f64 + lat0 / 131072.0);
    let mut rlat1 = air_dlat1 * (cpr_mod(j, 59) as f64 + lat1 / 131072.0);

    if rlat0 >= 270.0 {
        rlat0 -= 360.0;
    }
    if rlat1 >= 270.0 {
        rlat1 -= 360.0;
    }

    if rlat0 < -90.0 || rlat0 > 90.0 || rlat1 < -90.0 || rlat1 > 90.0 {
        return None;
    }

    if cpr_nl_function(rlat0) != cpr_nl_function(rlat1) {
        return None;
    }

    let (rlat, rlon) = if fflag {
        let ni = cpr_n_function(rlat1, true);
        let nl = cpr_nl_function(rlat1);
        let m = (((lon0 * (nl - 1) as f64) - (lon1 * nl as f64)) / 131072.0 + 0.5).floor() as i32;
        let rlon = cpr_dlon_function(rlat1, true, false) * (cpr_mod(m, ni) as f64 + lon1 / 131072.0);
        (rlat1, rlon)
    } else {
        let ni = cpr_n_function(rlat0, false);
        let nl = cpr_nl_function(rlat0);
        let m = (((lon0 * (nl - 1) as f64) - (lon1 * nl as f64)) / 131072.0 + 0.5).floor() as i32;
        let rlon = cpr_dlon_function(rlat0, false, false) * (cpr_mod(m, ni) as f64 + lon0 / 131072.0);
        (rlat0, rlon)
    };

    let rlon = rlon - ((rlon + 180.0) / 360.0).floor() * 360.0;
    Some((rlat, rlon))
}

/// Decode a pair of surface CPR frames given a reference position.
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

    let lat0 = even_lat as f64;
    let lat1 = odd_lat as f64;
    let lon0 = even_lon as f64;
    let lon1 = odd_lon as f64;

    let j = ((59.0 * lat0 - 60.0 * lat1) / 131072.0 + 0.5).floor() as i32;
    let mut rlat0 = air_dlat0 * (cpr_mod(j, 60) as f64 + lat0 / 131072.0);
    let mut rlat1 = air_dlat1 * (cpr_mod(j, 59) as f64 + lat1 / 131072.0);

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

    if rlat0 < -90.0 || rlat0 > 90.0 || rlat1 < -90.0 || rlat1 > 90.0 {
        return None;
    }

    if cpr_nl_function(rlat0) != cpr_nl_function(rlat1) {
        return None;
    }

    let (rlat, rlon) = if fflag {
        let ni = cpr_n_function(rlat1, true);
        let nl = cpr_nl_function(rlat1);
        let m = (((lon0 * (nl - 1) as f64) - (lon1 * nl as f64)) / 131072.0 + 0.5).floor() as i32;
        let rlon = cpr_dlon_function(rlat1, true, true) * (cpr_mod(m, ni) as f64 + lon1 / 131072.0);
        (rlat1, rlon)
    } else {
        let ni = cpr_n_function(rlat0, false);
        let nl = cpr_nl_function(rlat0);
        let m = (((lon0 * (nl - 1) as f64) - (lon1 * nl as f64)) / 131072.0 + 0.5).floor() as i32;
        let rlon = cpr_dlon_function(rlat0, false, true) * (cpr_mod(m, ni) as f64 + lon0 / 131072.0);
        (rlat0, rlon)
    };

    let mut rlon = rlon + ((reflon - rlon + 45.0) / 90.0).floor() * 90.0;
    rlon = rlon - ((rlon + 180.0) / 360.0).floor() * 360.0;
    Some((rlat, rlon))
}

/// Decode a single CPR frame given a nearby reference position.
pub fn decode_cpr_relative(
    reflat: f64,
    reflon: f64,
    cprlat: u32,
    cprlon: u32,
    fflag: bool,
    surface: bool,
) -> Option<(f64, f64)> {
    let fractional_lat = cprlat as f64 / 131072.0;
    let fractional_lon = cprlon as f64 / 131072.0;

    let air_dlat = (if surface { 90.0 } else { 360.0 }) / (if fflag { 59.0 } else { 60.0 });

    let j = (reflat / air_dlat).floor()
        + (0.5 + cpr_mod_double(reflat, air_dlat) / air_dlat - fractional_lat).floor();
    let mut rlat = air_dlat * (j + fractional_lat);
    if rlat >= 270.0 {
        rlat -= 360.0;
    }

    if rlat < -90.0 || rlat > 90.0 {
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
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Submit a new CPR frame for an aircraft.
    ///
    /// Returns a decoded `(lat, lon)` if a matching even/odd pair is available.
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
        assert!((lat - 52.2572).abs() < 0.001);
        assert!((lon - 3.9193).abs() < 0.001);
    }
}
