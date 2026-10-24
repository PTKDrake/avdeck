//! Phosphor Light glyphs (`assets/Phosphor-Light.ttf`), drawn in the text color.

use egui::{FontData, FontDefinitions, FontFamily};
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
pub const FOLD: &str = "\u{E530}";
pub const DEVICES: &str = "\u{E1E0}";
pub const SETTINGS: &str = "\u{E270}";
pub const CLOSE: &str = "\u{E4F6}";
pub const START: &str = "\u{E3D0}";
pub const STOP: &str = "\u{E46C}";
pub const ADD: &str = "\u{E3D4}";
pub const FOLDER: &str = "\u{E256}";
pub const REFRESH: &str = "\u{E094}";
pub const DELETE: &str = "\u{E4A6}";
pub const DUPLICATE: &str = "\u{E1CA}";
pub const EDIT: &str = "\u{E3B4}";
pub const MISSING: &str = "\u{E4F8}";
pub const WARNING: &str = "\u{E4E0}";
pub const DOWNLOAD: &str = "\u{E20C}";
pub const SNAPSHOT: &str = "\u{E1DA}";
pub const SEARCH: &str = "\u{E30C}";
pub const SUN: &str = "\u{E472}";
pub const MOON: &str = "\u{E330}";
pub const CHECK: &str = "\u{E182}";

/// Adds the icon font as a fallback of the proportional family.
pub fn install(fonts: &mut FontDefinitions) {
    fonts.font_data.insert(
        "phosphor".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/Phosphor-Light.ttf"
        ))),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        // First: Inter also has glyphs in the Private Use Area where Phosphor's icons live.
        family.insert(0, "phosphor".to_owned());
    }
}
