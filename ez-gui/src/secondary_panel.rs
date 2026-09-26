//! Secondary drawer panel rendering (⚙ More tools): Bookmarks, Scheduler,
//! Advanced, Settings, Scanner, Recorder, HowTo, Discord, MQTT, WebRemote, Layout, Customize.

use crate::ai_panel::AiPanel;
use crate::app::{SecondaryTool, SharedSnapshot, SharedState};
use crate::bookmark_manager::BookmarkPanel;
use crate::customize_panel::CustomizePanel;
use crate::demod::DemodWorker;
use crate::discord::DiscordNotifier;
use crate::discord_panel::DiscordPanel;
use crate::howto_panel::HowToPanel;
use crate::mqtt::MqttPublisher;
use crate::radio_ui::RadioUi;
use crate::recorder_panel::RecorderPanel;
use crate::rigctl::RigctlServer;
use crate::scanner::FrequencyScanner;
use crate::scheduler::SchedulerPanel;
use crate::status_bar::StatusBar;
use crate::web_remote::WebRemote;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct NotesExport {
    receiver: Option<crossbeam_channel::Receiver<Result<Option<std::path::PathBuf>, String>>>,
    message: String,
    failed: bool,
}

impl NotesExport {
    fn start(
        &mut self,
        context: egui::Context,
        notes: String,
        choose_path: impl FnOnce() -> Option<std::path::PathBuf> + Send + 'static,
    ) {
        if self.receiver.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.receiver = Some(rx);
        self.message = "Saving notes…".into();
        self.failed = false;
        std::thread::spawn(move || {
            let result = match choose_path() {
                Some(path) => std::fs::write(&path, notes)
                    .map(|()| Some(path.clone()))
                    .map_err(|error| {
                        format!("Could not save notes to {}: {error}", path.display())
                    }),
                None => Ok(None),
            };
            let _ = tx.send(result);
            context.request_repaint();
        });
    }

    fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Err("Notes export worker stopped unexpectedly.".into())
            }
        };
        self.receiver = None;
        self.failed = result.is_err();
        self.message = match result {
            Ok(Some(path)) => format!("Saved notes to {}", path.display()),
            Ok(None) => "Notes export cancelled.".into(),
            Err(error) => error,
        };
    }
}

pub struct SecondaryPanelContext<'a> {
    pub shared: &'a Arc<Mutex<SharedState>>,
    pub active_secondary_tool: &'a mut Option<SecondaryTool>,
    pub bookmark_panel: &'a mut BookmarkPanel,
    pub scheduler_panel: &'a mut SchedulerPanel,
    pub demod: &'a mut DemodWorker,
    pub scanner: &'a mut FrequencyScanner,
    pub ai_panel: &'a mut AiPanel,
    pub ai_ask_open: &'a mut bool,
    pub status_bar: &'a mut StatusBar,
    pub recorder_panel: &'a mut RecorderPanel,
    pub radio_ui: &'a mut RadioUi,
    pub howto_panel: &'a mut HowToPanel,
    pub discord_panel: &'a mut DiscordPanel,
    pub discord: &'a mut DiscordNotifier,
    pub mqtt: &'a mut MqttPublisher,
    pub web_remote: &'a mut WebRemote,
    pub rigctl: &'a mut RigctlServer,
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
            SecondaryTool::Sinks => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ctx.radio_ui.sinks_panel(ui, &mut state);
                }
            }
            SecondaryTool::BandPlan => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ctx.radio_ui.band_plan_panel(ui, &mut state);
                }
            }
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
            SecondaryTool::Rigctl => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ui.label(egui::RichText::new("Rigctl Server").strong());
                    ui.label(
                        "Expose a small Hamlib/rigctld-compatible control socket on localhost.",
                    );
                    let enabled = ui
                        .checkbox(&mut state.config.rigctl_enabled, "Enable loopback rigctl")
                        .changed();
                    let port = ui
                        .add(
                            egui::Slider::new(&mut state.config.rigctl_port, 1024..=65535)
                                .text("Port"),
                        )
                        .changed();
                    if enabled || port {
                        ctx.rigctl
                            .set_enabled(state.config.rigctl_enabled, state.config.rigctl_port);
                    }
                    if state.config.rigctl_enabled {
                        let bound = ctx.rigctl.bound_port();
                        if bound != 0 {
                            ui.colored_label(
                                egui::Color32::from_rgb(109, 190, 146),
                                format!("● Listening on 127.0.0.1:{bound}"),
                            );
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(216, 174, 85), "● Starting…");
                        }
                    } else {
                        ui.colored_label(ui.visuals().weak_text_color(), "○ Disabled");
                    }
                    ui.separator();
                    ui.label("Supported: f/F frequency, m/M mode, v/V AF volume, q quit.");
                }
            }
            SecondaryTool::Layout => {
                ui.label(egui::RichText::new("Layout").strong());
                ui.label("Reorder and show/hide the main tabs and tools.");
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
            SecondaryTool::FrequencyManager => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ui.label(egui::RichText::new("Frequency Manager").strong());
                    ui.label("Save, label, and recall frequencies.");
                    ui.separator();
                    let mut label = String::new();
                    ui.horizontal(|ui| {
                        ui.label("Label:");
                        ui.text_edit_singleline(&mut label);
                        if ui.button("Save current").clicked() {
                            let freq = state.source.frequency_hz;
                            state.config.freq_memory_hz.push(freq);
                            state.config.freq_memory_labels.push(label.clone());
                            label.clear();
                        }
                    });
                    ui.separator();
                    for i in (0..state.config.freq_memory_hz.len()).rev() {
                        let freq = state.config.freq_memory_hz[i];
                        let name = state.config.freq_memory_labels[i].clone();
                        ui.horizontal(|ui| {
                            if ui.button(&name).clicked() {
                                crate::radio_ui::tune(&mut state, freq);
                            }
                            if ui.button("🗑").clicked() {
                                state.config.freq_memory_hz.remove(i);
                                state.config.freq_memory_labels.remove(i);
                            }
                        });
                    }
                }
            }
            SecondaryTool::VfoColor => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ui.label(egui::RichText::new("VFO Color").strong());
                    ui.label("Customize the VFO A and B colors.");
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("VFO A:");
                        let mut a = state.config.theme_config.vfo_a_color.to_egui();
                        if ui.color_edit_button_srgba(&mut a).changed() {
                            let [r, g, b, al] = a.to_array();
                            state.config.theme_config.vfo_a_color =
                                crate::theme::Rgba::from_rgba(r, g, b, al);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("VFO B:");
                        let mut b = state.config.theme_config.vfo_b_color.to_egui();
                        if ui.color_edit_button_srgba(&mut b).changed() {
                            let [r, g, b, al] = b.to_array();
                            state.config.theme_config.vfo_b_color =
                                crate::theme::Rgba::from_rgba(r, g, b, al);
                        }
                    });
                }
            }
            SecondaryTool::Theme => {
                if let Ok(mut state) = ctx.shared.try_lock() {
                    ui.label(egui::RichText::new("Theme").strong());
                    ui.label("Switch between color themes.");
                    ui.separator();
                    for theme in ["dark", "light", "high_contrast", "solarized_dark", "nord"] {
                        if ui
                            .selectable_label(state.config.theme == theme, theme.replace('_', " "))
                            .clicked()
                        {
                            state.config.theme = theme.to_string();
                        }
                    }
                }
            }
            SecondaryTool::ModuleManager => {
                if let Ok(state) = ctx.shared.try_lock() {
                    ui.label(egui::RichText::new("Module Manager").strong());
                    ui.label("Active modules and their status.");
                    ui.separator();
                    let modules: Vec<(&str, &str)> = vec![
                        ("Radio", "active"),
                        ("ADS-B", if state.adsb_running { "active" } else { "idle" }),
                        (
                            "Meteor",
                            if state.source.source_mode == crate::source_manager::SourceMode::Replay
                            {
                                "active"
                            } else {
                                "idle"
                            },
                        ),
                        (
                            "Rigctl",
                            if state.config.rigctl_enabled {
                                "active"
                            } else {
                                "idle"
                            },
                        ),
                    ];
                    for (name, status) in modules {
                        ui.horizontal(|ui| {
                            ui.label(name);
                            ui.label(egui::RichText::new(status).color(if status == "active" {
                                egui::Color32::GREEN
                            } else {
                                egui::Color32::GRAY
                            }));
                        });
                    }
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
    let export_id = egui::Id::new("session_notes_export");
    let mut export = ui
        .ctx()
        .data_mut(|data| data.get_temp::<NotesExport>(export_id))
        .unwrap_or_default();
    export.poll();
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
                .add_enabled(export.receiver.is_none(), egui::Button::new("💾 Save to file").small())
                .on_hover_text("Save session notes to a text file.")
                .clicked()
            {
                export.start(ui.ctx().clone(), ctx.session_notes.clone(), || {
                    rfd::FileDialog::new().set_file_name("sdr_session_notes.txt")
                        .add_filter("Text", &["txt"]).save_file()
                });
            }
            if ui
                .small_button("Clear")
                .on_hover_text("Clear all session notes.")
                .clicked()
            {
                ctx.session_notes.clear();
            }
        });
        if !export.message.is_empty() {
            ui.colored_label(if export.failed { egui::Color32::RED } else { ui.visuals().text_color() }, &export.message);
        }
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(export_id, export));
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
