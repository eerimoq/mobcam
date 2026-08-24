pub mod decoder;
pub mod ffmpeg;
pub mod obs;
pub mod panic;
pub mod protocol;
pub mod socket;
pub mod source;
pub mod usbmux;

use obs::Level;

pub const PLUGIN_NAME: &str = "mobcam";
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

#[no_mangle]
pub extern "C" fn obs_module_load() -> bool {
    panic::guard("obs_module_load", false, || {
        obs::register(&source::info());

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
