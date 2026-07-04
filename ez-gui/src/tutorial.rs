use crate::user_level::UserLevel;

/// Simplified tutorial — single welcome dialog with level selector.
/// Returns true when the tutorial is fully dismissed.
pub fn render_tutorial(
    state: &mut crate::user_level::TutorialState,
    shared: &std::sync::Arc<std::sync::Mutex<crate::app::SharedState>>,
    ui: &mut egui::Ui,
) -> bool {
    let mut dismissed = false;

    egui::Window::new("Welcome to EZ-SDR")
        .id(egui::Id::new("tutorial_welcome"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(440.0)
        .collapsible(false)
        .resizable(false)
        .show(ui.ctx(), |ui| {
            ui.label(
                egui::RichText::new("Software-Defined Radio receiver. Listen to aircraft, satellites, FM radio, and more.")
                    .size(14.0),
            );
            ui.add_space(12.0);

            ui.label(egui::RichText::new("Experience level:").strong());
            ui.add_space(4.0);

            let levels = UserLevel::levels();
            let mut level_idx = state.level as usize;
            ui.horizontal(|ui| {
                for (i, lv) in levels.iter().enumerate() {
                    let is_sel = i == level_idx;
                    let fg = if is_sel {
                        egui::Color32::from_rgb(0, 168, 255)
                    } else {
                        egui::Color32::GRAY
                    };
                    if ui
                        .selectable_label(is_sel, egui::RichText::new(lv.label()).color(fg))
                        .clicked()
                    {
                        level_idx = i;
                    }
                }
            });
            state.level = levels[level_idx];

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("Get Started").size(14.0))
                            .min_size(egui::vec2(120.0, 32.0)),
                    )
                    .clicked()
                {
                    dismissed = true;
                    state.dismiss();
                    if let Ok(mut s) = shared.try_lock() {
                        s.config.tutorial_seen = true;
                        s.config.user_level = state.level.to_str().to_string();
                        s.config.save();
                    }
                }
                ui.label(
                    egui::RichText::new("? = keyboard shortcuts, left sidebar for tools")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            });
        });

    dismissed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tutorial_dismisses_on_get_started() {
        let mut state = crate::user_level::TutorialState::new();
        state.level_chosen = true;
        state.step = 0;
        assert!(state.active);
        state.dismiss();
        assert!(!state.active);
    }

    #[test]
    fn tutorial_state_new() {
        let state = crate::user_level::TutorialState::new();
        assert!(state.active);
        assert_eq!(state.level, UserLevel::Beginner);
        assert_eq!(state.step, 0);
    }
}
