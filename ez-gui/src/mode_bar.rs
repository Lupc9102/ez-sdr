//! Mode bar: top navigation bar containing the primary workspaces and the
//! Setup Guide and Settings popups.

use crate::adsb_panel::AdsBPanel;
use crate::app::SharedState;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Listen,
    Planes,
    Satellites,
    Meteor,
}

pub const MAIN_TABS: &[(&str, AppTab, &str, &str, &str)] = &[
    (
        "listen",
        AppTab::Listen,
        "🎧",
        "Radio",
        "Radio — Spectrum, tuning and audio",
    ),
    (
        "planes",
        AppTab::Planes,
        "✈",
        "ADS-B",
        "ADS-B — Live aircraft tracking",
    ),
    (
        "meteor",
        AppTab::Meteor,
        "🌦",
        "Meteor",
        "Meteor — Decode recorded .cs8 LRPT files",
    ),
];

/// Immediate-mode render function: one `&mut` per panel/widget by design.
/// Grouping these into a context struct would just move the field list.
#[allow(clippy::too_many_arguments)]
pub fn render_mode_bar(
    ui: &mut egui::Ui,
    shared: &Arc<Mutex<SharedState>>,
    current_tab: &mut AppTab,
    show_setup_guide: &mut bool,
    show_settings: &mut bool,
    adsb_panel: &mut AdsBPanel,
) {
    let colors = shared
        .try_lock()
        .map(|state| state.config.theme_config.clone())
        .unwrap_or_default();
    let radio_workspace = *current_tab == AppTab::Listen;
    ui.spacing_mut().item_spacing = if radio_workspace {
        egui::vec2(4.0, 2.0)
    } else {
        egui::vec2(6.0, 2.0)
    };
    ui.horizontal_centered(|ui| {
        if !radio_workspace {
            ui.label(
                egui::RichText::new("ez-sdr")
                    .strong()
                    .color(colors.text_dim.to_egui()),
            );
        }
        ui.separator();
        for (_, tab, _, label, tip) in MAIN_TABS.iter().copied() {
            let active = *current_tab == tab;
            let response = ui
                .add(
                    egui::Button::new(egui::RichText::new(label).size(13.0).color(if active {
                        colors.text_heading.to_egui()
                    } else {
                        colors.text_dim.to_egui()
                    }))
                    .selected(active)
                    .min_size(egui::vec2(74.0, 23.0)),
                )
                .on_hover_text(tip);
            if active {
                ui.painter().hline(
                    response.rect.x_range(),
                    response.rect.bottom(),
                    egui::Stroke::new(2.0, colors.accent.to_egui()),
                );
            }
            if response.clicked() {
                *current_tab = tab;
                if tab == AppTab::Planes {
                    adsb_panel.begin();
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("Tools", |ui| {
                if ui.button("Setup Guide").clicked() {
                    *show_setup_guide = true;
                    ui.close();
                }
                if ui.button("Settings").clicked() {
                    *show_settings = true;
                    ui.close();
                }
            });
            egui::warn_if_debug_build(ui);
        });
    });
}
