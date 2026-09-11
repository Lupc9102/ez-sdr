//! Integration tests for event bus and cross-module communication.
//!
//! NOTE: this file must NOT be wrapped in `#[cfg(test)]` — integration tests
//! under `tests/` compile as their own crate, where `cfg(test)` is false, so
//! the gate silently disabled all four tests below (they never ran in CI).

mod tests {
    use ez_gui::events::{AppEvent, EventBus};
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_event_bus_publish_and_drain() {
        let bus = EventBus::new();
        assert_eq!(bus.pending_count(), 0);

        bus.publish(AppEvent::SdrStarted);
        bus.publish(AppEvent::FrequencyChanged { hz: 100_000_000 });
        assert_eq!(bus.pending_count(), 2);

        let events = bus.drain();
        assert_eq!(events.len(), 2);
        assert_eq!(bus.pending_count(), 0);
    }

    #[test]
    fn test_event_bus_thread_safety() {
        let bus = Arc::new(EventBus::new());
        let mut handles = vec![];

        // Spawn 10 threads publishing events
        for i in 0..10 {
            let bus_clone = Arc::clone(&bus);
            let handle = thread::spawn(move || {
                for j in 0..10 {
                    bus_clone.publish(AppEvent::FrequencyChanged {
                        hz: (i * 1_000_000 + j * 100_000) as u64,
                    });
                }
            });
            handles.push(handle);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Should have 100 events (10 threads × 10 events)
        let events = bus.drain();
        assert_eq!(events.len(), 100);
    }

    #[test]
    fn test_event_bus_history() {
        let bus = EventBus::new();
        bus.publish(AppEvent::SdrStarted);
        bus.publish(AppEvent::FrequencyChanged { hz: 100_000_000 });
        bus.publish(AppEvent::SdrStopped);

        let recent = bus.recent_events(2);
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert_eq!(recent[0], AppEvent::SdrStopped);
        assert_eq!(recent[1], AppEvent::FrequencyChanged { hz: 100_000_000 });
    }

    #[test]
    fn test_event_bus_preserves_event_order() {
        let bus = EventBus::new();
        for i in 0..100 {
            bus.publish(AppEvent::FrequencyChanged { hz: i });
        }

        let events = bus.drain();
        for (idx, event) in events.iter().enumerate() {
            if let AppEvent::FrequencyChanged { hz } = event {
                assert_eq!(*hz, idx as u64);
            }
        }
    }
}
