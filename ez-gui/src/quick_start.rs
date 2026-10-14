//! Quick Start wizard for first-run experience.
//!
//! Replaces the removed tutorial system with a guided workflow that helps
//! beginners configure their SDR for common tasks: FM radio, aircraft tracking,
//! weather satellites, etc.

use crate::app::SharedState;
use std::sync::{Arc, Mutex};

/// Quick Start wizard state.
#[derive(Debug, Clone, PartialEq)]
pub enum QuickStartState {
    /// Not active.
    Inactive,
    /// Welcome screen - explain what ez-sdr does.
    Welcome,
    /// Detect and select SDR device.
    DeviceSelection,
    /// Choose what you want to do.
    WorkflowSelection,
    /// Configure for selected workflow.
    WorkflowConfiguration(Workflow),
    /// Final success screen.
    Complete,
}

/// Common SDR workflows for beginners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workflow {
    /// Listen to FM radio stations.
    FmRadio,
    /// Track aircraft with ADS-B.
    Aircraft,
    /// Decode existing Meteor LRPT recordings offline.
    MeteorLrpt,
    /// Monitor amateur radio bands.
    HamRadio,
    /// Custom/advanced configuration.
    Custom,
}

impl Workflow {
    fn icon(&self) -> &'static str {
        match self {
            Workflow::FmRadio => "📻",
            Workflow::Aircraft => "✈️",
            Workflow::MeteorLrpt => "🛰️",
            Workflow::HamRadio => "📡",
            Workflow::Custom => "⚙️",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Workflow::FmRadio => "FM Radio",
            Workflow::Aircraft => "Track Aircraft",
            Workflow::MeteorLrpt => "Meteor Offline Decoder",
            Workflow::HamRadio => "Ham Radio",
            Workflow::Custom => "Custom Setup",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Workflow::FmRadio => "Listen to local FM radio stations (88-108 MHz)",
            Workflow::Aircraft => {
                "Track nearby aircraft using 1090ES ADS-B or native 978 MHz UAT IQ at 2,083,333 samples/s; dump978-fa JSON is optional"
            }
            Workflow::MeteorLrpt => "Decode an existing Meteor LRPT recording (.cs8 or .cf32)",
            Workflow::HamRadio => "Monitor amateur radio bands (HF/VHF/UHF)",
            Workflow::Custom => "I'll configure it myself",
        }
    }

    /// Configure a workflow and persist completion. Meteor leaves the source alone.
    pub fn apply(&self, shared: &Arc<Mutex<SharedState>>) -> Result<(), String> {
        self.apply_with_source_at(shared, None, std::path::Path::new("ez_sdr_config.json"))
    }

    fn apply_with_source_at(
        &self,
        shared: &Arc<Mutex<SharedState>>,
        selected_source: Option<crate::source_manager::SourceMode>,
        save_path: &std::path::Path,
    ) -> Result<(), String> {
        let mut state = shared.lock().map_err(|_| "Failed to lock state")?;
        self.configure_state(&mut state, selected_source);
        if state.config.save_with_feedback(save_path) {
            Ok(())
        } else {
            Err(state
                .config
                .save_error
                .clone()
                .unwrap_or_else(|| "Could not save workflow settings".into()))
        }
    }

    /// Apply in-memory setup separately from persistence so the offline contract
    /// can be tested without modifying the user's saved configuration.
    fn configure_state(
        &self,
        state: &mut SharedState,
        selected_source: Option<crate::source_manager::SourceMode>,
    ) {
        state.config.quick_start_completed = true;
        if *self == Workflow::MeteorLrpt {
            state.selected_satellite = None;
            return;
        }
        if let Some(source_mode) = selected_source {
            state.source.source_mode = source_mode;
        }

        match self {
            Workflow::FmRadio => {
                // Configure for FM broadcast band
                state.config.default_freq_hz = 98_500_000; // 98.5 MHz - common station
                state.config.default_sample_rate = 2_048_000; // 2.048 MSps
                state.config.default_gain = 40.0;
                state.demod_mode = crate::sdr_panel::DemodMode::Wfm;
                state.config.advanced.fft_size = 4096;
                state.volume = 0.5;
                crate::radio_ui::set_power_squelch_preference(state, -50.0);
            }
            Workflow::Aircraft => {
                // Configure for ADS-B (1090 MHz)
                state.config.default_freq_hz = 1_090_000_000;
                state.config.default_sample_rate = 2_400_000; // 2.4 MSps for Mode-S
                state.config.default_gain = 49.0; // Max gain for weak signals
                state.demod_mode = crate::sdr_panel::DemodMode::Raw;
                state.config.advanced.fft_size = 2048;
            }
            Workflow::MeteorLrpt => unreachable!("offline Meteor returns before SDR setup"),
            Workflow::HamRadio => {
                // Configure for 2m band (144-148 MHz)
                state.config.default_freq_hz = 146_520_000; // 2m calling frequency
                state.config.default_sample_rate = 2_048_000;
                state.config.default_gain = 40.0;
                state.demod_mode = crate::sdr_panel::DemodMode::Fm;
                state.config.advanced.fft_size = 4096;
                state.volume = 0.5;
                crate::radio_ui::set_power_squelch_preference(state, -40.0);
            }
            Workflow::Custom => {
                // Leave defaults, user will configure manually
            }
        }

        let should_start = state.source.status == crate::source_manager::SourceStatus::Running;
        state.source.stop();
        crate::radio_ui::tune(state, state.config.default_freq_hz);
        state.source.center_frequency_hz = Some(state.source.frequency_hz);
        state.source.sample_rate_hz = state.config.default_sample_rate;
        state.source.gain_db = state.config.default_gain;
        if should_start {
            state.source.start();
        }
    }
}

pub struct QuickStartWizard {
    pub state: QuickStartState,
    selected_source: Option<crate::source_manager::SourceMode>,
    selected_workflow: Option<Workflow>,
    apply_error: Option<String>,
}

impl Default for QuickStartWizard {
    fn default() -> Self {
        Self {
            state: QuickStartState::Inactive,
            selected_source: None,
            selected_workflow: None,
            apply_error: None,
        }
    }
}

impl QuickStartWizard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the wizard.
    pub fn start(&mut self) {
        self.apply_error = None;
        self.state = QuickStartState::Welcome;
    }

    /// Dismiss the wizard.
    pub fn dismiss(&mut self) {
        self.apply_error = None;
        self.state = QuickStartState::Inactive;
    }

    /// Check if wizard is active.
    pub fn is_active(&self) -> bool {
        self.state != QuickStartState::Inactive
    }

    /// Render the wizard UI as a full-screen overlay.
    pub fn ui(&mut self, ctx: &egui::Context, shared: &Arc<Mutex<SharedState>>) {
        if !self.is_active() {
            return;
        }

        egui::Window::new("Quick Start")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(500.0);
                self.render_content(ui, shared, std::path::Path::new("ez_sdr_config.json"));
            });
    }

    /// Render the wizard steps into an existing UI. Used by the Setup Guide
    /// utility window so the guided workflow lives inside that window instead
    /// of a nested pop-up.
    pub fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        shared: &Arc<Mutex<SharedState>>,
        save_path: &std::path::Path,
    ) {
        match &self.state {
            QuickStartState::Inactive => {}
            QuickStartState::Welcome => self.ui_welcome(ui),
            QuickStartState::DeviceSelection => self.ui_device_selection(ui, shared),
            QuickStartState::WorkflowSelection => self.ui_workflow_selection(ui),
            QuickStartState::WorkflowConfiguration(workflow) => {
                self.ui_workflow_config(ui, shared, *workflow, save_path)
            }
            QuickStartState::Complete => self.ui_complete(ui),
        }
    }

    fn ui_welcome(&mut self, ui: &mut egui::Ui) {
        ui.heading("Welcome to EZ-SDR!");
        ui.add_space(20.0);

        ui.label("EZ-SDR brings radio listening, aircraft tracking, and Meteor LRPT decoding into one application.");
        ui.add_space(10.0);
        ui.label("Let's get you started in just a few steps:");
        ui.add_space(20.0);

        ui.horizontal(|ui| {
            ui.label("1️⃣");
            ui.label("Choose a signal source");
        });
        ui.horizontal(|ui| {
            ui.label("2️⃣");
            ui.label("Choose what you want to do");
        });
        ui.horizontal(|ui| {
            ui.label("3️⃣");
            ui.label("Listen, track aircraft, or decode a recording");
        });

        ui.add_space(40.0);

        ui.horizontal(|ui| {
            if ui.button("Get Started").clicked() {
                self.state = QuickStartState::DeviceSelection;
            }
            ui.add_space(20.0);
            if ui.button("Decode Meteor recording…").clicked() {
                self.selected_workflow = Some(Workflow::MeteorLrpt);
                self.state = QuickStartState::WorkflowConfiguration(Workflow::MeteorLrpt);
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Skip - I know what I'm doing").clicked() {
                self.dismiss();
            }
        });
    }

    fn ui_device_selection(&mut self, ui: &mut egui::Ui, _shared: &Arc<Mutex<SharedState>>) {
        ui.heading("Choose Your Signal Source");
        ui.add_space(20.0);

        ui.label("Demo works without hardware. RTL-SDR appears only in builds that include it.");
        ui.add_space(10.0);

        if ui
            .selectable_label(
                self.selected_source == Some(crate::source_manager::SourceMode::Simulated),
                "Demo signals (no SDR required)",
            )
            .clicked()
        {
            self.selected_source = Some(crate::source_manager::SourceMode::Simulated);
        }
        if cfg!(feature = "rtlsdr")
            && ui
                .selectable_label(
                    self.selected_source == Some(crate::source_manager::SourceMode::Hardware),
                    "RTL-SDR (first connected device)",
                )
                .clicked()
        {
            self.selected_source = Some(crate::source_manager::SourceMode::Hardware);
        }

        ui.add_space(30.0);

        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.state = QuickStartState::Welcome;
            }
            ui.add_space(20.0);
            if ui
                .add_enabled(self.selected_source.is_some(), egui::Button::new("Next →"))
                .clicked()
            {
                self.state = QuickStartState::WorkflowSelection;
            }
        });
    }

    fn ui_workflow_selection(&mut self, ui: &mut egui::Ui) {
        ui.heading("What would you like to do?");
        ui.add_space(20.0);

        ui.label("Choose live reception or offline recording decoding:");
        ui.add_space(20.0);

        let workflows = [
            Workflow::FmRadio,
            Workflow::Aircraft,
            Workflow::MeteorLrpt,
            Workflow::HamRadio,
            Workflow::Custom,
        ];

        for workflow in &workflows {
            ui.add_space(5.0);

            let is_selected = self.selected_workflow == Some(*workflow);
            let mut frame = egui::Frame::group(ui.style());

            if is_selected {
                frame = frame.fill(ui.visuals().selection.bg_fill);
            }

            let response = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(workflow.icon()).size(24.0));
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(workflow.label()).strong());
                        ui.label(
                            egui::RichText::new(workflow.description())
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                });
            });

            if response.response.interact(egui::Sense::click()).clicked() {
                self.selected_workflow = Some(*workflow);
            }
        }

        ui.add_space(30.0);

        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.state = QuickStartState::DeviceSelection;
            }
            ui.add_space(20.0);
            if ui
                .add_enabled(
                    self.selected_workflow.is_some(),
                    egui::Button::new("Next →"),
                )
                .clicked()
            {
                if let Some(workflow) = self.selected_workflow {
                    self.apply_error = None;
                    self.state = QuickStartState::WorkflowConfiguration(workflow);
                }
            }
        });
    }

    fn ui_workflow_config(
        &mut self,
        ui: &mut egui::Ui,
        shared: &Arc<Mutex<SharedState>>,
        workflow: Workflow,
        save_path: &std::path::Path,
    ) {
        ui.heading(format!("Configure: {}", workflow.label()));
        ui.add_space(20.0);

        ui.label(format!("{} {}", workflow.icon(), workflow.description()));
        ui.add_space(20.0);

        // Show what will be configured
        ui.group(|ui| {
            ui.label(
                egui::RichText::new(if workflow == Workflow::MeteorLrpt {
                    "Offline decoding:"
                } else {
                    "Auto-configuration:"
                })
                .strong(),
            );
            ui.add_space(5.0);

            match workflow {
                Workflow::FmRadio => {
                    ui.label("• Frequency: 98.5 MHz (common FM station)");
                    ui.label("• Sample rate: 2.048 MSps");
                    ui.label("• Mode: Wide FM (WFM)");
                    ui.label("• Gain: 40 dB");
                }
                Workflow::Aircraft => {
                    ui.label("• 1090ES: 1090 MHz at 2.4 MS/s");
                    ui.label("• Native UAT: 978 MHz at exactly 2,083,333 samples/s");
                    ui.label("• Optional compatibility input: dump978-fa direct JSON feed");
                    ui.label("• Mode: RAW (for Mode-S decoding)");
                    ui.label("• Gain: 49 dB (max)");
                }
                Workflow::MeteorLrpt => {
                    ui.label("• Open an existing .cs8 or .cf32 IQ file in the Meteor tab");
                    ui.label("• Use the sample rate stored with your recording");
                    ui.label("• Choose the matching Meteor preset and decode the images");
                    ui.label("• Export decoded image channels as PNG files");
                    ui.label("• No SDR connection or antenna is needed");
                }
                Workflow::HamRadio => {
                    ui.label("• Frequency: 146.520 MHz (2m calling)");
                    ui.label("• Sample rate: 2.048 MSps");
                    ui.label("• Mode: Narrow FM (NFM)");
                    ui.label("• Gain: 40 dB");
                }
                Workflow::Custom => {
                    ui.label("• No auto-configuration");
                    ui.label("• You'll configure manually");
                }
            }
        });

        if let Some(error) = &self.apply_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.label("Your workflow settings remain available for this session. Fix the save destination or permissions, then retry.");
        }

        ui.add_space(30.0);

        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.apply_error = None;
                self.state = QuickStartState::WorkflowSelection;
            }
            ui.add_space(20.0);
            let apply_label = if self.apply_error.is_some() {
                "Retry Save & Finish →"
            } else if workflow == Workflow::MeteorLrpt {
                "Finish Offline Setup →"
            } else {
                "Apply Configuration →"
            };
            if ui.button(apply_label).clicked() {
                self.apply_workflow(shared, workflow, save_path);
            }
        });
    }

    fn apply_workflow(
        &mut self,
        shared: &Arc<Mutex<SharedState>>,
        workflow: Workflow,
        save_path: &std::path::Path,
    ) {
        match workflow.apply_with_source_at(shared, self.selected_source.clone(), save_path) {
            Ok(()) => {
                self.apply_error = None;
                self.state = QuickStartState::Complete;
            }
            Err(error) => self.apply_error = Some(error),
        }
    }

    fn ui_complete(&mut self, ui: &mut egui::Ui) {
        ui.heading("🎉 All Set!");
        ui.add_space(20.0);

        let offline_meteor = self.selected_workflow == Some(Workflow::MeteorLrpt);
        ui.label(if offline_meteor {
            "Meteor is ready to decode your existing recordings."
        } else {
            "Your source and workflow defaults have been configured."
        });
        ui.add_space(20.0);

        ui.label("Next steps:");
        ui.add_space(10.0);
        if offline_meteor {
            ui.label("• Open the Meteor tab and choose an existing .cs8 or .cf32 file");
            ui.label("• Set the recording's sample rate and matching Meteor preset");
            ui.label("• Decode, inspect the channel images, and export PNG files");
        } else {
            ui.label("• Click 'Start' if the source was not already running");
            ui.label("• Adjust the frequency if needed");
            ui.label("• Open Settings to adjust receiver and display options");
            ui.label("• Explore the spectrum waterfall");
        }

        ui.add_space(40.0);

        if ui.button("Start Using EZ-SDR").clicked() {
            self.dismiss();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meteor_setup_leaves_idle_running_and_daemon_sources_unchanged() {
        use crate::source_manager::{SourceMode, SourceStatus};

        for (mode, running) in [
            (SourceMode::Simulated, false),
            (SourceMode::Simulated, true),
            (SourceMode::Daemon, false),
            (SourceMode::Replay, false),
        ] {
            let shared = crate::test_helpers::make_shared_state();
            let mut state = shared.lock().unwrap();
            state.source.source_mode = mode.clone();
            state.source.frequency_hz = 118_500_000;
            state.source.center_frequency_hz = Some(118_000_000);
            state.source.sample_rate_hz = 192_000;
            state.source.gain_db = 22.0;
            state.source.frequency_offset_hz = -1_000_000;
            if running {
                state.source.start();
            } else if mode == SourceMode::Daemon {
                // Model an existing pending daemon connection without a socket.
                state.source.status = SourceStatus::Opening;
            }
            state.selected_satellite = Some("Meteor-M2-3".into());
            state.recording = true;
            state.audio_running = true;
            let status = state.source.status.clone();
            let generation = state.source.stream_generation();
            let defaults = (
                state.config.default_freq_hz,
                state.config.default_sample_rate,
                state.config.default_gain,
                state.demod_mode,
            );

            // The wizard may have selected Hardware earlier. Offline setup must
            // ignore it, with no config-file writes from this in-memory helper.
            Workflow::MeteorLrpt.configure_state(&mut state, Some(SourceMode::Hardware));

            assert_eq!(state.source.source_mode, mode);
            assert_eq!(state.source.status, status);
            assert_eq!(state.source.stream_generation(), generation);
            assert_eq!(state.source.frequency_hz, 118_500_000);
            assert_eq!(state.source.center_frequency_hz, Some(118_000_000));
            assert_eq!(state.source.sample_rate_hz, 192_000);
            assert_eq!(state.source.gain_db, 22.0);
            assert_eq!(state.source.frequency_offset_hz, -1_000_000);
            assert_eq!(
                (
                    state.config.default_freq_hz,
                    state.config.default_sample_rate,
                    state.config.default_gain,
                    state.demod_mode,
                ),
                defaults
            );
            assert!(state.selected_satellite.is_none());
            assert!(state.recording, "ordinary Radio recording must continue");
            assert!(state.audio_running);
            assert!(state.config.quick_start_completed);
            state.source.stop();
        }
    }

    #[test]
    fn workflow_labels_exist() {
        let workflows = [
            Workflow::FmRadio,
            Workflow::Aircraft,
            Workflow::MeteorLrpt,
            Workflow::HamRadio,
            Workflow::Custom,
        ];

        for workflow in &workflows {
            assert!(!workflow.label().is_empty());
            assert!(!workflow.description().is_empty());
            assert!(!workflow.icon().is_empty());
        }
    }

    #[test]
    fn aircraft_workflow_describes_native_uat_and_optional_json_compatibility() {
        let description = Workflow::Aircraft.description();
        assert!(description.contains("978 MHz"));
        assert!(description.contains("2,083,333 samples/s"));
        assert!(description.contains("dump978-fa JSON is optional"));
        assert_eq!(
            crate::adsb_panel::AdsbRegion::Uat978.frequency_hz(),
            978_000_000
        );
        assert_eq!(
            crate::adsb_panel::AdsbRegion::Uat978.sample_rate_hz(),
            2_083_333
        );
    }

    #[test]
    fn failed_workflow_save_stays_retryable_until_persistence_succeeds() {
        let shared = crate::test_helpers::make_shared_state();
        let directory = std::env::temp_dir().join(format!(
            "ez-quick-start-save-{}-{}",
            std::process::id(),
            ez_proto::new_capture_epoch()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::create_dir_all(&path).unwrap();

        let mut wizard = QuickStartWizard::new();
        wizard.state = QuickStartState::WorkflowConfiguration(Workflow::Aircraft);
        wizard.apply_workflow(&shared, Workflow::Aircraft, &path);
        assert!(matches!(
            wizard.state,
            QuickStartState::WorkflowConfiguration(Workflow::Aircraft)
        ));
        assert!(wizard
            .apply_error
            .as_deref()
            .is_some_and(|error| error.contains("Could not save settings")));
        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| wizard.render_content(ui, &shared, &path),
        );
        fn gather(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(shape) => text.push_str(shape.galley.text()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| gather(shape, text)),
                _ => {}
            }
        }
        let mut rendered = String::new();
        output
            .shapes
            .iter()
            .for_each(|shape| gather(&shape.shape, &mut rendered));
        assert!(rendered.contains("Could not save settings"), "{rendered}");
        assert!(rendered.contains("Retry Save & Finish"), "{rendered}");
        assert!(rendered.contains("Fix the save destination"), "{rendered}");
        {
            let state = shared.lock().unwrap();
            assert_eq!(state.config.default_freq_hz, 1_090_000_000);
            assert_eq!(
                state.source.status,
                crate::source_manager::SourceStatus::Idle
            );
            assert_eq!(state.source.stream_generation(), 0);
        }

        std::fs::remove_dir(&path).unwrap();
        wizard.apply_workflow(&shared, Workflow::Aircraft, &path);
        assert_eq!(wizard.state, QuickStartState::Complete);
        assert!(wizard.apply_error.is_none());
        let saved: crate::config::AppConfig =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.default_freq_hz, 1_090_000_000);
        assert!(saved.quick_start_completed);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn quick_start_state_transitions() {
        let mut wizard = QuickStartWizard::new();
        assert_eq!(wizard.state, QuickStartState::Inactive);

        wizard.start();
        assert_eq!(wizard.state, QuickStartState::Welcome);

        wizard.dismiss();
        assert_eq!(wizard.state, QuickStartState::Inactive);
    }
}
