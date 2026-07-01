use std::sync::{Arc, Mutex};

use crate::app::SharedState;

/// One verifiable step in an antenna setup checklist.
pub struct ChecklistItem {
    pub label: &'static str,
    pub detail: &'static str,
    pub critical: bool,
    pub checked: bool,
}

/// A reusable gate component shown before a panel's real content.
///
/// Behaviour: re-shown (unchecked) on every app launch; a hard gate blocks
/// `Continue` until all critical items are checked; a `Skip for now` escape
/// is always available; a persisted `Don't show again` checkbox writes
/// `skip_antenna_checklists` to the config and auto-passes on future launches.
pub struct AntennaChecklist {
    shared: Arc<Mutex<SharedState>>,
    items: Vec<ChecklistItem>,
    passed: bool,
    title: &'static str,
    intro: &'static str,
    /// Filled when the user clicks "Skip for now" — drained by the host panel.
    pub pending_status: Option<String>,
}

impl AntennaChecklist {
    pub fn new(shared: Arc<Mutex<SharedState>>, title: &'static str, intro: &'static str) -> Self {
        Self { shared, items: vec![], passed: false, title, intro, pending_status: None }
    }

    pub fn for_sdr(shared: Arc<Mutex<SharedState>>) -> Self {
        let mut c = Self::new(
            shared,
            "📡 Antenna Setup Checklist — SDR Receiver",
            "Verify your antenna before receiving. Critical items must be checked to continue.",
        );
        c.items = vec![
            crit("Antenna connected", "MCX/SMA adapter seated firmly; no loose connector. A disconnected antenna gives only noise."),
            crit("Element length tuned for band", "Quarter-wave = 7500 / freq(MHz) cm. FM ~75 cm, airband ~60 cm, ADS-B ~7 cm. Don't leave the dipole collapsed for VHF."),
            crit("Correct polarization", "Vertical: FM/ADS-B/airband/marine/land-mobile. Horizontal: NOAA 137 MHz satellites. Mismatch costs ~20 dB."),
            item("Outdoors or near window", "Walls cost 10-30 dB. Outdoor at roofline is +10-20 dB. A window works for strong local signals only."),
            item("Away from RFI sources", "Keep clear of USB 3.0 ports, Wi-Fi routers, fluorescent ballasts, switching power supplies, and power lines."),
            item("Away from large metal objects", "Metal within ~1 wavelength reflects/distorts the pattern. Give the antenna clear surroundings."),
            item("Coax short & low-loss", "Avoid RG-58 at UHF (loses ~9 dB/10 m at 1 GHz). Use LMR-240/400 for runs over 5 m. Never coil excess coax."),
            item("PPM calibrated", "Tune a known reference (FM station, ADS-B at 1090.0) and adjust PPM in Settings until it lands exactly. RTL-SDR drifts ±20-50 ppm."),
            item("Gain ~40 dB (avoid overload)", "Start at 40 dB. If the spectrum 'flattens' or phantom signals appear, reduce gain. Overload looks like strong signals everywhere."),
            item("Sample rate <= 2.4 MSps", "RTL-SDR drops samples above 2.4 MSps on USB 2.0. Stay at 2.048 for ADS-B, 2.4 for general use."),
            item("Bias-tee only if LNA needs power", "Enable bias-tee in Source controls ONLY when powering a mast-mounted LNA. Off for passive antennas."),
        ];
        c
    }

    pub fn for_satellite(shared: Arc<Mutex<SharedState>>) -> Self {
        let mut c = Self::new(
            shared,
            "📡 Antenna Setup Checklist — Satellite Reception",
            "LEO weather sats transmit only ~4 W from 800+ km. A tuned antenna with a clear horizon is essential.",
        );
        c.items = vec![
            crit("Antenna type chosen for target", "V-dipole (53.4 cm arms @ 120 deg) or QFH/turnstile for 137 MHz NOAA/Meteor. Dish/helical for 1694 MHz GOES. Stock whip is too short."),
            crit("V-dipole mounted horizontal, arms N-S", "The 9A4QV V-dipole sits FLAT with arms pointing North-South. Its horizontal polarization rejects terrestrial FM by ~20 dB."),
            crit("Outside, clear horizon-to-horizon sky", "Satellites rise to the horizon. Trees/buildings in any direction kill low-elevation passes. Attic loses 3-6 dB; roofline is +10-20 dB."),
            item("Elevated >= 1-2 m above roof", "Get the antenna above nearby metal roofing and gutters to avoid multipath nulls."),
            item("Away from FM broadcast towers", "FM stations can desense the SDR. Use an FM notch filter if a 88-108 MHz tower is within 2 km."),
            item("Coax < 10 m + LNA at feedpoint", "Put a SAWbird+ NOAA (137) or GOES (1694) LNA AT the antenna, not the dongle. Coax loss before the LNA is unrecoverable."),
            item("Observer lat/lon set in Settings", "Pass prediction needs your position. Set it in Settings -> Satellite Observer Location, or passes/times will be wrong."),
            item("WFM, 34-40 kHz bandwidth", "NOAA APT is wide-FM. Set WFM mode and LPF cutoff 34-40 kHz. NFM is too narrow and clips the image data."),
            item("Auto-tune + Doppler on", "Enable both in the Satellite panel. A 137 MHz pass drifts up to ±3 kHz; 1694 MHz GOES is geostationary so Doppler is negligible."),
            item("GOES variant: RHCP + precisely pointed", "If receiving GOES at 1694 MHz: use a RHCP helical/dish, aim with a pointing calculator, fine-tune by peak signal. Budget ~$80-150."),
        ];
        c
    }

    pub fn for_adsb(shared: Arc<Mutex<SharedState>>) -> Self {
        let mut c = Self::new(
            shared,
            "📡 Antenna Setup Checklist — ADS-B (1090 MHz)",
            "1090 MHz is coax-loss-heavy and RFI-prone. A tuned outdoor antenna is the single biggest range upgrade.",
        );
        c.items = vec![
            crit("Quarter-wave ground-plane or collinear", "68.8 mm vertical element + 4 radials at 45 deg. Or a coaxial collinear (3-6 dB gain over monopole). Stock whip is too short for 1090 MHz."),
            crit("Mounted VERTICAL", "ADS-B is vertically polarized. A tilted/indoor antenna loses 10-20 dB. The element must point straight up."),
            crit("Outside, as high as possible", "Range roughly doubles for every doubling of height. Roof/mast mount gives 100-250 km; indoor gives <30 km."),
            item("Clear 360 deg horizon", "Aircraft at distance appear low on the horizon. Buildings/trees in any sector create reception holes."),
            item("Away from metal", "Keep 1+ m from ducting, gutters, chimneys, air-con units. Metal detunes the pattern and creates nulls."),
            item("Coax < 5 m or low-loss LMR-240/400", "RG-58 loses ~9 dB/10 m at 1 GHz. Use LMR-240 (2.4 dB) or LMR-400 (1.4 dB). Keep the run as short as possible."),
            item("LNA + 1090 MHz SAW bandpass filter", "A SAWbird+ ADS-B LNA (<1 dB NF + bandpass) rejects GSM/LTE at 940-960 MHz. Single biggest upgrade after height."),
            item("Bias-tee on if LNA needs power", "Enable bias-tee in Source controls to feed 4.5V to a SAWbird/external LNA. Off if passive antenna."),
            item("Tuned to exactly 1090.000 MHz", "Use the ADS-B tab's Start button (auto-tunes). Mode-S squitters are exactly on 1090; offset loses frames."),
            item("PPM calibrated", "10 ppm at 1090 MHz = 10.9 kHz error. Frames survive but frequency readout is wrong. Calibrate against a known reference."),
            item(">= 2 MSps, gain ~40 dB / AGC", "2.048 MSps is standard. Too much gain overloads on nearby aircraft; let AGC or ~40 dB handle it."),
            item("Away from Wi-Fi / USB 3.0 RFI", "USB 3.0 radiates 1-3 GHz. Use a USB 2.0 extension cable to move the dongle 1-2 m from the PC/router."),
        ];
        c
    }

    /// Render the checklist. Returns `true` when the host panel may show its
    /// real content (either the user passed the gate, or skip is persisted).
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        // Persisted skip: auto-pass silently.
        let skip_persisted = self.shared.try_lock().map(|s| s.config.skip_antenna_checklists).unwrap_or(false);
        if skip_persisted || self.passed {
            return true;
        }

        let all_critical = self.items.iter().filter(|i| i.critical).all(|i| i.checked);
        let n_crit = self.items.iter().filter(|i| i.critical).count();
        let n_crit_on = self.items.iter().filter(|i| i.critical && i.checked).count();
        let n_total = self.items.len();

        egui::Frame::group(ui.style())
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 130, 180)))
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(self.title).size(15.0).strong());
                ui.add_space(2.0);
                ui.label(egui::RichText::new(self.intro).small().color(egui::Color32::GRAY));
                ui.add_space(6.0);

                egui::Grid::new(ui.id().with("_cl_grid")).num_columns(2).spacing([6.0, 3.0]).show(ui, |ui| {
                    for (idx, it) in self.items.iter_mut().enumerate() {
                        let mark = if it.critical { egui::Color32::from_rgb(220, 90, 70) } else { egui::Color32::from_rgb(120, 160, 120) };
                        ui.colored_label(mark, if it.critical { "●" } else { "○" });
                        ui.horizontal(|ui| {
                            let mut checked = it.checked;
                            let resp = ui.checkbox(&mut checked, egui::RichText::new(it.label).strong());
                            it.checked = checked;
                            resp.on_hover_text(it.detail);
                            if !it.checked {
                                ui.label(egui::RichText::new(it.detail).small().color(egui::Color32::from_gray(150)))
                                    .on_hover_text(it.detail);
                            }
                        });
                        ui.end_row();
                        let _ = idx;
                    }
                });

                ui.add_space(4.0);
                let frac = if n_crit > 0 { n_crit_on as f32 / n_crit as f32 } else { 1.0 };
                ui.add(egui::ProgressBar::new(frac).text(format!("{} / {} critical checks", n_crit_on, n_crit)));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(format!("{} of {} total items", self.items.iter().filter(|i| i.checked).count(), n_total)).small().color(egui::Color32::GRAY));

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let continue_btn = ui.add_enabled(all_critical,
                        egui::Button::new(egui::RichText::new("✓ Continue").color(egui::Color32::from_rgb(80, 220, 120)))
                            .min_size(egui::vec2(100.0, 22.0)));
                    if continue_btn.clicked() {
                        self.passed = true;
                    }
                    if ui.button("Skip for now").on_hover_text("Proceed without completing the checklist. Recommended only if you've already verified your setup.").clicked() {
                        self.passed = true;
                        self.pending_status = Some("⚠ Skipped antenna checklist — reception may be poor".to_string());
                    }
                    let mut skip_box = false;
                    if ui.checkbox(&mut skip_box, "Don't show again").changed() && skip_box {
                        if let Ok(mut state) = self.shared.try_lock() {
                            state.config.skip_antenna_checklists = true;
                            state.config.save();
                        }
                        self.passed = true;
                        self.pending_status = Some("Antenna checklists disabled (re-enable in config)".to_string());
                    }
                });
                if !all_critical {
                    ui.add_space(2.0);
                    ui.colored_label(egui::Color32::from_rgb(200, 150, 80),
                        egui::RichText::new("Complete all ● critical items to continue, or use Skip for now.").small());
                }
            });

        // If passed this frame, let content render immediately below.
        self.passed
    }
}

fn crit(label: &'static str, detail: &'static str) -> ChecklistItem {
    ChecklistItem { label, detail, critical: true, checked: false }
}
fn item(label: &'static str, detail: &'static str) -> ChecklistItem {
    ChecklistItem { label, detail, critical: false, checked: false }
}
