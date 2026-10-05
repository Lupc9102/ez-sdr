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
    pub start_utc: String,
    pub end_utc: Option<String>,
    pub duration_sec: f64,
    pub bytes_total: u64,
    pub lost_samples: u64,
    pub complete: bool,
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
            start_utc,
            end_utc: None,
            duration_sec: 0.0,
            bytes_total: 0,
            lost_samples: 0,
            complete: false,
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
        })
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

    pub fn stop(&mut self) -> Result<Cf32RecordingMeta, String> {
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
        self.meta.duration_sec = self.offered_samples as f64 / f64::from(self.meta.sample_rate_hz);
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
        if self.handle.is_some() {
            let _ = self.stop();
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
    let n_pairs = samples.len() / 2;
    let mut out = Vec::with_capacity(n_pairs * 8);
    for i in 0..n_pairs {
        let i_val = samples[2 * i] as f32 - 127.4;
        let q_val = samples[2 * i + 1] as f32 - 127.4;
        out.extend_from_slice(&i_val.to_le_bytes());
        out.extend_from_slice(&q_val.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!((i_val - (-127.4)).abs() < 1e-5);
        assert!((q_val - 127.6).abs() < 1e-5);
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
