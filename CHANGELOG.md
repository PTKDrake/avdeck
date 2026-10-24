# Changelog

All notable changes to Avdeck. Versions follow [Semantic Versioning](https://semver.org); each
release's section is also its notes on the GitHub Releases page.

## 1.1.0 - 2026-10-24

Avdeck now draws its window with a small Direct3D 11 renderer of its own instead of eframe on
wgpu (Direct3D 12). It looks exactly the same and behaves the same, with a fraction of the memory
and CPU.

### Performance

Measured on a laptop with an AMD Radeon 780M (integrated GPU, so GPU memory is system RAM), at
150% scale, with the same build settings:

| | 1.0.0 | 1.1.0 |
|---|---|---|
| Memory at rest (private bytes) | 146 MB | **31 MB** |
| Memory at rest (working set, Task Manager's figure) | 158 MB | **52 MB** |
| Memory while streaming a phone screen | 173 MB | **59 MB** |
| CPU at rest (Devices page, or a still emulator screen) | ~99% of a core | **~0%** |
| CPU while streaming screen animations | usually ~40% of a core | **~10%** |
| GPU while streaming screen animations | ~2.3% | **~1.7%** |
| `avdeck.exe` | 19.8 MB | **12.6 MB** |

The frame rate of the emulator screen is unchanged (the same number of frames reach the screen
per second).

- **Much less memory.** A Direct3D 12 device and an egui window on it took about 110 MB on this
  GPU even at wgpu's leanest settings; a Direct3D 11 device and swap chain take about 25 MB.
- **No more busy CPU at rest.** With Direct3D 12, AMD's driver kept a thread spinning a full CPU
  core whenever Avdeck drew nothing, on screen or not. It also accounted for most of the CPU
  Avdeck used while streaming.
- **Cheaper emulator frames.** A new frame of the same size now overwrites the existing GPU
  texture; before, every frame created a new texture (about 15 MB for a phone screen).
- **Lean presentation.** A two-buffer flip-model swap chain that waits for the display before
  drawing, with one frame queued, at the display's refresh rate.
- **Smaller download.** 65 fewer crates in the dependency tree, and the executable is a third
  smaller.

### Fixed

- Closing Avdeck while it was minimized (for example from the taskbar) closed it at once: it did
  not ask what to do with running emulators and did not remember the open tabs for next time. It
  now follows the "When closing Avdeck" setting; when that is "Ask", the window comes back to show
  the question.

### Unchanged

- The look: screenshots of every page, menu and dialog and of an emulator tab, in the light and
  dark themes, are identical to 1.0.0 to the pixel. Rendering keeps egui's own shader, blending,
  texture filtering and dithering.
- Window behaviour: aspect-locked resizing, rotation, moving between monitors of different DPI,
  minimizing and restoring, single-instance hand-off (`avdeck --avd NAME`), saved tabs and window
  position.
- Input through egui-winit as before: keyboard and IME (Vietnamese and others), mouse, clipboard,
  drag and drop, and UI Automation / screen readers.
- The window icon is still egui's logo.
- Requirements: Windows 10 or 11 with any Direct3D 11 GPU; without one, Avdeck falls back to
  Windows' software renderer (WARP).

### Under the hood

- `render.rs`: egui on Direct3D 11. The shader is egui-wgpu's `egui.wgsl` as naga translates it to
  HLSL for wgpu's Direct3D 12 backend, compiled with the same FXC flags, so that the pixels match
  exactly.
- `host.rs`: the winit event loop eframe used to run, for one window: egui-winit input and output,
  repaints when egui asks for them, the window hidden until its first frame, minimized windows run
  at most every 100 ms, viewport commands.
- Dependencies: `eframe` (and with it `wgpu`, `naga`, `glow`) replaced by `egui`, `egui-winit`,
  `winit` and `windows`.

## 1.0.0 - 2026-10-23

First release.

- **One window, one tab per emulator.** Emulators run headless (`-qt-hide-window`); their screen
  and input go through the emulator's gRPC API. Tab strip, Ctrl+Tab and Ctrl+1..9; only the visible
  tab streams frames.
- **Live screen over shared memory**, about 35 fps at a few milliseconds of latency, sized to the
  device's aspect ratio and kept there while resizing and rotating, on monitors of any DPI.
- **Input**: mouse as touch (Ctrl+drag to pinch), wheel, keyboard with IME, two-way clipboard,
  drop an APK to install it or other files to copy them to `/sdcard/Download`.
- **Toolbar**: power, volume, rotate, screenshot, back, home, overview, and the emulator's
  Extended Controls window docked beside Avdeck.
- **Devices page**: a card per AVD with its shape, a live thumbnail and its state; start, open,
  stop, cold boot, start with options, snapshots, edit, duplicate, wipe, delete, show in Explorer.
- **Create devices** from every hardware profile and installed system image, and download more
  images with `sdkmanager` from the dialog.
- **Clean stops** over gRPC (the quick-boot snapshot is saved); hung emulators are reported and can
  be force-stopped or force-restarted.
- **Settings**: SDK and tool paths, GPU mode, snapshot saving, what closing a tab or the app does,
  System / Light / Dark theme.
- **Command line and single instance**: `avdeck --avd NAME` starts or attaches to a device, handing
  the request to an open Avdeck.
