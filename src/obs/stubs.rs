//! Stand-ins for the libobs symbols the unit tests link against.
//!
//! The tests exercise the parsers, the clock and the format tables, none of
//! which need a running OBS - but the crate still references libobs, and a test
//! binary has nothing to resolve those references against. Only the symbols the
//! tested paths actually reach are defined here; anything else stays undefined
//! and will fail the link rather than quietly return a wrong answer.

use std::time::Instant;

/// The OBS monotonic clock. The tests only care that it advances and that the
/// arithmetic around it is right, so any monotonic source will do.
#[no_mangle]
pub extern "C" fn os_gettime_ns() -> u64 {
    use std::sync::OnceLock;

    static START: OnceLock<Instant> = OnceLock::new();

    // A large offset so that a test subtracting from it cannot underflow just
    // because the process happened to have started a moment ago.
    const BASE_NS: u64 = 60 * 60 * 1_000_000_000;

    BASE_NS + START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}
