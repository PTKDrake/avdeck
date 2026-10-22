# Avdeck

**Avdeck** is an Android Emulator manager for Windows, written in Rust. It replaces the emulator's
own Qt window with a single, modern window: every running emulator gets a tab, and a Devices page
lists, creates and manages your virtual devices.

Emulators run headless (`-qt-hide-window`). Avdeck shows their screen and sends input through the
emulator's gRPC API, so the device looks and responds as it does in the stock window, with none of
its window problems on mixed-DPI, multi-monitor setups.

## Features

- **One window, one tab per emulator.** Switch with the tab strip, Ctrl+Tab or Ctrl+1..9. Only the
  visible tab streams frames, at the size it is shown.
- **Live screen over shared memory**: about 35 fps at a few milliseconds of latency. The window
  opens with the device's longer side at 80% of the monitor height and keeps the device's aspect
  ratio while it is resized or rotated.
- **Full input**: the mouse acts as touch (Ctrl+drag to pinch), the wheel scrolls, the keyboard
  types (including Vietnamese and other IME input), and the clipboard is synced both ways. Drop an
  APK onto the screen to install it; other files are copied to `/sdcard/Download`.
- **Toolbar**: power, volume, rotate, screenshot, back, home, overview, and the emulator's Extended
  Controls window (GPS, sensors, battery, ...), docked beside Avdeck.
- **Devices page**: a card for every AVD with its shape, a live thumbnail and its state (running in a
  tab, in the background, outside Avdeck, stopping, not responding). Start, open, stop, cold boot,
  start with options, snapshots, edit, duplicate, wipe, delete, show in Explorer.
- **Create devices** from every hardware profile (grouped and searchable) and any installed system
  image; download more images from `sdkmanager` without leaving the dialog.
- **Clean stops**: Stop asks the emulator to shut down over gRPC, so it saves its quick-boot
  snapshot. An emulator that hangs is reported as not responding and can be force-stopped, or
  force-restarted from its tab.
- **Settings**: Android SDK, emulator, adb, command-line tools, AVD folder and JDK paths for when
  the environment lacks them (placeholders show what was found and where), GPU mode, snapshot
  saving, what closing a tab or the app does, and a System / Light / Dark theme.
- **Command line and single instance**: `avdeck --avd NAME` starts or attaches to a device; when
  Avdeck is already open, the request goes to that window.

## Requirements

- Windows 10 or 11, x64.
- The Android SDK with the Android Emulator (developed against Emulator 37.2.12) and x86_64
  system images.
- To create devices and download images: the Android SDK Command-line Tools and a JDK.

Avdeck finds these by itself (see [Configuration](#configuration)); no environment variable is
required.

## Download

Each release on the repository's **Releases** page has `avdeck-vX.Y.Z-windows-x64.zip` with
`avdeck.exe`, this README and the font licenses, plus a `.sha256` checksum. Unzip it anywhere and
run `avdeck.exe`. Nothing needs to be installed.

## Build from source

Building needs the Rust stable toolchain for MSVC, version 1.95 or newer (required by egui 0.37).
`protoc` is vendored, so there is nothing else to install.

```powershell
cargo build --release
```

The app is a single executable, `target\release\avdeck.exe`; copy it anywhere you like.

## Usage

### Window

Start `avdeck.exe`. It opens on the Devices page: start a device with **Start**, or open one that
is already running with **Open**. Each running device gets its own tab with the toolbar on the right.
Closing a tab or the app asks whether the devices keep running in the background or stop. Tick
"Remember my choice" to skip the question; you can change the choice later in Settings.

### Command line

```text
avdeck [--avd NAME] [--cold-boot] [--wipe-data] [--no-snapshot-save] [--gpu MODE] [-- EMULATOR FLAGS]
```

| Option | Effect |
|---|---|
| `--avd NAME` | Open that AVD: attach when it already runs, start it otherwise |
| `--cold-boot` | Start without loading the quick-boot snapshot |
| `--wipe-data` | Start from a factory reset |
| `--no-snapshot-save` | Do not save the state when the device stops |
| `--gpu MODE` | `host` (default), `auto`, `swiftshader_indirect` or `angle_indirect` |
| `-- FLAGS` | Passed to the emulator as they are |

### Keyboard shortcuts

| Keys | Action |
|---|---|
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+1 | Devices |
| Ctrl+2 .. Ctrl+9 | Device tabs |
| Ctrl+, | Settings |
| Ctrl+Shift+D | Debug overlay: DPI, screen and frame size, frame rate |

In a device tab, other keys go to the device.

### Files

| What | Where |
|---|---|
| Settings | `%APPDATA%\avdeck\config.toml` |
| Emulator logs | `%LOCALAPPDATA%\avdeck\logs\<AVD>.log` |
| Screenshots | `Pictures\Avdeck` |

## Configuration

Each tool path is resolved in this order, and child processes (emulator, adb, avdmanager,
sdkmanager) receive the resolved values:

1. The path set in Settings.
2. The environment: `ANDROID_HOME` / `ANDROID_SDK_ROOT`, `ANDROID_AVD_HOME`, `ANDROID_USER_HOME`,
   `JAVA_HOME`, `PATH`.
3. The usual places: `%LOCALAPPDATA%\Android\Sdk`, `%USERPROFILE%\.android\avd`, and the JDK bundled
   with Android Studio.

## How it works

1. Avdeck starts `emulator.exe -avd NAME -grpc PORT -grpc-use-token -qt-hide-window` on free ports.
2. It reads the emulator's discovery file (`%TEMP%\avd\running\pid_<pid>.ini`) to get the gRPC port
   and the access token, and connects with `authorization: Bearer <token>`.
3. Frames arrive through a shared-memory region that Avdeck creates and the emulator writes
   into. They are uploaded to the GPU as an egui texture.
4. Touch, keys, text and rotation go back as gRPC calls. The emulator console is used as a
   fallback, for example to stop an emulator that no longer answers gRPC.

Emulators started by Avdeck, by Android Studio or from a script all appear on the Devices page.
Avdeck can only show the ones that run with gRPC.

## Tech stack

| Area | Technology |
|---|---|
| Language | Rust 2024 |
| UI | [egui](https://github.com/emilk/egui) / eframe 0.37 on wgpu |
| gRPC | [tonic](https://github.com/hyperium/tonic) 0.14 and prost, on tokio |
| Windows | `windows-sys` (window subclassing, DPI, shared memory, processes), `rfd` (file dialogs) |
| Config | serde and TOML |
| Fonts | Inter (text), Phosphor Light (icons) |

## Project layout

| Path | Purpose |
|---|---|
| `crates/avdeck` | The app: tabs, device screen and input, Devices page, management dialogs, Settings, theme, Win32 window handling |
| `crates/avdeck-core` | Logic with no UI and no gRPC: SDK and tool resolution, AVDs, system images, config, launching, emulator console, running emulators, geometry |
| `crates/avdeck-grpc` | gRPC client: generated stubs, bearer-token auth, shared-memory frames |
| `crates/avdeck-grpc/examples` | Tools for checking the emulator's gRPC behaviour: streaming measurements, frame inspection, Extended Controls, shutdown |
| `proto/` | Protos vendored from Android Emulator 37.2.12 |

## Development

```powershell
cargo test                          # unit tests (geometry, parsing, path resolution, ...)
cargo clippy --release --all-targets
cargo fmt
```

Geometry, discovery-file parsing and tool resolution are pure functions, tested with values
measured on real devices and recorded tool output.

GitHub Actions runs the same checks on every push to `master` and on pull requests (`ci.yml`,
which also keeps the built `avdeck.exe` as an artifact for 14 days).

### Releasing

1. Set the new version in `Cargo.toml` (`[workspace.package] version`) and commit.
2. Tag the commit and push the tag:

   ```powershell
   git tag v0.2.0
   git push origin v0.2.0
   ```

`release.yml` checks that the tag matches the version, runs the tests, builds the zip and
publishes the GitHub release with generated notes. A tag with a suffix such as `v0.2.0-beta.1`
makes a prerelease. Running the workflow by hand from the Actions tab builds the zip as an
artifact without publishing anything.

## Credits

- [Inter](https://rsms.me/inter/) 4.1 by Rasmus Andersson, SIL Open Font License 1.1
  (`crates/avdeck/assets/Inter-LICENSE.txt`).
- [Phosphor Icons](https://phosphoricons.com), light weight, MIT License
  (`crates/avdeck/assets/Phosphor-LICENSE.txt`). The font is subset to the Private Use Area so that
  it does not replace Inter's letters.
- Emulator protos from the Android Open Source Project, Apache License 2.0.

## Author

**PTKDrake** · [github.com/PTKDrake](https://github.com/PTKDrake) · kien192837@gmail.com
