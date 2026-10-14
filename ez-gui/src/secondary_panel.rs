//! Content for the Setup Guide and Settings utility popups.

use crate::app::SharedState;
use crate::quick_start::QuickStartWizard;
use std::sync::{Arc, Mutex};

pub fn render_setup_guide(
    quick_start: &mut QuickStartWizard,
    shared: &Arc<Mutex<SharedState>>,
    save_path: &std::path::Path,
    ui: &mut egui::Ui,
) {
    egui::ScrollArea::vertical()
        .id_salt("setup_guide_popup_scroll")
        .show(ui, |ui| {
            if !quick_start.is_active() {
                quick_start.start();
            }
            quick_start.render_content(ui, shared, save_path);
        });
}

pub fn render_settings(
    shared: &Arc<Mutex<SharedState>>,
    mqtt: &crate::mqtt::MqttPublisher,
    web_remote: &crate::web_remote::ListenerSnapshot,
    rigctl: &crate::web_remote::ListenerSnapshot,
    editor: &mut crate::config::SettingsEditor,
    config_path: &std::path::Path,
    ui: &mut egui::Ui,
) -> Option<crate::config::AppConfig> {
    if editor.draft().is_none() {
        if let Ok(state) = shared.try_lock() {
            editor.ensure_draft(&state.config);
        }
    }
    egui::ScrollArea::vertical()
        .id_salt("settings_popup_scroll")
        .show(ui, |ui| {
            let error_color = ui.visuals().error_fg_color;
            let normal_color = ui.visuals().text_color();
            let status_color = |status: &crate::web_remote::ListenerSnapshot| {
                if matches!(status.state, crate::web_remote::ListenerState::Failed(_)) {
                    error_color
                } else {
                    normal_color
                }
            };
            ui.horizontal(|ui| {
                ui.label("Web Remote actual status:");
                ui.colored_label(status_color(web_remote), web_remote.state.to_string());
            });
            ui.horizontal(|ui| {
                ui.label("rigctl actual status:");
                ui.colored_label(status_color(rigctl), rigctl.state.to_string());
            });
            ui.separator();
            let stats = mqtt.stats();
            ui.label(format!(
                "MQTT: {}",
                mqtt.connected_endpoint()
                    .unwrap_or_else(|| "disconnected".into())
            ));
            ui.label(format!(
                "Telemetry sets: {} · Submitted: {} · Pending: {}",
                stats.telemetry_sets, stats.submitted, stats.pending
            ));
            ui.label(format!(
                "Coalesced: {} · Backpressure: {}",
                stats.coalesced, stats.backpressure
            ));
            ui.label(format!(
                "Expired: {} · Dropped events: {} · Rejected: {}",
                stats.expired, stats.dropped_events, stats.rejected
            ));
            if let Some(seconds) = mqtt.reconnect_in_secs() {
                ui.label(format!("MQTT reconnect in {seconds}s"));
            }
            if let Some(error) = mqtt.last_error() {
                ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
            }
            ui.separator();
            let session_id = editor.session_id();
            let mut apply = None;
            if let Some(draft) = editor.draft_mut() {
                if draft.ui(ui, session_id, config_path) {
                    draft.normalize();
                    apply = Some(draft.clone());
                }
            }
            apply
        })
        .inner
}
