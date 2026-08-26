use super::module::c_string;
use super::sys;
use std::ffi::CStr;

pub struct Data(*mut sys::obs_data_t);

impl Data {
    pub fn from_raw(raw: *mut sys::obs_data_t) -> Self {
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

    pub fn array(&self, key: &CStr) -> DataArray {
        DataArray(unsafe { sys::obs_data_get_array(self.0, key.as_ptr()) })
    }

    pub fn set_string(&self, key: &CStr, value: &str) {
        let value = c_string(value);
        unsafe { sys::obs_data_set_string(self.0, key.as_ptr(), value.as_ptr()) }
    }

    pub fn set_array(&self, key: &CStr, array: &DataArray) {
        unsafe { sys::obs_data_set_array(self.0, key.as_ptr(), array.0) }
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
    pub fn new() -> Self {
        Self(unsafe { sys::obs_data_create() })
    }

    pub fn from_json(json: &CStr) -> Option<Self> {
        let data = unsafe { sys::obs_data_create_from_json(json.as_ptr()) };
        (!data.is_null()).then_some(Self(data))
    }

    pub fn string(&self, key: &CStr) -> String {
        Data(self.0).string(key)
    }

    pub fn set_string(&self, key: &CStr, value: &str) {
        Data(self.0).set_string(key, value)
    }
}

impl Default for OwnedData {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OwnedData {
    fn drop(&mut self) {
        unsafe { sys::obs_data_release(self.0) }
    }
}

pub struct DataArray(*mut sys::obs_data_array_t);

impl DataArray {
    pub fn new() -> Self {
        Self(unsafe { sys::obs_data_array_create() })
    }

    pub fn len(&self) -> usize {
        if self.0.is_null() {
            return 0;
        }
        unsafe { sys::obs_data_array_count(self.0) }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn item(&self, index: usize) -> Option<OwnedData> {
        if self.0.is_null() {
            return None;
        }
        let item = unsafe { sys::obs_data_array_item(self.0, index) };
        (!item.is_null()).then_some(OwnedData(item))
    }

    pub fn push(&self, item: &OwnedData) {
        if self.0.is_null() {
            return;
        }
        unsafe { sys::obs_data_array_push_back(self.0, item.0) };
    }
}

impl Default for DataArray {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DataArray {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }
        unsafe { sys::obs_data_array_release(self.0) }
    }
}
