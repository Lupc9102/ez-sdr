//! Simple bounded ring buffer for Vec<u8> items.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

struct Inner {
    queue: VecDeque<Vec<u8>>,
    capacity: usize,
    halted: bool,
}

pub struct Fifo {
    inner: Mutex<Inner>,
    not_empty: Condvar,
    not_full: Condvar,
}

impl Fifo {
    pub fn new(capacity: usize) -> Self {
        Fifo {
            inner: Mutex::new(Inner {
                queue: VecDeque::with_capacity(capacity),
                capacity,
                halted: false,
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
        }
    }

    /// Push an item. If the queue is full, blocks up to `timeout_ms`.
    /// Returns `Some(item)` if it could not be pushed (timeout or halted).
    pub fn push(&self, item: Vec<u8>, timeout_ms: u32) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap();
        let deadline = if timeout_ms > 0 {
            Some(Instant::now() + Duration::from_millis(timeout_ms as u64))
        } else {
            None
        };

        while inner.queue.len() >= inner.capacity && !inner.halted {
            match deadline {
                Some(d) => {
                    let dur = d.saturating_duration_since(Instant::now());
                    if dur.is_zero() {
                        return Some(item);
                    }
                    let (guard, timed_out) = self.not_full.wait_timeout(inner, dur).unwrap();
                    inner = guard;
                    if timed_out.timed_out() {
                        return Some(item);
                    }
                }
                None => {
                    inner = self.not_full.wait(inner).unwrap();
                }
            }
        }

        if inner.halted {
            return Some(item);
        }

        inner.queue.push_back(item);
        self.not_empty.notify_one();
        None
    }

    /// Pop an item. Blocks up to `timeout_ms` waiting.
    /// Returns `None` if the queue is empty, halted, or timed out.
    pub fn pop(&self, timeout_ms: u32) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap();
        let deadline = if timeout_ms > 0 {
            Some(Instant::now() + Duration::from_millis(timeout_ms as u64))
        } else {
            None
        };

        while inner.queue.is_empty() && !inner.halted {
            match deadline {
                Some(d) => {
                    let dur = d.saturating_duration_since(Instant::now());
                    if dur.is_zero() {
                        return None;
                    }
                    let (guard, timed_out) = self.not_empty.wait_timeout(inner, dur).unwrap();
                    inner = guard;
                    if timed_out.timed_out() {
                        return None;
                    }
                }
                None => return None,
            }
        }

        if inner.halted {
            return None;
        }

        let item = inner.queue.pop_front()?;
        self.not_full.notify_one();
        Some(item)
    }

    /// Halt the FIFO. Wake all waiters.
    pub fn halt(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.halted = true;
        inner.queue.clear();
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }

    /// Current number of items in the queue.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_halted(&self) -> bool {
        self.inner.lock().unwrap().halted
    }
}
