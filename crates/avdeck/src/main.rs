//! Avdeck: an Android Emulator manager. Emulators run headless and show in tabs, driven over
//! the emulator's gRPC API.
//!
//! Command line: see `instance`.

#![windows_subsystem = "windows"]

mod app;
mod devices;
mod extended;
mod host;
mod icons;
mod instance;
mod keyboard;
mod manage;
mod render;
mod session;
mod settings;
mod tab;
mod theme;
mod window;

fn main() {
    let request = match instance::Request::parse(std::env::args().skip(1)) {
        Ok(request) => request,
        Err(error) => {
            // No console (windows subsystem): say it in a message box.
            message_box(&format!(
                "{error}\n\nUsage: avdeck [--avd NAME] [--cold-boot] [--wipe-data] [--no-snapshot-save] [--gpu MODE] [-- EMULATOR FLAGS]"
            ));
            return;
        }
    };
    if instance::forward(&request) {
        return;
    }
    let window = egui::ViewportBuilder::default()
        .with_title("Avdeck")
        .with_app_id("avdeck")
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([720.0, 520.0]);
    let result = host::run(window, move |ctx| {
        theme::install(ctx);
        instance::set_context(ctx);
        app::App::new(ctx, request)
    });
    if let Err(error) = result {
        message_box(&format!("Avdeck stopped: {error:#}"));
    }
}

fn message_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MessageBoxW};
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Avdeck"));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_ICONWARNING,
        )
    };
}
