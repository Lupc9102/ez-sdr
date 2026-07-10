//! Hand-rolled little-endian binary framing for the high-rate data-plane WebSocket
//! (`crate::web::ws`'s `/ws/stream/{kind}/{id}`).
//!
//! `ServerEvent`'s existing `bincode` framing (`ez_proto::MessageCodec`) is exactly right for
//! the TCP control/data path between Rust processes, but forcing a browser to carry a
//! `bincode` decoder just to unwrap a `Vec<f32>` is unnecessary weight and a versioning
//! liability (bincode's wire format is not a stable public contract the way a hand-documented
//! byte layout is). Each function here instead emits one flat, fixed-then-variable layout a
//! few lines of `DataView` JS can decode directly. `AircraftTelemetry` snapshots are the one
//! exception — small, infrequent, and `Option`-heavy — and travel as plain JSON text frames
//! instead (see `crate::web::ws`), where the ergonomics of `JSON.parse` outweigh the binary
//! framing cost.
//!
//! All multi-byte integers and floats are little-endian. Every layout starts with a small
//! fixed header (metadata a UI needs before it can even size its buffers) followed by one
//! variable-length payload whose element count is itself part of the header.

use ez_proto::{AudioFrame, SpectrumFrame, TelemetryFrame};

/// `SpectrumFrame` wire layout:
///
/// | offset | size | field           |
/// |--------|------|-----------------|
/// | 0      | 8    | center_hz (u64) |
/// | 8      | 4    | sample_rate_hz (u32) |
/// | 12     | 8    | timestamp_ms (u64) |
/// | 20     | 4    | bin_count (u32) |
/// | 24     | 4*N  | bins (f32 * bin_count) |
#[must_use]
pub fn encode_spectrum(frame: &SpectrumFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + frame.bins.len() * 4);
    out.extend_from_slice(&frame.center_hz.to_le_bytes());
    out.extend_from_slice(&frame.sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&frame.timestamp_ms.to_le_bytes());
    out.extend_from_slice(&(frame.bins.len() as u32).to_le_bytes());
    for bin in &frame.bins {
        out.extend_from_slice(&bin.to_le_bytes());
    }
    out
}

/// `AudioFrame` wire layout:
///
/// | offset | size | field                |
/// |--------|------|----------------------|
/// | 0      | 4    | channel_id (u32)     |
/// | 4      | 4    | sample_rate_hz (u32) |
/// | 8      | 4    | sample_count (u32)   |
/// | 12     | 4*N  | samples (f32 * sample_count) |
#[must_use]
pub fn encode_audio(frame: &AudioFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + frame.samples.len() * 4);
    out.extend_from_slice(&frame.channel_id.to_le_bytes());
    out.extend_from_slice(&frame.sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&(frame.samples.len() as u32).to_le_bytes());
    for sample in &frame.samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// `TelemetryFrame` wire layout:
///
/// | offset | size | field              |
/// |--------|------|--------------------|
/// | 0      | 4    | channel_id (u32)   |
/// | 4      | 2    | apid (u16)         |
/// | 6      | 2    | flags (u16): bit0 = costas_locked, bit1 = frame_locked |
/// | 8      | 4    | width (u32)        |
/// | 12     | 4    | height (u32)       |
/// | 16     | 4    | rs_ok (u32)        |
/// | 20     | 4    | rs_failed (u32)    |
/// | 24     | 4    | pixel_count (u32)  |
/// | 28     | N    | pixels (u8 * pixel_count) |
///
/// The two bools are packed into one `u16` (rather than two trailing bytes) purely to keep
/// every field up to `pixel_count` at a 4-byte-aligned offset for the JS side's `DataView`
/// reads; it costs nothing since a `u16` was already the natural size for `apid`'s neighbor.
#[must_use]
pub fn encode_telemetry(frame: &TelemetryFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(28 + frame.pixels.len());
    out.extend_from_slice(&frame.channel_id.to_le_bytes());
    out.extend_from_slice(&frame.apid.to_le_bytes());
    let flags: u16 = (frame.costas_locked as u16) | ((frame.frame_locked as u16) << 1);
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&frame.width.to_le_bytes());
    out.extend_from_slice(&frame.height.to_le_bytes());
    out.extend_from_slice(&frame.rs_ok.to_le_bytes());
    out.extend_from_slice(&frame.rs_failed.to_le_bytes());
    out.extend_from_slice(&(frame.pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(&frame.pixels);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_layout_matches_the_documented_byte_offsets() {
        let frame = SpectrumFrame {
            center_hz: 433_000_000,
            sample_rate_hz: 2_000_000,
            bins: vec![-10.0, -20.5, 3.25],
            timestamp_ms: 123_456_789,
        };
        let bytes = encode_spectrum(&frame);

        assert_eq!(bytes.len(), 24 + 3 * 4);
        assert_eq!(u64::from_le_bytes(bytes[0..8].try_into().unwrap()), 433_000_000);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2_000_000);
        assert_eq!(
            u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            123_456_789
        );
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 3);
        assert_eq!(f32::from_le_bytes(bytes[24..28].try_into().unwrap()), -10.0);
        assert_eq!(f32::from_le_bytes(bytes[28..32].try_into().unwrap()), -20.5);
        assert_eq!(f32::from_le_bytes(bytes[32..36].try_into().unwrap()), 3.25);
    }

    #[test]
    fn audio_layout_matches_the_documented_byte_offsets() {
        let frame = AudioFrame {
            channel_id: 7,
            sample_rate_hz: 48_000,
            samples: vec![0.5, -0.5],
        };
        let bytes = encode_audio(&frame);

        assert_eq!(bytes.len(), 12 + 2 * 4);
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 48_000);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
        assert_eq!(f32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0.5);
        assert_eq!(f32::from_le_bytes(bytes[16..20].try_into().unwrap()), -0.5);
    }

    #[test]
    fn telemetry_layout_matches_the_documented_byte_offsets_and_packs_flags() {
        let frame = TelemetryFrame {
            channel_id: 3,
            apid: 65,
            width: 4,
            height: 2,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
            rs_ok: 10,
            rs_failed: 1,
            costas_locked: true,
            frame_locked: false,
        };
        let bytes = encode_telemetry(&frame);

        assert_eq!(bytes.len(), 28 + 8);
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 65);
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 0b01);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 10);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 8);
        assert_eq!(&bytes[28..36], &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn telemetry_flags_pack_both_bits_when_both_locked() {
        let frame = TelemetryFrame {
            channel_id: 0,
            apid: 0,
            width: 0,
            height: 0,
            pixels: Vec::new(),
            rs_ok: 0,
            rs_failed: 0,
            costas_locked: true,
            frame_locked: true,
        };
        let bytes = encode_telemetry(&frame);
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 0b11);
    }
}
