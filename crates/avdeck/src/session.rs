//! The connection to one emulator, on the app's tokio runtime: streams frames while its tab is
//! shown, at the size the tab displays them, and forwards input. The UI reads `State` and sends
//! `Input`; dropping the `Session` stops all of its tasks.

use anyhow::{Context, Result, bail};
use avdeck_core::display::Rotation;
use avdeck_core::running;
use avdeck_grpc::control::image_format::ImgFormat;
use avdeck_grpc::control::keyboard_event::KeyEventType;
use avdeck_grpc::control::physical_model_value::PhysicalType;
use avdeck_grpc::control::posture::PostureValue;
use avdeck_grpc::control::theming_style::Style;
use avdeck_grpc::control::{
    ClipData, ImageFormat, KeyboardEvent, PaneEntry, ParameterValue, PhysicalModelValue, Posture,
    ThemingStyle, Touch, TouchEvent,
};
use avdeck_grpc::{Auth, Emulator, Frame, SharedFrames};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::task::AbortHandle;

pub enum Input {
    /// Fingers in device pixels, natural orientation; finger `i` has identifier `i + 1`.
    /// Pressure 0 lifts them.
    Touch {
        points: Vec<(i32, i32)>,
        pressure: i32,
    },
    /// A named key such as `GoHome` (see `KeyboardEvent.key`), pressed and released.
    Key(&'static str),
    /// A host key going down or up, as a W3C key value (`a`, `Enter`, `Shift`, ...).
    KeyDown(String),
    KeyUp(String),
    /// Text the keyboard mapping cannot express (accents, IME output): pasted through the
    /// device clipboard.
    Text(String),
    /// Mouse wheel at a device position; `dy` is in 1/120 clicks, positive scrolls up.
    Wheel {
        x: i32,
        y: i32,
        dy: i32,
        rotation: Rotation,
    },
    /// Host clipboard text pasted into the device (Ctrl+V).
    Paste(String),
    /// Turn the device to this many quarter turns counterclockwise from portrait (0..4).
    Rotate(i32),
    /// Fold or unfold a foldable.
    Fold(bool),
    /// Save a PNG to `Pictures\Avdeck`.
    Screenshot,
    ExtendedControls,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the emulator to start and serve gRPC.
    #[default]
    Starting,
    Running,
    /// The connection ended, with the reason.
    Ended(String),
}

#[derive(Default)]
pub struct State {
    pub phase: Phase,
    /// The newest frame not yet uploaded by the UI.
    pub frame: Option<Frame>,
    /// Text the device copied, not yet put on the host clipboard.
    pub clipboard: Option<String>,
    /// The emulator's qemu process, which owns its Qt windows.
    pub pid: Option<u32>,
    pub console_port: Option<u16>,
    /// A message to show over the screen for a few seconds (screenshot saved, APK installed).
    pub notice: Option<String>,
    /// Frames shown per second over the last second, for the debug overlay.
    pub fps: f32,
    /// The connected client, for dialogs that call the emulator (snapshots).
    pub emulator: Option<Emulator>,
}

pub struct Session {
    pub state: Arc<Mutex<State>>,
    input: UnboundedSender<Input>,
    visible: watch::Sender<bool>,
    /// Size in physical pixels the frames are shown at.
    view: watch::Sender<(u32, u32)>,
    tasks: Arc<Mutex<Vec<AbortHandle>>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

/// What a session needs to know about its emulator.
#[derive(Clone)]
pub struct Target {
    pub avd_id: String,
    /// Display size in its natural orientation.
    pub natural: (u32, u32),
    pub grpc_port: u16,
}

impl Session {
    /// Connects to the emulator that serves (or will serve) gRPC on `target.grpc_port`, waiting
    /// up to three minutes for it to start.
    pub fn start(runtime: &Handle, target: Target, ctx: egui::Context) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let (input, inputs) = unbounded_channel();
        let (visible, visible_rx) = watch::channel(false);
        let (view, view_rx) = watch::channel(target.natural);
        let tasks = Arc::new(Mutex::new(Vec::new()));
        let context = Shared {
            state: state.clone(),
            ctx: ctx.clone(),
            tasks: tasks.clone(),
            runtime: runtime.clone(),
        };
        let main = runtime.spawn(async move {
            let result = run(&context, target, inputs, visible_rx, view_rx).await;
            let reason = match result {
                Ok(()) => "The emulator closed the connection.".to_owned(),
                Err(error) => format!("{error:#}"),
            };
            context.state.lock().unwrap().phase = Phase::Ended(reason);
            context.ctx.request_repaint();
        });
        tasks.lock().unwrap().push(main.abort_handle());
        Self {
            state,
            input,
            visible,
            view,
            tasks,
        }
    }

    pub fn send(&self, input: Input) {
        let _ = self.input.send(input);
    }

    /// Streams frames only while the tab is shown.
    pub fn set_visible(&self, visible: bool) {
        self.visible.send_if_modified(|current| {
            let changed = *current != visible;
            *current = visible;
            changed
        });
    }

    /// The physical size frames are shown at; the stream follows once it settles.
    pub fn set_view_size(&self, size: (u32, u32)) {
        self.view.send_if_modified(|current| {
            let changed = *current != size;
            *current = size;
            changed
        });
    }
}

struct Shared {
    state: Arc<Mutex<State>>,
    ctx: egui::Context,
    tasks: Arc<Mutex<Vec<AbortHandle>>>,
    runtime: Handle,
}

impl Shared {
    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        let handle = self.runtime.spawn(task);
        self.tasks.lock().unwrap().push(handle.abort_handle());
    }
}

/// The live discovery entry for `port`, once the emulator has written it.
async fn discover(port: u16) -> Result<running::Running> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Some(found) = running::live()
            .into_iter()
            .filter(|running| running.grpc_port == Some(port))
            .max_by_key(|running| running.pid)
        {
            return Ok(found);
        }
        if Instant::now() > deadline {
            bail!("The emulator did not start serving gRPC on port {port} within 3 minutes.");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Closes the emulator's Extended Controls window if it is open (within 2 s).
pub async fn close_extended_controls(mut emulator: Emulator) {
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        emulator.ui.close_extended_controls(()),
    )
    .await;
}

/// Asks the emulator on gRPC `port` to shut down cleanly, like closing its window: it saves the
/// quick-boot snapshot unless started with `-no-snapshot-save`. Returns once the request is out
/// (the call itself often never answers: the emulator goes away).
pub async fn shutdown(port: u16, token: Option<String>) {
    use avdeck_grpc::control::VmRunState;
    use avdeck_grpc::control::vm_run_state::RunState;
    let auth = match token.as_deref().map(Auth::bearer) {
        Some(Ok(auth)) => auth,
        _ => Auth::none(),
    };
    let Ok(Ok(mut emulator)) =
        tokio::time::timeout(Duration::from_secs(3), Emulator::connect(port, auth)).await
    else {
        return;
    };
    let request = VmRunState {
        state: RunState::Shutdown as i32,
    };
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        emulator.controller.set_vm_state(request),
    )
    .await;
}

async fn connect(port: u16, instance: &running::Running) -> Result<Emulator> {
    let auth = match &instance.grpc_token {
        Some(token) => Auth::bearer(token)?,
        None => Auth::none(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match Emulator::connect(port, auth.clone()).await {
            // A hung emulator accepts the connection but never answers.
            Ok(mut emulator) => match tokio::time::timeout(
                Duration::from_secs(5),
                emulator.controller.get_status(()),
            )
            .await
            .unwrap_or_else(|_| Err(tonic::Status::deadline_exceeded("no answer in 5 s")))
            {
                Ok(_) => return Ok(emulator),
                // The message of an auth failure echoes the token: report only the code.
                Err(status) if Instant::now() > deadline => {
                    bail!(
                        "The emulator does not answer (gRPC on port {port}: {:?}). It may be \
                         hung: stop it from Devices, then Force stop.",
                        status.code()
                    )
                }
                Err(_) => {}
            },
            Err(error) if Instant::now() > deadline => {
                return Err(error).context(format!("cannot connect to gRPC port {port}"));
            }
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn run(
    context: &Shared,
    target: Target,
    inputs: UnboundedReceiver<Input>,
    visible: watch::Receiver<bool>,
    view: watch::Receiver<(u32, u32)>,
) -> Result<()> {
    let instance = discover(target.grpc_port).await?;
    {
        let mut state = context.state.lock().unwrap();
        state.pid = Some(instance.pid);
        state.console_port = instance.console_port;
    }
    let emulator = connect(target.grpc_port, &instance).await?;
    {
        let mut state = context.state.lock().unwrap();
        state.phase = Phase::Running;
        state.emulator = Some(emulator.clone());
    }
    // An Extended Controls window left open by an Avdeck that ended abruptly: close it, it
    // would float on its own, undocked.
    context.spawn(close_extended_controls(emulator.clone()));
    context.ctx.request_repaint();

    // Phone and tablet AVDs have no mouse device in the guest, so `injectWheel` does nothing;
    // the wheel becomes slow one-finger drags instead.
    let (wheel, wheels) = unbounded_channel();
    context.spawn(scroll(emulator.clone(), wheels, target.natural));

    // Device clipboard to host.
    let mut clipboard_client = emulator.clone();
    let clipboard_state = context.state.clone();
    let clipboard_ctx = context.ctx.clone();
    context.spawn(async move {
        let Ok(response) = clipboard_client.controller.stream_clipboard(()).await else {
            return;
        };
        let mut stream = response.into_inner();
        while let Ok(Some(clip)) = stream.message().await {
            if !clip.text.is_empty() {
                clipboard_state.lock().unwrap().clipboard = Some(clip.text);
                clipboard_ctx.request_repaint();
            }
        }
    });

    context.spawn(forward_input(
        emulator.clone(),
        inputs,
        wheel,
        target.avd_id.clone(),
        context.state.clone(),
        context.ctx.clone(),
    ));

    stream_frames(context, emulator, target.natural, visible, view).await
}

/// Frames while the tab is visible, restarted at a new size once the view size settles.
async fn stream_frames(
    context: &Shared,
    emulator: Emulator,
    natural: (u32, u32),
    mut visible: watch::Receiver<bool>,
    mut view: watch::Receiver<(u32, u32)>,
) -> Result<()> {
    // Black frames are held back: during animations some AVDs (the Pixel 10 Pro image) stream
    // every other frame black. One that is still the newest after HOLD is real and is shown.
    const HOLD: Duration = Duration::from_millis(250);
    // A resize restarts the stream only once the size has been still this long.
    const SETTLE: Duration = Duration::from_millis(300);
    let mut shown: Vec<Instant> = Vec::new();
    loop {
        while !*visible.borrow_and_update() {
            if visible.changed().await.is_err() {
                return Ok(());
            }
        }
        let size = *view.borrow_and_update();
        // A fresh region for every stream: restarting at another size on the region of the
        // stream just cancelled crashes emulator 37.2.12 while frames are flowing (boot
        // animation), with any `-gpu` mode.
        let frames = SharedFrames::new(natural.0, natural.1)?;
        // Never more than the device's own resolution: the emulator only scales down.
        // The emulator scales down to fit this box, never above the device's resolution.
        let request = frames.request(size.0, size.1);
        let mut stream = emulator
            .controller
            .clone()
            .stream_screenshot(request)
            .await?
            .into_inner();
        let mut held: Option<(Frame, Instant)> = None;
        let mut resized: Option<Instant> = None;
        loop {
            let hold_until = held.as_ref().map(|(_, since)| *since + HOLD);
            let settle_until = resized.map(|since| since + SETTLE);
            tokio::select! {
                changed = visible.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    if !*visible.borrow() {
                        break;
                    }
                }
                changed = view.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    resized = Some(Instant::now());
                }
                () = sleep_until(settle_until) => break,
                () = sleep_until(hold_until) => {
                    if let Some((frame, _)) = held.take() {
                        publish(context, frame, &mut shown);
                    }
                }
                message = stream.message() => {
                    let Some(image) = message? else {
                        return Ok(());
                    };
                    let Some(frame) = frames.copy(&image) else {
                        continue;
                    };
                    if frame.is_black() {
                        let since = held.map_or_else(Instant::now, |(_, since)| since);
                        held = Some((frame, since));
                        continue;
                    }
                    held = None;
                    publish(context, frame, &mut shown);
                }
            }
        }
    }
}

fn publish(context: &Shared, frame: Frame, shown: &mut Vec<Instant>) {
    let now = Instant::now();
    shown.retain(|at| now - *at < Duration::from_secs(1));
    shown.push(now);
    let mut state = context.state.lock().unwrap();
    state.frame = Some(frame);
    state.fps = shown.len() as f32;
    drop(state);
    context.ctx.request_repaint();
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

async fn forward_input(
    mut sender: Emulator,
    mut inputs: UnboundedReceiver<Input>,
    wheel: UnboundedSender<(i32, i32, i32, Rotation)>,
    avd_id: String,
    state: Arc<Mutex<State>>,
    ctx: egui::Context,
) {
    while let Some(input) = inputs.recv().await {
        let _ = match input {
            Input::Touch { points, pressure } => sender
                .controller
                .send_touch(TouchEvent {
                    touches: points
                        .iter()
                        .zip(1..)
                        .map(|(&(x, y), identifier)| Touch {
                            x,
                            y,
                            identifier,
                            pressure,
                            ..Default::default()
                        })
                        .collect(),
                    display: 0,
                })
                .await
                .map(drop),
            Input::Key(key) => send_key(&mut sender, key.to_owned(), KeyEventType::Keypress).await,
            Input::KeyDown(key) => send_key(&mut sender, key, KeyEventType::Keydown).await,
            Input::KeyUp(key) => send_key(&mut sender, key, KeyEventType::Keyup).await,
            Input::Wheel { x, y, dy, rotation } => {
                let _ = wheel.send((x, y, dy, rotation));
                Ok(())
            }
            Input::Paste(text) | Input::Text(text) => paste(&mut sender, text).await,
            Input::Rotate(quarters) => sender
                .controller
                .set_physical_model(PhysicalModelValue {
                    target: PhysicalType::Rotation as i32,
                    value: Some(ParameterValue {
                        // Degrees around x, y, z; the screen turns around z.
                        data: vec![
                            0.0,
                            0.0,
                            [0.0, 90.0, 180.0, -90.0][quarters.rem_euclid(4) as usize],
                        ],
                    }),
                    ..Default::default()
                })
                .await
                .map(drop),
            Input::Fold(folded) => sender
                .controller
                .set_posture(Posture {
                    value: if folded {
                        PostureValue::PostureClosed
                    } else {
                        PostureValue::PostureOpened
                    } as i32,
                })
                .await
                .map(drop),
            Input::Screenshot => match sender
                .controller
                .get_screenshot(ImageFormat {
                    format: ImgFormat::Png as i32,
                    ..Default::default()
                })
                .await
            {
                Ok(image) => {
                    if let Ok(path) = save_screenshot(&avd_id, &image.into_inner().image) {
                        state.lock().unwrap().notice = Some(format!("Saved {}", path.display()));
                        ctx.request_repaint();
                    }
                    Ok(())
                }
                Err(status) => Err(status),
            },
            Input::ExtendedControls => {
                // Match Avdeck's dark theme; the window is docked by the UI once it shows.
                let _ = sender
                    .ui
                    .set_ui_theme(ThemingStyle {
                        style: Style::Dark as i32,
                    })
                    .await;
                sender
                    .ui
                    .show_extended_controls(PaneEntry::default())
                    .await
                    .map(drop)
            }
        };
    }
}

/// Puts `text` on the device clipboard and presses Ctrl+V.
async fn paste(emulator: &mut Emulator, text: String) -> Result<(), tonic::Status> {
    emulator.controller.set_clipboard(ClipData { text }).await?;
    send_key(emulator, "Control".to_owned(), KeyEventType::Keydown).await?;
    send_key(emulator, "v".to_owned(), KeyEventType::Keypress).await?;
    send_key(emulator, "Control".to_owned(), KeyEventType::Keyup).await
}

fn save_screenshot(avd_id: &str, png: &[u8]) -> Result<std::path::PathBuf> {
    let directory = std::path::PathBuf::from(
        std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?,
    )
    .join("Pictures")
    .join("Avdeck");
    std::fs::create_dir_all(&directory)?;
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let path = directory.join(format!("{avd_id}-{seconds}.png"));
    std::fs::write(&path, png)?;
    Ok(path)
}

async fn send_key(
    emulator: &mut Emulator,
    key: String,
    event_type: KeyEventType,
) -> Result<(), tonic::Status> {
    emulator
        .controller
        .send_key(KeyboardEvent {
            event_type: event_type as i32,
            key,
            ..Default::default()
        })
        .await
        .map(drop)
}

/// Turns wheel clicks into a drag with its own finger: down at the pointer, moved with every
/// click, lifted once the wheel has been still for a moment. The finger stops before it lifts,
/// so Android sees no fling.
async fn scroll(
    mut emulator: Emulator,
    mut wheels: UnboundedReceiver<(i32, i32, i32, Rotation)>,
    natural: (u32, u32),
) {
    use tokio::time::{sleep, timeout};
    // Distinct from the mouse finger (1) and the pinch finger (2).
    const FINGER: i32 = 10;
    const IDLE: Duration = Duration::from_millis(120);
    const STEPS: i32 = 4;
    let pixels_per_click = natural.0.max(natural.1) as i32 / 15;
    let inside = |(x, y): (i32, i32)| {
        (0..natural.0 as i32).contains(&x) && (0..natural.1 as i32).contains(&y)
    };

    while let Some((x, y, first, rotation)) = wheels.recv().await {
        let (down_x, down_y) = rotation.screen_down();
        let start = (x, y);
        let mut finger = start;
        let _ = finger_touch(&mut emulator, FINGER, finger, 1).await;
        let mut pending = first;
        loop {
            // Positive dy scrolls up: the content moves down, so the finger moves down.
            let distance = pending * pixels_per_click / 120;
            for _ in 0..STEPS {
                let next = (
                    finger.0 + down_x * distance / STEPS,
                    finger.1 + down_y * distance / STEPS,
                );
                if !inside(next) {
                    // Out of room: lift and start again where the pointer is.
                    let _ = finger_touch(&mut emulator, FINGER, finger, 0).await;
                    finger = start;
                    let _ = finger_touch(&mut emulator, FINGER, finger, 1).await;
                    break;
                }
                finger = next;
                let _ = finger_touch(&mut emulator, FINGER, finger, 1).await;
                sleep(Duration::from_millis(8)).await;
            }
            match timeout(IDLE, wheels.recv()).await {
                Ok(Some((_, _, dy, _))) => pending = dy,
                Ok(None) => return,
                Err(_) => break,
            }
        }
        let _ = finger_touch(&mut emulator, FINGER, finger, 0).await;
    }
}

async fn finger_touch(
    emulator: &mut Emulator,
    identifier: i32,
    (x, y): (i32, i32),
    pressure: i32,
) -> Result<(), tonic::Status> {
    emulator
        .controller
        .send_touch(TouchEvent {
            touches: vec![Touch {
                x,
                y,
                identifier,
                pressure,
                ..Default::default()
            }],
            display: 0,
        })
        .await
        .map(drop)
}
