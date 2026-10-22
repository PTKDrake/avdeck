//! Avdeck: an Android Emulator manager. Emulators run headless and show in tabs, driven over
//! the emulator's gRPC API.
//!
//! ```text
//! avdeck [--avd NAME]     open (start or attach) that AVD
//! ```

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
    let mut start = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--avd" {
            start = args.next();
        }
    }
    if instance::forward(start.as_deref()) {
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
            Ok(Box::new(app::App::new(&cc.egui_ctx, start)))
        }),
    )
}
