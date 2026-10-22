//! The Settings page: SDK and tool paths (when the environment has none, or to use others),
//! how emulators start, and what closing does.

use crate::app::App;
use crate::icons;
use crate::theme;
use avdeck_core::config::{CloseAction, Config};
use avdeck_core::launch::GPU_MODES;
use avdeck_core::sdk::{self, Found, Source};
use eframe::egui;
use std::path::PathBuf;

/// What the page edits, applied with Save.
pub struct SettingsForm {
    paths: [String; 6],
    gpu: Option<String>,
    no_snapshot_save: bool,
    close_tab: CloseAction,
    close_app: CloseAction,
    /// When Save was last pressed, to confirm it for a moment.
    saved: Option<std::time::Instant>,
}

/// Rows: label, what the browse button picks, and what the path must contain.
const ROWS: [(&str, Pick, &str); 6] = [
    (
        "Android SDK",
        Pick::Folder,
        "folder with platform-tools or emulator",
    ),
    ("Emulator", Pick::File, "emulator.exe"),
    ("adb", Pick::File, "adb.exe"),
    (
        "Command-line tools",
        Pick::Folder,
        "folder with avdmanager.bat and sdkmanager.bat",
    ),
    (
        "AVD folder",
        Pick::Folder,
        "where the .ini and .avd of each device live",
    ),
    (
        "JDK",
        Pick::Folder,
        "JDK home with bin\\java.exe (for avdmanager and sdkmanager)",
    ),
];

#[derive(Clone, Copy)]
enum Pick {
    Folder,
    File,
}

fn text(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn path(text: &str) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"');
    (!text.is_empty()).then(|| PathBuf::from(text))
}

impl From<&Config> for SettingsForm {
    fn from(config: &Config) -> Self {
        let tools = &config.tools;
        Self {
            paths: [
                text(&tools.sdk),
                text(&tools.emulator),
                text(&tools.adb),
                text(&tools.cmdline_tools),
                text(&tools.avd_home),
                text(&tools.jdk),
            ],
            gpu: config.launch.gpu.clone(),
            no_snapshot_save: config.launch.no_snapshot_save,
            close_tab: config.close_tab,
            close_app: config.close_app,
            saved: None,
        }
    }
}

impl SettingsForm {
    fn apply(&self, config: &mut Config) {
        let [sdk, emulator, adb, cmdline_tools, avd_home, jdk] = &self.paths;
        config.tools = sdk::Overrides {
            sdk: path(sdk),
            emulator: path(emulator),
            adb: path(adb),
            cmdline_tools: path(cmdline_tools),
            avd_home: path(avd_home),
            jdk: path(jdk),
        };
        config.launch.gpu = self.gpu.clone();
        config.launch.no_snapshot_save = self.no_snapshot_save;
        config.close_tab = self.close_tab;
        config.close_app = self.close_app;
    }
}

fn source(source: &Source) -> String {
    match source {
        Source::Settings => "set here".to_owned(),
        Source::Environment(name) => format!("from %{name}%"),
        Source::Path => "from PATH".to_owned(),
        Source::Default => "found automatically".to_owned(),
    }
}

fn close_action(ui: &mut egui::Ui, id: &str, value: &mut CloseAction) {
    let name = |action: CloseAction| match action {
        CloseAction::Ask => "Ask every time",
        CloseAction::Stop => "Stop the emulator",
        CloseAction::KeepRunning => "Keep it running",
    };
    egui::ComboBox::new(id, "")
        .selected_text(name(*value))
        .show_ui(ui, |ui| {
            for action in [
                CloseAction::Ask,
                CloseAction::Stop,
                CloseAction::KeepRunning,
            ] {
                ui.selectable_value(value, action, name(action));
            }
        });
}

fn section(ui: &mut egui::Ui, title: &str, description: &str, content: impl FnOnce(&mut egui::Ui)) {
    theme::card(ui)
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::title(title, 15.0));
            if !description.is_empty() {
                ui.label(theme::muted(ui, description));
            }
            ui.add_space(8.0);
            content(ui);
        });
    ui.add_space(14.0);
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // A readable column, centered in wide windows.
        let width = ui.available_width().min(760.0);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        egui::Frame::NONE
            .inner_margin(egui::Margin {
                left: (margin + 28.0).min(120.0) as i8,
                right: 28,
                top: 22,
                bottom: 22,
            })
            .show(ui, |ui| {
                ui.set_max_width(width);
                theme::heading(ui, "Settings");
                ui.label(theme::muted(
                    ui,
                    Config::path().map_or(String::new(), |path| path.display().to_string()),
                ));
                ui.add_space(16.0);

                let tools = app.tools.clone();
                let found: [&Option<Found>; 6] = [
                    &tools.sdk,
                    &tools.emulator,
                    &tools.adb,
                    &tools.cmdline_tools,
                    &tools.avd_home,
                    &tools.jdk,
                ];
                section(
                    ui,
                    "Android SDK and tools",
                    "Leave a field empty to use the environment (ANDROID_HOME, ANDROID_AVD_HOME, \
                     JAVA_HOME, PATH) or the usual locations.",
                    |ui| {
                        for (index, (label, pick, hint)) in ROWS.iter().enumerate() {
                            ui.label(egui::RichText::new(*label).family(theme::semibold()).size(13.0));
                            ui.horizontal(|ui| {
                                let field_width = ui.available_width() - 44.0;
                                ui.add(
                                    egui::TextEdit::singleline(&mut app.settings.paths[index])
                                        .hint_text(*hint)
                                        .desired_width(field_width)
                                        .margin(egui::Margin::symmetric(10, 7)),
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
                            let (text, color) = match found[index] {
                                Some(found) => {
                                    let mut text = format!(
                                        "{}  {}  ·  {}",
                                        icons::CHECK,
                                        found.path.display(),
                                        source(&found.source)
                                    );
                                    if index == 1
                                        && let Some(version) = sdk::emulator_version(&found.path)
                                    {
                                        text.push_str(&format!("  ·  version {version}"));
                                    }
                                    (text, palette.success)
                                }
                                None => (format!("{}  Not found", icons::MISSING), palette.danger),
                            };
                            ui.label(egui::RichText::new(text).size(12.0).color(color));
                            ui.add_space(8.0);
                        }
                    },
                );

                section(
                    ui,
                    "Starting devices",
                    "Defaults for Start; \"Start with options…\" overrides them once.",
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Graphics");
                            egui::ComboBox::new("gpu", "")
                                .selected_text(app.settings.gpu.as_deref().unwrap_or("host"))
                                .show_ui(ui, |ui| {
                                    for mode in GPU_MODES {
                                        ui.selectable_value(
                                            &mut app.settings.gpu,
                                            Some(mode.to_owned()),
                                            mode,
                                        );
                                    }
                                });
                            ui.label(theme::muted(ui, "host uses the GPU; snapshots saved with it need it."));
                        });
                        ui.checkbox(
                            &mut app.settings.no_snapshot_save,
                            "Don't save the device state when it stops",
                        );
                    },
                );

                section(ui, "Closing", "", |ui| {
                    egui::Grid::new("closing")
                        .num_columns(2)
                        .spacing([16.0, 10.0])
                        .show(ui, |ui| {
                            ui.label("Closing a device tab");
                            close_action(ui, "close-tab", &mut app.settings.close_tab);
                            ui.end_row();
                            ui.label("Closing Avdeck");
                            close_action(ui, "close-app", &mut app.settings.close_app);
                            ui.end_row();
                        });
                });

                ui.horizontal(|ui| {
                    if theme::primary(ui, format!("{}  Save", icons::CHECK)).clicked() {
                        app.settings.apply(&mut app.config);
                        app.save_config();
                        app.tools = sdk::resolve(&app.config.tools, &sdk::System);
                        app.rescan(true);
                        app.settings.saved = Some(std::time::Instant::now());
                    }
                    if theme::secondary(ui, "Revert").clicked() {
                        app.settings = SettingsForm::from(&app.config);
                    }
                    if app
                        .settings
                        .saved
                        .is_some_and(|at| at.elapsed().as_secs() < 3)
                    {
                        ui.label(egui::RichText::new(format!("{}  Saved", icons::CHECK)).color(palette.success));
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                    }
                });
            });
    });
}
