//! Listen-mode header: one-tap preset tiles, a plain-language smart-tune
//! box, the ambient Auto demod chip, and a humanized signal meter.

use crate::app::{SecondaryTool, SharedState};
use crate::sdr_panel::DemodMode;
use std::sync::{Arc, Mutex};

/// Render the interactive listen header and collapsing adjust section
pub fn render_listen_header(
    ui: &mut egui::Ui,
    shared: &Arc<Mutex<SharedState>>,
    listen_tune_input: &mut String,
    last_manual_tune_time: &mut std::time::Instant,
    active_secondary_tool: &mut Option<SecondaryTool>,
) {
    // Scope the shared lock to the interactive header; the Adjust expander
    // below takes its own lock so the two never overlap.
    {
        let Some(mut state) = shared.try_lock().ok() else {
            return;
        };

        // ── Preset tiles: one tap tunes + sets mode + starts the source ──
        let presets: &[(&str, u64)] = &[
            ("📻 FM Radio", 98_500_000),
            ("✈ Air Band", 128_000_000),
            ("🌦 Weather", 162_400_000),
            ("🚓 Public Safety", 155_000_000),
            ("📡 Ham 2m", 145_500_000),
            ("🚢 Marine", 156_800_000),
        ];
        ui.horizontal_wrapped(|ui| {
            for (label, freq) in presets {
                if ui
                    .button(*label)
                    .on_hover_text(format!(
                        "Tune {:.3} MHz · {} (Auto)",
                        *freq as f64 / 1e6,
                        DemodMode::for_frequency(*freq).label()
                    ))
                    .clicked()
                {
                    state.source.frequency_hz = *freq;
                    state.demod_mode = DemodMode::Auto;
                    state.source.start();
                    *last_manual_tune_time = std::time::Instant::now();
                }
            }
            if ui
                .button("⭐ Saved")
                .on_hover_text("Open your saved bookmarks")
                .clicked()
            {
                *active_secondary_tool = Some(SecondaryTool::Bookmarks);
            }
        });

        // ── Smart-tune box + Auto chip ──
        ui.horizontal(|ui| {
            ui.label("Tune:");
            let resp = ui.text_edit_singleline(listen_tune_input);
            if ui.button("Go").clicked()
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                let q = listen_tune_input.trim().to_lowercase();
                if !q.is_empty() {
                    // Numeric → MHz. Negative / non-finite input is ignored;
                    // in-range values tune directly, over-range clamps to the
                    // RTL-SDR tuning range used everywhere else
                    // (500 kHz–1770 MHz).
                    if let Ok(mhz) = q.parse::<f64>() {
                        if mhz.is_finite() && mhz > 0.0 {
                            state.source.frequency_hz =
                                ((mhz * 1e6) as u64).clamp(500_000, 1_770_000_000);
                        }
                    } else {
                        let named = match q.as_str() {
                            "weather" | "wx" | "noaa" => Some(162_400_000),
                            "air" | "airband" | "aviation" => Some(128_000_000),
                            "fm" | "radio" | "fm radio" => Some(98_500_000),
                            "marine" | "vhf" | "ship" => Some(156_800_000),
                            "ham" | "2m" | "ham 2m" => Some(145_500_000),
                            "police" | "public" | "public safety" => Some(155_000_000),
                            _ => None,
                        };
                        if let Some(hz) = named {
                            state.source.frequency_hz = hz;
                        } else if let Some(b) = state
                            .bookmarks
                            .bookmarks
                            .iter()
                            .find(|b| b.name.to_lowercase().contains(&q))
                        {
                            state.source.frequency_hz = b.frequency_hz;
                        }
                    }
                    state.demod_mode = DemodMode::Auto;
                    state.source.start();
                    *last_manual_tune_time = std::time::Instant::now();
                }
            }
            ui.add_space(8.0);
            // Auto → concrete mode chip. Click to pin a specific mode.
            let concrete = match state.demod_mode {
                DemodMode::Auto => DemodMode::for_frequency(state.source.frequency_hz),
                m => m,
            };
            let chip_label = if matches!(state.demod_mode, DemodMode::Auto) {
                format!("Auto → {}", concrete.label())
            } else {
                state.demod_mode.label().to_string()
            };
            if ui
                .button(egui::RichText::new(chip_label).strong())
                .on_hover_text(
                    "Auto picks the mode from the frequency. Click to pin a specific mode.",
                )
                .clicked()
            {
                state.demod_mode = match state.demod_mode {
                    DemodMode::Auto => DemodMode::Wfm,
                    DemodMode::Wfm => DemodMode::Fm,
                    DemodMode::Fm => DemodMode::Am,
                    DemodMode::Am => DemodMode::Usb,
                    DemodMode::Usb => DemodMode::Lsb,
                    DemodMode::Lsb => DemodMode::Auto,
                    _ => DemodMode::Auto,
                };
            }
        });

        // ── Plain-language signal meter ──
        let level = state.spectrum.signal_level();
        let (word, color) = if level > -30.0 {
            ("Strong ✓", egui::Color32::from_rgb(60, 220, 100))
        } else if level > -60.0 {
            ("Weak", egui::Color32::from_rgb(230, 200, 60))
        } else if level > -80.0 {
            ("Quiet", egui::Color32::from_rgb(180, 180, 190))
        } else {
            ("Silent", egui::Color32::from_rgb(120, 120, 130))
        };
        ui.horizontal(|ui| {
            ui.label("Signal:");
            ui.colored_label(color, word);
            ui.monospace(egui::RichText::new(format!("{level:>6.0} dB")).small());
        });
    } // end shared-lock scope

    // ── Adjust ▾ expander (default-collapsed): advanced controls ──
    egui::CollapsingHeader::new("⚙ Adjust")
        .default_open(false)
        .show(ui, |ui| {
            if let Ok(mut state) = shared.try_lock() {
                ui.add(egui::Slider::new(&mut state.source.gain_db, 0.0..=49.6).text("Gain (dB)"));
                ui.add(egui::Slider::new(&mut state.squelch, -100.0..=0.0).text("Squelch (dB)"));
                ui.checkbox(&mut state.source.bias_tee, "Bias-T");
            }
        });
}
