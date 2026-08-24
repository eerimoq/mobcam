use std::panic::{AssertUnwindSafe, catch_unwind};

pub fn guard<T>(name: &str, on_panic: T, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            let reason = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown");
            crate::obs::log(crate::obs::Level::Error, &format!("{name} panicked: {reason}"));
            on_panic
        }
    }
}
