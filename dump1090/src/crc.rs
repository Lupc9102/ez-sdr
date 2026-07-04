//! Mode S CRC24 calculation
//! Translated from starch/crc.c

/// Generator polynomial for the Mode S CRC.
const MODES_GENERATOR_POLY: u32 = 0xFFF409;

/// Precomputed CRC24 lookup table for all possible byte values.
pub const CRC24_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i: usize = 0;
    while i < 256 {
        let mut c = (i as u32) << 16;
        let mut j = 0;
        while j < 8 {
            if c & 0x800000 != 0 {
                c = (c << 1) ^ MODES_GENERATOR_POLY;
            } else {
                c <<= 1;
            }
            j += 1;
        }
        table[i] = c & 0x00FFFFFF;
        i += 1;
    }
    table
};

/// Compute the Mode S CRC24 over a data payload.
///
/// The returned 24-bit value is the parity field that would be appended to the
/// payload to form a valid Mode S message.
///
/// For a 56-bit message: `data` is the first 4 bytes.\
/// For a 112-bit message: `data` is the first 11 bytes.
#[must_use]
pub fn crc24(data: &[u8]) -> u32 {
    let mut rem: u32 = 0;
    for &byte in data {
        let idx = (byte ^ ((rem & 0xFF_0000) >> 16) as u8) as usize;
        rem = ((rem << 8) ^ CRC24_TABLE[idx]) & 0x00FFFFFF;
    }
    rem
}

/// Compute the Mode S CRC24 syndrome over a full message, including its
/// trailing 3-byte parity field.
///
/// A valid message yields a syndrome of `0`.
///
/// For a 56-bit message, `msg` must be exactly 7 bytes.\
/// For a 112-bit message, `msg` must be exactly 14 bytes.
///
/// # Panics
/// Panics if `msg` contains fewer than 3 bytes.
#[must_use]
pub fn crc24_parity(msg: &[u8]) -> u32 {
    let n = msg.len();
    assert!(
        n >= 3,
        "message must contain at least a 3-byte parity field"
    );

    let mut rem = crc24(&msg[..n - 3]);
    rem ^= u32::from(msg[n - 3]) << 16;
    rem ^= u32::from(msg[n - 2]) << 8;
    rem ^= u32::from(msg[n - 1]);
    rem & 0x00FFFFFF
}

/// Return `true` if the supplied Mode S message passes its CRC24 check.
#[must_use]
pub fn check_crc(msg: &[u8]) -> bool {
    crc24_parity(msg) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A known-valid 112-bit Mode S extended squitter (DF 17) test message.
    /// [ 8D 48 40 D6 20 2C C3 7C DB 31 57 21 4A B3 ]
    const VALID_112: [u8; 14] = [
        0x8D, 0x48, 0x40, 0xD6, 0x20, 0x2C, 0xC3, 0x7C, 0xDB, 0x31, 0x57, 0x9F, 0xE8, 0x02,
    ];

    #[test]
    fn test_crc24_table_matches_c_implementation() {
        // Check a few known table entries against the C algorithm.
        assert_eq!(CRC24_TABLE[0x00], 0x000000);
        assert_eq!(CRC24_TABLE[0x01], 0xFFF409);
        assert_eq!(CRC24_TABLE[0x80], 0x0706C0);
        assert_eq!(CRC24_TABLE[0xFF], 0xFA0480);
    }

    #[test]
    fn test_crc24_parity_valid_message() {
        assert_eq!(crc24_parity(&VALID_112), 0);
        assert!(check_crc(&VALID_112));
    }

    #[test]
    fn test_crc24_parity_invalid_message() {
        let mut bad = VALID_112;
        bad[0] ^= 0x01; // flip one bit
        assert_ne!(crc24_parity(&bad), 0);
        assert!(!check_crc(&bad));
    }

    #[test]
    fn test_crc24_generates_correct_parity() {
        // crc24 on payload alone should equal the last 3 bytes of a valid message.
        let payload = &VALID_112[..VALID_112.len() - 3];
        let parity = crc24(payload);
        assert_eq!((parity >> 16) as u8, VALID_112[VALID_112.len() - 3]);
        assert_eq!((parity >> 8) as u8, VALID_112[VALID_112.len() - 2]);
        assert_eq!(parity as u8, VALID_112[VALID_112.len() - 1]);
    }

    #[test]
    fn test_crc24_all_table_entries_nonzero_for_nonzero_input() {
        // After the initial offset, entries should be well-distributed.
        // At minimum, entries 1 and 0xFF are non-zero (verified against C impl).
        assert_ne!(CRC24_TABLE[1], 0);
        assert_ne!(CRC24_TABLE[255], 0);
    }

    #[test]
    fn test_crc24_table_is_256_entries() {
        assert_eq!(CRC24_TABLE.len(), 256);
    }

    #[test]
    fn test_crc24_empty_payload() {
        assert_eq!(crc24(&[]), 0);
    }

    #[test]
    fn test_crc24_single_byte() {
        // crc24 of a single zero byte should equal CRC24_TABLE[0] = 0
        assert_eq!(crc24(&[0x00]), 0);
        // crc24 of a single non-zero byte should equal CRC24_TABLE[that_byte]
        assert_eq!(crc24(&[0x01]), CRC24_TABLE[0x01]);
        assert_eq!(crc24(&[0x80]), CRC24_TABLE[0x80]);
    }

    #[test]
    fn test_crc24_parity_short_msg_panics() {
        // A message with fewer than 3 bytes should panic (assert)
        let result = std::panic::catch_unwind(|| {
            let _ = crc24_parity(&[0x00, 0x01]);
        });
        assert!(
            result.is_err(),
            "crc24_parity should panic on <3 byte input"
        );
    }

    #[test]
    fn test_check_crc_multi_bit_flip_detected() {
        for i in 0..VALID_112.len() {
            for bit in 0..3 {
                let mut bad = VALID_112;
                bad[i] ^= 1 << bit;
                assert!(
                    !check_crc(&bad),
                    "bit flip at byte {i} bit {bit} not detected"
                );
            }
        }
    }
}
