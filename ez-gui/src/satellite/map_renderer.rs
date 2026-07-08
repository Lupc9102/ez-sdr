use crate::satellite::types::{SatPosition, TrajectoryPoint, TrajectorySegment};
use egui::{Color32, Pos2, Rect, Stroke, Vec2};

pub struct MapRenderer {
    pub center_lat: f64,
    pub center_lon: f64,
    pub zoom: f32,
    trajectory: Vec<TrajectoryPoint>,
    screen_traj: Vec<Pos2>,
    sat_marker: Option<Pos2>,
    observer_marker: Pos2,
    pub in_pass_now: bool,
    #[allow(dead_code)]
    drag_start: Option<Pos2>,
}

impl MapRenderer {
    pub fn new(observer_lat: f64, observer_lon: f64) -> Self {
        Self {
            center_lat: observer_lat,
            center_lon: observer_lon,
            zoom: 4.0,
            trajectory: Vec::new(),
            screen_traj: Vec::new(),
            sat_marker: None,
            observer_marker: Pos2::ZERO,
            in_pass_now: false,
            drag_start: None,
        }
    }

    pub fn set_trajectory(&mut self, points: &[TrajectoryPoint]) {
        self.trajectory = points.to_vec();
    }

    pub fn set_satellite_position(&mut self, pos: Option<SatPosition>) {
        if let Some(p) = pos {
            self.sat_marker = Some(lat_lon_to_screen(
                p.lat,
                p.lon,
                self.center_lat,
                self.center_lon,
                self.zoom,
            ));
        } else {
            self.sat_marker = None;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ui.available_height().max(300.0)),
            egui::Sense::click_and_drag(),
        );

        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }

        // Pan via drag
        if response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            let sens = 0.01 * (18.0 - self.zoom).max(1.0) as f64;
            self.center_lon -= delta.x as f64 * sens;
            self.center_lat += delta.y as f64 * sens;
            self.center_lat = self.center_lat.clamp(-85.0, 85.0);
            self.center_lon = self.center_lon.clamp(-180.0, 180.0);
        }

        // Zoom via scroll
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom + scroll * 0.1).clamp(2.0, 18.0);
            }
        }

        let painter = ui.painter();
        let cx = rect.center().x;
        let cy = rect.center().y;

        // Background
        painter.rect_filled(rect, 0.0, Color32::from_rgb(12, 16, 24));

        // Grid lines (lat/lon)
        self.draw_grid(&painter, rect, cx, cy);

        // Trajectory polyline
        if !self.trajectory.is_empty() {
            let screen_pts: Vec<Pos2> = self
                .trajectory
                .iter()
                .map(|tp| {
                    lat_lon_to_screen(
                        tp.geo.lat,
                        tp.geo.lon,
                        self.center_lat,
                        self.center_lon,
                        self.zoom,
                    ) + rect.center().to_vec2()
                })
                .collect();
            self.screen_traj = screen_pts.clone();

            // Draw segment by segment with per-segment styling
            for i in 1..screen_pts.len() {
                let seg = self.trajectory[i].segment;
                let (color, width) = trajectory_segment_style(seg, self.in_pass_now);
                // Glow layer: wider, semi-transparent stroke behind the main line
                if self.in_pass_now && seg == TrajectorySegment::InPass {
                    painter.line_segment(
                        [screen_pts[i - 1], screen_pts[i]],
                        Stroke::new(
                            width + 8.0,
                            Color32::from_rgba_premultiplied(0, 255, 200, 40),
                        ),
                    );
                    painter.line_segment(
                        [screen_pts[i - 1], screen_pts[i]],
                        Stroke::new(
                            width + 4.0,
                            Color32::from_rgba_premultiplied(0, 255, 200, 80),
                        ),
                    );
                }
                painter.line_segment(
                    [screen_pts[i - 1], screen_pts[i]],
                    Stroke::new(width, color),
                );
            }

            // AOS / LOS markers
            if let Some(aos_idx) = self
                .trajectory
                .iter()
                .position(|tp| tp.segment == TrajectorySegment::InPass)
            {
                let aos_pt = screen_pts[aos_idx];
                painter.circle_filled(aos_pt, 5.0, Color32::GREEN);
                painter.text(
                    aos_pt + Vec2::new(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    "AOS",
                    egui::FontId::proportional(10.0),
                    Color32::GREEN,
                );

                if let Some(los_idx) = self
                    .trajectory
                    .iter()
                    .rposition(|tp| tp.segment == TrajectorySegment::InPass)
                {
                    if los_idx > aos_idx {
                        let los_pt = screen_pts[los_idx];
                        painter.circle_filled(los_pt, 5.0, Color32::RED);
                        painter.text(
                            los_pt + Vec2::new(6.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            "LOS",
                            egui::FontId::proportional(10.0),
                            Color32::RED,
                        );
                    }
                }
            }
        }

        // Observer marker
        self.observer_marker = lat_lon_to_screen(
            self.center_lat,
            self.center_lon,
            self.center_lat,
            self.center_lon,
            self.zoom,
        ) + rect.center().to_vec2();
        painter.circle_filled(self.observer_marker, 4.0, Color32::YELLOW);
        painter.text(
            self.observer_marker + Vec2::new(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            "YOU",
            egui::FontId::proportional(10.0),
            Color32::YELLOW,
        );

        // Satellite marker
        if let Some(sat_pos) = &self.sat_marker {
            let sat_abs = *sat_pos + rect.center().to_vec2();
            // Glow
            painter.circle_filled(
                sat_abs,
                8.0,
                Color32::from_rgba_premultiplied(0, 200, 255, 80),
            );
            painter.circle_filled(sat_abs, 4.0, Color32::from_rgb(0, 200, 255));
            // Azimuth line from observer to satellite
            painter.line_segment(
                [self.observer_marker, sat_abs],
                Stroke::new(1.0, Color32::from_rgba_premultiplied(0, 200, 255, 60)),
            );
        }

        // Zoom level indicator
        painter.text(
            Rect::from_min_size(rect.left_top(), Vec2::splat(100.0)).min,
            egui::Align2::LEFT_TOP,
            format!("Zoom: {:.0}x", self.zoom),
            egui::FontId::proportional(10.0),
            Color32::from_gray(100),
        );

        // Hover tooltip for trajectory
        if response.hovered() {
            if let Some(mouse_pos) = ui.ctx().pointer_interact_pos() {
                if rect.contains(mouse_pos) {
                    for (i, tp) in self.trajectory.iter().enumerate() {
                        if i < self.screen_traj.len() {
                            let d = (self.screen_traj[i] - mouse_pos).length();
                            if d < 12.0 {
                                let label =
                                    format!("{}  T{:.0}s", tp.segment.label(), tp.timestamp);
                                painter.text(
                                    mouse_pos + Vec2::new(0.0, -20.0),
                                    egui::Align2::CENTER_BOTTOM,
                                    label,
                                    egui::FontId::proportional(11.0),
                                    Color32::WHITE,
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Coordinate display
        let coord_str = format!("{:.2}°N, {:.2}°E", self.center_lat, self.center_lon);
        painter.text(
            Rect::from_min_size(
                rect.left_bottom() - Vec2::new(0.0, 16.0),
                Vec2::splat(200.0),
            )
            .min,
            egui::Align2::LEFT_BOTTOM,
            coord_str,
            egui::FontId::proportional(10.0),
            Color32::from_gray(100),
        );
    }

    fn draw_grid(&self, painter: &egui::Painter, rect: Rect, cx: f32, cy: f32) {
        let grid_color = Color32::from_rgba_premultiplied(60, 60, 80, 40);
        for lat in (-80..=80).step_by(20) {
            if (lat as f64 - self.center_lat).abs() > 80.0 {
                continue;
            }
            let p = lat_lon_to_screen(
                lat as f64,
                self.center_lon,
                self.center_lat,
                self.center_lon,
                self.zoom,
            );
            let y = cy + p.y;
            if y >= rect.top() && y <= rect.bottom() {
                painter.line_segment(
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                    Stroke::new(1.0, grid_color),
                );
            }
        }
        for lon in (-180..=180).step_by(30) {
            if (lon as f64 - self.center_lon).abs() > 180.0 {
                continue;
            }
            let p = lat_lon_to_screen(
                self.center_lat,
                lon as f64,
                self.center_lat,
                self.center_lon,
                self.zoom,
            );
            let x = cx + p.x;
            if x >= rect.left() && x <= rect.right() {
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(1.0, grid_color),
                );
            }
        }
    }
}

fn lat_lon_to_screen(lat: f64, lon: f64, center_lat: f64, center_lon: f64, zoom: f32) -> Pos2 {
    let scale = (2.0_f64).powf(zoom as f64) * 0.5;
    let dx = ((lon - center_lon) / 180.0) * scale * 200.0;
    let dy = -((lat - center_lat) / 90.0) * scale * 200.0;
    Pos2::new(dx as f32, dy as f32)
}

fn trajectory_segment_style(seg: TrajectorySegment, in_pass_now: bool) -> (Color32, f32) {
    match (seg, in_pass_now) {
        // AOS→LOS window: bright, thick, high-visibility while live.
        (TrajectorySegment::InPass, true) => (Color32::from_rgb(0, 255, 200), 5.0),
        (TrajectorySegment::InPass, false) => (Color32::from_rgb(0, 220, 170), 3.5),
        // Approach / past segments are dim so the pass arc stands out.
        (TrajectorySegment::PreAOS, _) => (Color32::from_gray(70), 1.0),
        (TrajectorySegment::PostLOS, _) => (Color32::from_gray(70), 1.0),
    }
}
