#![allow(unsafe_code)]

use eframe::NativeOptions;

fn main() -> eframe::Result {
    let launch_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            return show_storage_error(format!("Could not read the launch directory: {error}"))
        }
    };
    let executable_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
    let paths = match ez_gui::app_data::AppDataPaths::from_process(&launch_dir) {
        Ok(paths) => paths,
        Err(error) => return show_storage_error(error.to_string()),
    };
    let legacy = ez_gui::app_data::legacy_directories(&launch_dir, executable_dir.as_deref());
    if let Err(error) = paths.prepare(&legacy) {
        return show_storage_error(format!(
            "Could not prepare EZ-SDR's user data folders. Check permissions and available disk space.\nConfig: {}\nData: {}\nCache: {}\n{error}",
            paths.config_dir.display(),
            paths.data_dir.display(),
            paths.cache_dir.display()
        ));
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
        Box::new(move |cc| {
            Ok(Box::new(ez_gui::app::CentralApp::new_with_paths(
                cc,
                paths.clone(),
            )))
        }),
    )
}

struct StorageError(String);

impl eframe::App for StorageError {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("Your data directory could not be opened");
        ui.label("EZ-SDR needs writable per-user folders for settings, bookmarks, cache and recordings. Check the reported location's permissions and available disk space, then restart.");
        ui.colored_label(ui.visuals().error_fg_color, &self.0);
        if ui.button("Close").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn show_storage_error(message: String) -> eframe::Result {
    eframe::run_native(
        "EZ-SDR storage error",
        NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 260.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |_| Ok(Box::new(StorageError(message.clone())))),
    )
}
