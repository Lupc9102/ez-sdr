//! ICAO address filter — bloom-like bitset derived from dump1090’s `icao_filter.c`

use std::time::{Duration, Instant};

const FILTER_SIZE: usize = 4096;
const FILTER_MASK: usize = FILTER_SIZE - 1;
const U64_COUNT: usize = FILTER_SIZE / 64;
const FILTER_TTL: Duration = Duration::from_secs(60);

/// ICAO address filter backed by a 4096-bit bitset (~512 bytes).
///
/// Translates the hashing and membership logic from the C `icao_filter.c`
/// implementation into an idiomatic, allocation-free Rust struct.  Because
/// only a single bit is stored per hash bucket, different addresses may
/// collide and produce false positives.
pub struct IcaoFilter {
    bits: [u64; U64_COUNT],
    last_decay: Instant,
    insertions: u32,
}

impl IcaoFilter {
    /// Create a new, empty filter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bits: [0; U64_COUNT],
            last_decay: Instant::now(),
            insertions: 0,
        }
    }

    /// Expire learned addresses during long-running receiver sessions.
    ///
    /// The original fixed bitset has no way to distinguish an old aircraft
    /// from one seen recently. A rolling time window keeps the false-positive
    /// rate bounded without changing the compact hash representation.
    pub fn maintain(&mut self) {
        if self.last_decay.elapsed() >= FILTER_TTL {
            self.clear();
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
        self.maintain();
        let h = Self::icao_hash(addr);
        self.bits[h >> 6] |= 1u64 << (h & 63);
        self.insertions = self.insertions.saturating_add(1);
        // A busy receiver can fill the filter before the time window expires;
        // periodically rotate it to keep collisions from becoming universal.
        if self.insertions >= (FILTER_SIZE as u32 * 2) {
            self.clear();
        }
    }

    /// Returns `true` if `addr` has been added to the filter.
    ///
    /// Because this is a bitset representation, collisions between different
    /// addresses can produce false positives.
    #[must_use]
    pub fn contains(&self, addr: u32) -> bool {
        let h = Self::icao_hash(addr);
        (self.bits[h >> 6] >> (h & 63)) & 1 != 0
    }

    /// Clear the filter, removing all addresses.
    pub fn clear(&mut self) {
        self.bits = [0; U64_COUNT];
        self.last_decay = Instant::now();
        self.insertions = 0;
    }
}

impl Default for IcaoFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_filter_is_empty() {
        let f = IcaoFilter::new();
        assert!(!f.contains(0xABCDEF));
    }

    #[test]
    fn contains_after_add() {
        let mut f = IcaoFilter::new();
        f.add(0xABCDEF);
        assert!(f.contains(0xABCDEF));
    }

    #[test]
    fn contains_multiple_addresses() {
        let mut f = IcaoFilter::new();
        f.add(0x000001);
        f.add(0xABCDEF);
        f.add(0xFFFFFF);
        assert!(f.contains(0x000001));
        assert!(f.contains(0xABCDEF));
        assert!(f.contains(0xFFFFFF));
    }

    #[test]
    fn clear_removes_all() {
        let mut f = IcaoFilter::new();
        f.add(0xABCDEF);
        f.add(0x123456);
        f.clear();
        assert!(!f.contains(0xABCDEF));
        assert!(!f.contains(0x123456));
    }

    #[test]
    fn default_equals_new() {
        assert_eq!(
            IcaoFilter::default().contains(0xABCDEF),
            IcaoFilter::new().contains(0xABCDEF)
        );
    }

    #[test]
    fn hash_is_deterministic() {
        let mut f1 = IcaoFilter::new();
        let mut f2 = IcaoFilter::new();
        f1.add(0xDEADBE);
        f2.add(0xDEADBE);
        assert!(f1.contains(0xDEADBE));
        assert!(f2.contains(0xDEADBE));
    }

    #[test]
    fn zero_address() {
        let mut f = IcaoFilter::new();
        f.add(0x000000);
        assert!(f.contains(0x000000));
    }

    #[test]
    fn add_duplicate() {
        let mut f = IcaoFilter::new();
        f.add(0xABCDEF);
        f.add(0xABCDEF);
        assert!(f.contains(0xABCDEF));
    }

    #[test]
    fn capacity_many_addresses() {
        let mut f = IcaoFilter::new();
        for i in 0..4096u32 {
            f.add(i);
        }
        assert!(f.contains(0));
        assert!(f.contains(2048));
        assert!(f.contains(4095));
    }

    #[test]
    fn edge_case_max_icao() {
        let mut f = IcaoFilter::new();
        f.add(0xFFFFFF);
        assert!(f.contains(0xFFFFFF));
    }

    #[test]
    fn maintain_expires_addresses_after_ttl() {
        let mut filter = IcaoFilter::new();
        filter.add(0xABCDEF);
        filter.last_decay = Instant::now() - FILTER_TTL - Duration::from_secs(1);
        filter.maintain();
        assert!(!filter.contains(0xABCDEF));
        assert_eq!(filter.insertions, 0);
    }

    #[test]
    fn sustained_insertions_rotate_before_the_bitset_saturates() {
        let mut filter = IcaoFilter::new();
        for address in 0..(FILTER_SIZE as u32 * 2) {
            filter.add(address);
        }
        assert_eq!(filter.insertions, 0);
        assert!(filter.bits.iter().all(|word| *word == 0));
    }
}
