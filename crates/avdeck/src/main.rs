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
use eframe::egui_wgpu::{self, wgpu};
use std::sync::Arc;

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
            .with_min_inner_size([720.0, 520.0]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(low_memory_setup()),
            ..Default::default()
        },
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

/// egui's wgpu setup, cut down for memory. Measured on a Radeon 780M, where GPU memory is system
/// RAM (private bytes of Avdeck streaming a phone screen):
/// - The allocator reserves GPU memory in small blocks; by default it takes 128-256 MB blocks for
///   device memory and 64-128 MB for host memory, far more than a UI uses (Vulkan: 440 -> 262 MB).
/// - Only Direct3D 12 when there is a hardware DX12 GPU, instead of Vulkan, which wgpu prefers
///   (262 -> 145 MB). Enabling every backend and choosing among them would not do: each one
///   loads its driver. `WGPU_BACKEND` still chooses the backend.
fn low_memory_setup() -> egui_wgpu::WgpuSetupCreateNew {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    let device = setup.device_descriptor.clone();
    setup.device_descriptor = Arc::new(move |adapter| wgpu::DeviceDescriptor {
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..device(adapter)
    });
    if std::env::var_os("WGPU_BACKEND").is_none() && has_dx12_gpu() {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
    }
    setup
}

/// A GPU (not the WARP software rasterizer) drives Direct3D 12.
fn has_dx12_gpu() -> bool {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
        return false;
    };
    runtime
        .block_on(instance.enumerate_adapters(wgpu::Backends::DX12))
        .iter()
        .any(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu)
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
