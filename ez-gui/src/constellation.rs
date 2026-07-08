use crate::theme::ThemeConfig;
use std::collections::VecDeque;

const DEFAULT_CAP: usize = 8192;
const MAX_POINTS_PER_BATCH: usize = 512;

#[derive(Default)]
pub struct ConstellationDisplay {
    buf: VecDeque<(f32, f32)>,
    cap: usize,
    pub paused: bool,
    pub density_grid: Vec<u32>,
    grid_dim: usize,
}

impl ConstellationDisplay {
    pub fn new() -> Self {
        Self {
            buf: VecDeque::with_capacity(DEFAULT_CAP),
            cap: DEFAULT_CAP,
            paused: false,
            density_grid: vec![0u32; 32 * 32],
            grid_dim: 32,
        }
    }

    pub fn set_cap(&mut self, new_cap: usize) {
        self.cap = new_cap.clamp(512, 65536);
        while self.buf.len() > self.cap {
            self.buf.pop_front();
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
        let stride = (n_pairs / MAX_POINTS_PER_BATCH).max(1);
        let mut i = 0;
        let half_grid = (self.grid_dim / 2) as i32;
        while i < n_pairs {
            let i_val = f32::from(iq[2 * i]) - 127.4;
            let q_val = f32::from(iq[2 * i + 1]) - 127.4;
            self.buf.push_back((i_val, q_val));
            if self.buf.len() > self.cap {
                self.buf.pop_front();
            }
            let gi = ((i_val / 127.4 * half_grid as f32) as i32).clamp(-half_grid, half_grid - 1);
            let gq = ((q_val / 127.4 * half_grid as f32) as i32).clamp(-half_grid, half_grid - 1);
            let idx = ((half_grid + gq) * self.grid_dim as i32 + (half_grid + gi)) as usize;
            if idx < self.density_grid.len() {
                self.density_grid[idx] = self.density_grid[idx].saturating_add(1);
            }
            i += stride;
        }
    }

    pub fn reset_density(&mut self) {
        self.density_grid.fill(0);
    }

    fn compute_evm(&self) -> f64 {
        if self.buf.is_empty() {
            return 0.0;
        }
        let mean_i: f64 = self.buf.iter().map(|(i, _)| *i as f64).sum::<f64>() / self.buf.len() as f64;
        let mean_q: f64 = self.buf.iter().map(|(_, q)| *q as f64).sum::<f64>() / self.buf.len() as f64;
        let ideal_power = mean_i * mean_i + mean_q * mean_q;
        if ideal_power < 1e-12 {
            return 0.0;
        }
        let err_sum: f64 = self
            .buf
            .iter()
            .map(|(i, q)| {
                let di = *i as f64 - mean_i;
                let dq = *q as f64 - mean_q;
                di * di + dq * dq
            })
            .sum();
        (err_sum / self.buf.len() as f64 / ideal_power).sqrt() * 100.0
    }

    fn compute_phase_error(&self) -> f64 {
        if self.buf.is_empty() {
            return 0.0;
        }
        let mean_i: f64 = self.buf.iter().map(|(i, _)| *i as f64).sum::<f64>() / self.buf.len() as f64;
        let mean_q: f64 = self.buf.iter().map(|(_, q)| *q as f64).sum::<f64>() / self.buf.len() as f64;
        let mean_phase = mean_q.atan2(mean_i);
        let err_sum: f64 = self
            .buf
            .iter()
            .map(|(i, q)| {
                let phase = (*q as f64).atan2(*i as f64);
                let diff = (phase - mean_phase).abs();
                diff.min(std::f64::consts::TAU - diff)
            })
            .sum();
        (err_sum / self.buf.len() as f64).to_degrees()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, theme: &ThemeConfig) {
        ui.horizontal(|ui| {
            let freeze_label = if self.paused { "▶ Resume" } else { "⏸ Freeze" };
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

        let evm = self.compute_evm();
        let phase_err = self.compute_phase_error();
        let info_y = rect.bottom() + 4.0;
        let evm_color = if evm < 10.0 {
            theme.success.to_egui()
        } else if evm < 25.0 {
            theme.warning.to_egui()
        } else {
            theme.error.to_egui()
        };
        painter.text(
            egui::pos2(rect.left(), info_y),
            egui::Align2::LEFT_TOP,
            format!("EVM: {evm:.1}%"),
            egui::FontId::proportional(11.0),
            evm_color,
        );
        painter.text(
            egui::pos2(cx, info_y),
            egui::Align2::CENTER_TOP,
            format!("\u{3c6} err: {phase_err:.1}\u{b0}"),
            egui::FontId::proportional(11.0),
            theme.accent.to_egui(),
        );
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
}
