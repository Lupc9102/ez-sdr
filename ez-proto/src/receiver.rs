//! Shared bounded reception-rate accounting, independent of rendering and traffic.

use std::collections::VecDeque;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const MESSAGE_RATE_WINDOW_SECONDS: f64 = 5.0;
const BUCKET_MS: u128 = 100;
const BUCKETS: u64 = 50;

/// Five-second received-message rate, with 100 ms buckets and at most 50 entries.
/// Counters can reset on receiver restart; old reception then leaves the window.
#[derive(Debug)]
pub struct MessageRateWindow {
    origin: Instant,
    last_total: u64,
    buckets: VecDeque<(u64, u64)>,
}

impl Default for MessageRateWindow {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl MessageRateWindow {
    pub fn new(origin: Instant) -> Self {
        Self {
            origin,
            last_total: 0,
            buckets: VecDeque::new(),
        }
    }

    pub fn observe_total(&mut self, total: u64, now: Instant) {
        if total < self.last_total || now < self.origin {
            *self = Self::new(now);
        }
        let tick = (now.duration_since(self.origin).as_millis() / BUCKET_MS) as u64;
        while self
            .buckets
            .front()
            .is_some_and(|&(old, _)| tick.saturating_sub(old) >= BUCKETS)
        {
            self.buckets.pop_front();
        }
        let count = total.saturating_sub(self.last_total);
        self.last_total = total;
        if count == 0 {
            return;
        }
        if let Some((_, n)) = self.buckets.back_mut().filter(|(last, _)| *last == tick) {
            *n = n.saturating_add(count);
        } else {
            self.buckets.push_back((tick, count));
        }
    }

    pub fn rate_at(&self, now: Instant) -> f64 {
        let Some(elapsed) = now.checked_duration_since(self.origin) else {
            return 0.0;
        };
        let tick = (elapsed.as_millis() / BUCKET_MS) as u64;
        self.buckets
            .iter()
            .filter(|&&(old, _)| tick.saturating_sub(old) < BUCKETS)
            .map(|&(_, count)| count as f64)
            .sum::<f64>()
            / MESSAGE_RATE_WINDOW_SECONDS
    }
}

pub fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn received_rate_tracks_known_cadence_and_expires_without_new_messages() {
        let now = Instant::now();
        let mut rate = MessageRateWindow::new(now);
        for n in 1..=100 {
            rate.observe_total(n, now + Duration::from_millis((n - 1) * 50));
        }
        assert_eq!(rate.rate_at(now + Duration::from_millis(4999)), 20.0);
        assert!(rate.buckets.len() <= 50);
        assert_eq!(rate.rate_at(now + Duration::from_secs(10)), 0.0);
        rate.observe_total(101, now + Duration::from_secs(20));
        assert_eq!(rate.rate_at(now + Duration::from_secs(20)), 0.2);
        assert_eq!(rate.buckets.len(), 1);
    }

    #[test]
    fn receiver_restart_and_clock_rewind_retire_previous_rate() {
        let now = Instant::now();
        let mut rate = MessageRateWindow::new(now);
        rate.observe_total(100, now);
        rate.observe_total(3, now + Duration::from_secs(1));
        assert_eq!(rate.rate_at(now + Duration::from_secs(1)), 0.6);
        rate.observe_total(5, now);
        assert_eq!(rate.rate_at(now), 1.0);
    }
}
