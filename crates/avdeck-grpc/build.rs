//! Compiles the vendored emulator protos (`/proto`, from emulator 37.2.12) into client stubs.

use std::path::PathBuf;

const PROTOS: [&str; 3] = [
    "emulator_controller.proto",
    "snapshot_service.proto",
    "ui_controller_service.proto",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    println!("cargo:rerun-if-changed={}", proto_dir.display());

    // SAFETY: the build script is single-threaded; prost-build reads PROTOC to find protoc.
    unsafe { std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?) };
    let protos: Vec<PathBuf> = PROTOS.iter().map(|name| proto_dir.join(name)).collect();
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&protos, &[proto_dir, protoc_bin_vendored::include_path()?])?;
    Ok(())
}
