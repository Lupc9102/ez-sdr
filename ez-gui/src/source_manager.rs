#![deny(unsafe_op_in_unsafe_fn)]

//! SDR source configuration and management.
//!
//! Provides [`SourceManager`] for enumerating, selecting, and connecting to
//! SoapySDR-compatible devices (RTL-SDR, `HackRF`, Airspy, `LimeSDR`, etc.)
//! as well as file and network IQ sources.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crossbeam_channel::{bounded, Receiver, Sender};

/// Manages the SDR source lifecycle — starting, stopping, and reading IQ samples.
///
/// Supports three modes: real RTL-SDR hardware (behind `feature = "rtlsdr"`),
/// simulated multi-signal IQ generation for demo/testing, and replay from a
/// recorded IQ file.
pub struct SourceManager {
    /// Current source status (Idle / Opening / Running / Error).
    pub status: SourceStatus,
    /// Tuned center frequency in Hz.
    pub frequency_hz: u64,
    /// ADC sample rate in samples-per-second.
    pub sample_rate_hz: u32,
    /// RF gain in dB (typically 0.0–49.6 for RTL-SDR).
    pub gain_db: f64,
    /// Bias-T enable (4.5 V DC on antenna connector, RTL-SDR V3).
    pub bias_tee: bool,
    /// Frequency correction in parts-per-million.
    pub ppm_correction: i32,
    /// Direct-sampling mode for HF reception below 24 MHz.
    pub direct_sampling: bool,
    /// Device temperature in degrees Celsius (if available).
    pub temperature: f32,
    /// Current source operating mode.
    pub source_mode: SourceMode,
    /// Path to an IQ recording file (used in Replay mode).
    pub replay_file: Option<String>,
    /// Whether to loop the replay file when the end is reached.
    pub replay_loop: bool,
    /// Replay speed multiplier (0.1×–10×).
    pub replay_speed: f32,
    /// Current read position within the replay file (bytes).
    pub replay_position: u64,
    /// Total size of the replay file (bytes).
    pub replay_size: u64,
    tx: Option<Sender<Vec<u8>>>,
    rx: Option<Receiver<Vec<u8>>>,
    running: Arc<AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
}

/// The operating mode of the SDR source.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SourceMode {
    /// Generate synthetic IQ data (default; works without real hardware).
    #[default]
    Simulated,
    /// Read IQ samples from a previously recorded file.
    Replay,
}

/// The run-state of the SDR source.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    /// Source is stopped and ready to start.
    Idle,
    /// Source is initializing (device opening, etc.).
    Opening,
    /// Source is actively producing IQ samples.
    Running,
    /// Source encountered a fatal error (carries error message).
    Error(String),
}

impl SourceManager {
    /// Create a new `SourceManager` in `Idle` state with default tuning values.
    pub fn new() -> Self {
        let (tx, rx) = bounded(32);
        Self {
            status: SourceStatus::Idle,
            frequency_hz: 109_000_000,
            sample_rate_hz: 2_048_000,
            gain_db: 40.0,
            bias_tee: false,
            ppm_correction: 0,
            direct_sampling: false,
            temperature: 0.0,
            source_mode: SourceMode::Simulated,
            replay_file: None,
            replay_loop: false,
            replay_speed: 1.0,
            replay_position: 0,
            replay_size: 0,
            tx: Some(tx),
            rx: Some(rx),
            running: Arc::new(AtomicBool::new(false)),
            worker_handle: None,
        }
    }

    /// Start the SDR source.
    ///
    /// Spawns a worker thread that generates or replays IQ samples and sends
    /// them through the internal channel. Switches status to `Running`.
    pub fn start(&mut self) {
        if self.status == SourceStatus::Running {
            return;
        }
        self.status = SourceStatus::Opening;
        // Reuse the existing Arc<AtomicBool> rather than allocating a new one
        // on every start: any previously-detached worker still holds a clone,
        // and reallocating would orphan it (its `running` flag would never be
        // flipped by a later stop). Resetting the shared flag in-place ensures
        // all live and future workers observe the same signal.
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();

        // Recreate channel if needed (after a previous stop)
        let tx = if let Some(tx) = self.tx.take() {
            tx
        } else {
            let (new_tx, new_rx) = bounded(32);
            self.rx = Some(new_rx);
            new_tx
        };

        let freq = self.frequency_hz;
        let rate = self.sample_rate_hz;
        let _ppm = self.ppm_correction;
        let _bias = self.bias_tee;
        let _gain = self.gain_db;
        let source_mode = self.source_mode.clone();
        let replay_file = self.replay_file.clone();
        let replay_loop = self.replay_loop;
        let replay_speed = self.replay_speed;

        let handle = std::thread::spawn(move || {
            match source_mode {
                SourceMode::Replay => {
                    let path = if let Some(p) = replay_file {
                        p
                    } else {
                        let _ = tx.send(b"ERROR".to_vec());
                        return;
                    };
                    let file = if let Ok(f) = std::fs::File::open(&path) {
                        std::io::BufReader::new(f)
                    } else {
                        let _ = tx.send(b"ERROR".to_vec());
                        return;
                    };
                    use std::io::Read;
                    let mut reader = file;
                    let buf_size = 65536;
                    let mut buf = vec![0u8; buf_size];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => {
                                if replay_loop {
                                    let file2 = match std::fs::File::open(&path) {
                                        Ok(f) => std::io::BufReader::new(f),
                                        Err(_) => break,
                                    };
                                    reader = file2;
                                    continue;
                                }
                                break;
                            }
                            Ok(n) => {
                                let chunk = buf[..n].to_vec();
                                if tx.try_send(chunk).is_err() {
                                    break;
                                }
                                let sleep_ms = (n as f64 / (f64::from(rate) * 2.0) * 1000.0
                                    / f64::from(replay_speed))
                                    as u64;
                                std::thread::sleep(std::time::Duration::from_millis(
                                    sleep_ms.max(1),
                                ));
                                if !running.load(Ordering::SeqCst) {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                SourceMode::Simulated => {
                    #[cfg(feature = "rtlsdr")]
                    {
                        // SAFETY: `rtl_sdr_open` is an `unsafe` FFI wrapper but
                        // passes valid arguments to the wrapped C functions and
                        // initialises the device handle on success.
                        let dev = unsafe { rtl_sdr_open(freq, rate, _ppm, _bias, _gain) };
                        if dev.is_null() {
                            let _ = tx.send(b"ERROR".to_vec());
                            return;
                        }
                        let mut buf = vec![0u8; 16384 * 2];
                        while running.load(Ordering::SeqCst) {
                            // SAFETY: `dev` is checked non-null; `buf` is a
                            // mutable Vec with a valid pointer and length.
                            let n = unsafe { rtl_sdr_read_sync(dev, &mut buf) };
                            if n > 0 {
                                let _ = tx.try_send(buf[..n].to_vec());
                            }
                        }
                        // SAFETY: `dev` is non-null and was opened above.
                        unsafe {
                            rtl_sdr_close(dev);
                        }
                    }
                    #[cfg(not(feature = "rtlsdr"))]
                    {
                        // Demo mode: generate realistic multi-signal IQ data
                        let mut phase: f64 = 0.0;
                        let mut burst_phase: f64 = 0.0;
                        let buf_size = 16384;
                        let mut buf = vec![0u8; buf_size];
                        let sample_rate_f = f64::from(rate);
                        let center_freq_f = freq as f64;

                        while running.load(Ordering::SeqCst) {
                            let sleep_ms = (buf_size as f64 / sample_rate_f * 1000.0) as u64;
                            std::thread::sleep(std::time::Duration::from_millis(sleep_ms.max(1)));

                            for i in (0..buf_size).step_by(2) {
                                let t = phase / sample_rate_f;

                                // Noise floor (-80 dB relative)
                                let noise_i = (rand_f64(phase * 137.1) * 6.0 - 3.0) as i16;
                                let noise_q = (rand_f64(phase * 251.7) * 6.0 - 3.0) as i16;

                                // FM broadcast station at center + 200 kHz (-30 dB)
                                let fm_offset = 200_000.0;
                                let fm_phase =
                                    2.0 * std::f64::consts::PI * (center_freq_f + fm_offset) * t;
                                let fm_amp = 25.0;
                                let fm_i = (fm_amp * fm_phase.cos()) as i16;
                                let fm_q = (fm_amp * fm_phase.sin()) as i16;

                                // Narrowband FM signal at center - 100 kHz (-50 dB, intermittent)
                                let nbfm_offset = -100_000.0;
                                let nbfm_phase =
                                    2.0 * std::f64::consts::PI * (center_freq_f + nbfm_offset) * t;
                                let nbfm_env = if (burst_phase * 0.5).sin() > 0.3 {
                                    8.0
                                } else {
                                    0.0
                                };
                                let nbfm_i = (nbfm_env * nbfm_phase.cos()) as i16;
                                let nbfm_q = (nbfm_env * nbfm_phase.sin()) as i16;

                                // AM carrier at center + 50 kHz (-40 dB)
                                let am_offset = 50_000.0;
                                let am_phase =
                                    2.0 * std::f64::consts::PI * (center_freq_f + am_offset) * t;
                                let am_env = 12.0
                                    * (1.0 + 0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin());
                                let am_i = (am_env * am_phase.cos()) as i16;
                                let am_q = (am_env * am_phase.sin()) as i16;

                                // ADS-B-like pulse burst at center (-20 dB, periodic)
                                let pulse_active = (burst_phase * 0.1).sin() > 0.95;
                                let (pulse_i, pulse_q) = if pulse_active {
                                    let pulse_phase =
                                        2.0 * std::f64::consts::PI * center_freq_f * t;
                                    (40.0 * pulse_phase.cos(), 40.0 * pulse_phase.sin())
                                } else {
                                    (0.0, 0.0)
                                };

                                let total_i = noise_i + fm_i + nbfm_i + am_i + pulse_i as i16;
                                let total_q = noise_q + fm_q + nbfm_q + am_q + pulse_q as i16;

                                buf[i] = (i32::from(total_i) + 127).clamp(0, 255) as u8;
                                buf[i + 1] = (i32::from(total_q) + 127).clamp(0, 255) as u8;

                                phase += 1.0;
                                burst_phase += 1.0;
                                if phase >= sample_rate_f * 10.0 {
                                    phase -= sample_rate_f * 10.0;
                                }
                                if burst_phase >= 10000.0 {
                                    burst_phase -= 10000.0;
                                }
                            }

                            let _ = tx.try_send(buf.clone());
                        }
                    }
                }
            }
        });
        self.worker_handle = Some(handle);
        self.tx = None; // tx was moved into the worker thread
        self.status = SourceStatus::Running;
    }

    /// Stop the SDR source.
    ///
    /// Signals the worker thread to exit and recreates the sample channel for
    /// the next `start()` call. Status returns to `Idle`.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.worker_handle.take(); // detach thread — it will exit on next loop check
                                   // Recreate channel for next start()
        let (new_tx, new_rx) = bounded(32);
        self.tx = Some(new_tx);
        self.rx = Some(new_rx);
        self.status = SourceStatus::Idle;
    }

    /// Try to receive a pending chunk of IQ samples from the source thread.
    ///
    /// Returns `None` if no samples are available (non-blocking).
    #[must_use]
    pub fn recv_samples(&self) -> Option<Vec<u8>> {
        if let Some(rx) = &self.rx {
            rx.try_recv().ok()
        } else {
            None
        }
    }

    /// Render the egui-based source control panel (mode, frequency, gain, etc.).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Source");

        // Source mode selection
        ui.horizontal(|ui| {
            ui.label("Mode:");
            let src_label = if cfg!(feature = "rtlsdr") {
                "RTL-SDR"
            } else {
                "Simulated"
            };
            if ui
                .selectable_label(self.source_mode == SourceMode::Simulated, src_label)
                .clicked()
            {
                self.source_mode = SourceMode::Simulated;
            }
            if ui
                .selectable_label(self.source_mode == SourceMode::Replay, "File Replay")
                .clicked()
            {
                self.source_mode = SourceMode::Replay;
            }
        });

        if self.source_mode == SourceMode::Replay {
            ui.separator();
            ui.horizontal(|ui| {
                let mut path = self.replay_file.clone().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut path)
                            .desired_width(300.0)
                            .hint_text("Path to .iq / .bin / .raw file"),
                    )
                    .changed()
                {
                    if path.is_empty() {
                        self.replay_file = None;
                    } else {
                        self.replay_file = Some(path);
                    }
                }
                if ui
                    .button("📂 Browse")
                    .on_hover_text("Open a file picker to select an IQ recording file.")
                    .clicked()
                {
                    if let Some(picked) = rfd::FileDialog::new()
                        .add_filter("IQ files", &["iq", "bin", "raw", "cs8", "cu8", "cf32"])
                        .add_filter("All files", &["*"])
                        .pick_file()
                    {
                        self.replay_file = picked.to_str().map(std::string::ToString::to_string);
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.replay_loop, "Loop");
                ui.label("Speed:");
                ui.add(
                    egui::Slider::new(&mut self.replay_speed, 0.1..=10.0)
                        .text("x")
                        .logarithmic(true),
                );
            });
            if let Some(path) = &self.replay_file {
                ui.label(format!("File: {path}"));
                if self.replay_size > 0 {
                    let mb = self.replay_size as f64 / 1_048_576.0;
                    ui.label(format!("Size: {mb:.1} MB"));
                }
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            let (color, label) = match &self.status {
                SourceStatus::Running => (egui::Color32::GREEN, "Running"),
                SourceStatus::Idle => (egui::Color32::GRAY, "Idle"),
                SourceStatus::Opening => (egui::Color32::YELLOW, "Opening..."),
                SourceStatus::Error(e) => (egui::Color32::RED, e.as_str()),
            };
            ui.colored_label(color, format!("● {label}"));
            if self.replay_position > 0 && self.replay_size > 0 {
                let pct = self.replay_position as f64 / self.replay_size as f64 * 100.0;
                ui.separator();
                ui.label(format!("Pos: {pct:.1}%"));
            }
        });
        ui.add(
            egui::Slider::new(&mut self.frequency_hz, 500_000..=1_770_000_000)
                .text("Frequency (Hz)")
                .custom_formatter(|v, _| format!("{:.3} MHz", v / 1e6)),
        );
        ui.horizontal(|ui| {
            let mut freq_mhz = self.frequency_hz as f64 / 1e6;
            if ui
                .add(
                    egui::DragValue::new(&mut freq_mhz)
                        .speed(0.001)
                        .range(0.5..=1770.0)
                        .prefix("MHz "),
                )
                .changed()
            {
                self.frequency_hz = (freq_mhz * 1e6) as u64;
            }
            if ui.small_button("-1MHz").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_sub(1_000_000).max(500_000);
            }
            if ui.small_button("-100k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_sub(100_000).max(500_000);
            }
            if ui.small_button("-10k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_sub(10_000).max(500_000);
            }
            if ui.small_button("+10k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_add(10_000).min(1_770_000_000);
            }
            if ui.small_button("+100k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_add(100_000).min(1_770_000_000);
            }
            if ui.small_button("+1MHz").clicked() {
                self.frequency_hz = self
                    .frequency_hz
                    .saturating_add(1_000_000)
                    .min(1_770_000_000);
            }
        });
        ui.add(
            egui::Slider::new(&mut self.sample_rate_hz, 225_001..=3_200_000)
                .text("Sample rate (Hz)")
                .custom_formatter(|v, _| format!("{:.3} MSps", v / 1e6)),
        );
        if self.source_mode != SourceMode::Replay {
            ui.horizontal(|ui| {
                ui.label("Gain:");
                ui.add(egui::Slider::new(&mut self.gain_db, 0.0..=49.6).step_by(0.1).text("dB").custom_formatter(|v, _| format!("{v:.1} dB")))
                    .on_hover_text("RF gain in dB. RTL-SDR range: 0–49.6 dB in 0.9 dB steps.");
                ui.horizontal(|ui| {
                    for (label, val, tip) in [
                        ("Auto", 0.0, "Automatic gain control (AGC). Good starting point but can overload with strong signals."),
                        ("Low", 15.0, "~15 dB — use near strong transmitters to avoid overload / intermodulation."),
                        ("Med", 30.0, "~30 dB — good general-purpose starting point for most setups."),
                        ("High", 40.0, "~40 dB — use for weak signals: satellites, distant stations. Watch for overload."),
                        ("Max", 49.6, "49.6 dB maximum gain. Only use with weak signals and quiet RF environment."),
                    ] {
                        if ui.small_button(label).on_hover_text(tip).clicked() {
                            self.gain_db = val;
                        }
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.bias_tee, "Bias Tee (4.5V)")
                    .on_hover_text("Sends 4.5V DC down the coax center pin to power a mast-mounted LNA or filtered LNA. RTL-SDR Blog V3 only. Do NOT enable with passive antennas — it can damage cheap dongles.");
                ui.checkbox(&mut self.direct_sampling, "Direct Sampling")
                    .on_hover_text("Bypasses the RTL-SDR tuner and feeds the ADC directly — enables HF reception below 24 MHz (typically 500 kHz–14 MHz). RTL-SDR V3 only. Reduces sensitivity significantly.");
            });
            ui.add(egui::Slider::new(&mut self.ppm_correction, -100..=100).text("PPM correction"))
                .on_hover_text("Frequency error correction in parts-per-million. RTL-SDR crystals typically drift ±20–50 PPM. At 1090 MHz, 10 PPM = 10.9 kHz error. Tune to a known frequency (FM station, GPS L1) and adjust until it aligns.");
        }
        ui.horizontal(|ui| {
            if ui.button("Start").clicked() {
                self.start();
            }
            if ui.button("Stop").clicked() {
                self.stop();
            }
        });
        if self.temperature > 0.0 {
            ui.label(format!("Temperature: {:.1}°C", self.temperature));
        }
    }
}

/// Simple deterministic pseudo-random (LCG, no `sin()` — which gets slow for large values)
#[cfg(not(feature = "rtlsdr"))]
fn rand_f64(seed: f64) -> f64 {
    let x = seed * 1664525.0 + 1013904223.0;
    let frac = x - (x * (1.0 / 4294967296.0)).floor() * 4294967296.0;
    frac / 4294967296.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_manager_default_new() {
        let sm = SourceManager::new();
        assert_eq!(sm.status, SourceStatus::Idle);
        assert_eq!(sm.frequency_hz, 109_000_000);
        assert_eq!(sm.sample_rate_hz, 2_048_000);
        assert_eq!(sm.gain_db, 40.0);
        assert!(!sm.bias_tee);
        assert_eq!(sm.ppm_correction, 0);
        assert!(!sm.direct_sampling);
        assert_eq!(sm.temperature, 0.0);
        assert_eq!(sm.source_mode, SourceMode::Simulated);
        assert!(sm.replay_file.is_none());
        assert!(!sm.replay_loop);
        assert!((sm.replay_speed - 1.0).abs() < f32::EPSILON);
        assert_eq!(sm.replay_position, 0);
        assert_eq!(sm.replay_size, 0);
        assert!(sm.tx.is_some());
        assert!(sm.rx.is_some());
        assert!(!sm.running.load(Ordering::SeqCst));
        assert!(sm.worker_handle.is_none());
    }

    #[test]
    fn source_mode_default_is_simulated() {
        assert_eq!(SourceMode::default(), SourceMode::Simulated);
    }

    #[test]
    fn source_mode_debug_clone_partial_eq() {
        let a = SourceMode::Simulated;
        let b = SourceMode::Replay;
        assert_eq!(a, a);
        assert_ne!(a, b);
        assert_eq!(format!("{a:?}"), "Simulated");
        assert_eq!(a.clone(), a);
    }

    #[test]
    fn source_status_debug_clone_partial_eq() {
        let idle = SourceStatus::Idle;
        let running = SourceStatus::Running;
        let err1 = SourceStatus::Error("oops".into());
        let err2 = SourceStatus::Error("oops".into());
        assert_eq!(idle, idle);
        assert_ne!(idle, running);
        assert_eq!(err1, err2);
        assert_ne!(err1, idle);
        assert_eq!(idle.clone(), idle);
        assert_eq!(format!("{running:?}"), "Running");
        assert_eq!(format!("{err1:?}"), r#"Error("oops")"#);
    }

    #[test]
    fn stop_on_idle_does_not_panic() {
        let mut sm = SourceManager::new();
        sm.stop();
        assert_eq!(sm.status, SourceStatus::Idle);
    }

    #[test]
    fn recv_samples_returns_none_when_idle() {
        let sm = SourceManager::new();
        assert!(sm.recv_samples().is_none());
    }

    #[test]
    fn start_stop_lifecycle() {
        let mut sm = SourceManager::new();
        sm.start();
        assert_eq!(sm.status, SourceStatus::Running);
        assert!(sm.worker_handle.is_some());
        sm.stop();
        assert_eq!(sm.status, SourceStatus::Idle);
    }

    #[test]
    fn start_idempotent() {
        let mut sm = SourceManager::new();
        sm.start();
        sm.start(); // second start should be a no-op
        assert_eq!(sm.status, SourceStatus::Running);
        sm.stop();
    }

    #[cfg(not(feature = "rtlsdr"))]
    #[test]
    fn rand_f64_deterministic() {
        let a = rand_f64(0.0);
        let b = rand_f64(0.0);
        assert!((a - b).abs() < f64::EPSILON);
        // Range check
        assert!((0.0..1.0).contains(&a));
        // Different seeds yield different values
        let c = rand_f64(1.0);
        assert!((c - a).abs() > 1e-10, "expected different seeds to diverge");
        // Known value from LCG formula
        let known = 1013904223.0 / 4294967296.0;
        assert!((a - known).abs() < 1e-10);
    }
}

// SAFETY: This function calls raw FFI (`rtlsdr_open`, etc.) and must only be
// called when `feature = "rtlsdr"` is active and a real RTL-SDR device is
// available. The caller must ensure the returned pointer is eventually closed
// with `rtl_sdr_close`.
#[cfg(feature = "rtlsdr")]
unsafe fn rtl_sdr_open(
    freq: u64,
    rate: u32,
    ppm: i32,
    bias: bool,
    gain_db: f64,
) -> *mut std::ffi::c_void {
    extern "C" {
        fn rtlsdr_open(dev: *mut *mut std::ffi::c_void, index: u32) -> i32;
        fn rtlsdr_set_center_freq(dev: *mut std::ffi::c_void, freq: u32) -> i32;
        fn rtlsdr_set_sample_rate(dev: *mut std::ffi::c_void, rate: u32) -> i32;
        fn rtlsdr_set_tuner_gain_mode(dev: *mut std::ffi::c_void, manual: i32) -> i32;
        fn rtlsdr_set_tuner_gain(dev: *mut std::ffi::c_void, gain: i32) -> i32;
        fn rtlsdr_set_freq_correction(dev: *mut std::ffi::c_void, ppm: i32) -> i32;
        fn rtlsdr_set_bias_tee(dev: *mut std::ffi::c_void, on: i32) -> i32;
    }
    let mut dev: *mut std::ffi::c_void = std::ptr::null_mut();
    if unsafe { rtlsdr_open(&mut dev, 0) } != 0 {
        return std::ptr::null_mut();
    }
    let cleanup_and_fail = |dev: *mut std::ffi::c_void, what: &str| -> *mut std::ffi::c_void {
        eprintln!("rtlsdr: warning: failed to set {what}; closing device");
        unsafe {
            rtlsdr_close(dev);
        }
        std::ptr::null_mut()
    };
    if unsafe { rtlsdr_set_center_freq(dev, freq as u32) } < 0 {
        return cleanup_and_fail(dev, "center frequency");
    }
    if unsafe { rtlsdr_set_sample_rate(dev, rate) } < 0 {
        return cleanup_and_fail(dev, "sample rate");
    }
    if unsafe { rtlsdr_set_tuner_gain_mode(dev, 1) } < 0 {
        return cleanup_and_fail(dev, "tuner gain mode");
    }
    if unsafe { rtlsdr_set_tuner_gain(dev, (gain_db * 10.0) as i32) } < 0 {
        return cleanup_and_fail(dev, "tuner gain");
    }
    if unsafe { rtlsdr_set_freq_correction(dev, ppm) } < 0 {
        eprintln!("rtlsdr: warning: failed to set frequency correction");
    }
    let bias_on = if bias { 1 } else { 0 };
    if unsafe { rtlsdr_set_bias_tee(dev, bias_on) } < 0 {
        eprintln!("rtlsdr: warning: failed to set bias tee");
    }
    dev
}

// SAFETY: `dev` must be a valid device handle from `rtl_sdr_open`. `buf` must
// be a valid mutable slice. The FFI writes `n_read` bytes into the buffer.
#[cfg(feature = "rtlsdr")]
unsafe fn rtl_sdr_read_sync(dev: *mut std::ffi::c_void, buf: &mut [u8]) -> usize {
    extern "C" {
        fn rtlsdr_read_sync(
            dev: *mut std::ffi::c_void,
            buf: *mut u8,
            len: u32,
            n_read: *mut u32,
        ) -> i32;
    }
    let mut n_read = 0u32;
    unsafe {
        rtlsdr_read_sync(dev, buf.as_mut_ptr(), buf.len() as u32, &mut n_read);
    }
    n_read as usize
}

// SAFETY: `dev` must be a non-null handle from `rtl_sdr_open` that has not
// been closed yet. After this call the handle is invalid.
#[cfg(feature = "rtlsdr")]
unsafe fn rtl_sdr_close(dev: *mut std::ffi::c_void) {
    extern "C" {
        fn rtlsdr_close(dev: *mut std::ffi::c_void) -> i32;
    }
    unsafe {
        rtlsdr_close(dev);
    }
}
