//! Mode S / ADS-B message decoder
//! Translated from `mode_s.c`

use crate::crc::{check_crc, crc24_parity};
use crate::demod::ModesMessage;

/// Decode a raw Mode S message into altitude, callsign, and velocity fields.
///
/// Position is intentionally left unset here: airborne/surface CPR position
/// requires an even/odd frame pair (and, for surface, a reference position),
/// which a single stateless message cannot supply. Callers that need position
/// should pair frames themselves via [`crate::cpr::CprDecoder`].
#[must_use]
pub fn decode_mode_s_message(msg: &[u8]) -> Option<AircraftMessage> {
    if msg.len() < 7 {
        return None;
    }
    let df = extract_df(msg);
    let long_msg = !matches!(df, 0 | 4 | 5 | 11);
    let required_len = if long_msg { 14 } else { 7 };
    if msg.len() < required_len {
        return None;
    }
    let msg = &msg[..required_len];

    // DF11/17/18 carry the ICAO address directly and must pass CRC. Other
    // downlink formats overlay the address on the parity field, so the
    // recovered CRC remainder *is* the address.
    let icao = match df {
        11 | 17 | 18 => (u32::from(msg[1]) << 16) | (u32::from(msg[2]) << 8) | u32::from(msg[3]),
        _ => crc24_parity(msg),
    };
    if matches!(df, 11 | 17 | 18) && !check_crc(msg) {
        return None;
    }

    let mut am = AircraftMessage {
        icao,
        df,
        ..Default::default()
    };

    match df {
        17 | 18 => {
            let tc = msg[4] >> 3;
            match tc {
                1..=4 => {
                    let callsign = decode_callsign(msg);
                    if !callsign.is_empty() {
                        am.callsign = Some(callsign);
                    }
                }
                9..=18 => {
                    am.altitude = Some(decode_altitude(msg));
                }
                19 => {
                    am.velocity = decode_velocity(msg);
                }
                _ => {}
            }
        }
        0 | 4 | 5 | 16 | 20 | 21 => {
            am.altitude = Some(decode_altitude(msg));
        }
        _ => {}
    }

    Some(am)
}

/// Decode the 8-character flight identification (BDS 2,0) carried in an
/// ADS-B identification message (type code 1-4).
fn decode_callsign(msg: &[u8]) -> String {
    let chars = [
        (msg[5] >> 2) & 0x3F,
        ((msg[5] & 0x03) << 4) | (msg[6] >> 4),
        ((msg[6] & 0x0F) << 2) | (msg[7] >> 6),
        msg[7] & 0x3F,
        (msg[8] >> 2) & 0x3F,
        ((msg[8] & 0x03) << 4) | (msg[9] >> 4),
        ((msg[9] & 0x0F) << 2) | (msg[10] >> 6),
        msg[10] & 0x3F,
    ];

    chars
        .iter()
        .map(|&c| match c {
            0 | 32 => ' ',
            1..=26 => (b'A' + c - 1) as char,
            48..=57 => (b'0' + c - 48) as char,
            _ => ' ',
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Decode a barometric altitude field shared by surveillance replies and
/// ADS-B airborne position messages (type codes 9-18).
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

/// Decode ground speed and heading from an airborne velocity message
/// (BDS 0,9, type code 19), subtypes 1 and 2 (ground speed).
fn decode_velocity(msg: &[u8]) -> Option<(f64, f64)> {
    let st = msg[5] & 0x07;
    if st != 1 && st != 2 {
        return None;
    }

    let raw_ew = (u32::from(msg[5]) << 6) | (u32::from(msg[6]) >> 2);
    let ew_dir = if (raw_ew & 1) == 0 { 1.0 } else { -1.0 };
    let ew_vel = f64::from(raw_ew >> 1) - 1.0;

    let raw_ns = (u32::from(msg[6] & 3) << 8) | u32::from(msg[7]);
    let ns_dir = if (raw_ns & 1) == 0 { 1.0 } else { -1.0 };
    let ns_vel = f64::from(raw_ns >> 1) - 1.0;

    let vx = ew_vel * ew_dir;
    let vy = ns_vel * ns_dir;

    let speed_kt = (vx * vx + vy * vy).sqrt();
    let heading = (90.0 - vy.atan2(vx).to_degrees()).rem_euclid(360.0);

    Some((speed_kt, heading))
}

/// Decode a `ModesMessage` produced by the demodulator into an aircraft message.
#[must_use]
pub fn decode_mode_s(mm: &ModesMessage) -> Option<AircraftMessage> {
    if mm.msgbits == 0 {
        return None;
    }
    let am = AircraftMessage {
        icao: mm.addr,
        df: mm.msgtype,
        ..Default::default()
    };
    Some(am)
}

/// Downlink format (DF) extraction
#[must_use]
pub fn extract_df(msg: &[u8]) -> u8 {
    msg[0] >> 3
}

/// Decoded Mode S / ADS-B aircraft message containing ICAO address, downlink format,
/// and optional altitude, callsign, position, and velocity fields.
#[derive(Debug, Clone, Default)]
pub struct AircraftMessage {
    pub icao: u32,
    pub df: u8,
    pub altitude: Option<u32>,
    pub callsign: Option<String>,
    pub position: Option<(f64, f64)>,
    pub velocity: Option<(f64, f64)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_df_from_valid_msg() {
        let msg: [u8; 7] = [0x8E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        // 0x8E >> 3 = 0x11 = 17 (ADSB)
        assert_eq!(extract_df(&msg), 17);
    }

    #[test]
    fn extract_df_from_short_msg_works() {
        let msg: [u8; 3] = [0x05, 0x00, 0x00];
        assert_eq!(extract_df(&msg), 0);
    }

    #[test]
    fn extract_df_all_ones() {
        let msg: [u8; 3] = [0xFF, 0x00, 0x00];
        // 0xFF >> 3 = 31 (DF 31 is valid)
        assert_eq!(extract_df(&msg), 31);
    }

    #[test]
    fn decode_mode_s_zero_bits_returns_none() {
        let mm = ModesMessage {
            msgbits: 0,
            ..Default::default()
        };
        assert!(decode_mode_s(&mm).is_none());
    }

    #[test]
    fn decode_mode_s_with_bits_returns_message() {
        let mm = ModesMessage {
            msgbits: 112,
            msgtype: 17,
            addr: 0xABCDEF,
            ..Default::default()
        };
        let am = decode_mode_s(&mm).expect("should decode");
        assert_eq!(am.icao, 0xABCDEF);
        assert_eq!(am.df, 17);
    }

    #[test]
    fn decode_mode_s_message_stub_returns_none() {
        assert!(decode_mode_s_message(&[0x8E, 0x00, 0x00]).is_none());
    }

    /// A known-valid 112-bit DF17 identification message (type code 4,
    /// callsign "KLM 63EW"), shared with crc.rs's `VALID_112` test vector.
    const VALID_DF17_CALLSIGN: [u8; 14] = [
        0x8D, 0x48, 0x40, 0xD6, 0x20, 0x2C, 0xC3, 0x7C, 0xDB, 0x31, 0x57, 0x9F, 0xE8, 0x02,
    ];

    #[test]
    fn decode_mode_s_message_extracts_icao_and_callsign() {
        let am = decode_mode_s_message(&VALID_DF17_CALLSIGN).expect("valid message decodes");
        assert_eq!(am.df, 17);
        assert_eq!(am.icao, 0x4840D6);
        assert_eq!(am.callsign.as_deref(), Some("KLM 63EW"));
    }

    #[test]
    fn decode_mode_s_message_rejects_bad_crc() {
        let mut bad = VALID_DF17_CALLSIGN;
        bad[2] ^= 0x01;
        assert!(decode_mode_s_message(&bad).is_none());
    }

    #[test]
    fn decode_mode_s_message_too_short_for_df_returns_none() {
        // DF17 requires 14 bytes; only 7 are supplied.
        assert!(decode_mode_s_message(&VALID_DF17_CALLSIGN[..7]).is_none());
    }

    #[test]
    fn decode_mode_s_message_altitude_q_bit_set() {
        let mut msg = [0u8; 7];
        msg[0] = 0x20; // DF4, short surveillance altitude reply
        msg[5] = 0x10;
        msg[6] = 0x00;
        let am = decode_mode_s_message(&msg).expect("should decode");
        assert_eq!(am.df, 4);
        assert_eq!(am.altitude, Some(450));
    }

    #[test]
    fn decode_velocity_ground_speed_subtype() {
        // Subtype 1 (ground speed), east+40kt, north+40kt-ish encoding.
        let mut msg = [0u8; 14];
        msg[0] = 0x8D; // DF17
        msg[4] = 19 << 3; // type code 19
        msg[5] = 0x01; // subtype 1, ew sign bits start here
        let (speed, heading) = decode_velocity(&msg).expect("subtype 1/2 decodes");
        assert!(speed >= 0.0);
        assert!((0.0..360.0).contains(&heading));
    }

    #[test]
    fn decode_velocity_rejects_other_subtypes() {
        let mut msg = [0u8; 14];
        msg[5] = 0x00; // subtype 0 is not ground speed
        assert!(decode_velocity(&msg).is_none());
    }

    #[test]
    fn decode_callsign_maps_charset() {
        let msg = &VALID_DF17_CALLSIGN;
        assert_eq!(decode_callsign(msg), "KLM 63EW");
    }
}
