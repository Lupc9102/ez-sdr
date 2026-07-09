//! Generic lock-free multi-consumer fan-out, shared by every daemon-to-many-subscribers path
//! (wideband sample blocks, spectrum frames, audio frames, telemetry frames, ...).
//!
//! Publishing is O(subscribers) non-blocking `try_send` calls — never a mutex, never a
//! blocking send — so one slow or stalled subscriber (e.g. a heavy LRPT decode pipeline that
//! falls behind, or a GUI client on a slow link) can never stall the publisher or any other
//! subscriber. Subscriber registration/deregistration is comparatively rare (client
//! attach/detach, pipeline start/stop) and goes through an `ArcSwap<Vec<Subscriber<T>>>`
//! updated via `rcu`, so the hot publish path never takes a lock — it does one atomic load
//! and iterates the resulting snapshot.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};

/// What happens when a subscriber's queue is full and another value arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Evict the oldest queued value to make room for the newest one. Correct for realtime
    /// consumers (spectrum/waterfall, live audio): only the freshest data is ever useful, so
    /// a stale queued value is strictly worse than no value.
    DropOldest,
    /// Leave the queue untouched and drop the incoming value instead, counting it. Correct
    /// for consumers that must never reorder or skip-ahead in their input (LRPT frame sync,
    /// ADS-B message parsing) — under sustained overload they fall behind and lose the tail,
    /// but never see values out of order.
    DropIncoming,
}

struct Subscriber<T> {
    id: u64,
    tx: Sender<T>,
    /// A second handle onto the same bounded queue, used only to evict the head when
    /// [`OverflowPolicy::DropOldest`] finds the queue full. Racing with the subscriber's own
    /// `recv` here is harmless: crossbeam's channel is MPMC, so the eviction either pops the
    /// stale entry itself or discovers the subscriber already drained it (in which case the
    /// retried `try_send` below simply finds room) — either way the queue converges on
    /// "holds the newest value," which is the only guarantee this policy makes.
    evict_rx: Receiver<T>,
    policy: OverflowPolicy,
    dropped: Arc<AtomicU64>,
}

impl<T> Clone for Subscriber<T> {
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

struct BroadcasterInner<T> {
    subscribers: ArcSwap<Vec<Subscriber<T>>>,
    next_id: AtomicU64,
}

/// Cheaply cloneable handle to the fan-out. The producer thread holds one and calls
/// [`Broadcaster::publish`]; any number of consumer threads hold clones and call
/// [`Broadcaster::subscribe`].
pub struct Broadcaster<T> {
    inner: Arc<BroadcasterInner<T>>,
}

impl<T> Clone for Broadcaster<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Default for Broadcaster<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Broadcaster<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BroadcasterInner {
                subscribers: ArcSwap::from_pointee(Vec::new()),
                next_id: AtomicU64::new(0),
            }),
        }
    }

    /// Registers a new subscriber with a bounded queue of `capacity` values and the given
    /// overflow policy. Returns a [`BroadcasterHandle`] the caller uses to receive values;
    /// dropping the handle unsubscribes automatically.
    #[must_use]
    pub fn subscribe(&self, capacity: usize, policy: OverflowPolicy) -> BroadcasterHandle<T> {
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

        BroadcasterHandle {
            id,
            rx,
            dropped,
            inner: Arc::clone(&self.inner),
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

impl<T: Clone> Broadcaster<T> {
    /// Fans `value` out to every current subscriber. Never blocks: a full queue is handled
    /// per the subscriber's [`OverflowPolicy`] and a disconnected subscriber (handle already
    /// dropped) is silently skipped — its entry is reaped lazily by
    /// [`BroadcasterHandle`]'s `Drop`, not here, so `publish` never pays removal cost.
    pub fn publish(&self, value: T) {
        let subscribers = self.inner.subscribers.load();
        for sub in subscribers.iter() {
            match sub.tx.try_send(value.clone()) {
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
}

/// A single subscriber's receiving end of the fan-out. Dropping this unregisters the
/// subscriber, so a client/pipeline detaching is a plain `drop` with no explicit
/// broadcaster-side teardown call needed.
pub struct BroadcasterHandle<T> {
    id: u64,
    rx: Receiver<T>,
    dropped: Arc<AtomicU64>,
    inner: Arc<BroadcasterInner<T>>,
}

impl<T> BroadcasterHandle<T> {
    /// Blocks until a value is available. Note this subscriber's own sending half lives
    /// inside its `Subscriber` entry (removed only by this handle's own `Drop`), so unlike a
    /// plain channel this never observes "disconnected" just because every [`Broadcaster`]
    /// clone on the publish side went away — it simply waits for the next `publish` call,
    /// which may never come if the producer has stopped. Callers that need to exit a wait
    /// loop on shutdown should use [`Self::recv_timeout`] against an external stop signal
    /// instead of relying on this returning `None`.
    pub fn recv(&self) -> Option<T> {
        self.rx.recv().ok()
    }

    /// Returns immediately with `None` if no value is queued.
    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Blocks until a value is available or `timeout` elapses, whichever comes first.
    /// Intended for run loops that must periodically poll an external shutdown signal
    /// instead of blocking on [`Self::recv`] forever — see its doc comment for why this
    /// channel never disconnects on its own.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<T> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// How many values this subscriber has lost to its overflow policy since subscribing.
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl<T> Drop for BroadcasterHandle<T> {
    fn drop(&mut self) {
        let bcast = Broadcaster {
            inner: Arc::clone(&self.inner),
        };
        bcast.unsubscribe(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_fans_out_to_all_subscribers() {
        let bus = Broadcaster::new();
        let a = bus.subscribe(8, OverflowPolicy::DropIncoming);
        let b = bus.subscribe(8, OverflowPolicy::DropIncoming);

        bus.publish(1_i32);

        assert_eq!(a.try_recv(), Some(1));
        assert_eq!(b.try_recv(), Some(1));
    }

    #[test]
    fn drop_oldest_keeps_only_the_newest_value_under_a_slow_consumer() {
        let bus = Broadcaster::new();
        let sub = bus.subscribe(1, OverflowPolicy::DropOldest);

        bus.publish(1);
        bus.publish(2);
        bus.publish(3);

        assert_eq!(sub.try_recv(), Some(3));
        assert!(sub.try_recv().is_none(), "only one value should be queued");
        assert!(sub.dropped_count() >= 2);
    }

    #[test]
    fn drop_incoming_preserves_order_and_counts_losses() {
        let bus = Broadcaster::new();
        let sub = bus.subscribe(2, OverflowPolicy::DropIncoming);

        for i in 0..5 {
            bus.publish(i);
        }

        // Capacity 2, never evicted, so the two OLDEST values are what remain queued.
        assert_eq!(sub.try_recv(), Some(0));
        assert_eq!(sub.try_recv(), Some(1));
        assert!(sub.try_recv().is_none());
        assert_eq!(sub.dropped_count(), 3);
    }

    #[test]
    fn dropping_the_handle_unsubscribes() {
        let bus = Broadcaster::new();
        let sub = bus.subscribe(4, OverflowPolicy::DropIncoming);
        assert_eq!(bus.subscriber_count(), 1);

        drop(sub);
        assert_eq!(bus.subscriber_count(), 0);

        // Publishing to zero subscribers must not panic.
        bus.publish(0);
    }

    #[test]
    fn recv_timeout_returns_none_when_idle_then_some_once_published() {
        let bus = Broadcaster::new();
        let sub = bus.subscribe(4, OverflowPolicy::DropIncoming);

        assert!(sub.recv_timeout(Duration::from_millis(20)).is_none());

        bus.publish(42);
        assert_eq!(sub.recv_timeout(Duration::from_millis(20)), Some(42));
    }

    #[test]
    fn concurrent_subscribe_and_publish_does_not_panic_or_deadlock() {
        use std::thread;

        let bus: Broadcaster<u64> = Broadcaster::new();
        let publisher_bus = bus.clone();
        let publisher = thread::spawn(move || {
            for i in 0..2_000u64 {
                publisher_bus.publish(i);
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
