//! Bounded, repaint-independent native 978 MHz UAT capture consumer.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

use crate::capture_stream::CaptureStream;
use crate::source_manager::CaptureBlock;
use crate::uat_receiver::{from_native_message, TimedReport};

const EXPECTED_CENTER_HZ: u64 = 978_000_000;
const MAX_TRACKS: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Configuration {
    enabled: bool,
    reset_revision: u64,
}

#[derive(Clone, Default)]
pub(crate) struct LocalUatSnapshot {
    configuration: Configuration,
    pub generation: u64,
    pub epoch: Option<u64>,
    pub discontinuity_revision: u64,
    pub reports: Vec<(u64, TimedReport)>,
    pub receiver: ez_proto::ReceiverSnapshot,
    pub sample_rate_mismatch: bool,
    pub center_mismatch: bool,
    pub received_samples: u64,
    pub lost_samples: u64,
    pub dropped_blocks: u64,
    pub status: Option<String>,
}

struct DecoderState {
    decoder: uat_decode::UatDecoder,
    configuration: Configuration,
    snapshot: LocalUatSnapshot,
    next_stream_sample: u64,
    tracks: HashMap<u32, (u64, TimedReport)>,
    version: u64,
    rate_window: ez_proto::MessageRateWindow,
}

impl DecoderState {
    fn new() -> Self {
        Self {
            decoder: uat_decode::UatDecoder::new(uat_decode::SAMPLE_RATE_HZ)
                .expect("UAT decoder uses its published sample rate"),
            configuration: Configuration::default(),
            snapshot: LocalUatSnapshot::default(),
            next_stream_sample: 0,
            tracks: HashMap::new(),
            version: 0,
            rate_window: ez_proto::MessageRateWindow::default(),
        }
    }

    fn configure(&mut self, configuration: &Configuration) -> bool {
        if self.configuration == *configuration {
            return false;
        }
        if self.configuration.enabled != configuration.enabled
            || self.configuration.reset_revision != configuration.reset_revision
        {
            self.decoder.reset();
            self.tracks.clear();
            self.rate_window = ez_proto::MessageRateWindow::default();
            self.snapshot.receiver = ez_proto::ReceiverSnapshot::default();
            self.snapshot.epoch = None;
            self.snapshot.discontinuity_revision += 1;
        }
        self.configuration = configuration.clone();
        if !configuration.enabled {
            self.snapshot.status = Some("Native UAT capture is paused".into());
        } else {
            self.snapshot.status = None;
        }
        true
    }

    fn ingest(&mut self, block: &CaptureBlock) -> bool {
        let new_generation = self.snapshot.generation != block.source_generation;
        if new_generation {
            self.decoder = uat_decode::UatDecoder::new(uat_decode::SAMPLE_RATE_HZ)
                .expect("UAT decoder uses its published sample rate");
            self.snapshot = LocalUatSnapshot {
                generation: block.source_generation,
                received_samples: self.snapshot.received_samples,
                lost_samples: self.snapshot.lost_samples,
                dropped_blocks: self.snapshot.dropped_blocks,
                ..Default::default()
            };
            self.next_stream_sample = 0;
            self.tracks.clear();
            self.rate_window = ez_proto::MessageRateWindow::default();
        }

        let sample_count = block
            .float_samples
            .as_ref()
            .map_or(block.samples.len() / 2, |samples| samples.len());
        let missing = block
            .stream_sample_index
            .saturating_sub(self.next_stream_sample);
        self.snapshot.lost_samples = self.snapshot.lost_samples.saturating_add(missing);
        self.snapshot.received_samples = self
            .snapshot
            .received_samples
            .saturating_add(sample_count as u64);
        self.next_stream_sample = block
            .stream_sample_index
            .saturating_add(sample_count as u64);

        let rate_matches = block.clock.sample_stride != 0
            && u64::from(block.clock.source_sample_rate_hz)
                == u64::from(uat_decode::SAMPLE_RATE_HZ) * u64::from(block.clock.sample_stride);
        let center_matches = block.center_hz == EXPECTED_CENTER_HZ;
        self.snapshot.sample_rate_mismatch = !rate_matches;
        self.snapshot.center_mismatch = !center_matches;
        let epoch_changed = self.snapshot.epoch != Some(block.clock.epoch);
        let boundary = new_generation || epoch_changed || block.clock.discontinuous || missing > 0;
        if epoch_changed {
            self.decoder.reset();
            self.snapshot.epoch = Some(block.clock.epoch);
        }
        if boundary {
            self.decoder.reset();
            self.snapshot.discontinuity_revision += 1;
        }

        if !self.configuration.enabled {
            self.snapshot.status = Some("Native UAT capture is paused".into());
            return boundary;
        }
        if !rate_matches {
            self.snapshot.status = Some(format!(
                "Native UAT requires {} samples/s (capture is {}/{})",
                uat_decode::SAMPLE_RATE_HZ,
                block.clock.source_sample_rate_hz,
                block.clock.sample_stride
            ));
            return boundary;
        }
        if !center_matches {
            self.snapshot.status = Some(format!(
                "Tune the capture to 978 MHz (currently {:.3} MHz)",
                block.center_hz as f64 / 1_000_000.0
            ));
            return boundary;
        }
        self.snapshot.status = None;

        let normalized = block.float_samples.as_ref().map_or_else(
            || lrpt_decode::uc8_bytes_to_normalized(&block.samples),
            |samples| samples.to_vec(),
        );
        let (messages, gap) = self.decoder.push_at(block.stream_sample_index, &normalized);
        if gap.is_some() && !boundary {
            self.snapshot.discontinuity_revision += 1;
        }
        let now = Instant::now();
        let had_messages = !messages.is_empty();
        let mut total = self.snapshot.receiver.total_messages;
        let mut stats = self.snapshot.receiver.decode_stats;
        for message in messages {
            total = total.saturating_add(1);
            stats.0 = stats.0.saturating_add(1);
            let tail_samples = block
                .stream_sample_index
                .saturating_add(sample_count as u64)
                .saturating_sub(message.first_sample);
            let sample_age = Duration::from_secs_f64(
                tail_samples as f64 / f64::from(uat_decode::SAMPLE_RATE_HZ),
            );
            let received = block.observed_at.checked_sub(sample_age).unwrap_or(now);
            if let Some(report) = from_native_message(&message, received) {
                stats.1 = stats.1.saturating_add(1);
                self.version = self.version.wrapping_add(1).max(1);
                self.tracks
                    .insert(report.report.key, (self.version, report));
                if self.tracks.len() > MAX_TRACKS {
                    if let Some(oldest) = self
                        .tracks
                        .iter()
                        .min_by_key(|(_, (_, report))| report.received)
                        .map(|(key, _)| *key)
                    {
                        self.tracks.remove(&oldest);
                    }
                }
            } else {
                stats.2 = stats.2.saturating_add(1);
            }
        }
        self.rate_window.observe_total(total, now);
        self.snapshot.receiver = ez_proto::ReceiverSnapshot {
            timestamp_ms: ez_proto::unix_time_ms(),
            total_messages: total,
            decode_stats: stats,
            messages_per_second: self.rate_window.rate_at(now),
        };
        boundary || had_messages
    }

    fn snapshot(&mut self, dropped_blocks: u64) -> LocalUatSnapshot {
        self.snapshot.dropped_blocks = dropped_blocks;
        self.snapshot.configuration = self.configuration.clone();
        self.snapshot.reports = self.tracks.values().cloned().collect();
        self.snapshot.clone()
    }
}

pub(crate) struct LocalUatWorker {
    configuration: Arc<ArcSwap<Configuration>>,
    output: Arc<ArcSwap<LocalUatSnapshot>>,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LocalUatWorker {
    pub fn new(stream: CaptureStream) -> Self {
        let input = stream.subscribe(128);
        let configuration = Arc::new(ArcSwap::from_pointee(Configuration::default()));
        let output = Arc::new(ArcSwap::from_pointee(LocalUatSnapshot::default()));
        let running = Arc::new(AtomicBool::new(true));
        let worker_configuration = Arc::clone(&configuration);
        let worker_output = Arc::clone(&output);
        let worker_running = Arc::clone(&running);
        let thread = std::thread::Builder::new()
            .name("native-uat-decoder".into())
            .spawn(move || {
                let mut state = DecoderState::new();
                let mut published = Instant::now();
                while worker_running.load(Ordering::Acquire) {
                    let changed = state.configure(&worker_configuration.load());
                    let block = input.recv_timeout(Duration::from_millis(20));
                    let changed = state.configure(&worker_configuration.load()) || changed;
                    let boundary = block.as_ref().is_some_and(|block| state.ingest(block));
                    if changed || boundary || published.elapsed() >= Duration::from_millis(16) {
                        worker_output.store(Arc::new(state.snapshot(input.dropped_count())));
                        published = Instant::now();
                    }
                }
            })
            .expect("native UAT worker thread creation failed");
        Self {
            configuration,
            output,
            running,
            thread: Some(thread),
        }
    }

    pub fn configure(&self, enabled: bool) {
        let previous = self.configuration.load_full();
        let configuration = Configuration {
            enabled,
            reset_revision: previous.reset_revision,
        };
        if *previous != configuration {
            self.configuration.store(Arc::new(configuration));
        }
    }

    pub fn snapshot(&self) -> Option<Arc<LocalUatSnapshot>> {
        let snapshot = self.output.load_full();
        (snapshot.configuration == **self.configuration.load()).then_some(snapshot)
    }
}

impl Drop for LocalUatWorker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_manager::SourceMode;
    use num_complex::Complex32;

    const DOWNLINK_SYNC: u64 = 0xEACD_DA4E2;

    fn official_sample_iq() -> Vec<Complex32> {
        // First 34-byte downlink from FlightAware's published sample-data.txt.gz.
        // Parity was generated independently with dump978's upstream encoder.
        let codeword = hex("08a66ef1353e2d525fd4050911882aa038101d06b85d440be2a4c2a0000590000000d0e3c7ccb1fed50a5afd9d6aa963");
        let mut bits = (0..36)
            .map(|i| (DOWNLINK_SYNC >> (35 - i)) & 1 != 0)
            .collect::<Vec<_>>();
        bits.extend(
            codeword
                .iter()
                .flat_map(|byte| (0..8).map(move |index| byte & (1 << (7 - index)) != 0)),
        );
        let mut phase = 0.0f32;
        let mut iq = Vec::with_capacity(bits.len() * 2);
        for bit in bits {
            for _ in 0..2 {
                iq.push(Complex32::from_polar(0.8, phase));
                phase += if bit { 0.2 } else { -0.2 };
            }
        }
        iq
    }

    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn block(
        samples: Vec<Complex32>,
        generation: u64,
        offset: u64,
        epoch: u64,
        rate: u32,
        center: u64,
    ) -> CaptureBlock {
        let mut block = CaptureBlock::fixture(
            vec![127; samples.len() * 2],
            ez_proto::CaptureClock::new(epoch, offset, rate, None),
            generation,
            offset,
        );
        block.center_hz = center;
        block.float_samples = Some(Arc::from(samples));
        block.source_mode = SourceMode::Hardware;
        block
    }

    #[test]
    fn worker_state_decodes_official_sample_across_arbitrary_capture_partitions() {
        let iq = official_sample_iq();
        let mut state = DecoderState::new();
        state.configure(&Configuration {
            enabled: true,
            ..Default::default()
        });
        let mut offset = 0usize;
        for size in [1usize, 17, 31, 2, 113, 5, 97, 251, 89, 1000] {
            if offset == iq.len() {
                break;
            }
            let end = (offset + size).min(iq.len());
            let samples = iq[offset..end].to_vec();
            state.ingest(&block(
                samples,
                9,
                offset as u64,
                9,
                uat_decode::SAMPLE_RATE_HZ,
                EXPECTED_CENTER_HZ,
            ));
            offset = end;
        }
        if offset < iq.len() {
            state.ingest(&block(
                iq[offset..].to_vec(),
                9,
                offset as u64,
                9,
                uat_decode::SAMPLE_RATE_HZ,
                EXPECTED_CENTER_HZ,
            ));
        }
        let snapshot = state.snapshot(0);
        assert_eq!(snapshot.receiver.total_messages, 1);
        assert_eq!(snapshot.receiver.decode_stats.1, 1);
        assert_eq!(snapshot.reports.len(), 1);
        let report = &snapshot.reports[0].1.report;
        assert_eq!(report.key, 0xa66ef1);
        assert_eq!(report.callsign.as_deref(), Some("N5130E"));
        assert_eq!(report.position, Some((37.43639, 118.95972)));
        assert_eq!(report.altitude_ft, Some(975));
    }

    #[test]
    fn background_capture_subscriber_decodes_the_published_iq_fixture() {
        let stream = CaptureStream::default();
        let worker = LocalUatWorker::new(stream.clone());
        worker.configure(true);
        stream.start(9);
        let iq = official_sample_iq();
        let mut offset = 0usize;
        for size in [1usize, 17, 31, 2, 113, 5, 97, 251, 89, 1000] {
            if offset == iq.len() {
                break;
            }
            let end = (offset + size).min(iq.len());
            let mut capture = block(
                iq[offset..end].to_vec(),
                9,
                offset as u64,
                9,
                uat_decode::SAMPLE_RATE_HZ,
                EXPECTED_CENTER_HZ,
            );
            capture.session = Some(stream.session());
            stream.publish(&capture);
            offset = end;
        }
        if offset < iq.len() {
            let mut capture = block(
                iq[offset..].to_vec(),
                9,
                offset as u64,
                9,
                uat_decode::SAMPLE_RATE_HZ,
                EXPECTED_CENTER_HZ,
            );
            capture.session = Some(stream.session());
            stream.publish(&capture);
        }
        stream.session().finish();

        let deadline = Instant::now() + Duration::from_secs(3);
        let snapshot = loop {
            if let Some(snapshot) = worker
                .snapshot()
                .filter(|snapshot| snapshot.receiver.total_messages == 1)
            {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "native UAT worker did not decode IQ"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(snapshot.generation, 9);
        assert_eq!(snapshot.receiver.decode_stats.1, 1);
        assert_eq!(snapshot.reports.len(), 1);
        assert_eq!(
            snapshot.reports[0].1.report.callsign.as_deref(),
            Some("N5130E")
        );
        assert_eq!(snapshot.dropped_blocks, 0);
    }

    #[test]
    fn clock_tuning_and_sample_gaps_are_reported_and_reset_decoder_state() {
        let mut state = DecoderState::new();
        state.configure(&Configuration {
            enabled: true,
            ..Default::default()
        });
        state.ingest(&block(
            vec![Complex32::new(0.0, 0.0); 32],
            2,
            0,
            2,
            2_400_000,
            1_090_000_000,
        ));
        assert!(state.snapshot.sample_rate_mismatch);
        assert!(state.snapshot.center_mismatch);
        assert!(state
            .snapshot
            .status
            .as_deref()
            .unwrap()
            .contains("requires"));

        state.ingest(&block(
            vec![Complex32::new(0.0, 0.0); 8],
            2,
            40,
            2,
            uat_decode::SAMPLE_RATE_HZ,
            EXPECTED_CENTER_HZ,
        ));
        assert!(!state.snapshot.sample_rate_mismatch);
        assert!(!state.snapshot.center_mismatch);
        assert_eq!(state.snapshot.lost_samples, 8);
        assert_eq!(state.next_stream_sample, 48);
        assert!(state.snapshot.discontinuity_revision >= 2);
    }
}
