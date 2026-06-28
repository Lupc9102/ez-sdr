use std::sync::{Arc, Mutex};
use crate::app::SharedState;

pub struct RecorderPanel {
    shared: Arc<Mutex<SharedState>>,
    pub recording: bool,
    pub record_iq: bool,
    pub record_audio: bool,
    pub output_dir: String,
    pub start_time: Option<std::time::Instant>,
    pub bytes_written: u64,
    pub iq_writer: Option<std::io::BufWriter<std::fs::File>>,
    pub last_filename: String,
    pub last_error: String,
    disk_cache: (std::time::Instant, f64, String),
}

impl RecorderPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            recording: false,
            record_iq: true,
            record_audio: false,
            output_dir: "./recordings".to_string(),
            start_time: None,
            bytes_written: 0,
            iq_writer: None,
            last_filename: String::new(),
            last_error: String::new(),
            disk_cache: (std::time::Instant::now(), 99.9, "GB".to_string()),
        }
    }

    pub fn start_recording(&mut self) {
        if self.recording { return; }
        let dir = std::path::Path::new(&self.output_dir);
        self.last_error.clear();
        if let Err(e) = std::fs::create_dir_all(dir) {
            self.last_error = format!("Failed to create directory: {}", e);
            return;
        }
        let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let freq_mhz = if let Ok(state) = self.shared.try_lock() {
            state.source.frequency_hz as f64 / 1e6
        } else {
            0.0
        };
        let filename = format!("{}_{:.1}MHz.iq", ts, freq_mhz);
        let path = dir.join(&filename);
        match std::fs::File::create(&path) {
            Ok(file) => {
                self.iq_writer = Some(std::io::BufWriter::new(file));
                self.recording = true;
                self.start_time = Some(std::time::Instant::now());
                self.bytes_written = 0;
                self.last_filename = filename;
                if let Ok(mut state) = self.shared.try_lock() {
                    state.recording = true;
                }
            }
            Err(e) => {
                self.last_error = format!("Failed to create file: {}", e);
            }
        }
    }

    pub fn stop_recording(&mut self) {
        self.iq_writer.take();
        self.recording = false;
        if let Ok(mut state) = self.shared.try_lock() {
            state.recording = false;
        }
    }

    pub fn write_samples(&mut self, samples: &[u8]) {
        if self.recording {
            if let Some(writer) = &mut self.iq_writer {
                use std::io::Write;
                if let Err(e) = writer.write_all(samples) {
                    self.last_error = format!("Write error: {}", e);
                    self.stop_recording();
                } else {
                    self.bytes_written += samples.len() as u64;
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Recorder");

        if let Ok(state) = self.shared.try_lock() {
            ui.label(format!("Source: {:.3} MHz — {}",
                state.source.frequency_hz as f64 / 1e6,
                if self.recording { "RECORDING" } else { "idle" }
            ));
        }

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.record_iq, "Record IQ");
            ui.checkbox(&mut self.record_audio, "Record audio (WAV)");
        });

        ui.horizontal(|ui| {
            ui.label("Output dir:");
            ui.add(egui::TextEdit::singleline(&mut self.output_dir).desired_width(200.0));
        });

        if !self.last_filename.is_empty() {
            ui.label(format!("Last file: {}", self.last_filename));
        }

        if !self.last_error.is_empty() {
            ui.colored_label(egui::Color32::RED, &self.last_error);
        }

        ui.separator();

        if self.recording {
            if let Some(start) = self.start_time {
                let elapsed = start.elapsed().as_secs();
                let size_mb = self.bytes_written as f64 / 1_048_576.0;
                let (free_gb, unit) = self.cached_free_disk_space();

                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::RED, "● REC");
                    let mins = elapsed / 60;
                    let secs = elapsed % 60;
                    ui.monospace(format!("{:02}:{:02}", mins, secs));
                    ui.separator();
                    ui.label(format!("{:.1} MB", size_mb));
                    ui.separator();
                    ui.label(format!("{:.1} {} free", free_gb, unit));
                });

                // Data rate
                if elapsed > 0 {
                    let rate_mbps = self.bytes_written as f64 / elapsed as f64 / 1_048_576.0;
                    ui.label(format!("Rate: {:.2} MB/s", rate_mbps));
                }
            }
            if ui.button("■ Stop").clicked() {
                self.stop_recording();
            }
        } else {
            if ui.button("● Start Recording").clicked() {
                self.start_recording();
            }
        }
    }

    fn cached_free_disk_space(&mut self) -> (f64, String) {
        let (cached_at, cached_gb, ref cached_unit) = self.disk_cache;
        if cached_at.elapsed() < std::time::Duration::from_secs(5) {
            return (cached_gb, cached_unit.clone());
        }
        let result = free_disk_space_with_timeout(&self.output_dir);
        self.disk_cache = (std::time::Instant::now(), result.0, result.1.clone());
        result
    }
}

fn free_disk_space_with_timeout(path: &str) -> (f64, String) {
    let child = std::process::Command::new("df")
        .arg("-BM")
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let output = match child {
        Ok(mut c) => {
            use std::time::Duration;
            let start = std::time::Instant::now();
            loop {
                match c.try_wait() {
                    Ok(Some(status)) => {
                        if status.success() {
                            let out = c.wait_with_output().ok();
                            break out;
                        }
                        break None;
                    }
                    Ok(None) => {
                        if start.elapsed() > Duration::from_secs(3) {
                            let _ = c.kill();
                            let _ = c.wait();
                            break None;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break None,
                }
            }
        }
        Err(_) => None,
    };
    if let Some(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = stdout.lines().nth(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                if let Ok(avail) = parts[3].trim_end_matches('M').parse::<f64>() {
                    return (avail / 1024.0, "GB".to_string());
                }
            }
        }
    }
    (99.9, "GB".to_string())
}
