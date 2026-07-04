#![allow(unsafe_code)]

use eframe::NativeOptions;

mod adsb_decoder;
mod adsb_panel;
mod ai_panel;
mod airport_db;
mod antenna_checklist;
mod app;
mod audio_output;
mod bookmarks;
mod config;
mod demod;
mod discord;
mod discord_panel;
mod editor_panel;
mod howto_panel;
mod image_processing;
mod composite;
mod mqtt;
mod recorder_panel;
mod satellite_panel;
mod scanner;
mod scheduler;
mod sdr_panel;
mod source_manager;
mod spectrum;
#[cfg(test)]
mod test_helpers;
mod theme;
mod tle_engine;
mod tutorial;
mod user_level;
mod web_remote;

fn main() -> eframe::Result {
    // Force X11 on Linux — winit's Wayland backend has broken mouse input
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WINIT_UNIX_BACKEND").is_err() {
            std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        }
    }

    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "EZ-SDR Unified",
        options,
        Box::new(|cc| Ok(Box::new(app::CentralApp::new(cc)))),
    )
}
