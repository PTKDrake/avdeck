//! The connection to one emulator, on a background tokio runtime: streams frames into shared
//! memory and forwards input. The UI reads the latest frame and sends `Input`.

use anyhow::{Context, Result};
use avdeck_core::display::Rotation;
use avdeck_core::{avd, running};
use avdeck_grpc::control::image_format::ImgFormat;
use avdeck_grpc::control::keyboard_event::KeyEventType;
use avdeck_grpc::control::physical_model_value::PhysicalType;
use avdeck_grpc::control::theming_style::Style;
use avdeck_grpc::control::{
    ClipData, ImageFormat, KeyboardEvent, PaneEntry, ParameterValue, PhysicalModelValue,
    ThemingStyle, Touch, TouchEvent,
};
use avdeck_grpc::{Auth, Emulator, Frame, SharedFrames};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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
    /// Save a PNG to `Pictures\Avdeck`.
    Screenshot,
    ExtendedControls,
}

#[derive(Default)]
pub struct State {
    /// The newest frame not yet uploaded by the UI.
    pub frame: Option<Frame>,
    /// Panel size in its natural orientation, from the AVD's `config.ini`.
    pub natural: Option<(u32, u32)>,
    pub status: String,
    /// Text the device copied, not yet put on the host clipboard.
    pub clipboard: Option<String>,
    /// The emulator's qemu process, which owns its Qt windows.
    pub pid: Option<u32>,
}

pub struct Session {
    pub state: Arc<Mutex<State>>,
    input: UnboundedSender<Input>,
}

impl Session {
    /// Attaches to the emulator serving gRPC on `port`, or the first one that does.
    pub fn attach(port: Option<u16>, ctx: eframe::egui::Context) -> Self {
        let state = Arc::new(Mutex::new(State {
            status: "connecting…".to_owned(),
            ..Default::default()
        }));
        let (input, inputs) = unbounded_channel();
        let shared = state.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Runtime::new()
                .map_err(Into::into)
                .and_then(|runtime| runtime.block_on(run(port, &shared, &ctx, inputs)));
            let status = match result {
                Ok(()) => "the emulator closed the stream".to_owned(),
                Err(error) => format!("{error:#}"),
            };
            shared.lock().unwrap().status = status;
            ctx.request_repaint();
        });
        Self { state, input }
    }

    pub fn send(&self, input: Input) {
        let _ = self.input.send(input);
    }
}

async fn run(
    port: Option<u16>,
    state: &Arc<Mutex<State>>,
    ctx: &eframe::egui::Context,
    mut inputs: UnboundedReceiver<Input>,
) -> Result<()> {
    let instance = running::list()
        .into_iter()
        .filter(|instance| instance.grpc_port.is_some())
        .filter(|instance| port.is_none() || instance.grpc_port == port)
        .max_by_key(|instance| instance.pid)
        .context("no running emulator with gRPC (start one with -grpc <port> -grpc-use-token)")?;
    let grpc_port = instance.grpc_port.unwrap_or_default();
    let auth = match &instance.grpc_token {
        Some(token) => Auth::bearer(token)?,
        None => Auth::none(),
    };
    let emulator = Emulator::connect(grpc_port, auth).await?;
    let natural = avd::panel_size(&avd::avd_homes(None), &instance.avd_id)
        .map(|(width, height)| (width as u32, height as u32))
        .context("cannot read the AVD's display size")?;
    {
        let mut state = state.lock().unwrap();
        state.natural = Some(natural);
        state.pid = Some(instance.pid);
        state.status = format!("{} (gRPC {grpc_port})", instance.avd_id);
    }

    // Phone and tablet AVDs have no mouse device in the guest, so `injectWheel` does nothing;
    // the wheel becomes slow one-finger drags instead.
    let (wheel, wheels) = unbounded_channel();
    tokio::spawn(scroll(emulator.clone(), wheels, natural));

    // Device clipboard to host.
    let mut clipboard_client = emulator.clone();
    let clipboard_state = state.clone();
    let clipboard_ctx = ctx.clone();
    tokio::spawn(async move {
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

    let mut sender = emulator.clone();
    let avd_id = instance.avd_id.clone();
    tokio::spawn(async move {
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
                Input::Key(key) => sender
                    .controller
                    .send_key(KeyboardEvent {
                        event_type: KeyEventType::Keypress as i32,
                        key: key.to_owned(),
                        ..Default::default()
                    })
                    .await
                    .map(drop),
                Input::KeyDown(key) => send_key(&mut sender, key, KeyEventType::Keydown).await,
                Input::KeyUp(key) => send_key(&mut sender, key, KeyEventType::Keyup).await,
                Input::Wheel { x, y, dy, rotation } => {
                    let _ = wheel.send((x, y, dy, rotation));
                    Ok(())
                }
                Input::Paste(text) => {
                    let set = sender.controller.set_clipboard(ClipData { text }).await;
                    if set.is_ok() {
                        let _ = send_key(&mut sender, "Control".to_owned(), KeyEventType::Keydown)
                            .await;
                        let _ = send_key(&mut sender, "v".to_owned(), KeyEventType::Keypress).await;
                        let _ =
                            send_key(&mut sender, "Control".to_owned(), KeyEventType::Keyup).await;
                    }
                    set.map(drop)
                }
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
                Input::Screenshot => match sender
                    .controller
                    .get_screenshot(ImageFormat {
                        format: ImgFormat::Png as i32,
                        ..Default::default()
                    })
                    .await
                {
                    Ok(image) => {
                        let _ = save_screenshot(&avd_id, &image.into_inner().image);
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
    });

    let frames = SharedFrames::new(natural.0, natural.1)?;
    let mut stream = emulator
        .controller
        .clone()
        .stream_screenshot(frames.request(0, 0))
        .await?
        .into_inner();
    // Black frames are held back: during animations the emulator streams every other frame
    // black. One that is still the newest after HOLD is real (screen off) and is shown.
    const HOLD: std::time::Duration = std::time::Duration::from_millis(250);
    let mut held: Option<(Frame, std::time::Instant)> = None;
    loop {
        let next = match &held {
            Some((_, since)) => {
                match tokio::time::timeout(HOLD.saturating_sub(since.elapsed()), stream.message())
                    .await
                {
                    Ok(next) => next?,
                    Err(_) => {
                        let (frame, _) = held.take().expect("held frame");
                        state.lock().unwrap().frame = Some(frame);
                        ctx.request_repaint();
                        continue;
                    }
                }
            }
            None => stream.message().await?,
        };
        let Some(image) = next else {
            return Ok(());
        };
        let Some(frame) = frames.copy(&image) else {
            continue;
        };
        if frame.is_black() {
            let since = held.map_or_else(std::time::Instant::now, |(_, since)| since);
            held = Some((frame, since));
            continue;
        }
        held = None;
        state.lock().unwrap().frame = Some(frame);
        ctx.request_repaint();
    }
}

fn save_screenshot(avd_id: &str, png: &[u8]) -> Result<()> {
    let directory = std::path::PathBuf::from(
        std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?,
    )
    .join("Pictures")
    .join("Avdeck");
    std::fs::create_dir_all(&directory)?;
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    std::fs::write(directory.join(format!("{avd_id}-{seconds}.png")), png)?;
    Ok(())
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
    use std::time::Duration;
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
