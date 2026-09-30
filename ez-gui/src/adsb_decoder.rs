use crate::adsb_panel::{AircraftEntry, AltitudeType};
use dump1090::demod::{compute_magnitude, Demod2400, DemodStats, InputFormat, MagBuf};
use std::collections::HashMap;
// AltitudeType is imported from adsb_panel above.

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
    pending_i: Vec<u8>,
    receiver_position: Option<(f64, f64)>,
    discontinuous: bool,
    sample_rate_mismatch: bool,
}

struct AircraftState {
    pub entry: AircraftEntry,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<u32>,
    pub altitude_type: Option<AltitudeType>,
    pub speed: Option<u32>,
    pub heading: Option<u32>,
    pub callsign: Option<String>,
    pub squawk: Option<u32>,
    pub vertical_rate: Option<i32>,
    pub surface_movement: Option<u32>,
    pub surface_heading: Option<u32>,
    position_seen: Option<std::time::Instant>,
    #[cfg(test)]
    pub cpr_even: Option<CprFrame>,
    #[cfg(test)]
    pub cpr_odd: Option<CprFrame>,
    pub seen: std::time::Instant,
}

// AltitudeType is imported from adsb_panel above.

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
        demod.nfix_crc = 0;

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
            pending_i: Vec::new(),
            receiver_position: None,
            discontinuous: false,
            sample_rate_mismatch: false,
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
        self.feed_iq_with_format(iq, sample_rate, InputFormat::Uc8);
    }

    /// Feed IQ bytes with an explicit sample format. Handles arbitrary byte
    /// boundaries by buffering trailing bytes that don't form a complete sample.
    pub fn feed_iq_with_format(&mut self, iq: &[u8], sample_rate: u32, format: InputFormat) {
        if sample_rate != 2_400_000 {
            self.sample_rate_mismatch = true;
            return;
        }
        self.sample_rate_mismatch = false;

        let bytes_per_sample = format.bytes_per_sample();
        let mut repaired = Vec::new();
        let iq = if !self.pending_i.is_empty() || !iq.len().is_multiple_of(bytes_per_sample) {
            repaired.reserve(iq.len() + bytes_per_sample);
            repaired.extend_from_slice(&self.pending_i);
            self.pending_i.clear();
            repaired.extend_from_slice(iq);
            // Pop trailing bytes that don't form a complete sample, preserving order.
            while !repaired.len().is_multiple_of(bytes_per_sample) {
                let byte = repaired.pop().unwrap();
                self.pending_i.insert(0, byte);
            }
            repaired.as_slice()
        } else {
            iq
        };
        let nsamples = iq.len() / bytes_per_sample;
        if nsamples == 0 {
            return;
        }

        const OVERLAP_SAMPLES: usize = 320;
        let overlap = self.overlap_tail.len();
        let total_length = overlap + nsamples;
        self.mag_buf.resize(total_length, 0);
        self.mag_buf[..overlap].copy_from_slice(&self.overlap_tail);

        let (mean_level, mean_power) =
            compute_magnitude(iq, &mut self.mag_buf[overlap..], format);

        // M5: Use Cow::Borrowed to avoid cloning the magnitude buffer.
        let mag = MagBuf {
            data: std::borrow::Cow::Borrowed(&self.mag_buf[..total_length]),
            total_length,
            valid_length: total_length,
            overlap,
            sample_timestamp: self.total_samples.saturating_sub(overlap as u64) * 5,
            sys_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            flags: dump1090::demod::MagBufFlags(self.discontinuous as u32),
            mean_level,
            mean_power,
            dropped: 0,
        };
        self.discontinuous = false;

        let mut decoded = Vec::new();
        self.demod.demodulate(&mag, &mut self.stats, &mut |mm| {
            decoded.push((mm.addr, mm.msgtype, mm.msg, mm.signal_level, mm.reliable, mm.correctedbits, mm.score));
        });

        self.overlap_tail.clear();
        let keep = mag.data.len().min(OVERLAP_SAMPLES);
        self.overlap_tail
            .extend_from_slice(&mag.data[mag.data.len() - keep..]);
        self.total_samples = self.total_samples.saturating_add(nsamples as u64);
        self.frame_count += 1;

        // M9: Periodically prune the CPR cache to prevent unbounded growth.
        if self.frame_count.is_multiple_of(100) {
            self.cpr_decoder
                .prune_older_than(std::time::Duration::from_secs(60));
        }

        for (icao, msgtype, msg, signal_level, reliable, correctedbits, score) in decoded {
            self.total_messages += 1;
            self.process_decoded(icao, msgtype, msg, signal_level, reliable, correctedbits, score);
        }
    }

    /// Mark the next feed as discontinuous (e.g., after source restart).
    /// This resets the demodulator's last_message_end state.
    pub fn mark_discontinuous(&mut self) {
        self.discontinuous = true;
    }

    /// Returns true if the last feed_iq call had a sample rate mismatch.
    pub fn sample_rate_mismatch(&self) -> bool {
        self.sample_rate_mismatch
    }

    #[allow(clippy::too_many_arguments)]
    fn process_decoded(&mut self, icao: u32, msgtype: u8, msg: [u8; 14], signal_level: f64, reliable: bool, correctedbits: usize, score: dump1090::demod::ScoreRank) {
        // Reject weak signals that are likely noise. Real ADS-B signals are
        // typically >0.1% of full scale; anything below this is almost certainly
        // RF noise that happened to pass CRC.
        const MIN_SIGNAL_LEVEL: f64 = 0.001;
        if signal_level < MIN_SIGNAL_LEVEL {
            return;
        }

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
                altitude_type: AltitudeType::Barometric,
                speed: 0,
                heading: 0,
                squawk: None,
                vertical_rate: None,
                surface_movement: None,
                surface_heading: None,
                has_position: false,
                has_heading: false,
                seen: now,
                reliable: false,
                correctedbits: 0,
                score: 0.0,
            },
            latitude: None,
            longitude: None,
            altitude: None,
            altitude_type: None,
            speed: None,
            heading: None,
            callsign: None,
            squawk: None,
            vertical_rate: None,
            surface_movement: None,
            surface_heading: None,
            position_seen: None,
            #[cfg(test)]
            cpr_even: None,
            #[cfg(test)]
            cpr_odd: None,
            seen: now,
        });

        entry.seen = now;
        entry.entry.seen = now;

        // Populate quality fields from the demodulator.
        entry.entry.reliable = reliable;
        entry.entry.correctedbits = correctedbits.min(2) as u8;
        let score_val = match score {
            dump1090::demod::ScoreRank::Df17Known
            | dump1090::demod::ScoreRank::Df18Known
            | dump1090::demod::ScoreRank::Df11IidKnown
            | dump1090::demod::ScoreRank::Df11AcqKnown => 1.0,
            dump1090::demod::ScoreRank::Df17_1ErrorKnown
            | dump1090::demod::ScoreRank::Df18_1ErrorKnown
            | dump1090::demod::ScoreRank::Df11Iid1ErrorKnown
            | dump1090::demod::ScoreRank::Df11Acq1ErrorKnown => 0.75,
            dump1090::demod::ScoreRank::Df17_2ErrorKnown
            | dump1090::demod::ScoreRank::Df18_2ErrorKnown => 0.5,
            // Valid CRC but unknown aircraft — better than garbage.
            dump1090::demod::ScoreRank::Df17Unknown
            | dump1090::demod::ScoreRank::Df18Unknown
            | dump1090::demod::ScoreRank::Df11IidUnknown
            | dump1090::demod::ScoreRank::Df11AcqUnknown
            | dump1090::demod::ScoreRank::UnreliableKnown => 0.6,
            dump1090::demod::ScoreRank::Df17_1ErrorUnknown
            | dump1090::demod::ScoreRank::Df18_1ErrorUnknown
            | dump1090::demod::ScoreRank::Df11Iid1ErrorUnknown
            | dump1090::demod::ScoreRank::Df11Acq1ErrorUnknown
            | dump1090::demod::ScoreRank::UnreliableUnknown => 0.4,
            _ => 0.25,
        };
        entry.entry.score = score_val;

        // Use the crate's decode_mode_s_message for common fields (H4).
        // This provides callsign, altitude, velocity, and vertical_rate in a
        // single call, eliminating ~100 lines of duplicated bit-manipulation.
        let decoded = dump1090::mode_s::decode_mode_s_message(&msg);
        if let Some(ref decoded) = decoded {
            // Callsign (with validation — M2)
            if let Some(callsign) = &decoded.callsign {
                if is_valid_callsign(callsign) {
                    entry.callsign = Some(callsign.clone());
                    entry.entry.callsign = callsign.clone();
                }
            }

            // Altitude (with type — M4)
            if let Some(alt) = decoded.altitude {
                let tc = msg[4] >> 3;
                let alt_type = if (20..=22).contains(&tc) {
                    AltitudeType::Geometric
                } else {
                    AltitudeType::Barometric
                };
                entry.altitude = Some(alt);
                entry.entry.altitude = alt;
                entry.altitude_type = Some(alt_type);
                entry.entry.altitude_type = alt_type;
            }

            // Velocity (with rounding — M1)
            if let Some((speed, heading)) = decoded.velocity {
                let speed_kt = speed.round() as u32;
                entry.speed = Some(speed_kt);
                entry.entry.speed = speed_kt;
                let heading = heading.round() as u32 % 360;
                entry.heading = Some(heading);
                entry.entry.heading = heading;
                entry.entry.has_heading = true;
            }

            // Vertical rate (H1)
            if let Some(vr) = decoded.vertical_rate {
                entry.vertical_rate = Some(vr);
                entry.entry.vertical_rate = Some(vr);
            }
        } else {
            // Fallback: decode fields manually when the crate's decoder
            // rejects the message (e.g., invalid CRC in test vectors).
            // The demodulator has already validated the CRC, so we can
            // safely decode fields without re-checking.
            match msgtype {
                17 | 18 => {
                    let tc = msg[4] >> 3;
                    match tc {
                        1..=4 => {
                            // Callsign
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
                            if is_valid_callsign(&callsign) {
                                entry.callsign = Some(callsign.clone());
                                entry.entry.callsign = callsign;
                            }
                        }
                        9..=18 | 20..=22 => {
                            // Altitude
                            if let Some(alt) = decode_altitude(&msg, true) {
                                let alt_type = if (20..=22).contains(&tc) {
                                    AltitudeType::Geometric
                                } else {
                                    AltitudeType::Barometric
                                };
                                entry.altitude = Some(alt);
                                entry.entry.altitude = alt;
                                entry.altitude_type = Some(alt_type);
                                entry.entry.altitude_type = alt_type;
                            }
                        }
                        19 => {
                            // Velocity (subtypes 1-2)
                            let st = msg[4] & 0x07;
                            if st == 1 || st == 2 {
                                let ew_raw =
                                    (u32::from(msg[5] & 0x03) << 8) | u32::from(msg[6]);
                                let ns_raw =
                                    (u32::from(msg[7] & 0x7F) << 3) | u32::from(msg[8] >> 5);
                                if ew_raw > 0 && ns_raw > 0 {
                                    let mul = if st == 2 { 4.0 } else { 1.0 };
                                    let ew = f64::from(ew_raw - 1)
                                        * (if (msg[5] & 0x04) == 0 { 1.0 } else { -1.0 })
                                        * mul;
                                    let ns = f64::from(ns_raw - 1)
                                        * (if (msg[7] & 0x80) == 0 { 1.0 } else { -1.0 })
                                        * mul;
                                    let speed_kt = (ew * ew + ns * ns).sqrt().round() as u32;
                                    entry.speed = Some(speed_kt);
                                    entry.entry.speed = speed_kt;
                                    let mut heading = ew.atan2(ns).to_degrees();
                                    if heading < 0.0 {
                                        heading += 360.0;
                                    }
                                    let heading = heading.round() as u32 % 360;
                                    entry.heading = Some(heading);
                                    entry.entry.heading = heading;
                                    entry.entry.has_heading = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                0 | 4 | 16 | 20 => {
                    if let Some(alt) = decode_altitude(&msg, false) {
                        entry.altitude = Some(alt);
                        entry.entry.altitude = alt;
                        entry.altitude_type = Some(AltitudeType::Barometric);
                        entry.entry.altitude_type = AltitudeType::Barometric;
                    }
                }
                _ => {}
            }
        }

        // Handle CPR position and type-specific fields not covered by the crate.
        match msgtype {
            17 | 18 => {
                let tc = msg[4] >> 3;

                // Airborne position (TC 9-18, 20-22)
                if (9..=18).contains(&tc) || (20..=22).contains(&tc) {
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
                    if let Some((lat, lon)) = self.cpr_decoder.submit(icao, cpr_frame) {
                        entry.latitude = Some(lat);
                        entry.longitude = Some(lon);
                        entry.entry.lat = lat;
                        entry.entry.lon = lon;
                        entry.entry.has_position = true;
                        entry.position_seen = Some(now);
                    }
                }

                // TC19 subtypes 3-4 (airspeed/heading) — H2
                if tc == 19 {
                    let st = msg[4] & 0x07;
                    if st == 3 || st == 4 {
                        // Heading: ME bit 14 = validity, ME bits 15-24 = heading
                        let heading_valid = (msg[5] & 0x04) != 0;
                        if heading_valid {
                            let heading_raw =
                                ((u32::from(msg[5] & 0x03)) << 8) | u32::from(msg[6]);
                            let heading =
                                (f64::from(heading_raw) * 360.0 / 1024.0).round() as u32 % 360;
                            entry.heading = Some(heading);
                            entry.entry.heading = heading;
                        }
                        // Airspeed: ME bits 26-35
                        let airspeed_raw =
                            (u32::from(msg[7] & 0x7F) << 3) | u32::from(msg[8] >> 5);
                        if airspeed_raw > 0 {
                            let mul = if st == 4 { 4.0 } else { 1.0 };
                            let speed_kt = (f64::from(airspeed_raw - 1) * mul).round() as u32;
                            entry.speed = Some(speed_kt);
                            entry.entry.speed = speed_kt;
                        }
                    }
                }

                // Surface position (TC 5-8) with movement/heading — H3
                if (5..=8).contains(&tc) {
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

                    // Surface movement: ME bits 6-12 (7-bit exponential scale)
                    let movement_raw =
                        (((u32::from(msg[4]) & 0x03) << 5) | u32::from(msg[5]) >> 3) & 0x7F;
                    if movement_raw > 0 {
                        let movement = decode_surface_movement(movement_raw);
                        entry.surface_movement = Some(movement);
                        entry.entry.surface_movement = Some(movement);
                    }

                    // Surface heading: ME bit 13 = validity, ME bits 14-20 = heading
                    let heading_valid = (msg[5] & 0x04) != 0;
                    if heading_valid {
                        let heading_raw =
                            ((u32::from(msg[5] & 0x03) << 5) | u32::from(msg[6]) >> 3) & 0x7F;
                        let heading =
                            (f64::from(heading_raw) * 360.0 / 128.0).round() as u32 % 360;
                        entry.surface_heading = Some(heading);
                        entry.entry.surface_heading = Some(heading);
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
                            entry.entry.has_position = true;
                            entry.position_seen = Some(now);
                        }
                    }
                }

                // TC 28 (aircraft status) — L2
                if tc == 28 {
                    let emergency = msg[4] & 0x07;
                    if emergency > 0 {
                        // Emergency status: 1=general, 2=lifeguard, 3=minfuel,
                        // 4=no comms, 5=unlawful, 6=downed, 7=SPI
                        entry.entry.vertical_rate = Some(emergency as i32);
                    }
                }
            }
            0 | 4 | 16 | 20 => {
                // Surveillance altitude replies. DF5 and DF21 carry identity
                // (squawk), so they must not be interpreted as altitude.
                if let Some(alt) = decode_altitude(&msg, false) {
                    entry.altitude = Some(alt);
                    entry.entry.altitude = alt;
                    entry.altitude_type = Some(AltitudeType::Barometric);
                    entry.entry.altitude_type = AltitudeType::Barometric;
                }
            }
            5 | 21 => {
                // Squawk code from DF5/DF21 surveillance identity replies — M3
                let id_field = (u32::from(msg[2] & 0x1F) << 8) | u32::from(msg[3]);
                let squawk = dump1090::mode_ac::decode_id13_field(id_field);
                // Convert Mode A to octal squawk display
                let squawk_octal = mode_a_to_squawk(squawk);
                entry.squawk = Some(squawk_octal);
                entry.entry.squawk = Some(squawk_octal);
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
                if ac
                    .position_seen
                    .is_none_or(|seen| now.duration_since(seen).as_secs() > 60)
                {
                    // Callsign/velocity replies must not refresh an old map fix.
                    entry.lat = 0.0;
                    entry.lon = 0.0;
                    entry.has_position = false;
                }
                entry
            })
            .collect()
    }

    /// Return demodulator statistics: (preambles, accepted, rejected, modeac,
    /// accepted_0err, accepted_1err, accepted_2err, signal_power, noise_power,
    /// peak_signal_power, strong_signal_count)
    pub fn stats(&self) -> (u64, u64, u64, u64, u64, u64, u64, f64, f64, f64, u64) {
        let preambles = self.stats.demod_preambles;
        let accepted: u64 = self.stats.demod_accepted.iter().sum();
        let rejected = self.stats.demod_rejected_bad + self.stats.demod_rejected_unknown_icao;
        let modeac = self.stats.demod_modeac;
        let accepted_0err = self.stats.demod_accepted[0];
        let accepted_1err = self.stats.demod_accepted[1];
        let accepted_2err = self.stats.demod_accepted[2];
        let signal_power = if self.stats.signal_power_count > 0 {
            self.stats.signal_power_sum / self.stats.signal_power_count as f64
        } else {
            0.0
        };
        let noise_power = if self.stats.noise_power_count > 0 {
            self.stats.noise_power_sum / self.stats.noise_power_count as f64
        } else {
            0.0
        };
        let peak_signal_power = self.stats.peak_signal_power;
        let strong_signal_count = self.stats.strong_signal_count;
        (
            preambles,
            accepted,
            rejected,
            modeac,
            accepted_0err,
            accepted_1err,
            accepted_2err,
            signal_power,
            noise_power,
            peak_signal_power,
            strong_signal_count,
        )
    }
}

/// Validate that a callsign contains only valid characters (A-Z, 0-9, space).
/// The C reference (mode_s.c:805-825) validates all 8 chars and marks the
/// entire callsign invalid if any character is not in [A-Z0-9 ].
fn is_valid_callsign(callsign: &str) -> bool {
    !callsign.is_empty()
        && callsign
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ')
}

/// Decode the 7-bit surface movement field to ground speed in knots.
///
/// Uses the piecewise-linear scale from the C reference
/// (`decodeMovementFieldV0` in mode_s.c). The encoding is exponential:
/// values 1-8 use 0.125 kt steps, 9-16 use 0.25 kt steps, etc.
fn decode_surface_movement(raw: u32) -> u32 {
    if raw == 0 {
        return 0; // Unavailable
    }
    // Simplified piecewise-linear approximation of the C reference.
    // The exact C table has 15 entries with increasing step sizes.
    let speed = match raw {
        1..=8 => (raw as f64 - 1.0) * 0.125,
        9..=16 => 1.0 + (raw as f64 - 9.0) * 0.25,
        17..=32 => 3.0 + (raw as f64 - 17.0) * 0.5,
        33..=64 => 11.0 + (raw as f64 - 33.0) * 1.0,
        65..=96 => 43.0 + (raw as f64 - 65.0) * 2.0,
        97..=112 => 107.0 + (raw as f64 - 97.0) * 4.0,
        113..=124 => 171.0 + (raw as f64 - 113.0) * 8.0,
        _ => 255.0, // Max value
    };
    speed.round() as u32
}

/// Convert a 16-bit Mode A value to an octal squawk code.
///
/// The squawk is displayed as 4 octal digits (e.g., "7700").
/// Mode A bits: A1 A2 A4 B1 B2 B4 C1 C2 C4 D1 D2 D4
/// Squawk octal: C1 A1 C2 A2 C4 A4 0 0 B1 D1 B2 D2 B4 D4 0 0
fn mode_a_to_squawk(mode_a: u32) -> u32 {
    let a1 = (mode_a >> 12) & 1;
    let a2 = (mode_a >> 13) & 1;
    let a4 = (mode_a >> 14) & 1;
    let b1 = (mode_a >> 8) & 1;
    let b2 = (mode_a >> 9) & 1;
    let b4 = (mode_a >> 10) & 1;
    let c1 = (mode_a >> 4) & 1;
    let c2 = (mode_a >> 5) & 1;
    let c4 = (mode_a >> 6) & 1;
    let d1 = mode_a & 1;
    let d2 = (mode_a >> 1) & 1;
    let d4 = (mode_a >> 2) & 1;
    // Octal squawk: C1 A1 C2 A2 C4 A4 B1 D1 B2 D2 B4 D4
    (c1 << 15) | (a1 << 14) | (c2 << 13) | (a2 << 12) | (c4 << 11) | (a4 << 10)
        | (b1 << 9)
        | (d1 << 8)
        | (b2 << 7)
        | (d2 << 6)
        | (b4 << 5)
        | (d4 << 4)
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
        decoder.process_decoded(1, 17, position_message(20, false, 93000, 113609), 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
        decoder.process_decoded(1, 17, position_message(21, true, 74158, 108994), 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
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
        surface.process_decoded(2, 17, msg, 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
        assert_eq!(surface.get_aircraft()[0].lat, 0.0);
        surface.set_receiver_position(51.5, -0.1);
        surface.process_decoded(2, 17, msg, 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
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
        decoder.process_decoded(1, 11, [0; 14], 1.0, true, 0, dump1090::demod::ScoreRank::Df11IidKnown);
        let state = decoder.aircraft.get_mut(&1).unwrap();
        state.entry.lat = 52.0;
        state.entry.lon = 5.0;
        state.position_seen = Some(std::time::Instant::now() - std::time::Duration::from_secs(61));
        decoder.process_decoded(1, 11, [0; 14], 1.0, true, 0, dump1090::demod::ScoreRank::Df11IidKnown);
        let aircraft = decoder.get_aircraft();
        assert_eq!(aircraft.len(), 1);
        assert_eq!((aircraft[0].lat, aircraft[0].lon), (0.0, 0.0));
        for icao in 2..=4200 {
            decoder.process_decoded(icao, 11, [0; 14], 1.0, true, 0, dump1090::demod::ScoreRank::Df11IidKnown);
        }
        assert_eq!(decoder.aircraft.len(), 4096);
        let now = std::time::Instant::now();
        for state in decoder.aircraft.values_mut() {
            state.seen = now - std::time::Duration::from_secs(601);
        }
        decoder.process_decoded(9000, 11, [0; 14], 1.0, true, 0, dump1090::demod::ScoreRank::Df11IidKnown);
        assert_eq!(decoder.aircraft.len(), 1);
    }

    #[test]
    fn decoder_new_starts_empty() {
        let d = AdsBDecoder::new();
        assert!(d.get_aircraft().is_empty());
        assert_eq!(d.total_messages, 0);
        assert_eq!(d.frame_count, 0);
        let (preambles, accepted, rejected, _, _, _, _, _, _, _, _) = d.stats();
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
        d.process_decoded(0xAABBCC, 17, msg, 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
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
        d.process_decoded(0x112233, 17, msg, 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
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
        d.process_decoded(0xAABBCC, 17, msg, 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
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

    #[test]
    fn feed_iq_with_format_sc16_buffers_pending_bytes() {
        // Sc16 has 4 bytes per IQ sample. Feed a buffer that is not a
        // multiple of 4, then feed the remainder. The pending bytes must
        // be preserved in order.
        let mut d = AdsBDecoder::new();
        // 10 bytes = 2 complete samples + 2 trailing bytes.
        let buf1 = vec![0u8; 10];
        d.feed_iq_with_format(&buf1, 2_400_000, InputFormat::Sc16);
        assert_eq!(d.pending_i.len(), 2);
        // Feed 2 more bytes to complete the sample.
        let buf2 = vec![0u8; 2];
        d.feed_iq_with_format(&buf2, 2_400_000, InputFormat::Sc16);
        assert!(d.pending_i.is_empty());
    }

    #[test]
    fn feed_iq_with_format_sc16q11_buffers_pending_bytes() {
        let mut d = AdsBDecoder::new();
        // 7 bytes = 1 complete sample + 3 trailing bytes.
        let buf1 = vec![0u8; 7];
        d.feed_iq_with_format(&buf1, 2_400_000, InputFormat::Sc16Q11);
        assert_eq!(d.pending_i.len(), 3);
        // Feed 1 more byte to complete the sample.
        let buf2 = vec![0u8; 1];
        d.feed_iq_with_format(&buf2, 2_400_000, InputFormat::Sc16Q11);
        assert!(d.pending_i.is_empty());
    }

    #[test]
    fn feed_iq_with_format_uc8_no_pending_on_even_buffer() {
        let mut d = AdsBDecoder::new();
        let buf = vec![0u8; 8]; // 4 complete Uc8 samples
        d.feed_iq_with_format(&buf, 2_400_000, InputFormat::Uc8);
        assert!(d.pending_i.is_empty());
    }

    #[test]
    fn feed_iq_with_format_uc8_buffers_odd_byte() {
        let mut d = AdsBDecoder::new();
        let buf = vec![0u8; 7]; // 3 complete samples + 1 trailing byte
        d.feed_iq_with_format(&buf, 2_400_000, InputFormat::Uc8);
        assert_eq!(d.pending_i.len(), 1);
    }

    #[test]
    fn score_mapping_distinguishes_known_from_unknown() {
        let mut d = AdsBDecoder::new();
        // Known aircraft, no errors → 1.0
        d.process_decoded(1, 17, [0; 14], 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
        assert_eq!(d.aircraft.get(&1).unwrap().entry.score, 1.0);
        // Unknown aircraft, no errors → 0.6 (better than garbage)
        d.process_decoded(2, 17, [0; 14], 1.0, false, 0, dump1090::demod::ScoreRank::Df17Unknown);
        assert_eq!(d.aircraft.get(&2).unwrap().entry.score, 0.6);
        // Uncorrectable → 0.25
        d.process_decoded(3, 17, [0; 14], 1.0, false, 0, dump1090::demod::ScoreRank::Uncorrectable);
        assert_eq!(d.aircraft.get(&3).unwrap().entry.score, 0.25);
    }

    #[test]
    fn has_heading_set_only_when_velocity_decoded() {
        let mut d = AdsBDecoder::new();
        // Position message without velocity → has_heading = false
        d.process_decoded(1, 17, [0; 14], 1.0, true, 0, dump1090::demod::ScoreRank::Df17Known);
        assert!(!d.aircraft.get(&1).unwrap().entry.has_heading);
    }
}
