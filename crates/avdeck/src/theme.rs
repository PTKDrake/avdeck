//! Avdeck's look: Inter for text, Phosphor for icons, a light and a dark palette that follow
//! Windows, and the building blocks every page uses (buttons, chips, cards, toasts, device
//! silhouettes).

use crate::icons;
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Pos2, Rect,
    Response, RichText, Sense, Shadow, Stroke, TextStyle, Theme, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

pub struct Palette {
    /// Page background.
    pub bg: Color32,
    /// Tab bar and toolbar.
    pub chrome: Color32,
    /// Cards, dialogs, menus.
    pub surface: Color32,
    pub surface_hover: Color32,
    pub border: Color32,
    pub input: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    /// Toasts over the device screen.
    pub overlay: Color32,
    pub shadow: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const DARK: Palette = Palette {
    bg: hex(0x111318),
    chrome: hex(0x0B0D11),
    surface: hex(0x181B21),
    surface_hover: hex(0x20242C),
    border: hex(0x2A2F38),
    input: hex(0x0E1014),
    text: hex(0xE7E9EE),
    muted: hex(0x9AA3AF),
    faint: hex(0x6B7280),
    accent: hex(0x4C8DFF),
    accent_hover: hex(0x6AA0FF),
    on_accent: hex(0xFFFFFF),
    success: hex(0x3FB97A),
    warning: hex(0xE0A93B),
    danger: hex(0xEF5B5B),
    overlay: Color32::from_rgba_premultiplied(10, 12, 16, 225),
    shadow: Color32::from_black_alpha(110),
};

pub const LIGHT: Palette = Palette {
    bg: hex(0xF5F6F8),
    chrome: hex(0xEBEDF1),
    surface: hex(0xFFFFFF),
    surface_hover: hex(0xF1F3F7),
    border: hex(0xDADEE5),
    input: hex(0xFFFFFF),
    text: hex(0x15181E),
    muted: hex(0x5B6472),
    faint: hex(0x8A93A0),
    accent: hex(0x2F6FEB),
    accent_hover: hex(0x255FD0),
    on_accent: hex(0xFFFFFF),
    success: hex(0x1F9D57),
    warning: hex(0xB7791F),
    danger: hex(0xD64545),
    overlay: Color32::from_rgba_premultiplied(18, 20, 26, 220),
    shadow: Color32::from_black_alpha(40),
};

pub fn palette(ui: &Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Light, dark, or whatever Windows uses.
pub fn set_appearance(ctx: &egui::Context, appearance: avdeck_core::config::Appearance) {
    use avdeck_core::config::Appearance;
    ctx.set_theme(match appearance {
        Appearance::System => egui::ThemePreference::System,
        Appearance::Light => egui::ThemePreference::Light,
        Appearance::Dark => egui::ThemePreference::Dark,
    });
}

/// Fonts, both styles, and following the Windows light/dark setting.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let font = |bytes: &'static [u8]| Arc::new(FontData::from_static(bytes));
    fonts.font_data.insert(
        "inter".into(),
        font(include_bytes!("../assets/Inter-Regular.ttf")),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        font(include_bytes!("../assets/Inter-SemiBold.ttf")),
    );
    icons::install(&mut fonts);
    // Phosphor (Private Use Area only) first, then Inter; egui's defaults stay as fallbacks.
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(1, "inter".into());
    let mut semibold = vec!["phosphor".to_owned(), "inter-semibold".to_owned()];
    semibold.extend(proportional.iter().skip(2).cloned());
    fonts.families.insert(self::semibold(), semibold);
    ctx.set_fonts(fonts);

    ctx.set_style_of(Theme::Dark, style(&DARK, true));
    ctx.set_style_of(Theme::Light, style(&LIGHT, false));
    ctx.set_theme(egui::ThemePreference::System);
}

fn style(palette: &Palette, dark: bool) -> egui::Style {
    let mut style = egui::Style {
        text_styles: [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::new(22.0, semibold())),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into(),
        ..Default::default()
    };
    // Labels are not text to select; text fields still are.
    style.interaction.selectable_labels = false;
    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(8.0, 8.0);
    spacing.button_padding = vec2(12.0, 6.0);
    spacing.interact_size = vec2(32.0, 30.0);
    spacing.menu_margin = Margin::same(6);
    spacing.window_margin = Margin::same(20);
    spacing.combo_height = 320.0;

    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let radius = CornerRadius::same(8);
    visuals.panel_fill = palette.bg;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: palette.shadow,
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: palette.shadow,
    };
    visuals.extreme_bg_color = palette.input;
    visuals.text_edit_bg_color = Some(palette.input);
    visuals.faint_bg_color = palette.surface;
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.danger;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    // Hint text in empty fields must not read as a value.
    visuals.weak_text_color = Some(palette.faint);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.surface;
    widgets.noninteractive.weak_bg_fill = palette.surface;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.noninteractive.corner_radius = radius;
    for (state, fill, stroke) in [
        (&mut widgets.inactive, palette.surface, palette.border),
        (&mut widgets.hovered, palette.surface_hover, palette.faint),
        (&mut widgets.active, palette.surface_hover, palette.accent),
        (&mut widgets.open, palette.surface_hover, palette.border),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.5, palette.text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    style.visuals = visuals;
    style
}

pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).family(semibold()).size(22.0));
}

pub fn title(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(semibold()).size(size)
}

pub fn muted(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text).color(palette(ui).muted)
}

/// A filled button whose fill follows its state (normal, hovered, pressed).
fn filled(ui: &mut Ui, text: impl Into<String>, fill: Color32, hover: Color32) -> Response {
    let on = palette(ui).on_accent;
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (state, color) in [
            (&mut widgets.inactive, fill),
            (&mut widgets.hovered, hover),
            (&mut widgets.active, hover),
        ] {
            state.weak_bg_fill = color;
            state.bg_fill = color;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.5, on);
        }
        ui.add(egui::Button::new(RichText::new(text).color(on)).min_size(vec2(0.0, 32.0)))
    })
    .inner
}

/// The main action of a page or dialog.
pub fn primary(ui: &mut Ui, text: impl Into<String>) -> Response {
    let palette = palette(ui);
    filled(ui, text, palette.accent, palette.accent_hover)
}

/// A destructive action (delete, wipe).
pub fn danger(ui: &mut Ui, text: impl Into<String>) -> Response {
    let palette = palette(ui);
    filled(
        ui,
        text,
        palette.danger,
        palette.danger.gamma_multiply(0.85),
    )
}

pub fn secondary(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(egui::Button::new(text.into()).min_size(vec2(0.0, 32.0)))
}

/// A square icon button with a soft background on hover.
pub fn icon(ui: &mut Ui, glyph: &str, tooltip: &str, size: f32) -> Response {
    let palette = palette(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), palette.surface_hover);
    }
    let color = if response.hovered() {
        palette.text
    } else {
        palette.muted
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(size * 0.58),
        color,
    );
    response.on_hover_text(tooltip)
}

/// A rounded status pill: colored dot and text.
pub fn chip(ui: &mut Ui, color: Color32, text: &str) -> Response {
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let size = vec2(galley.size().x + 26.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), color.gamma_multiply(0.14));
    painter.circle_filled(pos2(rect.left() + 11.0, rect.center().y), 3.5, color);
    painter.galley(
        pos2(rect.left() + 19.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
}

/// A dark pill at the bottom of `rect`, over the device screen.
pub fn toast(ui: &Ui, rect: Rect, text: &str) {
    let palette = palette(ui);
    let painter = ui.painter();
    let galley = painter.layout(
        text.to_owned(),
        FontId::proportional(13.0),
        Color32::WHITE,
        (rect.width() - 48.0).max(120.0),
    );
    let size = galley.size() + vec2(28.0, 16.0);
    let place = Rect::from_center_size(
        pos2(rect.center().x, rect.bottom() - size.y / 2.0 - 20.0),
        size,
    );
    painter.rect_filled(place, CornerRadius::same(10), palette.overlay);
    painter.galley(place.min + vec2(14.0, 8.0), galley, Color32::WHITE);
}

/// A device outline at the shape of its display (`panel`, natural orientation), fitted and
/// centered in `rect`, its screen filled with `screen` or a texture.
pub fn device(
    ui: &Ui,
    rect: Rect,
    panel: (u32, u32),
    screen: Option<egui::TextureId>,
    lit: bool,
) -> Rect {
    let palette = palette(ui);
    let (width, height) = (panel.0.max(1) as f32, panel.1.max(1) as f32);
    let bezel = 5.0;
    let scale = ((rect.width() - 2.0 * bezel) / width).min((rect.height() - 2.0 * bezel) / height);
    let screen_size = vec2(width * scale, height * scale);
    let body = Rect::from_center_size(rect.center(), screen_size + Vec2::splat(2.0 * bezel));
    let short = body.width().min(body.height());
    let painter = ui.painter();
    painter.rect_filled(
        body,
        CornerRadius::same((short * 0.12).min(255.0) as u8),
        if ui.visuals().dark_mode {
            hex(0x2B3038)
        } else {
            hex(0x1E2228)
        },
    );
    let inner = body.shrink(bezel);
    let radius = CornerRadius::same((short * 0.09).min(255.0) as u8);
    match screen {
        Some(texture) => {
            painter.add(
                egui::epaint::RectShape::filled(inner, radius, Color32::WHITE)
                    .with_texture(texture, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
            );
        }
        None => {
            let fill = if lit {
                palette.accent.gamma_multiply(0.30)
            } else {
                hex(0x0C0E12)
            };
            painter.rect_filled(inner, radius, fill);
        }
    }
    inner
}

/// A popup menu's content: fixed width, items filling it, left-aligned.
pub fn menu<R>(ui: &mut Ui, width: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.set_width(width);
    // One line per item; long ones end in "…".
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
    ui.spacing_mut().button_padding = vec2(10.0, 7.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    // Disabled items stay readable.
    ui.visuals_mut().widgets.noninteractive.fg_stroke.color = palette(ui).faint;
    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), content)
        .inner
}

/// A menu item: icon and text, left-aligned, full width.
pub fn menu_item(ui: &mut Ui, glyph: &str, text: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(format!("{glyph}   {text}")).frame_when_inactive(false),
    )
}

/// An item of a searchable select: the value, its label, an optional hover text and group.
pub struct Choice<T> {
    pub value: T,
    pub label: String,
    pub hover: String,
    pub group: Option<&'static str>,
}

/// The search box at the top of a long list. Its text lives in memory under `id` while the
/// list is open; returns the lowercase query.
pub fn search_box(ui: &mut Ui, id: egui::Id) -> String {
    let mut query: String = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    let response = ui.add(
        egui::TextEdit::singleline(&mut query)
            .hint_text(format!("{}  Search", icons::SEARCH))
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(8, 6)),
    );
    // Typing goes straight into the box when the list opens.
    let opened = ui.data(|data| data.get_temp::<bool>(id.with("open")).is_none());
    if opened {
        response.request_focus();
    }
    ui.data_mut(|data| {
        data.insert_temp(id.with("open"), true);
        data.insert_temp(id, query.clone());
    });
    ui.add_space(4.0);
    query.trim().to_lowercase()
}

/// Forgets the query once the list has closed.
pub fn search_closed(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|data| {
        data.remove::<String>(id);
        data.remove::<bool>(id.with("open"));
    });
}

fn matches<T>(choice: &Choice<T>, query: &str) -> bool {
    query.is_empty()
        || choice.label.to_lowercase().contains(query)
        || choice.hover.to_lowercase().contains(query)
}

/// A combo box with a search box on top, for long lists; groups get small headings.
pub fn searchable_combo<T: Clone + PartialEq>(
    ui: &mut Ui,
    id: &str,
    selected_text: impl Into<egui::WidgetText>,
    width: f32,
    choices: &[Choice<T>],
    value: &mut T,
) -> Response {
    let search = egui::Id::unique(id).with("search");
    let inner = egui::ComboBox::new(id, "")
        .selected_text(selected_text)
        .truncate()
        .width(width)
        .height(360.0)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            let query = search_box(ui, search);
            let faint = palette(ui).faint;
            let mut group = None;
            let mut shown = 0;
            for choice in choices.iter().filter(|choice| matches(choice, &query)) {
                if choice.group != group {
                    group = choice.group;
                    if let Some(title) = group {
                        ui.add_space(4.0);
                        ui.label(RichText::new(title).small().color(faint));
                    }
                }
                let response = ui.selectable_value(value, choice.value.clone(), &choice.label);
                if !choice.hover.is_empty() {
                    response.on_hover_text(&choice.hover);
                }
                shown += 1;
            }
            if shown == 0 {
                ui.label(muted(ui, "Nothing matches"));
            }
        });
    if inner.inner.is_none() {
        search_closed(ui.ctx(), search);
    }
    inner.response
}

/// A large clickable choice in a dialog: icon, title and what choosing it does. The
/// recommended one has an accent border.
pub fn option_card(
    ui: &mut Ui,
    glyph: &str,
    title: &str,
    detail: &str,
    recommended: bool,
) -> Response {
    let palette = palette(ui);
    let width = ui.available_width();
    let detail_galley = ui.painter().layout(
        detail.to_owned(),
        FontId::proportional(12.5),
        palette.muted,
        width - 72.0,
    );
    let height = (22.0 + detail_galley.size().y + 26.0).max(62.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    let border = if hovered || recommended {
        palette.accent
    } else {
        palette.border
    };
    painter.rect_filled(
        rect,
        CornerRadius::same(10),
        if hovered {
            palette.surface_hover
        } else {
            palette.surface
        },
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(if recommended { 1.5 } else { 1.0 }, border),
        egui::StrokeKind::Inside,
    );
    let icon = pos2(rect.left() + 30.0, rect.center().y);
    painter.circle_filled(
        icon,
        16.0,
        palette
            .accent
            .gamma_multiply(if recommended { 0.25 } else { 0.12 }),
    );
    painter.text(
        icon,
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(17.0),
        palette.accent,
    );
    let text_left = rect.left() + 58.0;
    let title_top = rect.top() + (height - 20.0 - detail_galley.size().y) / 2.0;
    painter.text(
        pos2(text_left, title_top),
        egui::Align2::LEFT_TOP,
        title,
        FontId::new(14.5, semibold()),
        palette.text,
    );
    painter.galley(
        pos2(text_left, title_top + 20.0),
        detail_galley,
        palette.muted,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A card: surface, border, rounded, with padding.
pub fn card(ui: &Ui) -> egui::Frame {
    let palette = palette(ui);
    egui::Frame::NONE
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
}
