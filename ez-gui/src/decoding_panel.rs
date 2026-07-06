use crossbeam_channel::Receiver;
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecodePreset {
    MeteorM2_2,
    MeteorM2_3,
    MeteorM2_4,
    #[default]
    Custom,
}

impl DecodePreset {
    pub fn sample_rate(&self) -> u32 {
        2_048_000
    }
    pub fn symbol_rate(&self) -> u32 {
        72_000
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::MeteorM2_2 => "Meteor-M2-2 (137.1 MHz, LRPT)",
            Self::MeteorM2_3 => "Meteor-M2-3 (137.9 MHz, LRPT)",
            Self::MeteorM2_4 => "Meteor-M2-4 (137.1 MHz, LRPT)",
            Self::Custom => "Custom",
        }
    }
    pub fn all() -> &'static [DecodePreset] {
        &[
            Self::MeteorM2_2,
            Self::MeteorM2_3,
            Self::MeteorM2_4,
            Self::Custom,
        ]
    }
}

/// Maps a tracked satellite's display name to its decode preset. Returns `None`
/// for satellites with no decoder (NOAA/APT, ISS/Voice-APRS) — LRPT decoding
/// only exists for Meteor-M2.
pub fn satellite_to_preset(name: &str) -> Option<DecodePreset> {
    let n = name.to_ascii_lowercase();
    if n.contains("meteor-m2-2") {
        Some(DecodePreset::MeteorM2_2)
    } else if n.contains("meteor-m2-3") {
        Some(DecodePreset::MeteorM2_3)
    } else if n.contains("meteor-m2-4") {
        Some(DecodePreset::MeteorM2_4)
    } else {
        None
    }
}

pub struct DecodeRequest {
    pub file_path: String,
    pub preset: Option<DecodePreset>,
    pub sample_rate: u32,
    pub satellite_name: Option<String>,
}

#[derive(Default)]
pub struct DecodingPanel {
    pub file_path: String,
    pub preset: DecodePreset,
    pub sample_rate: u32,
    pub satellite_name: Option<String>,
    pub running: bool,
    pub progress: Option<lrpt_decode::DecodeProgress>,
    pub result: Option<lrpt_decode::DecodeResult>,
    pub error: Option<String>,
    pub no_decoder_reason: Option<String>,
    pub pending_status: Option<String>,
    progress_rx: Option<Receiver<lrpt_decode::DecodeProgress>>,
    done_rx: Option<Receiver<Result<lrpt_decode::DecodeResult, lrpt_decode::LrptError>>>,
    handle: Option<JoinHandle<()>>,
    preview_textures: Vec<(u16, egui::TextureHandle)>,
}

impl DecodingPanel {
    /// Called from the app's per-frame pending-request poll. Populates
    /// state from a completed satellite recording and auto-starts the
    /// decode when a preset is known; otherwise surfaces a "no decoder"
    /// state.
    pub fn request_from_satellite(&mut self, req: DecodeRequest) {
        self.file_path = req.file_path;
        self.satellite_name = req.satellite_name;
        self.sample_rate = req.sample_rate;
        self.result = None;
        self.error = None;
        self.no_decoder_reason = None;
        match req.preset {
            Some(preset) => {
                self.preset = preset;
                self.start_decode();
            }
            None => {
                self.no_decoder_reason = Some(format!(
                    "No decoder available for {} — LRPT decoding only supports Meteor-M2 satellites. Recording saved to: {}",
                    self.satellite_name.as_deref().unwrap_or("this satellite"),
                    self.file_path
                ));
            }
        }
    }

    pub fn start_decode(&mut self) {
        if self.file_path.is_empty() || self.running {
            return;
        }
        let (progress_tx, progress_rx) = crossbeam_channel::unbounded();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let path = self.file_path.clone();
        let sample_rate = self.sample_rate;
        let symbol_rate = self.preset.symbol_rate();
        self.handle = Some(std::thread::spawn(move || {
            let result = std::fs::read(&path)
                .map_err(lrpt_decode::LrptError::Io)
                .and_then(|bytes| {
                    let mut decoder =
                        lrpt_decode::LrptDecoder::new(sample_rate, symbol_rate, progress_tx);
                    decoder.push_samples_cf32(&bytes);
                    decoder.finish().ok_or(lrpt_decode::LrptError::NoSync)
                });
            let _ = done_tx.send(result);
        }));
        self.progress_rx = Some(progress_rx);
        self.done_rx = Some(done_rx);
        self.running = true;
        self.error = None;
        self.result = None;
    }

    /// Poll progress/completion channels. MUST be called every frame from
    /// `CentralApp::logic()` unconditionally (not only while the Decoding
    /// tab is visible) — otherwise a decode running in the background while
    /// the user is on another tab will never be observed as complete.
    pub fn tick_decode(&mut self) {
        if let Some(rx) = &self.progress_rx {
            while let Ok(p) = rx.try_recv() {
                self.progress = Some(p);
            }
        }
        if let Some(rx) = &self.done_rx {
            if let Ok(result) = rx.try_recv() {
                if let Some(h) = self.handle.take() {
                    let _ = h.join();
                }
                match result {
                    Ok(decoded) => {
                        self.pending_status = Some(format!(
                            "✅ Decode complete: {} image(s), {} RS-OK / {} RS-failed",
                            decoded.images.len(),
                            decoded.rs_ok,
                            decoded.rs_failed
                        ));
                        self.result = Some(decoded);
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        self.pending_status = Some(format!("❌ Decode failed: {e}"));
                    }
                }
                self.running = false;
                self.progress_rx = None;
                self.done_rx = None;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(reason) = &self.no_decoder_reason {
            ui.colored_label(egui::Color32::YELLOW, reason);
            return;
        }
        ui.horizontal(|ui| {
            ui.label("File:");
            ui.text_edit_singleline(&mut self.file_path);
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("cf32 recordings", &["cf32"])
                    .pick_file()
                {
                    self.file_path = path.to_string_lossy().to_string();
                }
            }
        });
        egui::ComboBox::from_label("Preset")
            .selected_text(self.preset.label())
            .show_ui(ui, |ui| {
                for p in DecodePreset::all() {
                    ui.selectable_value(&mut self.preset, *p, p.label());
                }
            });
        if ui
            .add_enabled(
                !self.running && !self.file_path.is_empty(),
                egui::Button::new("Decode"),
            )
            .clicked()
        {
            self.sample_rate = self.preset.sample_rate();
            self.start_decode();
        }
        if self.running {
            ui.spinner();
            if let Some(p) = &self.progress {
                ui.label(format!(
                    "Lines: {}  RS OK: {}  RS Failed: {}  Costas: {}  Frame: {}",
                    p.lines_decoded, p.rs_ok, p.rs_failed, p.costas_locked, p.frame_locked
                ));
            }
        }
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(result) = self.result.clone() {
            ui.label(format!("{} image(s) decoded", result.images.len()));
            if self.preview_textures.len() != result.images.len() {
                self.preview_textures.clear();
                for (apid, img) in &result.images {
                    let size = [img.width() as usize, img.height() as usize];
                    let rgba: Vec<u8> = img
                        .pixels()
                        .flat_map(|p| [p.0[0], p.0[0], p.0[0], 255])
                        .collect();
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    let handle = ui.ctx().load_texture(
                        format!("decode-apid-{apid}"),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.preview_textures.push((*apid, handle));
                }
            }
            for (apid, tex) in &self.preview_textures {
                ui.label(format!("APID {apid}"));
                ui.image((tex.id(), tex.size_vec2()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satellite_to_preset_matches_meteor_variants() {
        assert_eq!(
            satellite_to_preset("METEOR-M2-2"),
            Some(DecodePreset::MeteorM2_2)
        );
        assert_eq!(
            satellite_to_preset("Meteor-M2-3"),
            Some(DecodePreset::MeteorM2_3)
        );
        assert_eq!(
            satellite_to_preset("Meteor-M2-4"),
            Some(DecodePreset::MeteorM2_4)
        );
    }

    #[test]
    fn satellite_to_preset_returns_none_for_noaa() {
        assert_eq!(satellite_to_preset("NOAA 19"), None);
    }

    #[test]
    fn satellite_to_preset_returns_none_for_iss() {
        assert_eq!(satellite_to_preset("ISS (ZARYA)"), None);
    }

    #[test]
    fn satellite_to_preset_is_case_insensitive() {
        assert_eq!(
            satellite_to_preset("meteor-m2-2"),
            Some(DecodePreset::MeteorM2_2)
        );
    }

    #[test]
    fn request_from_satellite_sets_no_decoder_reason_when_preset_none() {
        let mut panel = DecodingPanel::default();
        panel.request_from_satellite(DecodeRequest {
            file_path: "/tmp/rec.cf32".to_string(),
            preset: None,
            sample_rate: 2_048_000,
            satellite_name: Some("NOAA 19".to_string()),
        });
        assert!(panel.no_decoder_reason.is_some());
        assert!(!panel.running);
        assert_eq!(panel.file_path, "/tmp/rec.cf32");
    }
}
