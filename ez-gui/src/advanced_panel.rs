//! Helpers and drawer UI for the `⚙ More → Advanced` panel.

use crate::config::AdvancedConfig;
use crate::spectrum::{ColorMap, WindowType};

/// Map a window name to a [`WindowType`].
pub fn window_from_str(s: &str) -> WindowType {
    match s {
        "Hamming" => WindowType::Hamming,
        "Blackman" => WindowType::Blackman,
        "FlatTop" => WindowType::FlatTop,
        _ => WindowType::Hann,
    }
}

/// Map a colour-map name to a [`ColorMap`].
pub fn color_from_str(s: &str) -> ColorMap {
    match s {
        "Viridis" => ColorMap::Viridis,
        "Plasma" => ColorMap::Plasma,
        "Magma" => ColorMap::Magma,
        "Inferno" => ColorMap::Inferno,
        "Turbo" => ColorMap::Turbo,
        "Grayscale" => ColorMap::Grayscale,
        "Hot" => ColorMap::Hot,
        _ => ColorMap::Classic,
    }
}

/// Generic labelled `f32` slider. Borrows the config field by `&mut`, updating
/// it in place and invoking `apply` when the user changes it. Returns `true`
/// when the value changed. `eng` is borrowed only for the `apply` call — never
/// simultaneously with `shared`, avoiding borrow conflicts in the panel.
pub fn slider_f32<E, F>(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
    eng: &mut E,
    apply: F,
) -> bool
where
    F: FnOnce(&mut E, f32),
{
    if ui
        .add(egui::Slider::new(value, range).text(label))
        .changed()
    {
        apply(eng, *value);
        true
    } else {
        false
    }
}

pub fn slider_usize<E, F>(
    ui: &mut egui::Ui,
    value: &mut usize,
    range: std::ops::RangeInclusive<usize>,
    label: &str,
    eng: &mut E,
    apply: F,
) -> bool
where
    F: FnOnce(&mut E, usize),
{
    if ui
        .add(egui::Slider::new(value, range).text(label))
        .changed()
    {
        apply(eng, *value);
        true
    } else {
        false
    }
}

pub fn slider_u32<E, F>(
    ui: &mut egui::Ui,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    label: &str,
    eng: &mut E,
    apply: F,
) -> bool
where
    F: FnOnce(&mut E, u32),
{
    if ui
        .add(egui::Slider::new(value, range).text(label))
        .changed()
    {
        apply(eng, *value);
        true
    } else {
        false
    }
}

/// Push every advanced setting from `shared.config` onto the live engines.
/// Used by the panel's "reset" button so the running app matches the saved
/// (or reset) profile.
pub fn push_advanced(demod: &mut crate::demod::DemodWorker, shared: &mut crate::app::SharedState) {
    let mode = shared.demod_mode.resolve(shared.source.frequency_hz);
    demod.configure_advanced(&shared.config.advanced, mode);
    let a = &shared.config.advanced;
    let s = &mut shared.spectrum;
    if !s.set_fft_size(a.fft_size) {
        s.set_fft_size(2048);
    }
    s.set_fft_rate(a.fft_rate);
    s.set_waterfall_visible(a.waterfall_visible);
    s.full_waterfall_update = a.full_waterfall_update;
    s.set_snr_smoothing(a.snr_smoothing, a.snr_smoothing_secs);
    s.set_window(window_from_str(&a.window));
    s.set_waterfall_history(a.wf_depth);
    s.waterfall_every_n = a.wf_speed.max(1);
    s.set_grid(a.grid);
    s.set_peak_hold_time(a.peak_hold_time);
    s.set_avg_alpha(a.avg_alpha);
    s.set_persistence(a.persistence);
    s.set_gradient_fill(a.gradient_fill);
    s.set_color_map(color_from_str(&shared.config.color_map));
    s.set_display_range(a.db_min, a.db_max);

    shared.source.tuner_agc = a.tuner_agc;
    shared.source.rtl_agc = a.rtl_agc;
    shared.source.direct_sampling = a.direct_sampling;
    shared.source.direct_sampling_branch = a.direct_sampling_branch;
    shared.source.frequency_offset_hz = shared.config.lo_offset_hz;
    shared.source.bias_tee = a.bias_tee;
    shared.source.rtl_device = a.rtl_device.clone();
    shared.source.offset_tuning = a.offset_tuning;
    shared.config.advanced.fft_size = shared.spectrum.fft_size();
    shared.config.advanced.fft_rate = shared.spectrum.fft_rate();
}

/// Render the `⚙ More → Advanced` drawer body.
///
/// Interactive widgets only update `shared.config.advanced` in place; the
/// running [`crate::demod::DemodWorker`] is re-synced from the whole config
/// once at the end of the panel (see [`crate::demod::DemodWorker::configure_advanced`]).
pub fn render_advanced(
    ui: &mut egui::Ui,
    shared: &mut crate::app::SharedState,
    demod: &mut crate::demod::DemodWorker,
) {
    ui.heading("Advanced / Experimental");
    ui.label(
        "Optional, power-user controls. Saved to your config and applied \
         immediately (RF-hardware options apply on the next device start).",
    );
    ui.separator();

    // ---------------- Audio / DSP ----------------
    ui.collapsing("Audio DSP", |ui| {
        let mut cutoff = shared.lpf_cutoff;
        if ui
            .add(egui::Slider::new(&mut cutoff, 100.0..=20_000.0).text("Audio cutoff (Hz)"))
            .changed()
        {
            shared.lpf_cutoff = cutoff;
            shared.config.advanced.audio_cutoff_hz = cutoff;
        }
        let mut agc = shared.config.advanced.agc_enabled;
        if ui.checkbox(&mut agc, "Audio AGC").changed() {
            shared.config.advanced.agc_enabled = agc;
        }
        slider_f32(
            ui,
            &mut shared.config.advanced.agc_target,
            0.01..=1.0,
            "AGC target",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.agc_attack_rate,
            1.0..=200.0,
            "AGC attack",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.agc_decay_rate,
            1.0..=20.0,
            "AGC decay",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.audio_hpf_hz,
            0.0..=20000.0,
            "High-pass (Hz)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.dc_blocker,
            0.0..=1.0,
            "DC blocker",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.deemph_tau_us,
            0.0..=100.0,
            "FM de-emph t (us)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.audio_gain,
            0.0..=10.0,
            "Audio gain",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.bass_db,
            -24.0..=24.0,
            "Bass (dB)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.treble_db,
            -24.0..=24.0,
            "Treble (dB)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.notch_hz,
            0.0..=20000.0,
            "Notch (Hz)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.notch_width_hz,
            10.0..=5000.0,
            "Notch width (Hz)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.noise_blanker,
            0.0..=1.0,
            "Noise blanker",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.pitch_octaves,
            -2.0..=2.0,
            "Pitch (octaves)",
            demod,
            |_, _| {},
        );
    });

    // ---------------- Display / Spectrum ----------------
    ui.collapsing("Display / Spectrum", |ui| {
        let dbmin = shared.config.advanced.db_min;
        let dbmax = shared.config.advanced.db_max;
        ui.horizontal(|ui| {
            ui.label("FFT size");
            egui::ComboBox::from_id_salt("advanced.fft_size")
                .selected_text(shared.config.advanced.fft_size.to_string())
                .show_ui(ui, |ui| {
                    for size in [256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536] {
                        if ui
                            .selectable_value(
                                &mut shared.config.advanced.fft_size,
                                size,
                                size.to_string(),
                            )
                            .changed()
                        {
                            shared.spectrum.set_fft_size(size);
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Window:");
            egui::ComboBox::from_label("")
                .selected_text(shared.config.advanced.window.clone())
                .show_ui(ui, |ui| {
                    for w in ["Hann", "Hamming", "Blackman", "FlatTop"] {
                        if ui
                            .selectable_label(shared.config.advanced.window == w, w)
                            .clicked()
                        {
                            shared.config.advanced.window = w.to_string();
                            shared.spectrum.set_window(window_from_str(w));
                        }
                    }
                });
        });
        slider_u32(
            ui,
            &mut shared.config.advanced.wf_speed,
            1..=16,
            "Waterfall speed",
            &mut shared.spectrum,
            |s, v| s.waterfall_every_n = v.max(1),
        );
        slider_usize(
            ui,
            &mut shared.config.advanced.wf_depth,
            32..=1024,
            "Waterfall depth",
            &mut shared.spectrum,
            |s, v| s.set_waterfall_history(v),
        );
        let mut grid = shared.config.advanced.grid;
        if ui.checkbox(&mut grid, "Grid lines").changed() {
            shared.config.advanced.grid = grid;
            shared.spectrum.set_grid(grid);
        }
        slider_f32(
            ui,
            &mut shared.config.advanced.peak_hold_time,
            0.1..=60.0,
            "Peak-hold time (s)",
            &mut shared.spectrum,
            |s, v| s.set_peak_hold_time(v),
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.avg_alpha,
            0.02..=1.0,
            "Trace new-sample weight",
            &mut shared.spectrum,
            |s, v| s.set_avg_alpha(v),
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.persistence,
            0.0..=0.98,
            "Persistence",
            &mut shared.spectrum,
            |s, v| s.set_persistence(v),
        );
        let mut grad = shared.config.advanced.gradient_fill;
        if ui.checkbox(&mut grad, "Gradient fill").changed() {
            shared.config.advanced.gradient_fill = grad;
            shared.spectrum.set_gradient_fill(grad);
        }
        ui.horizontal(|ui| {
            ui.label("Colour map:");
            egui::ComboBox::from_label("")
                .selected_text(shared.config.color_map.clone())
                .show_ui(ui, |ui| {
                    for c in [
                        "Classic",
                        "Viridis",
                        "Plasma",
                        "Magma",
                        "Inferno",
                        "Turbo",
                        "Grayscale",
                        "Hot",
                    ] {
                        if ui
                            .selectable_label(shared.config.color_map == c, c)
                            .clicked()
                        {
                            shared.config.color_map = c.to_string();
                            shared.spectrum.set_color_map(color_from_str(c));
                        }
                    }
                });
        });
        slider_f32(
            ui,
            &mut shared.config.advanced.db_min,
            -140.0..=-20.0,
            "dB floor",
            &mut shared.spectrum,
            |s, v| s.set_display_range(v, dbmax),
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.db_max,
            -40.0..=20.0,
            "dB ceiling",
            &mut shared.spectrum,
            |s, v| s.set_display_range(dbmin, v),
        );
    });

    // ---------------- RF / Source ----------------
    ui.collapsing("RF / Source", |ui| {
        let mut tagc = shared.config.advanced.tuner_agc;
        if ui.checkbox(&mut tagc, "Tuner AGC (next start)").changed() {
            shared.config.advanced.tuner_agc = tagc;
            shared.source.tuner_agc = tagc;
        }
        let mut ragc = shared.config.advanced.rtl_agc;
        if ui.checkbox(&mut ragc, "RTL AGC (next start)").changed() {
            shared.config.advanced.rtl_agc = ragc;
            shared.source.rtl_agc = ragc;
        }
        let mut ds = shared.config.advanced.direct_sampling;
        if ui
            .checkbox(&mut ds, "Direct sampling (next start)")
            .changed()
        {
            shared.config.advanced.direct_sampling = ds;
            shared.source.direct_sampling = ds;
        }
        let mut bt = shared.config.advanced.bias_tee;
        if ui.checkbox(&mut bt, "Bias-T (next start)").changed() {
            shared.config.advanced.bias_tee = bt;
            shared.source.bias_tee = bt;
        }
        let mut rfd = shared.config.advanced.rf_dc_remove;
        if ui.checkbox(&mut rfd, "RF DC removal").changed() {
            shared.config.advanced.rf_dc_remove = rfd;
        }
        let mut rfn = shared.config.advanced.rf_noise_blanker;
        if ui.checkbox(&mut rfn, "RF noise blanker").changed() {
            shared.config.advanced.rf_noise_blanker = rfn;
        }
        let mut rfnc = shared.config.advanced.rf_notch;
        if ui.checkbox(&mut rfnc, "RF notch").changed() {
            shared.config.advanced.rf_notch = rfnc;
        }
        slider_f32(
            ui,
            &mut shared.config.advanced.rf_notch_hz,
            100.0..=500_000.0,
            "RF notch (Hz)",
            demod,
            |_, _| {},
        );
        ui.add(
            egui::Slider::new(&mut shared.config.advanced.rf_decim, 1..=8)
                .text("Radio IQ decimation (rounded down to a power of two)"),
        );
    });

    // ---------------- Scan / Record / AI / Satellite ----------------
    ui.collapsing("Scan / Record / AI / Sat", |ui| {
        ui.horizontal(|ui| {
            ui.label("Scan direction:");
            egui::ComboBox::from_label("")
                .selected_text(shared.config.advanced.scan_direction.clone())
                .show_ui(ui, |ui| {
                    for d in ["up", "down"] {
                        if ui
                            .selectable_label(shared.config.advanced.scan_direction == d, d)
                            .clicked()
                        {
                            shared.config.advanced.scan_direction = d.to_string();
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Record format:");
            egui::ComboBox::from_label("")
                .selected_text(shared.config.advanced.record_format.clone())
                .show_ui(ui, |ui| {
                    for f in ["wav", "raw"] {
                        if ui
                            .selectable_label(shared.config.advanced.record_format == f, f)
                            .clicked()
                        {
                            shared.config.advanced.record_format = f.to_string();
                        }
                    }
                });
        });
        slider_u32(
            ui,
            &mut shared.config.advanced.record_split_mb,
            0..=2000,
            "Record split (MB)",
            demod,
            |_, _| {},
        );
        slider_u32(
            ui,
            &mut shared.config.advanced.ai_context_window,
            0..=128000,
            "AI context (tok)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.sat_elevation_offset,
            -20.0..=20.0,
            "Sat elev offset (deg)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.sat_azimuth_offset,
            -45.0..=45.0,
            "Sat az offset (deg)",
            demod,
            |_, _| {},
        );
        slider_f32(
            ui,
            &mut shared.config.advanced.map_zoom,
            0.5..=4.0,
            "Map zoom",
            demod,
            |_, _| {},
        );
        slider_u32(
            ui,
            &mut shared.config.advanced.pass_lead_min,
            1..=60,
            "Pass lead (min)",
            demod,
            |_, _| {},
        );
    });

    ui.separator();
    if ui.button("Reset all to defaults").clicked() {
        shared.config.advanced = AdvancedConfig::default();
        push_advanced(demod, shared);
    }

    // Sync the demod worker with the whole advanced config so every widget
    // edit above (and any config applied elsewhere) reaches the DSP thread.
    let mode = shared.demod_mode.resolve(shared.source.frequency_hz);
    demod.configure_advanced(&shared.config.advanced, mode);
}
