//! Native capture consumers own recording ingress, continuity and deadlines.
//! GUI code observes progress and requests Stop; it never forwards IQ bytes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

use crate::capture_stream::CaptureStream;
use crate::source_manager::CaptureBlock;

#[derive(Clone, Default)]
pub(crate) struct RecordingProgress {
    pub capture: Option<ez_proto::RecordingCapture>,
    pub bytes_written: u64,
    pub error: Option<String>,
    pub stopped: bool,
}

pub(crate) trait CaptureRecordingWriter: Send + 'static {
    type Final: Send + 'static;
    fn write_capture(&mut self, block: &CaptureBlock) -> Result<(), String>;
    fn progress(&self) -> RecordingProgress;
    fn fail(&mut self, error: String);
    fn input_overflow(&mut self, blocks: u64, samples: u64);
    fn source_ended(&mut self) {}
    fn finish(&mut self) -> Self::Final;
}

#[derive(Clone, Copy, Default)]
struct Control {
    stop_at: Option<Instant>,
    deadline: Option<Instant>,
}

impl Control {
    fn end(&self) -> Option<Instant> {
        match (self.stop_at, self.deadline) {
            (Some(stop), Some(deadline)) => Some(stop.min(deadline)),
            (stop, deadline) => stop.or(deadline),
        }
    }
}

pub(crate) struct CaptureRecorder<T: Send + 'static> {
    control: Arc<ArcSwap<Control>>,
    progress: Arc<ArcSwap<RecordingProgress>>,
    thread: Option<std::thread::JoinHandle<T>>,
}

impl<T: Send + 'static> CaptureRecorder<T> {
    pub fn start<W: CaptureRecordingWriter<Final = T>>(
        stream: CaptureStream,
        mut writer: W,
        deadline: Option<Instant>,
    ) -> Self {
        let input = stream.subscribe(128);
        let initial = stream.session();
        let mut owner = initial.progress().live.then_some(initial);
        let control = Arc::new(ArcSwap::from_pointee(Control {
            deadline,
            ..Default::default()
        }));
        let progress = Arc::new(ArcSwap::from_pointee(writer.progress()));
        let worker_control = Arc::clone(&control);
        let worker_progress = Arc::clone(&progress);
        let thread = std::thread::spawn(move || {
            let mut first = None;
            let mut published = Instant::now();
            let ingestion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
                let block = input.recv_timeout(Duration::from_millis(20));
                let end = worker_control.load().end();
                if let Some(block) = block {
                    if end.is_some_and(|end| block.observed_at > end) {
                        if owner.as_ref().is_some_and(|owner| !owner.progress().live) {
                            writer.source_ended();
                        }
                        break;
                    }
                    if owner.is_none() {
                        owner.clone_from(&block.session);
                    }
                    if owner.as_ref().is_some_and(|owner| {
                        block
                            .session
                            .as_ref()
                            .is_some_and(|session| !Arc::ptr_eq(session, owner))
                            || block.source_generation != owner.progress().generation
                    }) {
                        writer.source_ended();
                        break;
                    }
                    if first.is_none() {
                        first = block
                            .session
                            .clone()
                            .map(|session| (session, block.stream_sample_index));
                    }
                    if let Err(error) = writer.write_capture(&block) {
                        writer.fail(error);
                        break;
                    }
                } else if end.is_some_and(|end| Instant::now() >= end) {
                    if owner.as_ref().is_some_and(|owner| !owner.progress().live) {
                        writer.source_ended();
                    }
                    break;
                } else if owner.as_ref().is_some_and(|owner| !owner.progress().live) {
                    // A timeout means the accepted session's queue is drained.
                    // EOF needs no subsequent block to trigger finalization.
                    writer.source_ended();
                    break;
                }
                if input.dropped_count() > 0 {
                    break;
                }
                if let Some(error) = writer.progress().error {
                    writer.fail(error);
                    break;
                }
                if published.elapsed() >= Duration::from_millis(16) {
                    worker_progress.store(Arc::new(writer.progress()));
                    published = Instant::now();
                }
            }));
            if ingestion.is_err() {
                writer.fail("Capture recording worker panicked".into());
            }
            let dropped = input.dropped_count();
            if dropped > 0 {
                let accepted = writer
                    .progress()
                    .capture
                    .map_or(0, |capture| capture.samples);
                let missing = first.as_ref().map_or(0, |(session, start)| {
                    session
                        .progress()
                        .samples
                        .saturating_sub(*start)
                        .saturating_sub(accepted)
                });
                writer.input_overflow(dropped, missing);
                writer.fail("Recording stopped: capture input queue overflow".into());
            }
            // Unsubscribe before disk flushing: the source and other consumers
            // keep running while this file's accepted prefix is finalized.
            drop(input);
            let result = writer.finish();
            let mut progress = writer.progress();
            progress.stopped = true;
            worker_progress.store(Arc::new(progress));
            result
        });
        Self {
            control,
            progress,
            thread: Some(thread),
        }
    }

    pub fn progress(&self) -> Arc<RecordingProgress> {
        self.progress.load_full()
    }

    pub fn set_deadline(&self, deadline: Option<Instant>) {
        let previous = self.control.load();
        if previous.deadline != deadline {
            self.control.store(Arc::new(Control {
                stop_at: previous.stop_at,
                deadline,
            }));
        }
    }

    pub fn request_stop(&self) {
        let previous = self.control.load();
        if previous.stop_at.is_none() {
            self.control.store(Arc::new(Control {
                stop_at: Some(Instant::now()),
                deadline: previous.deadline,
            }));
        }
    }

    pub fn finish(&mut self) -> Result<T, String> {
        self.request_stop();
        self.thread
            .take()
            .ok_or_else(|| "Recording already finalized".to_string())?
            .join()
            .map_err(|_| "Capture recording worker panicked".to_string())
    }
}

impl<T: Send + 'static> Drop for CaptureRecorder<T> {
    fn drop(&mut self) {
        self.request_stop();
        // The owned worker still finalizes its files after the UI releases its
        // observer. Call finish() on a background task to collect the result.
    }
}
