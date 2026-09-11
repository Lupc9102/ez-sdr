use crate::satellite::types::SatelliteCatalogEntry;
use egui::Color32;

pub fn satellite_picker_ui(
    ui: &mut egui::Ui,
    catalog: &[SatelliteCatalogEntry],
    selected: &mut Option<usize>,
    search: &mut String,
    on_select: impl FnOnce(usize),
) {
    ui.vertical(|ui| {
        ui.label("Search / Filter");
        ui.add(
            egui::TextEdit::singleline(search)
                .desired_width(ui.available_width())
                .hint_text("Type satellite name…"),
        );
    });

    ui.add_space(4.0);

    let filtered: Vec<(usize, &SatelliteCatalogEntry)> = catalog
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            if search.is_empty() {
                return true;
            }
            let q = search.to_lowercase();
            e.name.to_lowercase().contains(&q)
                || e.tle_name.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
        })
        .collect();

    let mut clicked_idx: Option<usize> = None;

    egui::ScrollArea::vertical()
        .max_height(280.0)
        .show(ui, |ui| {
            for (orig_idx, entry) in &filtered {
                let is_selected = selected.as_ref() == Some(orig_idx);
                let bg = if is_selected {
                    Color32::from_rgb(0, 60, 100)
                } else {
                    Color32::from_rgb(18, 22, 30)
                };

                egui::Frame::new()
                    .fill(bg)
                    .stroke(egui::Stroke::new(
                        1.0,
                        if is_selected {
                            Color32::from_rgb(0, 168, 255)
                        } else {
                            Color32::from_gray(40)
                        },
                    ))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        let resp = ui
                            .scope(|ui| {
                                ui.horizontal(|ui| {
                                    let freq_str =
                                        format!("{:.3} MHz", entry.frequency_hz as f64 / 1e6);
                                    let mode_str = entry.mode;
                                    ui.strong(&entry.name);
                                    ui.label(
                                        egui::RichText::new(freq_str)
                                            .size(12.0)
                                            .color(Color32::from_rgb(0, 200, 255)),
                                    );
                                    ui.label(
                                        egui::RichText::new(mode_str)
                                            .size(11.0)
                                            .color(Color32::from_rgb(150, 150, 150)),
                                    );
                                    if entry.is_active_pass {
                                        ui.label(
                                            egui::RichText::new("LIVE")
                                                .size(10.0)
                                                .strong()
                                                .color(Color32::from_rgb(50, 255, 100)),
                                        );
                                    }
                                });
                                ui.label(
                                    egui::RichText::new(entry.description)
                                        .size(11.0)
                                        .color(Color32::from_gray(130)),
                                );
                            })
                            .response;
                        if resp.clicked() {
                            clicked_idx = Some(*orig_idx);
                        }
                    });
                ui.add_space(2.0);
            }

            if filtered.is_empty() {
                ui.colored_label(Color32::GRAY, "No matching satellites found");
            }
        });

    if let Some(idx) = clicked_idx {
        *selected = Some(idx);
        on_select(idx);
    }
}
