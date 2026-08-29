use super::module::c_string;
use super::sys;
use std::ffi::{CStr, c_void};

impl Default for Properties {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Properties(*mut sys::obs_properties_t);

impl Properties {
    pub fn new() -> Self {
        Self(unsafe { sys::obs_properties_create() })
    }

    pub fn add_string_list(&mut self, name: &CStr, description: &CStr) -> Property {
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

    /// # Safety
    /// `data` must outlive the property list.
    pub unsafe fn add_button(
        &mut self,
        name: &CStr,
        description: &CStr,
        callback: sys::obs_property_clicked_t,
        data: *mut c_void,
    ) {
        unsafe {
            sys::obs_properties_add_button2(self.0, name.as_ptr(), description.as_ptr(), callback, data);
        }
    }

    pub fn add_int(&mut self, name: &CStr, description: &CStr, min: i32, max: i32) -> Property {
        let property = unsafe { sys::obs_properties_add_int(self.0, name.as_ptr(), description.as_ptr(), min, max, 1) };
        Property(property)
    }

    pub fn add_bool(&mut self, name: &CStr, description: &CStr) -> Property {
        let property = unsafe { sys::obs_properties_add_bool(self.0, name.as_ptr(), description.as_ptr()) };
        Property(property)
    }

    pub fn into_raw(self) -> *mut sys::obs_properties_t {
        self.0
    }
}

pub struct Property(*mut sys::obs_property_t);

impl Property {
    pub fn from_raw(raw: *mut sys::obs_property_t) -> Self {
        Self(raw)
    }

    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    pub fn clear_list(&mut self) {
        if self.is_null() {
            return;
        }
        unsafe { sys::obs_property_list_clear(self.0) }
    }

    pub fn set_long_description(&mut self, description: &CStr) {
        if self.is_null() {
            return;
        }
        unsafe { sys::obs_property_set_long_description(self.0, description.as_ptr()) }
    }

    pub fn add_list_entry(&mut self, label: &str, value: &str) -> usize {
        if self.is_null() {
            return 0;
        }
        let label = c_string(label);
        let value = c_string(value);
        unsafe { sys::obs_property_list_add_string(self.0, label.as_ptr(), value.as_ptr()) }
    }

    pub fn add_translated_list_entry(&mut self, label: &CStr, value: &str) {
        if self.is_null() {
            return;
        }
        let value = c_string(value);
        unsafe {
            sys::obs_property_list_add_string(self.0, label.as_ptr(), value.as_ptr());
        }
    }

    pub fn disable_list_entry(&mut self, index: usize, value: bool) {
        if self.is_null() {
            return;
        }
        unsafe {
            sys::obs_property_list_item_disable(self.0, index, value);
        }
    }
}

/// # Safety
pub unsafe fn get(properties: *mut sys::obs_properties_t, name: &CStr) -> Property {
    Property(unsafe { sys::obs_properties_get(properties, name.as_ptr()) })
}
