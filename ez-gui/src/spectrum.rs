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
    avg_alpha: f32,
    peak_hold: Vec<f32>,
    show_peak_hold: bool,
    fft: Option<Arc<dyn Fft<f32>>>,
    window_cache: Vec<f32>,
    frame_counter: u32,
    hover_pos: Option<egui::Pos2>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowType {
    Hann,
    Hamming,
    Blackman,
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
            avg_alpha: 0.3,
            peak_hold: vec![-120.0; fft_size],
            show_peak_hold: false,
            fft,
            window_cache,
            frame_counter: 0,
            hover_pos: None,
        }
    }

    pub fn set_fft_size(&mut self, size: usize) {
        self.fft_size = size;
        self.spectrum_dbs = vec![-100.0; size];
        self.peak_hold = vec![-120.0; size];
        self.waterfall_pixels = vec![vec![0u8; size * 4]; self.waterfall_history];
        self.window_cache = self.window_type.generate(size);
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

    pub fn push_iq_samples(&mut self, iq: &[u8]) {
        if iq.len() < 2 { return; }
        let fft = match &self.fft {
            Some(f) => f,
            None => return,
        };
        let n_samples = iq.len() / 2;
        let fft_len = n_samples.min(self.fft_size);

        let mut buffer: Vec<Complex32> = (0..fft_len).map(|i| {
            let i_val = iq[2 * i] as f32 - 127.4;
            let q_val = iq[2 * i + 1] as f32 - 127.4;
            let w = if i < self.window_cache.len() { self.window_cache[i] } else { 1.0 };
            Complex32::new(i_val * w, q_val * w)
        }).collect();

        fft.process(&mut buffer);

        let scale = 1.0 / (fft_len as f32);
        for (i, c) in buffer.iter().enumerate() {
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
        for (i, db) in self.spectrum_dbs.iter().enumerate() {
            let normalized = ((db + 120.0) / 80.0).clamp(0.0, 1.0);
            let (r, g, b) = waterfall_color(normalized);
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
        });

        // Info bar
        ui.horizontal(|ui| {
            ui.monospace(format!("Center: {:.3} MHz", self.center_freq as f64 / 1e6));
            ui.monospace(format!("Span: {:.3} MHz", self.sample_rate as f64 / 1e6));
            ui.monospace(format!("Res: {:.1} Hz", self.sample_rate as f64 / self.fft_size as f64));
        });

        let avail = ui.available_size();
        let spectrum_height = avail.y * 0.35;
        let waterfall_height = avail.y * 0.65;

        // Spectrum plot
        let (spectrum_rect, response) = ui.allocate_exact_size(egui::vec2(avail.x, spectrum_height), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(spectrum_rect, 0.0, egui::Color32::from_rgb(8, 8, 14));

        let n = self.fft_size;
        let min_db = -120.0f32;
        let max_db = 0.0f32;
        let range = max_db - min_db;

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

        // Vertical grid lines (frequency)
        let n_grid = 8;
        let half_span = self.sample_rate as f64 / 2.0;
        for i in 0..=n_grid {
            let frac = i as f32 / n_grid as f32;
            let x = spectrum_rect.left() + frac * spectrum_rect.width();
            let offset_hz = -half_span + frac as f64 * self.sample_rate as f64;
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

        // Fill under spectrum
        {
            let mut mesh = egui::Mesh::default();
            let color_top = egui::Color32::from_rgba_premultiplied(30, 120, 200, 100);
            let color_bot = egui::Color32::from_rgba_premultiplied(10, 30, 60, 20);
            for i in 0..n {
                let x = spectrum_rect.left() + (i as f32 / n as f32) * spectrum_rect.width();
                let db = self.spectrum_dbs[i];
                let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                mesh.colored_vertex(egui::pos2(x, y), color_top);
                mesh.colored_vertex(egui::pos2(x, spectrum_rect.bottom()), color_bot);
            }
            for i in 0..n.saturating_sub(1) {
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

        // Peak hold
        if self.show_peak_hold {
            let mut prev_pos = None;
            for i in 0..n {
                let x = spectrum_rect.left() + (i as f32 / n as f32) * spectrum_rect.width();
                let db = self.peak_hold[i];
                let norm = ((db - min_db) / range).clamp(0.0, 1.0);
                let y = spectrum_rect.bottom() - norm * spectrum_height;
                if let Some(prev) = prev_pos {
                    painter.line_segment([prev, egui::pos2(x, y)], egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 80, 80)));
                }
                prev_pos = Some(egui::pos2(x, y));
            }
        }

        // Spectrum line
        {
            let mut prev_pos = None;
            for i in 0..n {
                let x = spectrum_rect.left() + (i as f32 / n as f32) * spectrum_rect.width();
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
                let offset_hz = (frac as f64 - 0.5) * self.sample_rate as f64;
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

        // Waterfall
        if self.frame_counter % 2 == 0 {
            let row = self.waterfall_row();
            self.waterfall_pixels.pop();
            self.waterfall_pixels.insert(0, row);
        }

        let (wf_rect, _) = ui.allocate_exact_size(egui::vec2(avail.x, waterfall_height), egui::Sense::hover());

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

        if let Some(tex) = &self.waterfall_texture {
            ui.painter().image(
                tex.id(),
                wf_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Waterfall frequency labels
        let wf_painter = ui.painter();
        for i in 0..=n_grid {
            let frac = i as f32 / n_grid as f32;
            let x = wf_rect.left() + frac * wf_rect.width();
            let offset_hz = -half_span + frac as f64 * self.sample_rate as f64;
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

fn waterfall_color(norm: f32) -> (u8, u8, u8) {
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
