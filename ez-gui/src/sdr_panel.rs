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
    pub bookmark_request: Option<(u64, String)>,
    freq_input: String,
    freq_input_active: bool,
}

impl SdrPanel {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            shared,
            squelch: -50.0,
            filter_bw: 12_000,
            bookmark_request: None,
            freq_input: String::new(),
            freq_input_active: false,
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
                let bm_freq = state.source.frequency_hz;
                let bm_mode = state.demod_mode.label().to_string();
                if ui.small_button("⭐").on_hover_text("Bookmark this frequency — saves it to your bookmarks list with the current mode.").clicked() {
                    self.bookmark_request = Some((bm_freq, bm_mode));
                }
            });
        }
        // Direct frequency entry
        ui.horizontal(|ui| {
            ui.label("Go to:").on_hover_text("Type a frequency and press Enter to jump. Examples: 145.5 (MHz), 145500000 (Hz), 145500k (kHz).");
            let resp = ui.add(egui::TextEdit::singleline(&mut self.freq_input)
                .desired_width(140.0)
                .hint_text("MHz or Hz, Enter to jump"));
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let s = self.freq_input.trim().to_lowercase();
                let parsed_hz: Option<u64> = if let Ok(v) = s.trim_end_matches("mhz").trim().parse::<f64>() {
                    Some((v * 1e6) as u64)
                } else if s.ends_with("khz") || s.ends_with('k') {
                    s.trim_end_matches("khz").trim_end_matches('k').trim().parse::<f64>().ok().map(|v| (v * 1e3) as u64)
                } else if s.ends_with("ghz") || s.ends_with('g') {
                    s.trim_end_matches("ghz").trim_end_matches('g').trim().parse::<f64>().ok().map(|v| (v * 1e9) as u64)
                } else {
                    s.parse::<u64>().ok()
                };
                if let Some(hz) = parsed_hz {
                    let hz = hz.clamp(500_000, 1_770_000_000);
                    if let Ok(mut state) = self.shared.try_lock() {
                        state.source.frequency_hz = hz;
                    }
                    self.freq_input.clear();
                }
            }
        });

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

        // Signal meter + SNR
        ui.separator();
        if let Ok(state) = self.shared.try_lock() {
            let signal = state.spectrum.signal_level();
            let noise_floor = state.spectrum.noise_floor();
            let peak = state.spectrum.peak_level();
            let snr = peak - noise_floor;
            let norm = ((signal + 120.0) / 120.0).clamp(0.0, 1.0);
            let color = if norm > 0.6 { egui::Color32::GREEN }
                else if norm > 0.3 { egui::Color32::YELLOW }
                else { egui::Color32::RED };
            // VU-style signal meter
            ui.horizontal(|ui| {
                ui.label("Signal:").on_hover_text("RF signal level in dBFS. Green zone (>-40 dB) = strong. Yellow (−60–40) = moderate. Red (<-60) = weak/noise.");
                // Draw custom colored bar using painter
                let desired = egui::vec2(180.0, 14.0);
                let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
                let response = response.on_hover_text(format!("Signal: {:.1} dBFS  SNR: {:.1} dB  Noise: {:.1} dB", signal, snr, noise_floor));
                let p = ui.painter();
                p.rect_filled(rect, 2.0, egui::Color32::from_rgb(15, 15, 25));
                // Zones: 0–50% red, 50–75% yellow, 75–100% green
                let zones = [
                    (0.0f32, 0.5f32, egui::Color32::from_rgb(150, 30, 30)),
                    (0.5f32, 0.75f32, egui::Color32::from_rgb(180, 150, 20)),
                    (0.75f32, 1.0f32, egui::Color32::from_rgb(30, 150, 50)),
                ];
                for (lo, hi, c) in &zones {
                    let fill = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + lo * rect.width(), rect.top()),
                        egui::pos2(rect.left() + hi * rect.width(), rect.bottom()),
                    );
                    p.rect_filled(fill, 0.0, egui::Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), 40));
                }
                // Filled bar up to signal level
                let fill_w = norm * rect.width();
                let fill_color = if norm > 0.75 { egui::Color32::from_rgb(50, 200, 80) }
                    else if norm > 0.5 { egui::Color32::from_rgb(220, 180, 30) }
                    else { egui::Color32::from_rgb(200, 50, 50) };
                p.rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height())),
                    2.0, fill_color,
                );
                // Tick marks every 20 dB
                for db in (-120..=0i32).step_by(20) {
                    let t = ((db as f32 - (-120.0)) / 120.0).clamp(0.0, 1.0);
                    let x = rect.left() + t * rect.width();
                    p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(0.5, egui::Color32::from_gray(80)));
                }
                // Level text
                p.text(egui::pos2(rect.right() - 2.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{:.0}dB", signal),
                    egui::FontId::monospace(9.0),
                    egui::Color32::WHITE);
                let _ = response;

                ui.separator();
                let snr_color = if snr > 20.0 { egui::Color32::GREEN }
                    else if snr > 10.0 { egui::Color32::YELLOW }
                    else { egui::Color32::GRAY };
                ui.colored_label(snr_color, format!("SNR {:.1} dB", snr))
                    .on_hover_text("Signal-to-Noise Ratio: peak dB minus estimated noise floor. >20 dB = excellent, 10–20 dB = good, <10 dB = marginal. Aim for >15 dB for clean audio.");
            });
        }

        // Demod quality indicators
        if let Ok(state) = self.shared.try_lock() {
            let mode = state.demod_mode;
            if mode == DemodMode::Fm || mode == DemodMode::Wfm {
                let dev_khz = state.fm_deviation_hz / 1000.0;
                let dev_color = if dev_khz > 75.0 { egui::Color32::RED }
                    else if dev_khz > 5.0 { egui::Color32::GREEN }
                    else { egui::Color32::GRAY };
                ui.horizontal(|ui| {
                    ui.colored_label(dev_color, format!("FM dev: {:.1} kHz", dev_khz))
                        .on_hover_text("FM frequency deviation. NFM: 5–12.5 kHz is normal. WFM broadcast: up to 75 kHz. >75 kHz = overmodulated.");
                    let peak_pct = (state.audio_peak * 100.0).min(100.0);
                    let peak_col = if peak_pct > 90.0 { egui::Color32::RED } else if peak_pct > 60.0 { egui::Color32::GREEN } else { egui::Color32::GRAY };
                    ui.colored_label(peak_col, format!("Audio: {:.0}%", peak_pct))
                        .on_hover_text("Normalized audio output level. 0% = silent, 100% = clipping risk. Adjust volume if consistently above 90%.");
                });
            }
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
