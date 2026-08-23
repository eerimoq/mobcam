//! MobCam, an OBS Studio source that receives video and audio from an iPhone or
//! iPad running Moblin over the USB cable, tunnelled through usbmuxd.
//!
//! The crate is built as a static library and linked into the OBS module CMake
//! produces. The entry points libobs looks up by name live in `obs::module`.

pub mod decoder;
pub mod ffmpeg;
pub mod obs;
pub mod panic;
pub mod plist;
pub mod protocol;
pub mod socket;
pub mod source;
pub mod usbmux;

use obs::Level;

/// The name the log lines are prefixed with, matching `name` in buildspec.json.
pub const PLUGIN_NAME: &str = "mobcam";

/// Kept in step with buildspec.json by the assertion in build.rs.
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

#[no_mangle]
pub extern "C" fn obs_module_load() -> bool {
    panic::guard("obs_module_load", false, || {
        let info = source::info();

        // SAFETY: the description is fully initialized and OBS copies it, which
        // is why passing a pointer to a local is sound here.
        unsafe {
            obs::sys::obs_register_source_s(&info, std::mem::size_of::<obs::sys::obs_source_info>());
        }

        obs_log!(Level::Info, "plugin loaded successfully (version {PLUGIN_VERSION})");

        true
    })
}

#[no_mangle]
pub extern "C" fn obs_module_unload() {
    panic::guard("obs_module_unload", (), || {
        obs_log!(Level::Info, "plugin unloaded");
    })
}
