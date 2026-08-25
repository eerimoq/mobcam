use super::module::c_string;
use super::sys;
use mobcam_core::Level;

fn to_obs(level: Level) -> i32 {
    let level = match level {
        Level::Error => sys::LOG_ERROR,
        Level::Warning => sys::LOG_WARNING,
        Level::Info => sys::LOG_INFO,
    };
    level as i32
}

fn write(level: Level, message: &str) {
    let line = c_string(&format!("[{}] {}", crate::PLUGIN_NAME, message));
    unsafe {
        sys::blog(to_obs(level), c"%s".as_ptr(), line.as_ptr());
    }
}

/// Sends everything the core logs to the OBS log.
pub fn install_logger() {
    mobcam_core::set_logger(write);
}
