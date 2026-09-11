//! Wideband sample-block fan-out: a [`SampleBus`] is a [`Broadcaster`] specialized to
//! [`SampleBlock`], carrying IQ from the single ingestion thread to every interested
//! subscriber (channelizer taps, recorders, ...). See [`crate::broadcast`] for the
//! underlying lock-free fan-out mechanics (RCU subscriber registry, per-subscriber overflow
//! policy, never a lock or blocking send on the hot publish path).
//!
//! Each [`SampleBlock`] carries its sample payload behind an `Arc<[Complex32]>`, so fanning
//! a block out to N subscribers is N pointer/refcount clones, never an N x copy of the
//! underlying samples.

use std::sync::Arc;

use num_complex::Complex32;

pub use crate::broadcast::OverflowPolicy;
use crate::broadcast::{Broadcaster, BroadcasterHandle};

/// A block of consecutive wideband IQ samples, tagged with enough metadata for a
/// subscriber to reconstruct absolute time/frequency without consulting the daemon state.
#[derive(Debug, Clone)]
pub struct SampleBlock {
    pub start_sample: u64,
    pub sample_rate_hz: u32,
    pub center_freq_hz: u64,
    pub samples: Arc<[Complex32]>,
}

/// Lock-free, non-blocking fan-out of [`SampleBlock`]s.
pub type SampleBus = Broadcaster<SampleBlock>;

/// A single subscriber's receiving end of a [`SampleBus`]. Dropping this unregisters the
/// subscriber, so a client/pipeline detaching is a plain `drop` with no explicit bus-side
/// teardown call needed.
pub type SampleBusHandle = BroadcasterHandle<SampleBlock>;

#[cfg(test)]
mod tests {
    use super::*;

    fn block(start_sample: u64, tag: f32) -> SampleBlock {
        SampleBlock {
            start_sample,
            sample_rate_hz: 2_048_000,
            center_freq_hz: 433_000_000,
            samples: Arc::from(vec![Complex32::new(tag, 0.0); 4]),
        }
    }

    #[test]
    fn publish_fans_out_to_all_subscribers() {
        let bus = SampleBus::new();
        let a = bus.subscribe(8, OverflowPolicy::DropIncoming);
        let b = bus.subscribe(8, OverflowPolicy::DropIncoming);

        bus.publish(block(0, 1.0));

        let ra = a.try_recv().expect("subscriber a should have a block");
        let rb = b.try_recv().expect("subscriber b should have a block");
        assert_eq!(ra.start_sample, 0);
        assert_eq!(rb.start_sample, 0);
        assert_eq!(ra.samples[0].re, 1.0);
    }

    #[test]
    fn dropping_the_handle_unsubscribes() {
        let bus = SampleBus::new();
        let sub = bus.subscribe(4, OverflowPolicy::DropIncoming);
        assert_eq!(bus.subscriber_count(), 1);

        drop(sub);
        assert_eq!(bus.subscriber_count(), 0);

        // Publishing to zero subscribers must not panic.
        bus.publish(block(0, 0.0));
    }

    #[test]
    fn publish_clones_the_arc_not_the_samples() {
        let bus = SampleBus::new();
        let _a = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let _b = bus.subscribe(4, OverflowPolicy::DropIncoming);

        let payload: Arc<[Complex32]> = Arc::from(vec![Complex32::new(0.0, 0.0); 1024]);
        assert_eq!(Arc::strong_count(&payload), 1);

        bus.publish(SampleBlock {
            start_sample: 0,
            sample_rate_hz: 1,
            center_freq_hz: 1,
            samples: Arc::clone(&payload),
        });

        // The two subscriber queues plus our local `payload` binding all now share the
        // same allocation — three owners, zero copies of the 1024-sample buffer.
        assert_eq!(Arc::strong_count(&payload), 3);
    }

    #[test]
    fn recv_timeout_returns_none_when_idle_then_some_once_published() {
        let bus = SampleBus::new();
        let sub = bus.subscribe(4, OverflowPolicy::DropIncoming);

        let timed_out = sub.recv_timeout(std::time::Duration::from_millis(20));
        assert!(timed_out.is_none());

        bus.publish(block(7, 1.0));
        let received = sub
            .recv_timeout(std::time::Duration::from_millis(20))
            .expect("block published just before this call should be there");
        assert_eq!(received.start_sample, 7);
    }

    #[test]
    fn drop_oldest_keeps_only_the_newest_block_under_a_slow_consumer() {
        let bus = SampleBus::new();
        let sub = bus.subscribe(1, OverflowPolicy::DropOldest);

        bus.publish(block(0, 1.0));
        bus.publish(block(1, 2.0));
        bus.publish(block(2, 3.0));

        let received = sub.try_recv().expect("expected the newest block");
        assert_eq!(received.start_sample, 2);
        assert!(sub.try_recv().is_none(), "only one block should be queued");
        assert!(sub.dropped_count() >= 2);
    }

    #[test]
    fn concurrent_subscribe_and_publish_does_not_panic_or_deadlock() {
        use std::thread;

        let bus = SampleBus::new();
        let publisher_bus = bus.clone();
        let publisher = thread::spawn(move || {
            for i in 0..2_000u64 {
                publisher_bus.publish(block(i, i as f32));
            }
        });

        let mut subscriber_threads = Vec::new();
        for _ in 0..4 {
            let bus = bus.clone();
            subscriber_threads.push(thread::spawn(move || {
                let handle = bus.subscribe(16, OverflowPolicy::DropOldest);
                let mut seen = 0;
                for _ in 0..500 {
                    if handle.try_recv().is_some() {
                        seen += 1;
                    }
                }
                seen
            }));
        }

        publisher.join().unwrap();
        for t in subscriber_threads {
            t.join().unwrap();
        }
    }
}
