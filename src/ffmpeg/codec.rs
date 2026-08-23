//! The decoder: the table entry libavcodec looks a codec up by, and the context
//! one stream is decoded in.

use std::ptr;

use super::sys;
use super::{Device, Frame, Packet, Status, INPUT_BUFFER_PADDING};

/// One entry in libavcodec's table of decoders. The table is static for the
/// life of the process, which is what makes this a plain copyable handle.
#[derive(Clone, Copy)]
pub struct Codec(*const sys::AVCodec);

impl Codec {
    /// The decoder for a codec id, or nothing when this build of libavcodec has
    /// none.
    pub fn find(id: sys::AVCodecID) -> Option<Self> {
        // SAFETY: a lookup by id, returning a static codec or null.
        let codec = unsafe { sys::avcodec_find_decoder(id) };

        (!codec.is_null()).then_some(Self(codec))
    }

    pub(super) fn as_ptr(self) -> *const sys::AVCodec {
        self.0
    }

    /// The pixel format frames from this codec arrive in on one kind of device,
    /// or nothing when it cannot be decoded on that kind at all.
    pub(super) fn hardware_pixel_format(self, kind: sys::AVHWDeviceType) -> Option<sys::AVPixelFormat> {
        for index in 0.. {
            // SAFETY: the codec is a static table entry, and its list of
            // hardware configurations ends with a null one.
            let config = unsafe { sys::avcodec_get_hw_config(self.0, index) };

            if config.is_null() {
                return None;
            }

            // SAFETY: config points into the codec's static configuration table.
            let config = unsafe { &*config };

            if (config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32) != 0 && config.device_type == kind
            {
                return Some(config.pix_fmt);
            }
        }

        None
    }
}

/// A decoder context, which is one stream being decoded.
///
/// It is allocated for a codec, configured, and only then opened; nothing set
/// before opening can be changed afterwards, which is why a configuration
/// message that differs from the last one starts a new context rather than
/// retuning this one.
pub struct Context {
    raw: *mut sys::AVCodecContext,
    open: bool,
}

impl Context {
    pub fn new(codec: Codec) -> Option<Self> {
        // SAFETY: codec came from Codec::find and is a static table entry.
        let raw = unsafe { sys::avcodec_alloc_context3(codec.as_ptr()) };

        (!raw.is_null()).then_some(Self { raw, open: false })
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Hands the decoder its configuration record - an avcC, an hvcC or an
    /// AudioSpecificConfig - which libavcodec takes over and frees with the
    /// context. Failing means the allocation failed.
    pub fn set_extradata(&mut self, record: &[u8]) -> bool {
        if record.is_empty() {
            return true;
        }

        // SAFETY: av_mallocz zeroes the allocation, so the padding past the
        // record is already what libavcodec expects to find there.
        let extradata = unsafe { sys::av_mallocz(record.len() + INPUT_BUFFER_PADDING) } as *mut u8;

        if extradata.is_null() {
            return false;
        }

        // SAFETY: the allocation has room for the record and the padding, and
        // the two regions cannot overlap. The context is not open yet, so
        // nothing is reading the field being replaced.
        unsafe {
            ptr::copy_nonoverlapping(record.as_ptr(), extradata, record.len());

            (*self.raw).extradata = extradata;
            (*self.raw).extradata_size = record.len() as i32;
        }

        true
    }

    /// The size the stream was announced with, which saves the decoder guessing
    /// it from the first frames.
    pub fn set_size(&mut self, width: i32, height: i32) {
        // SAFETY: the context is live and not open, so nothing else reads it.
        unsafe {
            (*self.raw).width = width;
            (*self.raw).height = height;
        }
    }

    /// Asks for the shortest path from an access unit to a frame: no reordering
    /// delay, and slice threading rather than frame threading, which would hold
    /// a frame back to keep its pipeline full.
    pub fn set_low_latency(&mut self) {
        // SAFETY: as above.
        unsafe {
            (*self.raw).flags |= sys::AV_CODEC_FLAG_LOW_DELAY as i32;
            (*self.raw).thread_type = sys::FF_THREAD_SLICE as i32;
        }
    }

    pub fn set_audio(&mut self, sample_rate: i32, channels: i32) {
        // SAFETY: as above; av_channel_layout_default fills in the layout it is
        // pointed at, which here is the context's own.
        unsafe {
            (*self.raw).sample_rate = sample_rate;
            sys::av_channel_layout_default(&mut (*self.raw).ch_layout, channels);
        }
    }

    /// Decodes on `device` rather than in software. The context takes a
    /// reference of its own, so the caller keeps holding theirs.
    pub fn set_hardware_device(&mut self, device: &Device) {
        // SAFETY: the device is live, and the reference taken here is released
        // when the context is freed.
        unsafe { (*self.raw).hw_device_ctx = sys::av_buffer_ref(device.as_ptr()) };
    }

    pub fn open(&mut self, codec: Codec) -> bool {
        // SAFETY: the context was allocated for this codec and is not open yet.
        let result = unsafe { sys::avcodec_open2(self.raw, codec.as_ptr(), ptr::null_mut()) };

        self.open = result >= 0;
        self.open
    }

    /// Drops whatever the decoder was in the middle of.
    pub fn flush(&mut self) {
        if !self.open {
            return;
        }

        // SAFETY: the context is open.
        unsafe { sys::avcodec_flush_buffers(self.raw) }
    }

    /// Hands one access unit to the decoder, in place: the packet points at the
    /// caller's buffer for the length of the call and no longer, so that buffer
    /// is the one that has to carry INPUT_BUFFER_PADDING past its end.
    pub fn send(&mut self, packet: &mut Packet, data: &[u8], pts: i64, keyframe: bool) -> Status {
        packet.set(data, pts, keyframe);

        // SAFETY: the context is open, and the packet's data outlives the call
        // because the borrow it holds is only ended below.
        let result = unsafe { sys::avcodec_send_packet(self.raw, packet.as_ptr()) };

        packet.clear();

        Status::of(result)
    }

    /// Takes one decoded frame back out, into `frame`.
    pub fn receive(&mut self, frame: &mut Frame) -> Status {
        // SAFETY: the context is open and the frame is live.
        Status::of(unsafe { sys::avcodec_receive_frame(self.raw, frame.as_ptr()) })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the context was allocated by avcodec_alloc_context3 and is
        // freed exactly once, here. This also drops the reference it holds on
        // the hardware device, which therefore has to outlive it.
        unsafe { sys::avcodec_free_context(&mut self.raw) }
    }
}
