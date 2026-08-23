//! The OBS monotonic clock.

use super::sys;

/// OBS places a frame on its timeline by comparing the timestamp on it against
/// this clock, so the timestamps have to be taken from here rather than from a
/// Rust Instant.
pub fn now_ns() -> u64 {
    // SAFETY: no arguments and no failure mode.
    unsafe { sys::os_gettime_ns() }
}
