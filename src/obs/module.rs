//! The module entry points libobs looks up by name when it loads the plugin.
//!
//! In C these come from the `OBS_DECLARE_MODULE()` and
//! `OBS_MODULE_USE_DEFAULT_LOCALE()` macros. Macros have no FFI form, so the
//! handful of functions they expand to are written out here instead.

use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

use super::sys;

/// Set by libobs immediately after loading the module and read for the rest of
/// the process' life, including by the locale lookup below.
static MODULE: AtomicPtr<sys::obs_module_t> = AtomicPtr::new(ptr::null_mut());

/// The translation table for the current OBS language.
static LOOKUP: AtomicPtr<sys::lookup_t> = AtomicPtr::new(ptr::null_mut());

pub struct Module;

impl Module {
    pub fn current() -> *mut sys::obs_module_t {
        MODULE.load(Ordering::Acquire)
    }
}

/// Looks up one string from `data/locale`, falling back to the key itself the
/// way `obs_module_text()` does.
///
/// The result borrows from the lookup table, which lives until OBS changes
/// language. Only `obs_module_set_locale` and `obs_module_free_locale` replace
/// it and both run on the OBS thread, the same thread that builds a properties
/// dialog, so a translated label cannot be freed while it is being used.
pub fn text(key: &'static CStr) -> &'static CStr {
    let lookup = LOOKUP.load(Ordering::Acquire);

    if lookup.is_null() {
        return key;
    }

    let mut out = key.as_ptr();

    // SAFETY: `lookup` is a table libobs handed us and has not been destroyed,
    // for the reason above. On a miss `out` is left pointing at the key.
    unsafe {
        sys::text_lookup_getstr(lookup, key.as_ptr(), &mut out);

        CStr::from_ptr(out)
    }
}

#[no_mangle]
pub extern "C" fn obs_module_set_pointer(module: *mut sys::obs_module_t) {
    MODULE.store(module, Ordering::Release);
}

/// libobs refuses to load a module built against a different API version, so
/// this has to be the version of the headers this was compiled against.
#[no_mangle]
pub extern "C" fn obs_module_ver() -> u32 {
    super::API_VERSION
}

/// Called by other code in this module in place of the C `obs_current_module()`.
#[no_mangle]
pub extern "C" fn obs_current_module() -> *mut sys::obs_module_t {
    Module::current()
}

#[no_mangle]
pub extern "C" fn obs_module_set_locale(locale: *const std::os::raw::c_char) {
    let default = c"en-US";

    // SAFETY: libobs passes a valid locale name, and the module pointer was set
    // before this is first called. The old table is destroyed only here and in
    // obs_module_free_locale, both of which libobs calls from one thread.
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
        // SAFETY: the pointer came from obs_module_load_locale and is taken out
        // of the static above, so nothing else can reach it now.
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

/// Turns a Rust string into one that can be handed to a C API for the duration
/// of a call. Truncates at an embedded NUL rather than failing, since the only
/// sources are device names and settings.
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
