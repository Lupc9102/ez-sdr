use std::sync::{Arc, Mutex};
use crate::app::SharedState;

pub struct SignalEvent {
    pub timestamp: String,
    pub frequency_hz: u64,
    pub mode: String,
    pub signal_db: f32,
}

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
    pub max_duration_mins: u32,
    delete_confirm: Option<String>,
    // Squelch-triggered recording
    pub squelch_record: bool,
    pub squelch_record_tail_ms: u64,
    squelch_record_last_active: Option<std::time::Instant>,
    pub squelch_record_count: u32,
    // Signal event log
    pub signal_monitor: bool,
    pub signal_log: std::collections::VecDeque<SignalEvent>,
    signal_last_logged: Option<std::time::Instant>,
    // Filename template
    pub filename_template: String,
    // Quick-start duration in seconds (0 = use max_duration_mins)
    quick_duration_secs: u64,
    // Peak audio level monitoring
    pub peak_level_dbfs: f32,
    peak_hold_time: Option<std::time::Instant>,
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
            max_duration_mins: 0,
            delete_confirm: None,
            squelch_record: false,
            squelch_record_tail_ms: 2000,
            squelch_record_last_active: None,
            squelch_record_count: 0,
            signal_monitor: false,
            signal_log: std::collections::VecDeque::with_capacity(200),
            signal_last_logged: None,
            filename_template: "{date}_{freq}MHz".to_string(),
            quick_duration_secs: 0,
            peak_level_dbfs: -120.0,
            peak_hold_time: None,
        }
    }

    fn apply_filename_template(&self, template: &str, ts_str: &str, freq_mhz: f64, mode: &str) -> String {
        template
            .replace("{date}", ts_str)
            .replace("{freq}", &format!("{:.3}", freq_mhz))
            .replace("{mode}", mode)
            .replace("{freq1}", &format!("{:.1}", freq_mhz))
            .replace("{freq0}", &format!("{:.0}", freq_mhz))
            .replace(' ', "_")
    }

    pub fn tick_squelch_record(&mut self, signal_db: f32, squelch_db: f32, freq_hz: u64, mode: &str) {
        let signal_active = signal_db > squelch_db && squelch_db > -90.0;
        let now = std::time::Instant::now();

        // Signal event log — throttle to one entry per 5s per activation
        if self.signal_monitor && signal_active {
            let log_gap = std::time::Duration::from_secs(5);
            let should_log = self.signal_last_logged.map(|t| now.duration_since(t) >= log_gap).unwrap_or(true);
            if should_log {
                self.signal_last_logged = Some(now);
                let ts = chrono::Local::now().format("%H:%M:%S").to_string();
                if self.signal_log.len() >= 200 { self.signal_log.pop_front(); }
                self.signal_log.push_back(SignalEvent {
                    timestamp: ts,
                    frequency_hz: freq_hz,
                    mode: mode.to_string(),
                    signal_db,
                });
            }
        }
        if !signal_active {
            self.signal_last_logged = None;
        }

        if !self.squelch_record { return; }
        if signal_active {
            self.squelch_record_last_active = Some(now);
            if !self.recording {
                self.start_recording();
                self.squelch_record_count += 1;
            }
        } else if self.recording {
            let tail = std::time::Duration::from_millis(self.squelch_record_tail_ms);
            let since = self.squelch_record_last_active.map(|t| now.duration_since(t)).unwrap_or(tail);
            if since >= tail {
                self.stop_recording();
            }
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
        let now = chrono::Local::now();
        let ts_str = now.format("%Y%m%d_%H%M%S").to_string();
        let timestamp_utc = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let (freq_hz, sample_rate_hz, gain_db, ppm_correction, demod_label) =
            if let Ok(state) = self.shared.try_lock() {
                (
                    state.source.frequency_hz,
                    state.source.sample_rate_hz,
                    state.source.gain_db,
                    state.source.ppm_correction,
                    state.demod_mode.label().to_string(),
                )
            } else {
                (0, 2_048_000, 0.0, 0, "NFM".to_string())
            };
        let freq_mhz = freq_hz as f64 / 1e6;

        let mut iq_filename = String::new();
        let mut wav_filename = String::new();

        let template = self.filename_template.clone();
        let base_name = self.apply_filename_template(&template, &ts_str, freq_mhz, &demod_label);

        if self.record_iq {
            let filename = format!("{}.iq", base_name);
            let path = dir.join(&filename);
            match std::fs::File::create(&path) {
                Ok(file) => {
                    self.iq_writer = Some(std::io::BufWriter::new(file));
                    self.last_filename = filename.clone();
                    iq_filename = filename;
                }
                Err(e) => {
                    self.last_error = format!("Failed to create IQ file: {}", e);
                    return;
                }
            }
        }
        if self.record_audio {
            let wf = format!("{}_audio.wav", base_name);
            let wav_path = dir.join(&wf);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            match hound::WavWriter::create(&wav_path, spec) {
                Ok(w) => {
                    self.wav_writer = Some(w);
                    if self.last_filename.is_empty() { self.last_filename = wf.clone(); }
                    wav_filename = wf;
                }
                Err(e) => {
                    self.last_error = format!("Failed to create WAV file: {}", e);
                }
            }
        }

        // Write sidecar JSON with recording metadata
        let sidecar_name = format!("{}.json", base_name);
        let sidecar_path = dir.join(&sidecar_name);
        let mut files_json = String::from("[");
        if !iq_filename.is_empty() {
            files_json.push_str(&format!("\"{}\"", iq_filename));
        }
        if !wav_filename.is_empty() {
            if !iq_filename.is_empty() { files_json.push(','); }
            files_json.push_str(&format!("\"{}\"", wav_filename));
        }
        files_json.push(']');
        let json = format!(
            "{{\n  \"frequency_hz\": {},\n  \"frequency_mhz\": {:.6},\n  \"sample_rate_hz\": {},\n  \"demod_mode\": \"{}\",\n  \"gain_db\": {:.1},\n  \"ppm_correction\": {},\n  \"timestamp_utc\": \"{}\",\n  \"files\": {}\n}}\n",
            freq_hz, freq_mhz, sample_rate_hz, demod_label, gain_db, ppm_correction, timestamp_utc, files_json
        );
        let _ = std::fs::write(&sidecar_path, json);

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
        self.peak_level_dbfs = -120.0;
        self.peak_hold_time = None;
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
            // Track peak level during recording
            if !audio.is_empty() {
                let peak = audio.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                let peak_dbfs = if peak > 0.0 {
                    20.0 * peak.log10()
                } else {
                    -120.0
                };
                if peak_dbfs > self.peak_level_dbfs {
                    self.peak_level_dbfs = peak_dbfs;
                    self.peak_hold_time = Some(std::time::Instant::now());
                }
            }
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

        ui.horizontal(|ui| {
            ui.label("Filename:").on_hover_text("Filename template for recordings. Tokens: {date}=timestamp, {freq}=frequency (3dp), {freq1}=frequency (1dp), {mode}=demod mode. Extension is added automatically.");
            ui.add(egui::TextEdit::singleline(&mut self.filename_template).desired_width(200.0))
                .on_hover_text("Example: '{date}_{freq}MHz' → '20240615_120000_145.500MHz.iq'\nTokens: {date} {freq} {freq1} {freq0} {mode}");
            if ui.small_button("Reset").clicked() {
                self.filename_template = "{date}_{freq}MHz".to_string();
            }
        });
        // Show preview of the next filename
        {
            let preview_ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
            let freq_mhz_preview = if let Ok(state) = self.shared.try_lock() {
                state.source.frequency_hz as f64 / 1e6
            } else { 145.5 };
            let mode_preview = if let Ok(state) = self.shared.try_lock() {
                state.demod_mode.label().to_string()
            } else { "NFM".to_string() };
            let preview = self.apply_filename_template(&self.filename_template.clone(), &preview_ts, freq_mhz_preview, &mode_preview);
            ui.label(egui::RichText::new(format!("→ {}.iq / .wav", preview)).small().color(egui::Color32::from_gray(150)))
                .on_hover_text("Preview of the next recording filename with current frequency and time.");
        }

        if !self.last_filename.is_empty() {
            ui.label(format!("Last file: {}", self.last_filename));
        }

        if !self.last_error.is_empty() {
            ui.colored_label(egui::Color32::RED, &self.last_error);
        }

        ui.separator();

        // Squelch-triggered recording
        ui.collapsing("🎙 VOX / Squelch-triggered recording", |ui| {
            ui.label("Automatically start and stop recording when a signal is detected above the squelch threshold. Each transmission becomes a separate file.");
            ui.horizontal(|ui| {
                let vox_label = if self.squelch_record {
                    egui::RichText::new("VOX ON").color(egui::Color32::from_rgb(80, 220, 120)).strong()
                } else {
                    egui::RichText::new("Enable VOX")
                };
                if ui.toggle_value(&mut self.squelch_record, vox_label)
                    .on_hover_text("When enabled, recording starts automatically when signal exceeds squelch, and stops after the tail delay.")
                    .changed() && self.squelch_record && self.recording {
                    self.stop_recording();
                }
                ui.add(egui::Slider::new(&mut self.squelch_record_tail_ms, 200u64..=10_000)
                    .step_by(200.0)
                    .text("Tail (ms)")
                    .custom_formatter(|v, _| {
                        if v < 1000.0 { format!("{:.0} ms", v) } else { format!("{:.1} s", v / 1000.0) }
                    }))
                    .on_hover_text("How long to continue recording after signal drops. Prevents chopping multi-part transmissions.");
            });
            if self.squelch_record_count > 0 {
                ui.label(format!("{} recordings captured this session", self.squelch_record_count));
            }
            if self.squelch_record && self.recording {
                ui.colored_label(egui::Color32::from_rgb(80, 220, 120), "● Recording active transmission…");
            } else if self.squelch_record {
                ui.colored_label(egui::Color32::GRAY, "◉ Waiting for signal…");
            }
        });
        ui.separator();

        // Signal event log / monitor
        ui.collapsing(format!("📋 Signal Log ({} events)", self.signal_log.len()), |ui| {
            ui.label("Timestamped log of signals detected above squelch. Useful for unattended monitoring — see what came through while you were away.");
            ui.horizontal(|ui| {
                let mon_label = if self.signal_monitor {
                    egui::RichText::new("Monitoring").color(egui::Color32::from_rgb(80, 220, 120)).strong()
                } else {
                    egui::RichText::new("Start Monitor")
                };
                ui.toggle_value(&mut self.signal_monitor, mon_label)
                    .on_hover_text("Log each new signal detection with timestamp, frequency, mode, and strength. Throttled to one entry per 5 seconds per activation.");
                if ui.small_button("🗑 Clear").on_hover_text("Clear all log entries.").clicked() {
                    self.signal_log.clear();
                }
                if !self.signal_log.is_empty() && ui.small_button("💾 Export CSV").on_hover_text("Save signal log to CSV file.").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name("ez_sdr_signal_log.csv")
                        .add_filter("CSV", &["csv"])
                        .save_file()
                    {
                        let mut csv = String::from("timestamp,frequency_hz,frequency_mhz,mode,signal_db\n");
                        for ev in &self.signal_log {
                            csv.push_str(&format!("{},{},{:.6},{},{:.1}\n",
                                ev.timestamp, ev.frequency_hz,
                                ev.frequency_hz as f64 / 1e6,
                                ev.mode, ev.signal_db));
                        }
                        let _ = std::fs::write(&path, csv);
                    }
                }
            });
            if self.signal_log.is_empty() {
                ui.colored_label(egui::Color32::GRAY, "No signals logged yet. Enable monitoring and set squelch above noise floor.");
            } else {
                egui::ScrollArea::vertical().max_height(180.0).id_salt("sig_log_scroll").show(ui, |ui| {
                    egui::Grid::new("sig_log_grid").num_columns(4).striped(true).min_col_width(50.0).show(ui, |ui| {
                        ui.label(egui::RichText::new("Time").strong());
                        ui.label(egui::RichText::new("Frequency").strong());
                        ui.label(egui::RichText::new("Mode").strong());
                        ui.label(egui::RichText::new("Level").strong());
                        ui.end_row();
                        for ev in self.signal_log.iter().rev() {
                            let freq_str = if ev.frequency_hz >= 1_000_000_000 {
                                format!("{:.3} GHz", ev.frequency_hz as f64 / 1e9)
                            } else {
                                format!("{:.3} MHz", ev.frequency_hz as f64 / 1e6)
                            };
                            let db_color = if ev.signal_db > -60.0 { egui::Color32::from_rgb(80, 220, 120) }
                                else if ev.signal_db > -80.0 { egui::Color32::from_rgb(220, 200, 80) }
                                else { egui::Color32::from_rgb(180, 180, 180) };
                            ui.monospace(&ev.timestamp);
                            ui.label(&freq_str);
                            ui.label(&ev.mode);
                            ui.colored_label(db_color, format!("{:.1} dB", ev.signal_db));
                            ui.end_row();
                        }
                    });
                });
            }
        });
        ui.separator();

        if self.recording {
            if let Some(start) = self.start_time {
                let elapsed = start.elapsed().as_secs();
                let size_mb = self.bytes_written as f64 / 1_048_576.0;
                let (free_gb, unit) = self.cached_free_disk_space();

                // Auto-stop when duration limit reached
                let limit_secs = if self.quick_duration_secs > 0 {
                    self.quick_duration_secs
                } else if self.max_duration_mins > 0 {
                    self.max_duration_mins as u64 * 60
                } else {
                    0
                };
                if limit_secs > 0 && elapsed >= limit_secs {
                    self.stop_recording();
                    self.quick_duration_secs = 0;
                    self.last_error.clear();
                } else {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::RED, "● REC");
                        let mins = elapsed / 60;
                        let secs = elapsed % 60;
                        ui.monospace(format!("{:02}:{:02}", mins, secs));
                        if limit_secs > 0 {
                            let rem = limit_secs.saturating_sub(elapsed);
                            ui.label(format!("→ {}:{:02} left", rem / 60, rem % 60));
                        }
                        ui.separator();
                        ui.label(format!("{:.1} MB", size_mb));
                        ui.separator();
                        ui.label(format!("{:.1} {} free", free_gb, unit));
                    });

                    // Data rate
                    if elapsed > 0 {
                        let rate_mbps = self.bytes_written as f64 / elapsed as f64 / 1_048_576.0;
                        ui.label(format!("Rate: {:.2} MB/s", rate_mbps));
                        // Estimate time until disk full
                        let (free_gb, unit) = self.cached_free_disk_space();
                        if rate_mbps > 0.0 {
                            let free_bytes = free_gb * if unit == "GB" { 1_073_741_824.0 } else { 1_099_511_627_776.0 };
                            let seconds_until_full = (free_bytes / 1_048_576.0) / rate_mbps;
                            let time_str = if seconds_until_full > 3600.0 {
                                format!("~{:.1}h until full", seconds_until_full / 3600.0)
                            } else if seconds_until_full > 60.0 {
                                format!("~{:.0}m until full", seconds_until_full / 60.0)
                            } else {
                                format!("~{:.0}s until full", seconds_until_full)
                            };
                            ui.colored_label(
                                if seconds_until_full < 3600.0 { egui::Color32::YELLOW } else { egui::Color32::GRAY },
                                time_str
                            ).on_hover_text("Estimated time before disk is full at current data rate");
                        }
                    }
                    // Peak audio level indicator (only if recording audio)
                    if self.record_audio {
                        ui.horizontal(|ui| {
                            ui.label("Peak:");
                            let peak_norm = ((self.peak_level_dbfs + 120.0) / 120.0).clamp(0.0, 1.0);
                            let clipping = self.peak_level_dbfs > -3.0;
                            let bar_color = if clipping { egui::Color32::RED } else if peak_norm > 0.7 { egui::Color32::YELLOW } else { egui::Color32::GREEN };
                            ui.add(egui::ProgressBar::new(peak_norm).text(format!("{:.1} dBFS", self.peak_level_dbfs))
                                .fill(bar_color)
                                .desired_width(150.0))
                                .on_hover_text(if clipping { "⚠ Clipping detected! Peak exceeds -3 dBFS" } else { "Audio level in decibels relative to full scale" });
                        });
                        // Decay peak hold after 3 seconds of not seeing a new peak
                        if let Some(hold_time) = self.peak_hold_time {
                            if hold_time.elapsed() > std::time::Duration::from_secs(3) {
                                self.peak_level_dbfs = self.peak_level_dbfs * 0.95 - 2.0;
                                if self.peak_level_dbfs < -120.0 {
                                    self.peak_level_dbfs = -120.0;
                                    self.peak_hold_time = None;
                                }
                            }
                        }
                    }
                    if ui.button("■ Stop").clicked() {
                        self.stop_recording();
                    }
                }
            }
        } else {
            ui.horizontal(|ui| {
                if ui.button("● Start Recording").clicked() {
                    self.start_recording();
                }
                ui.label("Stop after:").on_hover_text("Auto-stop recording after this duration. 0 = record until manually stopped.");
                egui::ComboBox::from_id_salt("rec_dur")
                    .selected_text(if self.max_duration_mins == 0 { "∞ unlimited".to_string() } else { format!("{} min", self.max_duration_mins) })
                    .show_ui(ui, |ui| {
                        for (label, val) in [("∞ unlimited", 0u32), ("5 min", 5), ("15 min", 15), ("30 min", 30), ("60 min", 60), ("120 min", 120)] {
                            ui.selectable_value(&mut self.max_duration_mins, val, label);
                        }
                    });
            });
            // Quick-start preset buttons
            if !self.recording {
                ui.horizontal(|ui| {
                    ui.label("Quick:").on_hover_text("Start recording immediately with a preset duration — no need to press Start separately.");
                    for (label, mins, secs) in [("30s", 0u32, 30u64), ("1m", 1, 60), ("5m", 5, 300), ("10m", 10, 600)] {
                        if ui.small_button(label).on_hover_text(format!("Record for {} then auto-stop.", label)).clicked() {
                            self.max_duration_mins = mins;
                            // For sub-minute durations, store as a fractional minute via a special field
                            // Use 0 mins with the auto_stop hack: set duration_secs override
                            self.quick_duration_secs = if mins == 0 { secs } else { 0 };
                            self.start_recording();
                        }
                    }
                });
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
                    egui::Grid::new("rec_file_grid").num_columns(4).striped(true).min_col_width(60.0).show(ui, |ui| {
                        ui.label(egui::RichText::new("File").strong());
                        ui.label(egui::RichText::new("Size").strong());
                        ui.label(egui::RichText::new("Date").strong());
                        ui.label("");
                        ui.end_row();
                        let files = self.file_list.clone();
                        let mut to_delete: Option<String> = None;
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
                            let confirming = self.delete_confirm.as_deref() == Some(&f.name);
                            if confirming {
                                if ui.small_button(egui::RichText::new("✓ Delete?").color(egui::Color32::RED))
                                    .on_hover_text("Click to confirm deletion. This cannot be undone.")
                                    .clicked()
                                {
                                    to_delete = Some(f.name.clone());
                                    self.delete_confirm = None;
                                }
                            } else {
                                if ui.small_button("🗑").on_hover_text("Delete this recording file.").clicked() {
                                    self.delete_confirm = Some(f.name.clone());
                                }
                            }
                            ui.end_row();
                        }
                        if let Some(name) = to_delete {
                            let path = std::path::Path::new(&self.output_dir).join(&name);
                            let _ = std::fs::remove_file(&path);
                            self.file_list_last_scan = None;
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
