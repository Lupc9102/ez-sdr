use crate::satellite::types::SatPosition;
use egui::{Color32, Shape, Stroke, Vec2};

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum DipoleType {
    VDipole137,
    Turnstile,
    Qfh,
    Helical,
}

impl DipoleType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::VDipole137 => "9A4QV V-Dipole (137 MHz)",
            Self::Turnstile => "Turnstile (crossed dipoles)",
            Self::Qfh => "QFH (quadrifilar helix)",
            Self::Helical => "Helical (GOES 1694 MHz)",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DipoleAlignment {
    pub compass_heading: f64,
    pub elevation_tilt: f64,
    pub dipole_type: DipoleType,
}

/// Convert a compass bearing in degrees to an 8-point cardinal name
/// (e.g. 135° → "Southeast"). Wraps negatives and values ≥ 360.
pub fn azimuth_to_cardinal(deg: f64) -> &'static str {
    const POINTS: [&str; 8] = [
        "North",
        "Northeast",
        "East",
        "Southeast",
        "South",
        "Southwest",
        "West",
        "Northwest",
    ];
    let normalized = deg.rem_euclid(360.0);
    // Each 8-point sector spans 45°; offset by half a sector so the
    // boundary sits mid-sector (337.5°..22.5° → North).
    let idx = (((normalized + 22.5) / 45.0) as usize) % 8;
    POINTS[idx]
}

pub fn compute_dipole_alignment(
    pos: SatPosition,
    dipole_type: DipoleType,
) -> Option<DipoleAlignment> {
    match dipole_type {
        DipoleType::VDipole137 => {
            // 9A4QV: horizontal V-dipole, 53.4 cm arms at 120°
            // V-opening faces satellite azimuth for best reception
            Some(DipoleAlignment {
                compass_heading: pos.azimuth,
                elevation_tilt: 0.0,
                dipole_type,
            })
        }
        DipoleType::Turnstile => Some(DipoleAlignment {
            compass_heading: pos.azimuth,
            elevation_tilt: 0.0,
            dipole_type,
        }),
        DipoleType::Qfh => Some(DipoleAlignment {
            compass_heading: pos.azimuth,
            elevation_tilt: 0.0,
            dipole_type,
        }),
        DipoleType::Helical => Some(DipoleAlignment {
            compass_heading: pos.azimuth,
            elevation_tilt: (90.0 - pos.elevation).max(0.0),
            dipole_type,
        }),
    }
}

pub fn compass_rose_ui(
    ui: &mut egui::Ui,
    heading_deg: f64,
    azimuth: f64,
    elevation: f64,
    distance_km: f64,
) {
    let size = ui.available_width().min(ui.available_height()).min(520.0);
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());

    if size <= 0.0 {
        return;
    }

    let painter = ui.painter();
    let center = rect.center();
    let radius = size * 0.38;

    // Background ring
    painter.circle_stroke(center, radius, Stroke::new(2.0, Color32::from_gray(60)));

    // Cardinal directions
    let cardinals: [(f64, &str); 8] = [
        (0.0, "N"),
        (45.0, "NE"),
        (90.0, "E"),
        (135.0, "SE"),
        (180.0, "S"),
        (225.0, "SW"),
        (270.0, "W"),
        (315.0, "NW"),
    ];

    for (angle_deg, label) in cardinals {
        let rad = angle_deg.to_radians();
        let pos = center
            + Vec2::new(
                radius * 0.85 * rad.sin() as f32,
                -radius * 0.85 * rad.cos() as f32,
            );
        painter.text(
            pos,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.0),
            Color32::from_gray(140),
        );
    }

    // Satellite direction arrow (azimuth from observer)
    let sat_rad = azimuth.to_radians();
    let arrow_len = radius * 0.65;
    let tip = center
        + Vec2::new(
            arrow_len * sat_rad.sin() as f32,
            -arrow_len * sat_rad.cos() as f32,
        );
    let base = center;
    let arrow_color = Color32::from_rgb(0, 220, 255);

    // Arrow shaft
    painter.line_segment([base, tip], Stroke::new(2.0, arrow_color));
    // Arrow head
    let head_size = 10.0;
    let head_angle = 0.4;
    let left = tip
        + Vec2::new(
            head_size * (sat_rad + head_angle).sin() as f32,
            -head_size * (sat_rad + head_angle).cos() as f32,
        );
    let right = tip
        + Vec2::new(
            head_size * (sat_rad - head_angle).sin() as f32,
            -head_size * (sat_rad - head_angle).cos() as f32,
        );
    painter.add(Shape::convex_polygon(
        vec![tip, left, right],
        arrow_color,
        Stroke::new(1.0, Color32::from_gray(0)),
    ));

    // Heading label
    painter.text(
        center + Vec2::new(0.0, -8.0),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}°", heading_deg),
        egui::FontId::proportional(26.0),
        Color32::from_rgb(0, 220, 255),
    );
    // Cardinal direction for the V-opening, directly under the bearing.
    painter.text(
        center + Vec2::new(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        azimuth_to_cardinal(heading_deg),
        egui::FontId::proportional(14.0),
        Color32::from_rgb(0, 255, 150),
    );

    // Inner ring
    painter.circle_stroke(
        center,
        radius * 0.3,
        Stroke::new(1.0, Color32::from_gray(40)),
    );

    // Elevation arc (small indicator below main compass)
    let el_rad = (90.0 - elevation).clamp(0.0, 90.0).to_radians();
    let el_len = radius * 0.25;
    let el_tip = center
        + Vec2::new(0.0, -el_len * (el_rad.sin()) as f32)
        + Vec2::new(radius * 0.2, radius * 0.3);
    let el_base = center + Vec2::new(radius * 0.2, radius * 0.3);
    painter.line_segment(
        [el_base, el_tip],
        Stroke::new(2.0, Color32::from_rgb(0, 255, 100)),
    );
    painter.text(
        el_tip + Vec2::new(0.0, -12.0),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}° el", elevation),
        egui::FontId::proportional(10.0),
        Color32::from_rgb(0, 255, 100),
    );

    // Distance text
    painter.text(
        center + Vec2::new(0.0, radius * 0.5),
        egui::Align2::CENTER_CENTER,
        format!("{:.0} km", distance_km),
        egui::FontId::proportional(10.0),
        Color32::from_gray(100),
    );
}

pub fn alignment_info_ui(ui: &mut egui::Ui, align: &DipoleAlignment, pos: &SatPosition) {
    ui.group(|ui| {
        ui.label(
            egui::RichText::new("Dipole Orientation")
                .size(15.0)
                .strong(),
        );
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("Antenna:");
            ui.colored_label(Color32::from_rgb(0, 200, 255), align.dipole_type.label());
        });

        // Primary instruction: which way the V-opening should point, as a
        // human-readable cardinal direction plus the exact bearing.
        ui.add_space(2.0);
        let cardinal = azimuth_to_cardinal(align.compass_heading);
        ui.horizontal(|ui| {
            ui.label("Point V-opening:");
            ui.colored_label(
                Color32::from_rgb(0, 255, 150),
                egui::RichText::new(format!("{}  ({:.0}°)", cardinal, align.compass_heading))
                    .size(22.0)
                    .strong(),
            );
        });

        ui.horizontal(|ui| {
            ui.label("Tilt:");
            if align.elevation_tilt > 0.0 {
                ui.colored_label(
                    Color32::from_rgb(255, 200, 80),
                    format!("{:.0}° from horizontal", align.elevation_tilt),
                );
            } else {
                ui.colored_label(Color32::GREEN, "Flat (horizontal)");
            }
        });

        ui.separator();
        ui.add_space(4.0);

        ui.label(egui::RichText::new("Live Satellite Position").strong());
        ui.horizontal(|ui| {
            ui.label("Azimuth:");
            ui.monospace(format!("{:.1}°", pos.azimuth));
        });
        ui.horizontal(|ui| {
            ui.label("Elevation:");
            ui.monospace(format!("{:.1}°", pos.elevation));
        });
        ui.horizontal(|ui| {
            ui.label("Distance:");
            ui.monospace(format!("{:.0} km", pos.distance_km));
        });
    });

    ui.add_space(8.0);

    // Reference card
    ui.group(|ui| {
        ui.label(egui::RichText::new("V-Dipole Reference (9A4QV)").strong());
        ui.add_space(4.0);
        ui.label("• Arm length: 53.4 cm each (quarter-wave @ 137.5 MHz)");
        ui.label("• Arm angle: 120° (V-shape)");
        ui.label("• Mount: HORIZONTAL on mast");
        ui.label("• Arms: North-South alignment");
        ui.label("• V-opening: FACES satellite azimuth");
        ui.label("• Feed: 75Ω coax + 1:1 balun at center");
        ui.label("• LNA: SAWbird+ NOAA at feedpoint (bias-tee)");
        ui.add_space(4.0);
        ui.colored_label(
            Color32::from_gray(140),
            "Horizontal polarization rejects vertical FM broadcast by ~20 dB.",
        );
    });
}

pub fn dipole_tutorial_ui(ui: &mut egui::Ui) {
    ui.collapsing("📖 Antenna Build Guide", |ui| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Why 9A4QV V-Dipole?").strong());
        ui.add_space(2.0);
        ui.label("The 9A4QV V-dipole is the most popular ground-based antenna for NOAA/Meteor weather satellite reception at 137 MHz:");
        ui.add_space(4.0);
        ui.label("1. Simple to build — two 53.4 cm wires/elements");
        ui.label("2. Good gain at low elevations where LEO satellites spend most of their pass");
        ui.label("3. Horizontal polarization naturally rejects terrestrial FM broadcast (vertical) by ~20 dB");
        ui.label("4. Lightweight — can be mounted on a camera tripod or mast");
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Step-by-step build:").strong());
        ui.add_space(2.0);
        ui.label("1. Cut two 53.4 cm lengths of wire (solid copper, 1.5-2.5 mm²)");
        ui.label("2. Solder each to the center and shield of a 75Ω coax (RG-6 works well)");
        ui.label("3. Mount on a cross-boom (wood/plastic, ~30 cm) at exactly 120° angle");
        ui.label("4. Attach a 1:1 current balun (ferrite core) at the feedpoint");
        ui.label("5. Place on a tripod or mast, minimum 1 m above roof");
        ui.label("6. Align arms North-South, V-opening facing your target pass azimuth");
        ui.label("7. Connect via bias-tee LNA → coax → SDR (keep coax < 10 m)");
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(Color32::from_rgb(80, 200, 120), "TIP");
            ui.separator();
            ui.label("Use the compass above to rotate the V-dipole so the opening points toward the satellite azimuth. Re-aim for each pass for maximum signal.");
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinal_maps_exact_bearings() {
        assert_eq!(azimuth_to_cardinal(0.0), "North");
        assert_eq!(azimuth_to_cardinal(45.0), "Northeast");
        assert_eq!(azimuth_to_cardinal(90.0), "East");
        assert_eq!(azimuth_to_cardinal(135.0), "Southeast");
        assert_eq!(azimuth_to_cardinal(180.0), "South");
        assert_eq!(azimuth_to_cardinal(225.0), "Southwest");
        assert_eq!(azimuth_to_cardinal(270.0), "West");
        assert_eq!(azimuth_to_cardinal(315.0), "Northwest");
    }

    #[test]
    fn cardinal_snaps_within_sector() {
        // 22.5° is the North/Northeast boundary → rounds up to Northeast.
        assert_eq!(azimuth_to_cardinal(22.5), "Northeast");
        // Just below the boundary stays North.
        assert_eq!(azimuth_to_cardinal(22.0), "North");
        assert_eq!(azimuth_to_cardinal(350.0), "North");
    }

    #[test]
    fn cardinal_wraps_out_of_range_bearings() {
        assert_eq!(azimuth_to_cardinal(360.0), "North");
        assert_eq!(azimuth_to_cardinal(405.0), "Northeast");
        assert_eq!(azimuth_to_cardinal(-45.0), "Northwest");
    }
}
