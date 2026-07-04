# Satellite Image Editor — Implementation Plan

## Scope
Core editing suite replicating SatDump's daily-use features:
- Image viewer with zoom/pan/pixel info
- 6-channel MSU-MR display
- RGB composite system (presets + expression editor)
- Processing filters (equalize, white balance, brightness/contrast, HSL, median blur, normalize, invert)
- Live histogram display
- PNG export

## Architecture Overview

### New files
| File | Purpose |
|------|---------|
| `ez-gui/src/editor_panel.rs` | Main editor UI panel (~600 lines) |
| `ez-gui/src/image_processing.rs` | Pure-Rust image processing algorithms (~500 lines) |
| `ez-gui/src/composite.rs` | Channel composite expression parser + presets (~300 lines) |

### Modified files
| File | Change |
|------|--------|
| `ez-gui/src/main.rs` | Add `mod editor_panel; mod image_processing; mod composite;` |
| `ez-gui/src/satellite_panel.rs` | Add Editor to SatelliteSubTab, pass decoded channels to SharedState |
| `ez-gui/src/app.rs` | Add EditorPanel to CentralApp, wire 4th sub-tab, auto-load channels |

### Data flow
```
Decode complete → channels: HashMap<u16, GrayImage> stored in SharedState
     ↓
Editor loads channels → user selects composite → expression evaluated
     ↓
Processing pipeline: equalize → white balance → brightness/contrast → HSL → output
     ↓
Output RGBA texture → viewer (zoom/pan) + histogram + export
```

---

## Step 1: Add channel storage to SharedState

**File:** `ez-gui/src/app.rs`

Add field to `SharedState` struct:
```rust
pub decoded_channels: std::collections::HashMap<u16, image::GrayImage>,
```

Initialize in `CentralApp::new()`:
```rust
decoded_channels: std::collections::HashMap::new(),
```

**File:** `ez-gui/src/satellite_panel.rs`

Modify `DecodeThreadResult` to include raw channel data:
```rust
struct DecodeThreadResult {
    result: Option<DecodeResultDisplay>,
    channels: std::collections::HashMap<u16, image::GrayImage>,
    error: Option<String>,
}
```

In `start_decode()`, capture channels from `decode_result.images`:
```rust
let mut channels = std::collections::HashMap::new();
for (apid, img) in &decode_result.images {
    channels.insert(*apid, img.clone());
}
// send in thread_result
```

In `tick_decode()`, when complete, pass channels to shared state:
```rust
if let Ok(mut state) = self.shared.try_lock() {
    state.decoded_channels = thread_result.channels;
}
```

---

## Step 2: Composite presets and expression parser

**File:** `ez-gui/src/composite.rs` (NEW)

### 2a. Composite presets

```rust
pub struct CompositePreset {
    pub name: &'static str,
    pub description: &'static str,
    pub r_expr: &'static str,
    pub g_expr: &'static str,
    pub b_expr: &'static str,
}

pub const METEOR_COMPOSITES: &[CompositePreset] = &[
    CompositePreset { name: "221", description: "Standard visible color", r_expr: "ch2", g_expr: "ch2", b_expr: "ch1" },
    CompositePreset { name: "421", description: "VIS/IR blend", r_expr: "ch4", g_expr: "ch2", b_expr: "ch1" },
    CompositePreset { name: "321", description: "False color IR", r_expr: "ch3", g_expr: "ch2", b_expr: "ch1" },
    CompositePreset { name: "654", description: "Thermal IR", r_expr: "ch6", g_expr: "ch5", b_expr: "ch4" },
    CompositePreset { name: "543", description: "IR composite", r_expr: "ch5", g_expr: "ch4", b_expr: "ch3" },
    CompositePreset { name: "Natural Color", description: "True-ish color", r_expr: "ch2", g_expr: "ch2", b_expr: "ch1" },
    CompositePreset { name: "MCIR", description: "Cloud IR overlay", r_expr: "ch4", g_expr: "ch4", b_expr: "ch4" },
    CompositePreset { name: "Vegetation (NDVI)", description: "NDVI approximation", r_expr: "ch2-ch1", g_expr: "ch2+ch1", b_expr: "ch1" },
    CompositePreset { name: "Day/Night", description: "VIS day, IR night", r_expr: "ch1>0.1?ch2:ch4", g_expr: "ch1>0.1?ch2:ch4", b_expr: "ch1>0.1?ch1:ch5" },
    CompositePreset { name: "Thermal Enhanced", description: "Enhanced thermal", r_expr: "ch6^0.7", g_expr: "ch5^0.7", b_expr: "ch4^0.7" },
];
```

### 2b. Expression parser

Simple recursive descent parser supporting:
- Variable references: `ch1`..`ch6` (normalized to 0.0..1.0)
- Arithmetic: `+`, `-`, `*`, `/`
- Power: `^`
- Parentheses
- Ternary: `expr ? expr : expr`
- Comparisons in ternary: `<`, `>`, `<=`, `>=`, `==`
- Numeric literals: `0.5`, `1.0`, etc.

```rust
pub struct ExprParser { tokens: Vec<Token>, pos: usize }

enum Token {
    Var(u8), Num(f32), Op(char),
    LParen, RParen, Question, Colon,
    Lt, Gt, Le, Ge, Eq,
}

impl ExprParser {
    pub fn parse(input: &str) -> Result<Expr, String> { ... }
}

pub enum Expr {
    Var(u8),
    Num(f32),
    BinOp { op: char, left: Box<Expr>, right: Box<Expr> },
    Ternary { cond: Box<Expr>, cond_op: CmpOp, then: Box<Expr>, else_: Box<Expr> },
}

impl Expr {
    pub fn eval(&self, channels: &[f32; 6]) -> f32 { ... }
}
```

---

## Step 3: Image processing algorithms

**File:** `ez-gui/src/image_processing.rs` (NEW)

All functions operate on `&mut [f32]` buffers (0.0..1.0 range).

```rust
pub fn equalize(pixels: &mut [f32]) { ... }
pub fn equalize_rgb(r: &mut [f32], g: &mut [f32], b: &mut [f32]) { ... }
pub fn white_balance(r: &mut [f32], g: &mut [f32], b: &mut [f32], percentile: f32) { ... }
pub fn normalize(pixels: &mut [f32]) { ... }
pub fn brightness_contrast(pixels: &mut [f32], brightness: f32, contrast: f32) { ... }
pub fn hue_saturation(rgb: &mut [f32], hue_shift: f32, saturation: f32, lightness: f32) { ... }
pub fn median_blur(width: u32, height: u32, pixels: &mut [f32]) { ... }
pub fn invert(pixels: &mut [f32]) { ... }
pub fn histogram(pixels: &[f32]) -> [u32; 256] { ... }

pub struct ProcessingPipeline {
    pub equalize: bool,
    pub equalize_per_channel: bool,
    pub white_balance: bool,
    pub brightness: f32,    // -1.0..1.0
    pub contrast: f32,      // 0.0..2.0
    pub hue_shift: f32,     // -180..180
    pub saturation: f32,    // 0.0..2.0
    pub lightness: f32,     // -1.0..1.0
    pub median_blur: bool,
    pub invert: bool,
}

impl ProcessingPipeline {
    pub fn apply(&self, r: &mut [f32], g: &mut [f32], b: &mut [f32]) { ... }
}
```

---

## Step 4: Editor panel UI

**File:** `ez-gui/src/editor_panel.rs` (NEW)

### 4a. State

```rust
pub struct EditorPanel {
    channels: std::collections::HashMap<u8, Vec<f32>>,  // ch_num -> pixels
    channel_width: u32,
    channel_height: u32,
    selected_preset: usize,
    custom_r: String,
    custom_g: String,
    custom_b: String,
    use_custom: bool,
    pipeline: image_processing::ProcessingPipeline,
    histogram_r: [u32; 256],
    histogram_g: [u32; 256],
    histogram_b: [u32; 256],
    show_histogram: bool,
    zoom: f32,
    pan_x: f32,
    pan_y: f32,
    output_texture: Option<egui::TextureHandle>,
    output_rgba: Vec<u8>,
    view_mode: ViewMode,
    selected_channel: u8,
    dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode { Composite, SingleChannel }
```

### 4b. UI Layout

```
┌─────────────────────────────────────────────────────┐
│ [Composite ▾] [221] [421] [Natural] [MCIR] [...]    │
│ ┌─ Custom Expression ─────────────────────────────┐ │
│ │ R: ch2    G: ch2    B: ch1                     │ │
│ └─────────────────────────────────────────────────┘ │
├──────────────────────────────┬──────────────────────┤
│                              │ Processing           │
│                              │ ☐ Equalize           │
│    IMAGE VIEWER              │ ☐ Equalize Per-Ch    │
│    (zoom/pan/pixel info)     │ ☐ White Balance      │
│                              │ Brightness [──●──]   │
│                              │ Contrast   [──●──]   │
│                              │ Hue        [──●──]   │
│                              │ Saturation [──●──]   │
│                              │ Lightness  [──●──]   │
│                              │ ☐ Median Blur        │
│                              │ ☐ Invert             │
│                              ├──────────────────────┤
│                              │ Histogram            │
│                              │ [RGB bars]           │
│                              ├──────────────────────┤
│                              │ Channels             │
│                              │ [ch1][ch2][ch3]      │
│                              │ [ch4][ch5][ch6]      │
│                              ├──────────────────────┤
│                              │ [Export PNG]         │
└──────────────────────────────┴──────────────────────┘
```

### 4c. Image viewer interaction

- **Zoom** with scroll wheel (0.1x to 20x)
- **Pan** with left mouse drag
- **Pixel info** on hover: shows (x, y) and R/G/B values

### 4d. Histogram

RGB overlay histogram (80px tall) with semi-transparent colored bars.

### 4e. Channel thumbnails

6 small clickable thumbnails (one per MSU-MR channel). Click to view single channel in grayscale.

---

## Step 5: Wire editor into app.rs

**File:** `ez-gui/src/app.rs`

### 5a. Add EditorPanel to CentralApp

```rust
editor_panel: crate::editor_panel::EditorPanel,
```

### 5b. Add Editor to SatelliteSubTab

**File:** `ez-gui/src/satellite_panel.rs`

```rust
pub enum SatelliteSubTab { Track, Advanced, Decode, Editor }
```

### 5c. Sub-tab bar update

```rust
(SatelliteSubTab::Editor, "🖼 Editor"),
```

### 5d. Dispatch

Right panel: `SatelliteSubTab::Editor => self.editor_panel.ui(ui)`
Central panel: Editor gets its own central panel for the image viewer.

### 5e. Auto-load + auto-switch

On decode complete: load channels into editor, auto-switch to Editor tab.

---

## Step 6: Export

```rust
fn export_png(&self) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG Image", &["png"])
        .save_file()
    {
        let img = image::RgbaImage::from_raw(self.channel_width, self.channel_height, self.output_rgba.clone()).unwrap();
        let _ = img.save(&path);
    }
}
```

---

## Step 7: Tests

### image_processing.rs
- `equalize_spreads_histogram`
- `invert_flips_values`
- `normalize_stretches_range`
- `brightness_contrast_adjusts`
- `histogram_counts_correctly`

### composite.rs
- `parse_simple_variable`
- `parse_arithmetic`
- `parse_power`
- `parse_ternary`
- `preset_221_combines_correctly`

### editor_panel.rs
- `test_new_editor_empty`
- `test_load_channels`

---

## File Change Summary

| File | Lines Changed | Description |
|------|--------------|-------------|
| `ez-gui/src/main.rs` | +3 | Add mod declarations |
| `ez-gui/src/editor_panel.rs` | +~600 | NEW: Editor UI, viewer, controls, histogram, export |
| `ez-gui/src/image_processing.rs` | +~500 | NEW: equalize, white balance, B/C, HSL, blur, invert, histogram |
| `ez-gui/src/composite.rs` | +~300 | NEW: expression parser + Meteor presets |
| `ez-gui/src/satellite_panel.rs` | +~30 | Add Editor to SatelliteSubTab, pass channels on decode |
| `ez-gui/src/app.rs` | +~70 | Add EditorPanel, wire sub-tab, auto-load, auto-switch |

**Total: ~1,500 new lines across 6 files**

---

## Key Design Decisions

1. **f32 pixel buffers internally** — All processing in 0.0..1.0 float space, converted to u8 at display/export only.
2. **Lazy recomposite** — `dirty` flag set on control change; recomposite once per frame max.
3. **Minimal expression parser** — Arithmetic + ternary covers 95% of SatDump daily-use composites. No Lua needed.
4. **Texture caching** — `output_texture` only recreated when `dirty` is true.
5. **Auto-switch to Editor** — After successful decode, user is taken to Editor tab with channels pre-loaded.
6. **Channel data via SharedState** — Clean separation between decode and edit layers.
