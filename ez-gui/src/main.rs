#![allow(unsafe_code)]

use eframe::NativeOptions;

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
        Box::new(|cc| Ok(Box::new(ez_gui::app::CentralApp::new(cc)))),
    )
}
