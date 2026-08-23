//! MobCam, an OBS Studio source that receives video and audio from an iPhone or
//! iPad running Moblin over the USB cable.
//!
//! This crate is built as a staticlib and linked into the OBS module that CMake
//! produces. While the port from C is in progress the remaining C files call in
//! here through the headers in `src/`, so the entry points below are `extern
//! "C"` and keep the names those headers declare.

pub mod ffmpeg;
pub mod obs;

mod panic;

/// Matches the `PLUGIN_NAME` the generated plugin-support.c used to define, and
/// the `name` in buildspec.json that CMake builds everything else from.
pub const PLUGIN_NAME: &str = "mobcam";

/// Kept in step with buildspec.json by the assertion in build.rs.
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Called from `obs_module_load()` while the module entry point is still C.
///
/// It exists to prove the whole pipeline end to end — cargo builds, the
/// staticlib is force-loaded into the module, and the generated libobs bindings
/// reach the real libobs — and is replaced by the Rust module entry point in
/// the last step of the port.
#[no_mangle]
pub extern "C" fn mobcam_rust_init() {
    panic::guard("mobcam_rust_init", (), || {
        obs_log!(obs::Level::Info, "Rust support initialized (version {PLUGIN_VERSION})");
    });
}
