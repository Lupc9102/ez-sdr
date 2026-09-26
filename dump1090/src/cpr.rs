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

    rlat0 += ((reflat - rlat0 + 45.0) / 90.0).floor() * 90.0;
    rlat1 += ((reflat - rlat1 + 45.0) / 90.0).floor() * 90.0;

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
    let rlon = air_dlon * (m + fractional_lon);
    // Normalize both antimeridian directions. Comparing raw longitudes would
    // reject a valid pair when the reference is +179° and the decoded point
    // is -179° (or vice versa), so compare the shortest wrapped delta.
    let rlon = rlon - ((rlon + 180.0) / 360.0).floor() * 360.0;
    let delta_lon = (rlon - reflon + 180.0).rem_euclid(360.0) - 180.0;
    if delta_lon.abs() > air_dlon / 2.0 {
        return None;
    }

    Some((rlat, rlon))
}

const MAX_CPR_CACHE_ENTRIES: usize = 2048;
const CPR_PAIR_TIMEOUT_SECS: f64 = 10.0;

/// Timestamped CPR frame for TTL pairing.
#[derive(Debug, Clone, Copy)]
struct TimedCprFrame {
    frame: CprFrame,
    time: std::time::Instant,
}

/// Per-aircraft CPR cache entry.
#[derive(Debug, Clone, Copy, Default)]
struct CprCacheEntry {
    even: Option<TimedCprFrame>,
    odd: Option<TimedCprFrame>,
    last_seen: Option<std::time::Instant>,
    last_position: Option<(f64, f64)>,
    last_position_time: Option<std::time::Instant>,
}

impl CprCacheEntry {
    fn prepare(&mut self, frame: CprFrame, now: std::time::Instant) {
        if self
            .even
            .or(self.odd)
            .is_some_and(|cached| cached.frame.cpr_type != frame.cpr_type)
        {
            // Airborne uses a 360-degree grid; surface uses a 90-degree grid.
            // Frames either side of landing/takeoff cannot form a global pair.
            self.even = None;
            self.odd = None;
            self.last_position = None;
            self.last_position_time = None;
        }
        if !self.last_position_time.is_some_and(|seen| {
            now.checked_duration_since(seen)
                .is_some_and(|age| age.as_secs() <= 60)
        }) {
            self.last_position = None;
            self.last_position_time = None;
        }
        self.last_seen = Some(now);
    }
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

    /// Prune entries older than `max_age`.
    pub fn prune_older_than(&mut self, max_age: std::time::Duration) {
        let now = std::time::Instant::now();
        self.cache.retain(|_, entry| {
            entry.last_seen.is_some_and(|t| {
                let dt = if now >= t {
                    now.duration_since(t)
                } else {
                    t.duration_since(now)
                };
                dt <= max_age
            })
        });
    }

    /// Evict oldest entries if cache is at or exceeds capacity.
    fn evict_oldest_if_full(&mut self, icao: u32) {
        if self.cache.len() >= MAX_CPR_CACHE_ENTRIES {
            self.prune_older_than(std::time::Duration::from_secs(60));
            let max_allowed = if self.cache.contains_key(&icao) {
                MAX_CPR_CACHE_ENTRIES
            } else {
                MAX_CPR_CACHE_ENTRIES.saturating_sub(1)
            };
            while self.cache.len() > max_allowed {
                if let Some((&oldest_icao, _)) =
                    self.cache.iter().min_by_key(|(_, entry)| entry.last_seen)
                {
                    self.cache.remove(&oldest_icao);
                } else {
                    break;
                }
            }
        }
    }

    /// Submit a new CPR frame with an explicit timestamp (useful for testing).
    #[must_use]
    pub fn submit_with_time(
        &mut self,
        icao: u32,
        frame: CprFrame,
        now: std::time::Instant,
    ) -> Option<(f64, f64)> {
        if frame.lat >= 131072 || frame.lon >= 131072 {
            return None;
        }
        // Automatic cap enforcement: evict oldest entry rather than clearing entire cache
        self.evict_oldest_if_full(icao);

        let entry = self.cache.entry(icao).or_default();
        entry.prepare(frame, now);

        let timed = TimedCprFrame { frame, time: now };

        match frame.cpr_type {
            CprType::Airborne => {
                let global = if frame.odd {
                    entry.odd = Some(timed);
                    entry.even.and_then(|even| {
                        let dt = if now >= even.time {
                            now.duration_since(even.time)
                        } else {
                            even.time.duration_since(now)
                        };
                        (dt.as_secs_f64() <= CPR_PAIR_TIMEOUT_SECS)
                            .then(|| {
                                decode_cpr_airborne(
                                    even.frame.lat,
                                    even.frame.lon,
                                    frame.lat,
                                    frame.lon,
                                    true,
                                )
                            })
                            .flatten()
                    })
                } else {
                    entry.even = Some(timed);
                    entry.odd.and_then(|odd| {
                        let dt = if now >= odd.time {
                            now.duration_since(odd.time)
                        } else {
                            odd.time.duration_since(now)
                        };
                        (dt.as_secs_f64() <= CPR_PAIR_TIMEOUT_SECS)
                            .then(|| {
                                decode_cpr_airborne(
                                    frame.lat,
                                    frame.lon,
                                    odd.frame.lat,
                                    odd.frame.lon,
                                    false,
                                )
                            })
                            .flatten()
                    })
                };
                let decoded = global.or_else(|| {
                    entry.last_position.and_then(|(lat, lon)| {
                        decode_cpr_relative(lat, lon, frame.lat, frame.lon, frame.odd, false)
                    })
                });
                if let Some(position) = decoded {
                    entry.last_position = Some(position);
                    entry.last_position_time = Some(now);
                }
                decoded
            }
            CprType::Surface => {
                if frame.odd {
                    entry.odd = Some(timed);
                } else {
                    entry.even = Some(timed);
                }
                None
            }
            CprType::Coarse => {
                if frame.odd {
                    entry.odd = Some(timed);
                } else {
                    entry.even = Some(timed);
                }
                None
            }
        }
    }

    /// Submit a surface CPR frame when a previously known aircraft position is
    /// available to resolve the 90-degree surface grid ambiguity.
    #[must_use]
    pub fn submit_surface_with_reference(
        &mut self,
        icao: u32,
        frame: CprFrame,
        reflat: f64,
        reflon: f64,
        now: std::time::Instant,
    ) -> Option<(f64, f64)> {
        if frame.cpr_type != CprType::Surface
            || frame.lat >= 131072
            || frame.lon >= 131072
            || !reflat.is_finite()
            || !reflon.is_finite()
            || !(-90.0..=90.0).contains(&reflat)
            || !(-180.0..=180.0).contains(&reflon)
        {
            return None;
        }
        self.evict_oldest_if_full(icao);

        let entry = self.cache.entry(icao).or_default();
        entry.prepare(frame, now);
        let timed = TimedCprFrame { frame, time: now };

        let result = if frame.odd {
            entry.odd = Some(timed);
            let even = entry.even?;
            let dt = if now >= even.time {
                now.duration_since(even.time)
            } else {
                even.time.duration_since(now)
            };
            if dt.as_secs_f64() > CPR_PAIR_TIMEOUT_SECS {
                None
            } else {
                decode_cpr_surface(
                    reflat,
                    reflon,
                    even.frame.lat,
                    even.frame.lon,
                    frame.lat,
                    frame.lon,
                    true,
                )
            }
        } else {
            entry.even = Some(timed);
            let odd = entry.odd?;
            let dt = if now >= odd.time {
                now.duration_since(odd.time)
            } else {
                odd.time.duration_since(now)
            };
            if dt.as_secs_f64() > CPR_PAIR_TIMEOUT_SECS {
                None
            } else {
                decode_cpr_surface(
                    reflat,
                    reflon,
                    frame.lat,
                    frame.lon,
                    odd.frame.lat,
                    odd.frame.lon,
                    false,
                )
            }
        };

        if let Some(position) = result {
            entry.last_position = Some(position);
            entry.last_position_time = Some(now);
        }
        result
    }

    /// Submit a new CPR frame for an aircraft using current time.
    ///
    /// Returns a decoded `(lat, lon)` if a matching even/odd pair within 10s is available.
    #[must_use]
    pub fn submit(&mut self, icao: u32, frame: CprFrame) -> Option<(f64, f64)> {
        self.submit_with_time(icao, frame, std::time::Instant::now())
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
        let timed = match (entry.even, entry.odd) {
            (Some(even), Some(odd)) => {
                if even.time >= odd.time {
                    even
                } else {
                    odd
                }
            }
            (Some(even), None) => even,
            (None, Some(odd)) => odd,
            (None, None) => return None,
        };
        decode_cpr_relative(
            reflat,
            reflon,
            timed.frame.lat,
            timed.frame.lon,
            timed.frame.odd,
            true,
        )
    }

    /// Clear all cached frames.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Return the number of tracked entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpr_reference_expires_and_surface_airborne_frames_never_pair() {
        let mut decoder = CprDecoder::new();
        let now = std::time::Instant::now();
        let even = CprFrame {
            cpr_type: CprType::Airborne,
            odd: false,
            lat: 93000,
            lon: 113609,
        };
        let odd = CprFrame {
            cpr_type: CprType::Airborne,
            odd: true,
            lat: 74158,
            lon: 108994,
        };
        assert!(decoder.submit_with_time(1, even, now).is_none());
        assert!(decoder.submit_with_time(1, odd, now).is_some());
        assert!(decoder
            .submit_with_time(1, even, now + std::time::Duration::from_secs(61))
            .is_none());
        decoder.clear();
        let surface_even = CprFrame {
            cpr_type: CprType::Surface,
            ..even
        };
        assert!(decoder.submit_with_time(1, surface_even, now).is_none());
        assert!(decoder.submit_with_time(1, odd, now).is_none());
        assert!(decoder.submit_with_time(1, even, now).is_some());
        decoder.clear();
        assert!(decoder.submit_with_time(1, even, now).is_none());
        assert!(decoder
            .submit_surface_with_reference(
                1,
                CprFrame {
                    cpr_type: CprType::Surface,
                    ..odd
                },
                52.0,
                8.0,
                now
            )
            .is_none());
    }

    #[test]
    fn cpr_invalid_fields_and_reference_do_not_poison_cache() {
        let mut decoder = CprDecoder::new();
        let now = std::time::Instant::now();
        let invalid = CprFrame {
            cpr_type: CprType::Airborne,
            odd: false,
            lat: 131072,
            lon: 0,
        };
        assert!(decoder.submit_with_time(1, invalid, now).is_none());
        assert_eq!(decoder.len(), 0);
        let surface = CprFrame {
            cpr_type: CprType::Surface,
            lat: 1,
            ..invalid
        };
        assert!(decoder
            .submit_surface_with_reference(1, surface, f64::NAN, 0.0, now)
            .is_none());
        assert_eq!(decoder.len(), 0);
    }

    #[test]
    fn cpr_surface_southern_hemisphere() {
        // Observer in southern hemisphere: reflat = -77.0, reflon = 166.0 (Antarctica)
        // With quadrant ambiguity resolved properly relative to reflat, latitude must be negative.
        let res = decode_cpr_surface(-77.0, 166.0, 93000, 113609, 74158, 108994, false);
        let (lat, _) = res.expect("surface decode should produce a coordinate");
        assert!(
            lat < 0.0,
            "Southern hemisphere latitude should be negative, got {lat}"
        );
        assert!(
            (lat - -76.935).abs() < 0.1,
            "Decoded lat {lat} should be near reference latitude"
        );
    }

    #[test]
    fn surface_decoder_uses_known_receiver_reference() {
        let mut decoder = CprDecoder::new();
        let now = std::time::Instant::now();
        let odd = CprFrame {
            cpr_type: CprType::Surface,
            odd: true,
            lat: 74158,
            lon: 108994,
        };
        let even = CprFrame {
            cpr_type: CprType::Surface,
            odd: false,
            lat: 93000,
            lon: 113609,
        };
        assert!(decoder
            .submit_surface_with_reference(0x123456, odd, -77.0, 166.0, now)
            .is_none());
        let (lat, lon) = decoder
            .submit_surface_with_reference(
                0x123456,
                even,
                -77.0,
                166.0,
                now + std::time::Duration::from_secs(1),
            )
            .expect("surface pair should resolve against the known receiver position");
        assert!((lat + 76.935).abs() < 0.1);
        let wrapped_delta = (lon - 166.0 + 180.0).rem_euclid(360.0) - 180.0;
        assert!(
            wrapped_delta.abs() <= 45.0,
            "surface longitude {lon} was not resolved to the receiver's 90-degree quadrant"
        );
    }

    #[test]
    fn cpr_ttl_rejects_stale_pair() {
        let mut decoder = CprDecoder::new();
        let now = std::time::Instant::now();
        let even = CprFrame {
            cpr_type: CprType::Airborne,
            odd: false,
            lat: 93000,
            lon: 113609,
        };
        let odd = CprFrame {
            cpr_type: CprType::Airborne,
            odd: true,
            lat: 74158,
            lon: 108994,
        };

        // Submit even frame at t = 0
        assert!(decoder.submit_with_time(0x123456, even, now).is_none());

        // Submit odd frame 15 seconds later (> 10s max TTL) -> must be rejected
        let stale_time = now + std::time::Duration::from_secs(15);
        assert!(decoder
            .submit_with_time(0x123456, odd, stale_time)
            .is_none());

        // Submit odd frame within 5 seconds -> must succeed
        let fresh_time = now + std::time::Duration::from_secs(5);
        let pos = decoder.submit_with_time(0x123456, odd, fresh_time);
        assert!(
            pos.is_some(),
            "Fresh frame within TTL should decode successfully"
        );
    }

    #[test]
    fn cpr_cache_pruning() {
        let mut decoder = CprDecoder::new();
        let now = std::time::Instant::now();
        let frame = CprFrame {
            cpr_type: CprType::Airborne,
            odd: false,
            lat: 93000,
            lon: 113609,
        };
        let _ =
            decoder.submit_with_time(0x123456, frame, now - std::time::Duration::from_secs(100));
        assert_eq!(decoder.len(), 1);
        decoder.prune_older_than(std::time::Duration::from_secs(30));
        assert_eq!(decoder.len(), 0);
    }

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
    fn established_airborne_track_updates_from_a_single_local_frame() {
        let mut decoder = CprDecoder::new();
        let now = std::time::Instant::now();
        let even = CprFrame {
            cpr_type: CprType::Airborne,
            odd: false,
            lat: 93000,
            lon: 113609,
        };
        let odd = CprFrame {
            cpr_type: CprType::Airborne,
            odd: true,
            lat: 74158,
            lon: 108994,
        };
        assert!(decoder.submit_with_time(0xABCDEF, odd, now).is_none());
        let established = decoder
            .submit_with_time(0xABCDEF, even, now + std::time::Duration::from_secs(1))
            .expect("global pair establishes the track");

        // The old odd frame is now outside the global-pair window. The new even frame must
        // still update using local CPR relative to the established aircraft position.
        let local = decoder
            .submit_with_time(0xABCDEF, even, now + std::time::Duration::from_secs(20))
            .expect("single-frame local CPR update");
        assert!((local.0 - established.0).abs() < 0.1);
        assert!((local.1 - established.1).abs() < 0.1);
    }

    #[test]
    fn relative_cpr_wraps_across_the_antimeridian() {
        // A reference just west of +180 must accept the equivalent decoded longitude just
        // east of -180 instead of treating it as a 359-degree jump.
        let encoded_lon = ((-179.9_f64).rem_euclid(360.0) / 6.0).fract() * 131072.0;
        let result = decode_cpr_relative(0.0, 179.9, 0, encoded_lon as u32, false, false)
            .expect("antimeridian-relative position should decode");
        assert!((-180.0..180.0).contains(&result.1));
        let wrapped_delta = (result.1 - 179.9 + 180.0).rem_euclid(360.0) - 180.0;
        assert!(wrapped_delta.abs() < 3.1);
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
