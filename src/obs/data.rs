use super::sys;
use std::ffi::CStr;

pub struct Data(*mut sys::obs_data_t);

impl Data {
    /// # Safety
    /// `raw` must be a settings object that outlives the returned value.
    pub unsafe fn from_raw(raw: *mut sys::obs_data_t) -> Self {
        Self(raw)
    }

    pub fn string(&self, key: &CStr) -> String {
        let value = unsafe { sys::obs_data_get_string(self.0, key.as_ptr()) };
        if value.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned()
    }

    pub fn int(&self, key: &CStr) -> i64 {
        unsafe { sys::obs_data_get_int(self.0, key.as_ptr()) }
    }

    pub fn bool(&self, key: &CStr) -> bool {
        unsafe { sys::obs_data_get_bool(self.0, key.as_ptr()) }
    }

    pub fn set_default_string(&self, key: &CStr, value: &CStr) {
        unsafe { sys::obs_data_set_default_string(self.0, key.as_ptr(), value.as_ptr()) }
    }

    pub fn set_default_int(&self, key: &CStr, value: i64) {
        unsafe { sys::obs_data_set_default_int(self.0, key.as_ptr(), value) }
    }

    pub fn set_default_bool(&self, key: &CStr, value: bool) {
        unsafe { sys::obs_data_set_default_bool(self.0, key.as_ptr(), value) }
    }
}

pub struct OwnedData(*mut sys::obs_data_t);

impl OwnedData {
    pub fn from_json(json: &CStr) -> Option<Self> {
        let data = unsafe { sys::obs_data_create_from_json(json.as_ptr()) };
        (!data.is_null()).then_some(Self(data))
    }

    pub fn string(&self, key: &CStr) -> String {
        Data(self.0).string(key)
    }
}

impl Drop for OwnedData {
    fn drop(&mut self) {
        unsafe { sys::obs_data_release(self.0) }
    }
}
