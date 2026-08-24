use super::media::{Audio, Frame};
use super::sys;
use std::ptr;

pub struct Source(*mut sys::obs_source_t);

unsafe impl Send for Source {}

unsafe impl Sync for Source {}

impl Source {
    pub fn from_raw(raw: *mut sys::obs_source_t) -> Self {
        Self(raw)
    }

    pub fn output_video(&self, frame: &Frame) {
        unsafe { sys::obs_source_output_video(self.0, frame) }
    }

    pub fn clear_video(&self) {
        unsafe { sys::obs_source_output_video(self.0, ptr::null()) }
    }

    pub fn output_audio(&self, audio: &Audio) {
        unsafe { sys::obs_source_output_audio(self.0, audio) }
    }

    pub fn set_async_unbuffered(&self, unbuffered: bool) {
        unsafe { sys::obs_source_set_async_unbuffered(self.0, unbuffered) }
    }

    pub fn showing(&self) -> bool {
        unsafe { sys::obs_source_showing(self.0) }
    }
}

pub fn register(info: &sys::obs_source_info) {
    unsafe { sys::obs_register_source_s(info, std::mem::size_of::<sys::obs_source_info>()) }
}
