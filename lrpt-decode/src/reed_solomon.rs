//! Reed-Solomon RS(255,223) decoding, 4-way interleaved.
//!
//! Meteor LRPT (per public descriptions of this CCSDS-derived downlink
//! family) protects each transport frame with 4 interleaved RS(255,223)
//! codewords over GF(2^8): the 223 data bytes of each codeword are
//! interleaved byte-by-byte across the frame (codeword 0 takes byte 0,
//! codeword 1 takes byte 1, codeword 2 takes byte 2, codeword 3 takes byte
//! 3, codeword 0 takes byte 4, ...), each corrects up to 16 byte errors,
//! and the 4x32=128 parity bytes are appended after the interleaved data.
//!
//! Uses the MIT-licensed `reed-solomon` crate (mersinvald/reed-solomon-rs)
//! for the actual GF(2^8) codeword encode/decode arithmetic.

use reed_solomon::{Decoder, Encoder};

/// Number of RS codewords interleaved per transport frame.
pub const INTERLEAVE: usize = 4;
/// RS data bytes per codeword.
pub const RS_DATA_LEN: usize = 223;
/// RS parity bytes per codeword.
pub const RS_PARITY_LEN: usize = 32;
/// RS codeword length (data + parity).
pub const RS_CODEWORD_LEN: usize = RS_DATA_LEN + RS_PARITY_LEN; // 255

/// Total transport-frame length this module expects: 4 interleaved
/// codewords of 255 bytes each.
pub const INTERLEAVED_FRAME_LEN: usize = RS_CODEWORD_LEN * INTERLEAVE; // 1020

/// Result of decoding one interleaved frame.
#[derive(Debug, Clone, Default)]
pub struct RsDecodeResult {
    /// Corrected/deinterleaved payload data (4 * 223 = 892 bytes), in
    /// original VCDU byte order, only populated for codewords that decoded
    /// successfully (failed codewords are zero-filled so callers still get
    /// a fixed-size buffer to index into if desired).
    pub data: Vec<u8>,
    /// Count of the 4 codewords that decoded successfully.
    pub codewords_ok: u32,
    /// Count of the 4 codewords that failed (uncorrectable).
    pub codewords_failed: u32,
}

/// Deinterleave `frame` (must be `INTERLEAVED_FRAME_LEN` bytes) into 4
/// separate RS codewords of `RS_CODEWORD_LEN` bytes each.
#[must_use]
pub fn deinterleave(frame: &[u8]) -> [[u8; RS_CODEWORD_LEN]; INTERLEAVE] {
    let mut codewords = [[0u8; RS_CODEWORD_LEN]; INTERLEAVE];
    for (i, &byte) in frame.iter().take(INTERLEAVED_FRAME_LEN).enumerate() {
        let cw = i % INTERLEAVE;
        let pos = i / INTERLEAVE;
        codewords[cw][pos] = byte;
    }
    codewords
}

/// Re-interleave 4 corrected data-only codewords (223 bytes each) back
/// into original byte order, producing `RS_DATA_LEN * INTERLEAVE` bytes.
#[must_use]
pub fn interleave_data(codewords: &[[u8; RS_DATA_LEN]; INTERLEAVE]) -> Vec<u8> {
    let mut out = vec![0u8; RS_DATA_LEN * INTERLEAVE];
    for pos in 0..RS_DATA_LEN {
        for cw in 0..INTERLEAVE {
            out[pos * INTERLEAVE + cw] = codewords[cw][pos];
        }
    }
    out
}

/// Encode 4 interleaved RS(255,223) codewords from `payload`
/// (`RS_DATA_LEN * INTERLEAVE` bytes), returning an interleaved frame of
/// `INTERLEAVED_FRAME_LEN` bytes. Used only by tests/synthetic signal
/// generation to build round-trippable fixtures.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub fn encode_interleaved(payload: &[u8]) -> Vec<u8> {
    assert_eq!(payload.len(), RS_DATA_LEN * INTERLEAVE);
    let encoder = Encoder::new(RS_PARITY_LEN);
    let mut codewords = [[0u8; RS_CODEWORD_LEN]; INTERLEAVE];
    for cw in 0..INTERLEAVE {
        let mut data = [0u8; RS_DATA_LEN];
        for pos in 0..RS_DATA_LEN {
            data[pos] = payload[pos * INTERLEAVE + cw];
        }
        let encoded = encoder.encode(&data);
        codewords[cw][..RS_DATA_LEN].copy_from_slice(&data);
        codewords[cw][RS_DATA_LEN..].copy_from_slice(encoded.ecc());
    }
    let mut frame = vec![0u8; INTERLEAVED_FRAME_LEN];
    for (i, slot) in frame.iter_mut().enumerate() {
        let cw = i % INTERLEAVE;
        let pos = i / INTERLEAVE;
        *slot = codewords[cw][pos];
    }
    frame
}

/// Decode an interleaved transport frame (`INTERLEAVED_FRAME_LEN` bytes),
/// correcting up to 16 byte errors per codeword independently, and
/// re-interleave the corrected payload data back into original order.
#[must_use]
pub fn decode_interleaved(frame: &[u8]) -> RsDecodeResult {
    let codewords = deinterleave(frame);
    let decoder = Decoder::new(RS_PARITY_LEN);
    let mut data_out = [[0u8; RS_DATA_LEN]; INTERLEAVE];
    let mut ok = 0u32;
    let mut failed = 0u32;

    for (cw_idx, codeword) in codewords.iter().enumerate() {
        match decoder.correct(codeword, None) {
            Ok(corrected) => {
                let corrected_data = corrected.data();
                data_out[cw_idx].copy_from_slice(&corrected_data[..RS_DATA_LEN]);
                ok += 1;
            }
            Err(_) => {
                failed += 1;
            }
        }
    }

    RsDecodeResult {
        data: interleave_data(&data_out),
        codewords_ok: ok,
        codewords_failed: failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> Vec<u8> {
        (0..(RS_DATA_LEN * INTERLEAVE))
            .map(|i| ((i * 173 + 29) % 256) as u8)
            .collect()
    }

    #[test]
    fn interleave_deinterleave_round_trip() {
        let frame: Vec<u8> = (0..INTERLEAVED_FRAME_LEN).map(|i| (i % 256) as u8).collect();
        let codewords = deinterleave(&frame);
        for i in 0..INTERLEAVED_FRAME_LEN {
            let cw = i % INTERLEAVE;
            let pos = i / INTERLEAVE;
            assert_eq!(codewords[cw][pos], frame[i]);
        }
    }

    #[test]
    fn encode_decode_round_trip_no_errors() {
        let payload = sample_payload();
        let frame = encode_interleaved(&payload);
        assert_eq!(frame.len(), INTERLEAVED_FRAME_LEN);
        let result = decode_interleaved(&frame);
        assert_eq!(result.codewords_ok, 4);
        assert_eq!(result.codewords_failed, 0);
        assert_eq!(result.data, payload);
    }

    #[test]
    fn corrects_injected_byte_errors_within_capacity() {
        let payload = sample_payload();
        let mut frame = encode_interleaved(&payload);
        // Inject 16 byte errors into codeword 0 (interleaved stride 4,
        // offset 0) -- exactly at RS's correction capacity.
        for k in 0..16 {
            let idx = k * INTERLEAVE; // byte positions belonging to codeword 0
            frame[idx] ^= 0xFF;
        }
        let result = decode_interleaved(&frame);
        assert_eq!(result.codewords_ok, 4);
        assert_eq!(result.codewords_failed, 0);
        assert_eq!(result.data, payload);
    }

    #[test]
    fn reports_failure_when_errors_exceed_capacity() {
        let payload = sample_payload();
        let mut frame = encode_interleaved(&payload);
        // Inject 20 byte errors into codeword 0 -- beyond t=16 capacity.
        for k in 0..20 {
            let idx = k * INTERLEAVE;
            frame[idx] ^= 0xFF;
        }
        let result = decode_interleaved(&frame);
        assert_eq!(result.codewords_failed, 1);
        assert_eq!(result.codewords_ok, 3);
    }

    #[test]
    fn other_codewords_unaffected_by_errors_in_one() {
        let payload = sample_payload();
        let mut frame = encode_interleaved(&payload);
        for k in 0..30 {
            let idx = k * INTERLEAVE + 1; // hammer codeword 1 only
            frame[idx] ^= 0xAA;
        }
        let result = decode_interleaved(&frame);
        assert_eq!(result.codewords_failed, 1);
        assert_eq!(result.codewords_ok, 3);
        // Codewords 0, 2, 3 payload bytes should still match originals.
        for pos in 0..RS_DATA_LEN {
            assert_eq!(result.data[pos * INTERLEAVE], payload[pos * INTERLEAVE]);
            assert_eq!(result.data[pos * INTERLEAVE + 2], payload[pos * INTERLEAVE + 2]);
            assert_eq!(result.data[pos * INTERLEAVE + 3], payload[pos * INTERLEAVE + 3]);
        }
    }
}
