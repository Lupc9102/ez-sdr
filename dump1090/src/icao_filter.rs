//! Exact, bounded membership for recently observed transmitter addresses.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const FILTER_SIZE: usize = 4096;
const FILTER_TTL: Duration = Duration::from_secs(60);

/// Full address keys, including namespace bits, with individual expiry.
/// Membership authorizes address/parity frames, so hash collisions must never
/// count as evidence of an observed transmitter.
pub struct IcaoFilter {
    addresses: HashMap<u32, Instant>,
    last_maintenance: Instant,
}

impl IcaoFilter {
    /// Create a new, empty filter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            addresses: HashMap::with_capacity(FILTER_SIZE),
            last_maintenance: Instant::now(),
        }
    }

    /// Expire learned addresses during long-running receiver sessions.
    pub fn maintain(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_maintenance) < Duration::from_secs(1) {
            return;
        }
        self.addresses
            .retain(|_, seen| now.duration_since(*seen) < FILTER_TTL);
        self.last_maintenance = now;
    }

    /// Refresh one address, evicting only the oldest unique key at capacity.
    pub fn add(&mut self, addr: u32) {
        if !self.addresses.contains_key(&addr) && self.addresses.len() >= FILTER_SIZE {
            self.maintain();
            if self.addresses.len() >= FILTER_SIZE {
                if let Some(oldest) = self
                    .addresses
                    .iter()
                    .min_by_key(|(_, seen)| **seen)
                    .map(|(&key, _)| key)
                {
                    self.addresses.remove(&oldest);
                }
            }
        }
        self.addresses.insert(addr, Instant::now());
    }

    /// Returns `true` if `addr` has been added to the filter.
    #[must_use]
    pub fn contains(&self, addr: u32) -> bool {
        self.addresses
            .get(&addr)
            .is_some_and(|seen| seen.elapsed() < FILTER_TTL)
    }

    /// Clear the filter, removing all addresses.
    pub fn clear(&mut self) {
        self.addresses.clear();
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
        filter.addresses.insert(
            0xABCDEF,
            Instant::now() - FILTER_TTL - Duration::from_secs(1),
        );
        filter.last_maintenance = Instant::now() - Duration::from_secs(2);
        filter.add(0x123456);
        assert!(!filter.contains(0xABCDEF));
        filter.maintain();
        assert!(!filter.contains(0xABCDEF));
        assert!(filter.contains(0x123456));
    }

    #[test]
    fn sustained_insertions_preserve_recent_addresses_and_bound_storage() {
        let mut filter = IcaoFilter::new();
        for address in 0..(FILTER_SIZE as u32 * 2) {
            filter.add(address);
        }
        assert_eq!(filter.addresses.len(), FILTER_SIZE);
        assert!(filter.contains(FILTER_SIZE as u32 * 2 - 1));
        assert!(!filter.contains(0));
    }

    #[test]
    fn collisions_and_namespaces_do_not_authorize_unseen_addresses() {
        let mut filter = IcaoFilter::new();
        filter.add(0xABCDEF);
        assert!(!filter.contains(0x000151));
        assert!(!filter.contains(0x01ABCDEF));
        for _ in 0..20_000 {
            filter.add(0xABCDEF);
        }
        assert!(filter.contains(0xABCDEF));
        assert_eq!(filter.addresses.len(), 1);
    }
}
