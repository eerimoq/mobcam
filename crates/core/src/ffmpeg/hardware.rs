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

    /// Whether the frames of this device are cheaper to map than to copy out.
    ///
    /// Mapping hands back a pointer into the buffer the decoder wrote, which
    /// costs nothing at all, but only pays off where that buffer is ordinary
    /// cached memory. The rest map into memory the processor reads slowly, so
    /// reading a whole frame out of it is dearer than the copy it saves. The
    /// kinds are matched by name because the ones worth mapping are not all in
    /// every build of FFmpeg.
    pub fn maps_cheaply(&self) -> bool {
        matches!(self.name().as_str(), "rkmpp" | "drm")
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { sys::av_buffer_unref(&mut self.raw) }
    }
}
