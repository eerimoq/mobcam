//! `obs_source_t`, the source instance the plugin outputs to.

use std::ptr;

use super::media::{Audio, Frame};
use super::sys;

/// A handle on one live source. OBS owns the source and keeps it alive for as
/// long as the plugin's instance of it exists, which outlives every handle
/// here, so this only ever calls through the pointer.
#[derive(Clone, Copy)]
pub struct Source(*mut sys::obs_source_t);

// SAFETY: outputting video and audio is all the worker thread does with a
// handle, and both calls are documented to be callable from any thread.
unsafe impl Send for Source {}
unsafe impl Sync for Source {}

impl Source {
    /// # Safety
    /// `raw` must be a source that outlives the returned value.
    pub unsafe fn from_raw(raw: *mut sys::obs_source_t) -> Self {
        Self(raw)
    }

    /// Hands one frame to OBS. It borrows the decoder's memory until the call
    /// returns, by which time OBS has copied what it keeps.
    pub fn output_video(&self, frame: &Frame) {
        // SAFETY: the source is live and the frame is read, not stored.
        unsafe { sys::obs_source_output_video(self.0, frame) }
    }

    /// Blanks the source, which is what a null frame means to OBS.
    pub fn clear_video(&self) {
        // SAFETY: the source is live.
        unsafe { sys::obs_source_output_video(self.0, ptr::null()) }
    }

    /// Hands one buffer of audio over, on the same terms as `output_video`.
    pub fn output_audio(&self, audio: &Audio) {
        // SAFETY: as above.
        unsafe { sys::obs_source_output_audio(self.0, audio) }
    }

    /// Whether OBS should show each frame as it arrives rather than buffering
    /// to smooth the timing out.
    pub fn set_async_unbuffered(&self, unbuffered: bool) {
        // SAFETY: as above.
        unsafe { sys::obs_source_set_async_unbuffered(self.0, unbuffered) }
    }

    /// True while the source is on a scene that is being shown.
    pub fn showing(&self) -> bool {
        // SAFETY: as above.
        unsafe { sys::obs_source_showing(self.0) }
    }
}

/// Tells OBS about the kind of source the plugin adds. The description is
/// copied, which is why a reference to a local is enough.
pub fn register(info: &sys::obs_source_info) {
    // SAFETY: the description is fully initialized, and the size tells OBS how
    // much of it this plugin was built to fill in.
    unsafe { sys::obs_register_source_s(info, std::mem::size_of::<sys::obs_source_info>()) }
}
