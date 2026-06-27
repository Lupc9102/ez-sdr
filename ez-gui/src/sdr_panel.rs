use std::sync::{Arc, Mutex};

use crate::app::SharedState;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DemodMode {
    Raw,
    Am,
    Fm,
    Wfm,
    Lsb,
    Usb,
}

pub struct SdrPanel {
    shared: Arc<Mutex<SharedState>>,
    pub squelch: f32,
    pub filter_bw: u32,
    pub selected_bookmark: Option<String>,
}

impl SdrPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            squelch: -50.0,
            filter_bw: 12_000,
            selected_bookmark: None,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("SDR Receiver");
        if let Ok(mut state) = self.shared.try_lock() {
            ui.horizontal(|ui| {
                if ui.selectable_label(state.demod_mode == DemodMode::Raw, "RAW").clicked() { state.demod_mode = DemodMode::Raw; }
                if ui.selectable_label(state.demod_mode == DemodMode::Am, "AM").clicked() { state.demod_mode = DemodMode::Am; }
                if ui.selectable_label(state.demod_mode == DemodMode::Fm, "FM").clicked() { state.demod_mode = DemodMode::Fm; }
                if ui.selectable_label(state.demod_mode == DemodMode::Wfm, "WFM").clicked() { state.demod_mode = DemodMode::Wfm; }
                if ui.selectable_label(state.demod_mode == DemodMode::Lsb, "LSB").clicked() { state.demod_mode = DemodMode::Lsb; }
                if ui.selectable_label(state.demod_mode == DemodMode::Usb, "USB").clicked() { state.demod_mode = DemodMode::Usb; }
            });
        }
        ui.add(egui::Slider::new(&mut self.filter_bw, 100..=250_000).text("Filter bandwidth (Hz)"));
        ui.add(egui::Slider::new(&mut self.squelch, -120.0..=0.0).text("Squelch (dB)"));
        if let Ok(mut state) = self.shared.try_lock() {
            state.source.ui(ui);
        }
    }
}
