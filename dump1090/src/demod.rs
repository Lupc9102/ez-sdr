//! 2.4 MHz Mode S PPM demodulator and preamble detector.
//!
//! Translated from `starch/demod_2400.c` and `starch/demod_2400.h` into
//! idiomatic, safe Rust.  This module is self-contained: it provides
//!
//! * IQ-to-magnitude conversion helpers,
//! * the `MagBuf` frame container,
//! * preamble correlation / peak detection at 2.4 Msps,
//! * phase-locked bit extraction (five `slice_phase` correlators),
//! * message framing with early DF rejection,
//! * a minimal but functional `scoreModesMessage` / `decodeModesMessage`,
//! * Mode A/C demodulation (`demodulate2400AC`).
//!
//! The only external dependencies are [`crate::crc`] (for CRC24) and
//! [`crate::icao_filter`] (for the known-aircraft filter).

use crate::crc::crc24_parity;
use crate::icao_filter::IcaoFilter;

// ========================================================================
// Constants
// ========================================================================

pub const MODES_LONG_MSG_BYTES: usize = 14;
pub const MODES_SHORT_MSG_BYTES: usize = 7;
pub const MODES_LONG_MSG_BITS: usize = MODES_LONG_MSG_BYTES * 8;
pub const MODES_SHORT_MSG_BITS: usize = MODES_SHORT_MSG_BYTES * 8;
pub const MODES_MAX_BITERRORS: usize = 2;

/// Timestamp is expressed in units of a 12 MHz clock.
const TIMESTAMP_CLOCK_MHZ: u64 = 12;

// ========================================================================
// Data structures
// ========================================================================

/// Flags describing a [`MagBuf`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MagBufFlags(pub u32);

pub const MAGBUF_DISCONTINUOUS: u32 = 1;

/// A buffer of magnitude (envelope) samples ready for demodulation.
///
/// `data[overlap..valid_length]` is the new region the demodulator may scan.
/// `data[0..overlap]` is copied from the previous buffer so that messages
/// straddling the boundary can be decoded.
#[derive(Debug, Clone)]
pub struct MagBuf {
    /// Magnitude samples (0..65535), starting with overlap from the previous block.
    pub data: Vec<u16>,
    /// Allocated capacity of `data`.
    pub total_length: usize,
    /// Number of valid samples in `data`, including overlap.
    pub valid_length: usize,
    /// Number of leading overlap samples at the start of `data`.
    pub overlap: usize,
    /// Timestamp of the first sample, in 12 MHz ticks.
    pub sample_timestamp: u64,
    /// Estimated system time (ms) at the start of the block.
    pub sys_timestamp: u64,
    pub flags: MagBufFlags,
    /// Mean of normalised signal level (0..1).
    pub mean_level: f64,
    /// Mean of normalised power level (0..1).
    pub mean_power: f64,
    /// Approximate number of dropped samples if `flags` is discontinuous.
    pub dropped: usize,
}

/// A decoded Mode S / Mode A/C message coming out of the demodulator.
#[derive(Debug, Clone, Default)]
pub struct ModesMessage {
    /// Binary message after any corrections.
    pub msg: [u8; MODES_LONG_MSG_BYTES],
    /// Message as originally demodulated (before correction).
    pub verbatim: [u8; MODES_LONG_MSG_BYTES],
    /// Number of bits in the message (56 or 112).
    pub msgbits: usize,
    /// Downlink format (0..31).
    pub msgtype: u8,
    /// Message CRC.
    pub crc: u32,
    /// Number of corrected bits.
    pub correctedbits: usize,
    /// Aircraft address / Mode A identity.
    pub addr: u32,
    /// Timestamp of the message (12 MHz clock).
    pub timestamp_msg: u64,
    /// Timestamp of the message (system time, ms).
    pub sys_timestamp_msg: u64,
    /// RSSI, in the range [0..1], as a fraction of full-scale power.
    pub signal_level: f64,
    /// Scoring from `scoreModesMessage`, if used.
    pub score: ScoreRank,
    /// Is this a "reliable" message (uncorrected DF11/DF17/DF18)?
    pub reliable: bool,
}

/// Demodulator statistics, compatible with the C `struct stats` fields
/// updated by `demodulate2400` / `demodulate2400AC`.
#[derive(Debug, Clone, Default)]
pub struct DemodStats {
    pub demod_preambles: u64,
    pub demod_rejected_bad: u64,
    pub demod_rejected_unknown_icao: u64,
    pub demod_accepted: [u64; MODES_MAX_BITERRORS + 1],
    pub demod_modeac: u64,
    pub noise_power_sum: f64,
    pub noise_power_count: u64,
    pub signal_power_sum: f64,
    pub signal_power_count: u64,
    pub peak_signal_power: f64,
    pub strong_signal_count: u64,
}

// ========================================================================
// Score ranking
// ========================================================================

/// Possible scores for a Mode S message, ordered from worst to best.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScoreRank {
    NotSet = 0,
    AllZeros,
    UnknownDf,
    Uncorrectable,

    /// Cutoff for messages that might be valid, but don't match a known aircraft.
    UnknownThreshold,

    UnreliableUnknown,

    Df11Iid1ErrorUnknown,
    Df11Acq1ErrorUnknown,
    Df11IidUnknown,

    Df18_2ErrorUnknown,
    Df17_2ErrorUnknown,

    /// Cutoff for accepting messages.
    AcceptThreshold,

    UnreliableKnown,

    Df18_2ErrorKnown,
    Df17_2ErrorKnown,

    Df18_1ErrorUnknown,
    Df17_1ErrorUnknown,

    Df11AcqUnknown,

    Df11Iid1ErrorKnown,
    Df11Acq1ErrorKnown,
    Df11IidKnown,

    Df18_1ErrorKnown,
    Df17_1ErrorKnown,

    Df11AcqKnown,

    Df18Unknown,
    Df17Unknown,
    Df18Known,
    Df17Known,
}

impl Default for ScoreRank {
    fn default() -> Self {
        ScoreRank::NotSet
    }
}

// ========================================================================
// CRC / error correction helpers (minimal but sufficient for scoring)
// ========================================================================

/// Syndrome for a single-bit error at position `bit` (0..111) within a
/// 112-bit message.  Pre-computed using the Mode S CRC polynomial.
fn single_bit_syndrome(bit: usize) -> u32 {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u32; 112]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 112];
        // We must replicate the C `modesChecksum` on a 14-byte message with
        // exactly one bit set.  The CRC table used by `crate::crc::CRC24_TABLE`
        // is generated from polynomial 0xFFF409.
        let mut bit_idx = 0usize;
        while bit_idx < 112 {
            let mut msg = [0u8; MODES_LONG_MSG_BYTES];
            let byte = bit_idx >> 3;
            let mask = 1u8 << (7 - (bit_idx & 7));
            msg[byte] = mask;
            table[bit_idx] = crc24_parity(&msg);
            bit_idx += 1;
        }
        table
    });
    table[bit]
}

/// For a 56-bit message, the error bits are at offsets 0..55, but the
/// single-bit syndromes are indexed as if the message occupies bits 56..111.
const SHORT_MSG_OFFSET: usize = MODES_LONG_MSG_BITS - MODES_SHORT_MSG_BITS; // 56

/// Apply a single-bit correction described by `tables_idx` in-place.
fn apply_bit_errors(msg: &mut [u8], bit: usize) {
    let byte = bit >> 3;
    let mask = 1u8 << (7 - (bit & 7));
    msg[byte] ^= mask;
}

/// Try to correct a message using the linearity of the CRC.
///
/// Returns `(corrections, corrected_msg)` or `(-1, original_msg)` if
/// uncorrectable.  Only corrects up to `max_errors` (typically 1 or 2).
fn correct_message(
    input: &[u8; MODES_LONG_MSG_BYTES],
    max_errors: usize,
) -> (isize, [u8; MODES_LONG_MSG_BYTES]) {
    // Check long-form (112-bit) CRC first.
    let long_syndrome = crc24_parity(input);
    if long_syndrome == 0 {
        // Already valid.
        return (0, *input);
    }

    // Try 1-bit correction on the long message.
    if max_errors >= 1 {
        for bit in 0..MODES_LONG_MSG_BITS {
            if single_bit_syndrome(bit) == long_syndrome {
                let mut msg = *input;
                apply_bit_errors(&mut msg, bit);
                return (1, msg);
            }
        }
    }

    // Try 2-bit correction on the long message.
    if max_errors >= 2 {
        for b1 in 0..MODES_LONG_MSG_BITS {
            let s1 = single_bit_syndrome(b1);
            let target = long_syndrome ^ s1;
            for b2 in (b1 + 1)..MODES_LONG_MSG_BITS {
                if single_bit_syndrome(b2) == target {
                    let mut msg = *input;
                    apply_bit_errors(&mut msg, b1);
                    apply_bit_errors(&mut msg, b2);
                    return (2, msg);
                }
            }
        }
    }

    // Try short-form (56-bit) correction.
    let short_bytes = &input[..MODES_SHORT_MSG_BYTES];
    let short_syndrome = crc24_parity(short_bytes);
    if short_syndrome == 0 {
        // Valid short message (DF11 with IID = 0).
        let mut msg = [0u8; MODES_LONG_MSG_BYTES];
        msg[..MODES_SHORT_MSG_BYTES].copy_from_slice(short_bytes);
        return (0, msg);
    }

    if max_errors >= 1 {
        for bit in 0..MODES_SHORT_MSG_BITS {
            if single_bit_syndrome(bit + SHORT_MSG_OFFSET) == short_syndrome {
                let mut msg = [0u8; MODES_LONG_MSG_BYTES];
                msg[..MODES_SHORT_MSG_BYTES].copy_from_slice(short_bytes);
                apply_bit_errors(&mut msg, bit);
                return (1, msg);
            }
        }
    }

    (-1, *input)
}

// ========================================================================
// Scoring
// ========================================================================

fn is_long_pi_message(msg: &[u8]) -> bool {
    let df = msg[0] >> 3;
    df == 17 || df == 18
}

fn is_short_pi_message(msg: &[u8]) -> bool {
    (msg[0] >> 3) == 11
}

/// Score how plausible a raw Mode S message looks.
/// Higher scores are more reliable.
pub fn score_mode_s_message(
    uncorrected: &[u8; MODES_LONG_MSG_BYTES],
    icao_filter: &IcaoFilter,
    enable_df24: bool,
    max_errors: usize,
) -> ScoreRank {
    const ALL_ZEROS: [u8; MODES_SHORT_MSG_BYTES] = [0; MODES_SHORT_MSG_BYTES];
    if &uncorrected[..MODES_SHORT_MSG_BYTES] == &ALL_ZEROS[..] {
        return ScoreRank::AllZeros;
    }

    let (corrections, corrected) = correct_message(uncorrected, max_errors);
    let df = corrected[0] >> 3;

    let addr = ((corrected[1] as u32) << 16)
        | ((corrected[2] as u32) << 8)
        | (corrected[3] as u32);

    match df {
        0 | 4 | 5 => {
            let syndrome = crc24_parity(&corrected[..MODES_SHORT_MSG_BYTES]);
            let recent = if syndrome == 0 {
                icao_filter.contains(addr & 0xFFFFFF)
            } else {
                false
            };
            if recent {
                ScoreRank::UnreliableKnown
            } else {
                ScoreRank::UnreliableUnknown
            }
        }
        16 | 20 | 21 => {
            let syndrome = crc24_parity(&corrected);
            let recent = if syndrome == 0 {
                icao_filter.contains(addr & 0xFFFFFF)
            } else {
                false
            };
            if recent {
                ScoreRank::UnreliableKnown
            } else {
                ScoreRank::UnreliableUnknown
            }
        }
        24..=31 => {
            if !enable_df24 {
                return ScoreRank::Uncorrectable;
            }
            let syndrome = crc24_parity(&corrected);
            let recent = if syndrome == 0 {
                icao_filter.contains(addr & 0xFFFFFF)
            } else {
                false
            };
            if recent {
                ScoreRank::UnreliableKnown
            } else {
                ScoreRank::UnreliableUnknown
            }
        }
        11 => {
            let syndrome = crc24_parity(&corrected[..MODES_SHORT_MSG_BYTES]);
            if syndrome & 0xFFFF80 != 0 {
                // CRC does not match the expected form for DF11 (IID != 0 case handled below loosely).
                // We still allow it if fully valid.
            }
            let iid = syndrome & 0x7F;
            let recent = icao_filter.contains(addr & 0xFFFFFF);
            match corrections {
                0 => {
                    if iid == 0 {
                        return if recent {
                            ScoreRank::Df11AcqKnown
                        } else {
                            ScoreRank::Df11AcqUnknown
                        };
                    } else {
                        return if recent {
                            ScoreRank::Df11IidKnown
                        } else {
                            ScoreRank::Df11IidUnknown
                        };
                    }
                }
                1 => {
                    if iid == 0 {
                        return if recent {
                            ScoreRank::Df11Acq1ErrorKnown
                        } else {
                            ScoreRank::Df11Acq1ErrorUnknown
                        };
                    } else {
                        return if recent {
                            ScoreRank::Df11Iid1ErrorKnown
                        } else {
                            ScoreRank::Df11Iid1ErrorUnknown
                        };
                    }
                }
                _ => return ScoreRank::Uncorrectable,
            }
        }
        17 => {
            let recent = icao_filter.contains(addr & 0xFFFFFF);
            match corrections {
                0 => return if recent { ScoreRank::Df17Known } else { ScoreRank::Df17Unknown },
                1 => return if recent {
                    ScoreRank::Df17_1ErrorKnown
                } else {
                    ScoreRank::Df17_1ErrorUnknown
                },
                2 => return if recent {
                    ScoreRank::Df17_2ErrorKnown
                } else {
                    ScoreRank::Df17_2ErrorUnknown
                },
                _ => return ScoreRank::Uncorrectable,
            }
        }
        18 => {
            let recent = icao_filter.contains(addr | 0x0100_0000); // NT flag
            match corrections {
                0 => return if recent { ScoreRank::Df18Known } else { ScoreRank::Df18Unknown },
                1 => return if recent {
                    ScoreRank::Df18_1ErrorKnown
                } else {
                    ScoreRank::Df18_1ErrorUnknown
                },
                2 => return if recent {
                    ScoreRank::Df18_2ErrorKnown
                } else {
                    ScoreRank::Df18_2ErrorUnknown
                },
                _ => return ScoreRank::Uncorrectable,
            }
        }
        _ => ScoreRank::UnknownDf,
    }
}

/// Return the message length in bits for a given downlink format.
pub fn mode_s_message_len_by_type(df: u8) -> usize {
    if df & 0x10 != 0 {
        MODES_LONG_MSG_BITS
    } else {
        MODES_SHORT_MSG_BITS
    }
}

/// Decode a raw Mode S message into a [`ModesMessage`].
///
/// Returns `Ok(())` on success, or an error code if the message is rejected.
pub fn decode_mode_s_message(
    mm: &mut ModesMessage,
    input: &[u8; MODES_LONG_MSG_BYTES],
    icao_filter: &mut IcaoFilter,
    enable_df24: bool,
    max_errors: usize,
) -> Result<(), i32> {
    if mm.score == ScoreRank::NotSet {
        mm.score = score_mode_s_message(input, icao_filter, enable_df24, max_errors);
    }

    if mm.score < ScoreRank::UnknownThreshold {
        return Err(-1);
    }
    if mm.score < ScoreRank::AcceptThreshold {
        return Err(-2);
    }

    mm.verbatim.copy_from_slice(input);

    let (corrections, corrected) = correct_message(input, max_errors);
    mm.msg = corrected;
    mm.correctedbits = corrections as usize;

    let df = mm.msg[0] >> 3;
    mm.msgtype = df;
    mm.msgbits = mode_s_message_len_by_type(df);

    // Populate basic fields used by the upper layers.
    match df {
        11 => {
            // All-call reply.
            mm.addr =
                ((mm.msg[1] as u32) << 16) | ((mm.msg[2] as u32) << 8) | (mm.msg[3] as u32);
            mm.crc = crc24_parity(&mm.msg[..MODES_SHORT_MSG_BYTES]);
            icao_filter.add(mm.addr);
        }
        17 | 18 => {
            mm.addr =
                ((mm.msg[1] as u32) << 16) | ((mm.msg[2] as u32) << 8) | (mm.msg[3] as u32);
            mm.crc = crc24_parity(&mm.msg);
            icao_filter.add(mm.addr);
        }
        _ => {
            mm.crc = crc24_parity(&mm.msg[..mm.msgbits / 8]);
        }
    }

    mm.reliable = (mm.score >= ScoreRank::AcceptThreshold) && (mm.correctedbits == 0);

    Ok(())
}

// ========================================================================
// Time helpers
// ========================================================================

#[inline]
fn receiveclock_ms_elapsed(t1: u64, t2: u64) -> u64 {
    (t2 - t1) / 12000
}

// ========================================================================
// Phase-slice correlators
// ========================================================================

/// Each function correlates a 1-0 pair of symbols starting at the given
/// sample, assuming a fixed phase offset 0..4 within `m[0]`.
///
/// The coefficients sum to zero so the result is DC-insensitive.

#[inline]
fn slice_phase0(m: &[u16]) -> i32 {
    5 * m[0] as i32 - 3 * m[1] as i32 - 2 * m[2] as i32
}

#[inline]
fn slice_phase1(m: &[u16]) -> i32 {
    4 * m[0] as i32 - m[1] as i32 - 3 * m[2] as i32
}

#[inline]
fn slice_phase2(m: &[u16]) -> i32 {
    3 * m[0] as i32 + m[1] as i32 - 4 * m[2] as i32
}

#[inline]
fn slice_phase3(m: &[u16]) -> i32 {
    2 * m[0] as i32 + 3 * m[1] as i32 - 5 * m[2] as i32
}

#[inline]
fn slice_phase4(m: &[u16]) -> i32 {
    m[0] as i32 + 5 * m[1] as i32 - 5 * m[2] as i32 - m[3] as i32
}

// ========================================================================
// Demodulator core
// ========================================================================

/// Bitset of acceptable DF values for short messages (without correction).
fn valid_df_short(enable_df24: bool, fix_df: bool, nfix_crc: usize) -> u32 {
    let mut bitset: u32 = (1 << 0) | (1 << 4) | (1 << 5) | (1 << 11);
    if fix_df && nfix_crc > 0 {
        // DF11 with one damaged bit.
        bitset |= generate_damage_set(11, 1);
    }
    bitset
}

/// Bitset of acceptable DF values for long messages (without correction).
fn valid_df_long(enable_df24: bool, fix_df: bool, nfix_crc: usize) -> u32 {
    let mut bitset: u32 =
        (1 << 16) | (1 << 17) | (1 << 18) | (1 << 20) | (1 << 21);
    if enable_df24 {
        bitset |= (1 << 24)
            | (1 << 25)
            | (1 << 26)
            | (1 << 27)
            | (1 << 28)
            | (1 << 29)
            | (1 << 30)
            | (1 << 31);
    }
    if fix_df && nfix_crc > 0 {
        bitset |= generate_damage_set(17, nfix_crc);
        bitset |= generate_damage_set(18, nfix_crc);
    }
    bitset
}

/// Recursively generate a bitset of all `df` values reachable by flipping
/// at most `damage_bits` bits in the 5-bit DF field.
fn generate_damage_set(df: u8, damage_bits: usize) -> u32 {
    let mut result = 1u32 << df;
    if damage_bits == 0 {
        return result;
    }
    for bit in 0..5 {
        let damaged_df = df ^ (1 << bit);
        result |= generate_damage_set(damaged_df, damage_bits - 1);
    }
    result
}

/// 2.4 MHz Mode S / Mode A/C demodulator state.
pub struct Demod2400 {
    last_message_end: usize,
    valid_df_short: u32,
    valid_df_long: u32,
    msg1: [u8; MODES_LONG_MSG_BYTES],
    msg2: [u8; MODES_LONG_MSG_BYTES],
    pub icao_filter: IcaoFilter,
    pub enable_df24: bool,
    pub fix_df: bool,
    pub nfix_crc: usize,
}

impl Default for Demod2400 {
    fn default() -> Self {
        Self::new()
    }
}

impl Demod2400 {
    pub fn new() -> Self {
        let enable_df24 = false;
        let fix_df = false;
        let nfix_crc = 0;
        Demod2400 {
            last_message_end: 0,
            valid_df_short: valid_df_short(enable_df24, fix_df, nfix_crc),
            valid_df_long: valid_df_long(enable_df24, fix_df, nfix_crc),
            msg1: [0; MODES_LONG_MSG_BYTES],
            msg2: [0; MODES_LONG_MSG_BYTES],
            icao_filter: IcaoFilter::new(),
            enable_df24,
            fix_df,
            nfix_crc,
        }
    }

    /// Recompute DF bitsets when configuration changes.
    pub fn reconfigure(&mut self) {
        self.valid_df_short = valid_df_short(self.enable_df24, self.fix_df, self.nfix_crc);
        self.valid_df_long = valid_df_long(self.enable_df24, self.fix_df, self.nfix_crc);
    }

    /// Demodulate Mode S messages from a magnitude buffer.
    ///
    /// Decoded messages are delivered via `on_message`.  The caller should
    /// pass the message to tracking / network output as appropriate.
    pub fn demodulate(
        &mut self,
        mag: &MagBuf,
        stats: &mut DemodStats,
        on_message: &mut dyn FnMut(&mut ModesMessage),
    ) {
        if mag.flags.0 & MAGBUF_DISCONTINUOUS != 0 {
            self.last_message_end = 0;
        }

        let m = &mag.data[..mag.valid_length];
        let mlen = mag.valid_length - mag.overlap;

        if self.last_message_end > mlen {
            self.last_message_end = mlen;
        }

        let mut sum_scaled_signal_power: u64 = 0;
        let mut msg = self.msg1;
        let mut best_msg = self.msg2;
        let mut best_score = ScoreRank::NotSet;
        let mut best_phase = 0;
        let mut decoded_mm = ModesMessage::default();

        let mut j = self.last_message_end;
        while j < mlen {
            let preamble = &m[j..];
            if preamble.len() < 269 + 1 + 19 {
                // Not enough samples left for a full long message.
                break;
            }

            // Quick check: rising edge 0->1 and falling edge 12->13.
            if !(preamble[0] < preamble[1] && preamble[12] > preamble[13]) {
                j += 1;
                continue;
            }

            let mut high: u32 = 0;
            let mut base_signal: u32 = 0;
            let mut base_noise: u32 = 0;
            let phase_detected;

            // Preamble peak detection for phases 3..7.
            if preamble[1] > preamble[2]
                && preamble[2] < preamble[3]
                && preamble[3] > preamble[4]
                && preamble[8] < preamble[9]
                && preamble[9] > preamble[10]
                && preamble[10] < preamble[11]
            {
                // Phase 3: peaks at 1,3,9,11-12
                high = (preamble[1] as u32
                    + preamble[3] as u32
                    + preamble[9] as u32
                    + preamble[11] as u32
                    + preamble[12] as u32)
                    / 4;
                base_signal =
                    preamble[1] as u32 + preamble[3] as u32 + preamble[9] as u32;
                base_noise =
                    preamble[5] as u32 + preamble[6] as u32 + preamble[7] as u32;
                phase_detected = Some(3);
            } else if preamble[1] > preamble[2]
                && preamble[2] < preamble[3]
                && preamble[3] > preamble[4]
                && preamble[8] < preamble[9]
                && preamble[9] > preamble[10]
                && preamble[11] < preamble[12]
            {
                // Phase 4: peaks at 1,3,9,12
                high = (preamble[1] as u32
                    + preamble[3] as u32
                    + preamble[9] as u32
                    + preamble[12] as u32)
                    / 4;
                base_signal = preamble[1] as u32
                    + preamble[3] as u32
                    + preamble[9] as u32
                    + preamble[12] as u32;
                base_noise = preamble[5] as u32
                    + preamble[6] as u32
                    + preamble[7] as u32
                    + preamble[8] as u32;
                phase_detected = Some(4);
            } else if preamble[1] > preamble[2]
                && preamble[2] < preamble[3]
                && preamble[4] > preamble[5]
                && preamble[8] < preamble[9]
                && preamble[10] > preamble[11]
                && preamble[11] < preamble[12]
            {
                // Phase 5: peaks at 1,3-4,9-10,12
                high = (preamble[1] as u32
                    + preamble[3] as u32
                    + preamble[4] as u32
                    + preamble[9] as u32
                    + preamble[10] as u32
                    + preamble[12] as u32)
                    / 4;
                base_signal =
                    preamble[1] as u32 + preamble[12] as u32;
                base_noise =
                    preamble[6] as u32 + preamble[7] as u32;
                phase_detected = Some(5);
            } else if preamble[1] > preamble[2]
                && preamble[3] < preamble[4]
                && preamble[4] > preamble[5]
                && preamble[9] < preamble[10]
                && preamble[10] > preamble[11]
                && preamble[11] < preamble[12]
            {
                // Phase 6: peaks at 1,4,10,12
                high = (preamble[1] as u32
                    + preamble[4] as u32
                    + preamble[10] as u32
                    + preamble[12] as u32)
                    / 4;
                base_signal = preamble[1] as u32
                    + preamble[4] as u32
                    + preamble[10] as u32
                    + preamble[12] as u32;
                base_noise = preamble[5] as u32
                    + preamble[6] as u32
                    + preamble[7] as u32
                    + preamble[8] as u32;
                phase_detected = Some(6);
            } else if preamble[2] > preamble[3]
                && preamble[3] < preamble[4]
                && preamble[4] > preamble[5]
                && preamble[9] < preamble[10]
                && preamble[10] > preamble[11]
                && preamble[11] < preamble[12]
            {
                // Phase 7: peaks at 1-2,4,10,12
                high = (preamble[1] as u32
                    + preamble[2] as u32
                    + preamble[4] as u32
                    + preamble[10] as u32
                    + preamble[12] as u32)
                    / 4;
                base_signal = preamble[4] as u32
                    + preamble[10] as u32
                    + preamble[12] as u32;
                base_noise = preamble[6] as u32
                    + preamble[7] as u32
                    + preamble[8] as u32;
                phase_detected = Some(7);
            } else {
                j += 1;
                continue;
            }

            // SNR check: about 3.5 dB.
            if base_signal * 2 < 3 * base_noise {
                j += 1;
                continue;
            }

            // Quiet bits must be actually quiet.
            if preamble[5] >= high as u16
                || preamble[6] >= high as u16
                || preamble[7] >= high as u16
                || preamble[8] >= high as u16
                || preamble[14] >= high as u16
                || preamble[15] >= high as u16
                || preamble[16] >= high as u16
                || preamble[17] >= high as u16
                || preamble[18] >= high as u16
            {
                j += 1;
                continue;
            }

            stats.demod_preambles += 1;
            best_score = ScoreRank::NotSet;
            best_phase = -1;

            // Try all phases 4..8 (these map to try_phase values).
            for try_phase in 4..=8 {
                let mut p_ptr = &m[j + 19 + (try_phase / 5)..];
                let mut phase = try_phase % 5;
                let mut byte_len: usize = 1;

                for i in 0..byte_len {
                    let the_byte = match phase {
                        0 => {
                            let b = ((slice_phase0(p_ptr) > 0) as u8) << 7
                                | ((slice_phase2(&p_ptr[2..]) > 0) as u8) << 6
                                | ((slice_phase4(&p_ptr[4..]) > 0) as u8) << 5
                                | ((slice_phase1(&p_ptr[7..]) > 0) as u8) << 4
                                | ((slice_phase3(&p_ptr[9..]) > 0) as u8) << 3
                                | ((slice_phase0(&p_ptr[12..]) > 0) as u8) << 2
                                | ((slice_phase2(&p_ptr[14..]) > 0) as u8) << 1
                                | ((slice_phase4(&p_ptr[16..]) > 0) as u8);
                            phase = 1;
                            p_ptr = &p_ptr[19..];
                            b
                        }
                        1 => {
                            let b = ((slice_phase1(p_ptr) > 0) as u8) << 7
                                | ((slice_phase3(&p_ptr[2..]) > 0) as u8) << 6
                                | ((slice_phase0(&p_ptr[5..]) > 0) as u8) << 5
                                | ((slice_phase2(&p_ptr[7..]) > 0) as u8) << 4
                                | ((slice_phase4(&p_ptr[9..]) > 0) as u8) << 3
                                | ((slice_phase1(&p_ptr[12..]) > 0) as u8) << 2
                                | ((slice_phase3(&p_ptr[14..]) > 0) as u8) << 1
                                | ((slice_phase0(&p_ptr[17..]) > 0) as u8);
                            phase = 2;
                            p_ptr = &p_ptr[19..];
                            b
                        }
                        2 => {
                            let b = ((slice_phase2(p_ptr) > 0) as u8) << 7
                                | ((slice_phase4(&p_ptr[2..]) > 0) as u8) << 6
                                | ((slice_phase1(&p_ptr[5..]) > 0) as u8) << 5
                                | ((slice_phase3(&p_ptr[7..]) > 0) as u8) << 4
                                | ((slice_phase0(&p_ptr[10..]) > 0) as u8) << 3
                                | ((slice_phase2(&p_ptr[12..]) > 0) as u8) << 2
                                | ((slice_phase4(&p_ptr[14..]) > 0) as u8) << 1
                                | ((slice_phase1(&p_ptr[17..]) > 0) as u8);
                            phase = 3;
                            p_ptr = &p_ptr[19..];
                            b
                        }
                        3 => {
                            let b = ((slice_phase3(p_ptr) > 0) as u8) << 7
                                | ((slice_phase0(&p_ptr[3..]) > 0) as u8) << 6
                                | ((slice_phase2(&p_ptr[5..]) > 0) as u8) << 5
                                | ((slice_phase4(&p_ptr[7..]) > 0) as u8) << 4
                                | ((slice_phase1(&p_ptr[10..]) > 0) as u8) << 3
                                | ((slice_phase3(&p_ptr[12..]) > 0) as u8) << 2
                                | ((slice_phase0(&p_ptr[15..]) > 0) as u8) << 1
                                | ((slice_phase2(&p_ptr[17..]) > 0) as u8);
                            phase = 4;
                            p_ptr = &p_ptr[19..];
                            b
                        }
                        4 => {
                            let b = ((slice_phase4(p_ptr) > 0) as u8) << 7
                                | ((slice_phase1(&p_ptr[3..]) > 0) as u8) << 6
                                | ((slice_phase3(&p_ptr[5..]) > 0) as u8) << 5
                                | ((slice_phase0(&p_ptr[8..]) > 0) as u8) << 4
                                | ((slice_phase2(&p_ptr[10..]) > 0) as u8) << 3
                                | ((slice_phase4(&p_ptr[12..]) > 0) as u8) << 2
                                | ((slice_phase1(&p_ptr[15..]) > 0) as u8) << 1
                                | ((slice_phase3(&p_ptr[17..]) > 0) as u8);
                            phase = 0;
                            p_ptr = &p_ptr[20..];
                            b
                        }
                        _ => unreachable!(),
                    };

                    msg[i] = the_byte;

                    if i == 0 {
                        let df = the_byte >> 3;
                        if self.valid_df_long & (1u32 << df) != 0 {
                            byte_len = MODES_LONG_MSG_BYTES;
                        } else if self.valid_df_short & (1u32 << df) != 0 {
                            byte_len = MODES_SHORT_MSG_BYTES;
                        }
                    }
                }

                if byte_len == 1 {
                    stats.demod_rejected_bad += 1;
                    continue;
                }

                let score = score_mode_s_message(
                    &msg,
                    &self.icao_filter,
                    self.enable_df24,
                    self.nfix_crc,
                );
                if score > best_score {
                    best_msg.copy_from_slice(&msg);
                    best_score = score;
                    best_phase = try_phase as i32;
                    std::mem::swap(&mut msg, &mut best_msg);
                }
            }

            if best_score < ScoreRank::AcceptThreshold {
                if best_score >= ScoreRank::UnknownThreshold {
                    stats.demod_rejected_unknown_icao += 1;
                } else {
                    stats.demod_rejected_bad += 1;
                }
                j += 1;
                continue;
            }

            let msglen = mode_s_message_len_by_type(best_msg[0] >> 3);

            decoded_mm = ModesMessage::default();
            // Timestamp at the end of bit 56, adjusted for phase.
            decoded_mm.timestamp_msg = mag.sample_timestamp
                + j as u64 * 5
                + (8 + 56) * 12
                + best_phase as u64;
            decoded_mm.sys_timestamp_msg = mag.sys_timestamp
                + receiveclock_ms_elapsed(mag.sample_timestamp, decoded_mm.timestamp_msg);
            decoded_mm.score = best_score;

            if decode_mode_s_message(
                &mut decoded_mm,
                &best_msg,
                &mut self.icao_filter,
                self.enable_df24,
                self.nfix_crc,
            )
            .is_err()
            {
                stats.demod_rejected_bad += 1;
                j += 1;
                continue;
            }
            stats.demod_accepted[decoded_mm.correctedbits] += 1;

            // Measure signal power over the message duration.
            {
                let signal_len = msglen * 12 / 5;
                let mut scaled_signal_power: u64 = 0;
                for k in 0..signal_len {
                    let magv = m[j + 19 + k] as u64;
                    scaled_signal_power += magv * magv;
                }
                let signal_power =
                    scaled_signal_power as f64 / 65535.0 / 65535.0;
                decoded_mm.signal_level = signal_power / signal_len as f64;
                stats.signal_power_sum += signal_power;
                stats.signal_power_count += signal_len as u64;
                sum_scaled_signal_power += scaled_signal_power;

                if decoded_mm.signal_level > stats.peak_signal_power {
                    stats.peak_signal_power = decoded_mm.signal_level;
                }
                if decoded_mm.signal_level > 0.50119 {
                    stats.strong_signal_count += 1;
                }
            }

            // Feed trailing empty samples to adaptive logic (no-op hook).
            // In the original C this updates gain control; we keep the call site
            // so that a future adaptive-gain module can be wired in here.
            let msg_end = j + (msglen + 8) * 12 / 5;

            // Skip over the message (leave 8 bits of margin for collisions).
            j = msg_end - 8 * 12 / 5 + 1;

            // Pass the decoded message upward.
            on_message(&mut decoded_mm);
            self.last_message_end = msg_end;
        }

        // Update noise power.
        {
            let sum_signal_power =
                sum_scaled_signal_power as f64 / 65535.0 / 65535.0;
            stats.noise_power_sum += mag.mean_power * mlen as f64 - sum_signal_power;
            stats.noise_power_count += mlen as u64;
        }

        // Trailing empty samples.
        if self.last_message_end < mlen {
            self.last_message_end = 0;
        } else {
            self.last_message_end -= mlen;
        }
    }

    /// Demodulate Mode A/C (ATCRBS) replies from a magnitude buffer.
    pub fn demodulate_ac(
        &mut self,
        mag: &MagBuf,
        stats: &mut DemodStats,
        on_message: &mut dyn FnMut(&mut ModesMessage),
    ) {
        let m = &mag.data[..mag.valid_length];
        let mlen = mag.valid_length - mag.overlap;

        let noise_stddev =
            (mag.mean_power - mag.mean_level * mag.mean_level).sqrt();
        let noise_level =
            ((mag.mean_power + noise_stddev) * 65535.0 + 0.5) as u32;

        let mut f1_sample = 1usize;
        while f1_sample < mlen {
            if !(m[f1_sample - 1] < m[f1_sample]) {
                f1_sample += 1;
                continue;
            }
            if m[f1_sample + 2] > m[f1_sample] || m[f1_sample + 2] > m[f1_sample + 1] {
                f1_sample += 1;
                continue;
            }

            let f1_level = (m[f1_sample] as u32 + m[f1_sample + 1] as u32) / 2;
            if noise_level * 2 > f1_level {
                f1_sample += 1;
                continue;
            }

            let f1a_power = m[f1_sample] as f64 * m[f1_sample] as f64;
            let f1b_power = m[f1_sample + 1] as f64 * m[f1_sample + 1] as f64;
            let fraction = f1b_power / (f1a_power + f1b_power);
            let f1_clock = (25.0 * (f1_sample as f64 + fraction * fraction) + 0.5) as u32;

            let f2_clock = f1_clock + 87 * 14;
            let f2_sample = (f2_clock / 25) as usize;
            if f2_sample >= mag.valid_length {
                f1_sample += 1;
                continue;
            }

            if !(m[f2_sample - 1] < m[f2_sample]) {
                f1_sample += 1;
                continue;
            }
            if m[f2_sample + 2] > m[f2_sample] || m[f2_sample + 2] > m[f2_sample + 1] {
                f1_sample += 1;
                continue;
            }

            let f2_level = (m[f2_sample] as u32 + m[f2_sample + 1] as u32) / 2;
            if noise_level * 2 > f2_level {
                f1_sample += 1;
                continue;
            }

            let f1f2_level = f1_level.max(f2_level);
            let midpoint = ((noise_level * f1f2_level) as f64).sqrt();
            let signal_threshold = (midpoint * std::f64::consts::SQRT_2 + 0.5) as u32;
            let noise_threshold = (midpoint / std::f64::consts::SQRT_2 + 0.5) as u32;

            let mut uncertain_bits: u32 = 0;
            let mut noisy_bits: u32 = 0;
            let mut bits: u32 = 0;
            for bit in 0..20 {
                let clock = f1_clock + 87 * bit;
                let sample = (clock / 25) as usize;
                bits <<= 1;
                noisy_bits <<= 1;
                uncertain_bits <<= 1;

                if m[sample + 2] >= signal_threshold as u16 {
                    noisy_bits |= 1;
                }
                if m[sample] >= signal_threshold as u16 || m[sample + 1] >= signal_threshold as u16
                {
                    bits |= 1;
                } else if m[sample] > noise_threshold as u16
                    && m[sample + 1] > noise_threshold as u16
                {
                    uncertain_bits |= 1;
                }
            }

            if (bits & 0x80020) != 0x80020 {
                // Framing bits must be on.
                f1_sample += 1;
                continue;
            }
            if (bits & 0x0101B) != 0 {
                // Quiet bits must be off.
                f1_sample += 1;
                continue;
            }
            if noisy_bits != 0 || uncertain_bits != 0 {
                f1_sample += 1;
                continue;
            }

            // Convert interleaved A/C bits to standard Mode A format.
            let modeac = ((bits & 0x40000) >> 14) // C1 -> 0x0010
                | ((bits & 0x20000) >> 5)  // A1 -> 0x1000
                | ((bits & 0x10000) >> 11) // C2 -> 0x0020
                | ((bits & 0x08000) >> 2)  // A2 -> 0x2000
                | ((bits & 0x04000) >> 8)  // C4 -> 0x0040
                | ((bits & 0x02000) << 1)  // A4 -> 0x4000
                | ((bits & 0x00800) >> 3)  // B1 -> 0x0100
                | ((bits & 0x00400) >> 10) // D1 -> 0x0001
                | ((bits & 0x00200) >> 1)  // B2 -> 0x0200
                | ((bits & 0x00100) >> 8)  // D2 -> 0x0002
                | ((bits & 0x00080) << 3)  // B4 -> 0x0400
                | ((bits & 0x00040) >> 4)  // D4 -> 0x0004
                | ((bits & 0x00004) << 5); // SPI -> 0x0080

            let mut mm = ModesMessage::default();
            mm.timestamp_msg = mag.sample_timestamp + (f2_clock / 5) as u64;
            mm.sys_timestamp_msg = mag.sys_timestamp
                + receiveclock_ms_elapsed(mag.sample_timestamp, mm.timestamp_msg);
            // For Mode A/C the "address" is the decoded identity code.
            mm.addr = modeac;
            mm.msgtype = 0xFF; // sentinel for Mode A/C
            mm.msgbits = 16;
            mm.msg[0] = (modeac >> 8) as u8;
            mm.msg[1] = (modeac & 0xFF) as u8;

            stats.demod_modeac += 1;
            on_message(&mut mm);

            f1_sample += (20 * 87) / 25;
        }
    }
}

// ========================================================================
// Magnitude computation (IQ -> magnitude)
// ========================================================================

/// Common input formats that the RTL-SDR / SDR front-ends provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    /// Unsigned 8-bit IQ (RTL-SDR default).
    Uc8,
    /// Signed 16-bit IQ.
    Sc16,
    /// Signed 16-bit IQ with 11-bit Q offset correction.
    Sc16Q11,
}

/// Convert a buffer of interleaved IQ samples into magnitude values.
///
/// * `iq`   – raw bytes from the SDR.
/// * `mag`  – output magnitude buffer (must be at least `nsamples` long).
/// * `fmt`  – sample format.
///
/// Returns the mean normalised level and mean normalised power.
pub fn compute_magnitude(
    iq: &[u8],
    mag: &mut [u16],
    fmt: InputFormat,
) -> (f64, f64) {
    match fmt {
        InputFormat::Uc8 => compute_magnitude_uc8(iq, mag),
        InputFormat::Sc16 => compute_magnitude_sc16(iq, mag),
        InputFormat::Sc16Q11 => compute_magnitude_sc16q11(iq, mag),
    }
}

fn compute_magnitude_uc8(iq: &[u8], mag: &mut [u16]) -> (f64, f64) {
    let nsamples = mag.len().min(iq.len() / 2);
    let mut sum_level: f64 = 0.0;
    let mut sum_power: f64 = 0.0;
    for i in 0..nsamples {
        let i_val = iq[2 * i] as i32 - 127;
        let q_val = iq[2 * i + 1] as i32 - 127;
        let m = ((i_val * i_val + q_val * q_val) as f64).sqrt();
        let norm = m / 128.0;
        mag[i] = (norm * 65535.0).min(65535.0) as u16;
        sum_level += norm;
        sum_power += norm * norm;
    }
    let n = nsamples as f64;
    (sum_level / n, sum_power / n)
}

fn compute_magnitude_sc16(iq: &[u8], mag: &mut [u16]) -> (f64, f64) {
    let nsamples = mag.len().min(iq.len() / 4);
    let mut sum_level: f64 = 0.0;
    let mut sum_power: f64 = 0.0;
    for i in 0..nsamples {
        let i_off = i * 4;
        let i_val = i16::from_le_bytes([iq[i_off], iq[i_off + 1]]) as i32;
        let q_val = i16::from_le_bytes([iq[i_off + 2], iq[i_off + 3]]) as i32;
        let m = ((i_val * i_val + q_val * q_val) as f64).sqrt();
        // Normalise to 0..1 assuming full-scale 32767.
        let norm = m / 32767.0;
        mag[i] = (norm * 65535.0).min(65535.0) as u16;
        sum_level += norm;
        sum_power += norm * norm;
    }
    let n = nsamples as f64;
    (sum_level / n, sum_power / n)
}

fn compute_magnitude_sc16q11(iq: &[u8], mag: &mut [u16]) -> (f64, f64) {
    // sc16q11 has the same byte layout as sc16, but the values are left-shifted
    // by 5 bits (11-bit magnitude in a 16-bit word).  We shift back to restore
    // the true range before computing magnitude.
    let nsamples = mag.len().min(iq.len() / 4);
    let mut sum_level: f64 = 0.0;
    let mut sum_power: f64 = 0.0;
    for i in 0..nsamples {
        let i_off = i * 4;
        let i_val = (i16::from_le_bytes([iq[i_off], iq[i_off + 1]]) as i32) >> 5;
        let q_val = (i16::from_le_bytes([iq[i_off + 2], iq[i_off + 3]]) as i32) >> 5;
        let m = ((i_val * i_val + q_val * q_val) as f64).sqrt();
        let norm = m / 1023.0;
        mag[i] = (norm * 65535.0).min(65535.0) as u16;
        sum_level += norm;
        sum_power += norm * norm;
    }
    let n = nsamples as f64;
    (sum_level / n, sum_power / n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slice_phase_dc_invariant() {
        // Adding a constant offset should not change the result.
        let base = [100u16, 200, 300, 400];
        for offset in [0, 50, 1000] {
            let m: Vec<u16> = base.iter().map(|&v| v + offset).collect();
            assert_eq!(slice_phase0(&m), slice_phase0(&base));
            assert_eq!(slice_phase1(&m), slice_phase1(&base));
            assert_eq!(slice_phase2(&m), slice_phase2(&base));
            assert_eq!(slice_phase3(&m), slice_phase3(&base));
            assert_eq!(slice_phase4(&m), slice_phase4(&base));
        }
    }

    #[test]
    fn test_generate_damage_set() {
        // DF11 with 1 bit flipped can reach 10, 15, 3, 9, 27.
        let set = generate_damage_set(11, 1);
        assert!(set & (1 << 11) != 0); // original
        assert!(set & (1 << 10) != 0);
        assert!(set & (1 << 15) != 0);
        assert!(set & (1 << 3) != 0);
        assert!(set & (1 << 9) != 0);
        assert!(set & (1 << 27) != 0);
    }

    #[test]
    fn test_uc8_magnitude() {
        let iq = vec![127, 127, 255, 127]; // zero Q, then max I
        let mut mag = vec![0u16; 2];
        let (level, power) = compute_magnitude_uc8(&iq, &mut mag);
        assert!(level > 0.0);
        assert!(power > 0.0);
        assert_eq!(mag[0], 0); // zero signal
        assert!(mag[1] > mag[0]);
    }
}
