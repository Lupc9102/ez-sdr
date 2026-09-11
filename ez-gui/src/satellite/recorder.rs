use crossbeam_channel::{bounded, Receiver, Sender};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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
    pub satellite: Option<String>,
    pub observer_lat: f64,
    pub observer_lon: f64,
}

pub struct Cf32StreamWriter {
    tx: Option<Sender<Vec<u8>>>,
    handle: Option<JoinHandle<()>>,
    path: PathBuf,
    start_time: Instant,
    bytes_written: Arc<AtomicU64>,
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

        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let sat_tag = satellite.as_deref().unwrap_or("SAT").replace(' ', "_");
        let freq_str = format!("{:.3}", center_freq_hz as f64 / 1e6);
        let filename = format!("{}_{}_{}MHz.cf32", ts, sat_tag, freq_str);
        let path = PathBuf::from(output_dir).join(&filename);

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(64);

        let bytes_written = Arc::new(AtomicU64::new(0));
        let bytes_written_thread = Arc::clone(&bytes_written);

        let path_clone = path.clone();
        let start_utc = chrono::Utc::now().to_rfc3339();

        let handle = std::thread::spawn(move || {
            let file = match File::create(&path_clone) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "Cf32StreamWriter: failed to create {}: {}",
                        path_clone.display(),
                        e
                    );
                    return;
                }
            };
            let mut writer = BufWriter::with_capacity(1_048_576, file);
            let flush_interval = std::time::Duration::from_millis(100);
            let mut last_flush = std::time::Instant::now();

            while let Ok(chunk) = rx.recv() {
                let converted = raw_iq_bytes_to_cf32_le(&chunk);
                if let Err(e) = writer.write_all(&converted) {
                    eprintln!("Cf32StreamWriter: write error: {}", e);
                    break;
                }
                bytes_written_thread.fetch_add(converted.len() as u64, Ordering::Relaxed);
                if last_flush.elapsed() >= flush_interval {
                    let _ = writer.flush();
                    last_flush = std::time::Instant::now();
                }
            }
            let _ = writer.flush();
        });

        let meta = Cf32RecordingMeta {
            frequency_hz: center_freq_hz,
            sample_rate_hz: sample_rate,
            format: "cf32_le".to_string(),
            start_utc,
            end_utc: None,
            duration_sec: 0.0,
            bytes_total: 0,
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
            meta,
        })
    }

    pub fn write(&mut self, samples: &[u8]) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(samples.to_vec());
        }
    }

    pub fn stop(&mut self) -> Result<Cf32RecordingMeta, String> {
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let elapsed = self.start_time.elapsed().as_secs_f64();
        self.meta.end_utc = Some(chrono::Utc::now().to_rfc3339());
        self.meta.duration_sec = elapsed;
        self.meta.bytes_total = self.bytes_written.load(Ordering::Relaxed);

        // Write sidecar JSON
        let sidecar_path = self.path.with_extension("json");
        if let Ok(json) = serde_json::to_string_pretty(&self.meta) {
            let _ = std::fs::write(&sidecar_path, json);
        }

        Ok(self.meta.clone())
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.handle.is_some()
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
