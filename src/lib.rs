pub mod devices;
pub mod obs;
pub mod source;
use mobcam_core::{Level, log, panic};

pub const PLUGIN_NAME: &str = "mobcam";
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_load() -> bool {
    obs::install_logger();
    panic::guard("obs_module_load", false, || {
        obs::register(&source::info());
        log!(Level::Info, "plugin loaded successfully (version {PLUGIN_VERSION})");
        true
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_unload() {
    panic::guard("obs_module_unload", (), || {
        log!(Level::Info, "plugin unloaded");
    })
}
