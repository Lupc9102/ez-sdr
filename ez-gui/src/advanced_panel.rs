//! Apply persisted advanced settings to the live spectrum and receiver engines.

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

/// Push every advanced setting from `shared.config` onto the live engines.
pub fn push_advanced(demod: &mut crate::demod::DemodWorker, shared: &mut crate::app::SharedState) {
    shared.config.normalize();
    let previous_hardware = shared.source.rtl_open_settings();
    let mode = shared.demod_mode.resolve(shared.source.frequency_hz);
    let a = &shared.config.advanced;
    let s = &mut shared.spectrum;
    s.set_fft_size(a.fft_size);
    s.set_fft_rate(a.fft_rate);
    s.set_waterfall_visible(a.waterfall_visible);
    s.full_waterfall_update = a.full_waterfall_update;
    s.set_snr_smoothing(a.snr_smoothing, a.snr_smoothing_secs);
    s.set_window(window_from_str(&a.window));
    s.set_waterfall_history(a.wf_depth);
    s.waterfall_every_n = a.wf_speed.max(1);
    s.set_grid(a.grid);
    s.set_peak_hold_time(a.peak_hold_time);
    s.set_smoothing(a.smoothing_enabled);
    s.set_smoothing_speed(1.0 - a.avg_alpha);
    s.set_avg_alpha(a.avg_alpha);
    s.set_fast_fft(a.fast_fft);
    s.set_persistence(a.persistence);
    s.set_gradient_fill(a.gradient_fill);
    s.set_color_map(color_from_str(&shared.config.color_map));
    s.set_display_range(a.db_min, a.db_max);
    s.wf_min_db = shared.config.wf_min_db;
    s.wf_max_db = shared.config.wf_max_db;

    shared.source.tuner_agc = a.tuner_agc;
    shared.source.rtl_agc = a.rtl_agc;
    shared.source.direct_sampling = a.direct_sampling;
    shared.source.direct_sampling_branch = a.direct_sampling_branch;
    shared.source.ppm_correction = shared.config.ppm_correction;
    shared.lo_offset_hz = shared.config.lo_offset_hz;
    shared.source.frequency_offset_hz = shared.lo_offset_hz;
    shared.source.bias_tee = a.bias_tee;
    shared.source.rtl_device = a.rtl_device.clone();
    shared.source.offset_tuning = a.offset_tuning;
    // Persist validated engine values before the first UI can write defaults
    // back into the imported profile. No device is opened on startup.
    crate::radio_ui::persist_display_settings(shared);
    // A loaded profile is already the selected mode's profile. Do not replace
    // it with mode defaults on the first frame or after a configuration import.
    shared.config.last_session_demod = shared.demod_mode.label().into();
    shared.config.radio_profiles.insert(
        shared.demod_mode.label().into(),
        crate::config::RadioModeProfile::capture(&shared.config.advanced),
    );
    crate::radio_ui::sync_demod_settings(shared);
    demod.configure_advanced(&shared.config.advanced, mode);
    if shared.source.source_mode == crate::source_manager::SourceMode::Hardware
        && previous_hardware != shared.source.rtl_open_settings()
    {
        crate::radio_ui::restart_if_running(&mut shared.source);
    }
}
