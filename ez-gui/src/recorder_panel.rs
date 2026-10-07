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
    // The native capture consumer enqueues into a bounded disk writer. These
    // handles also support direct-input fixtures before transfer to the consumer.
    iq_tx: Option<crossbeam_channel::Sender<Vec<u8>>>,
    iq_worker: Option<std::thread::JoinHandle<Result<u64, String>>>,
    continuous_iq:
        Option<crate::capture_recording::CaptureRecorder<crate::raw_recording::RawRecordingResult>>,
    native_iq_bytes: u64,
    audio_stream: Option<crate::demod::RadioAudioStream>,
    continuous_audio: Option<crate::audio_recording::AudioRecorder>,
    native_audio_bytes: u64,
    #[cfg(test)]
    direct_iq_fixture: bool,
    iq_dropped_chunks: u64,
    capture: Option<ez_proto::RecordingCapture>,
    metadata: Option<(std::path::PathBuf, serde_json::Value)>,
    iq_file_path: Option<std::path::PathBuf>,
    recording_failure: bool,
    stop_reason: Option<String>,
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
            continuous_iq: None,
            native_iq_bytes: 0,
            audio_stream: None,
            continuous_audio: None,
            native_audio_bytes: 0,
            #[cfg(test)]
            direct_iq_fixture: false,
            iq_dropped_chunks: 0,
            capture: None,
            metadata: None,
            iq_file_path: None,
            recording_failure: false,
            stop_reason: None,
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

    #[cfg(test)]
    fn new_direct(shared: Arc<Mutex<SharedState>>) -> Self {
        let mut panel = Self::new(shared);
        panel.direct_iq_fixture = true;
        panel
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
        self.last_error = status.stop_reason.clone().unwrap_or_default();
        self.daemon_recording = Some(status);
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
        self.request_stop();
    }

    fn request_stop(&mut self) -> bool {
        if !self.daemon_mode() {
            self.stop_recording();
            return true;
        }
        let result = self
            .shared
            .try_lock()
            .map_err(|_| "source state is busy; try again".to_string())
            .and_then(|mut state| state.source.stop_daemon_recording());
        match result {
            Ok(()) => {
                self.last_error = "Stopping daemon recording…".to_string();
                true
            }
            Err(error) => {
                self.last_error = error;
                false
            }
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
        if self.wav_writer.is_some() || self.continuous_audio.is_some() {
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

    pub(crate) fn set_audio_stream(&mut self, stream: Option<crate::demod::RadioAudioStream>) {
        self.audio_stream = stream;
    }

    pub fn start_recording(&mut self) {
        if self.recording {
            return;
        }
        let output_dir = self.output_dir.clone();
        self.capture = None;
        self.metadata = None;
        self.iq_file_path = None;
        self.recording_failure = false;
        self.stop_reason = None;
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

        let (
            freq_hz,
            tuned_hz,
            offset_hz,
            sample_rate_hz,
            gain_db,
            ppm_correction,
            demod_label,
            capture_stream,
        ) = if let Ok(state) = self.shared.try_lock() {
            let (center, rate, gain) = state.source.applied_capture_settings().unwrap_or((
                state.source.capture_center_frequency_hz(),
                state.source.sample_rate_hz,
                state.source.gain_db,
            ));
            (
                center,
                state.source.frequency_hz,
                state.source.frequency_offset_hz,
                rate,
                gain,
                state.source.ppm_correction,
                state.demod_mode.label().to_string(),
                state.source.capture_stream(),
            )
        } else {
            self.last_error = "Recorder: source is busy; try again".into();
            return;
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
                    let (tx, handle) =
                        crate::raw_recording::spawn_disk_writer(std::io::BufWriter::new(file));
                    self.iq_tx = Some(tx);
                    self.iq_worker = Some(handle);
                    self.iq_dropped_chunks = 0;
                    // Keep iq_writer None: writes go via the channel. The
                    // field is retained for API compat (always None when the
                    // worker is active).
                    self.iq_writer = None;
                    self.iq_file_path = Some(path);
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
            "complete": false,
            "format": if self.record_iq { "cu8" } else { "wav16" },
            "capture": null,
            "stop_reason": null,
            "lost_samples": 0,
            "capture_input_dropped_blocks": 0,
            "capture_input_lost_samples": 0,
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

        self.metadata = Some((sidecar_path, json));
        self.recording = true;
        self.start_time = Some(std::time::Instant::now());
        self.bytes_written = 0;
        self.native_iq_bytes = 0;
        self.native_audio_bytes = 0;
        #[cfg(test)]
        let continuous = !self.direct_iq_fixture;
        #[cfg(not(test))]
        let continuous = true;
        if continuous {
            if let (Some(tx), Some(disk), Some(path)) = (
                self.iq_tx.take(),
                self.iq_worker.take(),
                self.iq_file_path.clone(),
            ) {
                let (sidecar, metadata) =
                    self.metadata.as_ref().expect("created recording metadata");
                let writer = crate::raw_recording::RawCaptureWriter::new(
                    tx,
                    disk,
                    path,
                    sidecar.clone(),
                    metadata.clone(),
                    self.wav_writer.is_some(),
                );
                self.continuous_iq = Some(crate::capture_recording::CaptureRecorder::start(
                    capture_stream,
                    writer,
                    self.recording_deadline(),
                ));
            }
            if let Some(stream) = self.audio_stream.clone() {
                if let Some(writer) = self.wav_writer.take() {
                    let (sidecar, metadata) =
                        self.metadata.as_ref().expect("created recording metadata");
                    self.continuous_audio = Some(crate::audio_recording::AudioRecorder::start(
                        stream,
                        writer,
                        sidecar.with_extension("wav.json"),
                        metadata.clone(),
                        self.recording_deadline(),
                    ));
                }
            }
        }
        if let Ok(mut state) = self.shared.try_lock() {
            state.recording = true;
        }
    }

    pub fn stop_recording(&mut self) {
        if let Some(recorder) = &self.continuous_audio {
            recorder.request_stop();
        }
        if let Some(recorder) = &self.continuous_iq {
            recorder.request_stop();
        }
        if let Some(mut recorder) = self.continuous_iq.take() {
            match recorder.finish() {
                Ok(result) => {
                    self.update_native_iq_bytes(result.bytes_written);
                    self.capture = result.capture;
                    self.iq_dropped_chunks = result.dropped_chunks;
                    if let Some((_, metadata)) = &mut self.metadata {
                        *metadata = result.metadata;
                    }
                    if let Some(error) = result.error {
                        self.recording_failure = true;
                        self.stop_reason = Some(error.clone());
                        self.last_error = error;
                    }
                }
                Err(error) => {
                    self.capture = recorder.progress().capture.clone();
                    self.recording_failure = true;
                    self.stop_reason = Some(error.clone());
                    self.last_error = error;
                }
            }
        }
        if let Some(mut recorder) = self.continuous_audio.take() {
            match recorder.finish() {
                Ok(result) => {
                    self.update_native_audio_bytes(result.bytes);
                    if let Some((_, metadata)) = &mut self.metadata {
                        metadata["audio_capture"] =
                            serde_json::to_value(result.capture).unwrap_or_default();
                        for name in [
                            "audio_samples",
                            "audio_bytes_total",
                            "audio_sample_rate_hz",
                            "audio_duration_sec",
                            "audio_input_dropped_blocks",
                            "audio_native_lost_samples",
                            "audio_input_lost_samples",
                            "audio_native_partial_frame",
                            "peak_level_dbfs",
                        ] {
                            metadata[name] = result.metadata[name].clone();
                        }
                    }
                    if let Some(error) = result.error {
                        self.recording_failure = true;
                        self.stop_reason = Some(error.clone());
                        self.last_error = error;
                    }
                }
                Err(error) => {
                    self.recording_failure = true;
                    self.stop_reason = Some(error.clone());
                    self.last_error = error;
                }
            }
        }
        if let Some(mut writer) = self.iq_writer.take() {
            use std::io::Write;
            if let Err(error) = writer.flush() {
                self.recording_failure = true;
                self.last_error = format!("IQ flush failed: {error}");
            }
        }
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
                        self.recording_failure = true;
                        self.last_error = format!(
                            "IQ writer dropped {} chunks (disk too slow)",
                            self.iq_dropped_chunks
                        );
                    }
                }
                Ok(Err(e)) => {
                    self.recording_failure = true;
                    self.last_error = e;
                }
                Err(_) => {
                    self.recording_failure = true;
                    self.last_error = "IQ writer thread panicked".to_string();
                }
            }
        }
        if let Some(w) = self.wav_writer.take() {
            if let Err(e) = w.finalize() {
                self.recording_failure = true;
                self.last_error = format!("Failed to finalize WAV: {e}");
            }
        }
        if let Some((path, mut metadata)) = self.metadata.take() {
            if let Some(iq_path) = &self.iq_file_path {
                match std::fs::metadata(iq_path) {
                    Ok(file) => {
                        let bytes = file.len();
                        metadata["iq_bytes_total"] = bytes.into();
                        if let Some(capture) = &mut self.capture {
                            if capture.samples.saturating_mul(2) != bytes {
                                self.recording_failure = true;
                                metadata["lost_samples"] =
                                    capture.samples.saturating_sub(bytes / 2).into();
                                capture.samples = bytes / 2;
                                capture.next_source_sample_index = capture
                                    .first_clock
                                    .offset_samples(capture.samples)
                                    .map_or(capture.first_clock.source_sample_index, |clock| {
                                        clock.source_sample_index
                                    });
                            }
                        }
                        if !bytes.is_multiple_of(2) {
                            self.recording_failure = true;
                        }
                    }
                    Err(error) => {
                        self.recording_failure = true;
                        self.last_error = format!("IQ file metadata failed: {error}");
                    }
                }
            }
            metadata["capture"] =
                serde_json::to_value(&self.capture).unwrap_or(serde_json::Value::Null);
            if let Some(capture) = &self.capture {
                metadata["duration_sec"] = (capture.samples as f64
                    * f64::from(capture.first_clock.sample_stride)
                    / f64::from(capture.first_clock.source_sample_rate_hz))
                .into();
            }
            metadata["complete"] = (!self.recording_failure).into();
            metadata["stop_reason"] = self
                .stop_reason
                .clone()
                .or_else(|| self.recording_failure.then(|| self.last_error.clone()))
                .into();
            metadata["end_utc"] = chrono::Utc::now().to_rfc3339().into();
            let result = std::fs::File::create(&path)
                .map_err(|error| error.to_string())
                .and_then(|file| {
                    serde_json::to_writer_pretty(file, &metadata).map_err(|error| error.to_string())
                });
            if let Err(error) = result {
                self.last_error = format!("Failed to finalize recording metadata: {error}");
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

    /// Lifecycle deadlines run from the model even when controls are hidden.
    pub fn tick(&mut self) {
        if let Some(recorder) = &self.continuous_audio {
            recorder.set_deadline(self.recording_deadline());
            let progress = recorder.progress();
            let peak = recorder.peak_level();
            if peak > self.peak_level_dbfs {
                self.peak_level_dbfs = peak;
                self.peak_hold_time = Some(std::time::Instant::now());
            }
            self.update_native_audio_bytes(progress.bytes_written);
            if progress.stopped {
                self.stop_recording();
                return;
            }
        }
        if let Some(recorder) = &self.continuous_iq {
            recorder.set_deadline(self.recording_deadline());
            let progress = recorder.progress();
            self.update_native_iq_bytes(progress.bytes_written);
            self.capture.clone_from(&progress.capture);
            if progress.stopped {
                self.stop_recording();
                return;
            }
        }
        let limit = if self.quick_duration_secs > 0 {
            self.quick_duration_secs
        } else {
            u64::from(self.max_duration_mins) * 60
        };
        if self.recording
            && limit > 0
            && self
                .start_time
                .is_some_and(|start| start.elapsed().as_secs() >= limit)
            && self.request_stop()
        {
            self.quick_duration_secs = 0;
            self.start_time = None;
        }
    }

    fn update_native_iq_bytes(&mut self, bytes: u64) {
        self.bytes_written = self
            .bytes_written
            .saturating_sub(self.native_iq_bytes)
            .saturating_add(bytes);
        self.native_iq_bytes = bytes;
    }

    fn update_native_audio_bytes(&mut self, bytes: u64) {
        self.bytes_written = self
            .bytes_written
            .saturating_sub(self.native_audio_bytes)
            .saturating_add(bytes);
        self.native_audio_bytes = bytes;
    }

    fn recording_deadline(&self) -> Option<std::time::Instant> {
        let limit = if self.quick_duration_secs > 0 {
            self.quick_duration_secs
        } else {
            u64::from(self.max_duration_mins) * 60
        };
        self.start_time
            .filter(|_| limit > 0)?
            .checked_add(std::time::Duration::from_secs(limit))
    }

    #[cfg(test)]
    pub(crate) fn write_capture(
        &mut self,
        samples: &[u8],
        clock: ez_proto::CaptureClock,
        center_hz: u64,
    ) {
        if !self.recording || self.iq_tx.is_none() && self.iq_writer.is_none() || samples.is_empty()
        {
            return;
        }
        let rate = clock.source_sample_rate_hz / clock.sample_stride.max(1);
        let geometry_matches = self.metadata.as_ref().is_none_or(|(_, metadata)| {
            metadata["frequency_hz"].as_u64() == Some(center_hz)
                && metadata["sample_rate_hz"].as_u64() == Some(u64::from(rate))
        });
        let next = if !samples.len().is_multiple_of(2) {
            Err("Recording stopped: incomplete I/Q sample".into())
        } else if geometry_matches {
            ez_proto::RecordingCapture::accept(
                self.capture.as_ref(),
                clock,
                center_hz,
                rate,
                samples.len() as u64 / 2,
            )
        } else {
            Err(
                "Recording stopped: capture tuning or sample rate differs from the file header"
                    .into(),
            )
        };
        match next {
            Ok(next) => {
                if self.write_samples(samples) {
                    self.capture = Some(next);
                }
            }
            Err(reason) => {
                self.stop_reason = Some(reason.clone());
                self.recording_failure = true;
                self.last_error = reason;
                self.stop_recording();
            }
        }
    }

    // The production ingestion path checks the capture contract above first.
    #[cfg(test)]
    fn write_samples(&mut self, samples: &[u8]) -> bool {
        self.tick();
        if !self.recording || samples.is_empty() {
            return false;
        }
        if let Some(writer) = &mut self.iq_writer {
            use std::io::Write;
            if let Err(error) = writer.write_all(samples) {
                self.recording_failure = true;
                self.last_error = format!("Write error: {error}");
                self.stop_recording();
                return false;
            }
            self.bytes_written += samples.len() as u64;
            return true;
        }
        if let Some(tx) = &self.iq_tx {
            match tx.try_send(samples.to_vec()) {
                Ok(()) => {
                    self.bytes_written += samples.len() as u64;
                    return true;
                }
                Err(crossbeam_channel::TrySendError::Full(_)) => {
                    self.iq_dropped_chunks += 1;
                    self.recording_failure = true;
                    self.stop_reason = Some("Recording stopped: IQ writer queue overflow".into());
                    self.last_error = self.stop_reason.clone().unwrap();
                    self.stop_recording();
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    self.recording_failure = true;
                    self.last_error = "IQ writer disconnected".to_string();
                    self.stop_recording();
                }
            }
        }
        false
    }

    #[cfg(test)]
    pub fn write_audio_samples(&mut self, audio: &[f32]) {
        self.tick();
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
                {
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

    #[test]
    fn native_recording_consumers_finalize_each_clock_boundary_without_gui_ticks() {
        for boundary in 0..6 {
            let dir = recording_dir(&format!("native-boundary-{boundary}"));
            let shared = make_shared_state();
            let stream = {
                let mut state = shared.lock().unwrap();
                state.source.frequency_hz = 1_090_000_000;
                state.source.sample_rate_hz = 2_400_000;
                state.source.capture_stream()
            };
            stream.start(1);
            let mut panel = RecorderPanel::new(shared);
            panel.output_dir = dir.display().to_string();
            panel.filename_template = "native".into();
            panel.start_recording();
            let mut cf32 = crate::satellite::recorder::Cf32StreamWriter::start(
                dir.to_str().unwrap(),
                None,
                2_400_000,
                1_090_000_000,
                0.0,
                0.0,
            )
            .unwrap()
            .with_capture(stream.clone());
            let mut clock = ez_proto::CaptureClock::new(90, 700, 16_800_003, None);
            clock.sample_stride = 7;
            clock.discontinuous = true;
            let publish =
                |samples: Vec<u8>, clock: ez_proto::CaptureClock, index: u64, center: u64| {
                    let mut block =
                        crate::source_manager::CaptureBlock::fixture(samples, clock, 1, index);
                    block.center_hz = center;
                    block.session = Some(stream.session());
                    stream.publish(&block);
                };
            let first = [90, 110].repeat(64);
            let second = [140, 160].repeat(32);
            publish(first.clone(), clock, 0, 1_090_000_000);
            let mut next = clock.offset_samples(64).unwrap();
            next.discontinuous = false;
            publish(second.clone(), next, 64, 1_090_000_000);
            let mut bad = next.offset_samples(32).unwrap();
            let mut center = 1_090_000_000;
            match boundary {
                0 => {
                    bad.epoch += 1;
                    bad.source_sample_index = 0;
                }
                1 => center += 1_000_000,
                2 => bad.source_sample_rate_hz = 14_000_000,
                3 => bad.source_sample_index += 7,
                4 => bad.discontinuous = true,
                _ => bad.origin_unix_ns = Some(123),
            }
            publish([200, 210].repeat(10), bad, 96, center);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !panel.continuous_iq.as_ref().unwrap().progress().stopped {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(cf32.stop().unwrap_err().contains("Recording stopped"));
            // File and sidecar are final before the hidden frontend observes it.
            let expected = [first, second].concat();
            assert_eq!(std::fs::read(dir.join("native.iq")).unwrap(), expected);
            assert_eq!(
                std::fs::read(cf32.path()).unwrap(),
                lrpt_decode::uc8_bytes_to_cf32_le(&expected)
            );
            let raw_meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("native.json")).unwrap()).unwrap();
            let cf_meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(cf32.path().with_extension("json")).unwrap())
                    .unwrap();
            assert_eq!(raw_meta["capture"], cf_meta["capture"]);
            assert_eq!(raw_meta["capture"]["samples"], 96);
            assert_eq!(raw_meta["capture"]["next_source_sample_index"], 1372);
            assert_eq!(
                raw_meta["capture"]["first_clock"]["source_sample_rate_hz"],
                16_800_003
            );
            assert_eq!(raw_meta["capture"]["first_clock"]["sample_stride"], 7);
            assert_eq!(raw_meta["complete"], false);
            assert_eq!(cf_meta["complete"], false);
            assert_eq!(raw_meta["lost_samples"], 0);
            assert_eq!(cf_meta["lost_samples"], 0);
            assert!(panel.recording);
            panel.tick();
            assert!(!panel.recording);
            drop(panel);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn native_recording_deadline_finalizes_files_without_gui_ticks() {
        let dir = recording_dir("native-deadline");
        let shared = make_shared_state();
        {
            let mut state = shared.lock().unwrap();
            state.source.source_mode = crate::source_manager::SourceMode::Hardware;
            state.source.frequency_hz = 118_000_000;
            state.source.sample_rate_hz = 2_400_000;
            state
                .source
                .test_set_hardware_open(|_| Ok([90, 110].repeat(16_384)));
        }
        let mut panel = RecorderPanel::new(Arc::clone(&shared));
        panel.output_dir = dir.display().to_string();
        panel.filename_template = "deadline".into();
        panel.quick_duration_secs = 1;
        panel.start_recording();
        shared.lock().unwrap().source.start();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !panel.continuous_iq.as_ref().unwrap().progress().stopped {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // The backend flushed both file and sidecar while the GUI model stayed
        // untouched. A later UI tick only observes the completed result.
        let bytes = std::fs::read(dir.join("deadline.iq")).unwrap();
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("deadline.json")).unwrap()).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(metadata["complete"], true);
        assert_eq!(metadata["capture"]["samples"], bytes.len() as u64 / 2);
        assert!(metadata["duration_sec"].as_f64().unwrap() > 0.5);
        assert!(metadata["duration_sec"].as_f64().unwrap() <= 1.1);
        assert!(panel.recording);
        panel.tick();
        assert!(!panel.recording);
        shared.lock().unwrap().source.stop();
        drop(panel);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn raw_iq_recording_finalizes_before_any_epoch_geometry_or_gap_boundary() {
        for boundary in 0..6 {
            let dir = recording_dir(&format!("capture-boundary-{boundary}"));
            let shared = make_shared_state();
            {
                let mut state = shared.lock().unwrap();
                state.source.frequency_hz = 1_090_000_000;
                state.source.sample_rate_hz = 2_400_000;
            }
            let mut panel = RecorderPanel::new_direct(shared.clone());
            panel.output_dir = dir.display().to_string();
            panel.filename_template = "prefix".into();
            panel.start_recording();
            let mut clock = ez_proto::CaptureClock::new(9, 700, 2_400_000, None);
            clock.discontinuous = true;
            let first = [90, 110].repeat(40);
            let second = [140, 160].repeat(25);
            panel.write_capture(&first, clock, 1_090_000_000);
            let mut next = clock.offset_samples(40).unwrap();
            next.discontinuous = false;
            panel.write_capture(&second, next, 1_090_000_000);
            let mut bad = next.offset_samples(25).unwrap();
            let mut center = 1_090_000_000;
            match boundary {
                0 => {
                    bad.epoch += 1;
                    bad.source_sample_index = 0;
                }
                1 => center += 1_000_000,
                2 => bad.source_sample_rate_hz = 2_000_000,
                3 => bad.source_sample_index += 17,
                4 => bad.discontinuous = true,
                _ => bad.origin_unix_ns = Some(123),
            }
            panel.write_capture(&[200, 210].repeat(20), bad, center);
            assert!(!panel.recording);
            assert!(!shared.lock().unwrap().recording);
            assert!(panel.last_error.contains("Recording stopped"));
            panel.write_capture(&[200, 210], bad, center);
            let expected = [first, second].concat();
            assert_eq!(std::fs::read(dir.join("prefix.iq")).unwrap(), expected);
            let metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("prefix.json")).unwrap()).unwrap();
            assert_eq!(metadata["frequency_hz"], 1_090_000_000u64);
            assert_eq!(metadata["sample_rate_hz"], 2_400_000);
            assert_eq!(metadata["capture"]["first_clock"]["epoch"], 9);
            assert_eq!(
                metadata["capture"]["first_clock"]["source_sample_index"],
                700
            );
            assert_eq!(
                metadata["capture"]["first_clock"]["origin_unix_ns"],
                serde_json::Value::Null
            );
            assert_eq!(metadata["capture"]["samples"], 65);
            assert_eq!(metadata["capture"]["next_source_sample_index"], 765);
            assert_eq!(metadata["complete"], false);
            assert!(metadata["stop_reason"]
                .as_str()
                .unwrap()
                .contains("Recording stopped"));
            // Reopen this finalized prefix independently through the production
            // replay source and compare every byte with its source samples.
            let mut replay = crate::source_manager::SourceManager::new();
            replay.source_mode = crate::source_manager::SourceMode::Replay;
            replay.replay_file = Some(dir.join("prefix.iq").display().to_string());
            replay.sample_rate_hz = 2_400_000;
            replay.start();
            let mut bytes = Vec::new();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if let Some(block) = replay.recv_capture_block() {
                    bytes.extend(block.samples);
                }
                if replay.status == crate::source_manager::SourceStatus::Idle {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            replay.stop();
            assert_eq!(bytes, expected);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn raw_iq_queue_overflow_ends_the_file_before_later_samples_can_be_appended() {
        let dir = recording_dir("capture-writer-overflow");
        let shared = make_shared_state();
        let mut panel = RecorderPanel::new_direct(shared.clone());
        panel.output_dir = dir.display().to_string();
        panel.filename_template = "overflow".into();
        panel.start_recording();
        let (tx, rx) = crossbeam_channel::bounded(1);
        tx.send(vec![1, 2]).unwrap();
        let old = panel.iq_tx.replace(tx);
        drop(old); // actual file worker exits; replacement queue is deliberately full
        let state = shared.lock().unwrap();
        let center = state.source.capture_center_frequency_hz();
        let rate = state.source.sample_rate_hz;
        drop(state);
        let clock = ez_proto::CaptureClock::new(1, 0, rate, None);
        panel.write_capture(&[90, 110], clock, center);
        assert!(!panel.recording);
        assert_eq!(panel.iq_dropped_chunks, 1);
        panel.write_capture(&[140, 160], clock.offset_samples(1).unwrap(), center);
        assert!(std::fs::read(dir.join("overflow.iq")).unwrap().is_empty());
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("overflow.json")).unwrap()).unwrap();
        assert_eq!(metadata["complete"], false);
        assert!(metadata["stop_reason"]
            .as_str()
            .unwrap()
            .contains("overflow"));
        drop(rx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn make_shared_state() -> Arc<Mutex<crate::app::SharedState>> {
        Arc::new(Mutex::new(crate::app::SharedState {
            source: crate::source_manager::SourceManager::new(),
            spectrum: crate::spectrum::SpectrumAnalyzer::new(),
            config: crate::config::AppConfig::default(),
            input_format: dump1090::demod::InputFormat::Uc8,
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
            last_manual_tune: None,
            runtime_actions: std::collections::VecDeque::new(),
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
        let mut panel = RecorderPanel::new_direct(Arc::clone(&shared));
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
        let mut panel = RecorderPanel::new_direct(make_shared_state());
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
        let mut panel = RecorderPanel::new_direct(make_shared_state());
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
        let mut panel = RecorderPanel::new_direct(make_shared_state());
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
        let mut panel = RecorderPanel::new_direct(make_shared_state());
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
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result =
            panel.apply_filename_template("{date}_{freq}MHz", "20240101_120000", 145.5, "NFM");
        assert_eq!(result, "20240101_120000_145.500MHz");
    }

    #[test]
    fn apply_filename_template_all_tokens() {
        let panel = RecorderPanel::new_direct(make_shared_state());
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
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("{date} test file", "20240101", 100.0, "AM");
        assert_eq!(result, "20240101_test_file");
    }

    #[test]
    fn apply_filename_template_no_tokens() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("static_name", "ignored", 0.0, "RAW");
        assert_eq!(result, "static_name");
    }

    #[test]
    fn apply_filename_template_empty_template() {
        let panel = RecorderPanel::new_direct(make_shared_state());
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
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("{date}", "20240101_120000", 145.5, "NFM");
        assert_eq!(result, "20240101_120000");
    }

    #[test]
    fn apply_filename_template_only_freq() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("{freq}", "ignored", 145.5, "NFM");
        assert_eq!(result, "145.500");
    }

    #[test]
    fn apply_filename_template_only_mode() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("{mode}", "ignored", 100.0, "WFM");
        assert_eq!(result, "WFM");
    }

    #[test]
    fn apply_filename_template_adjacent_tokens() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("{date}{freq}{mode}", "20240101", 145.5, "NFM");
        assert_eq!(result, "20240101145.500NFM");
    }

    #[test]
    fn apply_filename_template_strange_chars_in_template() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("!@#$%^&*()", "ts", 100.0, "FM");
        assert_eq!(result, "!@#$%^&*()");
    }

    #[test]
    fn apply_filename_template_very_long_template() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let long_template = "a".repeat(500) + "{date}" + &"b".repeat(500);
        let result = panel.apply_filename_template(&long_template, "ts", 100.0, "FM");
        assert_eq!(result.len(), 1002); // 500 + 2 + 500
        assert!(result.contains("ts"));
    }

    #[test]
    fn apply_filename_template_repeated_token() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("{freq}_{freq}", "ignored", 100.5, "FM");
        assert_eq!(result, "100.500_100.500");
    }

    #[test]
    fn apply_filename_template_case_sensitive_no_match() {
        let panel = RecorderPanel::new_direct(make_shared_state());
        let result = panel.apply_filename_template("{Date}_{Freq}", "ts", 100.0, "FM");
        assert_eq!(result, "{Date}_{Freq}");
    }

    #[test]
    fn write_samples_not_recording_is_noop() {
        let mut panel = RecorderPanel::new_direct(make_shared_state());
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
        let mut panel = RecorderPanel::new_direct(shared);
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
