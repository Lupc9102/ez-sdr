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

impl Tracker {
    pub fn new() -> Self {
        Tracker {
            aircraft: HashMap::new(),
        }
    }

    pub fn update_from_message(&mut self, msg: &ModesMessage) {
        let entry = self.aircraft.entry(msg.addr).or_insert_with(|| AircraftState {
            addr: msg.addr,
            msg_count: 0,
            last_seen_ms: 0,
        });
        entry.msg_count += 1;
        entry.last_seen_ms = msg.sys_timestamp_msg;
    }

    pub fn len(&self) -> usize {
        self.aircraft.len()
    }
}
