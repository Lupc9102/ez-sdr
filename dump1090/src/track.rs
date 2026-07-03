//! Aircraft tracking - translated from track.c

use crate::demod::ModesMessage;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct AircraftState {
    pub addr: u32,
    pub msg_count: u64,
    pub last_seen_ms: u64,
}

pub struct Tracker {
    pub aircraft: HashMap<u32, AircraftState>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker {
    #[must_use]
    pub fn new() -> Self {
        Tracker {
            aircraft: HashMap::new(),
        }
    }

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

    #[must_use]
    pub fn len(&self) -> usize {
        self.aircraft.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aircraft.is_empty()
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
        let state = t.aircraft.get(&0xABCDEF).unwrap();
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
        assert_eq!(t.aircraft.get(&0xABCDEF).unwrap().msg_count, 3);
    }

    #[test]
    fn update_from_message_updates_last_seen() {
        let mut t = Tracker::new();
        t.update_from_message(&make_msg(0xABCDEF, 1000));
        t.update_from_message(&make_msg(0xABCDEF, 2000));
        assert_eq!(t.aircraft.get(&0xABCDEF).unwrap().last_seen_ms, 2000);
    }

    #[test]
    fn multiple_aircraft_tracked_separately() {
        let mut t = Tracker::new();
        t.update_from_message(&make_msg(0xAAAAAA, 1000));
        t.update_from_message(&make_msg(0xBBBBBB, 1000));
        t.update_from_message(&make_msg(0xAAAAAA, 2000));
        assert_eq!(t.len(), 2);
        assert_eq!(t.aircraft.get(&0xAAAAAA).unwrap().msg_count, 2);
        assert_eq!(t.aircraft.get(&0xBBBBBB).unwrap().msg_count, 1);
    }

    #[test]
    fn default_equals_new() {
        assert_eq!(Tracker::default().len(), Tracker::new().len());
    }
}
