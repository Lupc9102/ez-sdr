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
    pub paused: bool,
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
    pub auto_tune_on_hit: bool,
    pub last_export_msg: String,
    scan_start_time: Option<Instant>,
    total_hits_logged: u64,
    pub hit_flash: u32,
    show_histogram: bool,
    pub hold_on_active: bool,
    pub hold_resume_delay_ms: u64,
    holding: bool,
    hold_last_active: Option<Instant>,
    pub spectrum_visible_range: Option<(u64, u64)>,
}

impl FrequencyScanner {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            enabled: false,
            paused: false,
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
            auto_tune_on_hit: false,
            last_export_msg: String::new(),
            scan_start_time: None,
            total_hits_logged: 0,
            hit_flash: 0,
            show_histogram: true,
            hold_on_active: false,
            hold_resume_delay_ms: 1500,
            holding: false,
            hold_last_active: None,
            spectrum_visible_range: None,
        }
    }

    pub fn export_hits_csv(&mut self) {
        if self.hits.is_empty() {
            self.last_export_msg = "No hits to export.".to_string();
            return;
        }
        let default_name = format!("scanner_hits_{}.csv", chrono::Local::now().format("%Y%m%d_%H%M%S"));
        let path = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("CSV", &["csv"])
            .save_file();
        let path = match path {
            Some(p) => p,
            None => { self.last_export_msg = "Export cancelled.".to_string(); return; }
        };

        // Group hits by frequency: collect max strength and hit count
        let mut grouped: std::collections::BTreeMap<u64, (f32, u32)> = std::collections::BTreeMap::new();
        for hit in &self.hits {
            let entry = grouped.entry(hit.freq_hz).or_insert((-200.0, 0));
            if hit.strength_db > entry.0 { entry.0 = hit.strength_db; }
            entry.1 += 1;
        }

        let mut csv = String::from("Frequency_Hz,Frequency_MHz,Max_Strength_dB,Hit_Count\n");
        for (freq_hz, (max_db, count)) in &grouped {
            csv.push_str(&format!(
                "{},{:.4},{:.1},{}\n",
                freq_hz,
                *freq_hz as f64 / 1e6,
                max_db,
                count
            ));
        }
        match std::fs::write(&path, &csv) {
            Ok(_) => self.last_export_msg = format!("Exported {} frequencies ({} hits) to {}", grouped.len(), self.hits.len(), path.display()),
            Err(e) => self.last_export_msg = format!("Export failed: {}", e),
        }
    }

    pub fn start(&mut self) {
        self.enabled = true;
        if self.reset_on_start {
            self.hits.clear();
            self.total_hits_logged = 0;
        }
        self.scan_start_time = Some(Instant::now());
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

    pub fn pause(&mut self) {
        self.paused = true;
        self.status_text = format!("Paused at {:.3} MHz ({} signals)", self.current_freq_hz as f64 / 1e6, self.hits.len());
    }

    pub fn resume(&mut self) {
        self.paused = false;
        self.status_text = format!("Scanning {:.3}–{:.3} MHz", self.start_hz as f64 / 1e6, self.stop_hz as f64 / 1e6);
    }

    pub fn tick(&mut self, spectrum_peak_db: f32) {
        self.last_peak_db = spectrum_peak_db;
        if !self.enabled || self.step_hz == 0 || self.paused {
            return;
        }
        let now = Instant::now();
        let dwell = Duration::from_millis(self.dwell_ms);
        let elapsed = self.last_step_time.map(|t| now.duration_since(t)).unwrap_or(dwell);
        if elapsed < dwell {
            return;
        }

        let signal_active = spectrum_peak_db > self.threshold_db
            && self.current_freq_hz >= self.start_hz
            && self.current_freq_hz <= self.stop_hz;

        if signal_active {
            // Log the hit (dedup within ±step_hz)
            let half_step = self.step_hz / 2;
            let existing = self.hits.iter_mut().find(|h| {
                let diff = if h.freq_hz > self.current_freq_hz {
                    h.freq_hz - self.current_freq_hz
                } else {
                    self.current_freq_hz - h.freq_hz
                };
                diff <= half_step
            });
            if let Some(hit) = existing {
                if spectrum_peak_db > hit.strength_db {
                    hit.strength_db = spectrum_peak_db;
                    hit.freq_hz = self.current_freq_hz;
                    hit.timestamp = now;
                }
            } else {
                let hit = SignalHit {
                    freq_hz: self.current_freq_hz,
                    strength_db: spectrum_peak_db,
                    timestamp: now,
                };
                if self.auto_tune_on_hit {
                    self.tune_request_hz = Some(self.current_freq_hz);
                }
                self.hits.push(hit);
                self.total_hits_logged += 1;
                self.hit_flash = 45;
                if self.hits.len() > self.max_hits {
                    self.hits.remove(0);
                }
            }

            // Hold: stay on this frequency while signal is active
            if self.hold_on_active {
                self.hold_last_active = Some(now);
                self.holding = true;
                self.last_step_time = Some(now); // keep dwell timer alive
                self.status_text = format!("⏸ Holding {:.3} MHz ({:.0} dB)", self.current_freq_hz as f64 / 1e6, spectrum_peak_db);
                return;
            }
        }

        // Resume delay after signal drops
        if self.holding {
            let resume_delay = Duration::from_millis(self.hold_resume_delay_ms);
            let since_signal = self.hold_last_active.map(|t| now.duration_since(t)).unwrap_or(resume_delay);
            if since_signal < resume_delay {
                self.last_step_time = Some(now);
                return;
            }
            self.holding = false;
            self.hold_last_active = None;
            self.status_text = format!("Scanning {:.3}–{:.3} MHz", self.start_hz as f64 / 1e6, self.stop_hz as f64 / 1e6);
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
                if ui.button("⏹ Stop").on_hover_text("Stop the scan and keep hits.").clicked() {
                    self.stop();
                }
                if self.paused {
                    if ui.button("▶ Resume").on_hover_text("Continue scanning from where it paused.").clicked() {
                        self.resume();
                    }
                } else {
                    if ui.button("⏸ Pause").on_hover_text("Pause the sweep at the current frequency without clearing hits.").clicked() {
                        self.pause();
                    }
                }
            } else if ui.button("▶ Start Scan").on_hover_text("Begin sweeping the configured frequency range.").clicked() {
                self.start();
            }
            if ui.button("Sort by strength").on_hover_text("Sort the hit list by signal strength, strongest first.").clicked() {
                self.sort_hits_by_strength();
            }
            if ui.button("Clear hits").on_hover_text("Remove all logged signal hits.").clicked() {
                self.hits.clear();
            }
            if ui.button("Export CSV").on_hover_text("Save all hits to a CSV file in the current directory.").clicked() {
                self.export_hits_csv();
            }
            if let Some((vis_start, vis_stop)) = self.spectrum_visible_range {
                if ui.button("🔭 Scan visible").on_hover_text(
                    format!("Set scan range to the currently visible spectrum view ({:.3}–{:.3} MHz), then start.", vis_start as f64 / 1e6, vis_stop as f64 / 1e6)
                ).clicked() {
                    self.start_hz = vis_start;
                    self.stop_hz = vis_stop;
                    if !self.enabled { self.start(); }
                }
            }
            if ui.add_enabled(!self.hits.is_empty(), egui::Button::new("📌 Bookmark all"))
                .on_hover_text("Add all scanner hits as bookmarks in the 'Scanner' category. Skips frequencies already bookmarked.")
                .clicked()
            {
                if let Ok(mut state) = self.shared.lock() {
                    let existing: std::collections::HashSet<u64> = state.bookmarks.bookmarks.iter().map(|b| b.frequency_hz).collect();
                    let mut added = 0u32;
                    for hit in &self.hits {
                        if !existing.contains(&hit.freq_hz) {
                            state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                                name: format!("{:.3} MHz", hit.freq_hz as f64 / 1e6),
                                frequency_hz: hit.freq_hz,
                                mode: "NFM".to_string(),
                                bandwidth_hz: self.step_hz.max(12_500) as u32,
                                category: "Scanner".to_string(),
                                notes: format!("{:.1} dB", hit.strength_db),
                            });
                            added += 1;
                        }
                    }
                    self.last_export_msg = format!("Added {} to Bookmarks/Scanner.", added);
                }
            }
            ui.checkbox(&mut self.auto_tune_on_hit, "Auto-tune on hit")
                .on_hover_text("When enabled, the SDR tunes to each new signal hit immediately so you can hear it. Pauses the sweep while listening.");
            ui.checkbox(&mut self.hold_on_active, "Hold on activity")
                .on_hover_text("Pause the sweep whenever a signal is detected above the threshold. Resumes scanning after signal drops and the resume delay expires.");
            if self.hold_on_active {
                ui.add(egui::Slider::new(&mut self.hold_resume_delay_ms, 200u64..=5000u64)
                    .step_by(100.0)
                    .text("Resume delay (ms)")
                    .custom_formatter(|v, _| format!("{:.0} ms", v)))
                    .on_hover_text("How long to wait after signal drops before resuming the sweep. Longer values prevent premature resume on intermittent signals.");
            }
            ui.separator();
            let color = if self.enabled { egui::Color32::GREEN } else { egui::Color32::GRAY };
            ui.colored_label(color, &self.status_text);
            if !self.last_export_msg.is_empty() {
                ui.label(egui::RichText::new(&self.last_export_msg).color(egui::Color32::from_rgb(100, 220, 100)).small());
            }
        });

        ui.separator();

        // Band presets
        ui.horizontal_wrapped(|ui| {
            ui.label("Presets:").on_hover_text("Quick-fill start/stop/step for common band plans.");
            const BAND_PRESETS: &[(&str, u64, u64, u64, &str)] = &[
                ("FM Broadcast",    88_000_000,  108_000_000, 100_000, "88–108 MHz WFM broadcast"),
                ("Airband",        118_000_000,  137_000_000,  25_000, "118–137 MHz AM aviation voice"),
                ("Marine VHF",     156_000_000,  174_000_000,  25_000, "156–174 MHz NFM marine"),
                ("Ham 2m",         144_000_000,  146_000_000,  12_500, "144–146 MHz NFM amateur"),
                ("Ham 70cm",       430_000_000,  440_000_000,  12_500, "430–440 MHz NFM amateur"),
                ("PMR446",         446_006_250,  446_193_750,   6_250, "PMR446 licence-free 8-channel"),
                ("Weather NOAA",   162_400_000,  162_550_000,  25_000, "162.4–162.55 MHz NOAA WX"),
                ("ISM 433",        433_050_000,  434_790_000,  25_000, "433 MHz ISM/remote controls"),
                ("POCSAG 153",     153_000_000,  154_000_000,  25_000, "153 MHz pager band"),
                ("Ham 23cm",     1_240_000_000, 1_300_000_000, 25_000, "1.24–1.3 GHz amateur"),
            ];
            for &(name, start, stop, step, tip) in BAND_PRESETS {
                if ui.small_button(name).on_hover_text(tip).clicked() {
                    self.start_hz = start;
                    self.stop_hz = stop;
                    self.step_hz = step;
                }
            }
        });
        ui.separator();

        egui::Grid::new("scanner_controls").num_columns(2).show(ui, |ui| {
            ui.label("Start (MHz):").on_hover_text("Lowest frequency to scan. The sweep begins here.");
            let mut start = self.start_hz as f64 / 1e6;
            if ui.add(egui::DragValue::new(&mut start).speed(0.01).range(0.5..=1770.0).suffix(" MHz"))
                .on_hover_text("Drag or type to set the scan start frequency in MHz.")
                .changed()
            {
                self.start_hz = (start * 1e6) as u64;
                if self.start_hz > self.stop_hz { self.stop_hz = self.start_hz; }
            }
            ui.end_row();

            ui.label("Stop (MHz):").on_hover_text("Highest frequency to scan. The sweep ends here and wraps back to start.");
            let mut stop = self.stop_hz as f64 / 1e6;
            if ui.add(egui::DragValue::new(&mut stop).speed(0.01).range(0.5..=1770.0).suffix(" MHz"))
                .on_hover_text("Drag or type to set the scan stop frequency in MHz.")
                .changed()
            {
                self.stop_hz = (stop * 1e6) as u64;
                if self.stop_hz < self.start_hz { self.start_hz = self.stop_hz; }
            }
            ui.end_row();

            ui.label("Step:").on_hover_text("How much to advance per dwell. Match to signal bandwidth: 100 kHz for FM broadcast, 12.5 kHz for NFM voice, 25 kHz for aviation.");
            ui.horizontal(|ui| {
                let presets = [1_000u64, 10_000, 100_000, 250_000, 1_000_000];
                for p in presets {
                    let label = match p {
                        1_000   => "1k",
                        10_000  => "10k",
                        100_000 => "100k",
                        250_000 => "250k",
                        _       => "1M",
                    };
                    if ui.selectable_label(self.step_hz == p, label).clicked() {
                        self.step_hz = p;
                    }
                }
            });
            ui.end_row();

            ui.label("Step (Hz):").on_hover_text("Fine-tune the step size in Hz. 12500 = standard NFM channel spacing. 25000 = aviation. 200000 = FM broadcast.");
            ui.add(egui::DragValue::new(&mut self.step_hz).speed(1000.0).range(100..=10_000_000))
                .on_hover_text("Current step size in Hz.");
            ui.end_row();

            ui.label("Dwell (ms):").on_hover_text("Time to listen at each frequency before stepping. 200–500 ms is typical. Too short misses bursty signals (digital voice, packets).");
            ui.add(egui::Slider::new(&mut self.dwell_ms, 50..=5000))
                .on_hover_text("Dwell time per step in milliseconds.");
            ui.end_row();

            ui.label("Threshold (dB):").on_hover_text("Minimum signal level to log as a 'hit'. Start at -60 dB and adjust based on your local noise floor. Anything above threshold is logged.");
            ui.add(egui::Slider::new(&mut self.threshold_db, -120.0..=0.0))
                .on_hover_text("Signal strength threshold in dB. Only signals above this level are recorded as hits.");
            ui.end_row();

            ui.label("Progress:").on_hover_text("How far through the current sweep the scanner is. Resets at the start frequency after each full sweep.");
            ui.add(egui::ProgressBar::new(self.progress).show_percentage().desired_width(200.0));
            ui.end_row();

            ui.label("Cycle time:").on_hover_text("Estimated time for one complete sweep (start → stop → back to start). = number of steps × dwell time.");
            {
                let span = self.stop_hz.saturating_sub(self.start_hz);
                let steps = if self.step_hz > 0 { span / self.step_hz + 1 } else { 1 };
                let total_ms = steps * self.dwell_ms;
                let cycle_str = if total_ms < 1000 {
                    format!("{} ms", total_ms)
                } else if total_ms < 60_000 {
                    format!("{:.1} s ({} steps)", total_ms as f64 / 1000.0, steps)
                } else {
                    format!("{:.1} min ({} steps)", total_ms as f64 / 60_000.0, steps)
                };
                ui.label(cycle_str).on_hover_text("One full sweep takes this long. Reduce dwell time or widen the step to scan faster at the cost of missing short transmissions.");
            }
            ui.end_row();

            ui.label("Current:").on_hover_text("Signal level measured at the current step frequency. Green = above threshold (hit logged), grey = below threshold.");
            ui.colored_label(
                if self.last_peak_db > self.threshold_db { egui::Color32::GREEN } else { egui::Color32::GRAY },
                format!("{:.1} dB", self.last_peak_db)
            );
            ui.end_row();
        });

        ui.checkbox(&mut self.reset_on_start, "Reset hits on start")
            .on_hover_text("If checked, the hits list is cleared each time you press Start Scan. Uncheck to accumulate across multiple sweeps.");

        ui.separator();
        ui.horizontal(|ui| {
            // Flash badge on new hit
            if self.hit_flash > 0 {
                let alpha = ((self.hit_flash as f32 / 45.0) * 220.0) as u8;
                ui.colored_label(
                    egui::Color32::from_rgba_premultiplied(46, 204, 113, alpha),
                    "● HIT!",
                ).on_hover_text("A new signal was just detected above the threshold!");
                self.hit_flash = self.hit_flash.saturating_sub(1);
                ui.separator();
            }
            ui.label(format!("Signals: {}", self.hits.len()))
                .on_hover_text("Total unique signal hits currently in the table (limited to max_hits).");
            if let Some(start) = self.scan_start_time {
                let elapsed_secs = start.elapsed().as_secs_f64().max(1.0);
                let rate = self.total_hits_logged as f64 / (elapsed_secs / 60.0);
                ui.separator();
                ui.label(format!("Rate: {:.1}/min", rate))
                    .on_hover_text("Number of new signal hits detected per minute since the scan started. High rate = active or noisy band; low rate = quiet band.");
            }
            ui.separator();
            ui.toggle_value(&mut self.show_histogram, "📊 Histogram")
                .on_hover_text("Show a bar chart of signal hits distributed across the scan range.");
        });

        // Hits strength histogram
        if self.show_histogram && !self.hits.is_empty() {
            let hist_h = 50.0;
            let (hist_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), hist_h),
                egui::Sense::hover(),
            );
            let painter = ui.painter();
            painter.rect_filled(hist_rect, 2.0, egui::Color32::from_rgb(8, 8, 16));

            let span = self.stop_hz.saturating_sub(self.start_hz).max(1) as f64;
            let n_buckets = 60usize;
            let mut buckets = vec![-120.0f32; n_buckets];
            for hit in &self.hits {
                let pos = hit.freq_hz.saturating_sub(self.start_hz) as f64;
                let bucket = ((pos / span) * n_buckets as f64) as usize;
                let bucket = bucket.min(n_buckets - 1);
                if hit.strength_db > buckets[bucket] {
                    buckets[bucket] = hit.strength_db;
                }
            }
            let min_db_h = self.threshold_db - 10.0;
            let max_db_h = (self.hits.iter().map(|h| h.strength_db).fold(-120.0f32, f32::max) + 5.0).max(min_db_h + 10.0);
            let db_range = (max_db_h - min_db_h).max(1.0);
            let bar_w = hist_rect.width() / n_buckets as f32;
            for (i, &db) in buckets.iter().enumerate() {
                if db <= min_db_h { continue; }
                let norm = ((db - min_db_h) / db_range).clamp(0.0, 1.0);
                let bar_h = norm * hist_h;
                let x = hist_rect.left() + i as f32 * bar_w;
                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(x + 0.5, hist_rect.bottom() - bar_h),
                    egui::vec2(bar_w - 1.0, bar_h),
                );
                let col = if db > -20.0 { egui::Color32::from_rgb(46, 204, 113) }
                    else if db > -40.0 { egui::Color32::from_rgb(241, 196, 15) }
                    else { egui::Color32::from_rgb(200, 120, 50) };
                painter.rect_filled(bar_rect, 0.0, col);
            }
            // Threshold line
            let thresh_norm = ((self.threshold_db - min_db_h) / db_range).clamp(0.0, 1.0);
            let thresh_y = hist_rect.bottom() - thresh_norm * hist_h;
            painter.line_segment(
                [egui::pos2(hist_rect.left(), thresh_y), egui::pos2(hist_rect.right(), thresh_y)],
                egui::Stroke::new(0.6, egui::Color32::from_rgba_premultiplied(231, 76, 60, 150)),
            );
            painter.text(
                egui::pos2(hist_rect.right() - 2.0, thresh_y - 2.0),
                egui::Align2::RIGHT_BOTTOM,
                format!("thr {:.0}", self.threshold_db),
                egui::FontId::proportional(7.5),
                egui::Color32::from_rgba_premultiplied(231, 76, 60, 180),
            );
            // Labels
            painter.text(
                egui::pos2(hist_rect.left() + 2.0, hist_rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("Signal strength histogram  {:.3}–{:.3} MHz", self.start_hz as f64 / 1e6, self.stop_hz as f64 / 1e6),
                egui::FontId::proportional(7.5),
                egui::Color32::DARK_GRAY,
            );
        }

        let hit_color = |db: f32| -> egui::Color32 {
            if db > -20.0 { egui::Color32::GREEN }
            else if db > -40.0 { egui::Color32::YELLOW }
            else if db > -60.0 { egui::Color32::from_rgb(200, 150, 50) }
            else { egui::Color32::GRAY }
        };

        egui::ScrollArea::vertical().max_height(400.0).auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("hits_grid")
                .num_columns(6)
                .striped(true)
                .min_col_width(50.0)
                .show(ui, |ui| {
                    ui.strong("Freq");
                    ui.strong("Strength");
                    ui.strong("Level");
                    ui.strong("Time Ago");
                    ui.strong("Tune");
                    ui.strong("Del");
                    ui.end_row();

                    let hits_copy = self.hits.clone();
                    let mut remove_idx = None;
                    for (i, hit) in hits_copy.iter().enumerate() {
                        ui.monospace(format!("{:.3} MHz", hit.freq_hz as f64 / 1e6));
                        ui.colored_label(hit_color(hit.strength_db), format!("{:.1} dB", hit.strength_db));
                        // Mini strength bar
                        let norm = ((hit.strength_db + 120.0) / 120.0).clamp(0.0, 1.0);
                        let bar_w = 60.0;
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, 10.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 1.0, egui::Color32::from_rgb(20, 20, 30));
                        let fill_color = hit_color(hit.strength_db);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(rect.min, egui::vec2(norm * bar_w, rect.height())),
                            1.0, fill_color,
                        );
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
