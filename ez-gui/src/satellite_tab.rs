//! Satellite tab rendering: Track, Alignment, and Decode sub-tabs.

use crate::ai_panel::AiPanel;
use crate::app::SharedState;
use crate::constellation::ConstellationDisplay;
use crate::decoding_panel::DecodingPanel;
use crate::satellite_panel::{SatellitePanel, SatelliteSubTab};
use crate::status_bar::StatusBar;
use std::sync::{Arc, Mutex};

/// Immediate-mode render function: one `&mut` per panel by design.
/// Grouping these into a context struct would just move the field list.
#[allow(clippy::too_many_arguments)]
pub fn render_satellite_tab(
    ui: &mut egui::Ui,
    shared: &Arc<Mutex<SharedState>>,
    satellite_panel: &mut SatellitePanel,
    satellite_subtab: &mut SatelliteSubTab,
    constellation: &mut ConstellationDisplay,
    decoding_panel: &mut DecodingPanel,
    ai_panel: &mut AiPanel,
    status_bar: &mut StatusBar,
) {
    let theme = shared
        .try_lock()
        .map(|s| s.config.theme_config.clone())
        .unwrap_or_default();
    egui::Panel::top("sat_subtabs")
        .exact_size(36.0)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(4.0);
                for (subtab, label) in [
                    (SatelliteSubTab::Track, "🛰 Track"),
                    (SatelliteSubTab::Alignment, "🧭 Align"),
                    (SatelliteSubTab::Decode, "🛸 Decode"),
                ] {
                    let is_active = *satellite_subtab == subtab;
                    let fg = if is_active {
                        egui::Color32::from_rgb(0, 168, 255)
                    } else {
                        egui::Color32::GRAY
                    };
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(label).color(fg))
                                .fill(egui::Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        *satellite_subtab = subtab;
                    }
                }
            });
        });

    egui::Panel::bottom("sat_status")
        .exact_size(48.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let doppler = satellite_panel.doppler_hz;
                let dop_color = if doppler.abs() > 5000.0 {
                    egui::Color32::from_rgb(255, 120, 60)
                } else if doppler.abs() > 1000.0 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::from_rgb(120, 220, 120)
                };
                ui.colored_label(dop_color, format!("Doppler: {:+.2} kHz", doppler / 1000.0));
                ui.separator();
                ui.label(format!(
                    "Observer: {:.4}°N  {:.4}°E",
                    satellite_panel.observer_lat, satellite_panel.observer_lon
                ));
                if satellite_panel.auto_tune {
                    ui.separator();
                    ui.colored_label(egui::Color32::GREEN, "✓ Auto-tune");
                }
                if satellite_panel.cf32_recording {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, "● REC");
                }
            });
        });

    egui::Panel::right("sat_pipeline")
        .resizable(true)
        .default_size(280.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                satellite_panel.ui(ui, *satellite_subtab);
                if let Some(prompt) = satellite_panel.pending_ai_prompt.take() {
                    ai_panel.input = prompt;
                    status_bar.info("🤖 Satellite details sent to AI".to_string());
                }

                // Live signal-quality constellation — phase/amplitude view
                // (SatDump-style) while a pass is active or recording.
                if satellite_panel.cf32_recording || satellite_panel.map_renderer.in_pass_now {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(egui::RichText::new("📶 Signal Quality").strong());
                    ui.label(
                        egui::RichText::new("IQ constellation")
                            .small()
                            .color(egui::Color32::from_gray(140)),
                    );
                    constellation.ui(ui, &theme);
                }
            });
        });

    egui::CentralPanel::default().show(ui, |ui| match *satellite_subtab {
        SatelliteSubTab::Track => {
            satellite_panel.map_renderer.ui(ui, &theme);
        }
        SatelliteSubTab::Alignment => {
            render_satellite_alignment_central(ui, shared, satellite_panel, satellite_subtab);
        }
        SatelliteSubTab::Decode => {
            egui::ScrollArea::vertical().show(ui, |ui| {
                decoding_panel.ui(ui);
            });
        }
    });
}

pub fn render_satellite_alignment_central(
    ui: &mut egui::Ui,
    shared: &Arc<Mutex<SharedState>>,
    satellite_panel: &mut SatellitePanel,
    satellite_subtab: &mut SatelliteSubTab,
) {
    let theme = shared
        .try_lock()
        .map(|s| s.config.theme_config.clone())
        .unwrap_or_default();
    if let Some(idx) = satellite_panel.selected_sat_index {
        if idx < satellite_panel.satellite_catalog.len() {
            if let Some(pos) = satellite_panel.current_sat_position {
                let align = crate::satellite::alignment::compute_dipole_alignment(
                    pos,
                    crate::satellite::alignment::DipoleType::VDipole137,
                );
                if let Some(alignment) = align {
                    ui.vertical_centered(|ui| {
                        ui.add_space(12.0);
                        crate::satellite::alignment::compass_rose_ui(
                            ui,
                            alignment.compass_heading,
                            pos.azimuth,
                            pos.elevation,
                            pos.distance_km,
                            &theme,
                        );
                    });
                    return;
                }
            }
        }
    }
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.colored_label(
            egui::Color32::GRAY,
            "Pick a satellite (right panel) and wait for a live position update.",
        );
        if ui
            .button("← Pick a satellite")
            .on_hover_text("Open the Track tab to choose a satellite")
            .clicked()
        {
            *satellite_subtab = SatelliteSubTab::Track;
        }
    });
}
