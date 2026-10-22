//! Phosphor Light glyphs (`assets/Phosphor-Light.ttf`), drawn in the text color.

use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

pub const POWER: &str = "\u{E3DA}";
pub const VOLUME_UP: &str = "\u{E44A}";
pub const VOLUME_DOWN: &str = "\u{E44C}";
pub const ROTATE_LEFT: &str = "\u{E038}";
pub const ROTATE_RIGHT: &str = "\u{E036}";
pub const SCREENSHOT: &str = "\u{E10E}";
pub const BACK: &str = "\u{E138}";
pub const HOME: &str = "\u{E18A}";
pub const OVERVIEW: &str = "\u{E45E}";
pub const MORE: &str = "\u{E1FE}";

/// Adds the icon font as a fallback of the proportional family.
pub fn install(fonts: &mut FontDefinitions) {
    fonts.font_data.insert(
        "phosphor".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/Phosphor-Light.ttf"
        ))),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.push("phosphor".to_owned());
    }
}
