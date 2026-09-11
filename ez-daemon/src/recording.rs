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

const UC8_SCALE: f32 = 127.5;
const UC8_OFFSET: f32 = 127.5;

const RECV_POLL_INTERVAL: Duration = Duration::from_millis(200);

struct ActiveRecording {
    handle: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    path: PathBuf,
    start_time: Instant,
    bytes_written: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
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
        if self.active.contains_key(&channel_id) {
            return Err(anyhow!("channel {channel_id} is already recording"));
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
        let file = File::create(&path)
            .with_context(|| format!("creating recording file {}", path.display()))?;
        let writer = BufWriter::with_capacity(1_048_576, file);

        let stop = Arc::new(AtomicBool::new(false));
        let bytes_written = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let thread_bytes = Arc::clone(&bytes_written);
        let thread_error = Arc::clone(&error);

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
        Ok(recording.status(channel_id, false))
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
) {
    while !stop.load(Ordering::Relaxed) {
        let Some(block) = input.recv_timeout(RECV_POLL_INTERVAL) else {
            continue;
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
    }
    let _ = writer.flush();
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
    format!("{}_ch{channel_id}_{freq_mhz:.3}MHz.{ext}", now_ms())
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
    (component * UC8_SCALE + UC8_OFFSET)
        .round()
        .clamp(0.0, 255.0) as u8
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
    fn recording_reports_inactive_on_writer_error() {
        // Issue 22: If writer thread encounters an error, status must report active: false.
        let rec = ActiveRecording {
            handle: None,
            stop: Arc::new(AtomicBool::new(false)),
            path: PathBuf::from("/nonexistent/file.iq"),
            start_time: Instant::now(),
            bytes_written: Arc::new(AtomicU64::new(0)),
            error: Arc::new(Mutex::new(Some("disk full".to_string()))),
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
