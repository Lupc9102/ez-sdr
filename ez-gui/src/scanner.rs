use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::app::SharedState;

#[derive(Debug, Clone)]
pub struct SignalHit {
    pub freq_hz: u64,
    pub strength_db: f32,
    pub timestamp: Instant,
}

pub struct FrequencyScanner {
    #[allow(dead_code)]
    shared: Arc<Mutex<SharedState>>,
    pub enabled: bool,
    pub reset_on_start: bool,
    pub start_hz: u64,
    pub stop_hz: u64,
    pub step_hz: u64,
    pub dwell_ms: u64,
    pub threshold_db: f32,
    pub current_freq_hz: u64,
    last_step_time: Option<Instant>,
    pub hits: Vec<SignalHit>,
    pub max_hits: usize,
    pub status_text: String,
    pub progress: f32,
    pub last_peak_db: f32,
    pub tune_request_hz: Option<u64>,
}

impl FrequencyScanner {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            enabled: false,
            reset_on_start: true,
            start_hz: 88_000_000,
            stop_hz: 108_000_000,
            step_hz: 100_000,
            dwell_ms: 500,
            threshold_db: -60.0,
            current_freq_hz: 100_000_000,
            last_step_time: None,
            hits: Vec::new(),
            max_hits: 200,
            status_text: "Idle".into(),
            progress: 0.0,
            last_peak_db: -120.0,
            tune_request_hz: None,
        }
    }

    pub fn start(&mut self) {
        self.enabled = true;
        if self.reset_on_start {
            self.hits.clear();
        }
        self.current_freq_hz = self.start_hz;
        self.last_step_time = Some(Instant::now());
        self.tune_request_hz = Some(self.current_freq_hz);
        self.status_text = format!(
            "Scanning {:.3}–{:.3} MHz",
            self.start_hz as f64 / 1e6,
            self.stop_hz as f64 / 1e6
        );
    }

    pub fn stop(&mut self) {
        self.enabled = false;
        self.status_text = format!("Stopped ({} signals)", self.hits.len());
    }

    pub fn tick(&mut self, spectrum_peak_db: f32) {
        self.last_peak_db = spectrum_peak_db;
        if !self.enabled || self.step_hz == 0 {
            return;
        }
        let now = Instant::now();
        let dwell = Duration::from_millis(self.dwell_ms);
        let elapsed = self.last_step_time.map(|t| now.duration_since(t)).unwrap_or(dwell);
        if elapsed < dwell {
            return;
        }

        if spectrum_peak_db > self.threshold_db
            && self.current_freq_hz >= self.start_hz
            && self.current_freq_hz <= self.stop_hz
        {
            let hit = SignalHit {
                freq_hz: self.current_freq_hz,
                strength_db: spectrum_peak_db,
                timestamp: now,
            };
            self.hits.push(hit);
            if self.hits.len() > self.max_hits {
                self.hits.remove(0);
            }
        }

        let next = self.current_freq_hz.saturating_add(self.step_hz);
        if next > self.stop_hz {
            self.current_freq_hz = self.start_hz;
        } else {
            self.current_freq_hz = next;
        }
        self.tune_request_hz = Some(self.current_freq_hz);

        let span = self.stop_hz.saturating_sub(self.start_hz).max(1) as f32;
        let pos = self.current_freq_hz.saturating_sub(self.start_hz) as f32;
        self.progress = (pos / span).clamp(0.0, 1.0);
        self.last_step_time = Some(now);
    }

    pub fn sort_hits_by_strength(&mut self) {
        self.hits.sort_by(|a, b| b.strength_db.partial_cmp(&a.strength_db).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Frequency Scanner");

        ui.horizontal(|ui| {
            if self.enabled {
                if ui.button("⏹ Stop").clicked() {
                    self.stop();
                }
            } else if ui.button("▶ Start Scan").clicked() {
                self.start();
            }
            if ui.button("Sort by strength").clicked() {
                self.sort_hits_by_strength();
            }
            if ui.button("Clear hits").clicked() {
                self.hits.clear();
            }
            ui.separator();
            let color = if self.enabled { egui::Color32::GREEN } else { egui::Color32::GRAY };
            ui.colored_label(color, &self.status_text);
        });

        ui.separator();

        egui::Grid::new("scanner_controls").num_columns(2).show(ui, |ui| {
            ui.label("Start (MHz):");
            let mut start = self.start_hz as f64 / 1e6;
            if ui.add(egui::DragValue::new(&mut start).speed(0.01).range(0.5..=1770.0).suffix(" MHz")).changed() {
                self.start_hz = (start * 1e6) as u64;
                if self.start_hz > self.stop_hz { self.stop_hz = self.start_hz; }
            }
            ui.end_row();

            ui.label("Stop (MHz):");
            let mut stop = self.stop_hz as f64 / 1e6;
            if ui.add(egui::DragValue::new(&mut stop).speed(0.01).range(0.5..=1770.0).suffix(" MHz")).changed() {
                self.stop_hz = (stop * 1e6) as u64;
                if self.stop_hz < self.start_hz { self.start_hz = self.stop_hz; }
            }
            ui.end_row();

            ui.label("Step:");
            ui.horizontal(|ui| {
                let presets = [1_000u64, 10_000, 100_000, 250_000, 1_000_000];
                for p in presets {
                    if ui.selectable_label(self.step_hz == p, match p {
                        1_000 => "1k",
                        10_000 => "10k",
                        100_000 => "100k",
                        250_000 => "250k",
                        _ => "1M",
                    }).clicked() {
                        self.step_hz = p;
                    }
                }
            });
            ui.end_row();

            ui.label("Step (Hz):");
            ui.add(egui::DragValue::new(&mut self.step_hz).speed(1000.0).range(100..=10_000_000));
            ui.end_row();

            ui.label("Dwell (ms):");
            ui.add(egui::Slider::new(&mut self.dwell_ms, 50..=5000));
            ui.end_row();

            ui.label("Threshold (dB):");
            ui.add(egui::Slider::new(&mut self.threshold_db, -120.0..=0.0));
            ui.end_row();

            ui.label("Progress:");
            ui.add(egui::ProgressBar::new(self.progress).show_percentage().desired_width(200.0));
            ui.end_row();

            ui.label("Current:");
            ui.colored_label(
                if self.last_peak_db > self.threshold_db { egui::Color32::GREEN } else { egui::Color32::GRAY },
                format!("{:.1} dB", self.last_peak_db)
            );
            ui.end_row();
        });

        ui.checkbox(&mut self.reset_on_start, "Reset hits on start");

        ui.separator();
        ui.label(format!("Signals: {}", self.hits.len()));

        let hit_color = |db: f32| -> egui::Color32 {
            if db > -20.0 { egui::Color32::GREEN }
            else if db > -40.0 { egui::Color32::YELLOW }
            else if db > -60.0 { egui::Color32::from_rgb(200, 150, 50) }
            else { egui::Color32::GRAY }
        };

        egui::ScrollArea::vertical().max_height(400.0).auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("hits_grid")
                .num_columns(5)
                .striped(true)
                .min_col_width(55.0)
                .show(ui, |ui| {
                    ui.strong("Freq");
                    ui.strong("Strength");
                    ui.strong("Time Ago");
                    ui.strong("Tune");
                    ui.strong("Del");
                    ui.end_row();

                    let hits_copy = self.hits.clone();
                    let mut remove_idx = None;
                    for (i, hit) in hits_copy.iter().enumerate() {
                        ui.monospace(format!("{:.3} MHz", hit.freq_hz as f64 / 1e6));
                        ui.colored_label(hit_color(hit.strength_db), format!("{:.1} dB", hit.strength_db));
                        let ago = hit.timestamp.elapsed().as_secs();
                        ui.label(if ago < 60 { format!("{}s", ago) } else { format!("{}m", ago / 60) });
                        if ui.small_button("📡").clicked() {
                            self.tune_request_hz = Some(hit.freq_hz);
                        }
                        if ui.small_button("✕").clicked() {
                            remove_idx = Some(i);
                        }
                        ui.end_row();
                    }
                    if let Some(idx) = remove_idx {
                        if idx < self.hits.len() {
                            self.hits.remove(idx);
                        }
                    }
                });
        });
    }
}
