use super::sys;

pub fn now_ns() -> u64 {
    unsafe { sys::os_gettime_ns() }
}
