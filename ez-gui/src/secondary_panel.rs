//! Utility windows for Scanner, Recorder, AI, How-To, Setup Guide and Settings.
//! Each opens above the receiver workspace; lifecycle work runs in app logic.

use crate::ai_panel::AiPanel;
use crate::app::{SecondaryTool, SharedState};
use crate::howto_panel::HowToPanel;
use crate::quick_start::QuickStartWizard;
use crate::recorder_panel::RecorderPanel;
use crate::scanner::FrequencyScanner;
use crate::status_bar::StatusBar;
use std::sync::{Arc, Mutex};

pub struct SecondaryPanelContext<'a> {
    pub shared: &'a Arc<Mutex<SharedState>>,
    pub active_secondary_tool: &'a mut Option<SecondaryTool>,
    pub scanner: &'a mut FrequencyScanner,
    pub howto_panel: &'a mut HowToPanel,
    pub quick_start: &'a mut QuickStartWizard,
    pub status_bar: &'a mut StatusBar,
    pub ai_panel: &'a mut AiPanel,
    pub recorder_panel: &'a mut RecorderPanel,
    pub mqtt: &'a crate::mqtt::MqttPublisher,
}

pub fn render_secondary_panel(
    ctx: &mut SecondaryPanelContext<'_>,
    ui: &mut egui::Ui,
    tool: SecondaryTool,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {}", tool.icon(), tool.label()))
                .strong()
                .size(15.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Close window").clicked() {
                *ctx.active_secondary_tool = None;
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("secondary_panel_scroll")
        .show(ui, |ui| match tool {
            SecondaryTool::Recorder => ctx.recorder_panel.ui(ui),
            SecondaryTool::Ai => ctx.ai_panel.ui(ui),
            SecondaryTool::Settings => {
                let stats = ctx.mqtt.stats();
                ui.label(format!(
                    "MQTT: {}",
                    ctx.mqtt
                        .connected_endpoint()
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
                if let Some(seconds) = ctx.mqtt.reconnect_in_secs() {
                    ui.label(format!("MQTT reconnect in {seconds}s"));
                }
                if let Some(error) = ctx.mqtt.last_error() {
                    ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
                }
                ui.separator();
                if let Ok(mut state) = ctx.shared.try_lock() {
                    state.config.ui(ui);
                }
            }
            SecondaryTool::Scanner => {
                if let Ok(state) = ctx.shared.try_lock() {
                    ctx.scanner.spectrum_visible_range = Some((
                        state.spectrum.visible_left_hz,
                        state.spectrum.visible_right_hz,
                    ));
                }
                ctx.scanner.ui(ui);
                if let Some(prompt) = ctx.scanner.pending_ai_prompt.take() {
                    ctx.ai_panel.input = prompt;
                    ctx.ai_panel.send_message();
                    *ctx.active_secondary_tool = Some(SecondaryTool::Ai);
                    ctx.status_bar
                        .info("🤖 AI Agent opened for the scanner request".to_string());
                }
            }
            SecondaryTool::HowTo => ctx.howto_panel.ui(ui),
            SecondaryTool::SetupGuide => {
                if !ctx.quick_start.is_active() {
                    ctx.quick_start.start();
                }
                let shared = ctx.shared.clone();
                ctx.quick_start.render_content(ui, &shared);
            }
            _ => {}
        });
}
