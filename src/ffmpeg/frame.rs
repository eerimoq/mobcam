//! `AVFrame`, what comes back out of a decoder.

use super::sys;

/// One decoded frame.
///
/// The pixels or samples belong to libavcodec and stay where they are until the
/// frame is unreferenced or decoded into again, so the accessors hand out the
/// pointers rather than slices: how much lies behind one is a question for the
/// format and the line size, and OBS is handed the same pointers to read the
/// frame in place.
pub struct Frame(*mut sys::AVFrame);

impl Frame {
    /// How many planes a video frame can have at all, which is the size of the
    /// arrays `plane` reads.
    pub const PLANES: usize = 8;

    pub fn new() -> Option<Self> {
        // SAFETY: allocates, and returns null on failure, which is checked.
        let frame = unsafe { sys::av_frame_alloc() };

        (!frame.is_null()).then_some(Self(frame))
    }

    pub(super) fn as_ptr(&mut self) -> *mut sys::AVFrame {
        self.0
    }

    /// The header fields, which every accessor below reads through.
    fn get(&self) -> &sys::AVFrame {
        // SAFETY: the frame was allocated in new() and lives as long as self,
        // and the borrow returned cannot outlive that.
        unsafe { &*self.0 }
    }

    /// Drops what the frame was holding, which is what libavcodec wants before
    /// it decodes into it again.
    pub fn unref(&mut self) {
        // SAFETY: the frame is live; unreferencing one twice is harmless.
        unsafe { sys::av_frame_unref(self.0) }
    }

    /// Brings `source`, decoded into hardware memory, down into system memory
    /// here. Only the pixels come across, so the metadata is copied after.
    pub fn download(&mut self, source: &Frame) -> bool {
        // SAFETY: both frames are live, and the destination is unreferenced
        // first, which is what av_hwframe_transfer_data requires of it.
        unsafe {
            sys::av_frame_unref(self.0);

            sys::av_hwframe_transfer_data(self.0, source.0, 0) >= 0 && sys::av_frame_copy_props(self.0, source.0) >= 0
        }
    }

    pub fn pixel_format(&self) -> sys::AVPixelFormat {
        self.get().format
    }

    pub fn sample_format(&self) -> sys::AVSampleFormat {
        self.get().format
    }

    pub fn width(&self) -> i32 {
        self.get().width
    }

    pub fn height(&self) -> i32 {
        self.get().height
    }

    pub fn pts(&self) -> i64 {
        self.get().pts
    }

    pub fn colorspace(&self) -> sys::AVColorSpace {
        self.get().colorspace
    }

    pub fn color_trc(&self) -> sys::AVColorTransferCharacteristic {
        self.get().color_trc
    }

    /// True when the frame says its samples use the full range rather than the
    /// studio one. A pixel format can imply full range on its own, which is why
    /// this is only half the answer.
    pub fn is_full_range(&self) -> bool {
        self.get().color_range == sys::AVCOL_RANGE_JPEG
    }

    /// One plane of a video frame, and the bytes from the start of one row of it
    /// to the start of the next.
    pub fn plane(&self, index: usize) -> (*mut u8, i32) {
        let frame = self.get();

        (frame.data[index], frame.linesize[index])
    }

    pub fn samples(&self) -> i32 {
        self.get().nb_samples
    }

    pub fn sample_rate(&self) -> i32 {
        self.get().sample_rate
    }

    pub fn channels(&self) -> i32 {
        self.get().ch_layout.nb_channels
    }

    /// How many planes the samples are spread over: one per channel when the
    /// sample format is planar, and one in total when it is interleaved.
    pub fn audio_planes(&self) -> usize {
        // SAFETY: a table lookup by sample format, which returns false for one
        // it does not know.
        let planar = unsafe { sys::av_sample_fmt_is_planar(self.sample_format()) } != 0;

        if planar {
            self.channels().max(0) as usize
        } else {
            1
        }
    }

    /// One plane of audio, or null past the last of them.
    pub fn audio_plane(&self, index: usize) -> *mut u8 {
        if index >= self.audio_planes() {
            return std::ptr::null_mut();
        }

        // SAFETY: extended_data points at as many planes as the sample format
        // and the channel layout call for, which is what audio_planes counts.
        unsafe { *self.get().extended_data.add(index) }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: allocated by av_frame_alloc and freed exactly once, here.
        unsafe { sys::av_frame_free(&mut self.0) }
    }
}
