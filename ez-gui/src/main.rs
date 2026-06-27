use eframe::NativeOptions;

mod app;
mod source_manager;
mod spectrum;
mod sdr_panel;
mod satellite_panel;
mod adsb_panel;
mod recorder_panel;
mod ai_panel;
mod scheduler;
mod database;
mod config;
mod bookmarks;
mod web_remote;
mod mqtt;
mod tle_engine;

fn main() -> eframe::Result {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "EZ-SDR Unified",
        options,
        Box::new(|cc| Ok(Box::new(app::CentralApp::new(cc)))),
    )
}
