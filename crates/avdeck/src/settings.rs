//! The Settings page: SDK and tool paths (when the environment has none, or to use others),
//! how emulators start, what closing does, where screenshots go, and about Avdeck. The header with Save and Revert stays put while
//! the page scrolls; wide windows get two columns.

use crate::app::App;
use crate::icons;
use crate::theme;
use avdeck_core::config::{Appearance, CloseAction, Config};
use avdeck_core::launch::GPU_MODES;
use avdeck_core::screenshots;
use avdeck_core::sdk::{self, Found, Overrides};
use egui::{self, vec2};
use std::path::PathBuf;

/// What the page edits, applied with Save.
#[derive(Clone, PartialEq)]
pub struct SettingsForm {
    /// Paths set here; empty means "find it".
    paths: [String; 6],
    /// What is found without a setting, per field, shown as the placeholder.
    found: [Option<Found>; 6],
    gpu: Option<String>,
    no_snapshot_save: bool,
    close_tab: CloseAction,
    close_app: CloseAction,
    appearance: Appearance,
    /// Screenshot folder set here; empty means the emulator's setting, else `Pictures\Avdeck`.
    screenshots: String,
    /// Where screenshots go without a folder set here, shown as the placeholder.
    screenshots_found: Option<Found>,
}

/// Rows: label, what it is, what the browse button picks.
pub(crate) const ROWS: [(&str, &str, Pick); 6] = [
    (
        "Android SDK",
        "Folder with platform-tools or emulator",
        Pick::Folder,
    ),
    ("Emulator", "emulator.exe", Pick::File),
    ("adb", "adb.exe", Pick::File),
    (
        "Command-line tools",
        "Folder with avdmanager.bat and sdkmanager.bat",
        Pick::Folder,
    ),
    (
        "AVD folder",
        "Where each device's .ini and .avd live",
        Pick::Folder,
    ),
    (
        "JDK",
        "JDK home with bin\\java.exe, for avdmanager and sdkmanager",
        Pick::Folder,
    ),
];

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Pick {
    Folder,
    File,
}

fn path(text: &str) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"');
    (!text.is_empty()).then(|| PathBuf::from(text))
}

pub(crate) fn field(overrides: &mut Overrides, index: usize) -> &mut Option<PathBuf> {
    match index {
        0 => &mut overrides.sdk,
        1 => &mut overrides.emulator,
        2 => &mut overrides.adb,
        3 => &mut overrides.cmdline_tools,
        4 => &mut overrides.avd_home,
        _ => &mut overrides.jdk,
    }
}

fn found(tools: &sdk::Tools, index: usize) -> Option<Found> {
    match index {
        0 => tools.sdk.clone(),
        1 => tools.emulator.clone(),
        2 => tools.adb.clone(),
        3 => tools.cmdline_tools.clone(),
        4 => tools.avd_home.clone(),
        _ => tools.jdk.clone(),
    }
}

impl SettingsForm {
    pub fn new(config: &Config) -> Self {
        let mut overrides = config.tools.clone();
        let paths = std::array::from_fn(|index| {
            field(&mut overrides, index)
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        });
        // What each field finds by itself: resolve again without its own setting.
        let found = std::array::from_fn(|index| {
            let mut without = config.tools.clone();
            *field(&mut without, index) = None;
            found(&sdk::resolve(&without, &sdk::System), index)
        });
        Self {
            paths,
            found,
            gpu: config.launch.gpu.clone(),
            no_snapshot_save: config.launch.no_snapshot_save,
            close_tab: config.close_tab,
            close_app: config.close_app,
            appearance: config.appearance,
            screenshots: config
                .screenshots
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            screenshots_found: screenshots::folder(None),
        }
    }

    pub fn apply(&self, config: &mut Config) {
        for index in 0..ROWS.len() {
            *field(&mut config.tools, index) = path(&self.paths[index]);
        }
        config.launch.gpu = self.gpu.clone();
        config.launch.no_snapshot_save = self.no_snapshot_save;
        config.close_tab = self.close_tab;
        config.close_app = self.close_app;
        config.appearance = self.appearance;
        config.screenshots = path(&self.screenshots);
    }

    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Whether Save would change the configuration.
    fn changed(&self, config: &Config) -> bool {
        let mut applied = config.clone();
        self.apply(&mut applied);
        applied != *config
    }
}

/// Widest the page gets; below `TWO_COLUMNS` it is a single column.
const MAX_WIDTH: f32 = 1160.0;
const TWO_COLUMNS: f32 = 940.0;
const SIDE_COLUMN: f32 = 340.0;
const GAP: f32 = 16.0;

/// `add` in a column of `width`, centered in `ui`.
pub(crate) fn centered<R>(
    ui: &mut egui::Ui,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let margin = ((ui.available_width() - width) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(margin);
        ui.vertical(|ui| {
            ui.set_width(width);
            add(ui)
        })
        .inner
    })
    .inner
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    let width = (ui.available_width() - 64.0).clamp(300.0, MAX_WIDTH);

    // The header stays while the rest scrolls.
    ui.add_space(20.0);
    centered(ui, width, |ui| header(app, ui));
    ui.add_space(14.0);
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(
        line.x_range(),
        line.top(),
        egui::Stroke::new(1.0, palette.border),
    );

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.add_space(18.0);
            centered(ui, width, |ui| {
                if width >= TWO_COLUMNS {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = GAP;
                        ui.vertical(|ui| {
                            ui.set_width(width - SIDE_COLUMN - GAP);
                            tools(app, ui);
                            about(ui);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(SIDE_COLUMN);
                            appearance(app, ui);
                            starting(app, ui);
                            closing(app, ui);
                            screenshots(app, ui);
                        });
                    });
                } else if width >= 600.0 {
                    // One column for the paths, the two small cards side by side below.
                    tools(app, ui);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = GAP;
                        let half = (width - GAP) / 2.0;
                        ui.vertical(|ui| {
                            ui.set_width(half);
                            appearance(app, ui);
                            starting(app, ui);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(half);
                            closing(app, ui);
                            screenshots(app, ui);
                        });
                    });
                    about(ui);
                } else {
                    tools(app, ui);
                    appearance(app, ui);
                    starting(app, ui);
                    closing(app, ui);
                    screenshots(app, ui);
                    about(ui);
                }
            });
            ui.add_space(22.0);
        });
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    let changed = app.settings.changed(&app.config);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            theme::heading(ui, "Settings");
            ui.label(theme::muted(
                ui,
                Config::path().map_or(String::new(), |path| path.display().to_string()),
            ));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save = ui
                .add_enabled_ui(changed, |ui| {
                    theme::primary(ui, format!("{}  Save", icons::CHECK))
                })
                .inner;
            if save.clicked() {
                app.settings.apply(&mut app.config);
                app.config_changed();
                theme::set_appearance(ui.ctx(), app.config.appearance);
            }
            if ui
                .add_enabled_ui(changed, |ui| theme::secondary(ui, "Revert"))
                .inner
                .clicked()
            {
                app.settings = SettingsForm::new(&app.config);
            }
            if changed {
                ui.label(theme::muted(ui, "Unsaved changes"));
            }
            if theme::secondary(ui, "Run setup")
                .on_hover_text("The first-run welcome, environment check and preferences")
                .clicked()
            {
                // The setup edits the preferences from what is saved.
                app.settings = SettingsForm::new(&app.config);
                app.setup = Some(crate::setup::Setup::new());
            }
        });
    });
}

fn card(ui: &mut egui::Ui, title: &str, description: &str, content: impl FnOnce(&mut egui::Ui)) {
    theme::card(ui)
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::title(title, 15.0));
            if !description.is_empty() {
                ui.label(theme::muted(ui, description));
            }
            ui.add_space(10.0);
            content(ui);
        });
    ui.add_space(GAP);
}

fn tools(app: &mut App, ui: &mut egui::Ui) {
    card(
        ui,
        "Android SDK and tools",
        "Empty fields use what the environment (ANDROID_HOME, ANDROID_AVD_HOME, JAVA_HOME, PATH) \
         or the usual places give, shown in grey. Type or browse to use another path.",
        |ui| {
            // Widths fixed once for every row: a row that overflows by a few points would
            // widen the card, and the next row with it.
            let inner = ui.available_width();
            // Labels on the left when there is room for them.
            let label_width = if inner > 560.0 { 170.0 } else { 0.0 };
            let spacing = ui.spacing().item_spacing.x;
            let row = if label_width > 0.0 {
                inner - label_width - spacing
            } else {
                inner
            };
            let input_width = row - 32.0 - spacing - 6.0;
            for index in 0..ROWS.len() {
                if index > 0 {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }
                if label_width > 0.0 {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(label_width);
                            label(ui, index);
                        });
                        ui.vertical(|ui| path_field(app, ui, index, input_width));
                    });
                } else {
                    label(ui, index);
                    path_field(app, ui, index, input_width);
                }
            }
        },
    );
}

fn label(ui: &mut egui::Ui, index: usize) {
    let (label, what, _) = ROWS[index];
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(label)
            .family(theme::semibold())
            .size(13.5),
    );
    ui.label(
        egui::RichText::new(what)
            .size(12.0)
            .color(theme::palette(ui).faint),
    );
}

/// The path input (placeholder: what is found by itself; × clears a path set here), the
/// browse button, and where the path comes from.
fn path_field(app: &mut App, ui: &mut egui::Ui, index: usize, width: f32) {
    let palette = theme::palette(ui);
    let (_, what, pick) = ROWS[index];
    let found = app.settings.found[index].clone();
    ui.horizontal(|ui| {
        let placeholder = found
            .as_ref()
            .map_or(what.to_owned(), |found| found.path.display().to_string());
        path_input(
            ui,
            &mut app.settings.paths[index],
            &placeholder,
            width,
            "Clear: use what is found automatically",
        );
        if theme::icon(ui, icons::FOLDER, "Browse", 32.0).clicked() {
            let dialog = rfd::FileDialog::new();
            let picked = match pick {
                Pick::Folder => dialog.pick_folder(),
                Pick::File => dialog.add_filter("Program", &["exe"]).pick_file(),
            };
            if let Some(picked) = picked {
                app.settings.paths[index] = picked.display().to_string();
            }
        }
    });

    // Where the value comes from, or what is wrong.
    let typed = path(&app.settings.paths[index]);
    let (mut text, color) = match (&typed, &found) {
        (Some(typed), _) if !typed.exists() => (
            format!("{}  This path does not exist", icons::MISSING),
            palette.danger,
        ),
        (Some(_), _) => (format!("{}  Set here", icons::CHECK), palette.accent),
        (None, Some(found)) => (format!("{}  {}", icons::CHECK, found.via), palette.success),
        (None, None) => (
            format!("{}  Not found. {what}", icons::MISSING),
            palette.danger,
        ),
    };
    let effective = typed.or(found.map(|found| found.path));
    if index == 1
        && let Some(path) = &effective
        && let Some(version) = sdk::emulator_version(path)
    {
        text.push_str(&format!("  ·  version {version}"));
    }
    ui.label(egui::RichText::new(text).size(12.0).color(color));
}

/// A path input with `placeholder` when empty and a × at its end that clears it.
fn path_input(ui: &mut egui::Ui, text: &mut String, placeholder: &str, width: f32, clear: &str) {
    let palette = theme::palette(ui);
    let set = !text.trim().is_empty();
    let input = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(placeholder)
            .desired_width(width)
            .margin(egui::Margin {
                left: 10,
                right: if set { 32 } else { 10 },
                top: 7,
                bottom: 7,
            }),
    );
    if !set {
        return;
    }
    let rect = egui::Rect::from_center_size(
        egui::pos2(input.rect.right() - 16.0, input.rect.center().y),
        vec2(22.0, 22.0),
    );
    let response = ui
        .interact(rect, input.id.with("clear"), egui::Sense::click())
        .on_hover_text(clear);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), palette.surface_hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::CLOSE,
        egui::FontId::proportional(13.0),
        if response.hovered() {
            palette.text
        } else {
            palette.muted
        },
    );
    if response.clicked() {
        text.clear();
    }
}

/// System, Light, Dark as one segmented control; applied on Save.
pub(crate) fn appearance(app: &mut App, ui: &mut egui::Ui) {
    card(ui, "Appearance", "", |ui| {
        let palette = theme::palette(ui);
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(9), palette.input);
        let choices = [
            (Appearance::System, icons::DEVICES, "System"),
            (Appearance::Light, icons::SUN, "Light"),
            (Appearance::Dark, icons::MOON, "Dark"),
        ];
        let segment = (width - 6.0) / choices.len() as f32;
        // The selection pill slides to the chosen segment.
        let chosen = choices
            .iter()
            .position(|(value, _, _)| *value == app.settings.appearance)
            .unwrap_or(0);
        let at = ui.ctx().animate_value_with_time(
            ui.scope_id().with("appearance-pill"),
            chosen as f32,
            0.18,
        );
        let pill = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 3.0 + segment * at, rect.top() + 3.0),
            vec2(segment, 28.0),
        );
        ui.painter()
            .rect_filled(pill, egui::CornerRadius::same(7), palette.surface_hover);
        ui.painter().rect_stroke(
            pill,
            egui::CornerRadius::same(7),
            egui::Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        );
        for (index, (value, glyph, label)) in choices.into_iter().enumerate() {
            let cell = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 3.0 + segment * index as f32, rect.top() + 3.0),
                vec2(segment, 28.0),
            );
            let response = ui.interact(
                cell,
                ui.scope_id().with(("appearance", index)),
                egui::Sense::click(),
            );
            let selected = app.settings.appearance == value;
            let hot = if selected {
                1.0
            } else {
                theme::hover(ui, &response)
            };
            let color = palette.muted.lerp_to_gamma(palette.text, hot);
            ui.painter().text(
                cell.center(),
                egui::Align2::CENTER_CENTER,
                format!("{glyph}  {label}"),
                egui::FontId::proportional(13.5),
                color,
            );
            if response.clicked() {
                app.settings.appearance = value;
            }
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("System follows the Windows light or dark setting.")
                .size(12.0)
                .color(palette.faint),
        );
    });
}

pub(crate) fn starting(app: &mut App, ui: &mut egui::Ui) {
    card(
        ui,
        "Starting devices",
        "Defaults for Start. \"Start with options…\" overrides them once.",
        |ui| {
            // A combo box is its width plus its padding.
            let width = ui.available_width() - 16.0;
            ui.label("Graphics");
            egui::ComboBox::new("gpu", "")
                .selected_text(app.settings.gpu.as_deref().unwrap_or("host"))
                .width(width)
                .show_ui(ui, |ui| {
                    for mode in GPU_MODES {
                        ui.selectable_value(&mut app.settings.gpu, Some(mode.to_owned()), mode);
                    }
                });
            ui.label(
                egui::RichText::new("host uses the GPU; snapshots saved with it need it.")
                    .size(12.0)
                    .color(theme::palette(ui).faint),
            );
            ui.add_space(6.0);
            ui.checkbox(
                &mut app.settings.no_snapshot_save,
                "Don't save the device state when it stops",
            );
        },
    );
}

pub(crate) fn closing(app: &mut App, ui: &mut egui::Ui) {
    let name = |action: CloseAction| match action {
        CloseAction::Ask => "Ask every time",
        CloseAction::Stop => "Stop the emulator",
        CloseAction::KeepRunning => "Keep it running",
    };
    card(ui, "Closing", "What happens to running devices.", |ui| {
        let width = ui.available_width() - 16.0;
        for (label, id, value) in [
            (
                "Closing a device tab",
                "close-tab",
                &mut app.settings.close_tab,
            ),
            ("Closing Avdeck", "close-app", &mut app.settings.close_app),
        ] {
            ui.label(label);
            egui::ComboBox::new(id, "")
                .selected_text(name(*value))
                .width(width)
                .show_ui(ui, |ui| {
                    for action in [
                        CloseAction::Ask,
                        CloseAction::Stop,
                        CloseAction::KeepRunning,
                    ] {
                        ui.selectable_value(value, action, name(action));
                    }
                });
            ui.add_space(6.0);
        }
    });
}

/// The screenshot folder: set here, or the emulator's own setting, or `Pictures\Avdeck`.
pub(crate) fn screenshots(app: &mut App, ui: &mut egui::Ui) {
    card(
        ui,
        "Screenshots",
        "Where the toolbar's screenshot button saves. Empty follows the emulator's \
         \"Screenshot save location\" (Extended controls › Settings).",
        |ui| {
            let palette = theme::palette(ui);
            let spacing = ui.spacing().item_spacing.x;
            let width = ui.available_width() - 32.0 - spacing - 6.0;
            let found = app.settings.screenshots_found.clone();
            ui.horizontal(|ui| {
                let placeholder = found
                    .as_ref()
                    .map_or(String::new(), |found| found.path.display().to_string());
                path_input(
                    ui,
                    &mut app.settings.screenshots,
                    &placeholder,
                    width,
                    "Clear: follow the emulator's setting",
                );
                if theme::icon(ui, icons::FOLDER, "Browse", 32.0).clicked()
                    && let Some(picked) = rfd::FileDialog::new().pick_folder()
                {
                    app.settings.screenshots = picked.display().to_string();
                }
            });
            let typed = path(&app.settings.screenshots);
            let (text, color) = match (&typed, &found) {
                (Some(typed), _) if !typed.is_absolute() => (
                    format!(
                        "{}  Use a full path, such as D:\\Screenshots",
                        icons::MISSING
                    ),
                    palette.danger,
                ),
                (Some(_), _) => (format!("{}  Set here", icons::CHECK), palette.accent),
                (None, Some(found)) => {
                    (format!("{}  {}", icons::CHECK, found.via), palette.success)
                }
                (None, None) => (
                    format!("{}  USERPROFILE is not set", icons::MISSING),
                    palette.danger,
                ),
            };
            ui.label(egui::RichText::new(text).size(12.0).color(color));
            // Created by the first screenshot.
            let effective = typed
                .filter(|typed| typed.is_absolute())
                .or(found.map(|found| found.path));
            if let Some(folder) = effective.filter(|folder| folder.is_dir()) {
                ui.add_space(6.0);
                if theme::secondary(ui, format!("{}  Open folder", icons::FOLDER)).clicked() {
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(&folder)
                        .spawn();
                }
            }
        },
    );
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// `Name <email>`.
const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");

/// Name, version, copyright, links and the third-party parts with their licenses.
fn about(ui: &mut egui::Ui) {
    card(ui, "About", "", |ui| {
        let palette = theme::palette(ui);
        let (name, email) = AUTHOR
            .split_once(" <")
            .map_or((AUTHOR, ""), |(name, email)| {
                (name, email.trim_end_matches('>'))
            });
        ui.horizontal(|ui| {
            theme::app_icon(ui, 60.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::title("Avdeck", 18.0));
                    ui.label(theme::muted(ui, format!("version {VERSION}")));
                });
                ui.label(theme::muted(ui, format!("{DESCRIPTION}.")));
                ui.label(theme::muted(ui, format!("© 2026 {name}")));
            });
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to("GitHub", REPOSITORY);
            ui.label(theme::muted(ui, "·"));
            ui.hyperlink_to(
                "Release notes",
                format!("{REPOSITORY}/releases/tag/v{VERSION}"),
            );
            ui.label(theme::muted(ui, "·"));
            ui.hyperlink_to("Report an issue", format!("{REPOSITORY}/issues"));
            if !email.is_empty() {
                ui.label(theme::muted(ui, "·"));
                ui.hyperlink_to(email, format!("mailto:{email}"));
            }
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Includes")
                .family(theme::semibold())
                .size(13.0),
        );
        // Rows as tall as their text, not as a button (a wrapped row starts at this height).
        ui.spacing_mut().interact_size.y = 0.0;
        ui.spacing_mut().item_spacing.y = 4.0;
        for (part, url, license) in [
            (
                "Inter 4.1 by Rasmus Andersson",
                "https://rsms.me/inter/",
                "SIL Open Font License 1.1",
            ),
            ("Phosphor Icons", "https://phosphoricons.com", "MIT License"),
            (
                "Android Emulator gRPC protos, Android Open Source Project",
                "https://source.android.com",
                "Apache License 2.0",
            ),
        ] {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(egui::Hyperlink::from_label_and_url(
                    egui::RichText::new(part).size(12.5),
                    url,
                ));
                ui.label(egui::RichText::new(license).size(12.5).color(palette.faint));
            });
        }
        ui.label(
            egui::RichText::new("Built with Rust, egui, winit and tonic.")
                .size(12.5)
                .color(palette.faint),
        );
    });
}
