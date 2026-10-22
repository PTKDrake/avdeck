//! The emulator's Extended Controls window, a Qt window of the qemu process, kept as an owned
//! window of Avdeck: it stays above Avdeck, leaves the taskbar, minimizes with it, opens docked
//! beside it inside the work area and follows it when it moves or resizes.

use avdeck_core::display::{Bounds, dock_beside, follow};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWLP_HWNDPARENT, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsWindow, IsWindowVisible, PostMessageW, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPos, WM_CLOSE,
};
use windows_sys::core::BOOL;

/// Gap between Avdeck and the docked window, in physical pixels.
const GAP: i32 = 8;

struct Docked {
    window: isize,
    /// Owner bounds when the docked window was last placed.
    owner: Bounds,
}

static DOCKED: Mutex<Option<Docked>> = Mutex::new(None);

/// The visible Extended Controls window of the emulator process `pid`
/// (title `<AVD name> - Extended Controls`).
pub fn find(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: HWND,
    }
    unsafe extern "system" fn visit(window: HWND, search: LPARAM) -> BOOL {
        let search = unsafe { &mut *(search as *mut Search) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid != search.pid || unsafe { IsWindowVisible(window) } == 0 {
            return 1;
        }
        let mut title = [0u16; 256];
        let length = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
        if String::from_utf16_lossy(&title[..length.max(0) as usize])
            .ends_with(" - Extended Controls")
        {
            search.found = window;
            return 0;
        }
        1
    }
    let mut search = Search {
        pid,
        found: std::ptr::null_mut(),
    };
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    (!search.found.is_null()).then_some(search.found)
}

/// Makes `owner` own `extended` and docks it beside `owner`.
pub fn attach(extended: HWND, owner: HWND) {
    unsafe { SetWindowLongPtrW(extended, GWLP_HWNDPARENT, owner as isize) };
    let (Some(owner_bounds), Some(work)) = (visible_bounds(owner), work_area(owner)) else {
        return;
    };
    if let Some(side) = visible_bounds(extended) {
        let size = (side.2 - side.0, side.3 - side.1);
        let (x, y) = dock_beside(owner_bounds, size, work, GAP);
        move_visible_to(extended, side, (x, y));
    }
    *DOCKED.lock().unwrap() = Some(Docked {
        window: extended as isize,
        owner: owner_bounds,
    });
}

/// Called after Avdeck's window moved or resized: moves the docked window along.
pub fn owner_moved(owner: HWND) {
    let mut docked = DOCKED.lock().unwrap();
    let Some(state) = docked.as_mut() else {
        return;
    };
    let extended = state.window as HWND;
    if unsafe { IsWindow(extended) } == 0 {
        *docked = None;
        return;
    }
    let Some(after) = visible_bounds(owner) else {
        return;
    };
    let before = std::mem::replace(&mut state.owner, after);
    if unsafe { IsWindowVisible(extended) } == 0 || before == after {
        return;
    }
    if let Some(side) = visible_bounds(extended) {
        let (dx, dy) = follow(before, after, side);
        if (dx, dy) != (0, 0) {
            move_visible_to(extended, side, (side.0 + dx, side.1 + dy));
        }
    }
}

/// Before Avdeck's window closes: gives the window back to the emulator and hides it, so Windows
/// never destroys a window of another process along with its owner.
pub fn detach() {
    if let Some(state) = DOCKED.lock().unwrap().take() {
        let extended = state.window as HWND;
        if unsafe { IsWindow(extended) } != 0 {
            unsafe {
                SetWindowLongPtrW(extended, GWLP_HWNDPARENT, 0);
                PostMessageW(extended, WM_CLOSE, 0, 0);
            }
        }
    }
}

/// What the user sees of a window (DWM frame bounds, without the invisible resize borders).
fn visible_bounds(window: HWND) -> Option<Bounds> {
    let mut rect = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    } == 0;
    ok.then_some((rect.left, rect.top, rect.right, rect.bottom))
}

fn work_area(window: HWND) -> Option<Bounds> {
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let work = (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(info.rcWork)?;
    Some((work.left, work.top, work.right, work.bottom))
}

/// Moves `window` so its visible top-left corner lands on `target`.
fn move_visible_to(window: HWND, visible: Bounds, target: (i32, i32)) {
    let mut outer = RECT::default();
    if unsafe { GetWindowRect(window, &mut outer) } == 0 {
        return;
    }
    let x = target.0 - (visible.0 - outer.left);
    let y = target.1 - (visible.1 - outer.top);
    unsafe {
        SetWindowPos(
            window,
            std::ptr::null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}
