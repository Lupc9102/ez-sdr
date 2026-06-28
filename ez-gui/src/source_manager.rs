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
    tx: Option<Sender<Vec<u8>>>,
    rx: Option<Receiver<Vec<u8>>>,
    running: Arc<AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
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
            tx: Some(tx),
            rx: Some(rx),
            running: Arc::new(AtomicBool::new(false)),
            worker_handle: None,
        }
    }

    pub fn start(&mut self) {
        self.status = SourceStatus::Opening;
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let tx = self.tx.take().unwrap();
        let freq = self.frequency_hz;
        let rate = self.sample_rate_hz;
        let ppm = self.ppm_correction;
        let bias = self.bias_tee;

        let gain = self.gain_db;
        let handle = std::thread::spawn(move || {
            #[cfg(feature = "rtlsdr")]
            {
                let mut dev = unsafe { rtl_sdr_open(freq, rate, ppm, bias, gain) };
                if dev.is_null() {
                    let _ = tx.send(b"ERROR".to_vec());
                    return;
                }
                let mut buf = vec![0u8; 16384 * 2];
                while running.load(Ordering::SeqCst) {
                    let n = unsafe { rtl_sdr_read_sync(dev, &mut buf) };
                    if n > 0 {
                        if tx.try_send(buf[..n].to_vec()).is_err() {
                            // drop oldest
                        }
                    }
                }
                unsafe { rtl_sdr_close(dev); }
            }
            #[cfg(not(feature = "rtlsdr"))]
            {
                let mut phase = 0.0f64;
                let mut buf = vec![0u8; 16384];
                while running.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    for i in (0..buf.len()).step_by(2) {
                        let freq_mhz = freq as f64 / 1e6;
                        let amp = 30.0f64;
                        let noise = ((phase * freq_mhz * 0.001).sin() * amp) as i8;
                        buf[i] = (noise as i16 + 127).max(0).min(255) as u8;
                        buf[i + 1] = (noise as i16 + 127).max(0).min(255) as u8;
                        phase += rate as f64 * 0.001;
                    }
                    if tx.try_send(buf.clone()).is_err() {}
                }
            }
        });
        self.worker_handle = Some(handle);
        self.status = SourceStatus::Running;
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
        self.tx = None;
        self.status = SourceStatus::Idle;
    }

    pub fn poll(&mut self) {
        // Check for samples if needed
    }

    pub fn recv_samples(&self) -> Option<Vec<u8>> {
        if let Some(rx) = &self.rx {
            rx.try_recv().ok()
        } else {
            None
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("RTL-SDR V4 Source");
        ui.horizontal(|ui| {
            let color = match &self.status {
                SourceStatus::Running => egui::Color32::GREEN,
                SourceStatus::Idle => egui::Color32::GRAY,
                SourceStatus::Opening => egui::Color32::YELLOW,
                SourceStatus::Error(_) => egui::Color32::RED,
            };
            ui.colored_label(color, format!("● {:?}", self.status));
        });
        ui.add(egui::Slider::new(&mut self.frequency_hz, 500_000..=1_770_000_000).text("Frequency (Hz)").custom_formatter(|v, _| format!("{:.3} MHz", v / 1e6)));
        ui.add(egui::Slider::new(&mut self.sample_rate_hz, 225_001..=3_200_000).text("Sample rate (Hz)").custom_formatter(|v, _| format!("{:.3} MSps", v / 1e6)));
        ui.horizontal(|ui| {
            ui.label("Gain:");
            ui.add(egui::Slider::new(&mut self.gain_db, 0.0..=49.6).step_by(0.1).text("dB").custom_formatter(|v, _| format!("{:.1} dB", v)));
            if ui.button("Auto").clicked() {
                self.gain_db = 0.0;
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.bias_tee, "Bias Tee (4.5V)");
            ui.checkbox(&mut self.direct_sampling, "Direct Sampling");
        });
        ui.add(egui::Slider::new(&mut self.ppm_correction, -100..=100).text("PPM correction"));
        ui.horizontal(|ui| {
            if ui.button("Start").clicked() { self.start(); }
            if ui.button("Stop").clicked() { self.stop(); }
        });
        if self.temperature > 0.0 {
            ui.label(format!("Temperature: {:.1}°C", self.temperature));
        }
    }
}

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

unsafe fn rtl_sdr_read_sync(dev: *mut std::ffi::c_void, buf: &mut [u8]) -> usize {
    extern "C" {
        fn rtlsdr_read_sync(dev: *mut std::ffi::c_void, buf: *mut u8, len: u32, n_read: *mut u32) -> i32;
    }
    let mut n_read = 0u32;
    rtlsdr_read_sync(dev, buf.as_mut_ptr(), buf.len() as u32, &mut n_read);
    n_read as usize
}

unsafe fn rtl_sdr_close(dev: *mut std::ffi::c_void) {
    extern "C" {
        fn rtlsdr_close(dev: *mut std::ffi::c_void) -> i32;
    }
    rtlsdr_close(dev);
}


