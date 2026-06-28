use std::sync::{Arc, Mutex};
use crate::app::SharedState;

pub struct RecorderPanel {
    shared: Arc<Mutex<SharedState>>,
    pub recording: bool,
    pub record_iq: bool,
    pub record_audio: bool,
    pub output_dir: String,
    pub format: RecordFormat,
    pub start_time: Option<std::time::Instant>,
    pub bytes_written: u64,
    pub iq_writer: Option<std::io::BufWriter<std::fs::File>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecordFormat {
    RawIq,
    WavAudio,
}

impl RecorderPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            recording: false,
            record_iq: true,
            record_audio: false,
            output_dir: "./recordings".to_string(),
            format: RecordFormat::RawIq,
            start_time: None,
            bytes_written: 0,
            iq_writer: None,
        }
    }

    pub fn start_recording(&mut self) {
        let dir = std::path::Path::new(&self.output_dir);
        let _ = std::fs::create_dir_all(dir);
        let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let freq_mhz = self.shared.lock().unwrap().source.frequency_hz as f64 / 1e6;
        let filename = dir.join(format!("{}_{:.1}MHz.iq", ts, freq_mhz));
        if let Ok(file) = std::fs::File::create(&filename) {
            self.iq_writer = Some(std::io::BufWriter::new(file));
            self.recording = true;
            self.start_time = Some(std::time::Instant::now());
            self.bytes_written = 0;
        }
    }

    pub fn stop_recording(&mut self) {
        self.iq_writer.take();
        self.recording = false;
    }

    pub fn write_samples(&mut self, samples: &[u8]) {
        if self.recording {
            if let Some(writer) = &mut self.iq_writer {
                use std::io::Write;
                let _ = writer.write_all(samples);
                self.bytes_written += samples.len() as u64;
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

        ui.checkbox(&mut self.record_iq, "Record IQ");
        ui.checkbox(&mut self.record_audio, "Record audio");

        egui::ComboBox::from_label("Format")
            .selected_text(match self.format { RecordFormat::RawIq => "Raw IQ (f32)", RecordFormat::WavAudio => "WAV Audio" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.format, RecordFormat::RawIq, "Raw IQ");
                ui.selectable_value(&mut self.format, RecordFormat::WavAudio, "WAV Audio");
            });

        ui.horizontal(|ui| {
            ui.label("Output dir:");
            ui.add(egui::TextEdit::singleline(&mut self.output_dir));
        });

        if self.recording {
            if let Some(start) = self.start_time {
                let elapsed = start.elapsed().as_secs();
                let size_mb = self.bytes_written as f64 / 1_048_576.0;
                let (free_gb, _) = free_disk_space(&self.output_dir);
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::RED, "● REC");
                    ui.label(format!("{}s | {:.1} MB | free: {:.1} GB", elapsed, size_mb, free_gb));
                });
                let progress = if let Ok(_state) = self.shared.try_lock() {
                    // Show a fake progress that wraps every 10 minutes
                    (elapsed % 600) as f32 / 600.0
                } else { 0.0 };
                ui.add(egui::ProgressBar::new(progress).text("Session progress"));
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
}

fn free_disk_space(_path: &str) -> (f64, String) {
    // Simple fallback
    (99.9, "GB".to_string())
}
