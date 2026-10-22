//! Asks an emulator to shut down cleanly over gRPC (`setVmState(SHUTDOWN)`), like closing its
//! window: it saves the quick-boot snapshot unless started with `-no-snapshot-save`.
//!
//! ```text
//! cargo run --release -p avdeck-grpc --example shutdown -- 8555
//! ```

use anyhow::{Context, Result};
use avdeck_core::running;
use avdeck_grpc::control::VmRunState;
use avdeck_grpc::control::vm_run_state::RunState;
use avdeck_grpc::{Auth, Emulator};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let port: u16 = std::env::args().nth(1).context("gRPC port")?.parse()?;
    let instance = running::live()
        .into_iter()
        .find(|instance| instance.grpc_port == Some(port))
        .context("no running emulator with that gRPC port")?;
    let auth = match &instance.grpc_token {
        Some(token) => Auth::bearer(token)?,
        None => Auth::none(),
    };
    let mut emulator = Emulator::connect(port, auth).await?;
    // The call may not return: the emulator goes away while answering.
    let call = emulator.controller.set_vm_state(VmRunState {
        state: RunState::Shutdown as i32,
    });
    match tokio::time::timeout(Duration::from_secs(3), call).await {
        Ok(Ok(_)) => println!("shutdown accepted"),
        Ok(Err(status)) => println!("shutdown answered {:?}", status.code()),
        Err(_) => println!("no answer within 3 s (expected while it shuts down)"),
    }
    println!("qemu pid {}", instance.pid);
    Ok(())
}
