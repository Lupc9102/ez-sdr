//! Generic helpers for the `⚙ More → Advanced` drawer. The actual panel
//! rendering lives as `CentralApp::render_advanced` (in `app.rs`) because it
//! needs access to `CentralApp`'s private `demod`/`shared` fields.

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
pub fn slider_f32<E, F>(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, label: &str, eng: &mut E, apply: F) -> bool
where
    F: FnOnce(&mut E, f32),
{
    if ui.add(egui::Slider::new(value, range).text(label)).changed() {
        apply(eng, *value);
        true
    } else {
        false
    }
}

pub fn slider_usize<E, F>(ui: &mut egui::Ui, value: &mut usize, range: std::ops::RangeInclusive<usize>, label: &str, eng: &mut E, apply: F) -> bool
where
    F: FnOnce(&mut E, usize),
{
    if ui.add(egui::Slider::new(value, range).text(label)).changed() {
        apply(eng, *value);
        true
    } else {
        false
    }
}

pub fn slider_u32<E, F>(ui: &mut egui::Ui, value: &mut u32, range: std::ops::RangeInclusive<u32>, label: &str, eng: &mut E, apply: F) -> bool
where
    F: FnOnce(&mut E, u32),
{
    if ui.add(egui::Slider::new(value, range).text(label)).changed() {
        apply(eng, *value);
        true
    } else {
        false
    }
}

/// Push every advanced setting from `shared.config` onto the live engines.
/// Used at startup and by the panel's "reset" button so the running app
/// matches the saved (or reset) profile.
pub fn push_advanced(
    demod: &mut crate::demod::Demodulator,
    shared: &mut crate::app::SharedState,
) {
    let a = &shared.config.advanced;
    demod.set_agc_enabled(a.agc_enabled);
    demod.set_agc_target(a.agc_target);
    demod.set_agc_attack(a.agc_attack);
    demod.set_agc_decay(a.agc_decay);
    demod.set_audio_hpf(a.audio_hpf_hz);
    demod.set_dc_blocker(a.dc_blocker);
    demod.set_deemph_tau(a.deemph_tau_us);
    demod.set_audio_gain(a.audio_gain);
    demod.set_notch(a.notch_hz, a.notch_width_hz);
    demod.set_bass(a.bass_db);
    demod.set_treble(a.treble_db);
    demod.set_noise_blanker(a.noise_blanker);
    demod.set_pitch(a.pitch_octaves);
    demod.set_rf_dc_remove(a.rf_dc_remove);
    demod.set_rf_noise_blanker(a.rf_noise_blanker);
    demod.set_rf_notch(a.rf_notch, a.rf_notch_hz);
    demod.set_rf_decim(a.rf_decim);

    let s = &mut shared.spectrum;
    s.set_fft_size(a.fft_size);
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
    shared.source.bias_tee = a.bias_tee;
}
