//! The 🎨 Customize tab — full UI customization: theme gallery, colors,
//! visual effects (corner roundness, glow, gradients), typography, sidebar
//! layout, and theme import/export.

use crate::config::{AppConfig, LayoutItem};
use crate::theme::{Density, FontFamily, NamedTheme, ThemeConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomizeSubTab {
    Themes,
    Colors,
    Effects,
    Typography,
    Layout,
    ImportExport,
}

pub struct CustomizePanel {
    pub subtab: CustomizeSubTab,
    new_theme_name: String,
    renaming: Option<usize>,
    rename_buffer: String,
}

impl Default for CustomizePanel {
    fn default() -> Self {
        Self {
            subtab: CustomizeSubTab::Themes,
            new_theme_name: String::new(),
            renaming: None,
            rename_buffer: String::new(),
        }
    }
}

fn swatch(ui: &mut egui::Ui, theme: &ThemeConfig) {
    ui.horizontal(|ui| {
        for c in [
            theme.bg,
            theme.surface,
            theme.accent,
            theme.success,
            theme.warning,
            theme.error,
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, c.to_egui());
        }
    });
}

impl CustomizePanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig) {
        ui.horizontal(|ui| {
            for (tab, label) in [
                (CustomizeSubTab::Themes, "Themes"),
                (CustomizeSubTab::Colors, "Colors"),
                (CustomizeSubTab::Effects, "Effects"),
                (CustomizeSubTab::Typography, "Typography"),
                (CustomizeSubTab::Layout, "Layout"),
                (CustomizeSubTab::ImportExport, "Import/Export"),
            ] {
                if ui.selectable_label(self.subtab == tab, label).clicked() {
                    self.subtab = tab;
                }
            }
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("customize_scroll")
            .show(ui, |ui| match self.subtab {
                CustomizeSubTab::Themes => self.ui_themes(ui, cfg),
                CustomizeSubTab::Colors => self.ui_colors(ui, cfg),
                CustomizeSubTab::Effects => self.ui_effects(ui, cfg),
                CustomizeSubTab::Typography => self.ui_typography(ui, cfg),
                CustomizeSubTab::Layout => self.ui_layout(ui, cfg),
                CustomizeSubTab::ImportExport => self.ui_import_export(ui, cfg),
            });
    }

    fn ui_themes(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig) {
        ui.label(egui::RichText::new("Built-in presets").strong());
        ui.horizontal_wrapped(|ui| {
            for (name, preset_fn) in ThemeConfig::all_presets() {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        let is_active = cfg.theme_config.preset == *name;
                        let preview = preset_fn();
                        swatch(ui, &preview);
                        if ui.selectable_label(is_active, *name).clicked() {
                            cfg.theme_config = preview;
                            cfg.theme = (*name).to_string();
                            cfg.needs_apply = true;
                        }
                    });
                });
            }
        });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Your custom themes").strong());

        let mut activate: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut duplicate: Option<usize> = None;
        for (i, named) in cfg.custom_themes.iter().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    swatch(ui, &named.theme);
                    if self.renaming == Some(i) {
                        ui.text_edit_singleline(&mut self.rename_buffer);
                        if ui.small_button("✔").clicked() {
                            self.renaming = None;
                        }
                    } else {
                        ui.label(&named.name);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").on_hover_text("Delete").clicked() {
                            delete = Some(i);
                        }
                        if ui.small_button("⧉").on_hover_text("Duplicate").clicked() {
                            duplicate = Some(i);
                        }
                        if ui.small_button("✏").on_hover_text("Rename").clicked() {
                            self.renaming = Some(i);
                            self.rename_buffer = named.name.clone();
                        }
                        if ui.button("Activate").clicked() {
                            activate = Some(i);
                        }
                    });
                });
            });
        }
        if let Some(i) = activate {
            cfg.theme_config = cfg.custom_themes[i].theme.clone();
            cfg.theme = cfg.custom_themes[i].name.clone();
            cfg.needs_apply = true;
        }
        if let Some(i) = delete {
            cfg.custom_themes.remove(i);
            self.renaming = None;
        }
        if let Some(i) = duplicate {
            let mut clone = cfg.custom_themes[i].clone();
            clone.name = format!("{} copy", clone.name);
            clone.id = format!("{}-copy-{}", clone.id, cfg.custom_themes.len());
            cfg.custom_themes.push(clone);
        }
        if let Some(i) = self.renaming {
            if let Some(named) = cfg.custom_themes.get_mut(i) {
                named.name = self.rename_buffer.clone();
            }
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.new_theme_name)
                .on_hover_text(
                    "Name for a new saved theme, created from the currently active theme.",
                );
            if ui.button("💾 Save current as new theme").clicked()
                && !self.new_theme_name.trim().is_empty()
            {
                cfg.custom_themes.push(NamedTheme {
                    id: format!("custom-{}", cfg.custom_themes.len()),
                    name: self.new_theme_name.trim().to_string(),
                    theme: cfg.theme_config.clone(),
                });
                self.new_theme_name.clear();
            }
        });
    }

    fn ui_colors(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig) {
        cfg.theme_config.ui_editor(ui, &mut cfg.theme);
        if ui.button("Apply").clicked() {
            cfg.needs_apply = true;
        }
    }

    fn ui_effects(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig) {
        let t = &mut cfg.theme_config;

        ui.label(egui::RichText::new("Corner roundness").strong());
        ui.add(egui::Slider::new(&mut t.corner.buttons, 0.0..=20.0).text("Buttons"));
        ui.add(egui::Slider::new(&mut t.corner.panels, 0.0..=20.0).text("Panels"));
        ui.add(egui::Slider::new(&mut t.corner.windows, 0.0..=20.0).text("Windows"));

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Glow").strong())
            .on_hover_text("Soft glow around the active sidebar tab and status indicators.");
        ui.checkbox(&mut t.glow.enabled, "Enabled");
        ui.horizontal(|ui| {
            ui.label("Color:");
            let mut c = t.glow.color.to_egui();
            if ui.color_edit_button_srgba(&mut c).changed() {
                let [r, g, b, a] = c.to_srgba_unmultiplied();
                t.glow.color = crate::theme::Rgba::from_rgba(r, g, b, a);
            }
        });
        ui.add(egui::Slider::new(&mut t.glow.radius, 0.0..=30.0).text("Radius"));
        ui.add(egui::Slider::new(&mut t.glow.intensity, 0.0..=1.0).text("Intensity"));

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Spectrum fill gradient").strong());
        let mut remove_idx: Option<usize> = None;
        for i in 0..t.spectrum_gradient.stops.len() {
            ui.horizontal(|ui| {
                let (pos, color) = &mut t.spectrum_gradient.stops[i];
                ui.add(egui::Slider::new(pos, 0.0..=1.0).text("pos"));
                let mut c = color.to_egui();
                if ui.color_edit_button_srgba(&mut c).changed() {
                    let [r, g, b, a] = c.to_srgba_unmultiplied();
                    *color = crate::theme::Rgba::from_rgba(r, g, b, a);
                }
                if t.spectrum_gradient.stops.len() > 2 && ui.small_button("🗑").clicked() {
                    remove_idx = Some(i);
                }
            });
        }
        if let Some(i) = remove_idx {
            t.spectrum_gradient.stops.remove(i);
        }
        if ui.small_button("+ Add stop").clicked() {
            t.spectrum_gradient
                .stops
                .push((0.5, t.spectrum_gradient.sample(0.5)));
        }

        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(260.0), 24.0),
            egui::Sense::hover(),
        );
        crate::fx::gradient_rect_vertical(ui.painter(), rect, &t.spectrum_gradient);

        if ui.button("Apply").clicked() {
            cfg.needs_apply = true;
        }
    }

    fn ui_typography(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig) {
        let t = &mut cfg.theme_config;
        ui.label(egui::RichText::new("Font family").strong());
        ui.horizontal(|ui| {
            for family in FontFamily::ALL {
                if ui
                    .selectable_label(t.typography.family == family, family.label())
                    .clicked()
                {
                    t.typography.family = family;
                }
            }
        });

        ui.add_space(6.0);
        ui.add(egui::Slider::new(&mut t.typography.heading_size, 10.0..=36.0).text("Heading size"));
        ui.add(egui::Slider::new(&mut t.typography.body_size, 8.0..=24.0).text("Body size"));
        ui.add(egui::Slider::new(&mut t.typography.small_size, 6.0..=18.0).text("Small text size"));
        ui.add(
            egui::Slider::new(&mut t.typography.monospace_size, 8.0..=24.0).text("Monospace size"),
        );
        ui.add(egui::Slider::new(&mut t.typography.button_size, 8.0..=24.0).text("Button size"));

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Widget density").strong());
        ui.horizontal(|ui| {
            for d in Density::ALL {
                if ui
                    .selectable_label(t.spacing.density == d, d.label())
                    .clicked()
                {
                    t.spacing.density = d;
                }
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.heading("Preview heading");
        ui.label("Preview body text — the quick brown fox jumps over the lazy dog.");
        ui.small("Preview small text");
        ui.monospace("Preview monospace 0123456789");

        if ui.button("Apply").clicked() {
            cfg.needs_apply = true;
        }
    }

    fn ui_layout(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig) {
        ui.label(egui::RichText::new("Main tabs").strong())
            .on_hover_text("Show/hide and reorder the icons in the top of the sidebar. The Customize tab itself is always shown.");
        Self::reorder_list(ui, &mut cfg.layout.main_tabs, "main_tabs");

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Secondary tools").strong())
            .on_hover_text("Show/hide and reorder the tool icons below the main tabs.");
        Self::reorder_list(ui, &mut cfg.layout.secondary_tools, "secondary_tools");

        ui.add_space(8.0);
        if ui.button("Reset layout to defaults").clicked() {
            cfg.layout = crate::config::LayoutConfig::default();
        }
    }

    fn reorder_list(ui: &mut egui::Ui, items: &mut [LayoutItem], salt: &str) {
        let len = items.len();
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        for (i, item) in items.iter_mut().enumerate() {
            ui.push_id(format!("{salt}_{i}"), |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut item.visible, &item.id);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if i + 1 < len && ui.small_button("↓").clicked() {
                            move_down = Some(i);
                        }
                        if i > 0 && ui.small_button("↑").clicked() {
                            move_up = Some(i);
                        }
                    });
                });
            });
        }
        if let Some(i) = move_up {
            items.swap(i, i - 1);
        }
        if let Some(i) = move_down {
            items.swap(i, i + 1);
        }
    }

    fn ui_import_export(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig) {
        ui.label("Export or import just the active theme (colors, effects, typography) as a JSON file — separate from the full app config.");
        ui.horizontal(|ui| {
            if ui.button("📤 Export theme…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{}.theme.json", cfg.theme_config.preset))
                    .add_filter("JSON", &["json"])
                    .save_file()
                {
                    if let Ok(json) = serde_json::to_string_pretty(&cfg.theme_config) {
                        if let Err(e) = std::fs::write(&path, json) {
                            eprintln!(
                                "[customize] failed to export theme to {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
            if ui.button("📥 Import theme…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
                {
                    if let Ok(data) = std::fs::read_to_string(&path) {
                        if let Ok(loaded) = serde_json::from_str::<ThemeConfig>(&data) {
                            cfg.theme_config = loaded;
                            cfg.needs_apply = true;
                        }
                    }
                }
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.label("Reset everything (colors, effects, typography, layout) to factory defaults. Does not touch SDR/AI/MQTT settings.");
        if ui.button("Reset customization").clicked() {
            cfg.theme_config = ThemeConfig::default();
            cfg.theme = "dark".to_string();
            cfg.layout = crate::config::LayoutConfig::default();
            cfg.needs_apply = true;
        }
    }
}
