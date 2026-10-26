//! The first-run setup, over the whole window: a welcome, a check of the SDK and its tools (each
//! one run, hardware acceleration included) with a way to point at them, then the main
//! preferences. Settings can run it again.

use crate::app::App;
use crate::icons;
use crate::settings::{self, Pick, ROWS};
use crate::theme;
use avdeck_core::check::{self, Acceleration};
use avdeck_core::sdk::{self, Found, Source, Tools};
use egui::{self, RichText, vec2};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Longest a tool may take to answer.
const LIMIT: Duration = Duration::from_secs(20);
const STUDIO: &str = "https://developer.android.com/studio";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Environment,
    Preferences,
}

const STEPS: [(Step, &str); 3] = [
    (Step::Welcome, "Welcome"),
    (Step::Environment, "Environment"),
    (Step::Preferences, "Preferences"),
];

pub struct Setup {
    step: Step,
    /// The tools last checked; checked again when they change.
    checked: Option<Tools>,
    results: Arc<Mutex<Results>>,
}

impl Setup {
    pub fn new() -> Self {
        Self {
            step: Step::Welcome,
            checked: None,
            results: Default::default(),
        }
    }
}

/// A tool run in the background.
#[derive(Clone, Default)]
enum Probe<T> {
    /// The tool was not found: nothing to run.
    #[default]
    Skipped,
    Running,
    Done(T),
    Failed(String),
}

#[derive(Default)]
struct Results {
    adb: Probe<String>,
    java: Probe<String>,
    acceleration: Probe<Acceleration>,
}

/// Runs the tools of `tools`, each on its own thread.
fn start_checks(ctx: &egui::Context, tools: &Tools) -> Arc<Mutex<Results>> {
    let results = Arc::new(Mutex::new(Results::default()));
    let env = tools.child_env();
    probe(
        ctx,
        &results,
        tools.adb.as_ref().map(|adb| adb.path.clone()),
        &["version"],
        &env,
        |results| &mut results.adb,
        |output| match check::adb_version(&output.text) {
            Some(version) if output.success => Ok(version),
            _ => Err(first_line(&output.text)),
        },
    );
    probe(
        ctx,
        &results,
        tools
            .jdk
            .as_ref()
            .map(|jdk| jdk.path.join("bin").join("java.exe")),
        &["-version"],
        &env,
        |results| &mut results.java,
        |output| match check::java_version(&output.text) {
            Some(version) if output.success => Ok(version),
            _ => Err(first_line(&output.text)),
        },
    );
    probe(
        ctx,
        &results,
        tools
            .emulator
            .as_ref()
            .map(|emulator| emulator.path.clone()),
        &["-accel-check"],
        &env,
        |results| &mut results.acceleration,
        // The check exits with an error when acceleration is missing: read it either way.
        |output| check::acceleration(&output.text).ok_or_else(|| first_line(&output.text)),
    );
    results
}

fn probe<T: Send + 'static>(
    ctx: &egui::Context,
    results: &Arc<Mutex<Results>>,
    program: Option<PathBuf>,
    args: &'static [&'static str],
    env: &[(&'static str, OsString)],
    slot: fn(&mut Results) -> &mut Probe<T>,
    read: fn(&check::Output) -> Result<T, String>,
) {
    let Some(program) = program else {
        return;
    };
    *slot(&mut results.lock().unwrap()) = Probe::Running;
    let (ctx, results, env) = (ctx.clone(), results.clone(), env.to_vec());
    std::thread::spawn(move || {
        let outcome = match check::run(&program, args, &env, LIMIT) {
            Ok(output) => match read(&output) {
                Ok(value) => Probe::Done(value),
                Err(error) => Probe::Failed(error),
            },
            Err(error) => Probe::Failed(error.to_string()),
        };
        *slot(&mut results.lock().unwrap()) = outcome;
        ctx.request_repaint();
    });
}

/// The first line of a tool's output, shortened, to show as its error.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("No output");
    match line.char_indices().nth(140) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

/// Leaves the setup: remembers it was done and goes to the devices, or to Settings while the
/// SDK or the emulator is missing.
fn finish(app: &mut App) {
    app.config.setup_done = true;
    app.save_config();
    app.setup = None;
    app.page = if app.tools.sdk.is_none() || app.tools.emulator.is_none() {
        crate::app::Page::Settings
    } else {
        crate::app::Page::Devices
    };
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(setup) = &mut app.setup else {
        return;
    };
    if setup.checked.as_ref() != Some(&app.tools) {
        setup.checked = Some(app.tools.clone());
        setup.results = start_checks(ui.ctx(), &app.tools);
    }
    let step = setup.step;
    let palette = theme::palette(ui);
    let width = (ui.available_width() - 48.0).clamp(300.0, 720.0);

    ui.add_space(22.0);
    settings::centered(ui, width, |ui| steps(ui, step));
    ui.add_space(14.0);

    let footer = 64.0;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .max_height((ui.available_height() - footer).max(0.0))
        .show(ui, |ui| {
            ui.add_space(10.0);
            settings::centered(ui, width, |ui| match step {
                Step::Welcome => welcome(ui),
                Step::Environment => environment(app, ui),
                Step::Preferences => preferences(app, ui),
            });
            ui.add_space(16.0);
        });

    let line = ui.available_rect_before_wrap();
    ui.painter().hline(
        line.x_range(),
        line.top(),
        egui::Stroke::new(1.0, palette.border),
    );
    ui.add_space(14.0);
    settings::centered(ui, width, |ui| buttons(app, ui, step));
}

/// 1 Welcome — 2 Environment — 3 Preferences, with the current step lit and the done ones
/// ticked.
fn steps(ui: &mut egui::Ui, step: Step) {
    let palette = theme::palette(ui);
    let current = STEPS
        .iter()
        .position(|(each, _)| *each == step)
        .unwrap_or(0);
    ui.horizontal(|ui| {
        for (index, (_, name)) in STEPS.iter().enumerate() {
            if index > 0 {
                let (rect, _) = ui.allocate_exact_size(vec2(36.0, 24.0), egui::Sense::hover());
                ui.painter().hline(
                    rect.x_range().shrink(4.0),
                    rect.center().y,
                    egui::Stroke::new(1.0, palette.border),
                );
            }
            let (fill, text, mark) = match index.cmp(&current) {
                std::cmp::Ordering::Less => {
                    (palette.success, palette.muted, icons::CHECK.to_owned())
                }
                std::cmp::Ordering::Equal => {
                    (palette.accent, palette.text, (index + 1).to_string())
                }
                std::cmp::Ordering::Greater => {
                    (palette.border, palette.faint, (index + 1).to_string())
                }
            };
            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 12.0, fill);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                mark,
                egui::FontId::proportional(12.5),
                if index > current {
                    palette.muted
                } else {
                    palette.on_accent
                },
            );
            let label = RichText::new(*name).size(13.5).color(text);
            ui.label(if index == current {
                label.family(theme::semibold())
            } else {
                label
            });
        }
    });
}

fn welcome(ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    // The icon's tile leaves a margin: 72 points look as large as a 64-point tile.
    ui.add_space(14.0);
    theme::app_icon(ui, 72.0);
    ui.add_space(14.0);
    ui.label(theme::title("Welcome to Avdeck", 28.0));
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "Avdeck runs your Android Virtual Devices in one window: each device in its own \
             tab, with the emulator's toolbar beside its screen.",
        )
        .size(15.0)
        .color(palette.muted),
    );
    ui.add_space(26.0);
    for (glyph, title, text) in [
        (
            icons::DEVICES,
            "Devices in tabs",
            "Start several emulators and switch between them like browser tabs; closing \
             Avdeck can leave them running.",
        ),
        (
            icons::EDIT,
            "Manage your devices",
            "Create, edit, duplicate and wipe devices, download system images and take \
             snapshots.",
        ),
        (
            icons::FOLDER,
            "Your Android SDK",
            "Avdeck uses the SDK and tools already on this PC and changes nothing outside its \
             own settings. Next, it checks them.",
        ),
    ] {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(10),
                palette.accent.gamma_multiply(0.14),
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::proportional(19.0),
                palette.accent,
            );
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(theme::title(title, 14.5));
                ui.label(theme::muted(ui, text));
            });
        });
        ui.add_space(14.0);
    }
}

/// One row of the environment check.
struct Row {
    label: &'static str,
    /// What Avdeck needs it for; `None` when it is required.
    needed_for: Option<&'static str>,
    status: Status,
    /// Path, version, where it was found.
    detail: String,
    /// What is wrong and how to fix it.
    hint: Option<String>,
    /// The settings row it is set with (`settings::ROWS`), if any.
    setting: Option<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Ok,
    Checking,
    Warning,
    Missing,
}

fn environment(app: &mut App, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    ui.add_space(8.0);
    ui.label(theme::title("Check your environment", 24.0));
    ui.add_space(4.0);
    ui.label(theme::muted(
        ui,
        "Avdeck needs the Android SDK and its emulator; the other tools unlock more. Point at \
         anything not found, or at another copy, with the folder button.",
    ));
    ui.add_space(16.0);

    let rows = rows(app);
    let mut action = None;
    theme::card(ui)
        .inner_margin(egui::Margin::symmetric(16, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                if let Some(chosen) = row_ui(app, ui, row) {
                    action = Some(chosen);
                }
            }
        });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if theme::secondary(ui, format!("{}  Check again", icons::REFRESH)).clicked() {
            app.refresh_tools();
            if let Some(setup) = &mut app.setup {
                setup.checked = None;
            }
        }
        let missing = rows
            .iter()
            .filter(|row| row.status == Status::Missing)
            .count();
        let warnings = rows
            .iter()
            .filter(|row| row.status == Status::Warning)
            .count();
        let checking = rows.iter().any(|row| row.status == Status::Checking);
        let (text, color) = if checking {
            ("Checking…".to_owned(), palette.muted)
        } else if missing > 0 {
            (
                "Avdeck cannot start devices until the SDK and the emulator are found.".to_owned(),
                palette.danger,
            )
        } else if warnings > 0 {
            (
                format!(
                    "Ready. {warnings} {} can wait.",
                    if warnings == 1 { "item" } else { "items" }
                ),
                palette.warning,
            )
        } else {
            ("Everything is ready.".to_owned(), palette.success)
        };
        ui.label(RichText::new(text).color(color));
    });
    if rows.iter().any(|row| row.status == Status::Missing) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::muted(ui, "No SDK yet? Install"));
            ui.hyperlink_to("Android Studio", STUDIO);
            ui.label(theme::muted(ui, "and open its SDK Manager once."));
        });
    }

    if let Some((index, path)) = action {
        *settings::field(&mut app.config.tools, index) = path;
        app.config_changed();
    }
}

/// Picks a path for settings row `index`; `None` when cancelled.
fn browse(index: usize) -> Option<PathBuf> {
    let dialog = rfd::FileDialog::new();
    match ROWS[index].2 {
        Pick::Folder => dialog.pick_folder(),
        Pick::File => dialog.add_filter("Program", &["exe"]).pick_file(),
    }
}

/// Draws `row`; returns the path to set for its settings row (`None` inside: clear it) when the
/// user picks or clears one.
fn row_ui(app: &App, ui: &mut egui::Ui, row: &Row) -> Option<(usize, Option<PathBuf>)> {
    let palette = theme::palette(ui);
    let mut action = None;
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
        let (glyph, color) = match row.status {
            Status::Ok => (icons::CHECK, palette.success),
            Status::Warning => (icons::WARNING, palette.warning),
            Status::Missing => (icons::MISSING, palette.danger),
            Status::Checking => ("", palette.muted),
        };
        if row.status == Status::Checking {
            ui.put(rect, egui::Spinner::new().size(15.0));
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::proportional(18.0),
                color,
            );
        }
        // Buttons on the right, then the text in what is left.
        let buttons = if row.setting.is_some() { 72.0 } else { 0.0 };
        let text_width = (ui.available_width() - buttons).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(text_width);
            // One label: a horizontal row would be as tall as a button.
            let mut title = egui::text::LayoutJob::default();
            let style = ui.style().clone();
            theme::title(row.label, 14.0).color(palette.text).append_to(
                &mut title,
                &style,
                egui::FontSelection::Default,
                egui::Align::Center,
            );
            RichText::new(format!("   {}", row.needed_for.unwrap_or("Required")))
                .size(12.0)
                .color(palette.faint)
                .append_to(
                    &mut title,
                    &style,
                    egui::FontSelection::Default,
                    egui::Align::Center,
                );
            ui.label(title);
            ui.add(
                egui::Label::new(RichText::new(&row.detail).size(12.5).color(palette.muted))
                    .truncate(),
            )
            .on_hover_text(&row.detail);
            if let Some(hint) = &row.hint {
                ui.label(RichText::new(hint).size(12.5).color(color));
            }
        });
        if let Some(index) = row.setting {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if theme::icon(ui, icons::FOLDER, "Choose…", 30.0).clicked()
                    && let Some(path) = browse(index)
                {
                    action = Some((index, Some(path)));
                }
                let set = settings::field(&mut app.config.tools.clone(), index).is_some();
                if set
                    && theme::icon(ui, icons::CLOSE, "Use what is found automatically", 30.0)
                        .clicked()
                {
                    action = Some((index, None));
                }
            });
        }
    });
    ui.add_space(6.0);
    action
}

/// `path · version · where from`, skipping what is unknown. Where from is the variable the
/// path starts at (`%ANDROID_HOME%`, `PATH`): the path itself is already shown.
fn detail(found: &Found, version: Option<String>) -> String {
    let mut parts = vec![found.path.display().to_string()];
    parts.extend(version);
    let root = found.via.split('\\').next().unwrap_or_default();
    match found.source {
        Source::Settings => parts.push("set in Avdeck".to_owned()),
        _ if root.starts_with('%') || root == "PATH" => parts.push(format!("from {root}")),
        _ => {}
    }
    parts.join("  ·  ")
}

fn rows(app: &App) -> Vec<Row> {
    let tools = &app.tools;
    let results = app
        .setup
        .as_ref()
        .map(|setup| setup.results.clone())
        .unwrap_or_default();
    let results = results.lock().unwrap();
    let overrides = &app.config.tools;
    // A path set in Avdeck that is not what was found: it was rejected.
    let rejected = |index: usize, found: &Option<Found>| {
        settings::field(&mut overrides.clone(), index)
            .as_ref()
            .filter(|_| {
                found
                    .as_ref()
                    .is_none_or(|found| found.source != Source::Settings)
            })
            .map(|path| format!("{} is not valid here. ", path.display()))
            .unwrap_or_default()
    };
    // Found and run: its version, or why it does not run.
    let ran = |probe: &Probe<String>| match probe {
        Probe::Done(version) => (Status::Ok, Some(version.clone()), None),
        Probe::Running | Probe::Skipped => (Status::Checking, None, None),
        Probe::Failed(error) => (
            Status::Warning,
            None,
            Some(format!("Found, but it does not run: {error}")),
        ),
    };
    let row = |label, needed_for, setting: Option<usize>, found: &Option<Found>| Row {
        label,
        needed_for,
        status: Status::Ok,
        detail: found
            .as_ref()
            .map_or("Not found".to_owned(), |found| detail(found, None)),
        hint: None,
        setting,
    };
    let mut rows = Vec::new();

    let mut sdk = row("Android SDK", None, Some(0), &tools.sdk);
    if tools.sdk.is_none() {
        sdk.status = Status::Missing;
        sdk.hint = Some(format!(
            "{}Install it with Android Studio, or choose the folder that holds platform-tools \
             and emulator.",
            rejected(0, &tools.sdk)
        ));
    }
    rows.push(sdk);

    let mut emulator = row("Emulator", None, Some(1), &tools.emulator);
    match &tools.emulator {
        Some(found) => {
            emulator.detail = detail(found, sdk::emulator_version(&found.path));
        }
        None => {
            emulator.status = Status::Missing;
            emulator.hint = Some(format!(
                "{}Install \"Android Emulator\" in Android Studio's SDK Manager.",
                rejected(1, &tools.emulator)
            ));
        }
    }
    rows.push(emulator);

    let mut acceleration = Row {
        label: "Hardware acceleration",
        needed_for: Some("Needed for usable speed"),
        status: Status::Checking,
        detail: "Checked by the emulator".to_owned(),
        hint: None,
        setting: None,
    };
    match (&tools.emulator, &results.acceleration) {
        (None, _) => {
            acceleration.status = Status::Missing;
            acceleration.detail = "Needs the emulator".to_owned();
        }
        (_, Probe::Done(check)) if check.ready => {
            acceleration.status = Status::Ok;
            acceleration.detail = check.message.clone();
        }
        (_, Probe::Done(check)) => {
            acceleration.status = Status::Warning;
            acceleration.detail = check.message.clone();
            acceleration.hint = Some(
                "Turn on \"Windows Hypervisor Platform\" in Windows Features, restart, then \
                 check again. Without it devices run very slowly."
                    .to_owned(),
            );
        }
        (_, Probe::Failed(error)) => {
            acceleration.status = Status::Warning;
            acceleration.hint = Some(format!("The emulator did not answer: {error}"));
        }
        _ => {}
    }
    rows.push(acceleration);

    let mut adb = row(
        "adb",
        Some("Sends files and APKs to devices"),
        Some(2),
        &tools.adb,
    );
    match &tools.adb {
        Some(found) => {
            let (status, version, hint) = ran(&results.adb);
            (adb.status, adb.hint) = (status, hint);
            adb.detail = detail(found, version);
        }
        None => {
            adb.status = Status::Warning;
            adb.hint = Some(format!(
                "{}Install \"Android SDK Platform-Tools\" in Android Studio's SDK Manager.",
                rejected(2, &tools.adb)
            ));
        }
    }
    rows.push(adb);

    let mut cmdline = row(
        "Command-line tools",
        Some("Create devices, download system images"),
        Some(3),
        &tools.cmdline_tools,
    );
    match &tools.cmdline_tools {
        Some(found) => cmdline.detail = detail(found, sdk::cmdline_tools_version(&found.path)),
        None => {
            cmdline.status = Status::Warning;
            cmdline.hint = Some(format!(
                "{}Install \"Android SDK Command-line Tools\" in Android Studio's SDK Manager.",
                rejected(3, &tools.cmdline_tools)
            ));
        }
    }
    rows.push(cmdline);

    let mut avd_home = row(
        "AVD folder",
        Some("Where devices live"),
        Some(4),
        &tools.avd_home,
    );
    if tools.avd_home.is_none() {
        avd_home.status = Status::Warning;
        avd_home.hint = Some(format!(
            "{}Created with the first device.",
            rejected(4, &tools.avd_home)
        ));
    }
    rows.push(avd_home);

    let mut jdk = row(
        "JDK",
        Some("Runs the command-line tools"),
        Some(5),
        &tools.jdk,
    );
    match &tools.jdk {
        Some(found) => {
            let (status, version, hint) = ran(&results.java);
            (jdk.status, jdk.hint) = (status, hint);
            jdk.detail = detail(found, version);
        }
        None => {
            jdk.status = Status::Warning;
            jdk.hint = Some(format!(
                "{}Android Studio's own JDK works: choose its jbr folder.",
                rejected(5, &tools.jdk)
            ));
        }
    }
    rows.push(jdk);
    rows
}

fn preferences(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.label(theme::title("Preferences", 24.0));
    ui.add_space(4.0);
    ui.label(theme::muted(
        ui,
        "You can change these any time in Settings.",
    ));
    ui.add_space(16.0);
    let before = app.settings.appearance();
    let width = ui.available_width();
    if width >= 600.0 {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let half = (width - 16.0) / 2.0;
            ui.vertical(|ui| {
                ui.set_width(half);
                settings::appearance(app, ui);
                settings::starting(app, ui);
            });
            ui.vertical(|ui| {
                ui.set_width(half);
                settings::closing(app, ui);
            });
        });
    } else {
        settings::appearance(app, ui);
        settings::starting(app, ui);
        settings::closing(app, ui);
    }
    // Show the chosen look at once.
    let after = app.settings.appearance();
    if after != before {
        theme::set_appearance(ui.ctx(), after);
    }
}

fn buttons(app: &mut App, ui: &mut egui::Ui, step: Step) {
    let ready = app.tools.sdk.is_some() && app.tools.emulator.is_some();
    let mut go = None;
    ui.horizontal(|ui| {
        match step {
            Step::Welcome => {
                if ui
                    .add(egui::Button::new(theme::muted(ui, "Skip setup")).frame(false))
                    .clicked()
                {
                    finish(app);
                    return;
                }
            }
            Step::Environment => {
                if theme::secondary(ui, "Back").clicked() {
                    go = Some(Step::Welcome);
                }
            }
            Step::Preferences => {
                if theme::secondary(ui, "Back").clicked() {
                    go = Some(Step::Environment);
                }
            }
        }
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| match step {
                Step::Welcome => {
                    if theme::primary(ui, "Get started").clicked() {
                        go = Some(Step::Environment);
                    }
                }
                Step::Environment => {
                    let next = ui
                        .add_enabled_ui(ready, |ui| theme::primary(ui, "Continue"))
                        .inner;
                    if next.clicked() {
                        go = Some(Step::Preferences);
                    }
                    if !ready && theme::secondary(ui, "Continue anyway").clicked() {
                        go = Some(Step::Preferences);
                    }
                }
                Step::Preferences => {
                    if theme::primary(ui, format!("{}  Finish", icons::CHECK)).clicked() {
                        app.settings.apply(&mut app.config);
                        theme::set_appearance(ui.ctx(), app.config.appearance);
                        finish(app);
                    }
                }
            },
        );
    });
    if let (Some(step), Some(setup)) = (go, &mut app.setup) {
        setup.step = step;
    }
}
