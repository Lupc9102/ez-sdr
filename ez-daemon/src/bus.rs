//! Lock-free, non-blocking fan-out of wideband sample blocks from the single ingestion
//! thread to every interested subscriber (channelizer taps, recorders, ...).
//!
//! Publishing is O(subscribers) non-blocking `try_send` calls — never a mutex, never a
//! blocking send — so one slow or stalled subscriber (e.g. a heavy LRPT decode pipeline
//! that falls behind) can never stall the realtime ingestion thread or any other
//! subscriber. Subscriber registration/deregistration is comparatively rare (client
//! attach/detach, pipeline start/stop) and goes through an `ArcSwap<Vec<Subscriber>>`
//! updated via `rcu`, so the hot publish path never takes a lock — it does one atomic load
//! and iterates the resulting snapshot.
//!
//! Each [`SampleBlock`] carries its sample payload behind an `Arc<[Complex32]>`, so fanning
//! a block out to N subscribers is N pointer/refcount clones, never an N x copy of the
//! underlying samples.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use num_complex::Complex32;

/// A block of consecutive wideband IQ samples, tagged with enough metadata for a
/// subscriber to reconstruct absolute time/frequency without consulting the daemon state.
#[derive(Debug, Clone)]
pub struct SampleBlock {
    pub start_sample: u64,
    pub sample_rate_hz: u32,
    pub center_freq_hz: u64,
    pub samples: Arc<[Complex32]>,
}

/// What happens when a subscriber's queue is full and another block arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Evict the oldest queued block to make room for the newest one. Correct for realtime
    /// visualizers (spectrum/waterfall): only the freshest data is ever useful, so a stale
    /// queued block is strictly worse than no block.
    DropOldest,
    /// Leave the queue untouched and drop the incoming block instead, counting it. Correct
    /// for pipelines that must never reorder or skip-ahead in their input (LRPT frame sync,
    /// ADS-B message parsing) — under sustained overload they fall behind and lose the
    /// tail, but never see samples out of order.
    DropIncoming,
}

struct Subscriber {
    id: u64,
    tx: Sender<SampleBlock>,
    /// A second handle onto the same bounded queue, used only to evict the head when
    /// [`OverflowPolicy::DropOldest`] finds the queue full. Racing with the subscriber's
    /// own `recv` here is harmless: crossbeam's channel is MPMC, so the eviction either
    /// pops the stale entry itself or discovers the subscriber already drained it (in
    /// which case the retried `try_send` below simply finds room) — either way the queue
    /// converges on "holds the newest block," which is the only guarantee this policy
    /// makes.
    evict_rx: Receiver<SampleBlock>,
    policy: OverflowPolicy,
    dropped: Arc<AtomicU64>,
}

impl Clone for Subscriber {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            tx: self.tx.clone(),
            evict_rx: self.evict_rx.clone(),
            policy: self.policy,
            dropped: Arc::clone(&self.dropped),
        }
    }
}

struct SampleBusInner {
    subscribers: ArcSwap<Vec<Subscriber>>,
    next_id: AtomicU64,
}

/// Cheaply cloneable handle to the bus. The ingestion thread holds one and calls
/// [`SampleBus::publish`]; any number of pipeline threads hold clones and call
/// [`SampleBus::subscribe`].
#[derive(Clone)]
pub struct SampleBus {
    inner: Arc<SampleBusInner>,
}

impl Default for SampleBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SampleBus {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SampleBusInner {
                subscribers: ArcSwap::from_pointee(Vec::new()),
                next_id: AtomicU64::new(0),
            }),
        }
    }

    /// Registers a new subscriber with a bounded queue of `capacity` blocks and the given
    /// overflow policy. Returns a [`SampleBusHandle`] the caller uses to receive blocks;
    /// dropping the handle unsubscribes automatically.
    #[must_use]
    pub fn subscribe(&self, capacity: usize, policy: OverflowPolicy) -> SampleBusHandle {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = bounded(capacity.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let sub = Subscriber {
            id,
            tx,
            evict_rx: rx.clone(),
            policy,
            dropped: Arc::clone(&dropped),
        };

        self.inner.subscribers.rcu(|current| {
            let mut next = (**current).clone();
            next.push(sub.clone());
            next
        });

        SampleBusHandle {
            id,
            rx,
            dropped,
            inner: Arc::clone(&self.inner),
        }
    }

    /// Fans `block` out to every current subscriber. Never blocks: a full queue is handled
    /// per the subscriber's [`OverflowPolicy`] and a disconnected subscriber (handle
    /// already dropped) is silently skipped — its entry is reaped lazily by
    /// [`SampleBusHandle::drop`], not here, so `publish` never pays removal cost.
    pub fn publish(&self, block: SampleBlock) {
        let subscribers = self.inner.subscribers.load();
        for sub in subscribers.iter() {
            match sub.tx.try_send(block.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(rejected)) => match sub.policy {
                    OverflowPolicy::DropOldest => {
                        let _ = sub.evict_rx.try_recv();
                        let _ = sub.tx.try_send(rejected);
                        sub.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    OverflowPolicy::DropIncoming => {
                        sub.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                },
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.inner.subscribers.load().len()
    }

    fn unsubscribe(&self, id: u64) {
        self.inner.subscribers.rcu(|current| {
            current
                .iter()
                .filter(|s| s.id != id)
                .cloned()
                .collect::<Vec<_>>()
        });
    }
}

/// A single subscriber's receiving end of the bus. Dropping this unregisters the
/// subscriber, so a client/pipeline detaching is a plain `drop` with no explicit
/// bus-side teardown call needed.
pub struct SampleBusHandle {
    id: u64,
    rx: Receiver<SampleBlock>,
    dropped: Arc<AtomicU64>,
    inner: Arc<SampleBusInner>,
}

impl SampleBusHandle {
    /// Blocks until a block is available or the bus itself is gone.
    pub fn recv(&self) -> Option<SampleBlock> {
        self.rx.recv().ok()
    }

    /// Returns immediately with `None` if no block is queued.
    pub fn try_recv(&self) -> Option<SampleBlock> {
        self.rx.try_recv().ok()
    }

    /// How many blocks this subscriber has lost to its overflow policy since subscribing.
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for SampleBusHandle {
    fn drop(&mut self) {
        SampleBus {
            inner: Arc::clone(&self.inner),
        }
        .unsubscribe(self.id);
    }
}

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
    fn drop_incoming_preserves_order_and_counts_losses() {
        let bus = SampleBus::new();
        let sub = bus.subscribe(2, OverflowPolicy::DropIncoming);

        for i in 0..5u64 {
            bus.publish(block(i, i as f32));
        }

        // Capacity 2, never evicted, so the two OLDEST blocks are what remain queued.
        let first = sub.try_recv().expect("first queued block");
        let second = sub.try_recv().expect("second queued block");
        assert_eq!(first.start_sample, 0);
        assert_eq!(second.start_sample, 1);
        assert!(sub.try_recv().is_none());
        assert_eq!(sub.dropped_count(), 3);
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
