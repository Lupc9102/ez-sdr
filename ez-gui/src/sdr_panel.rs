use std::sync::{Arc, Mutex};

use crate::app::SharedState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodMode {
    Raw,
    Am,
    Fm,
    Wfm,
    Lsb,
    Usb,
}

impl DemodMode {
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "RAW" => Some(Self::Raw),
            "AM" => Some(Self::Am),
            "FM" | "NFM" => Some(Self::Fm),
            "WFM" => Some(Self::Wfm),
            "LSB" => Some(Self::Lsb),
            "USB" => Some(Self::Usb),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DemodMode::Raw => "RAW",
            DemodMode::Am => "AM",
            DemodMode::Fm => "FM",
            DemodMode::Wfm => "WFM",
            DemodMode::Lsb => "LSB",
            DemodMode::Usb => "USB",
        }
    }
}

pub struct SdrPanel {
    shared: Arc<Mutex<SharedState>>,
    pub squelch: f32,
    pub filter_bw: u32,
}

impl SdrPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            squelch: -50.0,
            filter_bw: 12_000,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("SDR Receiver");

        // Big frequency display with fine/coarse tuning
        if let Ok(mut state) = self.shared.try_lock() {
            ui.horizontal(|ui| {
                let mut freq_mhz = state.source.frequency_hz as f64 / 1e6;
                ui.monospace(egui::RichText::new(format!("{:.6}", freq_mhz)).size(24.0).color(egui::Color32::from_rgb(52, 152, 219)))
                    .on_hover_text("Current tuned center frequency. The SDR receives a band centered here. RTL-SDR range: 24 MHz – 1766 MHz.");
                ui.label(egui::RichText::new("MHz").size(14.0).color(egui::Color32::GRAY));
                let is_dragging = ui.add(egui::DragValue::new(&mut freq_mhz).speed(0.0001).range(0.5..=1770.0).suffix(" MHz"))
                    .on_hover_text("Drag left/right or type to tune. You can also click on the spectrum display to tune there.");
                if is_dragging.changed() || is_dragging.dragged() {
                    state.source.frequency_hz = (freq_mhz * 1e6) as u64;
                }
                if ui.small_button("-1M").on_hover_text("Tune down 1 MHz (keyboard: ↓)").clicked() {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(1_000_000).max(500_000);
                }
                if ui.small_button("+1M").on_hover_text("Tune up 1 MHz (keyboard: ↑)").clicked() {
                    state.source.frequency_hz = (state.source.frequency_hz + 1_000_000).min(1_770_000_000);
                }
                if ui.small_button("-100k").on_hover_text("Tune down 100 kHz (keyboard: ←)").clicked() {
                    state.source.frequency_hz = state.source.frequency_hz.saturating_sub(100_000).max(500_000);
                }
                if ui.small_button("+100k").on_hover_text("Tune up 100 kHz (keyboard: →)").clicked() {
                    state.source.frequency_hz = (state.source.frequency_hz + 100_000).min(1_770_000_000);
                }
            });
        }
        ui.separator();

        // Demod mode selector
        ui.horizontal(|ui| {
            if let Ok(mut state) = self.shared.try_lock() {
                for (mode, tip) in [
                    (DemodMode::Raw, "RAW I/Q — pass raw complex samples to external decoders"),
                    (DemodMode::Am,  "AM — Amplitude Modulation. Use for aviation (118–137 MHz), AM broadcast, shortwave"),
                    (DemodMode::Fm,  "NFM — Narrowband FM. Use for land mobile radio: police, fire, repeaters, NOAA weather"),
                    (DemodMode::Wfm, "WFM — Wideband FM. Use for commercial FM broadcast (88–108 MHz). Supports stereo + RDS"),
                    (DemodMode::Lsb, "LSB — Lower Sideband. Use for amateur HF voice below 10 MHz"),
                    (DemodMode::Usb, "USB — Upper Sideband. Use for amateur HF voice above 10 MHz, some utility/military"),
                ] {
                    if ui.selectable_label(state.demod_mode == mode, mode.label())
                        .on_hover_text(tip)
                        .clicked()
                    {
                        state.demod_mode = mode;
                    }
                }
            }
        });

        // Signal meter
        ui.separator();
        if let Ok(state) = self.shared.try_lock() {
            let signal = state.spectrum.signal_level();
            let norm = ((signal + 120.0) / 120.0).clamp(0.0, 1.0);
            let color = if norm > 0.6 { egui::Color32::GREEN }
                else if norm > 0.3 { egui::Color32::YELLOW }
                else { egui::Color32::RED };
            ui.horizontal(|ui| {
                ui.label("Signal:")
                    .on_hover_text("Estimated signal level in dBFS. Green = strong, yellow = moderate, red = weak or no signal. -60 dBFS or above is usually demodulable.");
                ui.add(egui::ProgressBar::new(norm).fill(color).text(format!("{:.1} dB", signal)));
            });
        }

        // Audio controls
        ui.separator();
        ui.horizontal(|ui| {
            let mut start_audio = false;
            let mut stop_audio = false;
            let mut volume = 0.5f32;
            let mut audio_running = false;
            if let Ok(state) = self.shared.try_lock() {
                volume = state.volume;
                audio_running = state.audio_running;
            }
            if audio_running {
                if ui.button("🔇 Stop Audio").on_hover_text("Stop audio playback through your speakers/headphones.").clicked() {
                    stop_audio = true;
                }
            } else {
                if ui.button("🔊 Start Audio").on_hover_text("Start playing the demodulated signal through your speakers/headphones.").clicked() {
                    start_audio = true;
                }
            }
            let mut vol = volume;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Vol"))
                .on_hover_text("Audio output volume. Does not affect the RF signal — only the speaker output level.")
                .changed()
            {
                volume = vol;
            }
            if start_audio || stop_audio || vol != volume {
                if let Ok(mut state) = self.shared.try_lock() {
                    state.volume = volume;
                    if start_audio { state.audio_running = true; }
                    else if stop_audio { state.audio_running = false; }
                }
            }
        });

        ui.separator();

        // Frequency presets (band quick-tune)
        ui.horizontal(|ui| {
            ui.label("Bands:").on_hover_text("Quick-tune presets. Click to jump immediately to that frequency.");
            const BANDS: &[(&str, u64, &str)] = &[
                ("BC FM",  100_000_000, "FM Broadcast band center (88–108 MHz). Mode: WFM"),
                ("Air",    118_000_000, "Aviation VHF voice band start (118–137 MHz). Mode: AM — not FM!"),
                ("2m",     144_000_000, "Amateur 2-meter band. FM repeaters, APRS at 144.390 MHz. Mode: NFM"),
                ("70cm",   430_000_000, "Amateur 70cm band. FM repeaters, digital modes. Mode: NFM"),
                ("NOAA 15",137_620_000, "NOAA 15 weather satellite (137.620 MHz). Mode: WFM 34 kHz"),
                ("NOAA 18",137_912_500, "NOAA 18 weather satellite (137.9125 MHz). Mode: WFM 34 kHz"),
                ("NOAA 19",137_100_000, "NOAA 19 weather satellite (137.100 MHz). Mode: WFM 34 kHz"),
                ("ISS",    145_800_000, "International Space Station (145.800 MHz). Mode: NFM"),
            ];
            if let Ok(mut state) = self.shared.try_lock() {
                for (name, freq_hz, tip) in BANDS {
                    if ui.small_button(*name).on_hover_text(*tip).clicked() {
                        state.source.frequency_hz = *freq_hz;
                    }
                }
            }
        });

        // Filter bandwidth and squelch
        ui.add(egui::Slider::new(&mut self.filter_bw, 100..=250_000).text("Filter BW (Hz)").logarithmic(true))
            .on_hover_text("Receiver filter bandwidth. Set just wider than the signal. WFM: 200 kHz, NFM voice: 12–16 kHz, AM voice: 8 kHz, SSB: 2.4 kHz. Too wide = more noise.");
        if let Ok(mut state) = self.shared.try_lock() {
            ui.add(egui::Slider::new(&mut state.lpf_cutoff, 100.0..=20000.0).text("Audio LPF (Hz)").logarithmic(true))
                .on_hover_text("Low-pass filter on audio output. Cuts high-frequency hiss above this frequency. Default 15 kHz is fine for voice. Lower for CW/Morse (~800 Hz).");
        }
        if ui.add(egui::Slider::new(&mut self.squelch, -120.0..=0.0).text("Squelch (dB)"))
            .on_hover_text("Signal level threshold. Audio is muted when signal drops below this value, silencing static between transmissions. Set ~5 dB above your noise floor.")
            .changed()
            || ui.input(|i| i.pointer.any_down())
        {
            if let Ok(mut state) = self.shared.try_lock() {
                state.squelch = self.squelch;
            }
        }

        ui.separator();

        // Source controls
        if let Ok(mut state) = self.shared.try_lock() {
            state.source.ui(ui);
        }
    }
}
