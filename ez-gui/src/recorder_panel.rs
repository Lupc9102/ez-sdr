use crate::app::SharedState;
use std::sync::{Arc, Mutex};

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
    // Async IQ writer: the UI thread only `try_send`s chunks into a bounded
    // channel (cap 128); a worker thread owns the file and does blocking
    // writes. Prevents 2.4 MSps disk stalls from freezing the render loop.
    iq_tx: Option<crossbeam_channel::Sender<Vec<u8>>>,
    iq_worker: Option<std::thread::JoinHandle<Result<u64, String>>>,
    iq_dropped_chunks: u64,
    audio_sample_rate: u32,
    pub last_filename: String,
    pub last_error: String,
    disk_cache: (std::time::Instant, f64, String),
    disk_space_rx: Option<crossbeam_channel::Receiver<(f64, String)>>,
    disk_space_checking: bool,
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
    daemon_recording: Option<ez_proto::RecordingStatus>,
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
            iq_tx: None,
            iq_worker: None,
            iq_dropped_chunks: 0,
            audio_sample_rate: 48_000,
            last_filename: String::new(),
            last_error: String::new(),
            disk_cache: (std::time::Instant::now(), 99.9, "GB".to_string()),
            disk_space_rx: None,
            disk_space_checking: false,
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
            daemon_recording: None,
        }
    }

    pub fn handle_daemon_recording(&mut self, status: ez_proto::RecordingStatus) {
        let was_active = self
            .daemon_recording
            .as_ref()
            .is_some_and(|previous| previous.active);
        self.recording = status.active;
        if status.active && !was_active {
            self.start_time = Some(std::time::Instant::now());
        } else if !status.active {
            self.start_time = None;
        }
        self.bytes_written = status.bytes_written;
        if let Some(path) = &status.path {
            self.last_filename = path.clone();
        }
        self.daemon_recording = Some(status);
        self.last_error.clear();
        if let Ok(mut state) = self.shared.try_lock() {
            state.recording = self.recording;
        }
    }

    fn daemon_mode(&self) -> bool {
        self.shared.try_lock().is_ok_and(|state| {
            state.source.source_mode == crate::source_manager::SourceMode::Daemon
        })
    }

    pub fn start(&mut self) {
        if !self.daemon_mode() {
            self.start_recording();
            return;
        }
        self.last_error.clear();
        if self.record_audio {
            self.last_error = "Daemon mode records IQ on the daemon host; audio WAV recording is not available yet.".to_string();
            return;
        }
        if !self.record_iq {
            self.last_error = "Select Record IQ before starting a daemon recording.".to_string();
            return;
        }
        let format = ez_proto::RecordingFormat::Cf32;
        let result = self
            .shared
            .try_lock()
            .map_err(|_| "source state is busy; try again".to_string())
            .and_then(|mut state| state.source.start_daemon_recording(format));
        match result {
            Ok(()) => self.last_error = "Starting daemon recording…".to_string(),
            Err(error) => self.last_error = error,
        }
    }

    pub fn stop(&mut self) {
        if !self.daemon_mode() {
            self.stop_recording();
            return;
        }
        let result = self
            .shared
            .try_lock()
            .map_err(|_| "source state is busy; try again".to_string())
            .and_then(|mut state| state.source.stop_daemon_recording());
        match result {
            Ok(()) => self.last_error = "Stopping daemon recording…".to_string(),
            Err(error) => self.last_error = error,
        }
    }

    fn apply_filename_template(
        &self,
        template: &str,
        ts_str: &str,
        freq_mhz: f64,
        mode: &str,
    ) -> String {
        template
            .replace("{date}", ts_str)
            .replace("{freq}", &format!("{freq_mhz:.3}"))
            .replace("{mode}", mode)
            .replace("{freq1}", &format!("{freq_mhz:.1}"))
            .replace("{freq0}", &format!("{freq_mhz:.0}"))
            .replace(' ', "_")
    }

    pub fn tick_squelch_record(
        &mut self,
        signal_db: f32,
        squelch_db: f32,
        freq_hz: u64,
        mode: &str,
    ) {
        let signal_active = signal_db > squelch_db && squelch_db > -90.0;
        let now = std::time::Instant::now();

        // Signal event log — throttle to one entry per 5s per activation
        if self.signal_monitor && signal_active {
            let log_gap = std::time::Duration::from_secs(5);
            let should_log = self
                .signal_last_logged
                .is_none_or(|t| now.duration_since(t) >= log_gap);
            if should_log {
                self.signal_last_logged = Some(now);
                let ts = chrono::Local::now().format("%H:%M:%S").to_string();
                if self.signal_log.len() >= 200 {
                    self.signal_log.pop_front();
                }
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

        if !self.squelch_record {
            return;
        }
        if signal_active {
            self.squelch_record_last_active = Some(now);
            if !self.recording {
                self.start();
                self.squelch_record_count += 1;
            }
        } else if self.recording {
            let tail = std::time::Duration::from_millis(self.squelch_record_tail_ms);
            let since = self
                .squelch_record_last_active
                .map_or(tail, |t| now.duration_since(t));
            if since >= tail {
                self.stop();
            }
        }
    }

    fn scan_recordings(&mut self) {
        let should_scan = self
            .file_list_last_scan
            .is_none_or(|t| t.elapsed().as_secs() >= 5);
        if !should_scan {
            return;
        }
        self.file_list_last_scan = Some(std::time::Instant::now());

        let dir = std::path::Path::new(&self.output_dir);
        let mut files: Vec<RecordingFile> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext != "iq" && ext != "wav" {
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let size_bytes = entry.metadata().map_or(0, |m| m.len());
                let modified = entry.metadata().and_then(|m| m.modified()).map_or_else(
                    |_| "?".to_string(),
                    |t| {
                        let secs = t
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs());
                        let ts = chrono::DateTime::<chrono::Local>::from(
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs),
                        );
                        ts.format("%Y-%m-%d %H:%M").to_string()
                    },
                );
                files.push(RecordingFile {
                    name,
                    size_bytes,
                    modified,
                });
            }
        }
        // Sort newest first by name (timestamps in filename)
        files.sort_by(|a, b| b.name.cmp(&a.name));
        self.file_list = files;
    }

    /// Keep the WAV clock equal to the actual demodulated/device audio clock.
    /// A rate transition finalizes the current session before accepting new audio.
    pub fn set_audio_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == 0 || self.audio_sample_rate == sample_rate {
            return;
        }
        if self.wav_writer.is_some() {
            self.stop_recording();
            let finalization_message = std::mem::take(&mut self.last_error);
            self.last_error = format!(
                "Recording stopped because the audio sample rate changed from {} to {sample_rate} Hz.",
                self.audio_sample_rate
            );
            if !finalization_message.is_empty() {
                self.last_error.push(' ');
                self.last_error.push_str(&finalization_message);
            }
        }
        self.audio_sample_rate = sample_rate;
    }

    pub fn start_recording(&mut self) {
        if self.recording {
            return;
        }
        let output_dir = self.output_dir.clone();
        self.last_error.clear();
        let adv_format = if let Ok(s) = self.shared.try_lock() {
            s.config.advanced.record_format.clone()
        } else {
            "wav".to_string()
        };
        let record_audio = self.record_audio && adv_format != "raw";
        if !self.record_iq && !record_audio {
            self.last_error =
                "Select an available IQ or audio recording format before starting.".into();
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&output_dir) {
            self.last_error = format!("Failed to create directory: {e}");
            return;
        }
        // Pre-flight: disk space check (warn if < 500 MB free)
        let (free_gb, unit) = self.cached_free_disk_space();
        let free_mb = if unit == "MB" {
            free_gb
        } else {
            free_gb * 1024.0
        };
        if free_mb < 500.0 {
            self.last_error = if free_mb == 0.0 {
                "⚠ Disk space check failed (df missing/timed out/unparseable). \
                 Cannot verify free space — recording may fail."
                    .to_string()
            } else {
                format!(
                    "⚠ Low disk space: only {free_gb:.0} {unit} free on recording drive. \
                     Recording may fail or be cut short."
                )
            };
            // Don't block — just warn. User can still record.
        }
        let dir = std::path::Path::new(&output_dir);
        let now = chrono::Local::now();
        let ts_str = now.format("%Y%m%d_%H%M%S").to_string();
        let timestamp_utc = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let (freq_hz, tuned_hz, offset_hz, sample_rate_hz, gain_db, ppm_correction, demod_label) =
            if let Ok(state) = self.shared.try_lock() {
                (
                    state.source.capture_center_frequency_hz(),
                    state.source.frequency_hz,
                    state.source.frequency_offset_hz,
                    state.source.sample_rate_hz,
                    state.source.gain_db,
                    state.source.ppm_correction,
                    state.demod_mode.label().to_string(),
                )
            } else {
                (0, 0, 0, 2_048_000, 0.0, 0, "NFM".to_string())
            };
        let freq_mhz = freq_hz as f64 / 1e6;

        let mut iq_filename = String::new();
        let mut wav_filename = String::new();

        let template = self.filename_template.clone();
        let requested_name =
            self.apply_filename_template(&template, &ts_str, freq_mhz, &demod_label);
        if requested_name.is_empty()
            || requested_name == "."
            || requested_name == ".."
            || requested_name.contains(['/', '\\'])
        {
            self.last_error =
                "Recording filename must be a nonempty name without path separators.".into();
            return;
        }
        let base_name = (0..10_000)
            .map(|suffix| {
                if suffix == 0 {
                    requested_name.clone()
                } else {
                    format!("{requested_name}_{suffix:03}")
                }
            })
            .find(|name| {
                [
                    format!("{name}.iq"),
                    format!("{name}_audio.wav"),
                    format!("{name}.json"),
                ]
                .iter()
                .all(|file| !dir.join(file).exists())
            });
        let Some(base_name) = base_name else {
            self.last_error =
                "No unused recording filename is available; choose another template.".into();
            return;
        };
        self.last_filename.clear();

        if self.record_iq {
            let filename = format!("{base_name}.iq");
            let path = dir.join(&filename);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    // Hand the file to a worker thread; the UI thread only
                    // pushes chunks into a bounded channel (never blocks).
                    let (tx, rx) = crossbeam_channel::bounded::<Vec<u8>>(128);
                    let handle = std::thread::spawn(move || -> Result<u64, String> {
                        use std::io::Write;
                        let mut writer = std::io::BufWriter::new(file);
                        let mut total: u64 = 0;
                        for chunk in rx {
                            if let Err(e) = writer.write_all(&chunk) {
                                return Err(format!("Write error: {e}"));
                            }
                            total += chunk.len() as u64;
                        }
                        if let Err(e) = writer.flush() {
                            return Err(format!("Flush error: {e}"));
                        }
                        Ok(total)
                    });
                    self.iq_tx = Some(tx);
                    self.iq_worker = Some(handle);
                    self.iq_dropped_chunks = 0;
                    // Keep iq_writer None: writes go via the channel. The
                    // field is retained for API compat (always None when the
                    // worker is active).
                    self.iq_writer = None;
                    self.last_filename = filename.clone();
                    iq_filename = filename;
                }
                Err(e) => {
                    self.last_error = format!("Failed to create IQ file: {e}");
                    return;
                }
            }
        }
        // "raw" advanced format writes IQ only (no WAV sidecar).
        if record_audio {
            let wf = format!("{base_name}_audio.wav");
            let wav_path = dir.join(&wf);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: self.audio_sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let wav = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&wav_path)
                .map_err(hound::Error::IoError)
                .and_then(|file| hound::WavWriter::new(std::io::BufWriter::new(file), spec));
            match wav {
                Ok(w) => {
                    self.wav_writer = Some(w);
                    if self.last_filename.is_empty() {
                        self.last_filename = wf.clone();
                    }
                    wav_filename = wf;
                }
                Err(e) => {
                    self.last_error = format!("Failed to create WAV file: {e}");
                    // Match the IQ-file failure path: abort the session instead
                    // of falling through with wav_writer = None. Without this,
                    // `recording` is set to true below and the UI shows a REC
                    // counter ticking while every audio sample is silently
                    // dropped. Also shut down any IQ worker already started
                    // above so we don't leave a half-created orphan file.
                    self.iq_tx.take();
                    if let Some(h) = self.iq_worker.take() {
                        let _ = h.join();
                    }
                    self.iq_writer.take();
                    return;
                }
            }
        }

        // Write sidecar JSON with recording metadata
        let sidecar_name = format!("{base_name}.json");
        let sidecar_path = dir.join(&sidecar_name);
        let mut files = Vec::new();
        if !iq_filename.is_empty() {
            files.push(iq_filename);
        }
        if !wav_filename.is_empty() {
            files.push(wav_filename);
        }
        let json = serde_json::json!({
            "frequency_hz": freq_hz,
            "frequency_mhz": freq_mhz,
            "tuned_frequency_hz": tuned_hz,
            "hardware_offset_hz": offset_hz,
            "sample_rate_hz": sample_rate_hz,
            "audio_sample_rate_hz": self.audio_sample_rate,
            "demod_mode": demod_label,
            "gain_db": gain_db,
            "ppm_correction": ppm_correction,
            "timestamp_utc": timestamp_utc,
            "files": files,
        });
        let sidecar = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sidecar_path)
            .map_err(|error| error.to_string())
            .and_then(|file| {
                serde_json::to_writer_pretty(file, &json).map_err(|error| error.to_string())
            });
        if let Err(error) = sidecar {
            self.stop_recording();
            self.last_error = format!("Failed to save recording metadata: {error}");
            return;
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
        // Shut down the IQ worker: drop the sender so its `for chunk in rx`
        // loop ends, then join to flush/close the file and collect the byte
        // count / error.
        self.iq_tx.take();
        if let Some(h) = self.iq_worker.take() {
            match h.join() {
                Ok(Ok(total)) => {
                    // Worker byte count is authoritative; bytes_written was
                    // incremented optimistically on push.
                    if self.record_iq {
                        self.bytes_written = self.bytes_written.max(total);
                    }
                    if self.iq_dropped_chunks > 0 {
                        self.last_error = format!(
                            "IQ writer dropped {} chunks (disk too slow)",
                            self.iq_dropped_chunks
                        );
                    }
                }
                Ok(Err(e)) => {
                    self.last_error = e;
                }
                Err(_) => {
                    self.last_error = "IQ writer thread panicked".to_string();
                }
            }
        }
        if let Some(w) = self.wav_writer.take() {
            if let Err(e) = w.finalize() {
                self.last_error = format!("Failed to finalize WAV: {e}");
            }
        }
        self.recording = false;
        self.peak_level_dbfs = -120.0;
        self.peak_hold_time = None;
        self.quick_duration_secs = 0;
        if let Ok(mut state) = self.shared.try_lock() {
            state.recording = false;
        }
    }

    pub fn write_samples(&mut self, samples: &[u8]) {
        if !self.recording || samples.is_empty() {
            return;
        }
        // Legacy sync path (kept for tests / if worker not started).
        if let Some(writer) = &mut self.iq_writer {
            use std::io::Write;
            if let Err(e) = writer.write_all(samples) {
                self.last_error = format!("Write error: {e}");
                self.stop_recording();
            } else {
                self.bytes_written += samples.len() as u64;
            }
            return;
        }
        // Fast path: non-blocking push to the worker. Never blocks the UI
        // thread even at 2.4 MSps; on backpressure drop and count.
        if let Some(tx) = &self.iq_tx {
            match tx.try_send(samples.to_vec()) {
                Ok(()) => {
                    self.bytes_written += samples.len() as u64;
                }
                Err(crossbeam_channel::TrySendError::Full(_)) => {
                    self.iq_dropped_chunks += 1;
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    self.last_error = "IQ writer disconnected".to_string();
                    self.stop_recording();
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
                let mut error = None;
                for &s in audio {
                    let sample = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                    if let Err(e) = writer.write_sample(sample) {
                        error = Some(format!("WAV write error: {e}"));
                        break;
                    }
                    self.bytes_written = self.bytes_written.saturating_add(2);
                }
                if let Some(error) = error {
                    self.stop_recording();
                    self.last_error = error;
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.scan_recordings();
        ui.heading("Recorder");

        let daemon_mode = self.daemon_mode();
        if daemon_mode {
            ui.colored_label(
                egui::Color32::from_rgb(120, 190, 255),
                "Daemon mode: recordings are written on the daemon host.",
            );
            if let Some(status) = &self.daemon_recording {
                ui.label(format!(
                    "{} — {:.1} MiB — {:.0}s",
                    status.path.as_deref().unwrap_or("waiting for path"),
                    status.bytes_written as f64 / 1_048_576.0,
                    status.duration_sec
                ));
            }
        }

        if let Ok(state) = self.shared.try_lock() {
            ui.label(format!(
                "Source: {:.3} MHz — {}",
                state.source.capture_center_frequency_hz() as f64 / 1e6,
                if self.recording { "RECORDING" } else { "idle" }
            ));
        }

        ui.add_enabled_ui(!daemon_mode, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.record_iq, "Record IQ");
                ui.checkbox(&mut self.record_audio, "Record audio (WAV)");
            })
        });

        ui.add_enabled_ui(!daemon_mode, |ui| {
            ui.horizontal(|ui| {
                ui.label("Output dir:");
                ui.add(egui::TextEdit::singleline(&mut self.output_dir).desired_width(200.0));
            })
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
                state.source.capture_center_frequency_hz() as f64 / 1e6
            } else {
                145.5
            };
            let mode_preview = if let Ok(state) = self.shared.try_lock() {
                state.demod_mode.label().to_string()
            } else {
                "NFM".to_string()
            };
            let preview = self.apply_filename_template(
                &self.filename_template.clone(),
                &preview_ts,
                freq_mhz_preview,
                &mode_preview,
            );
            ui.label(
                egui::RichText::new(format!("→ {preview}.iq / .wav"))
                    .small()
                    .color(egui::Color32::from_gray(150)),
            )
            .on_hover_text(
                "Preview of the next recording filename with current frequency and time.",
            );
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
                    self.stop();
                }
                ui.add(egui::Slider::new(&mut self.squelch_record_tail_ms, 200u64..=10_000)
                    .step_by(200.0)
                    .text("Tail (ms)")
                    .custom_formatter(|v, _| {
                        if v < 1000.0 { format!("{v:.0} ms") } else { format!("{:.1} s", v / 1000.0) }
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
                    u64::from(self.max_duration_mins) * 60
                } else {
                    0
                };
                if limit_secs > 0 && elapsed >= limit_secs {
                    self.stop();
                    self.quick_duration_secs = 0;
                    self.last_error.clear();
                } else {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::RED, "● REC");
                        let mins = elapsed / 60;
                        let secs = elapsed % 60;
                        ui.monospace(format!("{mins:02}:{secs:02}"));
                        if limit_secs > 0 {
                            let rem = limit_secs.saturating_sub(elapsed);
                            ui.label(format!("→ {}:{:02} left", rem / 60, rem % 60));
                        }
                        ui.separator();
                        ui.label(format!("{size_mb:.1} MB"));
                        ui.separator();
                        ui.label(format!("{free_gb:.1} {unit} free"));
                    });

                    // Data rate
                    if elapsed > 0 {
                        let rate_mbps = self.bytes_written as f64 / elapsed as f64 / 1_048_576.0;
                        ui.label(format!("Rate: {rate_mbps:.2} MB/s"));
                        // Estimate time until disk full
                        let (free_gb, unit) = self.cached_free_disk_space();
                        if rate_mbps > 0.0 {
                            let free_bytes = free_gb
                                * if unit == "GB" {
                                    1_073_741_824.0
                                } else {
                                    1_099_511_627_776.0
                                };
                            let seconds_until_full = (free_bytes / 1_048_576.0) / rate_mbps;
                            let time_str = if seconds_until_full > 3600.0 {
                                format!("~{:.1}h until full", seconds_until_full / 3600.0)
                            } else if seconds_until_full > 60.0 {
                                format!("~{:.0}m until full", seconds_until_full / 60.0)
                            } else {
                                format!("~{seconds_until_full:.0}s until full")
                            };
                            ui.colored_label(
                                if seconds_until_full < 3600.0 {
                                    egui::Color32::YELLOW
                                } else {
                                    egui::Color32::GRAY
                                },
                                time_str,
                            )
                            .on_hover_text(
                                "Estimated time before disk is full at current data rate",
                            );
                        }
                    }
                    // Peak audio level indicator (only if recording audio)
                    if self.record_audio {
                        ui.horizontal(|ui| {
                            ui.label("Peak:");
                            let peak_norm =
                                ((self.peak_level_dbfs + 120.0) / 120.0).clamp(0.0, 1.0);
                            let clipping = self.peak_level_dbfs > -3.0;
                            let bar_color = if clipping {
                                egui::Color32::RED
                            } else if peak_norm > 0.7 {
                                egui::Color32::YELLOW
                            } else {
                                egui::Color32::GREEN
                            };
                            ui.add(
                                egui::ProgressBar::new(peak_norm)
                                    .text(format!("{:.1} dBFS", self.peak_level_dbfs))
                                    .fill(bar_color)
                                    .desired_width(150.0),
                            )
                            .on_hover_text(if clipping {
                                "⚠ Clipping detected! Peak exceeds -3 dBFS"
                            } else {
                                "Audio level in decibels relative to full scale"
                            });
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
                        self.stop();
                    }
                }
            }
        } else {
            ui.horizontal(|ui| {
                if ui.button("● Start Recording").clicked() {
                    self.start();
                }
                ui.label("Stop after:").on_hover_text(
                    "Auto-stop recording after this duration. 0 = record until manually stopped.",
                );
                egui::ComboBox::from_id_salt("rec_dur")
                    .selected_text(if self.max_duration_mins == 0 {
                        "∞ unlimited".to_string()
                    } else {
                        format!("{} min", self.max_duration_mins)
                    })
                    .show_ui(ui, |ui| {
                        for (label, val) in [
                            ("∞ unlimited", 0u32),
                            ("5 min", 5),
                            ("15 min", 15),
                            ("30 min", 30),
                            ("60 min", 60),
                            ("120 min", 120),
                        ] {
                            ui.selectable_value(&mut self.max_duration_mins, val, label);
                        }
                    });
            });
            // Quick-start preset buttons
            if !self.recording && !daemon_mode {
                ui.horizontal(|ui| {
                    ui.label("Quick:").on_hover_text("Start recording immediately with a preset duration — no need to press Start separately.");
                    for (label, mins, secs) in [("30s", 0u32, 30u64), ("1m", 1, 60), ("5m", 5, 300), ("10m", 10, 600)] {
                        if ui.small_button(label).on_hover_text(format!("Record for {label} then auto-stop.")).clicked() {
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
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    ui.colored_label(egui::Color32::GRAY, "No recordings yet.");
                    ui.add_space(4.0);
                    if ui.add(egui::Button::new(egui::RichText::new("⏺  Record a 30-second sample").size(13.0))
                            .min_size(egui::vec2(240.0, 30.0)))
                        .on_hover_text("Starts a 30-second timed WAV recording of whatever you're currently listening to. Great first recording!")
                        .clicked() && !self.recording && !daemon_mode
                    {
                        self.quick_duration_secs = 30;
                        self.record_audio = true;
                        self.start();
                    }
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Recordings are saved to the output directory above.").small().color(egui::Color32::GRAY));
                });
                ui.add_space(6.0);
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
                            } else if ui.small_button("🗑").on_hover_text("Delete this recording file.").clicked() {
                                self.delete_confirm = Some(f.name.clone());
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

        ui.add_space(12.0);
        ui.separator();
        // ── Recording Guide ───────────────────────────────────────────────
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("📡 Recording Guide")
                .size(16.0)
                .strong(),
        );
        ui.add_space(4.0);

        ui.collapsing("IQ vs Audio recording — what's the difference?", |ui| {
            ui.add_space(4.0);
            egui::Grid::new("rec_types").num_columns(2).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Format").strong());
                ui.label(egui::RichText::new("Best for").strong());
                ui.end_row();
                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "IQ (raw)");
                ui.label("Raw I/Q samples. Large files (~8 MB/s at 2 MHz bandwidth). Use when you want to replay the full RF spectrum later, change demodulation settings, or post-process with tools like GNU Radio.");
                ui.end_row();
                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), "WAV (audio)");
                ui.label("Demodulated audio only. Small files (~90 KB/s). Use for NOAA APT decoding with SatDump/WXtoIMG, archiving voice transmissions, or sharing recordings.");
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
                ui.separator();
                ui.label("For satellite passes: record WAV audio (WFM mode, 34–40 kHz bandwidth), then decode with SatDump. IQ recording is unnecessary for NOAA APT.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Squelch-triggered recording — record only when there's activity", |ui| {
            ui.add_space(4.0);
            ui.label("Enable 'Record on squelch open' to automatically start/stop recording based on signal activity:");
            ui.add_space(2.0);
            ui.label("  1. Set the squelch level on the SDR panel to cut off static");
            ui.label("  2. Enable 'Record on squelch open' here");
            ui.label("  3. Set a tail time (default 500 ms) — recording continues briefly after signal ends to catch the full transmission");
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
                ui.separator();
                ui.label("This mode is ideal for monitoring a frequency for long periods — you'll only get recordings when something happens, saving disk space.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Recording satellite passes — step by step", |ui| {
            ui.add_space(4.0);
            ui.label("1.  Go to the Satellite tab, check 'Auto-record on pass'");
            ui.label("2.  The scheduler will automatically start recording 2 minutes before AOS");
            ui.label("3.  Recording stops 1 minute after LOS");
            ui.label("4.  Find the WAV file in the output directory");
            ui.label("5.  Open in SatDump (File → Open Baseband → select your .wav)");
            ui.label("6.  Select the correct Meteor satellite and click Decode");
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Alternative manual workflow:").strong());
            ui.label("  1. Tune to the current Meteor LRPT frequency and record RAW CF32 IQ");
            ui.label("  2. Click 'Start Recording' 2 minutes before the pass");
            ui.label("  3. Stop after the pass, then decode the .wav file offline");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 180, 0), "NOTE");
                ui.separator();
                ui.label("The built-in Meteor decoder consumes CF32 IQ recordings. WAV audio remains useful for ordinary demodulated-audio inspection.");
            });
        });

        ui.add_space(4.0);
        ui.collapsing("Disk space & file management", |ui| {
            ui.add_space(4.0);
            ui.label("  •  IQ recordings grow at ~8 MB/second — a 5-minute file is ~2.4 GB.");
            ui.label("  •  WAV audio recordings grow at ~90 KB/second — a 15-minute satellite pass is ~80 MB.");
            ui.label("  •  Use the file list below to review and delete old recordings.");
            ui.label("  •  Output directory defaults to the current directory — set a dedicated location for large collections.");
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 80, 80), "AVOID");
                ui.separator();
                ui.label("Don't leave IQ recording running unattended. A 10-minute IQ recording at 2.4 MHz uses about 4.8 GB of disk space.");
            });
        });
    }

    fn cached_free_disk_space(&mut self) -> (f64, String) {
        if let Some(rx) = &self.disk_space_rx {
            if let Ok(val) = rx.try_recv() {
                self.disk_cache = (std::time::Instant::now(), val.0, val.1);
                self.disk_space_checking = false;
            }
        }

        let (cached_at, cached_gb, ref cached_unit) = self.disk_cache;
        if cached_at.elapsed() >= std::time::Duration::from_secs(5) && !self.disk_space_checking {
            self.disk_space_checking = true;
            let (tx, rx) = crossbeam_channel::bounded(1);
            self.disk_space_rx = Some(rx);
            let path = self.output_dir.clone();
            std::thread::spawn(move || {
                let res = free_disk_space_with_timeout(&path);
                let _ = tx.send(res);
            });
        }

        (cached_gb, cached_unit.clone())
    }
}

impl Drop for RecorderPanel {
    fn drop(&mut self) {
        // Finish queued IQ and the WAV header before the process can exit.
        self.stop_recording();
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
    // Disk-space probe failed (df missing / timed out / unparseable output).
    // Return 0.0 GB so start_recording's 500 MB pre-flight guard triggers and
    // the user is warned, instead of falsely reporting ~100 GB free and
    // silently proceeding to ENOSPC mid-write.
    (0.0, "GB".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_shared_state() -> Arc<Mutex<crate::app::SharedState>> {
        Arc::new(Mutex::new(crate::app::SharedState {
            source: crate::source_manager::SourceManager::new(),
            spectrum: crate::spectrum::SpectrumAnalyzer::new(),
            config: crate::config::AppConfig::default(),
            bookmarks: crate::bookmarks::BookmarkDb::load_or_default(),
            scheduler: crate::scheduler::Scheduler::new(),
            tle: crate::tle_engine::TleEngine::new(),
            demod_mode: crate::sdr_panel::DemodMode::Fm,
            recording: false,
            adsb_running: false,
            selected_satellite: None,
            audio_running: false,
            volume: 0.5,
            squelch: -50.0,
            lpf_cutoff: 15000.0,
            fm_deviation_hz: 0.0,
            audio_peak: 0.0,
            freq_history: std::collections::VecDeque::with_capacity(20),
            vfo_b: 0,
            freq_memory: std::array::from_fn(|_| crate::app::FreqMemEntry::default()),
            tune_step_fine_hz: 100_000,
            tune_step_coarse_hz: 1_000_000,
            lo_offset_hz: 0,
            mqtt_connected: false,
            mqtt_enabled: false,
            bookmarks_modified: true,
            scanner_command: None,
        }))
    }

    fn recording_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ezsdr-runtime-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dropping_recorder_flushes_all_queued_iq_before_returning() {
        let dir = recording_dir("drop-flush");
        let shared = make_shared_state();
        let mut panel = RecorderPanel::new(Arc::clone(&shared));
        panel.output_dir = dir.display().to_string();
        panel.filename_template = "shutdown".into();
        panel.start_recording();
        assert!(panel.recording, "{}", panel.last_error);
        let block = vec![17; 65_536];
        for _ in 0..32 {
            panel.write_samples(&block);
        }
        assert_eq!(panel.iq_dropped_chunks, 0);
        drop(panel);
        assert!(!shared.lock().unwrap().recording);
        assert_eq!(
            std::fs::read(dir.join("shutdown.iq")).unwrap(),
            block.repeat(32)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repeated_template_preserves_prior_recordings_and_escapes_metadata() {
        let dir = recording_dir("collision");
        let mut panel = RecorderPanel::new(make_shared_state());
        panel.output_dir = dir.display().to_string();
        panel.filename_template = "quoted\"station".into();
        for bytes in [b"first".as_slice(), b"second".as_slice()] {
            panel.start_recording();
            assert!(panel.recording, "{}", panel.last_error);
            panel.write_samples(bytes);
            panel.stop_recording();
        }
        assert_eq!(
            std::fs::read(dir.join("quoted\"station.iq")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(dir.join("quoted\"station_001.iq")).unwrap(),
            b"second"
        );
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("quoted\"station_001.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["files"][0], "quoted\"station_001.iq");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wav_uses_real_audio_rate_and_finalizes_on_a_rate_change() {
        let dir = recording_dir("wav-rate");
        let mut panel = RecorderPanel::new(make_shared_state());
        panel.output_dir = dir.display().to_string();
        panel.filename_template = "audio".into();
        panel.record_iq = false;
        panel.record_audio = true;
        panel.set_audio_sample_rate(44_100);
        panel.start_recording();
        assert!(panel.recording, "{}", panel.last_error);
        panel.write_audio_samples(&vec![0.25; 44_100]);
        panel.set_audio_sample_rate(48_000);
        assert!(!panel.recording);
        assert!(panel.last_error.contains("44100 to 48000"));
        let wav = hound::WavReader::open(dir.join("audio_audio.wav")).unwrap();
        assert_eq!(wav.spec().sample_rate, 44_100);
        assert_eq!(wav.duration(), 44_100);
        drop(wav);
        panel.start_recording();
        assert!(panel.recording, "{}", panel.last_error);
        panel.stop_recording();
        assert_eq!(
            hound::WavReader::open(dir.join("audio_001_audio.wav"))
                .unwrap()
                .spec()
                .sample_rate,
            48_000
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_recording_configuration_and_save_path_do_not_start() {
        let dir = recording_dir("invalid");
        let mut panel = RecorderPanel::new(make_shared_state());
        panel.output_dir = dir.display().to_string();
        panel.record_iq = false;
        panel.record_audio = false;
        panel.start_recording();
        assert!(!panel.recording);
        assert!(panel.last_error.contains("format"));
        panel.record_iq = true;
        panel.filename_template = "../escape".into();
        panel.start_recording();
        assert!(!panel.recording);
        assert!(panel.last_error.contains("path separators"));
        let file = dir.join("regular-file");
        std::fs::write(&file, b"preserved").unwrap();
        panel.output_dir = file.display().to_string();
        panel.start_recording();
        assert!(!panel.recording);
        assert!(panel.last_error.contains("directory"));
        assert_eq!(std::fs::read(file).unwrap(), b"preserved");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wav_write_failure_stops_recording_and_does_not_count_unwritten_tail() {
        let dir = recording_dir("write-error");
        let path = dir.join("read-only-handle");
        std::fs::write(&path, b"unchanged").unwrap();
        let mut panel = RecorderPanel::new(make_shared_state());
        panel.wav_writer = Some(
            hound::WavWriter::new(
                std::io::BufWriter::new(std::fs::File::open(&path).unwrap()),
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 48_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap(),
        );
        panel.recording = true;
        panel.write_audio_samples(&vec![0.25; 10_000]);
        assert!(!panel.recording);
        assert!(panel.last_error.contains("WAV write error"));
        assert!(panel.bytes_written < 20_000);
        assert_eq!(std::fs::read(path).unwrap(), b"unchanged");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn free_disk_space_returns_reasonable_value() {
        let (amount, unit) = free_disk_space_with_timeout("/");
        assert!(amount > 0.0, "disk space should be positive, got {amount}");
        assert_eq!(unit, "GB", "unit should be GB");
    }

    #[test]
    fn free_disk_space_nonexistent_path_returns_fallback() {
        let (amount, unit) = free_disk_space_with_timeout("/nonexistent_path_xyz123");
        // Fallback now reports 0.0 (not 99.9) so the pre-flight guard triggers
        // instead of falsely claiming ~100 GB free on a disk we can't probe.
        assert_eq!(amount, 0.0);
        assert_eq!(unit, "GB");
    }

    #[test]
    fn free_disk_space_tmp_dir() {
        let (amount, unit) = free_disk_space_with_timeout("/tmp");
        assert!(amount > 0.0);
        assert_eq!(unit, "GB");
    }

    #[test]
    fn free_disk_space_dot_returns_something() {
        let (amount, unit) = free_disk_space_with_timeout(".");
        assert!(amount > 0.0);
        assert_eq!(unit, "GB");
    }

    #[test]
    fn apply_filename_template_basic() {
        let panel = RecorderPanel::new(make_shared_state());
        let result =
            panel.apply_filename_template("{date}_{freq}MHz", "20240101_120000", 145.5, "NFM");
        assert_eq!(result, "20240101_120000_145.500MHz");
    }

    #[test]
    fn apply_filename_template_all_tokens() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template(
            "{date}_{freq}_{freq1}_{freq0}_{mode}",
            "20240101",
            145.525,
            "NFM",
        );
        assert_eq!(result, "20240101_145.525_145.5_146_NFM");
    }

    #[test]
    fn apply_filename_template_spaces_replaced() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("{date} test file", "20240101", 100.0, "AM");
        assert_eq!(result, "20240101_test_file");
    }

    #[test]
    fn apply_filename_template_no_tokens() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("static_name", "ignored", 0.0, "RAW");
        assert_eq!(result, "static_name");
    }

    #[test]
    fn apply_filename_template_empty_template() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("", "ts", 100.0, "FM");
        assert_eq!(result, "");
    }

    #[test]
    fn free_disk_space_special_chars_path() {
        let result = free_disk_space_with_timeout("/tmp/test path with spaces");
        assert_eq!(result, (0.0, "GB".to_string()));
    }

    #[test]
    fn free_disk_space_very_long_path() {
        let long_path = "a".repeat(4096);
        let result = free_disk_space_with_timeout(&long_path);
        assert_eq!(result, (0.0, "GB".to_string()));
    }

    #[test]
    fn apply_filename_template_only_date() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("{date}", "20240101_120000", 145.5, "NFM");
        assert_eq!(result, "20240101_120000");
    }

    #[test]
    fn apply_filename_template_only_freq() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("{freq}", "ignored", 145.5, "NFM");
        assert_eq!(result, "145.500");
    }

    #[test]
    fn apply_filename_template_only_mode() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("{mode}", "ignored", 100.0, "WFM");
        assert_eq!(result, "WFM");
    }

    #[test]
    fn apply_filename_template_adjacent_tokens() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("{date}{freq}{mode}", "20240101", 145.5, "NFM");
        assert_eq!(result, "20240101145.500NFM");
    }

    #[test]
    fn apply_filename_template_strange_chars_in_template() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("!@#$%^&*()", "ts", 100.0, "FM");
        assert_eq!(result, "!@#$%^&*()");
    }

    #[test]
    fn apply_filename_template_very_long_template() {
        let panel = RecorderPanel::new(make_shared_state());
        let long_template = "a".repeat(500) + "{date}" + &"b".repeat(500);
        let result = panel.apply_filename_template(&long_template, "ts", 100.0, "FM");
        assert_eq!(result.len(), 1002); // 500 + 2 + 500
        assert!(result.contains("ts"));
    }

    #[test]
    fn apply_filename_template_repeated_token() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("{freq}_{freq}", "ignored", 100.5, "FM");
        assert_eq!(result, "100.500_100.500");
    }

    #[test]
    fn apply_filename_template_case_sensitive_no_match() {
        let panel = RecorderPanel::new(make_shared_state());
        let result = panel.apply_filename_template("{Date}_{Freq}", "ts", 100.0, "FM");
        assert_eq!(result, "{Date}_{Freq}");
    }

    #[test]
    fn write_samples_not_recording_is_noop() {
        let mut panel = RecorderPanel::new(make_shared_state());
        panel.write_samples(&[1, 2, 3, 4]);
        assert_eq!(panel.bytes_written, 0);
    }

    #[test]
    fn iq_worker_roundtrip_writes_file() {
        // Issue 47: UI thread pushes via channel; worker owns the file.
        let dir = std::env::temp_dir().join(format!(
            "ezsdr_rec_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let shared = make_shared_state();
        {
            let mut state = shared.lock().unwrap();
            state.source.center_frequency_hz = Some(118_000_000);
            state.source.frequency_hz = 118_500_000;
            state.source.sample_rate_hz = 2_400_000;
            state.config.advanced.rf_decim = 8;
        }
        let mut panel = RecorderPanel::new(shared);
        panel.output_dir = dir.to_string_lossy().to_string();
        panel.record_iq = true;
        panel.record_audio = false;
        panel.start_recording();
        assert!(panel.recording, "should be recording: {}", panel.last_error);
        panel.write_samples(b"hello-iq");
        panel.write_samples(b"more-bytes");
        panel.stop_recording();
        assert!(!panel.recording);
        let fname = panel.last_filename.clone();
        assert!(!fname.is_empty());
        let content = std::fs::read(dir.join(&fname)).expect("recorded IQ file should exist");
        assert_eq!(content, b"hello-iqmore-bytes");
        let sidecar = dir.join(std::path::Path::new(&fname).with_extension("json"));
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(sidecar).unwrap()).unwrap();
        assert_eq!(metadata["frequency_hz"], 118_000_000);
        assert_eq!(metadata["tuned_frequency_hz"], 118_500_000);
        assert_eq!(metadata["sample_rate_hz"], 2_400_000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
