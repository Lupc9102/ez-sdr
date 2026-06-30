use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use crossbeam_channel::{bounded, Receiver, Sender};

pub struct SourceManager {
    pub status: SourceStatus,
    pub frequency_hz: u64,
    pub sample_rate_hz: u32,
    pub gain_db: f64,
    pub bias_tee: bool,
    pub ppm_correction: i32,
    pub direct_sampling: bool,
    pub temperature: f32,
    pub source_mode: SourceMode,
    pub replay_file: Option<String>,
    pub replay_loop: bool,
    pub replay_speed: f32,
    pub replay_position: u64,
    pub replay_size: u64,
    tx: Option<Sender<Vec<u8>>>,
    rx: Option<Receiver<Vec<u8>>>,
    running: Arc<AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceMode {
    Simulated,
    Replay,
}

impl Default for SourceMode {
    fn default() -> Self { SourceMode::Simulated }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    Idle,
    Opening,
    Running,
    Error(String),
}

impl SourceManager {
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

    pub fn start(&mut self) {
        if self.status == SourceStatus::Running {
            return;
        }
        self.status = SourceStatus::Opening;
        self.running = Arc::new(AtomicBool::new(true));
        let running = self.running.clone();

        // Recreate channel if needed (after a previous stop)
        let tx = match self.tx.take() {
            Some(tx) => tx,
            None => {
                let (new_tx, new_rx) = bounded(32);
                self.rx = Some(new_rx);
                new_tx
            }
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
                    let path = match replay_file {
                        Some(p) => p,
                        None => {
                            let _ = tx.send(b"ERROR".to_vec());
                            return;
                        }
                    };
                    let file = match std::fs::File::open(&path) {
                        Ok(f) => std::io::BufReader::new(f),
                        Err(_) => {
                            let _ = tx.send(b"ERROR".to_vec());
                            return;
                        }
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
                                let sleep_ms = (n as f64 / (rate as f64 * 2.0) * 1000.0 / replay_speed as f64) as u64;
                                std::thread::sleep(std::time::Duration::from_millis(sleep_ms.max(1)));
                                if !running.load(Ordering::SeqCst) { break; }
                            }
                            Err(_) => break,
                        }
                    }
                }
                SourceMode::Simulated => {
                    #[cfg(feature = "rtlsdr")]
                    {
                        let mut dev = unsafe { rtl_sdr_open(freq, rate, _ppm, _bias, _gain) };
                        if dev.is_null() {
                            let _ = tx.send(b"ERROR".to_vec());
                            return;
                        }
                        let mut buf = vec![0u8; 16384 * 2];
                        while running.load(Ordering::SeqCst) {
                            let n = unsafe { rtl_sdr_read_sync(dev, &mut buf) };
                            if n > 0 {
                                let _ = tx.try_send(buf[..n].to_vec());
                            }
                        }
                        unsafe { rtl_sdr_close(dev); }
                    }
                    #[cfg(not(feature = "rtlsdr"))]
                    {
                        // Demo mode: generate realistic multi-signal IQ data
                        let mut phase: f64 = 0.0;
                        let mut burst_phase: f64 = 0.0;
                        let buf_size = 16384;
                        let mut buf = vec![0u8; buf_size];
                        let sample_rate_f = rate as f64;
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
                                let fm_phase = 2.0 * std::f64::consts::PI * (center_freq_f + fm_offset) * t;
                                let fm_amp = 25.0;
                                let fm_i = (fm_amp * fm_phase.cos()) as i16;
                                let fm_q = (fm_amp * fm_phase.sin()) as i16;

                                // Narrowband FM signal at center - 100 kHz (-50 dB, intermittent)
                                let nbfm_offset = -100_000.0;
                                let nbfm_phase = 2.0 * std::f64::consts::PI * (center_freq_f + nbfm_offset) * t;
                                let nbfm_env = if (burst_phase * 0.5).sin() > 0.3 { 8.0 } else { 0.0 };
                                let nbfm_i = (nbfm_env * nbfm_phase.cos()) as i16;
                                let nbfm_q = (nbfm_env * nbfm_phase.sin()) as i16;

                                // AM carrier at center + 50 kHz (-40 dB)
                                let am_offset = 50_000.0;
                                let am_phase = 2.0 * std::f64::consts::PI * (center_freq_f + am_offset) * t;
                                let am_env = 12.0 * (1.0 + 0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin());
                                let am_i = (am_env * am_phase.cos()) as i16;
                                let am_q = (am_env * am_phase.sin()) as i16;

                                // ADS-B-like pulse burst at center (-20 dB, periodic)
                                let pulse_active = (burst_phase * 0.1).sin() > 0.95;
                                let (pulse_i, pulse_q) = if pulse_active {
                                    let pulse_phase = 2.0 * std::f64::consts::PI * center_freq_f * t;
                                    (40.0 * pulse_phase.cos(), 40.0 * pulse_phase.sin())
                                } else {
                                    (0.0, 0.0)
                                };

                                let total_i = noise_i + fm_i + nbfm_i + am_i + pulse_i as i16;
                                let total_q = noise_q + fm_q + nbfm_q + am_q + pulse_q as i16;

                                buf[i] = (total_i as i32 + 127).clamp(0, 255) as u8;
                                buf[i + 1] = (total_q as i32 + 127).clamp(0, 255) as u8;

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

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.worker_handle.take(); // detach thread — it will exit on next loop check
        // Recreate channel for next start()
        let (new_tx, new_rx) = bounded(32);
        self.tx = Some(new_tx);
        self.rx = Some(new_rx);
        self.status = SourceStatus::Idle;
    }

    pub fn recv_samples(&self) -> Option<Vec<u8>> {
        if let Some(rx) = &self.rx {
            rx.try_recv().ok()
        } else {
            None
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Source");

        // Source mode selection
        ui.horizontal(|ui| {
            ui.label("Mode:");
            if ui.selectable_label(self.source_mode == SourceMode::Simulated, "Simulated").clicked() {
                self.source_mode = SourceMode::Simulated;
            }
            if ui.selectable_label(self.source_mode == SourceMode::Replay, "File Replay").clicked() {
                self.source_mode = SourceMode::Replay;
            }
        });

        if self.source_mode == SourceMode::Replay {
            ui.separator();
            ui.horizontal(|ui| {
                let mut path = self.replay_file.clone().unwrap_or_default();
                if ui.add(egui::TextEdit::singleline(&mut path).desired_width(300.0).hint_text("Path to .iq / .bin / .raw file")).changed() {
                    if !path.is_empty() {
                        self.replay_file = Some(path);
                    } else {
                        self.replay_file = None;
                    }
                }
                if ui.button("📂 Browse").on_hover_text("Open a file picker to select an IQ recording file.").clicked() {
                    if let Some(picked) = rfd::FileDialog::new()
                        .add_filter("IQ files", &["iq", "bin", "raw", "cs8", "cu8", "cf32"])
                        .add_filter("All files", &["*"])
                        .pick_file()
                    {
                        self.replay_file = picked.to_str().map(|s| s.to_string());
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.replay_loop, "Loop");
                ui.label("Speed:");
                ui.add(egui::Slider::new(&mut self.replay_speed, 0.1..=10.0).text("x").logarithmic(true));
            });
            if let Some(path) = &self.replay_file {
                ui.label(format!("File: {}", path));
                if self.replay_size > 0 {
                    let mb = self.replay_size as f64 / 1_048_576.0;
                    ui.label(format!("Size: {:.1} MB", mb));
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
            ui.colored_label(color, format!("● {}", label));
            if self.replay_position > 0 && self.replay_size > 0 {
                let pct = self.replay_position as f64 / self.replay_size as f64 * 100.0;
                ui.separator();
                ui.label(format!("Pos: {:.1}%", pct));
            }
        });
        ui.add(egui::Slider::new(&mut self.frequency_hz, 500_000..=1_770_000_000).text("Frequency (Hz)").custom_formatter(|v, _| format!("{:.3} MHz", v / 1e6)));
        ui.horizontal(|ui| {
            let mut freq_mhz = self.frequency_hz as f64 / 1e6;
            if ui.add(egui::DragValue::new(&mut freq_mhz).speed(0.001).range(0.5..=1770.0).prefix("MHz "))
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
                self.frequency_hz = self.frequency_hz.saturating_add(1_000_000).min(1_770_000_000);
            }
        });
        ui.add(egui::Slider::new(&mut self.sample_rate_hz, 225_001..=3_200_000).text("Sample rate (Hz)").custom_formatter(|v, _| format!("{:.3} MSps", v / 1e6)));
        if self.source_mode != SourceMode::Replay {
            ui.horizontal(|ui| {
                ui.label("Gain:");
                ui.add(egui::Slider::new(&mut self.gain_db, 0.0..=49.6).step_by(0.1).text("dB").custom_formatter(|v, _| format!("{:.1} dB", v)))
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
            if ui.button("Start").clicked() { self.start(); }
            if ui.button("Stop").clicked() { self.stop(); }
        });
        if self.temperature > 0.0 {
            ui.label(format!("Temperature: {:.1}°C", self.temperature));
        }
    }
}

/// Simple deterministic pseudo-random (LCG, no sin() — which gets slow for large values)
fn rand_f64(seed: f64) -> f64 {
    let x = seed * 1664525.0 + 1013904223.0;
    let frac = x - (x * (1.0 / 4294967296.0)).floor() * 4294967296.0;
    frac / 4294967296.0
}

#[cfg(feature = "rtlsdr")]
unsafe fn rtl_sdr_open(freq: u64, rate: u32, ppm: i32, bias: bool, gain_db: f64) -> *mut std::ffi::c_void {
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
    if rtlsdr_open(&mut dev, 0) != 0 { return std::ptr::null_mut(); }
    rtlsdr_set_center_freq(dev, freq as u32);
    rtlsdr_set_sample_rate(dev, rate);
    rtlsdr_set_tuner_gain_mode(dev, 1);
    rtlsdr_set_tuner_gain(dev, (gain_db * 10.0) as i32);
    rtlsdr_set_freq_correction(dev, ppm);
    rtlsdr_set_bias_tee(dev, if bias { 1 } else { 0 });
    dev
}

#[cfg(feature = "rtlsdr")]
unsafe fn rtl_sdr_read_sync(dev: *mut std::ffi::c_void, buf: &mut [u8]) -> usize {
    extern "C" {
        fn rtlsdr_read_sync(dev: *mut std::ffi::c_void, buf: *mut u8, len: u32, n_read: *mut u32) -> i32;
    }
    let mut n_read = 0u32;
    rtlsdr_read_sync(dev, buf.as_mut_ptr(), buf.len() as u32, &mut n_read);
    n_read as usize
}

#[cfg(feature = "rtlsdr")]
unsafe fn rtl_sdr_close(dev: *mut std::ffi::c_void) {
    extern "C" {
        fn rtlsdr_close(dev: *mut std::ffi::c_void) -> i32;
    }
    rtlsdr_close(dev);
}
