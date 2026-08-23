//! `obs_data_t`, the settings object OBS hands to a source.

use std::ffi::CStr;

use super::sys;

/// A borrowed settings object. OBS owns it for the duration of the call that
/// produced it, so this only ever reads through the pointer.
pub struct Data(*mut sys::obs_data_t);

impl Data {
    /// # Safety
    /// `raw` must be a settings object that outlives the returned value.
    pub unsafe fn from_raw(raw: *mut sys::obs_data_t) -> Self {
        Self(raw)
    }

    pub fn string(&self, key: &CStr) -> String {
        // SAFETY: the pointer is valid for the lifetime of self, and OBS
        // returns a NUL terminated string that lives until the object changes.
        let value = unsafe { sys::obs_data_get_string(self.0, key.as_ptr()) };

        if value.is_null() {
            return String::new();
        }

        // SAFETY: value is a NUL terminated string owned by the settings object.
        unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned()
    }

    pub fn int(&self, key: &CStr) -> i64 {
        // SAFETY: as above.
        unsafe { sys::obs_data_get_int(self.0, key.as_ptr()) }
    }

    pub fn bool(&self, key: &CStr) -> bool {
        // SAFETY: as above.
        unsafe { sys::obs_data_get_bool(self.0, key.as_ptr()) }
    }

    pub fn set_default_string(&self, key: &CStr, value: &CStr) {
        // SAFETY: as above; OBS copies the value.
        unsafe { sys::obs_data_set_default_string(self.0, key.as_ptr(), value.as_ptr()) }
    }

    pub fn set_default_int(&self, key: &CStr, value: i64) {
        // SAFETY: as above.
        unsafe { sys::obs_data_set_default_int(self.0, key.as_ptr(), value) }
    }

    pub fn set_default_bool(&self, key: &CStr, value: bool) {
        // SAFETY: as above.
        unsafe { sys::obs_data_set_default_bool(self.0, key.as_ptr(), value) }
    }
}

/// A settings object parsed from JSON, which this owns and releases.
pub struct OwnedData(*mut sys::obs_data_t);

impl OwnedData {
    /// Parses JSON with the parser libobs already carries, so the plugin does
    /// not pull in one of its own for the one small object it has to read.
    pub fn from_json(json: &CStr) -> Option<Self> {
        // SAFETY: json is NUL terminated; libobs copies what it needs.
        let data = unsafe { sys::obs_data_create_from_json(json.as_ptr()) };

        (!data.is_null()).then_some(Self(data))
    }

    pub fn string(&self, key: &CStr) -> String {
        Data(self.0).string(key)
    }
}

impl Drop for OwnedData {
    fn drop(&mut self) {
        // SAFETY: the pointer came from obs_data_create_from_json and is
        // released exactly once.
        unsafe { sys::obs_data_release(self.0) }
    }
}
