use std::collections::VecDeque;

const CAP: usize = 4096;
const MAX_POINTS_PER_BATCH: usize = 512;

#[derive(Default)]
pub struct ConstellationDisplay {
    buf: VecDeque<(f32, f32)>,
}

impl ConstellationDisplay {
    pub fn push_iq_samples(&mut self, iq: &[u8]) {
        let n_pairs = iq.len() / 2;
        if n_pairs == 0 {
            return;
        }
        let stride = (n_pairs / MAX_POINTS_PER_BATCH).max(1);
        let mut i = 0;
        while i < n_pairs {
            let i_val = f32::from(iq[2 * i]) - 127.4;
            let q_val = f32::from(iq[2 * i + 1]) - 127.4;
            self.buf.push_back((i_val, q_val));
            if self.buf.len() > CAP {
                self.buf.pop_front();
            }
            i += stride;
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) {
        let side = ui.available_width().min(260.0);
        let (rect, _resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(15));
        // crosshair
        painter.line_segment(
            [rect.left_center(), rect.right_center()],
            (1.0, egui::Color32::from_gray(60)),
        );
        painter.line_segment(
            [rect.center_top(), rect.center_bottom()],
            (1.0, egui::Color32::from_gray(60)),
        );
        if self.buf.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for signal…",
                egui::FontId::proportional(12.0),
                egui::Color32::GRAY,
            );
            return;
        }
        let half = side / 2.0 - 4.0;
        for &(i_val, q_val) in &self.buf {
            let px = rect.center().x + (i_val / 127.4) * half;
            let py = rect.center().y - (q_val / 127.4) * half;
            painter.circle_filled(egui::pos2(px, py), 1.5, egui::Color32::from_rgb(80, 220, 120));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_caps_at_4096() {
        let mut c = ConstellationDisplay::default();
        let batch: Vec<u8> = (0..(5000 * 2)).map(|i| (i % 256) as u8).collect();
        // Each call strides down to roughly MAX_POINTS_PER_BATCH points, so a
        // single batch can't overflow CAP on its own — push enough batches to
        // guarantee the ring buffer has exceeded its cap at least once.
        for _ in 0..10 {
            c.push_iq_samples(&batch);
            assert!(c.buf.len() <= CAP);
        }
        assert_eq!(c.buf.len(), CAP);
    }

    #[test]
    fn push_iq_samples_strides_large_batches() {
        let mut c = ConstellationDisplay::default();
        let n_pairs = 20_000;
        let batch: Vec<u8> = (0..(n_pairs * 2)).map(|i| (i % 256) as u8).collect();
        c.push_iq_samples(&batch);
        // Strided, not truncated-from-front and not all 20_000.
        assert!(c.buf.len() <= MAX_POINTS_PER_BATCH + 1);
        assert!(c.buf.len() > 1);
    }

    #[test]
    fn iq_to_point_matches_uc8_offset_convention() {
        let mut c = ConstellationDisplay::default();
        c.push_iq_samples(&[0, 255]);
        assert_eq!(c.buf.len(), 1);
        let (i_val, q_val) = c.buf[0];
        assert!((i_val - (-127.4)).abs() < 1e-3);
        assert!((q_val - 127.6).abs() < 1e-3);
    }

    #[test]
    fn empty_input_is_noop() {
        let mut c = ConstellationDisplay::default();
        c.push_iq_samples(&[]);
        assert!(c.buf.is_empty());
    }
}
