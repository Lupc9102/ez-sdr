use std::f32::consts::PI;
use num_complex::Complex32;
use rustfft::{FftPlanner, Fft};
use std::sync::Arc;

pub struct SpectrumAnalyzer {
    fft_size: usize,
    waterfall_history: usize,
    waterfall_pixels: Vec<Vec<u8>>,
    spectrum_dbs: Vec<f32>,
    waterfall_texture: Option<egui::TextureHandle>,
    center_freq: u64,
    sample_rate: u32,
    window_type: WindowType,
    color_map: ColorMap,
    zoom_factor: f32,
    zoom_offset: f32,
    markers: Vec<u64>,
    avg_alpha: f32,
    peak_hold: Vec<f32>,
    show_peak_hold: bool,
    display_min_db: f32,
    display_max_db: f32,
    fft: Option<Arc<dyn Fft<f32>>>,
    fft_input_buf: Vec<Complex32>,
    window_cache: Vec<f32>,
    frame_counter: u32,
    hover_pos: Option<egui::Pos2>,
    waterfall_dirty: bool,
    waterfall_every_n: u32,
    pub clicked_tune_freq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowType {
    Hann,
    Hamming,
    Blackman,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMap {
    Classic,
    Viridis,
    Magma,
    Grayscale,
    Hot,
}

impl WindowType {
    fn generate(&self, len: usize) -> Vec<f32> {
        match self {
            WindowType::Hann => (0..len).map(|i| {
                let n = i as f32;
                let size = len as f32;
                0.5 * (1.0 - (2.0 * PI * n / size).cos())
            }).collect(),
            WindowType::Hamming => (0..len).map(|i| {
                let n = i as f32;
                let size = len as f32;
                0.54 - 0.46 * (2.0 * PI * n / size).cos()
            }).collect(),
            WindowType::Blackman => (0..len).map(|i| {
                let n = i as f32;
                let size = len as f32;
                0.42 - 0.5 * (2.0 * PI * n / size).cos() + 0.08 * (4.0 * PI * n / size).cos()
            }).collect(),
        }
    }
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        let fft_size = 2048;
        let waterfall_history = 256;
        let window_type = WindowType::Hann;
        let window_cache = window_type.generate(fft_size);
        let mut planner = FftPlanner::<f32>::new();
        let fft = Some(planner.plan_fft_forward(fft_size));
        Self {
            fft_size,
            waterfall_history,
            waterfall_pixels: vec![vec![0u8; fft_size * 4]; waterfall_history],
            spectrum_dbs: vec![-100.0; fft_size],
            waterfall_texture: None,
            center_freq: 100_000_000,
            sample_rate: 2_048_000,
            window_type,
            color_map: ColorMap::Classic,
            zoom_factor: 1.0,
            zoom_offset: 0.5,
            markers: Vec::new(),
            avg_alpha: 0.3,
            peak_hold: vec![-120.0; fft_size],
            show_peak_hold: false,
            display_min_db: -120.0,
            display_max_db: 0.0,
            fft,
            fft_input_buf: Vec::with_capacity(4096),
            window_cache,
            frame_counter: 0,
            hover_pos: None,
            waterfall_dirty: true,
            waterfall_every_n: 2,
            clicked_tune_freq: None,
        }
    }

    pub fn set_fft_size(&mut self, size: usize) {
        self.fft_size = size;
        self.spectrum_dbs = vec![-100.0; size];
        self.peak_hold = vec![-120.0; size];
        self.waterfall_pixels = vec![vec![0u8; size * 4]; self.waterfall_history];
        self.window_cache = self.window_type.generate(size);
        self.fft_input_buf = Vec::with_capacity(size.max(4096));
        let mut planner = FftPlanner::<f32>::new();
        self.fft = Some(planner.plan_fft_forward(size));
        self.waterfall_texture = None;
    }

    pub fn update_params(&mut self, center_freq: u64, sample_rate: u32) {
        self.center_freq = center_freq;
        self.sample_rate = sample_rate;
    }

    pub fn signal_level(&self) -> f32 {
        if self.spectrum_dbs.is_empty() { return -120.0; }
        let sum: f32 = self.spectrum_dbs.iter().sum();
        sum / self.spectrum_dbs.len() as f32
    }

    pub fn peak_level(&self) -> f32 {
        self.spectrum_dbs.iter().cloned().fold(-120.0f32, f32::max)
    }

    pub fn min_level(&self) -> f32 {
        self.spectrum_dbs.iter().cloned().fold(0.0f32, f32::min)
    }

    pub fn noise_floor(&self) -> f32 {
        if self.spectrum_dbs.is_empty() { return -120.0; }
        let mut sorted = self.spectrum_dbs.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted[sorted.len() / 4].max(sorted[0])
    }

    pub fn push_iq_samples(&mut self, iq: &[u8]) {
        if iq.len() < 2 { return; }
        let fft = match &self.fft {
            Some(f) => f,
            None => return,
        };
        let n_samples = iq.len() / 2;
        let fft_len = n_samples.min(self.fft_size);

        self.fft_input_buf.clear();
        self.fft_input_buf.extend((0..fft_len).map(|i| {
            let i_val = iq[2 * i] as f32 - 127.4;
            let q_val = iq[2 * i + 1] as f32 - 127.4;
            let w = if i < self.window_cache.len() { self.window_cache[i] } else { 1.0 };
            Complex32::new(i_val * w, q_val * w)
        }));

        fft.process(&mut self.fft_input_buf);

        let scale = 1.0 / (fft_len as f32);
        for (i, c) in self.fft_input_buf.iter().enumerate() {
            let mag = c.norm() * scale;
            let db = if mag > 1e-10 { 20.0 * mag.log10() } else { -120.0 };
            self.spectrum_dbs[i] = self.avg_alpha * db + (1.0 - self.avg_alpha) * self.spectrum_dbs[i];
            if db > self.peak_hold[i] {
                self.peak_hold[i] = db;
            } else {
                self.peak_hold[i] = 0.999 * self.peak_hold[i] + 0.001 * db;
            }
        }
    }

    fn waterfall_row(&self) -> Vec<u8> {
        let mut pixels = vec![0u8; self.fft_size * 4];
        let range = (self.display_max_db - self.display_min_db).max(1.0);
        for (i, db) in self.spectrum_dbs.iter().enumerate() {
            let normalized = ((db - self.display_min_db) / range).clamp(0.0, 1.0);
            let (r, g, b) = color_map(self.color_map, normalized);
            pixels[i * 4] = r;
            pixels[i * 4 + 1] = g;
            pixels[i * 4 + 2] = b;
            pixels[i * 4 + 3] = 255;
        }
        pixels
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.frame_counter = self.frame_counter.wrapping_add(1);

        // Controls bar
        ui.horizontal(|ui| {
            ui.label("FFT:");
            for size in [512, 1024, 2048, 4096] {
                if ui.selectable_label(self.fft_size == size, size.to_string()).clicked() {
                    self.set_fft_size(size);
                }
            }
            ui.separator();
            ui.label("Win:");
            if ui.selectable_label(self.window_type == WindowType::Hann, "Hann").clicked() { self.window_type = WindowType::Hann; self.set_fft_size(self.fft_size); }
            if ui.selectable_label(self.window_type == WindowType::Hamming, "Hamming").clicked() { self.window_type = WindowType::Hamming; self.set_fft_size(self.fft_size); }
            if ui.selectable_label(self.window_type == WindowType::Blackman, "Blackman").clicked() { self.window_type = WindowType::Blackman; self.set_fft_size(self.fft_size); }
            ui.separator();
            if ui.toggle_value(&mut self.show_peak_hold, "Peak").clicked() {
                if !self.show_peak_hold {
                    self.peak_hold = vec![-120.0; self.fft_size];
                }
            }
            if ui.small_button("Clear WF").clicked() {
                self.waterfall_pixels = vec![vec![0u8; self.fft_size * 4]; self.waterfall_history];
                self.waterfall_dirty = true;
            }
            ui.separator();
            ui.label("Palette:");
            for (label, cmap) in [("Classic", ColorMap::Classic), ("Viridis", ColorMap::Viridis), ("Magma", ColorMap::Magma), ("Gray", ColorMap::Grayscale), ("Hot", ColorMap::Hot)] {
                if ui.selectable_label(self.color_map == cmap, label).clicked() {
                    self.color_map = cmap;
                    self.waterfall_dirty = true;
                }
            }
            ui.separator();
            ui.label("Avg:");
            ui.add(egui::Slider::new(&mut self.avg_alpha, 0.01..=0.99).text("α"));
            ui.separator();
            ui.label("WF:");
            for (label, n) in [("1x", 1u32), ("2x", 2), ("4x", 4), ("8x", 8)] {
                if ui.selectable_label(self.waterfall_every_n == n, label).clicked() {
                    self.waterfall_every_n = n;
                }
            }
            ui.separator();
            let mark_count = self.markers.len();
            ui.label(format!("Marks: {}", mark_count));
            if ui.small_button("Clear M").clicked() {
                self.markers.clear();
            }
            ui.label("Zoom:");
            if ui.small_button("1x").clicked() {
                self.zoom_factor = 1.0;
                self.zoom_offset = 0.5;
            }
            if self.zoom_factor > 1.0 {
                ui.label(format!("{:.0}x", self.zoom_factor));
            }
            ui.separator();
            ui.label("dB range:").on_hover_text("Adjust the visible dB range on the spectrum plot. Drag the Floor/Ceil values to zoom in on a particular signal level.");
            ui.add(egui::DragValue::new(&mut self.display_min_db).speed(1.0).range(-160.0..=-40.0).suffix(" floor"))
                .on_hover_text("Bottom of the dB scale. Default -120 dBFS.");
            ui.add(egui::DragValue::new(&mut self.display_max_db).speed(1.0).range(-40.0..=20.0).suffix(" ceil"))
                .on_hover_text("Top of the dB scale. Default 0 dBFS.");
            if self.display_min_db >= self.display_max_db - 10.0 {
                self.display_min_db = self.display_max_db - 10.0;
                self.waterfall_dirty = true;
            }
            if ui.small_button("⟳").on_hover_text("Reset dB range to default (-120 to 0)").clicked() {
                self.display_min_db = -120.0;
                self.display_max_db = 0.0;
                self.waterfall_dirty = true;
            }
            if ui.small_button("Auto-fit").on_hover_text("Automatically set the dB range to the current signal min/max, centering the display on your signals.").clicked() {
                if !self.spectrum_dbs.is_empty() {
                    let cur_min = self.spectrum_dbs.iter().cloned().fold(f32::INFINITY, f32::min);
                    let cur_max = self.spectrum_dbs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    let margin = ((cur_max - cur_min) * 0.1).max(5.0);
                    self.display_min_db = (cur_min - margin).max(-160.0);
                    self.display_max_db = (cur_max + margin).min(20.0);
                    self.waterfall_dirty = true;
                }
            }
        });

        // Info bar
        ui.horizontal(|ui| {
            ui.monospace(format!("Center: {:.3} MHz", self.center_freq as f64 / 1e6));
            ui.monospace(format!("Span: {:.3} MHz", self.sample_rate as f64 / 1e6));
            ui.monospace(format!("Res: {:.1} Hz", self.sample_rate as f64 / self.fft_size as f64));
            ui.separator();
            ui.monospace(format!("Min: {:.0} dB", self.min_level()));
            ui.monospace(format!("Floor: {:.0} dB", self.noise_floor()));
            ui.monospace(format!("Avg: {:.0} dB", self.signal_level()));
            ui.monospace(format!("Max: {:.0} dB", self.peak_level()));
        });

        let avail = ui.available_size();
        let spectrum_height = avail.y * 0.35;
        let waterfall_height = avail.y * 0.65;

        // Spectrum plot
        let (spectrum_rect, response) = ui.allocate_exact_size(egui::vec2(avail.x, spectrum_height), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(spectrum_rect, 0.0, egui::Color32::from_rgb(8, 8, 14));

        let n = self.fft_size;
        let min_db = self.display_min_db;
        let max_db = self.display_max_db;
        let range = (max_db - min_db).max(1.0);

        // Horizontal grid lines (dB)
        for db in (-120..=0).step_by(20) {
            let db = db as f32;
            let norm = ((db - min_db) / range).clamp(0.0, 1.0);
            let y = spectrum_rect.bottom() - norm * spectrum_height;
            painter.line_segment(
                [egui::pos2(spectrum_rect.left(), y), egui::pos2(spectrum_rect.right(), y)],
                egui::Stroke::new(0.5, egui::Color32::from_rgba_premultiplied(40, 40, 50, 128)),
            );
            painter.text(
                egui::pos2(spectrum_rect.left() + 4.0, y - 7.0),
                egui::Align2::LEFT_CENTER,
                format!("{:.0}", db),
                egui::FontId::proportional(9.0),
                egui::Color32::from_gray(100),
            );
        }

        // Zoom parameters
        let zoom_span = (self.sample_rate as f64 / self.zoom_factor as f64).max(self.sample_rate as f64 * 0.01);
        let zoom_center_offset = (self.zoom_offset as f64 - 0.5) * zoom_span;
        let left_hz = -zoom_span / 2.0 + zoom_center_offset;
        let right_hz = zoom_span / 2.0 + zoom_center_offset;

        // Vertical grid lines (frequency) with zoom support
        let n_grid = 8;
        for i in 0..=n_grid {
            let frac = i as f32 / n_grid as f32;
            let x = spectrum_rect.left() + frac * spectrum_rect.width();
            let offset_hz = left_hz + frac as f64 * zoom_span;
            let freq_mhz = (self.center_freq as f64 + offset_hz) / 1e6;
            painter.line_segment(
                [egui::pos2(x, spectrum_rect.top()), egui::pos2(x, spectrum_rect.bottom())],
                egui::Stroke::new(0.5, egui::Color32::from_rgba_premultiplied(40, 40, 50, 128)),
            );
            painter.text(
                egui::pos2(x, spectrum_rect.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                format!("{:.2}", freq_mhz),
                egui::FontId::proportional(8.0),
                egui::Color32::from_gray(90),
            );
        }

        // Band plan overlay
        {
            struct Band { name: &'static str, low_mhz: f64, high_mhz: f64, color: egui::Color32 }
            const BANDS: &[Band] = &[
                Band { name: "160m", low_mhz: 1.8, high_mhz: 2.0, color: egui::Color32::from_rgba_premultiplied(60, 180, 75, 30) },
                Band { name: "80m",  low_mhz: 3.5, high_mhz: 4.0, color: egui::Color32::from_rgba_premultiplied(60, 180, 75, 30) },
                Band { name: "40m",  low_mhz: 7.0, high_mhz: 7.3, color: egui::Color32::from_rgba_premultiplied(60, 180, 75, 30) },
                Band { name: "20m",  low_mhz: 14.0, high_mhz: 14.35, color: egui::Color32::from_rgba_premultiplied(60, 180, 75, 30) },
                Band { name: "15m",  low_mhz: 21.0, high_mhz: 21.45, color: egui::Color32::from_rgba_premultiplied(60, 180, 75, 30) },
                Band { name: "10m",  low_mhz: 28.0, high_mhz: 29.7, color: egui::Color32::from_rgba_premultiplied(60, 180, 75, 30) },
                Band { name: "6m",   low_mhz: 50.0, high_mhz: 54.0, color: egui::Color32::from_rgba_premultiplied(180, 120, 50, 30) },
                Band { name: "2m",   low_mhz: 144.0, high_mhz: 148.0, color: egui::Color32::from_rgba_premultiplied(180, 120, 50, 30) },
                Band { name: "70cm", low_mhz: 420.0, high_mhz: 450.0, color: egui::Color32::from_rgba_premultiplied(180, 80, 80, 30) },
            ];
            let center_mhz = self.center_freq as f64 / 1e6;
            let half_span_mhz = zoom_span / 2e6;
            let left_mhz = center_mhz - half_span_mhz + zoom_center_offset / 1e6;
            let right_mhz = center_mhz + half_span_mhz + zoom_center_offset / 1e6;
            for band in BANDS {
                let low = band.low_mhz.max(left_mhz);
                let high = band.high_mhz.min(right_mhz);
                if low < high {
                    let x1 = spectrum_rect.left() + ((low - left_mhz) / (right_mhz - left_mhz)) as f32 * spectrum_rect.width();
                    let x2 = spectrum_rect.left() + ((high - left_mhz) / (right_mhz - left_mhz)) as f32 * spectrum_rect.width();
                    let rect = egui::Rect::from_x_y_ranges(x1..=x2, spectrum_rect.top()..=spectrum_rect.bottom());
                    painter.rect_filled(rect, 0.0, band.color);
                    let label_x = (x1 + x2) / 2.0;
                    painter.text(
                        egui::pos2(label_x, spectrum_rect.top() + 8.0),
                        egui::Align2::CENTER_CENTER,
                        band.name,
                        egui::FontId::proportional(8.0),
                        egui::Color32::from_rgba_premultiplied(180, 180, 180, 100),
                    );
                }
            }
        }

        // Fill under spectrum (zoom-aware)
        {
            let mut mesh = egui::Mesh::default();
            let color_top = egui::Color32::from_rgba_premultiplied(30, 120, 200, 100);
            let color_bot = egui::Color32::from_rgba_premultiplied(10, 30, 60, 20);
            let half_span = self.sample_rate as f64 / 2.0;
            let first_bin = ((left_hz + half_span) / self.sample_rate as f64 * n as f64) as usize;
            let last_bin = ((right_hz + half_span) / self.sample_rate as f64 * n as f64) as usize;
            let first_bin = first_bin.clamp(0, n.saturating_sub(1));
            let last_bin = last_bin.clamp(first_bin + 1, n);
            let visible_bins = last_bin - first_bin;
            if visible_bins > 0 {
                for i in first_bin..last_bin {
                    let frac = (i - first_bin) as f32 / visible_bins.max(1) as f32;
                    let x = spectrum_rect.left() + frac * spectrum_rect.width();
                    let db = self.spectrum_dbs[i];
                    let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                    let y = spectrum_rect.bottom() - norm * spectrum_height;
                    mesh.colored_vertex(egui::pos2(x, y), color_top);
                    mesh.colored_vertex(egui::pos2(x, spectrum_rect.bottom()), color_bot);
                }
                for i in 0..visible_bins.saturating_sub(1) {
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
            let half_span = self.sample_rate as f64 / 2.0;
            let first_bin = ((left_hz + half_span) / self.sample_rate as f64 * n as f64) as usize;
            let last_bin = ((right_hz + half_span) / self.sample_rate as f64 * n as f64) as usize;
            let first_bin = first_bin.clamp(0, n.saturating_sub(1));
            let last_bin = last_bin.clamp(first_bin + 1, n);
            let visible_bins = (last_bin - first_bin).max(1);
            for i in first_bin..last_bin {
                let frac = (i - first_bin) as f32 / visible_bins as f32;
                let x = spectrum_rect.left() + frac * spectrum_rect.width();
                let db = self.peak_hold[i];
                let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                if let Some(prev) = prev_pos {
                    painter.line_segment([prev, egui::pos2(x, y)], egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 80, 80)));
                }
                prev_pos = Some(egui::pos2(x, y));
            }
        }

        // Spectrum line (zoom-aware)
        {
            let mut prev_pos = None;
            let half_span = self.sample_rate as f64 / 2.0;
            let first_bin = ((left_hz + half_span) / self.sample_rate as f64 * n as f64) as usize;
            let last_bin = ((right_hz + half_span) / self.sample_rate as f64 * n as f64) as usize;
            let first_bin = first_bin.clamp(0, n.saturating_sub(1));
            let last_bin = last_bin.clamp(first_bin + 1, n);
            let visible_bins = (last_bin - first_bin).max(1);
            for i in first_bin..last_bin {
                let frac = (i - first_bin) as f32 / visible_bins as f32;
                let x = spectrum_rect.left() + frac * spectrum_rect.width();
                let db = self.spectrum_dbs[i];
                let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                if let Some(prev) = prev_pos {
                    painter.line_segment([prev, egui::pos2(x, y)], egui::Stroke::new(1.5, egui::Color32::from_rgb(46, 204, 113)));
                }
                prev_pos = Some(egui::pos2(x, y));
            }
        }

        // Mouse hover readout
        if let Some(pointer) = response.hover_pos() {
            self.hover_pos = Some(pointer);
            let frac = ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
            let bin = (frac * n as f32) as usize;
            if bin < n {
                let db = self.spectrum_dbs[bin];
                let zoom_span = (self.sample_rate as f64 / self.zoom_factor as f64).max(self.sample_rate as f64 * 0.01);
                let zoom_center_offset = (self.zoom_offset as f64 - 0.5) * zoom_span;
                let left_hz = -zoom_span / 2.0 + zoom_center_offset;
                let offset_hz = left_hz + frac as f64 * zoom_span;
                let freq = self.center_freq as f64 + offset_hz;
                let freq_str = if freq >= 1e9 { format!("{:.3} GHz", freq / 1e9) }
                    else if freq >= 1e6 { format!("{:.3} MHz", freq / 1e6) }
                    else { format!("{:.1} kHz", freq / 1e3) };

                // Crosshair
                painter.line_segment(
                    [egui::pos2(pointer.x, spectrum_rect.top()), egui::pos2(pointer.x, spectrum_rect.bottom())],
                    egui::Stroke::new(0.5, egui::Color32::from_rgba_premultiplied(200, 200, 200, 128)),
                );
                painter.line_segment(
                    [egui::pos2(spectrum_rect.left(), pointer.y), egui::pos2(spectrum_rect.right(), pointer.y)],
                    egui::Stroke::new(0.5, egui::Color32::from_rgba_premultiplied(200, 200, 200, 128)),
                );

                // Tooltip box
                let tooltip = format!("{} | {:.1} dB", freq_str, db);
                let text_rect = egui::Rect::from_min_size(
                    egui::pos2(pointer.x + 12.0, pointer.y - 20.0),
                    egui::vec2(160.0, 16.0),
                );
                painter.rect_filled(text_rect, 2.0, egui::Color32::from_rgba_unmultiplied(20, 20, 30, 220));
                painter.text(
                    egui::pos2(text_rect.left() + 4.0, text_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &tooltip,
                    egui::FontId::monospace(10.0),
                    egui::Color32::from_rgb(46, 204, 113),
                );
            }
        }

        // Frequency markers
        for marker_freq in &self.markers {
            let offset_hz = *marker_freq as f64 - self.center_freq as f64;
            let zoom_span = (self.sample_rate as f64 / self.zoom_factor as f64).max(self.sample_rate as f64 * 0.01);
            let zoom_center_offset = (self.zoom_offset as f64 - 0.5) * zoom_span;
            let left_hz = -zoom_span / 2.0 + zoom_center_offset;
            let right_hz = zoom_span / 2.0 + zoom_center_offset;
            let frac = (offset_hz - left_hz) / (right_hz - left_hz);
            if (0.0..=1.0).contains(&frac) {
                let x = spectrum_rect.left() + frac as f32 * spectrum_rect.width();
                painter.line_segment(
                    [egui::pos2(x, spectrum_rect.top()), egui::pos2(x, spectrum_rect.bottom())],
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(255, 200, 50, 160)),
                );
                let label = format!("{:.3} MHz", *marker_freq as f64 / 1e6);
                painter.text(
                    egui::pos2(x, spectrum_rect.top() + 2.0),
                    egui::Align2::CENTER_TOP,
                    label,
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgba_premultiplied(255, 200, 50, 200),
                );
            }
        }

        // Click-to-tune, zoom, and markers on spectrum
        if response.clicked() {
            if let Some(pointer) = response.hover_pos() {
                let frac = ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
                let zoom_span = (self.sample_rate as f64 / self.zoom_factor as f64).max(self.sample_rate as f64 * 0.01);
                let zoom_center_offset = (self.zoom_offset as f64 - 0.5) * zoom_span;
                let left_hz = -zoom_span / 2.0 + zoom_center_offset;
                let offset_hz = left_hz + frac as f64 * zoom_span;
                let freq = (self.center_freq as f64 + offset_hz) as u64;
                self.clicked_tune_freq = Some(freq);
            }
        }
        if response.secondary_clicked() {
            self.zoom_factor = 1.0;
            self.zoom_offset = 0.5;
        }
        // Middle-click to add frequency marker
        if response.clicked_by(egui::PointerButton::Middle) {
            if let Some(pointer) = response.hover_pos() {
                let frac = ((pointer.x - spectrum_rect.left()) / spectrum_rect.width()).clamp(0.0, 1.0);
                let zoom_span = (self.sample_rate as f64 / self.zoom_factor as f64).max(self.sample_rate as f64 * 0.01);
                let zoom_center_offset = (self.zoom_offset as f64 - 0.5) * zoom_span;
                let left_hz = -zoom_span / 2.0 + zoom_center_offset;
                let offset_hz = left_hz + frac as f64 * zoom_span;
                let freq = (self.center_freq as f64 + offset_hz) as u64;
                self.markers.push(freq);
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
            if scroll_delta.y != 0.0 {
                if shift {
                    self.zoom_offset = (self.zoom_offset - scroll_delta.y.signum() * 0.02).clamp(0.0, 1.0);
                } else {
                    self.zoom_factor = (self.zoom_factor * (1.0 + scroll_delta.y * -0.1)).clamp(1.0, 200.0);
                }
            }
        }

        // Waterfall (speed controlled by waterfall_every_n)
        if self.frame_counter % self.waterfall_every_n == 0 {
            let row = self.waterfall_row();
            self.waterfall_pixels.pop();
            self.waterfall_pixels.insert(0, row);
            self.waterfall_dirty = true;
        }

        let (wf_rect, _) = ui.allocate_exact_size(egui::vec2(avail.x, waterfall_height), egui::Sense::hover());

        if self.waterfall_dirty {
            let mut rgba_bytes = Vec::with_capacity(self.fft_size * self.waterfall_history * 4);
            for row_data in &self.waterfall_pixels {
                rgba_bytes.extend_from_slice(row_data);
            }
            let rgba = egui::ColorImage::from_rgba_unmultiplied(
                [self.fft_size, self.waterfall_history],
                &rgba_bytes,
            );
            match &mut self.waterfall_texture {
                Some(tex) => {
                    tex.set(rgba, egui::TextureOptions::NEAREST);
                }
                None => {
                    self.waterfall_texture = Some(ui.ctx().load_texture(
                        "waterfall",
                        rgba,
                        egui::TextureOptions::NEAREST,
                    ));
                }
            }
            self.waterfall_dirty = false;
        }

        if let Some(tex) = &self.waterfall_texture {
            ui.painter().image(
                tex.id(),
                wf_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Waterfall frequency labels (zoom-aware)
        let wf_painter = ui.painter();
        for i in 0..=n_grid {
            let frac = i as f64 / n_grid as f64;
            let x = wf_rect.left() + (frac as f32) * wf_rect.width();
            let offset_hz = left_hz + frac * zoom_span;
            let freq_mhz = (self.center_freq as f64 + offset_hz) / 1e6;
            wf_painter.text(
                egui::pos2(x, wf_rect.top() + 2.0),
                egui::Align2::CENTER_TOP,
                format!("{:.2}", freq_mhz),
                egui::FontId::proportional(8.0),
                egui::Color32::from_rgba_premultiplied(180, 180, 180, 160),
            );
        }
    }
}

fn color_map(cmap: ColorMap, t: f32) -> (u8, u8, u8) {
    match cmap {
        ColorMap::Classic => waterfall_color_classic(t),
        ColorMap::Viridis => {
            let r = (t * 255.0) as u8;
            let g = ((1.0 - (t - 0.5).abs() * 2.0) * 255.0) as u8;
            let b = ((1.0 - t) * 255.0) as u8;
            (r, g, b)
        }
        ColorMap::Magma => {
            let r = (t.powf(1.5) * 255.0) as u8;
            let g = (t.powf(0.75) * (1.0 - t) * 255.0) as u8;
            let b = ((1.0 - t).powf(1.5) * 255.0) as u8;
            (r, g, b)
        }
        ColorMap::Grayscale => {
            let v = (t * 255.0) as u8;
            (v, v, v)
        }
        ColorMap::Hot => {
            let r = (t * 3.0).min(1.0) * 255.0;
            let g = ((t * 3.0 - 1.0).max(0.0).min(1.0)) * 255.0;
            let b = ((t * 3.0 - 2.0).max(0.0).min(1.0)) * 255.0;
            (r as u8, g as u8, b as u8)
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
