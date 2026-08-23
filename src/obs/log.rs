//! Logging into the OBS log, with the plugin name prefixed the way the C
//! `obs_log()` did.

use std::ffi::CString;

use super::sys;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Error,
    Warning,
    Info,
    Debug,
}

impl Level {
    fn to_obs(self) -> i32 {
        let level = match self {
            Level::Error => sys::LOG_ERROR,
            Level::Warning => sys::LOG_WARNING,
            Level::Info => sys::LOG_INFO,
            Level::Debug => sys::LOG_DEBUG,
        };

        level as i32
    }
}

/// Writes one line to the OBS log.
///
/// The message is passed to the variadic `blog()` as an argument to a literal
/// "%s" rather than as the format string itself, so a percent sign in a device
/// name cannot be interpreted as a conversion.
pub fn log(level: Level, message: &str) {
    let line = format!("[{}] {}", crate::PLUGIN_NAME, message);

    // A NUL in the middle of a device name would truncate the line rather than
    // lose it entirely, which is the better failure here.
    let line = CString::new(line).unwrap_or_else(|error| {
        let mut bytes = error.into_vec();
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(0);
        bytes.truncate(end);
        CString::new(bytes).expect("truncated at the first NUL")
    });

    // SAFETY: blog() is variadic; the format string is a literal "%s" and the
    // single argument is a valid NUL terminated string that outlives the call.
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
