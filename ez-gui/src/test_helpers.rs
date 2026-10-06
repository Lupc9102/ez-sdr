use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::app::{FreqMemEntry, SharedState};
use crate::bookmarks::BookmarkDb;
use crate::config::AppConfig;
use crate::scheduler::Scheduler;
use crate::source_manager::SourceManager;
use crate::spectrum::SpectrumAnalyzer;
use crate::tle_engine::TleEngine;

#[cfg(test)]
pub(crate) struct FaultFileWriter {
    file: std::fs::File,
    limit: Option<usize>,
    written: usize,
    flush_error: bool,
}

#[cfg(test)]
impl FaultFileWriter {
    pub fn new(file: std::fs::File, limit: Option<usize>, flush_error: bool) -> Self {
        Self {
            file,
            limit,
            written: 0,
            flush_error,
        }
    }
}

#[cfg(test)]
impl std::io::Write for FaultFileWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = self.limit.map_or(bytes.len(), |limit| {
            limit.saturating_sub(self.written).min(bytes.len())
        });
        if remaining == 0 {
            return Err(std::io::Error::other("injected disk write failure"));
        }
        let written = self.file.write(&bytes[..remaining])?;
        self.written += written;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.flush_error {
            return Err(std::io::Error::other("injected disk flush failure"));
        }
        self.file.flush()
    }
}

pub fn make_shared_state() -> Arc<Mutex<SharedState>> {
    Arc::new(Mutex::new(SharedState {
        source: SourceManager::new(),
        spectrum: SpectrumAnalyzer::new(),
        config: AppConfig::default(),
        input_format: dump1090::demod::InputFormat::Uc8,
        bookmarks: BookmarkDb::default(),
        scheduler: Scheduler::new(),
        tle: TleEngine::new(),
        demod_mode: crate::sdr_panel::DemodMode::Fm,
        recording: false,
        adsb_running: false,
        selected_satellite: None,
        audio_running: false,
        volume: 0.5,
        squelch: -50.0,
        lpf_cutoff: 15000.0,
        fm_deviation_hz: 0.0,
        audio_peak: 0.0,
        freq_history: VecDeque::with_capacity(20),
        vfo_b: 0,
        freq_memory: std::array::from_fn(|_| FreqMemEntry::default()),
        tune_step_fine_hz: 100_000,
        tune_step_coarse_hz: 1_000_000,
        lo_offset_hz: 0,
        mqtt_connected: false,
        mqtt_enabled: false,
        bookmarks_modified: true,
        scanner_command: None,
        last_manual_tune: None,
        runtime_actions: std::collections::VecDeque::new(),
    }))
}

#[allow(dead_code)]
pub fn run_ui<F>(f: F)
where
    F: FnOnce(&mut egui::Ui),
{
    let ctx = egui::Context::default();
    let mut f = Some(f);
    let _ = ctx.run_ui(egui::RawInput::default(), |_ctx| {
        egui::Area::new(egui::Id::new("__test_area")).show(_ctx, |ui| {
            if let Some(f) = f.take() {
                f(ui);
            }
        });
    });
}
