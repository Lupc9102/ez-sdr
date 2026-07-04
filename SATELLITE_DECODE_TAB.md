# Satellite Decode Tab (DSU) — Implementation Plan

## Current State

### What exists:
- `lrpt-decode` crate: Complete Meteor-M2 LRPT decoder (QPSK demod → frame sync → RS → image)
- `SatellitePanel`: Pass tracking + Doppler correction UI (sub-tabs: Track / Advanced)
- `DiscordNotifier`: Notification kinds `lrpt_decode_started`, `lrpt_decode_complete`, `lrpt_decode_error` defined but **never fired**
- `AppTab` enum: `Sdr`, `AdsB`, `Satellite`, `Ai` — only 4 tabs
- `satellite_panel.rs:11`: `live_decode: bool` field exists but is **unwired**
- `ez-gui/Cargo.toml`: Does NOT depend on `lrpt-decode` crate
- Recordings: `.iq` files with `.json` sidecar (frequency, sample_rate, etc.)
- `app.rs:196`: `satellite_advanced: bool` field controls Track vs Advanced sub-tab
- `app.rs:587`: `satellite_panel.pending_status` already wired to `status_flash`

### What's missing:
1. `lrpt-decode` not in ez-gui dependencies
2. No "Decode" sub-tab in Satellite tab
3. No file import UI for .iq recordings
4. No decode pipeline wiring (thread, progress, image display)
5. No Discord fire calls for LRPT events
6. No preset system for Meteor decode settings

---

## Step 1: Add `lrpt-decode` as dependency of `ez-gui`

**File:** `ez-gui/Cargo.toml` — add after line 15 (`dump1090`):

```toml
lrpt-decode = { path = "../lrpt-decode" }
```

---

## Step 2: Add `SatelliteSubTab` enum to satellite_panel.rs

**File:** `ez-gui/src/satellite_panel.rs` — add before the `SatellitePanel` struct (after imports):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatelliteSubTab {
    Track,
    Advanced,
    Decode,
}
```

---

## Step 3: Add decode state to `SatellitePanel` struct

**File:** `ez-gui/src/satellite_panel.rs`

Add these fields to the `SatellitePanel` struct (after existing fields):

```rust
// Decode pipeline state
pub decode_file_path: String,
pub decode_running: bool,
pub decode_progress: Option<lrpt_decode::DecodeProgress>,
pub decode_result: Option<DecodeResultDisplay>,
pub decode_satellite_preset: DecodePreset,
pub decode_sample_rate: u32,
pub decode_symbol_rate: u32,
pub decode_output_dir: String,
pub decode_error: Option<String>,
decode_rx: Option<crossbeam_channel::Receiver<lrpt_decode::DecodeProgress>>,
decode_done_rx: Option<crossbeam_channel::Receiver<DecodeThreadResult>>,
decode_handle: Option<std::thread::JoinHandle<()>>,
pub pending_decode_tab: bool,
```

Add these types BEFORE the struct:

```rust
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DecodeResultDisplay {
    pub satellite: String,
    pub lines_decoded: u32,
    pub rs_ok: u32,
    pub rs_failed: u32,
    pub image_paths: Vec<String>,
    pub elapsed_ms: u64,
}

/// Internal message sent from decode thread when finished.
struct DecodeThreadResult {
    result: Option<DecodeResultDisplay>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodePreset {
    MeteorM2_2,
    MeteorM2_3,
    MeteorM2_4,
    Custom,
}

impl DecodePreset {
    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::MeteorM2_2 | Self::MeteorM2_3 | Self::MeteorM2_4 => 2_048_000,
            Self::Custom => 2_048_000,
        }
    }

    pub fn symbol_rate(&self) -> u32 {
        match self {
            Self::MeteorM2_2 | Self::MeteorM2_3 | Self::MeteorM2_4 => 72_000,
            Self::Custom => 72_000,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::MeteorM2_2 => "Meteor-M2-2 (137.1 MHz, LRPT)",
            Self::MeteorM2_3 => "Meteor-M2-3 (137.9 MHz, LRPT)",
            Self::MeteorM2_4 => "Meteor-M2-4 (137.1 MHz, LRPT)",
            Self::Custom => "Custom",
        }
    }

    pub fn all() -> &'static [DecodePreset] {
        &[Self::MeteorM2_2, Self::MeteorM2_3, Self::MeteorM2_4, Self::Custom]
    }
}
```

Initialize in `SatellitePanel::new()`:

```rust
decode_file_path: String::new(),
decode_running: false,
decode_progress: None,
decode_result: None,
decode_satellite_preset: DecodePreset::MeteorM2_2,
decode_sample_rate: 2_048_000,
decode_symbol_rate: 72_000,
decode_output_dir: "./decoded".to_string(),
decode_error: None,
decode_rx: None,
decode_done_rx: None,
decode_handle: None,
pending_decode_tab: false,
```

---

## Step 4: Implement `start_decode()` method

**File:** `ez-gui/src/satellite_panel.rs`

Add method to `impl SatellitePanel`:

```rust
fn start_decode(&mut self) {
    let path = PathBuf::from(&self.decode_file_path);
    let sample_rate = self.decode_sample_rate;
    let symbol_rate = self.decode_symbol_rate;
    let output_dir = PathBuf::from(&self.decode_output_dir);
    let preset_label = self.decode_satellite_preset.label().to_string();

    self.decode_running = true;
    self.decode_progress = None;
    self.decode_result = None;
    self.decode_error = None;

    let (progress_tx, progress_rx) = crossbeam_channel::unbounded();
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);
    self.decode_rx = Some(progress_rx);
    self.decode_done_rx = Some(done_rx);

    let handle = std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(&output_dir);
        let start = std::time::Instant::now();

        let result = lrpt_decode::decode_file(&path, sample_rate, symbol_rate, progress_tx);

        let thread_result = match result {
            Ok(decode_result) => {
                let mut image_paths = Vec::new();
                for (apid, img) in &decode_result.images {
                    let filename = format!(
                        "{}_apid{}.png",
                        path.file_stem().unwrap_or_default().to_string_lossy(),
                        apid
                    );
                    let img_path = output_dir.join(&filename);
                    let _ = img.save(&img_path);
                    image_paths.push(img_path.display().to_string());
                }

                let elapsed = start.elapsed().as_millis() as u64;
                DecodeThreadResult {
                    result: Some(DecodeResultDisplay {
                        satellite: preset_label,
                        lines_decoded: decode_result
                            .images
                            .iter()
                            .map(|(_, img)| img.height() as u32)
                            .max()
                            .unwrap_or(0),
                        rs_ok: decode_result.rs_ok,
                        rs_failed: decode_result.rs_failed,
                        image_paths,
                        elapsed_ms: elapsed,
                    }),
                    error: None,
                }
            }
            Err(e) => DecodeThreadResult {
                result: None,
                error: Some(format!("{e}")),
            },
        };

        let _ = done_tx.send(thread_result);
    });

    self.decode_handle = Some(handle);
}
```

---

## Step 5: Implement `tick_decode()` method

**File:** `ez-gui/src/satellite_panel.rs`

Add method to `impl SatellitePanel`:

```rust
/// Poll decode thread for progress/completion. Call every UI frame.
pub fn tick_decode(&mut self) {
    if !self.decode_running {
        return;
    }

    // Drain progress updates
    if let Some(ref rx) = self.decode_rx {
        while let Ok(progress) = rx.try_recv() {
            self.decode_progress = Some(progress);
        }
    }

    // Check for completion
    if let Some(ref rx) = self.decode_done_rx {
        if let Ok(thread_result) = rx.try_recv() {
            self.decode_running = false;
            self.decode_rx = None;
            self.decode_done_rx = None;
            if let Some(h) = self.decode_handle.take() {
                let _ = h.join();
            }

            if let Some(result) = thread_result.result {
                let total = result.rs_ok + result.rs_failed;
                let pct = if total > 0 {
                    result.rs_ok as f32 / total as f32 * 100.0
                } else {
                    0.0
                };
                self.pending_status = Some(format!(
                    "✅ Decoded {} — {} lines, RS {:.1}%",
                    result.satellite, result.lines_decoded, pct
                ));
                self.decode_result = Some(result);
            } else if let Some(err) = thread_result.error {
                self.pending_status = Some(format!("❌ Decode failed: {err}"));
                self.decode_error = Some(err);
            }
        }
    }
}
```

---

## Step 6: Implement `ui_decode()` method

**File:** `ez-gui/src/satellite_panel.rs`

Add method to `impl SatellitePanel`:

```rust
pub fn ui_decode(&mut self, ui: &mut egui::Ui) {
    ui.heading("Satellite Signal Decode");
    ui.add_space(4.0);

    // ── Preset Picker ──
    ui.label(egui::RichText::new("Preset").strong());
    egui::ComboBox::from_id_salt("decode_preset")
        .selected_text(self.decode_satellite_preset.label())
        .show_ui(ui, |ui| {
            for preset in DecodePreset::all() {
                ui.selectable_value(
                    &mut self.decode_satellite_preset,
                    *preset,
                    preset.label(),
                );
            }
        });

    ui.add_space(8.0);
    ui.separator();

    // ── File Import ──
    ui.label(egui::RichText::new("Import Recording").strong());
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut self.decode_file_path)
            .on_hover_text("Path to .iq recording file");
        if ui.button("Browse...").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("IQ Recording", &["iq"])
                .add_filter("All Files", &["*"])
                .pick_file()
            {
                self.decode_file_path = path.display().to_string();
                if let Some(sidecar) = lrpt_decode::load_sidecar(&path) {
                    self.decode_sample_rate = sidecar.sample_rate_hz;
                    match sidecar.frequency_hz {
                        137_100_000..=137_110_000 => {
                            self.decode_satellite_preset = DecodePreset::MeteorM2_2;
                        }
                        137_900_000..=137_920_000 => {
                            self.decode_satellite_preset = DecodePreset::MeteorM2_3;
                        }
                        _ => {
                            self.decode_satellite_preset = DecodePreset::Custom;
                        }
                    }
                }
            }
        }
    });

    // File info
    if !self.decode_file_path.is_empty() {
        let path = std::path::Path::new(&self.decode_file_path);
        if path.exists() {
            let size_mb = std::fs::metadata(path)
                .map(|m| m.len() as f64 / 1e6)
                .unwrap_or(0.0);
            ui.label(format!(
                "  {} ({:.1} MB)",
                path.file_name().unwrap_or_default().to_string_lossy(),
                size_mb
            ));
        } else {
            ui.colored_label(egui::Color32::RED, "  File not found");
        }
    }

    ui.add_space(8.0);
    ui.separator();

    // ── Decode Parameters (collapsible) ──
    ui.collapsing("Decode Parameters", |ui| {
        egui::Grid::new("decode_params")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Sample Rate:");
                ui.add(egui::DragValue::new(&mut self.decode_sample_rate).suffix(" Hz"));
                ui.end_row();
                ui.label("Symbol Rate:");
                ui.add(egui::DragValue::new(&mut self.decode_symbol_rate).suffix(" Hz"));
                ui.end_row();
            });
    });

    ui.add_space(8.0);

    // ── Output Directory ──
    ui.horizontal(|ui| {
        ui.label("Output:");
        ui.text_edit_singleline(&mut self.decode_output_dir);
        if ui
            .button("📁")
            .on_hover_text("Select output directory")
            .clicked()
        {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                self.decode_output_dir = dir.display().to_string();
            }
        }
    });

    ui.add_space(8.0);

    // ── Decode Button ──
    let can_decode = !self.decode_file_path.is_empty()
        && std::path::Path::new(&self.decode_file_path).exists()
        && !self.decode_running;

    if can_decode {
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("▶ Decode")
                        .size(15.0)
                        .strong(),
                )
                .min_size(egui::vec2(ui.available_width(), 32.0)),
            )
            .clicked()
        {
            self.start_decode();
        }
    } else if self.decode_running {
        ui.colored_label(egui::Color32::YELLOW, "⏳ Decoding...");
    }

    // ── Progress ──
    if let Some(ref progress) = self.decode_progress {
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.label(egui::RichText::new("Decode Progress").strong());
            let total = progress.rs_ok + progress.rs_failed;
            let pct = if total > 0 {
                progress.rs_ok as f32 / total as f32
            } else {
                0.0
            };
            ui.add(
                egui::ProgressBar::new(pct).text(format!(
                    "RS OK: {} / {} ({:.1}%)",
                    progress.rs_ok,
                    total,
                    pct * 100.0
                )),
            );
            ui.label(format!("Lines decoded: {}", progress.lines_decoded));
            ui.horizontal(|ui| {
                ui.label(if progress.costas_locked {
                    "🟢 Costas locked"
                } else {
                    "🔴 Costas unlocked"
                });
                ui.separator();
                ui.label(if progress.frame_locked {
                    "🟢 Frame sync"
                } else {
                    "🔴 Frame unlock"
                });
            });

            // Image preview thumbnails
            for (apid, img) in &progress.preview {
                ui.add_space(4.0);
                ui.label(format!(
                    "APID {} — {}×{} px, {} lines",
                    apid,
                    img.width(),
                    img.height(),
                    img.height()
                ));
            }
        });
    }

    // ── Result ──
    if let Some(ref result) = self.decode_result {
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(50, 255, 100),
                egui::RichText::new("✅ Decode Complete").strong(),
            );
            ui.label(format!("Satellite: {}", result.satellite));
            ui.label(format!(
                "Lines: {} · RS OK: {} · RS Failed: {}",
                result.lines_decoded, result.rs_ok, result.rs_failed
            ));
            ui.label(format!("Time: {} ms", result.elapsed_ms));
            if !result.image_paths.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Output Images:").strong());
                for path in &result.image_paths {
                    ui.label(format!("  📷 {path}"));
                }
                if ui.button("📂 Open Output Folder").clicked() {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&self.decode_output_dir)
                        .spawn();
                }
            }
        });
    }

    // ── Error ──
    if let Some(ref err) = self.decode_error {
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.colored_label(
                egui::Color32::RED,
                egui::RichText::new("❌ Decode Error").strong(),
            );
            ui.label(err.as_str());
        });
    }

    ui.add_space(8.0);
    ui.separator();

    // ── Recent Recordings ──
    ui.collapsing("Recent Recordings", |ui| {
        let recordings_dir = std::path::Path::new("./recordings");
        if recordings_dir.exists() {
            let mut files: Vec<_> = std::fs::read_dir(recordings_dir)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "iq")
                        .unwrap_or(false)
                })
                .collect();
            files.sort_by(|a, b| {
                b.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH)
                    .cmp(
                        &a.metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::UNIX_EPOCH),
                    )
            });

            for entry in files.iter().take(10) {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let size = entry
                    .metadata()
                    .map(|m| m.len() as f64 / 1e6)
                    .unwrap_or(0.0);
                if ui
                    .selectable_label(
                        false,
                        format!("{name} ({size:.1} MB)"),
                    )
                    .clicked()
                {
                    self.decode_file_path = path.display().to_string();
                    if let Some(sidecar) = lrpt_decode::load_sidecar(&path) {
                        self.decode_sample_rate = sidecar.sample_rate_hz;
                        match sidecar.frequency_hz {
                            137_100_000..=137_110_000 => {
                                self.decode_satellite_preset = DecodePreset::MeteorM2_2;
                            }
                            137_900_000..=137_920_000 => {
                                self.decode_satellite_preset = DecodePreset::MeteorM2_3;
                            }
                            _ => {
                                self.decode_satellite_preset = DecodePreset::Custom;
                            }
                        }
                    }
                }
            }
        } else {
            ui.label("No recordings directory found");
        }
    });
}
```

---

## Step 7: Add `request_decode_tab()` helper

**File:** `ez-gui/src/satellite_panel.rs`

Add method to `impl SatellitePanel`:

```rust
/// Called from Track sub-tab to jump to Decode with a preset pre-filled.
pub fn request_decode_tab(&mut self, preset: DecodePreset) {
    self.decode_satellite_preset = preset;
    self.pending_decode_tab = true;
}
```

---

## Step 8: Modify `render_satellite_tab()` in app.rs

**File:** `ez-gui/src/app.rs`

### 8a. Change field type (line ~196)

Replace:
```rust
satellite_advanced: bool,
```
With:
```rust
satellite_subtab: crate::satellite_panel::SatelliteSubTab,
```

### 8b. Initialize in `CentralApp::new()` (find where `satellite_advanced` is initialized)

Replace:
```rust
satellite_advanced: false,
```
With:
```rust
satellite_subtab: crate::satellite_panel::SatelliteSubTab::Track,
```

### 8c. Add `last_decode_running` field (after `satellite_subtab`)

```rust
last_decode_running: bool,
```

Initialize to `false` in `new()`.

### 8d. Update sub-tab bar (line ~2668)

Replace:
```rust
for (advanced, label) in [(false, "🛰 Track"), (true, "⚙ Advanced")] {
    let is_active = self.satellite_advanced == advanced;
    // ... styling ...
    if resp.clicked() {
        self.satellite_advanced = advanced;
    }
}
```

With:
```rust
use crate::satellite_panel::SatelliteSubTab;
for (subtab, label) in [
    (SatelliteSubTab::Track, "🛰 Track"),
    (SatelliteSubTab::Advanced, "⚙ Advanced"),
    (SatelliteSubTab::Decode, "📡 Decode"),
] {
    let is_active = self.satellite_subtab == subtab;
    let fg = if is_active {
        egui::Color32::from_rgb(0, 168, 255)
    } else {
        egui::Color32::GRAY
    };
    if ui
        .add(
            egui::Button::new(egui::RichText::new(label).color(fg))
                .fill(egui::Color32::TRANSPARENT),
        )
        .clicked()
    {
        self.satellite_subtab = subtab;
    }
}
```

### 8e. Update right panel dispatch (line ~2722)

Replace:
```rust
if self.satellite_advanced {
    self.satellite_panel.ui_advanced(ui);
} else {
    self.satellite_panel.ui_simple(ui);
}
```

With:
```rust
match self.satellite_subtab {
    SatelliteSubTab::Track => self.satellite_panel.ui_simple(ui),
    SatelliteSubTab::Advanced => self.satellite_panel.ui_advanced(ui),
    SatelliteSubTab::Decode => self.satellite_panel.ui_decode(ui),
}
```

### 8f. Handle decode tab request (after the right panel, before central panel)

Add after the right panel block:
```rust
// Handle "jump to decode" request from Track sub-tab
if self.satellite_panel.pending_decode_tab {
    self.satellite_panel.pending_decode_tab = false;
    self.satellite_subtab = SatelliteSubTab::Decode;
}
```

### 8g. Update central panel for Decode sub-tab (line ~2737)

Replace:
```rust
egui::CentralPanel::default().show(ui, |ui| {
    self.render_satellite_world_map(ui);
});
```

With:
```rust
egui::CentralPanel::default().show(ui, |ui| {
    if self.satellite_subtab == SatelliteSubTab::Decode {
        self.render_decode_central(ui);
    } else {
        self.render_satellite_world_map(ui);
    }
});
```

---

## Step 9: Add `render_decode_central()` to app.rs

**File:** `ez-gui/src/app.rs`

Add new method to `impl CentralApp`:

```rust
fn render_decode_central(&mut self, ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(8, 14, 24));

    if let Some(ref result) = self.satellite_panel.decode_result {
        // Show decoded images
        if !result.image_paths.is_empty() {
            let padding = 16.0;
            let mut y = rect.top() + padding;

            painter.text(
                egui::pos2(rect.left() + padding, y),
                egui::Align2::LEFT_TOP,
                format!("Decoded: {}", result.satellite),
                egui::FontId::proportional(16.0),
                egui::Color32::from_rgb(0, 168, 255),
            );
            y += 28.0;

            painter.text(
                egui::pos2(rect.left() + padding, y),
                egui::Align2::LEFT_TOP,
                format!(
                    "{} lines · RS OK: {} · {} ms",
                    result.lines_decoded, result.rs_ok, result.elapsed_ms
                ),
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(180, 190, 200),
            );
            y += 24.0;

            // Try to load and display images
            for path in &result.image_paths {
                let img_path = std::path::Path::new(path);
                if let Ok(img) = image::open(img_path) {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    let available_w = rect.width() - padding * 2.0;
                    let scale = (available_w / w as f32).min(1.0);
                    let disp_w = w as f32 * scale;
                    let disp_h = h as f32 * scale;

                    let img_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.left() + padding, y),
                        egui::vec2(disp_w, disp_h),
                    );

                    // Convert to egui texture
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [w as usize, h as usize],
                        rgba.as_raw(),
                    );
                    let tex_handle = ui.ctx().load_texture(
                        format!("decode_{path}"),
                        color_image,
                        egui::TextureOptions::default(),
                    );
                    ui.painter().image(
                        tex_handle.id(),
                        img_rect,
                        egui::Rect::from_min_size(
                            egui::pos2(0.0, 0.0),
                            egui::vec2(1.0, 1.0),
                        ),
                        egui::Color32::WHITE,
                    );

                    y += disp_h + padding;
                }
            }
        } else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Decode complete — no images produced",
                egui::FontId::proportional(14.0),
                egui::Color32::from_rgb(120, 130, 140),
            );
        }
    } else if self.satellite_panel.decode_running {
        // Show progress in center
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Decoding satellite signal...",
            egui::FontId::proportional(18.0),
            egui::Color32::from_rgb(0, 168, 255),
        );
        if let Some(ref progress) = self.satellite_panel.decode_progress {
            let status = format!(
                "Lines: {} · RS OK: {} · RS Failed: {}",
                progress.lines_decoded, progress.rs_ok, progress.rs_failed
            );
            painter.text(
                egui::pos2(rect.center().x, rect.center().y + 30.0),
                egui::Align2::CENTER_TOP,
                status,
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(180, 190, 200),
            );
        }
    } else {
        // Empty state
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "📡 Import a recording and click Decode\n\n\
             Supports .iq files from EZ-SDR Recorder\n\
             and other RTL-SDR compatible recordings.",
            egui::FontId::proportional(14.0),
            egui::Color32::from_rgb(100, 110, 120),
        );
    }
}
```

---

## Step 10: Wire decode polling + Discord notifications in `app.rs` logic()

**File:** `ez-gui/src/app.rs`

In the `logic()` method, add after the existing satellite panel handling (~line 587, after `self.satellite_panel.pending_status.take()`):

```rust
// Poll satellite decode pipeline
self.satellite_panel.tick_decode();

// Fire Discord notifications for decode events
if self.satellite_panel.decode_running && !self.last_decode_running {
    let sat = self.satellite_panel.decode_satellite_preset.label();
    let embed = crate::discord::embed_lrpt_decode_started(sat);
    self.discord.fire("lrpt_decode_started", embed);
}
if !self.satellite_panel.decode_running && self.last_decode_running {
    if let Some(ref result) = self.satellite_panel.decode_result {
        let total = result.rs_ok + result.rs_failed;
        let rs_pct = if total > 0 {
            result.rs_ok as f32 / total as f32 * 100.0
        } else {
            0.0
        };
        let embed = crate::discord::embed_lrpt_decode_complete(
            &result.satellite,
            result.lines_decoded,
            rs_pct,
            result.image_paths.first().unwrap_or(&String::new()),
        );
        self.discord.fire("lrpt_decode_complete", embed);
    } else if let Some(ref err) = self.satellite_panel.decode_error {
        let sat = self.satellite_panel.decode_satellite_preset.label();
        let embed = crate::discord::embed_lrpt_decode_error(sat, err);
        self.discord.fire("lrpt_decode_error", embed);
    }
}
self.last_decode_running = self.satellite_panel.decode_running;
```

---

## Step 11: Update existing tests + add new tests

**File:** `ez-gui/src/satellite_panel.rs`

### Update `test_new_defaults`:
Add assertions for new fields:
```rust
assert!(!panel.decode_running);
assert!(panel.decode_file_path.is_empty());
assert_eq!(panel.decode_satellite_preset, DecodePreset::MeteorM2_2);
assert_eq!(panel.decode_sample_rate, 2_048_000);
assert_eq!(panel.decode_symbol_rate, 72_000);
assert!(!panel.pending_decode_tab);
```

### Add new tests:
```rust
#[test]
fn test_decode_preset_values() {
    assert_eq!(DecodePreset::MeteorM2_2.sample_rate(), 2_048_000);
    assert_eq!(DecodePreset::MeteorM2_2.symbol_rate(), 72_000);
    assert_eq!(DecodePreset::MeteorM2_3.sample_rate(), 2_048_000);
    assert_eq!(DecodePreset::MeteorM2_4.sample_rate(), 2_048_000);
    assert_eq!(DecodePreset::Custom.symbol_rate(), 72_000);
}

#[test]
fn test_tick_decode_no_op_when_not_running() {
    let mut panel = SatellitePanel::new(make_shared_state());
    panel.tick_decode(); // should not panic
    assert!(panel.decode_result.is_none());
    assert!(panel.decode_error.is_none());
}

#[test]
fn test_request_decode_tab() {
    let mut panel = SatellitePanel::new(make_shared_state());
    assert!(!panel.pending_decode_tab);
    panel.request_decode_tab(DecodePreset::MeteorM2_3);
    assert!(panel.pending_decode_tab);
    assert_eq!(panel.decode_satellite_preset, DecodePreset::MeteorM2_3);
}

#[test]
fn test_ui_decode_no_crash() {
    let mut panel = SatellitePanel::new(make_shared_state());
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
        egui::Area::new(egui::Id::new("test_decode")).show(ctx, |ui| {
            panel.ui_decode(ui);
        });
    });
}
```

---

## Step 12: Build verification

```bash
cd /home/lupc/Documents/ez-sdr/ez-sdr
cargo build --release -p ez-gui
cargo test --workspace
cargo clippy --workspace
```

---

## File Change Summary

| File | Change |
|------|--------|
| `ez-gui/Cargo.toml` | Add `lrpt-decode` dependency |
| `ez-gui/src/satellite_panel.rs` | Add `SatelliteSubTab`, `DecodePreset`, `DecodeResultDisplay`, `DecodeThreadResult`, decode state fields, `start_decode()`, `tick_decode()`, `ui_decode()`, `request_decode_tab()`, recent recordings section, 4 new tests |
| `ez-gui/src/app.rs` | Change `satellite_advanced: bool` → `satellite_subtab: SatelliteSubTab`, add `last_decode_running: bool`, update `render_satellite_tab()` sub-tab bar + dispatch + central panel, add `render_decode_central()`, add decode polling + Discord fire in `logic()` |

## Estimated Lines Changed
- `satellite_panel.rs`: +~400 lines (types, state, UI, methods, tests)
- `app.rs`: +~120 lines (enum import, field changes, render methods, Discord fire)
- `Cargo.toml`: +1 line

## Key Design Decisions
1. **Two-channel completion**: `progress_tx/rx` for live updates, `done_tx/rx` for thread completion signal (prevents race between last progress and thread exit)
2. **Decode runs in background thread**: Non-blocking UI, progress shown via crossbeam channel
3. **Image display in central panel**: Decoded images rendered as egui textures in the satellite tab's central area (replaces world map when in Decode sub-tab)
4. **Preset auto-detection**: Loading a `.iq` file auto-detects satellite from sidecar frequency
5. **Recent recordings**: Scans `./recordings/` directory for quick import
6. **Discord pings**: Fires on decode start, complete (with image path), and error — uses existing `lrpt_decode_*` notification kinds
