use std::sync::OnceLock;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Level {
    Error,
    Warning,
    Info,
}

pub type Logger = fn(Level, &str);

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn set_logger(logger: Logger) {
    let _ = LOGGER.set(logger);
}

pub fn log(level: Level, message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger(level, message);
    }
}

#[macro_export]
macro_rules! log {
    ($level:expr, $($arg:tt)*) => {
        $crate::log($level, &::std::format!($($arg)*))
    };
}

pub struct Changed<T>(Option<T>);

impl<T> Default for Changed<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T: PartialEq> Changed<T> {
    pub fn changed(&mut self, value: T) -> bool {
        if self.0.as_ref() == Some(&value) {
            return false;
        }
        self.0 = Some(value);
        true
    }
}
