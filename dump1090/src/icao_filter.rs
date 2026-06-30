//! ICAO address filter — bloom-like bitset derived from dump1090’s `icao_filter.c`

const FILTER_SIZE: usize = 4096;
const FILTER_MASK: usize = FILTER_SIZE - 1;
const U64_COUNT: usize = FILTER_SIZE / 64;

/// ICAO address filter backed by a 4096-bit bitset (~512 bytes).
///
/// Translates the hashing and membership logic from the C `icao_filter.c`
/// implementation into an idiomatic, allocation-free Rust struct.  Because
/// only a single bit is stored per hash bucket, different addresses may
/// collide and produce false positives.
pub struct IcaoFilter {
    bits: [u64; U64_COUNT],
}

impl IcaoFilter {
    /// Create a new, empty filter.
    pub fn new() -> Self {
        Self {
            bits: [0; U64_COUNT],
        }
    }

    /// Jenkins one-at-a-time hash (unrolled for 3 bytes), exactly as the C original.
    #[inline]
    fn icao_hash(addr: u32) -> usize {
        let mut hash = 0u32;

        hash += addr & 0xff;
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;

        hash += (addr >> 8) & 0xff;
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;

        hash += (addr >> 16) & 0xff;
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;

        hash = hash.wrapping_add(hash << 3);
        hash ^= hash >> 11;
        hash = hash.wrapping_add(hash << 15);

        (hash as usize) & FILTER_MASK
    }

    /// Set the bit corresponding to `addr`.
    pub fn add(&mut self, addr: u32) {
        let h = Self::icao_hash(addr);
        self.bits[h >> 6] |= 1u64 << (h & 63);
    }

    /// Returns `true` if `addr` has been added to the filter.
    ///
    /// Because this is a bitset representation, collisions between different
    /// addresses can produce false positives.
    pub fn contains(&self, addr: u32) -> bool {
        let h = Self::icao_hash(addr);
        (self.bits[h >> 6] >> (h & 63)) & 1 != 0
    }

    /// Clear the filter, removing all addresses.
    pub fn clear(&mut self) {
        self.bits = [0; U64_COUNT];
    }
}

impl Default for IcaoFilter {
    fn default() -> Self {
        Self::new()
    }
}
