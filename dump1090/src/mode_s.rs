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
    let mut am = AircraftMessage::default();
    am.icao = mm.addr;
    am.df = mm.msgtype;
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
