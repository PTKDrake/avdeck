//! The Avdeck window: a tab bar (Devices, one tab per emulator, Settings), the pages, window
//! sizing, and starting, attaching and stopping emulators.

use crate::extended;
use crate::icons;
use crate::session::Phase;
use crate::settings::SettingsForm;
use crate::tab::{EmulatorTab, TOOLBAR_WIDTH};
use crate::theme;
use crate::window;
use avdeck_core::avd::{self, Avd};
use avdeck_core::config::{CloseAction, Config, WindowPosition};
use avdeck_core::display;
use avdeck_core::running::{self, Running};
use avdeck_core::sdk::{self, Tools};
use avdeck_core::{console, launch};
use egui::{self, Color32, RichText, pos2, vec2};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Devices,
    Settings,
    /// The tab of this AVD id.
    Emulator(String),
}

pub enum Dialog {
    CloseTab(String),
    CloseApp,
    Error(String),
    /// Device management dialogs (create, edit, delete, ...), see `manage`.
    Manage(Box<crate::manage::Dialog>),
}

/// What the window was last sized for; a change resizes it.
#[derive(Clone, Debug, PartialEq)]
enum Layout {
    Page {
        pixels_per_point: u32,
    },
    Emulator {
        id: String,
        sideways: bool,
        pixels_per_point: u32,
    },
}

/// Narrowest window for a device tab: the tab bar and dialogs still fit.
const MIN_WINDOW_WIDTH: f32 = 280.0;

/// Shortest time between two frames. Every animation repaints the whole window, and presenting
/// does not always wait for vsync: on a Radeon 780M (Vulkan and DX12 alike) a hover fade ran at
/// ~1000 fps and kept the GPU two-thirds busy, against under a tenth at 60 fps. Emulator screens
/// update at 60 fps at most.
const FRAME_TIME: Duration = Duration::from_micros(1_000_000 / 60);

/// An emulator asked to stop, until its process is gone.
pub struct Stopping {
    pub pid: u32,
    pub since: Instant,
}

#[derive(Default)]
struct Fit {
    layout: Option<Layout>,
    /// A size was requested and has not arrived yet: don't learn the size from the window.
    pending: Option<(egui::Vec2, Instant)>,
    /// Inner size of the Devices and Settings pages.
    page_size: Option<egui::Vec2>,
    placed: bool,
}

pub struct App {
    runtime: tokio::runtime::Runtime,
    pub config: Config,
    pub tools: Tools,
    pub avds: Vec<Avd>,
    /// Running emulators, from their discovery files.
    pub live: Vec<Running>,
    scanned_live: Option<Instant>,
    scanned_avds: Option<Instant>,
    pub tabs: Vec<EmulatorTab>,
    pub page: Page,
    window: Option<isize>,
    fit: Fit,
    pub dialog: Option<Dialog>,
    pub settings: SettingsForm,
    /// Filter on the Devices page.
    pub search: String,
    /// Emulators asked to stop, by AVD id.
    pub stopping: std::collections::HashMap<String, Stopping>,
    /// AVDs to start as soon as their previous process is gone.
    start_when_stopped: std::collections::HashSet<String>,
    /// The page the tab bar showed last frame, to scroll a newly selected tab into view.
    shown_page: Option<Page>,
    /// Remember the choice in the close dialogs.
    remember: bool,
    /// Ctrl+Shift+D: DPI, sizes and frame rate over the screen.
    debug: bool,
    /// The user confirmed closing; let the window close.
    closing: bool,
    /// When the last frame started, to pace frames to `FRAME_TIME`.
    last_frame: Option<Instant>,
}

impl App {
    pub fn new(ctx: &egui::Context, request: crate::instance::Request) -> Self {
        let config = Config::load();
        let tools = sdk::resolve(&config.tools, &sdk::System);
        let settings = SettingsForm::new(&config);
        theme::set_appearance(ctx, config.appearance);
        let mut app = Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime"),
            page: Page::Devices,
            config,
            tools,
            avds: Vec::new(),
            live: Vec::new(),
            scanned_live: None,
            scanned_avds: None,
            tabs: Vec::new(),
            window: None,
            fit: Fit::default(),
            dialog: None,
            settings,
            search: String::new(),
            stopping: Default::default(),
            start_when_stopped: Default::default(),
            shown_page: None,
            remember: false,
            debug: false,
            closing: false,
            last_frame: None,
        };
        app.rescan(true);
        if app.tools.sdk.is_none() || app.tools.emulator.is_none() {
            app.page = Page::Settings;
        }
        // Tabs open when Avdeck last closed, if their emulators still run.
        for id in app.config.tabs.clone() {
            if app.live_grpc(&id).is_some() {
                app.open(ctx, &id);
            }
        }
        app.handle_request(ctx, &request);
        app
    }

    /// Re-reads running emulators (every second) and AVDs (every 5 s, or `force`).
    pub fn rescan(&mut self, force: bool) {
        let due = |at: Option<Instant>, every: u64| {
            at.is_none_or(|at| at.elapsed() > Duration::from_secs(every))
        };
        if force || due(self.scanned_live, 1) {
            self.live = running::live();
            self.scanned_live = Some(Instant::now());
            self.stopping
                .retain(|_, stopping| running::alive(stopping.pid));
        }
        if force || due(self.scanned_avds, 5) {
            self.avds = self
                .tools
                .avd_home
                .as_ref()
                .map(|home| avd::list(&home.path))
                .unwrap_or_default();
            self.scanned_avds = Some(Instant::now());
        }
    }

    pub fn runtime(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    pub fn tab(&self, id: &str) -> Option<&EmulatorTab> {
        self.tabs.iter().find(|tab| tab.avd.id == id)
    }

    fn tab_mut(&mut self, id: &str) -> Option<&mut EmulatorTab> {
        self.tabs.iter_mut().find(|tab| tab.avd.id == id)
    }

    /// The running emulator of `id` that serves gRPC.
    pub fn live_grpc(&self, id: &str) -> Option<&Running> {
        self.live
            .iter()
            .filter(|running| running.avd_id == id && running.grpc_port.is_some())
            .max_by_key(|running| running.pid)
    }

    pub fn live_any(&self, id: &str) -> Option<&Running> {
        self.live
            .iter()
            .filter(|running| running.avd_id == id)
            .max_by_key(|running| running.pid)
    }

    /// A command-line request: open its AVD, starting it with the requested options if needed.
    fn handle_request(&mut self, ctx: &egui::Context, request: &crate::instance::Request) {
        let Some(id) = &request.avd else { return };
        // A tab whose emulator has exited starts it again, with this request's options; a hung
        // one only shows, it offers to force a restart itself.
        let exited = self.tab(id).is_some_and(|tab| {
            matches!(tab.phase(), Phase::Ended(_)) && !tab.pid().is_some_and(running::alive)
        });
        if exited {
            self.close_tab(id);
            self.rescan(true);
        }
        if self.tab(id).is_some() || self.live_grpc(id).is_some() {
            self.open(ctx, id);
        } else {
            let options = request.options(&self.config.launch);
            self.start(ctx, id, &options);
        }
    }

    /// Shows `id`: its tab if open, else attaches to it if it runs, else starts it.
    pub fn open(&mut self, ctx: &egui::Context, id: &str) {
        if self.tab(id).is_some() {
            self.page = Page::Emulator(id.to_owned());
            return;
        }
        if let Some(port) = self.live_grpc(id).and_then(|running| running.grpc_port) {
            self.attach(ctx, id, port);
        } else {
            let options = self.config.launch.clone();
            self.start(ctx, id, &options);
        }
    }

    fn attach(&mut self, ctx: &egui::Context, id: &str, port: u16) {
        let Some(avd) = self.avds.iter().find(|avd| avd.id == id).cloned() else {
            self.dialog = Some(Dialog::Error(format!("No AVD named {id}.")));
            return;
        };
        let tab = EmulatorTab::new(self.runtime.handle(), avd, port, None, None, ctx);
        self.tabs.push(tab);
        self.page = Page::Emulator(id.to_owned());
    }

    /// Starts `id` headless with gRPC and opens its tab.
    pub fn start(&mut self, ctx: &egui::Context, id: &str, options: &launch::Options) {
        if let Err(error) = self.try_start(ctx, id, options) {
            self.dialog = Some(Dialog::Error(format!("Cannot start {id}: {error}")));
        }
    }

    fn try_start(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        options: &launch::Options,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        let emulator = self
            .tools
            .emulator
            .as_ref()
            .context("the emulator was not found (see Settings)")?
            .path
            .clone();
        let avd = self
            .avds
            .iter()
            .find(|avd| avd.id == id)
            .cloned()
            .context("no such AVD")?;
        if let Some(running) = self.live_any(id) {
            if running.grpc_port.is_some() {
                anyhow::bail!(
                    "it is already running; open it from Devices, or stop it there first"
                );
            }
            anyhow::bail!(
                "it is already running without gRPC (Android Studio or a script started it); \
                 stop it first"
            );
        }
        // Ports of emulators that are starting but have not written their discovery file yet
        // are bound already, so a bind test covers them.
        let ports = launch::pick_ports(launch::port_free).context("no free emulator ports")?;
        let args = launch::arguments(id, ports, options);
        let log = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("avdeck")
            .join("logs")
            .join(format!("{id}.log"));
        let child = launch::spawn(&emulator, &args, &self.tools.child_env(), &log)?;
        let tab = EmulatorTab::new(
            self.runtime.handle(),
            avd,
            ports.grpc,
            Some(child),
            Some(log),
            ctx,
        );
        self.tabs.push(tab);
        self.page = Page::Emulator(id.to_owned());
        self.scanned_live = None;
        Ok(())
    }

    /// Stops the emulator of `id` and closes its tab: a clean shutdown over gRPC (it saves its
    /// state), then the console's `kill` if it is still there after 15 s. The Devices page shows
    /// it as stopping, and offers Force stop once it has not exited after 40 s.
    pub fn stop(&mut self, id: &str) {
        let running = self.live_any(id).cloned();
        let pid = self
            .tab(id)
            .and_then(|tab| tab.pid())
            .or(running.as_ref().map(|running| running.pid));
        let grpc = running
            .as_ref()
            .and_then(|running| Some((running.grpc_port?, running.grpc_token.clone())));
        let console = self
            .tab(id)
            .and_then(|tab| tab.console_port())
            .or(running.as_ref().and_then(|running| running.console_port));
        self.close_tab(id);
        let Some(pid) = pid else { return };
        self.stopping.insert(
            id.to_owned(),
            Stopping {
                pid,
                since: Instant::now(),
            },
        );
        self.runtime.spawn(async move {
            if let Some((port, token)) = grpc {
                crate::session::shutdown(port, token).await;
            }
            for _ in 0..15 {
                if !running::alive(pid) {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            if let Some(port) = console {
                let _ = tokio::task::spawn_blocking(move || console::kill(port)).await;
            }
        });
    }

    /// Ends an emulator that did not stop, without saving its state.
    pub fn force_stop(&mut self, id: &str) {
        if let Some(stopping) = self.stopping.get(id) {
            running::terminate(stopping.pid);
        }
        self.scanned_live = None;
    }

    /// Closes the tab of `id`; its emulator keeps running unless stopped.
    fn close_tab(&mut self, id: &str) {
        if self.tab(id).and_then(|tab| tab.pid()) == extended::docked_pid() {
            extended::detach();
        }
        // Its Extended Controls go with the tab, docked or not.
        if let Some(emulator) = self.tab(id).and_then(|tab| tab.emulator()) {
            self.runtime
                .spawn(crate::session::close_extended_controls(emulator));
        }
        self.tabs.retain(|tab| tab.avd.id != id);
        if self.page == Page::Emulator(id.to_owned()) {
            self.page = self
                .tabs
                .last()
                .map_or(Page::Devices, |tab| Page::Emulator(tab.avd.id.clone()));
        }
    }

    fn request_close_tab(&mut self, id: &str) {
        let running = self
            .tab(id)
            .is_some_and(|tab| !matches!(tab.phase(), Phase::Ended(_)));
        match (running, self.config.close_tab) {
            (false, _) | (true, CloseAction::KeepRunning) => self.close_tab(id),
            (true, CloseAction::Stop) => self.stop(id),
            (true, CloseAction::Ask) => {
                self.remember = false;
                self.dialog = Some(Dialog::CloseTab(id.to_owned()));
            }
        }
    }

    /// Before the window closes: remember the tabs and where the window was.
    fn save_state(&mut self, ctx: &egui::Context) {
        self.config.tabs = self.tabs.iter().map(|tab| tab.avd.id.clone()).collect();
        if let Some(outer) = ctx.input(|input| input.viewport().outer_rect) {
            self.config.window = Some(WindowPosition {
                x: outer.min.x,
                y: outer.min.y,
            });
        }
        let _ = self.config.save();
        extended::detach();
        // Every tab's Extended Controls, docked or not, before the process ends (at most 2 s).
        let emulators: Vec<_> = self.tabs.iter().filter_map(|tab| tab.emulator()).collect();
        self.runtime.block_on(async {
            let tasks: Vec<_> = emulators
                .into_iter()
                .map(|emulator| tokio::spawn(crate::session::close_extended_controls(emulator)))
                .collect();
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                for task in tasks {
                    let _ = task.await;
                }
            })
            .await;
        });
    }

    pub fn save_config(&mut self) {
        if let Err(error) = self.config.save() {
            self.dialog = Some(Dialog::Error(format!("Cannot save settings: {error}")));
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) || self.closing {
            return;
        }
        let running = self
            .tabs
            .iter()
            .any(|tab| !matches!(tab.phase(), Phase::Ended(_)));
        match (running, self.config.close_app) {
            (false, _) | (true, CloseAction::KeepRunning) => self.save_state(ctx),
            (true, CloseAction::Stop) => {
                self.save_state(ctx);
                self.stop_all();
            }
            (true, CloseAction::Ask) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.remember = false;
                self.dialog = Some(Dialog::CloseApp);
            }
        }
    }

    /// Asks every open emulator to shut down, waiting at most 5 s: the app is about to exit.
    fn stop_all(&mut self) {
        let targets: Vec<(u16, Option<String>)> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let running = self.live_any(&tab.avd.id)?;
                Some((running.grpc_port?, running.grpc_token.clone()))
            })
            .collect();
        self.tabs.clear();
        self.runtime.block_on(async {
            let tasks: Vec<_> = targets
                .into_iter()
                .map(|(port, token)| tokio::spawn(crate::session::shutdown(port, token)))
                .collect();
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                for task in tasks {
                    let _ = task.await;
                }
            })
            .await;
        });
    }

    fn close_app(&mut self, ctx: &egui::Context, stop: bool) {
        if self.remember {
            self.config.close_app = if stop {
                CloseAction::Stop
            } else {
                CloseAction::KeepRunning
            };
        }
        self.save_state(ctx);
        if stop {
            self.stop_all();
        }
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        match &dialog {
            Dialog::CloseTab(id) => {
                let name = self.tab(id).map_or(id.clone(), |tab| tab.avd.name.clone());
                let mut choice = None;
                modal(ctx, "close-tab", |ui| {
                    ui.label(theme::title(format!("Close {name}?"), 18.0));
                    ui.label(theme::muted(
                        ui,
                        "The tab closes. What happens to the device?",
                    ));
                    ui.add_space(4.0);
                    if theme::option_card(
                        ui,
                        icons::START,
                        "Keep it running",
                        "It goes on in the background; open it again from Devices.",
                        true,
                    )
                    .clicked()
                    {
                        choice = Some(false);
                    }
                    if theme::option_card(
                        ui,
                        icons::POWER,
                        "Stop the device",
                        "Its state is saved; the next start resumes where it left off.",
                        false,
                    )
                    .clicked()
                    {
                        choice = Some(true);
                    }
                    if footer(ui, &mut self.remember) {
                        keep = false;
                    }
                });
                // Enter takes the recommended choice, Escape cancels.
                let (enter, escape) = ctx.input(|input| {
                    (
                        input.key_pressed(egui::Key::Enter),
                        input.key_pressed(egui::Key::Escape),
                    )
                });
                if enter && choice.is_none() {
                    choice = Some(false);
                }
                if escape {
                    keep = false;
                }
                if let Some(stop) = choice {
                    if self.remember {
                        self.config.close_tab = if stop {
                            CloseAction::Stop
                        } else {
                            CloseAction::KeepRunning
                        };
                        self.save_config();
                    }
                    if stop {
                        self.stop(id);
                    } else {
                        self.close_tab(id);
                    }
                    keep = false;
                }
            }
            Dialog::CloseApp => {
                let mut choice = None;
                modal(ctx, "close-app", |ui| {
                    ui.label(theme::title("Close Avdeck?", 18.0));
                    let names: Vec<&str> = self
                        .tabs
                        .iter()
                        .filter(|tab| !matches!(tab.phase(), Phase::Ended(_)))
                        .map(|tab| tab.avd.name.as_str())
                        .collect();
                    ui.label(theme::muted(
                        ui,
                        format!("Still running: {}.", names.join(", ")),
                    ));
                    ui.add_space(4.0);
                    let one = names.len() == 1;
                    let (keep_title, keep_detail, stop_title, stop_detail) = if one {
                        (
                            "Keep it running",
                            "It goes on in the background; Avdeck reopens its tab next time.",
                            "Stop it",
                            "Its state is saved; the next start resumes where it left off.",
                        )
                    } else {
                        (
                            "Keep them running",
                            "They go on in the background; Avdeck reopens their tabs next time.",
                            "Stop them all",
                            "Their state is saved; the next start resumes where they left off.",
                        )
                    };
                    if theme::option_card(ui, icons::START, keep_title, keep_detail, true).clicked()
                    {
                        choice = Some(false);
                    }
                    if theme::option_card(ui, icons::POWER, stop_title, stop_detail, false)
                        .clicked()
                    {
                        choice = Some(true);
                    }
                    if footer(ui, &mut self.remember) {
                        keep = false;
                    }
                });
                let (enter, escape) = ctx.input(|input| {
                    (
                        input.key_pressed(egui::Key::Enter),
                        input.key_pressed(egui::Key::Escape),
                    )
                });
                if enter && choice.is_none() {
                    choice = Some(false);
                }
                if escape {
                    keep = false;
                }
                if let Some(stop) = choice {
                    self.close_app(ctx, stop);
                    keep = false;
                }
            }
            Dialog::Error(message) => {
                modal(ctx, "error", |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(icons::WARNING)
                                .size(22.0)
                                .color(theme::palette(ui).danger),
                        );
                        ui.label(theme::title("Something went wrong", 18.0));
                    });
                    ui.label(theme::muted(ui, message.as_str()));
                    ui.with_layout(buttons(), |ui| {
                        if theme::primary(ui, "OK").clicked() {
                            keep = false;
                        }
                    });
                });
            }
            Dialog::Manage(_) => {
                let Dialog::Manage(inner) = dialog else {
                    unreachable!()
                };
                self.dialog = crate::manage::show(self, ctx, *inner)
                    .map(|dialog| Dialog::Manage(Box::new(dialog)));
                return;
            }
        }
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let palette = theme::palette(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Settings stays pinned on the right; the tabs scroll in the rest when they overflow.
            let strip = (ui.available_width() - 38.0).max(0.0);
            // The wheel scrolls the tabs sideways over the strip, no Ctrl or Shift needed.
            let strip_rect = egui::Rect::from_min_size(ui.cursor().min, vec2(strip, 40.0));
            if ui.rect_contains_pointer(strip_rect) {
                ui.ctx().input_mut(|input| {
                    let delta = input.smooth_scroll_delta;
                    input.smooth_scroll_delta = vec2(delta.x + delta.y, 0.0);
                });
            }
            egui::ScrollArea::horizontal()
                .id_salt("tab-strip")
                .max_width(strip)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        // In a narrow window (a phone at its smallest) Devices is just its icon.
                        let label = if strip < 380.0 { "" } else { "Devices" };
                        let (devices, _, response) = tab_widget(
                            ui,
                            self.page == Page::Devices,
                            None,
                            Some(icons::DEVICES),
                            label,
                            false,
                        );
                        if label.is_empty() {
                            response.on_hover_text("Devices (Ctrl+1)");
                        }
                        if devices {
                            self.page = Page::Devices;
                        }
                        let mut close = None;
                        for tab in &self.tabs {
                            let selected = self.page == Page::Emulator(tab.avd.id.clone());
                            let dot = match tab.phase() {
                                Phase::Starting => palette.warning,
                                Phase::Running => palette.success,
                                Phase::Ended(_) => palette.danger,
                            };
                            let (clicked, closed, response) =
                                tab_widget(ui, selected, Some(dot), None, &tab.avd.name, true);
                            if clicked {
                                self.page = Page::Emulator(tab.avd.id.clone());
                            }
                            // Bring the selected tab into view when the page changed (opened, Ctrl+Tab, ...).
                            if selected && self.shown_page.as_ref() != Some(&self.page) {
                                response.scroll_to_me(None);
                            }
                            if closed || response.middle_clicked() {
                                close = Some(tab.avd.id.clone());
                            }
                        }
                        if let Some(id) = close {
                            self.request_close_tab(&id);
                        }
                        // Open another device without going through the Devices page.
                        let add = theme::icon(ui, icons::ADD, "Open a device", 30.0);
                        egui::Popup::menu(&add).show(|ui| {
                            theme::menu(ui, 240.0, |ui| {
                                let closed: Vec<Avd> = self
                                    .avds
                                    .iter()
                                    .filter(|avd| self.tab(&avd.id).is_none())
                                    .cloned()
                                    .collect();
                                if closed.is_empty() {
                                    ui.label(theme::muted(ui, "Every device is open."));
                                }
                                for avd in closed {
                                    let running = self.live_grpc(&avd.id).is_some();
                                    let glyph = if running {
                                        icons::DEVICES
                                    } else {
                                        icons::START
                                    };
                                    if theme::menu_item(ui, glyph, &avd.name, true)
                                        .on_hover_text(if running {
                                            "Running: open it"
                                        } else {
                                            "Start it"
                                        })
                                        .clicked()
                                    {
                                        self.open(ctx, &avd.id);
                                        ui.close();
                                    }
                                }
                            });
                        });
                    });
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let settings = theme::icon(ui, icons::SETTINGS, "Settings (Ctrl+,)", 30.0);
                if self.page == Page::Settings {
                    ui.painter().rect_stroke(
                        settings.rect,
                        egui::CornerRadius::same(8),
                        egui::Stroke::new(1.0, palette.border),
                        egui::StrokeKind::Inside,
                    );
                }
                if settings.clicked() {
                    self.page = Page::Settings;
                }
            });
        });
        // Ctrl+Tab / Ctrl+Shift+Tab and Ctrl+1..9 switch tabs.
        let (next, previous, number) = ctx.input_mut(|input| {
            let next = input.consume_key(egui::Modifiers::CTRL, egui::Key::Tab);
            let previous = input.consume_key(
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                egui::Key::Tab,
            );
            let keys = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ];
            let number = keys
                .iter()
                .position(|&key| input.consume_key(egui::Modifiers::CTRL, key));
            (next, previous, number)
        });
        let mut pages = vec![Page::Devices];
        pages.extend(
            self.tabs
                .iter()
                .map(|tab| Page::Emulator(tab.avd.id.clone())),
        );
        let current = pages
            .iter()
            .position(|page| *page == self.page)
            .unwrap_or(0);
        if next {
            self.page = pages[(current + 1) % pages.len()].clone();
        } else if previous {
            self.page = pages[(current + pages.len() - 1) % pages.len()].clone();
        } else if let Some(index) = number {
            // Ctrl+1 is Devices, Ctrl+2 the first emulator, ...
            if let Some(page) = pages.get(index) {
                self.page = page.clone();
            }
        }
    }

    /// Locks resizing to the active emulator's aspect ratio and sizes the window: the device's
    /// longer side at 80% of the monitor height the first time, the tab's own size when
    /// switching to it, swapped when the device turns; the pages keep a size of their own.
    fn fit_window(&mut self, ctx: &egui::Context, hwnd: isize, tab_bar: f32) {
        if self.window.is_none() {
            window::install(hwnd as _);
            self.window = Some(hwnd);
        }
        let info = ctx.input(|input| input.viewport().clone());
        let (Some(monitor), Some(outer), Some(inner)) =
            (info.monitor_size, info.outer_rect, info.inner_rect)
        else {
            return;
        };
        if info.maximized == Some(true) || info.fullscreen == Some(true) {
            window::set_layout(0.0, (0, 0), 0);
            return;
        }
        let pixels_per_point = ctx.pixels_per_point();
        let ppp_bits = pixels_per_point.to_bits();
        let chrome = outer.size() - inner.size();
        if let Some((target, since)) = self.fit.pending
            && ((inner.size() - target).length() < 2.0
                || since.elapsed() > Duration::from_millis(600))
        {
            self.fit.pending = None;
        }

        let (layout, extras, aspect, shape) = match &self.page {
            Page::Emulator(id) => {
                let Some(tab) = self.tab(id) else { return };
                let Some(aspect) = tab.aspect() else {
                    window::set_layout(0.0, (0, 0), 0);
                    return;
                };
                let layout = Layout::Emulator {
                    id: id.clone(),
                    sideways: tab.rotation.sideways(),
                    pixels_per_point: ppp_bits,
                };
                (
                    layout,
                    vec2(TOOLBAR_WIDTH, tab_bar),
                    aspect,
                    tab.device_shape(),
                )
            }
            _ => {
                window::set_layout(0.0, (0, 0), 0);
                let layout = Layout::Page {
                    pixels_per_point: ppp_bits,
                };
                (layout, egui::Vec2::ZERO, 0.0, (0, 0))
            }
        };
        // The smallest screen that still fits the toolbar beside it.
        let min_screen = match &self.page {
            Page::Emulator(id) if aspect > 0.0 => self.tab(id).map_or(0.0, |tab| {
                display::min_screen_height(
                    tab.toolbar_height(),
                    TOOLBAR_WIDTH,
                    MIN_WINDOW_WIDTH,
                    aspect,
                )
            }),
            _ => 0.0,
        };
        if aspect > 0.0 {
            window::set_layout(
                aspect,
                (
                    (extras.x * pixels_per_point).round() as i32,
                    (extras.y * pixels_per_point).round() as i32,
                ),
                (min_screen * pixels_per_point).round() as i32,
            );
        }

        if self.fit.layout.as_ref() == Some(&layout) {
            // Learn the size the user gave the window.
            if self.fit.pending.is_none() {
                match &self.page {
                    Page::Emulator(id) => {
                        let id = id.clone();
                        if let Some(tab) = self.tab_mut(&id) {
                            tab.screen_size = Some(inner.size() - extras);
                        }
                    }
                    _ => self.fit.page_size = Some(inner.size()),
                }
            }
            return;
        }

        let previous = self.fit.layout.replace(layout.clone());
        // Never smaller than this: the pages need room, a device needs its toolbar.
        let minimum = if aspect > 0.0 {
            vec2(min_screen * aspect, min_screen) + extras
        } else {
            vec2(720.0, 520.0)
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(minimum));
        let mut keep_center = false;
        let size = match &layout {
            Layout::Page { .. } => self.fit.page_size.unwrap_or(vec2(960.0, 640.0)),
            Layout::Emulator { id, sideways, .. } => {
                let id = id.clone();
                let tab = self.tab_mut(&id).expect("tab");
                let screen = match (&previous, tab.screen_size) {
                    (_, None) => {
                        let (width, height) = display::initial_screen_size(monitor.y, 0.8, shape);
                        vec2(width, height)
                    }
                    // Turned: same size on screen, the other way round.
                    (
                        Some(Layout::Emulator {
                            id: before,
                            sideways: was,
                            ..
                        }),
                        Some(size),
                    ) if *before == id && was != sideways => {
                        keep_center = true;
                        vec2(size.y, size.x)
                    }
                    // Back to this tab, or a new DPI: keep the height, fit the width.
                    (_, Some(size)) => vec2(size.y * aspect, size.y),
                };
                tab.screen_size = Some(screen);
                screen + extras
            }
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        self.fit.pending = Some((size, Instant::now()));
        let new_outer = size + chrome;
        if !self.fit.placed {
            self.fit.placed = true;
            let position = match self.config.window {
                Some(saved) => pos2(saved.x, saved.y),
                // Centered on the monitor the window opened on (the primary one, at the origin).
                None => pos2(
                    (monitor.x - new_outer.x) / 2.0,
                    (monitor.y - new_outer.y) / 2.0,
                ),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
        } else if keep_center {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                outer.center() - new_outer / 2.0,
            ));
        }
    }

    /// Installs dropped APKs and copies other dropped files to `/sdcard/Download`, with adb.
    fn handle_dropped_files(&mut self) {
        let adb = self.tools.adb.as_ref().map(|adb| adb.path.clone());
        let env = self.tools.child_env();
        for tab in &mut self.tabs {
            let files = std::mem::take(&mut tab.dropped);
            if files.is_empty() {
                continue;
            }
            let (Some(adb), Some(port)) = (adb.clone(), tab.console_port()) else {
                tab.notify("adb was not found (see Settings)".to_owned());
                continue;
            };
            let state = tab.session.state.clone();
            let env = env.clone();
            tab.notify(format!("Sending {} file(s)…", files.len()));
            std::thread::spawn(move || {
                for file in files {
                    let name = file
                        .file_name()
                        .map_or(String::new(), |name| name.to_string_lossy().into_owned());
                    let apk = file
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("apk"));
                    let serial = format!("emulator-{port}");
                    let path = file.display().to_string();
                    let args: Vec<&str> = if apk {
                        vec!["-s", &serial, "install", "-r", "-t", &path]
                    } else {
                        vec!["-s", &serial, "push", &path, "/sdcard/Download/"]
                    };
                    let output = adb_command(&adb, &args, &env);
                    let text = match output {
                        Ok(_) if apk => format!("Installed {name}"),
                        Ok(_) => format!("Copied {name} to /sdcard/Download"),
                        Err(error) => {
                            format!("{name}: {}", error.lines().last().unwrap_or("failed"))
                        }
                    };
                    state.lock().unwrap().notice = Some(text);
                }
            });
        }
    }

    /// Docks the Extended Controls window beside Avdeck once it appears (within 3 s).
    fn dock_extended_controls(&mut self, ctx: &egui::Context) {
        let owner = self.window;
        for tab in &mut self.tabs {
            let Some(started) = tab.extended_search else {
                continue;
            };
            let found = tab.pid().and_then(extended::find);
            if let (Some(window), Some(owner)) = (found, owner) {
                extended::attach(window, owner as _);
                tab.extended_search = None;
            } else if started.elapsed() > Duration::from_secs(3) {
                tab.extended_search = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }
}

impl App {
    /// One frame of the window `hwnd`.
    pub fn ui(&mut self, ui: &mut egui::Ui, hwnd: isize) {
        let ctx = ui.ctx().clone();
        // Frames that come early (animations, a busy stream) wait their turn; the first frame
        // after a quiet spell, such as the answer to a click, does not.
        if let Some(due) = self.last_frame.map(|last| last + FRAME_TIME) {
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
        }
        self.last_frame = Some(Instant::now());
        self.rescan(false);
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::D)
        }) {
            self.debug = !self.debug;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Comma)) {
            self.page = Page::Settings;
        }
        // Requests from `avdeck --avd NAME` started again.
        for request in crate::instance::take() {
            self.handle_request(&ctx, &request);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        for tab in &mut self.tabs {
            tab.update(&ctx);
        }
        // Buttons inside a tab that need the app.
        let requests: Vec<(String, crate::tab::Request)> = self
            .tabs
            .iter_mut()
            .filter_map(|tab| Some((tab.avd.id.clone(), tab.request.take()?)))
            .collect();
        for (id, request) in requests {
            let pid = self.tab(&id).and_then(|tab| tab.pid());
            self.close_tab(&id);
            match request {
                crate::tab::Request::Close => {}
                crate::tab::Request::Restart => {
                    self.rescan(true);
                    self.open(&ctx, &id);
                }
                // The process lives on without answering: end it, start once it is gone.
                crate::tab::Request::ForceRestart => {
                    if let Some(pid) = pid {
                        running::terminate(pid);
                        self.stopping.insert(
                            id.clone(),
                            Stopping {
                                pid,
                                since: Instant::now(),
                            },
                        );
                    }
                    self.start_when_stopped.insert(id);
                }
            }
        }
        // Starts waiting for their old process to go.
        let ready: Vec<String> = self
            .start_when_stopped
            .iter()
            .filter(|id| !self.stopping.contains_key(*id) && self.live_any(id).is_none())
            .cloned()
            .collect();
        for id in ready {
            self.start_when_stopped.remove(&id);
            let options = self.config.launch.clone();
            self.start(&ctx, &id, &options);
        }
        if !self.start_when_stopped.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // Only the shown tab streams frames.
        let previous_extended = extended::docked_pid();
        for tab in &self.tabs {
            let shown = self.page == Page::Emulator(tab.avd.id.clone());
            tab.session.set_visible(shown);
            // Another tab's Extended Controls window goes away with its tab.
            if !shown && tab.pid().is_some() && tab.pid() == previous_extended {
                extended::detach();
            }
        }

        let chrome = theme::palette(ui).chrome;
        let tab_bar_page = self.page.clone();
        let tab_bar = egui::Panel::top("tabs")
            .frame(
                egui::Frame::NONE
                    .fill(chrome)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(ui, |ui| self.tab_bar(ui, &ctx));
        let tab_bar_height = tab_bar.response.rect.height();
        self.shown_page = Some(tab_bar_page);

        match self.page.clone() {
            Page::Emulator(id) => {
                let debug = self.debug;
                if let Some(tab) = self.tab_mut(&id) {
                    egui::Panel::right("toolbar")
                        .resizable(false)
                        .exact_size(TOOLBAR_WIDTH)
                        .frame(egui::Frame::NONE.fill(chrome))
                        .show(ui, |ui| tab.toolbar(ui));
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(Color32::BLACK))
                        .show(ui, |ui| {
                            if let Some(size) = tab.screen(ui, debug) {
                                tab.session.set_view_size(size);
                            }
                        });
                } else {
                    self.page = Page::Devices;
                }
            }
            Page::Devices => {
                egui::CentralPanel::default().show(ui, |ui| crate::devices::show(self, ui, &ctx));
            }
            Page::Settings => {
                egui::CentralPanel::default().show(ui, |ui| crate::settings::show(self, ui));
            }
        }

        self.dialogs(&ctx);
        self.fit_window(&ctx, hwnd, tab_bar_height);
        self.dock_extended_controls(&ctx);
        self.handle_dropped_files();
        self.handle_close_request(&ctx);
        // Keep statuses fresh on the Devices page.
        if self.page == Page::Devices {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

/// Runs adb without a console window; the error is its output.
fn adb_command(
    adb: &std::path::Path,
    args: &[&str],
    env: &[(&'static str, std::ffi::OsString)],
) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new(adb)
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|error| error.to_string())?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() && !text.contains("Failure") {
        Ok(text)
    } else {
        Err(text.trim().to_owned())
    }
}

/// A tab in the tab bar: optional status dot or icon, the label, and a close button shown on
/// hover and when selected. Returns (clicked, close clicked, response).
fn tab_widget(
    ui: &mut egui::Ui,
    selected: bool,
    dot: Option<Color32>,
    glyph: Option<&str>,
    label: &str,
    closable: bool,
) -> (bool, bool, egui::Response) {
    let palette = theme::palette(ui);
    let font = egui::FontId::proportional(13.5);
    // Hover eases in, keyed by the id the tab is about to get.
    let hot = ui.ctx().animate_bool_with_time(
        ui.next_auto_id().with("hover"),
        ui.ctx()
            .read_response(ui.next_auto_id())
            .is_some_and(|response| response.hovered()),
        0.12,
    );
    let text_color = if selected {
        palette.text
    } else {
        palette.muted.lerp_to_gamma(palette.text, hot)
    };
    // Long names end in "…"; the full name shows on hover.
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font, text_color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(150.0);
    let galley = ui.painter().layout_job(job);
    let elided = galley.elided;
    let lead = if dot.is_some() || glyph.is_some() {
        22.0
    } else {
        0.0
    };
    let tail = if closable { 24.0 } else { 0.0 };
    let text_width = if label.is_empty() {
        -8.0
    } else {
        galley.size().x
    };
    let size = vec2(12.0 + lead + text_width + tail + 12.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    // Named for screen readers and UI Automation, which see only what widgets describe.
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    let hovered = response.hovered();
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, egui::CornerRadius::same(8), palette.surface);
        painter.rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            egui::Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        );
    } else if hot > 0.0 {
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(8),
            palette.surface_hover.gamma_multiply(hot),
        );
    }
    let mut x = rect.left() + 12.0;
    if let Some(color) = dot {
        painter.circle_filled(pos2(x + 5.0, rect.center().y), 4.0, color);
        x += lead;
    } else if let Some(glyph) = glyph {
        painter.text(
            pos2(x + 6.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(16.0),
            text_color,
        );
        x += lead;
    }
    painter.galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    let mut closed = false;
    if closable && (hovered || selected) {
        let close = egui::Rect::from_center_size(
            pos2(rect.right() - 20.0, rect.center().y),
            vec2(20.0, 20.0),
        );
        let close_response = ui.interact(close, response.id.with("close"), egui::Sense::click());
        close_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Close {label}"))
        });
        if close_response.hovered() {
            ui.painter()
                .rect_filled(close, egui::CornerRadius::same(6), palette.surface_hover);
        }
        ui.painter().text(
            close.center(),
            egui::Align2::CENTER_CENTER,
            icons::CLOSE,
            egui::FontId::proportional(12.0),
            palette.muted,
        );
        closed = close_response.on_hover_text("Close tab").clicked();
    }
    let response = if elided {
        response.on_hover_text(label)
    } else {
        response
    };
    (response.clicked() && !closed, closed, response)
}

/// The bottom of a choice dialog: "Remember my choice" on the left, Cancel on the right.
/// Returns whether Cancel was clicked.
fn footer(ui: &mut egui::Ui, remember: &mut bool) -> bool {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.checkbox(remember, "Remember my choice");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            theme::secondary(ui, "Cancel").clicked()
        })
        .inner
    })
    .inner
}

/// The button row of a dialog: right-aligned, wrapping when the dialog is narrow.
pub fn buttons() -> egui::Layout {
    egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true)
}

/// A centered modal dialog.
pub fn modal(ctx: &egui::Context, id: &str, content: impl FnOnce(&mut egui::Ui)) {
    egui::Modal::new(egui::Id::unique(id))
        .frame(
            egui::Frame::popup(&ctx.style_of(ctx.theme()))
                .inner_margin(egui::Margin::same(22))
                .corner_radius(egui::CornerRadius::same(14)),
        )
        .show(ctx, |ui| {
            // As wide as the window allows (inside the 22 pt frame margins and 8 pt to spare on
            // each side), up to 520: text wraps, buttons wrap.
            let width = (ctx.content_rect().width() - 2.0 * 22.0 - 16.0).clamp(140.0, 520.0);
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 10.0;
            content(ui);
        });
}
