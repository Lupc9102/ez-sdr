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
    pub wav_writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
    pub last_filename: String,
    pub last_error: String,
    disk_cache: (std::time::Instant, f64, String),
    file_list: Vec<RecordingFile>,
    file_list_last_scan: Option<std::time::Instant>,
}

#[derive(Clone)]
struct RecordingFile {
    name: String,
    size_bytes: u64,
    modified: String,
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
            wav_writer: None,
            last_filename: String::new(),
            last_error: String::new(),
            disk_cache: (std::time::Instant::now(), 99.9, "GB".to_string()),
            file_list: Vec::new(),
            file_list_last_scan: None,
        }
    }

    fn scan_recordings(&mut self) {
        let should_scan = self.file_list_last_scan
            .map(|t| t.elapsed().as_secs() >= 5)
            .unwrap_or(true);
        if !should_scan { return; }
        self.file_list_last_scan = Some(std::time::Instant::now());

        let dir = std::path::Path::new(&self.output_dir);
        let mut files: Vec<RecordingFile> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext != "iq" && ext != "wav" { continue; }
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let modified = entry.metadata()
                    .and_then(|m| m.modified())
                    .map(|t| {
                        let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        let ts = chrono::DateTime::<chrono::Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs));
                        ts.format("%Y-%m-%d %H:%M").to_string()
                    })
                    .unwrap_or_else(|_| "?".to_string());
                files.push(RecordingFile { name, size_bytes, modified });
            }
        }
        // Sort newest first by name (timestamps in filename)
        files.sort_by(|a, b| b.name.cmp(&a.name));
        self.file_list = files;
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
        let ts_str = ts.to_string();
        if self.record_iq {
            let filename = format!("{}_{:.1}MHz.iq", ts_str, freq_mhz);
            let path = dir.join(&filename);
            match std::fs::File::create(&path) {
                Ok(file) => {
                    self.iq_writer = Some(std::io::BufWriter::new(file));
                    self.last_filename = filename;
                }
                Err(e) => {
                    self.last_error = format!("Failed to create IQ file: {}", e);
                    return;
                }
            }
        }
        if self.record_audio {
            let wav_filename = format!("{}_{:.1}MHz_audio.wav", ts_str, freq_mhz);
            let wav_path = dir.join(&wav_filename);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            match hound::WavWriter::create(&wav_path, spec) {
                Ok(w) => {
                    self.wav_writer = Some(w);
                    if self.last_filename.is_empty() { self.last_filename = wav_filename; }
                }
                Err(e) => {
                    self.last_error = format!("Failed to create WAV file: {}", e);
                }
            }
        }
        self.recording = true;
        self.start_time = Some(std::time::Instant::now());
        self.bytes_written = 0;
        if let Ok(mut state) = self.shared.try_lock() {
            state.recording = true;
        }
    }

    pub fn stop_recording(&mut self) {
        self.iq_writer.take();
        if let Some(w) = self.wav_writer.take() {
            let _ = w.finalize();
        }
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

    pub fn write_audio_samples(&mut self, audio: &[f32]) {
        if self.recording {
            if let Some(writer) = &mut self.wav_writer {
                for &s in audio {
                    let sample = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    if let Err(e) = writer.write_sample(sample) {
                        self.last_error = format!("WAV write error: {}", e);
                        break;
                    }
                }
                self.bytes_written = self.bytes_written.saturating_add(audio.len() as u64 * 2);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.scan_recordings();
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

        // Recordings file browser
        ui.separator();
        ui.collapsing(format!("Recordings ({} files)", self.file_list.len()), |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("↻ Refresh").on_hover_text("Rescan the output directory for .iq and .wav files.").clicked() {
                    self.file_list_last_scan = None;
                    self.scan_recordings();
                }
                if ui.small_button("📂 Open folder").on_hover_text("Open the recordings directory in your file manager.").clicked() {
                    let _ = std::process::Command::new("xdg-open").arg(&self.output_dir).spawn();
                }
            });
            if self.file_list.is_empty() {
                ui.label(egui::RichText::new("No .iq or .wav files found in output directory.").color(egui::Color32::GRAY));
            } else {
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    egui::Grid::new("rec_file_grid").num_columns(3).striped(true).min_col_width(60.0).show(ui, |ui| {
                        ui.label(egui::RichText::new("File").strong());
                        ui.label(egui::RichText::new("Size").strong());
                        ui.label(egui::RichText::new("Date").strong());
                        ui.end_row();
                        let files = self.file_list.clone();
                        for f in &files {
                            let size_str = if f.size_bytes > 1_073_741_824 {
                                format!("{:.1} GB", f.size_bytes as f64 / 1_073_741_824.0)
                            } else if f.size_bytes > 1_048_576 {
                                format!("{:.0} MB", f.size_bytes as f64 / 1_048_576.0)
                            } else {
                                format!("{:.0} KB", f.size_bytes as f64 / 1024.0)
                            };
                            ui.label(&f.name).on_hover_text(&f.name);
                            ui.label(&size_str);
                            ui.label(&f.modified);
                            ui.end_row();
                        }
                    });
                });
            }
        });
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
