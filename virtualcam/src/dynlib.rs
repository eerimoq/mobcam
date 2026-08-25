//! Loading libpulse and libasound at runtime.
//!
//! Neither is needed to build `mobcam-virtualcam`, and a machine that has only
//! one of them, or neither, still runs it; the backends that cannot be loaded
//! are simply not offered.

use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::c_char;
use std::ffi::c_int;
use std::ffi::c_void;

const RTLD_NOW: c_int = 2;

unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// A shared library, kept loaded for the rest of the process; nothing here ever
/// calls `dlclose`, so the symbols taken out of it stay valid.
pub struct Library(*mut c_void);

impl Library {
    pub fn open(name: &str) -> Option<Self> {
        let name = CString::new(name).ok()?;
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
        (!handle.is_null()).then_some(Self(handle))
    }

    /// # Safety
    ///
    /// `T` must be a function pointer whose signature matches the symbol.
    pub unsafe fn symbol<T: Copy>(&self, name: &CStr) -> Option<T> {
        const { assert!(size_of::<T>() == size_of::<*mut c_void>()) };
        let symbol = unsafe { dlsym(self.0, name.as_ptr()) };
        (!symbol.is_null()).then(|| unsafe { std::mem::transmute_copy(&symbol) })
    }
}
