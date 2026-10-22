//! Keeps the window at the device's aspect ratio while the user drags its edges, the way the
//! emulator's own window does: `WM_SIZING` is corrected before Windows applies it, so the screen
//! never shows bars. Also keeps the docked Extended Controls window with it (`extended`).
//! Classic subclassing; the app has a single window.

use avdeck_core::display::{Edge, lock_aspect};
use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWLP_WNDPROC, GetClientRect, GetWindowRect, SetWindowLongPtrW, WM_CLOSE,
    WM_COPYDATA, WM_SIZING, WM_WINDOWPOSCHANGED, WNDPROC,
};

/// The window procedure winit installed.
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
/// Screen aspect (width / height) as `f32` bits; 0 while unknown.
static ASPECT: AtomicU32 = AtomicU32::new(0);
/// Pixels of the client area around the screen (toolbar, tab bar), horizontally and vertically.
static EXTRA_X: AtomicI32 = AtomicI32::new(0);
static EXTRA_Y: AtomicI32 = AtomicI32::new(0);
/// Smallest screen height, in physical pixels.
static MIN_HEIGHT: AtomicI32 = AtomicI32::new(0);

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
    let result = unsafe { CallWindowProcW(previous, window, message, wparam, lparam) };
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
