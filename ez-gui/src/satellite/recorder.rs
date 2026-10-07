use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Cf32RecordingMeta {
    pub frequency_hz: u64,
    pub sample_rate_hz: u32,
    pub format: String,
    pub amplitude: lrpt_decode::Cf32Amplitude,
    pub start_utc: String,
    pub end_utc: Option<String>,
    pub duration_sec: f64,
    pub bytes_total: u64,
    pub lost_samples: u64,
    pub capture_input_dropped_blocks: u64,
    pub capture_input_lost_samples: u64,
    pub complete: bool,
    pub capture: Option<ez_proto::RecordingCapture>,
    pub stop_reason: Option<String>,
    pub satellite: Option<String>,
    pub observer_lat: f64,
    pub observer_lon: f64,
}

pub struct Cf32StreamWriter {
    tx: Option<Sender<Vec<u8>>>,
    handle: Option<JoinHandle<Result<(), String>>>,
    path: PathBuf,
    start_time: Instant,
    bytes_written: Arc<AtomicU64>,
    offered_samples: u64,
    dropped_samples: u64,
    error: Arc<Mutex<Option<String>>>,
    meta: Cf32RecordingMeta,
    capture_recorder: Option<crate::capture_recording::CaptureRecorder<Cf32CaptureResult>>,
}

pub(crate) struct Cf32CaptureResult {
    meta: Cf32RecordingMeta,
    result: Result<(), String>,
}

impl Cf32StreamWriter {
    pub fn start(
        output_dir: &str,
        satellite: Option<String>,
        sample_rate: u32,
        center_freq_hz: u64,
        observer_lat: f64,
        observer_lon: f64,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;

        if sample_rate == 0 {
            return Err("Recording sample rate must be positive".into());
        }
        static SESSION: AtomicU64 = AtomicU64::new(0);
        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S_%f");
        let sat_tag: String = satellite
            .as_deref()
            .unwrap_or("SAT")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let freq_str = format!("{:.3}", center_freq_hz as f64 / 1e6);
        let filename = format!(
            "{}_{}_{}MHz_{}_{}.cf32",
            ts,
            sat_tag,
            freq_str,
            std::process::id(),
            SESSION.fetch_add(1, Ordering::Relaxed)
        );
        let path = PathBuf::from(output_dir).join(&filename);
        // Readiness means the file actually exists. Never truncate a prior capture.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("Could not create {}: {error}", path.display()))?;

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(64);

        let bytes_written = Arc::new(AtomicU64::new(0));
        let bytes_written_thread = Arc::clone(&bytes_written);

        let error = Arc::new(Mutex::new(None));
        let start_utc = chrono::Utc::now().to_rfc3339();

        let handle = spawn_writer(
            rx,
            BufWriter::with_capacity(1_048_576, file),
            bytes_written_thread,
            Arc::clone(&error),
        );

        let meta = Cf32RecordingMeta {
            frequency_hz: center_freq_hz,
            sample_rate_hz: sample_rate,
            format: "cf32_le".to_string(),
            amplitude: lrpt_decode::Cf32Amplitude::Normalized,
            start_utc,
            end_utc: None,
            duration_sec: 0.0,
            bytes_total: 0,
            lost_samples: 0,
            capture_input_dropped_blocks: 0,
            capture_input_lost_samples: 0,
            complete: false,
            capture: None,
            stop_reason: None,
            satellite,
            observer_lat,
            observer_lon,
        };

        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            path,
            start_time: Instant::now(),
            bytes_written,
            offered_samples: 0,
            dropped_samples: 0,
            error,
            meta,
            capture_recorder: None,
        })
    }

    pub(crate) fn with_capture(self, stream: crate::capture_stream::CaptureStream) -> Self {
        let mut frontend = Self {
            tx: None,
            handle: None,
            path: self.path.clone(),
            start_time: self.start_time,
            bytes_written: Arc::clone(&self.bytes_written),
            offered_samples: 0,
            dropped_samples: 0,
            error: Arc::clone(&self.error),
            meta: self.meta.clone(),
            capture_recorder: None,
        };
        frontend.capture_recorder = Some(crate::capture_recording::CaptureRecorder::start(
            stream, self, None,
        ));
        frontend
    }

    pub fn write_capture(
        &mut self,
        samples: &[u8],
        clock: ez_proto::CaptureClock,
        center_hz: u64,
    ) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        let rate = clock.source_sample_rate_hz / clock.sample_stride.max(1);
        let next = if center_hz != self.meta.frequency_hz || rate != self.meta.sample_rate_hz {
            Err(
                "Recording stopped: capture tuning or sample rate differs from the file header"
                    .into(),
            )
        } else {
            ez_proto::RecordingCapture::accept(
                self.meta.capture.as_ref(),
                clock,
                center_hz,
                rate,
                samples.len() as u64 / 2,
            )
        };
        let next = next.inspect_err(|reason: &String| {
            self.meta.stop_reason = Some(reason.clone());
            if let Ok(mut error) = self.error.lock() {
                if error.is_none() {
                    *error = Some(reason.clone());
                }
            }
        })?;
        self.write(samples)?;
        self.meta.capture = Some(next);
        Ok(())
    }

    pub fn write(&mut self, samples: &[u8]) -> Result<(), String> {
        let count = (samples.len() / 2) as u64;
        self.offered_samples = self.offered_samples.saturating_add(count);
        let result = if !samples.len().is_multiple_of(2) {
            Err("Recorder received an incomplete I/Q sample".into())
        } else if let Some(message) = self.error() {
            Err(message)
        } else if let Some(tx) = &self.tx {
            enqueue_chunk(tx, samples)
        } else {
            Err("Recorder is stopped".into())
        };
        if let Err(message) = &result {
            self.dropped_samples = self.dropped_samples.saturating_add(count);
            if let Ok(mut error) = self.error.lock() {
                if error.is_none() {
                    *error = Some(message.clone());
                }
            }
        }
        result
    }

    pub fn request_stop(&mut self) {
        if let Some(recorder) = &self.capture_recorder {
            recorder.request_stop();
        } else {
            self.tx.take();
        }
    }

    pub fn stop(&mut self) -> Result<Cf32RecordingMeta, String> {
        if let Some(mut recorder) = self.capture_recorder.take() {
            let result = recorder.finish()?;
            self.meta = result.meta;
            result.result?;
            return Ok(self.meta.clone());
        }
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let result = h
                .join()
                .unwrap_or_else(|_| Err("Recorder worker panicked".into()));
            if let Err(message) = result {
                if let Ok(mut error) = self.error.lock() {
                    if error.is_none() {
                        *error = Some(message);
                    }
                }
            }
        }
        self.meta.end_utc = Some(chrono::Utc::now().to_rfc3339());
        self.meta.bytes_total = std::fs::metadata(&self.path)
            .map_err(|error| error.to_string())?
            .len();
        self.bytes_written
            .store(self.meta.bytes_total, Ordering::Relaxed);
        self.meta.lost_samples = self
            .offered_samples
            .saturating_sub(self.meta.bytes_total / 8)
            .max(self.dropped_samples);
        if let Some(capture) = &mut self.meta.capture {
            capture.samples = self.meta.bytes_total / 8;
            capture.next_source_sample_index = capture
                .first_clock
                .offset_samples(capture.samples)
                .map_or(capture.first_clock.source_sample_index, |clock| {
                    clock.source_sample_index
                });
        }
        self.meta.duration_sec = self.meta.capture.as_ref().map_or(
            self.offered_samples as f64 / f64::from(self.meta.sample_rate_hz),
            |capture| {
                self.meta.bytes_total as f64 / 8.0 * f64::from(capture.first_clock.sample_stride)
                    / f64::from(capture.first_clock.source_sample_rate_hz)
            },
        );
        if self.meta.stop_reason.is_none() {
            self.meta.stop_reason = self.error();
        }
        self.meta.complete = self.error().is_none()
            && self.meta.lost_samples == 0
            && self.meta.bytes_total.is_multiple_of(8);

        // Write sidecar JSON
        let sidecar_path = self.path.with_extension("json");
        let json = serde_json::to_string_pretty(&self.meta).map_err(|error| error.to_string())?;
        std::fs::write(&sidecar_path, json)
            .map_err(|error| format!("Recorder metadata failed: {error}"))?;
        if let Some(error) = self.error() {
            return Err(error);
        }
        Ok(self.meta.clone())
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        if let Some(recorder) = &self.capture_recorder {
            return !recorder.progress().stopped && self.error().is_none();
        }
        self.tx.is_some()
            && self
                .handle
                .as_ref()
                .is_some_and(|handle| !handle.is_finished())
            && self.error().is_none()
    }

    pub fn error(&self) -> Option<String> {
        self.error
            .lock()
            .map(|error| error.clone())
            .unwrap_or_else(|_| Some("Recorder error state is poisoned".into()))
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    pub fn filename(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for Cf32StreamWriter {
    fn drop(&mut self) {
        if self.handle.is_some() || self.capture_recorder.is_some() {
            self.request_stop();
            let closed = Self {
                tx: None,
                handle: None,
                path: self.path.clone(),
                start_time: self.start_time,
                bytes_written: Arc::clone(&self.bytes_written),
                offered_samples: self.offered_samples,
                dropped_samples: self.dropped_samples,
                error: Arc::clone(&self.error),
                meta: self.meta.clone(),
                capture_recorder: None,
            };
            let mut writer = std::mem::replace(self, closed);
            std::thread::spawn(move || {
                let _ = writer.stop();
            });
        }
    }
}

impl crate::capture_recording::CaptureRecordingWriter for Cf32StreamWriter {
    type Final = Cf32CaptureResult;

    fn write_capture(&mut self, block: &crate::source_manager::CaptureBlock) -> Result<(), String> {
        Cf32StreamWriter::write_capture(self, &block.samples, block.clock, block.center_hz)
    }

    fn progress(&self) -> crate::capture_recording::RecordingProgress {
        crate::capture_recording::RecordingProgress {
            capture: self.meta.capture.clone(),
            bytes_written: self.bytes_written(),
            error: self.error(),
            stopped: false,
        }
    }

    fn fail(&mut self, reason: String) {
        self.meta.stop_reason.get_or_insert_with(|| reason.clone());
        if let Ok(mut error) = self.error.lock() {
            error.get_or_insert(reason);
        }
    }

    fn input_overflow(&mut self, blocks: u64, samples: u64) {
        self.meta.capture_input_dropped_blocks = blocks;
        self.meta.capture_input_lost_samples = samples;
    }

    fn finish(&mut self) -> Cf32CaptureResult {
        let result = self.stop().map(|_| ());
        Cf32CaptureResult {
            meta: self.meta.clone(),
            result,
        }
    }
}

fn enqueue_chunk(tx: &Sender<Vec<u8>>, samples: &[u8]) -> Result<(), String> {
    tx.try_send(samples.to_vec()).map_err(|error| match error {
        TrySendError::Full(_) => {
            "Recorder queue overflow: samples were lost; recording is incomplete".into()
        }
        TrySendError::Disconnected(_) => "Recorder worker stopped before accepting samples".into(),
    })
}

fn spawn_writer<W: Write + Send + 'static>(
    rx: Receiver<Vec<u8>>,
    writer: W,
    count: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
) -> JoinHandle<Result<(), String>> {
    std::thread::spawn(move || {
        let result = write_chunks(rx, writer, &count);
        if let Err(message) = &result {
            if let Ok(mut error) = error.lock() {
                if error.is_none() {
                    *error = Some(message.clone());
                }
            }
        }
        result
    })
}

fn write_chunks<W: Write>(
    rx: Receiver<Vec<u8>>,
    mut writer: W,
    bytes_written: &AtomicU64,
) -> Result<(), String> {
    let flush_interval = std::time::Duration::from_millis(100);
    let mut last_flush = Instant::now();
    while let Ok(chunk) = rx.recv() {
        let converted = raw_iq_bytes_to_cf32_le(&chunk);
        writer
            .write_all(&converted)
            .map_err(|error| format!("Recorder write failed: {error}"))?;
        bytes_written.fetch_add(converted.len() as u64, Ordering::Relaxed);
        if last_flush.elapsed() >= flush_interval {
            writer
                .flush()
                .map_err(|error| format!("Recorder flush failed: {error}"))?;
            last_flush = Instant::now();
        }
    }
    writer
        .flush()
        .map_err(|error| format!("Recorder flush failed: {error}"))
}

fn raw_iq_bytes_to_cf32_le(samples: &[u8]) -> Vec<u8> {
    lrpt_decode::uc8_bytes_to_cf32_le(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BlockedFile {
        file: std::fs::File,
        gate: Option<(Sender<()>, Receiver<()>)>,
        fail_flush: bool,
    }

    impl Write for BlockedFile {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if let Some((entered, release)) = self.gate.take() {
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
            self.file.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.fail_flush {
                Err(std::io::Error::other("injected CF32 final flush failure"))
            } else {
                self.file.flush()
            }
        }
    }

    #[test]
    fn blocked_cf32_stop_keeps_decode_handoff_pending_until_files_finish() {
        for (drop_panel, fail_flush) in [(false, false), (false, true), (true, false)] {
            let dir = temp_dir();
            let mut writer = Cf32StreamWriter::start(
                dir.to_str().unwrap(),
                Some("ISS".into()),
                2_400_000,
                1_090_000_000,
                0.0,
                0.0,
            )
            .unwrap();
            writer.tx.take();
            writer.handle.take().unwrap().join().unwrap().unwrap();
            let path = writer.path().to_path_buf();
            let (entered, entered_rx) = bounded(1);
            let (release, release_rx) = bounded(1);
            let (tx, rx) = bounded(64);
            writer.handle = Some(spawn_writer(
                rx,
                BlockedFile {
                    file: std::fs::OpenOptions::new()
                        .write(true)
                        .truncate(true)
                        .open(&path)
                        .unwrap(),
                    gate: Some((entered, release_rx)),
                    fail_flush,
                },
                Arc::clone(&writer.bytes_written),
                Arc::clone(&writer.error),
            ));
            writer.tx = Some(tx);
            let stream = crate::capture_stream::CaptureStream::default();
            stream.start(1);
            let mut panel = crate::satellite_panel::SatellitePanel::new(
                crate::test_helpers::make_shared_state(),
            );
            panel.cf32_writer = Some(writer.with_capture(stream.clone()));
            panel.cf32_recording = true;
            let raw = [90, 110].repeat(10);
            let mut block = crate::source_manager::CaptureBlock::fixture(
                raw.clone(),
                ez_proto::CaptureClock::new(9, 100, 2_400_000, None),
                1,
                0,
            );
            block.session = Some(stream.session());
            stream.publish(&block);
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            let started = Instant::now();
            panel.stop_cf32_recording();
            let elapsed = started.elapsed();
            assert!(
                elapsed < std::time::Duration::from_millis(50),
                "CF32 Stop blocked for {elapsed:?}"
            );
            assert!(panel.cf32_recording && panel.is_cf32_finalizing());
            assert!(panel.pending_decode_request.is_none());
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
            let mut panel = Some(panel);
            if drop_panel {
                let started = Instant::now();
                drop(panel.take());
                assert!(started.elapsed() < std::time::Duration::from_millis(50));
            }
            release.send(()).unwrap();
            if let Some(panel) = &mut panel {
                panel.stop_cf32_and_wait();
                assert!(!panel.cf32_recording);
                assert_eq!(panel.pending_decode_request.is_some(), !fail_flush);
                if fail_flush {
                    assert!(panel
                        .pending_status
                        .as_ref()
                        .unwrap()
                        .contains("injected CF32 final flush failure"));
                }
            }
            let deadline = Instant::now() + std::time::Duration::from_secs(3);
            let metadata = loop {
                if let Some(metadata) = std::fs::read(path.with_extension("json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                {
                    break metadata;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            assert_eq!(metadata["complete"], !fail_flush);
            assert_eq!(metadata["capture"]["samples"], 10);
            assert_eq!(metadata["bytes_total"], 80);
            assert_eq!(std::fs::read(path).unwrap(), raw_iq_bytes_to_cf32_le(&raw));
            eprintln!("Blocked CF32 Stop: {} us; drop={drop_panel}, flush_error={fail_flush}, finalized 10 RF samples", elapsed.as_micros());
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn native_cf32_disk_failures_finalize_the_actual_prefix_and_stop_promptly() {
        for flush_failure in [false, true] {
            let dir = temp_dir();
            let mut writer = Cf32StreamWriter::start(
                dir.to_str().unwrap(),
                None,
                2_400_000,
                1_090_000_000,
                0.0,
                0.0,
            )
            .unwrap();
            // Replace only the filesystem writer with a deterministic fault;
            // capture ingestion, conversion, metadata and shutdown stay real.
            writer.tx.take();
            writer.handle.take().unwrap().join().unwrap().unwrap();
            let file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(writer.path())
                .unwrap();
            let fault = crate::test_helpers::FaultFileWriter::new(
                file,
                (!flush_failure).then_some(16),
                flush_failure,
            );
            let (tx, rx) = bounded(64);
            writer.handle = Some(spawn_writer(
                rx,
                fault,
                Arc::clone(&writer.bytes_written),
                Arc::clone(&writer.error),
            ));
            writer.tx = Some(tx);
            let stream = crate::capture_stream::CaptureStream::default();
            stream.start(1);
            let mut writer = writer.with_capture(stream.clone());
            let raw = [90, 110].repeat(10);
            let mut block = crate::source_manager::CaptureBlock::fixture(
                raw.clone(),
                ez_proto::CaptureClock::new(9, 100, 2_400_000, None),
                1,
                0,
            );
            block.session = Some(stream.session());
            stream.publish(&block);
            let deadline = Instant::now() + std::time::Duration::from_secs(3);
            while if flush_failure {
                writer.bytes_written() == 0
            } else {
                writer.is_running()
            } {
                assert!(Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let stopped = Instant::now();
            let error = writer.stop().unwrap_err();
            let latency = stopped.elapsed();
            assert!(latency < std::time::Duration::from_millis(500));
            assert!(error.contains(if flush_failure {
                "flush failure"
            } else {
                "write failure"
            }));
            let converted = lrpt_decode::uc8_bytes_to_cf32_le(&raw);
            let expected = if flush_failure {
                converted.as_slice()
            } else {
                &converted[..16]
            };
            assert_eq!(std::fs::read(writer.path()).unwrap(), expected);
            assert!(!writer.meta.complete);
            assert_eq!(
                writer.meta.capture.as_ref().unwrap().samples,
                expected.len() as u64 / 8
            );
            assert_eq!(
                writer
                    .meta
                    .capture
                    .as_ref()
                    .unwrap()
                    .next_source_sample_index,
                100 + expected.len() as u64 / 8
            );
            assert_eq!(writer.meta.lost_samples, 10 - expected.len() as u64 / 8);
            let meta: serde_json::Value = serde_json::from_slice(
                &std::fs::read(writer.path().with_extension("json")).unwrap(),
            )
            .unwrap();
            assert_eq!(meta["bytes_total"], expected.len() as u64);
            assert_eq!(meta["complete"], false);
            println!(
                "A09 native CF32 {} failure: stop/join {} us; {} preserved bytes",
                if flush_failure { "flush" } else { "write" },
                latency.as_micros(),
                writer.meta.bytes_total
            );
            drop(writer);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn cf32_capture_finalizes_only_the_contiguous_prefix_before_retune_rate_rewind_or_loss() {
        for boundary in 0..5 {
            let dir = temp_dir();
            let mut writer =
                Cf32StreamWriter::start(dir.to_str().unwrap(), None, 2_400_000, 1090, 0.0, 0.0)
                    .unwrap();
            let mut clock = ez_proto::CaptureClock::new(5, 700, 16_800_003, None);
            clock.sample_stride = 7;
            clock.discontinuous = true;
            let raw = [90, 110, 140, 160].repeat(32);
            writer.write_capture(&raw, clock, 1090).unwrap();
            let mut next = clock.offset_samples(raw.len() as u64 / 2).unwrap();
            next.discontinuous = false;
            let mut center = 1090;
            match boundary {
                0 => {
                    next.epoch += 1;
                    next.source_sample_index = 0;
                }
                1 => center += 100,
                2 => next.source_sample_rate_hz = 14_000_000,
                3 => next.source_sample_index += 7,
                _ => next.discontinuous = true,
            }
            assert!(writer
                .write_capture(&[200, 210], next, center)
                .unwrap_err()
                .contains("Recording stopped"));
            assert!(!writer.is_running());
            assert!(writer.stop().unwrap_err().contains("Recording stopped"));
            assert_eq!(
                std::fs::read(writer.path()).unwrap(),
                lrpt_decode::uc8_bytes_to_cf32_le(&raw)
            );
            let metadata: serde_json::Value = serde_json::from_slice(
                &std::fs::read(writer.path().with_extension("json")).unwrap(),
            )
            .unwrap();
            assert_eq!(metadata["capture"]["samples"], raw.len() as u64 / 2);
            assert_eq!(
                metadata["capture"]["first_clock"]["source_sample_rate_hz"],
                16_800_003
            );
            assert_eq!(metadata["capture"]["first_clock"]["sample_stride"], 7);
            assert_eq!(metadata["complete"], false);
            assert_eq!(metadata["lost_samples"], 0); // boundary block was never accepted
            use ez_daemon::hardware::{
                replay::{FileReplaySource, ReplayFormat},
                IqSource,
            };
            let mut replay = FileReplaySource::open(
                writer.path().to_str().unwrap(),
                ReplayFormat::Cf32Le,
                2_400_000,
                false,
                1000.0,
            )
            .unwrap();
            replay.start().unwrap();
            let mut samples = vec![num_complex::Complex32::default(); raw.len() / 2];
            assert_eq!(replay.read_iq(&mut samples).unwrap(), samples.len());
            assert_eq!(samples, lrpt_decode::uc8_bytes_to_normalized(&raw));
            replay.stop();
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    fn temp_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "ez-cf32-audit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn rapid_sessions_preserve_captures_and_matching_sidecars() {
        let dir = temp_dir();
        let mut paths = Vec::new();
        for n in 1..=5 {
            let mut writer = Cf32StreamWriter::start(
                dir.to_str().unwrap(),
                Some("ISS / SSTV".into()),
                2_400_000,
                137_100_000,
                0.0,
                0.0,
            )
            .unwrap();
            assert!(writer.is_running());
            writer.write(&[90, 110].repeat(n)).unwrap();
            let meta = writer.stop().unwrap();
            assert!(meta.complete);
            assert_eq!(meta.lost_samples, 0);
            assert_eq!(meta.bytes_total, n as u64 * 8);
            assert!(!writer.is_running());
            let path = writer.path().to_owned();
            assert!(!paths.contains(&path));
            let sidecar: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap())
                    .unwrap();
            assert_eq!(sidecar["bytes_total"], n as u64 * 8);
            paths.push(path);
        }
        for (n, path) in paths.iter().enumerate() {
            assert_eq!(std::fs::metadata(path).unwrap().len(), (n + 1) as u64 * 8);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn open_and_sidecar_failures_are_returned_to_the_caller() {
        let dir = temp_dir();
        std::fs::write(&dir, b"not a directory").unwrap();
        assert!(
            Cf32StreamWriter::start(dir.to_str().unwrap(), None, 2_400_000, 1, 0.0, 0.0).is_err()
        );
        std::fs::remove_file(&dir).unwrap();
        assert!(Cf32StreamWriter::start(
            dir.to_str().unwrap(),
            Some("X".repeat(300)),
            2_400_000,
            1,
            0.0,
            0.0
        )
        .err()
        .unwrap()
        .contains("Could not create"));
        let mut writer =
            Cf32StreamWriter::start(dir.to_str().unwrap(), None, 2_400_000, 1, 0.0, 0.0).unwrap();
        writer.write(&[90, 110]).unwrap();
        std::fs::create_dir(writer.path().with_extension("json")).unwrap();
        assert!(writer.stop().unwrap_err().contains("metadata failed"));
        assert!(!writer.is_running());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn overflow_is_reported_and_lost_samples_survive_finalization() {
        let dir = temp_dir();
        let mut writer =
            Cf32StreamWriter::start(dir.to_str().unwrap(), None, 2_400_000, 1, 0.0, 0.0).unwrap();
        // A controlled full queue makes overflow independent of disk speed.
        let (tx, rx) = bounded(1);
        tx.send(vec![127, 127]).unwrap();
        let old_tx = writer.tx.replace(tx);
        drop(old_tx);
        assert!(writer
            .write(&[90, 110, 140, 160])
            .unwrap_err()
            .contains("overflow"));
        assert!(!writer.is_running());
        assert!(writer.stop().unwrap_err().contains("overflow"));
        assert_eq!(writer.meta.lost_samples, 2);
        assert!(!writer.meta.complete);
        drop(rx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    struct FailingWriter {
        write_error: bool,
        flush_error: bool,
    }
    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.write_error {
                Err(std::io::Error::other("injected disk write error"))
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.flush_error {
                Err(std::io::Error::other("injected disk flush error"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn worker_reports_write_and_flush_failures() {
        for (write_error, flush_error, expected) in
            [(true, false, "write failed"), (false, true, "flush failed")]
        {
            let (tx, rx) = bounded(1);
            tx.send(vec![90, 110]).unwrap();
            drop(tx);
            let count = AtomicU64::new(0);
            let error = write_chunks(
                rx,
                FailingWriter {
                    write_error,
                    flush_error,
                },
                &count,
            )
            .unwrap_err();
            assert!(error.contains(expected));
            assert_eq!(
                count.load(Ordering::Relaxed),
                if write_error { 0 } else { 8 }
            );
        }
    }

    #[test]
    fn exited_worker_is_not_running_and_stop_returns_its_failure() {
        let dir = temp_dir();
        let mut writer =
            Cf32StreamWriter::start(dir.to_str().unwrap(), None, 2_400_000, 1, 0.0, 0.0).unwrap();
        writer.tx.take();
        writer.handle.take().unwrap().join().unwrap().unwrap();
        let (tx, rx) = bounded(1);
        writer.tx = Some(tx);
        writer.handle = Some(spawn_writer(
            rx,
            FailingWriter {
                write_error: true,
                flush_error: false,
            },
            Arc::clone(&writer.bytes_written),
            Arc::clone(&writer.error),
        ));
        writer.write(&[90, 110]).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        while writer.is_running() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!writer.is_running());
        assert!(writer.error().unwrap().contains("write failed"));
        assert!(writer.stop().unwrap_err().contains("write failed"));
        assert_eq!(writer.meta.lost_samples, 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn converts_byte_pair_to_two_le_floats() {
        let out = raw_iq_bytes_to_cf32_le(&[0, 255]);
        assert_eq!(out.len(), 8);
        let i_val = f32::from_le_bytes([out[0], out[1], out[2], out[3]]);
        let q_val = f32::from_le_bytes([out[4], out[5], out[6], out[7]]);
        assert_eq!(i_val, -1.0);
        assert_eq!(q_val, 1.0);
    }

    #[test]
    fn output_length_is_4x_input_length() {
        let input = vec![10u8; 200];
        let out = raw_iq_bytes_to_cf32_le(&input);
        assert_eq!(out.len(), input.len() * 4);
    }

    #[test]
    fn odd_trailing_byte_is_dropped() {
        let out = raw_iq_bytes_to_cf32_le(&[1, 2, 3]);
        assert_eq!(out.len(), 8);
    }
}
