//! Aircraft tracking - translated from track.c

use crate::demod::ModesMessage;
use std::collections::HashMap;

/// Per-aircraft tracking state: ICAO address, message count, and last seen timestamp.
#[derive(Debug, Default)]
pub struct AircraftState {
    pub addr: u32,
    pub msg_count: u64,
    pub last_seen_ms: u64,
}

/// Tracks unique aircraft seen via Mode S / ADS-B messages.
pub struct Tracker {
    pub aircraft: HashMap<u32, AircraftState>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker {
    /// Create a new, empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Tracker {
            aircraft: HashMap::new(),
        }
    }

    /// Update tracking state from a decoded Mode S message.
    /// Creates a new entry if the aircraft's ICAO address has not been seen before.
    pub fn update_from_message(&mut self, msg: &ModesMessage) {
        let entry = self
            .aircraft
            .entry(msg.addr)
            .or_insert_with(|| AircraftState {
                addr: msg.addr,
                msg_count: 0,
                last_seen_ms: 0,
            });
        entry.msg_count += 1;
        entry.last_seen_ms = msg.sys_timestamp_msg;
    }

    /// Number of unique aircraft tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.aircraft.len()
    }

    /// Returns `true` when no aircraft are being tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aircraft.is_empty()
    }

    /// Remove aircraft whose `last_seen_ms` is older than `cutoff_ms`.
    ///
    /// Without this the `aircraft` `HashMap` grows unbounded over multi-hour
    /// ADS-B sessions as aircraft leave range and are never seen again (the
    /// original `track.c` pruned at a 60 s idle threshold via `expireAircraft`).
    /// Returns the number of entries removed.
    pub fn prune_older_than(&mut self, cutoff_ms: u64) -> usize {
        let before = self.aircraft.len();
        self.aircraft.retain(|_, a| a.last_seen_ms >= cutoff_ms);
        before - self.aircraft.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demod::ModesMessage;

    fn make_msg(addr: u32, ts: u64) -> ModesMessage {
        ModesMessage {
            addr,
            sys_timestamp_msg: ts,
            ..ModesMessage::default()
        }
    }

    #[test]
    fn new_creates_empty_tracker() {
        let t = Tracker::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn update_from_message_creates_entry() {
        let mut t = Tracker::new();
        t.update_from_message(&make_msg(0xABCDEF, 1000));
        assert_eq!(t.len(), 1);
        let state = t
            .aircraft
            .get(&0xABCDEF)
            .expect("aircraft should exist after update_from_message");
        assert_eq!(state.addr, 0xABCDEF);
        assert_eq!(state.msg_count, 1);
        assert_eq!(state.last_seen_ms, 1000);
    }

    #[test]
    fn update_from_message_increments_msg_count() {
        let mut t = Tracker::new();
        let msg = make_msg(0xABCDEF, 1000);
        t.update_from_message(&msg);
        t.update_from_message(&msg);
        t.update_from_message(&msg);
        assert_eq!(t.len(), 1);
        assert_eq!(
            t.aircraft
                .get(&0xABCDEF)
                .expect("aircraft should exist after 3 updates")
                .msg_count,
            3
        );
    }

    #[test]
    fn update_from_message_updates_last_seen() {
        let mut t = Tracker::new();
        t.update_from_message(&make_msg(0xABCDEF, 1000));
        t.update_from_message(&make_msg(0xABCDEF, 2000));
        assert_eq!(
            t.aircraft
                .get(&0xABCDEF)
                .expect("aircraft should exist after last_seen update")
                .last_seen_ms,
            2000
        );
    }

    #[test]
    fn multiple_aircraft_tracked_separately() {
        let mut t = Tracker::new();
        t.update_from_message(&make_msg(0xAAAAAA, 1000));
        t.update_from_message(&make_msg(0xBBBBBB, 1000));
        t.update_from_message(&make_msg(0xAAAAAA, 2000));
        assert_eq!(t.len(), 2);
        assert_eq!(
            t.aircraft
                .get(&0xAAAAAA)
                .expect("AAAAAA should have msg_count 2")
                .msg_count,
            2
        );
        assert_eq!(
            t.aircraft
                .get(&0xBBBBBB)
                .expect("BBBBBB should have msg_count 1")
                .msg_count,
            1
        );
    }

    #[test]
    fn default_equals_new() {
        assert_eq!(Tracker::default().len(), Tracker::new().len());
    }

    #[test]
    fn prune_older_than_removes_idle_entries() {
        let mut t = Tracker::new();
        // Three aircraft, last seen at varying times.
        t.update_from_message(&make_msg(0xAAAAAA, 10_000));
        t.update_from_message(&make_msg(0xBBBBBB, 50_000));
        t.update_from_message(&make_msg(0xCCCCCC, 90_000));
        // Cutoff at 40s: only the 50s and 90s entries survive.
        let pruned = t.prune_older_than(40_000);
        assert_eq!(pruned, 1);
        assert!(!t.aircraft.contains_key(&0xAAAAAA));
        assert!(t.aircraft.contains_key(&0xBBBBBB));
        assert!(t.aircraft.contains_key(&0xCCCCCC));
        // Cutoff above the newest removes everything.
        let pruned = t.prune_older_than(100_000);
        assert_eq!(pruned, 2);
        assert!(t.is_empty());
    }
}
