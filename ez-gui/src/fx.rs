//! Painter helpers for gradients and glow, driven by [`crate::theme::Gradient`]
//! and [`crate::theme::GlowConfig`]. egui has no native blur/gradient-fill
//! primitives, so these build vertex-colored meshes / concentric fading
//! strokes instead.

use crate::theme::{Gradient, GlowConfig};

/// Paint a vertical (top→bottom) linear gradient into `rect` using a
/// vertex-colored triangle mesh — one quad per pair of adjacent stops.
pub fn gradient_rect_vertical(painter: &egui::Painter, rect: egui::Rect, gradient: &Gradient) {
    if gradient.stops.is_empty() {
        return;
    }
    let mut stops = gradient.stops.clone();
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if stops.len() == 1 {
        painter.rect_filled(rect, 0.0, stops[0].1.to_egui());
        return;
    }

    let mut mesh = egui::Mesh::default();
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        let y0 = rect.top() + rect.height() * t0;
        let y1 = rect.top() + rect.height() * t1;
        let color0 = c0.to_egui();
        let color1 = c1.to_egui();
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(rect.left(), y0), color0);
        mesh.colored_vertex(egui::pos2(rect.right(), y0), color0);
        mesh.colored_vertex(egui::pos2(rect.right(), y1), color1);
        mesh.colored_vertex(egui::pos2(rect.left(), y1), color1);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Paint a soft glow around `rect` as concentric fading rounded-rect strokes.
/// No-op if `glow.enabled` is false.
pub fn paint_glow(painter: &egui::Painter, rect: egui::Rect, corner_radius: f32, glow: &GlowConfig) {
    if !glow.enabled || glow.radius <= 0.0 {
        return;
    }
    const STEPS: usize = 6;
    let base = glow.color.to_egui();
    for i in 0..STEPS {
        let t = i as f32 / STEPS as f32;
        let expand = glow.radius * t;
        let alpha = (glow.intensity * (1.0 - t) * 90.0) as u8;
        let r = rect.expand(expand);
        let color = egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        painter.rect_stroke(
            r,
            egui::CornerRadius::same((corner_radius + expand) as u8),
            egui::Stroke::new(1.5, color),
            egui::StrokeKind::Outside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Rgba;

    #[test]
    fn gradient_rect_vertical_no_crash_empty_stops() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let painter = ctx.debug_painter();
        let gradient = Gradient { stops: vec![] };
        gradient_rect_vertical(&painter, egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0)), &gradient);
        ctx.end_pass();
    }

    #[test]
    fn paint_glow_disabled_is_noop() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let painter = ctx.debug_painter();
        let glow = GlowConfig { enabled: false, color: Rgba::from_rgb(255, 0, 0), radius: 10.0, intensity: 1.0 };
        paint_glow(&painter, egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0)), 4.0, &glow);
        ctx.end_pass();
    }
}
