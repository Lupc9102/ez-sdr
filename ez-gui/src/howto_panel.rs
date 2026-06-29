pub struct HowToPanel {
    pub selected_section: usize,
}

const SECTIONS: &[&str] = &[
    "What is SDR?",
    "RTL-SDR Hardware",
    "Antennas & Positioning",
    "SDR Panel Controls",
    "Spectrum & Waterfall",
    "Demodulation Modes",
    "ADS-B Aircraft Tracking",
    "Satellite Tracking (NOAA)",
    "Frequency Scanner",
    "Recorder",
    "Bookmarks & Scheduler",
    "Noise & Interference",
    "Frequency Reference",
    "SoapySDR & Other Hardware",
];

impl HowToPanel {
    pub fn new() -> Self {
        Self { selected_section: 0 }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("howto_sidebar")
                .max_height(f32::INFINITY)
                .show(ui, |ui| {
                    ui.set_width(175.0);
                    ui.heading("Topics");
                    ui.separator();
                    for (i, section) in SECTIONS.iter().enumerate() {
                        if ui.selectable_label(self.selected_section == i, *section).clicked() {
                            self.selected_section = i;
                        }
                    }
                });

            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("howto_content")
                .show(ui, |ui| {
                    match self.selected_section {
                        0  => self.section_what_is_sdr(ui),
                        1  => self.section_rtlsdr_hardware(ui),
                        2  => self.section_antennas(ui),
                        3  => self.section_sdr_panel(ui),
                        4  => self.section_spectrum(ui),
                        5  => self.section_demod_modes(ui),
                        6  => self.section_adsb(ui),
                        7  => self.section_satellite(ui),
                        8  => self.section_scanner(ui),
                        9  => self.section_recorder(ui),
                        10 => self.section_bookmarks(ui),
                        11 => self.section_noise(ui),
                        12 => self.section_freq_reference(ui),
                        13 => self.section_soapy(ui),
                        _  => {}
                    }
                });
        });
    }

    // ─── helpers ──────────────────────────────────────────────────────────

    fn h1(ui: &mut egui::Ui, text: &str) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text).size(20.0).strong());
        ui.add_space(6.0);
    }

    fn h2(ui: &mut egui::Ui, text: &str) {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(text).size(14.0).strong());
        ui.add_space(2.0);
    }

    fn tip(ui: &mut egui::Ui, text: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "TIP");
            ui.separator();
            ui.label(text);
        });
    }

    fn warn(ui: &mut egui::Ui, text: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(255, 180, 0), "NOTE");
            ui.separator();
            ui.label(text);
        });
    }

    fn bad(ui: &mut egui::Ui, text: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(255, 80, 80), "AVOID");
            ui.separator();
            ui.label(text);
        });
    }

    // ─── diagrams ─────────────────────────────────────────────────────────

    fn draw_signal_chain(ui: &mut egui::Ui) {
        let w = ui.available_width().min(580.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 54.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 6.0, egui::Color32::from_rgb(18, 22, 30));

        let labels = ["Antenna", "LNA / Filter", "ADC", "CPU / Software"];
        let colors = [
            egui::Color32::from_rgb(60, 100, 60),
            egui::Color32::from_rgb(60, 80, 120),
            egui::Color32::from_rgb(80, 60, 100),
            egui::Color32::from_rgb(40, 80, 140),
        ];
        let n = labels.len();
        let box_w = 110.0_f32;
        let gap   = (w - box_w * n as f32) / (n as f32 + 1.0);
        let cy    = rect.center().y;

        for (i, (label, color)) in labels.iter().zip(colors.iter()).enumerate() {
            let cx = rect.left() + gap + box_w * i as f32 + gap * i as f32 + box_w / 2.0;
            let r  = egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(box_w - 6.0, 32.0));
            p.rect_filled(r, 4.0, *color);
            p.rect_stroke(r, 4.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 130, 100)), egui::StrokeKind::Middle);
            p.text(r.center(), egui::Align2::CENTER_CENTER, label,
                egui::FontId::proportional(11.0), egui::Color32::WHITE);
            if i < n - 1 {
                let ax  = r.right() + 1.0;
                let ax2 = ax + gap - 2.0;
                let arr = egui::Stroke::new(1.5, egui::Color32::from_rgb(200, 200, 100));
                p.line_segment([egui::pos2(ax, cy), egui::pos2(ax2, cy)], arr);
                p.line_segment([egui::pos2(ax2, cy), egui::pos2(ax2 - 5.0, cy - 4.0)], arr);
                p.line_segment([egui::pos2(ax2, cy), egui::pos2(ax2 - 5.0, cy + 4.0)], arr);
            }
        }
    }

    fn draw_iq_diagram(ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 160.0), egui::Sense::hover());
        let p = ui.painter();
        let c = rect.center();
        let r = 62.0_f32;

        p.rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 28));
        p.circle_stroke(c, r, egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 55, 75)));

        let axis = egui::Stroke::new(1.0, egui::Color32::from_rgb(110, 110, 110));
        p.line_segment([egui::pos2(c.x - r - 8.0, c.y), egui::pos2(c.x + r + 8.0, c.y)], axis);
        p.line_segment([egui::pos2(c.x, c.y - r - 8.0), egui::pos2(c.x, c.y + r + 8.0)], axis);
        p.text(egui::pos2(c.x + r + 12.0, c.y), egui::Align2::LEFT_CENTER, "I",
            egui::FontId::proportional(13.0), egui::Color32::from_rgb(100, 210, 100));
        p.text(egui::pos2(c.x, c.y - r - 12.0), egui::Align2::CENTER_BOTTOM, "Q",
            egui::FontId::proportional(13.0), egui::Color32::from_rgb(100, 210, 100));

        let px = c.x + 38.0;
        let py = c.y - 33.0;
        p.line_segment([c, egui::pos2(px, py)],
            egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 210, 50)));
        p.line_segment([egui::pos2(px, c.y), egui::pos2(px, py)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 100, 230)));
        p.line_segment([egui::pos2(c.x, py), egui::pos2(px, py)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(230, 80, 80)));
        p.circle_filled(egui::pos2(px, py), 4.0, egui::Color32::from_rgb(255, 80, 80));
        p.text(egui::pos2(px + 5.0, (c.y + py) / 2.0), egui::Align2::LEFT_CENTER, "Q val",
            egui::FontId::proportional(9.0), egui::Color32::from_rgb(100, 130, 230));
        p.text(egui::pos2((c.x + px) / 2.0, c.y + 5.0), egui::Align2::CENTER_TOP, "I val",
            egui::FontId::proportional(9.0), egui::Color32::from_rgb(230, 100, 100));
        p.text(egui::pos2(px + 5.0, py - 2.0), egui::Align2::LEFT_BOTTOM, "sample",
            egui::FontId::proportional(9.0), egui::Color32::from_rgb(255, 210, 50));
    }

    fn draw_monopole(ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 110.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 28));
        let c = rect.center();
        // ground plane
        let gp = egui::Stroke::new(2.0, egui::Color32::from_rgb(160, 150, 100));
        p.line_segment([egui::pos2(c.x - 42.0, c.y + 22.0), egui::pos2(c.x + 42.0, c.y + 22.0)], gp);
        for dx in [-28.0_f32, -14.0, 0.0, 14.0, 28.0] {
            p.line_segment([egui::pos2(c.x + dx, c.y + 22.0), egui::pos2(c.x + dx - 8.0, c.y + 33.0)],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(130, 120, 80)));
        }
        // element
        p.line_segment([egui::pos2(c.x, c.y + 22.0), egui::pos2(c.x, c.y - 48.0)],
            egui::Stroke::new(2.5, egui::Color32::from_rgb(220, 220, 220)));
        // brace
        p.line_segment([egui::pos2(c.x + 2.0, c.y + 22.0), egui::pos2(c.x + 2.0, c.y - 48.0)],
            egui::Stroke::new(0.5, egui::Color32::from_rgb(100, 100, 100)));
        p.text(egui::pos2(c.x + 8.0, c.y - 14.0), egui::Align2::LEFT_CENTER, "λ/4",
            egui::FontId::proportional(11.0), egui::Color32::from_rgb(150, 230, 150));
        p.text(egui::pos2(c.x, c.y + 42.0), egui::Align2::CENTER_TOP, "ground plane",
            egui::FontId::proportional(9.0), egui::Color32::GRAY);
    }

    fn draw_vdipole(ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(220.0, 120.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 28));
        let c = rect.center();
        let arm = 78.0_f32;
        let angle = 60.0_f32.to_radians();
        let a1 = egui::pos2(c.x - arm * angle.sin(), c.y - arm * angle.cos());
        let a2 = egui::pos2(c.x + arm * angle.sin(), c.y - arm * angle.cos());
        let el  = egui::Stroke::new(2.5, egui::Color32::from_rgb(220, 220, 220));
        p.line_segment([c, a1], el);
        p.line_segment([c, a2], el);
        p.circle_filled(c, 4.0, egui::Color32::from_rgb(255, 200, 50));
        // angle arc
        p.circle_stroke(c, 22.0, egui::Stroke::new(0.5, egui::Color32::from_rgb(80, 80, 100)));
        p.text(egui::pos2(c.x, c.y + 5.0), egui::Align2::CENTER_TOP, "120°",
            egui::FontId::proportional(9.0), egui::Color32::from_rgb(140, 170, 140));
        // labels
        p.text(egui::pos2(a1.x - 4.0, a1.y - 4.0), egui::Align2::RIGHT_BOTTOM, "54.7 cm",
            egui::FontId::proportional(9.0), egui::Color32::from_rgb(180, 230, 180));
        p.text(egui::pos2(a2.x + 4.0, a2.y - 4.0), egui::Align2::LEFT_BOTTOM, "54.7 cm",
            egui::FontId::proportional(9.0), egui::Color32::from_rgb(180, 230, 180));
        p.text(egui::pos2(c.x, c.y + 26.0), egui::Align2::CENTER_TOP, "coax feed · orient horizontal",
            egui::FontId::proportional(9.0), egui::Color32::GRAY);
        p.text(egui::pos2(rect.center().x, rect.bottom() - 2.0), egui::Align2::CENTER_BOTTOM,
            "NOAA APT V-dipole @ 137 MHz", egui::FontId::proportional(9.0), egui::Color32::from_rgb(100, 180, 255));
    }

    fn draw_lna_chain(ui: &mut egui::Ui) {
        let w = ui.available_width().min(540.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 80.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 6.0, egui::Color32::from_rgb(18, 22, 30));

        // GOOD chain
        let good_y = rect.top() + 20.0;
        p.text(egui::pos2(rect.left() + 4.0, good_y), egui::Align2::LEFT_CENTER,
            "GOOD:", egui::FontId::proportional(10.0), egui::Color32::from_rgb(80, 220, 80));
        let items_good = ["Antenna", "LNA (mast)", "Coax", "SDR Dongle"];
        for (i, label) in items_good.iter().enumerate() {
            let x = rect.left() + 60.0 + i as f32 * 110.0;
            let r = egui::Rect::from_center_size(egui::pos2(x, good_y), egui::vec2(90.0, 22.0));
            let c = if *label == "LNA (mast)" { egui::Color32::from_rgb(50, 100, 50) }
                    else { egui::Color32::from_rgb(40, 55, 80) };
            p.rect_filled(r, 3.0, c);
            p.text(r.center(), egui::Align2::CENTER_CENTER, label,
                egui::FontId::proportional(9.5), egui::Color32::WHITE);
            if i < items_good.len() - 1 {
                let arr = egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 200, 100));
                let ax = r.right() + 1.0;
                let ax2 = ax + 18.0;
                p.line_segment([egui::pos2(ax, good_y), egui::pos2(ax2, good_y)], arr);
                p.line_segment([egui::pos2(ax2, good_y), egui::pos2(ax2 - 4.0, good_y - 3.0)], arr);
                p.line_segment([egui::pos2(ax2, good_y), egui::pos2(ax2 - 4.0, good_y + 3.0)], arr);
            }
        }

        // BAD chain
        let bad_y = rect.top() + 58.0;
        p.text(egui::pos2(rect.left() + 4.0, bad_y), egui::Align2::LEFT_CENTER,
            "BAD:", egui::FontId::proportional(10.0), egui::Color32::from_rgb(255, 80, 80));
        let items_bad = ["Antenna", "Long Coax", "LNA", "SDR Dongle"];
        for (i, label) in items_bad.iter().enumerate() {
            let x = rect.left() + 60.0 + i as f32 * 110.0;
            let r = egui::Rect::from_center_size(egui::pos2(x, bad_y), egui::vec2(90.0, 22.0));
            let c = if *label == "Long Coax" { egui::Color32::from_rgb(100, 40, 40) }
                    else { egui::Color32::from_rgb(60, 45, 45) };
            p.rect_filled(r, 3.0, c);
            p.text(r.center(), egui::Align2::CENTER_CENTER, label,
                egui::FontId::proportional(9.5), egui::Color32::WHITE);
            if i < items_bad.len() - 1 {
                let arr = egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 100, 100));
                let ax = r.right() + 1.0;
                let ax2 = ax + 18.0;
                p.line_segment([egui::pos2(ax, bad_y), egui::pos2(ax2, bad_y)], arr);
                p.line_segment([egui::pos2(ax2, bad_y), egui::pos2(ax2 - 4.0, bad_y - 3.0)], arr);
                p.line_segment([egui::pos2(ax2, bad_y), egui::pos2(ax2 - 4.0, bad_y + 3.0)], arr);
            }
        }
        // loss callout
        p.text(egui::pos2(rect.left() + 170.0, bad_y + 14.0), egui::Align2::CENTER_TOP,
            "cable loss raises noise floor BEFORE LNA", egui::FontId::proportional(8.5),
            egui::Color32::from_rgb(255, 130, 130));
    }

    fn draw_freq_chart(ui: &mut egui::Ui) {
        let w = ui.available_width().min(680.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 56.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 28));

        let f_min = 80.0_f32;
        let f_max = 1800.0_f32;
        let to_x  = |f: f32| rect.left() + (f - f_min) / (f_max - f_min) * rect.width();

        let bands: &[(f32, f32, [u8;3], &str)] = &[
            (88.0,  108.0,  [50,  100, 200], "FM"),
            (108.0, 118.0,  [80,  160, 80],  "Nav"),
            (118.0, 137.0,  [200, 150, 40],  "Air"),
            (137.0, 138.5,  [150, 80,  200], "Sat"),
            (144.0, 148.0,  [60,  180, 80],  "2m"),
            (162.0, 163.5,  [60,  200, 200], "WX"),
            (420.0, 450.0,  [60,  180, 80],  "70cm"),
            (432.0, 436.0,  [200, 200, 40],  "ISM"),
            (1088.0,1092.0, [220, 80,  80],  "ADS-B"),
            (1575.0,1576.5, [80,  160, 220], "GPS"),
        ];

        for &(fs, fe, col, label) in bands {
            let x1 = to_x(fs);
            let x2 = (to_x(fe)).max(x1 + 4.0);
            let bar = egui::Rect::from_min_max(
                egui::pos2(x1, rect.top() + 10.0),
                egui::pos2(x2, rect.bottom() - 14.0));
            p.rect_filled(bar, 2.0, egui::Color32::from_rgb(col[0], col[1], col[2]));
            if x2 - x1 > 14.0 {
                p.text(bar.center(), egui::Align2::CENTER_CENTER, label,
                    egui::FontId::proportional(8.0), egui::Color32::WHITE);
            }
        }

        // tick marks
        for f in [100.0_f32, 200.0, 400.0, 600.0, 800.0, 1000.0, 1200.0, 1400.0, 1600.0] {
            let x = to_x(f);
            p.line_segment([egui::pos2(x, rect.bottom() - 14.0), egui::pos2(x, rect.bottom() - 10.0)],
                egui::Stroke::new(0.8, egui::Color32::GRAY));
            p.text(egui::pos2(x, rect.bottom() - 8.0), egui::Align2::CENTER_TOP,
                format!("{}M", f as u32), egui::FontId::proportional(7.5), egui::Color32::GRAY);
        }
    }

    fn draw_polarization(ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(260.0, 90.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 28));

        // vertical antenna left
        let lx = rect.left() + 50.0;
        let cy = rect.center().y;
        p.line_segment([egui::pos2(lx, cy + 30.0), egui::pos2(lx, cy - 30.0)],
            egui::Stroke::new(3.0, egui::Color32::from_rgb(100, 220, 100)));
        p.text(egui::pos2(lx, cy + 34.0), egui::Align2::CENTER_TOP, "Vertical TX",
            egui::FontId::proportional(8.5), egui::Color32::from_rgb(100, 220, 100));

        // arrows showing radiated wave
        for dy in [-12.0_f32, 0.0, 12.0] {
            let ax = lx + 20.0;
            let arr = egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 200, 80));
            p.line_segment([egui::pos2(ax, cy + dy), egui::pos2(ax + 30.0, cy + dy)], arr);
            p.line_segment([egui::pos2(ax + 30.0, cy + dy), egui::pos2(ax + 25.0, cy + dy - 3.0)], arr);
            p.line_segment([egui::pos2(ax + 30.0, cy + dy), egui::pos2(ax + 25.0, cy + dy + 3.0)], arr);
        }

        // vertical RX (matched) — green
        let rx1 = rect.left() + 140.0;
        p.line_segment([egui::pos2(rx1, cy + 30.0), egui::pos2(rx1, cy - 30.0)],
            egui::Stroke::new(3.0, egui::Color32::from_rgb(100, 220, 100)));
        p.text(egui::pos2(rx1, cy + 34.0), egui::Align2::CENTER_TOP, "✓ Vertical RX",
            egui::FontId::proportional(8.5), egui::Color32::from_rgb(100, 220, 100));
        p.text(egui::pos2(rx1, cy - 36.0), egui::Align2::CENTER_BOTTOM, "0 dB loss",
            egui::FontId::proportional(8.5), egui::Color32::from_rgb(100, 220, 100));

        // horizontal RX (mismatched) — red
        let rx2 = rect.right() - 40.0;
        p.line_segment([egui::pos2(rx2 - 25.0, cy), egui::pos2(rx2 + 25.0, cy)],
            egui::Stroke::new(3.0, egui::Color32::from_rgb(220, 80, 80)));
        p.text(egui::pos2(rx2, cy + 34.0), egui::Align2::CENTER_TOP, "✗ Horiz RX",
            egui::FontId::proportional(8.5), egui::Color32::from_rgb(220, 80, 80));
        p.text(egui::pos2(rx2, cy - 36.0), egui::Align2::CENTER_BOTTOM, "~20 dB loss",
            egui::FontId::proportional(8.5), egui::Color32::from_rgb(220, 80, 80));
    }

    // ─── sections ─────────────────────────────────────────────────────────

    fn section_what_is_sdr(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "What is Software Defined Radio?");

        ui.label("In a traditional radio, every processing step — mixing, filtering, demodulation — is done in dedicated analog hardware. In an SDR the antenna feeds raw RF into an Analog-to-Digital Converter (ADC), and nearly all signal processing happens in software on a CPU.");
        ui.add_space(8.0);

        Self::draw_signal_chain(ui);
        ui.label(egui::RichText::new("Signal chain: everything after the ADC is software").italics()
            .color(egui::Color32::GRAY).size(10.0));
        ui.add_space(10.0);

        Self::h2(ui, "I/Q Sampling — the core concept");
        ui.label("A single real-valued ADC cannot capture phase information. SDRs sample two components simultaneously, offset by exactly 90°:");
        ui.label("  •  I (In-phase)    — samples the signal directly");
        ui.label("  •  Q (Quadrature) — same signal shifted 90°");
        ui.label("Each instant produces one complex number I + jQ. This preserves amplitude AND phase, which is required to demodulate AM, FM, SSB, PSK, QAM, or anything else entirely in software.");
        ui.add_space(8.0);

        Self::draw_iq_diagram(ui);
        ui.label(egui::RichText::new("Each red dot is one I/Q sample. The distance from origin = amplitude; angle = phase.")
            .italics().color(egui::Color32::GRAY).size(10.0));
        ui.add_space(10.0);

        Self::h2(ui, "Why SDR beats a fixed-function radio for exploration");
        ui.label("  •  One piece of hardware covers the entire spectrum your dongle can reach");
        ui.label("  •  Switch modulation mode in software — no hardware changes");
        ui.label("  •  Record raw I/Q and replay it offline, months later, with different demodulation");
        ui.label("  •  Spectrum and waterfall let you see every signal simultaneously");

        ui.add_space(8.0);
        Self::tip(ui, "The RTL-SDR processes all signals across its 2.4 MHz bandwidth simultaneously. Even if you're listening to one FM station, you can see all adjacent FM stations in the waterfall.");
    }

    fn section_rtlsdr_hardware(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "RTL-SDR Hardware");

        ui.label("The RTL-SDR is a ~$25–35 USB dongle originally designed for DVB-T TV reception, repurposed for wideband software-defined radio. It uses the Realtek RTL2832U USB chip paired with a Rafael Micro R820T2 tuner.");
        ui.add_space(8.0);

        Self::h2(ui, "Key specifications");
        egui::Grid::new("rtlsdr_specs").num_columns(2).striped(true).min_col_width(160.0).show(ui, |ui| {
            for (k, v) in &[
                ("Frequency range",         "24 MHz – 1766 MHz (with R820T2 tuner)"),
                ("HF / direct sampling",    "~500 kHz – 24 MHz (V3 only, quality varies)"),
                ("ADC resolution",          "8-bit → 256 amplitude levels → ~48 dB dynamic range"),
                ("Max stable sample rate",  "2.4 MHz  (3.2 MHz rated; drops samples above 2.4)"),
                ("Recommended sample rates","1.024 / 1.536 / 2.048 / 2.4 MHz"),
                ("Receive / Transmit",      "Receive ONLY — cannot transmit"),
                ("USB interface",           "USB 2.0 (use USB 2.0 port/hub for lowest noise)"),
            ] {
                ui.label(egui::RichText::new(*k).strong());
                ui.label(*v);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "RTL-SDR Blog V3 improvements over cheap clones");
        ui.label("If you have a choice, get the official RTL-SDR Blog V3:");
        egui::Grid::new("v3_features").num_columns(2).striped(true).show(ui, |ui| {
            for (feat, desc) in &[
                ("1 PPM TCXO",             "Temperature-stable crystal — leave PPM correction at 0"),
                ("Bias tee (4.5 V)",       "Powers mast-mounted LNAs over the coax center pin; software-enabled"),
                ("Metal enclosure",        "Shields the PCB from external RFI and reduces self-noise"),
                ("USB line filtering",     "Ferrite + capacitors block computer noise entering via USB cable"),
                ("Direct sampling header", "SMA input pins for sub-24 MHz HF reception"),
            ] {
                ui.label(egui::RichText::new(*feat).strong());
                ui.label(*desc);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "PPM frequency offset");
        ui.label("Cheap dongles without a TCXO have crystal errors of 5–100 PPM. At 1090 MHz (ADS-B), 10 PPM = 10.9 kHz offset. Signals will appear shifted from their true frequency.");
        ui.label("To calibrate: tune to a known-exact signal (GSM tower, GPS L1 at 1575.42 MHz, an airport ATIS), adjust PPM until the peak aligns with the known frequency.");
        Self::tip(ui, "Warm the dongle for 15–20 minutes before calibrating PPM — crystal frequency drifts significantly as it heats up. The V3 TCXO eliminates this entirely.");
        Self::warn(ui, "At 1090 MHz, a 10 PPM error = 10.9 kHz. ADS-B frames are only 2 MHz wide — you won't miss them — but your frequency readout will be wrong.");
    }

    fn section_antennas(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Antennas & Positioning");

        ui.label("The antenna is the most impactful part of any receive chain. A great antenna with a mediocre dongle beats a mediocre antenna with an expensive dongle every time.");
        ui.add_space(8.0);

        Self::h2(ui, "Quarter-wave length formula");
        ui.label(egui::RichText::new("    Element length (cm) = 7500 ÷ frequency (MHz)")
            .monospace().color(egui::Color32::from_rgb(140, 220, 140)).size(13.0));
        ui.add_space(4.0);

        egui::Grid::new("ant_lengths").num_columns(3).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Use case").strong());
            ui.label(egui::RichText::new("Freq").strong());
            ui.label(egui::RichText::new("Element length").strong());
            ui.end_row();
            for (use_case, freq, len) in &[
                ("NOAA APT (each arm)",  "137 MHz",   "54.7 cm"),
                ("NOAA Weather Radio",   "162 MHz",   "46.3 cm"),
                ("Aircraft airband",     "125 MHz",   "60.0 cm"),
                ("ADS-B aircraft",       "1090 MHz",  "6.9 cm"),
                ("FM Broadcast",         "98 MHz",    "76.5 cm"),
                ("2m Amateur (FM)",      "144 MHz",   "52.1 cm"),
                ("70cm Amateur",         "433 MHz",   "17.3 cm"),
                ("ACARS / Airband",      "130 MHz",   "57.7 cm"),
                ("AIS Maritime",         "162 MHz",   "46.3 cm"),
            ] {
                ui.label(*use_case);
                ui.colored_label(egui::Color32::from_rgb(180, 180, 180), *freq);
                ui.colored_label(egui::Color32::from_rgb(140, 220, 140), *len);
                ui.end_row();
            }
        });

        ui.add_space(12.0);
        Self::h2(ui, "Antenna types");

        egui::CollapsingHeader::new("Monopole / Whip  (general scanning)").default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(340.0);
                    ui.label("A single vertical element over a metal ground plane. λ/4 long. Vertically polarized, omnidirectional in the horizontal plane.");
                    ui.label("Best for: FM broadcast, VHF/UHF scanning, ADS-B, land mobile, everything general.");
                    Self::tip(ui, "The ground plane should be at least λ/4 wide in every direction. A biscuit tin lid works as a quick ground plane.");
                });
                ui.add_space(12.0);
                Self::draw_monopole(ui);
            });
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new("V-Dipole  (NOAA weather satellites)").default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(300.0);
                    ui.label("Two equal arms spread at ~120° opening angle, mounted horizontally. Ideal for LEO satellite passes.");
                    ui.label("Each arm 54.7 cm for 137 MHz. Mount it horizontal, arms pointing East–West for North–South pass coverage.");
                    Self::tip(ui, "A horizontal V-dipole provides ~20 dB rejection of vertical polarized ground signals (land mobile, FM), which boosts the satellite SNR significantly.");
                    Self::warn(ui, "You need a clear horizon in all directions — trees and buildings block low-elevation parts of the satellite pass.");
                });
                ui.add_space(12.0);
                Self::draw_vdipole(ui);
            });
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new("Discone  (wideband, best all-rounder)").default_open(false).show(ui, |ui| {
            ui.label("Wideband (up to 10:1 frequency ratio). One antenna covers ~25 MHz to 1.3+ GHz. Vertically polarized, omnidirectional.");
            ui.label("Popular commercial models: Tram 1411 (~$30), Diamond D130J. A DIY version from copper rods costs $5–15.");
            ui.label("Disc element diameter ≈ 0.7 × (λ/4 at lowest frequency). Cone element length = λ/4 at lowest frequency.");
            Self::tip(ui, "If you only buy one permanent antenna for an SDR, buy a discone. It covers everything from low VHF to L-band with no tuning.");
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new("Yagi  (directional, high-gain)").default_open(false).show(ui, |ui| {
            ui.label("High gain (6–15 dBd depending on element count) in one direction. Must be physically pointed at the target.");
            ui.label("Use for: weak satellite signals, long-range ADS-B, distant repeaters, DXing.");
            Self::bad(ui, "Don't use a Yagi for scanning — it has narrow bandwidth and you must keep pointing it at the target.");
            Self::warn(ui, "Polarization must match the target. A horizontal Yagi aimed at a vertical antenna loses ~20 dB immediately.");
        });

        ui.add_space(12.0);
        Self::h2(ui, "Polarization — why it matters");
        Self::draw_polarization(ui);
        ui.add_space(4.0);

        egui::Grid::new("polar_table").num_columns(2).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Mismatch").strong());
            ui.label(egui::RichText::new("Loss").strong());
            ui.end_row();
            ui.label("Vertical TX  →  Vertical RX  (matched)");
            ui.colored_label(egui::Color32::from_rgb(100, 220, 100), "0 dB — full signal");
            ui.end_row();
            ui.label("Vertical TX  →  Horizontal RX  (90° cross)");
            ui.colored_label(egui::Color32::from_rgb(255, 80, 80), "~20 dB — 100× weaker!");
            ui.end_row();
            ui.label("Circular TX  →  Linear RX  (mixed)");
            ui.colored_label(egui::Color32::from_rgb(255, 180, 0), "~3 dB — acceptable");
            ui.end_row();
        });
        ui.label("Vertically polarized (use a vertical antenna): FM broadcast, land mobile radio, ADS-B, airband, marine VHF, APRS, most repeaters.");
        ui.label("Note: below 30 MHz (HF), the ionosphere randomizes polarization — it doesn't matter there.");

        ui.add_space(10.0);
        Self::h2(ui, "Placement tips");
        Self::tip(ui, "Outdoors beats indoors by 10–30 dB. Building walls and metal-frame construction absorb VHF/UHF badly.");
        Self::tip(ui, "Every meter of height clears more of the horizon. On a chimney or rooftop is ideal.");
        Self::tip(ui, "Keep coax runs as short as possible. Use LMR-400 or equivalent low-loss cable for runs over 5 meters. Cheap RG-58 loses ~1 dB/meter at 1 GHz.");
        Self::bad(ui, "Don't coil excess coax — the coil forms an inductor that resonates and creates noise. Cut it to length or lay it flat.");
    }

    fn section_sdr_panel(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "SDR Panel Controls (SDR tab)");

        ui.label("The SDR tab is your main control center for the radio hardware. Open it to configure the source, frequency, gain, and audio.");
        ui.add_space(8.0);

        Self::h2(ui, "Source selection");
        egui::Grid::new("sdr_source").num_columns(2).striped(true).show(ui, |ui| {
            for (src, desc) in &[
                ("RTL-SDR",   "Direct RTL2832U driver. Use this for the RTL-SDR Blog dongle."),
                ("SoapySDR",  "Generic SoapySDR driver — works with AirSpy, HackRF, SDRplay, LimeSDR, etc."),
                ("Demo",      "Simulated signals (FM stations, ADS-B, etc.) — no hardware required."),
            ] {
                ui.colored_label(egui::Color32::from_rgb(150, 200, 255), *src);
                ui.label(*desc);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Frequency");
        ui.label("Tune to any frequency within your hardware's range. You can:");
        ui.label("  •  Type a MHz value directly in the frequency box");
        ui.label("  •  Use the up/down spinner arrows");
        ui.label("  •  Press ↑/↓ for ±1 MHz, ←/→ for ±100 kHz (from anywhere in the app)");
        ui.label("  •  Click anywhere on the Spectrum display to instantly tune there");

        ui.add_space(10.0);
        Self::h2(ui, "Sample rate");
        ui.label("Sets the instantaneous bandwidth you can see in the spectrum. At 2.048 MHz, the spectrum display shows 2.048 MHz of spectrum simultaneously.");
        egui::Grid::new("sample_rates").num_columns(2).striped(true).show(ui, |ui| {
            for (r, n) in &[
                ("1.024 MHz", "Low CPU, 1 MHz view — good for slow machines"),
                ("1.536 MHz", "Balanced — good for most uses"),
                ("2.048 MHz", "Standard choice — 2 MHz view"),
                ("2.4 MHz",   "Widest stable view for RTL-SDR"),
            ] {
                ui.monospace(*r);
                ui.label(*n);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Gain");
        ui.label("RF gain in dB, 0 to ~49.6 dB for the RTL-SDR (discrete steps). Higher gain amplifies both signal and noise equally — there's an optimal point.");
        ui.label("How to find optimal gain:");
        ui.label("  1. Start at 30 dB");
        ui.label("  2. Look at the noise floor on the spectrum display");
        ui.label("  3. Increase gain gradually — the noise floor will slowly rise");
        ui.label("  4. Stop when the noise floor starts rising as fast as signal strength");
        Self::bad(ui, "Don't max out gain. Overloaded receivers show smeared spectrum and phantom 'ghost' signals at wrong frequencies.");
        Self::bad(ui, "Don't use AGC (Auto Gain Control). The RTL2832U's AGC is designed for wideband DVB-T, not narrowband SDR use. It wrecks weak signals.");

        ui.add_space(10.0);
        Self::h2(ui, "Squelch");
        ui.label("Cuts audio output when signal level is below the threshold — silences static between transmissions. Adjust the slider until static stops but voice/signal still comes through.");
        Self::tip(ui, "Squelch threshold varies by band. Re-adjust whenever you change frequency. A good starting point: set squelch ~5 dB above the noise floor.");

        ui.add_space(10.0);
        Self::h2(ui, "Status bar");
        ui.label("The status bar at the bottom of the window shows: running/stopped indicator, current frequency, demod mode, sample rate, gain, recording indicator, audio indicator, peak dB, and noise floor. These update in real time.");
    }

    fn section_spectrum(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Spectrum Analyzer & Waterfall");

        ui.label("The Spectrum tab gives you two views of the RF environment around your tuned frequency:");
        ui.label("  •  Top half:    Spectrum — signal power (dB) vs frequency right now");
        ui.label("  •  Bottom half: Waterfall — frequency vs time, color = signal strength");
        ui.add_space(8.0);

        Self::h2(ui, "Reading the spectrum display");
        egui::Grid::new("spectrum_reading").num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in &[
                ("X-axis",        "Frequency in MHz"),
                ("Y-axis",        "Signal power in dB (relative)"),
                ("Noise floor",   "The flat baseline (typically −80 to −100 dBFS). Lower = less noise = better."),
                ("Signal peak",   "A bump above the noise floor. Height above floor = SNR."),
                ("SNR",           "Signal-to-Noise Ratio. 10 dB minimum for audio; 20+ dB for comfortable listening."),
                ("Spike at center","A DC spike at 0 Hz offset is normal for RTL-SDR. Avoid tuning signals exactly to center."),
            ] {
                ui.label(egui::RichText::new(*k).strong());
                ui.label(*v);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Reading the waterfall");
        ui.label("Time flows downward — each horizontal line is one frame. Signal strength is encoded in color:");
        egui::Grid::new("wfall_reading").num_columns(2).striped(true).show(ui, |ui| {
            for (pattern, meaning) in &[
                ("Dark/black",          "No signal — just noise"),
                ("Bright color",        "Strong signal"),
                ("Vertical stripe",     "Continuous carrier (FM broadcast, beacon, CW key-down)"),
                ("Short vertical burst","Push-to-talk voice or data packet"),
                ("Wide stripe",         "Wideband signal (FM broadcast ~200 kHz wide)"),
                ("Slanted stripe",      "Doppler shift — moving transmitter (aircraft, satellite)"),
                ("Regular pattern",     "Digital modulation — PSK, FSK, etc."),
                ("Wide hump at ~400MHz","USB 3.0 interference from your computer"),
            ] {
                ui.colored_label(egui::Color32::from_rgb(180, 220, 255), *pattern);
                ui.label(*meaning);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Controls");
        egui::Grid::new("spectrum_controls").num_columns(2).striped(true).show(ui, |ui| {
            for (ctrl, action) in &[
                ("Left-click",       "Tune to that frequency instantly"),
                ("Right-click",      "Reset zoom to full bandwidth"),
                ("Scroll wheel",     "Zoom in/out on the spectrum"),
                ("Shift + Scroll",   "Pan left/right"),
                ("Middle-click",     "Add a frequency marker"),
                ("Middle-drag",      "Pan the spectrum view"),
                ("Space (anywhere)", "Start / stop the SDR source"),
                ("? key",            "Toggle keyboard shortcut reference overlay"),
            ] {
                ui.monospace(*ctrl);
                ui.label(*action);
                ui.end_row();
            }
        });

        ui.add_space(8.0);
        Self::tip(ui, "Band plan overlays show you what each chunk of spectrum is allocated for — labels appear on the spectrum automatically.");
        Self::tip(ui, "A 'DC spike' (bright line at dead center) is normal for direct-conversion receivers like the RTL-SDR. Tune 100–200 kHz away from your target so it doesn't appear at center.");
    }

    fn section_demod_modes(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Demodulation Modes");

        ui.label("Demodulation converts the raw complex radio signal into audible audio (or data). Choosing the wrong mode for a signal produces silence or garbage.");
        ui.add_space(8.0);

        egui::Grid::new("demod_table").num_columns(4).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Mode").strong());
            ui.label(egui::RichText::new("Full Name").strong());
            ui.label(egui::RichText::new("Where to use it").strong());
            ui.label(egui::RichText::new("Bandwidth").strong());
            ui.end_row();

            let modes: &[(&str, &str, &str, &str)] = &[
                ("WFM", "Wideband FM",      "Commercial FM broadcast (88–108 MHz). Supports stereo & RDS decoding.", "~200 kHz"),
                ("NFM", "Narrowband FM",    "Land mobile radio: police, fire, EMS, weather radio, marine VHF, amateur FM repeaters, NOAA APT.", "8–16 kHz"),
                ("AM",  "Amplitude Mod.",   "Aviation ATC voice (118–137 MHz), AM broadcast (530–1700 kHz), shortwave, NDBs. AIRBAND IS ALWAYS AM.", "6–10 kHz"),
                ("USB", "Upper Sideband",   "Amateur HF voice above 10 MHz. Standard above 10 MHz by convention.", "2.4–3 kHz"),
                ("LSB", "Lower Sideband",   "Amateur HF voice below 10 MHz. Some HF utility and military traffic.", "2.4–3 kHz"),
                ("DSB", "Double Sideband",  "Non-directional beacons (NDB), some experimental transmissions.", "~6 kHz"),
                ("CW",  "Morse Code",       "Amateur and utility Morse. Use a very narrow filter (~500 Hz).", "~500 Hz"),
                ("RAW", "Raw I/Q",          "Records or passes through raw baseband. Feed into external decoders.", "variable"),
            ];

            for (mode, name, use_case, bw) in modes {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 255), *mode);
                ui.label(*name);
                ui.label(*use_case);
                ui.colored_label(egui::Color32::from_rgb(200, 200, 100), *bw);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        ui.label(egui::RichText::new("⚠  Most common beginner mistake").size(13.0).strong()
            .color(egui::Color32::from_rgb(255, 200, 50)));
        ui.label("All aviation voice on 118–137 MHz is AM, not FM. If you tune to an aircraft frequency and select NFM, you'll hear nothing. Always select AM for airband.");

        ui.add_space(10.0);
        Self::h2(ui, "Filter bandwidth guide");
        ui.label("Set filter bandwidth just slightly wider than the signal. Too wide = more noise and adjacent interference. Too narrow = distorted, muffled audio.");
        egui::Grid::new("bw_guide").num_columns(2).striped(true).show(ui, |ui| {
            for (sig, bw) in &[
                ("FM broadcast (WFM)",    "180–200 kHz"),
                ("NFM voice (land mobile)","12–16 kHz"),
                ("NOAA APT weather sat",  "34–40 kHz"),
                ("AM voice (airband)",    "6–10 kHz"),
                ("AM broadcast",          "8–10 kHz"),
                ("SSB voice (USB/LSB)",   "2.4–3 kHz"),
                ("CW Morse code",         "400–600 Hz"),
            ] {
                ui.label(*sig);
                ui.colored_label(egui::Color32::from_rgb(200, 200, 100), *bw);
                ui.end_row();
            }
        });
    }

    fn section_adsb(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "ADS-B Aircraft Tracking (1090 MHz)");

        ui.label("ADS-B (Automatic Dependent Surveillance – Broadcast) is a system where aircraft continuously broadcast their GPS position, altitude, speed, heading, and callsign every ~0.5 seconds.");
        ui.add_space(8.0);

        Self::h2(ui, "Technical details");
        egui::Grid::new("adsb_specs").num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in &[
                ("Frequency",              "1090.000 MHz  (Mode S squitter — exact)"),
                ("Modulation",             "PPM (Pulse Position Modulation) — 1 Mbps"),
                ("Required sample rate",   "2.048 MHz minimum"),
                ("Antenna polarization",   "Vertical"),
                ("Typical outdoor range",  "100–250 km with a simple quarter-wave antenna"),
                ("With filtered LNA",      "Up to 400+ km in flat terrain"),
            ] {
                ui.label(egui::RichText::new(*k).strong());
                ui.label(*v);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Getting started in ez-sdr");
        ui.label("1.  Open the ADS-B tab");
        ui.label("2.  Click Start ADS-B — ez-sdr auto-tunes to 1090 MHz, sets 2.048 MHz sample rate");
        ui.label("3.  Aircraft appear on the map within seconds of receiving their first position message");
        ui.label("4.  Click any aircraft dot on the map to look up its model, operator, and registration via Planespotters API");
        ui.label("5.  The table below the map shows ICAO hex, callsign, altitude (ft), speed (kts), heading, lat/lon, and message age");

        ui.add_space(10.0);
        Self::h2(ui, "Antenna for ADS-B");
        ui.label("  •  Quarter-wave monopole: 6.9 cm element on a metal ground plane — simplest option");
        ui.label("  •  Coaxial collinear (co-co): DIY from coax, gives 3–6 dB gain over a simple monopole");
        ui.label("  •  SAWbird+ ADS-B LNA: filtered LNA with <1 dB noise figure centered on 1090 MHz — single biggest upgrade");
        ui.label("  •  Mount outdoors, high as possible, with clear sky view above 0°");

        ui.add_space(10.0);
        Self::h2(ui, "Reading the map");
        ui.label("Green dots = aircraft. Cyan dot = selected aircraft. Callsign labeled next to each dot.");
        ui.label("Click a dot to select it and trigger a Planespotters API lookup for the aircraft model.");

        Self::tip(ui, "Aircraft typically stop transmitting once on the ground (stationary < 50 ft). Range drops quickly if trees or buildings are in the line of sight.");
        Self::warn(ui, "Not all aircraft have ADS-B transponders. Older light aircraft and some military may only have Mode A/C (altitude only, no position) or no transponder at all.");
    }

    fn section_satellite(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Satellite Tracking & NOAA Weather Images");

        ui.label("The Satellite tab lets you track Low Earth Orbit (LEO) satellites using TLE (Two-Line Element set) orbital data. ez-sdr predicts upcoming passes and can auto-tune and auto-record them.");
        ui.add_space(8.0);

        Self::h2(ui, "Active NOAA APT weather satellites (as of 2025)");
        egui::Grid::new("noaa_sats").num_columns(3).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Satellite").strong());
            ui.label(egui::RichText::new("Frequency").strong());
            ui.label(egui::RichText::new("Notes").strong());
            ui.end_row();
            for (sat, freq, note) in &[
                ("NOAA 15", "137.620 MHz",   "Active but aging — audio can be noisy"),
                ("NOAA 18", "137.9125 MHz",  "Most reliable as of 2024–2025"),
                ("NOAA 19", "137.100 MHz",   "Operational, good signal quality"),
                ("Meteor M2-3","137.900 MHz","Russian satellite; digital LRPT mode (not APT)"),
            ] {
                ui.label(*sat);
                ui.colored_label(egui::Color32::from_rgb(140, 220, 140), *freq);
                ui.label(*note);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Demodulation settings for NOAA APT");
        egui::Grid::new("apt_set").num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in &[
                ("Mode",        "WFM (Wideband FM)"),
                ("Bandwidth",   "34–40 kHz"),
                ("Sample rate", "Any rate ≥ 1 MHz — 2.048 MHz recommended"),
            ] {
                ui.label(egui::RichText::new(*k).strong());
                ui.label(*v);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Antenna for NOAA APT — V-dipole");
        Self::draw_vdipole(ui);
        ui.add_space(4.0);
        ui.label("  •  Each arm: 54.7 cm for 137 MHz");
        ui.label("  •  Opening angle: ~120° between the two arms");
        ui.label("  •  Mount horizontally (arms pointing East–West for a North–South satellite track)");
        ui.label("  •  Needs a completely clear horizon in all directions — no buildings, no trees");

        ui.add_space(10.0);
        Self::h2(ui, "Tracking a pass in ez-sdr");
        ui.label("1.  Settings tab: enter your observer lat/lon");
        ui.label("2.  Satellite tab → Download TLE to fetch the latest Two-Line Elements from Celestrak");
        ui.label("3.  Upcoming passes appear in the Scheduler tab with start time and max elevation angle");
        ui.label("4.  At pass start, ez-sdr auto-tunes to the correct frequency");
        ui.label("5.  Real-time Doppler correction is applied throughout the pass (~±3 kHz at 137 MHz)");
        ui.label("6.  Record the audio (Recorder tab) → decode offline with SatDump or WXtoImg");

        ui.add_space(8.0);
        Self::tip(ui, "Passes above 30° max elevation give the best images. A 90° (directly overhead) pass yields ~12 minutes of signal and a full-width image.");
        Self::tip(ui, "NOAA APT transmits at only 4 W. A clean antenna placement is critical — the satellite is 800+ km away.");
        Self::warn(ui, "NOAA 15/18/19 are aging spacecraft. NOAA 15 in particular has had anomalies. If you receive garbage from one satellite, try the others.");
    }

    fn section_scanner(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Frequency Scanner");

        ui.label("The Scanner tab automatically sweeps a frequency range, pausing at each step to measure signal strength. Signals above your threshold are logged as hits.");
        ui.add_space(8.0);

        Self::h2(ui, "Controls");
        egui::Grid::new("scan_ctrl").num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in &[
                ("Start / Stop (MHz)",  "The frequency range to sweep. Example: 88–108 MHz for the FM broadcast band."),
                ("Step",                "How far to advance per dwell. Match to signal bandwidth: 100 kHz for WFM, 12.5 kHz for NFM voice."),
                ("Dwell (ms)",          "How long to listen at each step before moving. 200–500 ms typical."),
                ("Threshold (dB)",      "Minimum signal level to log. Start around −60 dB and adjust based on local noise floor."),
                ("Reset on Start",      "Clear previous hits when starting a new scan sweep."),
                ("Sort by strength",    "Reorders the hit list by signal level, strongest first."),
            ] {
                ui.label(egui::RichText::new(*k).strong());
                ui.label(*v);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Hits table");
        ui.label("When a signal exceeds the threshold, it's logged with frequency, strength (dB), and time since last seen.");
        ui.label("  •  Click 📡 to instantly tune the SDR to that frequency");
        ui.label("  •  Click ✕ to remove a hit from the list");

        ui.add_space(10.0);
        Self::h2(ui, "Scan presets");
        egui::Grid::new("scan_presets").num_columns(3).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Use case").strong());
            ui.label(egui::RichText::new("Range").strong());
            ui.label(egui::RichText::new("Step / Dwell").strong());
            ui.end_row();
            for (use_case, range, step) in &[
                ("FM broadcast",          "88–108 MHz",    "100 kHz / 300 ms"),
                ("Land mobile (VHF)",     "150–174 MHz",   "12.5 kHz / 250 ms"),
                ("Land mobile (UHF)",     "450–512 MHz",   "12.5 kHz / 250 ms"),
                ("Aviation airband",      "118–137 MHz",   "25 kHz / 500 ms"),
                ("ISM device remotes",    "433–435 MHz",   "100 kHz / 200 ms"),
                ("Amateur 2m repeaters",  "144–148 MHz",   "20 kHz / 300 ms"),
            ] {
                ui.label(*use_case);
                ui.colored_label(egui::Color32::from_rgb(140, 220, 140), *range);
                ui.monospace(*step);
                ui.end_row();
            }
        });

        Self::tip(ui, "After scanning, click Sort by Strength to bubble the strongest signals to the top, then investigate each one in turn.");
        Self::warn(ui, "Very short dwell times (<100 ms) will miss bursty signals like digital voice, APRS packets, or FSK data bursts.");
    }

    fn section_recorder(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Recorder");

        ui.label("The Recorder tab lets you save received signal data to disk for offline processing, sharing, or replay.");
        ui.add_space(8.0);

        Self::h2(ui, "Recording modes");
        egui::Grid::new("rec_modes").num_columns(2).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("I/Q Recording").strong());
            ui.label("Records the raw complex baseband data as-is. Preserves the entire received spectrum — you can re-demodulate later at any mode, bandwidth, or center frequency within the recorded band. Larger file size.");
            ui.end_row();
            ui.label(egui::RichText::new("Audio Recording").strong());
            ui.label("Records demodulated audio as a WAV file. Much smaller. Cannot re-process the RF after the fact.");
            ui.end_row();
        });

        ui.add_space(10.0);
        Self::h2(ui, "File size reference");
        egui::Grid::new("rec_sizes").num_columns(2).striped(true).show(ui, |ui| {
            for (desc, size) in &[
                ("I/Q at 2.048 MHz (32-bit float complex)", "~15 MB / min"),
                ("I/Q at 2.4 MHz",                          "~18 MB / min"),
                ("Audio WAV 48 kHz mono 16-bit",            "~5.5 MB / min"),
            ] {
                ui.label(*desc);
                ui.colored_label(egui::Color32::from_rgb(200, 200, 100), *size);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Working with external tools");
        ui.label("I/Q recordings from ez-sdr can be opened in:");
        ui.label("  •  SDR++ — full I/Q playback and re-demodulation");
        ui.label("  •  SatDump — NOAA APT / Meteor M2 image decode from recorded audio");
        ui.label("  •  WXtoImg — NOAA APT image decode from recorded audio WAV");
        ui.label("  •  GNU Radio — any custom processing pipeline");
        ui.label("  •  Audacity — audio analysis, spectrogram of recorded audio");

        Self::tip(ui, "For NOAA APT satellite passes: record I/Q during the pass, then replay and decode offline. No real-time pressure — you can re-decode many times with different settings.");
        Self::tip(ui, "The Scheduler tab can auto-start and auto-stop recording when a scheduled satellite pass begins and ends.");
    }

    fn section_bookmarks(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Bookmarks & Scheduler");

        Self::h2(ui, "Bookmarks");
        ui.label("Save frequencies you use regularly for one-click tuning. Shown in the left sidebar.");
        ui.label("  •  Add: type a name and frequency, press Add");
        ui.label("  •  Tune: click any bookmark entry to immediately tune the SDR");
        ui.label("  •  Filter: use the search box at the top of the Bookmarks panel to find bookmarks by name");

        ui.add_space(6.0);
        Self::h2(ui, "Suggested bookmarks to start with");
        egui::Grid::new("bm_suggestions").num_columns(2).striped(true).show(ui, |ui| {
            for (name, freq) in &[
                ("NOAA 15",                    "137.620 MHz"),
                ("NOAA 18",                    "137.9125 MHz"),
                ("NOAA 19",                    "137.100 MHz"),
                ("FM band start",              "88.0 MHz"),
                ("Aviation (common ATIS)",     "126.400 MHz"),
                ("Marine Ch 16 (distress)",    "156.800 MHz"),
                ("NOAA Weather Radio (US ch1)","162.400 MHz"),
                ("ADS-B",                      "1090.000 MHz"),
                ("APRS (North America)",       "144.390 MHz"),
            ] {
                ui.label(*name);
                ui.colored_label(egui::Color32::from_rgb(140, 220, 140), *freq);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Scheduler");
        ui.label("The Scheduler automatically tunes to and optionally records satellite passes based on TLE predictions:");
        ui.label("  1.  Load TLE data in the Satellite tab (Download button → pulls from Celestrak)");
        ui.label("  2.  Set your location (lat/lon) in Settings");
        ui.label("  3.  Upcoming passes populate the Scheduler automatically with start time and max elevation");
        ui.label("  4.  At pass start, the SDR auto-tunes and applies real-time Doppler correction");
        Self::tip(ui, "You can leave ez-sdr running overnight. The Scheduler will auto-tune to every NOAA pass that crosses your horizon.");
    }

    fn section_noise(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Reducing Noise & Interference");

        ui.label("A flat, low noise floor is the foundation of good SDR performance. Here's how to improve yours, starting with the highest-impact changes.");
        ui.add_space(8.0);

        Self::h2(ui, "1. LNA placement — most impactful");
        ui.label("If you use an LNA, where you mount it is everything:");
        Self::draw_lna_chain(ui);
        ui.add_space(4.0);
        ui.label("Cable loss BEFORE the LNA adds directly to your system noise figure (Friis's formula). A 3 dB cable loss before the LNA raises your effective noise figure by 3 dB — permanently degrading all weak-signal reception.");
        ui.label("  •  Mount LNA at the antenna feedpoint, not at the dongle");
        ui.label("  •  RTL-SDR V3 bias tee powers LNAs over the coax — no extra cable needed");
        Self::bad(ui, "Don't use a wideband (unfiltered) LNA. It amplifies strong FM broadcast and cellular signals until the RTL-SDR overloads, creating phantom signals everywhere.");

        ui.add_space(10.0);
        Self::h2(ui, "2. USB cable ferrite chokes");
        ui.label("Your computer's USB bus radiates broadband noise. Ferrite cores on the USB cable are cheap and effective:");
        ui.label("  •  Wrap cable 2–3 times through a ferrite core near the computer AND near the dongle");
        ui.label("  •  Use #43 mix ferrite for VHF/UHF noise (30–300 MHz)");
        ui.label("  •  Use #31 mix ferrite for HF noise (1–30 MHz)");
        ui.label("  •  Effect: can reduce noise floor 10–15 dB");
        Self::tip(ui, "More turns = more impedance (scales as turns²). 3 turns through a single core is much better than 1 turn each through 3 cores.");

        ui.add_space(10.0);
        Self::h2(ui, "3. USB 3.0 interference (~400 MHz hump)");
        ui.label("USB 3.0 ports and hubs generate broadband switching noise centered around 400 MHz — visible as a wide hump in the waterfall.");
        ui.label("  •  Plug the SDR into a USB 2.0 port if you have one");
        ui.label("  •  Or use a quality USB 2.0 hub between the SDR and a USB 3.0 port");
        ui.label("  •  Or use a long USB cable + ferrites to distance the dongle from the machine");
        Self::bad(ui, "Generic cheap USB 3.0 hubs are the worst offenders. Name-brand hubs (Anker, Belkin, StarTech) have better filtering.");

        ui.add_space(10.0);
        Self::h2(ui, "4. Antenna placement");
        ui.label("  •  Outdoors beats indoors by 10–30 dB — walls absorb VHF/UHF badly");
        ui.label("  •  Mount as high as possible — every meter clears more horizon obstacles");
        ui.label("  •  Keep coax short; use quality cable (LMR-400) for long runs");
        ui.label("  •  Distance from power lines, routers, monitors, and computers");

        ui.add_space(10.0);
        Self::h2(ui, "5. Band-specific filters");
        egui::Grid::new("filters_table").num_columns(2).striped(true).show(ui, |ui| {
            for (filter, desc) in &[
                ("FM Broadcast trap (~$10–15)",  "High-pass filter blocks 88–108 MHz overload. Essential if you live near a strong FM transmitter."),
                ("SAWbird+ ADS-B",               "Filtered LNA: ~20 dB gain + 1090 MHz bandpass. Best upgrade for ADS-B range."),
                ("SAWbird+ NOAA",                "Filtered LNA: ~20 dB gain + 137 MHz bandpass. Dramatically improves satellite signal."),
                ("RTL-SDR Blog LNA (generic)",   "~20 dB gain, 0.1–2 GHz wideband. Only use if no strong interferers nearby."),
            ] {
                ui.label(egui::RichText::new(*filter).strong());
                ui.label(*desc);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "6. Gain optimization");
        ui.label("The right gain level reduces noise just as much as hardware changes:");
        ui.label("  1.  Open Spectrum tab and zoom into a frequency with signals of interest");
        ui.label("  2.  Increase gain slowly from ~20 dB upward");
        ui.label("  3.  The noise floor will rise gradually — that's okay");
        ui.label("  4.  Stop where signals are clearest. If noise rises as fast as signal, you've gone too far.");
        Self::bad(ui, "Never set gain to maximum. Strong signals (FM towers, cell towers) will produce intermodulation — you'll see ghost signals at wrong frequencies across the whole spectrum.");
    }

    fn section_freq_reference(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "Frequency Reference Chart");
        ui.label("What's actually out there in the RF spectrum, and what to use to receive it.");
        ui.add_space(8.0);

        Self::draw_freq_chart(ui);
        ui.label(egui::RichText::new("Colored bands on the chart above (80 MHz – 1800 MHz, RTL-SDR coverage shown)")
            .italics().color(egui::Color32::GRAY).size(10.0));
        ui.add_space(10.0);

        egui::Grid::new("freq_ref_table").num_columns(3).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Band / Service").strong());
            ui.label(egui::RichText::new("Frequency").strong());
            ui.label(egui::RichText::new("Mode / Notes").strong());
            ui.end_row();

            let entries: &[(&str, &str, &str)] = &[
                ("AM Broadcast",            "530–1700 kHz",        "AM. Below RTL-SDR range; need V3 direct sampling."),
                ("Shortwave Broadcast",     "3–30 MHz",            "AM/USB. International broadcasts. V3 direct sampling only."),
                ("CB Radio",                "26.965–27.405 MHz",   "AM or USB. 40 channels. Near V3 direct-sampling limit."),
                ("FM Broadcast",            "88–108 MHz",          "WFM. Stereo + RDS. Best first thing to receive."),
                ("Aviation Navigation",     "108–118 MHz",         "AM. VOR and ILS navigation beacons."),
                ("Aviation Voice (ATC)",    "118–137 MHz",         "AM — always AM! ATC, ATIS, ground, tower, approach."),
                ("NOAA APT Satellites",     "137.1 / 137.62 / 137.9125 MHz", "WFM 34 kHz. Weather images from LEO satellites."),
                ("NOAA Weather Radio (US)", "162.400–162.550 MHz", "NFM. 7 channels. 24/7 automated weather broadcasts."),
                ("APRS (North America)",    "144.390 MHz",         "NFM / AX.25 packet. Amateur radio position reports."),
                ("Amateur 2m",              "144–148 MHz",         "NFM voice (repeaters), USB (SSB), digital, APRS."),
                ("ACARS (aircraft data)",   "129.125 / 130.025 / 130.450 / 131.525 MHz", "AM. Airline text datalink messages."),
                ("Marine VHF",              "156–174 MHz",         "NFM. Ch 16 = 156.800 MHz (distress / calling)."),
                ("Amateur 70cm",            "420–450 MHz",         "NFM voice, USB, ATV, digital."),
                ("ISM (EU device remotes)", "433.92 MHz",          "OOK/FSK. Garage openers, weather stations, key fobs."),
                ("ISM (US devices)",        "902–928 MHz",         "LoRa, Zigbee, wireless utility meters."),
                ("ADS-B Aircraft",          "1090.000 MHz",        "Mode S squitter. Aircraft GPS position every 0.5 s."),
                ("GPS L1",                  "1575.420 MHz",        "BPSK. Navigation signal — receive only, no decode with RTL."),
                ("Iridium Satellite",       "1616–1626 MHz",       "Near top of RTL-SDR range. Paging + voice bursts."),
                ("AIS Maritime (ship GPS)", "161.975 / 162.025 MHz","NFM / GMSK. Ship position reports like ADS-B for boats."),
                ("POCSAG Pagers",           "152–158 MHz (varies)","NFM. Digital pager messages — hospital, fire dispatch."),
                ("ISS Amateur Radio",       "145.800 MHz",         "NFM. International Space Station voice & APRS."),
                ("Weather Balloons (sonde)","400–406 MHz",         "FSK. Position + atmospheric sensor telemetry."),
            ];

            for (band, freq, notes) in entries {
                ui.label(*band);
                ui.colored_label(egui::Color32::from_rgb(140, 220, 140), *freq);
                ui.label(*notes);
                ui.end_row();
            }
        });
    }

    fn section_soapy(&mut self, ui: &mut egui::Ui) {
        Self::h1(ui, "SoapySDR & Other Hardware");

        ui.label("SoapySDR is an open-source hardware abstraction layer. It provides a single API that works with many different SDR devices — install one plugin per device, and any SoapySDR-aware application (including ez-sdr) can use it.");
        ui.add_space(8.0);

        Self::h2(ui, "How it works");
        ui.label("Applications call the SoapySDR API. SoapySDR loads the right driver plugin at runtime. You install the framework once, then install per-device plugins:");
        egui::Grid::new("soapy_plugins").num_columns(2).striped(true).show(ui, |ui| {
            for (device, plugin) in &[
                ("RTL-SDR",       "SoapyRTLSDR"),
                ("HackRF One",    "SoapyHackRF"),
                ("AirSpy",        "SoapyAirspy"),
                ("LimeSDR",       "LimeSuite"),
                ("Ettus USRP",    "SoapyUHD"),
                ("SDRplay RSP",   "SoapySDRPlay"),
                ("BladeRF",       "SoapyBladeRF"),
                ("PlutoSDR",      "SoapyPlutoSDR"),
            ] {
                ui.label(*device);
                ui.colored_label(egui::Color32::from_rgb(150, 180, 255), *plugin);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "SDR device comparison");
        egui::Grid::new("sdr_compare").num_columns(4).striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Device").strong());
            ui.label(egui::RichText::new("Price").strong());
            ui.label(egui::RichText::new("ADC bits / BW").strong());
            ui.label(egui::RichText::new("Highlights").strong());
            ui.end_row();

            let devices: &[(&str, &str, &str, &str)] = &[
                ("RTL-SDR V3",       "~$30",   "8-bit / 2.4 MHz",  "Receive only. 24 MHz–1.766 GHz. Best entry-level choice."),
                ("AirSpy R2",        "~$170",  "12-bit / 10 MHz",  "Much better dynamic range. 24–1800 MHz."),
                ("AirSpy HF+",       "~$200",  "18-bit / 10 MHz",  "Exceptional HF/VHF performance. Very low noise figure."),
                ("HackRF One",       "~$350",  "8-bit / 20 MHz",   "TX + RX. 1 MHz–6 GHz. 8-bit ADC same as RTL."),
                ("SDRplay RSP1A",    "~$110",  "14-bit / 10 MHz",  "1 kHz–2 GHz. Built-in filters. Great HF."),
                ("LimeSDR Mini",     "~$160",  "12-bit / 30.72 MHz","TX + RX. 10 MHz–3.5 GHz. FPGA onboard."),
                ("PlutoSDR",         "~$200",  "12-bit / 20 MHz",  "TX + RX. 325 MHz–3.8 GHz. Easy to use."),
                ("Ettus USRP B210",  "~$1400", "12-bit / 56 MHz",  "Research-grade. Full-duplex. 70 MHz–6 GHz."),
            ];

            for (dev, price, adc, notes) in devices {
                ui.label(*dev);
                ui.colored_label(egui::Color32::from_rgb(220, 190, 100), *price);
                ui.monospace(*adc);
                ui.label(*notes);
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        Self::h2(ui, "Which SDR should I get?");
        egui::Grid::new("sdr_pick").num_columns(2).striped(true).show(ui, |ui| {
            for (goal, rec) in &[
                ("Just getting started",             "RTL-SDR V3 Blog — $30, huge community, thousands of tutorials"),
                ("Better HF / shortwave reception",  "AirSpy HF+ or SDRplay RSP1A — far better dynamic range on HF"),
                ("Need to transmit",                 "HackRF One (wide range) or PlutoSDR (simpler, 325 MHz–3.8 GHz)"),
                ("Wideband simultaneous capture",    "AirSpy R2 (10 MHz) or SDRplay RSP2 (10 MHz) — 12/14-bit ADC"),
                ("Serious research / FPGA",          "Ettus USRP or LimeSDR — much more expensive but research-grade"),
            ] {
                ui.label(egui::RichText::new(*goal).strong());
                ui.label(*rec);
                ui.end_row();
            }
        });

        ui.add_space(8.0);
        Self::tip(ui, "SoapyRemote lets you stream from an SDR attached to a Raspberry Pi on the roof over the network. The Pi mounts at the antenna; your laptop runs the software inside. Minimal cable loss.");
        Self::warn(ui, "The RTL-SDR's 8-bit ADC limits dynamic range to ~48 dB. If strong signals are present near weak targets (e.g., FM broadcast near airband), upgrading to a 12/14-bit SDR makes a dramatic difference.");
    }
}
