use std::sync::{Arc, Mutex};

use crate::app::SharedState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodMode {
    Raw,
    Am,
    Fm,
    Wfm,
    Lsb,
    Usb,
}

impl DemodMode {
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "RAW" => Some(Self::Raw),
            "AM" => Some(Self::Am),
            "FM" | "NFM" => Some(Self::Fm),
            "WFM" => Some(Self::Wfm),
            "LSB" => Some(Self::Lsb),
            "USB" => Some(Self::Usb),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DemodMode::Raw => "RAW",
            DemodMode::Am => "AM",
            DemodMode::Fm => "FM",
            DemodMode::Wfm => "WFM",
            DemodMode::Lsb => "LSB",
            DemodMode::Usb => "USB",
        }
    }
}

pub struct SdrPanel {
    shared: Arc<Mutex<SharedState>>,
    pub squelch: f32,
    pub filter_bw: u32,
    pub bookmark_request: Option<(u64, String)>,
    freq_input: String,
    auto_squelch: bool,
    auto_squelch_offset: f32,
}

impl SdrPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            squelch: -50.0,
            filter_bw: 12_000,
            bookmark_request: None,
            freq_input: String::new(),
            auto_squelch: false,
            auto_squelch_offset: 5.0,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("SDR Receiver");

        // Big frequency display with fine/coarse tuning
        if let Ok(mut state) = self.shared.try_lock() {
            ui.horizontal(|ui| {
                let mut freq_mhz = state.source.frequency_hz as f64 / 1e6;
                ui.monospace(egui::RichText::new(format!("{:.6}", freq_mhz)).size(24.0).color(egui::Color32::from_rgb(52, 152, 219)))
                    .on_hover_text("Current tuned center frequency. The SDR receives a band centered here. RTL-SDR range: 24 MHz – 1766 MHz.");
                ui.label(egui::RichText::new("MHz").size(14.0).color(egui::Color32::GRAY));
                let is_dragging = ui.add(egui::DragValue::new(&mut freq_mhz).speed(0.0001).range(0.5..=1770.0).suffix(" MHz"))
                    .on_hover_text("Drag left/right or type to tune. You can also click on the spectrum display to tune there.");
                if is_dragging.changed() || is_dragging.dragged() {
                    state.source.frequency_hz = (freq_mhz * 1e6) as u64;
                }
                if ui.small_button("-1M").on_hover_text("Tune down 1 MHz (keyboard: ↓)").clicked() {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(1_000_000).max(500_000);
                }
                if ui.small_button("+1M").on_hover_text("Tune up 1 MHz (keyboard: ↑)").clicked() {
                    state.source.frequency_hz = (state.source.frequency_hz + 1_000_000).min(1_770_000_000);
                }
                if ui.small_button("-100k").on_hover_text("Tune down 100 kHz (keyboard: ←)").clicked() {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(100_000).max(500_000);
                }
                if ui.small_button("+100k").on_hover_text("Tune up 100 kHz (keyboard: →)").clicked() {
                    state.source.frequency_hz = (state.source.frequency_hz + 100_000).min(1_770_000_000);
                }
                if ui.small_button("-10k").on_hover_text("Tune down 10 kHz — fine channel step").clicked() {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(10_000).max(500_000);
                }
                if ui.small_button("+10k").on_hover_text("Tune up 10 kHz — fine channel step").clicked() {
                    state.source.frequency_hz = (state.source.frequency_hz + 10_000).min(1_770_000_000);
                }
                let bm_freq = state.source.frequency_hz;
                let bm_mode = state.demod_mode.label().to_string();
                if ui.small_button("⭐").on_hover_text("Bookmark this frequency — saves it to your bookmarks list with the current mode.").clicked() {
                    self.bookmark_request = Some((bm_freq, bm_mode));
                }
                let copy_freq_str = format!("{:.6}", bm_freq as f64 / 1e6);
                if ui.small_button("📋").on_hover_text("Copy current frequency (MHz) to clipboard.").clicked() {
                    ui.ctx().copy_text(copy_freq_str);
                }
                // Show freeze state from spectrum
                let is_frozen = state.spectrum.frozen;
                if ui.small_button(if is_frozen { "▶ Unfreeze" } else { "❄" })
                    .on_hover_text(if is_frozen { "Unfreeze spectrum display" } else { "Freeze spectrum display (stops updating)" })
                    .clicked()
                {
                    state.spectrum.frozen = !state.spectrum.frozen;
                }
            });
        }
        // VFO A/B swap
        if let Ok(mut state) = self.shared.try_lock() {
            let vfo_b_mhz = state.vfo_b as f64 / 1e6;
            let cur_mhz = state.source.frequency_hz as f64 / 1e6;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("VFO A").strong().color(egui::Color32::from_rgb(52, 200, 100)));
                ui.monospace(format!("{:.3} MHz", cur_mhz));
                ui.separator();
                ui.label(egui::RichText::new("VFO B").color(egui::Color32::from_rgb(100, 180, 255)));
                ui.monospace(format!("{:.3} MHz", vfo_b_mhz));
                if ui.small_button("⇄ Swap")
                    .on_hover_text("Swap between VFO A and VFO B frequencies (keyboard: V). VFO B stores an alternate frequency for quick A/B comparison.")
                    .clicked()
                {
                    let tmp = state.source.frequency_hz;
                    state.source.frequency_hz = state.vfo_b;
                    state.vfo_b = tmp;
                }
                if ui.small_button("Set B here")
                    .on_hover_text("Save current frequency as VFO B without switching.")
                    .clicked()
                {
                    state.vfo_b = state.source.frequency_hz;
                }
            });
        }
        // Nearby bookmark hint
        if let Ok(state) = self.shared.try_lock() {
            let cur_freq = state.source.frequency_hz;
            let threshold_hz = 100_000u64; // ±100 kHz
            let nearest = state.bookmarks.bookmarks.iter()
                .map(|b| (b, if b.frequency_hz > cur_freq { b.frequency_hz - cur_freq } else { cur_freq - b.frequency_hz }))
                .min_by_key(|(_, d)| *d);
            if let Some((bm, dist)) = nearest {
                if dist > 0 && dist <= threshold_hz {
                    let dir = if bm.frequency_hz > cur_freq { "↑" } else { "↓" };
                    let dist_str = if dist >= 1000 { format!("{:.1} kHz", dist as f64 / 1000.0) } else { format!("{} Hz", dist) };
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(180, 200, 255),
                            format!("Near: {} ({}{} away)", bm.name, dir, dist_str),
                        ).on_hover_text(format!("Bookmark '{}' at {:.4} MHz is {} {} — press B to snap to it.", bm.name, bm.frequency_hz as f64 / 1e6, dist_str, if bm.frequency_hz > cur_freq { "above" } else { "below" }));
                    });
                }
            }
        }
        // Tuning step presets
        if let Ok(mut state) = self.shared.try_lock() {
            ui.horizontal(|ui| {
                ui.label("Step:").on_hover_text("Arrow key tuning step. ←→ = fine step, ↑↓ = coarse step. Shift multiplies by 10.");
                for (label, hz, tip) in [
                    ("1k",  1_000u64,   "1 kHz step — for SSB/CW tuning"),
                    ("5k",  5_000,       "5 kHz step — NFM channel spacing (narrow)"),
                    ("8.33k",8_333,      "8.33 kHz step — ICAO aviation channel spacing"),
                    ("10k", 10_000,      "10 kHz step"),
                    ("12.5k",12_500,     "12.5 kHz step — NFM standard spacing"),
                    ("25k", 25_000,      "25 kHz step — wide NFM / older PMR"),
                    ("50k", 50_000,      "50 kHz step"),
                    ("100k",100_000,     "100 kHz step (default fine)"),
                    ("200k",200_000,     "200 kHz step — FM broadcast channel spacing"),
                    ("1M",  1_000_000,   "1 MHz step (default coarse)"),
                ] {
                    let is_fine = state.tune_step_fine_hz == hz;
                    let is_coarse = state.tune_step_coarse_hz == hz;
                    let btn = ui.add(egui::Button::new(egui::RichText::new(label)
                        .color(if is_fine { egui::Color32::from_rgb(80, 200, 120) }
                               else if is_coarse { egui::Color32::from_rgb(100, 160, 255) }
                               else { egui::Color32::GRAY }))
                        .small())
                        .on_hover_text(format!("{} — click once: fine step (←→), click twice: coarse step (↑↓). Current fine: {} Hz, coarse: {} Hz.",
                            tip, state.tune_step_fine_hz, state.tune_step_coarse_hz));
                    if btn.clicked() {
                        if !is_fine {
                            state.tune_step_fine_hz = hz;
                        } else {
                            state.tune_step_coarse_hz = hz;
                        }
                    }
                }
            });
        }
        // Recent frequencies quick-access bar
        if let Ok(mut state) = self.shared.try_lock() {
            let cur = state.source.frequency_hz;
            // collect last 6 unique recent freqs that differ from current
            let recents: Vec<u64> = state.freq_history.iter()
                .cloned()
                .rev()
                .filter(|&f| f != cur)
                .collect::<std::collections::HashSet<u64>>()
                .into_iter()
                .take(8)
                .collect();
            if !recents.is_empty() {
                let mut sorted = recents;
                sorted.sort_by(|a, b| {
                    let da = if *a > cur { a - cur } else { cur - a };
                    let db = if *b > cur { b - cur } else { cur - b };
                    da.cmp(&db)
                });
                let sorted_clone = sorted.clone();
                ui.horizontal(|ui| {
                    ui.small("Recent:");
                    for &f in sorted_clone.iter().take(6) {
                        let label = if f >= 1_000_000_000 {
                            format!("{:.2}G", f as f64 / 1e9)
                        } else if f >= 100_000_000 {
                            format!("{:.1}M", f as f64 / 1e6)
                        } else {
                            format!("{:.3}M", f as f64 / 1e6)
                        };
                        if ui.small_button(egui::RichText::new(label).color(egui::Color32::from_rgb(160, 200, 255)))
                            .on_hover_text(format!("{:.6} MHz", f as f64 / 1e6))
                            .clicked()
                        {
                            state.source.frequency_hz = f;
                        }
                    }
                });
            }
        }
        // Direct frequency entry
        ui.horizontal(|ui| {
            ui.label("Go to:").on_hover_text("Type a frequency and press Enter to jump. Examples: 145.5 (MHz), 145500000 (Hz), 145500k (kHz).");
            let resp = ui.add(egui::TextEdit::singleline(&mut self.freq_input)
                .desired_width(140.0)
                .hint_text("MHz or Hz, Enter to jump"));
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let s = self.freq_input.trim().to_lowercase();
                let parsed_hz: Option<u64> = if let Ok(v) = s.trim_end_matches("mhz").trim().parse::<f64>() {
                    Some((v * 1e6) as u64)
                } else if s.ends_with("khz") || s.ends_with('k') {
                    s.trim_end_matches("khz").trim_end_matches('k').trim().parse::<f64>().ok().map(|v| (v * 1e3) as u64)
                } else if s.ends_with("ghz") || s.ends_with('g') {
                    s.trim_end_matches("ghz").trim_end_matches('g').trim().parse::<f64>().ok().map(|v| (v * 1e9) as u64)
                } else {
                    s.parse::<u64>().ok()
                };
                if let Some(hz) = parsed_hz {
                    let hz = hz.clamp(500_000, 1_770_000_000);
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.source.frequency_hz = hz;
                    }
                    self.freq_input.clear();
                }
            }
        });

        ui.separator();

        // Demod mode selector with bandwidth hints
        ui.horizontal(|ui| {
            if let Ok(mut state) = self.shared.try_lock() {
                for (mode, bw_hint, tip) in [
                    (DemodMode::Raw, "",       "RAW I/Q — pass raw complex samples to external decoders"),
                    (DemodMode::Am,  "8 kHz",  "AM — Amplitude Modulation. Aviation (118–137 MHz), AM broadcast, shortwave"),
                    (DemodMode::Fm,  "12.5 k", "NFM — Narrowband FM. Land mobile radio: police, fire, repeaters, NOAA weather"),
                    (DemodMode::Wfm, "200 k",  "WFM — Wideband FM. Commercial FM broadcast (88–108 MHz). Stereo + RDS"),
                    (DemodMode::Lsb, "2.4 k",  "LSB — Lower Sideband. Amateur HF voice below 10 MHz"),
                    (DemodMode::Usb, "2.4 k",  "USB — Upper Sideband. Amateur HF voice above 10 MHz, utility/military"),
                ] {
                    let selected = state.demod_mode == mode;
                    let label = if bw_hint.is_empty() {
                        mode.label().to_string()
                    } else {
                        format!("{} {}", mode.label(), bw_hint)
                    };
                    if ui.selectable_label(selected, label)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        state.demod_mode = mode;
                    }
                }
            }
        });

        // Band-aware demod auto-suggest
        if let Ok(mut state) = self.shared.try_lock() {
            let freq = state.source.frequency_hz;
            let current = state.demod_mode;
            if let Some((suggested, band_name, reason)) = suggest_demod_for_freq(freq) {
                if suggested != current {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(255, 200, 50), "💡");
                        ui.label(egui::RichText::new(format!("{} →", band_name)).color(egui::Color32::from_rgb(180, 180, 180)));
                        if ui.small_button(suggested.label())
                            .on_hover_text(format!("Switch to {} — {}", suggested.label(), reason))
                            .clicked()
                        {
                            state.demod_mode = suggested;
                        }
                        ui.label(egui::RichText::new(format!("({})", reason)).color(egui::Color32::GRAY).small());
                    });
                }
            }
        }

        // Signal meter + SNR
        ui.separator();
        if let Ok(state) = self.shared.try_lock() {
            let signal = state.spectrum.signal_level();
            let noise_floor = state.spectrum.noise_floor();
            let peak = state.spectrum.peak_level();
            let snr = peak - noise_floor;
            let norm = ((signal + 120.0) / 120.0).clamp(0.0, 1.0);
            // VU-style signal meter
            ui.horizontal(|ui| {
                ui.label("Signal:").on_hover_text("RF signal level in dBFS. Green zone (>-40 dB) = strong. Yellow (−60–40) = moderate. Red (<-60) = weak/noise.");
                // Draw custom colored bar using painter
                let desired = egui::vec2(180.0, 14.0);
                let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
                let response = response.on_hover_text(format!("Signal: {:.1} dBFS  SNR: {:.1} dB  Noise: {:.1} dB", signal, snr, noise_floor));
                let p = ui.painter();
                p.rect_filled(rect, 2.0, egui::Color32::from_rgb(15, 15, 25));
                // Zones: 0–50% red, 50–75% yellow, 75–100% green
                let zones = [
                    (0.0f32, 0.5f32, egui::Color32::from_rgb(150, 30, 30)),
                    (0.5f32, 0.75f32, egui::Color32::from_rgb(180, 150, 20)),
                    (0.75f32, 1.0f32, egui::Color32::from_rgb(30, 150, 50)),
                ];
                for (lo, hi, c) in &zones {
                    let fill = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + lo * rect.width(), rect.top()),
                        egui::pos2(rect.left() + hi * rect.width(), rect.bottom()),
                    );
                    p.rect_filled(fill, 0.0, egui::Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), 40));
                }
                // Filled bar up to signal level
                let fill_w = norm * rect.width();
                let fill_color = if norm > 0.75 { egui::Color32::from_rgb(50, 200, 80) }
                    else if norm > 0.5 { egui::Color32::from_rgb(220, 180, 30) }
                    else { egui::Color32::from_rgb(200, 50, 50) };
                p.rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height())),
                    2.0, fill_color,
                );
                // Tick marks every 20 dB
                for db in (-120..=0i32).step_by(20) {
                    let t = ((db as f32 - (-120.0)) / 120.0).clamp(0.0, 1.0);
                    let x = rect.left() + t * rect.width();
                    p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(0.5, egui::Color32::from_gray(80)));
                }
                // Level text
                p.text(egui::pos2(rect.right() - 2.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{:.0}dB", signal),
                    egui::FontId::monospace(9.0),
                    egui::Color32::WHITE);
                let _ = response;

                ui.separator();
                let snr_color = if snr > 20.0 { egui::Color32::GREEN }
                    else if snr > 10.0 { egui::Color32::YELLOW }
                    else { egui::Color32::GRAY };
                let (quality_str, quality_tip) = if snr > 25.0 {
                    ("● Excellent", "SNR >25 dB: very clean signal, audio will be clear.")
                } else if snr > 15.0 {
                    ("● Good", "SNR 15–25 dB: good reception, audio should be clean.")
                } else if snr > 8.0 {
                    ("● Marginal", "SNR 8–15 dB: weak signal, audio may be noisy or choppy.")
                } else {
                    ("● No signal", "SNR <8 dB: no meaningful signal at this frequency. Try moving the antenna, increasing gain, or tuning to a different frequency.")
                };
                ui.colored_label(snr_color, format!("SNR {:.1} dB", snr))
                    .on_hover_text("Signal-to-Noise Ratio: peak dB minus estimated noise floor. >20 dB = excellent, 10–20 dB = good, <10 dB = marginal. Aim for >15 dB for clean audio.");
                ui.colored_label(snr_color, quality_str).on_hover_text(quality_tip);
            });
        }

        // Signal history sparkline
        if let Ok(state) = self.shared.try_lock() {
            let history = state.spectrum.signal_history_snapshot();
            let history_max = state.spectrum.signal_history_max();
            if history.len() >= 2 {
                let desired = egui::vec2(180.0, 30.0);
                let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
                let response = response.on_hover_text("Signal strength over time (last 60s). Shows peaks only. Helps identify if a signal is continuous, periodic, or intermittent.");
                let p = ui.painter();
                p.rect_filled(rect, 2.0, egui::Color32::from_rgb(10, 10, 20));
                let n = history.len();
                let min_db = -120.0f32;
                let max_db = 0.0f32;
                let db_range = max_db - min_db;
                let points: Vec<egui::Pos2> = history.iter().enumerate().map(|(i, &db)| {
                    let x = rect.left() + (i as f32 / (history_max as f32 - 1.0).max(1.0)) * rect.width();
                    let norm = ((db - min_db) / db_range).clamp(0.0, 1.0);
                    let y = rect.bottom() - norm * rect.height();
                    egui::pos2(x, y)
                }).collect();
                for win in points.windows(2) {
                    let norm = (win[1].y - rect.top()) / rect.height();
                    let c = if norm < 0.25 { egui::Color32::from_rgb(50, 200, 80) }
                        else if norm < 0.5 { egui::Color32::from_rgb(180, 160, 30) }
                        else { egui::Color32::from_rgb(100, 60, 200) };
                    p.line_segment([win[0], win[1]], egui::Stroke::new(1.0, c));
                }
                // Label
                p.text(egui::pos2(rect.left() + 2.0, rect.top() + 2.0),
                    egui::Align2::LEFT_TOP, "60s",
                    egui::FontId::monospace(8.0), egui::Color32::from_gray(100));
                let _ = response;
                let _ = n;
            }
        }

        // Overload detection + smart gain
        if let Ok(mut state) = self.shared.try_lock() {
            let peak = state.spectrum.peak_level();
            let gain = state.source.gain_db;
            if peak > -15.0 && gain > 0.0 {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(255, 80, 80),
                        format!("⚠ Overload! Peak {:.0} dBFS", peak))
                        .on_hover_text("Signal is clipping the ADC — this causes distortion, ghost signals, and desensitization. Reduce gain.");
                    if ui.small_button("-10 dB")
                        .on_hover_text("Reduce gain by 10 dB to eliminate overload.")
                        .clicked()
                    {
                        state.source.gain_db = (gain - 10.0).max(0.0);
                    }
                });
            }
            if ui.small_button("Smart Gain")
                .on_hover_text(format!(
                    "Auto-adjust gain to target -30 dBFS peak. Current peak: {:.0} dBFS, current gain: {:.1} dB.",
                    peak, gain
                ))
                .clicked()
            {
                let adjustment = -30.0 - peak as f64;
                state.source.gain_db = (gain + adjustment).clamp(0.0, 49.6);
            }
            // Gain presets
            ui.horizontal(|ui| {
                ui.label("Gain:").on_hover_text("Quick gain presets for common scenarios.");
                for (label, db, tip) in [
                    ("Low",  10.0f64, "Low gain (10 dB) — for very strong nearby transmitters or when overloading."),
                    ("Med",  28.0, "Medium gain (28 dB) — good starting point for most environments."),
                    ("High", 42.0, "High gain (42 dB) — for weak distant signals. Watch for overload."),
                    ("Max",  49.6, "Maximum gain (49.6 dB) — only for very weak signals. High overload risk."),
                ] {
                    let is_active = (state.source.gain_db - db).abs() < 0.5;
                    let btn = ui.add(egui::Button::new(egui::RichText::new(label).small()
                        .color(if is_active { egui::Color32::BLACK } else { egui::Color32::from_rgb(200, 220, 200) }))
                        .fill(if is_active { egui::Color32::from_rgb(60, 160, 80) } else { egui::Color32::from_rgba_premultiplied(30, 50, 30, 80) })
                        .small())
                        .on_hover_text(tip);
                    if btn.clicked() { state.source.gain_db = db; }
                }
            });
        }

        // Demod quality indicators
        if let Ok(state) = self.shared.try_lock() {
            let mode = state.demod_mode;
            if mode == DemodMode::Fm || mode == DemodMode::Wfm {
                let dev_khz = state.fm_deviation_hz / 1000.0;
                let dev_color = if dev_khz > 75.0 { egui::Color32::RED }
                    else if dev_khz > 5.0 { egui::Color32::GREEN }
                    else { egui::Color32::GRAY };
                ui.horizontal(|ui| {
                    ui.colored_label(dev_color, format!("FM dev: {:.1} kHz", dev_khz))
                        .on_hover_text("FM frequency deviation. NFM: 5–12.5 kHz is normal. WFM broadcast: up to 75 kHz. >75 kHz = overmodulated.");
                });
                // Audio level meter bar
                let peak_frac = (state.audio_peak).clamp(0.0, 1.0);
                ui.horizontal(|ui| {
                    ui.label("Audio:");
                    let bar_w = 100.0f32;
                    let bar_h = 10.0f32;
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(20, 20, 30));
                    let fill_w = rect.width() * peak_frac;
                    let bar_color = if peak_frac > 0.9 { egui::Color32::from_rgb(220, 50, 50) }
                        else if peak_frac > 0.6 { egui::Color32::from_rgb(50, 200, 80) }
                        else if peak_frac > 0.05 { egui::Color32::from_rgb(40, 160, 60) }
                        else { egui::Color32::from_rgb(50, 70, 50) };
                    if fill_w > 0.5 {
                        painter.rect_filled(
                            egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, bar_h)),
                            2.0, bar_color,
                        );
                    }
                    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 70, 80)), egui::StrokeKind::Middle);
                    resp.on_hover_text(format!("Audio output level: {:.0}%. >90% = clipping risk — lower volume.", peak_frac * 100.0));
                });
            }
        }

        // Audio controls
        ui.separator();
        ui.horizontal(|ui| {
            let mut start_audio = false;
            let mut stop_audio = false;
            let mut volume = 0.5f32;
            let mut audio_running = false;
            if let Ok(state) = self.shared.try_lock() {
                volume = state.volume;
                audio_running = state.audio_running;
            }
            if audio_running {
                if ui.button("🔇 Stop Audio").on_hover_text("Stop audio playback through your speakers/headphones.").clicked() {
                    stop_audio = true;
                }
            } else {
                if ui.button("🔊 Start Audio").on_hover_text("Start playing the demodulated signal through your speakers/headphones.").clicked() {
                    start_audio = true;
                }
            }
            let mut vol = volume;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Vol"))
                .on_hover_text("Audio output volume. Does not affect the RF signal — only the speaker output level.")
                .changed()
            {
                volume = vol;
            }
            if start_audio || stop_audio || vol != volume {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.volume = volume;
                    if start_audio { state.audio_running = true; }
                    else if stop_audio { state.audio_running = false; }
                }
            }
        });

        ui.separator();

        // Frequency presets (band quick-tune) — also auto-sets demod mode
        ui.horizontal_wrapped(|ui| {
            ui.label("Bands:").on_hover_text("Quick-tune presets. Click to jump to that frequency and automatically switch to the right demodulation mode.");
            // (name, freq_hz, demod_mode_label, tip)
            const BANDS: &[(&str, u64, &str, &str)] = &[
                ("BC FM",   100_000_000, "WFM",  "FM Broadcast band center (88–108 MHz) → WFM mode"),
                ("Air",     118_000_000, "AM",   "Aviation VHF voice band (118–137 MHz) → AM mode (not FM!)"),
                ("NOAA WX", 162_400_000, "NFM",  "NOAA Weather Radio (162.400–162.550 MHz) → NFM mode"),
                ("Marine",  156_800_000, "NFM",  "Marine VHF distress channel 16 (156.8 MHz) → NFM mode"),
                ("2m",      144_000_000, "NFM",  "Amateur 2-meter band. Repeaters, APRS at 144.390 MHz → NFM"),
                ("APRS",    144_390_000, "NFM",  "APRS digipeater/tracker beacon (144.390 MHz US) → NFM mode"),
                ("70cm",    430_000_000, "NFM",  "Amateur 70cm band. FM repeaters, digital modes → NFM"),
                ("PMR446",  446_006_250, "NFM",  "PMR446 licence-free walkie-talkies (446.006–446.194 MHz) → NFM"),
                ("NOAA 15", 137_620_000, "WFM",  "NOAA 15 weather satellite (137.620 MHz) → WFM 34 kHz"),
                ("NOAA 18", 137_912_500, "WFM",  "NOAA 18 weather satellite (137.9125 MHz) → WFM 34 kHz"),
                ("NOAA 19", 137_100_000, "WFM",  "NOAA 19 weather satellite (137.100 MHz) → WFM 34 kHz"),
                ("ISS",     145_800_000, "NFM",  "International Space Station voice (145.800 MHz) → NFM"),
                ("ADS-B",  1_090_000_000, "RAW", "Mode-S/ADS-B aircraft transponders (1090 MHz) → RAW (ADS-B decoder)"),
            ];
            if let Ok(mut state) = self.shared.try_lock() {
                for (name, freq_hz, mode_str, tip) in BANDS {
                    if ui.small_button(*name).on_hover_text(*tip).clicked() {
                        state.source.frequency_hz = *freq_hz;
                        if let Some(mode) = DemodMode::from_label(mode_str) {
                            state.demod_mode = mode;
                        }
                    }
                }
            }
        });

        // Band info hint — tells beginner what they're likely listening to at current freq
        if let Ok(state) = self.shared.try_lock() {
            let freq = state.source.frequency_hz;
            // (start_hz, end_hz, description, color_rgb)
            const BAND_INFO: &[(u64, u64, &str, (u8, u8, u8))] = &[
                (148_000,    530_000,    "LW/MW broadcast. Amplitude-modulated radio stations, aviation beacons (NDB).", (180, 160, 100)),
                (530_000,  1_710_000,   "AM broadcast band. Local radio stations. Use AM mode.", (180, 160, 100)),
                (1_710_000, 30_000_000, "HF shortwave. International broadcast, amateur radio (use SSB), maritime, military.", (100, 180, 255)),
                (87_500_000, 108_000_000, "FM broadcast band. Local music/talk radio stations. Use WFM mode.", (80, 200, 120)),
                (108_000_000, 118_000_000, "VOR/ILS navigation aids. Aircraft instrument approaches. AM mode.", (200, 200, 80)),
                (118_000_000, 137_000_000, "Aviation VHF band. Air traffic control, ATIS, ground. Use AM mode (not FM!).", (200, 200, 80)),
                (136_000_000, 138_000_000, "Weather satellite downlink (NOAA APT at 137.1–137.9 MHz). Use WFM or RAW.", (100, 220, 220)),
                (144_000_000, 148_000_000, "Amateur 2m band. FM voice repeaters, APRS (144.390 MHz). Use NFM.", (160, 120, 255)),
                (156_000_000, 174_000_000, "Marine VHF. Channel 16 (distress) = 156.8 MHz. Use NFM.", (80, 160, 255)),
                (162_400_000, 162_600_000, "NOAA Weather Radio. Continuous weather broadcasts. Use NFM.", (100, 220, 220)),
                (430_000_000, 440_000_000, "Amateur 70cm band. FM repeaters, ATV, digital. Use NFM.", (160, 120, 255)),
                (433_050_000, 434_790_000, "433 MHz ISM band. Remote controls, key fobs, weather stations. Use NFM/AM.", (200, 140, 80)),
                (446_000_000, 446_200_000, "PMR446 walkie-talkies (licence-free). Use NFM.", (200, 140, 80)),
                (460_000_000, 470_000_000, "UHF land mobile. Business radios, public safety (varies by country). Use NFM.", (160, 160, 160)),
                (850_000_000, 900_000_000, "GSM 850 / cellular. Digital — you'll see wideband signal but no decodable audio.", (120, 120, 120)),
                (1_090_000_000, 1_090_000_000, "ADS-B Mode-S (1090 MHz). Aircraft transponders — use ADS-B tab with RAW mode.", (80, 200, 255)),
                (1_575_420_000, 1_575_420_000, "GPS L1 signal (1575.42 MHz). Very weak — needs a GPS LNA to receive.", (120, 200, 80)),
            ];
            let band_desc = BAND_INFO.iter().find(|(lo, hi, _, _)| {
                if lo == hi { freq.abs_diff(*lo) < 500_000 } else { freq >= *lo && freq <= *hi }
            });
            if let Some((_, _, desc, (r, g, b))) = band_desc {
                ui.horizontal(|ui| {
                    ui.label("📡").on_hover_text("Band information for the current frequency.");
                    ui.colored_label(egui::Color32::from_rgb(*r, *g, *b), *desc);
                });
            }
        }

        // Recent frequencies (last 8 from history, most recent first)
        if let Ok(state) = self.shared.try_lock() {
            if state.freq_history.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Recent:").on_hover_text("Last tuned frequencies — click to jump back.");
                    let hist: Vec<u64> = state.freq_history.iter().cloned().rev().skip(1).take(8).collect();
                    for freq in hist {
                        let label = format!("{:.3}", freq as f64 / 1e6);
                        if ui.small_button(&label).on_hover_text(format!("{:.3} MHz — click to retune", freq as f64 / 1e6)).clicked() {
                            drop(state); // release lock before re-acquiring
                            if let Ok(mut st) = self.shared.try_lock() {
                                st.source.frequency_hz = freq;
                            }
                            return;
                        }
                    }
                });
            }
        }

        // Filter bandwidth and squelch
        ui.add(egui::Slider::new(&mut self.filter_bw, 100..=250_000).text("Filter BW (Hz)").logarithmic(true))
            .on_hover_text("Receiver filter bandwidth. Set just wider than the signal. WFM: 200 kHz, NFM voice: 12–16 kHz, AM voice: 8 kHz, SSB: 2.4 kHz. Too wide = more noise.");
        if let Ok(mut state) = self.shared.try_lock() {
            ui.add(egui::Slider::new(&mut state.lpf_cutoff, 100.0..=20000.0).text("Audio LPF (Hz)").logarithmic(true))
                .on_hover_text("Low-pass filter on audio output. Cuts high-frequency hiss above this frequency. Default 15 kHz is fine for voice. Lower for CW/Morse (~800 Hz).");
        }
        // Auto-squelch tracking: update squelch every frame when enabled
        if self.auto_squelch {
            if let Ok(mut state) = self.shared.try_lock() {
                let noise = state.spectrum.noise_floor();
                if noise < -30.0 {
                    let tracked = (noise + self.auto_squelch_offset).min(0.0);
                    self.squelch = tracked;
                    state.squelch = tracked;
                }
            }
        }
        ui.horizontal(|ui| {
            let sq_resp = ui.add(egui::Slider::new(&mut self.squelch, -120.0..=0.0).text("Squelch (dB)"))
                .on_hover_text("Signal level threshold. Audio is muted when signal drops below this value, silencing static between transmissions. Set ~5 dB above your noise floor.");
            if sq_resp.changed() || ui.input(|i| i.pointer.any_down()) {
                self.auto_squelch = false;
                if let Ok(mut state) = self.shared.try_lock() {
                    state.squelch = self.squelch;
                }
            }
            if ui.small_button("Auto").on_hover_text("Set squelch to offset dB above current noise floor (one-shot).").clicked() {
                self.auto_squelch = false;
                if let Ok(mut state) = self.shared.try_lock() {
                    let noise = state.spectrum.noise_floor();
                    let auto_sq = (noise + self.auto_squelch_offset).min(0.0);
                    self.squelch = auto_sq;
                    state.squelch = auto_sq;
                }
            }
            let track_label = if self.auto_squelch {
                egui::RichText::new("Track ON").color(egui::Color32::from_rgb(80, 220, 120))
            } else {
                egui::RichText::new("Track")
            };
            if ui.small_button(track_label)
                .on_hover_text("Continuously track the noise floor and keep squelch at floor + offset. Adjusts automatically as conditions change.")
                .clicked()
            {
                self.auto_squelch = !self.auto_squelch;
            }
            ui.add(egui::DragValue::new(&mut self.auto_squelch_offset)
                .speed(0.5)
                .range(0.0..=30.0)
                .suffix(" dB offset"))
                .on_hover_text("How many dB above the noise floor to set squelch when using Auto or Track.");
            if ui.small_button("Off").on_hover_text("Disable squelch — audio always passes regardless of signal strength.").clicked() {
                self.auto_squelch = false;
                self.squelch = -120.0;
                if let Ok(mut state) = self.shared.try_lock() {
                    state.squelch = -120.0;
                }
            }
        });

        // Frequency identification
        if let Ok(state) = self.shared.try_lock() {
            let freq = state.source.frequency_hz;
            if let Some(info) = identify_frequency(freq) {
                ui.separator();
                ui.collapsing(format!("📻 {} — {}", info.band, info.short_desc), |ui| {
                    ui.label(egui::RichText::new(info.detail).color(egui::Color32::from_rgb(200, 200, 200)));
                    if !info.tips.is_empty() {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new(format!("💡 {}", info.tips)).small().color(egui::Color32::GRAY));
                    }
                });
            }
        }

        ui.separator();

        // LO / Upconverter offset
        if let Ok(mut state) = self.shared.try_lock() {
            ui.horizontal(|ui| {
                ui.label("LO Offset").on_hover_text("Frequency offset for upconverters (e.g. Ham It Up adds 125 MHz). The displayed frequency = tuned + offset. Set to 0 for direct SDR use.");
                let mut lo_mhz = state.lo_offset_hz as f64 / 1e6;
                let drag = ui.add(egui::DragValue::new(&mut lo_mhz)
                    .speed(0.1)
                    .suffix(" MHz")
                    .range(-2000.0..=2000.0));
                if drag.changed() {
                    state.lo_offset_hz = (lo_mhz * 1e6) as i64;
                }
                for (label, offset_mhz, tip) in [
                    ("0", 0.0f64, "No offset (direct SDR)"),
                    ("125", 125.0, "Ham It Up / generic 125 MHz upconverter"),
                    ("100", 100.0, "SpyVerter / 100 MHz upconverter"),
                    ("-125", -125.0, "125 MHz downconverter / negative offset"),
                ] {
                    if ui.small_button(label).on_hover_text(tip).clicked() {
                        state.lo_offset_hz = (offset_mhz * 1e6) as i64;
                    }
                }
            });
            if state.lo_offset_hz != 0 {
                let true_freq = state.source.frequency_hz as i64 + state.lo_offset_hz;
                if true_freq > 0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 200, 80),
                        format!("True frequency: {:.3} MHz (tuned {:.3} MHz + {:.0} MHz offset)",
                            true_freq as f64 / 1e6,
                            state.source.frequency_hz as f64 / 1e6,
                            state.lo_offset_hz as f64 / 1e6),
                    );
                }
            }
        }

        ui.separator();

        // Source controls
        if let Ok(mut state) = self.shared.try_lock() {
            state.source.ui(ui);
        }
    }
}

pub struct FreqIdInfo {
    pub band: &'static str,
    pub short_desc: &'static str,
    pub detail: &'static str,
    pub tips: &'static str,
}

pub fn identify_frequency(freq_hz: u64) -> Option<FreqIdInfo> {
    let entries: &[(u64, u64, &str, &str, &str, &str)] = &[
        (150_000,   500_000,   "LF/MF",         "Long & medium wave",
            "AM broadcast (MW), maritime beacons, time signals (DCF77/MSF).",
            "Use AM demod. Long-wave AM goes down to 150 kHz. DCF77 at 77.5 kHz carries atomic time."),
        (1_800_000, 3_500_000, "160m HF Amateur","Amateur 160m (Top Band)",
            "CW at 1.8 MHz, voice SSB from 1.84 MHz. Very long-range at night.",
            "Use LSB for voice, CW mode for Morse. Best reception after dark."),
        (3_500_000, 4_000_000, "80m HF Amateur", "Amateur 80m band",
            "Busy night band — CW, SSB voice, digital modes. Excellent DX at night.",
            "Use LSB. Expect crowded frequencies especially 3.5–3.8 MHz."),
        (7_000_000, 7_300_000, "40m HF Amateur", "Amateur 40m band",
            "CW and digital 7.0–7.07, SSB 7.1–7.3. Strong DX day and night.",
            "LSB below 10 MHz. FT8 digital at 7.074 MHz is very busy."),
        (10_000_000, 10_150_000, "30m HF Amateur", "Amateur 30m band",
            "CW and digital only. FT8 at 10.136 MHz. No phone allowed.",
            "USB. Narrow band — good for digital modes like FT8/FT4."),
        (14_000_000, 14_350_000, "20m HF Amateur", "Amateur 20m band",
            "Most popular HF amateur band. Excellent DX any time of day.",
            "USB above 10 MHz. FT8 at 14.074 MHz. SSB voice from 14.150 MHz."),
        (21_000_000, 21_450_000, "15m HF Amateur", "Amateur 15m band",
            "Good daytime DX, especially solar maximum. Opens to distant DX.",
            "USB. FT8 at 21.074 MHz. Active during day."),
        (24_890_000, 24_990_000, "12m HF Amateur", "Amateur 12m band",
            "Near full shortwave for DX. Best near solar maximum.",
            "USB. FT8 at 24.915 MHz."),
        (26_965_000, 27_405_000, "CB (Citizens Band)", "CB radio — 40 channels AM",
            "Truckers, 4x4 off-road, short-range comms. Channel 19 = 27.185 MHz trucker net.",
            "AM demod. Ch9 (27.065 MHz) is emergency channel. USB is used for DX on some channels."),
        (28_000_000, 29_700_000, "10m HF Amateur", "Amateur 10m band",
            "Excellent when solar cycle is active. Worldwide DX with modest antennas.",
            "USB. FT8 at 28.074 MHz. CW at 28.0–28.070 MHz."),
        (50_000_000, 54_000_000, "6m Amateur",    "Amateur 6m 'magic band'",
            "VHF sporadic-E propagation — can provide continent-wide DX unexpectedly.",
            "USB for voice/FT8. Known for surprise openings with low power."),
        (88_000_000, 108_000_000, "FM Broadcast", "Commercial FM radio (88–108 MHz)",
            "Stereo music, news, talk radio. RDS data embedded. WFM demod, wide 200 kHz BW.",
            "WFM mode, BW ~200 kHz. Many SDRs receive RDS text alongside audio."),
        (108_000_000, 118_000_000, "VOR/ILS",     "Aviation navigation aids",
            "VHF Omni-directional Range (VOR) and Instrument Landing System. Not voice.",
            "AM demod. These are navigation signals — you'll hear a morse identifier and tone."),
        (118_000_000, 137_000_000, "Aviation VHF", "Air Traffic Control (ATC)",
            "ATC talking to aircraft. Approach, ground, tower, ATIS, centre frequencies.",
            "AM demod. ATIS (airport weather) are automated — listen for your local airport."),
        (137_000_000, 138_000_000, "NOAA Satellites","Weather satellite downlinks",
            "NOAA 15/18/19 send APT image data at 137.5/137.9/137.1 MHz. Visible passes only.",
            "WFM 34 kHz BW. Use SDR# or NOAA-APT software to decode the image."),
        (144_000_000, 148_000_000, "Amateur 2m",   "2-meter amateur radio band",
            "Most active VHF amateur band. FM repeaters, simplex, satellite links, weak-signal.",
            "NFM for voice. 144.0–144.1 MHz CW/SSB DX. 144.390 MHz is APRS."),
        (150_000_000, 156_000_000, "Land Mobile",  "Public safety, utilities, business",
            "Police, fire, taxis, railways. Mix of NFM voice and digital (DMR, P25).",
            "NFM. Digital signals sound like fast data/buzzing — need separate decoder."),
        (156_000_000, 174_000_000, "Marine VHF",   "Maritime communications",
            "Channel 16 (156.8 MHz) = international distress and hailing. Working channels 17–28.",
            "NFM. DSC digital safety calls on Ch70 (156.525 MHz)."),
        (162_400_000, 162_600_000, "NOAA WX Radio","US NOAA Weather Radio",
            "7 channels of continuous weather broadcasts, warnings, forecasts.",
            "WFM or NFM. Automated voice — very strong signal near transmitters."),
        (406_000_000, 406_100_000, "EPIRB/PLB",    "Emergency distress beacons (406 MHz)",
            "EPIRB and PLB satellite-linked emergency beacons. Narrow digital bursts.",
            "NFM. Should be silent unless a genuine emergency — do not transmit here."),
        (420_000_000, 450_000_000, "Amateur 70cm",  "70-centimeter amateur band",
            "FM repeaters, weak-signal EME, ATV, digital modes. Most active: 430–440 MHz.",
            "NFM for voice repeaters. 432.1 MHz SSB weak-signal DX. 433.0 MHz simplex."),
        (433_000_000, 435_000_000, "ISM 433 MHz",  "License-free ISM devices",
            "Car key fobs, wireless doorbells, weather stations, cheap sensors.",
            "NFM or RAW. Short OOK/FSK bursts — decode with rtl_433 tool."),
        (450_000_000, 470_000_000, "UHF LMR",      "UHF land mobile radio",
            "Business, public safety, taxis, transport. Mix of FM voice and digital.",
            "NFM. DMR, P25, NXDN digital systems sound like buzzing/data bursts."),
        (890_000_000, 960_000_000, "GSM 900",      "2G cellular (GSM)",
            "Legacy 2G voice/SMS. Uplink 890–915 MHz, downlink 935–960 MHz.",
            "RAW — encrypted. You'll see signal but can't decode content legally."),
        (1_090_000_000, 1_090_000_000, "ADS-B",    "Aircraft position transponders",
            "ADS-B 1090ES — aircraft broadcast position, altitude, speed. 1 second updates.",
            "Use the ADS-B tab in ez-sdr! RAW mode + 2.4 MSps. Works at 1090 ±1 MHz."),
        (1_215_000_000, 1_240_000_000, "L-band Radar","Radar altimeters, navigation",
            "Radar altimeters and L-band surveillance radars. Pulsed signals.",
            "RAW. Short bursts visible in the waterfall."),
        (1_525_000_000, 1_559_000_000, "L-band Sat","L-band satellite downlinks",
            "Inmarsat/Iridium voice, AERO aviation data, SCADA, MSS phones.",
            "WFM/NFM. Inmarsat AERO at 1.5465 GHz carries ATC/aircraft data."),
        (1_559_000_000, 1_610_000_000, "GPS/GNSS",  "GPS/GALILEO/GLONASS signals",
            "Navigation satellite signals. Very weak broadband BPSK. L1 at 1575.42 MHz.",
            "RAW with wide BW. Use dedicated GPS software — too weak for audio."),
        (1_626_000_000, 1_661_000_000, "Iridium",   "Iridium satellite phones",
            "Iridium NEXT LEO satellite constellation. Burst data, voice, IoT links.",
            "RAW or WFM. Bursts every ~90 seconds when satellites pass."),
        (1_694_000_000, 1_700_000_000, "GOES Sat",  "GOES weather satellite downlinks",
            "GOES-16/17/18 East/West at 1694.1 MHz: HRIT full-disk weather images.",
            "RAW, needs 2+ MSps and special decoder (goestools/SatDump)."),
    ];
    for &(lo, hi, band, short_desc, detail, tips) in entries {
        if freq_hz >= lo && freq_hz <= hi.max(lo) {
            return Some(FreqIdInfo { band, short_desc, detail, tips });
        }
    }
    None
}

/// Returns (suggested_mode, band_name, reason) if the frequency matches a well-known band
/// and the suggestion would differ from the current mode.
fn suggest_demod_for_freq(freq_hz: u64) -> Option<(DemodMode, &'static str, &'static str)> {
    let bands: &[(u64, u64, DemodMode, &str, &str)] = &[
        (88_000_000,  108_000_000, DemodMode::Wfm, "FM Broadcast",     "WFM for commercial radio"),
        (118_000_000, 137_000_000, DemodMode::Am,  "Aviation",          "AM for air-to-ground voice"),
        (137_000_000, 138_000_000, DemodMode::Fm,  "NOAA APT",          "NFM for weather satellite"),
        (144_000_000, 148_000_000, DemodMode::Fm,  "Amateur 2m",        "NFM for repeaters/simplex"),
        (156_000_000, 174_000_000, DemodMode::Fm,  "Marine VHF",        "NFM for ship/coast guard"),
        (162_400_000, 162_600_000, DemodMode::Wfm, "NOAA Weather",      "WFM for NOAA broadcasts"),
        (420_000_000, 450_000_000, DemodMode::Fm,  "Amateur 70cm",      "NFM for amateur radio"),
        (26_965_000,  27_405_000,  DemodMode::Am,  "CB Radio",          "AM for Citizens Band"),
        (150_000_000, 156_000_000, DemodMode::Fm,  "Land Mobile",       "NFM for land mobile radio"),
        (450_000_000, 470_000_000, DemodMode::Fm,  "UHF LMR",           "NFM for UHF land mobile"),
    ];
    for &(lo, hi, mode, name, reason) in bands {
        if freq_hz >= lo && freq_hz <= hi {
            return Some((mode, name, reason));
        }
    }
    None
}
