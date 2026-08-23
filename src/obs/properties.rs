//! `obs_properties_t`, the description of the source's settings dialog.

use std::ffi::CStr;

use super::module::c_string;
use super::sys;

impl Default for Properties {
    fn default() -> Self {
        Self::new()
    }
}

/// The property list being built for a settings dialog. Ownership passes to OBS
/// when the pointer is returned from the `get_properties` callback.
pub struct Properties(*mut sys::obs_properties_t);

impl Properties {
    pub fn new() -> Self {
        // SAFETY: no arguments; returns a fresh list this now owns.
        Self(unsafe { sys::obs_properties_create() })
    }

    pub fn add_string_list(&mut self, name: &CStr, description: &CStr) -> Property {
        // SAFETY: the list is live and the strings outlive the call, which is
        // all obs_properties_add_list reads.
        let property = unsafe {
            sys::obs_properties_add_list(
                self.0,
                name.as_ptr(),
                description.as_ptr(),
                sys::OBS_COMBO_TYPE_LIST,
                sys::OBS_COMBO_FORMAT_STRING,
            )
        };

        Property(property)
    }

    pub fn add_button(&mut self, name: &CStr, description: &CStr, callback: sys::obs_property_clicked_t) {
        // SAFETY: as above.
        unsafe {
            sys::obs_properties_add_button2(
                self.0,
                name.as_ptr(),
                description.as_ptr(),
                callback,
                std::ptr::null_mut(),
            );
        }
    }

    pub fn add_int(&mut self, name: &CStr, description: &CStr, min: i32, max: i32) {
        // SAFETY: as above.
        unsafe {
            sys::obs_properties_add_int(self.0, name.as_ptr(), description.as_ptr(), min, max, 1);
        }
    }

    pub fn add_bool(&mut self, name: &CStr, description: &CStr) {
        // SAFETY: as above.
        unsafe {
            sys::obs_properties_add_bool(self.0, name.as_ptr(), description.as_ptr());
        }
    }

    /// Hands the list to OBS, which takes over freeing it.
    pub fn into_raw(self) -> *mut sys::obs_properties_t {
        self.0
    }
}

/// One property inside a list. Borrowed from the list that owns it.
pub struct Property(*mut sys::obs_property_t);

impl Property {
    /// # Safety
    /// `raw` must belong to a property list that outlives the returned value.
    pub unsafe fn from_raw(raw: *mut sys::obs_property_t) -> Self {
        Self(raw)
    }

    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    pub fn clear_list(&mut self) {
        if self.is_null() {
            return;
        }

        // SAFETY: the property belongs to a live list.
        unsafe { sys::obs_property_list_clear(self.0) }
    }

    /// Adds one entry, whose label is shown and whose value is stored.
    pub fn add_list_entry(&mut self, label: &str, value: &str) {
        if self.is_null() {
            return;
        }

        let label = c_string(label);
        let value = c_string(value);

        // SAFETY: both strings are NUL terminated and OBS copies them.
        unsafe {
            sys::obs_property_list_add_string(self.0, label.as_ptr(), value.as_ptr());
        }
    }

    /// Adds an entry whose label comes from the translation table, so the
    /// pointer is already a C string owned by the lookup.
    pub fn add_translated_entry(&mut self, label: &CStr, value: &str) {
        if self.is_null() {
            return;
        }

        let value = c_string(value);

        // SAFETY: as above.
        unsafe {
            sys::obs_property_list_add_string(self.0, label.as_ptr(), value.as_ptr());
        }
    }
}

/// Looks a property up by name in a list OBS handed back to a callback.
///
/// # Safety
/// `properties` must be a live property list.
pub unsafe fn get(properties: *mut sys::obs_properties_t, name: &CStr) -> Property {
    Property(sys::obs_properties_get(properties, name.as_ptr()))
}
