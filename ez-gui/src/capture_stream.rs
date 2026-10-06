//! The native capture data plane is independent of GUI repainting. Consumers
//! receive shared immutable blocks and account for their own bounded overflow.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use arc_swap::ArcSwap;
use ez_daemon::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};

use crate::source_manager::CaptureBlock;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureProgress {
    pub generation: u64,
    pub samples: u64,
    pub blocks: u64,
    pub live: bool,
}

#[derive(Clone, Default)]
pub(crate) struct CaptureStream {
    bus: Broadcaster<Arc<CaptureBlock>>,
    session: Arc<ArcSwap<CaptureSession>>,
    total_samples: Arc<AtomicU64>,
}

/// Queued blocks retain their worker's summary after a restart replaces the
/// current session. Final loss accounting therefore survives rapid retuning.
#[derive(Debug, Default)]
pub(crate) struct CaptureSession {
    progress: ArcSwap<CaptureProgress>,
}

impl CaptureSession {
    pub fn progress(&self) -> Arc<CaptureProgress> {
        self.progress.load_full()
    }

    pub fn finish(&self) {
        let mut progress = *self.progress();
        progress.live = false;
        self.progress.store(Arc::new(progress));
    }
}

impl CaptureStream {
    pub fn subscribe(&self, capacity: usize) -> BroadcasterHandle<Arc<CaptureBlock>> {
        self.bus.subscribe(capacity, OverflowPolicy::DropIncoming)
    }

    pub fn progress(&self) -> Arc<CaptureProgress> {
        self.session.load().progress()
    }

    pub fn has_consumers(&self) -> bool {
        self.bus.subscriber_count() > 0
    }

    pub fn total_samples(&self) -> u64 {
        self.total_samples.load(Ordering::Acquire)
    }

    pub fn session(&self) -> Arc<CaptureSession> {
        self.session.load_full()
    }

    pub fn start(&self, generation: u64) {
        self.session.store(Arc::new(CaptureSession {
            progress: ArcSwap::from_pointee(CaptureProgress {
                generation,
                live: true,
                ..Default::default()
            }),
        }));
    }

    pub fn publish(&self, block: &CaptureBlock) {
        let session = block
            .session
            .as_ref()
            .expect("published capture has a session");
        let previous = session.progress();
        self.total_samples
            .fetch_add((block.samples.len() / 2) as u64, Ordering::Release);
        self.bus.publish(Arc::new(block.clone()));
        session.progress.store(Arc::new(CaptureProgress {
            generation: block.source_generation,
            samples: block.stream_sample_index + (block.samples.len() / 2) as u64,
            blocks: previous.blocks + 1,
            live: true,
        }));
    }
}
