use std::ptr;

use super::sys;
use super::Codec;

const TYPES: [sys::AVHWDeviceType; 7] = [
    sys::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
    sys::AV_HWDEVICE_TYPE_D3D11VA,
    sys::AV_HWDEVICE_TYPE_CUDA,
    sys::AV_HWDEVICE_TYPE_VAAPI,
    sys::AV_HWDEVICE_TYPE_QSV,
    sys::AV_HWDEVICE_TYPE_DXVA2,
    sys::AV_HWDEVICE_TYPE_VDPAU,
];

pub struct Device {
    raw: *mut sys::AVBufferRef,
    kind: sys::AVHWDeviceType,
    format: sys::AVPixelFormat,
}

impl Device {
    pub fn open(codec: Codec) -> Option<Self> {
        TYPES.into_iter().find_map(|kind| Self::open_kind(codec, kind))
    }

    fn open_kind(codec: Codec, kind: sys::AVHWDeviceType) -> Option<Self> {
        let format = codec.hardware_pixel_format(kind)?;
        let mut raw = ptr::null_mut();

        let created = unsafe { sys::av_hwdevice_ctx_create(&mut raw, kind, ptr::null(), ptr::null_mut(), 0) };

        (created >= 0).then_some(Self { raw, kind, format })
    }

    pub(super) fn as_ptr(&self) -> *const sys::AVBufferRef {
        self.raw
    }

    pub fn format(&self) -> sys::AVPixelFormat {
        self.format
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
