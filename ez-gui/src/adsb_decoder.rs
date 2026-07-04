use crate::adsb_panel::AircraftEntry;
use dump1090::demod::{compute_magnitude, Demod2400, DemodStats, InputFormat, MagBuf};
use std::collections::HashMap;

pub struct AdsBDecoder {
    demod: Demod2400,
    stats: DemodStats,
    mag_buf: Vec<u16>,
    aircraft: HashMap<u32, AircraftState>,
    pub total_messages: u64,
    pub frame_count: u64,
}

struct AircraftState {
    pub entry: AircraftEntry,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<u32>,
    pub speed: Option<u32>,
    pub heading: Option<u32>,
    pub callsign: Option<String>,
    pub cpr_even: Option<CprFrame>,
    pub cpr_odd: Option<CprFrame>,
    pub seen: std::time::Instant,
}

struct CprFrame {
    pub raw_lat: u32,
    pub raw_lon: u32,
    pub timestamp: f64,
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
            mag_buf: vec![0u16; 131072 * 2],
            aircraft: HashMap::new(),
            total_messages: 0,
            frame_count: 0,
        }
    }

    pub fn feed_iq(&mut self, iq: &[u8], _sample_rate: u32) {
        let nsamples = iq.len() / 2;
        let overlap = 0;

        self.mag_buf.resize(nsamples.max(131072), 0);

        let (mean_level, mean_power) =
            compute_magnitude(iq, &mut self.mag_buf[..nsamples], InputFormat::Uc8);

        let mag = MagBuf {
            data: self.mag_buf[..nsamples].to_vec(),
            total_length: nsamples,
            valid_length: nsamples,
            overlap,
            sample_timestamp: self.frame_count * nsamples as u64 * 5,
            sys_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            flags: dump1090::demod::MagBufFlags(0),
            mean_level,
            mean_power,
            dropped: 0,
        };

        self.frame_count += 1;

        let mut decoded = Vec::new();
        self.demod.demodulate(&mag, &mut self.stats, &mut |mm| {
            decoded.push((mm.addr, mm.msgtype, mm.msg, mm.signal_level));
        });

        self.demod.demodulate_ac(&mag, &mut self.stats, &mut |mm| {
            decoded.push((mm.addr, mm.msgtype, mm.msg, mm.signal_level));
        });

        for (icao, msgtype, msg, _signal_level) in decoded {
            self.total_messages += 1;
            self.process_decoded(icao, msgtype, msg);
        }
    }

    fn process_decoded(&mut self, icao: u32, msgtype: u8, msg: [u8; 14]) {
        let now = std::time::Instant::now();

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
            cpr_even: None,
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
                    9..=18 => {
                        // Airborne position
                        let alt = decode_altitude(&msg);
                        entry.altitude = Some(alt);
                        entry.entry.altitude = alt;

                        let raw_lat = (u32::from(msg[5]) << 15)
                            | (u32::from(msg[6]) << 7)
                            | (u32::from(msg[7]) >> 1);
                        let raw_lon = (u32::from(msg[7] & 1) << 16)
                            | (u32::from(msg[8]) << 8)
                            | u32::from(msg[9]);
                        let is_even = (msg[6] & 1) == 0;

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

                        // Try to decode position
                        if let Some((lat, lon)) = try_cpr_decode(&entry.cpr_even, &entry.cpr_odd) {
                            entry.latitude = Some(lat);
                            entry.longitude = Some(lon);
                            entry.entry.lat = lat;
                            entry.entry.lon = lon;
                        }
                    }
                    19 => {
                        // Airborne velocity
                        let st = msg[5] & 0x07;
                        if st == 1 || st == 2 {
                            let raw_ew = (u32::from(msg[5]) << 6) | (u32::from(msg[6]) >> 2);
                            let ew_dir = if (raw_ew & 1) == 0 { 1 } else { -1 };
                            let ew_vel = (raw_ew >> 1) - 1;

                            let raw_ns = (u32::from(msg[6] & 3) << 8) | u32::from(msg[7]);
                            let ns_dir = if (raw_ns & 1) == 0 { 1 } else { -1 };
                            let ns_vel = (raw_ns >> 1) - 1;

                            let speed_kt = ((f64::from(ew_vel) * f64::from(ew_dir)).powi(2)
                                + (f64::from(ns_vel) * f64::from(ns_dir)).powi(2))
                            .sqrt() as u32;
                            entry.speed = Some(speed_kt);
                            entry.entry.speed = speed_kt;

                            let heading = (90.0
                                - (f64::from(ns_vel) * f64::from(ns_dir))
                                    .atan2(f64::from(ew_vel) * f64::from(ew_dir))
                                    .to_degrees())
                            .rem_euclid(360.0) as u32;
                            entry.heading = Some(heading);
                            entry.entry.heading = heading;
                        }
                    }
                    20..=22 => {
                        // Surface position
                        let raw_lat = (u32::from(msg[5]) << 15)
                            | (u32::from(msg[6]) << 7)
                            | (u32::from(msg[7]) >> 1);
                        let raw_lon = (u32::from(msg[7] & 1) << 16)
                            | (u32::from(msg[8]) << 8)
                            | u32::from(msg[9]);
                        let is_even = (msg[6] & 1) == 0;

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
                    _ => {}
                }
            }
            0 | 4 | 5 | 16 | 20 | 21 => {
                // Surveillance / altitude messages
                let alt = decode_altitude(&msg);
                entry.altitude = Some(alt);
                entry.entry.altitude = alt;
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
            .filter(|ac| now.duration_since(ac.seen).as_secs() < 60)
            .map(|ac| ac.entry.clone())
            .collect()
    }

    pub fn stats(&self) -> (u64, u64, u64) {
        let preambles = self.stats.demod_preambles;
        let accepted: u64 = self.stats.demod_accepted.iter().sum();
        let rejected = self.stats.demod_rejected_bad + self.stats.demod_rejected_unknown_icao;
        (preambles, accepted, rejected)
    }
}

fn decode_altitude(msg: &[u8]) -> u32 {
    let q = (msg[5] & 0x10) != 0;
    if q {
        let alt16 = (u32::from(msg[5]) << 1) | (u32::from(msg[6]) >> 7);
        ((alt16 & 0x1FF) * 25 + 1000) / 4
    } else {
        let m_bit = (msg[5] & 0x20) != 0;
        let n_bit = (msg[5] & 0x10) != 0;
        let d12 = u32::from(msg[5] & 0x0F);
        let d10 = u32::from((msg[6] >> 5) & 0x07);
        let d8 = u32::from((msg[6] >> 2) & 0x07);
        let d6 = u32::from(((msg[6] & 0x03) << 1) | ((msg[7] >> 6) & 0x01));
        let d4 = u32::from((msg[7] >> 2) & 0x0F);

        let m: u32 = if m_bit { 1600 } else { 0 };
        let n: u32 = if n_bit { 40 } else { 0 };

        d12 * 500 + d10 * 100 + d8 * 20 + d6 * 4 + d4 + m + n
    }
}

fn try_cpr_decode(even: &Option<CprFrame>, odd: &Option<CprFrame>) -> Option<(f64, f64)> {
    let even = even.as_ref()?;
    let odd = odd.as_ref()?;

    let dlat_even = 360.0 / 60.0;
    let dlat_odd = 360.0 / 59.0;

    let j = ((f64::from(even.raw_lat) / 131072.0 / dlat_even).floor()
        + (f64::from(odd.raw_lat) / 131072.0 / dlat_odd).floor()) as i32;

    let r_even = f64::from(even.raw_lat) / 131072.0;
    let r_odd = f64::from(odd.raw_lat) / 131072.0;
    let dlat_even_val = dlat_even;
    let dlat_odd_val = dlat_odd;

    let mut lat_even = dlat_even * (r_even - f64::from(j));
    let mut lat_odd =
        dlat_odd * (r_odd - f64::from(j) + if even.raw_lat < odd.raw_lat { 1.0 } else { 0.0 });

    if lat_even >= 270.0 {
        lat_even -= 360.0;
    }
    if lat_odd >= 270.0 {
        lat_odd -= 360.0;
    }

    let ni_even = if lat_even.abs() >= 87.0 {
        1
    } else {
        std::cmp::max(
            1,
            (60.0 - f64::from(even.raw_lat) / 131072.0 / dlat_even_val) as i32,
        )
    };
    let ni_odd = if lat_odd.abs() >= 87.0 {
        1
    } else {
        std::cmp::max(
            1,
            (59.0 - f64::from(odd.raw_lat) / 131072.0 / dlat_odd_val) as i32,
        )
    };

    let dlon_even = 360.0 / f64::from(ni_even);
    let dlon_odd = 360.0 / f64::from(ni_odd);

    let m = ((f64::from(even.raw_lon) / 131072.0 / dlon_even).floor()
        + (f64::from(odd.raw_lon) / 131072.0 / dlon_odd).floor()) as i32;

    let mut lon_even = dlon_even * (f64::from(even.raw_lon) / 131072.0 - f64::from(m));
    let mut lon_odd = dlon_odd
        * (f64::from(odd.raw_lon) / 131072.0 - f64::from(m)
            + if even.raw_lon < odd.raw_lon { 1.0 } else { 0.0 });

    if lon_even >= 180.0 {
        lon_even -= 360.0;
    }
    if lon_odd >= 180.0 {
        lon_odd -= 360.0;
    }

    let age = (even.timestamp - odd.timestamp).abs();
    if age < 10000.0 {
        let lat = f64::midpoint(lat_even, lat_odd);
        let lon = f64::midpoint(lon_even, lon_odd);
        Some((lat, lon))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn decode_altitude_q_bit_set() {
        // Q-bit (msg[5] & 0x10) set → 25ft encoding
        // msg[5]=0x10, msg[6]=0x00 → alt16 = 0x20 = 32 → (32*25+1000)/4 = 450
        let mut msg = [0u8; 14];
        msg[5] = 0x10;
        msg[6] = 0x00;
        assert_eq!(decode_altitude(&msg), 450);
    }

    #[test]
    fn decode_altitude_q_bit_clear_zero_fields() {
        // Q-bit clear, all data bits zero, m_bit/n_bit both clear → 0
        let msg = [0u8; 14];
        assert_eq!(decode_altitude(&msg), 0);
    }

    #[test]
    fn decode_altitude_q_bit_set_higher() {
        // msg[5]=0x10, msg[6]=0x80 → alt16 = 0x21 = 33 → (33*25+1000)/4 = 456
        let mut msg = [0u8; 14];
        msg[5] = 0x10;
        msg[6] = 0x80;
        assert_eq!(decode_altitude(&msg), 456);
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
    fn decode_altitude_max_qbit() {
        // Q-bit set, all altitude bits 1 → max value
        let mut msg = [0u8; 14];
        msg[5] = 0x10 | 0xFE; // Q-bit + upper bits 1
        msg[6] = 0xFF; // lower bits all 1
                       // alt16 = (0xFE << 1) | (0xFF >> 7) = 0x1FC | 0x01 = 0x1FD = 509
                       // ((509 & 0x1FF) * 25 + 1000) / 4 = (509 * 25 + 1000) / 4 = (12725 + 1000) / 4 = 13725 / 4 = 3431
        assert_eq!(decode_altitude(&msg), 3431);
    }

    #[test]
    fn decode_altitude_non_q_m_bit_with_digit_fields() {
        // Q-bit clear, M-bit set, some digit fields populated
        // Note: N-bit = Q-bit (both 0x10), so N-bit can never be set without entering Q-mode
        let mut msg = [0u8; 14];
        msg[5] = 0x20 | 0x05; // M-bit + d12=5
        msg[6] = 0b0010_1110; // d10=1, d8=3, d6 low=2
        msg[7] = 0b1011_1100; // d6 high=1, d4=15
                              // d12=5, d10=1, d8=3, d6=4, d4=15, m=1600, n=0
                              // 5*500 + 1*100 + 3*20 + 4*4 + 15 + 1600 = 2500+100+60+16+15+1600 = 4291
        assert_eq!(decode_altitude(&msg), 4291);
    }

    #[test]
    fn decode_altitude_non_q_with_data() {
        // Q-bit clear, all digit fields populated
        let mut msg = [0u8; 14];
        // d12 occupies msg[5] & 0x0F = 0x05 → 5
        // d10 occupies (msg[6] >> 5) & 0x07 → need msg[6] bits 5-7
        // d8  occupies (msg[6] >> 2) & 0x07 → need msg[6] bits 2-4
        // d6  occupies ((msg[6] & 0x03) << 1) | ((msg[7] >> 6) & 0x01)
        // d4  occupies (msg[7] >> 2) & 0x0F
        msg[5] = 0x05; // d12 = 5
        msg[6] = 0b0010_1110; // d10=1, d8=3, d6 low=2
        msg[7] = 0b1011_1100; // d6 high=1, d4=15
                              // d12=5, d10=1, d8=3, d6=((2 << 1) | 0) = 4, d4=15
                              // 5*500 + 1*100 + 3*20 + 4*4 + 15 = 2500+100+60+16+15 = 2691
        assert_eq!(decode_altitude(&msg), 2691);
    }

    #[test]
    fn decode_altitude_non_q_only_d12() {
        // Only d12 set, no M/N bits
        let mut msg = [0u8; 14];
        msg[5] = 0x05;
        // d12=5 → 5*500 = 2500
        assert_eq!(decode_altitude(&msg), 2500);
    }

    #[test]
    fn decode_altitude_non_q_m_bit_only() {
        // Only M-bit set → 1600
        let mut msg = [0u8; 14];
        msg[5] = 0x20; // m_bit only
        assert_eq!(decode_altitude(&msg), 1600);
    }
}
