use super::sys;

pub struct Frame(*mut sys::AVFrame);

impl Frame {
    pub const PLANES: usize = 8;

    pub fn new() -> Option<Self> {
        let frame = unsafe { sys::av_frame_alloc() };
        (!frame.is_null()).then_some(Self(frame))
    }

    pub(super) fn as_ptr(&mut self) -> *mut sys::AVFrame {
        self.0
    }

    fn get(&self) -> &sys::AVFrame {
        unsafe { &*self.0 }
    }

    pub fn unref(&mut self) {
        unsafe { sys::av_frame_unref(self.0) }
    }

    pub fn download(&mut self, source: &Frame) -> bool {
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

    pub fn is_full_range(&self) -> bool {
        self.get().color_range == sys::AVCOL_RANGE_JPEG
    }

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

    pub fn audio_planes(&self) -> usize {
        let planar = unsafe { sys::av_sample_fmt_is_planar(self.sample_format()) } != 0;
        if planar { self.channels().max(0) as usize } else { 1 }
    }

    pub fn audio_plane(&self, index: usize) -> *mut u8 {
        if index >= self.audio_planes() {
            return std::ptr::null_mut();
        }
        unsafe { *self.get().extended_data.add(index) }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { sys::av_frame_free(&mut self.0) }
    }
}
