use eframe::NativeOptions;

mod app;
mod source_manager;
mod spectrum;
mod sdr_panel;
mod satellite_panel;
mod adsb_panel;
mod adsb_decoder;
mod recorder_panel;
mod ai_panel;
mod scheduler;
mod database;
mod config;
mod bookmarks;
mod web_remote;
mod mqtt;
mod discord;
mod discord_panel;
mod tle_engine;
mod demod;
mod audio_output;
mod scanner;
mod howto_panel;
mod theme;

fn main() -> eframe::Result {
    // Force X11 on Linux — winit's Wayland backend has broken mouse input
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WINIT_UNIX_BACKEND").is_err() {
            std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        }
    }

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
