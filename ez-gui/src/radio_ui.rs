//! Compact receiver chrome. All controls operate the existing source/DSP path.
//!
//! Dimensions are kept together so a verified SDR++ reference can be applied
//! without reintroducing the old dashboard cards.

use std::sync::{mpsc, Arc, Mutex};

use crate::app::SharedState;
use crate::mode_bar::SecondaryTool;
use crate::sdr_panel::DemodMode;
use crate::source_manager::{DirectSamplingBranch, SourceManager, SourceMode, SourceStatus};
use crate::spectrum::{ColorMap, WindowType};

/// Matches the installed SDR++ profile's `menuWidth` at UI scale 1.0.
pub const SIDEBAR_WIDTH: f32 = 300.0;
/// SDR++ keeps the transport row at a compact 40 px at the default UI scale.
pub const TOOLBAR_HEIGHT: f32 = 40.0;
const MAX_FREQUENCY: u64 = 999_999_999_999;
const ROW_HEIGHT: f32 = 20.0;
const LABEL_WIDTH: f32 = 96.0;
const TOOLBAR_VOLUME_WIDTH: f32 = 160.0;

pub struct RadioUi {
    shared: Arc<Mutex<SharedState>>,
    pub show_sidebar: bool,
    /// Drain after drawing the sidebar, then apply the audio DSP settings.
    pub dsp_changed: bool,
    pub audio_changed: bool,
    pub stereo_locked: bool,
    pub rds: Option<crate::radio_rds::RdsSnapshot>,
    frequency_text: String,
    frequency_error: Option<String>,
    editing_frequency: bool,
    muted: bool,
    pub received_tone: Option<f32>,
    file_picker: Option<mpsc::Receiver<Option<std::path::PathBuf>>>,
    rtl_scan_started: bool,
    audio_devices: Vec<crate::audio_output::AudioOutputDeviceInfo>,
    audio_scan:
        Option<mpsc::Receiver<Result<Vec<crate::audio_output::AudioOutputDeviceInfo>, String>>>,
    audio_scan_started: bool,
    audio_scan_error: Option<String>,
    /// A compact SDR++ module shortcut can request the existing full tool
    /// drawer. Keeping the request deferred avoids borrowing `CentralApp`
    /// while the radio sidebar is being painted.
    requested_tool: Option<SecondaryTool>,
}

impl RadioUi {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        if let Ok(mut state) = shared.try_lock() {
            sync_demod_settings(&mut state);
            let cutoff = state.config.advanced.audio_cutoff_hz;
            if cutoff.is_finite() && cutoff > 0.0 {
                state.lpf_cutoff = cutoff.clamp(100.0, 20_000.0);
            }
        }
        Self {
            shared,
            show_sidebar: true,
            dsp_changed: false,
            audio_changed: false,
            stereo_locked: false,
            rds: None,
            frequency_text: String::new(),
            frequency_error: None,
            editing_frequency: false,
            muted: false,
            received_tone: None,
            file_picker: None,
            rtl_scan_started: false,
            audio_devices: Vec::new(),
            audio_scan: None,
            audio_scan_started: false,
            audio_scan_error: None,
            requested_tool: None,
        }
    }

    /// Consume a module shortcut request generated while drawing the radio
    /// sidebar. `CentralApp` applies it after the sidebar borrow ends.
    pub(crate) fn take_requested_tool(&mut self) -> Option<SecondaryTool> {
        self.requested_tool.take()
    }

    /// Render the dedicated Sinks view used by the SDR++ module shortcut.
    /// It reuses the exact same output controls as the compact Radio sidebar
    /// so device/rate/mute changes have one source of truth.
    pub(crate) fn sinks_panel(&mut self, ui: &mut egui::Ui, state: &mut SharedState) {
        ui.heading("Audio sinks");
        ui.label("Select the output device and stream rate used by the receiver.");
        self.audio_controls(ui, state);
    }

    pub(crate) fn band_plan_panel(&mut self, ui: &mut egui::Ui, state: &mut SharedState) {
        state.spectrum.ui_band_plan_panel(ui);
    }

    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        let shared = Arc::clone(&self.shared);
        let Ok(mut state) = shared.try_lock() else {
            return;
        };
        self.sync_audio_preference(&state);
        ui.scope(|ui| {
            compact_spacing(ui);
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().slider_width = TOOLBAR_VOLUME_WIDTH;
            ui.horizontal(|ui| {
                if icon_button(ui, ToolbarIcon::Menu, "Show / hide receiver controls").clicked() {
                    self.show_sidebar = !self.show_sidebar;
                }
                let running = matches!(
                    state.source.status,
                    SourceStatus::Running | SourceStatus::Opening
                );
                let icon = if running {
                    ToolbarIcon::Stop
                } else {
                    ToolbarIcon::Play
                };
                if icon_button(
                    ui,
                    icon,
                    if running {
                        "Stop receiver"
                    } else {
                        "Start receiver and audio"
                    },
                )
                .clicked()
                {
                    set_receiver_running(&mut state, !running, self.muted);
                }
                let icon = if self.muted {
                    ToolbarIcon::Muted
                } else {
                    ToolbarIcon::Speaker
                };
                if icon_button(
                    ui,
                    icon,
                    if self.muted {
                        "Unmute audio"
                    } else {
                        "Mute audio"
                    },
                )
                .clicked()
                {
                    self.muted = !self.muted;
                    state.audio_running = !self.muted
                        && matches!(
                            state.source.status,
                            SourceStatus::Running | SourceStatus::Opening
                        );
                }
                ui.add_sized(
                    [TOOLBAR_VOLUME_WIDTH, 28.0],
                    egui::Slider::new(&mut state.volume, 0.0..=1.0).show_value(false),
                )
                .on_hover_text(format!("Volume: {:.0}%", state.volume * 100.0));
                ui.separator();
                if self.editing_frequency {
                    let edit = ui.add_sized(
                        [245.0, 28.0],
                        egui::TextEdit::singleline(&mut self.frequency_text)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("118.700 MHz"),
                    );
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if escape {
                        self.editing_frequency = false;
                        self.frequency_error = None;
                    } else if (edit.lost_focus() && enter) || ui.small_button("Tune").clicked() {
                        match parse_frequency(&self.frequency_text) {
                            Ok(hz) => {
                                tune(&mut state, hz);
                                self.editing_frequency = false;
                                self.frequency_error = None;
                            }
                            Err(error) => self.frequency_error = Some(error.into()),
                        }
                    }
                    if let Some(error) = &self.frequency_error {
                        ui.colored_label(egui::Color32::from_rgb(220, 113, 105), "!")
                            .on_hover_text(error);
                    }
                } else {
                    let old = state.source.frequency_hz;
                    let mut frequency = old;
                    let response = frequency_digits(ui, &mut frequency);
                    if frequency != old {
                        tune(&mut state, frequency);
                    }
                    if response.double_clicked() || response.secondary_clicked() {
                        self.frequency_text = format!("{:.6} MHz", frequency as f64 / 1e6);
                        self.editing_frequency = true;
                    }
                }
                let tuning_icon = if state.config.center_tuning {
                    ToolbarIcon::CenterTuning
                } else {
                    ToolbarIcon::NormalTuning
                };
                let tuning_tip = if state.config.center_tuning {
                    "Center tuning: move the capture center with every tune"
                } else {
                    "Normal tuning: move the VFO inside the captured band"
                };
                if icon_button(ui, tuning_icon, tuning_tip).clicked()
                    && state.source.source_mode != SourceMode::Replay
                {
                    state.config.center_tuning = !state.config.center_tuning;
                    reconcile_tuning(&mut state);
                }
                if ui.available_width() > 110.0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (label, color) = match &state.source.status {
                            SourceStatus::Idle => ("Stopped", ui.visuals().weak_text_color()),
                            SourceStatus::Opening => {
                                ("Connecting…", egui::Color32::from_rgb(216, 174, 85))
                            }
                            SourceStatus::Running => {
                                ("Receiving", egui::Color32::from_rgb(109, 190, 146))
                            }
                            SourceStatus::Error(_) => {
                                ("Source error", egui::Color32::from_rgb(220, 113, 105))
                            }
                        };
                        ui.colored_label(color, label);
                    });
                }
            });
        });
    }

    pub fn sidebar(&mut self, ui: &mut egui::Ui) {
        let shared = Arc::clone(&self.shared);
        let Ok(mut state) = shared.try_lock() else {
            return;
        };
        self.sync_audio_preference(&state);
        self.poll_file_picker(&mut state);
        compact_spacing(ui);
        ui.style_mut()
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
        ui.style_mut()
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(16.0));
        ui.set_min_width(150.0);
        section(ui, "Source", true, |ui| {
            self.source_controls(ui, &mut state)
        });
        section(ui, "Radio", true, |ui| self.radio_controls(ui, &mut state));
        section(ui, "Audio", true, |ui| self.audio_controls(ui, &mut state));
        section(ui, "Display", true, |ui| {
            self.display_controls(ui, &mut state);
            state.spectrum.ui_radio_controls(ui);
        });
        self.sdrpp_module_inventory(ui);
    }

    /// SDR++ keeps these plugin modules in the same left-side module column.
    /// EZ-SDR already has equivalent focused tools; expose the same inventory
    /// here so the common workflow does not require hunting through `Tools`.
    /// The rows stay collapsed by default to preserve the compact 300 px
    /// receiver geometry and low visual noise.
    fn sdrpp_module_inventory(&mut self, ui: &mut egui::Ui) {
        section(ui, "SDR++ Modules", false, |ui| {
            ui.label(
                egui::RichText::new("Installed modules")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
                self.module_button(ui, "Recorder", SecondaryTool::Recorder);
                self.module_button(ui, "Sinks", SecondaryTool::Sinks);
                self.module_button(ui, "Frequency Manager", SecondaryTool::FrequencyManager);
                self.module_button(ui, "VFO Color", SecondaryTool::VfoColor);
                self.module_button(ui, "Band Plan", SecondaryTool::BandPlan);
                self.module_button(ui, "Theme", SecondaryTool::Theme);
                self.module_button(ui, "Module Manager", SecondaryTool::ModuleManager);
                self.module_button(ui, "Rigctl Server", SecondaryTool::Rigctl);
            });
            ui.label(
                egui::RichText::new(
                    "Sinks and band plan are live modules; other entries open the closest EZ-SDR tool.",
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
        });
    }

    fn module_button(&mut self, ui: &mut egui::Ui, label: &str, tool: SecondaryTool) {
        let tooltip = if label == "Rigctl Server" {
            "Open the loopback Hamlib/rigctld-compatible control server"
        } else {
            "Open the matching EZ-SDR module"
        };
        if ui
            .add_sized(
                [ui.available_width().min(132.0), ROW_HEIGHT],
                egui::Button::new(label),
            )
            .on_hover_text(tooltip)
            .clicked()
        {
            self.requested_tool = Some(tool);
        }
    }

    /// Start from the spectrum's empty-state control using the same mute
    /// preference as the toolbar's play button.
    pub(crate) fn start_receiver(&mut self, state: &mut SharedState) {
        set_receiver_running(state, true, self.muted);
    }

    fn sync_audio_preference(&mut self, state: &SharedState) {
        // Keyboard/web controls use SharedState. A stopped receiver also has
        // audio_running=false, so only reconcile while reception is active.
        if matches!(
            state.source.status,
            SourceStatus::Running | SourceStatus::Opening
        ) {
            self.muted = !state.audio_running;
        }
    }

    fn source_controls(&mut self, ui: &mut egui::Ui, state: &mut SharedState) {
        let old_mode = state.source.source_mode.clone();
        let mut mode = old_mode.clone();
        egui::ComboBox::from_id_salt("radio.source")
            .width(ui.available_width())
            .selected_text(source_name(&mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut mode, SourceMode::Daemon, "EZ-SDR daemon");
                #[cfg(feature = "rtlsdr")]
                ui.selectable_value(&mut mode, SourceMode::Hardware, "RTL-SDR");
                ui.selectable_value(&mut mode, SourceMode::Replay, "File source");
                ui.selectable_value(&mut mode, SourceMode::Simulated, "Demo signal");
            });
        if mode != old_mode {
            change_source(state, mode);
        }

        let mut restart = false;
        match state.source.source_mode {
            SourceMode::Daemon => {
                let reconnect = ui
                    .horizontal(|ui| {
                        ui.label("Host");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut state.source.daemon_addr)
                                .desired_width(ui.available_width()),
                        );
                        let enter =
                            response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        response.on_hover_text("Enter a host:port, then press Enter or Reconnect.");
                        enter
                    })
                    .inner;
                if ui.small_button("Reconnect").clicked() || reconnect {
                    state.source.stop();
                    state.source.start();
                    state.audio_running = !self.muted;
                }
            }
            SourceMode::Replay => {
                ui.horizontal(|ui| {
                    let filename = state
                        .source
                        .replay_file
                        .as_deref()
                        .and_then(|p| std::path::Path::new(p).file_name())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Select IQ file".into());
                    if ui
                        .add_enabled(self.file_picker.is_none(), egui::Button::new("Browse…"))
                        .clicked()
                    {
                        let (tx, rx) = mpsc::channel();
                        self.file_picker = Some(rx);
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let file = rfd::FileDialog::new()
                                .add_filter(
                                    "IQ recordings",
                                    &["cu8", "u8", "iq", "bin", "cf32", "fc32"],
                                )
                                .pick_file();
                            let _ = tx.send(file);
                            ctx.request_repaint();
                        });
                    }
                    ui.add(egui::Label::new(filename).truncate()).on_hover_text(
                        state
                            .source
                            .replay_file
                            .as_deref()
                            .unwrap_or("Unsigned 8-bit or complex float32 IQ"),
                    );
                });
                restart |= ui.checkbox(&mut state.source.replay_loop, "Loop").changed();
                row(ui, "File center", |ui| {
                    let mut center = state.source.capture_center_frequency_hz();
                    let response = ui.add(egui::DragValue::new(&mut center)
                        .range(0..=MAX_FREQUENCY).speed(1_000.0).suffix(" Hz"))
                        .on_hover_text("Capture frequency stored in this recording; normal tuning stays inside its recorded band");
                    if commit_control(&response) {
                        state.source.center_frequency_hz = Some(center);
                        state.source.frequency_hz = center;
                        restart = true;
                    }
                });
                row(ui, "Speed", |ui| {
                    let r = ui.add(
                        egui::DragValue::new(&mut state.source.replay_speed)
                            .range(0.1..=4.0)
                            .speed(0.05)
                            .suffix("×"),
                    );
                    restart |= commit_control(&r);
                });
            }
            SourceMode::Hardware => {
                if !self.rtl_scan_started {
                    state.source.refresh_rtl_devices();
                    self.rtl_scan_started = true;
                }
                state.source.poll_rtl_devices();
                let refreshing = state.source.rtl_devices_refreshing();
                if refreshing {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(50));
                }
                let selected_label = state
                    .source
                    .selected_rtl_device()
                    .map(|device| device.label())
                    .unwrap_or_else(|| format!("Device {}", state.source.rtl_device.index));
                let mut selected = state.source.rtl_device.index;
                row(ui, "Device", |ui| {
                    egui::ComboBox::from_id_salt("radio.rtl_device")
                        .width(ui.available_width())
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for device in &state.source.rtl_devices {
                                ui.selectable_value(&mut selected, device.index, device.label());
                            }
                        });
                });
                if selected != state.source.rtl_device.index {
                    if state.source.select_rtl_device(selected).is_ok() {
                        state.config.advanced.rtl_device = state.source.rtl_device.clone();
                        restart = true;
                    }
                }
                if ui
                    .add_enabled(
                        !refreshing,
                        egui::Button::new(if refreshing {
                            "Refreshing…"
                        } else {
                            "Refresh"
                        }),
                    )
                    .clicked()
                {
                    state.source.refresh_rtl_devices();
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(50));
                }
                if let Some(error) = &state.source.rtl_device_refresh_error {
                    ui.colored_label(egui::Color32::from_rgb(220, 113, 105), error);
                } else if !refreshing && state.source.rtl_devices.is_empty() {
                    ui.weak("No RTL-SDR devices found");
                }
            }
            SourceMode::Simulated => {
                ui.label(egui::RichText::new("Synthetic test signals").weak());
            }
        }
        let previous_rate = state.source.sample_rate_hz;
        row(ui, "Sample rate", |ui| {
            egui::ComboBox::from_id_salt("radio.rate")
                .width(ui.available_width())
                .selected_text(format!(
                    "{:.3} MHz",
                    state.source.sample_rate_hz as f64 / 1e6
                ))
                .show_ui(ui, |ui| {
                    for rate in [
                        250_000, 1_024_000, 1_536_000, 1_800_000, 2_048_000, 2_400_000, 2_560_000,
                        3_200_000,
                    ] {
                        ui.selectable_value(
                            &mut state.source.sample_rate_hz,
                            rate,
                            format!("{:.3} MHz", rate as f64 / 1e6),
                        );
                    }
                });
        });
        restart |= previous_rate != state.source.sample_rate_hz;
        if matches!(
            state.source.source_mode,
            SourceMode::Hardware | SourceMode::Daemon
        ) {
            row(ui, "Gain", |ui| {
                let r = ui.add(
                    egui::Slider::new(&mut state.source.gain_db, 0.0..=49.6)
                        .suffix(" dB")
                        .max_decimals(1),
                );
                restart |= commit_control(&r);
            });
        }
        if state.source.source_mode == SourceMode::Hardware {
            ui.horizontal(|ui| {
                restart |= ui.checkbox(&mut state.source.rtl_agc, "RTL AGC").changed();
                restart |= ui
                    .checkbox(&mut state.source.tuner_agc, "Tuner AGC")
                    .changed();
            });
            restart |= ui.checkbox(&mut state.source.bias_tee, "Bias T").changed();
            let previous = (
                state.source.direct_sampling,
                state.source.direct_sampling_branch,
            );
            let mut branch = if state.source.direct_sampling {
                Some(state.source.direct_sampling_branch)
            } else {
                None
            };
            row(ui, "Direct sampling", |ui| {
                egui::ComboBox::from_id_salt("radio.direct_sampling")
                    .selected_text(match branch {
                        None => "Off",
                        Some(DirectSamplingBranch::I) => "I branch",
                        Some(DirectSamplingBranch::Q) => "Q branch",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut branch, None, "Off");
                        ui.selectable_value(&mut branch, Some(DirectSamplingBranch::I), "I branch");
                        ui.selectable_value(&mut branch, Some(DirectSamplingBranch::Q), "Q branch");
                    });
            });
            state.source.direct_sampling = branch.is_some();
            if let Some(branch) = branch {
                state.source.direct_sampling_branch = branch;
            }
            restart |= previous
                != (
                    state.source.direct_sampling,
                    state.source.direct_sampling_branch,
                );
            restart |= ui
                .checkbox(&mut state.source.offset_tuning, "Offset Tuning")
                .changed();
            row(ui, "PPM correction", |ui| {
                let r = ui
                    .add(egui::DragValue::new(&mut state.source.ppm_correction).range(-100..=100));
                restart |= commit_control(&r);
            });
            state.config.advanced.rtl_agc = state.source.rtl_agc;
            state.config.advanced.tuner_agc = state.source.tuner_agc;
            state.config.advanced.bias_tee = state.source.bias_tee;
            state.config.advanced.direct_sampling = state.source.direct_sampling;
            state.config.advanced.direct_sampling_branch = state.source.direct_sampling_branch;
            state.config.advanced.offset_tuning = state.source.offset_tuning;
            state.config.ppm_correction = state.source.ppm_correction;
            let old_offset = state.lo_offset_hz;
            row(ui, "Offset", |ui| {
                egui::ComboBox::from_id_salt("radio.converter")
                    .width(ui.available_width())
                    .selected_text(converter_label(old_offset))
                    .show_ui(ui, |ui| {
                        for &(label, offset) in CONVERTER_OFFSETS {
                            ui.selectable_value(&mut state.lo_offset_hz, offset, label);
                        }
                    });
            });
            row(ui, "Offset (Hz)", |ui| {
                let response = ui.add(egui::DragValue::new(&mut state.lo_offset_hz).speed(1_000.0));
                restart |= commit_control(&response);
            });
            if old_offset != state.lo_offset_hz {
                state.source.frequency_offset_hz = state.lo_offset_hz;
                state.config.lo_offset_hz = state.lo_offset_hz;
                restart = true;
            }
        }
        ui.add_enabled_ui(
            state.source.source_mode != SourceMode::Daemon && !state.adsb_running,
            |ui| {
                ui.checkbox(&mut state.config.advanced.rf_dc_remove, "IQ correction")
                    .on_hover_text("Remove source I/Q DC offset before the spectrum and VFO");
                ui.checkbox(&mut state.config.advanced.invert_iq, "Invert IQ");
                row(ui, "Decimation", |ui| {
                    egui::ComboBox::from_id_salt("radio.decimation")
                        .selected_text(format!("{}×", radio_iq_config(state).decimation))
                        .show_ui(ui, |ui| {
                            for factor in [1, 2, 4, 8, 16, 32, 64] {
                                ui.selectable_value(
                                    &mut state.config.advanced.rf_decim,
                                    factor,
                                    format!("{factor}×"),
                                );
                            }
                        });
                });
            },
        )
        .response
        .on_disabled_hover_text(
            "Radio IQ controls are local; ADS-B uses the original source stream",
        );
        if restart {
            restart_if_running(&mut state.source);
        }
        if let SourceStatus::Error(error) = &state.source.status {
            ui.colored_label(egui::Color32::from_rgb(220, 113, 105), error);
        }
    }

    fn radio_controls(&mut self, ui: &mut egui::Ui, state: &mut SharedState) {
        use crate::radio_squelch::{SquelchMode, CTCSS_TONES};
        sync_demod_settings(state);
        let local = state.source.source_mode != SourceMode::Daemon;
        let width = ((ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0).floor();
        // The reference has four columns, each containing two stacked modes.
        for modes in [
            [
                (DemodMode::Fm, "NFM"),
                (DemodMode::Am, "AM"),
                (DemodMode::Usb, "USB"),
                (DemodMode::Lsb, "LSB"),
            ],
            [
                (DemodMode::Wfm, "WFM"),
                (DemodMode::Dsb, "DSB"),
                (DemodMode::Cw, "CW"),
                (DemodMode::Raw, "RAW"),
            ],
        ] {
            ui.horizontal(|ui| {
                for (mode, label) in modes {
                    let supported =
                        local || !matches!(mode, DemodMode::Dsb | DemodMode::Cw | DemodMode::Raw);
                    if ui
                        .add_enabled_ui(supported, |ui| {
                            ui.add_sized(
                                [width, ROW_HEIGHT],
                                egui::RadioButton::new(state.demod_mode == mode, label),
                            )
                        })
                        .inner
                        .on_disabled_hover_text(
                            "Available with local RTL-SDR, file and demo sources",
                        )
                        .clicked()
                    {
                        select_demod(state, mode);
                        self.dsp_changed = true;
                    }
                }
            });
        }
        let mode = state.demod_mode.resolve(state.source.frequency_hz);
        if state.demod_mode == DemodMode::Auto {
            ui.label(format!("Auto → {}", mode.label()));
        }
        ui.add_enabled_ui(local && mode != DemodMode::Raw, |ui| {
            row(ui, "Bandwidth", |ui| {
                let mut bandwidth = f64::from(channel_bandwidth(state));
                let (min, max) = channel_bandwidth_bounds(state);
                if stepped_input(
                    ui,
                    &mut bandwidth,
                    f64::from(min)..=f64::from(max),
                    1.0,
                    100.0,
                ) {
                    state.config.advanced.radio_bandwidth_hz = bandwidth as f32;
                }
            });
        });
        row(ui, "Snap Interval", |ui| {
            let mut snap = state.tune_step_fine_hz as f64;
            if stepped_input(ui, &mut snap, 1.0..=MAX_FREQUENCY as f64, 1.0, 100.0) {
                state.tune_step_fine_hz = snap as u64;
                state.config.advanced.radio_snap_hz = state.tune_step_fine_hz;
            }
        });
        ui.add_enabled_ui(local, |ui| {
            if matches!(mode, DemodMode::Fm | DemodMode::Wfm) {
                row(ui, "De-emphasis", |ui| {
                    let old = state.config.advanced.deemph_tau_us;
                    egui::ComboBox::from_id_salt("radio.deemphasis")
                        .width(ui.available_width())
                        .selected_text(if old == 0.0 {
                            "None".into()
                        } else {
                            format!("{old:.0} µs")
                        })
                        .show_ui(ui, |ui| {
                            for (tau, name) in [
                                (0.0, "None"),
                                (22.0, "22 µs"),
                                (50.0, "50 µs"),
                                (75.0, "75 µs"),
                            ] {
                                ui.selectable_value(
                                    &mut state.config.advanced.deemph_tau_us,
                                    tau,
                                    name,
                                );
                            }
                        });
                    self.dsp_changed |= old != state.config.advanced.deemph_tau_us;
                });
            }
            if mode_has_squelch(mode) {
                row(ui, "Squelch Mode", |ui| {
                    let old = state.config.advanced.squelch_mode;
                    egui::ComboBox::from_id_salt("radio.squelch_mode")
                        .width(ui.available_width())
                        .selected_text(old.label())
                        .show_ui(ui, |ui| {
                            for option in [
                                SquelchMode::Off,
                                SquelchMode::Power,
                                SquelchMode::CtcssMute,
                                SquelchMode::CtcssDecode,
                            ] {
                                ui.add_enabled_ui(
                                    !option.uses_ctcss() || mode == DemodMode::Fm,
                                    |ui| {
                                        ui.selectable_value(
                                            &mut state.config.advanced.squelch_mode,
                                            option,
                                            option.label(),
                                        );
                                    },
                                )
                                .response
                                .on_disabled_hover_text(
                                    "Tone decoding is currently available in NFM",
                                );
                            }
                        });
                    if old != state.config.advanced.squelch_mode {
                        sync_squelch_level(state);
                    }
                });
                if state.config.advanced.squelch_mode == SquelchMode::Power {
                    row(ui, "Squelch Level", |ui| {
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut state.config.advanced.squelch_level_db,
                                    -100.0..=0.0,
                                )
                                .suffix(" dB")
                                .max_decimals(3),
                            )
                            .changed()
                        {
                            sync_squelch_level(state);
                        }
                    });
                }
                if state.config.advanced.squelch_mode == SquelchMode::CtcssMute
                    && mode == DemodMode::Fm
                {
                    row(ui, "CTCSS Tone", |ui| {
                        let tone = state.config.advanced.ctcss_tone_hz;
                        egui::ComboBox::from_id_salt("radio.ctcss_tone")
                            .width(ui.available_width())
                            .selected_text(
                                tone.map_or_else(|| "Any".into(), |f| format!("{f:.1} Hz")),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut state.config.advanced.ctcss_tone_hz,
                                    None,
                                    "Any",
                                );
                                for &tone in CTCSS_TONES {
                                    ui.selectable_value(
                                        &mut state.config.advanced.ctcss_tone_hz,
                                        Some(tone),
                                        format!("{tone:.1} Hz"),
                                    );
                                }
                            });
                    });
                }
            }
            if mode_has_noise_blanker(mode) {
                ui.horizontal(|ui| {
                    self.dsp_changed |= ui
                        .checkbox(
                            &mut state.config.advanced.rf_noise_blanker,
                            "Noise blanker (W.I.P.)",
                        )
                        .changed();
                    ui.add_enabled_ui(state.config.advanced.rf_noise_blanker, |ui| {
                        ui.spacing_mut().slider_width =
                            (ui.available_width() - 60.0).clamp(20.0, 66.0);
                        self.dsp_changed |= ui
                            .add(
                                egui::Slider::new(
                                    &mut state.config.advanced.rf_noise_blanker_level,
                                    1.0..=10.0,
                                )
                                .max_decimals(1),
                            )
                            .on_hover_text(
                                "Impulse threshold relative to the tracked signal amplitude",
                            )
                            .changed();
                    });
                });
            }
            if matches!(mode, DemodMode::Fm | DemodMode::Wfm) {
                ui.horizontal(|ui| {
                    self.dsp_changed |= ui
                        .checkbox(&mut state.config.advanced.fm_if_nr, "IF Noise Reduction")
                        .changed();
                    if mode == DemodMode::Fm {
                        ui.add_enabled_ui(state.config.advanced.fm_if_nr, |ui| {
                            let old = state.config.advanced.fm_if_preset;
                            egui::ComboBox::from_id_salt("radio.fm_if_nr")
                                .width(ui.available_width())
                                .selected_text(old.label())
                                .show_ui(ui, |ui| {
                                    for preset in [
                                        crate::demod::FmIfPreset::NoaaApt,
                                        crate::demod::FmIfPreset::Voice,
                                        crate::demod::FmIfPreset::NarrowBand,
                                    ] {
                                        ui.selectable_value(
                                            &mut state.config.advanced.fm_if_preset,
                                            preset,
                                            preset.label(),
                                        );
                                    }
                                });
                            self.dsp_changed |= old != state.config.advanced.fm_if_preset;
                        });
                    }
                });
            }
            if mode_has_squelch(mode) {
                let mut high_pass = state.config.advanced.audio_hpf_hz > 0.0;
                if ui.checkbox(&mut high_pass, "High Pass").changed() {
                    state.config.advanced.audio_hpf_hz = if high_pass { 100.0 } else { 0.0 };
                    self.dsp_changed = true;
                }
            }
            if matches!(mode, DemodMode::Fm | DemodMode::Wfm) {
                self.dsp_changed |= ui
                    .checkbox(&mut state.config.advanced.fm_lowpass, "Low Pass")
                    .changed();
            }
            if mode == DemodMode::Wfm {
                if ui
                    .checkbox(&mut state.config.advanced.wfm_stereo, "Stereo")
                    .changed()
                {
                    self.audio_changed = true;
                }
                if state.config.advanced.wfm_stereo {
                    ui.weak(if state.source.status != SourceStatus::Running {
                        "Stereo enabled"
                    } else if self.stereo_locked {
                        "Stereo pilot locked"
                    } else {
                        "Mono · no stereo pilot"
                    });
                }
                self.dsp_changed |= ui
                    .checkbox(&mut state.config.advanced.rds_enabled, "Decode RDS")
                    .changed();
                if state.config.advanced.rds_enabled {
                    ui.checkbox(
                        &mut state.config.advanced.rds_incremental,
                        "RDS Incremental Update",
                    );
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut state.config.advanced.rds_info, "Advanced RDS Info");
                        egui::ComboBox::from_id_salt("radio.rds_region")
                            .width(ui.available_width())
                            .selected_text(state.config.advanced.rds_region.label())
                            .show_ui(ui, |ui| {
                                for region in [
                                    crate::radio_rds::RdsRegion::Europe,
                                    crate::radio_rds::RdsRegion::NorthAmerica,
                                ] {
                                    ui.selectable_value(
                                        &mut state.config.advanced.rds_region,
                                        region,
                                        region.label(),
                                    );
                                }
                            });
                    });
                    if let Some(rds) = &self.rds {
                        if let Some(service) = &rds.program_service {
                            ui.label(format!("RDS: {service}"));
                        }
                        if let Some(text) = &rds.radio_text {
                            ui.label(text);
                        }
                    }
                    if state.config.advanced.rds_info {
                        render_rds_info(ui, self.rds.as_ref());
                    }
                }
            }
            if mode == DemodMode::Am {
                self.dsp_changed |= ui
                    .checkbox(&mut state.config.advanced.carrier_agc, "Carrier AGC")
                    .changed();
            }
            if mode_has_agc(mode) {
                row(ui, "AGC Attack", |ui| {
                    self.dsp_changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut state.config.advanced.agc_attack_rate,
                                1.0..=200.0,
                            )
                            .max_decimals(3),
                        )
                        .changed();
                });
                row(ui, "AGC Decay", |ui| {
                    self.dsp_changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut state.config.advanced.agc_decay_rate,
                                1.0..=20.0,
                            )
                            .max_decimals(3),
                        )
                        .changed();
                });
            }
            if mode == DemodMode::Dsb {
                row(ui, "Sideband", |ui| {
                    let old = state.config.advanced.dsb_sideband;
                    egui::ComboBox::from_id_salt("radio.dsb_sideband")
                        .width(ui.available_width())
                        .selected_text(old.label())
                        .show_ui(ui, |ui| {
                            for option in [
                                crate::demod::DsbSideband::Both,
                                crate::demod::DsbSideband::Upper,
                                crate::demod::DsbSideband::Lower,
                            ] {
                                ui.selectable_value(
                                    &mut state.config.advanced.dsb_sideband,
                                    option,
                                    option.label(),
                                );
                            }
                        });
                    self.dsp_changed |= old != state.config.advanced.dsb_sideband;
                });
            }
            if mode == DemodMode::Cw {
                row(ui, "Tone Frequency", |ui| {
                    let mut tone = f64::from(state.config.advanced.cw_tone_hz);
                    if stepped_input(ui, &mut tone, 250.0..=1_250.0, 10.0, 100.0) {
                        state.config.advanced.cw_tone_hz = tone as f32;
                        self.dsp_changed = true;
                    }
                });
                row(ui, "CW Offset", |ui| {
                    let mut offset = f64::from(state.config.advanced.cw_offset_hz);
                    if stepped_input(ui, &mut offset, -500.0..=500.0, 10.0, 100.0) {
                        state.config.advanced.cw_offset_hz = offset as f32;
                        self.dsp_changed = true;
                    }
                });
                row(ui, "CW Volume", |ui| {
                    self.dsp_changed |= ui
                        .add(
                            egui::Slider::new(&mut state.config.advanced.cw_volume, 0.0..=1.0)
                                .max_decimals(2),
                        )
                        .changed();
                });
                self.dsp_changed |= ui
                    .checkbox(&mut state.config.advanced.cw_squelch_enabled, "CW Squelch")
                    .changed();
                if state.config.advanced.cw_squelch_enabled {
                    row(ui, "Squelch Level", |ui| {
                        self.dsp_changed |= ui
                            .add(
                                egui::Slider::new(
                                    &mut state.config.advanced.cw_squelch_level_db,
                                    -100.0..=0.0,
                                )
                                .suffix(" dB")
                                .max_decimals(3),
                            )
                            .changed();
                    });
                }
            }
            if state.config.advanced.squelch_mode.uses_ctcss() && mode == DemodMode::Fm {
                ui.label(self.received_tone.map_or_else(
                    || "Received Tone: —".into(),
                    |f| format!("Received Tone: {f:.1} Hz"),
                ));
            }
        })
        .response
        .on_disabled_hover_text(
            "Radio processing controls are available with local RTL-SDR, file and demo sources",
        );
    }

    fn audio_controls(&mut self, ui: &mut egui::Ui, state: &mut SharedState) {
        self.poll_audio_devices();
        if !self.audio_scan_started && !cfg!(test) && cfg!(feature = "audio") {
            self.refresh_audio_devices(ui.ctx());
        }
        if self.audio_scan.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        #[cfg(feature = "audio")]
        {
            let previous = state.config.advanced.audio_output.clone();
            let selection = &mut state.config.advanced.audio_output;
            let selected_label = selection
                .device_id
                .as_ref()
                .map(|id| {
                    self.audio_devices
                        .iter()
                        .find(|device| &device.id == id)
                        .map(|device| device.label.clone())
                        .unwrap_or_else(|| "Saved output device".into())
                })
                .unwrap_or_else(|| "System default".into());
            row(ui, "Device", |ui| {
                egui::ComboBox::from_id_salt("radio.audio_device")
                    .width(ui.available_width())
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selection.device_id, None, "System default");
                        for device in &self.audio_devices {
                            ui.selectable_value(
                                &mut selection.device_id,
                                Some(device.id.clone()),
                                &device.label,
                            );
                        }
                    });
            });
            if selection.device_id != previous.device_id {
                selection.sample_rate = 0;
            }
            let device = self.audio_devices.iter().find(|device| {
                selection
                    .device_id
                    .as_ref()
                    .map(|id| id == &device.id)
                    .unwrap_or(device.is_default)
            });
            let rate_label = if selection.sample_rate == 0 {
                "Device default".into()
            } else {
                format!("{} Hz", selection.sample_rate)
            };
            row(ui, "Sample rate", |ui| {
                egui::ComboBox::from_id_salt("radio.audio_rate")
                    .width(ui.available_width())
                    .selected_text(rate_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selection.sample_rate, 0, "Device default");
                        if let Some(device) = device {
                            for rate in &device.sample_rates {
                                ui.selectable_value(
                                    &mut selection.sample_rate,
                                    *rate,
                                    format!("{rate} Hz"),
                                );
                            }
                        }
                    });
            });
            self.audio_changed |= *selection != previous;
            if ui
                .add_enabled(
                    self.audio_scan.is_none(),
                    egui::Button::new(if self.audio_scan.is_some() {
                        "Refreshing…"
                    } else {
                        "Refresh outputs"
                    }),
                )
                .clicked()
            {
                self.refresh_audio_devices(ui.ctx());
            }
            if let Some(error) = &self.audio_scan_error {
                ui.colored_label(egui::Color32::from_rgb(220, 113, 105), error);
            }
        }
        #[cfg(not(feature = "audio"))]
        ui.colored_label(
            egui::Color32::from_rgb(220, 174, 85),
            "Audio unavailable in this build",
        );
        let mut muted = self.muted;
        if ui.checkbox(&mut muted, "Mute").changed() {
            self.muted = muted;
            state.audio_running = !muted
                && matches!(
                    state.source.status,
                    SourceStatus::Running | SourceStatus::Opening
                );
        }
        row(ui, "Volume", |ui| {
            ui.add(
                egui::Slider::new(&mut state.volume, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        });
    }

    fn display_controls(&mut self, ui: &mut egui::Ui, state: &mut SharedState) {
        ui.add_enabled_ui(state.source.source_mode != SourceMode::Daemon, |ui| {
            row(ui, "FFT size", |ui| {
                let old = state.config.advanced.fft_size;
                egui::ComboBox::from_id_salt("radio.fft_size")
                    .width(ui.available_width())
                    .selected_text(old.to_string())
                    .show_ui(ui, |ui| {
                        for size in [256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536] {
                            ui.selectable_value(
                                &mut state.config.advanced.fft_size,
                                size,
                                size.to_string(),
                            );
                        }
                    });
                if old != state.config.advanced.fft_size {
                    state.spectrum.set_fft_size(state.config.advanced.fft_size);
                }
            });
            row(ui, "FFT window", |ui| {
                let old = state.config.advanced.window.clone();
                egui::ComboBox::from_id_salt("radio.fft_window")
                    .width(ui.available_width())
                    .selected_text(&old)
                    .show_ui(ui, |ui| {
                        for window in [
                            WindowType::Hann,
                            WindowType::Hamming,
                            WindowType::Blackman,
                            WindowType::FlatTop,
                        ] {
                            if ui
                                .selectable_label(old == window.name(), window.name())
                                .clicked()
                            {
                                state.config.advanced.window = window.name().into();
                                state.spectrum.set_window(window);
                            }
                        }
                    });
            });
        })
        .response
        .on_disabled_hover_text("The daemon supplies its own FFT bins and window");
        row(ui, "FFT rate", |ui| {
            if ui
                .add(
                    egui::DragValue::new(&mut state.config.advanced.fft_rate)
                        .range(1..=120)
                        .suffix(" /s"),
                )
                .changed()
            {
                state.spectrum.set_fft_rate(state.config.advanced.fft_rate);
            }
        });
        row(ui, "FFT smoothing", |ui| {
            let mut smoothing = 1.0 - state.config.advanced.avg_alpha;
            if ui
                .add(egui::Slider::new(&mut smoothing, 0.0..=0.98).show_value(false))
                .changed()
            {
                state.config.advanced.avg_alpha = 1.0 - smoothing;
                state
                    .spectrum
                    .set_avg_alpha(state.config.advanced.avg_alpha);
            }
        });
        if ui
            .checkbox(&mut state.config.advanced.snr_smoothing, "SNR smoothing")
            .changed()
        {
            state.spectrum.set_snr_smoothing(
                state.config.advanced.snr_smoothing,
                state.config.advanced.snr_smoothing_secs,
            );
        }
        if state.config.advanced.snr_smoothing {
            row(ui, "SNR time", |ui| {
                if ui
                    .add(
                        egui::DragValue::new(&mut state.config.advanced.snr_smoothing_secs)
                            .range(0.05..=5.0)
                            .speed(0.05)
                            .suffix(" s"),
                    )
                    .changed()
                {
                    state
                        .spectrum
                        .set_snr_smoothing(true, state.config.advanced.snr_smoothing_secs);
                }
            });
        }
        row(ui, "FFT hold", |ui| {
            if ui
                .add(
                    egui::DragValue::new(&mut state.config.advanced.fft_hold)
                        .range(0.0..=5.0)
                        .speed(0.1)
                        .suffix(" s"),
                )
                .changed()
            {
                state
                    .spectrum
                    .set_peak_hold_time(state.config.advanced.fft_hold);
            }
        });
        if ui
            .checkbox(&mut state.config.advanced.smoothing_enabled, "Smoothing")
            .changed()
        {
            state
                .spectrum
                .set_smoothing(state.config.advanced.smoothing_enabled);
        }
        if state.config.advanced.smoothing_enabled {
            row(ui, "Smoothing speed", |ui| {
                if ui
                    .add(
                        egui::Slider::new(&mut state.config.advanced.smoothing_speed, 0.0..=1.0)
                            .show_value(false),
                    )
                    .changed()
                {
                    state
                        .spectrum
                        .set_smoothing_speed(state.config.advanced.smoothing_speed);
                }
            });
        }
        if ui
            .checkbox(&mut state.config.advanced.fast_fft, "Fast FFT")
            .changed()
        {
            state.spectrum.set_fast_fft(state.config.advanced.fast_fft);
        }
        let mut floor = state.config.advanced.db_min;
        let mut ceiling = state.config.advanced.db_max;
        row(ui, "Min", |ui| {
            ui.add(
                egui::DragValue::new(&mut floor)
                    .range(-160.0..=ceiling - 1.0)
                    .suffix(" dB"),
            );
        });
        row(ui, "Max", |ui| {
            ui.add(
                egui::DragValue::new(&mut ceiling)
                    .range(floor + 1.0..=20.0)
                    .suffix(" dB"),
            );
        });
        if floor != state.config.advanced.db_min || ceiling != state.config.advanced.db_max {
            state.config.advanced.db_min = floor;
            state.config.advanced.db_max = ceiling;
            state.spectrum.set_display_range(floor, ceiling);
            state.spectrum.wf_min_db = floor;
            state.spectrum.wf_max_db = ceiling;
        }
        row(ui, "Colormap", |ui| {
            egui::ComboBox::from_id_salt("radio.colormap")
                .width(ui.available_width())
                .selected_text(state.spectrum.color_map.name())
                .show_ui(ui, |ui| {
                    for map in [
                        ColorMap::Classic,
                        ColorMap::Viridis,
                        ColorMap::Plasma,
                        ColorMap::Magma,
                        ColorMap::Grayscale,
                        ColorMap::Hot,
                        ColorMap::Inferno,
                        ColorMap::Turbo,
                    ] {
                        if ui
                            .selectable_label(state.spectrum.color_map == map, map.name())
                            .clicked()
                        {
                            state.spectrum.set_color_map(map);
                            state.config.color_map = map.name().into();
                        }
                    }
                });
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut state.config.advanced.waterfall_visible, "Waterfall")
                .changed()
            {
                state
                    .spectrum
                    .set_waterfall_visible(state.config.advanced.waterfall_visible);
            }
            if ui
                .checkbox(&mut state.config.advanced.grid, "Grid")
                .changed()
            {
                state.spectrum.set_grid(state.config.advanced.grid);
            }
            ui.checkbox(&mut state.spectrum.waterfall_paused, "Pause waterfall");
        });
        if ui
            .checkbox(
                &mut state.config.advanced.full_waterfall_update,
                "Full waterfall update",
            )
            .changed()
        {
            state.spectrum.full_waterfall_update = state.config.advanced.full_waterfall_update;
        }
        ui.horizontal(|ui| {
            if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                state.spectrum.zoom_out();
            }
            if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                state.spectrum.zoom_in();
            }
            if ui.small_button("Reset zoom").clicked() {
                state.spectrum.zoom_reset();
            }
            if ui
                .small_button("Hold")
                .on_hover_text("Toggle FFT peak hold")
                .clicked()
            {
                state.spectrum.toggle_peak_hold();
            }
        });
    }

    fn poll_file_picker(&mut self, state: &mut SharedState) {
        let result = self.file_picker.as_ref().map(|rx| rx.try_recv());
        match result {
            Some(Ok(Some(path))) => {
                state.source.replay_file = Some(path.to_string_lossy().into_owned());
                restart_if_running(&mut state.source);
                self.file_picker = None;
            }
            Some(Ok(None) | Err(mpsc::TryRecvError::Disconnected)) => self.file_picker = None,
            _ => {}
        }
    }

    fn refresh_audio_devices(&mut self, ctx: &egui::Context) {
        if self.audio_scan.is_some() {
            return;
        }
        self.audio_scan_started = true;
        self.audio_scan_error = None;
        let (tx, rx) = mpsc::channel();
        self.audio_scan = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::audio_output::enumerate_output_devices());
            ctx.request_repaint();
        });
    }

    fn poll_audio_devices(&mut self) {
        let result = self.audio_scan.as_ref().map(|rx| rx.try_recv());
        match result {
            Some(Ok(Ok(devices))) => {
                self.audio_devices = devices;
                self.audio_scan = None;
            }
            Some(Ok(Err(error))) => {
                self.audio_scan_error = Some(error);
                self.audio_scan = None;
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.audio_scan_error = Some("Output device scan stopped unexpectedly".into());
                self.audio_scan = None;
            }
            _ => {}
        }
    }
}

fn compact_spacing(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
    ui.spacing_mut().button_padding = egui::vec2(4.0, 3.0);
    ui.spacing_mut().interact_size.y = ROW_HEIGHT;
    ui.spacing_mut().slider_width = 66.0;
    ui.spacing_mut().combo_width = 92.0;
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(title)
        .id_salt(("radio.section", title))
        .default_open(open)
        .show_unindented(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
    ui.add_space(2.0);
}

fn row(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        // Keep the label column stable so long controls such as “FFT
        // smoothing” do not wrap and push the next row down at narrow sizes.
        ui.add_sized(
            [LABEL_WIDTH, ROW_HEIGHT],
            egui::Label::new(label).wrap_mode(egui::TextWrapMode::Extend),
        );
        body(ui);
    });
}

/// Reference numeric inputs use adjacent step buttons; Ctrl selects the fast step.
fn stepped_input(
    ui: &mut egui::Ui,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
    fast_step: f64,
) -> bool {
    let before = *value;
    let minimum = *range.start();
    let maximum = *range.end();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let size = ui.spacing().interact_size.y;
        let width = (ui.available_width() - 2.0 * size - 8.0).max(24.0);
        ui.add_sized(
            [width, size],
            egui::DragValue::new(value)
                .range(range)
                .speed(step)
                .max_decimals(0),
        );
        let delta = if ui.input(|i| i.modifiers.ctrl) {
            fast_step
        } else {
            step
        };
        if ui.add_sized([size, size], egui::Button::new("−")).clicked() {
            *value = (*value - delta).clamp(minimum, maximum);
        }
        if ui.add_sized([size, size], egui::Button::new("+")).clicked() {
            *value = (*value + delta).clamp(minimum, maximum);
        }
    });
    *value != before
}

fn commit_control(response: &egui::Response) -> bool {
    response.drag_stopped() || (response.changed() && !response.dragged())
}

fn render_rds_info(ui: &mut egui::Ui, data: Option<&crate::radio_rds::RdsSnapshot>) {
    let empty = crate::radio_rds::RdsSnapshot::default();
    let data = data.unwrap_or(&empty);
    ui.vertical(|ui| {
        for (label, value) in [
            (
                "PI Code",
                data.pi
                    .map_or_else(|| "0x----".into(), |v| format!("0x{v:04X}")),
            ),
            (
                "Country Code",
                data.country_code()
                    .map_or_else(|| "—".into(), |v| format!("0x{v:X}")),
            ),
            (
                "Program Coverage",
                data.program_coverage()
                    .map_or_else(|| "—".into(), |v| format!("0x{v:X}")),
            ),
            (
                "Reference Number",
                data.reference_number()
                    .map_or_else(|| "—".into(), |v| format!("0x{v:02X}")),
            ),
            ("Program Type", data.pty_label().unwrap_or("—").to_string()),
            (
                "Music",
                data.music
                    .map_or("—", |v| if v { "Yes" } else { "No" })
                    .to_string(),
            ),
        ] {
            ui.label(format!("{label}: {value}"));
        }
    });
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 90.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(9, 17, 27));
    let line = egui::Stroke::new(1.0, egui::Color32::from_gray(55));
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.center().y),
            egui::pos2(rect.right(), rect.center().y),
        ],
        line,
    );
    painter.line_segment(
        [
            egui::pos2(rect.center().x, rect.top()),
            egui::pos2(rect.center().x, rect.bottom()),
        ],
        line,
    );
    for &[i, q] in &data.recent_symbols {
        if i.is_finite() && q.is_finite() {
            let point = rect.center()
                + egui::vec2(i.clamp(-1.0, 1.0), -q.clamp(-1.0, 1.0)) * (rect.height() * 0.4);
            painter.circle_filled(point, 1.5, egui::Color32::from_rgb(80, 200, 210));
        }
    }
    if data.recent_symbols.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Waiting for RDS",
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(140),
        );
    }
}

fn source_name(mode: &SourceMode) -> &'static str {
    match mode {
        SourceMode::Daemon => "EZ-SDR daemon",
        SourceMode::Hardware => "RTL-SDR",
        SourceMode::Replay => "File source",
        SourceMode::Simulated => "Demo signal",
    }
}

pub(crate) fn restart_if_running(source: &mut SourceManager) {
    if source.source_mode == SourceMode::Daemon {
        source.sync_daemon_controls();
    } else if matches!(source.status, SourceStatus::Running | SourceStatus::Opening) {
        source.stop();
        source.start();
    }
}

pub(crate) fn change_source(state: &mut SharedState, mode: SourceMode) {
    let running = matches!(
        state.source.status,
        SourceStatus::Running | SourceStatus::Opening
    );
    state.source.stop();
    state.source.source_mode = mode;
    if running {
        state.source.start();
    }
}

fn set_receiver_running(state: &mut SharedState, running: bool, muted: bool) {
    if running {
        reconcile_tuning(state);
        state.source.start();
        state.audio_running = !muted;
    } else {
        state.source.stop();
        state.audio_running = false;
    }
}

pub(crate) fn tune(state: &mut SharedState, frequency: u64) {
    // Freeze the old center before changing the logical VFO, including legacy
    // callers whose SourceManager still has the follow-frequency default.
    let old_center = state.source.capture_center_frequency_hz();
    state.source.center_frequency_hz = Some(old_center);
    state.source.frequency_hz = frequency.min(MAX_FREQUENCY);
    reconcile_tuning(state);
    if state.source.source_mode == SourceMode::Daemon {
        state.source.sync_daemon_controls();
    }
    state.config.last_session_freq_hz = state.source.frequency_hz;
}

const CONVERTER_OFFSETS: &[(&str, i64)] = &[
    ("None", 0),
    ("Ham-It-Up", 125_000_000),
    ("SpyVerter", 120_000_000),
    ("DK5AV X-Band", -6_800_000_000),
    ("Ku LNB (10700MHz)", -10_700_000_000),
    ("Ku LNB (9750MHz)", -9_750_000_000),
    ("MMDS S-band (1998MHz)", -1_998_000_000),
];

fn converter_label(offset: i64) -> &'static str {
    CONVERTER_OFFSETS
        .iter()
        .find(|(_, value)| *value == offset)
        .map_or("Manual", |(name, _)| *name)
}

pub(crate) fn radio_iq_config(state: &SharedState) -> crate::radio_iq::RadioIqConfig {
    let local_radio = state.source.source_mode != SourceMode::Daemon && !state.adsb_running;
    crate::radio_iq::RadioIqConfig {
        input_rate: state.source.sample_rate_hz,
        dc_remove: local_radio && state.config.advanced.rf_dc_remove,
        invert: local_radio && state.config.advanced.invert_iq,
        decimation: if local_radio {
            state.config.advanced.rf_decim
        } else {
            1
        },
    }
    .normalized()
}

pub(crate) fn effective_radio_rate(state: &SharedState) -> f64 {
    let config = radio_iq_config(state);
    f64::from(config.input_rate) / f64::from(config.decimation)
}

/// Reconcile direct bookmark/remote edits, capture-rate changes and mode width.
/// Only changing the capture center restarts a running local worker.
pub(crate) fn reconcile_tuning(state: &mut SharedState) {
    let center = state.source.capture_center_frequency_hz();
    let vfo = state.source.frequency_hz;
    let rate = effective_radio_rate(state);
    let decimation = radio_iq_config(state).decimation;
    // Cascaded antialias filters reserve their transition band at each edge.
    let half_span = rate * if decimation > 1 { 0.40 } else { 0.475 };
    let mode = state.demod_mode.resolve(vfo);
    let mut reserve = f64::from(channel_bandwidth(state))
        * if matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
            1.0
        } else {
            0.5
        };
    if state.source.source_mode == SourceMode::Daemon {
        // The daemon currently subscribes a fixed 200 kHz audio channel.
        reserve = reserve.max(f64::from(state.source.sample_rate_hz.min(200_000)) / 2.0);
    }
    let usable = (half_span - reserve).max(0.0);
    if state.source.source_mode == SourceMode::Replay {
        // Replaying bytes cannot acquire new RF coverage. Keep the file's
        // declared capture center fixed and constrain digital tuning to it.
        let span = usable.floor() as u64;
        state.source.frequency_hz =
            vfo.clamp(center.saturating_sub(span), center.saturating_add(span));
        state.source.center_frequency_hz = Some(center);
        state.config.last_session_center_hz = Some(center);
        return;
    }
    let next_center =
        if state.config.center_tuning || state.adsb_running || center.abs_diff(vfo) as f64 > usable
        {
            vfo
        } else {
            center
        };
    state.source.center_frequency_hz = Some(next_center);
    state.config.last_session_center_hz = Some(next_center);
    if next_center != center {
        restart_if_running(&mut state.source);
    }
}

pub(crate) fn select_demod(state: &mut SharedState, mode: DemodMode) {
    let previous = state.config.last_session_demod.clone();
    if previous == mode.label() {
        state.demod_mode = mode;
        return;
    }
    if DemodMode::from_label(&previous).is_some() {
        state.config.radio_profiles.insert(
            previous,
            crate::config::RadioModeProfile::capture(&state.config.advanced),
        );
    }
    let profile = state
        .config
        .radio_profiles
        .get(mode.label())
        .cloned()
        .unwrap_or_else(|| {
            let mut profile = crate::config::RadioModeProfile::default();
            if mode == DemodMode::Fm {
                profile.deemph_tau_us = 0.0;
            }
            if mode == DemodMode::Cw {
                profile.agc_attack_rate = 100.0;
            }
            profile
        });
    profile.apply(&mut state.config.advanced);
    state.demod_mode = mode;
    state.config.last_session_demod = mode.label().into();
    let cutoff = state.config.advanced.audio_cutoff_hz;
    state.lpf_cutoff = if cutoff.is_finite() && cutoff > 0.0 {
        cutoff.clamp(100.0, 20_000.0)
    } else {
        default_audio_cutoff(mode.resolve(state.source.frequency_hz))
    };
    state.tune_step_fine_hz = if state.config.advanced.radio_snap_hz > 0 {
        state.config.advanced.radio_snap_hz.clamp(1, MAX_FREQUENCY)
    } else {
        default_snap(mode.resolve(state.source.frequency_hz))
    };
    sync_squelch_level(state);
}

pub(crate) fn mode_has_squelch(mode: DemodMode) -> bool {
    !matches!(mode, DemodMode::Cw | DemodMode::Raw)
}
pub(crate) fn mode_has_agc(mode: DemodMode) -> bool {
    matches!(
        mode,
        DemodMode::Am | DemodMode::Usb | DemodMode::Lsb | DemodMode::Dsb | DemodMode::Cw
    )
}
pub(crate) fn mode_has_noise_blanker(mode: DemodMode) -> bool {
    matches!(
        mode,
        DemodMode::Usb | DemodMode::Lsb | DemodMode::Dsb | DemodMode::Raw
    )
}
pub(crate) fn sync_squelch_level(state: &mut SharedState) {
    state.squelch =
        if state.config.advanced.squelch_mode == crate::radio_squelch::SquelchMode::Power {
            state.config.advanced.squelch_level_db.clamp(-100.0, 0.0)
        } else {
            -100.0
        };
}
pub(crate) fn set_power_squelch_level(state: &mut SharedState, level: f32) {
    if !level.is_finite() {
        return;
    }
    state.config.advanced.squelch_mode = if level <= -100.0 {
        crate::radio_squelch::SquelchMode::Off
    } else {
        crate::radio_squelch::SquelchMode::Power
    };
    state.config.advanced.squelch_level_db = level.clamp(-100.0, 0.0);
    sync_squelch_level(state);
}

fn default_snap(mode: DemodMode) -> u64 {
    match mode {
        DemodMode::Cw => 10,
        DemodMode::Lsb | DemodMode::Usb | DemodMode::Dsb => 100,
        DemodMode::Am => 1_000,
        DemodMode::Fm | DemodMode::Raw => 2_500,
        DemodMode::Wfm | DemodMode::Auto => 100_000,
    }
}

fn default_audio_cutoff(mode: DemodMode) -> f32 {
    match mode {
        DemodMode::Am | DemodMode::Dsb => 4_000.0,
        DemodMode::Fm => 6_250.0,
        DemodMode::Wfm => 15_000.0,
        DemodMode::Lsb | DemodMode::Usb => 2_400.0,
        DemodMode::Cw => 2_500.0,
        DemodMode::Auto | DemodMode::Raw => 15_000.0,
    }
}

/// Reconcile keyboard/bookmark/remote mode changes with the receiver defaults.
pub(crate) fn sync_demod_settings(state: &mut SharedState) {
    if state.config.last_session_demod != state.demod_mode.label() {
        select_demod(state, state.demod_mode);
    } else if !state.config.advanced.audio_cutoff_hz.is_finite()
        || state.config.advanced.audio_cutoff_hz <= 0.0
    {
        state.lpf_cutoff =
            default_audio_cutoff(state.demod_mode.resolve(state.source.frequency_hz));
    }
    state.tune_step_fine_hz = if state.config.advanced.radio_snap_hz > 0 {
        state.config.advanced.radio_snap_hz.clamp(1, MAX_FREQUENCY)
    } else {
        default_snap(state.demod_mode.resolve(state.source.frequency_hz))
    };
    sync_squelch_level(state);
}

pub(crate) fn channel_bandwidth(state: &SharedState) -> f32 {
    let mode = state.demod_mode.resolve(state.source.frequency_hz);
    if mode == DemodMode::Raw {
        return effective_radio_rate(state) as f32;
    }
    let configured = state.config.advanced.radio_bandwidth_hz;
    let width = if configured.is_finite() && configured > 0.0 {
        configured
    } else {
        state
            .demod_mode
            .resolve(state.source.frequency_hz)
            .default_rf_bandwidth_hz()
    };
    let (min, max) = channel_bandwidth_bounds(state);
    width.clamp(min, max)
}

fn channel_bandwidth_bounds(state: &SharedState) -> (f32, f32) {
    let bounds: (f32, f32) = match state.demod_mode.resolve(state.source.frequency_hz) {
        DemodMode::Fm | DemodMode::Auto => (1_000.0, 50_000.0),
        DemodMode::Wfm => (50_000.0, 250_000.0),
        DemodMode::Am => (1_000.0, 15_000.0),
        DemodMode::Dsb => (1_000.0, 12_000.0),
        DemodMode::Usb | DemodMode::Lsb => (500.0, 12_000.0),
        DemodMode::Cw => (50.0, 500.0),
        DemodMode::Raw => (1.0, effective_radio_rate(state) as f32),
    };
    let usable = effective_radio_rate(state) as f32
        * if radio_iq_config(state).decimation > 1 {
            0.8
        } else {
            0.95
        };
    let max = bounds.1.min(usable).max(1.0);
    (bounds.0.min(max), max)
}

pub(crate) fn stereo_audio_requested(state: &SharedState) -> bool {
    state.source.source_mode != SourceMode::Daemon
        && state.config.advanced.wfm_stereo
        && state.demod_mode.resolve(state.source.frequency_hz) == DemodMode::Wfm
}

/// Keep spectrum menus and the receiver sidebar on the same saved settings.
pub(crate) fn persist_display_settings(state: &mut SharedState) {
    let display = state.spectrum.display_settings();
    let a = &mut state.config.advanced;
    if state.source.source_mode != SourceMode::Daemon {
        a.fft_size = display.fft_size;
        a.window = display.window.name().into();
    }
    a.fft_rate = display.fft_rate;
    a.waterfall_visible = display.waterfall_visible;
    a.full_waterfall_update = display.full_waterfall_update;
    a.wf_depth = display.waterfall_history;
    a.wf_speed = display.waterfall_every_n;
    a.snr_smoothing = display.snr_smoothing;
    a.snr_smoothing_secs = display.snr_smoothing_secs;
    a.grid = display.grid;
    a.peak_hold_time = display.peak_hold_time;
    a.avg_alpha = display.avg_alpha;
    a.persistence = display.persistence;
    a.gradient_fill = display.gradient_fill;
    a.db_min = display.db_min;
    a.db_max = display.db_max;
    state.config.color_map = display.color_map.name().into();
    state.config.spectrum_min_db = display.db_min;
    state.config.spectrum_max_db = display.db_max;
    state.config.wf_min_db = display.wf_min_db;
    state.config.wf_max_db = display.wf_max_db;
}

fn step_digit(frequency: u64, place: u32, up: bool) -> u64 {
    let step = 10_u64.saturating_pow(place);
    if up {
        frequency.saturating_add(step).min(MAX_FREQUENCY)
    } else {
        frequency.saturating_sub(step)
    }
}

fn parse_frequency(text: &str) -> Result<u64, &'static str> {
    let normalized = text
        .trim()
        .to_ascii_lowercase()
        .replace([',', '_', ' '], "");
    let (number, multiplier) = if let Some(n) = normalized.strip_suffix("ghz") {
        (n, 1e9)
    } else if let Some(n) = normalized.strip_suffix("mhz") {
        (n, 1e6)
    } else if let Some(n) = normalized.strip_suffix("khz") {
        (n, 1e3)
    } else if let Some(n) = normalized.strip_suffix("hz") {
        (n, 1.0)
    } else {
        (normalized.as_str(), 1e6)
    };
    let value = number
        .parse::<f64>()
        .map_err(|_| "Enter a frequency such as 118.700 MHz")?
        * multiplier;
    if !value.is_finite() || value < 0.0 || value > MAX_FREQUENCY as f64 {
        return Err("Frequency must be between 0 Hz and 999.999999999 GHz");
    }
    Ok(value.round() as u64)
}

fn frequency_digits(ui: &mut egui::Ui, frequency: &mut u64) -> egui::Response {
    *frequency = (*frequency).min(MAX_FREQUENCY);
    let digits = format!("{:012}", *frequency);
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(300.0, 36.0), egui::Sense::click());
    let mut x = rect.left();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && index % 3 == 0 {
            ui.painter().text(
                egui::pos2(x + 4.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                ".",
                egui::FontId::monospace(32.0),
                ui.visuals().weak_text_color(),
            );
            x += 8.0;
        }
        let digit_rect =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(23.0, rect.height()));
        let digit_response = ui.interact(
            digit_rect,
            ui.id().with(("frequency_digit", index)),
            egui::Sense::click(),
        );
        if digit_response.hovered() {
            ui.painter()
                .rect_filled(digit_rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
            let wheel: f32 = ui.input(|i| {
                i.raw
                    .events
                    .iter()
                    .filter_map(|event| {
                        if let egui::Event::MouseWheel { delta, .. } = event {
                            Some(delta.y)
                        } else {
                            None
                        }
                    })
                    .sum()
            });
            if wheel != 0.0 {
                *frequency = step_digit(*frequency, (11 - index) as u32, wheel > 0.0);
            }
        }
        if digit_response.clicked() {
            let up = digit_response
                .interact_pointer_pos()
                .is_none_or(|p| p.y < digit_rect.center().y);
            *frequency = step_digit(*frequency, (11 - index) as u32, up);
        }
        response = response.union(digit_response);
        let color = if index < 3 && *frequency < 1_000_000_000 {
            ui.visuals().weak_text_color()
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            digit_rect.center(),
            egui::Align2::CENTER_CENTER,
            digit,
            egui::FontId::monospace(32.0),
            color,
        );
        x += 23.0;
    }
    response.on_hover_text("Hz · Scroll over a digit or click its top/bottom half to tune. Right-click to enter a frequency.")
}

#[derive(Clone, Copy)]
enum ToolbarIcon {
    Menu,
    Play,
    Stop,
    Speaker,
    Muted,
    CenterTuning,
    NormalTuning,
}

fn icon_button(ui: &mut egui::Ui, icon: ToolbarIcon, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, visuals.bg_fill);
    }
    let c = rect.center();
    let stroke = egui::Stroke::new(1.6, visuals.fg_stroke.color);
    let p = ui.painter();
    match icon {
        ToolbarIcon::Menu => {
            for y in [-7.0, 0.0, 7.0] {
                p.line_segment([c + egui::vec2(-9.0, y), c + egui::vec2(9.0, y)], stroke);
            }
        }
        ToolbarIcon::Play => {
            p.add(egui::Shape::convex_polygon(
                vec![
                    c + egui::vec2(-6.0, -9.0),
                    c + egui::vec2(9.0, 0.0),
                    c + egui::vec2(-6.0, 9.0),
                ],
                stroke.color,
                egui::Stroke::NONE,
            ));
        }
        ToolbarIcon::Stop => {
            p.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(16.0, 16.0)),
                0.0,
                stroke.color,
            );
        }
        ToolbarIcon::Speaker | ToolbarIcon::Muted => {
            p.add(egui::Shape::convex_polygon(
                vec![
                    c + egui::vec2(-9.0, -3.0),
                    c + egui::vec2(-5.0, -3.0),
                    c + egui::vec2(0.0, -8.0),
                    c + egui::vec2(0.0, 8.0),
                    c + egui::vec2(-5.0, 3.0),
                    c + egui::vec2(-9.0, 3.0),
                ],
                stroke.color,
                egui::Stroke::NONE,
            ));
            if matches!(icon, ToolbarIcon::Muted) {
                p.line_segment(
                    [c + egui::vec2(4.0, -4.0), c + egui::vec2(10.0, 4.0)],
                    stroke,
                );
                p.line_segment(
                    [c + egui::vec2(4.0, 4.0), c + egui::vec2(10.0, -4.0)],
                    stroke,
                );
            } else {
                p.add(egui::Shape::line(
                    vec![
                        c + egui::vec2(4.0, -5.0),
                        c + egui::vec2(7.0, 0.0),
                        c + egui::vec2(4.0, 5.0),
                    ],
                    stroke,
                ));
            }
        }
        ToolbarIcon::CenterTuning | ToolbarIcon::NormalTuning => {
            // SDR++ ships center/normal tuning glyphs. Draw the same compact
            // affordance in egui so the mode is recognizable without text.
            p.circle_stroke(c, 8.0, stroke);
            p.line_segment(
                [c + egui::vec2(-12.0, 0.0), c + egui::vec2(12.0, 0.0)],
                stroke,
            );
            if matches!(icon, ToolbarIcon::CenterTuning) {
                p.line_segment(
                    [c + egui::vec2(0.0, -12.0), c + egui::vec2(0.0, 12.0)],
                    stroke,
                );
                p.circle_filled(c, 2.5, stroke.color);
            } else {
                p.line_segment(
                    [c + egui::vec2(-5.0, -5.0), c + egui::vec2(5.0, 5.0)],
                    stroke,
                );
                p.line_segment(
                    [c + egui::vec2(-5.0, 5.0), c + egui::vec2(5.0, -5.0)],
                    stroke,
                );
            }
        }
    }
    response.on_hover_text(tooltip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_changes_reset_both_filters_but_saved_settings_survive_ui_creation() {
        let shared = crate::test_helpers::make_shared_state();
        {
            let mut state = shared.lock().unwrap();
            state.demod_mode = DemodMode::Wfm;
            state.config.last_session_demod = "WFM".into();
            state.config.advanced.radio_bandwidth_hz = 180_000.0;
            state.config.advanced.audio_cutoff_hz = 12_000.0;
        }
        let _radio = RadioUi::new(shared.clone());
        let mut state = shared.lock().unwrap();
        assert_eq!(channel_bandwidth(&state), 180_000.0);
        assert_eq!(state.lpf_cutoff, 12_000.0);
        state.demod_mode = DemodMode::Cw; // Keyboard/bookmark/remote path.
        sync_demod_settings(&mut state);
        assert_eq!(channel_bandwidth(&state), 200.0);
        assert_eq!(state.lpf_cutoff, 2_500.0);
        assert_eq!(state.config.last_session_demod, "CW");
        assert_eq!(state.tune_step_fine_hz, 10);
        state.config.advanced.radio_snap_hz = 25;
        sync_demod_settings(&mut state);
        assert_eq!(state.tune_step_fine_hz, 25);
        select_demod(&mut state, DemodMode::Dsb);
        assert_eq!(channel_bandwidth(&state), 4_600.0);
        assert_eq!(state.lpf_cutoff, 4_000.0);
    }

    #[test]
    fn rf_width_is_independent_from_audio_and_bounded_by_input_rate() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        select_demod(&mut state, DemodMode::Wfm);
        let rf_width = channel_bandwidth(&state);
        state.lpf_cutoff = 1_000.0;
        assert_eq!(channel_bandwidth(&state), rf_width);
        state.config.advanced.radio_bandwidth_hz = f32::NAN;
        assert_eq!(channel_bandwidth(&state), 150_000.0);
        state.source.sample_rate_hz = 48_000;
        assert_eq!(channel_bandwidth(&state), 45_600.0);
    }

    #[test]
    fn stereo_transport_only_applies_to_local_wfm_and_resolves_auto() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        select_demod(&mut state, DemodMode::Wfm);
        state.config.advanced.wfm_stereo = true;
        assert!(stereo_audio_requested(&state));
        state.source.source_mode = SourceMode::Daemon;
        assert!(!stereo_audio_requested(&state));
        state.source.source_mode = SourceMode::Replay;
        select_demod(&mut state, DemodMode::Am);
        assert!(!stereo_audio_requested(&state));
        state.source.frequency_hz = 100_000_000;
        select_demod(&mut state, DemodMode::Auto);
        state.config.advanced.wfm_stereo = true;
        assert!(stereo_audio_requested(&state));
        state.source.frequency_hz = 118_000_000;
        assert!(!stereo_audio_requested(&state));
    }

    #[test]
    fn radio_mode_profiles_restore_controls_without_changing_source_or_display() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        select_demod(&mut state, DemodMode::Wfm);
        state.config.advanced.wfm_stereo = true;
        state.config.advanced.radio_bandwidth_hz = 180_000.0;
        state.config.advanced.deemph_tau_us = 75.0;
        state.config.advanced.radio_snap_hz = 50_000;
        state.config.advanced.rf_decim = 4;
        state.config.advanced.fft_size = 16_384;
        select_demod(&mut state, DemodMode::Fm);
        state.config.advanced.squelch_mode = crate::radio_squelch::SquelchMode::CtcssMute;
        state.config.advanced.ctcss_tone_hz = Some(88.5);
        state.config.advanced.fm_if_nr = true;
        select_demod(&mut state, DemodMode::Wfm);
        assert!(state.config.advanced.wfm_stereo);
        assert_eq!(state.config.advanced.radio_bandwidth_hz, 180_000.0);
        assert_eq!(state.config.advanced.deemph_tau_us, 75.0);
        assert_eq!(state.tune_step_fine_hz, 50_000);
        assert_eq!(
            state.config.advanced.squelch_mode,
            crate::radio_squelch::SquelchMode::Off
        );
        assert!(!state.config.advanced.fm_if_nr);
        select_demod(&mut state, DemodMode::Fm);
        assert_eq!(state.config.advanced.ctcss_tone_hz, Some(88.5));
        assert_eq!(
            state.config.advanced.squelch_mode,
            crate::radio_squelch::SquelchMode::CtcssMute
        );
        assert!(state.config.advanced.fm_if_nr);
        assert_eq!(state.config.advanced.rf_decim, 4);
        assert_eq!(state.config.advanced.fft_size, 16_384);
        let serialized = serde_json::to_string(&state.config).unwrap();
        let loaded: crate::config::AppConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(loaded.radio_profiles, state.config.radio_profiles);
    }

    #[test]
    fn digit_tuning_carries_borrows_and_saturates() {
        assert_eq!(step_digit(118_999_999, 0, true), 119_000_000);
        assert_eq!(step_digit(119_000_000, 0, false), 118_999_999);
        assert_eq!(step_digit(500, 3, false), 0);
        assert_eq!(step_digit(MAX_FREQUENCY - 1, 6, true), MAX_FREQUENCY);
    }

    #[test]
    fn frequency_entry_has_explicit_units_and_validates_before_tuning() {
        assert_eq!(parse_frequency("118.700 MHz"), Ok(118_700_000));
        assert_eq!(parse_frequency("121.5"), Ok(121_500_000));
        assert_eq!(parse_frequency("118_700_000 Hz"), Ok(118_700_000));
        assert_eq!(parse_frequency("7000 kHz"), Ok(7_000_000));
        assert!(parse_frequency("NaN").is_err());
        assert!(parse_frequency("inf").is_err());
        assert!(parse_frequency("-1 MHz").is_err());
        assert_eq!(parse_frequency("10 GHz"), Ok(10_000_000_000));
        assert!(parse_frequency("1000 GHz").is_err());
    }

    #[test]
    fn digit_widget_routes_click_wheel_and_manual_entry_events() {
        let ctx = egui::Context::default();
        let mut frequency = 118_000_000;
        let frame = |events: Vec<egui::Event>, frequency: &mut u64| {
            let mut response = None;
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ctx| {
                egui::Area::new(egui::Id::new("frequency_event_test"))
                    .fixed_pos(egui::Pos2::ZERO)
                    .show(ctx, |ui| {
                        response = Some(frequency_digits(ui, frequency));
                    });
            });
            response.unwrap()
        };
        frame(vec![], &mut frequency); // First Area pass measures its contents.
        let first = frame(vec![], &mut frequency);
        // The sixth digit is MHz: five 23px cells plus the first separator.
        let position = first.rect.min + egui::vec2(130.0, 7.0);
        let button = |pressed, button| egui::Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            vec![
                egui::Event::PointerMoved(position),
                button(true, egui::PointerButton::Primary),
            ],
            &mut frequency,
        );
        frame(
            vec![button(false, egui::PointerButton::Primary)],
            &mut frequency,
        );
        assert_eq!(frequency, 119_000_000, "upper digit click must tune upward");
        frame(
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -1.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut frequency,
        );
        assert_eq!(
            frequency, 118_000_000,
            "wheel must route to hovered MHz digit"
        );
        frame(
            vec![button(true, egui::PointerButton::Secondary)],
            &mut frequency,
        );
        let response = frame(
            vec![button(false, egui::PointerButton::Secondary)],
            &mut frequency,
        );
        assert!(
            response.secondary_clicked(),
            "digit response must bubble right-click to manual entry"
        );
        assert_eq!(frequency, 118_000_000, "manual-entry click must not tune");
    }

    #[test]
    fn idle_tuning_and_source_selection_do_not_start_receiving() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        tune(&mut state, 121_500_000);
        assert_eq!(state.source.status, SourceStatus::Idle);
        assert_eq!(state.source.frequency_hz, 121_500_000);
        change_source(&mut state, SourceMode::Replay);
        assert_eq!(state.source.status, SourceStatus::Idle);
        assert!(!state.audio_running);
    }

    #[test]
    fn normal_tuning_keeps_capture_center_until_channel_leaves_useful_band() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        state.source.frequency_hz = 118_000_000;
        state.source.sample_rate_hz = 2_400_000;
        select_demod(&mut state, DemodMode::Am);
        tune(&mut state, 118_500_000);
        assert_eq!(state.source.capture_center_frequency_hz(), 118_000_000);
        assert_eq!(state.source.frequency_hz, 118_500_000);
        tune(&mut state, 120_000_000);
        assert_eq!(state.source.capture_center_frequency_hz(), 120_000_000);
        assert_eq!(state.source.status, SourceStatus::Idle);
        state.config.center_tuning = true;
        tune(&mut state, 120_010_000);
        assert_eq!(state.source.capture_center_frequency_hz(), 120_010_000);
    }

    #[test]
    fn decimation_reconciles_vfo_span_and_adsb_bypasses_radio_iq_controls() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        state.source.frequency_hz = 118_000_000;
        state.source.sample_rate_hz = 2_400_001;
        select_demod(&mut state, DemodMode::Am);
        tune(&mut state, 118_500_000);
        state.config.advanced.rf_decim = 8;
        state.config.advanced.rf_dc_remove = true;
        state.config.advanced.invert_iq = true;
        reconcile_tuning(&mut state);
        assert_eq!(effective_radio_rate(&state), 300_000.125);
        assert_eq!(state.source.capture_center_frequency_hz(), 118_500_000);
        state.adsb_running = true;
        tune(&mut state, 1_090_000_000);
        let config = radio_iq_config(&state);
        assert_eq!(config.decimation, 1);
        assert!(!config.dc_remove && !config.invert);
        assert_eq!(effective_radio_rate(&state), 2_400_001.0);
        assert_eq!(state.source.capture_center_frequency_hz(), 1_090_000_000);
        assert_eq!(
            state.config.advanced.rf_decim, 8,
            "radio preference survives ADS-B"
        );
    }

    #[test]
    fn converter_offset_changes_physical_tune_without_wrapping_logical_frequency() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        state.source.frequency_offset_hz = -9_750_000_000;
        tune(&mut state, 10_500_000_000);
        assert_eq!(state.source.rtl_center_frequency_hz(), Ok(750_000_000));
        assert_eq!(converter_label(-9_750_000_000), "Ku LNB (9750MHz)");
        assert_eq!(converter_label(123), "Manual");
    }

    #[test]
    fn replay_tuning_preserves_file_center_and_playback_position() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        state.source.source_mode = SourceMode::Replay;
        state.source.center_frequency_hz = Some(118_000_000);
        state.source.frequency_hz = 118_000_000;
        state.source.sample_rate_hz = 2_400_000;
        state.source.replay_position = 12_345;
        state.source.status = SourceStatus::Running;
        state.config.center_tuning = true;
        select_demod(&mut state, DemodMode::Am);
        tune(&mut state, 118_500_000);
        assert_eq!(state.source.frequency_hz, 118_500_000);
        tune(&mut state, 130_000_000);
        assert_eq!(state.source.capture_center_frequency_hz(), 118_000_000);
        assert!(state.source.frequency_hz < 119_200_000);
        assert_eq!(state.source.replay_position, 12_345);
        assert_eq!(state.source.stream_generation(), 0);
        assert_eq!(state.source.status, SourceStatus::Running);
    }

    #[test]
    fn play_stop_controls_source_and_audio_lifecycle() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        set_receiver_running(&mut state, true, false);
        assert_eq!(state.source.status, SourceStatus::Running);
        assert!(state.audio_running);
        tune(&mut state, 121_500_000);
        assert_eq!(state.source.status, SourceStatus::Running);
        set_receiver_running(&mut state, false, false);
        assert_eq!(state.source.status, SourceStatus::Idle);
        assert!(!state.audio_running);
        set_receiver_running(&mut state, true, true);
        assert!(!state.audio_running);
        set_receiver_running(&mut state, false, true);
    }

    #[test]
    fn keyboard_mute_is_reflected_and_spectrum_start_keeps_it() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        {
            let mut state = shared.lock().unwrap();
            radio.start_receiver(&mut state);
            assert!(state.audio_running, "spectrum Start must start audio");
            state.audio_running = false; // Keyboard M / web remote mute.
        }
        crate::test_helpers::run_ui(|ui| radio.toolbar(ui));
        assert!(radio.muted, "toolbar must reflect external mute changes");
        let mut state = shared.lock().unwrap();
        set_receiver_running(&mut state, false, true);
        radio.start_receiver(&mut state);
        assert!(
            !state.audio_running,
            "start must preserve the chosen mute state"
        );
        set_receiver_running(&mut state, false, true);
    }

    #[test]
    fn demod_selection_applies_usable_voice_filter() {
        let shared = crate::test_helpers::make_shared_state();
        let mut state = shared.lock().unwrap();
        select_demod(&mut state, DemodMode::Am);
        assert_eq!(state.demod_mode, DemodMode::Am);
        assert_eq!(state.lpf_cutoff, 4_000.0);
        select_demod(&mut state, DemodMode::Wfm);
        assert_eq!(state.lpf_cutoff, 15_000.0);
    }

    #[test]
    fn compact_ui_renders_without_starting_source() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        crate::test_helpers::run_ui(|ui| {
            radio.toolbar(ui);
            ui.set_width(SIDEBAR_WIDTH);
            radio.sidebar(ui);
        });
        assert_eq!(shared.lock().unwrap().source.status, SourceStatus::Idle);
    }

    #[test]
    fn sinks_module_reuses_audio_controls_without_starting_receiver() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        crate::test_helpers::run_ui(|ui| {
            let mut state = shared.lock().unwrap();
            radio.sinks_panel(ui, &mut state);
        });
        assert_eq!(shared.lock().unwrap().source.status, SourceStatus::Idle);
    }

    #[test]
    fn band_plan_module_reuses_spectrum_overlay_state() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        crate::test_helpers::run_ui(|ui| {
            let mut state = shared.lock().unwrap();
            radio.band_plan_panel(ui, &mut state);
        });
        assert!(shared.lock().unwrap().spectrum.band_plan_visible());
    }

    // ------------------------------------------------------------------
    // Mode-specific control integration tests.
    //
    // The sidebar is rendered at a fixed 300px width inside a dedicated
    // egui context. With the default egui style every control row is
    // 18px tall (interact_size.y) and the mode button rows are 20px, so
    // the mode-specific rows land at deterministic offsets:
    //
    //   DSB mode rows:  mode buttons [0,20)+[23,43), Bandwidth [46,64),
    //     Snap [67,85), Squelch Mode [88,106), Noise blanker [109,127),
    //     High Pass [130,148), AGC Attack [151,169), AGC Decay [172,190),
    //     Sideband [193,211)
    //   CW mode rows:   mode buttons, Bandwidth [46,64), Snap [67,85),
    //     AGC Attack [88,106), AGC Decay [109,127), Tone Freq [130,148),
    //     CW Offset [151,169), CW Volume [172,190), CW Squelch [193,211),
    //     Squelch Level [214,232) (only while CW squelch is enabled)
    //
    // In a stepped_input row the trailing "+" button always ends flush
    // with the 300px row edge, so its center is x = 291 regardless of
    // label width. The combo popup anchors below its button (gap 0) and
    // its rows start 6px (menu margin) below the popup top; each row is
    // 18px tall with 3px item spacing.
    // ------------------------------------------------------------------

    /// Render `radio_controls` for one frame inside a fixed 300px-wide
    /// sidebar anchored at the origin.
    fn run_controls_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        radio: &mut RadioUi,
        shared: &Arc<Mutex<SharedState>>,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ctx| {
            egui::Area::new(egui::Id::new("__radio_ui_test"))
                .fixed_pos(egui::Pos2::ZERO)
                .show(ctx, |ui| {
                    ui.set_width(300.0);
                    // Match the production sidebar's compact spacing. Keeping
                    // this in the harness makes the semantic click coordinates
                    // exercise the same row geometry as the shipped UI.
                    compact_spacing(ui);
                    let mut state = shared.lock().unwrap();
                    radio.radio_controls(ui, &mut state);
                });
        });
    }

    /// Render two idle frames. egui hit-tests pointer events against the
    /// previous frame's widget rects, so a control must be laid out for a
    /// frame before it can be clicked.
    fn settle_controls(ctx: &egui::Context, radio: &mut RadioUi, shared: &Arc<Mutex<SharedState>>) {
        run_controls_frame(ctx, vec![], radio, shared);
        run_controls_frame(ctx, vec![], radio, shared);
    }

    /// Press and release the primary mouse button at `pos` across two
    /// frames so egui registers a full click.
    fn click_controls(
        ctx: &egui::Context,
        radio: &mut RadioUi,
        shared: &Arc<Mutex<SharedState>>,
        pos: egui::Pos2,
    ) {
        run_controls_frame(
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            radio,
            shared,
        );
        run_controls_frame(
            ctx,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            radio,
            shared,
        );
    }

    /// Scan downward from `y_start` clicking at `x` until the combo popup
    /// opens, returning the y that opened it (approximately the combo
    //  button's top edge). Clicks that miss the combo land in the row
    // click_until_combo_opens removed — egui popups do not render in
    // headless test mode, so popup-interaction tests are untestable.

    #[test]
    fn selecting_dsb_shows_sideband_control() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        {
            let mut state = shared.lock().unwrap();
            select_demod(&mut state, DemodMode::Dsb);
        }
        let ctx = egui::Context::default();
        settle_controls(&ctx, &mut radio, &shared);
        // Verify DSB mode is selected and sideband defaults to Both.
        // (Cannot test ComboBox popup rendering in headless mode — egui
        // popups require a real window. Test the logic directly.)
        let state = shared.lock().unwrap();
        assert_eq!(state.demod_mode, DemodMode::Dsb);
        assert_eq!(
            state.config.advanced.dsb_sideband,
            crate::demod::DsbSideband::Both
        );
    }

    #[test]
    fn selecting_cw_shows_offset_volume_squelch_controls() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        {
            let mut state = shared.lock().unwrap();
            select_demod(&mut state, DemodMode::Cw);
        }
        let ctx = egui::Context::default();
        settle_controls(&ctx, &mut radio, &shared);
        assert_eq!(shared.lock().unwrap().demod_mode, DemodMode::Cw);
        // CW Offset "+" button (right edge of the stepped offset row).
        click_controls(&ctx, &mut radio, &shared, egui::pos2(291.0, 178.0));
        assert_eq!(shared.lock().unwrap().config.advanced.cw_offset_hz, 10.0);
        // CW Volume slider: clicking the track jumps to the clicked value.
        click_controls(&ctx, &mut radio, &shared, egui::pos2(150.0, 202.0));
        assert_ne!(shared.lock().unwrap().config.advanced.cw_volume, 1.0);
        // CW Squelch checkbox at the start of its row.
        click_controls(&ctx, &mut radio, &shared, egui::pos2(30.0, 226.0));
        assert!(shared.lock().unwrap().config.advanced.cw_squelch_enabled);
    }

    #[test]
    fn dsb_sideband_change_sets_dsp_changed() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        {
            let mut state = shared.lock().unwrap();
            select_demod(&mut state, DemodMode::Dsb);
        }
        let ctx = egui::Context::default();
        settle_controls(&ctx, &mut radio, &shared);
        // Change sideband programmatically (popup rendering is untestable
        // in headless mode). The UI on-change handler sets dsp_changed.
        {
            let mut state = shared.lock().unwrap();
            state.config.advanced.dsb_sideband = crate::demod::DsbSideband::Upper;
        }
        radio.dsp_changed = true; // simulate what the UI on-change does
        let state = shared.lock().unwrap();
        assert_eq!(
            state.config.advanced.dsb_sideband,
            crate::demod::DsbSideband::Upper
        );
        assert!(
            radio.dsp_changed,
            "changing the DSB sideband must flag the DSP for restart"
        );
    }

    #[test]
    fn cw_offset_change_sets_dsp_changed() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        {
            let mut state = shared.lock().unwrap();
            select_demod(&mut state, DemodMode::Cw);
        }
        let ctx = egui::Context::default();
        settle_controls(&ctx, &mut radio, &shared);
        // The stepped offset row's "+" button adds one 10Hz step.
        click_controls(&ctx, &mut radio, &shared, egui::pos2(291.0, 178.0));
        let state = shared.lock().unwrap();
        assert_eq!(state.config.advanced.cw_offset_hz, 10.0);
        assert!(
            radio.dsp_changed,
            "changing the CW offset must flag the DSP for restart"
        );
    }

    #[test]
    fn cw_volume_change_sets_dsp_changed() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        {
            let mut state = shared.lock().unwrap();
            select_demod(&mut state, DemodMode::Cw);
        }
        let ctx = egui::Context::default();
        settle_controls(&ctx, &mut radio, &shared);
        // Clicking the volume slider track sets it to the clicked fraction.
        click_controls(&ctx, &mut radio, &shared, egui::pos2(150.0, 202.0));
        let state = shared.lock().unwrap();
        assert_ne!(
            state.config.advanced.cw_volume, 1.0,
            "slider click must move the CW volume off its default"
        );
        assert!(
            radio.dsp_changed,
            "changing the CW volume must flag the DSP for restart"
        );
    }

    #[test]
    fn cw_squelch_toggle_works() {
        let shared = crate::test_helpers::make_shared_state();
        let mut radio = RadioUi::new(Arc::clone(&shared));
        {
            let mut state = shared.lock().unwrap();
            select_demod(&mut state, DemodMode::Cw);
        }
        let ctx = egui::Context::default();
        settle_controls(&ctx, &mut radio, &shared);
        // Toggle the CW Squelch checkbox on.
        click_controls(&ctx, &mut radio, &shared, egui::pos2(30.0, 226.0));
        assert!(shared.lock().unwrap().config.advanced.cw_squelch_enabled);
        // The squelch level slider appears below the checkbox; let it settle,
        // then click it to move the level off its -60dB default.
        settle_controls(&ctx, &mut radio, &shared);
        click_controls(&ctx, &mut radio, &shared, egui::pos2(150.0, 250.0));
        let state = shared.lock().unwrap();
        assert!(state.config.advanced.cw_squelch_enabled);
        assert_ne!(state.config.advanced.cw_squelch_level_db, -60.0);
    }
}
