//! Mode S / ADS-B message decoder
//! Translated from `mode_s.c`

use crate::crc::{check_crc, crc24_parity};
use crate::demod::ModesMessage;

/// Wire identity and payload interpretation for DF18. Namespace bits occupy
/// the high byte of the tracking key, outside the 24-bit address and UAT keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedIdentity {
    pub key: u32,
    pub address_type: &'static str,
    pub source: &'static str,
}

/// Decode CF and the type-specific IMF before interpreting an extended squitter.
/// Coarse TIS-B, management and reserved payloads have no supported ES layout.
#[must_use]
pub fn df18_identity(msg: &[u8]) -> Option<ExtendedIdentity> {
    if msg.len() < 11 || msg[0] >> 3 != 18 {
        return None;
    }
    let address = (u32::from(msg[1]) << 16) | (u32::from(msg[2]) << 8) | u32::from(msg[3]);
    let cf = msg[0] & 7;
    let tc = msg[4] >> 3;
    let imf = match tc {
        5..=8 => msg[6] & 0x08 != 0,
        0 | 9..=18 | 20..=22 => msg[4] & 1 != 0,
        19 => msg[5] & 0x80 != 0,
        28 | 31 => msg[10] & 1 != 0,
        29 => msg[10] & 0x20 != 0,
        _ => false,
    };
    let (namespace, address_type, source) = match cf {
        0 => (0, "icao_non_transponder", "adsb"),
        1 => (0x10, "adsb_other", "adsb"),
        2 if imf => (0x11, "tisb_trackfile", "tisb"),
        2 => (0, "icao", "tisb"),
        5 => (0x12, "tisb_other", "tisb"),
        6 if imf => (0x13, "adsr_other", "adsr"),
        6 => (0, "icao", "adsr"),
        _ => return None,
    };
    Some(ExtendedIdentity {
        key: address | (namespace << 24),
        address_type,
        source,
    })
}

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

    if (df == 11 && crc24_parity(msg) & 0xFFFF80 != 0) || (matches!(df, 17 | 18) && !check_crc(msg))
    {
        return None;
    }

    decode_validated_mode_s_message(msg)
}

/// Extract fields from a message whose integrity was already checked by the
/// demodulator. Socket/file callers should use `decode_mode_s_message` instead.
#[must_use]
pub fn decode_validated_mode_s_message(msg: &[u8]) -> Option<AircraftMessage> {
    let df = extract_df(msg);
    let required_len = if matches!(df, 0 | 4 | 5 | 11) { 7 } else { 14 };
    if msg.len() < required_len {
        return None;
    }
    let msg = &msg[..required_len];
    let icao = if matches!(df, 11 | 17 | 18) {
        (u32::from(msg[1]) << 16) | (u32::from(msg[2]) << 8) | u32::from(msg[3])
    } else {
        crc24_parity(msg)
    };
    let mut am = AircraftMessage {
        icao,
        df,
        address_type: "icao",
        source: if matches!(df, 17 | 18) {
            "adsb"
        } else {
            "mode_s"
        },
        ..Default::default()
    };
    if df == 18 {
        let identity = df18_identity(msg)?;
        am.icao = identity.key;
        am.address_type = identity.address_type;
        am.source = identity.source;
    }

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
                5..=8 => {
                    am.on_ground = Some(true);
                    let movement = (u32::from(msg[4] & 7) << 4) | u32::from(msg[5] >> 4);
                    am.surface_movement = decode_surface_movement(movement);
                    if msg[5] & 0x08 != 0 {
                        let raw = (u32::from(msg[5] & 7) << 4) | u32::from(msg[6] >> 4);
                        // Surface operation status is needed to disambiguate track
                        // from heading. Preserve that uncertainty until supported.
                        am.surface_direction = Some(f64::from(raw) * 360.0 / 128.0);
                    }
                }
                9..=18 | 20..=22 => {
                    am.on_ground = Some(false);
                    am.altitude = decode_altitude(msg, true);
                }
                19 => {
                    am.on_ground = Some(false);
                    am.velocity = decode_velocity(msg);
                    if matches!(msg[4] & 7, 3 | 4) {
                        if msg[5] & 4 != 0 {
                            let raw = (u32::from(msg[5] & 3) << 8) | u32::from(msg[6]);
                            am.magnetic_heading = Some(f64::from(raw) * 360.0 / 1024.0);
                        }
                        let raw = (u32::from(msg[7] & 0x7f) << 3) | u32::from(msg[8] >> 5);
                        if raw != 0 {
                            am.airspeed =
                                Some(f64::from(raw - 1) * if msg[4] & 7 == 4 { 4.0 } else { 1.0 });
                            am.airspeed_true = msg[7] & 0x80 != 0;
                        }
                    }
                    am.vertical_rate = decode_vertical_rate(msg);
                }
                28 if msg[4] & 7 == 1 => {
                    am.emergency = Some(msg[5] >> 5);
                    let id = (u32::from(msg[5] & 0x1F) << 8) | u32::from(msg[6]);
                    if id != 0 {
                        am.squawk = Some(decode_squawk(id));
                    }
                }
                _ => {}
            }
        }
        // DF0/4/16/20 carry an altitude (AC) field. DF5/21 carry a squawk
        // (identity) field in the same bit positions — decoding it as an
        // altitude yields garbage, so they are excluded here.
        0 | 4 | 16 | 20 => {
            am.altitude = decode_altitude(msg, false);
        }
        5 | 21 => {
            let id = (u32::from(msg[2] & 0x1F) << 8) | u32::from(msg[3]);
            if id != 0 {
                am.squawk = Some(decode_squawk(id));
            }
        }
        _ => {}
    }

    Some(am)
}

fn decode_squawk(id: u32) -> u32 {
    let mode_a = crate::mode_ac::decode_id13_field(id);
    ((mode_a >> 12) & 7) * 1000
        + ((mode_a >> 8) & 7) * 100
        + ((mode_a >> 4) & 7) * 10
        + (mode_a & 7)
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

    let s: String = chars
        .iter()
        .map(|&c| match c {
            0 | 32 => ' ',
            1..=26 => (b'A' + c - 1) as char,
            48..=57 => (b'0' + c - 48) as char,
            _ => ' ',
        })
        .collect();
    s.trim().to_string()
}

/// Decode a barometric altitude field shared by surveillance replies and
/// ADS-B airborne position messages (type codes 9-18).
///
/// Bit layout follows the reference `decodeAC12Field`/`decodeAC13Field`:
/// the Q bit selects 25-ft encoding, otherwise the field is Gray-coded
/// Mode C and is resolved via [`crate::mode_ac`].
fn decode_altitude(msg: &[u8], is_df17: bool) -> Option<i32> {
    use crate::mode_ac::{decode_id13_field, mode_a_to_mode_c};
    if is_df17 {
        if msg.len() < 7 {
            return None;
        }
        // AC12 = ME bits 9-20: all of msg[5] plus the top nibble of msg[6].
        let ac12 = (u32::from(msg[5]) << 4) | (u32::from(msg[6]) >> 4);
        let q = (ac12 & 0x10) != 0;
        if q {
            let n = ((ac12 & 0x0FE0) >> 1) | (ac12 & 0x000F);
            let alt = (n as i32 * 25) - 1000;
            Some(alt)
        } else {
            // Gillham Mode C: rebuild the 13-bit field with M=0 inserted,
            // then Gray-decode exactly like the reference.
            let n13 = ((ac12 & 0x0FC0) << 1) | (ac12 & 0x003F);
            let n = mode_a_to_mode_c(decode_id13_field(n13))?;
            if n < -12 {
                return None;
            }
            Some(n * 100)
        }
    } else {
        // Short / long surveillance (DF0, 4, 16, 20): AC field is 13 bits at msg[2..4]
        if msg.len() < 4 {
            return None;
        }
        let ac = (u32::from(msg[2] & 0x1F) << 8) | u32::from(msg[3]);
        let m_bit = (ac & 0x0040) != 0;
        if m_bit {
            // Meters encoding is not implemented (reference: INVALID_ALTITUDE).
            return None;
        }
        let q = (ac & 0x0010) != 0;
        if q {
            let n = ((ac & 0x1F80) >> 2) | ((ac & 0x0020) >> 1) | (ac & 0x000F);
            let alt = (n as i32 * 25) - 1000;
            Some(alt)
        } else {
            // Gillham Mode C via Gray decode (reference `decodeAC13Field`).
            let n = mode_a_to_mode_c(decode_id13_field(ac))?;
            if n < -12 {
                return None;
            }
            Some(n * 100)
        }
    }
}

/// Decode ground speed and heading from an airborne velocity message
/// (BDS 0,9, type code 19), subtypes 1 and 2 (ground speed).
///
/// Bit layout follows the reference `decodeESAirborneVelocity` (ME bit
/// numbering, `me = &msg[4]`): subtype = ME bits 6-8, E/W sign = bit 14,
/// E/W magnitude = bits 15-24, N/S sign = bit 25, N/S magnitude = bits 26-35.
/// A zero magnitude means "not available" — like the reference, velocity is
/// only reported when *both* components are present.
fn decode_velocity(msg: &[u8]) -> Option<(f64, f64)> {
    if msg.len() < 9 {
        return None;
    }
    let st = msg[4] & 0x07;
    if st != 1 && st != 2 {
        return None;
    }

    let ew_raw = (u32::from(msg[5] & 0x03) << 8) | u32::from(msg[6]);
    let ns_raw = (u32::from(msg[7] & 0x7F) << 3) | u32::from(msg[8] >> 5);

    if ew_raw == 0 || ns_raw == 0 {
        return None;
    }

    // Subtype 2 represents supersonic velocity with 4-knot multiplier
    let mul = if st == 2 { 4.0 } else { 1.0 };
    let ew_vel = f64::from(ew_raw - 1) * if (msg[5] & 0x04) == 0 { 1.0 } else { -1.0 } * mul;
    let ns_vel = f64::from(ns_raw - 1) * if (msg[7] & 0x80) == 0 { 1.0 } else { -1.0 } * mul;

    let speed_kt = (ew_vel * ew_vel + ns_vel * ns_vel).sqrt();
    let mut heading = ew_vel.atan2(ns_vel).to_degrees();
    if heading < 0.0 {
        heading += 360.0;
    }

    Some((speed_kt, heading))
}

/// ADS-B v0 surface movement scale, used until operation status supplies a
/// version. Codes 0 and 125..127 mean unavailable, while code 1 means stopped.
fn decode_surface_movement(raw: u32) -> Option<f64> {
    Some(match raw {
        1 => 0.0,
        2..=8 => 0.125 + (f64::from(raw) - 2.0 + 0.5) * 0.125,
        9..=12 => 1.0 + (f64::from(raw) - 9.0 + 0.5) * 0.25,
        13..=38 => 2.0 + (f64::from(raw) - 13.0 + 0.5) * 0.5,
        39..=93 => 15.0 + (f64::from(raw) - 39.0 + 0.5),
        94..=108 => 70.0 + (f64::from(raw) - 94.0 + 0.5) * 2.0,
        109..=123 => 100.0 + (f64::from(raw) - 109.0 + 0.5) * 5.0,
        124 => 180.0,
        _ => return None,
    })
}

/// Decode TC19 vertical rate. The 9-bit field is in 64 ft/min increments with
/// zero reserved for "unavailable"; the sign bit selects climb or descent.
fn decode_vertical_rate(msg: &[u8]) -> Option<i32> {
    if msg.len() < 10 || msg[4] >> 3 != 19 {
        return None;
    }
    let raw = (u16::from(msg[8] & 0x07) << 6) | u16::from(msg[9] >> 2);
    if raw == 0 {
        return None;
    }
    let magnitude = i32::from(raw - 1) * 64;
    Some(if msg[8] & 0x08 == 0 {
        magnitude
    } else {
        -magnitude
    })
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

/// Downlink format (DF) extraction.
///
/// Returns 0 for an empty slice instead of panicking.
#[must_use]
pub fn extract_df(msg: &[u8]) -> u8 {
    msg.first().map_or(0, |b| b >> 3)
}

/// Decoded Mode S / ADS-B aircraft message containing ICAO address, downlink format,
/// and optional altitude, callsign, position, and velocity fields.
#[derive(Debug, Clone, Default)]
pub struct AircraftMessage {
    pub icao: u32,
    pub df: u8,
    pub address_type: &'static str,
    pub source: &'static str,
    pub altitude: Option<i32>,
    pub callsign: Option<String>,
    pub position: Option<(f64, f64)>,
    pub velocity: Option<(f64, f64)>,
    pub magnetic_heading: Option<f64>,
    pub airspeed: Option<f64>,
    pub airspeed_true: bool,
    pub surface_movement: Option<f64>,
    pub surface_direction: Option<f64>,
    pub on_ground: Option<bool>,
    pub vertical_rate: Option<i32>,
    pub emergency: Option<u8>,
    pub squawk: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(mut msg: [u8; 14]) -> [u8; 14] {
        let parity = crate::crc::crc24(&msg[..11]);
        msg[11] = (parity >> 16) as u8;
        msg[12] = (parity >> 8) as u8;
        msg[13] = parity as u8;
        msg
    }

    #[test]
    fn df18_control_fields_and_imf_preserve_address_namespaces() {
        for (cf, expected) in [(0, 0), (1, 0x10), (2, 0), (5, 0x12), (6, 0)] {
            let mut msg = [0; 14];
            msg[..5].copy_from_slice(&[(18 << 3) | cf, 0xAB, 0xCD, 0xEF, 11 << 3]);
            let decoded = decode_mode_s_message(&seal(msg)).unwrap();
            assert_eq!(decoded.icao, (expected << 24) | 0xABCDEF);
            if cf == 2 || cf == 6 {
                // Airborne-position IMF is ME bit 8.
                msg[4] |= 1;
                let non_icao = decode_mode_s_message(&seal(msg)).unwrap();
                assert_eq!(
                    non_icao.icao,
                    ((if cf == 2 { 0x11 } else { 0x13 }) << 24) | 0xABCDEF
                );
                assert_ne!(decoded.address_type, non_icao.address_type);
            }
        }
        for cf in [3, 4, 7] {
            let mut msg = [0; 14];
            msg[0] = (18 << 3) | cf;
            assert!(decode_mode_s_message(&seal(msg)).is_none());
        }
        for cf in [2, 6] {
            for (tc, byte, bit) in [
                (5, 6, 0x08),
                (19, 5, 0x80),
                (28, 10, 1),
                (29, 10, 0x20),
                (31, 10, 1),
            ] {
                let mut msg = [0; 14];
                msg[..4].copy_from_slice(&[(18 << 3) | cf, 0xAB, 0xCD, 0xEF]);
                msg[4] = tc << 3;
                assert_eq!(df18_identity(&msg).unwrap().key, 0xABCDEF);
                msg[byte] |= bit;
                assert_ne!(df18_identity(&msg).unwrap().key, 0xABCDEF);
            }
        }
    }

    #[test]
    fn tc28_emergency_and_squawk_have_separate_fields_from_vertical_rate() {
        for emergency in 0..=7 {
            let mut msg = [0; 14];
            msg[0] = 17 << 3;
            msg[4] = (28 << 3) | 1;
            // ID13 0x0aaa encodes squawk 7700.
            msg[5] = (emergency << 5) | 0x0a;
            msg[6] = 0xaa;
            let decoded = decode_mode_s_message(&seal(msg)).unwrap();
            assert_eq!(decoded.emergency, Some(emergency));
            assert_eq!(decoded.squawk, Some(7700));
            assert_eq!(decoded.vertical_rate, None);
            msg[4] = (28 << 3) | 2;
            assert_eq!(decode_mode_s_message(&seal(msg)).unwrap().emergency, None);
        }
    }

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
    fn extract_df_empty_returns_zero() {
        assert_eq!(extract_df(&[]), 0);
    }

    #[test]
    fn decode_mode_s_message_altitude_q_bit_set() {
        // DF4, AC13 = 0x0BA: M=0, Q set (bit 4), n = 58 → 58*25-1000 = 450 ft.
        let mut msg = [0u8; 7];
        msg[0] = 0x20; // DF4, short surveillance altitude reply
        msg[2] = 0x00;
        msg[3] = 0xBA;
        let am = decode_mode_s_message(&msg).expect("should decode");
        assert_eq!(am.df, 4);
        assert_eq!(am.altitude, Some(450));
    }

    #[test]
    fn decode_mode_s_message_altitude_m_bit_returns_none() {
        // AC13 = 0x1CA has M (bit 6) set → meters encoding, unimplemented.
        // (A previous revision mistook M for Q and returned Some(450).)
        let mut msg = [0u8; 7];
        msg[0] = 0x20; // DF4
        msg[2] = 0x01;
        msg[3] = 0xCA;
        let am = decode_mode_s_message(&msg).expect("should decode");
        assert_eq!(am.altitude, None);
    }

    #[test]
    fn decode_mode_s_message_df5_df21_have_no_altitude() {
        // DF5/DF21 carry squawk (identity), not altitude, in bits 20-32.
        let mut short = [0u8; 7];
        short[0] = 5 << 3; // DF5
        short[2] = 0x01;
        short[3] = 0xCA;
        let am = decode_mode_s_message(&short).expect("should decode");
        assert_eq!(am.df, 5);
        assert_eq!(am.altitude, None);

        let mut long = [0u8; 14];
        long[0] = 21 << 3; // DF21
        long[2] = 0x01;
        long[3] = 0xCA;
        let am = decode_mode_s_message(&long).expect("should decode");
        assert_eq!(am.df, 21);
        assert_eq!(am.altitude, None);
    }

    /// Inverse of [`crate::mode_ac::decode_id13_field`]: Mode A (16-bit
    /// numbering) back to the 13-bit wire field, for building test vectors.
    fn mode_a_to_ac13_field(ma: u32) -> u32 {
        (((ma >> 4) & 1) << 12)
            | (((ma >> 12) & 1) << 11)
            | (((ma >> 5) & 1) << 10)
            | (((ma >> 13) & 1) << 9)
            | (((ma >> 6) & 1) << 8)
            | (((ma >> 14) & 1) << 7)
            | (((ma >> 8) & 1) << 5)
            | ((ma & 1) << 4)
            | (((ma >> 9) & 1) << 3)
            | (((ma >> 1) & 1) << 2)
            | (((ma >> 10) & 1) << 1)
            | ((ma >> 2) & 1)
    }

    #[test]
    fn decode_altitude_gillham_round_trip_short() {
        // Encoder → wire field → Gray decoder must reproduce the altitude.
        for alt_ft in [-1000i32, -500, 0, 1200, 5000, 35000, 126700] {
            let mode_c = alt_ft / 100;
            let ma =
                crate::mode_ac::altitude_100ft_to_squawk(mode_c).expect("encoder should succeed");
            let ac13 = mode_a_to_ac13_field(ma);
            assert_eq!(
                crate::mode_ac::decode_id13_field(ac13),
                ma,
                "field mapping must invert for {alt_ft} ft"
            );
            let mut msg = [0u8; 7];
            msg[0] = 0x20; // DF4
            msg[2] = ((ac13 >> 8) & 0x1F) as u8;
            msg[3] = (ac13 & 0xFF) as u8;
            assert_eq!(
                decode_altitude(&msg, false),
                Some(alt_ft),
                "Gillham decode failed for {alt_ft} ft"
            );
        }
    }

    #[test]
    fn decode_altitude_gillham_round_trip_df17() {
        // DF17 Q=0 path: AC12 → 13-bit field (M=0 inserted) → Gray decode.
        for alt_ft in [-1000i32, -500, 0, 5000, 35000] {
            let mode_c = alt_ft / 100;
            let ma =
                crate::mode_ac::altitude_100ft_to_squawk(mode_c).expect("encoder should succeed");
            let n13 = mode_a_to_ac13_field(ma);
            assert_eq!(n13 & 0x40, 0, "M bit must be clear");
            let ac12 = ((n13 >> 1) & 0x0FC0) | (n13 & 0x003F);
            let mut msg = [0u8; 14];
            msg[0] = 0x8D; // DF17
            msg[4] = 11 << 3; // airborne position
            msg[5] = ((ac12 >> 4) & 0xFF) as u8;
            msg[6] = ((ac12 & 0x0F) << 4) as u8;
            assert_eq!(
                decode_altitude(&msg, true),
                Some(alt_ft),
                "DF17 Gillham decode failed for {alt_ft} ft"
            );
        }
    }

    #[test]
    fn decode_mode_s_df17_high_altitude_35000ft() {
        let mut msg = [0u8; 14];
        msg[0] = 0x8D; // DF17
        msg[4] = 11 << 3; // airborne position
        msg[5] = 0xB5;
        msg[6] = 0x00;
        let alt = decode_altitude(&msg, true);
        assert_eq!(
            alt,
            Some(35000),
            "35,000 ft airliner should decode to 35,000 ft"
        );
    }

    #[test]
    fn decode_velocity_supersonic_subtype() {
        // TC19 subtype 2: E/W raw 101 (mag 100 → 400 kt), N/S raw 51
        // (mag 50 → 200 kt) after the 4× supersonic multiplier.
        let mut msg = [0u8; 14];
        msg[0] = 0x8D; // DF17
        msg[4] = (19 << 3) | 2; // type code 19, subtype 2 (supersonic)
        msg[5] = 0x00; // E/W sign +, mag hi = 0
        msg[6] = 101; // E/W mag lo → raw 101
        msg[7] = 0x06; // N/S sign +, mag hi = 6
        msg[8] = 0x60; // N/S mag lo bits → raw (6<<3)|3 = 51
        let (speed, _) = decode_velocity(&msg).expect("supersonic subtype 2 decodes");
        assert!(
            (440.0..455.0).contains(&speed),
            "supersonic velocity should apply 4x multiplier, got {speed}"
        );
    }

    #[test]
    fn decode_velocity_ground_speed_subtype() {
        // Subtype 1: E/W +400 kt (raw 401), N/S +200 kt (raw 201).
        // speed = sqrt(400² + 200²) ≈ 447.21, heading = atan2(400,200) ≈ 63.43°.
        let mut msg = [0u8; 14];
        msg[0] = 0x8D; // DF17
        msg[4] = (19 << 3) | 1; // type code 19, subtype 1
        msg[5] = 0x01; // E/W sign +, mag hi = 1
        msg[6] = 0x91; // E/W raw = (1<<8)|0x91 = 401
        msg[7] = 0x19; // N/S sign +, mag hi = 25
        msg[8] = 0x20; // N/S raw = (25<<3)|1 = 201
        let (speed, heading) = decode_velocity(&msg).expect("subtype 1 decodes");
        assert!((speed - 447.21).abs() < 0.05, "speed wrong: {speed}");
        assert!((heading - 63.43).abs() < 0.05, "heading wrong: {heading}");
    }

    #[test]
    fn decode_velocity_signs_west_south() {
        // E/W −100 kt (raw 101, sign set), N/S −50 kt (raw 51, sign set).
        // heading = atan2(−100,−50) → ≈ 243.43°.
        let mut msg = [0u8; 14];
        msg[0] = 0x8D;
        msg[4] = (19 << 3) | 1;
        msg[5] = 0x04; // E/W sign −, mag hi = 0
        msg[6] = 101;
        msg[7] = 0x86; // N/S sign −, mag hi = 6
        msg[8] = 0x60; // raw 51
        let (speed, heading) = decode_velocity(&msg).expect("signed velocity decodes");
        assert!((speed - 111.80).abs() < 0.05, "speed wrong: {speed}");
        assert!((heading - 243.43).abs() < 0.05, "heading wrong: {heading}");
    }

    #[test]
    fn decode_velocity_unavailable_component_returns_none() {
        // A zero magnitude means "not available" (reference requires both).
        let mut msg = [0u8; 14];
        msg[0] = 0x8D;
        msg[4] = (19 << 3) | 1;
        msg[5] = 0x00;
        msg[6] = 0x00; // E/W raw 0 → unavailable
        msg[7] = 0x19;
        msg[8] = 0x20; // N/S present
        assert!(decode_velocity(&msg).is_none());

        msg[6] = 101; // E/W present now
        msg[7] = 0x00;
        msg[8] = 0x00; // N/S raw 0 → unavailable
        assert!(decode_velocity(&msg).is_none());
    }

    #[test]
    fn decode_velocity_rejects_other_subtypes() {
        let mut msg = [0u8; 14];
        msg[4] = 19 << 3; // subtype 0 is not ground speed
        assert!(decode_velocity(&msg).is_none());
        msg[4] = (19 << 3) | 3; // subtype 3 is airspeed, not handled here
        assert!(decode_velocity(&msg).is_none());
    }

    #[test]
    fn decode_vertical_rate_handles_climb_descent_and_unavailable() {
        let mut msg = [0u8; 14];
        msg[4] = (19 << 3) | 1;
        // raw=11 => (11-1)*64 = 640 ft/min climb.
        msg[8] = 0;
        msg[9] = 11 << 2;
        assert_eq!(decode_vertical_rate(&msg), Some(640));
        msg[8] |= 0x08;
        assert_eq!(decode_vertical_rate(&msg), Some(-640));
        msg[8] = 0;
        msg[9] = 0;
        assert_eq!(decode_vertical_rate(&msg), None);
    }

    #[test]
    fn decode_callsign_maps_charset() {
        let msg = &VALID_DF17_CALLSIGN;
        assert_eq!(decode_callsign(msg), "KLM 63EW");
    }
}
