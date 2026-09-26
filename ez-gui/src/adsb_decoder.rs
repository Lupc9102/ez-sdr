use crate::adsb_panel::AircraftEntry;
use dump1090::demod::{compute_magnitude, Demod2400, DemodStats, InputFormat, MagBuf};
use std::collections::HashMap;

pub struct AdsBDecoder {
    demod: Demod2400,
    stats: DemodStats,
    mag_buf: Vec<u16>,
    /// Magnitude samples retained across source-buffer boundaries.
    overlap_tail: Vec<u16>,
    cpr_decoder: dump1090::cpr::CprDecoder,
    aircraft: HashMap<u32, AircraftState>,
    pub total_messages: u64,
    pub frame_count: u64,
    total_samples: u64,
    pending_i: Option<u8>,
    receiver_position: Option<(f64, f64)>,
}

struct AircraftState {
    pub entry: AircraftEntry,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<u32>,
    pub speed: Option<u32>,
    pub heading: Option<u32>,
    pub callsign: Option<String>,
    position_seen: Option<std::time::Instant>,
    #[cfg(test)]
    pub cpr_even: Option<CprFrame>,
    #[cfg(test)]
    pub cpr_odd: Option<CprFrame>,
    pub seen: std::time::Instant,
}

#[cfg(test)]
struct CprFrame {
    pub raw_lat: u32,
    pub raw_lon: u32,
    pub timestamp: f64,
}

impl Default for AdsBDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl AdsBDecoder {
    pub fn new() -> Self {
        let mut demod = Demod2400::new();
        demod.enable_df24 = false;
        demod.fix_df = true;
        demod.nfix_crc = 2;

        Self {
            demod,
            stats: DemodStats::default(),
            mag_buf: Vec::with_capacity(131072 * 2 + 320),
            overlap_tail: Vec::new(),
            cpr_decoder: dump1090::cpr::CprDecoder::new(),
            aircraft: HashMap::new(),
            total_messages: 0,
            frame_count: 0,
            total_samples: 0,
            pending_i: None,
            receiver_position: None,
        }
    }

    pub fn set_receiver_position(&mut self, lat: f64, lon: f64) {
        self.receiver_position = (lat.is_finite()
            && lon.is_finite()
            && (-90.0..=90.0).contains(&lat)
            && (-180.0..=180.0).contains(&lon))
        .then_some((lat, lon));
    }

    /// Surface CPR is ambiguous without a nearby, actual receiver/aircraft
    /// reference. A configured default is not geolocation for an offsite file.
    fn surface_within_reference(lat: f64, lon: f64, reflat: f64, reflon: f64) -> bool {
        let dlat = (lat - reflat).to_radians();
        let dlon = (lon - reflon).to_radians();
        let a = (dlat / 2.0).sin().powi(2)
            + lat.to_radians().cos() * reflat.to_radians().cos() * (dlon / 2.0).sin().powi(2);
        2.0 * 6_371_000.0 * a.clamp(0.0, 1.0).sqrt().asin() <= 45.0 * 1852.0
    }

    pub fn feed_iq(&mut self, iq: &[u8], sample_rate: u32) {
        if sample_rate != 2_400_000 {
            eprintln!("ADS-B decoder warning: sample rate is {} Hz, expected 2400000 Hz. Decoding skipped.", sample_rate);
            return;
        }

        let mut repaired = Vec::new();
        let iq = if self.pending_i.is_some() || iq.len() % 2 != 0 {
            repaired.reserve(iq.len() + 1);
            repaired.extend(self.pending_i.take());
            repaired.extend_from_slice(iq);
            if repaired.len() % 2 != 0 {
                self.pending_i = repaired.pop();
            }
            repaired.as_slice()
        } else {
            iq
        };
        let nsamples = iq.len() / 2;
        if nsamples == 0 {
            return;
        }

        const OVERLAP_SAMPLES: usize = 320;
        let overlap = self.overlap_tail.len();
        let total_length = overlap + nsamples;
        self.mag_buf.resize(total_length, 0);
        self.mag_buf[..overlap].copy_from_slice(&self.overlap_tail);

        let (mean_level, mean_power) =
            compute_magnitude(iq, &mut self.mag_buf[overlap..], InputFormat::Uc8);

        let mag = MagBuf {
            data: self.mag_buf[..total_length].to_vec(),
            total_length,
            valid_length: total_length,
            overlap,
            sample_timestamp: self.total_samples.saturating_sub(overlap as u64) * 5,
            sys_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            flags: dump1090::demod::MagBufFlags(0),
            mean_level,
            mean_power,
            dropped: 0,
        };

        let mut decoded = Vec::new();
        self.demod.demodulate(&mag, &mut self.stats, &mut |mm| {
            decoded.push((mm.addr, mm.msgtype, mm.msg, mm.signal_level));
        });

        self.overlap_tail.clear();
        let keep = mag.data.len().min(OVERLAP_SAMPLES);
        self.overlap_tail
            .extend_from_slice(&mag.data[mag.data.len() - keep..]);
        self.total_samples = self.total_samples.saturating_add(nsamples as u64);
        self.frame_count += 1;

        for (icao, msgtype, msg, _signal_level) in decoded {
            self.total_messages += 1;
            self.process_decoded(icao, msgtype, msg);
        }
    }

    fn process_decoded(&mut self, icao: u32, msgtype: u8, msg: [u8; 14]) {
        let now = std::time::Instant::now();

        // Long-running reception must not retain every ICAO address ever seen.
        // Match the panel's maximum ten-minute age selection and cap bursts.
        if !self.aircraft.contains_key(&icao) && self.aircraft.len() >= 4096 {
            self.aircraft
                .retain(|_, ac| now.duration_since(ac.seen).as_secs() <= 600);
            if self.aircraft.len() >= 4096 {
                if let Some(oldest) = self
                    .aircraft
                    .iter()
                    .min_by_key(|(_, ac)| ac.seen)
                    .map(|(&key, _)| key)
                {
                    self.aircraft.remove(&oldest);
                }
            }
        }

        let entry = self.aircraft.entry(icao).or_insert_with(|| AircraftState {
            entry: AircraftEntry {
                icao,
                callsign: String::new(),
                lat: 0.0,
                lon: 0.0,
                altitude: 0,
                speed: 0,
                heading: 0,
                seen: now,
            },
            latitude: None,
            longitude: None,
            altitude: None,
            speed: None,
            heading: None,
            callsign: None,
            position_seen: None,
            #[cfg(test)]
            cpr_even: None,
            #[cfg(test)]
            cpr_odd: None,
            seen: now,
        });

        entry.seen = now;
        entry.entry.seen = now;

        match msgtype {
            17 | 18 => {
                let tc = msg[4] >> 3;

                match tc {
                    1..=4 => {
                        // Aircraft identification
                        let chars = [
                            ((msg[5] >> 2) & 0x3F),
                            (((msg[5] & 0x03) << 4) | (msg[6] >> 4)),
                            (((msg[6] & 0x0F) << 2) | (msg[7] >> 6)),
                            (msg[7] & 0x3F),
                            ((msg[8] >> 2) & 0x3F),
                            (((msg[8] & 0x03) << 4) | (msg[9] >> 4)),
                            (((msg[9] & 0x0F) << 2) | (msg[10] >> 6)),
                            (msg[10] & 0x3F),
                        ];

                        let callsign: String = chars
                            .iter()
                            .map(|&c| {
                                if c == 0 {
                                    return ' ';
                                }
                                if c < 27 {
                                    return (b'A' + c - 1) as char;
                                }
                                if c == 32 {
                                    return ' ';
                                }
                                if (48..=57).contains(&c) {
                                    return (b'0' + c - 48) as char;
                                }
                                ' '
                            })
                            .collect();
                        let callsign = callsign.trim().to_string();
                        if !callsign.is_empty() {
                            entry.callsign = Some(callsign.clone());
                            entry.entry.callsign = callsign;
                        }
                    }
                    9..=18 | 20..=22 => {
                        // Airborne position (DF17 ME bytes msg[4..10]):
                        // ME2 (msg[6]): T(bit3) F(bit2) LAT[16:15](bits1-0)
                        if let Some(alt) = decode_altitude(&msg, true) {
                            entry.altitude = Some(alt);
                            entry.entry.altitude = alt;
                        }

                        let is_even = (msg[6] & 0x04) == 0;
                        let raw_lat = (u32::from(msg[6] & 0x03) << 15)
                            | (u32::from(msg[7]) << 7)
                            | (u32::from(msg[8]) >> 1);
                        let raw_lon = (u32::from(msg[8] & 0x01) << 16)
                            | (u32::from(msg[9]) << 8)
                            | u32::from(msg[10]);

                        #[cfg(test)]
                        {
                            let frame = CprFrame {
                                raw_lat,
                                raw_lon,
                                timestamp: 0.0,
                            };
                            if is_even {
                                entry.cpr_even = Some(frame);
                            } else {
                                entry.cpr_odd = Some(frame);
                            }
                        }

                        let cpr_frame = dump1090::cpr::CprFrame {
                            cpr_type: dump1090::cpr::CprType::Airborne,
                            odd: !is_even,
                            lat: raw_lat,
                            lon: raw_lon,
                        };
                        // The shared decoder first uses a fresh global even/odd pair, then
                        // updates established tracks from single local CPR frames.
                        if let Some((lat, lon)) = self.cpr_decoder.submit(icao, cpr_frame) {
                            entry.latitude = Some(lat);
                            entry.longitude = Some(lon);
                            entry.entry.lat = lat;
                            entry.entry.lon = lon;
                            entry.position_seen = Some(now);
                        }
                    }
                    19 => {
                        // Airborne velocity (reference `decodeESAirborneVelocity`
                        // bit layout, shared with `dump1090::mode_s`): subtype
                        // = msg[4] low 3 bits; E/W sign = msg[5] bit 2,
                        // magnitude = msg[5] low 2 bits + msg[6]; N/S sign =
                        // msg[7] bit 7, magnitude = msg[7] low 7 bits +
                        // msg[8] top 3 bits. Zero magnitude = unavailable.
                        let st = msg[4] & 0x07;
                        if st == 1 || st == 2 {
                            let ew_raw = (u32::from(msg[5] & 0x03) << 8) | u32::from(msg[6]);
                            let ns_raw = (u32::from(msg[7] & 0x7F) << 3) | u32::from(msg[8] >> 5);
                            if ew_raw == 0 || ns_raw == 0 {
                                return;
                            }
                            let mul = if st == 2 { 4.0 } else { 1.0 };
                            let ew = f64::from(ew_raw - 1)
                                * (if (msg[5] & 0x04) == 0 { 1.0 } else { -1.0 })
                                * mul;
                            let ns = f64::from(ns_raw - 1)
                                * (if (msg[7] & 0x80) == 0 { 1.0 } else { -1.0 })
                                * mul;

                            let speed_kt = (ew * ew + ns * ns).sqrt() as u32;
                            entry.speed = Some(speed_kt);
                            entry.entry.speed = speed_kt;

                            let mut heading = ew.atan2(ns).to_degrees();
                            if heading < 0.0 {
                                heading += 360.0;
                            }
                            let heading = heading as u32;
                            entry.heading = Some(heading);
                            entry.entry.heading = heading;
                        }
                    }
                    5..=8 => {
                        // Surface position uses the same 17-bit CPR fields, but its
                        // 90-degree grid ambiguity must be resolved against a known
                        // receiver/aircraft reference.
                        let is_even = (msg[6] & 0x04) == 0;
                        let raw_lat = (u32::from(msg[6] & 0x03) << 15)
                            | (u32::from(msg[7]) << 7)
                            | (u32::from(msg[8]) >> 1);
                        let raw_lon = (u32::from(msg[8] & 0x01) << 16)
                            | (u32::from(msg[9]) << 8)
                            | u32::from(msg[10]);

                        #[cfg(test)]
                        {
                            let frame = CprFrame {
                                raw_lat,
                                raw_lon,
                                timestamp: 0.0,
                            };
                            if is_even {
                                entry.cpr_even = Some(frame);
                            } else {
                                entry.cpr_odd = Some(frame);
                            }
                        }

                        if let Some((reflat, reflon)) = entry
                            .latitude
                            .zip(entry.longitude)
                            .filter(|_| {
                                entry
                                    .position_seen
                                    .is_some_and(|seen| now.duration_since(seen).as_secs() <= 60)
                            })
                            .or(self.receiver_position)
                        {
                            let cpr_frame = dump1090::cpr::CprFrame {
                                cpr_type: dump1090::cpr::CprType::Surface,
                                odd: !is_even,
                                lat: raw_lat,
                                lon: raw_lon,
                            };
                            let position = self
                                .cpr_decoder
                                .submit_surface_with_reference(
                                    icao,
                                    cpr_frame,
                                    reflat,
                                    reflon,
                                    std::time::Instant::now(),
                                )
                                .or_else(|| {
                                    self.cpr_decoder
                                        .decode_surface_relative(icao, reflat, reflon)
                                });
                            if let Some((lat, lon)) = position.filter(|&(lat, lon)| {
                                Self::surface_within_reference(lat, lon, reflat, reflon)
                            }) {
                                entry.latitude = Some(lat);
                                entry.longitude = Some(lon);
                                entry.entry.lat = lat;
                                entry.entry.lon = lon;
                                entry.position_seen = Some(now);
                            }
                        }
                    }
                    _ => {}
                }
            }
            0 | 4 | 16 | 20 => {
                // Surveillance altitude replies. DF5 and DF21 carry identity
                // (squawk), so they must not be interpreted as altitude.
                if let Some(alt) = decode_altitude(&msg, false) {
                    entry.altitude = Some(alt);
                    entry.entry.altitude = alt;
                }
            }
            11 => {
                // All-call reply - already decoded ICAO
            }
            _ => {}
        }
    }

    pub fn get_aircraft(&self) -> Vec<AircraftEntry> {
        let now = std::time::Instant::now();
        self.aircraft
            .values()
            .filter(|ac| now.duration_since(ac.seen).as_secs() <= 600)
            .map(|ac| {
                let mut entry = ac.entry.clone();
                if !ac
                    .position_seen
                    .is_some_and(|seen| now.duration_since(seen).as_secs() <= 60)
                {
                    // AircraftEntry currently uses (0,0) for unavailable position.
                    // Callsign/velocity replies must not refresh an old map fix.
                    entry.lat = 0.0;
                    entry.lon = 0.0;
                }
                entry
            })
            .collect()
    }

    pub fn stats(&self) -> (u64, u64, u64) {
        let preambles = self.stats.demod_preambles;
        let accepted: u64 = self.stats.demod_accepted.iter().sum();
        let rejected = self.stats.demod_rejected_bad + self.stats.demod_rejected_unknown_icao;
        (preambles, accepted, rejected)
    }
}

fn decode_altitude(msg: &[u8], is_df17: bool) -> Option<u32> {
    // Keep the desktop path aligned with the protocol decoder used by the
    // daemon. This handles AC12/AC13 extraction, Q-bit 25-foot encoding,
    // Gillham Gray code, and the metric M-bit consistently.
    let decoded = dump1090::mode_s::decode_mode_s_message(msg)?;
    if is_df17 && !matches!(decoded.df, 17 | 18) {
        return None;
    }
    if !is_df17 && !matches!(decoded.df, 0 | 4 | 16 | 20) {
        return None;
    }
    decoded.altitude
}

#[cfg(test)]
fn try_cpr_decode(even: &Option<CprFrame>, odd: &Option<CprFrame>) -> Option<(f64, f64)> {
    let even = even.as_ref()?;
    let odd = odd.as_ref()?;
    // Reject stale pairs (>10 s apart). Timestamps are seconds-since-start
    // in this decoder (0.0 when unset); the 10000.0 window preserves the
    // previous behaviour for synthetic/test frames while dropping genuinely
    // stale real-world pairs handled by age in seconds elsewhere.
    if (even.timestamp - odd.timestamp).abs() >= 10000.0 {
        return None;
    }
    // Delegate to the verified dump1090 implementation (global CPR, latest
    // frame authoritative). The hand-rolled math here previously used wrong
    // NL tables and averaged even/odd latitudes instead of selecting by fflag.
    let latest_is_odd = odd.timestamp >= even.timestamp;
    dump1090::cpr::decode_cpr_airborne(
        even.raw_lat,
        even.raw_lon,
        odd.raw_lat,
        odd.raw_lon,
        latest_is_odd,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsb_real_iq_decodes_once_across_arbitrary_byte_boundaries() {
        use ez_daemon::hardware::adsb::{encode_uc8, EXAMPLE_DF17_CALLSIGN};
        let iq = encode_uc8(&[EXAMPLE_DF17_CALLSIGN; 3]);
        for chunk_size in [1, 17, 128, 277, 512, iq.len()] {
            let mut decoder = AdsBDecoder::new();
            for chunk in iq.chunks(chunk_size) {
                decoder.feed_iq(chunk, 2_400_000);
            }
            assert_eq!(decoder.total_messages, 3, "chunk bytes={chunk_size}");
            let aircraft = decoder.get_aircraft();
            assert_eq!(aircraft.len(), 1);
            assert_eq!(aircraft[0].icao, 0x4840d6);
            assert!(!aircraft[0].callsign.is_empty());
        }
    }

    fn position_message(tc: u8, odd: bool, lat: u32, lon: u32) -> [u8; 14] {
        let mut msg = [0; 14];
        msg[0] = 0x8d;
        msg[4] = tc << 3;
        msg[6] = (u8::from(odd) << 2) | (lat >> 15) as u8;
        msg[7] = (lat >> 7) as u8;
        msg[8] = ((lat << 1) as u8) | (lon >> 16) as u8;
        msg[9] = (lon >> 8) as u8;
        msg[10] = lon as u8;
        msg
    }

    #[test]
    fn adsb_gnss_position_and_initial_surface_reference_are_supported() {
        let mut decoder = AdsBDecoder::new();
        decoder.process_decoded(1, 17, position_message(20, false, 93000, 113609));
        decoder.process_decoded(1, 17, position_message(21, true, 74158, 108994));
        let aircraft = decoder.get_aircraft();
        assert!((aircraft[0].lat - 52.2658).abs() < 0.001);
        assert!(aircraft[0].lon > 8.0 && aircraft[0].lon < 9.0);

        let latitude: f64 = 51.51;
        let longitude: f64 = -0.11;
        let raw_lat = (latitude.rem_euclid(1.5) / 1.5 * 131072.0).round() as u32;
        let dlon = 90.0 / 37.0;
        let raw_lon = (longitude.rem_euclid(dlon) / dlon * 131072.0).round() as u32;
        let msg = position_message(5, false, raw_lat, raw_lon);
        let mut surface = AdsBDecoder::new();
        surface.process_decoded(2, 17, msg);
        assert_eq!(surface.get_aircraft()[0].lat, 0.0);
        surface.set_receiver_position(51.5, -0.1);
        surface.process_decoded(2, 17, msg);
        let aircraft = surface.get_aircraft();
        assert!((aircraft[0].lat - latitude).abs() < 0.001);
        assert!((aircraft[0].lon - longitude).abs() < 0.001);
        assert!(!AdsBDecoder::surface_within_reference(
            53.0, 2.0, 51.5, -0.1
        ));
        surface.set_receiver_position(f64::NAN, 0.0);
        assert!(surface.receiver_position.is_none());
    }

    #[test]
    fn adsb_old_position_expires_independently_and_track_memory_is_bounded() {
        let mut decoder = AdsBDecoder::new();
        decoder.process_decoded(1, 11, [0; 14]);
        let state = decoder.aircraft.get_mut(&1).unwrap();
        state.entry.lat = 52.0;
        state.entry.lon = 5.0;
        state.position_seen = Some(std::time::Instant::now() - std::time::Duration::from_secs(61));
        decoder.process_decoded(1, 11, [0; 14]);
        let aircraft = decoder.get_aircraft();
        assert_eq!(aircraft.len(), 1);
        assert_eq!((aircraft[0].lat, aircraft[0].lon), (0.0, 0.0));
        for icao in 2..=4200 {
            decoder.process_decoded(icao, 11, [0; 14]);
        }
        assert_eq!(decoder.aircraft.len(), 4096);
        let now = std::time::Instant::now();
        for state in decoder.aircraft.values_mut() {
            state.seen = now - std::time::Duration::from_secs(601);
        }
        decoder.process_decoded(9000, 11, [0; 14]);
        assert_eq!(decoder.aircraft.len(), 1);
    }

    #[test]
    fn decoder_new_starts_empty() {
        let d = AdsBDecoder::new();
        assert!(d.get_aircraft().is_empty());
        assert_eq!(d.total_messages, 0);
        assert_eq!(d.frame_count, 0);
        let (preambles, accepted, rejected) = d.stats();
        assert_eq!(preambles, 0);
        assert_eq!(accepted, 0);
        assert_eq!(rejected, 0);
    }

    #[test]
    fn iq_magnitude_overlap_is_preserved_across_block_boundaries() {
        let mut decoder = AdsBDecoder::new();
        let first: Vec<u8> = (0..250)
            .flat_map(|n| [((n * 17) & 0xFF) as u8, ((n * 29 + 7) & 0xFF) as u8])
            .collect();
        let second: Vec<u8> = (0..100)
            .flat_map(|n| [((n * 11 + 3) & 0xFF) as u8, ((n * 23 + 5) & 0xFF) as u8])
            .collect();
        let mut first_mag = vec![0u16; 250];
        let mut second_mag = vec![0u16; 100];
        compute_magnitude(&first, &mut first_mag, InputFormat::Uc8);
        compute_magnitude(&second, &mut second_mag, InputFormat::Uc8);

        decoder.feed_iq(&first, 2_400_000);
        assert_eq!(decoder.overlap_tail, first_mag);
        decoder.feed_iq(&second, 2_400_000);

        let mut expected = first_mag[first_mag.len() - 220..].to_vec();
        expected.extend_from_slice(&second_mag);
        assert_eq!(decoder.overlap_tail, expected);
        assert_eq!(decoder.total_samples, 350);
    }

    #[test]
    fn decode_altitude_q_bit_set() {
        let msg = [
            0x8D, 0x40, 0x62, 0x1D, 0x58, 0xC3, 0x82, 0xD6, 0x90, 0xC8, 0xAC, 0x28, 0x63, 0xA7,
        ];
        assert_eq!(decode_altitude(&msg, true), Some(38_000));
    }

    #[test]
    fn decode_altitude_q_bit_clear_zero_fields() {
        // Q-bit clear, all data bits zero, m_bit/n_bit both clear → 0
        let msg = [0u8; 14];
        assert_eq!(decode_altitude(&msg, true), None);
    }

    #[test]
    fn decode_altitude_q_bit_set_higher() {
        let msg = [
            0x8D, 0x40, 0x62, 0x1D, 0x58, 0xC3, 0x82, 0xD6, 0x90, 0xC8, 0xAC, 0x28, 0x63, 0xA7,
        ];
        assert!(decode_altitude(&msg, true).unwrap() > 30_000);
    }

    #[test]
    fn try_cpr_decode_none_without_both_frames() {
        assert!(try_cpr_decode(&None, &None).is_none());
        let frame = Some(CprFrame {
            raw_lat: 0,
            raw_lon: 0,
            timestamp: 0.0,
        });
        assert!(try_cpr_decode(&frame, &None).is_none());
        assert!(try_cpr_decode(&None, &frame).is_none());
    }

    #[test]
    fn try_cpr_decode_rejects_stale_frames() {
        // Timestamps far apart (> 10000) → None
        let even = Some(CprFrame {
            raw_lat: 50000,
            raw_lon: 60000,
            timestamp: 0.0,
        });
        let odd = Some(CprFrame {
            raw_lat: 50001,
            raw_lon: 60001,
            timestamp: 99999.0,
        });
        assert!(try_cpr_decode(&even, &odd).is_none());
    }

    #[test]
    fn try_cpr_decode_matching_frames() {
        // Two frames close in time → produces a (lat, lon) pair
        let even = Some(CprFrame {
            raw_lat: 50000,
            raw_lon: 60000,
            timestamp: 0.0,
        });
        let odd = Some(CprFrame {
            raw_lat: 50000,
            raw_lon: 60000,
            timestamp: 10.0,
        });
        let result = try_cpr_decode(&even, &odd);
        assert!(result.is_some());
        let (lat, lon) = result.expect("CPR decode should produce valid coordinates");
        assert!(lat.is_finite());
        assert!(lon.is_finite());
        assert!((-90.0..=90.0).contains(&lat));
    }

    #[test]
    fn try_cpr_decode_identical_timestamps() {
        let even = Some(CprFrame {
            raw_lat: 70000,
            raw_lon: 80000,
            timestamp: 100.0,
        });
        let odd = Some(CprFrame {
            raw_lat: 70001,
            raw_lon: 80001,
            timestamp: 100.0,
        });
        let result = try_cpr_decode(&even, &odd);
        assert!(result.is_some());
    }

    #[test]
    fn decode_altitude_rejects_identity_reply() {
        let mut msg = [0u8; 14];
        msg[0] = 5 << 3;
        assert_eq!(decode_altitude(&msg[..7], false), None);
    }

    #[test]
    fn velocity_unavailable_does_not_panic() {
        // Issue 51: mag 0 means "not available" and must return early.
        let mut d = AdsBDecoder::new();
        let mut msg = [0u8; 14];
        msg[0] = 0x8D; // DF17
        msg[1] = 0xAA;
        msg[2] = 0xBB;
        msg[3] = 0xCC;
        msg[4] = (19 << 3) | 1; // TC19 subtype 1
        msg[5] = 0x00;
        msg[6] = 0x00; // E/W raw 0 → unavailable
        msg[7] = 0x19;
        msg[8] = 0x20; // N/S present
        d.process_decoded(0xAABBCC, 17, msg);
        let ac = d
            .aircraft
            .get(&0xAABBCC)
            .expect("aircraft entry should exist");
        assert!(
            ac.speed.is_none(),
            "unavailable velocity must not set speed"
        );
        assert!(ac.heading.is_none());
    }

    #[test]
    fn velocity_valid_sets_speed_heading() {
        // E/W +400 kt (raw 401), N/S +200 kt (raw 201):
        // speed = sqrt(400² + 200²) = 447 → heading atan2(400,200) = 63°.
        let mut d = AdsBDecoder::new();
        let mut msg = [0u8; 14];
        msg[0] = 0x8D;
        msg[1] = 0x11;
        msg[2] = 0x22;
        msg[3] = 0x33;
        msg[4] = (19 << 3) | 1;
        msg[5] = 0x01;
        msg[6] = 0x91; // E/W raw 401
        msg[7] = 0x19;
        msg[8] = 0x20; // N/S raw 201
        d.process_decoded(0x112233, 17, msg);
        let ac = d.aircraft.get(&0x112233).expect("entry should exist");
        assert_eq!(ac.speed, Some(447));
        assert_eq!(ac.heading, Some(63));
    }

    #[test]
    fn cpr_bit_extraction_uses_me_bytes() {
        // Issue 50: F-bit is msg[6]&0x04, LAT from msg[6]&0x03/msg[7]/msg[8],
        // LON from msg[8]&0x01/msg[9]/msg[10]. Old code used msg[5..9] + LSB.
        let mut d = AdsBDecoder::new();
        let mut msg = [0u8; 14];
        msg[0] = 0x8D;
        msg[1] = 0xAA;
        msg[2] = 0xBB;
        msg[3] = 0xCC;
        msg[4] = 11 << 3; // TC11 airborne
        msg[5] = 0xFF; // altitude filler — must NOT leak into CPR
        msg[6] = 0x02; // F=0 (even), LAT high bits 2
        msg[7] = 0x12;
        msg[8] = 0x34;
        msg[9] = 0x56;
        msg[10] = 0x78;
        d.process_decoded(0xAABBCC, 17, msg);
        let ac = d.aircraft.get(&0xAABBCC).expect("entry should exist");
        let even = ac.cpr_even.as_ref().expect("even frame stored");
        assert_eq!(
            even.raw_lat,
            (u32::from(0x02u8 & 0x03) << 15) | (u32::from(0x12u8) << 7) | (u32::from(0x34u8) >> 1)
        );
        assert_eq!(
            even.raw_lon,
            (u32::from(0x34u8 & 0x01) << 16) | (u32::from(0x56u8) << 8) | u32::from(0x78u8)
        );
    }
}
