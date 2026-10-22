//! M0 spike: launches (or attaches to) an emulator with gRPC, measures screenshot streaming over
//! gRPC bytes and over shared memory (MMAP transport), and checks touch and key input.
//!
//! ```text
//! cargo run --release -p avdeck-grpc --example spike -- --avd Pixel_10_Pro_API_37_2 --launch
//! ```
//!
//! Options: `--avd NAME`, `--launch`, `--no-token` (launch without `-grpc-use-token`), `--console 5554`, `--grpc 8554`,
//! `--seconds 10`, `--scale 1.0`, `--modes grpc,shm,file`, `--tap X,Y`, `--out DIR`.

use anyhow::{Context, Result, bail};
use avdeck_core::running;
use avdeck_grpc::control::image_format::ImgFormat;
use avdeck_grpc::control::image_transport::TransportChannel;
use avdeck_grpc::control::keyboard_event::KeyEventType;
use avdeck_grpc::control::{ImageFormat, ImageTransport, KeyboardEvent, Touch, TouchEvent};
use avdeck_grpc::{Auth, Emulator};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, GetCurrentProcess, GetProcessTimes};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Pixels inside the gRPC `Image` message.
    Grpc,
    /// Named shared memory (`CreateFileMappingW` on the page file), created as `SHM_<handle>`.
    Shm,
    /// A file mapping passed as a `file:///` URL.
    File,
}

struct Options {
    avd: Option<String>,
    launch: bool,
    token: bool,
    console: u16,
    grpc: u16,
    seconds: u64,
    scale: f64,
    modes: Vec<Mode>,
    tap: Option<(i32, i32)>,
    out: PathBuf,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            avd: None,
            launch: false,
            token: true,
            console: 5554,
            grpc: 8554,
            seconds: 10,
            scale: 1.0,
            modes: vec![Mode::Grpc, Mode::Shm, Mode::File],
            tap: None,
            out: PathBuf::from(r"D:\Android\emulator-diagnostics\spike"),
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--avd" => options.avd = Some(value()?),
                "--launch" => options.launch = true,
                "--no-token" => options.token = false,
                "--console" => options.console = value()?.parse()?,
                "--grpc" => options.grpc = value()?.parse()?,
                "--seconds" => options.seconds = value()?.parse()?,
                "--scale" => options.scale = value()?.parse()?,
                "--out" => options.out = value()?.into(),
                "--modes" => {
                    options.modes = value()?
                        .split(',')
                        .map(|mode| match mode {
                            "grpc" => Ok(Mode::Grpc),
                            "shm" => Ok(Mode::Shm),
                            "file" => Ok(Mode::File),
                            other => bail!("unknown mode {other}"),
                        })
                        .collect::<Result<_>>()?;
                }
                "--tap" => {
                    let text = value()?;
                    let (x, y) = text.split_once(',').context("--tap X,Y")?;
                    options.tap = Some((x.parse()?, y.parse()?));
                }
                other => bail!("unknown option {other}"),
            }
        }
        Ok(options)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = Options::parse()?;
    fs::create_dir_all(&options.out)?;
    if options.launch {
        launch(&options)?;
    }
    let started = Instant::now();
    let mut emulator = connect(options.grpc).await?;
    println!("connected after {:.1}s", started.elapsed().as_secs_f64());
    wait_booted(&mut emulator).await?;
    println!("booted after {:.1}s", started.elapsed().as_secs_f64());
    print_discovery();
    if let Some(avd) = &options.avd {
        let homes = avdeck_core::avd::avd_homes(None);
        println!(
            "config.ini panel: {:?}",
            avdeck_core::avd::panel_size(&homes, avd)
        );
    }

    // Single screenshots: PNG as delivered, RGBA saved as-is to check the row order.
    let timer = Instant::now();
    let png = emulator
        .controller
        .get_screenshot(image_format(ImgFormat::Png, 0, 0, None))
        .await?
        .into_inner();
    println!(
        "getScreenshot(PNG): {} bytes in {} ms",
        png.image.len(),
        timer.elapsed().as_millis()
    );
    fs::write(options.out.join("screenshot.png"), &png.image)?;

    let timer = Instant::now();
    let raw = emulator
        .controller
        .get_screenshot(image_format(ImgFormat::Rgba8888, 0, 0, None))
        .await?
        .into_inner();
    let raw_format = raw.format.clone().unwrap_or_default();
    println!(
        "getScreenshot(RGBA8888): {}x{} rotation {:?}, {} bytes in {} ms",
        raw_format.width,
        raw_format.height,
        raw_format.rotation.map(|rotation| rotation.rotation),
        raw.image.len(),
        timer.elapsed().as_millis()
    );
    save_png(
        &options.out.join("screenshot-rgba.png"),
        raw_format.width,
        raw_format.height,
        &raw.image,
    )?;
    let display = (raw_format.width as i32, raw_format.height as i32);
    let width = (raw_format.width as f64 * options.scale).round() as u32;
    let height = (raw_format.height as f64 * options.scale).round() as u32;

    for &mode in &options.modes {
        if let Err(error) = measure(&emulator, mode, width, height, display, &options).await {
            println!("{mode:?}: FAILED: {error:#}");
        }
    }

    check_input(&mut emulator, display, &options).await
}

fn launch(options: &Options) -> Result<()> {
    let avd = options.avd.as_deref().context("--launch needs --avd")?;
    let sdk = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .find_map(std::env::var_os)
        .context("ANDROID_HOME is not set")?;
    let log = File::create(options.out.join("emulator.log"))?;
    let mut command = Command::new(PathBuf::from(sdk).join("emulator").join("emulator.exe"));
    command
        .args(["-avd", avd, "-qt-hide-window", "-gpu", "host"])
        .args(["-port", &options.console.to_string()])
        .args(["-grpc", &options.grpc.to_string()]);
    if options.token {
        command.arg("-grpc-use-token");
    }
    // Without its own (hidden) console the emulator would share ours.
    let child = command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    println!("launched emulator pid {} ({avd})", child.id());
    Ok(())
}

/// Waits for the emulator's discovery file (for the token, if any) and for gRPC to answer.
async fn connect(port: u16) -> Result<Emulator> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let error = match auth(port) {
            None => "no discovery file for this port yet".to_owned(),
            Some(auth) => match Emulator::connect(port, auth?).await {
                Ok(mut emulator) => match emulator.controller.get_status(()).await {
                    Ok(_) => return Ok(emulator),
                    // The message of an auth failure echoes the token.
                    Err(status) => format!("{:?}", status.code()),
                },
                Err(error) => format!("{error}"),
            },
        };
        if Instant::now() > deadline {
            bail!("cannot reach gRPC port {port}: {error}");
        }
        sleep(Duration::from_secs(1)).await;
    }
}

/// Bearer token from the discovery file; none when the emulator runs without `-grpc-use-token`.
fn auth(port: u16) -> Option<Result<Auth>> {
    let running = running::list()
        .into_iter()
        .filter(|running| running.grpc_port == Some(port))
        .max_by_key(|running| running.pid)?;
    Some(match running.grpc_token.as_deref() {
        Some(token) => Auth::bearer(token).map_err(Into::into),
        None => Ok(Auth::none()),
    })
}

async fn wait_booted(emulator: &mut Emulator) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let status = emulator.controller.get_status(()).await?.into_inner();
        if status.booted {
            println!(
                "emulator {} uptime {} ms, booted",
                status.version, status.uptime
            );
            return Ok(());
        }
        if Instant::now() > deadline {
            bail!("not booted after 300 s");
        }
        sleep(Duration::from_secs(1)).await;
    }
}

/// Discovery files of running emulators (risk R4); `Running` redacts the token.
fn print_discovery() {
    for running in running::list() {
        println!("discovery: {running:?}");
    }
}

fn image_format(
    format: ImgFormat,
    width: u32,
    height: u32,
    transport: Option<&Shared>,
) -> ImageFormat {
    ImageFormat {
        format: format as i32,
        width,
        height,
        transport: transport.map(|shared| ImageTransport {
            channel: TransportChannel::Mmap as i32,
            handle: shared.handle.clone(),
        }),
        ..Default::default()
    }
}

/// Streams RGBA frames for `options.seconds` while swiping, so the screen keeps changing.
async fn measure(
    emulator: &Emulator,
    mode: Mode,
    width: u32,
    height: u32,
    display: (i32, i32),
    options: &Options,
) -> Result<()> {
    let length = width as usize * height as usize * 4;
    let shared = match mode {
        Mode::Grpc => None,
        Mode::Shm => Some(Shared::memory(
            &format!("avdeck-spike-{}", std::process::id()),
            length,
        )?),
        Mode::File => Some(Shared::file(&options.out.join("frames.bin"), length)?),
    };
    let request = image_format(ImgFormat::Rgba8888, width, height, shared.as_ref());
    let mut stream = emulator
        .controller
        .clone()
        .stream_screenshot(request)
        .await?
        .into_inner();
    let input = tokio::spawn(swipe_forever(emulator.clone(), display));

    let cpu_before = cpu_time();
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(options.seconds);
    let (mut frames, mut empty, mut bytes) = (0u32, 0u32, 0usize);
    let (mut first_seq, mut last_seq) = (None, 0u32);
    let mut latencies = Vec::new();
    let mut arrivals = Vec::new();
    let mut last: Option<(u32, u32, Vec<u8>)> = None;
    let result = loop {
        let image = match tokio::time::timeout_at(deadline, stream.message()).await {
            Err(_) | Ok(Ok(None)) => break Ok(()),
            Ok(Err(status)) => break Err(status),
            Ok(Ok(Some(image))) => image,
        };
        let format = image.format.unwrap_or_default();
        if format.width == 0 {
            empty += 1;
            continue;
        }
        frames += 1;
        arrivals.push(Instant::now());
        bytes += image.image.len();
        first_seq.get_or_insert(image.seq);
        last_seq = image.seq;
        if image.timestamp_us > 0 {
            latencies.push(now_us().saturating_sub(image.timestamp_us) as f64 / 1000.0);
        }
        last = Some((format.width, format.height, image.image));
    };
    let wall = started.elapsed().as_secs_f64();
    let cpu = cpu_time() - cpu_before;
    input.abort();
    result?;

    latencies.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        latencies
            .get(((latencies.len() as f64 - 1.0) * p).round() as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    // Busiest one-second window: what the transport sustains while the screen animates.
    let peak = arrivals
        .iter()
        .enumerate()
        .map(|(start, &at)| {
            arrivals[start..]
                .iter()
                .take_while(|&&later| later - at < Duration::from_secs(1))
                .count()
        })
        .max()
        .unwrap_or(0);
    let dropped = first_seq.map_or(0, |first| (last_seq - first + 1).saturating_sub(frames));
    println!(
        "{mode:?} {width}x{height}: {frames} frames in {wall:.1}s = {:.1} fps (peak {peak} in 1s), {} empty, ~{dropped} skipped by seq, \
         {:.1} MB/s over gRPC, latency p50 {:.1} ms p95 {:.1} ms, client CPU {:.0}% of one core",
        frames as f64 / wall,
        empty,
        bytes as f64 / wall / 1e6,
        percentile(0.5),
        percentile(0.95),
        cpu / wall * 100.0,
    );

    let Some((frame_width, frame_height, pixels)) = last else {
        bail!("no frames received");
    };
    let frame_length = frame_width as usize * frame_height as usize * 4;
    let pixels = match &shared {
        None => pixels,
        Some(shared) => {
            if !pixels.is_empty() {
                println!("    note: {} bytes still sent over gRPC", pixels.len());
            }
            shared.bytes()[..frame_length.min(shared.length)].to_vec()
        }
    };
    let lit = pixels
        .chunks(4)
        .filter(|pixel| pixel[..3] != [0, 0, 0])
        .count();
    println!(
        "    last frame {frame_width}x{frame_height}, {:.0}% non-black pixels",
        lit as f64 * 100.0 / (pixels.len() / 4).max(1) as f64
    );
    save_png(
        &options
            .out
            .join(format!("stream-{mode:?}.png").to_lowercase()),
        frame_width,
        frame_height,
        &pixels,
    )
}

/// Opens the app drawer and goes home again, over and over.
async fn swipe_forever(mut emulator: Emulator, (width, height): (i32, i32)) {
    loop {
        let from = (width / 2, height * 85 / 100);
        let to = (width / 2, height * 35 / 100);
        let _ = swipe(&mut emulator, from, to).await;
        sleep(Duration::from_millis(700)).await;
        let _ = key(&mut emulator, "GoHome").await;
        sleep(Duration::from_millis(700)).await;
    }
}

async fn swipe(emulator: &mut Emulator, from: (i32, i32), to: (i32, i32)) -> Result<Duration> {
    const STEPS: i32 = 12;
    let mut slowest = Duration::ZERO;
    for step in 0..=STEPS {
        let x = from.0 + (to.0 - from.0) * step / STEPS;
        let y = from.1 + (to.1 - from.1) * step / STEPS;
        slowest = slowest.max(touch(emulator, x, y, 1).await?);
        sleep(Duration::from_millis(16)).await;
    }
    slowest = slowest.max(touch(emulator, to.0, to.1, 0).await?);
    Ok(slowest)
}

/// Sends one touch point; pressure 0 lifts the finger. Returns the call's round trip.
async fn touch(emulator: &mut Emulator, x: i32, y: i32, pressure: i32) -> Result<Duration> {
    let started = Instant::now();
    emulator
        .controller
        .send_touch(TouchEvent {
            touches: vec![Touch {
                x,
                y,
                identifier: 1,
                pressure,
                ..Default::default()
            }],
            display: 0,
        })
        .await?;
    Ok(started.elapsed())
}

async fn key(emulator: &mut Emulator, key: &str) -> Result<Duration> {
    let started = Instant::now();
    emulator
        .controller
        .send_key(KeyboardEvent {
            event_type: KeyEventType::Keypress as i32,
            key: key.to_owned(),
            ..Default::default()
        })
        .await?;
    Ok(started.elapsed())
}

/// Home, app drawer, then an optional tap, with a screenshot after each step.
async fn check_input(
    emulator: &mut Emulator,
    (width, height): (i32, i32),
    options: &Options,
) -> Result<()> {
    let home = key(emulator, "GoHome").await?;
    sleep(Duration::from_secs(1)).await;
    save_screenshot(emulator, &options.out.join("input-1-home.png")).await?;

    let from = (width / 2, height * 85 / 100);
    let to = (width / 2, height * 35 / 100);
    let swipe = swipe(emulator, from, to).await?;
    sleep(Duration::from_secs(1)).await;
    save_screenshot(emulator, &options.out.join("input-2-drawer.png")).await?;
    println!(
        "input: sendKey {} ms, slowest sendTouch {} ms",
        home.as_millis(),
        swipe.as_millis()
    );

    if let Some((x, y)) = options.tap {
        touch(emulator, x, y, 1).await?;
        sleep(Duration::from_millis(50)).await;
        touch(emulator, x, y, 0).await?;
        sleep(Duration::from_secs(2)).await;
        save_screenshot(emulator, &options.out.join("input-3-tap.png")).await?;
        println!("tapped {x},{y}");
    }
    println!("screenshots in {}", options.out.display());
    Ok(())
}

async fn save_screenshot(emulator: &mut Emulator, path: &Path) -> Result<()> {
    let image = emulator
        .controller
        .get_screenshot(image_format(ImgFormat::Png, 0, 0, None))
        .await?
        .into_inner();
    fs::write(path, image.image)?;
    Ok(())
}

fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() < expected {
        bail!(
            "{}: {} bytes for {width}x{height}",
            path.display(),
            rgba.len()
        );
    }
    let mut encoder = png::Encoder::new(File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(&rgba[..expected])?;
    Ok(())
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// User plus kernel time of this process, in seconds.
fn cpu_time() -> f64 {
    let mut times = [FILETIME::default(); 4];
    let [creation, exit, kernel, user] = &mut times;
    unsafe { GetProcessTimes(GetCurrentProcess(), creation, exit, kernel, user) };
    let seconds = |time: &FILETIME| {
        (((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64) as f64 / 1e7
    };
    seconds(&times[2]) + seconds(&times[3])
}

/// A memory region the emulator writes frames into; `handle` is what goes into `ImageTransport`.
struct Shared {
    handle: String,
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    length: usize,
    _file: Option<File>,
}

impl Shared {
    fn memory(name: &str, length: usize) -> Result<Self> {
        // On Windows the emulator opens the mapping `SHM_<handle>`, not `<handle>` itself.
        let wide: Vec<u16> = OsStr::new(&format!("SHM_{name}"))
            .encode_wide()
            .chain([0])
            .collect();
        let mapping = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                (length as u64 >> 32) as u32,
                length as u32,
                wide.as_ptr(),
            )
        };
        Self::map(name.to_owned(), mapping, length, None)
    }

    fn file(path: &Path, length: usize) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(length as u64)?;
        let mapping = unsafe {
            CreateFileMappingW(
                file.as_raw_handle() as HANDLE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                0,
                std::ptr::null(),
            )
        };
        let url = format!("file:///{}", path.display().to_string().replace('\\', "/"));
        Self::map(url, mapping, length, Some(file))
    }

    fn map(handle: String, mapping: HANDLE, length: usize, file: Option<File>) -> Result<Self> {
        if mapping.is_null() {
            bail!(
                "CreateFileMappingW failed: {}",
                std::io::Error::last_os_error()
            );
        }
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, length) };
        if view.Value.is_null() {
            let error = std::io::Error::last_os_error();
            unsafe { CloseHandle(mapping) };
            bail!("MapViewOfFile failed: {error}");
        }
        Ok(Self {
            handle,
            mapping,
            view,
            length,
            _file: file,
        })
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, self.length) }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.view);
            CloseHandle(self.mapping);
        }
    }
}
