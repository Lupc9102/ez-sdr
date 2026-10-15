//! Continuous local ADS-B ingestion. The UI reads a replaceable model snapshot;
//! its repaint rate never determines how many RF samples reach the decoder.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

use crate::adsb_decoder::AdsBDecoder;
use crate::adsb_panel::AircraftEntry;
use crate::capture_stream::{CaptureProgress, CaptureSession, CaptureStream};
use crate::source_manager::{CaptureBlock, SourceMode};

#[derive(Clone, Debug, Default, PartialEq)]
struct Configuration {
    enabled: bool,
    receiver_position: Option<(f64, f64)>,
    reset_revision: u64,
}

#[derive(Clone, Default)]
pub(crate) struct LocalAdsbSnapshot {
    configuration: Configuration,
    pub generation: u64,
    pub epoch: Option<u64>,
    pub discontinuity_revision: u64,
    pub aircraft: Vec<AircraftEntry>,
    pub receiver: ez_proto::ReceiverSnapshot,
    pub sample_rate_mismatch: bool,
    pub received_samples: u64,
    pub lost_samples: u64,
    pub total_received_samples: u64,
    pub total_lost_samples: u64,
    pub source_samples: u64,
    pub dropped_blocks: u64,
    pub complete: bool,
}

struct DecoderState {
    decoder: AdsBDecoder,
    configuration: Configuration,
    snapshot: LocalAdsbSnapshot,
    next_stream_sample: u64,
    session: Option<Arc<CaptureSession>>,
}

impl DecoderState {
    fn new() -> Self {
        Self {
            decoder: AdsBDecoder::new(),
            configuration: Configuration::default(),
            snapshot: LocalAdsbSnapshot::default(),
            next_stream_sample: 0,
            session: None,
        }
    }

    fn configure(&mut self, configuration: &Configuration) -> bool {
        if self.configuration == *configuration {
            return false;
        }
        if self.configuration.enabled != configuration.enabled
            || self.configuration.reset_revision != configuration.reset_revision
        {
            self.decoder = AdsBDecoder::new();
            self.snapshot.epoch = None;
            self.snapshot.discontinuity_revision += 1;
        }
        self.configuration = configuration.clone();
        true
    }

    fn ingest(&mut self, block: &CaptureBlock) -> bool {
        if self.snapshot.generation != block.source_generation {
            if let Some(session) = &self.session {
                // FIFO order guarantees there are no older accepted blocks
                // behind the first block from a new source generation.
                self.account_progress(&session.progress(), 0, self.snapshot.dropped_blocks);
            }
            self.decoder = AdsBDecoder::new();
            self.snapshot = LocalAdsbSnapshot {
                generation: block.source_generation,
                total_received_samples: self.snapshot.total_received_samples,
                total_lost_samples: self.snapshot.total_lost_samples,
                dropped_blocks: self.snapshot.dropped_blocks,
                ..Default::default()
            };
            self.next_stream_sample = 0;
            self.session.clone_from(&block.session);
        }
        let missing = block
            .stream_sample_index
            .saturating_sub(self.next_stream_sample);
        self.snapshot.lost_samples += missing;
        self.snapshot.total_lost_samples += missing;
        self.snapshot.received_samples += (block.samples.len() / 2) as u64;
        self.snapshot.total_received_samples += (block.samples.len() / 2) as u64;
        self.next_stream_sample = block.stream_sample_index + (block.samples.len() / 2) as u64;
        self.snapshot.complete = false;

        let boundary = self.snapshot.epoch != Some(block.clock.epoch)
            || block.clock.discontinuous
            || missing > 0;
        if self.snapshot.epoch != Some(block.clock.epoch) {
            self.decoder = AdsBDecoder::new();
            self.snapshot.epoch = Some(block.clock.epoch);
        }
        if boundary {
            self.snapshot.discontinuity_revision += 1;
        }
        let observer = (block.source_mode == SourceMode::Hardware)
            .then_some(self.configuration.receiver_position)
            .flatten();
        let (lat, lon) = observer.unwrap_or((f64::NAN, f64::NAN));
        self.decoder.set_receiver_position(lat, lon);
        if self.configuration.enabled
            && block.center_hz > 1_088_000_000
            && block.center_hz < 1_092_000_000
        {
            let mut clock = block.clock;
            clock.discontinuous |= missing > 0;
            // All local native sources, including CF32 replay conversion,
            // publish CU8. User-selected file formats never reinterpret this bus.
            self.decoder.feed_iq_with_clock(
                &block.samples,
                clock,
                dump1090::demod::InputFormat::Uc8,
            );
        }
        boundary
    }

    fn account_progress(&mut self, progress: &CaptureProgress, queued: usize, drops: u64) -> bool {
        self.snapshot.dropped_blocks = drops;
        if progress.generation == 0 || progress.generation != self.snapshot.generation {
            return false;
        }
        self.snapshot.source_samples = progress.samples;
        if !progress.live && queued == 0 && !self.snapshot.complete {
            let tail = progress.samples.saturating_sub(self.next_stream_sample);
            self.snapshot.lost_samples += tail;
            self.snapshot.total_lost_samples += tail;
            self.next_stream_sample = progress.samples;
            self.snapshot.complete = true;
            if tail > 0 {
                self.decoder.mark_discontinuous();
                self.snapshot.discontinuity_revision += 1;
            }
            return true;
        }
        false
    }

    fn account_finished_stream(&mut self, stream: &CaptureStream, queued: usize) -> bool {
        let current = stream.progress();
        if current.generation == 0 || current.live || queued > 0 {
            return false;
        }
        let changed = current.generation != self.snapshot.generation;
        if changed {
            // A whole short generation can be lost while an older queue is
            // full. The aggregate publisher count accounts for those samples
            // even when no accepted block retained that generation's session.
            self.decoder = AdsBDecoder::new();
            self.snapshot = LocalAdsbSnapshot {
                generation: current.generation,
                source_samples: current.samples,
                lost_samples: current.samples,
                total_received_samples: self.snapshot.total_received_samples,
                total_lost_samples: self.snapshot.total_lost_samples,
                dropped_blocks: self.snapshot.dropped_blocks,
                complete: true,
                ..Default::default()
            };
        }
        let lost = stream
            .total_samples()
            .saturating_sub(self.snapshot.total_received_samples);
        let new_loss = lost != self.snapshot.total_lost_samples;
        self.snapshot.total_lost_samples = lost;
        if new_loss {
            self.snapshot.discontinuity_revision += 1;
        }
        changed || new_loss
    }

    fn snapshot(&mut self) -> LocalAdsbSnapshot {
        self.snapshot.configuration = self.configuration.clone();
        self.snapshot.aircraft = self.decoder.get_aircraft();
        self.snapshot.receiver = self.decoder.receiver_snapshot();
        self.snapshot.sample_rate_mismatch = self.decoder.sample_rate_mismatch();
        self.snapshot.clone()
    }
}

pub(crate) struct LocalAdsbWorker {
    configuration: Arc<ArcSwap<Configuration>>,
    output: Arc<ArcSwap<LocalAdsbSnapshot>>,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LocalAdsbWorker {
    pub fn new(stream: CaptureStream) -> Self {
        // ~0.87 s at 2.4 MS/s and 16,384-sample reads. A CPU stall can
        // overflow this bound; native indices and final progress account for it.
        let input = stream.subscribe(128);
        let configuration = Arc::new(ArcSwap::from_pointee(Configuration::default()));
        let output = Arc::new(ArcSwap::from_pointee(LocalAdsbSnapshot::default()));
        let running = Arc::new(AtomicBool::new(true));
        let worker_configuration = Arc::clone(&configuration);
        let worker_output = Arc::clone(&output);
        let worker_running = Arc::clone(&running);
        let thread = std::thread::spawn(move || {
            let mut state = DecoderState::new();
            let mut published = Instant::now();
            while worker_running.load(Ordering::Acquire) {
                let changed = state.configure(&worker_configuration.load());
                let block = input.recv_timeout(Duration::from_millis(20));
                // Apply controls again after the wait, so a change without IQ
                // cannot leave the first subsequent block using stale settings.
                let changed = state.configure(&worker_configuration.load()) || changed;
                let boundary = block.as_ref().is_some_and(|block| state.ingest(block));
                let progress = state
                    .session
                    .as_ref()
                    .map_or_else(|| stream.progress(), |session| session.progress());
                let finished =
                    state.account_progress(&progress, input.queued_len(), input.dropped_count());
                let finished =
                    state.account_finished_stream(&stream, input.queued_len()) || finished;
                if changed
                    || boundary
                    || finished
                    || published.elapsed() >= Duration::from_millis(16)
                {
                    worker_output.store(Arc::new(state.snapshot()));
                    published = Instant::now();
                }
            }
        });
        Self {
            configuration,
            output,
            running,
            thread: Some(thread),
        }
    }

    pub fn configure(&self, enabled: bool, position: Option<(f64, f64)>) {
        let receiver_position = position.filter(|&(lat, lon)| {
            lat.is_finite()
                && lon.is_finite()
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lon)
        });
        let previous = self.configuration.load_full();
        let configuration = Configuration {
            enabled,
            receiver_position,
            reset_revision: previous.reset_revision,
        };
        if *previous != configuration {
            self.configuration.store(Arc::new(configuration));
        }
    }

    pub fn reset(&self) {
        let mut configuration = (**self.configuration.load()).clone();
        configuration.reset_revision = configuration.reset_revision.wrapping_add(1);
        self.configuration.store(Arc::new(configuration));
    }

    pub fn snapshot(&self) -> Option<Arc<LocalAdsbSnapshot>> {
        let snapshot = self.output.load_full();
        (snapshot.configuration == **self.configuration.load()).then_some(snapshot)
    }
}

impl Drop for LocalAdsbWorker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::source_manager::SourceManager;

    fn wait_for_snapshot(worker: &LocalAdsbWorker) -> Arc<LocalAdsbSnapshot> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(snapshot) = worker.snapshot().filter(|snapshot| snapshot.complete) {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "capture consumer did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(crate) fn crossing_block() -> Vec<u8> {
        let waveform = ez_daemon::hardware::adsb::encode_uc8(&[
            ez_daemon::hardware::adsb::EXAMPLE_DF17_CALLSIGN,
        ]);
        let mut block = vec![127; 16_384 * 2];
        let start = block.len() - 400;
        for (index, byte) in waveform.into_iter().enumerate() {
            let position = (start + index) % block.len();
            block[position] = byte;
        }
        block
    }

    #[test]
    fn paced_2400_capture_decodes_cross_block_frames_during_preview_stall() {
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Hardware;
        source.frequency_hz = 1_090_000_000;
        source.sample_rate_hz = 2_400_000;
        let block = crossing_block();
        source.test_set_hardware_open(move |_| Ok(block.clone()));
        let stream = source.capture_stream();
        let worker = LocalAdsbWorker::new(stream.clone());
        worker.configure(true, None);
        source.start();
        let deadline = Instant::now() + Duration::from_secs(5);
        while stream.progress().samples == 0 {
            assert!(Instant::now() < deadline, "capture source did not start");
            std::thread::sleep(Duration::from_millis(1));
        }
        // No GUI reads at all. This fills the legacy preview queue in ~55 ms.
        std::thread::sleep(Duration::from_millis(800));
        // Loaded test runners may schedule the paced fixture late. Keep the
        // undrained preview stalled until the original sample threshold too.
        while stream.progress().samples < 1_500_000 {
            assert!(
                Instant::now() < deadline,
                "capture did not reach its sample target"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        source.stop();
        source.wait_for_shutdown();
        let snapshot = wait_for_snapshot(&worker);
        let progress = stream.progress();
        assert!(progress.samples >= 1_500_000);
        assert_eq!(snapshot.generation, source.stream_generation());
        assert_eq!(snapshot.received_samples, progress.samples);
        assert_eq!(snapshot.source_samples, progress.samples);
        assert_eq!(snapshot.lost_samples, 0);
        assert_eq!(snapshot.dropped_blocks, 0);
        assert_eq!(snapshot.receiver.total_messages, progress.blocks - 1);
        assert_eq!(snapshot.aircraft.len(), 1);
        assert_eq!(snapshot.aircraft[0].callsign, "KLM 63EW");
        println!(
            "A09 stalled preview: {} samples, {} blocks, {} cross-block messages, {} lost samples",
            progress.samples,
            progress.blocks,
            snapshot.receiver.total_messages,
            snapshot.lost_samples,
        );
    }

    #[test]
    fn terminal_progress_accounts_for_missing_tail_exactly_once() {
        let mut state = DecoderState::new();
        state.snapshot.generation = 4;
        state.snapshot.received_samples = 16;
        state.next_stream_sample = 16;
        let progress = CaptureProgress {
            generation: 4,
            samples: 24,
            blocks: 3,
            live: false,
        };
        assert!(!state.account_progress(&progress, 1, 1));
        assert!(state.account_progress(&progress, 0, 1));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.lost_samples, 8);
        assert_eq!(
            snapshot.received_samples + snapshot.lost_samples,
            snapshot.source_samples
        );
        assert_eq!(snapshot.dropped_blocks, 1);
        assert!(snapshot.complete);
        assert!(!state.account_progress(&progress, 0, 1));
        assert_eq!(state.snapshot().lost_samples, 8);
    }

    #[test]
    fn bounded_bus_loss_resets_real_decode_and_survives_restart_with_a_lost_tail() {
        let stream = CaptureStream::default();
        let input = stream.subscribe(1);
        stream.start(10);
        let old_session = stream.session();
        let raw = ez_daemon::hardware::adsb::encode_uc8(&[
            ez_daemon::hardware::adsb::EXAMPLE_DF17_CALLSIGN,
        ]);
        let clock = ez_proto::CaptureClock::new(10, 0, 2_400_000, None);
        let publish = |samples: Vec<u8>, offset: u64, generation: u64, epoch: u64| {
            let mut block = CaptureBlock::fixture(
                samples,
                ez_proto::CaptureClock {
                    epoch,
                    source_sample_index: offset,
                    ..clock
                },
                generation,
                offset,
            );
            block.session = Some(stream.session());
            stream.publish(&block);
        };
        let mut state = DecoderState::new();
        state.configure(&Configuration {
            enabled: true,
            ..Default::default()
        });
        let split = 470; // middle of the real frame's payload
        publish(raw[..split].to_vec(), 0, 10, 10);
        publish(raw[split..split + 80].to_vec(), (split / 2) as u64, 10, 10);
        state.ingest(&input.try_recv().unwrap());
        publish(
            raw[split + 80..].to_vec(),
            ((split + 80) / 2) as u64,
            10,
            10,
        );
        state.ingest(&input.try_recv().unwrap());
        assert_eq!(
            state.snapshot().receiver.total_messages,
            0,
            "never stitch a frame across loss"
        );
        assert_eq!(state.snapshot.lost_samples, 40);
        let samples = (raw.len() / 2) as u64;
        publish(raw.clone(), samples, 10, 10);
        publish(vec![127; 16], samples * 2, 10, 10); // lost final 8 samples
        old_session.finish();
        let old_total = old_session.progress().samples;
        stream.start(11);
        publish(vec![127; 32], 0, 11, 11); // new generation's first 16 samples also overflow
        state.ingest(&input.try_recv().unwrap());
        assert_eq!(state.snapshot().receiver.total_messages, 1);
        publish(raw.clone(), 16, 11, 11);
        state.ingest(&input.try_recv().unwrap());
        stream.session().finish();
        state.account_progress(&stream.progress(), 0, input.dropped_count());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.generation, 11);
        assert_eq!(snapshot.receiver.total_messages, 1);
        assert_eq!(snapshot.lost_samples, 16);
        assert_eq!(snapshot.total_lost_samples, 40 + 8 + 16);
        assert_eq!(snapshot.dropped_blocks, 3);
        assert_eq!(
            snapshot.total_received_samples + snapshot.total_lost_samples,
            old_total + stream.progress().samples
        );
        assert!(snapshot.complete);
    }

    #[test]
    fn completely_lost_generations_remain_in_final_sample_accounting() {
        let stream = CaptureStream::default();
        let input = stream.subscribe(1);
        stream.start(1);
        let old_session = stream.session();
        let publish = |generation: u64, offset: u64, count: usize| {
            let mut block = CaptureBlock::fixture(
                vec![127; count * 2],
                ez_proto::CaptureClock::new(generation, offset, 2_400_000, None),
                generation,
                offset,
            );
            block.session = Some(stream.session());
            stream.publish(&block);
        };
        publish(1, 0, 16);
        publish(1, 16, 8);
        old_session.finish();
        stream.start(2);
        publish(2, 0, 8);
        stream.session().finish();
        stream.start(3);
        publish(3, 0, 4);
        stream.session().finish();
        let mut state = DecoderState::new();
        state.ingest(&input.try_recv().unwrap());
        state.account_progress(&old_session.progress(), 0, input.dropped_count());
        assert!(state.account_finished_stream(&stream, 0));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.generation, 3);
        assert_eq!(snapshot.received_samples, 0);
        assert_eq!(snapshot.lost_samples, 4);
        assert_eq!(snapshot.total_received_samples, 16);
        assert_eq!(snapshot.total_lost_samples, 20);
        assert_eq!(snapshot.dropped_blocks, 3);
        assert_eq!(
            snapshot.total_received_samples + snapshot.total_lost_samples,
            stream.total_samples()
        );
        assert!(!state.account_finished_stream(&stream, 0));
    }
}
