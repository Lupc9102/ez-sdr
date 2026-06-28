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

        // Filter and squelch
        ui.add(egui::Slider::new(&mut self.filter_bw, 100..=250_000).text("Filter bandwidth (Hz)").logarithmic(true));
        ui.add(egui::Slider::new(&mut self.squelch, -120.0..=0.0).text("Squelch (dB)"));

        ui.separator();

        // Source controls
        if let Ok(mut state) = self.shared.try_lock() {
            state.source.ui(ui);
        }
    }
}
