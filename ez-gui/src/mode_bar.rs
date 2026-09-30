//! Mode bar: top navigation bar containing the primary workspaces, a curated
//! utility menu, and the optional AI assistant popup.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryTool {
    Bookmarks,
    Scanner,
    Recorder,
    Sinks,
    BandPlan,
    Scheduler,
    Settings,
    HowTo,
    SetupGuide,
    Discord,
    Mqtt,
    WebRemote,
    Rigctl,
    Layout,
    Customize,
    Advanced,
    FrequencyManager,
    VfoColor,
    Theme,
    ModuleManager,
}

impl SecondaryTool {
    pub fn icon(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "⭐",
            SecondaryTool::Scanner => "🔍",
            SecondaryTool::Recorder => "⏺",
            SecondaryTool::Sinks => "🔊",
            SecondaryTool::BandPlan => "🗺",
            SecondaryTool::Scheduler => "🗓",
            SecondaryTool::Settings => "⚙",
            SecondaryTool::HowTo => "❓",
            SecondaryTool::SetupGuide => "🧭",
            SecondaryTool::Discord => "💬",
            SecondaryTool::Mqtt => "📡",
            SecondaryTool::WebRemote => "🌐",
            SecondaryTool::Rigctl => "📻",
            SecondaryTool::Layout => "🧩",
            SecondaryTool::Customize => "🎨",
            SecondaryTool::Advanced => "🧪",
            SecondaryTool::FrequencyManager => "📋",
            SecondaryTool::VfoColor => "🎨",
            SecondaryTool::Theme => "🎭",
            SecondaryTool::ModuleManager => "📦",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "Bookmarks",
            SecondaryTool::Scanner => "Scanner",
            SecondaryTool::Recorder => "Recorder",
            SecondaryTool::Sinks => "Sinks",
            SecondaryTool::BandPlan => "Band Plan",
            SecondaryTool::Scheduler => "Scheduler",
            SecondaryTool::Settings => "Settings",
            SecondaryTool::HowTo => "How To",
            SecondaryTool::SetupGuide => "Setup Guide",
            SecondaryTool::Discord => "Discord",
            SecondaryTool::Mqtt => "MQTT",
            SecondaryTool::WebRemote => "Web Remote",
            SecondaryTool::Rigctl => "Rigctl Server",
            SecondaryTool::Layout => "Layout",
            SecondaryTool::Customize => "Customize",
            SecondaryTool::Advanced => "Advanced",
            SecondaryTool::FrequencyManager => "Frequency Manager",
            SecondaryTool::VfoColor => "VFO Color",
            SecondaryTool::Theme => "Theme",
            SecondaryTool::ModuleManager => "Module Manager",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            SecondaryTool::Bookmarks => "bookmarks",
            SecondaryTool::Scanner => "scanner",
            SecondaryTool::Recorder => "recorder",
            SecondaryTool::Sinks => "sinks",
            SecondaryTool::BandPlan => "band_plan",
            SecondaryTool::Scheduler => "scheduler",
            SecondaryTool::Settings => "settings",
            SecondaryTool::HowTo => "howto",
            SecondaryTool::SetupGuide => "setup_guide",
            SecondaryTool::Discord => "discord",
            SecondaryTool::Mqtt => "mqtt",
            SecondaryTool::WebRemote => "webremote",
            SecondaryTool::Rigctl => "rigctl",
            SecondaryTool::Layout => "layout",
            SecondaryTool::Customize => "customize",
            SecondaryTool::Advanced => "advanced",
            SecondaryTool::FrequencyManager => "frequency_manager",
            SecondaryTool::VfoColor => "vfo_color",
            SecondaryTool::Theme => "theme",
            SecondaryTool::ModuleManager => "module_manager",
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
            | SecondaryTool::SetupGuide
            | SecondaryTool::Customize => Beginner,
            // Power tools.
            SecondaryTool::Scanner
            | SecondaryTool::Recorder
            | SecondaryTool::Sinks
            | SecondaryTool::BandPlan
            | SecondaryTool::Scheduler
            | SecondaryTool::Advanced => Advanced,
            // Expert integrations.
            SecondaryTool::Discord
            | SecondaryTool::Mqtt
            | SecondaryTool::WebRemote
            | SecondaryTool::Rigctl
            | SecondaryTool::Layout => ClerkMaxwell,
            SecondaryTool::FrequencyManager
            | SecondaryTool::VfoColor
            | SecondaryTool::Theme
            | SecondaryTool::ModuleManager => Advanced,
        }
    }

    /// Whether this tool should be listed for a user at `level`.
    pub fn available_at(&self, level: crate::user_level::UserLevel) -> bool {
        (level as usize) >= (self.min_level() as usize)
    }
}

/// Secondary tools that belong in the primary Tools menu.
///
/// The enum below intentionally retains the integration and legacy variants
/// so saved layouts and the secondary renderer stay backwards compatible, but
/// the main menu only exposes receiver workflow tools.  Integrations
/// (Discord/MQTT/Web Remote/Rigctl), duplicate views (Band Plan, Frequency
/// Manager, Layout), and cosmetic/status-only entries remain available to
/// existing configuration readers without adding another side menu.
pub const ALL_SECONDARY_TOOLS: &[SecondaryTool] = &[
    SecondaryTool::Scanner,
    SecondaryTool::HowTo,
    SecondaryTool::SetupGuide,
    SecondaryTool::Settings,
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
                *active_secondary_tool = None;
                if tab == AppTab::Planes {
                    adsb_panel.begin();
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("Tools", |ui| {
                for tool in ALL_SECONDARY_TOOLS.iter().copied() {
                    if ui.button(tool.label()).clicked() {
                        *active_secondary_tool = Some(tool);
                        ui.close();
                    }
                }
            });
            egui::warn_if_debug_build(ui);
        });
    });
}
