//! One Avdeck at a time: starting it again (`avdeck --avd NAME`, as `start-emulator.ps1` does)
//! hands the request to the running window through `WM_COPYDATA` and exits.

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

/// AVD ids (or "" to just come to the front) received from other starts.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// Requests handed over since the last call.
pub fn take() -> Vec<String> {
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
    PENDING
        .lock()
        .unwrap()
        .push(String::from_utf8_lossy(bytes).into_owned());
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
    true
}

/// Hands `avd` to an Avdeck window of another process; false when there is none.
pub fn forward(avd: Option<&str>) -> bool {
    let Some((window, pid)) = find() else {
        return false;
    };
    // Let the running instance take the foreground.
    unsafe { AllowSetForegroundWindow(pid) };
    let text = avd.unwrap_or_default();
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
