//! Shared widget layer built on top of `theme.rs` design tokens and
//! `fx.rs` painter primitives. Keeps those two modules focused (pure
//! tokens, pure painters) while this one composes them into ready-to-use
//! widgets shared across the Listen/Satellites/Planes tabs.

use crate::theme::ThemeConfig;

/// A bordered, headered collapsible section — the standard sidebar
/// building block across all tabs, mirroring SDR++'s stacked left-sidebar
/// modules.
///
/// `id_key` must be unique per call site (e.g. `"listen.frequency"`).
/// `CollapsingHeader` derives its persisted open/closed state from an `Id`
/// that defaults to the header text, so two cards that happen to share a
/// title (e.g. a future second "Source" card on another tab) would
/// otherwise silently share collapsed/open state; `push_id` scopes it.
pub fn module_card<R>(
    ui: &mut egui::Ui,
    theme: &ThemeConfig,
    id_key: &str,
    icon: &str,
    title: &str,
    default_open: bool,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let mut result = None;
    egui::Frame::new()
        .fill(theme.surface.to_egui())
        .stroke(egui::Stroke::new(1.0, theme.text_dim.with_alpha(60).to_egui()))
        .corner_radius(egui::CornerRadius::same(
            theme.corner.panels.clamp(0.0, 255.0) as u8,
        ))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.push_id(id_key, |ui| {
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("{icon}  {title}"))
                        .color(theme.text_heading.to_egui())
                        .strong(),
                )
                .default_open(default_open)
                .show(ui, |ui| {
                    result = Some(add_contents(ui));
                });
            });
        });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_contents_and_returns_value_when_default_open() {
        let theme = ThemeConfig::dark();
        let ctx = egui::Context::default();
        let mut returned = None;
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new("test".into()).show(ctx, |ui| {
                returned = module_card(ui, &theme, "test.open", "🔧", "Card", true, |_ui| 42);
            });
        });
        assert_eq!(returned, Some(42));
    }

    #[test]
    fn skips_contents_when_default_closed() {
        let theme = ThemeConfig::dark();
        let ctx = egui::Context::default();
        let mut returned = None;
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new("test".into()).show(ctx, |ui| {
                returned = module_card(ui, &theme, "test.closed", "🔧", "Card", false, |_ui| 42);
            });
        });
        assert_eq!(returned, None);
    }

    #[test]
    fn distinct_id_keys_give_independent_state_for_same_title() {
        // The whole reason `id_key` is a required param: two cards sharing a
        // title must not share CollapsingHeader's persisted open/closed state.
        let theme = ThemeConfig::dark();
        let ctx = egui::Context::default();
        let mut a_ran = false;
        let mut b_ran = false;
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::Area::new("test".into()).show(ctx, |ui| {
                module_card(ui, &theme, "tab_a.source", "📡", "Source", true, |_ui| {
                    a_ran = true
                });
                module_card(ui, &theme, "tab_b.source", "📡", "Source", false, |_ui| {
                    b_ran = true
                });
            });
        });
        assert!(a_ran, "default_open=true card should run its contents");
        assert!(
            !b_ran,
            "a same-titled card with a different id_key and default_open=false must stay closed"
        );
    }
}
