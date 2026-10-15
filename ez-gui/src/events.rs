//! Thread-safe event bus for decoupled communication across panels and threads.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    FrequencyChanged {
        hz: u64,
    },
    RecordingStarted {
        filename: String,
    },
    RecordingStopped {
        duration_secs: u64,
    },
    RecordingStateChanged {
        recording: bool,
    },
    ScannerStateChanged {
        enabled: bool,
    },
    SpectrumRangeChanged {
        min_db: f32,
        max_db: f32,
    },
    DemodChanged {
        mode: String,
    },
    StatusMessage {
        text: String,
        severity: MessageSeverity,
    },
    Error {
        context: String,
        message: String,
    },
    AircraftDetected {
        icao: u32,
        callsign: Option<String>,
    },
    SatellitePassStarting {
        name: String,
        aos: String,
    },
    SignalDetected {
        freq_hz: u64,
        strength_db: f32,
    },
    SdrStarted,
    SdrStopped,
}

impl AppEvent {
    pub fn is_status_message(&self) -> bool {
        matches!(self, AppEvent::StatusMessage { .. })
    }
}

#[derive(Debug, Clone)]
pub struct EventBus {
    inner: Arc<Mutex<EventBusInner>>,
    max_history: usize,
}

#[derive(Debug, Default)]
struct EventBusInner {
    queue: VecDeque<AppEvent>,
    history: VecDeque<AppEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventBusInner::default())),
            max_history: 200,
        }
    }

    /// Publish an event to the bus.
    pub fn publish(&self, event: AppEvent) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.queue.push_back(event.clone());
            inner.history.push_back(event);
            if inner.history.len() > self.max_history {
                inner.history.pop_front();
            }
        }
    }

    /// Drain all pending events from the queue.
    pub fn drain(&self) -> Vec<AppEvent> {
        if let Ok(mut inner) = self.inner.lock() {
            inner.queue.drain(..).collect()
        } else {
            Vec::new()
        }
    }

    /// Return up to `count` recent events from history (most recent first).
    pub fn recent_events(&self, count: usize) -> Vec<AppEvent> {
        if let Ok(inner) = self.inner.lock() {
            inner.history.iter().rev().take(count).cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Return count of pending events in queue.
    pub fn pending_count(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.queue.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_bus_publish_and_drain() {
        let bus = EventBus::new();
        assert_eq!(bus.pending_count(), 0);

        bus.publish(AppEvent::SdrStarted);
        bus.publish(AppEvent::FrequencyChanged { hz: 100_000_000 });
        assert_eq!(bus.pending_count(), 2);

        let drained = bus.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0], AppEvent::SdrStarted);
        assert_eq!(drained[1], AppEvent::FrequencyChanged { hz: 100_000_000 });

        assert_eq!(bus.pending_count(), 0);
        assert!(bus.drain().is_empty());
    }

    #[test]
    fn test_event_bus_history() {
        let bus = EventBus::new();
        bus.publish(AppEvent::SdrStarted);
        bus.publish(AppEvent::SdrStopped);

        let recent = bus.recent_events(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0], AppEvent::SdrStopped);
        assert_eq!(recent[1], AppEvent::SdrStarted);
    }
}
