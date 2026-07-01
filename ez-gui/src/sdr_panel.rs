use std::sync::{Arc, Mutex};

use crate::app::SharedState;

fn format_hz(hz: u32) -> String {
    if hz >= 1_000_000 {
        format!("{:.2} MHz", hz as f64 / 1e6)
    } else if hz >= 1_000 {
        format!("{:.1} kHz", hz as f64 / 1e3)
    } else {
        format!("{} Hz", hz)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodMode {
    Raw,
    Am,
    Fm,
    Wfm,
    Lsb,
    Usb,
}

impl DemodMode {
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "RAW" => Some(Self::Raw),
            "AM" => Some(Self::Am),
            "FM" | "NFM" => Some(Self::Fm),
            "WFM" => Some(Self::Wfm),
            "LSB" => Some(Self::Lsb),
            "USB" => Some(Self::Usb),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DemodMode::Raw => "RAW",
            DemodMode::Am => "AM",
            DemodMode::Fm => "FM",
            DemodMode::Wfm => "WFM",
            DemodMode::Lsb => "LSB",
            DemodMode::Usb => "USB",
        }
    }
}

pub struct SdrPanel {
    shared: Arc<Mutex<SharedState>>,
    pub squelch: f32,
    pub filter_bw: u32,
    pub bookmark_request: Option<(u64, String)>,
    pub pending_ai_freq: Option<u64>,
    pub tune_request: Option<u64>,
    freq_input: String,
    freq_input_error: String,
    freq_input_error_time: Option<std::time::Instant>,
    auto_squelch: bool,
    auto_squelch_offset: f32,
    audio_peak_hold: f32,
    audio_peak_hold_time: Option<std::time::Instant>,
    signal_log: Vec<(u64, f32, std::time::SystemTime)>, // freq_hz, snr_db, timestamp
    last_logged_freq: u64,
    session_start: std::time::Instant,
    best_snr_this_session: f32,
    frequencies_explored: std::collections::HashSet<u64>,
    signal_alert_threshold: f32,
    last_alert_freq: u64,
    auto_record_enabled: bool,
    auto_record_threshold: f32,
    last_auto_recorded_freq: u64,
    show_mode_guide: bool,
    show_memory_editor: bool,
    memory_labels_edit: [String; 9],
    // Antenna setup checklist gate (shown before panel content)
    checklist: crate::antenna_checklist::AntennaChecklist,
    pub pending_status: Option<String>,
    // Airport frequency finder
    airport_db: crate::airport_db::AirportDb,
    airport_search: String,
    airport_type_filter: String,
    selected_airport: Option<String>,
    expanded_antenna_for: Option<f64>,
    airport_dl_progress: Option<(usize, usize)>,
    airport_dl_msg: String,
    // Inline expand toggles for lower user levels
    expand_ppm: bool,
    expand_lo_offset: bool,
    expand_vfo_b: bool,
    expand_memory: bool,
    expand_airport: bool,
    expand_bias_tee: bool,
    expand_step: bool,
}

impl SdrPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared: shared.clone(),
            squelch: -50.0,
            filter_bw: 12_000,
            bookmark_request: None,
            pending_ai_freq: None,
            tune_request: None,
            freq_input: String::new(),
            freq_input_error: String::new(),
            freq_input_error_time: None,
            auto_squelch: false,
            auto_squelch_offset: 5.0,
            audio_peak_hold: 0.0,
            audio_peak_hold_time: None,
            signal_log: Vec::new(),
            last_logged_freq: 0,
            session_start: std::time::Instant::now(),
            best_snr_this_session: 0.0,
            frequencies_explored: std::collections::HashSet::new(),
            signal_alert_threshold: 15.0, // Alert when SNR > 15 dB
            last_alert_freq: 0,
            auto_record_enabled: false,
            auto_record_threshold: 20.0, // Record when SNR > 20 dB
            last_auto_recorded_freq: 0,
            show_mode_guide: false,
            show_memory_editor: false,
            memory_labels_edit: std::array::from_fn(|_| String::new()),
            checklist: crate::antenna_checklist::AntennaChecklist::for_sdr(shared.clone()),
            pending_status: None,
            airport_db: crate::airport_db::AirportDb::load(),
            airport_search: String::new(),
            airport_type_filter: "all".to_string(),
            selected_airport: None,
            expanded_antenna_for: None,
            airport_dl_progress: None,
            airport_dl_msg: String::new(),
            expand_ppm: false,
            expand_lo_offset: false,
            expand_vfo_b: false,
            expand_memory: false,
            expand_airport: false,
            expand_bias_tee: false,
            expand_step: false,
        }
    }

    pub fn ui_source(&mut self, ui: &mut egui::Ui) {
        // Antenna setup checklist gate — must pass before showing panel content.
        if !self.checklist.ui(ui) {
            if let Some(msg) = self.checklist.pending_status.take() {
                self.pending_status = Some(msg);
            }
            return;
        }
        if let Some(msg) = self.checklist.pending_status.take() {
            self.pending_status = Some(msg);
        }

        // Determine user level for adaptive UI
        let user_level = self.shared.try_lock()
            .map(|s| crate::user_level::UserLevel::from_str(&s.config.user_level))
            .unwrap_or(crate::user_level::UserLevel::Beginner);
        let show_advanced = user_level.show_advanced_controls();
        let has_expand = user_level.has_inline_expand();
        let is_beginner = user_level.simplify_layout();

        ui.heading("SDR Receiver");

        // Start/Stop + source mode at the very top for discoverability
        if let Ok(mut state) = self.shared.try_lock() {
            let is_running = state.source.status == crate::source_manager::SourceStatus::Running;
            let is_opening = state.source.status == crate::source_manager::SourceStatus::Opening;
            ui.horizontal_wrapped(|ui| {
                if is_running {
                    if ui.add(egui::Button::new(egui::RichText::new("■ Stop").color(egui::Color32::from_rgb(220, 80, 80))))
                        .on_hover_text("Stop the SDR source (keyboard: Space)")
                        .clicked()
                    {
                        state.source.stop();
                    }
                } else if is_opening {
                    ui.add_enabled(false, egui::Button::new("⌛ Starting…"));
                } else {
                    if ui.add(egui::Button::new(egui::RichText::new("▶ Start").color(egui::Color32::from_rgb(80, 220, 120))))
                        .on_hover_text("Start the SDR source and begin receiving (keyboard: Space)")
                        .clicked()
                    {
                        state.source.start();
                    }
                }
                ui.separator();
                // Source mode selector
                let source_label = if cfg!(feature = "rtlsdr") { "RTL-SDR" } else { "Demo" };
                let source_help = if cfg!(feature = "rtlsdr") {
                    "Real RTL-SDR hardware — receiving live signals from the connected device."
                } else {
                    "Simulated demo mode — generates realistic signals so you can explore without hardware."
                };
                ui.label("Mode:").on_hover_text("Select how to receive signals: RTL-SDR = live hardware, File = replay a recorded IQ file.");
                if ui.selectable_label(state.source.source_mode == crate::source_manager::SourceMode::Simulated, source_label)
                    .on_hover_text(source_help)
                    .clicked()
                {
                    state.source.source_mode = crate::source_manager::SourceMode::Simulated;
                }
                if ui.selectable_label(state.source.source_mode == crate::source_manager::SourceMode::Replay, "File Replay")
                    .on_hover_text("Replay a previously recorded IQ file (.iq / .bin / .raw). Configure the path in the Source section below.")
                    .clicked()
                {
                    state.source.source_mode = crate::source_manager::SourceMode::Replay;
                }
                ui.separator();
                // Compact status indicator
                let (dot_color, status_text) = match &state.source.status {
                    crate::source_manager::SourceStatus::Running => (egui::Color32::from_rgb(50, 220, 80), "Running"),
                    crate::source_manager::SourceStatus::Idle    => (egui::Color32::GRAY, "Idle"),
                    crate::source_manager::SourceStatus::Opening => (egui::Color32::YELLOW, "Opening…"),
                    crate::source_manager::SourceStatus::Error(_)=> (egui::Color32::RED, "Error"),
                };
                ui.colored_label(dot_color, format!("● {}", status_text))
                    .on_hover_text("SDR source status. Press Space to toggle start/stop from anywhere.");
                // MQTT status
                if state.mqtt_enabled {
                    if state.mqtt_connected {
                        ui.colored_label(egui::Color32::from_rgb(46, 204, 113), "MQTT ✓")
                            .on_hover_text("MQTT broker connected — publishing SDR state and ADS-B data.");
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(200, 150, 50), "MQTT ⏳")
                            .on_hover_text("MQTT enabled but waiting for connection — retrying every 10s.");
                    }
                }
            });
        }
        ui.separator();

        // Big frequency display with fine/coarse tuning
        if let Ok(mut state) = self.shared.try_lock() {
            // Row 1: Frequency readout + DragValue
            ui.horizontal(|ui| {
                let mut freq_mhz = state.source.frequency_hz as f64 / 1e6;
                ui.monospace(egui::RichText::new(format!("{:.6}", freq_mhz)).size(24.0).color(egui::Color32::from_rgb(52, 152, 219)))
                    .on_hover_text("Current tuned frequency. RTL-SDR range: 24 MHz – 1766 MHz.");
                ui.label(egui::RichText::new("MHz").size(14.0).color(egui::Color32::GRAY));
                let is_dragging = ui.add(egui::DragValue::new(&mut freq_mhz).speed(0.0001).range(0.5..=1770.0).suffix(" MHz"))
                    .on_hover_text("Drag or type to tune. Click spectrum to tune there.");
                if is_dragging.changed() || is_dragging.dragged() {
                    self.tune_request = Some((freq_mhz * 1e6) as u64);
                }
            });
            // Row 2: Tuning buttons + actions (wraps when tab is narrow)
            let cur_freq_for_btns = state.source.frequency_hz;
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("-1M").on_hover_text("−1 MHz (↓)").clicked() {
                    self.tune_request = Some(cur_freq_for_btns.saturating_sub(1_000_000).max(500_000));
                }
                if ui.small_button("+1M").on_hover_text("+1 MHz (↑)").clicked() {
                    self.tune_request = Some((cur_freq_for_btns + 1_000_000).min(1_770_000_000));
                }
                if ui.small_button("-100k").on_hover_text("−100 kHz (←)").clicked() {
                    self.tune_request = Some(cur_freq_for_btns.saturating_sub(100_000).max(500_000));
                }
                if ui.small_button("+100k").on_hover_text("+100 kHz (→)").clicked() {
                    self.tune_request = Some((cur_freq_for_btns + 100_000).min(1_770_000_000));
                }
                ui.separator();
                let bm_freq = state.source.frequency_hz;
                let bm_mode = state.demod_mode.label().to_string();
                if ui.small_button("⭐").on_hover_text("Bookmark this frequency").clicked() {
                    self.bookmark_request = Some((bm_freq, bm_mode));
                }
                if ui.small_button("🤖").on_hover_text("Ask AI about this frequency").clicked() {
                    self.pending_ai_freq = Some(bm_freq);
                }
                let is_frozen = state.spectrum.frozen;
                if ui.small_button(if is_frozen { "▶" } else { "❄" })
                    .on_hover_text(if is_frozen { "Unfreeze spectrum" } else { "Freeze spectrum" })
                    .clicked()
                {
                    state.spectrum.frozen = !state.spectrum.frozen;
                }
            });
            if !is_beginner {
                ui.separator();
                // Band presets
                let current_freq = state.source.frequency_hz;
                let band_presets: &[(&str, u64, &str)] = &[
                    ("LW (153-279 kHz)",        153_000,      "Longwave broadcast"),
                    ("MW/AM (530-1710 kHz)",    1_000_000,    "AM broadcast band"),
                    ("Shortwave (2.3-30 MHz)",  10_000_000,   "Shortwave HF band"),
                    ("CB Radio (27 MHz)",       27_000_000,   "Citizens Band"),
                    ("6m HAM (50-54 MHz)",      50_000_000,   "6m amateur band"),
                    ("FM Broadcast (88-108)",   100_000_000,  "WFM broadcast radio"),
                    ("Air Band (118-137 MHz)",  120_000_000,  "AM aviation"),
                    ("2m HAM (144-148 MHz)",    145_000_000,  "2m amateur band"),
                    ("Marine VHF (156-174)",    160_000_000,  "Marine radio"),
                    ("70cm HAM (430-440 MHz)",  435_000_000,  "70cm amateur band"),
                    ("GMRS/FRS (462-467 MHz)",  462_000_000,  "GMRS/FRS"),
                    ("UHF (700-900 MHz)",       800_000_000,  "Cellular/UHF TV"),
                    ("ADS-B (1090 MHz)",        1_090_000_000,"Aircraft transponder"),
                    ("L-Band (1.5-1.7 GHz)",    1_500_000_000,"GPS/satellite"),
                ];
                let selected = band_presets.iter()
                    .position(|(_, freq, _)| {
                        let diff = if *freq > current_freq { *freq - current_freq } else { current_freq - *freq };
                        diff < 2_000_000
                    });
                let combo = egui::ComboBox::from_id_salt("band_presets")
                    .selected_text(if let Some(idx) = selected { band_presets[idx].0 } else { "Band…" })
                    .width(ui.available_width().min(250.0).max(80.0))
                    .show_ui(ui, |ui| {
                        for (i, &(name, freq, _desc)) in band_presets.iter().enumerate() {
                            if ui.selectable_label(selected == Some(i), name).clicked() {
                                self.tune_request = Some(freq);
                            }
                        }
                    });
                combo.response.on_hover_text("Jump to a common frequency band. The dropdown shows which band your current frequency is nearest to.");
            }
        }

        // Frequency information and mode suggestion
        if !is_beginner {
            if let Ok(state) = self.shared.try_lock() {
                if let Some(info) = identify_frequency(state.source.frequency_hz) {
                    // Show band info
                    let hover = if info.what_to_hear.is_empty() {
                        format!("{}", info.detail)
                    } else {
                        format!("{}\n🔊 {}", info.detail, info.what_to_hear)
                    };
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 255),
                        format!("📍 {}: {}", info.band, info.short_desc))
                        .on_hover_text(hover);

                    // Parse the tips field to extract suggested mode
                    let suggested_mode = if info.tips.contains("LSB") {
                        Some(("LSB", "for voice"))
                    } else if info.tips.contains("USB") {
                        Some(("USB", "for voice"))
                    } else if info.tips.contains("WFM") {
                        Some(("WFM", "for broadcast"))
                    } else if info.tips.contains("NFM") || info.tips.contains("FM") {
                        Some(("FM", "for narrowband"))
                    } else if info.tips.contains("AM") {
                        Some(("AM", "for AM broadcast"))
                    } else if info.tips.contains("RAW") {
                        Some(("RAW", "for digital"))
                    } else {
                        None
                    };

                    if let Some((mode, desc)) = suggested_mode {
                        if state.demod_mode.label() != mode {
                            ui.horizontal(|ui| {
                                ui.colored_label(egui::Color32::from_rgb(200, 200, 100),
                                    format!("💡 Suggested: {} ({})", mode, desc));
                                if ui.small_button("Apply").on_hover_text(format!("Switch to {} mode for this frequency", mode)).clicked() {
                                    drop(state);
                                    if let Ok(mut state_mut) = self.shared.try_lock() {
                                        if let Some(new_mode) = DemodMode::from_label(mode) {
                                            state_mut.demod_mode = new_mode;
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            }

            // Quick tune presets
            if let Ok(mut state) = self.shared.try_lock() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("🎯 Quick Tune:").on_hover_text("One-click tuning to popular frequencies with auto-mode selection");
                    let presets = [
                        ("📻 FM", 100_000_000u64, DemodMode::Wfm, "FM radio broadcast"),
                        ("🛩️ ADS-B", 1_090_000_000, DemodMode::Raw, "Aircraft tracking"),
                        ("🛰️ NOAA 15", 137_620_000, DemodMode::Wfm, "Weather satellite APT"),
                        ("☁️ NOAA WX", 162_550_000, DemodMode::Fm, "NOAA weather radio"),
                        ("📡 ISS", 145_800_000, DemodMode::Fm, "International Space Station"),
                        ("📍 GPS L1", 1_575_420_000, DemodMode::Raw, "GPS L1 signal"),
                        ("🔬 2m Ham", 145_500_000, DemodMode::Fm, "2m Amateur band"),
                    ];
                    for (label, freq, mode, tooltip) in presets.iter() {
                        if ui.small_button(*label).on_hover_text(*tooltip).clicked() {
                            self.tune_request = Some(*freq);
                            state.demod_mode = *mode;
                        }
                    }
                });

                // Recent frequencies quick access
                if !state.config.recent_frequencies.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("📜 Recent:").on_hover_text("Frequencies you've tuned to recently. Click to jump back.");
                        let recent = state.config.recent_frequencies.clone();
                        for (idx, freq_hz) in recent.iter().rev().enumerate() {
                            if idx >= 5 { break; } // Show last 5
                            let freq_mhz = *freq_hz as f64 / 1e6;
                            if ui.small_button(format!("{:.3}", freq_mhz)).on_hover_text(format!("{:.6} MHz", freq_mhz)).clicked() {
                                self.tune_request = Some(*freq_hz);
                            }
                        }
                    });
                }

            }
        }

        // VFO A/B swap (hidden for beginners unless expanded)
        let show_vfo_b = show_advanced || !user_level.simplify_layout();
        if show_vfo_b || (has_expand && self.expand_vfo_b) {
            if let Ok(mut state) = self.shared.try_lock() {
                let vfo_b_mhz = state.vfo_b as f64 / 1e6;
                let cur_mhz = state.source.frequency_hz as f64 / 1e6;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("VFO A").strong().color(egui::Color32::from_rgb(52, 200, 100)));
                    ui.monospace(format!("{:.3} MHz", cur_mhz));
                    ui.separator();
                    ui.label(egui::RichText::new("VFO B").color(egui::Color32::from_rgb(100, 180, 255)));
                    ui.monospace(format!("{:.3} MHz", vfo_b_mhz));
                    if ui.small_button("⇄ Swap")
                        .on_hover_text("Swap between VFO A and VFO B frequencies (keyboard: V). VFO B stores an alternate frequency for quick A/B comparison.")
                        .clicked()
                    {
                        let tmp = state.source.frequency_hz;
                        state.source.frequency_hz = state.vfo_b;
                        state.vfo_b = tmp;
                    }
                    if ui.small_button("Set B here")
                        .on_hover_text("Save current frequency as VFO B without switching.")
                        .clicked()
                    {
                        state.vfo_b = state.source.frequency_hz;
                    }
                });

                // VFO A/B difference indicator
                if state.vfo_b > 0 {
                    let diff_hz = (state.source.frequency_hz as i64 - state.vfo_b as i64).abs();
                    let diff_str = if diff_hz >= 1_000_000 {
                        format!("Δ {:.3} MHz", diff_hz as f64 / 1e6)
                    } else if diff_hz >= 1_000 {
                        format!("Δ {:.1} kHz", diff_hz as f64 / 1e3)
                    } else {
                        format!("Δ {} Hz", diff_hz)
                    };
                    ui.colored_label(egui::Color32::from_rgb(180, 150, 200), &diff_str)
                        .on_hover_text(format!("Frequency offset between VFO A and VFO B: {}", diff_str));
                }
            }
        } else if has_expand {
            if ui.button(egui::RichText::new("⚙ Show VFO B (advanced)").size(12.0)).clicked() {
                self.expand_vfo_b = true;
            }
        }

        // LO offset indicator (hidden for Beginner/Intermediate unless expanded)
        let show_lo = show_advanced;
        if show_lo || (has_expand && self.expand_lo_offset) {
            if let Ok(state) = self.shared.try_lock() {
                if state.lo_offset_hz != 0 {
                    ui.horizontal(|ui| {
                        ui.label("LO offset:").on_hover_text("Local oscillator offset for upconverter/downconverter configurations.");
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 180, 50),
                            format!("{:+.1} MHz", state.lo_offset_hz as f64 / 1e6)
                        ).on_hover_text(format!("Active LO offset. True frequency = {} + {} = {:.6} MHz",
                            state.source.frequency_hz as f64 / 1e6,
                            state.lo_offset_hz as f64 / 1e6,
                            (state.source.frequency_hz as i64 + state.lo_offset_hz).max(0) as f64 / 1e6
                        ));
                    });
                }
            }
        } else if has_expand {
            if ui.button(egui::RichText::new("⚙ Show LO offset (advanced)").size(12.0)).clicked() {
                self.expand_lo_offset = true;
            }
        }

        if !is_beginner {
            // Sample rate quick buttons
            if let Ok(mut state) = self.shared.try_lock() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Sample rate:").on_hover_text("Receiver sample rate. Higher = wider spectrum, slower updates.");
                    for (label, rate_hz) in [
                        ("1M", 1_000_000u32),
                        ("1.536M", 1_536_000),
                        ("2M", 2_000_000),
                        ("2.4M", 2_400_000),
                        ("2.88M", 2_880_000),
                    ] {
                        let is_active = state.source.sample_rate_hz == rate_hz;
                        if ui.selectable_label(is_active, label)
                            .on_hover_text(format!("Set sample rate to {} SPS", rate_hz))
                            .clicked()
                        {
                            state.source.sample_rate_hz = rate_hz;
                        }
                    }
                    let current_rate = state.source.sample_rate_hz / 1_000_000;
                    let rate_remainder = (state.source.sample_rate_hz % 1_000_000) / 1000;
                    if rate_remainder > 0 {
                        ui.label(format!("({}.{}M)", current_rate, rate_remainder))
                            .on_hover_text("Current sample rate");
                    }
                });
            }

            // Gain quick buttons
            if let Ok(mut state) = self.shared.try_lock() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Gain:").on_hover_text("RF amplifier gain. Higher = more sensitivity but risks overload. Sweet spot usually 30-45 dB.");
                    for (label, gain_db) in [
                        ("Off", 0.0f64),
                        ("15dB", 15.0),
                        ("30dB", 30.0),
                        ("40dB", 40.0),
                        ("Max", 49.6),
                    ] {
                        let is_active = (state.source.gain_db - gain_db).abs() < 0.5;
                        if ui.selectable_label(is_active, label)
                            .on_hover_text(format!("Set gain to {:.1} dB", gain_db))
                            .clicked()
                        {
                            state.source.gain_db = gain_db;
                        }
                    }
                    ui.label(format!("({:.1} dB)", state.source.gain_db))
                        .on_hover_text("Current gain setting");
                });

                // Gain optimization suggestion
                let audio_peak = state.audio_peak;
                let snr_db = state.spectrum.peak_level() - state.spectrum.noise_floor();
                let gain_suggestion = if audio_peak > 0.95 {
                    Some((
                        "⚠️ Reduce gain — audio clipping!",
                        egui::Color32::from_rgb(220, 80, 80),
                        "Your signal is too strong and causing distortion. Lower the gain by 3–5 dB.",
                    ))
                } else if audio_peak > 0.85 {
                    Some((
                        "⚠️ Gain is high (clipping risk)",
                        egui::Color32::from_rgb(255, 180, 80),
                        "Signal is strong but approaching saturation. Consider reducing gain slightly.",
                    ))
                } else if snr_db > 0.0 && snr_db < 10.0 && state.source.gain_db < 40.0 {
                    Some((
                        "💡 Try increasing gain",
                        egui::Color32::from_rgb(100, 200, 255),
                        "Signal is weak. You might improve reception by increasing gain (up to 45 dB).",
                    ))
                } else {
                    None
                };

                if let Some((msg, color, tooltip)) = gain_suggestion {
                    ui.colored_label(color, msg)
                        .on_hover_text(tooltip);
                }
            }

            // Quick tuning checklist for beginners
            if let Ok(state) = self.shared.try_lock() {
                let is_running = state.source.status == crate::source_manager::SourceStatus::Running;
                let audio_on = state.audio_running;
                let gain_ok = state.source.gain_db >= 25.0 && state.source.gain_db <= 45.0;
                let snr = state.spectrum.peak_level() - state.spectrum.noise_floor();
                let signal_ok = snr > 8.0;

                let show_checklist = !is_running;
                if show_checklist {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("📋 Tuning Checklist").small().color(egui::Color32::from_rgb(180, 180, 100)));
                        ui.horizontal(|ui| {
                            let status_text = if is_running {
                                "✓ SDR running"
                            } else {
                                "⚠️ Press ▶ Start"
                            };
                            let color = if is_running { egui::Color32::GREEN } else { egui::Color32::YELLOW };
                            ui.colored_label(color, egui::RichText::new(status_text).small());
                            ui.separator();
                            let audio_text = if audio_on {
                                "✓ Audio on"
                            } else {
                                "⚠️ Start audio"
                            };
                            let color = if audio_on { egui::Color32::GREEN } else { egui::Color32::YELLOW };
                            ui.colored_label(color, egui::RichText::new(audio_text).small());
                            ui.separator();
                            let gain_text = if gain_ok {
                                "✓ Gain OK"
                            } else if state.source.gain_db < 25.0 {
                                "⚠️ Gain too low"
                            } else {
                                "⚠️ Gain too high"
                            };
                            let color = if gain_ok { egui::Color32::GREEN } else { egui::Color32::YELLOW };
                            ui.colored_label(color, egui::RichText::new(gain_text).small());
                            ui.separator();
                            let signal_text = if signal_ok && is_running {
                                "✓ Signal found"
                            } else if is_running {
                                "⚠️ No signal"
                            } else {
                                "❌ Start to check"
                            };
                            let color = if signal_ok && is_running { egui::Color32::GREEN } else { egui::Color32::RED };
                            ui.colored_label(color, egui::RichText::new(signal_text).small());
                        });
                    });
                }
            }
        }

        // S-Meter style signal strength indicator
        if let Ok(state) = self.shared.try_lock() {
            let peak = state.spectrum.peak_level();
            let noise = state.spectrum.noise_floor();
            let snr = peak - noise;

            // S-meter scale: S1-S9+ (standard amateur radio scale)
            // S9 = -73dBm relative, S units are roughly 6dB apart
            let s_value = if snr > 40.0 { 9 } else if snr > 34.0 { 8 }
                         else if snr > 28.0 { 7 } else if snr > 22.0 { 6 }
                         else if snr > 16.0 { 5 } else if snr > 10.0 { 4 }
                         else if snr > 4.0 { 3 } else if snr > -2.0 { 2 }
                         else if snr > 0.0 { 1 } else { 0 };

            let meter_color = match s_value {
                9 => egui::Color32::from_rgb(255, 0, 0),       // Red: +20 (very strong)
                8 => egui::Color32::from_rgb(255, 100, 0),     // Orange
                7 => egui::Color32::from_rgb(200, 200, 0),     // Yellow
                5..=6 => egui::Color32::from_rgb(0, 200, 0),  // Green
                3..=4 => egui::Color32::from_rgb(0, 150, 150), // Cyan
                _ => egui::Color32::from_rgb(100, 100, 100),  // Gray
            };

            ui.horizontal(|ui| {
                ui.label("📶 S-Meter:").on_hover_text("Signal strength meter (S0-S9+). S1-3 = weak, S4-6 = good, S7-9 = strong, S9+ = very strong.");

                // Visual bar
                let bar_width = ui.available_width().min(180.0).max(60.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_width, 14.0), egui::Sense::hover());
                let painter = ui.painter();

                painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(20, 20, 30));
                let fill_frac = (s_value as f32 / 9.0).clamp(0.0, 1.0);
                if fill_frac > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * fill_frac, rect.height())),
                        2.0, meter_color
                    );
                }

                let s_text = if s_value == 0 { "S0".to_string() } else if s_value == 9 { "S9+".to_string() } else { format!("S{}", s_value) };
                ui.colored_label(meter_color, format!("{} (SNR {:.0}dB)", s_text, snr))
                    .on_hover_text(format!("Signal strength: {} | Peak: {:.0}dBFS | Noise floor: {:.0}dBFS", s_text, peak, noise));
            });
        }

        if !is_beginner {
            // Signal logging: auto-record strong signals and track statistics
            if let Ok(state) = self.shared.try_lock() {
                let peak = state.spectrum.peak_level();
                let noise = state.spectrum.noise_floor();
                let snr = peak - noise;
                let current_freq = state.source.frequency_hz;

                // Track best SNR
                if snr > self.best_snr_this_session {
                    self.best_snr_this_session = snr;
                }

                // Track explored frequencies
                self.frequencies_explored.insert(current_freq);

                // Log if we found a new strong signal (SNR > 8dB and different frequency)
                if snr > 8.0 && current_freq != self.last_logged_freq {
                    self.signal_log.push((current_freq, snr, std::time::SystemTime::now()));
                    self.last_logged_freq = current_freq;
                    // Keep only last 20 signals
                    if self.signal_log.len() > 20 {
                        self.signal_log.remove(0);
                    }
                }
            }

            // Signal alert system
            if let Ok(state) = self.shared.try_lock() {
                let peak = state.spectrum.peak_level();
                let noise = state.spectrum.noise_floor();
                let snr = peak - noise;
                let current_freq = state.source.frequency_hz;

                // Check if a strong signal appeared
                if snr > self.signal_alert_threshold && current_freq != self.last_alert_freq && self.signal_alert_threshold > 0.0 {
                    self.last_alert_freq = current_freq;
                }

                // Auto-record indicator (actual recording control is in recorder panel)
                if self.auto_record_enabled && snr > self.auto_record_threshold && current_freq != self.last_auto_recorded_freq {
                    self.last_auto_recorded_freq = current_freq;
                }

                // Show alert indicator if threshold set and strong signal active
                if self.signal_alert_threshold > 0.0 && snr > self.signal_alert_threshold {
                    let freq_mhz = current_freq as f64 / 1e6;
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100),
                        format!("🔔 ALERT! Strong signal {:.3} MHz, SNR {:.0}dB", freq_mhz, snr))
                        .on_hover_text("A strong signal detected! Adjust threshold to change sensitivity.");
                }
            }

            // Alert and auto-record controls
            ui.horizontal(|ui| {
                ui.label("🔔 Alert Threshold:").on_hover_text("Get notified when a strong signal appears. Set to 0 to disable.");
                ui.add(egui::Slider::new(&mut self.signal_alert_threshold, 0.0..=30.0).text("dB").step_by(1.0))
                    .on_hover_text("Alert when SNR exceeds this value");
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.auto_record_enabled, "🎙️ Auto-Record").on_hover_text("Automatically record audio when strong signal detected");
                if self.auto_record_enabled {
                    ui.label("at SNR > ");
                    ui.add(egui::Slider::new(&mut self.auto_record_threshold, 10.0..=30.0).text("dB").step_by(1.0))
                        .on_hover_text("Auto-record when SNR exceeds this threshold");
                }
            });

            // Peak finder: find and auto-tune to strongest signal
            if let Ok(mut state) = self.shared.try_lock() {
                if ui.button("🔍 Find Peak: Auto-Tune to Strongest Signal").on_hover_text("Instantly jump to the strongest signal currently visible. Perfect for discovering what's on air!").clicked() {
                    let peak_freq = state.spectrum.peak_freq_hz();
                    if peak_freq > 0 {
                        self.tune_request = Some(peak_freq);
                    }
                }
            }

            // Display reception statistics
            let session_duration = self.session_start.elapsed().as_secs();
            let session_mins = session_duration / 60;
            let session_secs = session_duration % 60;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("⏱ {:.0}m{:.0}s", session_mins, session_secs)).small().color(egui::Color32::GRAY))
                    .on_hover_text("Session duration");
                ui.separator();
                ui.label(egui::RichText::new(format!("📍 {}", self.frequencies_explored.len())).small().color(egui::Color32::GRAY))
                    .on_hover_text("Frequencies explored");
                ui.separator();
                ui.label(egui::RichText::new(format!("📡 {}", self.signal_log.len())).small().color(egui::Color32::GRAY))
                    .on_hover_text("Signals detected");
                ui.separator();
                let best_color = if self.best_snr_this_session > 20.0 { egui::Color32::GREEN }
                                 else if self.best_snr_this_session > 10.0 { egui::Color32::YELLOW }
                                 else { egui::Color32::GRAY };
                ui.colored_label(best_color, egui::RichText::new(format!("🎯 {:.0}dB", self.best_snr_this_session)).small())
                    .on_hover_text("Best SNR this session");
            });

            // Display signal log
            if !self.signal_log.is_empty() {
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("📊 Signal Log ({})", self.signal_log.len())).small()
                )
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (freq_hz, snr, _time) in self.signal_log.iter().rev().take(8) {
                            let freq_mhz = *freq_hz as f64 / 1e6;
                            let btn_text = format!("{:.3}MHz ({:.0}dB)", freq_mhz, snr);
                            if ui.small_button(&btn_text)
                                .on_hover_text(format!("Tune to {:.6} MHz — SNR {:.1} dB", freq_mhz, snr))
                                .clicked()
                            {
                                if let Ok(mut state) = self.shared.try_lock() {
                                    state.source.frequency_hz = *freq_hz;
                                }
                            }
                        }
                        if ui.small_button("🗑").on_hover_text("Clear signal log").clicked() {
                            self.signal_log.clear();
                        }
                    });
                });
            }
        }

        if !is_beginner {
            // Nearest bookmark distance indicator
            if let Ok(state) = self.shared.try_lock() {
                let cur_freq = state.source.frequency_hz;
                if !state.bookmarks.bookmarks.is_empty() {
                    let nearest = state.bookmarks.bookmarks.iter()
                        .map(|b| (b, if b.frequency_hz > cur_freq { b.frequency_hz - cur_freq } else { cur_freq - b.frequency_hz }))
                        .min_by_key(|(_, d)| *d);
                    if let Some((bm, dist)) = nearest {
                        let threshold_hz = 100_000u64; // ±100 kHz
                        if dist > 0 && dist <= threshold_hz {
                            let dir = if bm.frequency_hz > cur_freq { "↑" } else { "↓" };
                            let dist_str = if dist >= 1000 { format!("{:.1} kHz", dist as f64 / 1000.0) } else { format!("{} Hz", dist) };
                            let is_very_close = dist <= 1_000; // Within 1 kHz
                            let label_color = if is_very_close {
                                egui::Color32::from_rgb(100, 220, 100) // bright green for very close
                            } else {
                                egui::Color32::from_rgb(180, 200, 255) // blue for nearby
                            };
                            let label_text = if is_very_close {
                                format!("🎯 {} ({}{}!)", bm.name, dir, dist_str)
                            } else {
                                format!("Near: {} ({}{} away)", bm.name, dir, dist_str)
                            };
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    label_color,
                                    label_text,
                                ).on_hover_text(format!("Bookmark '{}' at {:.4} MHz is {} {} — press B to snap to it.", bm.name, bm.frequency_hz as f64 / 1e6, dist_str, if bm.frequency_hz > cur_freq { "above" } else { "below" }));
                            });
                        }
                    }
                }
            }
            // Tuning step presets
            if let Ok(mut state) = self.shared.try_lock() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Step:").on_hover_text("Arrow key tuning step. ←→ = fine step, ↑↓ = coarse step. Shift multiplies by 10.");
                    for (label, hz, tip) in [
                        ("1k",  1_000u64,   "1 kHz step — for SSB/CW tuning"),
                        ("5k",  5_000,       "5 kHz step — NFM channel spacing (narrow)"),
                        ("8.33k",8_333,      "8.33 kHz step — ICAO aviation channel spacing"),
                        ("10k", 10_000,      "10 kHz step"),
                        ("12.5k",12_500,     "12.5 kHz step — NFM standard spacing"),
                        ("25k", 25_000,      "25 kHz step — wide NFM / older PMR"),
                        ("50k", 50_000,      "50 kHz step"),
                        ("100k",100_000,     "100 kHz step (default fine)"),
                        ("200k",200_000,     "200 kHz step — FM broadcast channel spacing"),
                        ("1M",  1_000_000,   "1 MHz step (default coarse)"),
                    ] {
                        let is_fine = state.tune_step_fine_hz == hz;
                        let is_coarse = state.tune_step_coarse_hz == hz;
                        let btn = ui.add(egui::Button::new(egui::RichText::new(label)
                            .color(if is_fine { egui::Color32::from_rgb(80, 200, 120) }
                                   else if is_coarse { egui::Color32::from_rgb(100, 160, 255) }
                                   else { egui::Color32::GRAY }))
                            .small())
                            .on_hover_text(format!("{} — click once: fine step (←→), click twice: coarse step (↑↓). Current fine: {} Hz, coarse: {} Hz.",
                                tip, state.tune_step_fine_hz, state.tune_step_coarse_hz));
                        if btn.clicked() {
                            if !is_fine {
                                state.tune_step_fine_hz = hz;
                            } else {
                                state.tune_step_coarse_hz = hz;
                            }
                        }
                    }

                    // Current step indicator
                    let fine_label = if state.tune_step_fine_hz >= 1_000_000 {
                        format!("{:.1}M", state.tune_step_fine_hz as f64 / 1e6)
                    } else {
                        format!("{:.0}k", state.tune_step_fine_hz as f64 / 1e3)
                    };
                    let coarse_label = if state.tune_step_coarse_hz >= 1_000_000 {
                        format!("{:.1}M", state.tune_step_coarse_hz as f64 / 1e6)
                    } else {
                        format!("{:.0}k", state.tune_step_coarse_hz as f64 / 1e3)
                    };
                    ui.colored_label(egui::Color32::GRAY, format!("←→:{} ↑↓:{}", fine_label, coarse_label))
                        .on_hover_text(format!("Fine step (← →): {} Hz. Coarse step (↑ ↓): {} Hz. Shift×10 for multiplier.",
                            state.tune_step_fine_hz, state.tune_step_coarse_hz));
                });
            }
            // Recent frequencies quick-access bar
            if let Ok(mut state) = self.shared.try_lock() {
                let cur = state.source.frequency_hz;
                // collect last 6 unique recent freqs that differ from current
                let mut recents: Vec<u64> = Vec::with_capacity(8);
                for &f in state.freq_history.iter().rev() {
                    if f != cur && !recents.contains(&f) {
                        recents.push(f);
                        if recents.len() >= 8 { break; }
                    }
                }
                if !recents.is_empty() {
                    let mut sorted = recents;
                    sorted.sort_by(|a, b| {
                        let da = if *a > cur { a - cur } else { cur - a };
                        let db = if *b > cur { b - cur } else { cur - b };
                        da.cmp(&db)
                    });
                    ui.horizontal(|ui| {
                        ui.small("Recent:");
                        for &f in sorted.iter().take(6) {
                            let label = if f >= 1_000_000_000 {
                                format!("{:.2}G", f as f64 / 1e9)
                            } else if f >= 100_000_000 {
                                format!("{:.1}M", f as f64 / 1e6)
                            } else {
                                format!("{:.3}M", f as f64 / 1e6)
                            };
                            if ui.small_button(egui::RichText::new(label).color(egui::Color32::from_rgb(160, 200, 255)))
                                .on_hover_text(format!("{:.6} MHz", f as f64 / 1e6))
                                .clicked()
                            {
                                state.source.frequency_hz = f;
                            }
                        }
                    });
                }
            }
        }
        // Direct frequency entry
        ui.horizontal(|ui| {
            ui.label("Go to:").on_hover_text("Type a frequency and press Enter to jump. Examples: 145.5 (MHz), 145500000 (Hz), 145500k (kHz).");
            let resp = ui.add(egui::TextEdit::singleline(&mut self.freq_input)
                .desired_width(140.0)
                .hint_text("MHz or Hz, Enter to jump"));
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let s = self.freq_input.trim().to_lowercase();
                let parsed_hz: Option<u64> = if let Ok(v) = s.trim_end_matches("mhz").trim().parse::<f64>() {
                    Some((v * 1e6) as u64)
                } else if s.ends_with("khz") || s.ends_with('k') {
                    s.trim_end_matches("khz").trim_end_matches('k').trim().parse::<f64>().ok().map(|v| (v * 1e3) as u64)
                } else if s.ends_with("ghz") || s.ends_with('g') {
                    s.trim_end_matches("ghz").trim_end_matches('g').trim().parse::<f64>().ok().map(|v| (v * 1e9) as u64)
                } else {
                    s.parse::<u64>().ok()
                };
                if let Some(hz) = parsed_hz {
                    let hz = hz.clamp(500_000, 1_770_000_000);
                    self.tune_request = Some(hz);
                    self.freq_input.clear();
                    self.freq_input_error.clear();
                    self.freq_input_error_time = None;
                } else if !s.is_empty() {
                    self.freq_input_error = format!("Could not parse '{}' — use MHz/kHz/GHz or raw Hz", s);
                    self.freq_input_error_time = Some(std::time::Instant::now());
                }
            }
        });
        // Show frequency entry error if recent
        if let Some(error_time) = self.freq_input_error_time {
            if error_time.elapsed().as_secs_f32() < 3.0 {
                let alpha = ((1.0 - error_time.elapsed().as_secs_f32() / 3.0) * 255.0) as u8;
                ui.colored_label(
                    egui::Color32::from_rgba_unmultiplied(220, 100, 80, alpha),
                    &self.freq_input_error
                );
            } else {
                self.freq_input_error_time = None;
            }
        }

        if !is_beginner {
            // Popular frequency bands quick-jump
            ui.collapsing("📻 Quick bands", |ui| {
                ui.horizontal_wrapped(|ui| {
                    if let Ok(mut state) = self.shared.try_lock() {
                        for (name, freq_hz, tip) in [
                            ("CB", 27_000_000u64, "Citizen Band (27 MHz)"),
                            ("2m", 145_500_000, "2-meter amateur band (145–146 MHz)"),
                            ("70cm", 435_000_000, "70-centimeter amateur band (430–440 MHz)"),
                            ("Airband", 118_000_000, "Aviation band (118–137 MHz)"),
                            ("Marine", 156_800_000, "Marine VHF (156–163 MHz)"),
                            ("NOAA", 137_500_000, "Weather satellites (137–138 MHz)"),
                            ("FM Bcast", 100_000_000, "FM radio (88–108 MHz)"),
                            ("800 MHz", 800_000_000, "800 MHz trunked radio"),
                            ("1.2G", 1_200_000_000, "1.2 GHz ISM / amateur"),
                        ] {
                            if ui.small_button(name)
                                .on_hover_text(tip)
                                .clicked()
                            {
                                state.source.frequency_hz = freq_hz;
                            }
                        }
                    }
                });
            });

            // Frequency memory display
            if let Ok(state) = self.shared.try_lock() {
                let has_memory = state.freq_memory.iter().any(|m: &crate::app::FreqMemEntry| m.freq_hz > 0);
                if has_memory {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("📝 Memory:").strong());
                        for (i, mem) in state.freq_memory.iter().enumerate() {
                            if mem.freq_hz > 0 {
                                let display = if mem.label.is_empty() {
                                    format!("M{}: {:.3}M", i + 1, mem.freq_hz as f64 / 1e6)
                                } else {
                                    format!("M{}: {}", i + 1, mem.label)
                                };
                                if ui.small_button(&display)
                                    .on_hover_text(format!("{:.4} MHz — click to recall, or press Alt+{} to recall, Alt+Shift+{} to save",
                                        mem.freq_hz as f64 / 1e6, i + 1, i + 1))
                                    .clicked()
                                {
                                    self.tune_request = Some(mem.freq_hz);
                                }
                            }
                        }
                    });
                }
            }

            ui.separator();

            // Memory label editor
            if let Ok(state) = self.shared.try_lock() {
                let has_memory = state.freq_memory.iter().any(|m| m.freq_hz > 0);
                if has_memory {
                    if ui.button("✏ Edit Memory Labels").on_hover_text("Click to customize names for your frequency memory slots (M1–M9)").clicked() {
                        self.show_memory_editor = !self.show_memory_editor;
                        if self.show_memory_editor {
                            for (i, mem) in state.freq_memory.iter().enumerate() {
                                self.memory_labels_edit[i] = mem.label.clone();
                            }
                        }
                    }
                    if self.show_memory_editor {
                        ui.label("Customize memory slot names:");
                        for (i, mem) in state.freq_memory.iter().enumerate() {
                            if mem.freq_hz > 0 {
                                ui.horizontal(|ui| {
                                    ui.label(format!("M{}:", i + 1)).on_hover_text(format!("{:.4} MHz", mem.freq_hz as f64 / 1e6));
                                    ui.text_edit_singleline(&mut self.memory_labels_edit[i]);
                                });
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Save").on_hover_text("Save custom labels for memory slots").clicked() {
                                if let Ok(mut state) = self.shared.try_lock() {
                                    for (i, mem) in state.freq_memory.iter_mut().enumerate() {
                                        if mem.freq_hz > 0 {
                                            mem.label = self.memory_labels_edit[i].clone();
                                        }
                                    }
                                }
                                self.show_memory_editor = false;
                            }
                            if ui.button("Cancel").clicked() {
                                self.show_memory_editor = false;
                            }
                        });
                    }
                }
            }

            ui.separator();
        }
    }

    pub fn ui_demod(&mut self, ui: &mut egui::Ui) {
        let user_level = self.shared.try_lock()
            .map(|s| crate::user_level::UserLevel::from_str(&s.config.user_level))
            .unwrap_or(crate::user_level::UserLevel::Beginner);
        let show_advanced = user_level.show_advanced_controls();
        let has_expand = user_level.has_inline_expand();
        let is_beginner = user_level.simplify_layout();

        // Demodulation Mode Quick Guide
        if ui.button("📖 Mode Guide").on_hover_text("Show a quick reference for what to expect in each demodulation mode").clicked() {
            self.show_mode_guide = !self.show_mode_guide;
        }

        if self.show_mode_guide {
            ui.group(|ui| {
                ui.label(egui::RichText::new("📖 Demodulation Mode Guide").small().color(egui::Color32::from_rgb(100, 200, 200)));
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("🎙️ Voice Modes:").small().strong());
                        ui.label("• AM: Crackly, clear voice (aviation, shortwave)");
                        ui.label("• NFM: Crystal clear voice (police, fire, repeaters)");
                        ui.label("• WFM: Stereo, music quality (FM radio broadcast)");
                        ui.label("• LSB/USB: Weak voice signals (HF, DX, weak stations)");
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("📊 Data Modes:").small().strong());
                        ui.label("• RAW: Digital pulses/ADSB (planes, weather, digital)");
                        ui.label("• CW: Dit-dit-dah morse code (ham radio, beacons)");
                    });
                });
            });
        }

        // Demod mode selector with bandwidth hints (wraps when tab is narrow)
        ui.horizontal_wrapped(|ui| {
            if let Ok(mut state) = self.shared.try_lock() {
                for (mode, bw_hint, _tip, detailed_tip) in [
                    (DemodMode::Raw, "",       "RAW I/Q", "Raw I/Q samples — pass to external decoders like GQRX plugins. No audio filtering."),
                    (DemodMode::Am,  "8 kHz",  "AM", "Amplitude Modulation with 8 kHz audio bandwidth. Aviation (118–137 MHz), AM broadcast, shortwave. Good for voice and morse."),
                    (DemodMode::Fm,  "12.5 k", "NFM", "Narrowband FM (12.5 kHz) for digital and voice. Land mobile radio: police, fire, repeaters, NOAA weather. Crystal clear voice."),
                    (DemodMode::Wfm, "200 k",  "WFM", "Wideband FM (200 kHz) with full audio fidelity and stereo. FM broadcast (88–108 MHz). Music broadcasts, RDS data included."),
                    (DemodMode::Lsb, "2.4 k",  "LSB", "Lower Sideband (2.4 kHz audio) for HF below 10 MHz. Amateur voice, CW, digital modes. SSB efficiency uses half the spectrum."),
                    (DemodMode::Usb, "2.4 k",  "USB", "Upper Sideband (2.4 kHz audio) for HF above 10 MHz, utility, military. Narrow bandwidth for weak signal DX."),
                ] {
                    let selected = state.demod_mode == mode;
                    let label = if bw_hint.is_empty() {
                        mode.label().to_string()
                    } else {
                        format!("{} {}", mode.label(), bw_hint)
                    };
                    if ui.selectable_label(selected, label)
                        .on_hover_text(detailed_tip)
                        .clicked()
                    {
                        state.demod_mode = mode;
                    }
                }
            }
        });

        if !is_beginner {
            // Band-aware demod auto-suggest
            if let Ok(mut state) = self.shared.try_lock() {
                let freq = state.source.frequency_hz;
                let current = state.demod_mode;
                if let Some((suggested, band_name, reason)) = suggest_demod_for_freq(freq) {
                    if suggested != current {
                        ui.horizontal(|ui| {
                            ui.colored_label(egui::Color32::from_rgb(255, 200, 50), "💡");
                            ui.label(egui::RichText::new(format!("{} →", band_name)).color(egui::Color32::from_rgb(180, 180, 180)));
                            if ui.small_button(suggested.label())
                                .on_hover_text(format!("Switch to {} — {}", suggested.label(), reason))
                                .clicked()
                            {
                                state.demod_mode = suggested;
                            }
                            ui.label(egui::RichText::new(format!("({})", reason)).color(egui::Color32::GRAY).small());
                        });
                    }
                }
            }
        }

        // Signal meter + SNR
        ui.separator();
        if let Ok(state) = self.shared.try_lock() {
            let signal = state.spectrum.signal_level();
            let noise_floor = state.spectrum.noise_floor();
            let peak = state.spectrum.peak_level();
            let snr = peak - noise_floor;
            let norm = ((signal + 120.0) / 120.0).clamp(0.0, 1.0);
            // VU-style signal meter
            ui.horizontal(|ui| {
                ui.label("Signal:").on_hover_text("RF signal level in dBFS. Green zone (>-40 dB) = strong. Yellow (−60–40) = moderate. Red (<-60) = weak/noise.");
                // Draw custom colored bar using painter
                let bar_w = ui.available_width().min(200.0).max(60.0);
                let (rect, response) = ui.allocate_exact_size(egui::vec2(bar_w, 14.0), egui::Sense::hover());
                let response = response.on_hover_text(format!("Signal: {:.1} dBFS  SNR: {:.1} dB  Noise: {:.1} dB", signal, snr, noise_floor));
                let p = ui.painter();
                p.rect_filled(rect, 2.0, egui::Color32::from_rgb(15, 15, 25));
                // Zones: 0–50% red, 50–75% yellow, 75–100% green
                let zones = [
                    (0.0f32, 0.5f32, egui::Color32::from_rgb(150, 30, 30)),
                    (0.5f32, 0.75f32, egui::Color32::from_rgb(180, 150, 20)),
                    (0.75f32, 1.0f32, egui::Color32::from_rgb(30, 150, 50)),
                ];
                for (lo, hi, c) in &zones {
                    let fill = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + lo * rect.width(), rect.top()),
                        egui::pos2(rect.left() + hi * rect.width(), rect.bottom()),
                    );
                    p.rect_filled(fill, 0.0, egui::Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), 40));
                }
                // Filled bar up to signal level
                let fill_w = norm * rect.width();
                let fill_color = if norm > 0.75 { egui::Color32::from_rgb(50, 200, 80) }
                    else if norm > 0.5 { egui::Color32::from_rgb(220, 180, 30) }
                    else { egui::Color32::from_rgb(200, 50, 50) };
                p.rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height())),
                    2.0, fill_color,
                );
                // Tick marks every 20 dB
                for db in (-120..=0i32).step_by(20) {
                    let t = ((db as f32 - (-120.0)) / 120.0).clamp(0.0, 1.0);
                    let x = rect.left() + t * rect.width();
                    p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(0.5, egui::Color32::from_gray(80)));
                }
                // Level text
                p.text(egui::pos2(rect.right() - 2.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{:.0}dB", signal),
                    egui::FontId::monospace(9.0),
                    egui::Color32::WHITE);
                let _ = response;

                ui.separator();
                let snr_color = if snr > 20.0 { egui::Color32::GREEN }
                    else if snr > 10.0 { egui::Color32::YELLOW }
                    else { egui::Color32::GRAY };
                let (quality_str, quality_tip) = if snr > 25.0 {
                    ("● Excellent", "SNR >25 dB: very clean signal, audio will be clear.")
                } else if snr > 15.0 {
                    ("● Good", "SNR 15–25 dB: good reception, audio should be clean.")
                } else if snr > 8.0 {
                    ("● Marginal", "SNR 8–15 dB: weak signal, audio may be noisy or choppy.")
                } else {
                    ("● No signal", "SNR <8 dB: no meaningful signal at this frequency. Try moving the antenna, increasing gain, or tuning to a different frequency.")
                };
                ui.colored_label(snr_color, format!("SNR {:.1} dB", snr))
                    .on_hover_text("Signal-to-Noise Ratio: peak dB minus estimated noise floor. >20 dB = excellent, 10–20 dB = good, <10 dB = marginal. Aim for >15 dB for clean audio.");
                ui.colored_label(snr_color, quality_str).on_hover_text(quality_tip);
            });
        }

        if !is_beginner {
            // Signal history sparkline
            if let Ok(state) = self.shared.try_lock() {
                let history = state.spectrum.signal_history_snapshot();
                let history_max = state.spectrum.signal_history_max();
                if history.len() >= 2 {
                    let spark_w = ui.available_width().min(200.0).max(60.0);
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(spark_w, 30.0), egui::Sense::hover());
                    let response = response.on_hover_text("Signal strength over time (last 60s). Shows peaks only. Helps identify if a signal is continuous, periodic, or intermittent.");
                    let p = ui.painter();
                    p.rect_filled(rect, 2.0, egui::Color32::from_rgb(10, 10, 20));
                    let n = history.len();
                    let min_db = -120.0f32;
                    let max_db = 0.0f32;
                    let db_range = max_db - min_db;
                    let points: Vec<egui::Pos2> = history.iter().enumerate().map(|(i, &db)| {
                        let x = rect.left() + (i as f32 / (history_max as f32 - 1.0).max(1.0)) * rect.width();
                        let norm = ((db - min_db) / db_range).clamp(0.0, 1.0);
                        let y = rect.bottom() - norm * rect.height();
                        egui::pos2(x, y)
                    }).collect();
                    for win in points.windows(2) {
                        let norm = (win[1].y - rect.top()) / rect.height();
                        let c = if norm < 0.25 { egui::Color32::from_rgb(50, 200, 80) }
                            else if norm < 0.5 { egui::Color32::from_rgb(180, 160, 30) }
                            else { egui::Color32::from_rgb(100, 60, 200) };
                        p.line_segment([win[0], win[1]], egui::Stroke::new(1.0, c));
                    }
                    p.text(egui::pos2(rect.left() + 2.0, rect.top() + 2.0),
                        egui::Align2::LEFT_TOP, "60s",
                        egui::FontId::monospace(8.0), egui::Color32::from_gray(100));
                    let _ = response;
                    let _ = n;
                }
            }
        }

        if !is_beginner {
            // Overload detection + smart gain
            if let Ok(mut state) = self.shared.try_lock() {
                let peak = state.spectrum.peak_level();
                let gain = state.source.gain_db;
                if peak > -15.0 && gain > 0.0 {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(255, 80, 80),
                            format!("⚠ Overload! Peak {:.0} dBFS", peak))
                            .on_hover_text("Signal is clipping the ADC — this causes distortion, ghost signals, and desensitization. Reduce gain.");
                        if ui.small_button("-10 dB")
                            .on_hover_text("Reduce gain by 10 dB to eliminate overload.")
                            .clicked()
                        {
                            state.source.gain_db = (gain - 10.0).max(0.0);
                        }
                    });
                }
                if ui.small_button("Smart Gain")
                    .on_hover_text(format!(
                        "Auto-adjust gain to target -30 dBFS peak. Current peak: {:.0} dBFS, current gain: {:.1} dB.",
                        peak, gain
                    ))
                    .clicked()
                {
                    let adjustment = -30.0 - peak as f64;
                    state.source.gain_db = (gain + adjustment).clamp(0.0, 49.6);
                }
                // Gain presets
                ui.horizontal_wrapped(|ui| {
                    ui.label("Gain:").on_hover_text("Quick gain presets for common scenarios.");
                    for (label, db, tip) in [
                        ("Low",  10.0f64, "Low gain (10 dB) — for very strong nearby transmitters or when overloading."),
                        ("Med",  28.0, "Medium gain (28 dB) — good starting point for most environments."),
                        ("High", 42.0, "High gain (42 dB) — for weak distant signals. Watch for overload."),
                        ("Max",  49.6, "Maximum gain (49.6 dB) — only for very weak signals. High overload risk."),
                    ] {
                        let is_active = (state.source.gain_db - db).abs() < 0.5;
                        let btn = ui.add(egui::Button::new(egui::RichText::new(label).small()
                            .color(if is_active { egui::Color32::BLACK } else { egui::Color32::from_rgb(200, 220, 200) }))
                            .fill(if is_active { egui::Color32::from_rgb(60, 160, 80) } else { egui::Color32::from_rgba_premultiplied(30, 50, 30, 80) })
                            .small())
                            .on_hover_text(tip);
                        if btn.clicked() { state.source.gain_db = db; }
                    }
                });

                // Gain optimization suggestion
                {
                    let signal_level = state.spectrum.signal_level();
                    let current_gain = state.source.gain_db;
                    let (suggestion, tip_color) = if signal_level > 0.0 {
                        ("⚠ Overload!", egui::Color32::from_rgb(220, 80, 80))
                    } else if signal_level > -20.0 {
                        ("✓ Good level", egui::Color32::from_rgb(100, 200, 80))
                    } else if signal_level > -60.0 && current_gain < 45.0 {
                        ("↑ Try higher gain", egui::Color32::from_rgb(200, 200, 80))
                    } else if signal_level < -80.0 && current_gain >= 45.0 {
                        ("↓ Max gain, still weak", egui::Color32::from_rgb(200, 150, 80))
                    } else {
                        ("", egui::Color32::GRAY)
                    };
                    if !suggestion.is_empty() {
                        ui.colored_label(tip_color, suggestion)
                            .on_hover_text("Gain indicator based on current signal level. Adjust gain for best reception without overload.");
                    }

                }
            }
        }

        // PPM correction quick presets (hidden for Beginner/Intermediate unless expanded)
        let show_ppm = show_advanced;
        if show_ppm || (has_expand && self.expand_ppm) {
            if let Ok(mut state) = self.shared.try_lock() {
                ui.horizontal(|ui| {
                    ui.label("PPM:").on_hover_text("Parts-per-million frequency error correction. RTL-SDR chips often have ±25 PPM error.");
                    for (label, ppm) in [("0", 0i32), ("+10", 10), ("+25", 25), ("-10", -10), ("-25", -25)] {
                        let is_active = state.source.ppm_correction == ppm;
                        let btn = ui.add(egui::Button::new(egui::RichText::new(label).small()
                            .color(if is_active { egui::Color32::BLACK } else { egui::Color32::GRAY }))
                            .fill(if is_active { egui::Color32::from_rgb(100, 180, 255) } else { egui::Color32::from_rgba_premultiplied(30, 40, 60, 60) })
                            .small())
                            .on_hover_text(format!("Set frequency correction to {} PPM", ppm));
                        if btn.clicked() {
                            state.source.ppm_correction = ppm;
                        }
                    }
                });
            }
        } else if has_expand {
            if ui.button(egui::RichText::new("⚙ Show PPM (advanced)").size(12.0)).clicked() {
                self.expand_ppm = true;
            }
        }

        if !is_beginner {
            // Demod quality indicators
            if let Ok(state) = self.shared.try_lock() {
                let mode = state.demod_mode;
                if mode == DemodMode::Fm || mode == DemodMode::Wfm {
                    let dev_khz = state.fm_deviation_hz / 1000.0;
                    let (dev_color, dev_tip) = match mode {
                        DemodMode::Fm => {
                            if dev_khz > 13.0 {
                                (egui::Color32::RED, "NFM deviation too high (>13 kHz) — signal clipping/overmodulation")
                            } else if dev_khz >= 4.5 && dev_khz <= 12.5 {
                                (egui::Color32::GREEN, "NFM deviation in ideal range (4.5–12.5 kHz)")
                            } else if dev_khz >= 2.0 && dev_khz < 4.5 {
                                (egui::Color32::YELLOW, "NFM deviation low (2–4.5 kHz) — weak signal?")
                            } else {
                                (egui::Color32::GRAY, "NFM deviation too low (<2 kHz)")
                            }
                        },
                        DemodMode::Wfm => {
                            if dev_khz > 80.0 {
                                (egui::Color32::RED, "WFM deviation excessive (>80 kHz)")
                            } else if dev_khz >= 50.0 && dev_khz <= 75.0 {
                                (egui::Color32::GREEN, "WFM deviation ideal (50–75 kHz)")
                            } else if dev_khz >= 30.0 && dev_khz < 50.0 {
                                (egui::Color32::YELLOW, "WFM deviation low (30–50 kHz) — weak signal")
                            } else {
                                (egui::Color32::GRAY, "WFM deviation very low")
                            }
                        },
                        _ => (egui::Color32::GRAY, "N/A"),
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(dev_color, format!("FM dev: {:.1} kHz", dev_khz))
                            .on_hover_text(dev_tip);
                    });
                    // Audio level meter bar with peak hold
                    let peak_frac = (state.audio_peak).clamp(0.0, 1.0);
                    if peak_frac > self.audio_peak_hold {
                        self.audio_peak_hold = peak_frac;
                        self.audio_peak_hold_time = Some(std::time::Instant::now());
                    } else if let Some(held_since) = self.audio_peak_hold_time {
                        if held_since.elapsed().as_secs_f32() > 3.0 {
                            self.audio_peak_hold = 0.0;
                            self.audio_peak_hold_time = None;
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("Audio:");
                        let bar_w = ui.available_width().min(150.0).max(40.0);
                        let bar_h = 10.0f32;
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(20, 20, 30));
                        let fill_w = rect.width() * peak_frac;
                        let bar_color = if peak_frac > 0.9 { egui::Color32::from_rgb(220, 50, 50) }
                            else if peak_frac > 0.6 { egui::Color32::from_rgb(50, 200, 80) }
                            else if peak_frac > 0.05 { egui::Color32::from_rgb(40, 160, 60) }
                            else { egui::Color32::from_rgb(50, 70, 50) };
                        if fill_w > 0.5 {
                            painter.rect_filled(
                                egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, bar_h)),
                                2.0, bar_color,
                            );
                        }
                        if self.audio_peak_hold > 0.01 {
                            let peak_x = rect.left() + rect.width() * self.audio_peak_hold;
                            painter.line_segment(
                                [egui::pos2(peak_x, rect.top()), egui::pos2(peak_x, rect.bottom())],
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 200, 100)),
                            );
                        }
                        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 70, 80)), egui::StrokeKind::Middle);
                        resp.on_hover_text(format!("Audio output level: {:.0}%. Peak hold (orange line): {:.0}%. >90% = clipping risk — lower volume.", peak_frac * 100.0, self.audio_peak_hold * 100.0));
                    });
                }
            }
        }

        // Audio controls
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let mut start_audio = false;
            let mut stop_audio = false;
            let mut volume = 0.5f32;
            let mut audio_running = false;
            if let Ok(state) = self.shared.try_lock() {
                volume = state.volume;
                audio_running = state.audio_running;
            }
            if audio_running {
                if ui.button("🔇 Stop Audio").on_hover_text("Stop audio playback through your speakers/headphones.").clicked() {
                    stop_audio = true;
                }
            } else {
                if ui.button("🔊 Start Audio").on_hover_text("Start playing the demodulated signal through your speakers/headphones.").clicked() {
                    start_audio = true;
                }
            }
            let mut vol = volume;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Vol"))
                .on_hover_text("Audio output volume. Does not affect the RF signal — only the speaker output level.")
                .changed()
            {
                volume = vol;
            }
            // Volume presets
            for (label, preset_vol) in [("Mute", 0.0f32), ("25%", 0.25), ("50%", 0.50), ("75%", 0.75), ("Max", 1.0)] {
                if ui.small_button(label)
                    .on_hover_text(format!("Set volume to {}", label))
                    .clicked()
                {
                    volume = preset_vol;
                    vol = preset_vol;
                }
            }
            if start_audio || stop_audio || vol != volume {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.volume = volume;
                    if start_audio { state.audio_running = true; }
                    else if stop_audio { state.audio_running = false; }
                }
            }
        });

        ui.separator();

        if !is_beginner {
            // Frequency presets (band quick-tune) — one-click: tune + mode + gain + filter BW + start audio
            ui.horizontal_wrapped(|ui| {
                ui.label("Bands:").on_hover_text("One-click quick-start presets. Each button tunes to that frequency, picks the right demodulation mode, sets a sensible gain and filter bandwidth, and starts audio — so you hear sound immediately.");
                const BANDS: &[(&str, u64, &str, f64, u32, &str)] = &[
                    ("BC FM",   100_000_000, "WFM",  40.0, 200_000, "FM Broadcast band center (88–108 MHz) → WFM mode. Hear music/talk radio."),
                    ("Air",     118_000_000, "AM",   40.0, 8_000,   "Aviation VHF voice band (118–137 MHz) → AM mode (not FM!). Hear air traffic control."),
                    ("NOAA WX", 162_400_000, "NFM",  40.0, 12_500,  "NOAA Weather Radio (162.400–162.550 MHz) → NFM mode. Continuous weather broadcast."),
                    ("Marine",  156_800_000, "NFM",  40.0, 12_500,  "Marine VHF distress channel 16 (156.8 MHz) → NFM mode."),
                    ("2m",      144_000_000, "NFM",  40.0, 12_500,  "Amateur 2-meter band. Repeaters, APRS at 144.390 MHz → NFM"),
                    ("APRS",    144_390_000, "NFM",  40.0, 12_500,  "APRS digipeater/tracker beacon (144.390 MHz US) → NFM mode"),
                    ("70cm",    430_000_000, "NFM",  42.0, 12_500,  "Amateur 70cm band. FM repeaters, digital modes → NFM"),
                    ("PMR446",  446_006_250, "NFM",  42.0, 12_500,  "PMR446 licence-free walkie-talkies (446.006–446.194 MHz) → NFM"),
                    ("NOAA 15", 137_620_000, "WFM",  45.0, 200_000, "NOAA 15 weather satellite (137.620 MHz) → WFM 34 kHz"),
                    ("NOAA 18", 137_912_500, "WFM",  45.0, 200_000, "NOAA 18 weather satellite (137.9125 MHz) → WFM 34 kHz"),
                    ("NOAA 19", 137_100_000, "WFM",  45.0, 200_000, "NOAA 19 weather satellite (137.100 MHz) → WFM 34 kHz"),
                    ("ISS",     145_800_000, "NFM",  45.0, 12_500,  "International Space Station voice (145.800 MHz) → NFM"),
                    ("ADS-B",  1_090_000_000, "RAW", 40.0, 250_000, "Mode-S/ADS-B aircraft transponders (1090 MHz) → RAW (use ADS-B tab)"),
                ];
                if let Ok(mut state) = self.shared.try_lock() {
                    for (name, freq_hz, mode_str, gain_db, filter_bw, tip) in BANDS {
                        let mode_color = match *mode_str {
                            "WFM" => egui::Color32::from_rgb(100, 200, 100),
                            "NFM" => egui::Color32::from_rgb(150, 150, 255),
                            "AM"  => egui::Color32::from_rgb(255, 200, 100),
                            "RAW" => egui::Color32::from_rgb(150, 150, 150),
                            _ => egui::Color32::WHITE,
                        };
                        let label = egui::RichText::new(format!("{} {}", name, mode_str)).color(mode_color).small();
                        if ui.small_button(label).on_hover_text(*tip).clicked() {
                            state.source.frequency_hz = *freq_hz;
                            if let Some(mode) = DemodMode::from_label(mode_str) {
                                state.demod_mode = mode;
                            }
                            state.source.gain_db = *gain_db;
                            state.audio_running = true;
                            self.filter_bw = *filter_bw;
                            state.lpf_cutoff = match *mode_str {
                                "WFM" => 15000.0,
                                "NFM" | "AM" => 3000.0,
                                _ => state.lpf_cutoff,
                            };
                        }
                    }
                }
            });

            // Band info hint — tells beginner what they're likely listening to at current freq
            if let Ok(state) = self.shared.try_lock() {
                let freq = state.source.frequency_hz;
                const BAND_INFO: &[(u64, u64, &str, (u8, u8, u8))] = &[
                    (148_000,    530_000,    "LW/MW broadcast. Amplitude-modulated radio stations, aviation beacons (NDB).", (180, 160, 100)),
                    (530_000,  1_710_000,   "AM broadcast band. Local radio stations. Use AM mode.", (180, 160, 100)),
                    (1_710_000, 30_000_000, "HF shortwave. International broadcast, amateur radio (use SSB), maritime, military.", (100, 180, 255)),
                    (87_500_000, 108_000_000, "FM broadcast band. Local music/talk radio stations. Use WFM mode.", (80, 200, 120)),
                    (108_000_000, 118_000_000, "VOR/ILS navigation aids. Aircraft instrument approaches. AM mode.", (200, 200, 80)),
                    (118_000_000, 137_000_000, "Aviation VHF band. Air traffic control, ATIS, ground. Use AM mode (not FM!).", (200, 200, 80)),
                    (136_000_000, 138_000_000, "Weather satellite downlink (NOAA APT at 137.1–137.9 MHz). Use WFM or RAW.", (100, 220, 220)),
                    (144_000_000, 148_000_000, "Amateur 2m band. FM voice repeaters, APRS (144.390 MHz). Use NFM.", (160, 120, 255)),
                    (156_000_000, 174_000_000, "Marine VHF. Channel 16 (distress) = 156.8 MHz. Use NFM.", (80, 160, 255)),
                    (162_400_000, 162_600_000, "NOAA Weather Radio. Continuous weather broadcasts. Use NFM.", (100, 220, 220)),
                    (430_000_000, 440_000_000, "Amateur 70cm band. FM repeaters, ATV, digital. Use NFM.", (160, 120, 255)),
                    (433_050_000, 434_790_000, "433 MHz ISM band. Remote controls, key fobs, weather stations. Use NFM/AM.", (200, 140, 80)),
                    (446_000_000, 446_200_000, "PMR446 walkie-talkies (licence-free). Use NFM.", (200, 140, 80)),
                    (460_000_000, 470_000_000, "UHF land mobile. Business radios, public safety (varies by country). Use NFM.", (160, 160, 160)),
                    (850_000_000, 900_000_000, "GSM 850 / cellular. Digital — you'll see wideband signal but no decodable audio.", (120, 120, 120)),
                    (1_090_000_000, 1_090_000_000, "ADS-B Mode-S (1090 MHz). Aircraft transponders — use ADS-B tab with RAW mode.", (80, 200, 255)),
                    (1_575_420_000, 1_575_420_000, "GPS L1 signal (1575.42 MHz). Very weak — needs a GPS LNA to receive.", (120, 200, 80)),
                ];
                let band_desc = BAND_INFO.iter().find(|(lo, hi, _, _)| {
                    if lo == hi { freq.abs_diff(*lo) < 500_000 } else { freq >= *lo && freq <= *hi }
                });
                if let Some((_, _, desc, (r, g, b))) = band_desc {
                    ui.horizontal(|ui| {
                        ui.label("📡").on_hover_text("Band information for the current frequency.");
                        ui.colored_label(egui::Color32::from_rgb(*r, *g, *b), *desc);
                    });
                }
            }

            // Recent frequencies (last 8 from history, most recent first)
            if let Ok(mut state) = self.shared.try_lock() {
                if state.freq_history.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Recent:").on_hover_text("Last tuned frequencies — click to jump back.");
                        let hist: Vec<u64> = state.freq_history.iter().cloned().rev().skip(1).take(8).collect();
                        for freq in hist {
                            let label = format!("{:.3}", freq as f64 / 1e6);
                            ui.horizontal(|ui| {
                                ui.set_width_range(0.0..=120.0);
                                if ui.small_button(&label).on_hover_text(format!("{:.3} MHz — click to retune", freq as f64 / 1e6)).clicked() {
                                    state.source.frequency_hz = freq;
                                    return;
                                }
                                if ui.small_button("📋").on_hover_text("Copy frequency").clicked() {
                                    ui.ctx().copy_text(format!("{:.6}", freq as f64 / 1e6));
                                }
                            });
                        }
                        if ui.small_button("🗑").on_hover_text("Clear all frequency history").clicked() {
                            state.freq_history.clear();
                        }
                    });
                }
            }
        }

        if !is_beginner {
            // Filter bandwidth controls (hidden for beginners)
            if let Ok(mut state) = self.shared.try_lock() {
                let _bw_resp = ui.add(egui::Slider::new(&mut self.filter_bw, 100..=250_000).text("Filter BW (Hz)").logarithmic(true))
                    .on_hover_text("Receiver filter bandwidth. Set just wider than the signal. WFM: 200 kHz, NFM voice: 12–16 kHz, AM voice: 8 kHz, SSB: 2.4 kHz. Too wide = more noise.");

                let (suggested_hz, tip) = match state.demod_mode {
                    DemodMode::Raw => (0, "RAW: no filter applied"),
                    DemodMode::Am => (8_000, "AM: 8 kHz typical for voice"),
                    DemodMode::Fm => (12_500, "NFM: 12.5 kHz standard"),
                    DemodMode::Wfm => (200_000, "WFM: 200 kHz for stereo broadcast"),
                    DemodMode::Lsb | DemodMode::Usb => (2_400, "SSB: 2.4 kHz for voice"),
                };
                if suggested_hz > 0 && (self.filter_bw as i32 - suggested_hz as i32).abs() > 1000 {
                    let suggestion_color = if self.filter_bw < suggested_hz {
                        egui::Color32::from_rgb(180, 200, 100)
                    } else {
                        egui::Color32::from_rgb(100, 150, 255)
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(suggestion_color, format!("💡 {}: {}", state.demod_mode.label(), format_hz(suggested_hz)))
                            .on_hover_text(tip);
                        if ui.small_button("Apply").on_hover_text(format!("Set filter to {} Hz", suggested_hz)).clicked() {
                            self.filter_bw = suggested_hz as u32;
                        }
                    });
                }

                ui.horizontal(|ui| {
                    ui.label("RF Filter Presets:").on_hover_text("Quick filter bandwidth presets optimized for common modes");
                    let presets = [
                        ("Voice", 12_500u32, "12.5 kHz - NFM voice"),
                        ("AM Bcast", 8_000, "8 kHz - AM radio"),
                        ("FM Bcast", 200_000, "200 kHz - WFM stereo"),
                        ("SSB", 2_400, "2.4 kHz - SSB voice"),
                        ("CW", 500, "500 Hz - Morse code"),
                    ];
                    for (label, bw, tooltip) in presets.iter() {
                        if ui.small_button(*label).on_hover_text(*tooltip).clicked() {
                            self.filter_bw = *bw;
                        }
                    }
                });

                ui.add(egui::Slider::new(&mut state.lpf_cutoff, 100.0..=20000.0).text("Audio LPF (Hz)").logarithmic(true))
                    .on_hover_text("Low-pass filter on audio output. Cuts high-frequency hiss above this frequency. Default 15 kHz is fine for voice. Lower for CW/Morse (~800 Hz).");

                ui.horizontal(|ui| {
                    ui.label("Presets:").on_hover_text("Quick audio filter presets");
                    for (label, hz, tip) in [
                        ("CW", 800.0f32, "Morse/CW: 800 Hz narrow filter"),
                        ("Voice", 3000.0, "Voice: 3 kHz standard"),
                        ("Music", 8000.0, "Music/broadcast: 8 kHz"),
                        ("Wide", 15000.0, "Default: 15 kHz"),
                    ] {
                        if ui.small_button(label).on_hover_text(tip).clicked() {
                            state.lpf_cutoff = hz;
                        }
                    }
                });
            }
        }
        // Auto-squelch tracking: update squelch every frame when enabled
        if self.auto_squelch {
            if let Ok(mut state) = self.shared.try_lock() {
                let noise = state.spectrum.noise_floor();
                if noise < -30.0 {
                    let tracked = (noise + self.auto_squelch_offset).min(0.0);
                    self.squelch = tracked;
                    state.squelch = tracked;
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            let sq_resp = ui.add(egui::Slider::new(&mut self.squelch, -120.0..=0.0).text("Squelch (dB)"))
                .on_hover_text("Signal level threshold. Audio is muted when signal drops below this value, silencing static between transmissions. Set ~5 dB above your noise floor.");

            // Show squelch relative to noise floor
            if let Ok(state) = self.shared.try_lock() {
                let noise = state.spectrum.noise_floor();
                let offset = self.squelch - noise;
                let offset_color = if offset < 0.0 {
                    egui::Color32::from_rgb(220, 100, 80) // red: below noise floor
                } else if offset < 3.0 {
                    egui::Color32::from_rgb(220, 180, 80) // orange: too tight
                } else if offset > 20.0 {
                    egui::Color32::from_rgb(180, 150, 255) // purple: possibly too high
                } else {
                    egui::Color32::from_rgb(100, 200, 100) // green: ideal
                };
                ui.colored_label(offset_color, format!("+{:.1}dB", offset))
                    .on_hover_text(format!("Squelch is {:.1} dB above noise floor ({:.1} dB). Ideal: 3–10 dB above floor.", offset, noise));

                // Squelch open indicator
                let signal_level = state.spectrum.signal_level();
                let is_open = signal_level > self.squelch;
                let indicator_text = if is_open { "◉ OPEN" } else { "◉ closed" };
                let indicator_color = if is_open {
                    egui::Color32::from_rgb(80, 220, 120)
                } else {
                    egui::Color32::GRAY
                };
                ui.colored_label(indicator_color, indicator_text)
                    .on_hover_text(format!("Squelch status: signal {:.1} dB {} threshold {:.1} dB",
                        signal_level, if is_open { "above" } else { "below" }, self.squelch));
            }

            if sq_resp.changed() || ui.input(|i| i.pointer.any_down()) {
                self.auto_squelch = false;
                if let Ok(mut state) = self.shared.try_lock() {
                    state.squelch = self.squelch;
                }
            }
            if ui.small_button("Auto").on_hover_text("Set squelch to offset dB above current noise floor (one-shot).").clicked() {
                self.auto_squelch = false;
                if let Ok(mut state) = self.shared.try_lock() {
                    let noise = state.spectrum.noise_floor();
                    let auto_sq = (noise + self.auto_squelch_offset).min(0.0);
                    self.squelch = auto_sq;
                    state.squelch = auto_sq;
                }
            }
            let track_label = if self.auto_squelch {
                egui::RichText::new("Track ON").color(egui::Color32::from_rgb(80, 220, 120))
            } else {
                egui::RichText::new("Track")
            };
            if ui.small_button(track_label)
                .on_hover_text("Continuously track the noise floor and keep squelch at floor + offset. Adjusts automatically as conditions change.")
                .clicked()
            {
                self.auto_squelch = !self.auto_squelch;
            }
            ui.add(egui::DragValue::new(&mut self.auto_squelch_offset)
                .speed(0.5)
                .range(0.0..=30.0)
                .suffix(" dB offset"))
                .on_hover_text("How many dB above the noise floor to set squelch when using Auto or Track.");
            if ui.small_button("Off").on_hover_text("Disable squelch — audio always passes regardless of signal strength.").clicked() {
                self.auto_squelch = false;
                self.squelch = -120.0;
                if let Ok(mut state) = self.shared.try_lock() {
                    state.squelch = -120.0;
                }
            }
        });

        // Squelch presets row
        ui.horizontal(|ui| {
            ui.label("Squelch presets:").on_hover_text("Quick squelch level adjustment");
            for (label, level_db) in [
                ("Very Loose", -100.0f32),
                ("Loose", -80.0),
                ("Normal", -60.0),
                ("Tight", -40.0),
                ("Very Tight", -20.0),
            ] {
                if ui.small_button(label)
                    .on_hover_text(format!("Set squelch to {} dB", level_db))
                    .clicked()
                {
                    self.squelch = level_db;
                    self.auto_squelch = false;
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.squelch = level_db;
                    }
                }
            }
            if ui.small_button("📋").on_hover_text("Copy current squelch value to clipboard.").clicked() {
                ui.ctx().copy_text(format!("{:.1}", self.squelch));
            }
        });

        // Frequency identification
        if let Ok(state) = self.shared.try_lock() {
            let freq = state.source.frequency_hz;
            let audio_running = state.audio_running;
            if let Some(info) = identify_frequency(freq) {
                ui.separator();
                ui.collapsing(format!("📻 {} — {}", info.band, info.short_desc), |ui| {
                    ui.label(egui::RichText::new(info.detail).color(egui::Color32::from_rgb(200, 200, 200)));
                    if !info.tips.is_empty() {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new(format!("💡 {}", info.tips)).small().color(egui::Color32::GRAY));
                    }
                });
                if !info.what_to_hear.is_empty() && !audio_running {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(100, 220, 140),
                            format!("🔊 Press Start Audio above to hear: {}", info.what_to_hear));
                    });
                }
            }
        }

        ui.separator();

        // LO / Upconverter offset
        if let Ok(mut state) = self.shared.try_lock() {
            ui.horizontal(|ui| {
                ui.label("LO Offset").on_hover_text("Frequency offset for upconverters (e.g. Ham It Up adds 125 MHz). The displayed frequency = tuned + offset. Set to 0 for direct SDR use.");
                let mut lo_mhz = state.lo_offset_hz as f64 / 1e6;
                let drag = ui.add(egui::DragValue::new(&mut lo_mhz)
                    .speed(0.1)
                    .suffix(" MHz")
                    .range(-2000.0..=2000.0));
                if drag.changed() {
                    state.lo_offset_hz = (lo_mhz * 1e6) as i64;
                }
                for (label, offset_mhz, tip) in [
                    ("0", 0.0f64, "No offset (direct SDR)"),
                    ("125", 125.0, "Ham It Up / generic 125 MHz upconverter"),
                    ("100", 100.0, "SpyVerter / 100 MHz upconverter"),
                    ("-125", -125.0, "125 MHz downconverter / negative offset"),
                ] {
                    if ui.small_button(label).on_hover_text(tip).clicked() {
                        state.lo_offset_hz = (offset_mhz * 1e6) as i64;
                    }
                }
            });
            if state.lo_offset_hz != 0 {
                let true_freq = state.source.frequency_hz as i64 + state.lo_offset_hz;
                if true_freq > 0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 200, 80),
                        format!("True frequency: {:.3} MHz (tuned {:.3} MHz + {:.0} MHz offset)",
                            true_freq as f64 / 1e6,
                            state.source.frequency_hz as f64 / 1e6,
                            state.lo_offset_hz as f64 / 1e6),
                    );
                }
            }
        }

        ui.separator();

        // Source controls
        if let Ok(mut state) = self.shared.try_lock() {
            state.source.ui(ui);
        }

        ui.add_space(16.0);
        // ── Airport Frequency Finder (collapsible, optional) ──────────
        ui.collapsing("✈️ Airport Frequency Finder", |ui| {
            ui.add_space(4.0);
            // Download progress
            if let Some((done, total)) = self.airport_dl_progress {
                let frac = if total > 0 { done as f32 / total.max(done) as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).text(format!("{:.1} / {:.1} MB", done as f64 / 1e6, total.max(done) as f64 / 1e6)));
                ui.label(&self.airport_dl_msg);
                if done >= total && total > 0 {
                    if ui.button("✓ Load airports").clicked() {
                        let fresh = crate::airport_db::AirportDb::load();
                        self.airport_db = fresh;
                        self.airport_dl_progress = None;
                        self.airport_dl_msg = "Loaded {count} airports".to_string();
                    }
                }
                return;
            }
            // Search row
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.add(egui::TextEdit::singleline(&mut self.airport_search).hint_text("ICAO / IATA / name").desired_width(140.0));
                ui.label("Type:");
                let types = ["all", "large_airport", "medium_airport", "small_airport", "heliport"];
                let cur_idx = types.iter().position(|t| *t == self.airport_type_filter).unwrap_or(0);
                egui::ComboBox::from_id_salt("ap_type_filter")
                    .selected_text(types[cur_idx])
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for &t in &types {
                            if ui.selectable_label(self.airport_type_filter == t, t).clicked() {
                                self.airport_type_filter = t.to_string();
                            }
                        }
                    });
                if !self.airport_db.cached_sqlite {
                    if ui.small_button("🌐 Download full world DB")
                        .on_hover_text("Download airports.csv + airport-frequencies.csv (~14 MB) from OurAirports. One-time cache.")
                        .clicked()
                    {
                        self.airport_dl_progress = Some((0, 1));
                        self.airport_dl_msg = "Downloading airports & frequencies...".to_string();
                        let result = crate::airport_db::AirportDb::download_full_blocking(|_, _| {});
                        match result {
                            Ok(count) => {
                                self.airport_dl_progress = Some((1, 1));
                                self.airport_dl_msg = format!("Downloaded {} airports.", count);
                            }
                            Err(e) => {
                                self.airport_dl_progress = None;
                                self.airport_dl_msg = format!("Error: {}", e);
                            }
                        }
                    }
                } else {
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 80), "✓ Full DB cached");
                }
            });
            ui.add_space(4.0);

            // Search results
            let results = self.airport_db.search(&self.airport_search, &self.airport_type_filter, 50);
            egui::ScrollArea::vertical().id_salt("airport_results").max_height(200.0).show(ui, |ui| {
                for a in &results {
                    let selected = self.selected_airport.as_deref() == Some(&a.ident);
                    let label = format!("{} — {} ({})", a.code(), a.name, a.country);
                    if ui.selectable_label(selected, label).clicked() {
                        self.selected_airport = Some(a.ident.clone());
                        self.expanded_antenna_for = None;
                    }
                }
            });
            ui.add_space(4.0);

            // Selected airport frequency table
            if let Some(ref ident) = self.selected_airport.clone() {
                if let Some(ap) = self.airport_db.airport(ident) {
                    let freqs = self.airport_db.frequencies_for(ident);

                    egui::Frame::group(ui.style()).inner_margin(6.0).show(ui, |ui| {
                        ui.label(egui::RichText::new(format!("{} ({})", ap.name, ap.code())).size(14.0).strong());
                        ui.label(format!("{} | lat {:.3} lon {:.3} | {}", ap.atype, ap.lat, ap.lon, ap.country));
                    });
                    ui.add_space(4.0);

                    egui::ScrollArea::vertical().id_salt("airport_freq_list").max_height(600.0).show(ui, |ui| {
                        // Header
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Type").strong());
                            ui.separator();
                            ui.label(egui::RichText::new("Function").strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new("Antenna").strong());
                                ui.separator();
                                ui.label(egui::RichText::new("Tune").strong());
                                ui.separator();
                                ui.label(egui::RichText::new("Freq (MHz)").strong());
                            });
                        });
                        ui.separator();

                        // Universal emergency always shown
                        for &(t, desc, mhz) in &[("EMERG", "VHF Guard (International)", 121.5), ("EMERG", "UHF Guard (Military)", 243.0)] {
                            ui.horizontal(|ui| {
                                ui.colored_label(egui::Color32::from_rgb(220, 70, 70), egui::RichText::new(t).strong());
                                ui.separator();
                                ui.label(desc);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("📏").clicked() {
                                        self.expanded_antenna_for = if self.expanded_antenna_for == Some(mhz) { None } else { Some(mhz) };
                                    }
                                    if ui.small_button("▶").clicked() {
                                        if let Ok(mut state) = self.shared.try_lock() {
                                            self.tune_request = Some((mhz * 1_000_000.0) as u64);
                                            state.demod_mode = crate::sdr_panel::DemodMode::Am;
                                            state.lpf_cutoff = 8000.0;
                                        }
                                        self.pending_status = Some(format!("Tuned to {:.3} MHz AM (emergency)", mhz));
                                    }
                                    ui.colored_label(egui::Color32::from_rgb(255, 220, 80), format!("{:.3}", mhz));
                                });
                            });
                            // Antenna card
                            if self.expanded_antenna_for == Some(mhz) {
                                render_antenna_card(ui, mhz);
                            }
                        }

                        if !freqs.is_empty() {
                            ui.separator();
                        }

                        for f in freqs {
                            let ft = f.freq_type;
                            let desc = if f.description.is_empty() { &f.raw_type } else { &f.description };
                            let freq_key = f.frequency_mhz;
                            ui.horizontal(|ui| {
                                ui.colored_label(ft.badge_color(), egui::RichText::new(ft.label()).strong());
                                ui.separator();
                                ui.label(desc);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("📏").clicked() {
                                        self.expanded_antenna_for = if self.expanded_antenna_for == Some(freq_key) { None } else { Some(freq_key) };
                                    }
                                    if ui.small_button("▶").clicked() {
                                        let dims = crate::airport_db::antenna_dims(f.frequency_mhz);
                                        if let Ok(mut state) = self.shared.try_lock() {
                                            self.tune_request = Some((f.frequency_mhz * 1_000_000.0) as u64);
                                            if let Some(mode) = crate::sdr_panel::DemodMode::from_label(dims.suggested_mode) {
                                                state.demod_mode = mode;
                                            }
                                            state.lpf_cutoff = dims.suggested_bw_hz as f32;
                                        }
                                        self.pending_status = Some(format!("Tuned to {:.3} MHz ({}) — {}", f.frequency_mhz, dims.suggested_mode, dims.suggested_antenna));
                                    }
                                    ui.colored_label(egui::Color32::from_rgb(255, 220, 80), format!("{:.3}", f.frequency_mhz));
                                });
                            });
                            // Antenna card for this frequency
                            if self.expanded_antenna_for == Some(freq_key) {
                                render_antenna_card(ui, freq_key);
                            }
                        }
                    });

                    if !freqs.is_empty() {
                        ui.add_space(2.0);
                        if ui.small_button("📌 Bookmark all").on_hover_text("Save all airport frequencies as bookmarks").clicked() {
                            if let Ok(mut state) = self.shared.try_lock() {
                                for f in freqs {
                                    let name = format!("{} {} — {:.3}", ap.code(), f.freq_type.label(), f.frequency_mhz);
                                    state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                                        name,
                                        frequency_hz: (f.frequency_mhz * 1_000_000.0) as u64,
                                        mode: "AM".to_string(),
                                        bandwidth_hz: 8_000,
                                        category: format!("Aviation — {}", ap.code()),
                                        notes: String::new(),
                                        starred: false,
                                    });
                                    state.bookmarks_modified = true;
                                    state.spectrum.bookmark_freqs_dirty = true;
                                }
                                self.pending_status = Some(format!("Bookmarked {} frequencies for {}", freqs.len(), ap.code()));
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.ui_source(ui);
        self.ui_demod(ui);
    }
}

fn render_antenna_card(ui: &mut egui::Ui, freq_mhz: f64) {
    let dims = crate::airport_db::antenna_dims(freq_mhz);
    egui::Frame::dark_canvas(ui.style()).inner_margin(4.0).show(ui, |ui| {
        ui.label(egui::RichText::new("📡 Antenna Dimensions").size(12.0).strong());
        ui.add_space(2.0);
        ui.label(format!("  ¼-wave element:      {:.1} cm  ({:.0} mm)", dims.quarter_wave_cm, dims.quarter_wave_cm * 10.0));
        ui.label(format!("  ½-wave dipole total:  {:.1} cm", dims.half_wave_dipole_cm));
        ui.label(format!("  Ground-plane radial:  {:.1} cm each, 4 × @ 45°", dims.ground_plane_radial_cm));
        ui.label(format!("  Coax collinear seg:   {:.1} cm (RG-58, VF 0.66)", dims.coax_collinear_segment_cm));
        ui.label(format!("  Suggested antenna:  {}", dims.suggested_antenna));
        ui.label(format!("  Mode: {}   BW: {} Hz", dims.suggested_mode, dims.suggested_bw_hz));
        ui.label(format!("  λ = {:.1} m  |  f = {:.3} MHz", 300.0 / freq_mhz, freq_mhz));
    });
}

pub struct FreqIdInfo {
    pub band: &'static str,
    pub short_desc: &'static str,
    pub detail: &'static str,
    pub tips: &'static str,
    pub what_to_hear: &'static str,
}

pub fn identify_frequency(freq_hz: u64) -> Option<FreqIdInfo> {
    let entries: &[(u64, u64, &str, &str, &str, &str)] = &[
        (150_000,   500_000,   "LF/MF",         "Long & medium wave",
            "AM broadcast (MW), maritime beacons, time signals (DCF77/MSF).",
            "Use AM demod. Long-wave AM goes down to 150 kHz. DCF77 at 77.5 kHz carries atomic time."),
        (1_800_000, 3_500_000, "160m HF Amateur","Amateur 160m (Top Band)",
            "CW at 1.8 MHz, voice SSB from 1.84 MHz. Very long-range at night.",
            "Use LSB for voice, CW mode for Morse. Best reception after dark."),
        (3_500_000, 4_000_000, "80m HF Amateur", "Amateur 80m band",
            "Busy night band — CW, SSB voice, digital modes. Excellent DX at night.",
            "Use LSB. Expect crowded frequencies especially 3.5–3.8 MHz."),
        (7_000_000, 7_300_000, "40m HF Amateur", "Amateur 40m band",
            "CW and digital 7.0–7.07, SSB 7.1–7.3. Strong DX day and night.",
            "LSB below 10 MHz. FT8 digital at 7.074 MHz is very busy."),
        (10_000_000, 10_150_000, "30m HF Amateur", "Amateur 30m band",
            "CW and digital only. FT8 at 10.136 MHz. No phone allowed.",
            "USB. Narrow band — good for digital modes like FT8/FT4."),
        (14_000_000, 14_350_000, "20m HF Amateur", "Amateur 20m band",
            "Most popular HF amateur band. Excellent DX any time of day.",
            "USB above 10 MHz. FT8 at 14.074 MHz. SSB voice from 14.150 MHz."),
        (21_000_000, 21_450_000, "15m HF Amateur", "Amateur 15m band",
            "Good daytime DX, especially solar maximum. Opens to distant DX.",
            "USB. FT8 at 21.074 MHz. Active during day."),
        (24_890_000, 24_990_000, "12m HF Amateur", "Amateur 12m band",
            "Near full shortwave for DX. Best near solar maximum.",
            "USB. FT8 at 24.915 MHz."),
        (26_965_000, 27_405_000, "CB (Citizens Band)", "CB radio — 40 channels AM",
            "Truckers, 4x4 off-road, short-range comms. Channel 19 = 27.185 MHz trucker net.",
            "AM demod. Ch9 (27.065 MHz) is emergency channel. USB is used for DX on some channels."),
        (28_000_000, 29_700_000, "10m HF Amateur", "Amateur 10m band",
            "Excellent when solar cycle is active. Worldwide DX with modest antennas.",
            "USB. FT8 at 28.074 MHz. CW at 28.0–28.070 MHz."),
        (50_000_000, 54_000_000, "6m Amateur",    "Amateur 6m 'magic band'",
            "VHF sporadic-E propagation — can provide continent-wide DX unexpectedly.",
            "USB for voice/FT8. Known for surprise openings with low power."),
        (88_000_000, 108_000_000, "FM Broadcast", "Commercial FM radio (88–108 MHz)",
            "Stereo music, news, talk radio. RDS data embedded. WFM demod, wide 200 kHz BW.",
            "WFM mode, BW ~200 kHz. Many SDRs receive RDS text alongside audio."),
        (108_000_000, 118_000_000, "VOR/ILS",     "Aviation navigation aids",
            "VHF Omni-directional Range (VOR) and Instrument Landing System. Not voice.",
            "AM demod. These are navigation signals — you'll hear a morse identifier and tone."),
        (118_000_000, 137_000_000, "Aviation VHF", "Air Traffic Control (ATC)",
            "ATC talking to aircraft. Approach, ground, tower, ATIS, centre frequencies.",
            "AM demod. ATIS (airport weather) are automated — listen for your local airport."),
        (137_000_000, 138_000_000, "NOAA Satellites","Weather satellite downlinks",
            "NOAA 15/18/19 send APT image data at 137.5/137.9/137.1 MHz. Visible passes only.",
            "WFM 34 kHz BW. Use SDR# or NOAA-APT software to decode the image."),
        (144_000_000, 148_000_000, "Amateur 2m",   "2-meter amateur radio band",
            "Most active VHF amateur band. FM repeaters, simplex, satellite links, weak-signal.",
            "NFM for voice. 144.0–144.1 MHz CW/SSB DX. 144.390 MHz is APRS."),
        (150_000_000, 156_000_000, "Land Mobile",  "Public safety, utilities, business",
            "Police, fire, taxis, railways. Mix of NFM voice and digital (DMR, P25).",
            "NFM. Digital signals sound like fast data/buzzing — need separate decoder."),
        (156_000_000, 174_000_000, "Marine VHF",   "Maritime communications",
            "Channel 16 (156.8 MHz) = international distress and hailing. Working channels 17–28.",
            "NFM. DSC digital safety calls on Ch70 (156.525 MHz)."),
        (162_400_000, 162_600_000, "NOAA WX Radio","US NOAA Weather Radio",
            "7 channels of continuous weather broadcasts, warnings, forecasts.",
            "WFM or NFM. Automated voice — very strong signal near transmitters."),
        (406_000_000, 406_100_000, "EPIRB/PLB",    "Emergency distress beacons (406 MHz)",
            "EPIRB and PLB satellite-linked emergency beacons. Narrow digital bursts.",
            "NFM. Should be silent unless a genuine emergency — do not transmit here."),
        (420_000_000, 450_000_000, "Amateur 70cm",  "70-centimeter amateur band",
            "FM repeaters, weak-signal EME, ATV, digital modes. Most active: 430–440 MHz.",
            "NFM for voice repeaters. 432.1 MHz SSB weak-signal DX. 433.0 MHz simplex."),
        (433_000_000, 435_000_000, "ISM 433 MHz",  "License-free ISM devices",
            "Car key fobs, wireless doorbells, weather stations, cheap sensors.",
            "NFM or RAW. Short OOK/FSK bursts — decode with rtl_433 tool."),
        (450_000_000, 470_000_000, "UHF LMR",      "UHF land mobile radio",
            "Business, public safety, taxis, transport. Mix of FM voice and digital.",
            "NFM. DMR, P25, NXDN digital systems sound like buzzing/data bursts."),
        (890_000_000, 960_000_000, "GSM 900",      "2G cellular (GSM)",
            "Legacy 2G voice/SMS. Uplink 890–915 MHz, downlink 935–960 MHz.",
            "RAW — encrypted. You'll see signal but can't decode content legally."),
        (1_090_000_000, 1_090_000_000, "ADS-B",    "Aircraft position transponders",
            "ADS-B 1090ES — aircraft broadcast position, altitude, speed. 1 second updates.",
            "Use the ADS-B tab in ez-sdr! RAW mode + 2.4 MSps. Works at 1090 ±1 MHz."),
        (1_215_000_000, 1_240_000_000, "L-band Radar","Radar altimeters, navigation",
            "Radar altimeters and L-band surveillance radars. Pulsed signals.",
            "RAW. Short bursts visible in the waterfall."),
        (1_525_000_000, 1_559_000_000, "L-band Sat","L-band satellite downlinks",
            "Inmarsat/Iridium voice, AERO aviation data, SCADA, MSS phones.",
            "WFM/NFM. Inmarsat AERO at 1.5465 GHz carries ATC/aircraft data."),
        (1_559_000_000, 1_610_000_000, "GPS/GNSS",  "GPS/GALILEO/GLONASS signals",
            "Navigation satellite signals. Very weak broadband BPSK. L1 at 1575.42 MHz.",
            "RAW with wide BW. Use dedicated GPS software — too weak for audio."),
        (1_626_000_000, 1_661_000_000, "Iridium",   "Iridium satellite phones",
            "Iridium NEXT LEO satellite constellation. Burst data, voice, IoT links.",
            "RAW or WFM. Bursts every ~90 seconds when satellites pass."),
        (1_694_000_000, 1_700_000_000, "GOES Sat",  "GOES weather satellite downlinks",
            "GOES-16/17/18 East/West at 1694.1 MHz: HRIT full-disk weather images.",
            "RAW, needs 2+ MSps and special decoder (goestools/SatDump)."),
    ];
    for &(lo, hi, band, short_desc, detail, tips) in entries {
        if freq_hz >= lo && freq_hz <= hi.max(lo) {
            let what_to_hear = what_to_hear_for_band(band);
            return Some(FreqIdInfo { band, short_desc, detail, tips, what_to_hear });
        }
    }
    None
}

fn what_to_hear_for_band(band: &str) -> &'static str {
    match band {
        "FM Broadcast"      => "music, news, or talk radio",
        "Aviation VHF"      => "air traffic control voice (pilots + towers)",
        "Marine VHF"        => "coast guard, ships, harbour calls",
        "NOAA WX Radio"     => "automated weather forecast and alerts",
        "NOAA Satellites"   => "a distinctive chirping APT image data signal",
        "Amateur 2m"        => "amateur radio voice, APRS data bursts",
        "Amateur 70cm"      => "amateur radio repeaters and digital modes",
        "Land Mobile"       => "professional voice radio (police, fire, business)",
        "CB (Citizens Band)"=> "truckers and CB radio operators",
        "LF/MF"             => "AM broadcast stations or navigation beacons",
        "ADS-B"             => "nothing audible — use the ADS-B tab to see aircraft",
        "VOR/ILS"           => "a morse-code station identifier and nav tone",
        _                   => "",
    }
}

/// Returns (suggested_mode, band_name, reason) if the frequency matches a well-known band
/// and the suggestion would differ from the current mode.
fn suggest_demod_for_freq(freq_hz: u64) -> Option<(DemodMode, &'static str, &'static str)> {
    let bands: &[(u64, u64, DemodMode, &str, &str)] = &[
        // HF amateur bands (LSB preferred below 10 MHz)
        (1_800_000,   2_000_000,   DemodMode::Lsb,  "160m Band",         "LSB for voice; CW for Morse"),
        (3_500_000,   4_000_000,   DemodMode::Lsb,  "80m Band",          "LSB for voice/digital"),
        (7_000_000,   7_300_000,   DemodMode::Lsb,  "40m Band",          "LSB for voice/FT8"),
        (10_100_000,  10_150_000,  DemodMode::Usb,  "30m Band",          "USB for digital modes (CW/data only)"),
        (14_000_000,  14_350_000,  DemodMode::Usb,  "20m Band",          "USB for voice/FT8 (most popular)"),
        (21_000_000,  21_450_000,  DemodMode::Usb,  "15m Band",          "USB for voice/FT8"),
        (24_890_000,  24_990_000,  DemodMode::Usb,  "12m Band",          "USB for voice/FT8"),
        (28_000_000,  29_700_000,  DemodMode::Usb,  "10m Band",          "USB for voice/FT8/CW"),
        (50_000_000,  54_000_000,  DemodMode::Usb,  "6m Band",           "USB for voice/FT8 (sporadic-E)"),
        // Medium wave
        (150_000,     500_000,     DemodMode::Am,   "LF/MF Band",        "AM for beacons/time signals"),
        (26_965_000,  27_405_000,  DemodMode::Am,   "CB Radio",          "AM for Citizens Band"),
        // VHF/UHF
        (88_000_000,  108_000_000, DemodMode::Wfm, "FM Broadcast",      "WFM for commercial radio"),
        (118_000_000, 137_000_000, DemodMode::Am,  "Aviation",          "AM for air-to-ground voice"),
        (137_000_000, 138_000_000, DemodMode::Fm,  "NOAA APT",          "NFM for weather satellite"),
        (144_000_000, 148_000_000, DemodMode::Fm,  "Amateur 2m",        "NFM for repeaters/simplex"),
        (150_000_000, 156_000_000, DemodMode::Fm,  "Land Mobile",       "NFM for land mobile radio"),
        (156_000_000, 174_000_000, DemodMode::Fm,  "Marine VHF",        "NFM for ship/coast guard"),
        (162_400_000, 162_600_000, DemodMode::Wfm, "NOAA Weather",      "WFM for NOAA broadcasts"),
        (406_000_000, 406_100_000, DemodMode::Fm,  "EPIRB/PLB",         "NFM for emergency beacons"),
        (420_000_000, 450_000_000, DemodMode::Fm,  "Amateur 70cm",      "NFM for repeaters/digital"),
        (433_000_000, 435_000_000, DemodMode::Fm,  "ISM 433 MHz",       "NFM or RAW for sensor data"),
        (450_000_000, 470_000_000, DemodMode::Fm,  "UHF LMR",           "NFM for UHF land mobile"),
        // Microwave/satellite
        (1_090_000_000, 1_090_000_000, DemodMode::Raw, "ADS-B",         "RAW for aircraft transponders"),
    ];
    for &(lo, hi, mode, name, reason) in bands {
        if freq_hz >= lo && freq_hz <= hi.max(lo) {
            return Some((mode, name, reason));
        }
    }
    None
}
