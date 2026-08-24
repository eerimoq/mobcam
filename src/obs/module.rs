use super::sys;
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

static MODULE: AtomicPtr<sys::obs_module_t> = AtomicPtr::new(ptr::null_mut());
static LOOKUP: AtomicPtr<sys::lookup_t> = AtomicPtr::new(ptr::null_mut());

pub struct Module;

impl Module {
    pub fn current() -> *mut sys::obs_module_t {
        MODULE.load(Ordering::Acquire)
    }
}

pub fn text(key: &'static CStr) -> &'static CStr {
    let lookup = LOOKUP.load(Ordering::Acquire);
    if lookup.is_null() {
        return key;
    }
    let mut out = key.as_ptr();
    unsafe {
        sys::text_lookup_getstr(lookup, key.as_ptr(), &mut out);
        CStr::from_ptr(out)
    }
}

#[no_mangle]
pub extern "C" fn obs_module_set_pointer(module: *mut sys::obs_module_t) {
    MODULE.store(module, Ordering::Release);
}

#[no_mangle]
pub extern "C" fn obs_module_ver() -> u32 {
    super::API_VERSION
}

#[no_mangle]
pub extern "C" fn obs_current_module() -> *mut sys::obs_module_t {
    Module::current()
}

#[no_mangle]
pub extern "C" fn obs_module_set_locale(locale: *const std::os::raw::c_char) {
    let default = c"en-US";
    let lookup = unsafe {
        let previous = LOOKUP.swap(ptr::null_mut(), Ordering::AcqRel);
        if !previous.is_null() {
            sys::text_lookup_destroy(previous);
        }
        sys::obs_module_load_locale(Module::current(), default.as_ptr(), locale)
    };
    LOOKUP.store(lookup, Ordering::Release);
}

#[no_mangle]
pub extern "C" fn obs_module_free_locale() {
    let previous = LOOKUP.swap(ptr::null_mut(), Ordering::AcqRel);
    if !previous.is_null() {
        unsafe {
            sys::text_lookup_destroy(previous);
        }
    }
}

#[no_mangle]
pub extern "C" fn obs_module_name() -> *const std::os::raw::c_char {
    c"MobCam".as_ptr()
}

#[no_mangle]
pub extern "C" fn obs_module_description() -> *const std::os::raw::c_char {
    c"Use an iPhone or iPad running Moblin as a camera, over the USB cable".as_ptr()
}

pub fn c_string(value: &str) -> CString {
    match CString::new(value) {
        Ok(value) => value,
        Err(error) => {
            let mut bytes = error.into_vec();
            let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(0);
            bytes.truncate(end);
            CString::new(bytes).expect("truncated at the first NUL")
        }
    }
}
