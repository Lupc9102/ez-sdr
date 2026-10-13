#![allow(unsafe_code)]

use eframe::NativeOptions;

fn main() -> eframe::Result {
    #[cfg(windows)]
    if let Err(error) = ez_gui::app_data::initialize_windows_directory() {
        return eframe::run_native(
            "EZ-SDR storage error",
            NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 240.0]),
                renderer: eframe::Renderer::Glow,
                ..Default::default()
            },
            Box::new(move |_| Ok(Box::new(StorageError(error.to_string())))),
        );
    }
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

#[cfg(windows)]
struct StorageError(String);

#[cfg(windows)]
impl eframe::App for StorageError {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("Your data directory could not be opened");
        ui.label("EZ-SDR needs a writable folder at %LOCALAPPDATA%\\EZ-SDR for settings, cache and recordings. Check that folder's permissions and available disk space, then restart.");
        ui.colored_label(ui.visuals().error_fg_color, &self.0);
        if ui.button("Close").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
