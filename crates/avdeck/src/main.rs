//! Avdeck: an Android Emulator manager. Emulators run headless and show in tabs, driven over
//! the emulator's gRPC API.
//!
//! Command line: see `instance`.

#![windows_subsystem = "windows"]

mod app;
mod devices;
mod extended;
mod icons;
mod instance;
mod keyboard;
mod manage;
mod session;
mod settings;
mod tab;
mod theme;
mod window;

use eframe::egui;

fn main() -> eframe::Result {
    let request = match instance::Request::parse(std::env::args().skip(1)) {
        Ok(request) => request,
        Err(error) => {
            // No console (windows subsystem): say it in a message box.
            message_box(&format!(
                "{error}\n\nUsage: avdeck [--avd NAME] [--cold-boot] [--wipe-data] [--no-snapshot-save] [--gpu MODE] [-- EMULATOR FLAGS]"
            ));
            return Ok(());
        }
    };
    if instance::forward(&request) {
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Avdeck")
            .with_app_id("avdeck")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([200.0, 200.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Avdeck",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            instance::set_context(&cc.egui_ctx);
            Ok(Box::new(app::App::new(&cc.egui_ctx, request)))
        }),
    )
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
