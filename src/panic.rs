//! Keeping panics on this side of the FFI boundary.
//!
//! Unwinding out of an `extern "C"` function is undefined behaviour, and
//! aborting instead would take OBS down along with the plugin. Every entry
//! point the C side calls therefore runs inside `guard`, which turns a panic
//! into a logged message and a caller-supplied failure value.

use std::panic::{catch_unwind, AssertUnwindSafe};

pub fn guard<T>(name: &str, on_panic: T, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            let reason = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown");

            crate::obs_log!(crate::obs::Level::Error, "{name} panicked: {reason}");

            on_panic
        }
    }
}
