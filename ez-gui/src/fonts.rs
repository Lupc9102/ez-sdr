//! Bundled font installation for the [`crate::theme::FontFamily`] picker.

use crate::theme::FontFamily;

static DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
static DEJAVU_SANS_MONO: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");

/// Register bundled fonts and select which family backs egui's
/// `Proportional` / `Monospace` slots.
pub fn install(ctx: &egui::Context, family: FontFamily) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "dejavu_sans".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(DEJAVU_SANS)),
    );
    fonts.font_data.insert(
        "dejavu_sans_mono".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(DEJAVU_SANS_MONO)),
    );

    match family {
        FontFamily::EguiDefault => {
            // egui's own defaults are already first in the family lists.
        }
        FontFamily::DejaVu => {
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "dejavu_sans".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "dejavu_sans_mono".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}
