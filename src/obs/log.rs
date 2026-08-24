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

pub fn log(level: Level, message: &str) {
    let line = c_string(&format!("[{}] {}", crate::PLUGIN_NAME, message));

    unsafe {
        sys::blog(level.to_obs(), c"%s".as_ptr(), line.as_ptr());
    }
}

#[macro_export]
macro_rules! obs_log {
    ($level:expr, $($arg:tt)*) => {
        $crate::obs::log($level, &::std::format!($($arg)*))
    };
}
