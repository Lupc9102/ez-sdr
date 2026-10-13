//! Content for the Setup Guide and Settings utility popups.

use crate::app::SharedState;
use crate::quick_start::QuickStartWizard;
use std::sync::{Arc, Mutex};

pub fn render_setup_guide(
    quick_start: &mut QuickStartWizard,
    shared: &Arc<Mutex<SharedState>>,
    ui: &mut egui::Ui,
) {
    egui::ScrollArea::vertical()
        .id_salt("setup_guide_popup_scroll")
        .show(ui, |ui| {
            if !quick_start.is_active() {
                quick_start.start();
            }
            quick_start.render_content(ui, shared);
        });
}

pub fn render_settings(
    shared: &Arc<Mutex<SharedState>>,
    mqtt: &crate::mqtt::MqttPublisher,
    ui: &mut egui::Ui,
) {
    egui::ScrollArea::vertical()
        .id_salt("settings_popup_scroll")
        .show(ui, |ui| {
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
            if let Ok(mut state) = shared.try_lock() {
                state.config.ui(ui);
            }
        });
}
