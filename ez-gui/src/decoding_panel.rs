use crossbeam_channel::Receiver;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingFormat {
    Cs8,
    Cf32,
}

impl RecordingFormat {
    fn from_path(path: &str) -> Option<Self> {
        match Path::new(path)
            .extension()?
            .to_str()?
            .to_ascii_lowercase()
            .as_str()
        {
            "cs8" => Some(Self::Cs8),
            "cf32" => Some(Self::Cf32),
            _ => None,
        }
    }

    fn bytes_per_sample(self) -> usize {
        match self {
            Self::Cs8 => 2,
            Self::Cf32 => 8,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Cs8 => "CS8 · signed 8-bit I/Q",
            Self::Cf32 => "CF32 · little-endian float I/Q",
        }
    }
}

#[derive(Debug)]
enum FileDecodeOutcome {
    Complete(lrpt_decode::DecodeResult),
    Cancelled,
}

#[derive(Clone, Copy)]
enum FileOperationKind {
    Import,
    Export,
}

enum FileOperationOutcome {
    Import(std::path::PathBuf),
    Export {
        folder: std::path::PathBuf,
        count: usize,
    },
}

struct FileOperation {
    kind: FileOperationKind,
    receiver: Receiver<Result<Option<FileOperationOutcome>, String>>,
}

fn validate_rates(sample_rate: u32, symbol_rate: u32) -> Result<(), String> {
    if sample_rate == 0 || symbol_rate == 0 {
        return Err("Sample and symbol rates must be greater than zero.".into());
    }
    if u64::from(sample_rate) < u64::from(symbol_rate) * 2 {
        return Err("Sample rate must be at least twice the symbol rate.".into());
    }
    Ok(())
}

/// Stream regular files in small, sample-aligned blocks so Cancel remains responsive.
/// Decoder snapshots are forwarded without blocking or accumulating full-size images.
fn decode_recording(
    path: &Path,
    format: RecordingFormat,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: crossbeam_channel::Sender<lrpt_decode::DecodeProgress>,
    cancel: &AtomicBool,
    bytes_read: &AtomicU64,
) -> Result<FileDecodeOutcome, String> {
    validate_rates(sample_rate, symbol_rate)?;
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular IQ recording file.".into());
    }
    let sample_bytes = format.bytes_per_sample();
    if metadata.len() == 0 || metadata.len() % sample_bytes as u64 != 0 {
        return Err(format!(
            "Recording is empty or has an incomplete I/Q sample; {} needs {sample_bytes} bytes per sample.",
            format.label()
        ));
    }
    let (decoder_tx, decoder_rx) = crossbeam_channel::bounded(1);
    let mut decoder = lrpt_decode::LrptDecoder::new(sample_rate, symbol_rate, decoder_tx);
    let mut reader = std::io::BufReader::new(file);
    let mut chunk = vec![0; 16_384 * sample_bytes];
    let mut remaining = metadata.len();
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(FileDecodeOutcome::Cancelled);
        }
        let length = remaining.min(chunk.len() as u64) as usize;
        reader
            .read_exact(&mut chunk[..length])
            .map_err(|error| error.to_string())?;
        match format {
            RecordingFormat::Cs8 => decoder.push_samples_cs8(&chunk[..length]),
            RecordingFormat::Cf32 => {
                if let Some(component) = chunk[..length]
                    .chunks_exact(4)
                    .position(|bytes| !f32::from_le_bytes(bytes.try_into().unwrap()).is_finite())
                {
                    let sample = (metadata.len() - remaining) / 8 + component as u64 / 2;
                    return Err(format!(
                        "Recording contains a non-finite CF32 value at sample {sample}. Use finite little-endian float I/Q samples."
                    ));
                }
                decoder.push_samples_cf32(&chunk[..length]);
            }
        }
        remaining -= length as u64;
        bytes_read.fetch_add(length as u64, Ordering::Relaxed);
        if let Some(progress) = decoder_rx.try_iter().last() {
            let _ = progress_tx.try_send(progress);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(FileDecodeOutcome::Cancelled);
    }
    decoder
        .finish()
        .map(FileDecodeOutcome::Complete)
        .ok_or_else(|| lrpt_decode::LrptError::NoSync.to_string())
}

/// Save each channel without overwriting a previous export from this or another pass.
fn export_pngs(
    folder: &Path,
    recording: &str,
    result: &lrpt_decode::DecodeResult,
) -> Result<usize, String> {
    let stem = Path::new(recording)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("meteor");
    for (apid, image) in &result.images {
        let mut suffix = 0u32;
        let (path, file) = loop {
            let filename = if suffix == 0 {
                format!("{stem}_apid_{apid}.png")
            } else {
                format!("{stem}_apid_{apid}_{suffix}.png")
            };
            let path = folder.join(filename);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => break (path, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    suffix = suffix.checked_add(1).ok_or("Too many existing exports")?;
                }
                Err(error) => return Err(format!("Could not save {}: {error}", path.display())),
            }
        };
        let mut writer = std::io::BufWriter::new(file);
        let saved = image
            .write_to(&mut writer, image::ImageFormat::Png)
            .map_err(|error| error.to_string())
            .and_then(|()| writer.flush().map_err(|error| error.to_string()));
        if let Err(error) = saved {
            drop(writer);
            let _ = std::fs::remove_file(&path);
            return Err(format!("Could not save {}: {error}", path.display()));
        }
    }
    Ok(result.images.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecodePreset {
    MeteorM2_3,
    MeteorM2_4,
    #[default]
    Custom,
}

impl DecodePreset {
    pub fn sample_rate(&self) -> u32 {
        2_048_000
    }
    pub fn symbol_rate(&self) -> u32 {
        80_000
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::MeteorM2_3 => "Meteor-M2-3 (137.9 MHz, LRPT)",
            Self::MeteorM2_4 => "Meteor-M2-4 (137.1 MHz, LRPT)",
            Self::Custom => "Custom",
        }
    }
    pub fn all() -> &'static [DecodePreset] {
        &[Self::MeteorM2_3, Self::MeteorM2_4, Self::Custom]
    }
}

/// Maps a tracked satellite's display name to its decode preset. Returns `None`
/// for satellites with no decoder (NOAA/APT, ISS/Voice-APRS) — LRPT decoding
/// only exists for Meteor-M2.
pub fn satellite_to_preset(name: &str) -> Option<DecodePreset> {
    let n = name.to_ascii_lowercase();
    if n.contains("meteor-m2-3") {
        Some(DecodePreset::MeteorM2_3)
    } else if n.contains("meteor-m2-4") {
        Some(DecodePreset::MeteorM2_4)
    } else {
        None
    }
}

pub struct DecodeRequest {
    pub file_path: String,
    pub preset: Option<DecodePreset>,
    pub sample_rate: u32,
    pub satellite_name: Option<String>,
}

#[derive(Default)]
pub struct DecodingPanel {
    pub file_path: String,
    pub preset: DecodePreset,
    pub sample_rate: u32,
    pub symbol_rate: u32,
    pub satellite_name: Option<String>,
    pub running: bool,
    pub progress: Option<lrpt_decode::DecodeProgress>,
    pub result: Option<Arc<lrpt_decode::DecodeResult>>,
    pub error: Option<String>,
    pub no_decoder_reason: Option<String>,
    pub pending_status: Option<String>,
    progress_rx: Option<Receiver<lrpt_decode::DecodeProgress>>,
    done_rx: Option<Receiver<Result<FileDecodeOutcome, String>>>,
    handle: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    bytes_read: Arc<AtomicU64>,
    total_bytes: u64,
    decoded_recording: String,
    export_notice: Option<String>,
    file_operation: Option<FileOperation>,
    preview_textures: Vec<(u16, egui::TextureHandle, egui::Vec2)>,
    preview_dirty: bool,
    daemon_telemetry: BTreeMap<u16, ez_proto::TelemetryFrame>,
    daemon_textures: BTreeMap<u16, egui::TextureHandle>,
}

impl Drop for DecodingPanel {
    fn drop(&mut self) {
        // The worker checks between short IQ blocks. Never wait for a whole
        // recording on the UI thread during app shutdown.
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl DecodingPanel {
    /// Applies one progressive LRPT image from `ez-daemon`. Frames are keyed by APID so the
    /// latest image for each Meteor channel stays visible while the decoder continues to add
    /// scanlines. Validate dimensions before retaining server-provided pixels.
    pub fn ingest_daemon_telemetry(
        &mut self,
        frame: ez_proto::TelemetryFrame,
    ) -> Result<(), String> {
        let expected = usize::try_from(frame.width)
            .ok()
            .and_then(|width| {
                usize::try_from(frame.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| "LRPT image dimensions overflow this platform".to_string())?;
        if expected != frame.pixels.len() {
            return Err(format!(
                "LRPT APID {} image is {}x{} but contains {} pixels",
                frame.apid,
                frame.width,
                frame.height,
                frame.pixels.len()
            ));
        }
        self.daemon_textures.remove(&frame.apid);
        self.daemon_telemetry.insert(frame.apid, frame);
        Ok(())
    }

    /// Called from the app's per-frame pending-request poll. Populates
    /// state from a completed satellite recording and auto-starts the
    /// decode when a preset is known; otherwise surfaces a "no decoder"
    /// state.
    pub fn request_from_satellite(&mut self, req: DecodeRequest) {
        if self.input_locked() {
            self.pending_status = Some(
                "The Meteor decoder is busy. Finish the current operation before opening another recording."
                    .into(),
            );
            return;
        }
        self.clear_decode_state();
        self.file_path = req.file_path;
        self.satellite_name = req.satellite_name;
        self.sample_rate = req.sample_rate;
        self.symbol_rate = req
            .preset
            .map(|preset| preset.symbol_rate())
            .unwrap_or_default();
        self.no_decoder_reason = None;
        match req.preset {
            Some(preset) => {
                self.preset = preset;
                self.start_decode();
            }
            None => {
                self.no_decoder_reason = Some(format!(
                    "No decoder available for {} — LRPT decoding only supports Meteor-M2 satellites. Recording saved to: {}",
                    self.satellite_name.as_deref().unwrap_or("this satellite"),
                    self.file_path
                ));
            }
        }
    }

    pub fn start_decode(&mut self) {
        if self.file_path.is_empty() || self.input_locked() {
            return;
        }
        self.clear_decode_state();
        let Some(format) = RecordingFormat::from_path(&self.file_path) else {
            self.error = Some("Choose a .cs8 or .cf32 Meteor IQ recording.".to_string());
            return;
        };
        let (progress_tx, progress_rx) = crossbeam_channel::bounded(2);
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let path = self.file_path.clone();
        let sample_rate = if self.sample_rate == 0 {
            self.preset.sample_rate()
        } else {
            self.sample_rate
        };
        let symbol_rate = if self.symbol_rate == 0 {
            self.preset.symbol_rate()
        } else {
            self.symbol_rate
        };
        if let Err(error) = validate_rates(sample_rate, symbol_rate) {
            self.error = Some(error);
            return;
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        self.bytes_read = Arc::new(AtomicU64::new(0));
        self.total_bytes = std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        self.decoded_recording = path.clone();
        let cancel = Arc::clone(&self.cancel);
        let bytes_read = Arc::clone(&self.bytes_read);
        self.handle = Some(std::thread::spawn(move || {
            let result = decode_recording(
                Path::new(&path),
                format,
                sample_rate,
                symbol_rate,
                progress_tx,
                &cancel,
                &bytes_read,
            );
            let _ = done_tx.send(result);
        }));
        self.progress_rx = Some(progress_rx);
        self.done_rx = Some(done_rx);
        self.running = true;
        self.error = None;
        self.result = None;
        self.progress = None;
        self.preview_textures.clear();
        self.preview_dirty = true;
        self.export_notice = None;
        self.pending_status = None;
    }

    pub fn cancel_decode(&mut self) {
        if self.running {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn input_locked(&self) -> bool {
        self.running || self.file_operation.is_some()
    }

    fn start_file_operation(
        &mut self,
        kind: FileOperationKind,
        ctx: &egui::Context,
        operation: impl FnOnce() -> Result<Option<FileOperationOutcome>, String> + Send + 'static,
    ) {
        if self.input_locked() {
            return;
        }
        let (sender, receiver) = crossbeam_channel::bounded(1);
        self.file_operation = Some(FileOperation { kind, receiver });
        self.export_notice = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(operation());
            ctx.request_repaint();
        });
    }

    fn open_recording_with(
        &mut self,
        ctx: &egui::Context,
        choose: impl FnOnce() -> Option<std::path::PathBuf> + Send + 'static,
    ) {
        self.start_file_operation(FileOperationKind::Import, ctx, move || {
            Ok(choose().map(FileOperationOutcome::Import))
        });
    }

    fn export_recording_with(
        &mut self,
        ctx: &egui::Context,
        choose: impl FnOnce() -> Option<std::path::PathBuf> + Send + 'static,
    ) {
        let Some(result) = self
            .result
            .as_ref()
            .filter(|result| !result.images.is_empty())
        else {
            return;
        };
        // Share immutable pixels; a whole-pass clone on the UI thread can
        // itself be expensive even before the PNG encoder starts.
        let result = Arc::clone(result);
        let recording = self.decoded_recording.clone();
        self.start_file_operation(FileOperationKind::Export, ctx, move || {
            let Some(folder) = choose() else {
                return Ok(None);
            };
            let count = export_pngs(&folder, &recording, &result)?;
            Ok(Some(FileOperationOutcome::Export { folder, count }))
        });
    }

    fn poll_file_operation(&mut self) {
        let completion = self.file_operation.as_ref().and_then(|operation| {
            match operation.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(crossbeam_channel::TryRecvError::Empty) => None,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(Err("File operation worker stopped unexpectedly.".into()))
                }
            }
        });
        if let Some(completion) = completion {
            self.file_operation = None;
            match completion {
                Ok(Some(FileOperationOutcome::Import(path))) => {
                    self.clear_decode_state();
                    self.file_path = path.to_string_lossy().into_owned();
                }
                Ok(Some(FileOperationOutcome::Export { folder, count })) => {
                    let notice = format!("Saved {count} PNG(s) to {}", folder.display());
                    self.export_notice = Some(notice.clone());
                    self.pending_status = Some(notice);
                    self.error = None;
                }
                Ok(None) => {}
                Err(error) => {
                    self.pending_status = Some(error.clone());
                    self.error = Some(error);
                }
            }
        }
    }

    fn clear_decode_state(&mut self) {
        self.result = None;
        self.progress = None;
        self.error = None;
        self.preview_textures.clear();
        self.preview_dirty = true;
        self.export_notice = None;
        self.pending_status = None;
        self.total_bytes = 0;
        self.bytes_read.store(0, Ordering::Relaxed);
        self.decoded_recording.clear();
        self.cancel.store(false, Ordering::Relaxed);
    }

    /// Poll progress/completion channels. MUST be called every frame from
    /// `CentralApp::logic()` unconditionally (not only while the Decoding
    /// tab is visible) — otherwise a decode running in the background while
    /// the user is on another tab will never be observed as complete.
    pub fn tick_decode(&mut self) {
        self.poll_file_operation();
        if let Some(rx) = &self.progress_rx {
            while let Ok(p) = rx.try_recv() {
                self.progress = Some(p);
                self.preview_dirty = true;
            }
        }
        if let Some(rx) = &self.done_rx {
            let completion = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(Err("Decoder worker stopped unexpectedly.".into()))
                }
                Err(crossbeam_channel::TryRecvError::Empty) => None,
            };
            if let Some(result) = completion {
                if let Some(h) = self.handle.take() {
                    let _ = h.join();
                }
                match result {
                    Ok(FileDecodeOutcome::Complete(decoded)) => {
                        self.pending_status = Some(format!(
                            "✅ Decode complete: {} image(s), {} RS-OK / {} RS-failed",
                            decoded.images.len(),
                            decoded.rs_ok,
                            decoded.rs_failed
                        ));
                        self.result = Some(Arc::new(decoded));
                        self.progress = None;
                        self.preview_textures.clear();
                        self.preview_dirty = true;
                    }
                    Ok(FileDecodeOutcome::Cancelled) => {
                        self.pending_status = Some("Decode cancelled.".into());
                        self.progress = None;
                        self.preview_textures.clear();
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        self.pending_status = Some(format!("❌ Decode failed: {e}"));
                    }
                }
                self.running = false;
                self.progress_rx = None;
                self.done_rx = None;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.ui_with_daemon(ui, true);
    }

    /// Render the file-import-only workspace, hiding telemetry arriving from
    /// live daemon capture. The dedicated Meteor tab is strictly offline.
    pub fn ui_offline(&mut self, ui: &mut egui::Ui) {
        self.ui_with_daemon(ui, false);
    }

    fn blocking_notice(&self, show_daemon: bool) -> Option<&str> {
        show_daemon
            .then(|| self.no_decoder_reason.as_deref())
            .flatten()
    }

    fn ui_with_daemon(&mut self, ui: &mut egui::Ui, show_daemon: bool) {
        if let Some(reason) = self.blocking_notice(show_daemon) {
            ui.colored_label(egui::Color32::YELLOW, reason);
            return;
        }
        ui.horizontal_top(|ui| {
            let controls_height = ui.available_height();
            ui.allocate_ui_with_layout(
                egui::vec2(320.0, controls_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(294.0);
                        self.ui_input_controls(ui);
                    });
                },
            );
            ui.separator();
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.ui_results(ui, show_daemon));
            });
        });
    }

    fn ui_input_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Input recording");
        ui.label(
            egui::RichText::new("Decode an existing Meteor IQ capture")
                .small()
                .color(egui::Color32::GRAY),
        );
        ui.add_space(8.0);
        let mut input_changed = false;
        ui.add_enabled_ui(!self.input_locked(), |ui| {
            ui.label("File");
            input_changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.file_path)
                        .desired_width(f32::INFINITY)
                        .hint_text(".cs8 or .cf32 path"),
                )
                .changed();
            if ui.button("Open recording…").clicked() {
                self.open_recording_with(ui.ctx(), || {
                    rfd::FileDialog::new()
                        .add_filter("Meteor IQ recordings", &["cs8", "cf32"])
                        .add_filter("All files", &["*"])
                        .pick_file()
                });
            }
            if !self.file_path.is_empty() {
                let kind = RecordingFormat::from_path(&self.file_path)
                    .map(RecordingFormat::label)
                    .unwrap_or("Unsupported format · choose .cs8 or .cf32");
                ui.label(
                    egui::RichText::new(kind)
                        .small()
                        .color(egui::Color32::from_rgb(145, 165, 181)),
                );
            }
            ui.add_space(8.0);
            egui::ComboBox::from_label("Preset")
                .selected_text(self.preset.label())
                .show_ui(ui, |ui| {
                    for preset in DecodePreset::all() {
                        if ui
                            .selectable_value(&mut self.preset, *preset, preset.label())
                            .changed()
                        {
                            self.sample_rate = preset.sample_rate();
                            self.symbol_rate = preset.symbol_rate();
                            input_changed = true;
                        }
                    }
                });
            if self.sample_rate == 0 {
                self.sample_rate = self.preset.sample_rate();
            }
            if self.symbol_rate == 0 {
                self.symbol_rate = self.preset.symbol_rate();
            }
            ui.horizontal(|ui| {
                ui.label("Sample");
                input_changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.sample_rate)
                            .speed(1_000.0)
                            .range(100_000..=10_000_000)
                            .suffix(" sps"),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Symbol");
                input_changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.symbol_rate)
                            .speed(100.0)
                            .range(10_000..=500_000)
                            .suffix(" sps"),
                    )
                    .changed();
            });
            ui.label(
                egui::RichText::new("Use the sample rate of your recording.")
                    .small()
                    .color(egui::Color32::GRAY),
            );
        });
        if input_changed {
            self.clear_decode_state();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.input_locked() && !self.file_path.is_empty(),
                    egui::Button::new("▶ Decode"),
                )
                .clicked()
            {
                self.start_decode();
            }
            if ui
                .add_enabled(!self.input_locked(), egui::Button::new("Clear"))
                .clicked()
            {
                self.file_path.clear();
                self.clear_decode_state();
            }
            if self.running
                && ui
                    .add_enabled(
                        !self.cancel.load(Ordering::Relaxed),
                        egui::Button::new("Cancel"),
                    )
                    .clicked()
            {
                self.cancel_decode();
            }
        });
        if let Some(operation) = &self.file_operation {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(match operation.kind {
                    FileOperationKind::Import => "Choose a recording in the file dialog…",
                    FileOperationKind::Export => "Choosing a folder / saving PNGs…",
                });
            });
        }
        if self.running {
            // Also repaint during noise-only input, when there may be no
            // frame-lock snapshots to wake the UI.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if self.cancel.load(Ordering::Relaxed) {
                    "Cancelling…"
                } else {
                    "Decoding in background…"
                });
            });
            let read = self
                .bytes_read
                .load(Ordering::Relaxed)
                .min(self.total_bytes);
            let fraction = if self.total_bytes == 0 {
                0.0
            } else {
                read as f32 / self.total_bytes as f32
            };
            ui.add(egui::ProgressBar::new(fraction).text(format!(
                "{:.1} / {:.1} MiB · {:.0}%",
                read as f64 / 1_048_576.0,
                self.total_bytes as f64 / 1_048_576.0,
                fraction * 100.0
            )));
            if let Some(progress) = &self.progress {
                ui.label(format!(
                    "{} lines · RS {} ok / {} failed",
                    progress.lines_decoded, progress.rs_ok, progress.rs_failed
                ));
                ui.label(format!(
                    "Carrier {} · frame {}",
                    if progress.costas_locked {
                        "locked"
                    } else {
                        "searching"
                    },
                    if progress.frame_locked {
                        "locked"
                    } else {
                        "searching"
                    }
                ));
            }
        } else if self.result.is_some() {
            ui.label("Decode complete.");
        } else if self.cancel.load(Ordering::Relaxed) {
            ui.label("Decode cancelled.");
        }
        if let Some(error) = &self.error {
            ui.add_space(6.0);
            ui.colored_label(egui::Color32::from_rgb(225, 105, 105), error);
        }
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Offline decoder · no SDR recording controls")
                .small()
                .color(egui::Color32::GRAY),
        );
    }

    fn ui_results(&mut self, ui: &mut egui::Ui, show_daemon: bool) {
        ui.heading("Decoded imagery");
        if show_daemon && !self.daemon_telemetry.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("Live daemon LRPT").strong());
            let frames: Vec<_> = self
                .daemon_telemetry
                .iter()
                .map(|(&apid, frame)| (apid, frame.clone()))
                .collect();
            for (apid, frame) in frames {
                ui.label(format!(
                    "APID {apid} · {}×{} · Costas {} · Frame {} · RS {} ok / {} failed",
                    frame.width,
                    frame.height,
                    if frame.costas_locked {
                        "locked"
                    } else {
                        "searching"
                    },
                    if frame.frame_locked {
                        "locked"
                    } else {
                        "searching"
                    },
                    frame.rs_ok,
                    frame.rs_failed
                ));
                if !self.daemon_textures.contains_key(&apid) {
                    let rgba: Vec<u8> = frame
                        .pixels
                        .iter()
                        .flat_map(|&value| [value, value, value, 255])
                        .collect();
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &rgba,
                    );
                    self.daemon_textures.insert(
                        apid,
                        ui.ctx().load_texture(
                            format!("daemon-lrpt-apid-{apid}"),
                            image,
                            egui::TextureOptions::LINEAR,
                        ),
                    );
                }
                if let Some(texture) = self.daemon_textures.get(&apid) {
                    let natural = texture.size_vec2();
                    ui.image((
                        texture.id(),
                        natural * (ui.available_width().max(1.0) / natural.x).min(1.0),
                    ));
                }
            }
        }
        let mut export_requested = false;
        if let Some(result) = &self.result {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} image(s) · RS {} ok / {} failed",
                    result.images.len(),
                    result.rs_ok,
                    result.rs_failed
                ));
                export_requested = ui
                    .add_enabled(
                        !self.input_locked() && !result.images.is_empty(),
                        egui::Button::new("Save PNGs…"),
                    )
                    .clicked();
            });
            if result.images.is_empty() {
                ui.label("Frames decoded, but no Meteor image channels were recovered.");
            }
        }
        if export_requested {
            self.export_recording_with(ui.ctx(), || rfd::FileDialog::new().pick_folder());
        }
        if let Some(notice) = &self.export_notice {
            ui.label(notice);
        }
        let images = self
            .result
            .as_ref()
            .map(|result| &result.images)
            .or_else(|| self.progress.as_ref().map(|progress| &progress.preview));
        if let Some(images) = images.filter(|images| !images.is_empty()) {
            if self.result.is_none() {
                ui.label(egui::RichText::new("Channel preview · partial decode").small());
            }
            if self.preview_dirty || self.preview_textures.len() != images.len() {
                self.preview_textures.clear();
                for (apid, image) in images {
                    // A long pass can exceed the GPU's maximum texture side.
                    // Sample a bounded display copy; retain original pixels for
                    // PNG export and original aspect ratio for the scroll view.
                    let max_side = ui
                        .ctx()
                        .input(|input| input.max_texture_side)
                        .clamp(1, 2_048);
                    let width = image.width() as usize;
                    let height = image.height() as usize;
                    let step = width.max(height).div_ceil(max_side).max(1);
                    let display = image::GrayImage::from_fn(
                        width.div_ceil(step) as u32,
                        height.div_ceil(step) as u32,
                        |x, y| *image.get_pixel(x * step as u32, y * step as u32),
                    );
                    let size = [display.width() as usize, display.height() as usize];
                    let color = egui::ColorImage::from_gray(size, display.as_raw());
                    self.preview_textures.push((
                        *apid,
                        ui.ctx().load_texture(
                            format!("decode-apid-{apid}"),
                            color,
                            egui::TextureOptions::LINEAR,
                        ),
                        egui::vec2(width as f32, height as f32),
                    ));
                }
                self.preview_dirty = false;
            }
            for (apid, texture, natural) in &self.preview_textures {
                ui.label(format!(
                    "{} · APID {apid}",
                    lrpt_decode::apid_channel_label(*apid).unwrap_or("Image channel")
                ));
                ui.image((
                    texture.id(),
                    *natural * (ui.available_width().max(1.0) / natural.x).min(1.0),
                ));
            }
        } else if self.result.is_none() && (!show_daemon || self.daemon_telemetry.is_empty()) {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(if self.running {
                    "Searching for Meteor frames. Channel images will appear as they decode."
                } else {
                    "Decoded channel images will appear here."
                })
                .color(egui::Color32::GRAY),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ez-meteor-panel-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn recording(&self, name: &str, data: &[u8]) -> std::path::PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, data).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A stored RS-coded MSU-MR packet containing fourteen grey JPEG MCUs.
    /// CCSDS randomization and K=7 (171,133) coding turn it into a noiseless,
    /// signed OQPSK IQ file. This exercises image recovery through the file
    /// worker; it is deliberately not an off-air acceptance fixture.
    fn synthetic_signed_oqpsk() -> Vec<u8> {
        let mut cadu = 0x1ACF_FC1Du32.to_be_bytes().to_vec();
        let mut randomizer = 0xffu8;
        for transport_byte in include_bytes!("../tests/fixtures/meteor-gray-segment.rs255.bin") {
            let mut byte = 0;
            for _ in 0..8 {
                byte = (byte << 1) | (randomizer & 1);
                let feedback =
                    ((randomizer >> 7) ^ (randomizer >> 5) ^ (randomizer >> 3) ^ randomizer) & 1;
                randomizer = (randomizer >> 1) | (feedback << 7);
            }
            cadu.push(byte ^ transport_byte);
        }
        let mut information = vec![0u8; 256];
        information.extend(
            cadu.iter()
                .flat_map(|byte| (0..8).rev().map(move |shift| (byte >> shift) & 1)),
        );
        information.extend(std::iter::repeat_n(0, 256));
        let mut level = 0;
        let mut state = 0;
        let symbols: Vec<[i8; 2]> = information
            .into_iter()
            .map(|bit| {
                level ^= bit;
                let register: u8 = (level << 6) | state;
                state = register >> 1;
                [0o171, 0o133].map(|polynomial| {
                    let coded = ((register & polynomial).count_ones() & 1) ^ 1;
                    if coded == 0 {
                        40
                    } else {
                        -40
                    }
                })
            })
            .collect();
        let mut iq = Vec::new();
        for sample in 0..symbols.len() * 8 + 4 {
            let i = (sample / 8).min(symbols.len() - 1);
            let q = (sample.saturating_sub(4) / 8).min(symbols.len() - 1);
            iq.extend([symbols[i][0] as u8, symbols[q][1] as u8]);
        }
        iq
    }

    #[test]
    fn signed_cs8_worker_recovers_image_exports_pixels_and_reports_all_bytes() {
        let directory = TestDirectory::new();
        let iq = synthetic_signed_oqpsk();
        let path = directory.recording("meteor.CS8", &iq);
        let format = RecordingFormat::from_path(path.to_str().unwrap()).unwrap();
        let (tx, _rx) = crossbeam_channel::bounded(2);
        let bytes_read = AtomicU64::new(0);
        let outcome = decode_recording(
            &path,
            format,
            8_000,
            1_000,
            tx,
            &AtomicBool::new(false),
            &bytes_read,
        )
        .unwrap();
        let FileDecodeOutcome::Complete(result) = outcome else {
            panic!("unexpected cancellation")
        };
        assert_eq!(
            result.rs_ok, 4,
            "all four interleaved RS codewords should decode"
        );
        assert_eq!(result.rs_failed, 0);
        assert_eq!(bytes_read.load(Ordering::Relaxed), iq.len() as u64);
        assert_eq!(result.images.len(), 1);
        let (apid, decoded) = &result.images[0];
        assert_eq!(*apid, 65);
        assert_eq!(decoded.dimensions(), (1568, 8));
        for (x, _, pixel) in decoded.enumerate_pixels() {
            assert_eq!(pixel.0[0], if x < 112 { 128 } else { 0 });
        }
        assert_eq!(
            export_pngs(&directory.0, path.to_str().unwrap(), &result).unwrap(),
            1
        );
        let exported = image::open(directory.0.join("meteor_apid_65.png"))
            .unwrap()
            .to_luma8();
        assert_eq!(&exported, decoded);
    }

    #[test]
    fn recording_format_and_invalid_rates_are_rejected() {
        assert_eq!(
            RecordingFormat::from_path("capture.CS8"),
            Some(RecordingFormat::Cs8)
        );
        assert_eq!(
            RecordingFormat::from_path("capture.cf32"),
            Some(RecordingFormat::Cf32)
        );
        assert_eq!(RecordingFormat::from_path("capture.cu8"), None);
        let mut panel = DecodingPanel::default();
        panel.file_path = "capture.cu8".into();
        panel.result = Some(Arc::new(lrpt_decode::DecodeResult::default()));
        panel.start_decode();
        assert!(!panel.running);
        assert!(
            panel.result.is_none(),
            "invalid input must not retain a previous pass"
        );
        assert!(panel.error.as_deref().unwrap().contains(".cs8 or .cf32"));
        for (sample, symbol) in [(0, 1), (1, 0), (1_000, 800), (u32::MAX, u32::MAX)] {
            assert!(validate_rates(sample, symbol).is_err());
        }
        assert!(validate_rates(160_000, 80_000).is_ok());
        panel.file_path = "capture.cs8".into();
        panel.sample_rate = 100_000;
        panel.symbol_rate = 80_000;
        panel.start_decode();
        assert!(!panel.running);
        assert!(panel.error.as_deref().unwrap().contains("twice"));
    }

    #[test]
    fn worker_rejects_empty_and_partial_iq_samples() {
        let directory = TestDirectory::new();
        for (name, data, format) in [
            ("empty.cs8", vec![], RecordingFormat::Cs8),
            ("partial.cs8", vec![0; 3], RecordingFormat::Cs8),
            ("partial.cf32", vec![0; 7], RecordingFormat::Cf32),
        ] {
            let path = directory.recording(name, &data);
            let (tx, _rx) = crossbeam_channel::bounded(2);
            let error = decode_recording(
                &path,
                format,
                8_000,
                1_000,
                tx,
                &AtomicBool::new(false),
                &AtomicU64::new(0),
            )
            .unwrap_err();
            assert!(error.contains("empty or has an incomplete"));
        }
    }

    #[test]
    fn worker_honors_cancellation_before_consuming_file() {
        let directory = TestDirectory::new();
        let path = directory.recording("cancel.cs8", &[0; 4096]);
        let (tx, _rx) = crossbeam_channel::bounded(2);
        let read = AtomicU64::new(0);
        let outcome = decode_recording(
            &path,
            RecordingFormat::Cs8,
            8_000,
            1_000,
            tx,
            &AtomicBool::new(true),
            &read,
        )
        .unwrap();
        assert!(matches!(outcome, FileDecodeOutcome::Cancelled));
        assert_eq!(read.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cancel_and_drop_signal_the_background_worker() {
        let mut panel = DecodingPanel::default();
        panel.running = true;
        let cancelled = Arc::clone(&panel.cancel);
        panel.cancel_decode();
        assert!(cancelled.load(Ordering::Relaxed));
        cancelled.store(false, Ordering::Relaxed);
        drop(panel);
        assert!(cancelled.load(Ordering::Relaxed));
    }

    #[test]
    fn background_completion_does_not_require_visible_tab() {
        let directory = TestDirectory::new();
        let path = directory.recording("quiet.cs8", &[0; 4096]);
        let mut panel = DecodingPanel::default();
        panel.file_path = path.to_string_lossy().into_owned();
        panel.start_decode();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while panel.running && std::time::Instant::now() < deadline {
            panel.tick_decode();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!panel.running, "worker did not finish before deadline");
        assert!(panel.handle.is_none());
        assert!(panel
            .error
            .as_deref()
            .unwrap()
            .contains("no CADU frame sync"));
        assert_eq!(panel.bytes_read.load(Ordering::Relaxed), 4096);
    }

    #[test]
    fn unexpected_worker_disconnect_is_reported() {
        let mut panel = DecodingPanel::default();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        panel.done_rx = Some(receiver);
        panel.running = true;
        drop(sender);
        panel.tick_decode();
        assert!(!panel.running);
        assert_eq!(
            panel.error.as_deref(),
            Some("Decoder worker stopped unexpectedly.")
        );
    }

    #[test]
    fn png_exports_preserve_previous_passes_and_pixels() {
        let directory = TestDirectory::new();
        let image = image::GrayImage::from_raw(2, 2, vec![0, 85, 170, 255]).unwrap();
        let result = lrpt_decode::DecodeResult {
            images: vec![(65, image.clone())],
            ..Default::default()
        };
        let prior = directory.0.join("pass_apid_65.png");
        std::fs::write(&prior, b"prior export").unwrap();
        for _ in 0..2 {
            assert_eq!(
                export_pngs(&directory.0, "/recordings/pass.cs8", &result).unwrap(),
                1
            );
        }
        assert_eq!(std::fs::read(prior).unwrap(), b"prior export");
        for suffix in [1, 2] {
            let exported = image::open(directory.0.join(format!("pass_apid_65_{suffix}.png")))
                .unwrap()
                .to_luma8();
            assert_eq!(exported, image);
        }
    }

    #[test]
    fn long_channel_preview_fits_gpu_limit_and_png_keeps_full_pixels() {
        let directory = TestDirectory::new();
        let decoded =
            image::GrayImage::from_fn(1_568, 4_096, |x, y| image::Luma([((x + y) % 256) as u8]));
        let result = Arc::new(lrpt_decode::DecodeResult {
            images: vec![(65, decoded.clone())],
            ..Default::default()
        });
        let mut panel = DecodingPanel::default();
        panel.result = Some(Arc::clone(&result));
        let context = egui::Context::default();
        let _ = context.run_ui(
            egui::RawInput {
                max_texture_side: Some(1_024),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_000.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| panel.ui_offline(ui),
        );
        assert!(panel.preview_textures[0]
            .1
            .size()
            .iter()
            .all(|side| *side <= 1_024));
        export_pngs(&directory.0, "long.cs8", &result).unwrap();
        let exported = image::open(directory.0.join("long_apid_65.png"))
            .unwrap()
            .to_luma8();
        assert_eq!(exported, decoded);
    }

    #[test]
    fn nonfinite_cf32_reports_the_bad_sample_without_consuming_the_recording() {
        let directory = TestDirectory::new();
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&f32::NAN.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.resize(200_000, 0);
        let path = directory.recording("bad.cf32", &data);
        let (progress_tx, _progress_rx) = crossbeam_channel::bounded(1);
        let bytes_read = AtomicU64::new(0);
        let error = decode_recording(
            &path,
            RecordingFormat::Cf32,
            8_000,
            1_000,
            progress_tx,
            &AtomicBool::new(false),
            &bytes_read,
        )
        .unwrap_err();
        assert!(
            error.contains("non-finite") && error.contains("sample 1"),
            "{error}"
        );
        assert_eq!(bytes_read.load(Ordering::Relaxed), 0);
    }

    fn wait_for_file_operation(panel: &mut DecodingPanel) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while panel.file_operation.is_some() && std::time::Instant::now() < deadline {
            panel.tick_decode();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(panel.file_operation.is_none(), "file worker did not finish");
        assert!(!panel.input_locked());
    }

    #[test]
    fn recording_dialog_runs_off_ui_and_applies_selection_only_after_completion() {
        let mut panel = DecodingPanel::default();
        panel.file_path = "previous.cs8".into();
        let context = egui::Context::default();
        let ui_thread = std::thread::current().id();
        let (release, wait) = crossbeam_channel::bounded(1);
        panel.open_recording_with(&context, move || {
            assert_ne!(std::thread::current().id(), ui_thread);
            wait.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Some("selected.cs8".into())
        });
        // Returning here while the chooser waits proves it cannot block UI
        // rendering, audio delivery, or the aircraft receiver's frame poll.
        assert!(panel.input_locked());
        panel.start_decode();
        assert!(!panel.running);
        assert_eq!(panel.file_path, "previous.cs8");
        let output = context.run_ui(egui::RawInput::default(), |ui| panel.ui_offline(ui));
        assert!(rendered_text(&output.shapes).contains("Choose a recording"));
        release.send(()).unwrap();
        wait_for_file_operation(&mut panel);
        assert_eq!(panel.file_path, "selected.cs8");
        assert!(panel.error.is_none());
    }

    #[test]
    fn folder_dialog_and_png_export_share_pixels_and_finish_in_background() {
        let directory = TestDirectory::new();
        let folder = directory.0.clone();
        std::fs::write(folder.join("pass_apid_65.png"), b"existing export").unwrap();
        let decoded = Arc::new(lrpt_decode::DecodeResult {
            images: vec![(
                65,
                image::GrayImage::from_raw(2, 2, vec![0, 85, 170, 255]).unwrap(),
            )],
            ..Default::default()
        });
        let mut panel = DecodingPanel::default();
        panel.decoded_recording = "pass.cs8".into();
        panel.result = Some(Arc::clone(&decoded));
        let context = egui::Context::default();
        let ui_thread = std::thread::current().id();
        let (release, wait) = crossbeam_channel::bounded(1);
        panel.export_recording_with(&context, move || {
            assert_ne!(std::thread::current().id(), ui_thread);
            wait.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Some(folder)
        });
        assert!(panel.input_locked());
        assert!(Arc::ptr_eq(panel.result.as_ref().unwrap(), &decoded));
        assert_eq!(
            Arc::strong_count(&decoded),
            3,
            "worker should share pixels, not clone them"
        );
        assert!(!directory.0.join("pass_apid_65_1.png").exists());
        release.send(()).unwrap();
        wait_for_file_operation(&mut panel);
        assert!(panel
            .export_notice
            .as_deref()
            .unwrap()
            .starts_with("Saved 1 PNG(s)"));
        assert_eq!(
            std::fs::read(directory.0.join("pass_apid_65.png")).unwrap(),
            b"existing export"
        );
        assert_eq!(
            image::open(directory.0.join("pass_apid_65_1.png"))
                .unwrap()
                .to_luma8(),
            decoded.images[0].1
        );
        assert!(Arc::ptr_eq(panel.result.as_ref().unwrap(), &decoded));
    }

    #[test]
    fn cancelled_dialog_and_failed_export_unlock_without_losing_decoded_images() {
        let directory = TestDirectory::new();
        let mut panel = DecodingPanel::default();
        panel.file_path = "previous.cs8".into();
        panel.result = Some(Arc::new(lrpt_decode::DecodeResult {
            images: vec![(65, image::GrayImage::new(2, 2))],
            ..Default::default()
        }));
        let context = egui::Context::default();
        panel.open_recording_with(&context, || None);
        wait_for_file_operation(&mut panel);
        assert_eq!(panel.file_path, "previous.cs8");
        assert!(panel.result.is_some());
        let missing_folder = directory.0.join("missing-directory");
        panel.export_recording_with(&context, move || Some(missing_folder));
        wait_for_file_operation(&mut panel);
        assert!(panel.error.as_deref().unwrap().contains("Could not save"));
        assert!(panel.result.is_some());
    }

    fn rendered_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        fn append(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(shape) => {
                    text.push_str(&shape.galley.job.text);
                    text.push('\n');
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        append(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for shape in shapes {
            append(&shape.shape, &mut text);
        }
        text
    }

    #[test]
    fn results_header_status_export_and_channels_stack_vertically() {
        fn text_rect(shapes: &[egui::epaint::ClippedShape], prefix: &str) -> egui::Rect {
            fn find(shape: &egui::Shape, prefix: &str) -> Option<egui::Rect> {
                match shape {
                    egui::Shape::Text(text) if text.galley.job.text.starts_with(prefix) => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, prefix)),
                    _ => None,
                }
            }
            shapes
                .iter()
                .find_map(|shape| find(&shape.shape, prefix))
                .unwrap_or_else(|| panic!("missing text {prefix}"))
        }
        let mut panel = DecodingPanel::default();
        panel.result = Some(Arc::new(lrpt_decode::DecodeResult {
            images: vec![
                (65, image::GrayImage::new(200, 80)),
                (66, image::GrayImage::new(200, 80)),
            ],
            ..Default::default()
        }));
        let context = egui::Context::default();
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| panel.ui_offline(ui),
        );
        let heading = text_rect(&output.shapes, "Decoded imagery");
        let status = text_rect(&output.shapes, "2 image(s)");
        let save = text_rect(&output.shapes, "Save PNGs");
        let first = text_rect(&output.shapes, "MSU-MR Channel 2");
        let second = text_rect(&output.shapes, "MSU-MR Channel 3");
        assert!(status.top() >= heading.bottom());
        assert!(first.top() >= status.bottom());
        assert!(second.top() >= first.bottom() + 80.0);
        assert!((heading.left() - first.left()).abs() < 1.0);
        assert!((first.left() - second.left()).abs() < 1.0);
        assert!(save.right() <= 1000.0 && second.right() <= 1000.0);
    }

    #[test]
    fn offline_ui_renders_import_and_progressive_images_without_live_telemetry() {
        let mut panel = DecodingPanel::default();
        panel.no_decoder_reason = Some("No decoder for this selected satellite".into());
        panel
            .ingest_daemon_telemetry(ez_proto::TelemetryFrame {
                channel_id: 4,
                apid: 65,
                width: 2,
                height: 2,
                pixels: vec![1, 2, 3, 4],
                rs_ok: 3,
                rs_failed: 1,
                costas_locked: true,
                frame_locked: true,
            })
            .unwrap();
        let context = egui::Context::default();
        let render = |panel: &mut DecodingPanel| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| panel.ui_offline(ui),
            )
        };
        let output = render(&mut panel);
        let text = rendered_text(&output.shapes);
        assert!(text.contains("Open recording"));
        assert!(!text.contains("No decoder for"));
        assert!(!text.contains("Live daemon LRPT"));
        assert!(panel.daemon_textures.is_empty());

        let (tx, rx) = crossbeam_channel::bounded(2);
        panel.progress_rx = Some(rx);
        panel.running = true;
        panel.total_bytes = 1024;
        panel.bytes_read.store(512, Ordering::Relaxed);
        for height in [2, 3] {
            tx.send(lrpt_decode::DecodeProgress {
                preview: vec![(65, image::GrayImage::new(2, height))],
                ..Default::default()
            })
            .unwrap();
            panel.tick_decode();
            let output = render(&mut panel);
            let text = rendered_text(&output.shapes);
            assert!(text.contains("Channel preview"));
            assert!(text.contains("Cancel"));
            assert!(text.contains("50%"));
            assert_eq!(panel.preview_textures.len(), 1);
            assert_eq!(panel.preview_textures[0].1.size(), [2, height as usize]);
            assert!(panel.daemon_textures.is_empty());
        }
    }

    #[test]
    fn satellite_to_preset_matches_meteor_variants() {
        assert_eq!(
            satellite_to_preset("Meteor-M2-3"),
            Some(DecodePreset::MeteorM2_3)
        );
        assert_eq!(
            satellite_to_preset("Meteor-M2-4"),
            Some(DecodePreset::MeteorM2_4)
        );
    }

    #[test]
    fn satellite_to_preset_returns_none_for_noaa() {
        assert_eq!(satellite_to_preset("NOAA 19"), None);
    }

    #[test]
    fn satellite_to_preset_returns_none_for_iss() {
        assert_eq!(satellite_to_preset("ISS (ZARYA)"), None);
    }

    #[test]
    fn satellite_to_preset_is_case_insensitive() {
        assert_eq!(
            satellite_to_preset("meteor-m2-3"),
            Some(DecodePreset::MeteorM2_3)
        );
    }

    #[test]
    fn request_from_satellite_sets_no_decoder_reason_when_preset_none() {
        let mut panel = DecodingPanel::default();
        panel.request_from_satellite(DecodeRequest {
            file_path: "/tmp/rec.cf32".to_string(),
            preset: None,
            sample_rate: 2_048_000,
            satellite_name: Some("NOAA 19".to_string()),
        });
        assert!(panel.no_decoder_reason.is_some());
        assert!(panel.blocking_notice(true).is_some());
        assert!(panel.blocking_notice(false).is_none());
        assert!(!panel.running);
        assert_eq!(panel.file_path, "/tmp/rec.cf32");
    }

    #[test]
    fn daemon_telemetry_is_retained_by_apid() {
        let mut panel = DecodingPanel::default();
        panel
            .ingest_daemon_telemetry(ez_proto::TelemetryFrame {
                channel_id: 4,
                apid: 65,
                width: 2,
                height: 2,
                pixels: vec![1, 2, 3, 4],
                rs_ok: 3,
                rs_failed: 1,
                costas_locked: true,
                frame_locked: true,
            })
            .unwrap();

        let frame = panel.daemon_telemetry.get(&65).unwrap();
        assert_eq!(frame.pixels, vec![1, 2, 3, 4]);
        assert_eq!(frame.rs_ok, 3);
    }

    #[test]
    fn daemon_telemetry_rejects_mismatched_dimensions() {
        let mut panel = DecodingPanel::default();
        let err = panel
            .ingest_daemon_telemetry(ez_proto::TelemetryFrame {
                channel_id: 4,
                apid: 65,
                width: 2,
                height: 2,
                pixels: vec![1, 2, 3],
                rs_ok: 0,
                rs_failed: 0,
                costas_locked: false,
                frame_locked: false,
            })
            .unwrap_err();
        assert!(err.contains("contains 3 pixels"));
        assert!(panel.daemon_telemetry.is_empty());
    }
}
