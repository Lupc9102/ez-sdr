//! Utility window content for the curated Tools menu. Only the Scanner,
//! How-To, and Settings tools remain dispatchable; each opens in its own
//! secondary window above the receiver workspace.

use crate::ai_panel::AiPanel;
use crate::app::{SecondaryTool, SharedState};
use crate::howto_panel::HowToPanel;
use crate::scanner::FrequencyScanner;
use crate::status_bar::StatusBar;
use std::sync::{Arc, Mutex};

pub struct SecondaryPanelContext<'a> {
    pub shared: &'a Arc<Mutex<SharedState>>,
    pub active_secondary_tool: &'a mut Option<SecondaryTool>,
    pub scanner: &'a mut FrequencyScanner,
    pub howto_panel: &'a mut HowToPanel,
    pub status_bar: &'a mut StatusBar,
    pub ai_panel: &'a mut AiPanel,
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
            SecondaryTool::Settings => {
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
                    ctx.status_bar
                        .info("🤖 Scanner request sent to AI Agent".to_string());
                }
            }
            SecondaryTool::HowTo => ctx.howto_panel.ui(ui),
            _ => {}
        });
}
