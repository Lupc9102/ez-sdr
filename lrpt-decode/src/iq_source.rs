//! Raw IQ byte <-> `Complex32` conversion.
//!
//! Two source formats are supported:
//! - Raw interleaved unsigned-8-bit I/Q byte pairs (RTL-SDR "Uc8"
//!   convention), decoded by [`iq_bytes_to_complex`] using the exact same
//!   `f32::from(byte) - 127.4` offset convention as
//!   `ez-gui/src/spectrum.rs::push_iq_samples`.
//! - Interleaved little-endian float32 I/Q pairs (the current `.cf32`
//!   recorder format — see
//!   `ez-gui/src/satellite/recorder.rs::raw_iq_bytes_to_cf32_le`), decoded
//!   by [`cf32_le_bytes_to_complex`].

use num_complex::Complex32;

/// Convert raw interleaved u8 IQ bytes to `Complex32` samples.
///
/// Any trailing unpaired byte (odd-length input) is ignored.
#[must_use]
pub fn iq_bytes_to_complex(bytes: &[u8]) -> Vec<Complex32> {
    let n = bytes.len() / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let i_val = f32::from(bytes[2 * i]) - 127.4;
        let q_val = f32::from(bytes[2 * i + 1]) - 127.4;
        out.push(Complex32::new(i_val, q_val));
    }
    out
}

/// Convert LE-float32-interleaved I/Q bytes (current `.cf32` recorder format
/// — see `ez-gui/src/satellite/recorder.rs::raw_iq_bytes_to_cf32_le`) to
/// `Complex32`. 8 bytes per sample pair (4-byte LE f32 I, 4-byte LE f32 Q).
/// A trailing partial pair is ignored.
#[must_use]
pub fn cf32_le_bytes_to_complex(bytes: &[u8]) -> Vec<Complex32> {
    let n = bytes.len() / 8;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let off = i * 8;
        let re = f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let im = f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        out.push(Complex32::new(re, im));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_byte_maps_to_negative_offset() {
        let out = iq_bytes_to_complex(&[0, 0]);
        assert_eq!(out.len(), 1);
        assert!((out[0].re - (-127.4)).abs() < 1e-6);
        assert!((out[0].im - (-127.4)).abs() < 1e-6);
    }

    #[test]
    fn mid_value_maps_near_zero() {
        // 127 -> -0.4, 128 -> 0.6
        let out = iq_bytes_to_complex(&[127, 128]);
        assert!((out[0].re - (-0.4)).abs() < 1e-5);
        assert!((out[0].im - 0.6).abs() < 1e-5);
    }

    #[test]
    fn odd_trailing_byte_is_ignored() {
        let out = iq_bytes_to_complex(&[10, 20, 30]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn multiple_pairs_convert_in_order() {
        let out = iq_bytes_to_complex(&[0, 255, 255, 0]);
        assert_eq!(out.len(), 2);
        assert!((out[0].re - (-127.4)).abs() < 1e-5);
        assert!((out[0].im - 127.6).abs() < 1e-5);
        assert!((out[1].re - 127.6).abs() < 1e-5);
        assert!((out[1].im - (-127.4)).abs() < 1e-5);
    }

    #[test]
    fn cf32_le_zero_bytes_map_to_zero() {
        let out = cf32_le_bytes_to_complex(&[0u8; 8]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].re, 0.0);
        assert_eq!(out[0].im, 0.0);
    }

    #[test]
    fn cf32_le_known_value_round_trip() {
        let re: f32 = 0.25;
        let im: f32 = -0.75;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&re.to_le_bytes());
        bytes.extend_from_slice(&im.to_le_bytes());
        let out = cf32_le_bytes_to_complex(&bytes);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].re, re);
        assert_eq!(out[0].im, im);
    }

    #[test]
    fn cf32_le_trailing_partial_pair_ignored() {
        let re: f32 = 1.0;
        let im: f32 = 2.0;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&re.to_le_bytes());
        bytes.extend_from_slice(&im.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3]); // trailing partial pair
        let out = cf32_le_bytes_to_complex(&bytes);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].re, re);
        assert_eq!(out[0].im, im);
    }
}
