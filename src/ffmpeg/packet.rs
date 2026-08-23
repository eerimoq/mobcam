//! `AVPacket`, one access unit on its way into a decoder.

use std::ptr;

use super::sys;

/// The packet a stream reuses for every access unit.
///
/// It never owns the bytes it points at. They are borrowed from the receive
/// buffer for the length of one send and dropped again straight after, which is
/// what lets an access unit be decoded where it was received rather than copied
/// first.
pub struct Packet(*mut sys::AVPacket);

impl Packet {
    pub fn new() -> Option<Self> {
        // SAFETY: allocates, and returns null on failure, which is checked.
        let packet = unsafe { sys::av_packet_alloc() };

        (!packet.is_null()).then_some(Self(packet))
    }

    pub(super) fn as_ptr(&mut self) -> *mut sys::AVPacket {
        self.0
    }

    /// Points the packet at one access unit. The borrow of `data` lasts until
    /// `clear`, which is why both are called from the one place that sends.
    pub(super) fn set(&mut self, data: &[u8], pts: i64, keyframe: bool) {
        // SAFETY: the packet is live, and unreferencing it first releases
        // anything a previous send left on it and resets the flags.
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

    /// Ends the borrow `set` took, so that the packet cannot outlive it.
    pub(super) fn clear(&mut self) {
        // SAFETY: the packet is live.
        unsafe {
            (*self.0).data = ptr::null_mut();
            (*self.0).size = 0;
        }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: allocated by av_packet_alloc and freed exactly once, here.
        unsafe { sys::av_packet_free(&mut self.0) }
    }
}
