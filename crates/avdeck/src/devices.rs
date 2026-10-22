//! The Devices page: a grid of cards, one per AVD, with its shape, state and actions.

use crate::app::{App, Dialog};
use crate::icons;
use crate::manage;
use crate::session::Phase;
use crate::theme;
use avdeck_core::avd::Avd;
use avdeck_core::launch;
use eframe::egui::{self, Color32, RichText, Stroke, vec2};

enum Status {
    /// Shown in a tab of this window.
    Tab(Phase),
    /// Running headless, attachable over gRPC.
    Headless,
    /// Running without gRPC (started elsewhere): can only be stopped.
    Outside,
    /// Asked to stop, still running.
    Stopping,
    /// Asked to stop 40 s ago and still running: it no longer answers.
    NotResponding,
    Stopped,
}

impl Status {
    fn running(&self) -> bool {
        !matches!(
            self,
            Status::Stopped
                | Status::Tab(Phase::Ended(_))
                | Status::Stopping
                | Status::NotResponding
        )
    }
}

fn status(app: &App, avd: &Avd) -> Status {
    if let Some(stopping) = app.stopping.get(&avd.id) {
        if stopping.since.elapsed().as_secs() >= 40 {
            Status::NotResponding
        } else {
            Status::Stopping
        }
    } else if let Some(tab) = app.tab(&avd.id) {
        Status::Tab(tab.phase())
    } else if app.live_grpc(&avd.id).is_some() {
        Status::Headless
    } else if app.live_any(&avd.id).is_some() {
        Status::Outside
    } else {
        Status::Stopped
    }
}

const CARD_MIN: f32 = 230.0;
const CARD_HEIGHT: f32 = 318.0;
const GAP: f32 = 16.0;

pub fn show(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(28, 22))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    theme::heading(ui, "Devices");
                    let count = app.avds.len();
                    let place = app.tools.avd_home.as_ref().map_or_else(
                        || "no AVD folder".to_owned(),
                        |home| home.path.display().to_string(),
                    );
                    let plural = if count == 1 { "" } else { "s" };
                    ui.label(theme::muted(
                        ui,
                        format!("{count} device{plural}  ·  {place}"),
                    ));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::primary(ui, format!("{}  Create device", icons::ADD)).clicked() {
                        app.dialog = Some(Dialog::Manage(Box::new(manage::Dialog::create(app))));
                    }
                    if theme::icon(ui, icons::REFRESH, "Refresh", 32.0).clicked() {
                        app.rescan(true);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut app.search)
                            .hint_text(format!("{}  Search", icons::SEARCH))
                            .desired_width(200.0)
                            .margin(egui::Margin::symmetric(10, 7)),
                    );
                });
            });
            ui.add_space(18.0);

            let query = app.search.trim().to_lowercase();
            let avds: Vec<Avd> = app
                .avds
                .iter()
                .filter(|avd| {
                    query.is_empty()
                        || avd.name.to_lowercase().contains(&query)
                        || avd.id.to_lowercase().contains(&query)
                        || avd
                            .device
                            .as_ref()
                            .is_some_and(|device| device.to_lowercase().contains(&query))
                })
                .cloned()
                .collect();
            if avds.is_empty() {
                empty(app, ui, query.is_empty());
                return;
            }

            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    let width = ui.available_width();
                    let columns = (((width + GAP) / (CARD_MIN + GAP)).floor() as usize).max(1);
                    let card_width = (width - GAP * (columns - 1) as f32) / columns as f32;
                    for row in avds.chunks(columns) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = GAP;
                            for avd in row {
                                ui.allocate_ui_with_layout(
                                    vec2(card_width, CARD_HEIGHT),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| card(app, ui, ctx, avd, card_width),
                                );
                            }
                        });
                        ui.add_space(GAP);
                    }
                });
        });
}

fn empty(app: &mut App, ui: &mut egui::Ui, nothing_at_all: bool) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icons::DEVICES)
                .size(48.0)
                .color(theme::palette(ui).faint),
        );
        ui.add_space(8.0);
        if nothing_at_all {
            ui.label(theme::title("No devices yet", 18.0));
            ui.label(theme::muted(
                ui,
                "Create one, or point Settings at the folder that holds your AVDs.",
            ));
            ui.add_space(12.0);
            if theme::primary(ui, format!("{}  Create device", icons::ADD)).clicked() {
                app.dialog = Some(Dialog::Manage(Box::new(manage::Dialog::create(app))));
            }
        } else {
            ui.label(theme::title("Nothing matches", 18.0));
        }
    });
}

fn card(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, avd: &Avd, width: f32) {
    let palette = theme::palette(ui);
    let state = status(app, avd);
    let hovered = ui.rect_contains_pointer(egui::Rect::from_min_size(
        ui.cursor().min,
        vec2(width, CARD_HEIGHT),
    ));
    let frame = theme::card(ui).stroke(Stroke::new(
        1.0,
        if hovered {
            palette.faint
        } else {
            palette.border
        },
    ));
    frame.show(ui, |ui| {
        ui.set_width(width - 30.0);
        ui.set_height(CARD_HEIGHT - 30.0);

        // The device at its real shape; a live thumbnail while its tab shows it.
        let (preview, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), 150.0), egui::Sense::hover());
        let tab = app.tab(&avd.id);
        let shape = tab.map_or(avd.panel.unwrap_or((1080, 2400)), |tab| tab.device_shape());
        let texture = tab.and_then(|tab| tab.texture_id());
        theme::device(ui, preview, shape, texture, state.running());
        ui.add_space(10.0);

        ui.label(theme::title(&avd.name, 15.0))
            .on_hover_text(&avd.id);
        let mut details = Vec::new();
        if let Some(api) = &avd.api {
            details.push(format!("API {api}"));
        }
        if let Some((width, height)) = avd.panel {
            details.push(format!("{width}×{height}"));
        }
        if let Some(ram) = avd.ram_mb {
            details.push(format!("{} GB", ram as f32 / 1024.0));
        }
        ui.label(
            RichText::new(details.join("  ·  "))
                .size(12.5)
                .color(palette.muted),
        );
        ui.add_space(4.0);

        let (text, color) = match &state {
            Status::Tab(Phase::Starting) => ("Starting", palette.warning),
            Status::Tab(Phase::Running) => ("Open in a tab", palette.success),
            Status::Tab(Phase::Ended(_)) => ("Disconnected", palette.danger),
            Status::Headless => ("Running in background", palette.success),
            Status::Outside => ("Running outside Avdeck", palette.accent),
            Status::Stopping => ("Stopping…", palette.warning),
            Status::NotResponding => ("Not responding", palette.danger),
            Status::Stopped => ("Stopped", palette.faint),
        };
        let chip = theme::chip(ui, color, text);
        if matches!(state, Status::Outside) {
            chip.on_hover_text(
                "Started without gRPC (Android Studio or a script), so Avdeck cannot show it. \
                 Stop it here and start it again from Avdeck.",
            );
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                if matches!(state, Status::NotResponding)
                    && theme::danger(ui, "Force stop")
                        .on_hover_text("End the emulator process now; unsaved state is lost")
                        .clicked()
                {
                    app.force_stop(&avd.id);
                }
                if matches!(state, Status::Stopping) {
                    ui.spinner();
                }
                let main = match state {
                    Status::Tab(_) => Some("Show".to_owned()),
                    Status::Headless => Some("Open".to_owned()),
                    Status::Outside | Status::Stopping | Status::NotResponding => None,
                    Status::Stopped => Some(format!("{}  Start", icons::START)),
                };
                if let Some(label) = main
                    && theme::primary(ui, label).clicked()
                {
                    app.open(ctx, &avd.id);
                }
                // A tab that lost its emulator while the process lives on (hung): still stoppable.
                let alive = app.live_any(&avd.id).is_some();
                if (state.running() || matches!(state, Status::Outside) || alive)
                    && !matches!(state, Status::Stopping | Status::NotResponding)
                    && theme::secondary(ui, format!("{}  Stop", icons::STOP))
                        .on_hover_text("Stop and save the device state")
                        .clicked()
                {
                    app.stop(&avd.id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let more = theme::icon(ui, icons::MORE, "More", 32.0);
                    let running = !matches!(state, Status::Stopped | Status::NotResponding);
                    egui::Popup::menu(&more).show(|ui| {
                        ui.set_min_width(210.0);
                        menu(app, ui, ctx, avd, running);
                    });
                });
            });
        });
    });
}

fn item(ui: &mut egui::Ui, glyph: &str, text: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(format!("{glyph}   {text}"))
            .frame(false)
            .min_size(vec2(ui.available_width(), 28.0)),
    )
    .clicked()
}

type Action = Box<dyn FnOnce(&mut App)>;

fn open_dialog(dialog: manage::Dialog) -> Action {
    Box::new(move |app| app.dialog = Some(Dialog::Manage(Box::new(dialog))))
}

fn menu(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, avd: &Avd, running: bool) {
    let base = app.config.launch.clone();
    let danger = theme::palette(ui).danger;
    let mut chosen: Option<Action> = None;
    if item(ui, icons::START, "Cold boot", !running) {
        let options = launch::Options {
            cold_boot: true,
            ..base.clone()
        };
        let (ctx, id) = (ctx.clone(), avd.id.clone());
        chosen = Some(Box::new(move |app| app.start(&ctx, &id, &options)));
    }
    if item(ui, icons::START, "Start with options…", !running) {
        chosen = Some(open_dialog(manage::Dialog::start(avd, &base)));
    }
    if item(ui, icons::SNAPSHOT, "Snapshots…", running) {
        chosen = Some(open_dialog(manage::Dialog::snapshots(app, avd)));
    }
    ui.separator();
    if item(ui, icons::EDIT, "Edit…", !running) {
        chosen = Some(open_dialog(manage::Dialog::edit(avd)));
    }
    if item(ui, icons::DUPLICATE, "Duplicate…", !running) {
        chosen = Some(open_dialog(manage::Dialog::duplicate(avd)));
    }
    if item(ui, icons::FOLDER, "Show in Explorer", true) {
        let _ = std::process::Command::new("explorer.exe")
            .arg(&avd.dir)
            .spawn();
        ui.close();
    }
    ui.separator();
    ui.scope(|ui| {
        if !running {
            ui.visuals_mut().override_text_color = Some(danger);
        }
        if item(ui, icons::WARNING, "Wipe data…", !running) {
            chosen = Some(open_dialog(manage::Dialog::wipe(avd)));
        }
        if item(ui, icons::DELETE, "Delete…", !running) {
            chosen = Some(open_dialog(manage::Dialog::delete(avd)));
        }
    });
    if let Some(action) = chosen {
        action(app);
        ui.close();
    }
    let _: Color32 = danger;
}
