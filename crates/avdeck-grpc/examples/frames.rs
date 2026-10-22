//! Prints every change in the size or rotation of streamed frames and how much of each frame is
//! fully transparent, for debugging what the app shows.
//!
//! ```text
//! cargo run --release -p avdeck-grpc --example frames -- [--grpc 8554] [--seconds 10] [--rotate DEG] [--inline] [--scale F]
//! ```

use anyhow::{Context, Result};
use avdeck_core::running;
use avdeck_grpc::{Auth, Emulator, SharedFrames};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let (mut port, mut seconds, mut rotate) = (8554u16, 10u64, None::<f32>);
    let (mut inline, mut scale) = (false, 1.0f32);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--grpc" => port = args.next().context("--grpc PORT")?.parse()?,
            "--seconds" => seconds = args.next().context("--seconds N")?.parse()?,
            "--inline" => inline = true,
            "--scale" => scale = args.next().context("--scale F")?.parse()?,
            "--rotate" => rotate = Some(args.next().context("--rotate DEG")?.parse()?),
            other => anyhow::bail!("unknown option {other}"),
        }
    }
    let instance = running::list()
        .into_iter()
        .filter(|instance| instance.grpc_port == Some(port))
        .max_by_key(|instance| instance.pid)
        .context("no emulator with that gRPC port")?;
    let auth = match &instance.grpc_token {
        Some(token) => Auth::bearer(token)?,
        None => Auth::none(),
    };
    let mut emulator = Emulator::connect(port, auth).await?;
    if let Some(degrees) = rotate {
        use avdeck_grpc::control::physical_model_value::PhysicalType;
        use avdeck_grpc::control::{ParameterValue, PhysicalModelValue};
        emulator
            .controller
            .set_physical_model(PhysicalModelValue {
                target: PhysicalType::Rotation as i32,
                value: Some(ParameterValue {
                    data: vec![0.0, 0.0, degrees],
                }),
                ..Default::default()
            })
            .await?;
    }
    let frames = SharedFrames::new(2856, 2856)?;
    let mut stream = emulator
        .controller
        .clone()
        .stream_screenshot({
            let size = ((1280.0 * scale) as u32, (2856.0 * scale) as u32);
            let mut request = frames.request(size.0, size.1);
            if inline {
                // Pixels inside the gRPC messages instead of shared memory.
                request.transport = None;
            }
            request
        })
        .await?
        .into_inner();
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let (mut last, mut count) = (None, 0u32);
    let mut transparent = Vec::new();
    while let Ok(Ok(Some(image))) = tokio::time::timeout_at(deadline, stream.message()).await {
        count += 1;
        if let Some(frame) = frames.copy(&image) {
            let total = (frame.pixels.len() / 4).max(1);
            let clear = frame.pixels.chunks(4).filter(|pixel| pixel[3] == 0).count();
            let black = frame
                .pixels
                .chunks(4)
                .filter(|pixel| pixel[..3] == [0, 0, 0])
                .count();
            transparent.push((clear * 100 / total, black * 100 / total));
        }
        let format = image.format.unwrap_or_default();
        let shape = (
            format.width,
            format.height,
            format.rotation.map_or(-1, |rotation| rotation.rotation),
        );
        if last != Some(shape) {
            println!(
                "{:>6} ms frame {count}: {}x{} rotation {}",
                started.elapsed().as_millis(),
                shape.0,
                shape.1,
                shape.2
            );
            last = Some(shape);
        }
    }
    println!("{count} frames, (% alpha 0, % black) per frame: {transparent:?}");
    Ok(())
}
