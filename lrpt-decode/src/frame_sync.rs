//! CCSDS CADU attached-sync-marker correlation and frame extraction.
//!
//! Correlates a recovered bitstream against the standard CCSDS attached
//! sync marker `0x1ACFFC1D` (32 bits), tolerating some bit errors (the
//! signal may be noisy even after RS-quality demod). The upstream
//! differential-decode stage already resolves the Costas loop's 4-fold
//! *quadrant* rotation ambiguity (see `diff_decode::differential_decode`,
//! which is invariant to a constant rotation offset by construction), so
//! this module only needs to match the marker in its standard orientation.
//! The remaining ambiguity -- which dibit-to-bits convention the on-air
//! signal actually uses -- is handled one level up, in
//! `LrptDecoder::maybe_swap_mapping`, by falling back to the alternate
//! mapping if this module never achieves lock. `marker_rotations` below is
//! kept as an additional hardening option (unused today, all synthetic and
//! expected real-world cases are covered by the mapping fallback alone) in
//! case real captures ever reveal a residual byte/rotation issue this
//! doesn't already cover.
//!
//! Total CADU length (sync marker + coded transport frame) for this
//! downlink family is commonly documented as 1024 bytes; this module
//! treats the post-marker payload length as a parameter
//! (`transport_frame_len`) so it stays consistent with whatever
//! `reed_solomon::INTERLEAVED_FRAME_LEN` turns out to require --
//! 1024 - 4 (marker bytes) = 1020 bytes, which matches
//! `reed_solomon::INTERLEAVED_FRAME_LEN` (4 * 255 = 1020) exactly.

/// Standard CCSDS CADU attached sync marker (32 bits).
pub const SYNC_MARKER: u32 = 0x1ACF_FC1D;
/// Marker length in bits.
pub const SYNC_MARKER_BITS: usize = 32;
/// Total CADU frame length in bytes (4-byte sync marker + 1020-byte
/// interleaved transport frame), per public descriptions of this downlink
/// family, cross-checked against `reed_solomon::INTERLEAVED_FRAME_LEN`.
pub const CADU_LEN_BYTES: usize = 1024;
/// Length of the transport frame (payload after the sync marker) in bytes.
/// Documents the relationship to `reed_solomon::INTERLEAVED_FRAME_LEN`
/// (checked by `cadu_len_matches_reed_solomon_interleaved_frame_len`); not
/// read outside tests since callers slice `cadu[4..]` directly.
#[allow(dead_code)]
pub const TRANSPORT_FRAME_LEN: usize = CADU_LEN_BYTES - 4;

/// Maximum Hamming distance (bit errors) tolerated when matching the sync
/// marker; a real signal at reasonable SNR should hit near-zero errors,
/// but some slack handles residual noise.
const MAX_MARKER_ERRORS: u32 = 2;

/// Compute the 4 bit-rotations of the 32-bit sync marker. Rotating the
/// marker pattern covers the case where the recovered bitstream is offset
/// by a whole number of dibits/quadrant-rotation relative to the true
/// on-air framing (residual ambiguity that differential decoding alone may
/// not fully pin down depending on which mapping convention was guessed
/// upstream).
#[allow(dead_code)] // reserved hardening hook, see module docs
#[must_use]
pub fn marker_rotations() -> [u32; 4] {
    let mut rotations = [0u32; 4];
    for (i, rotation) in rotations.iter_mut().enumerate() {
        let shift = i * 8; // byte-granularity rotations (2 bits/dibit * 4)
        *rotation = SYNC_MARKER.rotate_left(shift as u32);
    }
    rotations
}

/// Pack a MSB-first bitstream slice of length 32 into a `u32`.
fn bits_to_u32(bits: &[u8]) -> u32 {
    debug_assert_eq!(bits.len(), 32);
    let mut v = 0u32;
    for &b in bits {
        v = (v << 1) | u32::from(b & 1);
    }
    v
}

/// Result of a successful sync search: bit offset in the input stream
/// where the marker begins, and how many bit errors the match had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncMatch {
    pub bit_offset: usize,
    pub errors: u32,
}

/// Search `bits` (MSB-first bitstream) for the best sync marker match at
/// any bit offset, checking all bit positions and returning the
/// lowest-Hamming-distance match at or below `MAX_MARKER_ERRORS`, if any.
#[must_use]
pub fn find_sync(bits: &[u8]) -> Option<SyncMatch> {
    if bits.len() < SYNC_MARKER_BITS {
        return None;
    }
    let mut best: Option<SyncMatch> = None;
    for offset in 0..=(bits.len() - SYNC_MARKER_BITS) {
        let window = bits_to_u32(&bits[offset..offset + SYNC_MARKER_BITS]);
        let errors = (window ^ SYNC_MARKER).count_ones();
        if errors <= MAX_MARKER_ERRORS {
            match best {
                Some(b) if b.errors <= errors => {}
                _ => {
                    best = Some(SyncMatch {
                        bit_offset: offset,
                        errors,
                    })
                }
            }
            if errors == 0 {
                break; // Perfect match, no need to keep scanning.
            }
        }
    }
    best
}

/// Streaming frame synchronizer: accumulates bits, finds the sync marker,
/// and yields fixed-length CADU frames (marker + transport frame),
/// tracking lock state across calls so it can be fed incrementally (live
/// decode) and can reacquire after signal fade.
#[derive(Debug, Default)]
pub struct FrameSync {
    bit_buffer: Vec<u8>,
    locked: bool,
    /// Bit offset within `bit_buffer` where the next CADU is expected once
    /// locked (relative addressing maintained by draining consumed bits).
    consecutive_misses: u32,
}

/// Total bits in one CADU (marker + transport frame).
const CADU_BITS: usize = CADU_LEN_BYTES * 8;

/// Unlocked buffer cap: keep at most this many of the newest bits while
/// searching for sync. Without it a noise-only capture accumulates ~1 Mbit/s
/// (≈1 byte/bit) forever — multi-GB over a long pass — and `find_sync`
/// re-scans the whole buffer per call (quadratic on top).
const MAX_UNLOCKED_BITS: usize = CADU_BITS * 4;

impl FrameSync {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Feed newly-recovered bits (MSB-first, one entry per bit) and drain
    /// out as many complete CADU frames as are now available. Each
    /// returned frame is `CADU_LEN_BYTES` bytes: the 4-byte sync marker
    /// followed by the `TRANSPORT_FRAME_LEN`-byte transport frame.
    pub fn push_bits(&mut self, bits: &[u8]) -> Vec<Vec<u8>> {
        self.bit_buffer.extend_from_slice(bits);
        let mut frames = Vec::new();

        loop {
            if !self.locked {
                match find_sync(&self.bit_buffer) {
                    Some(m) if m.bit_offset + CADU_BITS <= self.bit_buffer.len() => {
                        // Drop everything before the marker.
                        self.bit_buffer.drain(0..m.bit_offset);
                        self.locked = true;
                        self.consecutive_misses = 0;
                    }
                    _ => break, // not enough data yet, or no marker found
                }
            }

            if self.bit_buffer.len() < CADU_BITS {
                break;
            }

            // We're locked and have a full CADU's worth of bits at the
            // front of the buffer. Verify the marker is still there
            // (within tolerance); if not, we've lost lock.
            let marker_bits = &self.bit_buffer[0..SYNC_MARKER_BITS];
            let marker_val = bits_to_u32(marker_bits);
            let errors = (marker_val ^ SYNC_MARKER).count_ones();

            if errors > MAX_MARKER_ERRORS {
                self.consecutive_misses += 1;
                if self.consecutive_misses > 4 {
                    self.locked = false;
                }
                // Slide forward by 1 bit and retry search.
                self.bit_buffer.drain(0..1);
                continue;
            }
            self.consecutive_misses = 0;

            let frame_bits: Vec<u8> = self.bit_buffer.drain(0..CADU_BITS).collect();
            let frame_bytes = bits_to_bytes_msb(&frame_bits);
            frames.push(frame_bytes);
        }

        if !self.locked && self.bit_buffer.len() > MAX_UNLOCKED_BITS {
            // Bound retained noise only after searching this input. Trimming
            // before scanning discards valid early CADUs in large read blocks.
            let drop = self.bit_buffer.len() - MAX_UNLOCKED_BITS;
            self.bit_buffer.drain(0..drop);
        }
        frames
    }
}

/// Pack an arbitrary-length MSB-first bitstream into bytes (length must be
/// a multiple of 8).
fn bits_to_bytes_msb(bits: &[u8]) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_to_bits(v: u32) -> Vec<u8> {
        (0..32).rev().map(|i| ((v >> i) & 1) as u8).collect()
    }

    #[test]
    fn finds_exact_marker_at_known_offset() {
        let mut bits = vec![0u8; 100];
        let marker_bits = u32_to_bits(SYNC_MARKER);
        bits.splice(37..37 + 32, marker_bits);
        let m = find_sync(&bits).expect("should find marker");
        assert_eq!(m.bit_offset, 37);
        assert_eq!(m.errors, 0);
    }

    #[test]
    fn finds_marker_with_a_few_bit_errors() {
        let mut bits = vec![1u8; 80];
        let mut marker_bits = u32_to_bits(SYNC_MARKER);
        // Flip 2 bits to simulate noise.
        marker_bits[3] ^= 1;
        marker_bits[19] ^= 1;
        bits.splice(20..20 + 32, marker_bits);
        let m = find_sync(&bits).expect("should still find marker within tolerance");
        assert_eq!(m.bit_offset, 20);
        assert_eq!(m.errors, 2);
    }

    #[test]
    fn returns_none_when_marker_absent() {
        let bits = vec![0u8; 200];
        assert!(find_sync(&bits).is_none());
    }

    #[test]
    fn frame_sync_extracts_full_cadu_from_stream() {
        let mut fs = FrameSync::new();
        let marker_bits = u32_to_bits(SYNC_MARKER);

        // Build one CADU worth of bits: marker + pseudo-random payload.
        let mut cadu_bits = marker_bits.clone();
        for i in 0..(TRANSPORT_FRAME_LEN * 8) {
            cadu_bits.push(((i * 7 + 3) % 2) as u8);
        }
        assert_eq!(cadu_bits.len(), CADU_BITS);

        // Prepend some junk before the marker, then push two CADUs back to back.
        let mut stream = vec![0u8; 15];
        stream.extend_from_slice(&cadu_bits);
        stream.extend_from_slice(&cadu_bits);

        let frames = fs.push_bits(&stream);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), CADU_LEN_BYTES);
        assert_eq!(frames[0], frames[1]);
        assert!(fs.is_locked());
    }

    #[test]
    fn frame_sync_handles_incremental_feeding() {
        let mut fs = FrameSync::new();
        let marker_bits = u32_to_bits(SYNC_MARKER);
        let mut cadu_bits = marker_bits;
        for i in 0..(TRANSPORT_FRAME_LEN * 8) {
            cadu_bits.push(((i * 3 + 1) % 2) as u8);
        }

        // Feed in small chunks.
        let mut all_frames = Vec::new();
        for chunk in cadu_bits.chunks(97) {
            all_frames.extend(fs.push_bits(chunk));
        }
        assert_eq!(all_frames.len(), 1);
        assert_eq!(all_frames[0].len(), CADU_LEN_BYTES);
    }

    #[test]
    fn marker_rotations_are_distinct() {
        let rotations = marker_rotations();
        let unique: std::collections::HashSet<u32> = rotations.iter().copied().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn unlocked_buffer_stays_bounded_on_noise() {
        // Noise-only input never locks: memory must stay flat (~4 CADUs),
        // not grow ~1 MB/s forever.
        let mut sync = FrameSync::new();
        let noise = vec![0u8; CADU_BITS * 10];
        for _ in 0..20 {
            sync.push_bits(&noise);
        }
        assert!(
            sync.bit_buffer.len() <= MAX_UNLOCKED_BITS,
            "buffer grew to {} bits",
            sync.bit_buffer.len()
        );
        assert!(!sync.is_locked());
    }

    #[test]
    fn cadu_len_matches_reed_solomon_interleaved_frame_len() {
        assert_eq!(
            TRANSPORT_FRAME_LEN,
            crate::reed_solomon::INTERLEAVED_FRAME_LEN
        );
    }
}
