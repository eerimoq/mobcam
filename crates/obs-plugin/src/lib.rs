pub mod devices;
pub mod obs;
pub mod source;
use mobcam_core::ffmpeg;
use mobcam_core::{Level, log, panic};

pub const PLUGIN_NAME: &str = "mobcam";
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

fn module_load() -> bool {
    if let Err(error) = ffmpeg::version::check() {
        log!(Level::Error, "not loading the plugin because {error}");
        return false;
    }
    obs::register(&source::info());
    log!(
        Level::Info,
        "plugin loaded successfully (version {PLUGIN_VERSION}, {})",
        ffmpeg::version::loaded()
    );
    true
}

fn module_unload() {
    log!(Level::Info, "plugin unloaded");
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_load() -> bool {
    obs::install_logger();
    panic::guard("obs_module_load", false, module_load)
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_unload() {
    panic::guard("obs_module_unload", (), module_unload)
}
