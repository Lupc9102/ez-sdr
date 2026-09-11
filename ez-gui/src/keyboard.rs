use crate::app::SharedState;
use crate::status_bar::StatusBar;

/// Outcomes from keyboard shortcut processing that require CentralApp fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardOutcome {
    /// User requested toggle of keyboard help overlay (`?`).
    pub toggle_help: bool,
    /// User requested start/stop recording (`Ctrl+R`).
    pub toggle_recording: bool,
    /// User requested toggle of frequency scanner (`S`).
    pub toggle_scanner: bool,
    /// Frequency was changed, caller should update `last_manual_tune_time`.
    pub freq_changed: bool,
}

/// Keyboard event handler extracted from CentralApp.
#[derive(Debug, Default)]
pub struct KeyboardHandler;

impl KeyboardHandler {
    pub fn new() -> Self {
        Self
    }

    /// Process keyboard inputs and apply relevant state changes.
    pub fn handle_input(
        &mut self,
        ctx: &egui::Context,
        state: &mut SharedState,
        status_bar: &mut StatusBar,
        scanner_enabled: bool,
        freq_history_idx: &mut Option<usize>,
        last_history_freq: &mut u64,
    ) -> KeyboardOutcome {
        let mut outcome = KeyboardOutcome::default();

        ctx.input(|i| {
            // ? : toggle keyboard help
            if i.key_pressed(egui::Key::Questionmark) {
                outcome.toggle_help = true;
            }

            // Space: toggle start/stop
            if i.key_pressed(egui::Key::Space) {
                if state.source.status == crate::source_manager::SourceStatus::Running {
                    state.source.stop();
                } else {
                    state.source.start();
                }
            }

            // Arrow keys: tune up/down/left/right (coarse = up/down, fine = left/right)
            // Shift modifier doubles the step
            let fine = state.tune_step_fine_hz * if i.modifiers.shift { 10 } else { 1 };
            let coarse = state.tune_step_coarse_hz * if i.modifiers.shift { 10 } else { 1 };
            if i.key_pressed(egui::Key::ArrowUp) && !i.modifiers.alt {
                state.source.frequency_hz = (state.source.frequency_hz + coarse).min(1_770_000_000);
                outcome.freq_changed = true;
            }
            if i.key_pressed(egui::Key::ArrowDown) && !i.modifiers.alt {
                state.source.frequency_hz = state
                    .source
                    .frequency_hz
                    .saturating_sub(coarse)
                    .max(500_000);
                outcome.freq_changed = true;
            }
            if i.key_pressed(egui::Key::ArrowRight) && !i.modifiers.alt {
                state.source.frequency_hz = (state.source.frequency_hz + fine).min(1_770_000_000);
                outcome.freq_changed = true;
            }
            if i.key_pressed(egui::Key::ArrowLeft) && !i.modifiers.alt {
                state.source.frequency_hz =
                    state.source.frequency_hz.saturating_sub(fine).max(500_000);
                outcome.freq_changed = true;
            }

            // Alt+Left/Right: frequency history back/forward
            if i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft) {
                let hist: Vec<u64> = state.freq_history.iter().copied().collect();
                if !hist.is_empty() {
                    let cur_idx = freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                    if cur_idx > 0 {
                        let new_idx = cur_idx - 1;
                        *freq_history_idx = Some(new_idx);
                        state.source.frequency_hz = hist[new_idx];
                        *last_history_freq = hist[new_idx];
                        outcome.freq_changed = true;
                    }
                }
            }
            if i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight) {
                let hist: Vec<u64> = state.freq_history.iter().copied().collect();
                if !hist.is_empty() {
                    let cur_idx = freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                    if cur_idx + 1 < hist.len() {
                        let new_idx = cur_idx + 1;
                        *freq_history_idx = Some(new_idx);
                        state.source.frequency_hz = hist[new_idx];
                        *last_history_freq = hist[new_idx];
                        outcome.freq_changed = true;
                    }
                }
            }

            // F1-F6: select demod mode
            if i.key_pressed(egui::Key::F1) {
                state.demod_mode = crate::sdr_panel::DemodMode::Raw;
            }
            if i.key_pressed(egui::Key::F2) {
                state.demod_mode = crate::sdr_panel::DemodMode::Am;
            }
            if i.key_pressed(egui::Key::F3) {
                state.demod_mode = crate::sdr_panel::DemodMode::Fm;
            }
            if i.key_pressed(egui::Key::F4) {
                state.demod_mode = crate::sdr_panel::DemodMode::Wfm;
            }
            if i.key_pressed(egui::Key::F5) {
                state.demod_mode = crate::sdr_panel::DemodMode::Lsb;
            }
            if i.key_pressed(egui::Key::F6) {
                state.demod_mode = crate::sdr_panel::DemodMode::Usb;
            }

            // Alt+letter: quick demod mode shortcuts
            if i.modifiers.alt && i.key_pressed(egui::Key::F) {
                state.demod_mode = crate::sdr_panel::DemodMode::Fm;
                status_bar.info("NFM".to_string());
            }
            if i.modifiers.alt && i.key_pressed(egui::Key::W) {
                state.demod_mode = crate::sdr_panel::DemodMode::Wfm;
                status_bar.info("WFM".to_string());
            }
            if i.modifiers.alt && i.key_pressed(egui::Key::A) {
                state.demod_mode = crate::sdr_panel::DemodMode::Am;
                status_bar.info("AM".to_string());
            }
            if i.modifiers.alt && i.key_pressed(egui::Key::U) {
                state.demod_mode = crate::sdr_panel::DemodMode::Usb;
                status_bar.info("USB".to_string());
            }
            if i.modifiers.alt && i.key_pressed(egui::Key::L) {
                state.demod_mode = crate::sdr_panel::DemodMode::Lsb;
                status_bar.info("LSB".to_string());
            }
            if i.modifiers.alt && i.key_pressed(egui::Key::R) {
                state.demod_mode = crate::sdr_panel::DemodMode::Raw;
                status_bar.info("RAW".to_string());
            }

            // Ctrl+R: toggle recording
            if i.modifiers.ctrl && i.key_pressed(egui::Key::R) {
                outcome.toggle_recording = true;
            }

            // M: toggle audio mute
            if i.key_pressed(egui::Key::M) {
                state.audio_running = !state.audio_running;
            }

            // Ctrl+S: save config
            if i.modifiers.ctrl && i.key_pressed(egui::Key::S) {
                let recent: Vec<u64> = state.freq_history.iter().copied().collect();
                state.config.recent_frequencies = recent;
                let (min_db, max_db) = state.spectrum.display_range();
                state.config.spectrum_min_db = min_db;
                state.config.spectrum_max_db = max_db;
                state.config.ppm_correction = state.source.ppm_correction;
                state.config.vfo_b_hz = state.vfo_b;
                state.config.wf_min_db = state.spectrum.wf_min_db;
                state.config.wf_max_db = state.spectrum.wf_max_db;
                state.config.lo_offset_hz = state.lo_offset_hz;
                state.config.color_map = state.spectrum.color_map.name().to_string();
                state.config.freq_memory_hz = state.freq_memory.iter().map(|m| m.freq_hz).collect();
                state.config.freq_memory_labels =
                    state.freq_memory.iter().map(|m| m.label.clone()).collect();
                state.config.last_session_freq_hz = state.source.frequency_hz;
                state.config.last_session_gain_db = state.source.gain_db;
                state.config.last_session_demod = state.demod_mode.label().to_string();
                state.config.save();
                status_bar
                    .success("💾 Config saved (freq, gain, demod, spectrum range)".to_string());
            }

            // F: freeze/unfreeze spectrum
            if i.key_pressed(egui::Key::F) && !i.modifiers.ctrl && !i.modifiers.alt {
                state.spectrum.frozen = !state.spectrum.frozen;
            }

            // C: cycle waterfall colormap
            if i.key_pressed(egui::Key::C) && !i.modifiers.ctrl && !i.modifiers.alt {
                state.spectrum.cycle_colormap();
            }

            // Ctrl++ / Ctrl+- / Ctrl+0: zoom in/out/reset
            if (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals))
                && i.modifiers.ctrl
            {
                state.spectrum.zoom_in();
            }
            if i.key_pressed(egui::Key::Minus) && i.modifiers.ctrl && !i.modifiers.alt {
                state.spectrum.zoom_out();
            }
            if i.key_pressed(egui::Key::Num0) && i.modifiers.ctrl {
                state.spectrum.zoom_reset();
            }

            // P: toggle peak hold
            if i.key_pressed(egui::Key::P) && !i.modifiers.ctrl && !i.modifiers.alt {
                let on = state.spectrum.toggle_peak_hold();
                status_bar.info(if on {
                    "Peak Hold ON".to_string()
                } else {
                    "Peak Hold OFF".to_string()
                });
            }

            // V: swap VFO A/B
            if i.key_pressed(egui::Key::V) && !i.modifiers.ctrl && !i.modifiers.alt {
                std::mem::swap(&mut state.source.frequency_hz, &mut state.vfo_b);
                outcome.freq_changed = true;
            }

            // 1-9: bookmark / memory recall / memory save
            {
                let mem_keys = [
                    (egui::Key::Num1, 0usize),
                    (egui::Key::Num2, 1),
                    (egui::Key::Num3, 2),
                    (egui::Key::Num4, 3),
                    (egui::Key::Num5, 4),
                    (egui::Key::Num6, 5),
                    (egui::Key::Num7, 6),
                    (egui::Key::Num8, 7),
                    (egui::Key::Num9, 8),
                ];
                for (key, idx) in mem_keys {
                    if i.key_pressed(key) && !i.modifiers.ctrl {
                        if i.modifiers.alt && i.modifiers.shift {
                            state.freq_memory[idx].freq_hz = state.source.frequency_hz;
                            if state.freq_memory[idx].label.is_empty() {
                                state.freq_memory[idx].label =
                                    format!("{:.4} MHz", state.source.frequency_hz as f64 / 1e6);
                            }
                            status_bar.success(format!(
                                "💾 M{} saved: {:.4} MHz",
                                idx + 1,
                                state.source.frequency_hz as f64 / 1e6
                            ));
                            break;
                        } else if i.modifiers.alt {
                            if state.freq_memory[idx].freq_hz > 0 {
                                state.source.frequency_hz = state.freq_memory[idx].freq_hz;
                                outcome.freq_changed = true;
                                status_bar.success(format!(
                                    "🔁 M{} recalled: {:.4} MHz",
                                    idx + 1,
                                    state.freq_memory[idx].freq_hz as f64 / 1e6
                                ));
                            }
                            break;
                        } else if !i.modifiers.shift {
                            if let Some(bm) = state.bookmarks.bookmarks.get(idx) {
                                state.source.frequency_hz = bm.frequency_hz;
                                outcome.freq_changed = true;
                            }
                            break;
                        }
                    }
                }
            }

            // B: tune to nearest bookmark
            if i.key_pressed(egui::Key::B) && !i.modifiers.ctrl && !i.modifiers.alt {
                let cur = state.source.frequency_hz;
                let nearest = state
                    .bookmarks
                    .bookmarks
                    .iter()
                    .min_by_key(|b| (b.frequency_hz as i64 - cur as i64).unsigned_abs())
                    .map(|bm| (bm.frequency_hz, bm.name.clone()));
                if let Some((freq, name)) = nearest {
                    state.source.frequency_hz = freq;
                    outcome.freq_changed = true;
                    status_bar.success(format!("⭐ {name}"));
                }
            }

            // [ / ] : frequency history back/forward
            if i.key_pressed(egui::Key::OpenBracket) && !i.modifiers.ctrl && !i.modifiers.alt {
                let hist: Vec<u64> = state.freq_history.iter().copied().collect();
                if !hist.is_empty() {
                    let cur_idx = freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                    if cur_idx > 0 {
                        let new_idx = cur_idx - 1;
                        *freq_history_idx = Some(new_idx);
                        state.source.frequency_hz = hist[new_idx];
                        *last_history_freq = hist[new_idx];
                        outcome.freq_changed = true;
                    }
                }
            }
            if i.key_pressed(egui::Key::CloseBracket) && !i.modifiers.ctrl && !i.modifiers.alt {
                let hist: Vec<u64> = state.freq_history.iter().copied().collect();
                if !hist.is_empty() {
                    let cur_idx = freq_history_idx.unwrap_or(hist.len().saturating_sub(1));
                    if cur_idx + 1 < hist.len() {
                        let new_idx = cur_idx + 1;
                        *freq_history_idx = Some(new_idx);
                        state.source.frequency_hz = hist[new_idx];
                        *last_history_freq = hist[new_idx];
                        outcome.freq_changed = true;
                    }
                }
            }

            // G / Shift+G: gain step up / down by 5 dB
            if i.key_pressed(egui::Key::G) && !i.modifiers.ctrl && !i.modifiers.alt {
                if i.modifiers.shift {
                    state.source.gain_db = (state.source.gain_db - 5.0).max(0.0);
                    status_bar.info(format!("Gain: {:.0} dB", state.source.gain_db));
                } else {
                    state.source.gain_db = (state.source.gain_db + 5.0).min(49.0);
                    status_bar.info(format!("Gain: {:.0} dB", state.source.gain_db));
                }
            }

            // Ctrl+Up/Down: volume up / down by 10%
            if i.modifiers.ctrl && i.key_pressed(egui::Key::ArrowUp) {
                state.volume = (state.volume + 0.1).min(1.0);
                status_bar.info(format!("Volume: {:.0}%", state.volume * 100.0));
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::ArrowDown) {
                state.volume = (state.volume - 0.1).max(0.0);
                status_bar.info(format!("Volume: {:.0}%", state.volume * 100.0));
            }

            // Ctrl+B: quick bookmark current frequency
            if i.modifiers.ctrl && i.key_pressed(egui::Key::B) {
                let freq = state.source.frequency_hz;
                let mode = state.demod_mode.label().to_string();
                let name = format!("{:.3} MHz", freq as f64 / 1e6);
                let already = state
                    .bookmarks
                    .bookmarks
                    .iter()
                    .any(|b| b.frequency_hz == freq);
                if already {
                    status_bar.info("⭐ Already bookmarked".to_string());
                } else {
                    state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                        name: name.clone(),
                        frequency_hz: freq,
                        mode,
                        bandwidth_hz: 12_500,
                        category: "Quick".to_string(),
                        notes: String::new(),
                        starred: false,
                    });
                    state.bookmarks_modified = true;
                    state.spectrum.bookmark_freqs_dirty = true;
                    status_bar.success(format!("🔖 Bookmarked {name}"));
                }
            }

            // T: tune to spectrum peak frequency
            if i.key_pressed(egui::Key::T) && !i.modifiers.ctrl && !i.modifiers.alt {
                let peak_hz = state.spectrum.peak_freq_hz();
                if peak_hz > 0 {
                    state.source.frequency_hz = peak_hz;
                    outcome.freq_changed = true;
                    status_bar.info(format!("📡 Peak: {:.3} MHz", peak_hz as f64 / 1e6));
                }
            }

            // S key: toggle scanner on/off
            if i.key_pressed(egui::Key::S) && !i.modifiers.ctrl && !i.modifiers.alt {
                outcome.toggle_scanner = true;
                if scanner_enabled {
                    status_bar.info("🔍 Scanner: OFF".to_string());
                } else {
                    status_bar.info("🔍 Scanner: ON".to_string());
                }
            }

            // R key: reset spectrum dB range to default (-120 to 0)
            if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl && !i.modifiers.alt {
                state.spectrum.set_display_range(-120.0, 0.0);
                status_bar.info("📊 dB range reset to -120…0".to_string());
            }
        });

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_handler_creation() {
        let _handler = KeyboardHandler::new();
        let outcome = KeyboardOutcome::default();
        assert!(!outcome.toggle_help);
        assert!(!outcome.toggle_recording);
        assert!(!outcome.toggle_scanner);
        assert!(!outcome.freq_changed);
    }
}
