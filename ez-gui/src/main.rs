#![allow(unsafe_code)]

use eframe::NativeOptions;

fn main() -> eframe::Result {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "ez-sdr",
        options,
        Box::new(|cc| Ok(Box::new(ez_gui::app::CentralApp::new(cc)))),
    )
}
