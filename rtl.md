# RTL-SDR Blog V4 Fix: Agent Handoff Document

This document provides a complete, self-contained briefing for the next agent to pick up and complete the fix for RTL-SDR Blog V4 detection, waterfall display, and audio output in `ez-sdr`.

---

## 1. Task Summary & Objectives

### Problem Statement
The user reported:
> *"my rtl sdr v4 is not detected and when it was being detected pressing play did nothing, no waterfall no audio nothing, app working not frozen.. simple nothing. but now its two errors on the side with no rtl sdr detection so fix it"*

### Objectives
1. **RTL-SDR Blog V4 reliably detected** on app startup without error banners on the side panel.
2. **Pressing Play immediately streams IQ data** from the dongle.
3. **Spectrum analyzer and waterfall display real-time RF activity**.
4. **Audio output works cleanly** with correct channel mapping and no deadlocks.
5. All workspace tests pass and the completed fix is committed per repo guidelines.

---

## 2. Hardware & Environment Status

The hardware and Linux driver setup have been thoroughly probed and verified:
- **Device**: RTL-SDR Blog V4 is connected on `Bus 001 Device 002: ID 0bda:2838 Realtek Semiconductor Corp. RTL2838 DVB-T`.
- **Permissions**: User `lupc` has read/write permissions via `plugdev` group and ACL on `/dev/bus/usb/001/002`.
- **Drivers**:
  - Distro `/lib/x86_64-linux-gnu/librtlsdr.so.2.0.1` is the old Osmocom library (does not support R828D tuner in V4).
  - The official RTL-SDR Blog V4 driver is properly installed at `/usr/local/lib/librtlsdr.so.2.0.2` and registered in `ldconfig`.
  - `pkg-config --libs librtlsdr` returns `-L/usr/local/lib -lrtlsdr`.
- **Hardware verification test**:
  - `/usr/local/bin/rtl_test` confirms:
    `RTLSDRBlog, Blog V4, SN: 00000001`, `Found Rafael Micro R828D tuner`, `RTL-SDR Blog V4 Detected`.
  - A standalone test directly invoking `rtlsdr_sys` successfully read 1,015,808 samples across 31 chunks in 1 second with 0 dropped frames.

---

## 3. Root Cause Analysis

Three distinct regressions caused the failure modes:

### Issue 1: "Now its two errors on the side with no rtl sdr detection"
- **Root Cause**: In `ez-gui/Cargo.toml`, `rtlsdr` was **not included in `default` features**:
  ```toml
  [features]
  default = ["audio"] # <-- rtlsdr is missing!
  ```
  When the app is launched via `cargo run -p ez-gui` (without `--features rtlsdr`), fallback stubs compile:
  1. `enumerate_rtl_devices()` returns `Err("RTL-SDR discovery requires a hardware-enabled build (--features rtlsdr).")`. In `radio_ui.rs`, this populates `rtl_device_refresh_error` and displays **Error 1** under the device dropdown.
  2. When Play is clicked, `start_hardware()` returns `Err("This build does not include RTL-SDR support (--features rtlsdr).")`. This sets `source.status = SourceStatus::Error(...)` and displays **Error 2** in the source panel banner.

### Issue 2: "Pressing play did nothing, no waterfall"
- **Root Cause**: In commit `fe6d68c`, a duplicate `spectrum_worker: SpectrumWorker` was added to `CentralApp` in `ez-gui/src/app.rs`.
- In `CentralApp::update()`, instead of passing IQ samples to `state.spectrum.push_complex_samples(&wideband_iq)`, the code called `self.spectrum_worker.try_send_iq(wideband_iq)`.
- `wideband_iq` has 16,384 samples (from the USB block size), but `self.spectrum_worker` was created with `fft_size = 2048`. In `ez-gui/src/spectrum.rs:84`:
  ```rust
  if iq_window.len() != fft_size {
      continue; // Dropped EVERY single chunk because 16384 != 2048!
  }
  ```
- Furthermore, `SpectrumAnalyzer` already has an internal worker, ring buffer (`iq_ring`), FFT window hopper, and waterfall rendering engine. Bypassing `push_complex_samples` starved `SpectrumAnalyzer` of all samples, resulting in a blank/frozen waterfall.

### Issue 3: "No audio nothing, app working not frozen"
- **Root Cause**: In commit `fe6d68c`, `CentralApp.audio` was changed from `AudioOutput` to `AudioWorker::new_uninitialized()`.
- `AudioWorker::new_uninitialized()` creates dummy channels and **never spawns an audio thread**.
- In `CentralApp::update()`, the audio stream start logic was replaced by a no-op comment: `// AudioWorker starts its thread in new(); just reset demod.`. The CPAL audio stream was never started when Play was pressed!
- In contrast, `AudioOutput` in `ez-gui/src/audio_output.rs` is the proper lifecycle manager that starts the CPAL stream via `start_with_selection_channels(...)`, handles device errors, and manages stops on receiver pause.

---

## 4. Current Repository State

Unstaged changes exist in:
- `ez-gui/src/app.rs`
- `ez-gui/src/source_manager.rs`

Inspect with `git diff` before editing.

---

## 5. Implementation Steps for Next Agent

### Step 1: Update `ez-gui/Cargo.toml`
Enable `rtlsdr` in default features so normal builds (`cargo run -p ez-gui`) include hardware support:
```toml
[features]
default = ["audio", "rtlsdr"]
```

### Step 2: Update `ez-gui/src/source_manager.rs`
1. **Device Identification**:
   Enhance `is_v4_device` to check product/manufacturer strings as well as serial number:
   ```rust
   #[cfg(all(feature = "rtlsdr", not(test)))]
   fn is_v4_device(serial: Option<&str>, product: Option<&str>, manufacturer: Option<&str>) -> bool {
       serial.map(|s| s.starts_with("00000001")).unwrap_or(false)
           || product.map(|p| p.contains("Blog V4") || p.contains("V4")).unwrap_or(false)
           || manufacturer.map(|m| m.contains("RTLSDRBlog")).unwrap_or(false)
   }
   ```
   In `enumerate_rtl_devices()`:
   ```rust
   let name = if (name.contains("Generic") || name.contains("RTL"))
       && is_v4_device(serial.as_deref(), product.as_deref(), manufacturer.as_deref())
   {
       "RTL-SDR Blog V4".to_string()
   } else {
       name
   };
   ```
2. **Auto-select single connected device in `poll_rtl_devices`**:
   When exactly 1 device is discovered, auto-populate `self.rtl_device` so the user doesn't need to manually click the dropdown:
   ```rust
   Ok(devices) => {
       self.rtl_devices = devices;
       if self.rtl_devices.len() == 1 {
           let d = &self.rtl_devices[0];
           self.rtl_device = RtlDeviceSelection {
               index: d.index,
               serial: d.serial.clone(),
           };
           self.rtl_device_refresh_error = None;
       } else {
           match resolve_rtl_device(&self.rtl_device, &self.rtl_devices) {
               Ok(index) => {
                   self.rtl_device.index = index;
                   self.rtl_device_refresh_error = None;
               }
               Err(error) => self.rtl_device_refresh_error = Some(error),
           }
       }
   }
   ```

### Step 3: Update `ez-gui/src/app.rs`
1. **Restore `AudioOutput` in `CentralApp`**:
   - Field `audio`: type `crate::audio_output::AudioOutput`.
   - In `CentralApp::new`:
     `audio: crate::audio_output::AudioOutput::new(),`
   - Remove duplicate `spectrum_worker` field from `CentralApp` (SpectrumAnalyzer handles its own worker).
2. **Restore Spectrum Batching**:
   - In `CentralApp::update()`, restore `let mut spectrum_batch: Vec<Vec<Complex32>> = Vec::new();`.
   - In the sample consumption loop:
     ```rust
     if self.current_tab == AppTab::Listen {
         spectrum_batch.push(wideband_iq);
     }
     ```
   - In the `state.spectrum` update block:
     ```rust
     state.spectrum.update_params_exact(center, radio_rate);
     state.spectrum.vfo_freq_hz = Some(freq);
     for samples in &spectrum_batch {
         state.spectrum.push_complex_samples(samples);
     }
     state.spectrum.try_recv_spectrum();
     ```
3. **Restore Audio Lifecycle & Channel Duplication**:
   - In `audio_action` handling:
     ```rust
     if let Some((audio_running, selection, input_channels)) = audio_action {
         if audio_running && !self.audio.is_running() {
             if !self.audio.has_failed() {
                 match self.audio.start_with_selection_channels(
                     crossbeam_channel::never(),
                     &selection,
                     input_channels,
                 ) {
                     Ok(()) => {
                         self.recorder_panel.set_audio_sample_rate(self.audio.sample_rate());
                         self.demod_worker.request_reset();
                     }
                     Err(error) => {
                         self.audio.mark_failed();
                         let _ = self.audio.take_error();
                         self.status_bar.warning(format!(
                             "Audio: {error}. Stop and restart the receiver to retry."
                         ));
                     }
                 }
             }
         } else if !audio_running && (self.audio.is_running() || self.audio.has_failed()) {
             self.audio.stop();
             self.daemon_audio.reset();
         }
     }
     ```
   - In audio sample feeding: push interleaved stereo or mono duplicated into stereo:
     ```rust
     if audio_running {
         let mut stereo = Vec::with_capacity(audio.len() * 2);
         for sample in audio {
             stereo.push(sample);
             stereo.push(sample);
         }
         let _ = self.audio.push_audio(stereo);
     }
     ```
4. **Trigger initial RTL scan if in Hardware mode**:
   In `CentralApp::new`:
   ```rust
   {
       let mut state = shared.lock().expect("shared state mutex poisoned");
       if state.source.source_mode == crate::source_manager::SourceMode::Hardware {
           state.source.refresh_rtl_devices();
       }
   }
   ```

---

## 6. Verification Plan

1. **Workspace Compilation**:
   ```bash
   cargo check --workspace
   ```
2. **Run Tests**:
   ```bash
   cargo test -p ez-gui
   ```
   Verify all 688 unit tests pass without regressions.
3. **Manual App Verification**:
   ```bash
   cargo run -p ez-gui
   ```
   - Check that RTL-SDR Blog V4 is detected in the Source panel under "Hardware".
   - Press Play.
   - Verify spectrum graph and waterfall scroll with real live RF noise/signals.
   - Tune to a local FM station (e.g. 88-108 MHz) or AM/NFM signal and verify audio output plays.
4. **Commit**:
   Per `CLAUDE.md`:
   > *"Always commit changes once a task is complete and verified (build/tests pass) — don't leave finished work uncommitted."*
   ```bash
   git add ez-gui/Cargo.toml ez-gui/src/app.rs ez-gui/src/source_manager.rs
   git commit -m "fix(rtlsdr): enable rtlsdr feature by default, fix Blog V4 detection, waterfall, and audio"
   ```
