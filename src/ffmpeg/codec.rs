use super::sys;
use super::{Device, Frame, INPUT_BUFFER_PADDING, Packet, Status};
use std::ptr;

#[derive(Clone, Copy)]
pub struct Codec(*const sys::AVCodec);

impl Codec {
    pub fn find(id: sys::AVCodecID) -> Option<Self> {
        let codec = unsafe { sys::avcodec_find_decoder(id) };
        (!codec.is_null()).then_some(Self(codec))
    }

    pub(super) fn as_ptr(self) -> *const sys::AVCodec {
        self.0
    }

    pub(super) fn supports_hardware_device(self, kind: sys::AVHWDeviceType) -> bool {
        for index in 0.. {
            let config = unsafe { sys::avcodec_get_hw_config(self.0, index) };
            if config.is_null() {
                return false;
            }
            let config = unsafe { &*config };
            if (config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32) != 0 && config.device_type == kind
            {
                return true;
            }
        }

        false
    }
}

pub struct Context {
    raw: *mut sys::AVCodecContext,
    open: bool,
}

impl Context {
    pub fn new(codec: Codec) -> Option<Self> {
        let raw = unsafe { sys::avcodec_alloc_context3(codec.as_ptr()) };
        (!raw.is_null()).then_some(Self { raw, open: false })
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_extradata(&mut self, record: &[u8]) -> bool {
        if record.is_empty() {
            return true;
        }
        let extradata = unsafe { sys::av_mallocz(record.len() + INPUT_BUFFER_PADDING) } as *mut u8;
        if extradata.is_null() {
            return false;
        }
        unsafe {
            ptr::copy_nonoverlapping(record.as_ptr(), extradata, record.len());
            (*self.raw).extradata = extradata;
            (*self.raw).extradata_size = record.len() as i32;
        }
        true
    }

    pub fn set_size(&mut self, width: i32, height: i32) {
        unsafe {
            (*self.raw).width = width;
            (*self.raw).height = height;
        }
    }

    pub fn set_low_latency(&mut self) {
        unsafe {
            (*self.raw).flags |= sys::AV_CODEC_FLAG_LOW_DELAY as i32;
            (*self.raw).thread_type = sys::FF_THREAD_SLICE as i32;
        }
    }

    pub fn set_audio(&mut self, sample_rate: i32, channels: i32) {
        unsafe {
            (*self.raw).sample_rate = sample_rate;
            sys::av_channel_layout_default(&mut (*self.raw).ch_layout, channels);
        }
    }

    pub fn set_hardware_device(&mut self, device: &Device) {
        unsafe { (*self.raw).hw_device_ctx = sys::av_buffer_ref(device.as_ptr()) };
    }

    pub fn open(&mut self, codec: Codec) -> bool {
        let result = unsafe { sys::avcodec_open2(self.raw, codec.as_ptr(), ptr::null_mut()) };
        self.open = result >= 0;
        self.open
    }

    pub fn flush(&mut self) {
        if !self.open {
            return;
        }
        unsafe { sys::avcodec_flush_buffers(self.raw) }
    }

    pub fn send(&mut self, packet: &mut Packet, data: &[u8], pts: i64, keyframe: bool) -> Status {
        packet.set(data, pts, keyframe);
        let result = unsafe { sys::avcodec_send_packet(self.raw, packet.as_ptr()) };
        packet.clear();
        Status::of(result)
    }

    pub fn receive(&mut self, frame: &mut Frame) -> Status {
        Status::of(unsafe { sys::avcodec_receive_frame(self.raw, frame.as_ptr()) })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { sys::avcodec_free_context(&mut self.raw) }
    }
}
