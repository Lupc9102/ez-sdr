use crate::satellite::types::{GeoPoint, SatPosition, TrajectoryPoint, TrajectorySegment};
use crate::theme::ThemeConfig;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};

pub struct MapRenderer {
    pub center_lat: f64,
    pub center_lon: f64,
    pub zoom: f32,
    trajectory: Vec<TrajectoryPoint>,
    screen_traj: Vec<Pos2>,
    satellite: Option<SatPosition>,
    observer: GeoPoint,
    pub follow_observer: bool,
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
            satellite: None,
            observer: GeoPoint {
                lat: observer_lat,
                lon: observer_lon,
            },
            follow_observer: false,
            in_pass_now: false,
            drag_start: None,
        }
    }

    pub fn set_trajectory(&mut self, points: &[TrajectoryPoint]) {
        self.trajectory = points.to_vec();
        self.screen_traj.clear();
    }

    pub fn set_satellite_position(&mut self, pos: Option<SatPosition>) {
        self.satellite = pos;
    }

    pub fn set_observer_position(&mut self, lat: f64, lon: f64) {
        self.observer = GeoPoint { lat, lon };
        if self.follow_observer {
            self.recenter_on_observer();
        }
    }

    pub fn recenter_on_observer(&mut self) {
        self.center_lat = self.observer.lat;
        self.center_lon = self.observer.lon;
    }

    fn project(&self, point: GeoPoint, rect: Rect) -> Pos2 {
        lat_lon_to_screen(
            point.lat,
            point.lon,
            self.center_lat,
            self.center_lon,
            self.zoom,
        ) + rect.center().to_vec2()
    }

    #[cfg(test)]
    pub(crate) fn trajectory(&self) -> &[TrajectoryPoint] {
        &self.trajectory
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, theme: &ThemeConfig) {
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut self.follow_observer, "Follow observer")
                .changed()
                && self.follow_observer
            {
                self.recenter_on_observer();
            }
            if ui.button("Recenter observer").clicked() {
                self.recenter_on_observer();
            }
        });
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ui.available_height().max(300.0)),
            egui::Sense::click_and_drag(),
        );

        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }

        // Pan via drag
        if response.dragged_by(egui::PointerButton::Primary) {
            self.follow_observer = false;
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

        // Time drives the subtle pulse on the live-pass satellite halo.
        let pulse = ui.input(|i| (i.time * 3.0).sin() as f32) * 0.5 + 0.5;

        let painter = ui.painter();
        let cx = rect.center().x;
        let cy = rect.center().y;

        // Background
        painter.rect_filled(rect, 0.0, theme.bg.to_egui());

        // Grid lines (lat/lon)
        self.draw_grid(painter, rect, cx, cy, theme);

        // Trajectory polyline
        if !self.trajectory.is_empty() {
            let screen_pts: Vec<Pos2> = self
                .trajectory
                .iter()
                .map(|tp| self.project(tp.geo, rect))
                .collect();
            self.screen_traj = screen_pts.clone();

            // Draw segment by segment with per-segment styling
            for i in 1..screen_pts.len() {
                let seg = self.trajectory[i].segment;
                let (color, width) = trajectory_segment_style(seg, self.in_pass_now, theme);
                // Glow layer: wider, semi-transparent stroke behind the main line
                if self.in_pass_now && seg == TrajectorySegment::InPass {
                    painter.line_segment(
                        [screen_pts[i - 1], screen_pts[i]],
                        Stroke::new(width + 8.0, theme.success.with_alpha(40).to_egui()),
                    );
                    painter.line_segment(
                        [screen_pts[i - 1], screen_pts[i]],
                        Stroke::new(width + 4.0, theme.success.with_alpha(80).to_egui()),
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
                painter.circle_filled(aos_pt, 5.0, theme.success.to_egui());
                painter.text(
                    aos_pt + Vec2::new(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    "AOS",
                    egui::FontId::proportional(10.0),
                    theme.success.to_egui(),
                );

                if let Some(los_idx) = self
                    .trajectory
                    .iter()
                    .rposition(|tp| tp.segment == TrajectorySegment::InPass)
                {
                    if los_idx > aos_idx {
                        let los_pt = screen_pts[los_idx];
                        painter.circle_filled(los_pt, 5.0, theme.error.to_egui());
                        painter.text(
                            los_pt + Vec2::new(6.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            "LOS",
                            egui::FontId::proportional(10.0),
                            theme.error.to_egui(),
                        );
                    }
                }
            }
        }

        // Observer marker
        let observer_marker = self.project(self.observer, rect);
        painter.circle_filled(observer_marker, 4.0, theme.warning.to_egui());
        painter.text(
            observer_marker + Vec2::new(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            "YOU",
            egui::FontId::proportional(10.0),
            theme.warning.to_egui(),
        );

        // Satellite marker
        if let Some(sat_pos) = self.satellite {
            let sat_abs = self.project(
                GeoPoint {
                    lat: sat_pos.lat,
                    lon: sat_pos.lon,
                },
                rect,
            );
            // Pulsing halo while a pass is live — the eyecandy the user asked for.
            if self.in_pass_now {
                let halo_alpha = (40.0 + pulse * 70.0) as u8;
                painter.circle_filled(
                    sat_abs,
                    14.0,
                    theme.accent.with_alpha(halo_alpha / 3).to_egui(),
                );
                painter.circle_filled(sat_abs, 10.0, theme.accent.with_alpha(halo_alpha).to_egui());
            }
            // Glow
            painter.circle_filled(sat_abs, 8.0, theme.accent.with_alpha(80).to_egui());
            painter.circle_filled(sat_abs, 4.0, theme.accent.to_egui());
            // Azimuth line from observer to satellite
            painter.line_segment(
                [observer_marker, sat_abs],
                Stroke::new(1.0, theme.accent.with_alpha(60).to_egui()),
            );
        }

        // Zoom level indicator
        painter.text(
            Rect::from_min_size(rect.left_top(), Vec2::splat(100.0)).min,
            egui::Align2::LEFT_TOP,
            format!("Zoom: {:.0}x", self.zoom),
            egui::FontId::proportional(10.0),
            theme.text_dim.to_egui(),
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
                                    theme.text_heading.to_egui(),
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Coordinate display
        let coord_str = format!(
            "View center: {:.2}°, {:.2}°",
            self.center_lat, self.center_lon
        );
        painter.text(
            Rect::from_min_size(
                rect.left_bottom() - Vec2::new(0.0, 16.0),
                Vec2::splat(200.0),
            )
            .min,
            egui::Align2::LEFT_BOTTOM,
            coord_str,
            egui::FontId::proportional(10.0),
            theme.text_dim.to_egui(),
        );
    }

    fn draw_grid(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        cx: f32,
        cy: f32,
        theme: &ThemeConfig,
    ) {
        let grid_color = theme.spectrum_grid.to_egui();
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

fn trajectory_segment_style(
    seg: TrajectorySegment,
    in_pass_now: bool,
    theme: &ThemeConfig,
) -> (Color32, f32) {
    match (seg, in_pass_now) {
        // AOS→LOS window: bright, thick, high-visibility while live.
        (TrajectorySegment::InPass, true) => (theme.success.to_egui(), 5.0),
        (TrajectorySegment::InPass, false) => (theme.success.with_alpha(200).to_egui(), 3.5),
        // Approach / past segments are dim so the pass arc stands out.
        (TrajectorySegment::PreAOS, _) => (theme.text_dim.to_egui(), 1.0),
        (TrajectorySegment::PostLOS, _) => (theme.text_dim.to_egui(), 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(
        map: &mut MapRenderer,
        context: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| map.ui(ui, &ThemeConfig::default()),
        )
    }

    #[test]
    fn marker_and_trajectory_share_each_frames_pan_zoom_and_rectangle_projection() {
        let mut map = MapRenderer::new(51.5, -0.1);
        let geo = GeoPoint {
            lat: 51.6,
            lon: 0.0,
        };
        map.set_trajectory(&[
            TrajectoryPoint {
                geo,
                segment: TrajectorySegment::InPass,
                timestamp: 0.0,
            },
            TrajectoryPoint {
                geo: GeoPoint {
                    lat: 51.61,
                    lon: 0.01,
                },
                segment: TrajectorySegment::PostLOS,
                timestamp: 1.0,
            },
        ]);
        map.set_satellite_position(Some(SatPosition {
            lat: geo.lat,
            lon: geo.lon,
            azimuth: 0.0,
            elevation: 30.0,
            distance_km: 800.0,
            timestamp: 0.0,
        }));
        let context = egui::Context::default();
        let theme = ThemeConfig::default();
        let mut previous = None;
        for (lat, lon, zoom) in [(51.5, -0.1, 4.0), (51.55, -0.05, 6.0), (51.65, 0.05, 8.0)] {
            map.center_lat = lat;
            map.center_lon = lon;
            map.zoom = zoom;
            let output = render(&mut map, &context, vec![]);
            let satellite = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if circle.radius == 4.0 && circle.fill == theme.accent.to_egui() =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("real map did not draw satellite marker");
            let observer = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if circle.radius == 4.0 && circle.fill == theme.warning.to_egui() =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("real map did not draw observer marker");
            assert_eq!(satellite, map.screen_traj[0]);
            let expected_delta =
                lat_lon_to_screen(map.observer.lat, map.observer.lon, lat, lon, zoom)
                    - lat_lon_to_screen(geo.lat, geo.lon, lat, lon, zoom);
            assert!((observer - satellite - expected_delta).length() < 0.001);
            if let Some(previous) = previous {
                assert_ne!(satellite, previous);
            }
            previous = Some(satellite);
        }
        map.set_satellite_position(None);
        map.set_trajectory(&[]);
        assert!(map.screen_traj.is_empty());
        let output = render(&mut map, &context, vec![]);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(circle) if circle.radius == 4.0 && circle.fill == theme.accent.to_egui())));
    }

    #[test]
    fn map_drag_disables_follow_and_observer_updates_preserve_camera() {
        let mut map = MapRenderer::new(51.5, -0.1);
        map.follow_observer = true;
        let context = egui::Context::default();
        render(&mut map, &context, vec![]);
        let press = Pos2::new(400.0, 300.0);
        render(
            &mut map,
            &context,
            vec![
                egui::Event::PointerMoved(press),
                egui::Event::PointerButton {
                    pos: press,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        let moved = press + Vec2::new(40.0, 20.0);
        render(&mut map, &context, vec![egui::Event::PointerMoved(moved)]);
        assert!(!map.follow_observer);
        let camera = (map.center_lat, map.center_lon);
        assert_ne!(camera, (51.5, -0.1));
        map.set_observer_position(-33.8688, 151.2093);
        assert_eq!((map.center_lat, map.center_lon), camera);
        map.recenter_on_observer();
        assert_eq!((map.center_lat, map.center_lon), (-33.8688, 151.2093));
        map.follow_observer = true;
        map.set_observer_position(40.7128, -74.0060);
        assert_eq!((map.center_lat, map.center_lon), (40.7128, -74.0060));
    }
}
