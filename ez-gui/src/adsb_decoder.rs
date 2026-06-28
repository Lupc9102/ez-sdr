use dump1090::demod::{Demod2400, DemodStats, InputFormat, MagBuf, ModesMessage, compute_magnitude, MAGBUF_DISCONTINUOUS};
use dump1090::mode_s::decode_mode_s_message;
use crate::adsb_panel::AircraftEntry;
use std::collections::HashMap;

pub struct AdsBDecoder {
    demod: Demod2400,
    stats: DemodStats,
    mag_buf: Vec<u16>,
    overlap_buf: Vec<u16>,
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
    pub is_even: bool,
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
            overlap_buf: Vec::new(),
            aircraft: HashMap::new(),
            total_messages: 0,
            frame_count: 0,
        }
    }

    pub fn feed_iq(&mut self, iq: &[u8], sample_rate: u32) {
        let nsamples = iq.len() / 2;
        let overlap = 0;

        self.mag_buf.resize(nsamples.max(131072), 0);

        let (mean_level, mean_power) = compute_magnitude(iq, &mut self.mag_buf[..nsamples], InputFormat::Uc8);

        let mut mag = MagBuf {
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
                            ((msg[5] >> 2) & 0x3F) as u8,
                            (((msg[5] & 0x03) << 4) | (msg[6] >> 4)) as u8,
                            (((msg[6] & 0x0F) << 2) | (msg[7] >> 6)) as u8,
                            (msg[7] & 0x3F) as u8,
                            ((msg[8] >> 2) & 0x3F) as u8,
                            (((msg[8] & 0x03) << 4) | (msg[9] >> 4)) as u8,
                            (((msg[9] & 0x0F) << 2) | (msg[10] >> 6)) as u8,
                            (msg[10] & 0x3F) as u8,
                        ];

                        let callsign: String = chars
                            .iter()
                            .map(|&c| {
                                if c == 0 { return ' '; }
                                if c < 27 { return (b'A' + c - 1) as char; }
                                if c == 32 { return ' '; }
                                if c >= 48 && c <= 57 { return (b'0' + c - 48) as char; }
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

                        let raw_lat = ((msg[5] as u32) << 15)
                            | ((msg[6] as u32) << 7)
                            | ((msg[7] as u32) >> 1);
                        let raw_lon = (((msg[7] & 1) as u32) << 16)
                            | ((msg[8] as u32) << 8)
                            | (msg[9] as u32);
                        let is_even = (msg[6] & 1) == 0;

                        let frame = CprFrame {
                            raw_lat,
                            raw_lon,
                            timestamp: 0.0,
                            is_even,
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
                            let raw_ew = ((msg[5] as u32) << 6) | ((msg[6] as u32) >> 2);
                            let ew_dir = if (raw_ew & 1) == 0 { 1 } else { -1 };
                            let ew_vel = (raw_ew >> 1) - 1;

                            let raw_ns = (((msg[6] & 3) as u32) << 8) | (msg[7] as u32);
                            let ns_dir = if (raw_ns & 1) == 0 { 1 } else { -1 };
                            let ns_vel = (raw_ns >> 1) - 1;

                            let speed_kt = ((ew_vel as f64 * ew_dir as f64).powi(2)
                                + (ns_vel as f64 * ns_dir as f64).powi(2))
                                .sqrt() as u32;
                            entry.speed = Some(speed_kt);
                            entry.entry.speed = speed_kt;

                            let heading = (90.0
                                - (ns_vel as f64 * ns_dir as f64)
                                    .atan2(ew_vel as f64 * ew_dir as f64)
                                    .to_degrees())
                                .rem_euclid(360.0) as u32;
                            entry.heading = Some(heading);
                            entry.entry.heading = heading;
                        }
                    }
                    20..=22 => {
                        // Surface position
                        let raw_lat = ((msg[5] as u32) << 15)
                            | ((msg[6] as u32) << 7)
                            | ((msg[7] as u32) >> 1);
                        let raw_lon = (((msg[7] & 1) as u32) << 16)
                            | ((msg[8] as u32) << 8)
                            | (msg[9] as u32);
                        let is_even = (msg[6] & 1) == 0;

                        let frame = CprFrame {
                            raw_lat,
                            raw_lon,
                            timestamp: 0.0,
                            is_even,
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
        let alt16 = ((msg[5] as u32) << 1) | ((msg[6] as u32) >> 7);
        ((alt16 & 0x1FF) * 25 + 1000) / 4
    } else {
        let m_bit = (msg[5] & 0x20) != 0;
        let n_bit = (msg[5] & 0x10) != 0;
        let d12 = (msg[5] & 0x0F) as u32;
        let d10 = ((msg[6] >> 5) & 0x07) as u32;
        let d8 = ((msg[6] >> 2) & 0x07) as u32;
        let d6 = (((msg[6] & 0x03) << 1) | ((msg[7] >> 6) & 0x01)) as u32;
        let d4 = ((msg[7] >> 2) & 0x0F) as u32;

        let m: u32 = if m_bit { 1600 } else { 0 };
        let n: u32 = if n_bit { 40 } else { 0 };

        d12 * 500 + d10 * 100 + d8 * 20 + d6 * 4 + d4 + m + n
    }
}

fn try_cpr_decode(
    even: &Option<CprFrame>,
    odd: &Option<CprFrame>,
) -> Option<(f64, f64)> {
    let even = even.as_ref()?;
    let odd = odd.as_ref()?;

    let dlat_even = 360.0 / 60.0;
    let dlat_odd = 360.0 / 59.0;

    let j = ((even.raw_lat as f64 / 131072.0 / dlat_even).floor()
        + (odd.raw_lat as f64 / 131072.0 / dlat_odd).floor())
        as i32;

    let r_even = even.raw_lat as f64 / 131072.0;
    let r_odd = odd.raw_lat as f64 / 131072.0;
    let dlat_even_val = dlat_even;
    let dlat_odd_val = dlat_odd;

    let mut lat_even = dlat_even * (r_even - j as f64);
    let mut lat_odd = dlat_odd * (r_odd - j as f64 + if even.raw_lat < odd.raw_lat { 1.0 } else { 0.0 });

    if lat_even >= 270.0 { lat_even -= 360.0; }
    if lat_odd >= 270.0 { lat_odd -= 360.0; }

    let ni_even = if lat_even.abs() >= 87.0 { 1 } else { std::cmp::max(1, (60.0 - even.raw_lat as f64 / 131072.0 / dlat_even_val) as i32) };
    let ni_odd = if lat_odd.abs() >= 87.0 { 1 } else { std::cmp::max(1, (59.0 - odd.raw_lat as f64 / 131072.0 / dlat_odd_val) as i32) };

    let dlon_even = 360.0 / ni_even as f64;
    let dlon_odd = 360.0 / ni_odd as f64;

    let m = ((even.raw_lon as f64 / 131072.0 / dlon_even).floor()
        + (odd.raw_lon as f64 / 131072.0 / dlon_odd).floor()) as i32;

    let mut lon_even = dlon_even * (even.raw_lon as f64 / 131072.0 - m as f64);
    let mut lon_odd = dlon_odd * (odd.raw_lon as f64 / 131072.0 - m as f64 + if even.raw_lon < odd.raw_lon { 1.0 } else { 0.0 });

    if lon_even >= 180.0 { lon_even -= 360.0; }
    if lon_odd >= 180.0 { lon_odd -= 360.0; }

    let age = (even.timestamp - odd.timestamp).abs();
    if age < 10000.0 {
        let lat = (lat_even + lat_odd) / 2.0;
        let lon = (lon_even + lon_odd) / 2.0;
        Some((lat, lon))
    } else {
        None
    }
}
