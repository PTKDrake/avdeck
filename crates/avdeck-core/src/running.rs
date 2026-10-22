//! Emulators running right now, from the discovery files every emulator writes to
//! `%TEMP%\avd\running\pid_<pid>.ini` (pid of `qemu-system-x86_64.exe`). An emulator started
//! with `-grpc <port> -grpc-use-token` lists its port and its own gRPC token there; that token
//! is not the console token in `%USERPROFILE%\.emulator_console_auth_token`.

use std::fmt;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Eq)]
pub struct Running {
    pub pid: u32,
    /// AVD name as passed to `-avd`, e.g. `Pixel_10_Pro_API_37_2`.
    pub avd_id: String,
    pub avd_dir: Option<PathBuf>,
    /// Console port; the adb serial is `emulator-<console_port>`.
    pub console_port: Option<u16>,
    pub grpc_port: Option<u16>,
    pub grpc_token: Option<String>,
}

impl fmt::Debug for Running {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Running")
            .field("pid", &self.pid)
            .field("avd_id", &self.avd_id)
            .field("avd_dir", &self.avd_dir)
            .field("console_port", &self.console_port)
            .field("grpc_port", &self.grpc_port)
            .field(
                "grpc_token",
                &self.grpc_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

pub fn discovery_dir() -> Option<PathBuf> {
    let temp = std::env::var_os("TEMP").or_else(|| std::env::var_os("TMP"))?;
    Some(PathBuf::from(temp).join("avd").join("running"))
}

/// One discovery file; `None` when it names no AVD.
pub fn parse(pid: u32, text: &str) -> Option<Running> {
    let values = crate::ini::parse(text);
    let port = |key: &str| values.get(key).and_then(|value| value.parse().ok());
    Some(Running {
        pid,
        avd_id: values.get("avd.id")?.clone(),
        avd_dir: values.get("avd.dir").map(PathBuf::from),
        console_port: port("port.serial"),
        grpc_port: port("grpc.port"),
        grpc_token: values
            .get("grpc.token")
            .filter(|token| !token.is_empty())
            .cloned(),
    })
}

/// Every discovery file. Files of crashed emulators can linger, so callers that need a live
/// emulator still have to connect to it.
pub fn list() -> Vec<Running> {
    let Some(entries) = discovery_dir().and_then(|dir| fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let pid = name
                .to_str()?
                .strip_prefix("pid_")?
                .strip_suffix(".ini")?
                .parse()
                .ok()?;
            parse(pid, &fs::read_to_string(entry.path()).ok()?)
        })
        .collect()
}

/// The process exists and has not exited.
pub fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let mut code = 0;
    let ok = unsafe { GetExitCodeProcess(process, &mut code) } != 0;
    unsafe { CloseHandle(process) };
    ok && code == STILL_ACTIVE as u32
}

/// Ends the process at once, without letting it save anything. For an emulator that no longer
/// answers its console or gRPC.
pub fn terminate(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if process.is_null() {
        return false;
    }
    let ok = unsafe { TerminateProcess(process, 1) } != 0;
    unsafe { CloseHandle(process) };
    ok
}

/// Discovery files of emulators that are still running.
pub fn live() -> Vec<Running> {
    list()
        .into_iter()
        .filter(|running| alive(running.pid))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written by emulator 37.2.12 for `-port 5554 -grpc 8554 -grpc-use-token` (token shortened).
    const DISCOVERY: &str = r#"launcher.dir=D:\Android\SDK\emulator
emulator.build=16428233
port.serial=5554
emulator.version=37.2.12.0
avd.id=Pixel_10_Pro_API_37_2
port.adb=5555
avd.name=Pixel 10 Pro API 37 2
avd.dir=D:\Android\avd\Pixel_10_Pro_API_37_2.avd
cmdline="D:\\Android\\SDK\\emulator\\qemu\\windows-x86_64\\qemu-system-x86_64.exe" "-avd" "Pixel_10_Pro_API_37_2" "-grpc" "8554" "-grpc-use-token"
grpc.token=abc+def/ghi=
grpc.port=8554
grpc.allowlist=D:\Android\SDK\emulator\lib\emulator_access.json
"#;

    #[test]
    fn parses_a_grpc_discovery_file() {
        let running = parse(34604, DISCOVERY).unwrap();
        assert_eq!(running.avd_id, "Pixel_10_Pro_API_37_2");
        assert_eq!(
            running.avd_dir,
            Some(PathBuf::from(r"D:\Android\avd\Pixel_10_Pro_API_37_2.avd"))
        );
        assert_eq!(running.console_port, Some(5554));
        assert_eq!(running.grpc_port, Some(8554));
        // Base64 tokens end in '=', which must survive splitting on the first '='.
        assert_eq!(running.grpc_token.as_deref(), Some("abc+def/ghi="));
        assert!(!format!("{running:?}").contains("abc"));
    }

    #[test]
    fn this_process_is_alive_and_a_bogus_pid_is_not() {
        assert!(alive(std::process::id()));
        assert!(!alive(0xFFFF_FFF0));
    }

    #[test]
    fn grpc_fields_are_optional() {
        let running = parse(1, "avd.id=Pixel_Tablet\nport.serial=5556\n").unwrap();
        assert_eq!(running.console_port, Some(5556));
        assert_eq!(running.grpc_port, None);
        assert_eq!(running.grpc_token, None);
        assert_eq!(parse(1, "port.serial=5556\n"), None);
    }
}
