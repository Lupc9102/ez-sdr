//! Mode bar: top navigation bar containing task modes (Listen, Planes, Satellites),
//! progressive disclosure deck (⚙ More), and ambient AI toggle (🤖 Ask).

use crate::adsb_panel::AdsBPanel;
use crate::app::SharedState;
use crate::quick_start::QuickStartWizard;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Listen,
    Planes,
    Satellites,
}

pub const MAIN_TABS: &[(&str, AppTab, &str, &str, &str)] = &[
    (
        "listen",
        AppTab::Listen,
        "🎧",
        "Listen",
        "Listen — Spectrum, tuning, radio",
    ),
    (
        "planes",
        AppTab::Planes,
        "✈",
        "Planes",
        "Planes — Live aircraft tracking (ADS-B)",
    ),
    (
        "satellites",
        AppTab::Satellites,
        "🛰",
        "Satellites",
        "Satellites — Passes, Doppler, image decode",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryTool {
    Bookmarks,
    Scanner,
    Recorder,
    Scheduler,
    Settings,
    HowTo,
    Discord,
    Mqtt,
    WebRemote,
    Layout,
    Customize,
    Advanced,
}

impl SecondaryTool {
    pub fn icon(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "⭐",
            SecondaryTool::Scanner => "🔍",
            SecondaryTool::Recorder => "⏺",
            SecondaryTool::Scheduler => "🗓",
            SecondaryTool::Settings => "⚙",
            SecondaryTool::HowTo => "❓",
            SecondaryTool::Discord => "💬",
            SecondaryTool::Mqtt => "📡",
            SecondaryTool::WebRemote => "🌐",
            SecondaryTool::Layout => "🧩",
            SecondaryTool::Customize => "🎨",
            SecondaryTool::Advanced => "🧪",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "Bookmarks",
            SecondaryTool::Scanner => "Scanner",
            SecondaryTool::Recorder => "Recorder",
            SecondaryTool::Scheduler => "Scheduler",
            SecondaryTool::Settings => "Settings",
            SecondaryTool::HowTo => "How To",
            SecondaryTool::Discord => "Discord",
            SecondaryTool::Mqtt => "MQTT",
            SecondaryTool::WebRemote => "Web Remote",
            SecondaryTool::Layout => "Layout",
            SecondaryTool::Customize => "Customize",
            SecondaryTool::Advanced => "Advanced",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "bookmarks",
            SecondaryTool::Scanner => "scanner",
            SecondaryTool::Recorder => "recorder",
            SecondaryTool::Scheduler => "scheduler",
            SecondaryTool::Settings => "settings",
            SecondaryTool::HowTo => "howto",
            SecondaryTool::Discord => "discord",
            SecondaryTool::Mqtt => "mqtt",
            SecondaryTool::WebRemote => "webremote",
            SecondaryTool::Layout => "layout",
            SecondaryTool::Customize => "customize",
            SecondaryTool::Advanced => "advanced",
        }
    }

    /// Lowest [`crate::user_level::UserLevel`] at which this tool appears in the ⚙ More deck.
    pub fn min_level(&self) -> crate::user_level::UserLevel {
        use crate::user_level::UserLevel::*;
        match self {
            // Always available.
            SecondaryTool::Bookmarks
            | SecondaryTool::Settings
            | SecondaryTool::HowTo
            | SecondaryTool::Customize => Beginner,
            // Power tools.
            SecondaryTool::Scanner
            | SecondaryTool::Recorder
            | SecondaryTool::Scheduler
            | SecondaryTool::Advanced => Advanced,
            // Expert integrations.
            SecondaryTool::Discord
            | SecondaryTool::Mqtt
            | SecondaryTool::WebRemote
            | SecondaryTool::Layout => ClerkMaxwell,
        }
    }

    /// Whether this tool should be listed for a user at `level`.
    pub fn available_at(&self, level: crate::user_level::UserLevel) -> bool {
        (level as usize) >= (self.min_level() as usize)
    }
}

/// All secondary tools, in the fallback order used when a layout id is
/// missing from [`crate::config::LayoutConfig::secondary_tools`].
pub const ALL_SECONDARY_TOOLS: &[SecondaryTool] = &[
    SecondaryTool::Bookmarks,
    SecondaryTool::Scanner,
    SecondaryTool::Recorder,
    SecondaryTool::Scheduler,
    SecondaryTool::Discord,
    SecondaryTool::Mqtt,
    SecondaryTool::WebRemote,
    SecondaryTool::Layout,
    SecondaryTool::Settings,
    SecondaryTool::HowTo,
    SecondaryTool::Advanced,
];

/// Immediate-mode render function: one `&mut` per panel/widget by design.
/// Grouping these into a context struct would just move the field list.
#[allow(clippy::too_many_arguments)]
pub fn render_mode_bar(
    ui: &mut egui::Ui,
    shared: &Arc<Mutex<SharedState>>,
    current_tab: &mut AppTab,
    active_secondary_tool: &mut Option<SecondaryTool>,
    adsb_panel: &mut AdsBPanel,
    quick_start: &mut QuickStartWizard,
    show_keyboard_help: &mut bool,
    ai_ask_open: &mut bool,
) {
    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(10, 13, 18));

    let glow = shared
        .try_lock()
        .map(|state| state.config.theme_config.glow)
        .unwrap_or_default();

    ui.horizontal_centered(|ui| {
        ui.add_space(6.0);
        // ── Task modes ──────────────────────────────────────────────
        // Read from layout config; fall back to MAIN_TABS order for missing entries.
        let mut tabs_to_render = Vec::new();
        let mut rendered_ids = std::collections::HashSet::new();
        if let Ok(state) = shared.try_lock() {
            for item in &state.config.layout.main_tabs {
                if item.visible {
                    for (id, tab, icon, label, tip) in MAIN_TABS.iter().copied() {
                        if id == item.id {
                            tabs_to_render.push((tab, icon, label, tip));
                            rendered_ids.insert(id);
                            break;
                        }
                    }
                }
            }
        }
        // Fallback: append any tabs not in layout config.
        for (id, tab, icon, label, tip) in MAIN_TABS.iter().copied() {
            if !rendered_ids.contains(id) {
                tabs_to_render.push((tab, icon, label, tip));
            }
        }

        for (tab, icon, label, tip) in tabs_to_render {
            let is_active = *current_tab == tab && active_secondary_tool.is_none();
            let fg = if is_active {
                egui::Color32::from_rgb(0, 168, 255)
            } else {
                egui::Color32::from_rgb(160, 170, 180)
            };
            let bg = if is_active {
                egui::Color32::from_rgb(20, 26, 34)
            } else {
                egui::Color32::TRANSPARENT
            };
            let btn = egui::Button::new(
                egui::RichText::new(format!("{icon}  {label}"))
                    .color(fg)
                    .size(16.0),
            )
            .fill(bg)
            .min_size(egui::vec2(0.0, 34.0));
            let resp = ui.add(btn);
            if is_active {
                crate::fx::paint_glow(ui.painter(), resp.rect, 4.0, &glow);
            }
            if resp.on_hover_text(tip).clicked() {
                *current_tab = tab;
                *active_secondary_tool = None;
                if tab == AppTab::Planes {
                    adsb_panel.begin();
                }
            }
            ui.add_space(2.0);
        }

        // ── Ambient tools, pushed to the right edge ─────────────────
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // ⚙ More — deck of hidden tools, gated by user level.
            let level = shared
                .try_lock()
                .map(|s| crate::user_level::UserLevel::from_name(&s.config.user_level))
                .unwrap_or(crate::user_level::UserLevel::Beginner);
            ui.menu_button(egui::RichText::new("⚙ More").size(15.0), |ui| {
                ui.set_min_width(180.0);
                let mut chosen: Option<SecondaryTool> = None;

                // Build tools list from layout config; fall back to ALL_SECONDARY_TOOLS order.
                let mut tools_to_show = Vec::new();
                let mut rendered_tool_ids = std::collections::HashSet::new();
                if let Ok(state) = shared.try_lock() {
                    for item in &state.config.layout.secondary_tools {
                        if item.visible {
                            for tool in ALL_SECONDARY_TOOLS.iter().copied() {
                                if tool.id() == item.id {
                                    if tool.available_at(level) {
                                        tools_to_show.push(tool);
                                        rendered_tool_ids.insert(item.id.clone());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                // Fallback: append any tools not in layout config.
                for tool in ALL_SECONDARY_TOOLS.iter().copied() {
                    if !rendered_tool_ids.contains(tool.id()) && tool.available_at(level) {
                        tools_to_show.push(tool);
                    }
                }

                for tool in tools_to_show {
                    if ui
                        .button(format!("{}  {}", tool.icon(), tool.label()))
                        .clicked()
                    {
                        chosen = Some(tool);
                    }
                }
                ui.separator();
                if ui.button("🎨  Customize").clicked() {
                    chosen = Some(SecondaryTool::Customize);
                }
                if ui.button("🚀  Quick Start Wizard").clicked() {
                    quick_start.start();
                }
                if ui.button("⌨  Keyboard Shortcuts (?)").clicked() {
                    *show_keyboard_help = !*show_keyboard_help;
                }
                if let Some(tool) = chosen {
                    *active_secondary_tool = if *active_secondary_tool == Some(tool) {
                        None
                    } else {
                        Some(tool)
                    };
                }
            });

            // 🤖 Ask — ambient AI slide-over toggle.
            let ask_fg = if *ai_ask_open {
                egui::Color32::from_rgb(0, 168, 255)
            } else {
                egui::Color32::from_rgb(160, 170, 180)
            };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("🤖 Ask").color(ask_fg).size(15.0))
                        .fill(egui::Color32::TRANSPARENT),
                )
                .on_hover_text("Ask the AI assistant (available in every mode)")
                .clicked()
            {
                *ai_ask_open = !*ai_ask_open;
            }
        });
    });
}
