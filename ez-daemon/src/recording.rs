//! Daemon-owned recording manager: writes a virtual channel's complex sample stream to disk
//! in either `Cf32` or `RawU8` format, keyed by [`ChannelId`].
//!
//! Unlike the `pipelines/` modules (each a single-input, single-thread `tick`/`run` consumer
//! of one fixed [`SampleBusHandle`]), a recording manager fans over a dynamic, changing set
//! of channels: recordings start and stop independently of one another and of any client's
//! connection lifetime. Each active recording gets its own dedicated OS thread pulling
//! directly from that channel's broadcaster subscription and writing to its own file, so one
//! recording's disk latency can never stall another recording or the pipeline feeding it —
//! there is no shared polling loop to starve.
//!
//! Because each recording subscribes directly to the channel's `Broadcaster<SampleBlock>`
//! output (obtained once, up front, via `Channelizer::subscribe`) rather than being driven
//! by a per-client message loop, a recording keeps running across client attach/detach
//! cycles exactly like every other pipeline.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use num_complex::Complex32;

use ez_proto::{ChannelId, RecordingFormat, RecordingStatus};

use crate::bus::SampleBusHandle;

const RECV_POLL_INTERVAL: Duration = Duration::from_millis(200);

struct RecordingMetadata {
    path: PathBuf,
    json: serde_json::Value,
    stop_reason: Arc<Mutex<Option<String>>>,
}

struct ActiveRecording {
    handle: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    path: PathBuf,
    start_time: Instant,
    bytes_written: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
    stop_reason: Arc<Mutex<Option<String>>>,
}

impl ActiveRecording {
    fn is_alive(&self) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(h) = &self.handle {
            if h.is_finished() {
                return false;
            }
        }
        if let Ok(guard) = self.error.lock() {
            if guard.is_some() {
                return false;
            }
        }
        true
    }

    fn status(&self, channel_id: ChannelId, active: bool) -> RecordingStatus {
        RecordingStatus {
            channel_id,
            stop_reason: self
                .stop_reason
                .lock()
                .ok()
                .and_then(|reason| reason.clone())
                .or_else(|| self.error.lock().ok().and_then(|error| error.clone())),
            active: active && self.is_alive(),
            path: Some(self.path.display().to_string()),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            duration_sec: self.start_time.elapsed().as_secs_f64(),
        }
    }

    /// Signals the writer thread to stop and waits for it to flush and exit.
    fn join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ActiveRecording {
    fn drop(&mut self) {
        self.join();
    }
}

/// Owns zero or more active per-channel recordings, each written by its own thread.
pub struct RecordingManager {
    output_dir: PathBuf,
    active: HashMap<ChannelId, ActiveRecording>,
}

impl RecordingManager {
    #[must_use]
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            output_dir: output_dir.into(),
            active: HashMap::new(),
        }
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active.values().filter(|r| r.is_alive()).count()
    }

    /// Starts recording `channel_id`'s sample stream (pulled from `input`) to a new file in
    /// `format`. Fails if `channel_id` is already recording, or the output file can't be
    /// created; the file is created synchronously so a failure is reported immediately
    /// rather than surfacing only inside the writer thread.
    pub fn start_recording(
        &mut self,
        channel_id: ChannelId,
        format: RecordingFormat,
        center_freq_hz: u64,
        input: SampleBusHandle,
    ) -> Result<()> {
        if self
            .active
            .get(&channel_id)
            .is_some_and(ActiveRecording::is_alive)
        {
            return Err(anyhow!("channel {channel_id} is already recording"));
        }
        if let Some(mut ended) = self.active.remove(&channel_id) {
            ended.join();
        }

        std::fs::create_dir_all(&self.output_dir).with_context(|| {
            format!(
                "creating recording output dir {}",
                self.output_dir.display()
            )
        })?;

        let path = self
            .output_dir
            .join(recording_filename(channel_id, center_freq_hz, format));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("creating recording file {}", path.display()))?;
        let metadata_path = PathBuf::from(format!("{}.json", path.display()));
        let metadata = serde_json::json!({
            "format": if format == RecordingFormat::Cf32 { "cf32_le" } else { "cu8" },
            "amplitude": if format == RecordingFormat::Cf32 { Some(lrpt_decode::Cf32Amplitude::Normalized) } else { None },
            "requested_frequency_hz": center_freq_hz,
            "frequency_hz": null, "sample_rate_hz": null, "capture": null,
            "complete": false, "stop_reason": null, "creator": "ez-daemon"
        });
        let metadata_result = (|| -> Result<()> {
            let mut sidecar = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&metadata_path)?;
            if let Err(error) = sidecar.write_all(&serde_json::to_vec_pretty(&metadata)?) {
                drop(sidecar);
                let _ = std::fs::remove_file(&metadata_path);
                return Err(error.into());
            }
            Ok(())
        })();
        if let Err(error) = metadata_result {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(error.context("writing recording metadata"));
        }
        let writer = BufWriter::with_capacity(1_048_576, file);

        let stop = Arc::new(AtomicBool::new(false));
        let bytes_written = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let thread_bytes = Arc::clone(&bytes_written);
        let thread_error = Arc::clone(&error);
        let stop_reason = Arc::new(Mutex::new(None));
        let thread_stop_reason = Arc::clone(&stop_reason);

        let handle = std::thread::Builder::new()
            .name(format!("ez-daemon-recorder-{channel_id}"))
            .spawn(move || {
                record_thread(
                    &input,
                    writer,
                    format,
                    &thread_stop,
                    &thread_bytes,
                    &thread_error,
                    Some(RecordingMetadata {
                        path: metadata_path,
                        json: metadata,
                        stop_reason: thread_stop_reason,
                    }),
                )
            })
            .with_context(|| format!("spawning recorder thread for channel {channel_id}"))?;

        self.active.insert(
            channel_id,
            ActiveRecording {
                handle: Some(handle),
                stop,
                path,
                start_time: Instant::now(),
                bytes_written,
                error,
                stop_reason,
            },
        );
        Ok(())
    }

    /// Stops `channel_id`'s active recording (if any), flushing and closing its file, and
    /// returns a final status snapshot.
    pub fn stop_recording(&mut self, channel_id: ChannelId) -> Result<RecordingStatus> {
        let mut recording = self
            .active
            .remove(&channel_id)
            .ok_or_else(|| anyhow!("channel {channel_id} is not recording"))?;
        recording.join();
        let error = recording.error.lock().ok().and_then(|error| error.clone());
        if let Some(error) = error {
            let path = recording.path.display().to_string();
            // Retain the terminal status for periodic client delivery even when
            // an explicit stop returns the writer's actual finalization error.
            self.active.insert(channel_id, recording);
            return Err(anyhow!("recording {path} failed: {error}"));
        }
        Ok(recording.status(channel_id, false))
    }

    pub(crate) fn forget_finished(&mut self, channel_id: ChannelId) {
        if self
            .active
            .get(&channel_id)
            .is_some_and(|recording| !recording.is_alive())
        {
            self.active.remove(&channel_id);
        }
    }

    #[must_use]
    pub fn status(&self, channel_id: ChannelId) -> Option<RecordingStatus> {
        self.active
            .get(&channel_id)
            .map(|r| r.status(channel_id, true))
    }

    #[must_use]
    pub fn statuses(&self) -> Vec<RecordingStatus> {
        self.active
            .iter()
            .map(|(id, r)| r.status(*id, true))
            .collect()
    }
}

fn record_thread(
    input: &SampleBusHandle,
    mut writer: BufWriter<File>,
    format: RecordingFormat,
    stop: &AtomicBool,
    bytes_written: &AtomicU64,
    error: &Mutex<Option<String>>,
    metadata: Option<RecordingMetadata>,
) {
    let mut capture: Option<ez_proto::RecordingCapture> = None;
    while !stop.load(Ordering::Relaxed) {
        let Some(block) = input.recv_timeout(RECV_POLL_INTERVAL) else {
            continue;
        };
        if block.samples.is_empty() {
            continue;
        }
        let next_capture = match ez_proto::RecordingCapture::accept(
            capture.as_ref(),
            block.capture_clock(),
            block.center_freq_hz,
            block.sample_rate_hz,
            block.samples.len() as u64,
        ) {
            Ok(capture) => capture,
            Err(reason) => {
                if let Some(metadata) = &metadata {
                    if let Ok(mut message) = metadata.stop_reason.lock() {
                        *message = Some(reason);
                    }
                } else if let Ok(mut message) = error.lock() {
                    *message = Some(reason);
                }
                break;
            }
        };
        let bytes = match format {
            RecordingFormat::Cf32 => complex_to_cf32_le_bytes(&block.samples),
            RecordingFormat::RawU8 => complex_to_uc8_bytes(&block.samples),
        };
        if let Err(e) = writer.write_all(&bytes) {
            if let Ok(mut guard) = error.lock() {
                *guard = Some(e.to_string());
            }
            break;
        }
        bytes_written.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        capture = Some(next_capture);
    }
    if let Err(failure) = writer.flush() {
        if let Ok(mut message) = error.lock() {
            if message.is_none() {
                *message = Some(format!("flush failed: {failure}"));
            }
        }
    }
    if let Some(RecordingMetadata {
        path,
        json: mut metadata,
        stop_reason,
    }) = metadata
    {
        let size = writer.get_ref().metadata().map(|file| file.len());
        let bytes = size.unwrap_or_else(|failure| {
            if let Ok(mut message) = error.lock() {
                if message.is_none() {
                    *message = Some(format!("file metadata failed: {failure}"));
                }
            }
            bytes_written.load(Ordering::Relaxed)
        });
        bytes_written.store(bytes, Ordering::Relaxed);
        let sample_bytes = if format == RecordingFormat::Cf32 {
            8
        } else {
            2
        };
        let intact = capture
            .as_ref()
            .is_some_and(|capture| capture.samples.saturating_mul(sample_bytes) == bytes);
        if let Some(capture) = &mut capture {
            capture.samples = bytes / sample_bytes;
            capture.next_source_sample_index = capture
                .first_clock
                .offset_samples(capture.samples)
                .map_or(capture.first_clock.source_sample_index, |clock| {
                    clock.source_sample_index
                });
            metadata["frequency_hz"] = capture.center_frequency_hz.into();
            metadata["sample_rate_hz"] = capture.sample_rate_hz.into();
            metadata["duration_sec"] = (capture.samples as f64
                * f64::from(capture.first_clock.sample_stride)
                / f64::from(capture.first_clock.source_sample_rate_hz))
            .into();
        }
        let reason = stop_reason
            .lock()
            .ok()
            .and_then(|message| message.clone())
            .or_else(|| error.lock().ok().and_then(|message| message.clone()));
        metadata["capture"] = serde_json::to_value(capture).unwrap_or(serde_json::Value::Null);
        metadata["bytes_total"] = bytes.into();
        metadata["complete"] =
            (reason.is_none() && intact && bytes.is_multiple_of(sample_bytes)).into();
        metadata["stop_reason"] = reason.into();
        if let Err(failure) = serde_json::to_vec_pretty(&metadata)
            .map_err(anyhow::Error::from)
            .and_then(|json| std::fs::write(&path, json).map_err(anyhow::Error::from))
        {
            if let Ok(mut message) = error.lock() {
                *message = Some(format!("recording metadata failed: {failure}"));
            }
        }
    }
}

fn recording_filename(
    channel_id: ChannelId,
    center_freq_hz: u64,
    format: RecordingFormat,
) -> String {
    let ext = match format {
        RecordingFormat::Cf32 => "cf32",
        RecordingFormat::RawU8 => "iq",
    };
    let freq_mhz = center_freq_hz as f64 / 1e6;
    static SESSION: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}_{}_{}_ch{channel_id}_{freq_mhz:.3}MHz.{ext}",
        now_ms(),
        std::process::id(),
        SESSION.fetch_add(1, Ordering::Relaxed)
    )
}

fn complex_to_cf32_le_bytes(samples: &[Complex32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 8);
    for s in samples {
        out.extend_from_slice(&s.re.to_le_bytes());
        out.extend_from_slice(&s.im.to_le_bytes());
    }
    out
}

fn complex_to_uc8_bytes(samples: &[Complex32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        out.push(component_to_uc8(s.re));
        out.push(component_to_uc8(s.im));
    }
    out
}

fn component_to_uc8(component: f32) -> u8 {
    lrpt_decode::normalized_component_to_uc8(component)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{OverflowPolicy, SampleBlock, SampleBus};

    fn block(samples: Vec<Complex32>) -> SampleBlock {
        SampleBlock {
            observed_at: None,
            capture: None,
            start_sample: 0,
            sample_rate_hz: 48_000,
            center_freq_hz: 100_000_000,
            samples: Arc::from(samples),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ez-daemon-recording-tests-{tag}-{}", now_ms()))
    }

    #[test]
    fn complex_to_cf32_le_bytes_round_trips_through_f32_le() {
        let samples = vec![Complex32::new(1.5, -2.5), Complex32::new(-0.25, 3.0)];
        let bytes = complex_to_cf32_le_bytes(&samples);
        assert_eq!(bytes.len(), 16);
        let re0 = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let im0 = f32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(re0, 1.5);
        assert_eq!(im0, -2.5);
    }

    #[test]
    fn complex_to_uc8_bytes_maps_normalized_range_to_full_dynamic_range() {
        let samples = vec![Complex32::new(-1.0, 1.0), Complex32::new(0.0, 0.0)];
        let bytes = complex_to_uc8_bytes(&samples);
        assert_eq!(bytes, vec![0, 255, 128, 128]);
    }

    #[test]
    fn component_to_uc8_clamps_out_of_range_values() {
        assert_eq!(component_to_uc8(1000.0), 255);
        assert_eq!(component_to_uc8(-1000.0), 0);
        assert_eq!(component_to_uc8(0.0), 128);
    }

    #[test]
    fn flush_failure_is_retained_and_reported_when_recording_stops() {
        let dir = temp_dir("flush-error");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("read-only-handle");
        std::fs::write(&path, b"preserved").unwrap();
        let mut writer = BufWriter::new(File::open(&path).unwrap());
        writer.write_all(b"pending samples").unwrap();
        let bus = SampleBus::new();
        let input = bus.subscribe(1, OverflowPolicy::DropOldest);
        let stop = Arc::new(AtomicBool::new(true));
        let bytes_written = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        record_thread(
            &input,
            writer,
            RecordingFormat::RawU8,
            &stop,
            &bytes_written,
            &error,
            None,
        );
        assert!(error
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("flush failed"));
        let mut manager = RecordingManager::new(&dir);
        manager.active.insert(
            42,
            ActiveRecording {
                handle: None,
                stop,
                path: path.clone(),
                start_time: Instant::now(),
                bytes_written,
                error,
                stop_reason: Arc::new(Mutex::new(None)),
            },
        );
        assert!(manager
            .stop_recording(42)
            .unwrap_err()
            .to_string()
            .contains("flush failed"));
        assert_eq!(std::fs::read(path).unwrap(), b"preserved");
        let terminal = manager.status(42).unwrap();
        assert!(!terminal.active);
        assert!(terminal.stop_reason.unwrap().contains("flush failed"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recording_reports_inactive_on_writer_error() {
        // Issue 22: If writer thread encounters an error, status must report active: false.
        let rec = ActiveRecording {
            handle: None,
            stop: Arc::new(AtomicBool::new(false)),
            path: PathBuf::from("/nonexistent/file.iq"),
            start_time: Instant::now(),
            bytes_written: Arc::new(AtomicU64::new(0)),
            error: Arc::new(Mutex::new(Some("disk full".to_string()))),
            stop_reason: Arc::new(Mutex::new(None)),
        };
        let status = rec.status(1, true);
        assert!(
            !status.active,
            "recording with error must report active = false"
        );
        assert!(!rec.is_alive());
    }

    #[test]
    fn start_recording_rejects_a_channel_already_recording() {
        let dir = temp_dir("dup");
        let mut mgr = RecordingManager::new(&dir);
        let bus = SampleBus::new();
        let h1 = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let h2 = bus.subscribe(4, OverflowPolicy::DropIncoming);

        mgr.start_recording(1, RecordingFormat::Cf32, 100_000_000, h1)
            .unwrap();
        let err = mgr
            .start_recording(1, RecordingFormat::Cf32, 100_000_000, h2)
            .unwrap_err();
        assert!(err.to_string().contains("already recording"));

        mgr.stop_recording(1).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_recording_on_an_unknown_channel_is_an_error() {
        let dir = temp_dir("unknown");
        let mut mgr = RecordingManager::new(&dir);
        let err = mgr.stop_recording(42).unwrap_err();
        assert!(err.to_string().contains("not recording"));
    }

    #[test]
    fn status_reflects_bytes_written_and_active_count() {
        let dir = temp_dir("status");
        let mut mgr = RecordingManager::new(&dir);
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);

        assert_eq!(mgr.active_count(), 0);
        assert!(mgr.status(1).is_none());

        mgr.start_recording(1, RecordingFormat::Cf32, 100_000_000, handle)
            .unwrap();
        assert_eq!(mgr.active_count(), 1);

        bus.publish(block(vec![Complex32::new(0.1, 0.2); 100]));

        // Give the writer thread a moment to drain the block and write it.
        let deadline = Instant::now() + Duration::from_secs(2);
        while mgr.status(1).map(|s| s.bytes_written).unwrap_or(0) == 0 && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }

        let status = mgr.status(1).expect("channel 1 is recording");
        assert!(status.active);
        assert_eq!(status.bytes_written, 100 * 8);

        let final_status = mgr.stop_recording(1).unwrap();
        assert!(!final_status.active);
        assert_eq!(mgr.active_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recording_file_contains_exactly_the_written_cf32_bytes() {
        let dir = temp_dir("file-contents");
        let mut mgr = RecordingManager::new(&dir);
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);

        mgr.start_recording(7, RecordingFormat::Cf32, 137_500_000, handle)
            .unwrap();
        bus.publish(block(vec![Complex32::new(1.0, -1.0); 4]));

        let deadline = Instant::now() + Duration::from_secs(2);
        while mgr.status(7).map(|s| s.bytes_written).unwrap_or(0) == 0 && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }

        let status = mgr.stop_recording(7).unwrap();
        let path = status.path.expect("path recorded");
        let contents = std::fs::read(&path).unwrap();
        assert_eq!(contents.len(), 4 * 8);
        assert_eq!(f32::from_le_bytes(contents[0..4].try_into().unwrap()), 1.0);
        assert_eq!(f32::from_le_bytes(contents[4..8].try_into().unwrap()), -1.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dropping_the_manager_stops_writer_threads_without_panicking() {
        let dir = temp_dir("drop");
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        {
            let mut mgr = RecordingManager::new(&dir);
            mgr.start_recording(1, RecordingFormat::RawU8, 100_000_000, handle)
                .unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[path = "recording_capture_tests.rs"]
mod capture_tests;
