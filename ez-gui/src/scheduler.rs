use std::sync::{Arc, Mutex};
use crate::app::SharedState;
use crate::tle_engine::TleEngine;

pub struct Scheduler {
    pub jobs: Vec<ScheduledJob>,
}

#[derive(Debug, Clone)]
pub struct ScheduledJob {
    pub satellite: String,
    pub aos: String,
    pub los: String,
    pub auto_record: bool,
    pub frequency_hz: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { jobs: vec![] }
    }

    pub fn poll_shared(&mut self, shared: &Arc<Mutex<SharedState>>) {
        if let Ok(mut _state) = shared.try_lock() {
            // TODO: check TLE passes, trigger auto-tune / record
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, _tle: &TleEngine) {
        ui.heading("Scheduler");
        if ui.button("Refresh TLEs + compute passes").clicked() {
            // TODO: download TLEs, compute next 48h
        }
        for job in &self.jobs {
            ui.label(format!("{}  {} → {}", job.satellite, job.aos, job.los));
        }
    }
}
