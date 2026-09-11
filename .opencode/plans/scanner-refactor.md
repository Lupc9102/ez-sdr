# SDR Frequency Scanner Refactor & Calibration Upgrade

## Context

Three issues need to be resolved in the EZ-SDR frequency scanner module:

1. **"ai configure" broken**: `app.rs:1287` calls `self.scanner.apply_command(&cmd)` but `FrequencyScanner` has no `apply_command` method -- compilation error that prevents AI-initiated scanner configuration.
2. **Preset selection unreliable**: Preset buttons (`scanner.rs:815-821`) directly mutate `start_hz/stop_hz/step_hz`, but when a scan is already running, the scanner's `current_freq_hz` and `progress` are not reset. The sweep can end up out-of-range or not restart properly.
3. **Antenna calibration loop missing**: No interactive dipole-tuning workflow exists.

---

## Task 1: Fix `apply_command` -- `scanner.rs`

**File**: `ez-gui/src/scanner.rs`

Add `apply_command` method to `FrequencyScanner` after `stop_memory_scan` (~line 137):

```rust
pub fn apply_command(&mut self, cmd: &crate::app::ScannerCommand) {
    if let Some(hz) = cmd.start_hz {
        self.start_hz = hz;
    }
    if let Some(hz) = cmd.stop_hz {
        self.stop_hz = hz;
    }
    if let Some(hz) = cmd.step_hz {
        self.step_hz = hz;
    }
    if let Some(ms) = cmd.dwell_ms {
        self.dwell_ms = ms;
    }
    if let Some(db) = cmd.threshold_db {
        self.threshold_db = db;
    }
    if let Some(run) = cmd.run {
        if run {
            if !self.enabled {
                self.start();
            }
        } else {
            self.stop();
        }
    }
    // Clamp current_freq_hz into the new range if scan is running
    if self.enabled {
        if self.current_freq_hz < self.start_hz || self.current_freq_hz > self.stop_hz {
            self.current_freq_hz = self.start_hz;
            self.tune_request_hz = Some(self.current_freq_hz);
        }
    }
    self.status_text = if self.enabled {
        format!(
            "Scanning {:.3}--{:.3} MHz",
            self.start_hz as f64 / 1e6,
            self.stop_hz as f64 / 1e6
        )
    } else {
        "Scanner configured (idle)".into()
    };
}
```

---

## Task 2: Fix preset selection -- `scanner.rs`

**File**: `ez-gui/src/scanner.rs`, lines 815-821

Replace the preset button handler to also reset current frequency when running:

```rust
for &(name, start, stop, step, tip) in BAND_PRESETS {
    if ui.small_button(name).on_hover_text(tip).clicked() {
        self.start_hz = start;
        self.stop_hz = stop;
        self.step_hz = step;
        // If scan is running, clamp current frequency into new range
        if self.enabled {
            if self.current_freq_hz < self.start_hz || self.current_freq_hz > self.stop_hz {
                self.current_freq_hz = self.start_hz;
                self.tune_request_hz = Some(self.current_freq_hz);
            }
            self.progress = 0.0;
            self.status_text = format!(
                "Scanning {:.3}--{:.3} MHz",
                self.start_hz as f64 / 1e6,
                self.stop_hz as f64 / 1e6
            );
        }
    }
}
```

---

## Task 3: Antenna Calibration Loop -- `scanner.rs`

### 3a. Add calibration phase enum

Before `HitsSort` enum (~line 58):

```rust
#[derive(Debug, Clone, PartialEq)]
enum CalibrationPhase {
    Idle,
    WaitingForInitialMeasurement,
    WaitingForExtension,
    Complete,
}

#[derive(Debug, Clone)]
struct CalibrationMeasurement {
    element_length_cm: f64,
    freq_hz: u64,
    strength_db: f32,
}
```

### 3b. Add calibration fields to `FrequencyScanner` struct

After `hits_sort` field:

```rust
// Antenna calibration
pub calibration_active: bool,
calibration_phase: CalibrationPhase,
calibration_step: usize,
calibration_element_length_cm: f64,
calibration_measurements: Vec<CalibrationMeasurement>,
calibration_msg: String,
calibration_freqs_at_lengths: Vec<(f64, u64)>,
calibration_delta_log: Vec<String>,
```

### 3c. Initialize in `new()`

Add to the `Self { ... }` block:

```rust
calibration_active: false,
calibration_phase: CalibrationPhase::Idle,
calibration_step: 0,
calibration_element_length_cm: 10.0,
calibration_measurements: Vec::new(),
calibration_msg: String::new(),
calibration_freqs_at_lengths: Vec::new(),
calibration_delta_log: Vec::new(),
```

### 3d. Add calibration methods

```rust
pub fn start_calibration(&mut self) {
    self.calibration_active = true;
    self.calibration_phase = CalibrationPhase::WaitingForInitialMeasurement;
    self.calibration_step = 0;
    self.calibration_element_length_cm = 10.0;
    self.calibration_measurements.clear();
    self.calibration_freqs_at_lengths.clear();
    self.calibration_delta_log.clear();
    self.calibration_msg = format!(
        "Step 1/3: Construct a 180-degree dipole with each element at {:.0} cm. \
         Tune to the strongest signal and press 'Log Measurement'.",
        self.calibration_element_length_cm
    );
}

pub fn log_calibration_measurement(&mut self, peak_freq_hz: u64, peak_db: f32) {
    self.calibration_measurements.push(CalibrationMeasurement {
        element_length_cm: self.calibration_element_length_cm,
        freq_hz: peak_freq_hz,
        strength_db: peak_db,
    });
    self.calibration_freqs_at_lengths
        .push((self.calibration_element_length_cm, peak_freq_hz));

    if self.calibration_freqs_at_lengths.len() >= 2 {
        let prev = self.calibration_freqs_at_lengths
            [self.calibration_freqs_at_lengths.len() - 2];
        let curr = *self.calibration_freqs_at_lengths.last().unwrap();
        let delta_hz = curr.1 as i64 - prev.1 as i64;
        let delta_cm = curr.0 - prev.0;
        self.calibration_delta_log.push(format!(
            "{:.0}cm -> {:.0}cm: {:+.3} kHz ({:+} Hz/cm)",
            prev.0,
            curr.0,
            delta_hz as f64 / 1e3,
            delta_hz as i64 / delta_cm.max(1.0) as i64
        ));
    }

    self.calibration_step += 1;
    self.calibration_element_length_cm += 10.0;

    // 3 measurements total: 10cm, 20cm, 30cm
    if self.calibration_measurements.len() >= 3 {
        self.calibration_phase = CalibrationPhase::Complete;
        self.calibration_active = false;
        let mut summary = String::from("=== Antenna Calibration Complete ===\n");
        for m in &self.calibration_measurements {
            summary.push_str(&format!(
                "  {:.0} cm -> {:.3} MHz ({:.1} dB)\n",
                m.element_length_cm,
                m.freq_hz as f64 / 1e6,
                m.strength_db
            ));
        }
        summary.push_str("\nDeltas:\n");
        for d in &self.calibration_delta_log {
            summary.push_str(&format!("  {d}\n"));
        }
        if self.calibration_measurements.len() >= 2 {
            let first = self.calibration_measurements.first().unwrap();
            let last = self.calibration_measurements.last().unwrap();
            let total_shift = last.freq_hz as i64 - first.freq_hz as i64;
            summary.push_str(&format!(
                "\nTotal shift: {:+.3} kHz over {:.0} cm",
                total_shift as f64 / 1e3,
                last.element_length_cm - first.element_length_cm
            ));
        }
        self.calibration_msg = summary;
    } else {
        self.calibration_phase = CalibrationPhase::WaitingForExtension;
        self.calibration_msg = format!(
            "Step {}/3: Extend each element by +10 cm (now {:.0} cm total). \
             Tune to the strongest signal and press 'Log Measurement'.",
            self.calibration_measurements.len() + 1,
            self.calibration_element_length_cm
        );
    }
}

pub fn stop_calibration(&mut self) {
    self.calibration_active = false;
    self.calibration_phase = CalibrationPhase::Idle;
    self.calibration_msg.clear();
}
```

### 3e. Add calibration UI

Insert in `ui()` method, after the memory scan collapsing section (~line 857) and before the grid controls:

```rust
ui.collapsing("Antenna Calibration (dipole tuning)", |ui| {
    ui.label("Interactive loop for physical antenna tuning. Follow prompts to measure how element length affects resonant frequency.");
    ui.add_space(4.0);

    if !self.calibration_active && self.calibration_phase == CalibrationPhase::Idle {
        if ui.button("Start Calibration").on_hover_text(
            "Begin a 3-step dipole calibration: 10cm, 20cm, 30cm elements"
        ).clicked() {
            self.start_calibration();
        }
    }

    if self.calibration_active {
        ui.colored_label(egui::Color32::from_rgb(255, 200, 50), &self.calibration_msg);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Log Measurement").on_hover_text(
                "Record the current peak frequency as the resonant point for this antenna length."
            ).clicked() {
                let freq = self.current_freq_hz;
                let db = self.last_peak_db;
                self.log_calibration_measurement(freq, db);
            }
            if ui.button("Cancel").clicked() {
                self.stop_calibration();
            }
        });
        let progress = self.calibration_measurements.len() as f32 / 3.0;
        ui.add(egui::ProgressBar::new(progress).desired_width(200.0)
            .text(format!("{}/3 measurements", self.calibration_measurements.len())));
    }

    if self.calibration_phase == CalibrationPhase::Complete && !self.calibration_msg.is_empty() {
        ui.group(|ui| {
            ui.label(egui::RichText::new("Results").strong());
            ui.label(&self.calibration_msg);
        });
        ui.horizontal(|ui| {
            if ui.button("Restart").clicked() {
                self.start_calibration();
            }
            if ui.button("Dismiss").clicked() {
                self.calibration_phase = CalibrationPhase::Idle;
                self.calibration_msg.clear();
            }
        });
    }
});
```

---

## Files to modify

| File | Change |
|------|--------|
| `ez-gui/src/scanner.rs` | Add `apply_command()`, fix preset handler, add calibration enum/state/methods/UI |

---

## Verification

1. `cargo build` -- confirms `apply_command` compiles, no type mismatches
2. `cargo test` -- existing unit tests in `scanner.rs` should still pass
3. Manual:
   - Click preset while scanning -> range changes, sweep stays in-bounds
   - AI `configure_scanner` tool call -> settings applied, sweep starts/stops correctly
   - Calibration section -> 3-step flow advances with Log Measurement, summary shows deltas
