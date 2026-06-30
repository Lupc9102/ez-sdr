//! Utility helpers - translated from util.c

use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

/// Global exit flag (replaces Modes.exit).
pub static EXIT: AtomicBool = AtomicBool::new(false);

/// Return current wall-clock time in milliseconds.
pub fn mstime() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
