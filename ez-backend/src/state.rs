use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SharedState {
    pub source: SourceState,
    pub spectrum: SpectrumState,
    pub demod_mode: String,
    pub recording: bool,
    pub adsb_running: bool,
    pub selected_satellite: Option<String>,
    pub bookmarks: Vec<Bookmark>,
    pub passes: Vec<PassInfo>,
    pub aircraft: Vec<AircraftInfo>,
    pub record_path: Option<String>,
    pub record_start: Option<std::time::Instant>,
    pub record_bytes: u64,
    pub fft_size: usize,
}

pub struct SourceState {
    pub frequency_hz: u64,
    pub sample_rate_hz: u32,
    pub gain_db: f64,
    pub bias_tee: bool,
    pub running: bool,
}

pub struct SpectrumState {
    pub fft_size: usize,
    pub spectrum_dbs: Vec<f32>,
    pub waterfall_top: Vec<u8>,
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
    pub source: SnapshotSource,
    pub spectrum: Vec<f32>,
    pub waterfall: Vec<u8>,
    pub fft_size: usize,
    pub demod_mode: String,
    pub recording: bool,
    pub adsb_running: bool,
    pub selected_satellite: Option<String>,
    pub bookmarks: Vec<Bookmark>,
    pub passes: Vec<PassInfo>,
    pub aircraft: Vec<AircraftInfo>,
    pub record_bytes: u64,
    pub record_secs: u64,
    pub center_freq_hz: u64,
    pub sample_rate_hz: u32,
}

#[derive(Serialize, Clone)]
pub struct SnapshotSource {
    pub frequency_hz: u64,
    pub sample_rate_hz: u32,
    pub gain_db: f64,
    pub bias_tee: bool,
    pub running: bool,
    pub status: String,
}

#[derive(Serialize, Clone)]
pub struct Bookmark {
    pub name: String,
    pub frequency_hz: u64,
    pub mode: String,
    pub category: String,
}

#[derive(Serialize, Clone)]
pub struct PassInfo {
    pub satellite: String,
    pub aos: String,
    pub los: String,
    pub max_elevation: f64,
    pub frequency_hz: u64,
}

#[derive(Serialize, Clone)]
pub struct AircraftInfo {
    pub icao: u32,
    pub callsign: String,
    pub lat: f64,
    pub lon: f64,
    pub altitude: u32,
    pub speed: u32,
    pub heading: u32,
    pub age_secs: u64,
}

impl SourceState {
    pub fn start(&mut self) {
        self.running = true;
    }
    pub fn stop(&mut self) {
        self.running = false;
    }
}

impl SharedState {
    pub fn new() -> Self {
        let fft_size = 2048;
        Self {
            source: SourceState {
                frequency_hz: 100_000_000,
                sample_rate_hz: 2_048_000,
                gain_db: 40.0,
                bias_tee: false,
                running: true,
            },
            spectrum: SpectrumState {
                fft_size,
                spectrum_dbs: vec![-80.0; fft_size],
                waterfall_top: vec![0u8; fft_size * 4],
            },
            demod_mode: "FM".into(),
            recording: false,
            adsb_running: false,
            selected_satellite: None,
            bookmarks: builtin_bookmarks(),
            passes: builtin_passes(),
            aircraft: vec![],
            record_path: None,
            record_start: None,
            record_bytes: 0,
            fft_size,
        }
    }

    pub fn push_iq(&mut self, iq: &[u8]) {
        use num_complex::Complex32;
        use rustfft::Fft;

        let n = (iq.len() / 2).min(self.fft_size);
        if n == 0 { return; }

        let mut buf: Vec<Complex32> = (0..n).map(|i| {
            let re = iq[2 * i] as f32 - 127.5;
            let im = iq[2 * i + 1] as f32 - 127.5;
            Complex32::new(re, im)
        }).collect();

        let mut planner = rustfft::FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        fft.process(&mut buf);

        let scale = 1.0 / n as f32;
        for i in 0..n {
            let mag = buf[i].norm() * scale;
            let db = if mag > 1e-10 { 20.0 * mag.log10() } else { -120.0 };
            let alpha = 0.3;
            self.spectrum.spectrum_dbs[i] = alpha * db + (1.0 - alpha) * self.spectrum.spectrum_dbs[i];
        }

        // Waterfall row
        let mut row = Vec::with_capacity(n * 4);
        for i in 0..n {
            let norm = ((self.spectrum.spectrum_dbs[i] + 100.0) / 80.0).clamp(0.0, 1.0);
            let (r, g, b) = spectrum_color(norm);
            row.extend_from_slice(&[r, g, b, 255]);
        }
        self.spectrum.waterfall_top = row;
    }

    pub fn snapshot(&mut self) -> Snapshot {
        let record_secs = self.record_start
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);

        Snapshot {
            source: SnapshotSource {
                frequency_hz: self.source.frequency_hz,
                sample_rate_hz: self.source.sample_rate_hz,
                gain_db: self.source.gain_db,
                bias_tee: self.source.bias_tee,
                running: self.source.running,
                status: if self.source.running { "Running" } else { "Idle" }.into(),
            },
            spectrum: self.spectrum.spectrum_dbs.clone(),
            waterfall: self.spectrum.waterfall_top.clone(),
            fft_size: self.fft_size,
            demod_mode: self.demod_mode.clone(),
            recording: self.recording,
            adsb_running: self.adsb_running,
            selected_satellite: self.selected_satellite.clone(),
            bookmarks: self.bookmarks.clone(),
            passes: self.passes.clone(),
            aircraft: self.aircraft.clone(),
            record_bytes: self.record_bytes,
            record_secs,
            center_freq_hz: self.source.frequency_hz,
            sample_rate_hz: self.source.sample_rate_hz,
        }
    }

    pub fn start_recording(&mut self) {
        let dir = std::path::Path::new("./recordings");
        let _ = std::fs::create_dir_all(dir);
        let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let freq_mhz = self.source.frequency_hz as f64 / 1e6;
        let path = dir.join(format!("{}_{:.1}MHz.iq", ts, freq_mhz));
        self.record_path = Some(path.to_string_lossy().to_string());
        self.record_start = Some(std::time::Instant::now());
        self.record_bytes = 0;
        self.recording = true;
    }

    pub fn stop_recording(&mut self) {
        self.recording = false;
        self.record_path = None;
        self.record_start = None;
    }

    pub fn start_adsb(&mut self) {
        self.adsb_running = true;
    }

    pub fn stop_adsb(&mut self) {
        self.adsb_running = false;
    }

    pub fn refresh_bookmarks(&mut self) {
        self.passes = builtin_passes();
    }
}

fn spectrum_color(norm: f32) -> (u8, u8, u8) {
    if norm < 0.25 {
        let t = norm / 0.25;
        ((t * 80.0) as u8, 0, (128.0 + t * 127.0) as u8)
    } else if norm < 0.5 {
        let t = (norm - 0.25) / 0.25;
        (0, (t * 200.0) as u8, (255.0 - t * 155.0) as u8)
    } else if norm < 0.75 {
        let t = (norm - 0.5) / 0.25;
        ((t * 255.0) as u8, (200.0 + t * 55.0) as u8, (100.0 - t * 100.0) as u8)
    } else {
        let t = (norm - 0.75) / 0.25;
        (255, (255.0 - t * 100.0) as u8, 0)
    }
}

fn builtin_bookmarks() -> Vec<Bookmark> {
    vec![
        Bookmark { name: "NOAA 15 APT".into(), frequency_hz: 137_620_000, mode: "WFM".into(), category: "Weather".into() },
        Bookmark { name: "NOAA 18 APT".into(), frequency_hz: 137_912_500, mode: "WFM".into(), category: "Weather".into() },
        Bookmark { name: "NOAA 19 APT".into(), frequency_hz: 137_100_000, mode: "WFM".into(), category: "Weather".into() },
        Bookmark { name: "Meteor-M2-2 LRPT".into(), frequency_hz: 137_100_000, mode: "WFM".into(), category: "Weather".into() },
        Bookmark { name: "ADS-B 1090".into(), frequency_hz: 1_090_000_000, mode: "RAW".into(), category: "Aviation".into() },
        Bookmark { name: "Airband VHF".into(), frequency_hz: 118_000_000, mode: "AM".into(), category: "Aviation".into() },
        Bookmark { name: "FM Radio".into(), frequency_hz: 100_000_000, mode: "WFM".into(), category: "Broadcast".into() },
        Bookmark { name: "ISS Voice".into(), frequency_hz: 145_800_000, mode: "NFM".into(), category: "Space".into() },
        Bookmark { name: "Ham 2m".into(), frequency_hz: 145_500_000, mode: "NFM".into(), category: "Ham".into() },
        Bookmark { name: "Ham 70cm".into(), frequency_hz: 435_000_000, mode: "NFM".into(), category: "Ham".into() },
        Bookmark { name: "LoRa 868".into(), frequency_hz: 868_100_000, mode: "RAW".into(), category: "IoT".into() },
        Bookmark { name: "ISM 433".into(), frequency_hz: 433_920_000, mode: "RAW".into(), category: "IoT".into() },
    ]
}

fn builtin_passes() -> Vec<PassInfo> {
    vec![
        PassInfo { satellite: "NOAA 19".into(), aos: "14:23 UTC".into(), los: "14:38 UTC".into(), max_elevation: 67.0, frequency_hz: 137_100_000 },
        PassInfo { satellite: "ISS".into(), aos: "15:10 UTC".into(), los: "15:16 UTC".into(), max_elevation: 45.0, frequency_hz: 145_800_000 },
        PassInfo { satellite: "NOAA 18".into(), aos: "16:45 UTC".into(), los: "17:00 UTC".into(), max_elevation: 32.0, frequency_hz: 137_912_500 },
        PassInfo { satellite: "Meteor-M2-2".into(), aos: "18:02 UTC".into(), los: "18:14 UTC".into(), max_elevation: 55.0, frequency_hz: 137_100_000 },
        PassInfo { satellite: "NOAA 15".into(), aos: "19:30 UTC".into(), los: "19:45 UTC".into(), max_elevation: 28.0, frequency_hz: 137_620_000 },
    ]
}
