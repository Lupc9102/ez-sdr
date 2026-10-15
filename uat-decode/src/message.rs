//! Public decoded UAT message and ADS-B state types.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageKind {
    DownlinkShort,
    DownlinkLong,
    Uplink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AirGroundState {
    Airborne,
    Supersonic,
    Ground,
    Reserved,
    Invalid,
}

/// Decoded state carried by the ADS-B payload, when a downlink is an ADS-B message.
#[derive(Clone, Debug, PartialEq)]
pub struct AdsbReport {
    /// Address qualifier (0 through 8, as defined by DO-282B / dump978).
    pub address_qualifier: u8,
    pub address: u32,
    pub callsign: Option<String>,
    pub flight_plan_id: Option<String>,
    pub position: Option<(f64, f64)>,
    pub pressure_altitude_ft: Option<i32>,
    pub geometric_altitude_ft: Option<i32>,
    pub north_velocity_kt: Option<i32>,
    pub east_velocity_kt: Option<i32>,
    pub vertical_velocity_barometric_fpm: Option<i32>,
    pub vertical_velocity_geometric_fpm: Option<i32>,
    pub ground_speed_kt: Option<f64>,
    pub true_track_deg: Option<f64>,
    pub magnetic_heading_deg: Option<f64>,
    pub true_heading_deg: Option<f64>,
    pub air_ground_state: Option<AirGroundState>,
}

/// A successfully synchronized and FEC-corrected UAT frame.
#[derive(Clone, Debug, PartialEq)]
pub struct UatMessage {
    pub kind: MessageKind,
    /// Corrected payload with parity removed: 18, 34, or 432 bytes.
    pub payload: Vec<u8>,
    pub corrected_symbols: usize,
    /// Absolute index of the first sample of the sync word in the input stream.
    pub first_sample: u64,
    /// Mean squared magnitude in dBFS over the received frame samples.
    pub rssi_dbfs: f32,
    pub adsb: Option<AdsbReport>,
}

fn bits(bytes: &[u8], first: usize, last: usize) -> u32 {
    let mut result = 0;
    for bit in first..=last {
        result = (result << 1) | u32::from((bytes[(bit - 1) / 8] >> (7 - ((bit - 1) % 8))) & 1);
    }
    result
}

fn one(bytes: &[u8], bit: usize) -> bool {
    bits(bytes, bit, bit) != 0
}

fn rounded(value: f64, digits: f64) -> f64 {
    (value * digits).round() / digits
}

pub(crate) fn decode_adsb(payload: &[u8]) -> Option<AdsbReport> {
    if payload.len() != 18 && payload.len() != 34 {
        return None;
    }
    let payload_type = bits(payload, 1, 5);
    let qualifier = bits(payload, 6, 8) as u8;
    let address = bits(payload, 9, 32);
    let mut report = AdsbReport {
        address_qualifier: qualifier,
        address,
        callsign: None,
        flight_plan_id: None,
        position: None,
        pressure_altitude_ft: None,
        geometric_altitude_ft: None,
        north_velocity_kt: None,
        east_velocity_kt: None,
        vertical_velocity_barometric_fpm: None,
        vertical_velocity_geometric_fpm: None,
        ground_speed_kt: None,
        true_track_deg: None,
        magnetic_heading_deg: None,
        true_heading_deg: None,
        air_ground_state: None,
    };

    // ADS-B payload-type table 2-10: payload types 0..10 include the SV part.
    if payload_type > 10 {
        return Some(report);
    }
    let raw_lat = bits(payload, 33, 55);
    let raw_lon = bits(payload, 56, 78);
    let raw_alt = bits(payload, 81, 92);
    if raw_alt != 0 {
        let altitude = (raw_alt as i32 - 41) * 25;
        if one(payload, 80) {
            report.geometric_altitude_ft = Some(altitude);
        } else {
            report.pressure_altitude_ft = Some(altitude);
        }
    }
    let nic = bits(payload, 93, 96);
    if raw_lat != 0 || raw_lon != 0 || nic != 0 {
        let mut lat = raw_lat as f64 * 360.0 / 16_777_216.0;
        if lat > 90.0 {
            lat -= 180.0;
        }
        let mut lon = raw_lon as f64 * 360.0 / 16_777_216.0;
        if lon > 180.0 {
            lon -= 360.0;
        }
        report.position = Some((rounded(lat, 100_000.0), rounded(lon, 100_000.0)));
    }
    let state = bits(payload, 97, 98);
    report.air_ground_state = Some(match state {
        0 => AirGroundState::Airborne,
        1 => AirGroundState::Supersonic,
        2 => AirGroundState::Ground,
        _ => AirGroundState::Reserved,
    });

    match state {
        0 | 1 => {
            let scale = if state == 1 { 4 } else { 1 };
            let raw_ns = bits(payload, 101, 110);
            if raw_ns != 0 {
                let sign = if one(payload, 100) { -1 } else { 1 };
                report.north_velocity_kt = Some(sign * (raw_ns as i32 - 1) * scale);
            }
            let raw_ew = bits(payload, 112, 121);
            if raw_ew != 0 {
                let sign = if one(payload, 111) { -1 } else { 1 };
                report.east_velocity_kt = Some(sign * (raw_ew as i32 - 1) * scale);
            }
            if let (Some(north), Some(east)) = (report.north_velocity_kt, report.east_velocity_kt) {
                report.ground_speed_kt =
                    Some(rounded(((north * north + east * east) as f64).sqrt(), 10.0));
                let mut track = (east as f64).atan2(north as f64).to_degrees();
                if track < 0.0 {
                    track += 360.0;
                }
                report.true_track_deg = Some(rounded(track, 10.0));
            }
            let vv_source = bits(payload, 122, 122);
            let raw_vv = bits(payload, 124, 132);
            if raw_vv != 0 {
                let sign = if one(payload, 123) { -1 } else { 1 };
                let velocity = sign * (raw_vv as i32 - 1) * 64;
                if vv_source == 0 {
                    report.vertical_velocity_barometric_fpm = Some(velocity);
                } else {
                    report.vertical_velocity_geometric_fpm = Some(velocity);
                }
            }
        }
        2 => {
            let raw_speed = bits(payload, 101, 110);
            if raw_speed != 0 {
                report.ground_speed_kt = Some((raw_speed - 1) as f64);
            }
            let heading_type = bits(payload, 111, 112);
            let angle = rounded(bits(payload, 113, 121) as f64 * 360.0 / 512.0, 10.0);
            match heading_type {
                1 => report.true_track_deg = Some(angle),
                2 => report.magnetic_heading_deg = Some(angle),
                3 => report.true_heading_deg = Some(angle),
                _ => {}
            }
        }
        _ => {}
    }

    if payload_type == 1 || payload_type == 3 {
        let raw1 = bits(payload, 137, 152);
        let raw2 = bits(payload, 153, 168);
        let raw3 = bits(payload, 169, 184);
        let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ *??";
        let indexes = [
            (raw1 / 40) % 40,
            raw1 % 40,
            (raw2 / 1600) % 40,
            (raw2 / 40) % 40,
            raw2 % 40,
            (raw3 / 1600) % 40,
            (raw3 / 40) % 40,
            raw3 % 40,
        ];
        let mut text = indexes
            .iter()
            .map(|i| alphabet[*i as usize] as char)
            .collect::<String>();
        while text.ends_with(' ') || text.ends_with('*') {
            text.pop();
        }
        if !text.is_empty() {
            if one(payload, 215) {
                report.callsign = Some(text);
            } else if text.len() == 4 && text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                report.flight_plan_id = Some(text);
            }
        }
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_downlink_lengths() {
        assert!(decode_adsb(&[0; 432]).is_none());
    }

    #[test]
    fn decodes_header_and_ground_state_from_basic_payload() {
        let mut payload = [0u8; 18];
        payload[0] = 0x12; // payload type 2, ICAO qualifier
        payload[1] = 0x12;
        payload[2] = 0x34;
        payload[3] = 0x56;
        payload[12] = 0x80; // on-ground air/ground state
        let report = decode_adsb(&payload).unwrap();
        assert_eq!(report.address, 0x123456);
        assert_eq!(report.address_qualifier, 2);
        assert_eq!(report.air_ground_state, Some(AirGroundState::Ground));
    }
}
