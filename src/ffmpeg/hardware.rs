//! The GPU a stream is decoded on, when it has one.

use std::ptr;

use super::sys;
use super::Codec;

/// Hardware decoders to try, in the order they are preferred. Every machine has
/// only one or two of these, and creating a device for the rest simply fails,
/// which is what picks the right one.
const TYPES: [sys::AVHWDeviceType; 7] = [
    sys::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
    sys::AV_HWDEVICE_TYPE_D3D11VA,
    sys::AV_HWDEVICE_TYPE_CUDA,
    sys::AV_HWDEVICE_TYPE_VAAPI,
    sys::AV_HWDEVICE_TYPE_QSV,
    sys::AV_HWDEVICE_TYPE_DXVA2,
    sys::AV_HWDEVICE_TYPE_VDPAU,
];

/// A hardware device a codec decodes on, along with the pixel format its frames
/// come back in.
pub struct Device {
    raw: *mut sys::AVBufferRef,
    kind: sys::AVHWDeviceType,
    format: sys::AVPixelFormat,
}

impl Device {
    /// Opens the first device this machine has that can decode this codec.
    /// Finding none is an ordinary outcome, and leaves the caller decoding in
    /// software.
    pub fn open(codec: Codec) -> Option<Self> {
        TYPES.into_iter().find_map(|kind| Self::open_kind(codec, kind))
    }

    fn open_kind(codec: Codec, kind: sys::AVHWDeviceType) -> Option<Self> {
        let format = codec.hardware_pixel_format(kind)?;
        let mut raw = ptr::null_mut();

        // SAFETY: creating a device for hardware the machine does not have
        // fails rather than misbehaving, which is what picks the right one. The
        // pointer is only read back when the call succeeded.
        let created = unsafe { sys::av_hwdevice_ctx_create(&mut raw, kind, ptr::null(), ptr::null_mut(), 0) };

        (created >= 0).then_some(Self { raw, kind, format })
    }

    pub(super) fn as_ptr(&self) -> *const sys::AVBufferRef {
        self.raw
    }

    /// The pixel format frames decoded on this device arrive in, which is what
    /// tells a frame that still has to be brought down into system memory from
    /// one that is already there.
    pub fn format(&self) -> sys::AVPixelFormat {
        self.format
    }

    /// What to call the device in a log line.
    pub fn name(&self) -> String {
        // SAFETY: returns a static string for a known device type, or null.
        super::name(unsafe { sys::av_hwdevice_get_type_name(self.kind) }).unwrap_or_else(|| String::from("hardware"))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: the reference came from av_hwdevice_ctx_create and is dropped
        // exactly once, here. Anything else holding one of its own - a codec
        // context, in particular - keeps the device alive past this.
        unsafe { sys::av_buffer_unref(&mut self.raw) }
    }
}
