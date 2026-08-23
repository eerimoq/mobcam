//! Logging into the OBS log, with the plugin name prefixed the way the C
//! `obs_log()` did.

use super::module::c_string;
use super::sys;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Error,
    Warning,
    Info,
}

impl Level {
    fn to_obs(self) -> i32 {
        let level = match self {
            Level::Error => sys::LOG_ERROR,
            Level::Warning => sys::LOG_WARNING,
            Level::Info => sys::LOG_INFO,
        };

        level as i32
    }
}

/// Writes one line to the OBS log.
///
/// The message is passed to the variadic `blog()` as the argument of a literal
/// "%s" rather than as the format string itself, so a percent sign in a device
/// name cannot be read as a conversion.
pub fn log(level: Level, message: &str) {
    let line = c_string(&format!("[{}] {}", crate::PLUGIN_NAME, message));

    // SAFETY: blog() is variadic; the format is a literal "%s" and its one
    // argument is a valid NUL terminated string that outlives the call.
    unsafe {
        sys::blog(level.to_obs(), c"%s".as_ptr(), line.as_ptr());
    }
}

/// `obs_log(LOG_INFO, "...")`, spelled the way Rust code expects to spell it.
#[macro_export]
macro_rules! obs_log {
    ($level:expr, $($arg:tt)*) => {
        $crate::obs::log($level, &::std::format!($($arg)*))
    };
}
