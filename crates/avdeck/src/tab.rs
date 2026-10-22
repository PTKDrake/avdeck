//! One emulator tab: the device screen, the toolbar beside it, and input forwarding.

use crate::icons;
use crate::keyboard;
use crate::session::{Input, Phase, Session, Target};
use crate::theme;
use avdeck_core::avd::Avd;
use avdeck_core::display::{self, Rotation};
use eframe::egui::{self, Color32, ColorImage, Sense, TextureHandle, TextureOptions, pos2, vec2};
use std::process::Child;
use std::time::Instant;

#[derive(Clone, Copy)]
enum Action {
    Key(&'static str),
    /// Quarter turns counterclockwise.
    Rotate(i32),
    Fold,
    Screenshot,
    ExtendedControls,
}

/// The emulator's Qt toolbar, top to bottom: icon, tooltip, action. `None` is a separator.
const TOOLBAR: [Option<(&str, &str, Action)>; 13] = [
    Some((icons::POWER, "Power", Action::Key("Power"))),
    Some((icons::VOLUME_UP, "Volume up", Action::Key("AudioVolumeUp"))),
    Some((
        icons::VOLUME_DOWN,
        "Volume down",
        Action::Key("AudioVolumeDown"),
    )),
    None,
    Some((icons::ROTATE_LEFT, "Rotate left", Action::Rotate(1))),
    Some((icons::ROTATE_RIGHT, "Rotate right", Action::Rotate(-1))),
    Some((
        icons::SCREENSHOT,
        "Take screenshot (Pictures\\Avdeck)",
        Action::Screenshot,
    )),
    None,
    Some((icons::BACK, "Back", Action::Key("GoBack"))),
    Some((icons::HOME, "Home", Action::Key("GoHome"))),
    Some((icons::OVERVIEW, "Overview", Action::Key("AppSwitch"))),
    None,
    Some((icons::MORE, "Extended controls", Action::ExtendedControls)),
];

/// Width of the toolbar panel, in points.
pub const TOOLBAR_WIDTH: f32 = 44.0;

pub struct EmulatorTab {
    pub avd: Avd,
    pub session: Session,
    /// The emulator process, when Avdeck started it.
    pub child: Option<Child>,
    pub log: Option<std::path::PathBuf>,
    texture: Option<TextureHandle>,
    /// Size of the last frame, in its displayed orientation.
    pub frame_size: (u32, u32),
    pub rotation: Rotation,
    /// Size in points the screen is shown at; follows the window, swaps on rotation.
    pub screen_size: Option<egui::Vec2>,
    folded: bool,
    /// Last device position sent while the mouse button is down.
    touching: Option<(i32, i32)>,
    /// The current drag started with Ctrl: a two-finger pinch.
    pinching: bool,
    /// Modifiers already sent to the device.
    modifiers: egui::Modifiers,
    /// Since when the Extended Controls window is awaited, to dock it once it shows.
    pub extended_search: Option<Instant>,
    /// A message over the screen (screenshot saved, APK installed) and since when.
    notice: Option<(String, Instant)>,
    /// Files dropped on the screen, for the app to install or push with adb.
    pub dropped: Vec<std::path::PathBuf>,
    /// Something only the app can do (restart, close), asked from inside the tab.
    pub request: Option<Request>,
}

pub enum Request {
    Restart,
    /// End the process that no longer answers, then start again.
    ForceRestart,
    Close,
}

impl EmulatorTab {
    pub fn new(
        runtime: &tokio::runtime::Handle,
        avd: Avd,
        grpc_port: u16,
        child: Option<Child>,
        log: Option<std::path::PathBuf>,
        ctx: &egui::Context,
    ) -> Self {
        let natural = avd.panel.unwrap_or((1080, 2400));
        let target = Target {
            avd_id: avd.id.clone(),
            natural,
            grpc_port,
        };
        Self {
            session: Session::start(runtime, target, ctx.clone()),
            avd,
            child,
            log,
            texture: None,
            frame_size: (0, 0),
            rotation: Rotation::Portrait,
            screen_size: None,
            folded: false,
            touching: None,
            pinching: false,
            modifiers: egui::Modifiers::default(),
            extended_search: None,
            notice: None,
            dropped: Vec::new(),
            request: None,
        }
    }

    /// Height in points the toolbar needs: its buttons, separators and margins.
    pub fn toolbar_height(&self) -> f32 {
        let buttons = TOOLBAR.iter().flatten().count() + usize::from(self.avd.foldable);
        let separators = TOOLBAR.len() - TOOLBAR.iter().flatten().count();
        8.0 + buttons as f32 * 36.0 + separators as f32 * 13.0 + 8.0
    }

    pub fn natural(&self) -> (u32, u32) {
        self.avd.panel.unwrap_or((1080, 2400))
    }

    pub fn phase(&self) -> Phase {
        self.session.state.lock().unwrap().phase.clone()
    }

    pub fn pid(&self) -> Option<u32> {
        self.session.state.lock().unwrap().pid
    }

    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture.as_ref().map(TextureHandle::id)
    }

    /// Shows `text` over the screen for a few seconds.
    pub fn notify(&mut self, text: String) {
        self.notice = Some((text, Instant::now()));
    }

    pub fn emulator(&self) -> Option<avdeck_grpc::Emulator> {
        self.session.state.lock().unwrap().emulator.clone()
    }

    pub fn console_port(&self) -> Option<u16> {
        self.session.state.lock().unwrap().console_port
    }

    /// Aspect ratio of what the screen shows now, once a frame arrived.
    pub fn aspect(&self) -> Option<f32> {
        (self.frame_size.0 > 0).then(|| self.frame_size.0 as f32 / self.frame_size.1 as f32)
    }

    /// Takes the newest frame and clipboard text from the session.
    pub fn update(&mut self, ctx: &egui::Context) {
        let (frame, clipboard, notice) = {
            let mut state = self.session.state.lock().unwrap();
            (
                state.frame.take(),
                state.clipboard.take(),
                state.notice.take(),
            )
        };
        if let Some(text) = clipboard {
            ctx.copy_text(text);
        }
        if let Some(text) = notice {
            self.notice = Some((text, Instant::now()));
        }
        if let Some(frame) = frame {
            let image = ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.pixels,
            );
            match &mut self.texture {
                Some(texture) => texture.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture =
                        Some(ctx.load_texture(&self.avd.id, image, TextureOptions::LINEAR))
                }
            }
            // The stream may be scaled: keep the shape, the rotation tells the orientation.
            self.frame_size = (frame.width, frame.height);
            self.rotation = Rotation::from_proto(frame.rotation);
        }
        // An emulator Avdeck started that died before serving gRPC: say why.
        if let Some(child) = &mut self.child
            && let Ok(Some(status)) = child.try_wait()
            && self.phase() == Phase::Starting
        {
            let tail = self
                .log
                .as_ref()
                .and_then(|log| std::fs::read_to_string(log).ok())
                .map(|text| {
                    let lines: Vec<&str> = text.lines().rev().take(6).collect();
                    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
                })
                .unwrap_or_default();
            self.session.state.lock().unwrap().phase =
                Phase::Ended(format!("The emulator exited ({status}).\n{tail}"));
            self.child = None;
        }
    }

    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui);
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(8.0);
            for item in TOOLBAR {
                let Some((icon, tooltip, action)) = item else {
                    ui.add_space(5.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 1.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, palette.border);
                    ui.add_space(5.0);
                    continue;
                };
                if theme::icon(ui, icon, tooltip, 34.0).clicked() {
                    self.act(action);
                }
                // Foldables get fold/unfold right after rotation.
                if matches!(action, Action::Rotate(-1)) && self.avd.foldable {
                    let tooltip = if self.folded { "Unfold" } else { "Fold" };
                    if theme::icon(ui, icons::FOLD, tooltip, 34.0).clicked() {
                        self.act(Action::Fold);
                    }
                }
            }
        });
    }

    fn act(&mut self, action: Action) {
        let input = match action {
            Action::Key(key) => Input::Key(key),
            Action::Rotate(quarters) => Input::Rotate(self.rotation.quarters() + quarters),
            Action::Fold => {
                self.folded = !self.folded;
                Input::Fold(self.folded)
            }
            Action::Screenshot => Input::Screenshot,
            Action::ExtendedControls => {
                self.extended_search = Some(Instant::now());
                Input::ExtendedControls
            }
        };
        self.session.send(input);
    }

    /// The device screen filling `ui`, with input forwarding. Returns the physical size the
    /// frame is shown at.
    pub fn screen(&mut self, ui: &mut egui::Ui, debug: bool) -> Option<(u32, u32)> {
        let ctx = ui.ctx().clone();
        let Some(texture) = &self.texture else {
            self.placeholder(ui);
            return None;
        };
        let area = ui.max_rect();
        let pixels_per_point = ctx.pixels_per_point();
        // Fit the device's shape (frames may be scaled down, the shape stays).
        let screen = display::fit(
            display::Rect::new(area.min.x, area.min.y, area.width(), area.height()),
            self.device_shape(),
            pixels_per_point,
        );
        let rect =
            egui::Rect::from_min_size(pos2(screen.x, screen.y), vec2(screen.width, screen.height));
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        let natural = self.natural();
        let response = ui.interact(rect, ui.scope_id().with("screen"), Sense::click_and_drag());
        let pressed = response.is_pointer_button_down_on();
        let position = response.interact_pointer_pos().and_then(|point| {
            display::to_device((point.x, point.y), screen, natural, self.rotation)
        });
        if pressed && self.touching.is_none() {
            // Ctrl at the start of a drag pinches, as in the emulator's own window.
            self.pinching = ui.input(|input| input.modifiers.ctrl);
        }
        match (pressed, position, self.touching) {
            (true, Some(device), last) if last != Some(device) => {
                self.touch(device, natural, 1);
                self.touching = Some(device);
            }
            (false, _, Some(device)) => {
                self.touch(device, natural, 0);
                self.touching = None;
            }
            _ => {}
        }
        let hover = response.hover_pos().and_then(|point| {
            display::to_device((point.x, point.y), screen, natural, self.rotation)
        });
        self.forward_keyboard_and_wheel(ui, hover);

        // Files dragged over the window: say what dropping does; dropped: install or copy.
        let (hovering, dropped) = ui.input(|input| {
            (
                !input.raw.hovered_files.is_empty(),
                input.raw.dropped_files.clone(),
            )
        });
        if hovering {
            theme::toast(
                ui,
                rect,
                "Drop an APK to install it; other files go to /sdcard/Download",
            );
        }
        for file in dropped {
            let path = file.path();
            if !path.as_os_str().is_empty() {
                self.dropped.push(path.to_owned());
            }
        }
        if let Some((text, since)) = &self.notice {
            if since.elapsed().as_secs() < 4 {
                theme::toast(ui, rect, text);
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            } else {
                self.notice = None;
            }
        }
        if debug {
            let fps = self.session.state.lock().unwrap().fps;
            let text = format!(
                "{:.0}% DPI · screen {}x{} px · frame {}x{} · {fps:.0} fps",
                pixels_per_point * 100.0,
                (screen.width * pixels_per_point).round(),
                (screen.height * pixels_per_point).round(),
                self.frame_size.0,
                self.frame_size.1,
            );
            theme::toast(ui, rect, &text);
        }
        Some((
            (screen.width * pixels_per_point).round() as u32,
            (screen.height * pixels_per_point).round() as u32,
        ))
    }

    /// Before the first frame: the device's outline with what is happening, or why it ended.
    fn placeholder(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui);
        let area = ui.max_rect();
        let phase = self.phase();
        let outline = egui::Rect::from_center_size(
            area.center() - vec2(0.0, 60.0),
            vec2(area.width().min(220.0), area.height().min(260.0) * 0.8),
        );
        theme::device(
            ui,
            outline,
            self.device_shape(),
            None,
            phase != Phase::Starting,
        );
        let mut column = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(area.left() + 24.0, outline.bottom() + 24.0),
                    egui::pos2(area.right() - 24.0, area.bottom()),
                ))
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        match phase {
            Phase::Starting | Phase::Running => {
                column.spinner();
                let text = if phase == Phase::Starting {
                    format!("Starting {}…", self.avd.name)
                } else {
                    "Waiting for the screen…".to_owned()
                };
                column.label(theme::title(text, 15.0));
                column.label(theme::muted(
                    &column,
                    "Booting from a snapshot takes a few seconds.",
                ));
            }
            Phase::Ended(reason) => {
                // Still running but not answering: hung. Starting again means ending it first.
                let hung = self.pid().is_some_and(avdeck_core::running::alive);
                column.label(
                    egui::RichText::new(icons::WARNING)
                        .size(26.0)
                        .color(palette.danger),
                );
                if hung {
                    column.label(theme::title("Not responding", 15.0));
                    column
                        .label(theme::muted(
                            &column,
                            "The emulator is still running but stopped answering.",
                        ))
                        .on_hover_text(&reason);
                } else {
                    column.label(theme::title("Not connected", 15.0));
                    column.label(theme::muted(&column, reason.as_str()));
                }
                column.add_space(8.0);
                // One centered button per line.
                if hung {
                    if theme::danger(&mut column, "Force stop and start again")
                        .on_hover_text("Ends the hung emulator; its unsaved state is lost")
                        .clicked()
                    {
                        self.request = Some(Request::ForceRestart);
                    }
                } else if theme::primary(&mut column, format!("{}  Start again", icons::START))
                    .clicked()
                {
                    self.request = Some(Request::Restart);
                }
                if column
                    .add(egui::Button::new("Close tab").frame(false))
                    .clicked()
                {
                    self.request = Some(Request::Close);
                }
            }
        }
    }

    /// The device's display in its current orientation, at native resolution. Rotation is
    /// relative to the natural orientation (portrait for phones, landscape for tablets).
    pub fn device_shape(&self) -> (u32, u32) {
        let (width, height) = self.natural();
        if self.rotation.sideways() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// One finger, or two mirrored through the screen center while pinching.
    fn touch(&self, (x, y): (i32, i32), natural: (u32, u32), pressure: i32) {
        let mut points = vec![(x, y)];
        if self.pinching {
            points.push((natural.0 as i32 - 1 - x, natural.1 as i32 - 1 - y));
        }
        self.session.send(Input::Touch { points, pressure });
    }

    /// Keys, modifiers, text, clipboard shortcuts and the wheel, as the Qt window forwards them.
    fn forward_keyboard_and_wheel(&mut self, ui: &egui::Ui, hover: Option<(i32, i32)>) {
        let (events, modifiers) = ui.input(|input| (input.events.clone(), input.modifiers));
        for (name, now, before) in [
            ("Shift", modifiers.shift, self.modifiers.shift),
            ("Control", modifiers.ctrl, self.modifiers.ctrl),
            ("Alt", modifiers.alt, self.modifiers.alt),
        ] {
            if now != before {
                let name = name.to_owned();
                self.session.send(if now {
                    Input::KeyDown(name)
                } else {
                    Input::KeyUp(name)
                });
            }
        }
        self.modifiers = modifiers;

        for event in events {
            match event {
                egui::Event::Key { key, pressed, .. } => {
                    if let Some(value) = keyboard::key_value(key) {
                        self.session.send(if pressed {
                            Input::KeyDown(value)
                        } else {
                            Input::KeyUp(value)
                        });
                    }
                }
                // Typed text the key events already cover is ASCII; anything else (Vietnamese
                // accents, IME output) goes in through the clipboard.
                egui::Event::Text(text) if !text.is_ascii() => {
                    self.session.send(Input::Text(text));
                }
                // egui turns Ctrl+C/X/V into these instead of key events; Control is already down.
                egui::Event::Copy | egui::Event::Cut => {
                    let letter = if event == egui::Event::Copy { "c" } else { "x" };
                    self.session.send(Input::KeyDown(letter.to_owned()));
                    self.session.send(Input::KeyUp(letter.to_owned()));
                }
                egui::Event::Paste(text) => self.session.send(Input::Paste(text)),
                egui::Event::MouseWheel { unit, delta, .. } => {
                    let Some((x, y)) = hover else { continue };
                    let clicks = match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Point => delta.y / 50.0,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                    };
                    let dy = (clicks * 120.0).round() as i32;
                    if dy != 0 {
                        self.session.send(Input::Wheel {
                            x,
                            y,
                            dy,
                            rotation: self.rotation,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}
