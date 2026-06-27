use std::f32::consts::PI;
use num_complex::Complex32;
use rustfft::FftPlanner;

pub struct SpectrumAnalyzer {
    fft_size: usize,
    waterfall_history: usize,
    waterfall_pixels: Vec<Vec<u8>>,
    spectrum_dbs: Vec<f32>,
    texture: Option<egui::TextureHandle>,
    waterfall_texture: Option<egui::TextureHandle>,
    center_freq: u64,
    sample_rate: u32,
    window_type: WindowType,
    avg_alpha: f32,
    peak_hold: Vec<f32>,
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
        Self {
            fft_size,
            waterfall_history,
            waterfall_pixels: vec![vec![0u8; fft_size * 4]; waterfall_history],
            spectrum_dbs: vec![0.0; fft_size],
            texture: None,
            waterfall_texture: None,
            center_freq: 109_000_000,
            sample_rate: 2_048_000,
            window_type: WindowType::Hann,
            avg_alpha: 0.3,
            peak_hold: vec![-120.0; fft_size],
        }
    }

    pub fn set_fft_size(&mut self, size: usize) {
        self.fft_size = size;
        self.spectrum_dbs = vec![0.0; size];
        self.peak_hold = vec![-120.0; size];
        self.waterfall_pixels = vec![vec![0u8; size * 4]; self.waterfall_history];
    }

    pub fn push_iq_samples(&mut self, iq: &[u8]) {
        if iq.len() < 2 { return; }
        let n_samples = iq.len() / 2;
        let fft_len = n_samples.min(self.fft_size);
        let window = self.window_type.generate(fft_len);

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_len);

        let mut buffer: Vec<Complex32> = (0..fft_len).map(|i| {
            let i_val = iq[2 * i] as f32 - 127.4;
            let q_val = iq[2 * i + 1] as f32 - 127.4;
            Complex32::new(i_val * window[i], q_val * window[i])
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
                self.peak_hold[i] = 0.9999 * self.peak_hold[i] + 0.0001 * db;
            }
        }
    }

    fn spectrum_to_pixels(&self) -> Vec<u8> {
        let mut pixels = vec![0u8; self.fft_size * 3];
        for (i, db) in self.spectrum_dbs.iter().enumerate() {
            let normalized = ((db + 120.0) / 60.0).clamp(0.0, 1.0);
            let (r, g, b) = db_to_rgb(normalized);
            pixels[i * 3] = r;
            pixels[i * 3 + 1] = g;
            pixels[i * 3 + 2] = b;
        }
        pixels
    }

    fn waterfall_row(&self) -> Vec<u8> {
        let mut pixels = vec![0u8; self.fft_size * 4];
        for (i, db) in self.spectrum_dbs.iter().enumerate() {
            let normalized = ((db + 120.0) / 60.0).clamp(0.0, 1.0);
            let (r, g, b) = waterfall_color(normalized);
            pixels[i * 4] = r;
            pixels[i * 4 + 1] = g;
            pixels[i * 4 + 2] = b;
            pixels[i * 4 + 3] = 255;
        }
        pixels
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("FFT Size:");
            for size in [512, 1024, 2048, 4096] {
                if ui.selectable_label(self.fft_size == size, size.to_string()).clicked() {
                    self.set_fft_size(size);
                }
            }
            ui.separator();
            ui.label("Window:");
            if ui.selectable_label(self.window_type == WindowType::Hann, "Hann").clicked() { self.window_type = WindowType::Hann; }
            if ui.selectable_label(self.window_type == WindowType::Hamming, "Hamming").clicked() { self.window_type = WindowType::Hamming; }
            if ui.selectable_label(self.window_type == WindowType::Blackman, "Blackman").clicked() { self.window_type = WindowType::Blackman; }
            ui.separator();
            ui.label(format!("Center: {:.3} MHz", self.center_freq as f64 / 1e6));
            ui.label(format!("BW: {:.3} MHz", self.sample_rate as f64 / 1e6));
        });

        let avail = ui.available_size();
        let spectrum_height = avail.y * 0.35;
        let waterfall_height = avail.y * 0.65;

        // Spectrum trace
        let (spectrum_rect, _) = ui.allocate_exact_size(egui::vec2(avail.x, spectrum_height), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(spectrum_rect, 0.0, egui::Color32::from_rgb(10, 10, 15));

        let pixels = self.spectrum_to_pixels();
        let n = self.fft_size;
        for i in 0..n {
            let x = spectrum_rect.left() + (i as f32 / n as f32) * spectrum_rect.width();
            let db = self.spectrum_dbs[i];
            let norm = ((db + 120.0) / 60.0).clamp(0.0, 1.0);
            let h = norm * spectrum_height;
            let color = egui::Color32::from_rgb(pixels[i * 3], pixels[i * 3 + 1], pixels[i * 3 + 2]);
            painter.line_segment(
                [egui::pos2(x, spectrum_rect.bottom()), egui::pos2(x, spectrum_rect.bottom() - h)],
                egui::Stroke::new(1.0, color),
            );
        }

        // Grid lines
        for db in [-120.0f32, -100.0, -80.0, -60.0, -40.0, -20.0, 0.0] {
            let norm = ((db + 120.0) / 60.0).clamp(0.0, 1.0);
            let y = spectrum_rect.bottom() - norm * spectrum_height;
            painter.line_segment(
                [egui::pos2(spectrum_rect.left(), y), egui::pos2(spectrum_rect.right(), y)],
                egui::Stroke::new(0.5, egui::Color32::from_rgba_premultiplied(60, 60, 60, 128)),
            );
            painter.text(
                egui::pos2(spectrum_rect.left() + 5.0, y - 6.0),
                egui::Align2::LEFT_CENTER,
                format!("{:.0} dB", db),
                egui::FontId::proportional(10.0),
                egui::Color32::from_gray(150),
            );
        }

        // Waterfall
        let row = self.waterfall_row();
        self.waterfall_pixels.pop();
        self.waterfall_pixels.insert(0, row);

        let (wf_rect, _) = ui.allocate_exact_size(egui::vec2(avail.x, waterfall_height), egui::Sense::hover());
        let mut rgba_bytes = Vec::with_capacity(self.fft_size * self.waterfall_history * 4);
        for row_data in &self.waterfall_pixels {
            rgba_bytes.extend_from_slice(row_data);
        }

        let rgba = egui::ColorImage::from_rgba_unmultiplied(
            [self.fft_size, self.waterfall_history],
            &rgba_bytes,
        );

        let texture = ui.ctx().load_texture(
            "waterfall",
            rgba,
            egui::TextureOptions::default(),
        );

        ui.painter().image(
            texture.id(),
            wf_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}

fn db_to_rgb(norm: f32) -> (u8, u8, u8) {
    let r = (norm * 2.0).min(1.0) * 255.0;
    let g = ((norm - 0.5) * 2.0).clamp(0.0, 1.0) * 255.0;
    let b = ((1.0 - norm) * 2.0).min(1.0) * 255.0;
    (r as u8, g as u8, b as u8)
}

fn waterfall_color(norm: f32) -> (u8, u8, u8) {
    if norm < 0.25 {
        let t = norm / 0.25;
        ((t * 100.0) as u8, 0, ((0.5 + t * 0.5) * 255.0) as u8)
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
