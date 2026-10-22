//! Avdeck app. M1 so far: attaches to one running emulator, shows its screen and forwards mouse
//! input as touches plus a few hardware keys.
//!
//! ```text
//! avdeck [--grpc PORT]
//! ```

mod extended;
mod icons;
mod keyboard;
mod session;
mod window;

use avdeck_core::display::{self, Rotation};
use eframe::egui::{self, Color32, ColorImage, Sense, TextureHandle, TextureOptions, pos2, vec2};
use session::{Input, Session};

#[derive(Clone, Copy)]
enum Action {
    Key(&'static str),
    /// Quarter turns counterclockwise.
    Rotate(i32),
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

fn main() -> eframe::Result {
    let mut port = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--grpc" {
            port = args.next().and_then(|value| value.parse().ok());
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Avdeck")
            .with_inner_size([420.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Avdeck",
        options,
        Box::new(move |cc| {
            let mut fonts = egui::FontDefinitions::default();
            icons::install(&mut fonts);
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(App {
                session: Session::attach(port, cc.egui_ctx.clone()),
                texture: None,
                frame_size: (0, 0),
                rotation: Rotation::Portrait,
                touching: None,
                pinching: false,
                toolbar_width: 0.0,
                window: None,
                extended_search: None,
                sized_for: None,
                modifiers: egui::Modifiers::default(),
            }))
        }),
    )
}

struct App {
    session: Session,
    texture: Option<TextureHandle>,
    frame_size: (u32, u32),
    rotation: Rotation,
    /// Last device position sent while the mouse button is down.
    touching: Option<(i32, i32)>,
    /// Width the toolbar panel took in the last frame, in points.
    toolbar_width: f32,
    /// Avdeck's own window, once its procedure is hooked.
    window: Option<isize>,
    /// Since when the Extended Controls window is awaited, to dock it once it shows.
    extended_search: Option<std::time::Instant>,
    /// Orientation and DPI the window was last sized for.
    sized_for: Option<(bool, u32)>,
    /// The current drag started with Ctrl: a two-finger pinch.
    pinching: bool,
    /// Modifiers already sent to the device.
    modifiers: egui::Modifiers,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, native: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (frame, natural, status, pid) = {
            let mut state = self.session.state.lock().unwrap();
            if let Some(text) = state.clipboard.take() {
                ctx.copy_text(text);
            }
            (
                state.frame.take(),
                state.natural,
                state.status.clone(),
                state.pid,
            )
        };
        if let Some(frame) = frame {
            let image = ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.pixels,
            );
            match &mut self.texture {
                Some(texture) => texture.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ctx.load_texture("screen", image, TextureOptions::LINEAR))
                }
            }
            self.frame_size = (frame.width, frame.height);
            self.rotation = Rotation::from_proto(frame.rotation);
        }

        let toolbar = egui::Panel::right("toolbar")
            .resizable(false)
            .exact_size(40.0)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    for item in TOOLBAR {
                        let Some((icon, tooltip, action)) = item else {
                            ui.separator();
                            continue;
                        };
                        let button = egui::Button::new(egui::RichText::new(icon).size(20.0))
                            .min_size(vec2(32.0, 32.0))
                            .frame(false);
                        if ui.add(button).on_hover_text(tooltip).clicked() {
                            self.session.send(match action {
                                Action::Key(key) => Input::Key(key),
                                Action::Rotate(quarters) => {
                                    Input::Rotate(self.rotation.quarters() + quarters)
                                }
                                Action::Screenshot => Input::Screenshot,
                                Action::ExtendedControls => {
                                    self.extended_search = Some(std::time::Instant::now());
                                    Input::ExtendedControls
                                }
                            });
                        }
                    }
                });
            });
        self.toolbar_width = toolbar.response.rect.width();

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::BLACK))
            .show(ui, |ui| {
                let (Some(texture), Some(natural)) = (&self.texture, natural) else {
                    ui.centered_and_justified(|ui| ui.label(&status));
                    return;
                };
                let area = ui.max_rect();
                let screen = display::fit(
                    display::Rect::new(area.min.x, area.min.y, area.width(), area.height()),
                    self.frame_size,
                    ctx.pixels_per_point(),
                );
                let rect = egui::Rect::from_min_size(
                    pos2(screen.x, screen.y),
                    vec2(screen.width, screen.height),
                );
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );

                let response =
                    ui.interact(rect, ui.scope_id().with("screen"), Sense::click_and_drag());
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
            });
        self.fit_window(&ctx, native);
        self.dock_extended_controls(&ctx, pid);
    }
}

impl App {
    /// Docks the Extended Controls window beside Avdeck once it appears (within 3 s).
    fn dock_extended_controls(&mut self, ctx: &egui::Context, pid: Option<u32>) {
        let Some(started) = self.extended_search else {
            return;
        };
        let found = pid.and_then(extended::find);
        if let (Some(extended), Some(owner)) = (found, self.window) {
            extended::attach(extended, owner as _);
            self.extended_search = None;
        } else if started.elapsed() > std::time::Duration::from_secs(3) {
            self.extended_search = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Locks resizing to the screen's aspect ratio. Sizes the window when it opens (the device's
    /// longer side at 80% of the monitor height), swaps width and height when the device turns,
    /// and corrects the width after a DPI change.
    fn fit_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if self.frame_size.0 == 0 || self.toolbar_width <= 0.0 {
            return;
        }
        if self.window.is_none()
            && let Ok(handle) = frame.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            window::install(handle.hwnd.get() as _);
            self.window = Some(handle.hwnd.get());
        }
        let pixels_per_point = ctx.pixels_per_point();
        window::set_layout(
            self.frame_size.0 as f32 / self.frame_size.1 as f32,
            (self.toolbar_width * pixels_per_point).round() as i32,
        );

        let layout = (self.rotation.sideways(), pixels_per_point.to_bits());
        if self.sized_for == Some(layout) {
            return;
        }
        let info = ctx.input(|input| input.viewport().clone());
        let (Some(monitor), Some(outer), Some(inner)) =
            (info.monitor_size, info.outer_rect, info.inner_rect)
        else {
            return;
        };
        let current = (inner.width() - self.toolbar_width, inner.height());
        let screen = match self.sized_for {
            // First frame: the device's longer side is 80% of the monitor height.
            None => display::initial_screen_size(monitor.y, 0.8, self.frame_size),
            // Turned: same size on screen, the other way round.
            Some((sideways, _)) if sideways != layout.0 => (current.1, current.0),
            // New DPI: keep the height, fix the width to the aspect ratio.
            Some(_) => (
                current.1 * self.frame_size.0 as f32 / self.frame_size.1 as f32,
                current.1,
            ),
        };
        let size = vec2(screen.0 + self.toolbar_width, screen.1);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        let chrome = outer.size() - inner.size();
        let new_outer = size + chrome;
        let position = if self.sized_for.is_none() {
            // Centered on the monitor the window opened on (the primary one, at the origin).
            pos2(
                (monitor.x - new_outer.x) / 2.0,
                (monitor.y - new_outer.y) / 2.0,
            )
        } else {
            // Turned around its center.
            outer.center() - new_outer / 2.0
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
        self.sized_for = Some(layout);
    }

    /// One finger, or two mirrored through the screen center while pinching.
    fn touch(&self, (x, y): (i32, i32), natural: (u32, u32), pressure: i32) {
        let mut points = vec![(x, y)];
        if self.pinching {
            points.push((natural.0 as i32 - 1 - x, natural.1 as i32 - 1 - y));
        }
        self.session.send(Input::Touch { points, pressure });
    }

    /// Keys, modifiers, clipboard shortcuts and the wheel, as the Qt window forwards them.
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
