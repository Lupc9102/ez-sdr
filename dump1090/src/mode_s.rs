//! Mode S / ADS-B message decoder
//! Translated from mode_s.c

use crate::demod::ModesMessage;

/// Decode Mode S message from raw bits
pub fn decode_mode_s_message(msg: &[u8]) -> Option<AircraftMessage> {
    // TODO: translate from legacy mode_s.c
    let _ = msg;
    None
}

/// Decode a `ModesMessage` produced by the demodulator into an aircraft message.
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
pub fn extract_df(msg: &[u8]) -> u8 {
    msg[0] >> 3
}

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
}
