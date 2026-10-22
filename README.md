# Avdeck

An Android Emulator manager for Windows, written in Rust. Emulators run headless
(`-qt-hide-window`); Avdeck shows each one in a tab of its own window, with the emulator toolbar,
and drives it over the emulator's gRPC API. See `PLAN.md` (Vietnamese) for the roadmap and the
decisions behind it.

## Use

```text
avdeck [--avd NAME] [--cold-boot] [--wipe-data] [--no-snapshot-save] [--gpu MODE] [-- EMULATOR FLAGS]
```

- Without arguments it opens on the Devices page. With `--avd` it opens that AVD: attaches when
  it already runs with gRPC, starts it otherwise. A second start hands its request to the open
  window and exits.
- Devices: start, open, stop (saving the device state), create, edit, duplicate, wipe, delete,
  snapshots. Settings: SDK, emulator, adb, command-line tools, AVD folder and JDK when the
  environment lacks them; GPU mode; what closing a tab or Avdeck does.
- In a device tab: mouse = touch (Ctrl+drag pinches), keyboard, wheel, clipboard both ways,
  drop an APK to install it (other files go to `/sdcard/Download`), the toolbar on the right.
- Shortcuts: Ctrl+Tab / Ctrl+Shift+Tab, Ctrl+1 (Devices), Ctrl+2..9 (device tabs), Ctrl+,
  (Settings), Ctrl+Shift+D (debug overlay: DPI, sizes, frame rate).
- Settings live in `%APPDATA%\avdeck\config.toml`, emulator logs in
  `%LOCALAPPDATA%\avdeck\logs\<AVD>.log`, screenshots in `Pictures\Avdeck`.

## Build and install

Requires the Rust MSVC toolchain on Windows. `protoc` is vendored, nothing else to install.

```powershell
cargo build --release
cargo test
Copy-Item target\release\avdeck.exe D:\Android\tools\avdeck.exe -Force   # close Avdeck first
```

`D:\Android\start-emulator.ps1 <avd-or-part>` (and the `p10p` profile function) opens AVDs in the
installed `avdeck.exe`.

## Layout

| Path | Purpose |
|---|---|
| `crates/avdeck` | The app (eframe/egui): tabs, device pages, settings, management dialogs, theme, Win32 window handling |
| `crates/avdeck-core` | No UI, no gRPC: SDK/tool resolution (`sdk`), AVDs (`avd`), system images (`images`), config, starting emulators (`launch`), console (`console`), running emulators (`running`), geometry (`display`) |
| `crates/avdeck-grpc` | gRPC client: generated stubs, bearer-token auth, `Emulator`, shared-memory frames (`SharedFrames`) |
| `crates/avdeck-grpc/examples` | `spike` (M0 measurements), `frames` (frame shapes and black frames), `extended` (Extended Controls) |
| `proto/` | Protos vendored from emulator 37.2.12 |
| `docs/grpc-notes.md` | How the emulator's gRPC API behaves on Windows, with measurements |

## History

The repo started as `emu-window`, a daemon that kept the emulator's Qt windows fitted and their
toolbar docked on mixed-DPI setups. Avdeck replaces the Qt window, so the daemon was removed; it
is in the first commit on `master`.
