//! Keeps the window at the device's aspect ratio while the user drags its edges, the way the
//! emulator's own window does: `WM_SIZING` is corrected before Windows applies it, so the screen
//! never shows bars. Also keeps the docked Extended Controls window with it (`extended`), keeps
//! the cursor on the point it grabbed when a drag changes the DPI, and places windows on the
//! monitors. Classic subclassing; the app has a single window.

use avdeck_core::display::{
    Bounds, Edge, centered, keep_grab, keep_inside, lock_aspect, most_overlapped,
};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromWindow,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWLP_WNDPROC, GetClientRect, GetCursorPos, GetWindowRect,
    MONITORINFOF_PRIMARY, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW,
    SetWindowPos, WM_CLOSE, WM_COPYDATA, WM_DPICHANGED, WM_EXITSIZEMOVE, WM_MOVING, WM_SIZING,
    WM_WINDOWPOSCHANGED, WNDPROC,
};
use windows_sys::core::BOOL;

/// The window procedure winit installed.
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
/// Screen aspect (width / height) as `f32` bits; 0 while unknown.
static ASPECT: AtomicU32 = AtomicU32::new(0);
/// Pixels of the client area around the screen (toolbar, tab bar), horizontally and vertically.
static EXTRA_X: AtomicI32 = AtomicI32::new(0);
static EXTRA_Y: AtomicI32 = AtomicI32::new(0);
/// Smallest screen height, in physical pixels.
static MIN_HEIGHT: AtomicI32 = AtomicI32::new(0);
/// The user is dragging the window (not resizing it).
static MOVING: AtomicBool = AtomicBool::new(false);

pub fn install(window: HWND) {
    if PREVIOUS.load(Ordering::Relaxed) != 0 {
        return;
    }
    let previous =
        unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, procedure as *const () as isize) };
    PREVIOUS.store(previous, Ordering::Relaxed);
}

/// Locks resizing to `aspect` for the screen area, which is the client area minus `extra`
/// physical pixels and at least `min_height` tall; an aspect of 0 unlocks.
pub fn set_layout(aspect: f32, extra: (i32, i32), min_height: i32) {
    ASPECT.store(aspect.to_bits(), Ordering::Relaxed);
    MIN_HEIGHT.store(min_height, Ordering::Relaxed);
    EXTRA_X.store(extra.0, Ordering::Relaxed);
    EXTRA_Y.store(extra.1, Ordering::Relaxed);
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: PREVIOUS holds the procedure returned by SetWindowLongPtrW (or 0 = None).
    let previous: WNDPROC = unsafe { std::mem::transmute(PREVIOUS.load(Ordering::Relaxed)) };
    if message == WM_COPYDATA && crate::instance::receive(lparam) {
        return 1;
    }
    if message == WM_CLOSE {
        crate::extended::detach();
    }
    match message {
        WM_MOVING => MOVING.store(true, Ordering::Relaxed),
        WM_EXITSIZEMOVE => MOVING.store(false, Ordering::Relaxed),
        _ => {}
    }
    // SAFETY: for WM_DPICHANGED, lparam points at the suggested window RECT.
    let suggested = (message == WM_DPICHANGED && MOVING.load(Ordering::Relaxed))
        .then(|| unsafe { *(lparam as *const RECT) });
    let result = unsafe { CallWindowProcW(previous, window, message, wparam, lparam) };
    if let Some(suggested) = suggested {
        unsafe { keep_cursor(window, suggested) };
    }
    if message == WM_SIZING {
        // SAFETY: for WM_SIZING, lparam points at the proposed window RECT.
        unsafe { correct(window, wparam as u32, &mut *(lparam as *mut RECT)) };
    }
    if message == WM_WINDOWPOSCHANGED {
        crate::extended::owner_moved(window);
    }
    result
}

unsafe fn correct(window: HWND, edge: u32, rect: &mut RECT) {
    let aspect = f32::from_bits(ASPECT.load(Ordering::Relaxed));
    let Some(edge) = Edge::from_wmsz(edge) else {
        return;
    };
    if aspect <= 0.0 {
        return;
    }
    let (mut outer, mut client) = (RECT::default(), RECT::default());
    if unsafe { GetWindowRect(window, &mut outer) == 0 || GetClientRect(window, &mut client) == 0 }
    {
        return;
    }
    let frame = (
        (outer.right - outer.left) - client.right,
        (outer.bottom - outer.top) - client.bottom,
    );
    let (left, top, right, bottom) = lock_aspect(
        edge,
        (rect.left, rect.top, rect.right, rect.bottom),
        (
            frame.0 + EXTRA_X.load(Ordering::Relaxed),
            frame.1 + EXTRA_Y.load(Ordering::Relaxed),
        ),
        aspect,
        MIN_HEIGHT.load(Ordering::Relaxed),
    );
    *rect = RECT {
        left,
        top,
        right,
        bottom,
    };
}

/// A window dragged onto a monitor of another DPI: winit sizes it for the new DPI at the top of
/// the rectangle Windows suggested, then nudges it until most of it is on the new monitor, which
/// slides it from under the cursor (a tall window shrinking toward the cursor is often still
/// mostly on the old monitor). Put it back where Windows suggested, for its actual size.
unsafe fn keep_cursor(window: HWND, suggested: RECT) {
    let (mut cursor, mut outer) = (POINT::default(), RECT::default());
    if unsafe { GetCursorPos(&mut cursor) == 0 || GetWindowRect(window, &mut outer) == 0 } {
        return;
    }
    let (x, y) = keep_grab(
        bounds(suggested),
        (cursor.x, cursor.y),
        (outer.right - outer.left, outer.bottom - outer.top),
    );
    if (x, y) != (outer.left, outer.top) {
        move_to(window, (x, y));
    }
}

/// What to do with the window once it has its first size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settle {
    /// Move it inside the work area of its monitor.
    Inside,
    /// Center it on the primary monitor.
    Center,
}

/// Before the window first shows: puts it where it was `saved` (outer bounds) if that is still
/// on a monitor, else centers it on the primary monitor. Returns what to do once it is sized.
///
/// `AVDECK_WINDOW_POSITION=X,Y` puts its top-left corner there instead, monitors or not:
/// automated UI tests keep their window off-screen.
pub fn place(window: HWND, saved: Option<Bounds>) -> Option<Settle> {
    if let Some(position) = forced_position() {
        move_to(window, position);
        return None;
    }
    let monitors: Vec<Bounds> = monitors().iter().map(|monitor| monitor.bounds).collect();
    match saved {
        Some(saved) if most_overlapped(saved, &monitors).is_some() => {
            move_to(window, (saved.0, saved.1));
            Some(Settle::Inside)
        }
        _ => {
            settle(window, Settle::Center);
            Some(Settle::Center)
        }
    }
}

fn forced_position() -> Option<(i32, i32)> {
    let value = std::env::var("AVDECK_WINDOW_POSITION").ok()?;
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Moves the window as `how` says, never leaving any of it outside the work area.
pub fn settle(window: HWND, how: Settle) {
    let Some(visible) = visible_bounds(window).or_else(|| outer_bounds(window)) else {
        return;
    };
    let size = (visible.2 - visible.0, visible.3 - visible.1);
    let (target, work) = match how {
        Settle::Inside => ((visible.0, visible.1), work_area(window)),
        Settle::Center => {
            let primary = monitors().into_iter().find(|monitor| monitor.primary);
            let Some(work) = primary.map(|monitor| monitor.work) else {
                return;
            };
            (centered(size, work), Some(work))
        }
    };
    let Some(work) = work else {
        return;
    };
    let (dx, dy) = keep_inside(
        (target.0, target.1, target.0 + size.0, target.1 + size.1),
        work,
    );
    let target = (target.0 + dx, target.1 + dy);
    if target != (visible.0, visible.1) {
        move_visible_to(window, visible, target);
    }
}

/// A monitor in physical pixels: all of it, and its work area (without the taskbar).
pub struct Monitor {
    pub bounds: Bounds,
    pub work: Bounds,
    pub primary: bool,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
        let list = unsafe { &mut *(list as *mut Vec<Monitor>) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            list.push(Monitor {
                bounds: bounds(info.rcMonitor),
                work: bounds(info.rcWork),
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        1
    }
    let mut list = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(visit),
            &mut list as *mut Vec<Monitor> as LPARAM,
        )
    };
    list
}

fn bounds(rect: RECT) -> Bounds {
    (rect.left, rect.top, rect.right, rect.bottom)
}

/// The window's bounds with its invisible resize borders (`GetWindowRect`).
pub fn outer_bounds(window: HWND) -> Option<Bounds> {
    let mut rect = RECT::default();
    (unsafe { GetWindowRect(window, &mut rect) } != 0).then_some(bounds(rect))
}

/// What the user sees of a window (DWM frame bounds, without the invisible resize borders).
pub fn visible_bounds(window: HWND) -> Option<Bounds> {
    let mut rect = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    } == 0;
    ok.then_some(bounds(rect))
}

/// Work area of the monitor the window is on, or nearest to.
pub fn work_area(window: HWND) -> Option<Bounds> {
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(bounds(info.rcWork))
}

/// Moves `window` so its visible top-left corner lands on `target`.
pub fn move_visible_to(window: HWND, visible: Bounds, target: (i32, i32)) {
    let Some(outer) = outer_bounds(window) else {
        return;
    };
    move_to(
        window,
        (
            target.0 - (visible.0 - outer.0),
            target.1 - (visible.1 - outer.1),
        ),
    );
}

/// Moves `window`'s outer top-left corner to `target`.
fn move_to(window: HWND, target: (i32, i32)) {
    unsafe {
        SetWindowPos(
            window,
            std::ptr::null_mut(),
            target.0,
            target.1,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}
