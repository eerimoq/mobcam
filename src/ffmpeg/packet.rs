use super::sys;
use std::ptr;

pub struct Packet(*mut sys::AVPacket);

impl Packet {
    pub fn new() -> Option<Self> {
        let packet = unsafe { sys::av_packet_alloc() };
        (!packet.is_null()).then_some(Self(packet))
    }

    pub(super) fn as_ptr(&mut self) -> *mut sys::AVPacket {
        self.0
    }

    pub(super) fn set(&mut self, data: &[u8], pts: i64, keyframe: bool) {
        unsafe {
            sys::av_packet_unref(self.0);
            (*self.0).data = data.as_ptr() as *mut u8;
            (*self.0).size = data.len() as i32;
            (*self.0).pts = pts;
            (*self.0).dts = pts;
            if keyframe {
                (*self.0).flags |= sys::AV_PKT_FLAG_KEY as i32;
            }
        }
    }

    pub(super) fn clear(&mut self) {
        unsafe {
            (*self.0).data = ptr::null_mut();
            (*self.0).size = 0;
        }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { sys::av_packet_free(&mut self.0) }
    }
}
