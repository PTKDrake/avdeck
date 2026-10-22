//! Opens the emulator's Extended Controls window on a pane, for inspecting how it behaves.
//!
//! ```text
//! cargo run --release -p avdeck-grpc --example extended -- [--grpc 8554] [--pane 1] [--close]
//! ```

use anyhow::{Context, Result};
use avdeck_core::running;
use avdeck_grpc::control::theming_style::Style;
use avdeck_grpc::control::{PaneEntry, ThemingStyle};
use avdeck_grpc::{Auth, Emulator};

#[tokio::main]
async fn main() -> Result<()> {
    let (mut port, mut pane, mut close) = (8554u16, 0i32, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--grpc" => port = args.next().context("--grpc PORT")?.parse()?,
            "--pane" => pane = args.next().context("--pane INDEX")?.parse()?,
            "--close" => close = true,
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
    println!("qemu pid {}", instance.pid);
    if close {
        let status = emulator.ui.close_extended_controls(()).await?.into_inner();
        println!("closed, visibility changed: {}", status.visibility_changed);
        return Ok(());
    }
    emulator
        .ui
        .set_ui_theme(ThemingStyle {
            style: Style::Dark as i32,
        })
        .await?;
    let status = emulator
        .ui
        .show_extended_controls(PaneEntry {
            index: pane,
            ..Default::default()
        })
        .await?
        .into_inner();
    println!("shown, visibility changed: {}", status.visibility_changed);
    Ok(())
}
