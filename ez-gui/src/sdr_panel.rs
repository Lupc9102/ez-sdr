use std::sync::{Arc, Mutex};

use crate::app::SharedState;

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
}

impl SdrPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            squelch: -50.0,
            filter_bw: 12_000,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("SDR Receiver");

        // Big frequency display with fine/coarse tuning
        if let Ok(mut state) = self.shared.try_lock() {
            ui.horizontal(|ui| {
                let mut freq_mhz = state.source.frequency_hz as f64 / 1e6;
                ui.monospace(egui::RichText::new(format!("{:.6}", freq_mhz)).size(24.0).color(egui::Color32::from_rgb(52, 152, 219)));
                ui.label(egui::RichText::new("MHz").size(14.0).color(egui::Color32::GRAY));
                let is_dragging = ui.add(egui::DragValue::new(&mut freq_mhz).speed(0.0001).range(0.5..=1770.0).suffix(" MHz"));
                if is_dragging.changed() || is_dragging.dragged() {
                    state.source.frequency_hz = (freq_mhz * 1e6) as u64;
                }
                if ui.small_button("-1M").clicked() {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(1_000_000).max(500_000);
                }
                if ui.small_button("+1M").clicked() {
                    state.source.frequency_hz = (state.source.frequency_hz + 1_000_000).min(1_770_000_000);
                }
                if ui.small_button("-100k").clicked() {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(100_000).max(500_000);
                }
                if ui.small_button("+100k").clicked() {
                    state.source.frequency_hz = (state.source.frequency_hz + 100_000).min(1_770_000_000);
                }
            });
        }
        ui.separator();

        // Demod mode selector
        ui.horizontal(|ui| {
            if let Ok(mut state) = self.shared.try_lock() {
                for mode in [DemodMode::Raw, DemodMode::Am, DemodMode::Fm, DemodMode::Wfm, DemodMode::Lsb, DemodMode::Usb] {
                    if ui.selectable_label(state.demod_mode == mode, mode.label()).clicked() {
                        state.demod_mode = mode;
                    }
                }
            }
        });

        // Signal meter
        ui.separator();
        if let Ok(state) = self.shared.try_lock() {
            let signal = state.spectrum.signal_level();
            let norm = ((signal + 120.0) / 120.0).clamp(0.0, 1.0);
            let color = if norm > 0.6 { egui::Color32::GREEN }
                else if norm > 0.3 { egui::Color32::YELLOW }
                else { egui::Color32::RED };
            ui.horizontal(|ui| {
                ui.label("Signal:");
                ui.add(egui::ProgressBar::new(norm).fill(color).text(format!("{:.1} dB", signal)));
            });
        }

        // Audio controls
        ui.separator();
        ui.horizontal(|ui| {
            let mut start_audio = false;
            let mut stop_audio = false;
            let mut volume = 0.5f32;
            let mut audio_running = false;
            if let Ok(state) = self.shared.try_lock() {
                volume = state.volume;
                audio_running = state.audio_running;
            }
            if audio_running {
                if ui.button("🔇 Stop Audio").clicked() {
                    stop_audio = true;
                }
            } else {
                if ui.button("🔊 Start Audio").clicked() {
                    start_audio = true;
                }
            }
            let mut vol = volume;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Vol")).changed() {
                volume = vol;
            }
            if start_audio || stop_audio || vol != volume {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.volume = volume;
                    if start_audio {
                        state.audio_running = true;
                    } else if stop_audio {
                        state.audio_running = false;
                    }
                }
            }
        });

        ui.separator();

        // Frequency presets (band quick-tune)
        ui.horizontal(|ui| {
            ui.label("Bands:");
            const BANDS: &[(&str, u64)] = &[
                ("BC FM", 100_000_000),
                ("Air", 118_000_000),
                ("2m", 144_000_000),
                ("70cm", 430_000_000),
                ("NOAA 15", 137_620_000),
                ("NOAA 18", 137_912_500),
                ("NOAA 19", 137_100_000),
                ("ISS", 145_800_000),
            ];
            if let Ok(mut state) = self.shared.try_lock() {
                for (name, freq_hz) in BANDS {
                    if ui.small_button(*name).clicked() {
                        state.source.frequency_hz = *freq_hz;
                    }
                }
            }
        });

        // Filter bandwidth and squelch
        ui.add(egui::Slider::new(&mut self.filter_bw, 100..=250_000).text("Filter BW (Hz)").logarithmic(true));
        if let Ok(mut state) = self.shared.try_lock() {
            ui.add(egui::Slider::new(&mut state.lpf_cutoff, 100.0..=20000.0).text("Audio LPF (Hz)").logarithmic(true));
        }
        if ui.add(egui::Slider::new(&mut self.squelch, -120.0..=0.0).text("Squelch (dB)")).changed()
            || ui.input(|i| i.pointer.any_down())
        {
            if let Ok(mut state) = self.shared.try_lock() {
                state.squelch = self.squelch;
            }
        }

        ui.separator();

        // Source controls
        if let Ok(mut state) = self.shared.try_lock() {
            state.source.ui(ui);
        }
    }
}
