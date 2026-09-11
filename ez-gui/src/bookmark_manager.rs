/// Bookmark management extracted from CentralApp.
///
/// This module handles all bookmark operations, removing ~300 lines from app.rs.
use crate::bookmarks::BookmarkDb;
use std::sync::{Arc, Mutex};

/// Bookmark manager with search and filtering.
pub struct BookmarkManager {
    db: Arc<Mutex<BookmarkDb>>,
    filter: String,
    sort_by: BookmarkSort,
    category_filter: Option<String>,
}

/// Bookmark sorting options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkSort {
    Frequency,
    Name,
    Category,
    RecentlyUsed,
}

impl BookmarkManager {
    pub fn new(db: Arc<Mutex<BookmarkDb>>) -> Self {
        Self {
            db,
            filter: String::new(),
            sort_by: BookmarkSort::Frequency,
            category_filter: None,
        }
    }

    /// Set search filter.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
    }

    /// Get current filter.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Set category filter.
    pub fn set_category_filter(&mut self, category: Option<String>) {
        self.category_filter = category;
    }

    /// Set sort order.
    pub fn set_sort(&mut self, sort: BookmarkSort) {
        self.sort_by = sort;
    }

    /// Get filtered and sorted bookmarks.
    pub fn get_filtered(&self) -> Vec<crate::bookmarks::Bookmark> {
        if let Ok(db) = self.db.lock() {
            let mut bookmarks = db.bookmarks.clone();

            // Apply text filter
            if !self.filter.is_empty() {
                let filter_lower = self.filter.to_lowercase();
                bookmarks.retain(|b| {
                    b.name.to_lowercase().contains(&filter_lower)
                        || b.notes.to_lowercase().contains(&filter_lower)
                        || b.category.to_lowercase().contains(&filter_lower)
                });
            }

            // Apply category filter
            if let Some(cat) = &self.category_filter {
                bookmarks.retain(|b| &b.category == cat);
            }

            // Sort
            match self.sort_by {
                BookmarkSort::Frequency => {
                    bookmarks.sort_by_key(|b| b.frequency_hz);
                }
                BookmarkSort::Name => {
                    bookmarks.sort_by(|a, b| a.name.cmp(&b.name));
                }
                BookmarkSort::Category => {
                    bookmarks.sort_by(|a, b| a.category.cmp(&b.category));
                }
                BookmarkSort::RecentlyUsed => {
                    // Would need usage tracking - for now, keep order
                }
            }

            bookmarks
        } else {
            Vec::new()
        }
    }

    /// Get all unique categories.
    pub fn get_categories(&self) -> Vec<String> {
        if let Ok(db) = self.db.lock() {
            let mut categories: Vec<String> =
                db.bookmarks.iter().map(|b| b.category.clone()).collect();
            categories.sort();
            categories.dedup();
            categories
        } else {
            Vec::new()
        }
    }

    /// Add a bookmark.
    pub fn add(&mut self, bookmark: crate::bookmarks::Bookmark) -> Result<(), String> {
        if let Ok(mut db) = self.db.lock() {
            db.bookmarks.push(bookmark);
            db.save();
            Ok(())
        } else {
            Err("Failed to lock bookmark database".to_string())
        }
    }

    /// Remove a bookmark by index.
    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        if let Ok(mut db) = self.db.lock() {
            if index < db.bookmarks.len() {
                db.bookmarks.remove(index);
                db.save();
                Ok(())
            } else {
                Err("Bookmark index out of range".to_string())
            }
        } else {
            Err("Failed to lock bookmark database".to_string())
        }
    }

    /// Update a bookmark.
    pub fn update(
        &mut self,
        index: usize,
        bookmark: crate::bookmarks::Bookmark,
    ) -> Result<(), String> {
        if let Ok(mut db) = self.db.lock() {
            if index < db.bookmarks.len() {
                db.bookmarks[index] = bookmark;
                db.save();
                Ok(())
            } else {
                Err("Bookmark index out of range".to_string())
            }
        } else {
            Err("Failed to lock bookmark database".to_string())
        }
    }

    /// Import bookmarks from CSV.
    pub fn import_csv(&mut self, csv_data: &str) -> (usize, String) {
        if let Ok(mut db) = self.db.lock() {
            db.import_csv(csv_data)
        } else {
            (0, "Failed to lock bookmark database".to_string())
        }
    }

    /// Export bookmarks to CSV.
    pub fn export_csv(&self) -> (String, String) {
        if let Ok(db) = self.db.lock() {
            db.export_csv()
        } else {
            (
                String::new(),
                "Failed to lock bookmark database".to_string(),
            )
        }
    }
}

/// Interactive bookmark management panel extracted from CentralApp.
pub struct BookmarkPanel {
    pub filter: String,
    pub show_starred_only: bool,
    pub bm_import_msg: String,
    pub show_add_bm: bool,
    pub new_bm_name: String,
    pub new_bm_freq_mhz: String,
    pub new_bm_mode: String,
    pub new_bm_category: String,
    pub new_bm_notes: String,
    pub new_bm_error: String,
    pub edit_bm_idx: Option<usize>,
    pub edit_bm_name: String,
    pub edit_bm_freq_mhz: String,
    pub edit_bm_mode: String,
    pub edit_bm_category: String,
    pub edit_bm_notes: String,
}

impl Default for BookmarkPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl BookmarkPanel {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            show_starred_only: false,
            bm_import_msg: String::new(),
            show_add_bm: false,
            new_bm_name: String::new(),
            new_bm_freq_mhz: String::new(),
            new_bm_mode: "WFM".to_string(),
            new_bm_category: String::new(),
            new_bm_notes: String::new(),
            new_bm_error: String::new(),
            edit_bm_idx: None,
            edit_bm_name: String::new(),
            edit_bm_freq_mhz: String::new(),
            edit_bm_mode: String::new(),
            edit_bm_category: String::new(),
            edit_bm_notes: String::new(),
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        shared: &Arc<Mutex<crate::app::SharedState>>,
        last_manual_tune_time: &mut std::time::Instant,
        ai_panel_input: &mut String,
        status_bar: &mut crate::status_bar::StatusBar,
    ) {
        let bm_count = if let Ok(state) = shared.try_lock() {
            state.bookmarks.bookmarks.len()
        } else {
            0
        };
        ui.heading("Frequency Bookmarks");
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{bm_count} bookmarks"));
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter...").desired_width(150.0));
            ui.toggle_value(&mut self.show_starred_only, "⭐ Starred").on_hover_text("Show only starred (favorite) bookmarks");
            if ui.button("💾 Save").on_hover_text("Save all bookmarks to ez_sdr_bookmarks.json in the current directory.").clicked() {
                if let Ok(state) = shared.try_lock() {
                    state.bookmarks.save();
                }
            }
            if ui.button("📂 Load").on_hover_text("Reload bookmarks from ez_sdr_bookmarks.json, replacing the current list.").clicked() {
                if let Ok(mut state) = shared.try_lock() {
                    if let Some(loaded) = crate::bookmarks::BookmarkDb::load_saved() {
                        state.bookmarks.bookmarks = loaded;
                        state.bookmarks_modified = true;
                        state.spectrum.bookmark_freqs_dirty = true;
                    }
                }
            }
            if ui.button("📥 Import CSV").on_hover_text("Import bookmarks from a CSV file (columns: name,frequency_hz,mode,category). Appends to current list.").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).pick_file() {
                    if let Some(path_str) = path.to_str() {
                        if let Ok(mut state) = shared.try_lock() {
                            let (count, err) = state.bookmarks.import_csv(path_str);
                            if err.is_empty() {
                                self.bm_import_msg = format!("Imported {count} bookmarks.");
                                state.bookmarks_modified = true;
                                state.spectrum.bookmark_freqs_dirty = true;
                            } else {
                                self.bm_import_msg = err;
                            }
                        }
                    }
                }
            }
            if ui.button("📤 Export CSV").on_hover_text("Export all bookmarks to a timestamped CSV file in the current directory.").clicked() {
                if let Ok(state) = shared.try_lock() {
                    let (path, err) = state.bookmarks.export_csv();
                    if err.is_empty() {
                        self.bm_import_msg = format!("Exported to {path}");
                    } else {
                        self.bm_import_msg = err;
                    }
                }
            }
            if ui.small_button("A→Z").on_hover_text("Sort all bookmarks alphabetically by name within each category.").clicked() {
                if let Ok(mut state) = shared.try_lock() {
                    state.bookmarks.bookmarks.sort_by_key(|a| a.name.to_lowercase());
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
            if ui.small_button("Hz↑").on_hover_text("Sort all bookmarks by frequency (lowest first) within each category.").clicked() {
                if let Ok(mut state) = shared.try_lock() {
                    state.bookmarks.bookmarks.sort_by_key(|b| b.frequency_hz);
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
            if ui.button(if self.show_add_bm { "✕ Cancel" } else { "+ Add" })
                .on_hover_text("Add a new bookmark for the current or any frequency.")
                .clicked()
            {
                self.show_add_bm = !self.show_add_bm;
                self.new_bm_error.clear();
                if self.show_add_bm {
                    if let Ok(state) = shared.try_lock() {
                        self.new_bm_freq_mhz = format!("{:.4}", state.source.frequency_hz as f64 / 1e6);
                        self.new_bm_mode = state.demod_mode.label().to_string();
                    }
                }
            }
        });

        // Add bookmark form
        if self.show_add_bm {
            ui.group(|ui| {
                ui.label(egui::RichText::new("New Bookmark").strong());
                egui::Grid::new("add_bm_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_bm_name)
                                .desired_width(200.0)
                                .hint_text("e.g. Local Police"),
                        );
                        ui.end_row();
                        ui.label("Freq (MHz):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_bm_freq_mhz)
                                .desired_width(120.0)
                                .hint_text("145.5"),
                        );
                        ui.end_row();
                        ui.label("Mode:");
                        egui::ComboBox::from_id_salt("bm_mode_combo")
                            .selected_text(self.new_bm_mode.as_str())
                            .width(ui.available_width().min(100.0))
                            .show_ui(ui, |ui| {
                                for m in ["NFM", "WFM", "AM", "USB", "LSB", "RAW"] {
                                    ui.selectable_value(&mut self.new_bm_mode, m.to_string(), m);
                                }
                            });
                        ui.end_row();
                        ui.label("Category:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_bm_category)
                                .desired_width(150.0)
                                .hint_text("Custom"),
                        );
                        ui.end_row();
                        ui.label("Notes:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_bm_notes)
                                .desired_width(250.0)
                                .hint_text("Optional notes about this signal"),
                        );
                        ui.end_row();
                    });
                if !self.new_bm_error.is_empty() {
                    ui.colored_label(egui::Color32::RED, self.new_bm_error.as_str());
                }
                if ui.button("Save Bookmark").clicked() {
                    let name = self.new_bm_name.trim().to_string();
                    let freq_str = self.new_bm_freq_mhz.trim().to_string();
                    match freq_str.parse::<f64>() {
                        Ok(mhz) if mhz > 0.0 && !name.is_empty() => {
                            let bm = crate::bookmarks::Bookmark {
                                name,
                                frequency_hz: (mhz * 1e6) as u64,
                                mode: self.new_bm_mode.clone(),
                                bandwidth_hz: 12_500,
                                category: if self.new_bm_category.trim().is_empty() {
                                    "Custom".to_string()
                                } else {
                                    self.new_bm_category.trim().to_string()
                                },
                                notes: self.new_bm_notes.trim().to_string(),
                                starred: false,
                            };
                            if let Ok(mut state) = shared.try_lock() {
                                state.bookmarks.bookmarks.push(bm);
                                state.bookmarks_modified = true;
                                state.spectrum.bookmark_freqs_dirty = true;
                            }
                            self.new_bm_name.clear();
                            self.new_bm_notes.clear();
                            self.new_bm_error.clear();
                            self.show_add_bm = false;
                        }
                        Ok(_) => self.new_bm_error = "Frequency must be > 0 MHz".to_string(),
                        Err(_) if name.is_empty() => {
                            self.new_bm_error = "Name cannot be empty".to_string();
                        }
                        Err(_) => {
                            self.new_bm_error =
                                "Invalid frequency — enter a number like 145.5".to_string();
                        }
                    }
                }
            });
        }

        if !self.bm_import_msg.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(100, 220, 100),
                self.bm_import_msg.as_str(),
            );
        }

        ui.separator();

        let filter_lower = self.filter.to_lowercase();
        let bookmarks_snapshot = if let Ok(state) = shared.try_lock() {
            state.bookmarks.bookmarks.clone()
        } else {
            return;
        };

        // Category quick-filter chips
        {
            let mut all_cats: Vec<String> = bookmarks_snapshot
                .iter()
                .map(|b| b.category.clone())
                .collect();
            all_cats.sort();
            all_cats.dedup();
            if all_cats.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    ui.small("Filter by:");
                    for cat in &all_cats {
                        let is_active = filter_lower == cat.to_lowercase();
                        let btn = ui
                            .add(
                                egui::Button::new(egui::RichText::new(cat.as_str()).small().color(
                                    if is_active {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::from_rgb(180, 200, 240)
                                    },
                                ))
                                .fill(if is_active {
                                    egui::Color32::from_rgb(80, 160, 255)
                                } else {
                                    egui::Color32::from_rgba_premultiplied(40, 60, 100, 80)
                                })
                                .small(),
                            )
                            .on_hover_text(format!(
                                "Click to filter by '{cat}' category. Click again to clear."
                            ));
                        if btn.clicked() {
                            if is_active {
                                self.filter.clear();
                            } else {
                                self.filter = cat.clone();
                            }
                        }
                    }
                    if !filter_lower.is_empty() && ui.small_button("✕ Clear").clicked() {
                        self.filter.clear();
                    }
                });
            }
        }

        let filtered: Vec<(usize, &crate::bookmarks::Bookmark)> = bookmarks_snapshot
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                let matches_starred = !self.show_starred_only || b.starred;
                let matches_text = filter_lower.is_empty()
                    || b.name.to_lowercase().contains(&filter_lower)
                    || b.category.to_lowercase().contains(&filter_lower)
                    || b.mode.to_lowercase().contains(&filter_lower)
                    || b.freq_display().contains(&filter_lower);
                matches_starred && matches_text
            })
            .collect();

        let mut categories: Vec<String> =
            filtered.iter().map(|(_, b)| b.category.clone()).collect();
        categories.sort();
        categories.dedup();

        let mut delete_idx: Option<usize> = None;
        let mut duplicate_idx: Option<usize> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for cat in &categories {
                let cat_count = filtered.iter().filter(|(_, b)| &b.category == cat).count();
                let cat_header = format!("{cat} ({cat_count})");
                ui.collapsing(cat_header, |ui| {
                    for (orig_idx, bm) in filtered.iter().filter(|(_, b)| &b.category == cat) {
                        let is_editing = self.edit_bm_idx == Some(*orig_idx);
                        let row_response = ui.horizontal(|ui| {
                            if is_editing {
                                // Inline edit row
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_name).desired_width(120.0).hint_text("Name"));
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_freq_mhz).desired_width(70.0).hint_text("MHz"));
                                egui::ComboBox::from_id_salt(format!("edit_mode_{orig_idx}"))
                                    .selected_text(self.edit_bm_mode.as_str())
                                    .width(ui.available_width().min(80.0))
                                    .show_ui(ui, |ui| {
                                        for m in ["NFM", "WFM", "AM", "USB", "LSB", "RAW"] {
                                            ui.selectable_value(&mut self.edit_bm_mode, m.to_string(), m);
                                        }
                                    });
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_category).desired_width(80.0).hint_text("Category"));
                                ui.add(egui::TextEdit::singleline(&mut self.edit_bm_notes).desired_width(120.0).hint_text("Notes"));
                                if ui.small_button("✓").on_hover_text("Save changes").clicked() {
                                    if let Ok(freq_mhz) = self.edit_bm_freq_mhz.trim().parse::<f64>() {
                                        if let Ok(mut state) = shared.try_lock() {
                                            if let Some(bm) = state.bookmarks.bookmarks.get_mut(*orig_idx) {
                                                bm.name = self.edit_bm_name.trim().to_string();
                                                bm.frequency_hz = (freq_mhz * 1e6) as u64;
                                                bm.mode = self.edit_bm_mode.clone();
                                                bm.category = if self.edit_bm_category.trim().is_empty() { "Custom".into() } else { self.edit_bm_category.trim().to_string() };
                                                bm.notes = self.edit_bm_notes.trim().to_string();
                                                state.bookmarks_modified = true;
                                                state.spectrum.bookmark_freqs_dirty = true;
                                            }
                                        }
                                    }
                                    self.edit_bm_idx = None;
                                }
                                if ui.small_button("✕").on_hover_text("Cancel edit").clicked() {
                                    self.edit_bm_idx = None;
                                }
                            } else {
                                if *orig_idx < 9 {
                                    ui.colored_label(egui::Color32::from_rgb(100, 180, 255), format!("[{}]", orig_idx + 1))
                                        .on_hover_text(format!("Press {} to tune here instantly", orig_idx + 1));
                                }
                                ui.label(&bm.name);
                                ui.monospace(bm.freq_display());
                                ui.small(&bm.mode);
                                let tune_tip = if bm.notes.is_empty() {
                                    format!("Double-click or click 'Tune' to tune to {} in {} mode", bm.freq_display(), bm.mode)
                                } else {
                                    format!("Double-click or click 'Tune' to tune to {} in {} mode\n{}", bm.freq_display(), bm.mode, bm.notes)
                                };
                                if ui.small_button("Tune")
                                    .on_hover_text(&tune_tip)
                                    .clicked()
                                {
                                    if let Ok(mut state) = shared.try_lock() {
                                        state.source.frequency_hz = bm.frequency_hz;
                                        *last_manual_tune_time = std::time::Instant::now();
                                        if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&bm.mode) {
                                            state.demod_mode = mode;
                                        }
                                    }
                                }
                                if ui.small_button("✏")
                                    .on_hover_text("Edit this bookmark")
                                    .clicked()
                                {
                                    self.edit_bm_idx = Some(*orig_idx);
                                    self.edit_bm_name = bm.name.clone();
                                    self.edit_bm_freq_mhz = format!("{:.4}", bm.frequency_hz as f64 / 1e6);
                                    self.edit_bm_mode = bm.mode.clone();
                                    self.edit_bm_category = bm.category.clone();
                                    self.edit_bm_notes = bm.notes.clone();
                                }
                                let star_icon = if bm.starred { "⭐" } else { "☆" };
                                if ui.small_button(star_icon)
                                    .on_hover_text(if bm.starred { "Remove from favorites" } else { "Add to favorites" })
                                    .clicked()
                                {
                                    if let Ok(mut state) = shared.try_lock() {
                                        if let Some(bookmark) = state.bookmarks.bookmarks.get_mut(*orig_idx) {
                                            bookmark.starred = !bookmark.starred;
                                        }
                                    }
                                }
                                if ui.small_button("📋")
                                    .on_hover_text(format!("Copy {} to clipboard", bm.freq_display()))
                                    .clicked()
                                {
                                    ui.ctx().copy_text(bm.freq_display());
                                }
                                if ui.small_button("⧉")
                                    .on_hover_text("Duplicate this bookmark")
                                    .clicked()
                                {
                                    duplicate_idx = Some(*orig_idx);
                                }
                                if ui.small_button("🗑")
                                    .on_hover_text("Delete this bookmark")
                                    .clicked()
                                {
                                    delete_idx = Some(*orig_idx);
                                }
                                let bm_freq_mhz = bm.frequency_hz as f64 / 1e6;
                                if ui.small_button("🤖")
                                    .on_hover_text(format!("Ask AI about {}", bm.freq_display()))
                                    .clicked()
                                {
                                    *ai_panel_input = format!(
                                        "Tell me about the bookmark \"{}\": {:.4} MHz ({} mode). \
                                         What signals should I expect here, and what are the best settings?",
                                        bm.name, bm_freq_mhz, bm.mode
                                    );
                                    status_bar.info(
                                        format!("🤖 AI prompt ready for {} — switch to the AI tab", bm.freq_display())
                                    );
                                }
                            }
                        });
                        // Double-click to tune
                        if !is_editing && row_response.response.double_clicked() {
                            if let Ok(mut state) = shared.try_lock() {
                                state.source.frequency_hz = bm.frequency_hz;
                                if let Some(mode) = crate::sdr_panel::DemodMode::from_label(&bm.mode) {
                                    state.demod_mode = mode;
                                }
                            }
                        }
                    }
                });
            }
        });
        if let Some(idx) = delete_idx {
            if let Ok(mut state) = shared.try_lock() {
                if idx < state.bookmarks.bookmarks.len() {
                    state.bookmarks.bookmarks.remove(idx);
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
        }
        if let Some(idx) = duplicate_idx {
            if let Ok(mut state) = shared.try_lock() {
                if let Some(bm) = state.bookmarks.bookmarks.get(idx).cloned() {
                    let mut copy = bm;
                    copy.name = format!("{} (copy)", copy.name);
                    state.bookmarks.bookmarks.push(copy);
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_manager_creation() {
        let db = Arc::new(Mutex::new(BookmarkDb::new()));
        let manager = BookmarkManager::new(db);
        assert_eq!(manager.filter(), "");
    }

    #[test]
    fn bookmark_manager_filter() {
        let db = Arc::new(Mutex::new(BookmarkDb::new()));
        let mut manager = BookmarkManager::new(db);
        manager.set_filter("test".to_string());
        assert_eq!(manager.filter(), "test");
    }
}
