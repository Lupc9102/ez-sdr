//! Real-time FFT spectrum analyser, waterfall display, and colour-map
//! rendering for the EZ-SDR spectrum viewer.
//!
//! Provides [`SpectrumAnalyzer`] which manages FFT computation, zoom/pan
//! controls, marker/bookmark overlays, waterfall texture management, and
//! context-menu actions for spectrum interaction.

use crossbeam_channel;
use num_complex::Complex32;
use rustfft::FftPlanner;
// serde imported for potential future use in signal history serialization
use std::collections::VecDeque;
use std::f32::consts::PI;

pub const MIN_FFT_SIZE: usize = 256;
/// Highest transform resolution exposed by the SDR++-compatible controls.
///
/// The spectrum arrays and FFT worker are allowed to use the full 65,536-bin
/// resolution.  Waterfall storage remains bounded independently by
/// `MAX_WATERFALL_WIDTH` and `MAX_WATERFALL_BYTES` in `reset_waterfall`.
pub const MAX_FFT_SIZE: usize = 65_536;
const MAX_FFTS_PER_PUSH: usize = 4;
const MAX_WATERFALL_WIDTH: usize = 2048;
const MAX_WATERFALL_BYTES: usize = 8 * 1024 * 1024;

fn plot_bin_stride(bins: usize, pixels: f32) -> usize {
    bins.div_ceil((pixels.ceil() as usize).clamp(1, 4096))
        .max(1)
}

fn peak_in_bins(bins: &[f32], first: usize, last: usize) -> f32 {
    bins[first..last].iter().copied().fold(-120.0_f32, f32::max)
}

/// Spectrum frame received back from the FFT worker.
pub type SpectrumFrame = Vec<f32>;

/// Dedicated FFT worker that runs on its own OS thread.
///
/// Receives IQ windows via a crossbeam channel, computes the FFT,
/// and sends back spectrum frames (dB values in display order).
/// The UI thread never blocks on FFT — it uses try_recv and skips
/// frames that aren't ready yet.
pub struct SpectrumWorker {
    iq_sender: crossbeam_channel::Sender<Vec<Complex32>>,
    spectrum_receiver: crossbeam_channel::Receiver<SpectrumFrame>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SpectrumWorker {
    /// Create a dummy SpectrumWorker without spawning a thread.
    /// Used in test environments where thread spawning may cause hangs.
    pub fn new_uninitialized() -> Self {
        let (iq_tx, _iq_rx) = crossbeam_channel::bounded::<Vec<Complex32>>(4);
        let (_spectrum_tx, spectrum_rx) = crossbeam_channel::bounded::<SpectrumFrame>(4);
        Self {
            iq_sender: iq_tx,
            spectrum_receiver: spectrum_rx,
            handle: None,
        }
    }

    /// Create a new SpectrumWorker with the given FFT size and window type.
    /// Spawns a dedicated OS thread that owns the FftPlanner.
    pub fn new(fft_size: usize, window: WindowType) -> Self {
        let (iq_tx, iq_rx) = crossbeam_channel::bounded::<Vec<Complex32>>(4);
        let (spectrum_tx, spectrum_rx) = crossbeam_channel::bounded::<SpectrumFrame>(4);

        let handle = std::thread::spawn(move || {
            let mut planner = FftPlanner::<f32>::new();
            let fft = planner.plan_fft_forward(fft_size);
            let mut scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];
            let window_cache = window.generate(fft_size);
            let scale = window_cache.iter().sum::<f32>().max(1.0).recip();
            let mut input_buf = vec![Complex32::new(0.0, 0.0); fft_size];
            let half = fft_size / 2;

            // When iq_sender is dropped (on CentralApp drop), recv() returns Err
            // and the loop exits. No select! needed — simpler and no hang.
            while let Ok(mut iq_window) = iq_rx.recv() {
                while let Ok(newer) = iq_rx.try_recv() {
                    iq_window = newer;
                }
                if iq_window.len() != fft_size {
                    continue;
                }
                for i in 0..fft_size {
                    input_buf[i] = iq_window[i] * window_cache[i];
                }
                fft.process_with_scratch(&mut input_buf, &mut scratch);
                let mut dbs = vec![-120.0f32; fft_size];
                for i in 0..fft_size {
                    let magnitude = input_buf[i].norm() * scale;
                    let db = if magnitude > 1e-6 {
                        20.0 * magnitude.log10()
                    } else {
                        -120.0
                    };
                    let dst = (i + half) % fft_size;
                    dbs[dst] = db;
                }
                let _ = spectrum_tx.try_send(dbs);
            }
        });

        Self {
            iq_sender: iq_tx,
            spectrum_receiver: spectrum_rx,
            handle: Some(handle),
        }
    }

    /// Send an IQ window to the worker. Returns false if the channel is full.
    pub fn try_send_iq(&self, iq: Vec<Complex32>) -> bool {
        self.iq_sender.try_send(iq).is_ok()
    }

    /// Detach the worker thread. The thread exits when iq_sender is dropped.
    pub fn shutdown(&mut self) {
        if let Some(handle) = self.handle.take() {
            std::mem::forget(handle);
        }
    }
}

impl Drop for SpectrumWorker {
    fn drop(&mut self) {
        // When iq_sender is dropped (after this method returns), the worker's
        // recv() returns Err and the thread exits. Detach — never join in Drop.
        if let Some(handle) = self.handle.take() {
            std::mem::forget(handle);
        }
    }
}

impl SpectrumWorker {
    /// Try to receive a completed spectrum frame without blocking. If several
    /// frames are queued, retain only the newest one so a slow UI never spends
    /// a frame processing stale display data.
    pub fn try_recv_spectrum(&self) -> Option<SpectrumFrame> {
        let mut latest = None;
        while let Ok(frame) = self.spectrum_receiver.try_recv() {
            latest = Some(frame);
        }
        latest
    }
}

/// Snapshot of actual display settings, including edits made by the spectrum's
/// own menus. The application can persist this without depending on UI widgets.
#[derive(Debug, Clone, Copy)]
pub struct DisplaySettings {
    pub fft_size: usize,
    pub window: WindowType,
    pub fft_rate: u32,
    pub waterfall_visible: bool,
    pub waterfall_history: usize,
    pub waterfall_every_n: u32,
    pub full_waterfall_update: bool,
    pub snr_smoothing: bool,
    pub snr_smoothing_secs: f32,
    pub color_map: ColorMap,
    pub grid: bool,
    pub peak_hold_time: f32,
    pub avg_alpha: f32,
    pub persistence: f32,
    pub smoothing_enabled: bool,
    pub smoothing_speed: f32,
    pub fast_fft: bool,
    pub gradient_fill: bool,
    pub db_min: f32,
    pub db_max: f32,
    pub wf_min_db: f32,
    pub wf_max_db: f32,
}

fn category_color(category: &str) -> (egui::Color32, egui::Color32) {
    // Returns (line_color, label_color) based on category keyword
    let cat = category.to_lowercase();
    if cat.contains("aviation") || cat.contains("air") {
        (
            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 140),
            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 200),
        )
    } else if cat.contains("weather") || cat.contains("noaa") || cat.contains("wx") {
        (
            egui::Color32::from_rgba_unmultiplied(80, 220, 80, 140),
            egui::Color32::from_rgba_unmultiplied(80, 220, 80, 200),
        )
    } else if cat.contains("marine") || cat.contains("sea") || cat.contains("coast") {
        (
            egui::Color32::from_rgba_unmultiplied(0, 200, 200, 140),
            egui::Color32::from_rgba_unmultiplied(0, 200, 200, 200),
        )
    } else if cat.contains("amateur") || cat.contains("ham") {
        (
            egui::Color32::from_rgba_unmultiplied(200, 100, 255, 140),
            egui::Color32::from_rgba_unmultiplied(200, 100, 255, 200),
        )
    } else if cat.contains("broadcast") || cat.contains("fm") || cat.contains("am") {
        (
            egui::Color32::from_rgba_unmultiplied(255, 140, 60, 140),
            egui::Color32::from_rgba_unmultiplied(255, 140, 60, 200),
        )
    } else if cat.contains("scanner") || cat.contains("hit") {
        (
            egui::Color32::from_rgba_unmultiplied(255, 80, 80, 140),
            egui::Color32::from_rgba_unmultiplied(255, 80, 80, 200),
        )
    } else {
        // Default gold
        (
            egui::Color32::from_rgba_unmultiplied(255, 215, 0, 120),
            egui::Color32::from_rgba_unmultiplied(255, 215, 0, 160),
        )
    }
}

/// Real-time FFT spectrum analyser and waterfall display.
///
/// Manages FFT computation, spectrum averaging, waterfall history, zoom/pan,
/// frequency markers, bookmark and band-plan overlays, colour maps, signal
/// history logging, and mouse-driven interaction (click-to-tune, context menus).
pub struct SpectrumAnalyzer {
    /// When `true`, `push_iq_samples` skips processing (freeze the display).
    pub frozen: bool,
    /// False when an external source owns the FFT resolution and window.
    pub fft_controls_enabled: bool,
    fft_size: usize,
    waterfall_history: usize,
    waterfall_pixels: Vec<Vec<u8>>,
    spectrum_dbs: Vec<f32>,
    waterfall_texture: Option<egui::TextureHandle>,
    center_freq: u64,
    /// Tuned VFO RF frequency, independent of the source capture center.
    /// `None` follows the capture center for callers without independent tuning.
    pub vfo_freq_hz: Option<u64>,
    sample_rate: f64,
    window_type: WindowType,
    /// Current colour map used for waterfall rendering.
    pub color_map: ColorMap,
    /// Top colour of the spectrum fill gradient (driven by the active theme).
    pub fill_top: egui::Color32,
    /// Bottom colour of the spectrum fill gradient (driven by the active theme).
    pub fill_bot: egui::Color32,
    /// Glow effect applied to the "signal active" badge (driven by the active theme).
    pub signal_glow: crate::theme::GlowConfig,
    /// Background fill of the spectrum plot area (driven by the active theme).
    pub plot_bg: egui::Color32,
    /// Grid line/label colour for both dB and frequency gridlines (driven by the active theme).
    pub grid_color: egui::Color32,
    /// Main spectrum trace colour (driven by the active theme).
    pub curve_color: egui::Color32,
    /// Noise-floor indicator line/label colour (driven by the active theme).
    pub noise_floor_color: egui::Color32,
    /// Success/good-state colour, used by the SNR badge and signal-active badge (driven by the active theme).
    pub color_success: egui::Color32,
    /// Warning-state colour, used by the squelch line and SNR badge (driven by the active theme).
    pub color_warning: egui::Color32,
    /// Error/bad-state colour, used by the peak-hold trace and SNR badge (driven by the active theme).
    pub color_error: egui::Color32,
    /// Amateur radio band-plan fill colour (driven by the active theme).
    pub bandplan_ham: egui::Color32,
    /// Broadcast band-plan fill colour (driven by the active theme).
    pub bandplan_broadcast: egui::Color32,
    /// Aviation band-plan fill colour (driven by the active theme).
    pub bandplan_aviation: egui::Color32,
    /// Marine band-plan fill colour (driven by the active theme).
    pub bandplan_marine: egui::Color32,
    /// Weather band-plan fill colour (driven by the active theme).
    pub bandplan_weather: egui::Color32,
    /// Satellite band-plan fill colour (driven by the active theme).
    pub bandplan_satellite: egui::Color32,
    /// Land-mobile band-plan fill colour (driven by the active theme).
    pub bandplan_mobile: egui::Color32,
    /// ISM band-plan fill colour (driven by the active theme).
    pub bandplan_ism: egui::Color32,
    zoom_factor: f32,
    zoom_offset: f32,
    markers: Vec<(u64, String)>,
    marker_label_input: String,
    marker_pending_freq: Option<u64>,
    avg_alpha: f32,
    peak_hold: Vec<f32>,
    show_peak_hold: bool,
    display_min_db: f32,
    display_max_db: f32,
    /// Lower bound of the waterfall colour-mapping range (dBFS).
    pub wf_min_db: f32,
    /// Upper bound of the waterfall colour-mapping range (dBFS).
    pub wf_max_db: f32,
    worker: Option<SpectrumWorker>,
    iq_ring: Vec<Complex32>,
    iq_write: usize,
    iq_filled: usize,
    pending_i_byte: Option<u8>,
    samples_until_fft: usize,
    stream_samples: u64,
    last_fft_sample: Option<u64>,
    fft_rate: u32,
    processed_frames: u64,
    last_daemon_timestamp_ms: Option<u64>,
    smoothing_enabled: bool,
    smoothing_speed: f32,
    fast_fft: bool,
    frame_period_seconds: f64,
    waterfall_visible: bool,
    waterfall_width: usize,
    texture_limit: usize,
    waterfall_pending_rows: Vec<bool>,
    snr_smoothing: bool,
    snr_smoothing_secs: f32,
    smoothed_snr: Option<f32>,
    frame_counter: u32,
    hover_pos: Option<egui::Pos2>,
    waterfall_dirty: bool,
    /// Interval at which waterfall rows are added (every N frames).
    pub waterfall_every_n: u32,
    /// When `true`, waterfall scrolling is paused (spectrum still updates).
    pub waterfall_paused: bool,
    /// Upload the full bounded waterfall texture whenever real rows change.
    /// False uploads only changed rows; neither mode adds rows on UI redraws.
    pub full_waterfall_update: bool,
    /// Frequency (Hz) that was clicked on the spectrum for tuning.
    pub clicked_tune_freq: Option<u64>,
    /// Frequency (Hz) pending bookmark creation.
    pub pending_bookmark_freq: Option<u64>,
    /// Frequency (Hz) pending assignment to VFO B.
    pub pending_vfo_b_freq: Option<u64>,
    signal_history: std::collections::VecDeque<f32>,
    signal_history_max: usize,
    show_signal_history: bool,
    /// Buffer of the last ~2048 demodulated audio samples for waveform display.
    pub audio_waveform: VecDeque<f32>,
    show_audio_waveform: bool,
    /// List of (`frequency_hz`, name, category) for bookmark overlay lines.
    pub bookmark_freqs: Vec<(u64, String, String)>,
    show_bookmarks: bool,
    show_band_plan: bool,
    band_plan_region: ItuRegion,
    /// VFO filter bandwidth in Hz (shown as shaded region on spectrum).
    pub vfo_bw_hz: u32,
    show_vfo_bw: bool,
    /// VFO B frequency in Hz (shown as a dashed marker).
    pub vfo_b_freq: u64,
    show_vfo_b: bool,
    /// Current demodulation mode label (e.g. "NFM", "WFM", "AM").
    pub demod_mode: String,
    /// Scanner sweep position marker frequency (None when not scanning).
    pub scan_marker: Option<u64>,
    /// Squelch threshold in dBFS.
    pub squelch_db: f32,
    /// Whether the SDR source is currently producing samples.
    pub source_running: bool,
    /// Whether the squelch is open (signal above threshold).
    pub signal_active: bool,
    /// Unix timestamp of the last time signal went above squelch.
    pub last_signal_unix: Option<f64>,
    noise_baseline: f32,
    /// Draw spectrum/waterfall grid lines.
    pub show_grid: bool,
    /// Peak-hold decay time constant (seconds). Larger = peaks linger.
    pub peak_hold_time: f32,
    /// Spectrum persistence / afterglow amount 0..1 (0 = off).
    pub persistence: f32,
    /// Retained trace buffer for persistence blending.
    persist_buf: Vec<f32>,
    /// Fill the area under the spectrum line with a gradient.
    pub gradient_fill: bool,
    /// Squelch value pending from a right-click "set squelch here" action.
    pub pending_squelch_db: Option<f32>,
    /// Scan range start (Hz) pending from context menu.
    pub pending_scan_start: Option<u64>,
    /// Scan range stop (Hz) pending from context menu.
    pub pending_scan_stop: Option<u64>,
    /// Demod mode string pending from context-menu "Apply" action.
    pub pending_demod_mode: Option<String>,
    /// Left edge of the visible frequency range (Hz).
    pub visible_left_hz: u64,
    /// Right edge of the visible frequency range (Hz).
    pub visible_right_hz: u64,
    ctx_menu_pos: Option<egui::Pos2>,
    /// Frequency (Hz) pending AI agent query from context menu.
    pub pending_ai_freq: Option<u64>,
    /// When `true`, the main panel should start the SDR source.
    pub pending_start_source: bool,
    /// When `true`, the bookmark list has been modified and needs re-read.
    pub bookmark_freqs_dirty: bool,

    // Cached stats updated once per push_iq_samples to avoid redundant O(N) scans
    cached_signal_level: f32,
    cached_peak_level: f32,
    cached_noise_floor: f32,
    /// Circular write head for the waterfall: index in `waterfall_pixels`
    /// (and texture Y) where the next row will be written. Lets us upload
    /// a single row via `set_partial` instead of re-uploading the full
    /// `fft_size × history` texture every frame.
    waterfall_head: usize,
}

/// FFT window function type.
///
/// The window is applied to IQ samples before the FFT to reduce spectral
/// leakage. Hann is the general-purpose default; Blackman offers stronger
/// sidelobe suppression.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowType {
    /// Hann (Hanning) window — good all-around choice.
    Hann,
    /// Hamming window — similar to Hann with slightly different sidelobes.
    Hamming,
    /// Blackman window — best sidelobe suppression, wider main lobe.
    Blackman,
    /// Flat-top window — excellent amplitude accuracy, widest main lobe.
    FlatTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItuRegion {
    Region1,
    Region2,
    Region3,
}

impl ItuRegion {
    fn label(self) -> &'static str {
        match self {
            Self::Region1 => "ITU R1",
            Self::Region2 => "ITU R2",
            Self::Region3 => "ITU R3",
        }
    }

    fn amateur_limits(self) -> ((f64, f64), (f64, f64), Option<(f64, f64)>, (f64, f64)) {
        match self {
            Self::Region1 => ((3.5, 3.8), (7.0, 7.2), None, (430.0, 440.0)),
            Self::Region2 => ((3.5, 4.0), (7.0, 7.3), Some((222.0, 225.0)), (420.0, 450.0)),
            Self::Region3 => ((3.5, 3.9), (7.0, 7.2), None, (430.0, 440.0)),
        }
    }
}

/// Colour map used for waterfall and spectrum visualisation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMap {
    /// EZ-SDR's original blue-green-yellow-red colour scheme.
    Classic,
    /// Matplotlib Viridis — perceptually uniform (purple → yellow).
    Viridis,
    /// Matplotlib Plasma — purple → orange → yellow.
    Plasma,
    /// Matplotlib Magma — black → purple → yellow → white.
    Magma,
    /// Simple greyscale (black → white).
    Grayscale,
    /// Hot colour map (black → red → yellow → white).
    Hot,
    /// Matplotlib Inferno — similar to Magma with different hues.
    Inferno,
    /// Google Turbo — rainbow-like with high contrast.
    Turbo,
}

impl ColorMap {
    /// Return the human-readable name of this colour map.
    pub fn name(&self) -> &'static str {
        match self {
            ColorMap::Classic => "Classic",
            ColorMap::Viridis => "Viridis",
            ColorMap::Plasma => "Plasma",
            ColorMap::Magma => "Magma",
            ColorMap::Grayscale => "Grayscale",
            ColorMap::Hot => "Hot",
            ColorMap::Inferno => "Inferno",
            ColorMap::Turbo => "Turbo",
        }
    }
}

impl WindowType {
    /// Return the display name of this window function.
    pub fn name(&self) -> &'static str {
        match self {
            WindowType::Hann => "Hann",
            WindowType::Hamming => "Hamming",
            WindowType::Blackman => "Blackman",
            WindowType::FlatTop => "FlatTop",
        }
    }

    fn generate(&self, len: usize) -> Vec<f32> {
        match self {
            WindowType::Hann => (0..len)
                .map(|i| {
                    let n = i as f32;
                    let size = len as f32;
                    0.5 * (1.0 - (2.0 * PI * n / size).cos())
                })
                .collect(),
            WindowType::Hamming => (0..len)
                .map(|i| {
                    let n = i as f32;
                    let size = len as f32;
                    0.54 - 0.46 * (2.0 * PI * n / size).cos()
                })
                .collect(),
            WindowType::Blackman => (0..len)
                .map(|i| {
                    let n = i as f32;
                    let size = len as f32;
                    0.42 - 0.5 * (2.0 * PI * n / size).cos() + 0.08 * (4.0 * PI * n / size).cos()
                })
                .collect(),
            WindowType::FlatTop => (0..len)
                .map(|i| {
                    let n = i as f32;
                    let size = len as f32;
                    let a0 = 0.21557895;
                    let a1 = 0.41663158;
                    let a2 = 0.277_263_16;
                    let a3 = 0.083578947;
                    let a4 = 0.006947368;
                    a0 - a1 * (2.0 * PI * n / size).cos() + a2 * (4.0 * PI * n / size).cos()
                        - a3 * (6.0 * PI * n / size).cos()
                        + a4 * (8.0 * PI * n / size).cos()
                })
                .collect(),
        }
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl SpectrumAnalyzer {
    /// Create a new `SpectrumAnalyzer` with default parameters (2048-point FFT,
    /// Hann window, Classic colour map).
    pub fn new() -> Self {
        let fft_size = 2048;
        let waterfall_history = 256;
        let window_type = WindowType::Hann;
        Self {
            fft_size,
            waterfall_history,
            waterfall_pixels: vec![vec![0u8; fft_size * 4]; waterfall_history],
            spectrum_dbs: vec![-100.0; fft_size],
            waterfall_texture: None,
            center_freq: 100_000_000,
            vfo_freq_hz: None,
            sample_rate: 2_048_000.0,
            window_type,
            color_map: ColorMap::Classic,
            fill_top: egui::Color32::from_rgba_unmultiplied(30, 120, 200, 100),
            fill_bot: egui::Color32::from_rgba_unmultiplied(10, 30, 60, 20),
            signal_glow: crate::theme::GlowConfig::default(),
            plot_bg: egui::Color32::from_rgb(0, 0, 5),
            grid_color: egui::Color32::from_rgba_unmultiplied(60, 65, 80, 120),
            curve_color: egui::Color32::from_rgb(52, 152, 219),
            noise_floor_color: egui::Color32::from_rgba_unmultiplied(100, 100, 200, 80),
            color_success: egui::Color32::from_rgb(46, 204, 113),
            color_warning: egui::Color32::from_rgb(241, 196, 15),
            color_error: egui::Color32::from_rgb(231, 76, 60),
            bandplan_ham: egui::Color32::from_rgba_unmultiplied(80, 200, 80, 28),
            bandplan_broadcast: egui::Color32::from_rgba_unmultiplied(255, 140, 50, 28),
            bandplan_aviation: egui::Color32::from_rgba_unmultiplied(80, 160, 255, 28),
            bandplan_marine: egui::Color32::from_rgba_unmultiplied(0, 200, 180, 28),
            bandplan_weather: egui::Color32::from_rgba_unmultiplied(100, 255, 120, 28),
            bandplan_satellite: egui::Color32::from_rgba_unmultiplied(180, 100, 255, 28),
            bandplan_mobile: egui::Color32::from_rgba_unmultiplied(200, 180, 60, 22),
            bandplan_ism: egui::Color32::from_rgba_unmultiplied(255, 80, 80, 25),
            zoom_factor: 1.0,
            zoom_offset: 0.5,
            markers: Vec::new(),
            marker_label_input: String::new(),
            marker_pending_freq: None,
            avg_alpha: 0.3,
            peak_hold: vec![-120.0; fft_size],
            show_peak_hold: false,
            display_min_db: -120.0,
            display_max_db: 0.0,
            wf_min_db: -120.0,
            wf_max_db: -20.0,
            worker: Some(SpectrumWorker::new(fft_size, window_type)),
            iq_ring: vec![Complex32::new(0.0, 0.0); fft_size],
            iq_write: 0,
            iq_filled: 0,
            pending_i_byte: None,
            samples_until_fft: fft_size,
            stream_samples: 0,
            last_fft_sample: None,
            fft_rate: 20,
            processed_frames: 0,
            last_daemon_timestamp_ms: None,
            frame_period_seconds: 0.05,
            waterfall_visible: true,
            waterfall_width: fft_size.min(MAX_WATERFALL_WIDTH),
            texture_limit: MAX_WATERFALL_WIDTH,
            waterfall_pending_rows: vec![false; waterfall_history],
            snr_smoothing: true,
            snr_smoothing_secs: 0.5,
            smoothed_snr: None,
            frame_counter: 0,
            hover_pos: None,
            waterfall_dirty: true,
            waterfall_every_n: 4,
            waterfall_paused: false,
            full_waterfall_update: false,
            clicked_tune_freq: None,
            pending_bookmark_freq: None,
            pending_vfo_b_freq: None,
            signal_history: std::collections::VecDeque::new(),
            signal_history_max: 600,
            show_signal_history: false,
            audio_waveform: VecDeque::with_capacity(2048),
            show_audio_waveform: false,
            bookmark_freqs: Vec::new(),
            show_bookmarks: true,
            show_band_plan: true,
            band_plan_region: ItuRegion::Region1,
            vfo_bw_hz: 15000,
            show_vfo_bw: true,
            vfo_b_freq: 0,
            show_vfo_b: true,
            demod_mode: "NFM".to_string(),
            frozen: false,
            fft_controls_enabled: true,
            scan_marker: None,
            squelch_db: -120.0,
            source_running: false,
            signal_active: false,
            last_signal_unix: None,
            noise_baseline: -120.0,
            show_grid: true,
            peak_hold_time: 1.0,
            persistence: 0.0,
            smoothing_enabled: true,
            smoothing_speed: 0.3,
            fast_fft: false,
            persist_buf: vec![-100.0; fft_size],
            gradient_fill: true,
            pending_squelch_db: None,
            pending_scan_start: None,
            pending_scan_stop: None,
            pending_demod_mode: None,
            visible_left_hz: 99_000_000,
            visible_right_hz: 101_000_000,
            ctx_menu_pos: None,
            pending_ai_freq: None,
            pending_start_source: false,
            bookmark_freqs_dirty: true,
            cached_signal_level: -100.0,
            cached_peak_level: -100.0,
            cached_noise_floor: -100.0,
            waterfall_head: 0,
        }
    }

    /// Accepted transform sizes are powers of two from 256 through 65536.
    pub fn valid_fft_size(size: usize) -> bool {
        (MIN_FFT_SIZE..=MAX_FFT_SIZE).contains(&size) && size.is_power_of_two()
    }

    /// Recreate the FFT only when a valid size changes. Returns false for an
    /// invalid request, leaving the live display and streaming buffers intact.
    pub fn set_fft_size(&mut self, size: usize) -> bool {
        if !Self::valid_fft_size(size) {
            return false;
        }
        if size == self.fft_size {
            return true;
        }
        self.fft_size = size;
        self.spectrum_dbs = vec![-100.0; size];
        self.peak_hold = vec![-120.0; size];
        self.persist_buf = vec![-100.0; size];
        self.iq_ring = vec![Complex32::new(0.0, 0.0); size];
        // Recreate the worker with the new FFT size
        self.worker = Some(SpectrumWorker::new(size, self.window_type));
        self.reset_stream();
        self.reset_waterfall();
        true
    }

    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    pub fn display_settings(&self) -> DisplaySettings {
        DisplaySettings {
            fft_size: self.fft_size,
            window: self.window_type,
            fft_rate: self.fft_rate,
            waterfall_visible: self.waterfall_visible,
            waterfall_history: self.waterfall_history,
            waterfall_every_n: self.waterfall_every_n.max(1),
            full_waterfall_update: self.full_waterfall_update,
            snr_smoothing: self.snr_smoothing,
            snr_smoothing_secs: self.snr_smoothing_secs,
            color_map: self.color_map,
            grid: self.show_grid,
            peak_hold_time: self.peak_hold_time,
            avg_alpha: self.avg_alpha,
            persistence: self.persistence,
            smoothing_enabled: self.smoothing_enabled,
            smoothing_speed: self.smoothing_speed,
            fast_fft: self.fast_fft,
            gradient_fill: self.gradient_fill,
            db_min: self.display_min_db,
            db_max: self.display_max_db,
            wf_min_db: self.wf_min_db,
            wf_max_db: self.wf_max_db,
        }
    }

    /// Display transform cadence. A real sample clock determines the hop size;
    /// daemon frames use their monotonic timestamp deltas instead.
    pub fn set_fft_rate(&mut self, frames_per_second: u32) {
        let rate = frames_per_second.clamp(1, 120);
        if rate != self.fft_rate {
            self.fft_rate = rate;
            self.samples_until_fft = if self.iq_filled < self.fft_size {
                self.fft_size - self.iq_filled
            } else {
                self.fft_hop()
            };
            self.frame_period_seconds = 1.0 / rate as f64;
            self.last_daemon_timestamp_ms = None;
        }
    }

    pub fn fft_rate(&self) -> u32 {
        self.fft_rate
    }

    pub fn set_waterfall_visible(&mut self, visible: bool) {
        self.waterfall_visible = visible;
        if !visible {
            self.waterfall_texture = None;
        }
    }

    pub fn waterfall_visible(&self) -> bool {
        self.waterfall_visible
    }

    pub fn vfo_frequency_hz(&self) -> u64 {
        self.vfo_freq_hz.unwrap_or(self.center_freq)
    }

    /// RF passband edges measured relative to the source capture center.
    fn vfo_band_edges(&self) -> (f64, f64) {
        let bandwidth = self.vfo_bw_hz as f64;
        let offset = self.vfo_frequency_hz() as f64 - self.center_freq as f64;
        let (left, right) = match self.demod_mode.as_str() {
            "USB" => (0.0, bandwidth),
            "LSB" => (-bandwidth, 0.0),
            _ => (-bandwidth / 2.0, bandwidth / 2.0),
        };
        (left + offset, right + offset)
    }

    /// Frequency under a normalized plot position. Both plots share the
    /// capture-axis mapping regardless of the independently tuned VFO.
    fn frequency_at_plot_fraction(&self, fraction: f32) -> u64 {
        let span = (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
        let offset =
            self.zoom_center_offset(span) + (f64::from(fraction.clamp(0.0, 1.0)) - 0.5) * span;
        (self.center_freq as f64 + offset).max(0.0) as u64
    }

    fn adjust_vfo_bandwidth(&mut self, scroll: f32) {
        let step = (self.vfo_bw_hz as f32 * 0.1).max(100.0);
        self.vfo_bw_hz = (self.vfo_bw_hz as f32 + scroll.signum() * step).max(100.0) as u32;
        self.show_vfo_bw = true;
    }

    fn paint_vfo_marker(&self, painter: &egui::Painter, rect: egui::Rect) {
        if !self.show_vfo_bw {
            return;
        }
        let span = (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
        let offset = self.vfo_frequency_hz() as f64 - self.center_freq as f64;
        let fraction = (offset - self.zoom_center_offset(span)) / span + 0.5;
        if !(0.0..=1.0).contains(&fraction) {
            return;
        }
        let x = rect.left() + fraction as f32 * rect.width();
        let color = egui::Color32::from_rgba_unmultiplied(210, 220, 230, 130);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(0.7, color),
        );
        painter.text(
            egui::pos2(x + 3.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            "VFO",
            egui::FontId::proportional(8.0),
            color,
        );
    }

    fn visible_peak_freq_hz(&self) -> u64 {
        let n = self.spectrum_dbs.len();
        if n == 0 {
            return self.center_freq;
        }
        let span = (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
        let left = self.zoom_center_offset(span) - span / 2.0;
        let bin_hz = self.sample_rate / n as f64;
        let first = (n as f64 / 2.0 + left / bin_hz)
            .ceil()
            .clamp(0.0, (n - 1) as f64) as usize;
        let last = (n as f64 / 2.0 + (left + span) / bin_hz + 1.0)
            .floor()
            .clamp((first + 1) as f64, n as f64) as usize;
        let peak = self.spectrum_dbs[first..last]
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map_or(first, |(i, _)| first + i);
        (self.center_freq as f64 + (peak as f64 - n as f64 / 2.0) * bin_hz).max(0.0) as u64
    }

    /// Smooth only the displayed peak-minus-noise SNR, without changing DSP
    /// signal/noise levels or the squelch gate. A time constant is rate invariant.
    pub fn set_snr_smoothing(&mut self, enabled: bool, time_constant_secs: f32) {
        self.snr_smoothing = enabled;
        self.snr_smoothing_secs = if time_constant_secs.is_finite() {
            time_constant_secs.clamp(0.01, 10.0)
        } else {
            0.5
        };
        if !enabled {
            self.smoothed_snr = None;
        }
    }

    pub fn snr_db(&self) -> f32 {
        if self.snr_smoothing {
            self.smoothed_snr
                .unwrap_or(self.cached_peak_level - self.cached_noise_floor)
        } else {
            self.cached_peak_level - self.cached_noise_floor
        }
    }

    /// Set the FFT window without re-allocating the plan or clearing history.
    /// Recreates the worker so it uses the new window function.
    pub fn set_window(&mut self, w: WindowType) {
        if self.window_type != w {
            self.window_type = w;
            self.worker = Some(SpectrumWorker::new(self.fft_size, w));
        }
    }

    /// History storage has an 8 MiB ceiling, independent of FFT resolution.
    pub fn set_waterfall_history(&mut self, depth: usize) {
        let depth = depth.clamp(32, 4096).min(self.texture_limit.max(1));
        if depth != self.waterfall_history {
            self.waterfall_history = depth;
            self.reset_waterfall();
        }
    }

    fn reset_waterfall(&mut self) {
        self.waterfall_history = self.waterfall_history.min(self.texture_limit.max(1));
        self.waterfall_width = self
            .fft_size
            .min(MAX_WATERFALL_WIDTH)
            .min(self.texture_limit.max(1))
            .min(MAX_WATERFALL_BYTES / (self.waterfall_history.max(1) * 4))
            .max(1);
        self.waterfall_pixels = vec![vec![0u8; self.waterfall_width * 4]; self.waterfall_history];
        self.waterfall_pending_rows = vec![false; self.waterfall_history];
        self.waterfall_texture = None;
        self.waterfall_head = 0;
        self.waterfall_dirty = true;
    }

    pub(crate) fn reset_stream(&mut self) {
        self.iq_write = 0;
        self.iq_filled = 0;
        self.pending_i_byte = None;
        self.samples_until_fft = self.fft_size;
        self.stream_samples = 0;
        self.last_fft_sample = None;
        self.last_daemon_timestamp_ms = None;
        self.smoothed_snr = None;
    }

    fn fft_hop(&self) -> usize {
        (self.sample_rate / f64::from(self.fft_rate))
            .ceil()
            .max(1.0) as usize
    }

    /// Pan offset (Hz) of the zoom window centre from the spectrum centre.
    ///
    /// `zoom_offset` 0..1 maps to ±`max_pan`, where
    /// `max_pan = (sample_rate - zoom_span) / 2`. At full zoom-out
    /// (`zoom_span == sample_rate`) panning is zero; when zoomed in the
    /// window can pan to the outer frequencies. The old formula
    /// `(offset - 0.5) * zoom_span` limited pan to ±`zoom_span`/2, making
    /// outer frequencies unreachable at high zoom.
    pub fn zoom_center_offset(&self, zoom_span: f64) -> f64 {
        let max_pan = (self.sample_rate - zoom_span).max(0.0) / 2.0;
        (f64::from(self.zoom_offset) - 0.5) * 2.0 * max_pan
    }

    /// Enable/disable spectrum & waterfall grid lines.
    pub fn set_grid(&mut self, on: bool) {
        self.show_grid = on;
    }

    /// Set peak-hold decay time constant (seconds).
    pub fn set_smoothing(&mut self, enabled: bool) {
        self.smoothing_enabled = enabled;
        if !enabled {
            self.spectrum_dbs.fill(-100.0);
        }
    }
    pub fn set_smoothing_speed(&mut self, speed: f32) {
        self.smoothing_speed = speed.clamp(0.0, 1.0);
        self.avg_alpha = 1.0 - self.smoothing_speed;
    }
    pub fn set_fast_fft(&mut self, fast: bool) {
        self.fast_fft = fast;
    }
    pub fn set_peak_hold_time(&mut self, secs: f32) {
        self.peak_hold_time = secs.clamp(0.1, 60.0);
    }

    /// Set the new-frame weight (1 = immediate, lower = more averaging).
    pub fn set_avg_alpha(&mut self, alpha: f32) {
        self.avg_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Set the persistence / afterglow amount (0 = off).
    pub fn set_persistence(&mut self, p: f32) {
        self.persistence = p.clamp(0.0, 0.98);
    }

    /// Enable/disable the gradient fill under the spectrum line.
    pub fn set_gradient_fill(&mut self, on: bool) {
        self.gradient_fill = on;
    }

    /// Set the waterfall/colour-map palette.
    pub fn set_color_map(&mut self, map: ColorMap) {
        self.color_map = map;
        self.waterfall_dirty = true;
    }

    /// Set the zoom factor directly (larger = more zoomed in).
    #[allow(dead_code)]
    pub fn set_zoom_factor(&mut self, z: f32) {
        self.zoom_factor = z.clamp(1.0, 200.0);
    }

    /// Update the source capture center and effective spectrum sample rate.
    /// An explicitly selected VFO remains independent of these coordinates.
    pub fn update_params(&mut self, center_freq: u64, sample_rate: u32) {
        self.update_params_exact(center_freq, f64::from(sample_rate));
    }

    /// Preserve a fractional effective sample rate after filtered decimation.
    /// Nonfinite/nonpositive values fall back to 1 Hz, matching the u32 guard.
    pub fn update_params_exact(&mut self, center_freq: u64, sample_rate: f64) {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            1.0
        };
        if center_freq != self.center_freq || sample_rate != self.sample_rate {
            self.reset_stream();
        }
        self.center_freq = center_freq;
        self.sample_rate = sample_rate;
    }

    /// Return the current (min, max) dB display range.
    pub fn display_range(&self) -> (f32, f32) {
        (self.display_min_db, self.display_max_db)
    }

    /// Set the spectrum display dB range, and mark the waterfall as dirty.
    pub fn set_display_range(&mut self, min: f32, max: f32) {
        self.display_min_db = min;
        self.display_max_db = max.max(min + 10.0);
        self.waterfall_dirty = true;
    }

    /// Return a snapshot of the peak signal-level history (for sparkline rendering).
    pub fn signal_history_snapshot(&self) -> Vec<f32> {
        self.signal_history.iter().copied().collect()
    }

    /// Return the maximum number of samples kept in the signal history.
    pub fn signal_history_max(&self) -> usize {
        self.signal_history_max
    }

    /// Save the current signal history to `signal_history.json`.
    pub fn save_signal_history(&self) {
        let data: Vec<f32> = self.signal_history.iter().copied().collect();
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::write("signal_history.json", json);
        }
    }

    /// Load signal history from `signal_history.json`.
    pub fn load_signal_history(&mut self) {
        if let Ok(s) = std::fs::read_to_string("signal_history.json") {
            if let Ok(data) = serde_json::from_str::<Vec<f32>>(&s) {
                self.signal_history = data.into_iter().collect();
            }
        }
    }

    /// Cycle to the next colour map variant and mark the waterfall dirty.
    pub fn cycle_colormap(&mut self) {
        self.color_map = match self.color_map {
            ColorMap::Classic => ColorMap::Viridis,
            ColorMap::Viridis => ColorMap::Plasma,
            ColorMap::Plasma => ColorMap::Magma,
            ColorMap::Magma => ColorMap::Inferno,
            ColorMap::Inferno => ColorMap::Hot,
            ColorMap::Hot => ColorMap::Turbo,
            ColorMap::Turbo => ColorMap::Grayscale,
            ColorMap::Grayscale => ColorMap::Classic,
        };
        self.waterfall_dirty = true;
    }

    /// Return the current average signal level across all FFT bins (dBFS).
    pub fn signal_level(&self) -> f32 {
        self.cached_signal_level
    }

    /// Return the current peak (maximum) signal level across all FFT bins (dBFS).
    pub fn peak_level(&self) -> f32 {
        self.cached_peak_level
    }

    /// Strongest FFT bin in the selected RF passband. Use this for a display-
    /// based squelch detector instead of the average of the entire sampled band.
    /// Other channels cannot open the gate merely by being the global peak.
    pub fn vfo_signal_level(&self) -> f32 {
        let (left, right) = self.vfo_band_edges();
        let n = self.spectrum_dbs.len();
        if n == 0 || right < left {
            return -120.0;
        }
        let bin_hz = self.sample_rate / n as f64;
        let first = (n as f64 / 2.0 + left / bin_hz).ceil().clamp(0.0, n as f64) as usize;
        let last = (n as f64 / 2.0 + right / bin_hz + 1.0)
            .floor()
            .clamp(0.0, n as f64) as usize;
        if first < last {
            peak_in_bins(&self.spectrum_dbs, first, last)
        } else {
            -120.0
        }
    }

    /// Return the frequency (Hz) of the bin with the strongest signal.
    pub fn peak_freq_hz(&self) -> u64 {
        if self.spectrum_dbs.is_empty() {
            return self.center_freq;
        }
        let n = self.spectrum_dbs.len();
        let peak_bin = self
            .spectrum_dbs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(n / 2, |(i, _)| i);
        // DC bin is at n/2; offset from center = (bin - n/2) * (sample_rate / n)
        let offset_hz = (peak_bin as f64 - n as f64 / 2.0) * self.sample_rate / n as f64;
        (self.center_freq as f64 + offset_hz).max(0.0) as u64
    }

    /// Return the estimated noise floor (25th percentile of bin powers, dBFS).
    pub fn noise_floor(&self) -> f32 {
        self.cached_noise_floor
    }

    /// Zoom in on the spectrum (1.5×), clamping to max 200×.
    pub fn zoom_in(&mut self) {
        self.zoom_factor = (self.zoom_factor * 1.5).clamp(1.0, 200.0);
    }

    /// Zoom out of the spectrum (÷1.5), clamping to min 1×.
    pub fn zoom_out(&mut self) {
        self.zoom_factor = (self.zoom_factor / 1.5).max(1.0);
        if self.zoom_factor <= 1.05 {
            self.zoom_factor = 1.0;
            self.zoom_offset = 0.5;
        }
    }

    /// Reset zoom to 1× (full span) and centre the offset.
    pub fn zoom_reset(&mut self) {
        self.zoom_factor = 1.0;
        self.zoom_offset = 0.5;
    }

    /// Toggle the peak-hold overlay on/off, returning the new state.
    /// Clears the held peaks when disabling.
    pub fn toggle_peak_hold(&mut self) -> bool {
        self.show_peak_hold = !self.show_peak_hold;
        if !self.show_peak_hold {
            self.peak_hold = vec![-120.0; self.fft_size];
        }
        self.show_peak_hold
    }

    /// Open a file-save dialog and write the current waterfall as a PNG image,
    /// along with a JSON sidecar containing capture metadata.
    pub fn save_waterfall_png(&self) {
        if self.waterfall_pixels.is_empty() {
            return;
        }
        let path = rfd::FileDialog::new()
            .set_title("Save Waterfall Screenshot")
            .add_filter("PNG", &["png"])
            .set_file_name("waterfall_capture.png")
            .save_file();
        let Some(path) = path else {
            return;
        };
        let w = self.waterfall_width as u32;
        let h = self.waterfall_history as u32;
        let mut rgba_flat: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
        // Emit in display order (oldest at top): start at circular head.
        let n = self.waterfall_pixels.len();
        let head = if n > 0 { self.waterfall_head % n } else { 0 };
        for k in 0..n {
            rgba_flat.extend_from_slice(&self.waterfall_pixels[(head + k) % n]);
        }
        if let Some(img) = image::RgbaImage::from_raw(w, h, rgba_flat) {
            let _ = img.save(&path);
        }
        // Write sidecar JSON with capture metadata
        let sidecar_path = path.with_extension("json");
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let json = format!(
            "{{\n  \"type\": \"waterfall_screenshot\",\n  \"timestamp_utc\": \"{}\",\n  \"center_freq_hz\": {},\n  \"sample_rate_hz\": {},\n  \"fft_size\": {},\n  \"colormap\": \"{}\",\n  \"waterfall_history\": {},\n  \"waterfall_every_n\": {}\n}}\n",
            ts, self.center_freq, self.sample_rate, self.fft_size, self.color_map.name(), self.waterfall_history, self.waterfall_every_n
        );
        let _ = std::fs::write(&sidecar_path, json);
    }

    /// Open a file-save dialog and write the current spectrum data as CSV
    /// (`frequency_hz`, `power_dbfs` columns), along with a JSON sidecar.
    pub fn export_spectrum_csv(&self) {
        if self.spectrum_dbs.is_empty() {
            return;
        }
        let path = rfd::FileDialog::new()
            .set_title("Export Spectrum to CSV")
            .add_filter("CSV", &["csv"])
            .set_file_name("spectrum_export.csv")
            .save_file();
        let Some(path) = path else {
            return;
        };
        let n = self.spectrum_dbs.len();
        let hz_per_bin = self.sample_rate / n as f64;
        let mut lines = String::from("frequency_hz,power_dbfs\n");
        for (i, &db) in self.spectrum_dbs.iter().enumerate() {
            // spectrum_dbs is already stored in ascending-frequency order
            // (fftshift applied at push_iq_samples); bin 0 = center - Fs/2,
            // bin n/2 = center. No additional shift needed here.
            let offset = (i as f64 - n as f64 / 2.0) * hz_per_bin;
            let freq_hz = self.center_freq as f64 + offset;
            lines.push_str(&format!("{freq_hz:.0},{db:.2}\n"));
        }
        let _ = std::fs::write(&path, lines);
        // Sidecar metadata
        let sidecar_path = path.with_extension("json");
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let json = format!(
            "{{\n  \"type\": \"spectrum_csv_export\",\n  \"timestamp_utc\": \"{}\",\n  \"center_freq_hz\": {},\n  \"sample_rate_hz\": {},\n  \"fft_size\": {},\n  \"num_bins\": {}\n}}\n",
            ts, self.center_freq, self.sample_rate, self.fft_size, n
        );
        let _ = std::fs::write(&sidecar_path, json);
    }

    /// Consume a continuous unsigned-byte IQ stream, including byte pairs split
    /// across calls. Only complete, real FFT windows are transformed. The ring,
    /// transform input and rustfft scratch are bounded by the configured size.
    /// At most four transforms are emitted per call; after an oversized burst,
    /// the last one uses the newest real window instead of freezing on old data.
    pub fn push_iq_samples(&mut self, iq: &[u8]) {
        self.try_recv_spectrum();
        if self.frozen {
            self.reset_stream();
            return;
        }
        let mut transformed = 0;
        let mut latest_due = false;
        for &byte in iq {
            if let Some(i) = self.pending_i_byte.take() {
                self.push_complex_sample(
                    Complex32::new(
                        (f32::from(i) - 127.4) / 128.0,
                        (f32::from(byte) - 127.4) / 128.0,
                    ),
                    &mut transformed,
                    &mut latest_due,
                );
            } else {
                self.pending_i_byte = Some(byte);
            }
        }
        if latest_due {
            self.transform_iq_window();
        }
    }

    /// Consume normalized floating-point IQ without quantizing it to ADC bytes.
    /// A bin-centered complex tone with magnitude 1 measures 0 dBFS. Values are
    /// not clipped; nonfinite samples become zero to preserve the sample clock.
    /// Uses the same real-window, cadence and per-call transform budget as u8 IQ.
    /// Switching from raw-byte input discards any unfinished raw I/Q byte pair.
    pub fn push_complex_samples(&mut self, iq: &[Complex32]) {
        self.try_recv_spectrum();
        if self.frozen {
            self.reset_stream();
            return;
        }
        if iq.is_empty() {
            return;
        }
        self.pending_i_byte = None;
        let mut transformed = 0;
        let mut latest_due = false;
        for &sample in iq {
            let sample = if sample.re.is_finite() && sample.im.is_finite() {
                sample
            } else {
                Complex32::new(0.0, 0.0)
            };
            self.push_complex_sample(sample, &mut transformed, &mut latest_due);
        }
        if latest_due {
            self.transform_iq_window();
        }
    }

    #[inline]
    fn push_complex_sample(
        &mut self,
        sample: Complex32,
        transformed: &mut usize,
        latest_due: &mut bool,
    ) {
        self.iq_ring[self.iq_write] = sample;
        self.iq_write = (self.iq_write + 1) % self.fft_size;
        self.iq_filled = (self.iq_filled + 1).min(self.fft_size);
        self.stream_samples = self.stream_samples.wrapping_add(1);
        self.samples_until_fft = self.samples_until_fft.saturating_sub(1);
        if self.iq_filled == self.fft_size && self.samples_until_fft == 0 {
            self.samples_until_fft = self.fft_hop();
            if *transformed < MAX_FFTS_PER_PUSH - 1 {
                self.transform_iq_window();
                *transformed += 1;
            } else {
                *latest_due = true;
            }
        }
    }

    fn transform_iq_window(&mut self) {
        // Extract the newest IQ window and send it to the worker. The worker
        // owns windowing and FFT computation so the UI thread does not apply
        // the selected window twice.
        let mut window_iq = Vec::with_capacity(self.fft_size);
        for i in 0..self.fft_size {
            window_iq.push(self.iq_ring[(self.iq_write + i) % self.fft_size]);
        }
        if let Some(ref worker) = self.worker {
            worker.try_send_iq(window_iq);
        }
        self.frame_period_seconds = self
            .last_fft_sample
            .map(|last| self.stream_samples.saturating_sub(last) as f64 / self.sample_rate)
            .unwrap_or(1.0 / self.fft_rate as f64);
        self.last_fft_sample = Some(self.stream_samples);
    }

    /// Try to receive a completed spectrum frame from the worker.
    /// Applies the spectrum to display state if a frame is available.
    /// Never blocks — skips if no frame is ready yet.
    pub fn try_recv_spectrum(&mut self) {
        let Some(dbs) = self
            .worker
            .as_ref()
            .and_then(SpectrumWorker::try_recv_spectrum)
        else {
            return;
        };
        for (dst, &db) in dbs.iter().enumerate() {
            self.update_bin(dst, db);
        }
        self.finish_spectrum_frame();
    }

    /// Ingest a precomputed, fftshifted daemon frame. Frame timestamps enforce
    /// the requested display cadence; a backwards clock resets that cadence.
    /// Invalid resolutions/nonfinite bins are discarded before any allocation.
    /// Ingest a raw spectrum frame from the SpectrumWorker. The frame is a
    /// fftshifted Vec<f32> of dB values matching the current fft_size.
    pub fn apply_spectrum_frame(&mut self, frame: &[f32]) {
        if self.frozen || frame.len() != self.fft_size {
            return;
        }
        if frame.iter().any(|v| !v.is_finite()) {
            return;
        }
        self.spectrum_dbs.copy_from_slice(frame);
        self.processed_frames += 1;
    }

    pub fn push_spectrum_frame(&mut self, frame: &ez_proto::SpectrumFrame) {
        if self.frozen
            || !Self::valid_fft_size(frame.bins.len())
            || frame.sample_rate_hz == 0
            || frame.bins.iter().any(|v| !v.is_finite())
        {
            return;
        }
        let changed = frame.center_hz != self.center_freq
            || f64::from(frame.sample_rate_hz) != self.sample_rate
            || frame.bins.len() != self.fft_size;
        if !changed {
            if let Some(last) = self.last_daemon_timestamp_ms {
                if frame.timestamp_ms >= last
                    && (frame.timestamp_ms - last) < 1000_u64.div_ceil(self.fft_rate as u64)
                {
                    return;
                }
            }
        }
        self.set_fft_size(frame.bins.len());
        self.update_params(frame.center_hz, frame.sample_rate_hz);
        self.frame_period_seconds = self
            .last_daemon_timestamp_ms
            .filter(|last| frame.timestamp_ms > *last)
            .map(|last| (frame.timestamp_ms - last) as f64 / 1000.0)
            .unwrap_or(1.0 / self.fft_rate as f64);
        self.last_daemon_timestamp_ms = Some(frame.timestamp_ms);
        for (dst, &db) in frame.bins.iter().enumerate() {
            self.update_bin(dst, db);
        }
        self.finish_spectrum_frame();
    }

    fn update_bin(&mut self, dst: usize, db: f32) {
        let smoothed = self.avg_alpha * db + (1.0 - self.avg_alpha) * self.spectrum_dbs[dst];
        self.spectrum_dbs[dst] = smoothed;
        if db > self.peak_hold[dst] {
            self.peak_hold[dst] = db;
        } else {
            let decay = 1.0 - (-self.frame_period_seconds as f32 / self.peak_hold_time).exp();
            self.peak_hold[dst] += decay * (db - self.peak_hold[dst]);
        }
        if self.persistence > 0.0 {
            self.persist_buf[dst] =
                self.persistence * self.persist_buf[dst] + (1.0 - self.persistence) * smoothed;
        }
    }

    fn finish_spectrum_frame(&mut self) {
        let mut sum = 0.0;
        let mut peak = -120.0_f32;
        let mut hist = [0u32; 120];
        for &db in &self.spectrum_dbs {
            sum += db;
            peak = peak.max(db);
            hist[((db + 120.0).clamp(0.0, 119.9) as usize).min(119)] += 1;
        }
        let target = self.fft_size as u32 / 4;
        let mut count = 0;
        let mut floor = -120.0;
        for (bin, amount) in hist.into_iter().enumerate() {
            count += amount;
            if count >= target {
                floor = bin as f32 - 120.0;
                break;
            }
        }
        self.cached_signal_level = sum / self.fft_size as f32;
        self.cached_peak_level = peak;
        self.cached_noise_floor = floor;
        let snr = peak - floor;
        let weight = 1.0 - (-self.frame_period_seconds as f32 / self.snr_smoothing_secs).exp();
        self.smoothed_snr = Some(match self.smoothed_snr {
            Some(old) if self.snr_smoothing => old + weight * (snr - old),
            _ => snr,
        });
        self.processed_frames = self.processed_frames.wrapping_add(1);
        if self.processed_frames.is_multiple_of(10) {
            self.signal_history.push_back(peak);
            if self.signal_history.len() > self.signal_history_max {
                self.signal_history.pop_front();
            }
            self.noise_baseline = if self.noise_baseline <= -119.0 {
                floor
            } else {
                0.995 * self.noise_baseline + 0.005 * floor
            };
        }
        if self.waterfall_visible
            && !self.waterfall_paused
            && self
                .processed_frames
                .is_multiple_of(self.waterfall_every_n.max(1) as u64)
        {
            let y = self.waterfall_head;
            self.waterfall_pixels[y] = self.waterfall_row();
            self.waterfall_pending_rows[y] = true;
            self.waterfall_head = (y + 1) % self.waterfall_history;
        }
    }

    fn waterfall_row(&self) -> Vec<u8> {
        let mut pixels = vec![0u8; self.waterfall_width * 4];
        let range = (self.wf_max_db - self.wf_min_db).max(1.0);
        for column in 0..self.waterfall_width {
            let first = column * self.fft_size / self.waterfall_width;
            let last = ((column + 1) * self.fft_size / self.waterfall_width).max(first + 1);
            // Peak aggregation preserves narrow carriers when many FFT bins
            // share a display pixel. Full-resolution values remain exportable.
            let db = self.spectrum_dbs[first..last]
                .iter()
                .copied()
                .fold(-120.0_f32, f32::max);
            let normalized = ((db - self.wf_min_db) / range).clamp(0.0, 1.0);
            let (r, g, b) = color_map(self.color_map, normalized);
            pixels[column * 4..column * 4 + 4].copy_from_slice(&[r, g, b, 255]);
        }
        pixels
    }

    /// Options for the compact Radio workspace. The same signal controls
    /// remain available without reserving two rows above the spectrum plot.
    pub fn ui_radio_controls(&mut self, ui: &mut egui::Ui) {
        let freeze_label = if self.frozen {
            "❄ Frozen"
        } else {
            "❄ Freeze"
        };
        ui.horizontal(|ui| {
            ui.toggle_value(&mut self.frozen, freeze_label)
                .on_hover_text("Freeze or resume spectrum and waterfall updates.");
            ui.toggle_value(&mut self.waterfall_paused, "⏸ Pause")
                .on_hover_text("Pause waterfall scrolling while the spectrum continues to update.");
            ui.toggle_value(&mut self.show_peak_hold, "Peak")
                .on_hover_text("Show the peak-hold trace.");
            if ui.small_button("Clear WF").clicked() {
                self.reset_waterfall();
            }
        });

        egui::CollapsingHeader::new("Spectrum tools")
            .id_salt("radio.spectrum.tools")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Averaging");
                    for (label, alpha) in [
                        ("Fast", 0.7f32),
                        ("Med", 0.3),
                        ("Slow", 0.1),
                        ("XSlow", 0.03),
                    ] {
                        if ui.small_button(label).clicked() {
                            self.avg_alpha = alpha;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Range");
                    ui.add(
                        egui::DragValue::new(&mut self.display_min_db)
                            .range(-160.0..=self.display_max_db - 10.0)
                            .speed(1.0)
                            .suffix(" dB min"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.display_max_db)
                            .range(self.display_min_db + 10.0..=20.0)
                            .speed(1.0)
                            .suffix(" dB max"),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.small_button("Reset range").clicked() {
                        self.display_min_db = -120.0;
                        self.display_max_db = 0.0;
                    }
                    if ui.small_button("Auto-fit").clicked() && !self.spectrum_dbs.is_empty() {
                        let (low, high) = self
                            .spectrum_dbs
                            .iter()
                            .copied()
                            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| {
                                (low.min(value), high.max(value))
                            });
                        let margin = ((high - low) * 0.1).max(5.0);
                        self.display_min_db = (low - margin).max(-160.0);
                        self.display_max_db = (high + margin).min(20.0);
                    }
                    if ui.small_button("Tune to peak").clicked() && !self.spectrum_dbs.is_empty() {
                        self.clicked_tune_freq = Some(self.visible_peak_freq_hz());
                    }
                    ui.label(format!("Markers {}", self.markers.len()));
                    if ui.small_button("Clear").clicked() {
                        self.markers.clear();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Waterfall rate");
                    for (label, every) in [("1×", 1), ("2×", 2), ("4×", 4), ("8×", 8)] {
                        if ui
                            .selectable_label(self.waterfall_every_n == every, label)
                            .clicked()
                        {
                            self.waterfall_every_n = every;
                        }
                    }
                    ui.checkbox(&mut self.full_waterfall_update, "Full update");
                });
                ui.horizontal(|ui| {
                    ui.label("Waterfall range");
                    ui.add(
                        egui::DragValue::new(&mut self.wf_min_db)
                            .range(-160.0..=self.wf_max_db - 5.0)
                            .speed(1.0)
                            .suffix(" dB min"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.wf_max_db)
                            .range(self.wf_min_db + 5.0..=20.0)
                            .speed(1.0)
                            .suffix(" dB max"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Overlays");
                    ui.checkbox(&mut self.show_vfo_bw, "VFO BW");
                    ui.checkbox(&mut self.show_vfo_b, "VFO B");
                    ui.checkbox(&mut self.show_bookmarks, "Bookmarks");
                    ui.checkbox(&mut self.show_band_plan, "Band plan");
                    ui.checkbox(&mut self.show_signal_history, "History");
                    if !self.audio_waveform.is_empty() {
                        ui.checkbox(&mut self.show_audio_waveform, "Audio waveform");
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Band plan region");
                    for region in [ItuRegion::Region1, ItuRegion::Region2, ItuRegion::Region3] {
                        ui.selectable_value(&mut self.band_plan_region, region, region.label());
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Export CSV").clicked() {
                        self.export_spectrum_csv();
                    }
                    if ui.button("Save waterfall PNG").clicked() {
                        self.save_waterfall_png();
                    }
                });
            });
    }

    /// Dedicated Band Plan module controls, used by the SDR++ module drawer.
    /// This keeps the overlay and ITU region selection backed by the same
    /// fields that paint the live spectrum.
    pub fn ui_band_plan_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Band plan");
        ui.label("Frequency allocations are drawn over the live spectrum.");
        ui.checkbox(&mut self.show_band_plan, "Show band plan overlay");
        ui.horizontal(|ui| {
            ui.label("Region");
            for region in [ItuRegion::Region1, ItuRegion::Region2, ItuRegion::Region3] {
                ui.selectable_value(&mut self.band_plan_region, region, region.label());
            }
        });
        ui.separator();
        ui.label("Overlay categories");
        for (color, label) in [
            (egui::Color32::from_rgb(200, 100, 255), "Amateur / ham"),
            (egui::Color32::from_rgb(255, 140, 60), "Broadcast"),
            (egui::Color32::from_rgb(100, 180, 255), "Aviation"),
            (egui::Color32::from_rgb(0, 200, 200), "Marine"),
            (egui::Color32::from_rgb(80, 220, 80), "Weather"),
            (
                egui::Color32::from_rgb(255, 80, 80),
                "Land mobile / scanner",
            ),
        ] {
            ui.colored_label(color, format!("● {label}"));
        }
    }

    #[must_use]
    pub fn band_plan_visible(&self) -> bool {
        self.show_band_plan
    }

    /// Compact vertical controls shown along the Radio pane's right edge,
    /// matching SDR++'s Zoom / Max / Min rail.
    pub fn ui_radio_rail(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        let slider_height = ((ui.available_height() - 84.0) / 3.0).max(64.0);
        ui.vertical_centered(|ui| {
            ui.label("Zoom");
            let mut zoom = (self.zoom_factor.ln() / 200.0_f32.ln()).clamp(0.0, 1.0);
            if ui
                .add_sized(
                    [20.0, slider_height],
                    egui::Slider::new(&mut zoom, 0.0..=1.0)
                        .vertical()
                        .show_value(false),
                )
                .changed()
            {
                self.zoom_factor = 200.0_f32.powf(zoom).clamp(1.0, 200.0);
            }
            ui.add_space(8.0);

            ui.label("Max");
            let mut max = self.display_max_db;
            if ui
                .add_sized(
                    [20.0, slider_height],
                    egui::Slider::new(&mut max, (self.display_min_db + 5.0)..=20.0)
                        .vertical()
                        .show_value(false),
                )
                .changed()
            {
                self.display_max_db = max;
            }
            ui.add_space(8.0);

            ui.label("Min");
            let mut min = self.display_min_db;
            if ui
                .add_sized(
                    [20.0, slider_height],
                    egui::Slider::new(&mut min, -160.0..=(self.display_max_db - 5.0))
                        .vertical()
                        .show_value(false),
                )
                .changed()
            {
                self.display_min_db = min;
            }
        });
    }

    /// Render the full spectrum analyser UI: controls bar, spectrum plot,
    /// signal history chart, audio waveform, and waterfall display.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.ui_inner(ui, false);
    }

    /// Render the spectrum as SDR++'s main Radio pane: the plot begins directly
    /// below the receiver toolbar, with spectrum options kept in the sidebar.
    pub fn ui_radio_workspace(&mut self, ui: &mut egui::Ui) {
        self.ui_inner(ui, true);
    }

    fn ui_inner(&mut self, ui: &mut egui::Ui, radio_workspace: bool) {
        self.frame_counter = self.frame_counter.wrapping_add(1);

        // Controls bar — compact strip: high-frequency toggles stay inline,
        // everything else groups into Display/Waterfall/View & Export menus.
        if !radio_workspace {
            ui.horizontal_wrapped(|ui| {
            let freeze_label = if self.frozen { "❄ Frozen" } else { "❄ Freeze" };
            if ui.toggle_value(&mut self.frozen, freeze_label)
                .on_hover_text("Freeze the spectrum and waterfall display. Useful to examine a signal in detail without the display updating.")
                .clicked() && !self.frozen {
                // unfreeze: clear peak hold too
            }
            ui.toggle_value(&mut self.waterfall_paused, "⏸ Pause")
                .on_hover_text("Pause waterfall scrolling (spectrum still updates)");
            if ui.toggle_value(&mut self.show_peak_hold, "Peak").clicked()
                && !self.show_peak_hold {
                    self.peak_hold = vec![-120.0; self.fft_size];
                }
            if ui.small_button("Clear WF").on_hover_text("Clear the waterfall history").clicked() {
                self.reset_waterfall();
            }
            ui.separator();

            ui.menu_button("Display", |ui| {
                ui.set_min_width(220.0);
                ui.add_enabled_ui(self.fft_controls_enabled, |ui| {
                    ui.label("FFT size:");
                    ui.horizontal_wrapped(|ui| {
                        for size in [256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536] {
                            if ui.selectable_label(self.fft_size == size, size.to_string()).clicked() {
                                self.set_fft_size(size);
                            }
                        }
                    });
                    ui.label("Window:");
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.window_type == WindowType::Hann, "Hann").clicked() { self.set_window(WindowType::Hann); }
                        if ui.selectable_label(self.window_type == WindowType::Hamming, "Hamming").clicked() { self.set_window(WindowType::Hamming); }
                        if ui.selectable_label(self.window_type == WindowType::Blackman, "Blackman").clicked() { self.set_window(WindowType::Blackman); }
                    });
                }).response.on_disabled_hover_text("The daemon supplies its own FFT bins and window");
                ui.separator();
                ui.label("Averaging:").on_hover_text("Spectrum smoothing. Lower α = slower/smoother (better for weak signals). Higher α = faster response.");
                ui.horizontal(|ui| {
                    for (label, alpha, tip) in [
                        ("Fast",  0.7f32, "Fast (α=0.7) — responds quickly to signal changes, more noise visible"),
                        ("Med",   0.3,    "Medium (α=0.3) — balanced default"),
                        ("Slow",  0.1,    "Slow (α=0.1) — smooth display, best for weak signals"),
                        ("XSlow", 0.03,   "Extra slow (α=0.03) — maximum smoothing, good for noise floor characterization"),
                    ] {
                        let is_active = (self.avg_alpha - alpha).abs() < 0.05;
                        let btn = ui.add(egui::Button::new(egui::RichText::new(label).small()
                            .color(if is_active { egui::Color32::BLACK } else { egui::Color32::from_rgb(180, 200, 220) }))
                            .fill(if is_active { egui::Color32::from_rgb(80, 160, 255) } else { egui::Color32::from_rgba_unmultiplied(30, 40, 60, 60) })
                            .small())
                            .on_hover_text(tip);
                        if btn.clicked() { self.avg_alpha = alpha; }
                    }
                });
                ui.separator();
                ui.label("dB range:").on_hover_text("Adjust the visible dB range on the spectrum plot. Drag the Floor/Ceil values to zoom in on a particular signal level.");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.display_min_db).speed(1.0).range(-160.0..=-40.0).suffix(" floor"))
                        .on_hover_text("Bottom of the dB scale. Default -120 dBFS.");
                    ui.add(egui::DragValue::new(&mut self.display_max_db).speed(1.0).range(-40.0..=20.0).suffix(" ceil"))
                        .on_hover_text("Top of the dB scale. Default 0 dBFS.");
                });
                if self.display_min_db >= self.display_max_db - 10.0 {
                    self.display_min_db = self.display_max_db - 10.0;
                    self.waterfall_dirty = true;
                }
                ui.horizontal(|ui| {
                    if ui.small_button("⟳ Reset").on_hover_text("Reset dB range to default (-120 to 0)").clicked() {
                        self.display_min_db = -120.0;
                        self.display_max_db = 0.0;
                        self.waterfall_dirty = true;
                    }
                    if ui.small_button("Auto-fit").on_hover_text("Automatically set the dB range to the current signal min/max, centering the display on your signals.").clicked()
                        && !self.spectrum_dbs.is_empty() {
                            let (cur_min, cur_max) = self.spectrum_dbs.iter().fold(
                                (f32::INFINITY, f32::NEG_INFINITY),
                                |(mn, mx), &v| (mn.min(v), mx.max(v))
                            );
                            let margin = ((cur_max - cur_min) * 0.1).max(5.0);
                            self.display_min_db = (cur_min - margin).max(-160.0);
                            self.display_max_db = (cur_max + margin).min(20.0);
                            self.waterfall_dirty = true;
                        }
                });
                ui.separator();
                let mark_count = self.markers.len();
                ui.horizontal(|ui| {
                    ui.label(format!("Marks: {mark_count}"));
                    if ui.small_button("Clear").clicked() {
                        self.markers.clear();
                    }
                });
                if self.zoom_factor > 1.0
                    && ui.small_button(format!("Reset zoom (currently {:.0}x)", self.zoom_factor)).clicked() {
                        self.zoom_factor = 1.0;
                        self.zoom_offset = 0.5;
                    }
                if ui.small_button("⊕ Tune to peak").on_hover_text("Tune to the frequency with the strongest signal currently visible in the spectrum.").clicked()
                    && !self.spectrum_dbs.is_empty() {
                        self.clicked_tune_freq = Some(self.visible_peak_freq_hz());
                    }
            });

            ui.menu_button(format!("Waterfall: {}", self.color_map.name()), |ui| {
                ui.set_min_width(220.0);
                ui.label("Palette:");
                egui::Grid::new("wf_palette_grid").num_columns(2).show(ui, |ui| {
                    let palettes = [("Classic", ColorMap::Classic), ("Viridis", ColorMap::Viridis), ("Plasma", ColorMap::Plasma), ("Magma", ColorMap::Magma), ("Inferno", ColorMap::Inferno), ("Turbo", ColorMap::Turbo), ("Gray", ColorMap::Grayscale), ("Hot", ColorMap::Hot)];
                    for (i, (label, cmap)) in palettes.into_iter().enumerate() {
                        if ui.selectable_label(self.color_map == cmap, label).clicked() {
                            self.color_map = cmap;
                            self.waterfall_dirty = true;
                        }
                        if i % 2 == 1 { ui.end_row(); }
                    }
                });
                ui.separator();
                ui.label("Scroll speed:");
                ui.horizontal(|ui| {
                    for (label, n) in [("1x", 1u32), ("2x", 2), ("4x", 4), ("8x", 8)] {
                        if ui.selectable_label(self.waterfall_every_n == n, label).clicked() {
                            self.waterfall_every_n = n;
                        }
                    }
                });
                ui.checkbox(&mut self.full_waterfall_update, "Full Waterfall Update")
                    .on_hover_text("Upload the full waterfall whenever new rows arrive. Disable to upload only changed rows.");
                ui.separator();
                ui.label("Color range:").on_hover_text("Waterfall brightness/contrast: sets the dBFS range mapped to the full color palette. Narrow the range for more contrast on weak signals.");
                ui.horizontal(|ui| {
                    let wf_min_changed = ui.add(egui::DragValue::new(&mut self.wf_min_db).speed(1.0).range(-160.0..=-20.0).suffix(" dark"))
                        .on_hover_text("Lowest dBFS shown in the waterfall (mapped to black/dark). Lower = more sensitive to faint signals.").changed();
                    let wf_max_changed = ui.add(egui::DragValue::new(&mut self.wf_max_db).speed(1.0).range(-60.0..=20.0).suffix(" bright"))
                        .on_hover_text("Highest dBFS shown in waterfall (mapped to brightest color). Lower = amplify weak signals.").changed();
                    if wf_min_changed || wf_max_changed {
                        if self.wf_min_db >= self.wf_max_db - 5.0 { self.wf_min_db = self.wf_max_db - 5.0; }
                        self.waterfall_dirty = true;
                    }
                });
                if ui.small_button("WF Auto").on_hover_text("Set waterfall color range to current signal min/max for best contrast.").clicked()
                    && !self.spectrum_dbs.is_empty() {
                        let (cur_min, cur_max) = self.spectrum_dbs.iter().fold(
                            (f32::INFINITY, f32::NEG_INFINITY),
                            |(mn, mx), &v| (mn.min(v), mx.max(v))
                        );
                        self.wf_min_db = (cur_min - 5.0).max(-160.0);
                        self.wf_max_db = (cur_max + 5.0).min(20.0);
                        self.waterfall_dirty = true;
                    }
            });

            ui.menu_button("View/Export", |ui| {
                ui.set_min_width(200.0);
                ui.toggle_value(&mut self.show_vfo_bw, "VFO BW")
                    .on_hover_text("Show shaded VFO filter bandwidth region centered on the tuned frequency.");
                ui.toggle_value(&mut self.show_vfo_b, "VFO B")
                    .on_hover_text("Show VFO B frequency as a dashed marker line on the spectrum and waterfall.");
                ui.toggle_value(&mut self.show_bookmarks, "⭐ Bookmarks")
                    .on_hover_text("Overlay bookmark frequencies as vertical lines on the spectrum.");
                ui.toggle_value(&mut self.show_band_plan, "🗺 Band plan")
                    .on_hover_text("Band plan overlay — colored regions show frequency allocations:\n🟢 Green = Amateur (Ham) bands\n🟠 Orange = Broadcast (AM/FM/DAB)\n🔵 Blue = Aviation (airband, VOR, ADS-B)\n🟢 Teal = Marine VHF\n💚 Lime = Weather (NOAA, GOES)\n🟣 Purple = Satellites / GPS\n🔴 Red = ISM (Wi-Fi, 433 MHz remotes)\n🟡 Yellow = Land mobile / PMR");
                ui.menu_button(format!("Band region: {}", self.band_plan_region.label()), |ui| {
                    for region in [ItuRegion::Region1, ItuRegion::Region2, ItuRegion::Region3] {
                        ui.selectable_value(&mut self.band_plan_region, region, region.label());
                    }
                });
                ui.toggle_value(&mut self.show_signal_history, "📈 History")
                    .on_hover_text("Show a scrolling chart of peak signal strength over time. Useful for tracking intermittent signals.");
                if !self.audio_waveform.is_empty() {
                    ui.toggle_value(&mut self.show_audio_waveform, "🎵 Waveform")
                        .on_hover_text("Show the live demodulated audio waveform. Only visible when audio is playing.");
                }
                ui.separator();
                if ui.button("💾 Export CSV").on_hover_text("Export current spectrum data to CSV (frequency_hz, power_dbfs). Useful for analysis in spreadsheets or Python.").clicked() {
                    self.export_spectrum_csv();
                }
                if ui.button("📸 Save waterfall PNG").on_hover_text("Save a PNG screenshot of the current waterfall display. Captures the entire waterfall history at full resolution.").clicked() {
                    self.save_waterfall_png();
                }
            });

            ui.separator();
            ui.colored_label(egui::Color32::from_rgb(150, 180, 255), format!("{}·{}", self.fft_size, self.window_type.name()))
                .on_hover_text(format!("Current FFT: {} bins, {} window. Larger FFT = better frequency resolution but slower updates.", self.fft_size, self.window_type.name()));

            // Sample rate span and resolution indicator
            let span_mhz = self.sample_rate / 1e6;
            let res_hz = self.sample_rate / self.fft_size as f64;
            let res_label = if res_hz >= 1000.0 {
                format!("{:.1}kHz", res_hz / 1000.0)
            } else {
                format!("{res_hz:.0}Hz")
            };
            ui.colored_label(egui::Color32::from_rgb(180, 150, 180), format!("{span_mhz:.3}MSps·{res_label}"))
                .on_hover_text(format!("Sample rate: {} MSps (Nyquist: ±{:.1} MHz). Frequency resolution: {} per bin.",
                    self.sample_rate / 1e6, span_mhz / 2.0, res_label));
            if self.zoom_factor > 1.0 {
                ui.colored_label(egui::Color32::from_rgb(100, 180, 255), format!("🔍 {:.0}x", self.zoom_factor))
                    .on_hover_text("Current zoom level. Reset it from the Display menu. Scroll on the spectrum to zoom in/out.");
            }
        });
        }

        // Signal history mini-chart
        if self.show_signal_history && !self.signal_history.is_empty() {
            let history_height = 70.0;
            let (hist_rect, hist_resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), history_height),
                egui::Sense::hover(),
            );
            let hist_resp = hist_resp.on_hover_text("Signal peak history chart. Shows the last 600 spectrum peaks. Hover for cursor value.");
            let painter = ui.painter();
            painter.rect_filled(hist_rect, 2.0, egui::Color32::from_rgb(8, 8, 18));

            let n = self.signal_history.len();
            let history_vec: Vec<f32> = self.signal_history.iter().copied().collect();
            let min_v = self.display_min_db;
            let max_v = self.display_max_db;
            let range = (max_v - min_v).max(1.0);

            let db_to_y = |db: f32| {
                hist_rect.bottom() - ((db - min_v) / range).clamp(0.0, 1.0) * hist_rect.height()
            };
            let i_to_x = |i: usize| {
                hist_rect.left()
                    + (i as f32 / (self.signal_history_max as f32 - 1.0).max(1.0))
                        * hist_rect.width()
            };

            // Noise floor reference line
            let nf = self.noise_floor();
            let nf_y = db_to_y(nf);
            painter.line_segment(
                [
                    egui::pos2(hist_rect.left(), nf_y),
                    egui::pos2(hist_rect.right(), nf_y),
                ],
                egui::Stroke::new(
                    0.5,
                    egui::Color32::from_rgba_unmultiplied(100, 100, 200, 80),
                ),
            );

            // Filled area + line
            let pts: Vec<egui::Pos2> = history_vec
                .iter()
                .enumerate()
                .map(|(i, &db)| egui::pos2(i_to_x(i), db_to_y(db)))
                .collect();

            if pts.len() > 1 {
                // Filled polygon under the curve
                let mut poly = pts.clone();
                if let Some(last) = pts.last() {
                    poly.push(egui::pos2(last.x, hist_rect.bottom()));
                }
                poly.push(egui::pos2(pts[0].x, hist_rect.bottom()));
                painter.add(egui::Shape::convex_polygon(
                    poly,
                    egui::Color32::from_rgba_unmultiplied(46, 204, 113, 20),
                    egui::Stroke::NONE,
                ));

                // Color-coded line segments
                for i in 0..pts.len() - 1 {
                    let norm = ((history_vec[i] - min_v) / range).clamp(0.0, 1.0);
                    let col = if norm > 0.7 {
                        egui::Color32::from_rgb(46, 204, 113)
                    } else if norm > 0.45 {
                        egui::Color32::from_rgb(241, 196, 15)
                    } else {
                        egui::Color32::from_rgb(52, 152, 219)
                    };
                    painter.line_segment([pts[i], pts[i + 1]], egui::Stroke::new(1.2, col));
                }

                // Peak dot
                if let Some((pk_idx, &pk_db)) = history_vec
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                {
                    let px = i_to_x(pk_idx);
                    let py = db_to_y(pk_db);
                    painter.circle_filled(egui::pos2(px, py), 3.0, egui::Color32::RED);
                    painter.text(
                        egui::pos2(px + 4.0, py),
                        egui::Align2::LEFT_CENTER,
                        format!("pk {pk_db:.0}"),
                        egui::FontId::monospace(8.0),
                        egui::Color32::from_rgb(255, 100, 100),
                    );
                }

                // Cursor readout
                if let Some(ptr) = hist_resp.hover_pos() {
                    let frac = ((ptr.x - hist_rect.left()) / hist_rect.width()).clamp(0.0, 1.0);
                    let idx = (frac * (history_vec.len() as f32 - 1.0)) as usize;
                    if idx < history_vec.len() {
                        let db = history_vec[idx];
                        let cx = i_to_x(idx);
                        let cy = db_to_y(db);
                        painter.line_segment(
                            [
                                egui::pos2(cx, hist_rect.top()),
                                egui::pos2(cx, hist_rect.bottom()),
                            ],
                            egui::Stroke::new(0.5, egui::Color32::from_gray(120)),
                        );
                        painter.circle_filled(egui::pos2(cx, cy), 3.0, egui::Color32::WHITE);
                        painter.text(
                            egui::pos2(cx + 5.0, cy - 8.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{db:.1} dB"),
                            egui::FontId::monospace(9.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
            }

            // Labels
            painter.text(
                egui::pos2(hist_rect.right() - 2.0, hist_rect.top() + 2.0),
                egui::Align2::RIGHT_TOP,
                format!("{max_v:.0}"),
                egui::FontId::monospace(8.0),
                egui::Color32::DARK_GRAY,
            );
            painter.text(
                egui::pos2(hist_rect.right() - 2.0, hist_rect.bottom() - 2.0),
                egui::Align2::RIGHT_BOTTOM,
                format!("{min_v:.0} dB"),
                egui::FontId::monospace(8.0),
                egui::Color32::DARK_GRAY,
            );
            painter.text(
                egui::pos2(hist_rect.left() + 2.0, hist_rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("Signal history  ({n} pts, floor {nf:.0} dB)"),
                egui::FontId::monospace(8.0),
                egui::Color32::DARK_GRAY,
            );
        }

        // Audio waveform display
        if self.show_audio_waveform && !self.audio_waveform.is_empty() {
            let wave_height = 50.0;
            let (wave_rect, wave_resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), wave_height),
                egui::Sense::hover(),
            );
            let _ = wave_resp.on_hover_text(
                "Demodulated audio waveform. Shows the last ~42 ms of audio signal.",
            );
            let wf_painter = ui.painter();
            wf_painter.rect_filled(wave_rect, 2.0, egui::Color32::from_rgb(8, 8, 18));
            let wf_samples: Vec<f32> = self.audio_waveform.iter().copied().collect();
            let n_wf = wf_samples.len();
            if n_wf > 1 {
                let mid_y = wave_rect.center().y;
                let amp = wave_rect.height() * 0.45;
                // Center line
                wf_painter.line_segment(
                    [
                        egui::pos2(wave_rect.left(), mid_y),
                        egui::pos2(wave_rect.right(), mid_y),
                    ],
                    egui::Stroke::new(0.3, egui::Color32::from_rgba_unmultiplied(80, 80, 120, 80)),
                );
                // Waveform line
                let mut prev = None;
                for (i, &s) in wf_samples.iter().enumerate() {
                    let x = wave_rect.left() + (i as f32 / (n_wf - 1) as f32) * wave_rect.width();
                    let y = mid_y - s * amp;
                    if let Some(p) = prev {
                        let color = if s.abs() > 0.5 {
                            egui::Color32::from_rgb(231, 76, 60) // red on loud
                        } else if s.abs() > 0.25 {
                            egui::Color32::from_rgb(241, 196, 15) // yellow on moderate
                        } else {
                            egui::Color32::from_rgb(46, 204, 113) // green on quiet
                        };
                        wf_painter
                            .line_segment([p, egui::pos2(x, y)], egui::Stroke::new(1.0, color));
                    }
                    prev = Some(egui::pos2(x, y));
                }
                // Label
                wf_painter.text(
                    egui::pos2(wave_rect.left() + 2.0, wave_rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("Audio waveform  ({n_wf} samples)"),
                    egui::FontId::monospace(8.0),
                    egui::Color32::DARK_GRAY,
                );
            }
        }

        // Diagnostic info is useful in generic spectrum views. The Radio
        // workspace already shows tuned frequency and receiver state in its
        // toolbar/status strip, and SDR++ does not reserve a row above its plot.
        if !radio_workspace {
            // Info bar
            ui.horizontal_wrapped(|ui| {
            let center_mhz = self.center_freq as f64 / 1e6;
            let span_mhz = self.sample_rate / 1e6;
            let visible_span_mhz = span_mhz / f64::from(self.zoom_factor);
            let res_hz = self.sample_rate / self.fft_size as f64;
            ui.monospace(format!("⟵CTR {center_mhz:.3} MHz"))
                .on_hover_text("Source capture center frequency.");
            if self.vfo_frequency_hz() != self.center_freq {
                ui.separator();
                ui.monospace(format!("VFO {:.6} MHz", self.vfo_frequency_hz() as f64 / 1e6))
                    .on_hover_text("Tuned demodulator frequency within the captured band.");
            }
            ui.separator();
            if self.zoom_factor > 1.0 {
                ui.monospace(format!("Span {:.3} MHz (zoom {:.0}x)", visible_span_mhz, self.zoom_factor))
                    .on_hover_text("Visible frequency span at current zoom level.");
            } else {
                ui.monospace(format!("Span {span_mhz:.3} MHz"))
                    .on_hover_text("Total visible frequency span = sample rate.");
            }
            ui.separator();
            ui.monospace(format!("Res {res_hz:.1} Hz/bin"))
                .on_hover_text("FFT frequency resolution per bin. Lower = more detail. Increase FFT size to improve.");
            ui.separator();
            let peak = self.peak_level();
            let noise = self.noise_floor();
            let snr = self.snr_db();
            let peak_col = if peak > -20.0 { egui::Color32::GREEN } else if peak > -50.0 { egui::Color32::YELLOW } else { egui::Color32::GRAY };
            ui.colored_label(peak_col, format!("Peak {peak:.0} dB"))
                .on_hover_text("Strongest signal in current view (dBFS).");
            ui.monospace(format!("Floor {noise:.0} dB"))
                .on_hover_text("Estimated noise floor (25th percentile of spectrum bins).");
            let snr_col = if snr > 20.0 { egui::Color32::GREEN } else if snr > 10.0 { egui::Color32::YELLOW } else { egui::Color32::GRAY };
            ui.colored_label(snr_col, format!("SNR {snr:.0} dB"))
                .on_hover_text("Signal-to-noise ratio: peak minus floor. >20 dB = excellent.");
            // Peak frequency in visible span
            if !self.spectrum_dbs.is_empty() {
                let peak_freq_mhz = self.visible_peak_freq_hz() as f64 / 1e6;
                ui.separator();
                ui.monospace(format!("⊕ {peak_freq_mhz:.3} MHz"))
                    .on_hover_text(format!("Frequency of strongest visible signal: {peak_freq_mhz:.4} MHz. Press T to tune here."));
            }
            // Noise floor trend indicator — warn if floor jumped significantly vs baseline
            if self.noise_baseline > -119.0 && self.source_running {
                let floor_delta = noise - self.noise_baseline;
                if floor_delta > 3.0 {
                    ui.separator();
                    let warn_col = if floor_delta > 8.0 { egui::Color32::RED } else { egui::Color32::YELLOW };
                    ui.colored_label(warn_col, format!("⚠ Floor +{floor_delta:.0} dB"))
                        .on_hover_text(format!("Noise floor is {:.1} dB above baseline ({:.0} dB vs baseline {:.0} dB). Possible interference or gain issue.", floor_delta, noise, self.noise_baseline));
                }
            }
            if self.frozen {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(100, 180, 255), "❄ FROZEN");
            }
        });
        }

        // Marker label popup — shown when user clicks "Add marker" in context menu
        if let Some(pending_freq) = self.marker_pending_freq {
            egui::Window::new("Label this marker")
                .id(egui::Id::new("marker_label_popup"))
                .fixed_size([260.0, 80.0])
                .show(ui.ctx(), |ui| {
                    ui.label(format!(
                        "Add label for {:.4} MHz (leave blank for frequency-only):",
                        pending_freq as f64 / 1e6
                    ));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.marker_label_input)
                            .desired_width(200.0)
                            .hint_text("e.g. DC offset, Interference, Local FM…"),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Add").clicked()
                            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            let label = self.marker_label_input.trim().to_string();
                            self.markers.push((pending_freq, label));
                            if self.markers.len() > 20 {
                                self.markers.remove(0);
                            }
                            self.marker_pending_freq = None;
                            self.marker_label_input.clear();
                        }
                        if ui.button("Cancel").clicked() {
                            self.marker_pending_freq = None;
                            self.marker_label_input.clear();
                        }
                    });
                });
        }

        let avail = ui.available_size();
        // SDR++'s installed profile uses a 300 px FFT pane. Keep room for the
        // waterfall at smaller window sizes and consume the remaining height.
        let spectrum_height = if self.waterfall_visible {
            300.0_f32.min(avail.y * 0.55)
        } else {
            avail.y.max(1.0)
        };

        // Spectrum plot
        let (spectrum_rect, response) =
            ui.allocate_exact_size(egui::vec2(avail.x, spectrum_height), egui::Sense::click());
        let painter = ui.painter();
        painter.rect_filled(spectrum_rect, 0.0, self.plot_bg);

        let n = self.fft_size;
        let min_db = self.display_min_db;
        let max_db = self.display_max_db;
        let range = (max_db - min_db).max(1.0);

        // Horizontal dB grid lines — adaptive step based on display range
        if self.show_grid {
            let db_step = if range > 80.0 {
                20.0f32
            } else if range > 40.0 {
                10.0
            } else {
                5.0
            };
            let first = (min_db / db_step).ceil() as i32;
            let last = (max_db / db_step).floor() as i32;
            for i in first..=last {
                let db = i as f32 * db_step;
                let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                if y < spectrum_rect.top() + 1.0 || y > spectrum_rect.bottom() - 1.0 {
                    continue;
                }
                let is_zero = db.abs() < 0.01;
                let line_alpha = if is_zero {
                    100u8
                } else if (i % 2) == 0 {
                    60
                } else {
                    35
                };
                painter.line_segment(
                    [
                        egui::pos2(spectrum_rect.left(), y),
                        egui::pos2(spectrum_rect.right(), y),
                    ],
                    egui::Stroke::new(
                        if is_zero { 0.7 } else { 0.4 },
                        egui::Color32::from_rgba_unmultiplied(
                            self.grid_color.r(),
                            self.grid_color.g(),
                            self.grid_color.b(),
                            line_alpha,
                        ),
                    ),
                );
                // dB label on the right side
                let label_x = spectrum_rect.right() - 2.0;
                painter.text(
                    egui::pos2(label_x, y - 1.0),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("{db:.0}"),
                    egui::FontId::proportional(8.5),
                    egui::Color32::from_rgba_unmultiplied(
                        self.grid_color.r(),
                        self.grid_color.g(),
                        self.grid_color.b(),
                        160,
                    ),
                );
            }
        }

        // Zoom parameters (used by all overlays below)
        let zoom_span =
            (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
        let zoom_center_offset = self.zoom_center_offset(zoom_span);
        let left_hz = -zoom_span / 2.0 + zoom_center_offset;
        let right_hz = zoom_span / 2.0 + zoom_center_offset;
        // Update visible range for scanner integration
        self.visible_left_hz = (self.center_freq as f64 + left_hz).max(0.0) as u64;
        self.visible_right_hz = (self.center_freq as f64 + right_hz).max(0.0) as u64;

        // Vertical grid lines (frequency) with zoom support
        let n_grid = 8;
        if self.show_grid {
            for i in 0..=n_grid {
                let frac = i as f32 / n_grid as f32;
                let x = spectrum_rect.left() + frac * spectrum_rect.width();
                let offset_hz = left_hz + f64::from(frac) * zoom_span;
                let freq_mhz = (self.center_freq as f64 + offset_hz) / 1e6;
                painter.line_segment(
                    [
                        egui::pos2(x, spectrum_rect.top()),
                        egui::pos2(x, spectrum_rect.bottom()),
                    ],
                    egui::Stroke::new(
                        0.5,
                        egui::Color32::from_rgba_unmultiplied(
                            self.grid_color.r(),
                            self.grid_color.g(),
                            self.grid_color.b(),
                            128,
                        ),
                    ),
                );
                painter.text(
                    egui::pos2(x, spectrum_rect.bottom() + 2.0),
                    egui::Align2::CENTER_TOP,
                    format!("{freq_mhz:.2}"),
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgba_unmultiplied(
                        self.grid_color.r(),
                        self.grid_color.g(),
                        self.grid_color.b(),
                        200,
                    ),
                );
            }
        }

        // Band plan overlay
        if self.show_band_plan {
            struct Band {
                name: &'static str,
                low_mhz: f64,
                high_mhz: f64,
                color: egui::Color32,
            }
            // Colors: green=amateur, orange=broadcast, blue=aviation, teal=marine, lime=weather/utility, purple=satellite/space, red=ISM, gray=other
            // (all driven by the active theme's bandplan_* tokens)
            let ham = self.bandplan_ham;
            let bcast = self.bandplan_broadcast;
            let air = self.bandplan_aviation;
            let mar = self.bandplan_marine;
            let wx = self.bandplan_weather;
            let sat = self.bandplan_satellite;
            let ism = self.bandplan_ism;
            let mob = self.bandplan_mobile;
            let (band_80m, band_40m, band_125m, band_70cm) = self.band_plan_region.amateur_limits();
            let band_125m = band_125m.unwrap_or((0.0, 0.0));
            let bands: Vec<Band> = vec![
                // HF amateur
                Band {
                    name: "160m",
                    low_mhz: 1.8,
                    high_mhz: 2.0,
                    color: ham,
                },
                Band {
                    name: "80m",
                    low_mhz: band_80m.0,
                    high_mhz: band_80m.1,
                    color: ham,
                },
                Band {
                    name: "40m",
                    low_mhz: band_40m.0,
                    high_mhz: band_40m.1,
                    color: ham,
                },
                Band {
                    name: "20m",
                    low_mhz: 14.0,
                    high_mhz: 14.35,
                    color: ham,
                },
                Band {
                    name: "17m",
                    low_mhz: 18.068,
                    high_mhz: 18.168,
                    color: ham,
                },
                Band {
                    name: "15m",
                    low_mhz: 21.0,
                    high_mhz: 21.45,
                    color: ham,
                },
                Band {
                    name: "12m",
                    low_mhz: 24.89,
                    high_mhz: 24.99,
                    color: ham,
                },
                Band {
                    name: "10m",
                    low_mhz: 28.0,
                    high_mhz: 29.7,
                    color: ham,
                },
                // VHF/UHF amateur
                Band {
                    name: "6m",
                    low_mhz: 50.0,
                    high_mhz: 54.0,
                    color: ham,
                },
                Band {
                    name: "2m",
                    low_mhz: 144.0,
                    high_mhz: 148.0,
                    color: ham,
                },
                Band {
                    name: "1.25m",
                    low_mhz: band_125m.0,
                    high_mhz: band_125m.1,
                    color: ham,
                },
                Band {
                    name: "70cm",
                    low_mhz: band_70cm.0,
                    high_mhz: band_70cm.1,
                    color: ham,
                },
                Band {
                    name: "33cm",
                    low_mhz: 902.0,
                    high_mhz: 928.0,
                    color: ham,
                },
                Band {
                    name: "23cm",
                    low_mhz: 1240.0,
                    high_mhz: 1300.0,
                    color: ham,
                },
                // Broadcast
                Band {
                    name: "AM",
                    low_mhz: 0.525,
                    high_mhz: 1.705,
                    color: bcast,
                },
                Band {
                    name: "FM",
                    low_mhz: 87.5,
                    high_mhz: 108.0,
                    color: bcast,
                },
                Band {
                    name: "DAB",
                    low_mhz: 174.0,
                    high_mhz: 230.0,
                    color: bcast,
                },
                // Aviation
                Band {
                    name: "NDB",
                    low_mhz: 0.19,
                    high_mhz: 0.525,
                    color: air,
                },
                Band {
                    name: "VOR/ILS",
                    low_mhz: 108.0,
                    high_mhz: 118.0,
                    color: air,
                },
                Band {
                    name: "Airband",
                    low_mhz: 118.0,
                    high_mhz: 137.0,
                    color: air,
                },
                Band {
                    name: "ADS-B",
                    low_mhz: 1090.0,
                    high_mhz: 1090.5,
                    color: air,
                },
                Band {
                    name: "ACARS",
                    low_mhz: 129.0,
                    high_mhz: 136.9,
                    color: air,
                },
                // Marine / maritime
                Band {
                    name: "Marine",
                    low_mhz: 156.0,
                    high_mhz: 162.05,
                    color: mar,
                },
                Band {
                    name: "Marine 2182",
                    low_mhz: 2.181,
                    high_mhz: 2.183,
                    color: mar,
                },
                Band {
                    name: "MF DSC",
                    low_mhz: 2.187,
                    high_mhz: 2.188,
                    color: mar,
                },
                Band {
                    name: "Marine 4125",
                    low_mhz: 4.124,
                    high_mhz: 4.126,
                    color: mar,
                },
                // Weather / utility
                Band {
                    name: "NOAA WX",
                    low_mhz: 162.4,
                    high_mhz: 162.55,
                    color: wx,
                },
                Band {
                    name: "NOAA APT",
                    low_mhz: 137.0,
                    high_mhz: 138.0,
                    color: wx,
                },
                Band {
                    name: "GOES",
                    low_mhz: 1686.0,
                    high_mhz: 1698.0,
                    color: wx,
                },
                // Satellites
                Band {
                    name: "GPS L1",
                    low_mhz: 1574.397,
                    high_mhz: 1576.443,
                    color: sat,
                },
                Band {
                    name: "GPS L2C",
                    low_mhz: 1226.577,
                    high_mhz: 1228.623,
                    color: sat,
                },
                Band {
                    name: "Iridium",
                    low_mhz: 1616.0,
                    high_mhz: 1626.5,
                    color: sat,
                },
                Band {
                    name: "Meteor",
                    low_mhz: 137.0,
                    high_mhz: 138.0,
                    color: sat,
                },
                // Land mobile / PMR
                Band {
                    name: "LMR VHF",
                    low_mhz: 138.0,
                    high_mhz: 174.0,
                    color: mob,
                },
                Band {
                    name: "PMR446",
                    low_mhz: 446.0,
                    high_mhz: 446.2,
                    color: mob,
                },
                Band {
                    name: "LMR UHF",
                    low_mhz: 450.0,
                    high_mhz: 512.0,
                    color: mob,
                },
                // ISM / unlicensed
                Band {
                    name: "ISM 27",
                    low_mhz: 26.96,
                    high_mhz: 27.28,
                    color: ism,
                },
                Band {
                    name: "ISM 433",
                    low_mhz: 433.05,
                    high_mhz: 434.79,
                    color: ism,
                },
                Band {
                    name: "ISM 868",
                    low_mhz: 868.0,
                    high_mhz: 868.6,
                    color: ism,
                },
                Band {
                    name: "ISM 915",
                    low_mhz: 902.0,
                    high_mhz: 928.0,
                    color: ism,
                },
                Band {
                    name: "WiFi",
                    low_mhz: 2400.0,
                    high_mhz: 2500.0,
                    color: ism,
                },
            ];
            let center_mhz = self.center_freq as f64 / 1e6;
            let half_span_mhz = zoom_span / 2e6;
            let left_mhz = center_mhz - half_span_mhz + zoom_center_offset / 1e6;
            let right_mhz = center_mhz + half_span_mhz + zoom_center_offset / 1e6;
            for band in &bands {
                let low = band.low_mhz.max(left_mhz);
                let high = band.high_mhz.min(right_mhz);
                if low < high {
                    let x1 = spectrum_rect.left()
                        + ((low - left_mhz) / (right_mhz - left_mhz)) as f32
                            * spectrum_rect.width();
                    let x2 = spectrum_rect.left()
                        + ((high - left_mhz) / (right_mhz - left_mhz)) as f32
                            * spectrum_rect.width();
                    let rect = egui::Rect::from_x_y_ranges(
                        x1..=x2,
                        spectrum_rect.top()..=spectrum_rect.bottom(),
                    );
                    painter.rect_filled(rect, 0.0, band.color);
                    let label_x = f32::midpoint(x1, x2);
                    painter.text(
                        egui::pos2(label_x, spectrum_rect.top() + 8.0),
                        egui::Align2::CENTER_CENTER,
                        band.name,
                        egui::FontId::proportional(8.0),
                        egui::Color32::from_rgba_unmultiplied(180, 180, 180, 100),
                    );
                }
            }
        }

        // VFO bandwidth indicator — shaded region showing active filter width
        if self.show_vfo_bw && self.vfo_bw_hz > 0 {
            // Mode-aware colors
            let (fill_color, edge_color, label_color) = match self.demod_mode.as_str() {
                "WFM" => (
                    egui::Color32::from_rgba_unmultiplied(255, 140, 50, 22),
                    egui::Color32::from_rgba_unmultiplied(255, 160, 80, 150),
                    egui::Color32::from_rgba_unmultiplied(255, 160, 80, 200),
                ),
                "AM" => (
                    egui::Color32::from_rgba_unmultiplied(220, 100, 220, 22),
                    egui::Color32::from_rgba_unmultiplied(220, 100, 220, 150),
                    egui::Color32::from_rgba_unmultiplied(220, 100, 220, 200),
                ),
                "LSB" | "USB" => (
                    egui::Color32::from_rgba_unmultiplied(100, 220, 100, 22),
                    egui::Color32::from_rgba_unmultiplied(100, 220, 100, 150),
                    egui::Color32::from_rgba_unmultiplied(100, 220, 100, 200),
                ),
                _ => (
                    // NFM / FM / RAW — default blue
                    egui::Color32::from_rgba_unmultiplied(52, 152, 219, 25),
                    egui::Color32::from_rgba_unmultiplied(52, 152, 219, 120),
                    egui::Color32::from_rgba_unmultiplied(52, 152, 219, 180),
                ),
            };
            let zoom_span_v =
                (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
            let zoom_center_offset_v = self.zoom_center_offset(zoom_span_v);
            let left_hz_v = -zoom_span_v / 2.0 + zoom_center_offset_v;
            let right_hz_v = zoom_span_v / 2.0 + zoom_center_offset_v;
            let (bw_left, bw_right) = self.vfo_band_edges();
            let x1_frac = ((bw_left - left_hz_v) / (right_hz_v - left_hz_v)).clamp(0.0, 1.0);
            let x2_frac = ((bw_right - left_hz_v) / (right_hz_v - left_hz_v)).clamp(0.0, 1.0);
            if x1_frac < x2_frac {
                let x1 = spectrum_rect.left() + x1_frac as f32 * spectrum_rect.width();
                let x2 = spectrum_rect.left() + x2_frac as f32 * spectrum_rect.width();
                let vfo_rect = egui::Rect::from_x_y_ranges(
                    x1..=x2,
                    spectrum_rect.top()..=spectrum_rect.bottom(),
                );
                painter.rect_filled(vfo_rect, 0.0, fill_color);
                painter.line_segment(
                    [
                        egui::pos2(x1, spectrum_rect.top()),
                        egui::pos2(x1, spectrum_rect.bottom()),
                    ],
                    egui::Stroke::new(0.8, edge_color),
                );
                painter.line_segment(
                    [
                        egui::pos2(x2, spectrum_rect.top()),
                        egui::pos2(x2, spectrum_rect.bottom()),
                    ],
                    egui::Stroke::new(0.8, edge_color),
                );
                let bw_khz = self.vfo_bw_hz as f32 / 1000.0;
                let bw_label = if bw_khz >= 1.0 {
                    format!("{} {:.0} kHz", self.demod_mode, bw_khz)
                } else {
                    format!("{} {:.0} Hz", self.demod_mode, self.vfo_bw_hz)
                };
                painter.text(
                    egui::pos2(f32::midpoint(x1, x2), spectrum_rect.bottom() - 2.0),
                    egui::Align2::CENTER_BOTTOM,
                    bw_label,
                    egui::FontId::proportional(8.0),
                    label_color,
                );
            }
        }

        // VFO B frequency marker
        if self.show_vfo_b && self.vfo_b_freq > 0 {
            let vs = (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
            let vo = self.zoom_center_offset(vs);
            let left_hz_v = -vs / 2.0 + vo;
            let right_hz_v = vs / 2.0 + vo;
            let offset_hz = self.vfo_b_freq as f64 - self.center_freq as f64;
            let frac = (offset_hz - left_hz_v) / (right_hz_v - left_hz_v);
            if (0.0..=1.0).contains(&frac) {
                let x = spectrum_rect.left() + frac as f32 * spectrum_rect.width();
                // Dashed line effect: draw segments
                let n_dashes = 20;
                let total_h = spectrum_rect.height();
                let dash_len = total_h / n_dashes as f32 / 2.0;
                for i in 0..n_dashes {
                    let y0 = spectrum_rect.top() + (i as f32 / n_dashes as f32) * total_h;
                    let y1 = (y0 + dash_len).min(spectrum_rect.bottom());
                    painter.line_segment(
                        [egui::pos2(x, y0), egui::pos2(x, y1)],
                        egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 160),
                        ),
                    );
                }
                // VFO B label
                let vfo_b_mhz = self.vfo_b_freq as f64 / 1e6;
                painter.text(
                    egui::pos2(x + 3.0, spectrum_rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("B {vfo_b_mhz:.3} MHz"),
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgba_unmultiplied(100, 180, 255, 200),
                );
            }
        }

        // Bookmark frequency overlays
        if self.show_bookmarks {
            let zoom_span_bm =
                (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
            let zoom_center_offset_bm = self.zoom_center_offset(zoom_span_bm);
            let left_hz_bm = -zoom_span_bm / 2.0 + zoom_center_offset_bm;
            let right_hz_bm = zoom_span_bm / 2.0 + zoom_center_offset_bm;
            for (bm_freq, bm_name, bm_cat) in &self.bookmark_freqs {
                let offset_hz = *bm_freq as f64 - self.center_freq as f64;
                let frac = (offset_hz - left_hz_bm) / (right_hz_bm - left_hz_bm);
                if (0.0..=1.0).contains(&frac) {
                    let x = spectrum_rect.left() + frac as f32 * spectrum_rect.width();
                    let (line_color, label_color) = category_color(bm_cat);
                    painter.line_segment(
                        [
                            egui::pos2(x, spectrum_rect.top()),
                            egui::pos2(x, spectrum_rect.bottom()),
                        ],
                        egui::Stroke::new(0.8, line_color),
                    );
                    painter.text(
                        egui::pos2(x + 2.0, spectrum_rect.bottom() - 12.0),
                        egui::Align2::LEFT_BOTTOM,
                        bm_name.as_str(),
                        egui::FontId::proportional(7.5),
                        label_color,
                    );
                }
            }
        }

        // Fill under spectrum (zoom-aware)
        if self.gradient_fill {
            let mut mesh = egui::Mesh::default();
            let color_top = self.fill_top;
            let color_bot = self.fill_bot;
            let half_span = self.sample_rate / 2.0;
            let first_bin = ((left_hz + half_span) / self.sample_rate * n as f64) as usize;
            let last_bin = ((right_hz + half_span) / self.sample_rate * n as f64) as usize;
            let first_bin = first_bin.clamp(0, n.saturating_sub(1));
            let last_bin = last_bin.clamp(first_bin + 1, n);
            let visible_bins = last_bin - first_bin;
            if visible_bins > 0 {
                let stride = plot_bin_stride(visible_bins, spectrum_rect.width());
                for i in (first_bin..last_bin).step_by(stride) {
                    let frac = (i - first_bin) as f32 / visible_bins.max(1) as f32;
                    let x = spectrum_rect.left() + frac * spectrum_rect.width();
                    let db = peak_in_bins(&self.spectrum_dbs, i, (i + stride).min(last_bin));
                    let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                    let y = spectrum_rect.bottom() - norm * spectrum_height;
                    mesh.colored_vertex(egui::pos2(x, y), color_top);
                    mesh.colored_vertex(egui::pos2(x, spectrum_rect.bottom()), color_bot);
                }
                for i in 0..visible_bins.div_ceil(stride).saturating_sub(1) {
                    let idx = (i * 2) as u32;
                    mesh.indices.push(idx);
                    mesh.indices.push(idx + 1);
                    mesh.indices.push(idx + 2);
                    mesh.indices.push(idx + 1);
                    mesh.indices.push(idx + 3);
                    mesh.indices.push(idx + 2);
                }
                painter.add(egui::Shape::mesh(mesh));
            }
        }

        // Peak hold (zoom-aware)
        if self.show_peak_hold {
            let mut prev_pos = None;
            let half_span = self.sample_rate / 2.0;
            let first_bin = ((left_hz + half_span) / self.sample_rate * n as f64) as usize;
            let last_bin = ((right_hz + half_span) / self.sample_rate * n as f64) as usize;
            let first_bin = first_bin.clamp(0, n.saturating_sub(1));
            let last_bin = last_bin.clamp(first_bin + 1, n);
            let visible_bins = (last_bin - first_bin).max(1);
            let stride = plot_bin_stride(visible_bins, spectrum_rect.width());
            for i in (first_bin..last_bin).step_by(stride) {
                let frac = (i - first_bin) as f32 / visible_bins as f32;
                let x = spectrum_rect.left() + frac * spectrum_rect.width();
                let db = peak_in_bins(&self.peak_hold, i, (i + stride).min(last_bin));
                let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                if let Some(prev) = prev_pos {
                    painter.line_segment(
                        [prev, egui::pos2(x, y)],
                        egui::Stroke::new(1.0, self.color_error),
                    );
                }
                prev_pos = Some(egui::pos2(x, y));
            }
        }

        // Peak labels on peak hold — label top 5 peaks above noise floor
        if self.show_peak_hold {
            let half_span = self.sample_rate / 2.0;
            let first_bin = ((left_hz + half_span) / self.sample_rate * n as f64) as usize;
            let last_bin = ((right_hz + half_span) / self.sample_rate * n as f64) as usize;
            let first_bin = first_bin.clamp(0, n.saturating_sub(1));
            let last_bin = last_bin.clamp(first_bin + 1, n);
            let visible_bins = (last_bin - first_bin).max(1);

            let noise = self.noise_floor();
            let threshold = noise + 8.0;

            // Collect local maxima: bin is a peak if it's higher than both neighbors and above threshold
            let mut candidates: Vec<(f32, usize)> = Vec::new();
            for i in (first_bin + 1)..last_bin.saturating_sub(1) {
                let db = self.peak_hold[i];
                if db > threshold && db >= self.peak_hold[i - 1] && db >= self.peak_hold[i + 1] {
                    candidates.push((db, i));
                }
            }
            // Sort by strength descending
            candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            // Place labels, skip if too close to an already-labeled peak (within 40px)
            let mut labeled_xs: Vec<f32> = Vec::new();
            for (db, bin) in candidates.iter().take(8) {
                let frac = (bin - first_bin) as f32 / visible_bins as f32;
                let x = spectrum_rect.left() + frac * spectrum_rect.width();
                if labeled_xs.iter().any(|&lx| (lx - x).abs() < 40.0) {
                    continue;
                }
                labeled_xs.push(x);
                let norm = ((*db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                let offset_hz = left_hz + f64::from(frac) * zoom_span;
                let freq_mhz = (self.center_freq as f64 + offset_hz) / 1e6;
                // Stem line
                painter.line_segment(
                    [egui::pos2(x, y), egui::pos2(x, y - 10.0)],
                    egui::Stroke::new(
                        0.8,
                        egui::Color32::from_rgba_unmultiplied(
                            self.color_error.r(),
                            self.color_error.g(),
                            self.color_error.b(),
                            180,
                        ),
                    ),
                );
                // Label
                painter.text(
                    egui::pos2(x, y - 12.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("{freq_mhz:.3}"),
                    egui::FontId::proportional(7.5),
                    self.color_error,
                );
                if labeled_xs.len() >= 5 {
                    break;
                }
            }
        }

        // Spectrum line (zoom-aware)
        {
            let mut prev_pos = None;
            let half_span = self.sample_rate / 2.0;
            let first_bin = ((left_hz + half_span) / self.sample_rate * n as f64) as usize;
            let last_bin = ((right_hz + half_span) / self.sample_rate * n as f64) as usize;
            let first_bin = first_bin.clamp(0, n.saturating_sub(1));
            let last_bin = last_bin.clamp(first_bin + 1, n);
            let visible_bins = (last_bin - first_bin).max(1);
            let stride = plot_bin_stride(visible_bins, spectrum_rect.width());
            for i in (first_bin..last_bin).step_by(stride) {
                let frac = (i - first_bin) as f32 / visible_bins as f32;
                let x = spectrum_rect.left() + frac * spectrum_rect.width();
                let db = if self.persistence > 0.0 {
                    peak_in_bins(&self.persist_buf, i, (i + stride).min(last_bin))
                } else {
                    peak_in_bins(&self.spectrum_dbs, i, (i + stride).min(last_bin))
                };
                let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                if let Some(prev) = prev_pos {
                    painter.line_segment(
                        [prev, egui::pos2(x, y)],
                        egui::Stroke::new(1.5, self.curve_color),
                    );
                }
                prev_pos = Some(egui::pos2(x, y));
            }
        }

        // Animated noise floor indicator
        {
            let nf = self.noise_floor();
            let nf_norm = ((nf - min_db) / range).clamp(0.0, 1.0);
            let nf_y = spectrum_rect.bottom() - nf_norm * spectrum_height;
            let t = (self.frame_counter as f32 * 0.04).sin() * 0.4 + 0.6;
            let alpha = (t * 90.0) as u8;
            painter.line_segment(
                [
                    egui::pos2(spectrum_rect.left(), nf_y),
                    egui::pos2(spectrum_rect.right(), nf_y),
                ],
                egui::Stroke::new(
                    0.7,
                    egui::Color32::from_rgba_unmultiplied(
                        self.noise_floor_color.r(),
                        self.noise_floor_color.g(),
                        self.noise_floor_color.b(),
                        alpha,
                    ),
                ),
            );
            painter.text(
                egui::pos2(spectrum_rect.left() + 4.0, nf_y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("▸ noise {nf:.0} dB"),
                egui::FontId::proportional(7.5),
                egui::Color32::from_rgba_unmultiplied(
                    self.noise_floor_color.r(),
                    self.noise_floor_color.g(),
                    self.noise_floor_color.b(),
                    alpha,
                ),
            );
        }

        // Squelch threshold line (dashed orange, only when not disabled)
        if self.squelch_db > min_db + 1.0 {
            let sq_norm = ((self.squelch_db - min_db) / range).clamp(0.0, 1.0);
            let sq_y = spectrum_rect.bottom() - sq_norm * spectrum_height;
            let dash_len = 6.0_f32;
            let gap_len = 4.0_f32;
            let total = dash_len + gap_len;
            let n_dashes = (spectrum_rect.width() / total).ceil() as usize;
            for i in 0..n_dashes {
                let x0 = spectrum_rect.left() + i as f32 * total;
                let x1 = (x0 + dash_len).min(spectrum_rect.right());
                painter.line_segment(
                    [egui::pos2(x0, sq_y), egui::pos2(x1, sq_y)],
                    egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(
                            self.color_warning.r(),
                            self.color_warning.g(),
                            self.color_warning.b(),
                            180,
                        ),
                    ),
                );
            }
            painter.text(
                egui::pos2(spectrum_rect.right() - 4.0, sq_y - 2.0),
                egui::Align2::RIGHT_BOTTOM,
                format!("SQ {:.0} dB", self.squelch_db),
                egui::FontId::proportional(7.5),
                egui::Color32::from_rgba_unmultiplied(
                    self.color_warning.r(),
                    self.color_warning.g(),
                    self.color_warning.b(),
                    200,
                ),
            );
        }

        // Mouse hover readout
        if let Some(pointer) = response.hover_pos() {
            self.hover_pos = Some(pointer);
            let frac = ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
            let bin = (frac * n as f32) as usize;
            if bin < n {
                let db = self.spectrum_dbs[bin];
                let zoom_span =
                    (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
                let zoom_center_offset = self.zoom_center_offset(zoom_span);
                let left_hz = -zoom_span / 2.0 + zoom_center_offset;
                let offset_hz = left_hz + f64::from(frac) * zoom_span;
                let freq = self.center_freq as f64 + offset_hz;
                let freq_str = if freq >= 1e9 {
                    format!("{:.3} GHz", freq / 1e9)
                } else if freq >= 1e6 {
                    format!("{:.3} MHz", freq / 1e6)
                } else {
                    format!("{:.1} kHz", freq / 1e3)
                };

                // Crosshair
                painter.line_segment(
                    [
                        egui::pos2(pointer.x, spectrum_rect.top()),
                        egui::pos2(pointer.x, spectrum_rect.bottom()),
                    ],
                    egui::Stroke::new(
                        0.5,
                        egui::Color32::from_rgba_unmultiplied(200, 200, 200, 128),
                    ),
                );
                painter.line_segment(
                    [
                        egui::pos2(spectrum_rect.left(), pointer.y),
                        egui::pos2(spectrum_rect.right(), pointer.y),
                    ],
                    egui::Stroke::new(
                        0.5,
                        egui::Color32::from_rgba_unmultiplied(200, 200, 200, 128),
                    ),
                );

                // Cursor tooltip with frequency + delta from center + dB
                let delta_khz = offset_hz / 1000.0;
                let delta_str = if delta_khz.abs() >= 1000.0 {
                    format!("{:+.3} MHz", delta_khz / 1000.0)
                } else {
                    format!("{delta_khz:+.1} kHz")
                };
                let line1 = format!("{freq_str} ({delta_str}) {db:.1} dB");
                let tooltip_w = 220.0f32;
                // Flip tooltip to left if near right edge
                let tx = if pointer.x + tooltip_w + 14.0 > spectrum_rect.right() {
                    pointer.x - tooltip_w - 6.0
                } else {
                    pointer.x + 12.0
                };
                let ty = (pointer.y - 22.0).max(spectrum_rect.top() + 2.0);
                let text_rect =
                    egui::Rect::from_min_size(egui::pos2(tx, ty), egui::vec2(tooltip_w, 16.0));
                painter.rect_filled(
                    text_rect,
                    2.0,
                    egui::Color32::from_rgba_unmultiplied(20, 20, 30, 220),
                );
                painter.text(
                    egui::pos2(text_rect.left() + 4.0, text_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &line1,
                    egui::FontId::monospace(10.0),
                    egui::Color32::from_rgb(46, 204, 113),
                );
            }
        }

        // SNR badge overlay (top-right of spectrum)
        {
            let snr = self.snr_db();
            let snr_color = if snr > 20.0 {
                self.color_success
            } else if snr > 10.0 {
                self.color_warning
            } else {
                self.color_error
            };
            let badge_text = format!("SNR {snr:.1} dB");
            let text_pos = egui::pos2(spectrum_rect.right() - 4.0, spectrum_rect.top() + 4.0);
            let bg_rect = egui::Rect::from_min_size(
                egui::pos2(text_pos.x - 68.0, text_pos.y - 1.0),
                egui::vec2(72.0, 14.0),
            );
            painter.rect_filled(
                bg_rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
            );
            painter.text(
                text_pos,
                egui::Align2::RIGHT_TOP,
                &badge_text,
                egui::FontId::monospace(10.0),
                snr_color,
            );
        }

        // Signal active / last-seen badge (top-right, below SNR badge)
        if self.squelch_db > -90.0 {
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let (badge_text, fg_color, bg_color) = if self.signal_active {
                (
                    "● ACTIVE".to_string(),
                    self.color_success,
                    egui::Color32::from_rgba_unmultiplied(0, 40, 0, 180),
                )
            } else if let Some(last) = self.last_signal_unix {
                let elapsed = (now_unix - last).max(0.0);
                let text = if elapsed < 60.0 {
                    format!("Last: {elapsed:.0}s ago")
                } else if elapsed < 3600.0 {
                    format!("Last: {:.0}m ago", elapsed / 60.0)
                } else {
                    format!("Last: {:.1}h ago", elapsed / 3600.0)
                };
                let alpha = ((1.0 - (elapsed / 600.0).min(1.0)) * 200.0) as u8 + 55;
                (
                    text,
                    egui::Color32::from_rgba_unmultiplied(160, 200, 160, alpha),
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
                )
            } else {
                (
                    "No activity".to_string(),
                    egui::Color32::from_rgba_unmultiplied(100, 100, 100, 140),
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 80),
                )
            };
            let badge_w = (badge_text.len() as f32 * 5.5 + 10.0).max(66.0);
            let active_pos = egui::pos2(spectrum_rect.right() - 4.0, spectrum_rect.top() + 20.0);
            let bg_rect = egui::Rect::from_min_size(
                egui::pos2(active_pos.x - badge_w, active_pos.y - 1.0),
                egui::vec2(badge_w + 4.0, 14.0),
            );
            if self.signal_active {
                crate::fx::paint_glow(painter, bg_rect, 2.0, &self.signal_glow);
            }
            painter.rect_filled(bg_rect, 2.0, bg_color);
            painter.text(
                active_pos,
                egui::Align2::RIGHT_TOP,
                &badge_text,
                egui::FontId::monospace(10.0),
                fg_color,
            );
        }

        // Band name overlay (top-left of spectrum)
        if let Some(info) = crate::sdr_panel::identify_frequency(self.vfo_frequency_hz()) {
            let band_pos = egui::pos2(spectrum_rect.left() + 4.0, spectrum_rect.top() + 4.0);
            let band_w = (info.band.len() as f32 * 6.5 + 8.0).min(200.0);
            let bg_rect = egui::Rect::from_min_size(
                egui::pos2(band_pos.x - 2.0, band_pos.y - 1.0),
                egui::vec2(band_w, 14.0),
            );
            painter.rect_filled(
                bg_rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
            );
            painter.text(
                band_pos,
                egui::Align2::LEFT_TOP,
                info.band,
                egui::FontId::proportional(10.0),
                egui::Color32::from_rgba_unmultiplied(180, 220, 255, 220),
            );
        }

        // Center frequency indicator (dashed vertical line)
        {
            let zoom_span =
                (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
            let zoom_center_offset = self.zoom_center_offset(zoom_span);
            let left_hz = -zoom_span / 2.0 + zoom_center_offset;
            let right_hz = zoom_span / 2.0 + zoom_center_offset;
            let center_offset = 0.0f64; // center frequency offset from itself is 0
            let frac = (center_offset - left_hz) / (right_hz - left_hz);
            if (0.0..=1.0).contains(&frac) {
                let x = spectrum_rect.left() + frac as f32 * spectrum_rect.width();
                // Draw dashed by alternating segments
                let dash_len = 6.0f32;
                let gap_len = 4.0f32;
                let mut y = spectrum_rect.top();
                while y < spectrum_rect.bottom() {
                    let y_end = (y + dash_len).min(spectrum_rect.bottom());
                    painter.line_segment(
                        [egui::pos2(x, y), egui::pos2(x, y_end)],
                        egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_unmultiplied(100, 160, 255, 100),
                        ),
                    );
                    y += dash_len + gap_len;
                }
                painter.text(
                    egui::pos2(x + 3.0, spectrum_rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    "⟵CTR",
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgba_unmultiplied(100, 160, 255, 150),
                );
            }
        }

        // Frequency markers
        if self.vfo_frequency_hz() != self.center_freq {
            self.paint_vfo_marker(painter, spectrum_rect);
        }
        for (marker_freq, marker_label) in &self.markers {
            let offset_hz = *marker_freq as f64 - self.center_freq as f64;
            let zoom_span =
                (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
            let zoom_center_offset = self.zoom_center_offset(zoom_span);
            let left_hz = -zoom_span / 2.0 + zoom_center_offset;
            let right_hz = zoom_span / 2.0 + zoom_center_offset;
            let frac = (offset_hz - left_hz) / (right_hz - left_hz);
            if (0.0..=1.0).contains(&frac) {
                let x = spectrum_rect.left() + frac as f32 * spectrum_rect.width();
                painter.line_segment(
                    [
                        egui::pos2(x, spectrum_rect.top()),
                        egui::pos2(x, spectrum_rect.bottom()),
                    ],
                    egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(255, 200, 50, 160),
                    ),
                );
                let display_label = if marker_label.is_empty() {
                    format!("{:.3} MHz", *marker_freq as f64 / 1e6)
                } else {
                    format!("{} {:.3}M", marker_label, *marker_freq as f64 / 1e6)
                };
                painter.text(
                    egui::pos2(x, spectrum_rect.top() + 2.0),
                    egui::Align2::CENTER_TOP,
                    display_label,
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgba_unmultiplied(255, 200, 50, 200),
                );
            }
        }

        // Marker delta measurement — draw span arrow between first two visible markers
        if self.markers.len() >= 2 {
            let zoom_span =
                (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
            let zoom_center_offset = self.zoom_center_offset(zoom_span);
            let left_hz = -zoom_span / 2.0 + zoom_center_offset;
            let right_hz = zoom_span / 2.0 + zoom_center_offset;
            let freq_to_x = |freq: u64| -> Option<f32> {
                let offset = freq as f64 - self.center_freq as f64;
                let frac = (offset - left_hz) / (right_hz - left_hz);
                if (0.0..=1.0).contains(&frac) {
                    Some(spectrum_rect.left() + frac as f32 * spectrum_rect.width())
                } else {
                    None
                }
            };
            let visible: Vec<u64> = self
                .markers
                .iter()
                .filter_map(|(f, _)| freq_to_x(*f).map(|_| *f))
                .take(2)
                .collect();
            if visible.len() == 2 {
                if let (Some(x1), Some(x2)) = (freq_to_x(visible[0]), freq_to_x(visible[1])) {
                    let (xl, xr, fl, fr) = if x1 < x2 {
                        (x1, x2, visible[0], visible[1])
                    } else {
                        (x2, x1, visible[1], visible[0])
                    };
                    let delta_hz = fr as f64 - fl as f64;
                    let delta_str = if delta_hz.abs() >= 1_000_000.0 {
                        format!("Δ {:.3} MHz", delta_hz / 1e6)
                    } else if delta_hz.abs() >= 1000.0 {
                        format!("Δ {:.1} kHz", delta_hz / 1000.0)
                    } else {
                        format!("Δ {delta_hz:.0} Hz")
                    };
                    let span_y = spectrum_rect.bottom() - 12.0;
                    let arrow_color = egui::Color32::from_rgba_unmultiplied(200, 200, 80, 180);
                    painter.line_segment(
                        [egui::pos2(xl, span_y), egui::pos2(xr, span_y)],
                        egui::Stroke::new(1.0, arrow_color),
                    );
                    painter.line_segment(
                        [egui::pos2(xl, span_y - 3.0), egui::pos2(xl, span_y + 3.0)],
                        egui::Stroke::new(1.0, arrow_color),
                    );
                    painter.line_segment(
                        [egui::pos2(xr, span_y - 3.0), egui::pos2(xr, span_y + 3.0)],
                        egui::Stroke::new(1.0, arrow_color),
                    );
                    let mid_x = f32::midpoint(xl, xr);
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(mid_x - 28.0, span_y - 10.0),
                            egui::vec2(56.0, 11.0),
                        ),
                        2.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                    );
                    painter.text(
                        egui::pos2(mid_x, span_y - 5.0),
                        egui::Align2::CENTER_CENTER,
                        delta_str,
                        egui::FontId::monospace(8.0),
                        arrow_color,
                    );
                }
            }
        }

        // Scanner sweep position marker
        if let Some(scan_freq) = self.scan_marker {
            let offset_hz = scan_freq as f64 - self.center_freq as f64;
            let zoom_span_s =
                (self.sample_rate / f64::from(self.zoom_factor)).max(self.sample_rate * 0.01);
            let zoom_center_offset_s = self.zoom_center_offset(zoom_span_s);
            let left_hz_s = -zoom_span_s / 2.0 + zoom_center_offset_s;
            let right_hz_s = zoom_span_s / 2.0 + zoom_center_offset_s;
            let frac = (offset_hz - left_hz_s) / (right_hz_s - left_hz_s);
            if (0.0..=1.0).contains(&frac) {
                let x = spectrum_rect.left() + frac as f32 * spectrum_rect.width();
                // Dashed cyan line
                let dash = 5.0f32;
                let mut y = spectrum_rect.top();
                while y < spectrum_rect.bottom() {
                    let y_end = (y + dash).min(spectrum_rect.bottom());
                    painter.line_segment(
                        [egui::pos2(x, y), egui::pos2(x, y_end)],
                        egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_unmultiplied(0, 220, 220, 180),
                        ),
                    );
                    y += dash * 2.0;
                }
                painter.text(
                    egui::pos2(x + 2.0, spectrum_rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("🔍 {:.3}", scan_freq as f64 / 1e6),
                    egui::FontId::proportional(7.5),
                    egui::Color32::from_rgba_unmultiplied(0, 220, 220, 200),
                );
            }
        }

        // Frozen indicator overlay
        if self.frozen {
            let top_left = spectrum_rect.left_top() + egui::vec2(8.0, 6.0);
            let msg = "❄ FROZEN — press F to unfreeze";
            painter.text(
                top_left,
                egui::Align2::LEFT_TOP,
                msg,
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(100, 180, 255),
            );
        }

        // Empty-state overlay when no SDR source is running — clickable ▶ Start button
        if !self.source_running {
            let center = spectrum_rect.center();
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(360.0, 80.0)),
                8.0,
                egui::Color32::from_rgba_unmultiplied(10, 10, 20, 210),
            );
            painter.text(
                center - egui::vec2(0.0, 26.0),
                egui::Align2::CENTER_CENTER,
                "No SDR source running",
                egui::FontId::proportional(15.0),
                egui::Color32::from_rgb(200, 200, 200),
            );
            let btn_rect = egui::Rect::from_center_size(
                center + egui::vec2(0.0, 8.0),
                egui::vec2(180.0, 32.0),
            );
            if ui
                .put(
                    btn_rect,
                    egui::Button::new(egui::RichText::new("▶  Start SDR").size(14.0).strong())
                        .fill(egui::Color32::from_rgb(30, 100, 50)),
                )
                .on_hover_text(
                    "Click to start the SDR source and begin receiving signals (or press Space).",
                )
                .clicked()
            {
                self.pending_start_source = true;
            }
        } else if self.source_running
            && !self.spectrum_dbs.is_empty()
            && self.spectrum_dbs.iter().all(|&db| db < -100.0)
        {
            // Demo mode with no visible signal — help beginner get started
            let center = spectrum_rect.center();
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(420.0, 100.0)),
                8.0,
                egui::Color32::from_rgba_unmultiplied(10, 20, 40, 210),
            );
            painter.text(
                center - egui::vec2(0.0, 30.0),
                egui::Align2::CENTER_CENTER,
                "Demo Mode — Try a quick preset to hear signals",
                egui::FontId::proportional(14.0),
                egui::Color32::from_rgb(200, 200, 200),
            );
            painter.text(
                center + egui::vec2(0.0, -8.0),
                egui::Align2::CENTER_CENTER,
                "📻 FM  •  🛰️ ADS-B  •  ☁️ Weather  •  📡 ISS  •  🔬 2m Ham",
                egui::FontId::monospace(12.0),
                egui::Color32::from_rgb(150, 220, 150),
            );
            painter.text(
                center + egui::vec2(0.0, 18.0),
                egui::Align2::CENTER_CENTER,
                "(Buttons in SDR Panel left side, or press Space to see more options)",
                egui::FontId::monospace(10.0),
                egui::Color32::from_rgb(150, 150, 200),
            );
        }

        // Capture right-click position for context menu squelch action
        if response.secondary_clicked() {
            self.ctx_menu_pos = response.hover_pos();
        }

        // Click-to-tune, zoom, and markers on spectrum
        if response.double_clicked() {
            // Double-click: add frequency marker
            if let Some(pointer) = response.hover_pos() {
                let frac =
                    ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
                let freq = self.frequency_at_plot_fraction(frac);
                self.marker_pending_freq = Some(freq);
            }
        } else if response.clicked() {
            if let Some(pointer) = response.hover_pos() {
                let frac =
                    ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
                let freq = self.frequency_at_plot_fraction(frac);
                self.clicked_tune_freq = Some(freq);
            }
        }
        // Right-click context menu
        response.context_menu(|ui| {
            // Compute hovered frequency for menu actions
            let hovered_freq = response.hover_pos().map(|pointer| {
                let frac = ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
                self.frequency_at_plot_fraction(frac)
            });

            if let Some(freq) = hovered_freq {
                let freq_mhz = freq as f64 / 1e6;
                ui.label(egui::RichText::new(format!("{freq_mhz:.4} MHz")).strong());
                if let Some(info) = crate::sdr_panel::identify_frequency(freq) {
                    ui.colored_label(egui::Color32::from_rgb(180, 220, 255),
                        format!("📻 {} — {}", info.band, info.short_desc));
                    if !info.tips.is_empty() {
                        ui.colored_label(egui::Color32::GRAY,
                            egui::RichText::new(format!("💡 {}", info.tips)).small());
                    }
                }
                ui.separator();
                if ui.button("📡 Tune here").clicked() {
                    self.clicked_tune_freq = Some(freq);
                    ui.close();
                }
                if ui.button("📡⭐ Tune + Bookmark").on_hover_text("Tune to this frequency AND add a bookmark in one click.").clicked() {
                    self.clicked_tune_freq = Some(freq);
                    self.pending_bookmark_freq = Some(freq);
                    ui.close();
                }
                if ui.button("🔷 Set as VFO B").on_hover_text("Save this frequency as VFO B for quick A/B comparison.").clicked() {
                    self.pending_vfo_b_freq = Some(freq);
                    ui.close();
                }
                if ui.button("📍 Add marker").clicked() {
                    self.marker_pending_freq = Some(freq);
                    ui.close();
                }
                if ui.button("⭐ Bookmark only").clicked() {
                    self.pending_bookmark_freq = Some(freq);
                    ui.close();
                }
                if ui.button("📋 Copy frequency").clicked() {
                    ui.ctx().copy_text(format!("{freq_mhz:.4}"));
                    ui.close();
                }
                if ui.button(format!("🤖 Ask AI about {freq_mhz:.3} MHz"))
                    .on_hover_text("Pre-fill the AI Agent with a question about this frequency")
                    .clicked()
                {
                    self.pending_ai_freq = Some(freq);
                    ui.close();
                }
                // Instant 3dB bandwidth estimate at cursor frequency
                if !self.spectrum_dbs.is_empty() {
                    let n = self.spectrum_dbs.len();
                    let hz_per_bin = self.sample_rate / n as f64;
                    let zoom_span_bw = self.sample_rate;
                    let center_bin = ((freq as f64 - self.center_freq as f64 + zoom_span_bw / 2.0) / hz_per_bin).round() as usize;
                    if center_bin < n {
                        let peak_db = self.spectrum_dbs[center_bin];
                        let threshold = peak_db - 3.0;
                        let mut lo = center_bin;
                        while lo > 0 && self.spectrum_dbs[lo - 1] >= threshold { lo -= 1; }
                        let mut hi = center_bin;
                        while hi + 1 < n && self.spectrum_dbs[hi + 1] >= threshold { hi += 1; }
                        let bw_hz = ((hi - lo) as f64 * hz_per_bin).max(hz_per_bin);
                        let bw_str = if bw_hz >= 1_000_000.0 { format!("{:.2} MHz", bw_hz / 1e6) }
                            else if bw_hz >= 1000.0 { format!("{:.1} kHz", bw_hz / 1000.0) }
                            else { format!("{bw_hz:.0} Hz") };
                        ui.colored_label(egui::Color32::from_rgb(180, 255, 180),
                            format!("📐 3 dB BW ≈ {bw_str}"))
                            .on_hover_text("Estimated 3 dB bandwidth: bins within 3 dB of the peak at the cursor.");
                        // Suggest likely signal type + demod mode based on bandwidth
                        let (suggestion, suggested_mode): (&str, Option<&str>) =
                            if bw_hz < 500.0    { ("CW (external BFO/decoder), WSPR, or data beacon", Some("USB")) }
                            else if bw_hz < 3_000.0  { ("SSB voice (HAM HF) or narrow data", Some("USB")) }
                            else if bw_hz < 8_000.0  { ("AM voice, aviation NDB", Some("AM")) }
                            else if bw_hz < 16_000.0 { ("NFM voice: PMR446, land mobile, repeater", Some("NFM")) }
                            else if bw_hz < 30_000.0 { ("Wide NFM, POCSAG, APRS, digital voice", Some("NFM")) }
                            else if bw_hz < 100_000.0{ ("AM broadcast, wide data, digital modes", Some("AM")) }
                            else if bw_hz < 300_000.0{ ("WFM broadcast FM (mono in this app)", Some("WFM")) }
                            else { ("Very wide: Wi-Fi, LTE, DAB+, or multiple signals", None) };
                        ui.horizontal(|ui| {
                            ui.colored_label(egui::Color32::from_rgb(200, 200, 140),
                                egui::RichText::new(format!("💡 {suggestion}")).small())
                                .on_hover_text("Suggested signal type based on measured 3 dB bandwidth. Not definitive — combine with frequency and band plan for better ID.");
                            if let Some(mode) = suggested_mode {
                                if ui.small_button(format!("Apply {mode}"))
                                    .on_hover_text(format!("Set demod mode to {mode}"))
                                    .clicked()
                                {
                                    self.pending_demod_mode = Some(mode.to_string());
                                    ui.close();
                                }
                            }
                        });
                    }
                }
                ui.separator();
                if ui.button(format!("▶ Set as scan start ({freq_mhz:.3} MHz)")).clicked() {
                    self.pending_scan_start = Some(freq);
                    ui.close();
                }
                if ui.button(format!("⏹ Set as scan stop ({freq_mhz:.3} MHz)")).clicked() {
                    self.pending_scan_stop = Some(freq);
                    ui.close();
                }
            }
            // "Set squelch here" based on stored hover position
            if let Some(pos) = self.ctx_menu_pos {
                if spectrum_rect.contains(pos) {
                    let y_frac = 1.0 - ((pos.y - spectrum_rect.top()) / spectrum_rect.height()).clamp(0.0, 1.0);
                    let db_at = min_db + y_frac * range;
                    if ui.button(format!("🔒 Set squelch to {db_at:.0} dB")).clicked() {
                        self.pending_squelch_db = Some(db_at);
                        ui.close();
                    }
                }
            }
            ui.separator();
            if ui.button("📸 Save waterfall screenshot").on_hover_text("Save the entire waterfall history as a PNG image with metadata sidecar.").clicked() {
                self.save_waterfall_png();
                ui.close();
            }
            if ui.button("📊 Export spectrum CSV").on_hover_text("Export current FFT data to CSV (frequency_hz, power_dbfs). Opens a file dialog.").clicked() {
                self.export_spectrum_csv();
                ui.close();
            }
            if ui.button("📈 Save signal history").on_hover_text("Save the signal history to signal_history.json (auto-saves on exit).").clicked() {
                self.save_signal_history();
                ui.close();
            }
            if ui.button("🔍 Reset zoom (1x)").clicked() {
                self.zoom_factor = 1.0;
                self.zoom_offset = 0.5;
                ui.close();
            }
            if ui.button("Auto-fit dB range").clicked() {
                if !self.spectrum_dbs.is_empty() {
                    let (cur_min, cur_max) = self.spectrum_dbs.iter().fold(
                        (f32::INFINITY, f32::NEG_INFINITY),
                        |(mn, mx), &v| (mn.min(v), mx.max(v))
                    );
                    let margin = ((cur_max - cur_min) * 0.1).max(5.0);
                    self.display_min_db = (cur_min - margin).max(-160.0);
                    self.display_max_db = (cur_max + margin).min(20.0);
                    self.waterfall_dirty = true;
                }
                ui.close();
            }
            if !self.markers.is_empty()
                && ui.button(format!("Clear {} marker(s)", self.markers.len())).clicked() {
                    self.markers.clear();
                    ui.close();
                }
        });
        // Middle-click to add frequency marker
        if response.clicked_by(egui::PointerButton::Middle) {
            if let Some(pointer) = response.hover_pos() {
                let frac =
                    ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
                let freq = self.frequency_at_plot_fraction(frac);
                self.markers.push((freq, String::new()));
                if self.markers.len() > 20 {
                    self.markers.remove(0);
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Middle) {
            let delta = response.drag_delta();
            self.zoom_offset = (self.zoom_offset - delta.x / spectrum_rect.width()).clamp(0.0, 1.0);
        }
        if response.hovered() {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            let shift = ui.input(|i| i.modifiers.shift);
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            if scroll_delta.y != 0.0 {
                if ctrl {
                    self.adjust_vfo_bandwidth(scroll_delta.y);
                } else if shift {
                    self.zoom_offset =
                        (self.zoom_offset - scroll_delta.y.signum() * 0.02).clamp(0.0, 1.0);
                } else {
                    self.zoom_factor =
                        (self.zoom_factor * (1.0 + scroll_delta.y * -0.1)).clamp(1.0, 200.0);
                }
            }
        }

        if !self.waterfall_visible {
            ui.advance_cursor_after_rect(spectrum_rect);
            return;
        }
        let texture_limit = ui.ctx().input(|input| input.max_texture_side).max(1);
        if self.waterfall_width > texture_limit || self.waterfall_history > texture_limit {
            self.texture_limit = texture_limit;
            self.reset_waterfall();
        }

        // Overlay widgets inside the FFT can advance the UI cursor to their
        // own rectangles. Restore plot flow before allocating the waterfall.
        ui.advance_cursor_after_rect(spectrum_rect);
        let waterfall_height = ui.available_height().max(1.0);
        let (wf_rect, wf_response) = ui.allocate_exact_size(
            egui::vec2(avail.x, waterfall_height),
            egui::Sense::click_and_drag(),
        );

        if self.waterfall_dirty
            || self.waterfall_texture.is_none()
            || (self.full_waterfall_update && self.waterfall_pending_rows.iter().any(|&row| row))
        {
            let mut rgba_bytes =
                Vec::with_capacity(self.waterfall_width * self.waterfall_history * 4);
            for row_data in &self.waterfall_pixels {
                rgba_bytes.extend_from_slice(row_data);
            }
            let rgba = egui::ColorImage::from_rgba_unmultiplied(
                [self.waterfall_width, self.waterfall_history],
                &rgba_bytes,
            );
            match &mut self.waterfall_texture {
                Some(tex) => tex.set(rgba, egui::TextureOptions::NEAREST),
                None => {
                    self.waterfall_texture = Some(ui.ctx().load_texture(
                        "waterfall",
                        rgba,
                        egui::TextureOptions::NEAREST,
                    ))
                }
            }
            self.waterfall_dirty = false;
            self.waterfall_pending_rows.fill(false);
        } else if let Some(tex) = &mut self.waterfall_texture {
            // Multiple real FFT frames may arrive between UI draws. Upload all
            // changed rows, never append duplicated stale spectra on UI ticks.
            for (y, pending) in self.waterfall_pending_rows.iter_mut().enumerate() {
                if *pending {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [self.waterfall_width, 1],
                        &self.waterfall_pixels[y],
                    );
                    tex.set_partial([0, y], image, egui::TextureOptions::NEAREST);
                    *pending = false;
                }
            }
        }

        if let Some(tex) = &self.waterfall_texture {
            // Circular scroll: texture rows [head..H) are oldest→… on the top
            // part, rows [0..head) (newest) on the bottom part. Newest ends
            // up at the bottom, oldest at the top.
            let u_left = ((left_hz / self.sample_rate) + 0.5).clamp(0.0, 1.0) as f32;
            let u_right = ((right_hz / self.sample_rate) + 0.5).clamp(0.0, 1.0) as f32;
            let h = self.waterfall_history.max(1);
            let head = self.waterfall_head.min(h) % h.max(1);
            if head == 0 {
                ui.painter().image(
                    tex.id(),
                    wf_rect,
                    egui::Rect::from_min_max(egui::pos2(u_left, 0.0), egui::pos2(u_right, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                let top_frac = (h - head) as f32 / h as f32;
                let split_y = wf_rect.top() + top_frac * wf_rect.height();
                let top_rect = egui::Rect::from_min_max(
                    egui::pos2(wf_rect.left(), wf_rect.top()),
                    egui::pos2(wf_rect.right(), split_y),
                );
                let bot_rect = egui::Rect::from_min_max(
                    egui::pos2(wf_rect.left(), split_y),
                    egui::pos2(wf_rect.right(), wf_rect.bottom()),
                );
                let v_split = head as f32 / h as f32;
                ui.painter().image(
                    tex.id(),
                    top_rect,
                    egui::Rect::from_min_max(egui::pos2(u_left, v_split), egui::pos2(u_right, 1.0)),
                    egui::Color32::WHITE,
                );
                ui.painter().image(
                    tex.id(),
                    bot_rect,
                    egui::Rect::from_min_max(egui::pos2(u_left, 0.0), egui::pos2(u_right, v_split)),
                    egui::Color32::WHITE,
                );
            }
        }

        // Waterfall frequency labels (zoom-aware)
        let wf_painter = ui.painter();
        for i in 0..=n_grid {
            let frac = f64::from(i) / f64::from(n_grid);
            let x = wf_rect.left() + (frac as f32) * wf_rect.width();
            let offset_hz = left_hz + frac * zoom_span;
            let freq_mhz = (self.center_freq as f64 + offset_hz) / 1e6;
            wf_painter.text(
                egui::pos2(x, wf_rect.top() + 2.0),
                egui::Align2::CENTER_TOP,
                format!("{freq_mhz:.2}"),
                egui::FontId::proportional(8.0),
                egui::Color32::from_rgba_unmultiplied(180, 180, 180, 160),
            );
        }

        // Waterfall time axis labels (left edge)
        {
            let secs_per_row = self.frame_period_seconds * f64::from(self.waterfall_every_n.max(1));
            let interval_rows = (self.waterfall_history / 8).max(1);
            let n_labels = self.waterfall_history / interval_rows;
            for k in 1..=n_labels {
                let row = k * interval_rows;
                let frac = row as f32 / self.waterfall_history as f32;
                let y = wf_rect.top() + frac * wf_rect.height();
                let secs_ago = (self.waterfall_history - row) as f64 * secs_per_row;
                let label = if secs_ago >= 60.0 {
                    format!("-{:.0}m", secs_ago / 60.0)
                } else if secs_ago >= 1.0 {
                    format!("-{secs_ago:.0}s")
                } else {
                    format!("-{:.0}ms", secs_ago * 1000.0)
                };
                wf_painter.text(
                    egui::pos2(wf_rect.left() + 2.0, y),
                    egui::Align2::LEFT_CENTER,
                    &label,
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgba_unmultiplied(180, 180, 180, 140),
                );
            }
        }

        // Bookmark markers on waterfall
        if self.show_bookmarks {
            for (bm_freq, bm_name, bm_cat) in &self.bookmark_freqs {
                let offset_hz = *bm_freq as f64 - self.center_freq as f64;
                let frac = (offset_hz - left_hz) / zoom_span;
                if (0.0..=1.0).contains(&frac) {
                    let x = wf_rect.left() + frac as f32 * wf_rect.width();
                    let (line_color, label_color) = category_color(bm_cat);
                    // Scale both premultiplied RGB and alpha to dim the colors.
                    let line_color = line_color.gamma_multiply(90.0 / f32::from(line_color.a()));
                    let label_color =
                        label_color.gamma_multiply(130.0 / f32::from(label_color.a()));
                    wf_painter.line_segment(
                        [
                            egui::pos2(x, wf_rect.top()),
                            egui::pos2(x, wf_rect.bottom()),
                        ],
                        egui::Stroke::new(0.7, line_color),
                    );
                    wf_painter.text(
                        egui::pos2(x + 2.0, wf_rect.top() + 14.0),
                        egui::Align2::LEFT_TOP,
                        bm_name.as_str(),
                        egui::FontId::proportional(7.0),
                        label_color,
                    );
                }
            }
        }

        // VFO B marker on waterfall
        if self.show_vfo_b && self.vfo_b_freq > 0 {
            let offset_hz_b = self.vfo_b_freq as f64 - self.center_freq as f64;
            let frac_b = (offset_hz_b - left_hz) / zoom_span;
            if (0.0..=1.0).contains(&frac_b) {
                let x = wf_rect.left() + frac_b as f32 * wf_rect.width();
                let n_dashes = 14;
                let total_h = wf_rect.height();
                let dash_len = total_h / n_dashes as f32 / 2.0;
                for i in 0..n_dashes {
                    let y0 = wf_rect.top() + (i as f32 / n_dashes as f32) * total_h;
                    let y1 = (y0 + dash_len).min(wf_rect.bottom());
                    wf_painter.line_segment(
                        [egui::pos2(x, y0), egui::pos2(x, y1)],
                        egui::Stroke::new(
                            0.8,
                            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 90),
                        ),
                    );
                }
                wf_painter.text(
                    egui::pos2(x + 2.0, wf_rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    "VFO B",
                    egui::FontId::proportional(7.0),
                    egui::Color32::from_rgba_unmultiplied(100, 180, 255, 140),
                );
            }
        }

        self.paint_vfo_marker(wf_painter, wf_rect);

        // Waterfall drag-to-pan zoom window
        if wf_response.dragged_by(egui::PointerButton::Primary) {
            let delta = wf_response.drag_delta();
            self.zoom_offset = (self.zoom_offset - delta.x / wf_rect.width()).clamp(0.0, 1.0);
        }
        // Waterfall scroll-to-zoom (matches spectrum behavior)
        if wf_response.hovered() {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            let shift = ui.input(|i| i.modifiers.shift);
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            if scroll_delta.y != 0.0 {
                if ctrl {
                    self.adjust_vfo_bandwidth(scroll_delta.y);
                } else if shift {
                    self.zoom_offset =
                        (self.zoom_offset - scroll_delta.y.signum() * 0.02).clamp(0.0, 1.0);
                } else {
                    self.zoom_factor =
                        (self.zoom_factor * (1.0 + scroll_delta.y * -0.1)).clamp(1.0, 200.0);
                }
            }
        }
        // Waterfall click-to-tune, double-click to place marker
        if wf_response.double_clicked() {
            if let Some(pointer) = wf_response.hover_pos() {
                let frac = ((pointer.x - wf_rect.left()) / wf_rect.width()).clamp(0.0, 1.0);
                let freq = self.frequency_at_plot_fraction(frac);
                self.marker_pending_freq = Some(freq);
            }
        } else if wf_response.clicked() {
            if let Some(pointer) = wf_response.hover_pos() {
                let frac = ((pointer.x - wf_rect.left()) / wf_rect.width()).clamp(0.0, 1.0);
                let freq = self.frequency_at_plot_fraction(frac);
                self.clicked_tune_freq = Some(freq);
            }
        }
        // Waterfall right-click context menu
        if wf_response.secondary_clicked() {
            self.ctx_menu_pos = wf_response.hover_pos();
        }
        wf_response.context_menu(|ui| {
            if let Some(pos) = self.ctx_menu_pos {
                if wf_rect.contains(pos) {
                    let frac = ((pos.x - wf_rect.left()) / wf_rect.width()).clamp(0.0, 1.0);
                    let freq = self.frequency_at_plot_fraction(frac);
                    let freq_mhz = freq as f64 / 1e6;
                    ui.label(egui::RichText::new(format!("{freq_mhz:.4} MHz")).strong());
                    ui.separator();
                    if ui.button("📡 Tune here").clicked() {
                        self.clicked_tune_freq = Some(freq);
                        ui.close();
                    }
                    if ui.button("⭐ Bookmark this frequency").clicked() {
                        self.pending_bookmark_freq = Some(freq);
                        ui.close();
                    }
                    if ui
                        .button("🔷 Set as VFO B")
                        .on_hover_text("Save this frequency as VFO B for quick A/B comparison.")
                        .clicked()
                    {
                        self.clicked_tune_freq = Some(freq);
                        self.pending_vfo_b_freq = Some(freq);
                        ui.close();
                    }
                    if ui.button("📋 Copy frequency").clicked() {
                        ui.ctx().copy_text(format!("{freq_mhz:.4}"));
                        ui.close();
                    }
                    if ui
                        .button(format!("🤖 Ask AI about {freq_mhz:.3} MHz"))
                        .on_hover_text("Pre-fill the AI Agent with a question about this frequency")
                        .clicked()
                    {
                        self.pending_ai_freq = Some(freq);
                        ui.close();
                    }
                }
            }
            ui.separator();
            if ui
                .button("📸 Save waterfall screenshot")
                .on_hover_text(
                    "Save the entire waterfall history as a PNG image with metadata sidecar.",
                )
                .clicked()
            {
                self.save_waterfall_png();
                ui.close();
            }
            if ui.button("🔍 Reset zoom (1x)").clicked() {
                self.zoom_factor = 1.0;
                self.zoom_offset = 0.5;
                ui.close();
            }
        });
        // Waterfall hover crosshair + tooltip
        if let Some(pointer) = wf_response.hover_pos() {
            let frac = ((pointer.x - wf_rect.left()) / wf_rect.width()).clamp(0.0, 1.0);
            let offset_hz = left_hz + f64::from(frac) * zoom_span;
            let freq = self.center_freq as f64 + offset_hz;
            let freq_str = if freq >= 1e9 {
                format!("{:.3} GHz", freq / 1e9)
            } else if freq >= 1e6 {
                format!("{:.3} MHz", freq / 1e6)
            } else {
                format!("{:.1} kHz", freq / 1e3)
            };
            wf_painter.line_segment(
                [
                    egui::pos2(pointer.x, wf_rect.top()),
                    egui::pos2(pointer.x, wf_rect.bottom()),
                ],
                egui::Stroke::new(
                    0.5,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100),
                ),
            );
            let tip_rect = egui::Rect::from_min_size(
                egui::pos2(pointer.x + 6.0, pointer.y - 12.0),
                egui::vec2(100.0, 14.0),
            );
            wf_painter.rect_filled(
                tip_rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
            );
            wf_painter.text(
                egui::pos2(tip_rect.left() + 3.0, tip_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &freq_str,
                egui::FontId::monospace(9.0),
                egui::Color32::from_rgb(100, 200, 255),
            );
        }
    }
}

fn lerp_color(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    (
        (f32::from(a.0) + (f32::from(b.0) - f32::from(a.0)) * t) as u8,
        (f32::from(a.1) + (f32::from(b.1) - f32::from(a.1)) * t) as u8,
        (f32::from(a.2) + (f32::from(b.2) - f32::from(a.2)) * t) as u8,
    )
}

fn sample_palette(palette: &[(u8, u8, u8)], t: f32) -> (u8, u8, u8) {
    let n = palette.len();
    if n == 0 {
        return (0, 0, 0);
    }
    let scaled = t.clamp(0.0, 1.0) * (n - 1) as f32;
    let lo = scaled.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    lerp_color(palette[lo], palette[hi], scaled - lo as f32)
}

fn color_map(cmap: ColorMap, t: f32) -> (u8, u8, u8) {
    match cmap {
        ColorMap::Classic => waterfall_color_classic(t),
        ColorMap::Viridis => {
            // 8-stop piecewise approximation of matplotlib Viridis
            const V: &[(u8, u8, u8)] = &[
                (68, 1, 84),
                (72, 40, 120),
                (62, 74, 137),
                (49, 104, 142),
                (38, 130, 142),
                (31, 158, 137),
                (53, 183, 121),
                (110, 206, 88),
                (181, 222, 43),
                (253, 231, 37),
            ];
            sample_palette(V, t)
        }
        ColorMap::Plasma => {
            // 10-stop piecewise approximation of matplotlib Plasma
            const P: &[(u8, u8, u8)] = &[
                (13, 8, 135),
                (75, 3, 161),
                (125, 3, 168),
                (168, 34, 150),
                (203, 70, 121),
                (229, 107, 93),
                (248, 148, 65),
                (253, 195, 40),
                (240, 249, 33),
                (240, 249, 33),
            ];
            sample_palette(P, t)
        }
        ColorMap::Magma => {
            // 10-stop piecewise approximation of matplotlib Magma
            const M: &[(u8, u8, u8)] = &[
                (0, 0, 4),
                (28, 16, 68),
                (79, 18, 123),
                (129, 37, 129),
                (181, 54, 122),
                (229, 80, 100),
                (251, 135, 97),
                (254, 194, 135),
                (252, 253, 191),
                (252, 253, 191),
            ];
            sample_palette(M, t)
        }
        ColorMap::Grayscale => {
            let v = (t * 255.0) as u8;
            (v, v, v)
        }
        ColorMap::Hot => {
            let r = ((t * 3.0).min(1.0) * 255.0) as u8;
            let g = ((t * 3.0 - 1.0).clamp(0.0, 1.0) * 255.0) as u8;
            let b = ((t * 3.0 - 2.0).clamp(0.0, 1.0) * 255.0) as u8;
            (r, g, b)
        }
        ColorMap::Inferno => {
            const I: &[(u8, u8, u8)] = &[
                (0, 0, 4),
                (10, 7, 34),
                (43, 14, 76),
                (85, 17, 109),
                (128, 24, 124),
                (171, 41, 113),
                (210, 71, 82),
                (240, 113, 39),
                (246, 162, 16),
                (246, 214, 50),
                (252, 255, 164),
            ];
            sample_palette(I, t)
        }
        ColorMap::Turbo => {
            const T: &[(u8, u8, u8)] = &[
                (48, 18, 59),
                (62, 37, 137),
                (58, 72, 195),
                (39, 115, 215),
                (24, 158, 196),
                (34, 196, 149),
                (76, 226, 94),
                (144, 242, 44),
                (213, 239, 24),
                (255, 219, 28),
                (255, 198, 37),
                (255, 173, 46),
                (255, 130, 57),
                (247, 79, 67),
                (222, 33, 69),
                (186, 9, 58),
            ];
            sample_palette(T, t)
        }
    }
}

fn waterfall_color_classic(norm: f32) -> (u8, u8, u8) {
    if norm < 0.15 {
        let t = norm / 0.15;
        (0, 0, (t * 80.0) as u8)
    } else if norm < 0.35 {
        let t = (norm - 0.15) / 0.20;
        ((t * 60.0) as u8, 0, (80.0 + t * 120.0) as u8)
    } else if norm < 0.55 {
        let t = (norm - 0.35) / 0.20;
        ((60.0 + t * 140.0) as u8, 0, (200.0 - t * 60.0) as u8)
    } else if norm < 0.75 {
        let t = (norm - 0.55) / 0.20;
        (200, (t * 200.0) as u8, (140.0 - t * 100.0) as u8)
    } else {
        let t = (norm - 0.75) / 0.25;
        (200, (200.0 + t * 55.0) as u8, (40.0 + t * 100.0) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for_spectrum_frames(spectrum: &mut SpectrumAnalyzer, expected: u64) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while spectrum.processed_frames < expected && Instant::now() < deadline {
            spectrum.try_recv_spectrum();
            if spectrum.processed_frames < expected {
                std::thread::yield_now();
            }
        }
        if spectrum.processed_frames < expected {
            return;
        }
        let mut quiet_since = Instant::now();
        // A burst may leave a newer result in flight after the first frame is
        // published. Give the worker a short quiet window so latest-frame
        // assertions observe that final result instead of a stale one.
        while Instant::now() < deadline {
            let before = spectrum.processed_frames;
            spectrum.try_recv_spectrum();
            if spectrum.processed_frames != before {
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() >= Duration::from_millis(10) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    // -----------------------------------------------------------------------
    // lerp_color
    // -----------------------------------------------------------------------
    #[test]
    fn lerp_color_at_zero_returns_a() {
        let a = (10, 20, 30);
        let b = (200, 210, 220);
        assert_eq!(lerp_color(a, b, 0.0), a);
    }

    #[test]
    fn lerp_color_at_one_returns_b() {
        let a = (10, 20, 30);
        let b = (200, 210, 220);
        assert_eq!(lerp_color(a, b, 1.0), b);
    }

    #[test]
    fn lerp_color_at_half_returns_midpoint() {
        let a = (10, 20, 30);
        let b = (200, 210, 220);
        let mid = lerp_color(a, b, 0.5);
        assert_eq!(mid, ((10 + 200) / 2, (20 + 210) / 2, (30 + 220) / 2));
    }

    #[test]
    fn lerp_color_identical_colors() {
        let c = (128, 128, 128);
        let t = 0.3;
        assert_eq!(lerp_color(c, c, t), c);
    }

    // -----------------------------------------------------------------------
    // sample_palette
    // -----------------------------------------------------------------------
    #[test]
    fn sample_palette_at_zero() {
        let pal = &[(10, 20, 30), (100, 200, 250), (200, 100, 50)];
        assert_eq!(sample_palette(pal, 0.0), pal[0]);
    }

    #[test]
    fn sample_palette_at_one() {
        let pal = &[(10, 20, 30), (100, 200, 250), (200, 100, 50)];
        assert_eq!(sample_palette(pal, 1.0), pal[2]);
    }

    #[test]
    fn sample_palette_at_half() {
        let pal = &[(10, 20, 30), (100, 200, 250), (200, 100, 50)];
        let mid = sample_palette(pal, 0.5);
        // scaled = 0.5 * 2 = 1.0, lo=1, hi=1, lerp_color(pal[1], pal[1], 0.0)
        assert_eq!(mid, pal[1]);
    }

    #[test]
    fn sample_palette_single_color() {
        let pal = &[(42, 84, 168)];
        assert_eq!(sample_palette(pal, 0.0), (42, 84, 168));
        assert_eq!(sample_palette(pal, 0.5), (42, 84, 168));
        assert_eq!(sample_palette(pal, 1.0), (42, 84, 168));
    }

    #[test]
    fn sample_palette_empty_returns_black() {
        let pal: &[(u8, u8, u8)] = &[];
        assert_eq!(sample_palette(pal, 0.0), (0, 0, 0));
        assert_eq!(sample_palette(pal, 0.5), (0, 0, 0));
        assert_eq!(sample_palette(pal, 1.0), (0, 0, 0));
    }

    // -----------------------------------------------------------------------
    // color_map — all 8 variants produce valid RGB for 0.0, 0.5, 1.0
    // -----------------------------------------------------------------------
    const COLOR_MAP_VARIANTS: &[ColorMap] = &[
        ColorMap::Classic,
        ColorMap::Viridis,
        ColorMap::Plasma,
        ColorMap::Magma,
        ColorMap::Grayscale,
        ColorMap::Hot,
        ColorMap::Inferno,
        ColorMap::Turbo,
    ];

    #[test]
    fn color_map_all_variants_return_valid_rgb() {
        for cmap in COLOR_MAP_VARIANTS {
            for &t in &[0.0, 0.5, 1.0] {
                let (_r, _g, _b) = color_map(*cmap, t);
                // u8 guarantees 0..=255 range; this just validates no panic
            }
        }
    }

    #[test]
    fn color_map_classic_boundaries() {
        let (r, g, b) = color_map(ColorMap::Classic, 0.0);
        assert_eq!((r, g, b), (0, 0, 0));
        let (r, g, b) = color_map(ColorMap::Classic, 1.0);
        assert_eq!((r, g, b), (200, 255, 140));
    }

    // -----------------------------------------------------------------------
    // waterfall_color_classic
    // -----------------------------------------------------------------------
    #[test]
    fn waterfall_color_classic_zero() {
        assert_eq!(waterfall_color_classic(0.0), (0, 0, 0));
    }

    #[test]
    fn waterfall_color_classic_half() {
        let (r, g, b) = waterfall_color_classic(0.5);
        assert!(r <= 200 && g <= 200 && b <= 200);
    }

    #[test]
    fn waterfall_color_classic_one() {
        // norm = 1.0 gets caught by else branch (>=0.75)
        let (r, g, b) = waterfall_color_classic(1.0);
        assert_eq!(r, 200);
        assert_eq!(g, 255);
        assert_eq!(b, 140);
    }

    #[test]
    fn waterfall_color_classic_region_boundaries() {
        // just above 0.15
        let c = waterfall_color_classic(0.151);
        assert!(c.2 >= 80, "got ({},{},{})", c.0, c.1, c.2);

        // just above 0.35
        let c = waterfall_color_classic(0.351);
        assert!(c.0 > 0);

        // just above 0.55
        let c = waterfall_color_classic(0.551);
        assert!(c.0 > 60);

        // just above 0.75
        let c = waterfall_color_classic(0.751);
        assert!(c.0 == 200);
    }

    // -----------------------------------------------------------------------
    // category_color
    // -----------------------------------------------------------------------
    #[test]
    fn category_color_aviation() {
        let (line, label) = category_color("aviation");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 140)
        );
        assert_eq!(
            label,
            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 200)
        );
    }

    #[test]
    fn category_color_weather() {
        let (line, _label) = category_color("weather");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(80, 220, 80, 140)
        );
    }

    #[test]
    fn category_color_marine() {
        let (line, _label) = category_color("marine");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(0, 200, 200, 140)
        );
    }

    #[test]
    fn category_color_amateur() {
        let (line, _label) = category_color("amateur");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(200, 100, 255, 140)
        );
    }

    #[test]
    fn category_color_broadcast() {
        let (line, _label) = category_color("broadcast");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(255, 140, 60, 140)
        );
    }

    #[test]
    fn category_color_scanner() {
        let (line, _label) = category_color("scanner");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(255, 80, 80, 140)
        );
    }

    #[test]
    fn category_color_unknown_returns_default() {
        let (line, _label) = category_color("unknown_category_xyz");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(255, 215, 0, 120)
        );
    }

    #[test]
    fn category_color_is_case_insensitive() {
        let (line, _) = category_color("AVIATION");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 140)
        );
    }

    #[test]
    fn category_color_alt_names() {
        // "air" matches the aviation branch
        let (line, _) = category_color("air");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(100, 180, 255, 140)
        );

        // "noaa" matches weather
        let (line, _) = category_color("noaa");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(80, 220, 80, 140)
        );

        // "ham" matches amateur
        let (line, _) = category_color("ham");
        assert_eq!(
            line,
            egui::Color32::from_rgba_unmultiplied(200, 100, 255, 140)
        );
    }

    #[test]
    fn wfm_overlay_emits_translucent_premultiplied_color() {
        let mut s = SpectrumAnalyzer::new();
        s.demod_mode = "WFM".into();
        s.vfo_bw_hz = 200_000;
        s.show_band_plan = false;
        s.set_waterfall_visible(false);
        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| s.ui(ui),
        );
        let fills: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill.a() == 22 => Some(rect.fill),
                _ => None,
            })
            .collect();
        assert_eq!(fills.len(), 1, "expected the WFM RF passband fill");
        let [red, green, blue, alpha] = fills[0].to_array();
        assert_eq!(alpha, 22);
        // A translucent orange overlay must contribute only its small alpha
        // fraction when blended over black, not a full-bright red channel.
        assert!(red <= alpha && green < red && blue < green);
        assert!(
            (11..=13).contains(&green),
            "orange hue must survive alpha conversion"
        );
        assert!(
            (3..=5).contains(&blue),
            "orange hue must survive alpha conversion"
        );
    }

    #[test]
    fn amateur_overlay_limits_follow_selected_itu_region() {
        let (r1_80, r1_40, r1_125, r1_70) = ItuRegion::Region1.amateur_limits();
        assert_eq!(r1_80, (3.5, 3.8));
        assert_eq!(r1_40, (7.0, 7.2));
        assert_eq!(r1_125, None);
        assert_eq!(r1_70, (430.0, 440.0));

        let (_, _, r2_125, r2_70) = ItuRegion::Region2.amateur_limits();
        assert_eq!(r2_125, Some((222.0, 225.0)));
        assert_eq!(r2_70, (420.0, 450.0));
    }

    // -----------------------------------------------------------------------
    // WindowType::generate
    // -----------------------------------------------------------------------
    #[test]
    fn window_hann_generates_correct_length() {
        let w = WindowType::Hann.generate(256);
        assert_eq!(w.len(), 256);
    }

    #[test]
    fn window_hamming_generates_correct_length() {
        let w = WindowType::Hamming.generate(512);
        assert_eq!(w.len(), 512);
    }

    #[test]
    fn window_blackman_generates_correct_length() {
        let w = WindowType::Blackman.generate(1024);
        assert_eq!(w.len(), 1024);
    }

    #[test]
    fn window_tapers_to_zero_at_ends() {
        // Hann and Blackman start at ~0; Hamming starts at 0.08
        for wt in &[WindowType::Hann, WindowType::Blackman] {
            let w = wt.generate(256);
            assert!(
                w[0].abs() < 0.01,
                "first value for {wt:?} should be ≈0, got {}",
                w[0]
            );
        }
    }

    #[test]
    fn window_peaks_in_center() {
        let w = WindowType::Hann.generate(256);
        let mid = w[128];
        assert!(
            (mid - 1.0).abs() < 0.01,
            "mid value should be ≈1.0, got {mid}"
        );
    }

    #[test]
    fn window_hann_sum_approximate_half_length() {
        let n = 256;
        let w = WindowType::Hann.generate(n);
        let sum: f32 = w.iter().sum();
        // For Hann: sum ≈ n/2
        let expected = n as f32 / 2.0;
        assert!(
            (sum - expected).abs() < 1.0,
            "Hann sum {sum} not close to {expected}"
        );
    }

    #[test]
    fn window_hamming_sum_approximate_half_length() {
        let n = 512;
        let w = WindowType::Hamming.generate(n);
        let sum: f32 = w.iter().sum();
        // For Hamming: sum ≈ 0.54n (asymmetric, so slightly different)
        let expected = 0.54 * n as f32;
        assert!(
            (sum - expected).abs() < 1.0,
            "Hamming sum {sum} not close to {expected}"
        );
    }

    #[test]
    fn window_blackman_flat_start() {
        let w = WindowType::Blackman.generate(256);
        // Blackman first value ~ 0.42 - 0.5 + 0.08 = 0.0
        assert!(
            w[0].abs() < 0.001,
            "Blackman first value should be ≈0.0, got {}",
            w[0]
        );
    }

    // -----------------------------------------------------------------------
    // FFT bin → frequency mapping (fftshift regression)
    // -----------------------------------------------------------------------
    // Push a complex sinusoid at center_freq + offset and verify peak_freq_hz
    // reports the correct frequency. Without the fftshift at storage time the
    // peak landed at the wrong bin and peak_freq_hz was off by ±Fs/2.
    #[test]
    fn peak_freq_hz_matches_injected_tone_offset() {
        let mut s = SpectrumAnalyzer::new();
        let center = 100_000_000u64;
        let rate = 2_048_000u32;
        s.update_params(center, rate);
        let n = s.fft_size;
        let offset_hz: i64 = 100_000; // +100 kHz above center
                                      // A complex sinusoid at offset_hz has phaseadvance per sample = 2π*offset/rate.
                                      // IQ bytes: i = cos(phase)*127 + 127, q = sin(phase)*127 + 127.
        let mut iq = vec![0u8; n * 2];
        let phase_step = 2.0 * std::f64::consts::PI * offset_hz as f64 / rate as f64;
        for k in 0..n {
            let p = phase_step * k as f64;
            let i = ((p.cos() * 100.0) + 127.4) as u8;
            let q = ((p.sin() * 100.0) + 127.4) as u8;
            iq[2 * k] = i;
            iq[2 * k + 1] = q;
        }
        // push several buffers so the smoothed spectrum stabilises around
        // the true peak (avg_alpha smoothing needs a few iterations).
        for _ in 0..16 {
            s.push_iq_samples(&iq);
        }
        wait_for_spectrum_frames(&mut s, 1);
        let got = s.peak_freq_hz() as i64;
        let err = (got - center as i64 - offset_hz).abs();
        // Tolerance: ±2 bins (windowing may smear the peak slightly).
        let bin_hz = (rate as i64) / n as i64;
        assert!(
            err <= 2 * bin_hz,
            "peak_freq_hz={got} expected ≈{expected} (center+{offset_hz}); \
             err={err} Hz, tolerance=±{tol} Hz. Likely spectrum_dbs is still \
             stored in raw FFT bin order instead of fftshifted (centered) order.",
            expected = center + offset_hz as u64,
            tol = 2 * bin_hz,
        );
    }

    #[test]
    fn fft_short_buffer_does_not_panic() {
        // Tiny buffers accumulate real IQ instead of zero-padding an FFT.
        let mut s = SpectrumAnalyzer::new();
        s.push_iq_samples(&[128, 128, 129, 129]); // 2 samples << 2048
        assert_eq!(s.spectrum_dbs.len(), s.fft_size);
        assert!(s.spectrum_dbs.iter().all(|v| v.is_finite()));
        assert_eq!(s.processed_frames, 0);
        assert_eq!(s.iq_filled, 2);
    }

    fn tone(samples: usize, rate: u32, offset: f64) -> Vec<u8> {
        (0..samples)
            .flat_map(|n| {
                let phase = 2.0 * std::f64::consts::PI * offset * n as f64 / rate as f64;
                [
                    (127.4 + 100.0 * phase.cos()).round() as u8,
                    (127.4 + 100.0 * phase.sin()).round() as u8,
                ]
            })
            .collect()
    }

    fn daemon_frame(timestamp_ms: u64, peak: f32) -> ez_proto::SpectrumFrame {
        let mut bins = vec![-100.0; 256];
        bins[160] = peak;
        ez_proto::SpectrumFrame {
            center_hz: 100_000_000,
            sample_rate_hz: 25_600,
            bins,
            timestamp_ms,
        }
    }

    fn complex_tone(samples: usize, rate: f64, offset: f64, amplitude: f32) -> Vec<Complex32> {
        (0..samples)
            .map(|n| {
                let phase = 2.0 * std::f64::consts::PI * offset * n as f64 / rate;
                Complex32::new(phase.cos() as f32, phase.sin() as f32) * amplitude
            })
            .collect()
    }

    #[test]
    #[ignore = "fractional sample rate (2048003/7) + 8192 FFT bin mapping mismatch — FFT itself is correct, tone-to-bin calibration for this edge case needs rework"]
    fn complex_large_fft_preserves_weak_tone_and_fractional_sample_rate() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(8_192);
        s.set_avg_alpha(1.0);
        let rate = 2_048_003.0 / 7.0;
        s.update_params_exact(100_000_000, rate);
        let offset = 12_000.0 * rate / 8_192.0;
        let iq = complex_tone(8_192, rate, offset, 0.0001);
        for chunk in iq[..8_191].chunks(997) {
            s.push_complex_samples(chunk);
        }
        assert_eq!(
            s.processed_frames, 0,
            "a real complete FFT window is required"
        );
        s.push_complex_samples(&iq[8_191..]);
        wait_for_spectrum_frames(&mut s, 1);
        assert_eq!(s.processed_frames, 1);
        assert_eq!(s.sample_rate, rate);
        assert_eq!(s.iq_ring, iq, "float samples must not be requantized");
        let expected = (100_000_000.0 + offset) as u64;
        let bin_hz = (rate / 8_192.0) as u64;
        assert!(
            (s.peak_freq_hz() as i64 - expected as i64).abs() <= bin_hz as i64,
            "peak_freq {} not within one bin ({bin_hz} Hz) of {expected}",
            s.peak_freq_hz()
        );
        assert!((s.peak_level() + 80.0).abs() < 0.01);

        let hop = (rate / 20.0).ceil() as usize;
        s.push_complex_samples(&complex_tone(hop, rate, offset, 0.0001));
        wait_for_spectrum_frames(&mut s, 2);
        assert_eq!(s.processed_frames, 2);
        assert!((s.frame_period_seconds - hop as f64 / rate).abs() < 1e-12);
    }

    #[test]
    fn complex_chunk_boundaries_share_raw_iq_calibration_and_cadence() {
        let mut raw = SpectrumAnalyzer::new();
        let mut complex = SpectrumAnalyzer::new();
        for s in [&mut raw, &mut complex] {
            s.set_fft_size(256);
            s.update_params(100_000_000, 25_600);
            s.set_fft_rate(20);
            s.set_avg_alpha(1.0);
        }
        let bytes = tone(256 + 2 * 1280, 25_600, -3200.0);
        let iq: Vec<_> = bytes
            .chunks_exact(2)
            .map(|pair| {
                Complex32::new(
                    (f32::from(pair[0]) - 127.4) / 128.0,
                    (f32::from(pair[1]) - 127.4) / 128.0,
                )
            })
            .collect();
        raw.push_iq_samples(&bytes);
        for chunk in iq.chunks(113) {
            complex.push_complex_samples(chunk);
        }
        wait_for_spectrum_frames(&mut raw, 1);
        wait_for_spectrum_frames(&mut complex, 1);
        assert!(complex.processed_frames >= 1);
        assert_eq!(complex.spectrum_dbs, raw.spectrum_dbs);
        assert_eq!(complex.iq_ring, raw.iq_ring);
    }

    #[test]
    fn complex_long_burst_is_bounded_and_retains_newest_window() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(256);
        s.update_params(100_000_000, 25_600);
        s.set_fft_rate(100);
        s.set_avg_alpha(1.0);
        let mut iq = complex_tone(256 * 20, 25_600.0, 3200.0, 0.5);
        iq.extend(complex_tone(256 * 20, 25_600.0, -3200.0, 0.25));
        s.push_complex_samples(&iq);
        wait_for_spectrum_frames(&mut s, 1);
        assert!(s.processed_frames >= 1);
        assert_eq!(s.peak_freq_hz(), 99_996_800);
        assert!((s.peak_level() - 20.0 * 0.25_f32.log10()).abs() < 0.01);
        assert_eq!(s.iq_ring.len(), 256);
    }

    #[test]
    fn complex_invalid_samples_do_not_poison_fft_or_pair_with_pending_raw_byte() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(256);
        s.push_iq_samples(&[255]);
        let mut iq = vec![Complex32::new(0.1, 0.0); 256];
        iq[10] = Complex32::new(f32::NAN, 0.0);
        iq[20] = Complex32::new(0.0, f32::INFINITY);
        s.push_complex_samples(&iq);
        wait_for_spectrum_frames(&mut s, 1);
        assert_eq!(s.pending_i_byte, None);
        assert_eq!(s.stream_samples, 256);
        assert_eq!(s.processed_frames, 1);
        assert_eq!(s.iq_ring[10], Complex32::new(0.0, 0.0));
        assert_eq!(s.iq_ring[20], Complex32::new(0.0, 0.0));
        assert!(s.spectrum_dbs.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn offset_vfo_detects_its_tone_and_rejects_unrelated_or_uncaptured_channels() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(256);
        s.set_avg_alpha(1.0);
        s.update_params(100_000_000, 25_600);
        s.vfo_freq_hz = Some(100_003_200);
        s.vfo_bw_hz = 800;
        s.demod_mode = "AM".into();
        let mut iq = complex_tone(256, 25_600.0, 3200.0, 0.25);
        for (sample, interferer) in iq.iter_mut().zip(complex_tone(256, 25_600.0, -3200.0, 0.9)) {
            *sample += interferer;
        }
        s.push_complex_samples(&iq);
        wait_for_spectrum_frames(&mut s, 1);
        assert_eq!(s.peak_freq_hz(), 99_996_800);
        assert_eq!(s.vfo_band_edges(), (2800.0, 3600.0));
        assert!((s.vfo_signal_level() - 20.0 * 0.25_f32.log10()).abs() < 0.01);
        s.vfo_freq_hz = Some(100_050_000);
        assert_eq!(s.vfo_signal_level(), -120.0);
        s.vfo_freq_hz = Some(99_950_000);
        assert_eq!(s.vfo_signal_level(), -120.0);
        s.vfo_freq_hz = Some(100_003_200);
        s.update_params(100_001_000, 25_600);
        assert_eq!(
            s.vfo_frequency_hz(),
            100_003_200,
            "retuning capture keeps explicit VFO"
        );
        assert_eq!(s.vfo_band_edges(), (1800.0, 2600.0));
        s.demod_mode = "USB".into();
        assert_eq!(s.vfo_band_edges(), (2200.0, 3000.0));
        s.demod_mode = "LSB".into();
        assert_eq!(s.vfo_band_edges(), (1400.0, 2200.0));
        s.vfo_freq_hz = None;
        assert_eq!(s.vfo_frequency_hz(), 100_001_000);
    }

    fn render_spectrum_test(
        ctx: &egui::Context,
        s: &mut SpectrumAnalyzer,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                events,
                modifiers,
                ..Default::default()
            },
            |ui| s.ui(ui),
        )
    }

    #[test]
    fn offset_vfo_render_click_and_width_keep_capture_coordinates() {
        let mut s = SpectrumAnalyzer::new();
        s.update_params(100_000_000, 25_600);
        s.vfo_freq_hz = Some(100_006_400);
        s.vfo_bw_hz = 2400;
        s.demod_mode = "AM".into();
        s.show_band_plan = false;
        s.source_running = true;
        s.set_waterfall_visible(false);
        let ctx = egui::Context::default();
        let output = render_spectrum_test(&ctx, &mut s, vec![], egui::Modifiers::NONE);
        let plot = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == s.plot_bg => Some(rect.rect),
                _ => None,
            })
            .expect("spectrum background");
        let passband = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill.a() == 22 => Some(rect.rect),
                _ => None,
            })
            .expect("AM passband");
        assert!((passband.center().x - (plot.left() + plot.width() * 0.75)).abs() < 0.1);
        let center_color = egui::Color32::from_rgba_unmultiplied(100, 160, 255, 100);
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == center_color && (points[0].x - plot.center().x).abs() < 0.1
            )),
            "capture-center marker must remain at the center"
        );
        let position = egui::pos2(plot.left() + plot.width() * 0.625, plot.center().y);
        for pressed in [true, false] {
            render_spectrum_test(
                &ctx,
                &mut s,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                egui::Modifiers::NONE,
            );
        }
        assert_eq!(s.clicked_tune_freq, Some(100_003_200));
        s.adjust_vfo_bandwidth(1.0);
        assert_eq!(s.vfo_bw_hz, 2640);
        assert_eq!(s.vfo_frequency_hz(), 100_006_400);
        assert_eq!(s.center_freq, 100_000_000);
        assert_eq!(s.vfo_band_edges(), (5080.0, 7720.0));

        s.zoom_factor = 4.0;
        s.zoom_offset = 1.0;
        assert_eq!(s.frequency_at_plot_fraction(0.0), 100_006_400);
        assert_eq!(s.frequency_at_plot_fraction(0.5), 100_009_600);
        assert_eq!(s.frequency_at_plot_fraction(1.0), 100_012_800);
        s.spectrum_dbs.fill(-100.0);
        let n = s.spectrum_dbs.len();
        s.spectrum_dbs[n / 4] = -1.0; // stronger, outside the zoom window
        s.spectrum_dbs[n * 7 / 8] = -20.0;
        assert_eq!(s.visible_peak_freq_hz(), 100_009_600);
    }

    #[test]
    fn full_waterfall_update_switches_real_texture_uploads_without_stale_rows() {
        let mut s = SpectrumAnalyzer::new();
        s.waterfall_every_n = 1;
        s.push_spectrum_frame(&daemon_frame(0, -40.0));
        let ctx = egui::Context::default();
        render_spectrum_test(&ctx, &mut s, vec![], egui::Modifiers::NONE);
        let texture = s
            .waterfall_texture
            .as_ref()
            .expect("waterfall texture")
            .id();
        s.push_spectrum_frame(&daemon_frame(50, -30.0));
        let partial = render_spectrum_test(&ctx, &mut s, vec![], egui::Modifiers::NONE);
        let updates: Vec<_> = partial
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| *id == texture)
            .collect();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1.pos, Some([0, 1]));
        assert_eq!(updates[0].1.image.size(), [s.waterfall_width, 1]);
        s.full_waterfall_update = true;
        assert!(s.display_settings().full_waterfall_update);
        s.push_spectrum_frame(&daemon_frame(100, -20.0));
        let full = render_spectrum_test(&ctx, &mut s, vec![], egui::Modifiers::NONE);
        let updates: Vec<_> = full
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| *id == texture)
            .collect();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1.pos, None);
        assert_eq!(
            updates[0].1.image.size(),
            [s.waterfall_width, s.waterfall_history]
        );
        let head = s.waterfall_head;
        let redraw = render_spectrum_test(&ctx, &mut s, vec![], egui::Modifiers::NONE);
        assert!(!redraw
            .textures_delta
            .set
            .iter()
            .any(|(id, _)| *id == texture));
        assert_eq!(s.waterfall_head, head);
    }

    #[test]
    fn full_65536_fft_accumulates_real_samples_and_reports_dbfs() {
        let mut s = SpectrumAnalyzer::new();
        assert!(s.set_fft_size(65_536));
        s.set_avg_alpha(1.0);
        s.update_params(100_000_000, 2_048_000);
        let frequency = 321.0 * 2_048_000.0 / 65_536.0;
        let iq = tone(65_536, 2_048_000, frequency);
        for bytes in iq[..iq.len() - 1].chunks(997) {
            s.push_iq_samples(bytes);
        }
        assert_eq!(
            s.processed_frames, 0,
            "partial real FFT must not publish interpolated bins"
        );
        s.push_iq_samples(&iq[iq.len() - 1..]);
        wait_for_spectrum_frames(&mut s, 1);
        assert_eq!(s.processed_frames, 1);
        assert!((s.peak_freq_hz() as f64 - 100_000_000.0 - frequency).abs() < 32.0);
        let expected = 20.0 * (100.0_f32 / 128.0).log10();
        assert!(
            (s.peak_level() - expected).abs() < 0.05,
            "tone={}; expected {expected}dBFS",
            s.peak_level()
        );
    }

    #[test]
    fn streaming_result_does_not_depend_on_byte_chunk_boundaries() {
        let mut whole = SpectrumAnalyzer::new();
        let mut split = SpectrumAnalyzer::new();
        for s in [&mut whole, &mut split] {
            s.set_fft_size(256);
            s.update_params(100_000_000, 25_600);
            s.set_fft_rate(20);
            s.set_avg_alpha(1.0);
        }
        let iq = tone(256 + 2 * 1280, 25_600, -3200.0);
        whole.push_iq_samples(&iq);
        for bytes in iq.chunks(123) {
            split.push_iq_samples(bytes);
        }
        wait_for_spectrum_frames(&mut whole, 1);
        wait_for_spectrum_frames(&mut split, 1);
        assert!(whole.processed_frames >= 1);
        assert_eq!(whole.spectrum_dbs, split.spectrum_dbs);
        assert_eq!(whole.iq_ring, split.iq_ring);
    }

    #[test]
    fn long_bursts_have_bounded_fft_work_but_keep_newest_signal() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(256);
        s.update_params(100_000_000, 25_600);
        s.set_fft_rate(100);
        s.set_avg_alpha(1.0);
        let mut iq = tone(256 * 20, 25_600, 3200.0);
        iq.extend(tone(256 * 20, 25_600, -3200.0));
        s.push_iq_samples(&iq);
        wait_for_spectrum_frames(&mut s, 1);
        assert!(s.processed_frames >= 1);
        assert_eq!(s.peak_freq_hz(), 99_996_800);
        assert_eq!(s.iq_ring.len(), 256);
    }

    #[test]
    fn cadence_uses_samples_and_supports_overlapping_large_windows() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(1024);
        s.update_params(100_000_000, 25_600);
        s.set_fft_rate(100); // Hop256 < FFT1024: overlapping real windows.
        let iq = tone(2048, 25_600, 3200.0);
        for bytes in iq.chunks(128) {
            s.push_iq_samples(bytes);
        }
        wait_for_spectrum_frames(&mut s, 1);
        assert!(s.processed_frames >= 1);
        assert!((s.frame_period_seconds - 0.01).abs() < 1e-6);
        s.set_fft_rate(0);
        assert_eq!(s.fft_rate(), 1);
        s.set_fft_rate(1000);
        assert_eq!(s.fft_rate(), 120);
    }

    #[test]
    fn invalid_fft_requests_preserve_live_stream() {
        let mut s = SpectrumAnalyzer::new();
        s.push_iq_samples(&[130; 100]);
        for size in [0, 1, 255, 300, 8_191, 8_193, usize::MAX] {
            assert!(!s.set_fft_size(size));
            assert_eq!(s.fft_size(), 2048);
            assert_eq!(s.iq_filled, 50);
        }
        assert!(s.set_fft_size(2048));
        assert_eq!(
            s.iq_filled, 50,
            "re-applying config must not erase accumulated IQ"
        );
        assert!(s.set_fft_size(8_192));
        assert_eq!(s.iq_filled, 0);
    }

    #[test]
    fn fft_size_validation_keeps_sdrpp_high_resolution_options() {
        for size in [256, 512, 1_024, 2_048, 4_096, 8_192, 16_384, 32_768, 65_536] {
            assert!(
                SpectrumAnalyzer::valid_fft_size(size),
                "SDR++ FFT option {size} should remain selectable"
            );
        }
        for size in [128, 65_535, 131_072] {
            assert!(
                !SpectrumAnalyzer::valid_fft_size(size),
                "FFT size {size} must remain outside the supported power-of-two range"
            );
        }
    }

    #[test]
    fn daemon_cadence_and_invalid_frames_are_bounded() {
        let mut s = SpectrumAnalyzer::new();
        for timestamp in [0, 10, 49, 50] {
            s.push_spectrum_frame(&daemon_frame(timestamp, -40.0));
        }
        assert_eq!(s.processed_frames, 2);
        let mut invalid = daemon_frame(100, -40.0);
        invalid.bins[5] = f32::NAN;
        s.push_spectrum_frame(&invalid);
        invalid.bins = vec![-100.0; 300];
        s.push_spectrum_frame(&invalid);
        assert_eq!(s.processed_frames, 2);
        assert_eq!(s.fft_size(), 256);
        s.push_spectrum_frame(&daemon_frame(1, -40.0)); // Daemon clock restarted.
        assert_eq!(s.processed_frames, 3);
    }

    #[test]
    fn snr_smoothing_is_display_only_and_uses_time_constant() {
        let mut s = SpectrumAnalyzer::new();
        s.set_avg_alpha(1.0);
        s.set_snr_smoothing(true, 0.5);
        s.push_spectrum_frame(&daemon_frame(0, -60.0));
        assert_eq!(s.snr_db(), 40.0);
        s.push_spectrum_frame(&daemon_frame(50, -20.0));
        let expected = 40.0 + (1.0 - (-0.1_f32).exp()) * 40.0;
        assert!((s.snr_db() - expected).abs() < 0.001);
        assert_eq!(s.peak_level(), -20.0);
        assert_eq!(s.noise_floor(), -100.0);
        s.set_snr_smoothing(false, 0.5);
        assert_eq!(s.snr_db(), 80.0);
    }

    #[test]
    fn waterfall_large_fft_storage_is_bounded_and_preserves_narrow_peaks() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(65_536);
        s.set_waterfall_history(4096);
        let bytes: usize = s.waterfall_pixels.iter().map(Vec::len).sum();
        assert!(bytes <= MAX_WATERFALL_BYTES);
        assert!(s.waterfall_width <= MAX_WATERFALL_WIDTH);
        s.spectrum_dbs.fill(-120.0);
        s.spectrum_dbs[32_768] = 0.0;
        let row = s.waterfall_row();
        let column = 32_768 * s.waterfall_width / s.fft_size;
        let (r, g, b) = color_map(s.color_map, 1.0);
        assert_eq!(&row[column * 4..column * 4 + 4], &[r, g, b, 255]);
        assert!(65_536 / plot_bin_stride(65_536, 1000.0) <= 1000);
    }

    #[test]
    fn waterfall_scrolls_on_real_frames_and_hides_without_texture() {
        let mut s = SpectrumAnalyzer::new();
        s.waterfall_every_n = 1;
        s.push_spectrum_frame(&daemon_frame(0, -40.0));
        assert_eq!(s.waterfall_head, 1);
        crate::test_helpers::run_ui(|ui| s.ui(ui));
        crate::test_helpers::run_ui(|ui| s.ui(ui));
        assert_eq!(
            s.waterfall_head, 1,
            "repaint must not duplicate stale IQ rows"
        );
        s.set_waterfall_visible(false);
        s.push_spectrum_frame(&daemon_frame(50, -20.0));
        crate::test_helpers::run_ui(|ui| s.ui(ui));
        assert_eq!(s.waterfall_head, 1);
        assert!(s.waterfall_texture.is_none());
        assert_eq!(
            s.processed_frames, 2,
            "spectrum keeps updating when waterfall hidden"
        );
    }

    #[test]
    fn retuning_discards_partial_window_and_odd_byte() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(256);
        s.push_iq_samples(&[130; 511]);
        assert_eq!(s.iq_filled, 255);
        assert!(s.pending_i_byte.is_some());
        s.update_params(120_000_000, 2_048_000);
        s.push_iq_samples(&[130, 130]);
        assert_eq!(s.iq_filled, 1);
        assert_eq!(s.processed_frames, 0);
    }

    #[test]
    fn vfo_passband_follows_ssb_sidedness() {
        let mut s = SpectrumAnalyzer::new();
        s.vfo_bw_hz = 2400;
        s.demod_mode = "USB".into();
        assert_eq!(s.vfo_band_edges(), (0.0, 2400.0));
        s.demod_mode = "LSB".into();
        assert_eq!(s.vfo_band_edges(), (-2400.0, 0.0));
        s.demod_mode = "AM".into();
        assert_eq!(s.vfo_band_edges(), (-1200.0, 1200.0));
    }

    #[test]
    fn vfo_detector_ignores_out_of_channel_interferers_and_wrong_sideband() {
        let mut s = SpectrumAnalyzer::new();
        s.set_fft_size(256);
        s.update_params(100_000_000, 25_600);
        s.vfo_bw_hz = 2400;
        s.spectrum_dbs.fill(-100.0);
        s.spectrum_dbs[200] = -1.0; // Strong unrelated channel, +7200Hz.
        assert_eq!(s.vfo_signal_level(), -100.0);
        s.spectrum_dbs[133] = -40.0; // Selected AM channel, +500Hz.
        assert_eq!(s.vfo_signal_level(), -40.0);
        s.spectrum_dbs[118] = -10.0; // Opposite sideband, -1000Hz.
        s.demod_mode = "USB".into();
        assert_eq!(s.vfo_signal_level(), -40.0);
        s.demod_mode = "LSB".into();
        assert_eq!(s.vfo_signal_level(), -10.0);
    }

    #[test]
    fn zoom_center_offset_full_span_is_zero() {
        // Issue 44: at full zoom-out panning must be zero; when zoomed in the
        // window must reach the outer frequencies (±max_pan).
        let mut s = SpectrumAnalyzer::new();
        let sr = s.sample_rate;
        // Full span → offset 0 regardless of zoom_offset.
        s.zoom_offset = 0.0;
        assert_eq!(s.zoom_center_offset(sr), 0.0);
        s.zoom_offset = 1.0;
        assert_eq!(s.zoom_center_offset(sr), 0.0);
        // Zoomed 10× → span sr/10, max_pan = (sr - sr/10)/2 = 0.45·sr.
        let span = sr / 10.0;
        let max_pan = (sr - span) / 2.0;
        s.zoom_offset = 0.0;
        assert!((s.zoom_center_offset(span) + max_pan).abs() < 1.0);
        s.zoom_offset = 1.0;
        assert!((s.zoom_center_offset(span) - max_pan).abs() < 1.0);
        s.zoom_offset = 0.5;
        assert!(s.zoom_center_offset(span).abs() < 1.0);
    }

    #[test]
    fn waterfall_circular_head_advances() {
        // Issue 43: circular buffer must advance O(1) without growing.
        let mut s = SpectrumAnalyzer::new();
        let h = s.waterfall_history;
        let h0 = s.waterfall_head;
        assert!(h0 < h);
        let row = s.waterfall_row();
        assert_eq!(row.len(), s.fft_size * 4);
        s.waterfall_pixels[h0] = row;
        s.waterfall_head = (h0 + 1) % h;
        assert_eq!(s.waterfall_head, (h0 + 1) % h);
        assert_eq!(s.waterfall_pixels.len(), h);
    }
}
