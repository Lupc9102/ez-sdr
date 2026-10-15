use ez_gui::{
    radio_ui::RadioUi,
    source_manager::SourceMode,
    test_helpers::{make_shared_state, run_ui},
};

fn main() {
    let shared = make_shared_state();
    let mut radio = RadioUi::new(shared.clone());
    {
        let mut state = shared.lock().unwrap();
        state.source.source_mode = SourceMode::Hardware;
        state.source.ppm_correction = 3;
        state.config.ppm_correction = 37; // Same field edited by the Settings popup.
    }
    println!("Before Radio sidebar: popup preference=37 ppm, source=3 ppm");
    run_ui(|ui| radio.sidebar(ui));
    let state = shared.lock().unwrap();
    println!(
        "After Radio sidebar: popup preference={} ppm, source={} ppm",
        state.config.ppm_correction, state.source.ppm_correction
    );
    assert_eq!(
        state.config.ppm_correction, 3,
        "expected the existing source to overwrite the pending popup edit"
    );
}
