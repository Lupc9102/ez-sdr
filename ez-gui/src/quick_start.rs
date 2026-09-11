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
    /// Receive weather satellite images (NOAA).
    WeatherSatellites,
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
            Workflow::WeatherSatellites => "🛰️",
            Workflow::HamRadio => "📡",
            Workflow::Custom => "⚙️",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Workflow::FmRadio => "FM Radio",
            Workflow::Aircraft => "Track Aircraft",
            Workflow::WeatherSatellites => "Weather Satellites",
            Workflow::HamRadio => "Ham Radio",
            Workflow::Custom => "Custom Setup",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Workflow::FmRadio => "Listen to local FM radio stations (88-108 MHz)",
            Workflow::Aircraft => "Track nearby aircraft using ADS-B (1090 MHz)",
            Workflow::WeatherSatellites => "Receive NOAA weather satellite images (137 MHz)",
            Workflow::HamRadio => "Monitor amateur radio bands (HF/VHF/UHF)",
            Workflow::Custom => "I'll configure it myself",
        }
    }

    /// Auto-configure the SDR for this workflow.
    pub fn apply(&self, shared: &Arc<Mutex<SharedState>>) -> Result<(), String> {
        let mut state = shared.lock().map_err(|_| "Failed to lock state")?;

        match self {
            Workflow::FmRadio => {
                // Configure for FM broadcast band
                state.config.default_freq_hz = 98_500_000; // 98.5 MHz - common station
                state.config.default_sample_rate = 2_048_000; // 2.048 MSps
                state.config.default_gain = 40.0;
                state.demod_mode = crate::sdr_panel::DemodMode::Wfm;
                state.config.advanced.fft_size = 4096;
                state.volume = 0.5;
                state.squelch = -50.0;
            }
            Workflow::Aircraft => {
                // Configure for ADS-B (1090 MHz)
                state.config.default_freq_hz = 1_090_000_000;
                state.config.default_sample_rate = 2_400_000; // 2.4 MSps for Mode-S
                state.config.default_gain = 49.0; // Max gain for weak signals
                state.demod_mode = crate::sdr_panel::DemodMode::Raw;
                state.config.advanced.fft_size = 2048;
            }
            Workflow::WeatherSatellites => {
                // Configure for NOAA APT satellites (137.x MHz)
                state.config.default_freq_hz = 137_620_000; // NOAA 18
                state.config.default_sample_rate = 2_048_000;
                state.config.default_gain = 40.0;
                state.demod_mode = crate::sdr_panel::DemodMode::Wfm;
                state.config.advanced.fft_size = 4096;
            }
            Workflow::HamRadio => {
                // Configure for 2m band (144-148 MHz)
                state.config.default_freq_hz = 146_520_000; // 2m calling frequency
                state.config.default_sample_rate = 2_048_000;
                state.config.default_gain = 40.0;
                state.demod_mode = crate::sdr_panel::DemodMode::Fm;
                state.config.advanced.fft_size = 4096;
                state.volume = 0.5;
                state.squelch = -40.0;
            }
            Workflow::Custom => {
                // Leave defaults, user will configure manually
            }
        }

        // Save config
        state.config.quick_start_completed = true;
        state.config.save();
        Ok(())
    }
}

pub struct QuickStartWizard {
    pub state: QuickStartState,
    detected_devices: Vec<String>,
    selected_device: Option<usize>,
    selected_workflow: Option<Workflow>,
}

impl Default for QuickStartWizard {
    fn default() -> Self {
        Self {
            state: QuickStartState::Inactive,
            detected_devices: Vec::new(),
            selected_device: None,
            selected_workflow: None,
        }
    }
}

impl QuickStartWizard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the wizard.
    pub fn start(&mut self) {
        self.state = QuickStartState::Welcome;
    }

    /// Dismiss the wizard.
    pub fn dismiss(&mut self) {
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

                match &self.state {
                    QuickStartState::Inactive => {}
                    QuickStartState::Welcome => self.ui_welcome(ui),
                    QuickStartState::DeviceSelection => self.ui_device_selection(ui, shared),
                    QuickStartState::WorkflowSelection => self.ui_workflow_selection(ui),
                    QuickStartState::WorkflowConfiguration(workflow) => {
                        self.ui_workflow_config(ui, shared, *workflow)
                    }
                    QuickStartState::Complete => self.ui_complete(ui),
                }
            });
    }

    fn ui_welcome(&mut self, ui: &mut egui::Ui) {
        ui.heading("Welcome to EZ-SDR!");
        ui.add_space(20.0);

        ui.label("EZ-SDR is an all-in-one software-defined radio application.");
        ui.add_space(10.0);
        ui.label("Let's get you started in just a few steps:");
        ui.add_space(20.0);

        ui.horizontal(|ui| {
            ui.label("1️⃣");
            ui.label("Detect your SDR device");
        });
        ui.horizontal(|ui| {
            ui.label("2️⃣");
            ui.label("Choose what you want to do");
        });
        ui.horizontal(|ui| {
            ui.label("3️⃣");
            ui.label("Start listening!");
        });

        ui.add_space(40.0);

        ui.horizontal(|ui| {
            if ui.button("Get Started").clicked() {
                self.state = QuickStartState::DeviceSelection;
            }
            ui.add_space(20.0);
            if ui.button("Skip - I know what I'm doing").clicked() {
                self.dismiss();
            }
        });
    }

    fn ui_device_selection(&mut self, ui: &mut egui::Ui, _shared: &Arc<Mutex<SharedState>>) {
        ui.heading("Select Your SDR Device");
        ui.add_space(20.0);

        // Detect devices on first render
        if self.detected_devices.is_empty() {
            // Try to enumerate SoapySDR devices
            self.detected_devices = vec![
                "RTL-SDR (auto-detect)".to_string(),
                "HackRF One".to_string(),
                "Airspy".to_string(),
                "File / Replay".to_string(),
            ];
        }

        ui.label("Select your SDR device from the list:");
        ui.add_space(10.0);

        for (idx, device) in self.detected_devices.iter().enumerate() {
            if ui
                .selectable_label(self.selected_device == Some(idx), device)
                .clicked()
            {
                self.selected_device = Some(idx);
            }
        }

        ui.add_space(30.0);

        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.state = QuickStartState::Welcome;
            }
            ui.add_space(20.0);
            if ui
                .add_enabled(self.selected_device.is_some(), egui::Button::new("Next →"))
                .clicked()
            {
                self.state = QuickStartState::WorkflowSelection;
            }
        });
    }

    fn ui_workflow_selection(&mut self, ui: &mut egui::Ui) {
        ui.heading("What would you like to do?");
        ui.add_space(20.0);

        ui.label("Choose a workflow to auto-configure your SDR:");
        ui.add_space(20.0);

        let workflows = [
            Workflow::FmRadio,
            Workflow::Aircraft,
            Workflow::WeatherSatellites,
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
    ) {
        ui.heading(format!("Configure: {}", workflow.label()));
        ui.add_space(20.0);

        ui.label(format!("{} {}", workflow.icon(), workflow.description()));
        ui.add_space(20.0);

        // Show what will be configured
        ui.group(|ui| {
            ui.label(egui::RichText::new("Auto-configuration:").strong());
            ui.add_space(5.0);

            match workflow {
                Workflow::FmRadio => {
                    ui.label("• Frequency: 98.5 MHz (common FM station)");
                    ui.label("• Sample rate: 2.048 MSps");
                    ui.label("• Mode: Wide FM (WFM)");
                    ui.label("• Gain: 40 dB");
                }
                Workflow::Aircraft => {
                    ui.label("• Frequency: 1090 MHz (ADS-B)");
                    ui.label("• Sample rate: 2.4 MSps");
                    ui.label("• Mode: RAW (for Mode-S decoding)");
                    ui.label("• Gain: 49 dB (max)");
                }
                Workflow::WeatherSatellites => {
                    ui.label("• Frequency: 137.620 MHz (NOAA 18)");
                    ui.label("• Sample rate: 2.048 MSps");
                    ui.label("• Mode: Wide FM (WFM)");
                    ui.label("• Gain: 40 dB");
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

        ui.add_space(30.0);

        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.state = QuickStartState::WorkflowSelection;
            }
            ui.add_space(20.0);
            if ui.button("Apply Configuration →").clicked() {
                // Apply the configuration
                if let Err(e) = workflow.apply(shared) {
                    eprintln!("Failed to apply workflow config: {}", e);
                }
                self.state = QuickStartState::Complete;
            }
        });
    }

    fn ui_complete(&mut self, ui: &mut egui::Ui) {
        ui.heading("🎉 All Set!");
        ui.add_space(20.0);

        ui.label("Your SDR has been configured and is ready to use.");
        ui.add_space(20.0);

        ui.label("Next steps:");
        ui.add_space(10.0);
        ui.label("• Click 'Start' to begin receiving");
        ui.label("• Adjust the frequency if needed");
        ui.label("• Check the '?' How To tab for detailed guides");
        ui.label("• Explore the spectrum waterfall");

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
    fn workflow_labels_exist() {
        let workflows = [
            Workflow::FmRadio,
            Workflow::Aircraft,
            Workflow::WeatherSatellites,
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
    fn quick_start_state_transitions() {
        let mut wizard = QuickStartWizard::new();
        assert_eq!(wizard.state, QuickStartState::Inactive);

        wizard.start();
        assert_eq!(wizard.state, QuickStartState::Welcome);

        wizard.dismiss();
        assert_eq!(wizard.state, QuickStartState::Inactive);
    }
}
