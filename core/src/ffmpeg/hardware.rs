use super::Codec;
use super::sys;
use std::ptr;

pub struct Device {
    raw: *mut sys::AVBufferRef,
    kind: sys::AVHWDeviceType,
}

impl Device {
    pub fn open(codec: Codec) -> Option<Self> {
        codec.hardware_device_types().find_map(Self::open_kind)
    }

    fn open_kind(kind: sys::AVHWDeviceType) -> Option<Self> {
        let mut raw = ptr::null_mut();
        let created = unsafe { sys::av_hwdevice_ctx_create(&mut raw, kind, ptr::null(), ptr::null_mut(), 0) };
        (created >= 0).then_some(Self { raw, kind })
    }

    pub(super) fn as_ptr(&self) -> *const sys::AVBufferRef {
        self.raw
    }

    pub fn name(&self) -> String {
        super::name(unsafe { sys::av_hwdevice_get_type_name(self.kind) }).unwrap_or_else(|| String::from("hardware"))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { sys::av_buffer_unref(&mut self.raw) }
    }
}
