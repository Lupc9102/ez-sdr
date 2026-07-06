//! QPSK differential decoding.
//!
//! A 4th-power Costas loop locks phase to within a 4-fold ambiguity (the
//! loop cannot distinguish 0/90/180/270 degree rotations of the QPSK
//! constellation). LRPT resolves this the standard way any CCSDS-derived
//! QPSK downlink does: the transmitter differentially encodes each dibit
//! against the previous one, so the *change* in phase carries the data
//! rather than the absolute phase. Decoding differentially -- comparing
//! each symbol's quadrant against the previous symbol's quadrant -- is
//! invariant to any constant phase rotation applied to the whole stream.
//!
//! The exact quadrant-to-dibit mapping and rotation direction are not
//! nailed down by public specs alone, so this module exposes the mapping
//! as configurable, and correctness is ultimately confirmed empirically by
//! stage 4 (frame sync lock / RS success) as documented in the module-level
//! plan. Four `DiffMapping` variants are provided so the frame-sync stage
//! can try all of them if needed.

use num_complex::Complex32;

/// A dibit: 2 bits, values 0..=3.
pub type Dibit = u8;

/// Slice a phase-tracked QPSK symbol into a quadrant index 0..=3.
///
/// Quadrant numbering follows standard mathematical angle convention,
/// counter-clockwise from the positive real axis:
/// - 0: I>0, Q>0  (0-90 deg)
/// - 1: I<0, Q>0  (90-180 deg)
/// - 2: I<0, Q<0  (180-270 deg)
/// - 3: I>0, Q<0  (270-360 deg)
#[must_use]
pub fn symbol_to_quadrant(sym: Complex32) -> u8 {
    match (sym.re >= 0.0, sym.im >= 0.0) {
        (true, true) => 0,
        (false, true) => 1,
        (false, false) => 2,
        (true, false) => 3,
    }
}

/// Differentially decode a stream of quadrant indices: each output dibit
/// is the (mod-4) difference between consecutive quadrants, which is
/// invariant to any constant rotation offset applied uniformly to the
/// whole stream (i.e. immune to the Costas loop's 4-fold ambiguity).
///
/// The first symbol has no predecessor and is dropped (output has one
/// fewer element than input).
#[must_use]
pub fn differential_decode(quadrants: &[u8]) -> Vec<Dibit> {
    if quadrants.is_empty() {
        return Vec::new();
    }
    quadrants
        .windows(2)
        .map(|w| (w[1] + 4 - w[0]) % 4)
        .collect()
}

/// Differentially *encode* a stream of dibits (inverse of
/// `differential_decode`, given a starting quadrant). Used to build
/// synthetic test/reference signals: `quadrants[i] = (quadrants[i-1] +
/// dibit[i-1]) % 4`.
#[cfg_attr(not(test), allow(dead_code))] // synthetic-signal generation only
#[must_use]
pub fn differential_encode(dibits: &[Dibit], start_quadrant: u8) -> Vec<u8> {
    let mut quadrants = Vec::with_capacity(dibits.len() + 1);
    let mut q = start_quadrant % 4;
    quadrants.push(q);
    for &d in dibits {
        q = (q + d) % 4;
        quadrants.push(q);
    }
    quadrants
}

/// Convention for mapping a 2-bit dibit to a pair of output bits (MSB,LSB
/// order vs LSB,MSB, and any Gray-code swap). Since the true on-air
/// convention cannot be determined from public specs alone with 100%
/// certainty, `frame_sync` tries all variants and keeps whichever locks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DibitMapping {
    /// bit0 = dibit bit1 (MSB), bit1 = dibit bit0 (LSB) -- natural binary.
    Natural,
    /// Gray-coded: swap based on XOR of the two dibit bits.
    Gray,
}

/// Expand a dibit into its two constituent bits (MSB first in the output)
/// according to `mapping`.
#[must_use]
pub fn dibit_to_bits(dibit: Dibit, mapping: DibitMapping) -> [u8; 2] {
    let d = dibit & 0b11;
    match mapping {
        DibitMapping::Natural => [(d >> 1) & 1, d & 1],
        DibitMapping::Gray => {
            // Standard 2-bit Gray code: 00->00, 01->01, 11->10, 10->11
            let gray = d ^ (d >> 1);
            [(gray >> 1) & 1, gray & 1]
        }
    }
}

/// Convert a slice of dibits into a bitstream (MSB-first per dibit).
#[must_use]
pub fn dibits_to_bits(dibits: &[Dibit], mapping: DibitMapping) -> Vec<u8> {
    let mut bits = Vec::with_capacity(dibits.len() * 2);
    for &d in dibits {
        let [b0, b1] = dibit_to_bits(d, mapping);
        bits.push(b0);
        bits.push(b1);
    }
    bits
}

/// Pack a bitstream (MSB-first) into bytes. Any trailing partial byte
/// (fewer than 8 bits remaining) is dropped.
#[cfg_attr(not(test), allow(dead_code))] // synthetic-signal generation / tests only
#[must_use]
pub fn bits_to_bytes(bits: &[u8]) -> Vec<u8> {
    bits.chunks_exact(8)
        .map(|chunk| {
            let mut byte = 0u8;
            for &b in chunk {
                byte = (byte << 1) | (b & 1);
            }
            byte
        })
        .collect()
}

/// Unpack bytes into a MSB-first bitstream.
#[cfg_attr(not(test), allow(dead_code))] // synthetic-signal generation / tests only
#[must_use]
pub fn bytes_to_bits(bytes: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for &byte in bytes {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1);
        }
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrant_slicing_matches_expected_regions() {
        assert_eq!(symbol_to_quadrant(Complex32::new(1.0, 1.0)), 0);
        assert_eq!(symbol_to_quadrant(Complex32::new(-1.0, 1.0)), 1);
        assert_eq!(symbol_to_quadrant(Complex32::new(-1.0, -1.0)), 2);
        assert_eq!(symbol_to_quadrant(Complex32::new(1.0, -1.0)), 3);
    }

    #[test]
    fn differential_encode_decode_round_trip() {
        let dibits = [0u8, 1, 2, 3, 3, 2, 1, 0, 1, 1, 2, 3];
        for start in 0..4 {
            let quadrants = differential_encode(&dibits, start);
            let decoded = differential_decode(&quadrants);
            assert_eq!(decoded, dibits, "start_quadrant={start}");
        }
    }

    #[test]
    fn differential_decode_is_invariant_to_constant_rotation() {
        let dibits = [0u8, 2, 1, 3, 3, 0, 2, 1];
        let base_quadrants = differential_encode(&dibits, 0);
        // Apply a constant rotation (simulating Costas 4-fold ambiguity).
        for rotation in 0..4 {
            let rotated: Vec<u8> = base_quadrants.iter().map(|&q| (q + rotation) % 4).collect();
            let decoded = differential_decode(&rotated);
            assert_eq!(decoded, dibits, "rotation={rotation}");
        }
    }

    #[test]
    fn empty_input_produces_empty_output() {
        assert!(differential_decode(&[]).is_empty());
    }

    #[test]
    fn bits_bytes_round_trip() {
        let original = vec![0xDEu8, 0xAD, 0xBE, 0xEF, 0x12];
        let bits = bytes_to_bits(&original);
        assert_eq!(bits.len(), original.len() * 8);
        let restored = bits_to_bytes(&bits);
        assert_eq!(restored, original);
    }

    #[test]
    fn dibit_natural_mapping_round_trips_all_values() {
        for d in 0..4u8 {
            let bits = dibit_to_bits(d, DibitMapping::Natural);
            let restored = (bits[0] << 1) | bits[1];
            assert_eq!(restored, d);
        }
    }

    #[test]
    fn dibit_gray_mapping_is_distinct_permutation() {
        let mut seen = std::collections::HashSet::new();
        for d in 0..4u8 {
            let bits = dibit_to_bits(d, DibitMapping::Gray);
            let val = (bits[0] << 1) | bits[1];
            seen.insert(val);
        }
        // Gray mapping should still be a bijection over 0..4.
        assert_eq!(seen.len(), 4);
    }
}
