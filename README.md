# Avdeck

An Android Emulator manager for Windows, written in Rust. Emulators run headless
(`-qt-hide-window`); Avdeck shows each one in a tab of its own window, with a device frame and
toolbar, and drives it over the emulator's gRPC API. See `PLAN.md` (Vietnamese) for the roadmap.

Status: **M0 done** (gRPC spike, see `docs/grpc-notes.md`). There is no app yet.

## Build

Requires the Rust MSVC toolchain on Windows. `protoc` is vendored, nothing else to install.

```powershell
cargo build --release
cargo test
```

## Layout

| Path | Purpose |
|---|---|
| `crates/avdeck-core` | AVD lookup (`avd`), running emulators from discovery files (`running`). No UI, no gRPC |
| `crates/avdeck-grpc` | gRPC client: generated stubs, bearer-token auth, `Emulator` connection |
| `crates/avdeck-grpc/examples/spike.rs` | M0 spike: launch/attach, measure screen streaming, test input |
| `proto/` | Protos vendored from emulator 37.2.12 (`D:\Android\SDK\emulator\lib`) |
| `docs/grpc-notes.md` | How the emulator's gRPC API behaves on Windows, with measurements |

## Spike

```powershell
cargo run --release -p avdeck-grpc --example spike -- --avd Pixel_10_Pro_API_37_2 --launch
```

Launches the AVD headless with gRPC on port 8554 (or attaches with `--grpc PORT` when it already
runs), streams the screen over gRPC bytes, shared memory and a mapped file, then sends keys and
touches. Screenshots go to `D:\Android\emulator-diagnostics\spike`. Options are listed at the top
of `spike.rs`.

## History

The repo started as `emu-window`, a daemon that kept the emulator's Qt windows fitted and their
toolbar docked on mixed-DPI setups. Avdeck replaces the Qt window, so the daemon was removed; it
is in the first commit on `master`.
