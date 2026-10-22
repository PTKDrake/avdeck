//! Starting an emulator headless with gRPC: command line, free ports, and the process itself.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// `CREATE_NO_WINDOW`: the emulator is a console program and would otherwise open a console.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// GPU modes offered for `-gpu`.
pub const GPU_MODES: [&str; 4] = ["host", "auto", "swiftshader_indirect", "angle_indirect"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Ignore the quick-boot snapshot (`-no-snapshot-load`).
    pub cold_boot: bool,
    /// Factory reset (`-wipe-data`); implies a cold boot.
    pub wipe_data: bool,
    /// Do not save the quick-boot snapshot on exit (`-no-snapshot-save`).
    pub no_snapshot_save: bool,
    /// `-gpu` mode; `None` keeps the AVD's `hw.gpu.mode`. A settings file without it means
    /// `host`: quick-boot snapshots saved with `-gpu host` load only with it.
    #[serde(default = "host_gpu")]
    pub gpu: Option<String>,
    /// Anything else, passed as is.
    pub extra: Vec<String>,
}

fn host_gpu() -> Option<String> {
    Some("host".to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ports {
    /// Console port; adb uses `console + 1` and the serial is `emulator-<console>`.
    pub console: u16,
    pub grpc: u16,
}

pub fn arguments(avd: &str, ports: Ports, options: &Options) -> Vec<String> {
    let mut args: Vec<String> = [
        "-avd",
        avd,
        "-port",
        &ports.console.to_string(),
        "-grpc",
        &ports.grpc.to_string(),
        "-grpc-use-token",
        "-qt-hide-window",
    ]
    .map(str::to_owned)
    .into();
    if let Some(gpu) = &options.gpu {
        args.extend(["-gpu".to_owned(), gpu.clone()]);
    }
    if options.wipe_data {
        args.push("-wipe-data".to_owned());
    }
    if options.cold_boot || options.wipe_data {
        args.push("-no-snapshot-load".to_owned());
    }
    if options.no_snapshot_save {
        args.push("-no-snapshot-save".to_owned());
    }
    args.extend(options.extra.iter().cloned());
    args
}

/// The first console port pair (5554/5555, 5556/5557, ... up to 5584, the emulator's range) and
/// gRPC port (from 8554) that are free.
pub fn pick_ports(is_free: impl Fn(u16) -> bool) -> Option<Ports> {
    let console = (5554..=5584)
        .step_by(2)
        .find(|&port| is_free(port) && is_free(port + 1))?;
    let grpc = (8554..8654).find(|&port| is_free(port))?;
    Some(Ports { console, grpc })
}

pub fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Starts `emulator` without a console window, with the resolved SDK environment, writing its
/// output to `log`. The emulator keeps running if Avdeck exits.
pub fn spawn(
    emulator: &Path,
    args: &[String],
    env: &[(&'static str, OsString)],
    log: &Path,
) -> io::Result<Child> {
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let output = File::create(log)?;
    Command::new(emulator)
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .current_dir(emulator.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTS: Ports = Ports {
        console: 5554,
        grpc: 8554,
    };

    #[test]
    fn default_command_line() {
        assert_eq!(
            arguments("Pixel_Tablet", PORTS, &Options::default()).join(" "),
            "-avd Pixel_Tablet -port 5554 -grpc 8554 -grpc-use-token -qt-hide-window"
        );
    }

    #[test]
    fn options_on_the_command_line() {
        let options = Options {
            wipe_data: true,
            no_snapshot_save: true,
            gpu: Some("host".to_owned()),
            extra: vec!["-netdelay".to_owned(), "gprs".to_owned()],
            ..Default::default()
        };
        assert_eq!(
            arguments("P", PORTS, &options)[8..].join(" "),
            "-gpu host -wipe-data -no-snapshot-load -no-snapshot-save -netdelay gprs"
        );
    }

    #[test]
    fn picks_the_first_free_pairs() {
        // 5554 and 5557 are taken: 5554/5555 and 5556/5557 are out, 5558/5559 is free.
        let taken = [5554, 5557, 8554];
        let ports = pick_ports(|port| !taken.contains(&port)).unwrap();
        assert_eq!(
            ports,
            Ports {
                console: 5558,
                grpc: 8555
            }
        );
        assert_eq!(pick_ports(|_| false), None);
    }
}
