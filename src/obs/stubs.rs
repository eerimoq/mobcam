use std::time::Instant;

#[no_mangle]
pub extern "C" fn os_gettime_ns() -> u64 {
    use std::sync::OnceLock;

    static START: OnceLock<Instant> = OnceLock::new();

    const BASE_NS: u64 = 60 * 60 * 1_000_000_000;

    BASE_NS + START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}
