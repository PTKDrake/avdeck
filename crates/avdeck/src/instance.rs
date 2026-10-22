//! The command line, and one Avdeck at a time: starting it again (`avdeck --avd NAME`, as
//! `start-emulator.ps1` does) hands the request to the running window through `WM_COPYDATA`
//! and exits.
//!
//! ```text
//! avdeck [--avd NAME] [--cold-boot] [--wipe-data] [--no-snapshot-save] [--gpu MODE]
//!        [-- EMULATOR FLAGS...]
//! ```

use eframe::egui;
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::{HWND, LPARAM, MAX_PATH};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, EnumWindows, GetWindowTextW, GetWindowThreadProcessId, SendMessageW,
    WM_COPYDATA,
};
use windows_sys::core::BOOL;

/// Marks our `WM_COPYDATA` ("AVDK").
const MAGIC: usize = 0x4156_444B;

/// What a start of Avdeck asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// Open (start or attach) this AVD; `None` only brings the window to the front.
    pub avd: Option<String>,
    pub cold_boot: bool,
    pub wipe_data: bool,
    pub no_snapshot_save: bool,
    pub gpu: Option<String>,
    /// Flags after `--`, passed to the emulator as they are.
    pub extra: Vec<String>,
}

impl Request {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut request = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--avd" => request.avd = Some(args.next().ok_or("--avd needs a name")?),
                "--cold-boot" => request.cold_boot = true,
                "--wipe-data" => request.wipe_data = true,
                "--no-snapshot-save" => request.no_snapshot_save = true,
                "--gpu" => request.gpu = Some(args.next().ok_or("--gpu needs a mode")?),
                "--" => request.extra.extend(args.by_ref()),
                other => return Err(format!("unknown option {other}")),
            }
        }
        Ok(request)
    }

    /// The options to start with: the settings' defaults plus what was asked for.
    pub fn options(&self, base: &avdeck_core::launch::Options) -> avdeck_core::launch::Options {
        avdeck_core::launch::Options {
            cold_boot: base.cold_boot || self.cold_boot,
            wipe_data: base.wipe_data || self.wipe_data,
            no_snapshot_save: base.no_snapshot_save || self.no_snapshot_save,
            gpu: self.gpu.clone().or_else(|| base.gpu.clone()),
            extra: base.extra.iter().chain(&self.extra).cloned().collect(),
        }
    }

    /// The arguments again, for handing over (none contains a newline).
    fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(avd) = &self.avd {
            args.extend(["--avd".to_owned(), avd.clone()]);
        }
        for (set, flag) in [
            (self.cold_boot, "--cold-boot"),
            (self.wipe_data, "--wipe-data"),
            (self.no_snapshot_save, "--no-snapshot-save"),
        ] {
            if set {
                args.push(flag.to_owned());
            }
        }
        if let Some(gpu) = &self.gpu {
            args.extend(["--gpu".to_owned(), gpu.clone()]);
        }
        if !self.extra.is_empty() {
            args.push("--".to_owned());
            args.extend(self.extra.iter().cloned());
        }
        args
    }
}

/// Requests received from other starts.
static PENDING: Mutex<Vec<Request>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// Requests handed over since the last call.
pub fn take() -> Vec<Request> {
    std::mem::take(&mut PENDING.lock().unwrap())
}

/// Called by the window procedure for `WM_COPYDATA`; true when it was ours.
pub fn receive(lparam: LPARAM) -> bool {
    // SAFETY: for WM_COPYDATA, lparam points at a COPYDATASTRUCT valid during the call.
    let data = unsafe { &*(lparam as *const COPYDATASTRUCT) };
    if data.dwData != MAGIC {
        return false;
    }
    let bytes = if data.cbData == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) }
    };
    let text = String::from_utf8_lossy(bytes);
    let args = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned);
    if let Ok(request) = Request::parse(args) {
        PENDING.lock().unwrap().push(request);
    }
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
    true
}

/// Hands `request` to an Avdeck window of another process; false when there is none.
pub fn forward(request: &Request) -> bool {
    let Some((window, pid)) = find() else {
        return false;
    };
    // Let the running instance take the foreground.
    unsafe { AllowSetForegroundWindow(pid) };
    let text = request.to_args().join("\n");
    let data = COPYDATASTRUCT {
        dwData: MAGIC,
        cbData: text.len() as u32,
        lpData: text.as_ptr() as *mut _,
    };
    unsafe {
        SendMessageW(
            window,
            WM_COPYDATA,
            0,
            &data as *const COPYDATASTRUCT as LPARAM,
        )
    };
    true
}

/// A top-level window titled "Avdeck" of another `avdeck.exe`.
fn find() -> Option<(HWND, u32)> {
    struct Search {
        own: u32,
        found: Option<(HWND, u32)>,
    }
    unsafe extern "system" fn visit(window: HWND, search: LPARAM) -> BOOL {
        let search = unsafe { &mut *(search as *mut Search) };
        let mut title = [0u16; 64];
        let length = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
        if String::from_utf16_lossy(&title[..length.max(0) as usize]) != "Avdeck" {
            return 1;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid != search.own && is_avdeck(pid) {
            search.found = Some((window, pid));
            return 0;
        }
        1
    }
    let mut search = Search {
        own: std::process::id(),
        found: None,
    };
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    search.found
}

fn is_avdeck(pid: u32) -> bool {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let mut path = [0u16; MAX_PATH as usize];
    let mut length = path.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length)
    } != 0;
    unsafe { windows_sys::Win32::Foundation::CloseHandle(process) };
    ok && String::from_utf16_lossy(&path[..length as usize])
        .to_lowercase()
        .ends_with("\\avdeck.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn parses_the_command_line() {
        let request = Request::parse(args(
            "--avd Pixel_10_Pro_API_37_2 --cold-boot --gpu host -- -no-audio -netdelay gprs",
        ))
        .unwrap();
        assert_eq!(request.avd.as_deref(), Some("Pixel_10_Pro_API_37_2"));
        assert!(request.cold_boot && !request.wipe_data);
        assert_eq!(request.gpu.as_deref(), Some("host"));
        assert_eq!(request.extra, ["-no-audio", "-netdelay", "gprs"]);
        assert_eq!(Request::parse(args("")).unwrap(), Request::default());
        assert!(Request::parse(args("--avd")).is_err());
        assert!(Request::parse(args("--dance")).is_err());
    }

    #[test]
    fn hands_over_the_same_request() {
        let request =
            Request::parse(args("--avd P --wipe-data --no-snapshot-save -- -no-audio")).unwrap();
        assert_eq!(Request::parse(request.to_args()).unwrap(), request);
    }

    #[test]
    fn adds_to_the_default_options() {
        let base = avdeck_core::launch::Options {
            gpu: Some("host".to_owned()),
            extra: vec!["-no-boot-anim".to_owned()],
            ..Default::default()
        };
        let request = Request::parse(args("--avd P --cold-boot -- -no-audio")).unwrap();
        let options = request.options(&base);
        assert!(options.cold_boot);
        assert_eq!(options.gpu.as_deref(), Some("host"));
        assert_eq!(options.extra, ["-no-boot-anim", "-no-audio"]);
    }
}
