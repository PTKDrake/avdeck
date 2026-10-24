//! Runs the app in a winit window drawn by `render`: egui input and output through egui-winit,
//! a repaint whenever egui asks for one. What eframe did for the app, for a single window.

use crate::app::App;
use crate::render::{self, Renderer};
use anyhow::Context;
use egui::{ViewportBuilder, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
use egui_winit::{ActionRequested, accesskit_winit};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// How often a minimized window runs while egui keeps asking for repaints.
const HIDDEN_REPAINT_INTERVAL: Duration = Duration::from_millis(100);
/// A redraw asked of Windows that has not come by then runs anyway.
const MISSED_REDRAW_TIMEOUT: Duration = Duration::from_millis(250);

pub enum UserEvent {
    /// egui asked for a repaint at `when`, during pass `pass`.
    Repaint {
        when: Instant,
        pass: u64,
    },
    AccessKit(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::AccessKit(event)
    }
}

/// Opens the window `builder` describes and runs `create`'s app in it until the window closes.
pub fn run(
    builder: ViewportBuilder,
    create: impl FnOnce(&egui::Context) -> App,
) -> anyhow::Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let mut host = Host {
        builder,
        create: Some(create),
        proxy: event_loop.create_proxy(),
        running: None,
        error: None,
    };
    event_loop.run_app(&mut host)?;
    match host.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Host<F> {
    builder: ViewportBuilder,
    create: Option<F>,
    proxy: EventLoopProxy<UserEvent>,
    running: Option<Running>,
    error: Option<anyhow::Error>,
}

/// Field order is drop order: the window goes last.
struct Running {
    app: App,
    renderer: Renderer,
    state: egui_winit::State,
    ctx: egui::Context,
    info: ViewportInfo,
    /// Clipboard actions and the like from viewport commands, done after painting.
    actions: Vec<ActionRequested>,
    /// When egui wants the next frame.
    repaint_at: Option<Instant>,
    /// A redraw was asked of Windows; past this, run without it.
    redraw_due: Option<Instant>,
    beginning: Instant,
    /// The window starts hidden and shows with its first frame (no white flash).
    shown: bool,
    /// The app let a close request through.
    close: bool,
    hwnd: isize,
    window: Window,
}

impl<F: FnOnce(&egui::Context) -> App> Host<F> {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        ctx.options_mut(|options| options.max_passes = 2.try_into().expect("nonzero"));
        // Fonts of the system for characters Inter and egui's fonts lack.
        ctx.add_font_provider(std::sync::Arc::new(
            egui_system_font_provider::SystemFontProvider::new(),
        ));
        let window =
            egui_winit::create_window(&ctx, event_loop, &self.builder.clone().with_visible(false))?;
        let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
            anyhow::bail!("not a Win32 window");
        };
        let hwnd = handle.hwnd.get();
        set_icon(hwnd);
        let size = window.inner_size();
        let renderer = Renderer::new(
            windows::Win32::Foundation::HWND(hwnd as _),
            [size.width, size.height],
        )
        .context("Direct3D 11 is not available")?;

        let mut info = ViewportInfo::default();
        egui_winit::update_viewport_info(&mut info, &ctx, &window, true);
        let pixels_per_point = egui_winit::pixels_per_point(&ctx, &window);
        ctx.input_mut(|input| {
            input.raw.viewports.insert(ViewportId::ROOT, info.clone());
            input.pixels_per_point = pixels_per_point;
        });
        let proxy = Mutex::new(self.proxy.clone());
        ctx.set_request_repaint_callback(move |request| {
            let _ = proxy.lock().unwrap().send_event(UserEvent::Repaint {
                when: Instant::now() + request.delay,
                pass: request.current_cumulative_pass_nr,
            });
        });
        let mut state = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            event_loop.system_theme(),
            Some(render::MAX_TEXTURE_SIDE),
        );
        state.init_accesskit(event_loop, &window, self.proxy.clone());
        let create = self.create.take().expect("created once");
        let app = create(&ctx);
        let mut running = Running {
            app,
            renderer,
            state,
            ctx,
            info,
            actions: Vec::new(),
            repaint_at: None,
            redraw_due: None,
            beginning: Instant::now(),
            shown: false,
            close: false,
            hwnd,
            window,
        };
        running.run(event_loop)?;
        self.running = Some(running);
        Ok(())
    }

    /// Ends the loop on an error, which `run` returns.
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error.get_or_insert(error);
        event_loop.exit();
    }

    /// Runs the step `step` of the running app, then exits if the app closed or failed.
    fn with_running(
        &mut self,
        event_loop: &ActiveEventLoop,
        step: impl FnOnce(&mut Running, &ActiveEventLoop) -> anyhow::Result<()>,
    ) {
        let Some(running) = &mut self.running else {
            return;
        };
        let result = step(running, event_loop);
        let close = running.close;
        if let Err(error) = result {
            self.fail(event_loop, error);
        } else if close {
            event_loop.exit();
        }
    }
}

impl<F: FnOnce(&egui::Context) -> App> ApplicationHandler<UserEvent> for Host<F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_none()
            && self.create.is_some()
            && let Err(error) = self.start(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        self.with_running(event_loop, |running, event_loop| {
            running.window_event(event_loop, &event)
        });
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        self.with_running(event_loop, |running, _| {
            if let DeviceEvent::MouseMotion { delta } = event
                && running.state.on_mouse_motion(delta)
            {
                running.repaint(Instant::now());
            }
            Ok(())
        });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.with_running(event_loop, |running, event_loop| match event {
            UserEvent::Repaint { when, pass } => {
                // A request from a pass already followed by a new one is stale.
                let current = running.ctx.cumulative_pass_nr_for(ViewportId::ROOT);
                if current == pass || current == pass + 1 {
                    let when = if running.hidden() {
                        when.max(Instant::now() + HIDDEN_REPAINT_INTERVAL)
                    } else {
                        when
                    };
                    running.repaint(when);
                }
                Ok(())
            }
            UserEvent::AccessKit(event) => match event.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    running.ctx.enable_accesskit();
                    running.run(event_loop)
                }
                accesskit_winit::WindowEvent::ActionRequested(request) => {
                    running.state.on_accesskit_action_request(request);
                    running.repaint(Instant::now());
                    Ok(())
                }
                accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                    running.ctx.disable_accesskit();
                    Ok(())
                }
            },
        });
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.with_running(event_loop, |running, event_loop| {
            let now = Instant::now();
            if let Some(at) = running.repaint_at
                && at <= now
            {
                running.repaint_at = None;
                if running.hidden() {
                    // No WM_PAINT for a minimized window: run it here, at a slower pace.
                    running.run(event_loop)?;
                    if let Some(at) = &mut running.repaint_at {
                        *at = (*at).min(now + HIDDEN_REPAINT_INTERVAL);
                    }
                } else {
                    running.window.request_redraw();
                    running
                        .redraw_due
                        .get_or_insert(now + MISSED_REDRAW_TIMEOUT);
                }
            }
            if running.redraw_due.is_some_and(|due| due <= now) {
                running.run(event_loop)?;
            }
            event_loop.set_control_flow(
                match running
                    .repaint_at
                    .into_iter()
                    .chain(running.redraw_due)
                    .min()
                {
                    Some(at) => ControlFlow::WaitUntil(at),
                    None => ControlFlow::Wait,
                },
            );
            Ok(())
        });
    }
}

impl Running {
    fn hidden(&self) -> bool {
        self.window.is_visible() == Some(false) || self.window.is_minimized() == Some(true)
    }

    fn repaint(&mut self, when: Instant) {
        self.repaint_at = Some(self.repaint_at.map_or(when, |at| at.min(when)));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &WindowEvent,
    ) -> anyhow::Result<()> {
        let mut now = false;
        match event {
            WindowEvent::RedrawRequested => return self.run(event_loop),
            WindowEvent::Resized(_) => {
                let size = self.window.inner_size();
                self.renderer
                    .resize([size.width, size.height])
                    .context("cannot resize the window's buffers")?;
                // Draw the new size at once, while Windows still resizes the window.
                now = true;
            }
            WindowEvent::Occluded(occluded) => self.info.occluded = Some(*occluded),
            WindowEvent::CloseRequested => {
                if self.close {
                    return Ok(());
                }
                // The app decides (`ViewportCommand::CancelClose`).
                self.info.events.push(ViewportEvent::Close);
                self.ctx.request_repaint();
            }
            _ => {}
        }
        if self.state.on_window_event(&self.window, event).repaint {
            if now {
                self.run(event_loop)?;
            } else {
                self.repaint(Instant::now());
            }
        }
        Ok(())
    }

    /// One egui pass of the app, then painting.
    fn run(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        self.redraw_due = None;
        egui_winit::update_viewport_info(&mut self.info, &self.ctx, &self.window, false);
        let close_requested = self.info.close_requested();
        // A minimized window shows nothing: no pass (its input waits for the next one), unless
        // the app has to answer a close request.
        let visible = self.info.visible().unwrap_or(true)
            || self.actions.iter().any(ActionRequested::wants_paint);
        if !visible && !close_requested {
            std::thread::sleep(Duration::from_millis(10));
            return Ok(());
        }
        let mut input = self.state.take_egui_input(&self.window);
        input.time = Some(self.beginning.elapsed().as_secs_f64());
        input.viewports = std::iter::once((ViewportId::ROOT, self.info.clone())).collect();

        let (app, hwnd) = (&mut self.app, self.hwnd);
        let output = self.ctx.run_ui(input, |ui| app.ui(ui, hwnd));
        let commands = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|viewport| viewport.commands.clone())
            .unwrap_or_default();
        if close_requested {
            if !commands.contains(&ViewportCommand::CancelClose) {
                self.close = true;
            } else if !visible {
                // The app asks first (closed from the taskbar): show the question.
                self.window.set_minimized(false);
                self.window.focus_window();
            }
        }
        self.info.events.clear();
        self.state.handle_platform_output_with_event_loop(
            &self.window,
            event_loop,
            output.platform_output,
        );

        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        // eframe's clear color; the panels cover it.
        let clear =
            egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32();
        self.renderer
            .paint(
                clear,
                &primitives,
                output.pixels_per_point,
                &output.textures_delta,
            )
            .context("cannot draw the window (graphics driver reset?)")?;
        for action in std::mem::take(&mut self.actions) {
            let event = match action {
                ActionRequested::Cut => egui::Event::Cut,
                ActionRequested::Copy => egui::Event::Copy,
                ActionRequested::Paste => match self.state.clipboard_text() {
                    Some(text) if !text.is_empty() => {
                        egui::Event::Paste(text.replace("\r\n", "\n"))
                    }
                    _ => continue,
                },
                _ => continue,
            };
            self.state.egui_input_mut().events.push(event);
        }
        if !self.shown {
            self.shown = true;
            self.window.set_visible(true);
        }
        egui_winit::process_viewport_commands(
            &self.ctx,
            &mut self.info,
            commands,
            &self.window,
            &mut self.actions,
        );
        if self.hidden() {
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

/// egui's logo as the window and taskbar icon, as eframe set it.
fn set_icon(window: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, GetSystemMetrics, ICON_BIG, ICON_SMALL, LR_DEFAULTCOLOR,
        SM_CXICON, SM_CXSMICON, SendMessageW, WM_SETICON,
    };
    let Ok(image) = image::load_from_memory(include_bytes!("../assets/egui-icon.png")) else {
        return;
    };
    let image = image.to_rgba8();
    for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
        let side = unsafe { GetSystemMetrics(metric) };
        let scaled =
            image::imageops::resize(&image, side as u32, side as u32, image::imageops::Lanczos3);
        let mut png = Vec::new();
        if scaled
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_err()
        {
            return;
        }
        // 0x00030000: the icon format version.
        let icon = unsafe {
            CreateIconFromResourceEx(
                png.as_mut_ptr(),
                png.len() as u32,
                1,
                0x0003_0000,
                side,
                side,
                LR_DEFAULTCOLOR,
            )
        };
        if !icon.is_null() {
            unsafe { SendMessageW(window as _, WM_SETICON, kind as usize, icon as isize) };
        }
    }
}
