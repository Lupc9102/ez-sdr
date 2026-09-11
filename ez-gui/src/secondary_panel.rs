//! Secondary drawer panel rendering (⚙ More tools): Bookmarks, Scheduler,
//! Advanced, Settings, Scanner, Recorder, HowTo, Discord, MQTT, WebRemote, Layout, Customize.

use crate::ai_panel::AiPanel;
use crate::app::{SecondaryTool, SharedSnapshot, SharedState};
use crate::bookmark_manager::BookmarkPanel;
use crate::customize_panel::CustomizePanel;
use crate::demod::Demodulator;
use crate::discord::DiscordNotifier;
use crate::discord_panel::DiscordPanel;
use crate::howto_panel::HowToPanel;
use crate::mqtt::MqttPublisher;
use crate::recorder_panel::RecorderPanel;
use crate::scanner::FrequencyScanner;
use crate::scheduler::SchedulerPanel;
use crate::status_bar::StatusBar;
use crate::web_remote::WebRemote;
use std::sync::{Arc, Mutex};

pub struct SecondaryPanelContext<'a> {
    pub shared: &'a Arc<Mutex<SharedState>>,
    pub active_secondary_tool: &'a mut Option<SecondaryTool>,
    pub bookmark_panel: &'a mut BookmarkPanel,
    pub scheduler_panel: &'a mut SchedulerPanel,
    pub demod: &'a mut Demodulator,
    pub scanner: &'a mut FrequencyScanner,
    pub ai_panel: &'a mut AiPanel,
    pub ai_ask_open: &'a mut bool,
    pub status_bar: &'a mut StatusBar,
    pub recorder_panel: &'a mut RecorderPanel,
    pub howto_panel: &'a mut HowToPanel,
    pub discord_panel: &'a mut DiscordPanel,
    pub discord: &'a mut DiscordNotifier,
    pub mqtt: &'a mut MqttPublisher,
    pub web_remote: &'a mut WebRemote,
    pub customize_panel: &'a mut CustomizePanel,
    pub last_manual_tune_time: &'a mut std::time::Instant,
    pub session_notes: &'a mut String,
}

pub fn render_secondary_panel(
    ctx: &mut SecondaryPanelContext<'_>,
    ui: &mut egui::Ui,
    tool: SecondaryTool,
    snapshot: &Option<SharedSnapshot>,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {}", tool.icon(), tool.label()))
                .strong()
                .size(15.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Close panel").clicked() {
                *ctx.active_secondary_tool = None;
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("secondary_panel_scroll")
        .show(ui, |ui| match tool {
            SecondaryTool::Bookmarks => render_bookmarks_full(ctx, ui),
            SecondaryTool::Scheduler => render_scheduler_full(ctx, ui, snapshot),
            SecondaryTool::Advanced => render_advanced(ctx, ui),
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
                    *ctx.ai_ask_open = true;
                    ctx.status_bar
                        .info("🤖 Scanner request sent to AI Agent".to_string());
                }
            }
            SecondaryTool::Recorder => ctx.recorder_panel.ui(ui),
            SecondaryTool::HowTo => ctx.howto_panel.ui(ui),
            SecondaryTool::Discord => {
                ctx.discord_panel.ui(ui, ctx.discord, ctx.shared);
            }
            SecondaryTool::Mqtt => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ui.label(egui::RichText::new("MQTT publishing").strong());
                    ui.label("Stream SDR / ADS-B / satellite state to an MQTT broker.");
                    ui.horizontal(|ui| {
                        ui.label("Broker:");
                        ui.text_edit_singleline(&mut state.config.mqtt_broker);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Topic prefix:");
                        ui.text_edit_singleline(&mut state.config.mqtt_topic_prefix);
                    });
                    let mut enabled = !state.config.mqtt_broker.is_empty() && state.mqtt_enabled;
                    if ui.checkbox(&mut enabled, "Enable MQTT").clicked() {
                        state.mqtt_enabled = enabled;
                        if enabled && state.config.mqtt_broker.is_empty() {
                            state.config.mqtt_broker = "localhost".to_string();
                        }
                        ctx.mqtt.set_enabled(
                            state.mqtt_enabled,
                            state.config.mqtt_broker.clone(),
                            state.config.mqtt_topic_prefix.clone(),
                        );
                    }
                    let status = if ctx.mqtt.is_connected() {
                        egui::RichText::new("● Connected").color(egui::Color32::GREEN)
                    } else if state.mqtt_enabled {
                        egui::RichText::new("● Connecting…").color(egui::Color32::YELLOW)
                    } else {
                        egui::RichText::new("○ Disabled").color(egui::Color32::GRAY)
                    };
                    ui.label(status);
                }
            }
            SecondaryTool::WebRemote => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ui.label(egui::RichText::new("Web Remote").strong());
                    ui.label("Control ez-sdr from a browser via the local web API.");
                    let cb = ui.checkbox(&mut state.config.web_remote_enabled, "Enable web remote");
                    let sl = ui.add(
                        egui::Slider::new(&mut state.config.web_remote_port, 1024..=65535)
                            .text("Port"),
                    );
                    if cb.changed() || sl.changed() {
                        ctx.web_remote.set_enabled(
                            state.config.web_remote_enabled,
                            state.config.web_remote_port,
                        );
                    }
                    let status = if state.config.web_remote_enabled {
                        egui::RichText::new(format!(
                            "● Listening on :{}",
                            state.config.web_remote_port
                        ))
                        .color(egui::Color32::GREEN)
                    } else {
                        egui::RichText::new("○ Disabled").color(egui::Color32::GRAY)
                    };
                    ui.label(status);
                }
            }
            SecondaryTool::Layout => {
                ui.label(egui::RichText::new("Layout").strong());
                ui.label("Reorder and show/hide the three task modes and tools.");
                ctx.customize_panel.subtab = crate::customize_panel::CustomizeSubTab::Layout;
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ctx.customize_panel.ui(ui, &mut state.config);
                }
            }
            SecondaryTool::Customize => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ctx.customize_panel.ui(ui, &mut state.config);
                }
            }
        });
}

pub fn render_bookmarks_full(ctx: &mut SecondaryPanelContext<'_>, ui: &mut egui::Ui) {
    ctx.bookmark_panel.ui(
        ui,
        ctx.shared,
        ctx.last_manual_tune_time,
        &mut ctx.ai_panel.input,
        ctx.status_bar,
    );
}

pub fn render_scheduler_full(
    ctx: &mut SecondaryPanelContext<'_>,
    ui: &mut egui::Ui,
    snapshot: &Option<SharedSnapshot>,
) {
    ctx.scheduler_panel.ui(ui, ctx.shared, snapshot);

    // Session notes — lightweight text scratchpad
    ui.separator();
    ui.collapsing("📝 Session Notes", |ui| {
        ui.label(
            egui::RichText::new(
                "Jot down frequencies, signal notes, or observations for this session.",
            )
            .small()
            .color(egui::Color32::GRAY),
        );
        ui.add(
            egui::TextEdit::multiline(ctx.session_notes)
                .desired_rows(6)
                .desired_width(f32::INFINITY)
                .hint_text(
                    "e.g. 'Strong signal at 145.500 MHz — probably a local repeater. Heard voice at 156.800 MHz marine ch16.'",
                ),
        );
        ui.horizontal(|ui| {
            if ui
                .small_button("💾 Save to file")
                .on_hover_text("Save session notes to a text file.")
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("sdr_session_notes.txt")
                    .add_filter("Text", &["txt"])
                    .save_file()
                {
                    let _ = std::fs::write(&path, &*ctx.session_notes);
                }
            }
            if ui
                .small_button("Clear")
                .on_hover_text("Clear all session notes.")
                .clicked()
            {
                ctx.session_notes.clear();
            }
        });
    });
}

pub fn render_advanced(ctx: &mut SecondaryPanelContext<'_>, ui: &mut egui::Ui) {
    let mut guard = match ctx.shared.try_lock() {
        Ok(s) => s,
        Err(_) => {
            ui.label("(settings temporarily busy)");
            return;
        }
    };
    crate::advanced_panel::render_advanced(ui, &mut guard, ctx.demod);
}
