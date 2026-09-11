//! Mode A/C decoding — translation of `mode_ac.c`

/// Convert a 13-bit Mode S altitude field (as carried in DF0/4/16/20 replies,
/// or reconstructed from a DF17 12-bit field with M=0 inserted) from wire
/// order into 16-bit Mode A numbering.
///
/// Bit mapping follows `decodeID13Field` in the reference `mode_s.c`:
/// field C1→`0x0010`, A1→`0x1000`, C2→`0x0020`, A2→`0x2000`, C4→`0x0040`,
/// A4→`0x4000`, B1→`0x0100`, D1/Q→`0x0001`, B2→`0x0200`, D2→`0x0002`,
/// B4→`0x0400`, D4→`0x0004` (field M/X bit is dropped).
#[must_use]
pub fn decode_id13_field(id13: u32) -> u32 {
    let mut hex_gillham = 0u32;
    if id13 & 0x1000 != 0 {
        hex_gillham |= 0x0010;
    } // C1
    if id13 & 0x0800 != 0 {
        hex_gillham |= 0x1000;
    } // A1
    if id13 & 0x0400 != 0 {
        hex_gillham |= 0x0020;
    } // C2
    if id13 & 0x0200 != 0 {
        hex_gillham |= 0x2000;
    } // A2
    if id13 & 0x0100 != 0 {
        hex_gillham |= 0x0040;
    } // C4
    if id13 & 0x0080 != 0 {
        hex_gillham |= 0x4000;
    } // A4
      // Bit 6 (0x0040) is X/M — skipped, as in the reference.
    if id13 & 0x0020 != 0 {
        hex_gillham |= 0x0100;
    } // B1
    if id13 & 0x0010 != 0 {
        hex_gillham |= 0x0001;
    } // D1
    if id13 & 0x0008 != 0 {
        hex_gillham |= 0x0200;
    } // B2
    if id13 & 0x0004 != 0 {
        hex_gillham |= 0x0002;
    } // D2
    if id13 & 0x0002 != 0 {
        hex_gillham |= 0x0400;
    } // B4
    if id13 & 0x0001 != 0 {
        hex_gillham |= 0x0004;
    } // D4
    hex_gillham
}

/// Convert a Mode A value (16-bit numbering, see [`decode_id13_field`]) to
/// Mode C altitude in 100-ft units, ported from `mode_ac.c::ModeAToModeC`.
///
/// Returns `None` for invalid codes (zero C bits, `OneHundreds` out of 1..=5,
/// or forbidden zero-bits set).
#[must_use]
pub fn mode_a_to_mode_c(mode_a: u32) -> Option<i32> {
    if (mode_a & 0xFFFF8889) != 0 || (mode_a & 0x000000F0) == 0 {
        return None;
    }

    let mut five_hundreds = 0u32;
    if mode_a & 0x0002 != 0 {
        five_hundreds ^= 0x0FF;
    }
    if mode_a & 0x0004 != 0 {
        five_hundreds ^= 0x07F;
    }
    if mode_a & 0x1000 != 0 {
        five_hundreds ^= 0x03F;
    }
    if mode_a & 0x2000 != 0 {
        five_hundreds ^= 0x01F;
    }
    if mode_a & 0x4000 != 0 {
        five_hundreds ^= 0x00F;
    }
    if mode_a & 0x0100 != 0 {
        five_hundreds ^= 0x007;
    }
    if mode_a & 0x0200 != 0 {
        five_hundreds ^= 0x003;
    }
    if mode_a & 0x0400 != 0 {
        five_hundreds ^= 0x001;
    }

    let mut one_hundreds = 0u32;
    if mode_a & 0x0010 != 0 {
        one_hundreds ^= 0x007;
    }
    if mode_a & 0x0020 != 0 {
        one_hundreds ^= 0x003;
    }
    if mode_a & 0x0040 != 0 {
        one_hundreds ^= 0x001;
    }
    if (one_hundreds & 5) == 5 {
        one_hundreds ^= 2;
    }
    if one_hundreds > 5 {
        return None;
    }

    // Correct order of OneHundreds (reference: odd FiveHundreds inverts it).
    if (five_hundreds & 1) != 0 {
        one_hundreds = 6 - one_hundreds;
    }

    Some((five_hundreds * 5 + one_hundreds) as i32 - 13)
}

/// Convert altitude in 100-ft units to a Mode A squawk code.
///
/// This is the inverse of the original C function `ModeAToModeC`.
/// Valid altitudes range from -12 (−1200 ft) to 1267 (126 700 ft).
/// Returns `None` for out-of-range or mathematically invalid inputs.
///
/// This utility has tests but no production callers yet.
/// It exists as a building block for future Mode A/C encoding needs
/// (e.g., synthetic squawk generation or round-trip validation in the
/// `mode_s` decoder).
#[must_use]
pub fn altitude_100ft_to_squawk(mode_c: i32) -> Option<u32> {
    // From ModeAToModeC: altitude = FiveHundreds × 5 + OneHundreds − 13
    // where OneHundreds ∈ {1,2,3,4,5} and FiveHundreds ∈ [0, 255].
    let x = mode_c.checked_add(13)?;
    if !(1..=1280).contains(&x) {
        // 1280 = 255 × 5 + 5 → max FiveHundreds = 255 with OneHundreds = 5
        return None;
    }

    let x = x as u32;
    let oh_calc = x % 5;
    let oh = if oh_calc == 0 { 5 } else { oh_calc };
    let fh = (x - oh) / 5;

    // Inverse of the decoder's `if (FiveHundreds & 1) OneHundreds = 6 - OneHundreds`
    // correction (`mode_a_to_mode_c`): with an odd FiveHundreds the C bits must
    // carry the complemented value, otherwise decode returns a mirrored altitude.
    let oh_enc = if (fh & 1) != 0 { 6 - oh } else { oh };

    // ── Encode OneHundreds into C bits (binary-to-Gray for 3-bit) ──
    // Forward mapping (Gray-to-binary with 7→5 fix):
    //   C4 C2 C1 → decoded OH
    //   1  0  0 → 1
    //   1  1  0 → 2
    //   0  1  0 → 3
    //   0  1  1 → 4
    //   0  0  1 → 5
    let c_val: u32 = match oh_enc {
        1 => 0b100,
        2 => 0b110,
        3 => 0b010,
        4 => 0b011,
        5 => 0b001,
        _ => return None,
    };

    // ── Encode FiveHundreds into D / A / B bits (bijection via Gray code) ──
    // Forward does gray-to-binary: FiveHundreds = decode_gray(D2..B4)
    // Reverse: gray = binary_to_gray(FiveHundreds) = fh ^ (fh >> 1)
    let g = (fh as u8) ^ ((fh as u8) >> 1);

    // Gray bit → Mode A squawk bit
    let d2 = u32::from((g >> 7) & 1);
    let d4 = u32::from((g >> 6) & 1);
    let a1 = u32::from((g >> 5) & 1);
    let a2 = u32::from((g >> 4) & 1);
    let a4 = u32::from((g >> 3) & 1);
    let b1 = u32::from((g >> 2) & 1);
    let b2 = u32::from((g >> 1) & 1);
    let b4 = u32::from(g & 1);

    let mode_a = (d2 << 1)   // D2 — bit 1
        | (d4 << 2)   // D4 — bit 2
        | ((c_val & 1) << 4)       // C1 — bit 4
        | (((c_val >> 1) & 1) << 5) // C2 — bit 5
        | (((c_val >> 2) & 1) << 6) // C4 — bit 6
        | (b1 << 8)   // B1 — bit 8
        | (b2 << 9)   // B2 — bit 9
        | (b4 << 10)  // B4 — bit 10
        | (a1 << 12)  // A1 — bit 12
        | (a2 << 13)  // A2 — bit 13
        | (a4 << 14); // A4 — bit 14

    Some(mode_a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_c_round_trip() {
        // Full valid range, not just low altitudes: the high range exercises
        // D2/D4/A-bit paths (e.g. FiveHundreds > 42 sets gray bit 7 → D2).
        for alt in -12..=1267 {
            let ma = altitude_100ft_to_squawk(alt).expect("encoding should succeed");
            let back = mode_a_to_mode_c(ma).expect("decoding should succeed");
            assert_eq!(alt, back, "round-trip failed for altitude {alt}");
        }
    }

    #[test]
    fn mode_c_decode_id13_known_bits() {
        // Spot-check the ID13→Mode A table against the reference mapping:
        // field C1 (bit 12) → 0x0010, A1 (bit 11) → 0x1000, D4 (bit 0) → 0x0004.
        assert_eq!(decode_id13_field(0x1000), 0x0010);
        assert_eq!(decode_id13_field(0x0800), 0x1000);
        assert_eq!(decode_id13_field(0x0001), 0x0004);
        assert_eq!(decode_id13_field(0x0000), 0x0000);
    }

    #[test]
    fn mode_c_rejects_d1_and_zero_c_bits() {
        // D1 set is illegal for altitude; C1..C4 all zero is illegal.
        assert!(mode_a_to_mode_c(0x0001).is_none());
        assert!(mode_a_to_mode_c(0x1200).is_none());
    }

    #[test]
    fn mode_c_altitude_ground_level() {
        let ma = altitude_100ft_to_squawk(0).expect("0 is within valid range");
        let alt = mode_a_to_mode_c(ma).expect("decoded squawk should be valid");
        assert_eq!(alt, 0);
    }

    #[test]
    fn mode_c_altitude_lower_bound() {
        // −1200 ft → −12 (100-ft units)
        let ma = altitude_100ft_to_squawk(-12).expect("-12 is within valid range");
        assert_ne!(ma & 0x000000F0, 0);
    }

    #[test]
    fn mode_c_returns_none_for_out_of_range() {
        assert!(altitude_100ft_to_squawk(-13).is_none());
        assert!(altitude_100ft_to_squawk(1268).is_none());
    }

    #[test]
    fn mode_c_known_good_example() {
        // 3500 ft → 35 (100-ft units)
        let ma = altitude_100ft_to_squawk(35).expect("35 is within valid range");
        // C bits should be non-zero (C2 set for OH=3)
        assert_ne!(ma & 0x000000F0, 0);
        // D1 must stay clear
        assert_eq!(ma & 1, 0);
        // SPI must stay clear
        assert_eq!(ma & 0x0080, 0);
    }
}
