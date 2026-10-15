use crate::theme::ThemeConfig;
use std::collections::VecDeque;

const DEFAULT_CAP: usize = 8192;
const MAX_POINTS_PER_BATCH: usize = 512;

pub struct ConstellationDisplay {
    buf: VecDeque<(f32, f32)>,
    cap: usize,
    pub paused: bool,
    pub density_grid: Vec<u32>,
    grid_dim: usize,
    capture: Option<(u64, u32, crate::source_manager::SourceMode, u64)>,
}

impl Default for ConstellationDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstellationDisplay {
    pub fn new() -> Self {
        Self {
            buf: VecDeque::with_capacity(DEFAULT_CAP),
            cap: DEFAULT_CAP,
            paused: false,
            density_grid: vec![0u32; 32 * 32],
            grid_dim: 32,
            capture: None,
        }
    }

    #[allow(dead_code)]
    pub fn set_cap(&mut self, new_cap: usize) {
        self.cap = new_cap.clamp(512, 65536);
        while self.buf.len() > self.cap {
            self.evict_oldest();
        }
    }

    pub fn push_iq_samples(&mut self, iq: &[u8]) {
        if self.paused {
            return;
        }
        let n_pairs = iq.len() / 2;
        if n_pairs == 0 {
            return;
        }
        let stride = n_pairs.div_ceil(MAX_POINTS_PER_BATCH);
        let mut i = 0;
        while i < n_pairs {
            let i_val = f32::from(iq[2 * i]) - 127.4;
            let q_val = f32::from(iq[2 * i + 1]) - 127.4;
            self.buf.push_back((i_val, q_val));
            let idx = self.density_index(i_val, q_val);
            self.density_grid[idx] += 1;
            if self.buf.len() > self.cap {
                self.evict_oldest();
            }
            i += stride;
        }
    }

    /// Density and scatter points cover the same selected-sample window.
    pub fn reset_density(&mut self) {
        self.buf.clear();
        self.density_grid.fill(0);
    }

    pub fn observe_capture(
        &mut self,
        center_hz: u64,
        sample_rate_hz: u32,
        source: &crate::source_manager::SourceMode,
        generation: u64,
    ) {
        let capture = (center_hz, sample_rate_hz, source.clone(), generation);
        if self.capture.as_ref() != Some(&capture) {
            self.reset_density();
            self.capture = Some(capture);
        }
    }

    fn density_index(&self, i: f32, q: f32) -> usize {
        let half = (self.grid_dim / 2) as i32;
        let x = ((i / 127.4 * half as f32) as i32).clamp(-half, half - 1);
        let y = ((q / 127.4 * half as f32) as i32).clamp(-half, half - 1);
        ((half + y) * self.grid_dim as i32 + half + x) as usize
    }

    fn evict_oldest(&mut self) {
        if let Some((i, q)) = self.buf.pop_front() {
            let index = self.density_index(i, q);
            self.density_grid[index] -= 1;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, theme: &ThemeConfig) {
        ui.label("Raw IQ geometry");
        ui.label(egui::RichText::new("ADC samples before carrier/timing recovery").small());
        ui.horizontal(|ui| {
            let freeze_label = if self.paused {
                "▶ Resume"
            } else {
                "⏸ Freeze"
            };
            if ui.small_button(freeze_label).clicked() {
                self.paused = !self.paused;
            }
            if self.paused {
                ui.colored_label(theme.warning.to_egui(), "FROZEN");
            }
            if ui.small_button("↺ Reset").clicked() {
                self.reset_density();
            }
        });
        let side = ui.available_width().min(320.0);
        let (rect, _resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.waterfall_bg.to_egui());

        let half = side / 2.0 - 6.0;
        let cx = rect.center().x;
        let cy = rect.center().y;

        painter.circle_stroke(
            egui::pos2(cx, cy),
            half,
            egui::Stroke::new(1.0, theme.text_dim.with_alpha(70).to_egui()),
        );

        painter.line_segment(
            [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
            egui::Stroke::new(1.0, theme.text_dim.with_alpha(90).to_egui()),
        );
        painter.line_segment(
            [egui::pos2(cx, rect.top()), egui::pos2(cx, rect.bottom())],
            egui::Stroke::new(1.0, theme.text_dim.with_alpha(90).to_egui()),
        );

        if self.buf.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for signal\u{2026}",
                egui::FontId::proportional(13.0),
                theme.text_dim.to_egui(),
            );
            return;
        }

        let max_density = *self.density_grid.iter().max().unwrap_or(&1).max(&1);
        let alpha_scale = 160.0 / max_density as f32;
        for gy in 0..self.grid_dim {
            for gx in 0..self.grid_dim {
                let val = self.density_grid[gy * self.grid_dim + gx];
                if val == 0 {
                    continue;
                }
                let nx = (gx as f32 / self.grid_dim as f32) * 2.0 - 1.0;
                let ny = (gy as f32 / self.grid_dim as f32) * 2.0 - 1.0;
                let px = cx + nx * half;
                let py = cy - ny * half;
                let alpha = ((val as f32 * alpha_scale).min(200.0)) as u8;
                let cell_half = half / self.grid_dim as f32;
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        egui::pos2(px, py),
                        egui::vec2(cell_half * 2.0, cell_half * 2.0),
                    ),
                    0.0,
                    theme.success.with_alpha(alpha).to_egui(),
                );
            }
        }

        for &(i_val, q_val) in &self.buf {
            let px = cx + (i_val / 127.4) * half;
            let py = cy - (q_val / 127.4) * half;
            painter.circle_filled(
                egui::pos2(px, py),
                1.2,
                theme.success.with_alpha(180).to_egui(),
            );
        }

        let info_y = rect.bottom() + 4.0;
        painter.text(
            egui::pos2(rect.right(), info_y),
            egui::Align2::RIGHT_TOP,
            format!("{} pts", self.buf.len()),
            egui::FontId::proportional(11.0),
            theme.text_dim.to_egui(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_caps_at_cap() {
        let mut c = ConstellationDisplay::new();
        c.set_cap(512);
        let batch: Vec<u8> = (0..(5000 * 2)).map(|i| (i % 256) as u8).collect();
        for _ in 0..10 {
            c.push_iq_samples(&batch);
            assert!(c.buf.len() <= 512);
        }
    }

    #[test]
    fn push_iq_samples_strides_large_batches() {
        let mut c = ConstellationDisplay::new();
        let n_pairs = 20_000;
        let batch: Vec<u8> = (0..(n_pairs * 2)).map(|i| (i % 256) as u8).collect();
        c.push_iq_samples(&batch);
        assert!(c.buf.len() <= MAX_POINTS_PER_BATCH + 1);
        assert!(c.buf.len() > 1);
    }

    #[test]
    fn iq_to_point_matches_uc8_offset_convention() {
        let mut c = ConstellationDisplay::new();
        c.push_iq_samples(&[0, 255]);
        assert_eq!(c.buf.len(), 1);
        let (i_val, q_val) = c.buf[0];
        assert!((i_val - (-127.4)).abs() < 1e-3);
        assert!((q_val - 127.6).abs() < 1e-3);
    }

    #[test]
    fn empty_input_is_noop() {
        let mut c = ConstellationDisplay::new();
        c.push_iq_samples(&[]);
        assert!(c.buf.is_empty());
    }

    #[test]
    fn pause_prevents_samples() {
        let mut c = ConstellationDisplay::new();
        c.paused = true;
        c.push_iq_samples(&[100, 100, 200, 200]);
        assert!(c.buf.is_empty());
    }

    #[test]
    fn density_grid_accumulates() {
        let mut c = ConstellationDisplay::new();
        c.push_iq_samples(&[0, 0, 128, 128, 255, 255]);
        let total: u32 = c.density_grid.iter().sum();
        assert!(total > 0);
    }

    #[test]
    fn density_and_points_share_a_bounded_window_including_shrink_and_reset() {
        let mut view = ConstellationDisplay::new();
        view.set_cap(1024);
        let old = view.density_index(-100.4, -100.4);
        for _ in 0..20 {
            view.push_iq_samples(&vec![27; 1024]);
        }
        assert_eq!(
            view.density_grid.iter().sum::<u32>() as usize,
            view.buf.len()
        );
        for _ in 0..20 {
            view.push_iq_samples(&vec![227; 1024]);
        }
        assert_eq!(view.buf.len(), 1024);
        assert_eq!(
            view.density_grid[old], 0,
            "evicted capture points leave no density"
        );
        assert_eq!(view.density_grid.iter().sum::<u32>(), 1024);
        view.set_cap(512);
        assert_eq!(view.density_grid.iter().sum::<u32>(), 512);
        view.reset_density();
        assert!(view.buf.is_empty());
        assert_eq!(view.density_grid.iter().sum::<u32>(), 0);
        view.push_iq_samples(&[127, 128]);
        assert_eq!(view.density_grid.iter().sum::<u32>(), 1);
    }

    #[test]
    fn capture_changes_retire_old_points_and_density_even_while_frozen() {
        use crate::source_manager::SourceMode;
        let mut view = ConstellationDisplay::new();
        view.observe_capture(100_000_000, 2_400_000, &SourceMode::Replay, 1);
        view.push_iq_samples(&[0, 255]);
        view.observe_capture(100_000_000, 2_400_000, &SourceMode::Replay, 1);
        assert_eq!(view.buf.len(), 1);
        for key in [
            (101_000_000, 2_400_000, SourceMode::Replay, 1),
            (101_000_000, 2_048_000, SourceMode::Replay, 1),
            (101_000_000, 2_048_000, SourceMode::Replay, 2),
            (101_000_000, 2_048_000, SourceMode::Simulated, 2),
        ] {
            view.paused = true;
            view.observe_capture(key.0, key.1, &key.2, key.3);
            assert!(view.buf.is_empty());
            assert_eq!(view.density_grid.iter().sum::<u32>(), 0);
            assert!(view.paused);
            view.paused = false;
            view.push_iq_samples(&[255, 0]);
        }
    }

    #[test]
    fn raw_qpsk_rotating_carrier_and_noise_views_make_no_symbol_quality_claim() {
        let qpsk: Vec<u8> = (0..1024)
            .flat_map(|n| match n % 4 {
                0 => [167, 167],
                1 => [87, 167],
                2 => [87, 87],
                _ => [167, 87],
            })
            .collect();
        let rotating: Vec<u8> = (0..1024)
            .flat_map(|n| {
                let p = n as f32 * 0.1;
                [
                    (127.4 + 40.0 * p.cos()).round() as u8,
                    (127.4 + 40.0 * p.sin()).round() as u8,
                ]
            })
            .collect();
        let noise: Vec<u8> = (0..2048).map(|n| ((n * 73 + n / 3) % 256) as u8).collect();
        for samples in [qpsk, rotating, noise] {
            let mut view = ConstellationDisplay::new();
            view.push_iq_samples(&samples);
            let ctx = egui::Context::default();
            let output = ctx.run_ui(Default::default(), |ui| {
                view.ui(ui, &ThemeConfig::default())
            });
            let labels: String = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) => Some(t.galley.text()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(labels.contains("Raw IQ geometry"));
            assert!(labels.contains("before carrier/timing recovery"));
            assert!(!labels.contains("EVM"));
            assert!(!labels.contains("err:"));
            assert_eq!(
                view.density_grid.iter().sum::<u32>() as usize,
                view.buf.len()
            );
        }
    }
}
