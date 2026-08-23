//! Decoding the video and audio the phone sends, with libavcodec.
//!
//! Access units are handed to the decoder in place, out of the receive buffer,
//! which is why that buffer carries the trailing padding libavcodec requires.
//! Nothing is converted here: a format OBS cannot take is dropped and reported,
//! which keeps libswscale out of the plugin and a frame of latency out of the
//! stream.

use std::ffi::CStr;
use std::ptr;

use crate::ffmpeg::sys as av;
use crate::obs::{self, media, Audio, Frame, Level};
use crate::obs_log;
use crate::protocol::{
    audio_codec_name, video_codec_name, AudioConfig, AudioFrame, VideoConfig, VideoFrame, AUDIO_CODEC_AAC_LC,
    VIDEO_CODEC_H264, VIDEO_CODEC_HEVC,
};

/// Access units are decoded in place, so the buffer they live in must have this
/// many zeroed bytes after them.
pub const INPUT_PADDING: usize = 64;

const _: () = assert!(
    INPUT_PADDING >= av::AV_INPUT_BUFFER_PADDING_SIZE as usize,
    "INPUT_PADDING is smaller than what this libavcodec requires"
);

/// Hardware decoders to try, in the order they are preferred. Every machine has
/// only one or two of these, and creating a device for the rest simply fails,
/// which is what picks the right one here.
const HARDWARE_TYPES: [av::AVHWDeviceType; 7] = [
    av::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
    av::AV_HWDEVICE_TYPE_D3D11VA,
    av::AV_HWDEVICE_TYPE_CUDA,
    av::AV_HWDEVICE_TYPE_VAAPI,
    av::AV_HWDEVICE_TYPE_QSV,
    av::AV_HWDEVICE_TYPE_DXVA2,
    av::AV_HWDEVICE_TYPE_VDPAU,
];

/// The hardware device a stream decodes on, when it has one.
struct Hardware {
    device: *mut av::AVBufferRef,
    /// Frames arrive in this format and are copied down into system memory.
    frame: *mut av::AVFrame,
    kind: av::AVHWDeviceType,
    format: av::AVPixelFormat,
}

impl Drop for Hardware {
    fn drop(&mut self) {
        // SAFETY: both pointers were produced by the matching allocators and
        // are released exactly once, here.
        unsafe {
            av::av_buffer_unref(&mut self.device);
            av::av_frame_free(&mut self.frame);
        }
    }
}

/// One decoder and the config message it was opened for.
struct Stream {
    context: *mut av::AVCodecContext,
    packet: *mut av::AVPacket,
    frame: *mut av::AVFrame,

    codec: u8,
    record: Vec<u8>,
    /// What the caller asked for, which is not always what was opened.
    hardware_requested: bool,
    hardware: Option<Hardware>,
}

impl Stream {
    fn new() -> Option<Self> {
        // SAFETY: both allocate and return null on failure, which is checked.
        let (packet, frame) = unsafe { (av::av_packet_alloc(), av::av_frame_alloc()) };

        if packet.is_null() || frame.is_null() {
            // SAFETY: freeing whichever of the two did succeed.
            unsafe {
                av::av_packet_free(&mut { packet });
                av::av_frame_free(&mut { frame });
            }

            return None;
        }

        Some(Self {
            context: ptr::null_mut(),
            packet,
            frame,
            codec: 0,
            record: Vec::new(),
            hardware_requested: false,
            hardware: None,
        })
    }

    fn is_open(&self) -> bool {
        !self.context.is_null()
    }

    /// True when the stream is already decoding exactly this configuration, so
    /// that a repeated config message costs nothing.
    fn configured(&self, codec: u8, hardware: bool, record: &[u8]) -> bool {
        self.is_open() && self.codec == codec && self.hardware_requested == hardware && self.record == record
    }

    fn close(&mut self) {
        if !self.context.is_null() {
            // SAFETY: the context was allocated by avcodec_alloc_context3 and
            // is freed once. This also drops its reference on the hardware
            // device, so the device must outlive the call, which it does.
            unsafe { av::avcodec_free_context(&mut self.context) }
        }

        self.hardware = None;
        self.record.clear();
    }

    fn flush(&mut self) {
        if self.is_open() {
            // SAFETY: the context is open.
            unsafe { av::avcodec_flush_buffers(self.context) }
        }
    }

    /// Allocates a context holding the configuration record as extradata.
    fn begin(&mut self, codec: *const av::AVCodec, record: &[u8]) -> bool {
        // SAFETY: codec was returned by avcodec_find_decoder and is not null.
        self.context = unsafe { av::avcodec_alloc_context3(codec) };

        if self.context.is_null() {
            return false;
        }

        if record.is_empty() {
            return true;
        }

        let size = record.len() + av::AV_INPUT_BUFFER_PADDING_SIZE as usize;

        // SAFETY: av_mallocz zeroes the allocation, so the padding past the
        // record is already what libavcodec expects.
        let extradata = unsafe { av::av_mallocz(size) } as *mut u8;

        if extradata.is_null() {
            self.close();
            return false;
        }

        // SAFETY: extradata has room for record plus the padding, and the two
        // regions do not overlap.
        unsafe {
            ptr::copy_nonoverlapping(record.as_ptr(), extradata, record.len());

            (*self.context).extradata = extradata;
            (*self.context).extradata_size = record.len() as i32;
        }

        true
    }

    fn open(&mut self, codec: *const av::AVCodec, wire_codec: u8, hardware: bool, record: &[u8]) -> bool {
        // SAFETY: the context was allocated in begin() and not yet opened.
        if unsafe { av::avcodec_open2(self.context, codec, ptr::null_mut()) } < 0 {
            self.close();
            return false;
        }

        self.codec = wire_codec;
        self.hardware_requested = hardware;
        self.record = record.to_vec();

        true
    }

    /// Attaches the first hardware device this codec decodes on. Attaching none
    /// is a normal outcome and leaves the context decoding in software.
    fn attach_hardware(&mut self, codec: *const av::AVCodec) {
        for kind in HARDWARE_TYPES {
            let Some(format) = hardware_pixel_format(codec, kind) else {
                continue;
            };

            let mut device: *mut av::AVBufferRef = ptr::null_mut();

            // SAFETY: creating a device for hardware the machine does not have
            // fails rather than misbehaving, which is what picks the right one.
            let created = unsafe { av::av_hwdevice_ctx_create(&mut device, kind, ptr::null(), ptr::null_mut(), 0) };

            if created < 0 {
                continue;
            }

            // SAFETY: allocation, checked below.
            let frame = unsafe { av::av_frame_alloc() };

            if frame.is_null() {
                // SAFETY: the device was just created and is dropped unused.
                unsafe { av::av_buffer_unref(&mut device) };
                return;
            }

            // SAFETY: the context takes its own reference on the device, so
            // both this struct and the context can release theirs.
            unsafe { (*self.context).hw_device_ctx = av::av_buffer_ref(device) };

            self.hardware = Some(Hardware {
                device,
                frame,
                kind,
                format,
            });

            return;
        }
    }

    /// Hands one access unit to the decoder. The receive buffer is padded, so
    /// it can be referenced in place; avcodec_send_packet copies what it keeps.
    fn send(&mut self, data: &[u8], pts: i64, keyframe: bool) -> bool {
        // SAFETY: the packet is live. Its data pointer is cleared again below,
        // so it never outlives the borrow of the caller's buffer.
        let result = unsafe {
            let packet = self.packet;

            av::av_packet_unref(packet);

            (*packet).data = data.as_ptr() as *mut u8;
            (*packet).size = data.len() as i32;
            (*packet).pts = pts;
            (*packet).dts = pts;

            if keyframe {
                (*packet).flags |= av::AV_PKT_FLAG_KEY as i32;
            }

            let result = av::avcodec_send_packet(self.context, packet);

            (*packet).data = ptr::null_mut();
            (*packet).size = 0;

            result
        };

        result >= 0 || result == averror(libc_eagain())
    }

    /// Brings a decoded frame down into system memory, since that is the only
    /// place OBS can take it from. A frame already there is passed through,
    /// which covers a hardware decoder falling back to software mid-stream.
    fn download(&mut self, frame: *mut av::AVFrame) -> Option<*mut av::AVFrame> {
        let Some(hardware) = self.hardware.as_ref() else {
            return Some(frame);
        };

        // SAFETY: frame was filled in by avcodec_receive_frame.
        if unsafe { (*frame).format } != hardware.format {
            return Some(frame);
        }

        // SAFETY: self.frame is the destination and is unreferenced first, as
        // av_hwframe_transfer_data requires.
        unsafe {
            av::av_frame_unref(self.frame);

            if av::av_hwframe_transfer_data(self.frame, frame, 0) < 0 {
                return None;
            }

            if av::av_frame_copy_props(self.frame, frame) < 0 {
                return None;
            }
        }

        Some(self.frame)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.close();

        // SAFETY: both were allocated in new() and are freed exactly once.
        unsafe {
            av::av_packet_free(&mut self.packet);
            av::av_frame_free(&mut self.frame);
        }
    }
}

/// EAGAIN as libavcodec reports it. FFmpeg's AVERROR macro is arithmetic on the
/// platform errno, which bindgen cannot emit.
fn libc_eagain() -> i32 {
    // EAGAIN is 11 on Linux and 35 on the BSDs, macOS included; Windows builds
    // of FFmpeg use the same value as the MSVC runtime.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    return 35;

    #[cfg(target_os = "linux")]
    return 11;

    #[cfg(windows)]
    return 11;
}

fn averror(errno: i32) -> i32 {
    -errno
}

/// libavcodec's end-of-stream error. FFmpeg builds it with the FFERRTAG macro,
/// which bindgen cannot emit, so the same four character tag is spelled out.
const AVERROR_EOF: i32 = -((b'E' as i32) | ((b'O' as i32) << 8) | ((b'F' as i32) << 16) | ((b' ' as i32) << 24));

/// The pixel format frames from this codec arrive in on one kind of device, or
/// nothing when the codec cannot be decoded on it at all.
fn hardware_pixel_format(codec: *const av::AVCodec, kind: av::AVHWDeviceType) -> Option<av::AVPixelFormat> {
    for index in 0.. {
        // SAFETY: codec is valid; the list ends with a null entry.
        let config = unsafe { av::avcodec_get_hw_config(codec, index) };

        if config.is_null() {
            return None;
        }

        // SAFETY: config points into the codec's static configuration table.
        let config = unsafe { &*config };

        if (config.methods & av::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32) != 0 && config.device_type == kind {
            return Some(config.pix_fmt);
        }
    }

    None
}

fn hardware_name(kind: av::AVHWDeviceType) -> String {
    // SAFETY: returns a static string for a known device type, or null.
    let name = unsafe { av::av_hwdevice_get_type_name(kind) };

    if name.is_null() {
        return String::from("hardware");
    }

    // SAFETY: a static NUL terminated string owned by libavutil.
    unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
}

fn pixel_format_name(format: av::AVPixelFormat) -> String {
    // SAFETY: returns a static string, or null for an unknown format.
    let name = unsafe { av::av_get_pix_fmt_name(format) };

    if name.is_null() {
        return format.to_string();
    }

    // SAFETY: a static NUL terminated string owned by libavutil.
    unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
}

fn sample_format_name(format: av::AVSampleFormat) -> String {
    // SAFETY: returns a static string, or null for an unknown format.
    let name = unsafe { av::av_get_sample_fmt_name(format) };

    if name.is_null() {
        return format.to_string();
    }

    // SAFETY: a static NUL terminated string owned by libavutil.
    unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
}

/// What a decoded frame is handed to. The frame borrows the decoder's memory
/// and is only valid until the call returns; its timestamp is left for the
/// caller to fill in from the pts.
pub trait Sink {
    fn video(&mut self, frame: &mut Frame, pts_us: u64);
    fn audio(&mut self, audio: &mut Audio, pts_us: u64);
}

pub struct Decoder {
    video: Stream,
    audio: Stream,

    /// Decode video on the GPU when the machine has something that can.
    hardware: bool,
    /// Video is held back until the first keyframe after every open.
    got_keyframe: bool,

    /// Unsupported formats are logged once instead of once per frame.
    logged_pixel_format: Option<av::AVPixelFormat>,
    logged_transfer_failure: bool,
    logged_sample_format: Option<av::AVSampleFormat>,
    logged_channels: Option<i32>,
}

// SAFETY: a Decoder owns its libavcodec contexts outright - nothing else holds
// a pointer into one - and it is only ever used from the single worker thread
// it is moved to. It is deliberately not Sync: two threads decoding through the
// same context at once is exactly what libavcodec does not allow.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        Some(Self {
            video: Stream::new()?,
            audio: Stream::new()?,
            hardware: false,
            got_keyframe: false,
            logged_pixel_format: None,
            logged_transfer_failure: false,
            logged_sample_format: None,
            logged_channels: None,
        })
    }

    /// Chooses whether video is decoded on the GPU. Hardware decoding is a
    /// request rather than a demand: a machine with nothing that decodes the
    /// codec falls back to software instead of failing. Takes effect the next
    /// time the video decoder is opened.
    pub fn set_hardware(&mut self, hardware: bool) {
        self.hardware = hardware;
    }

    /// Drops whatever the decoders were in the middle of and waits for a
    /// keyframe again. Called when a connection starts, since the frames on
    /// either side of it belong to different encoder sessions.
    pub fn reset(&mut self) {
        self.video.flush();
        self.audio.flush();
        self.got_keyframe = false;
    }

    /// Opens, or reopens, the video decoder for a config message.
    pub fn configure_video(&mut self, config: &VideoConfig<'_>) -> bool {
        let codec_id = match config.codec {
            VIDEO_CODEC_H264 => av::AV_CODEC_ID_H264,
            VIDEO_CODEC_HEVC => av::AV_CODEC_ID_HEVC,
            codec => {
                obs_log!(Level::Warning, "unsupported video codec {codec}");
                return false;
            }
        };

        if self.video.configured(config.codec, self.hardware, config.record) {
            return true;
        }

        self.video.close();
        self.got_keyframe = false;

        // SAFETY: a lookup by id, returning a static codec or null.
        let codec = unsafe { av::avcodec_find_decoder(codec_id) };

        if codec.is_null() {
            obs_log!(Level::Error, "no {} decoder available", video_codec_name(config.codec));
            return false;
        }

        // The avcC or hvcC record goes in unmodified, which is what makes the
        // decoder expect the length prefixed access units that arrive on the
        // wire rather than Annex-B.
        if !self.video.begin(codec, config.record) {
            return false;
        }

        // SAFETY: the context was just allocated and is not open yet.
        unsafe {
            let context = self.video.context;

            (*context).width = i32::from(config.width);
            (*context).height = i32::from(config.height);
            (*context).flags |= av::AV_CODEC_FLAG_LOW_DELAY as i32;
            // Frame threading would add a frame of latency to a live camera.
            (*context).thread_type = av::FF_THREAD_SLICE as i32;
        }

        if self.hardware {
            self.video.attach_hardware(codec);
        }

        if !self.video.open(codec, config.codec, self.hardware, config.record) {
            obs_log!(
                Level::Error,
                "failed to open the {} decoder",
                video_codec_name(config.codec)
            );
            return false;
        }

        self.logged_pixel_format = None;
        self.logged_transfer_failure = false;

        let where_ = match self.video.hardware.as_ref() {
            Some(hardware) => hardware_name(hardware.kind),
            None => String::from("software"),
        };

        obs_log!(
            Level::Info,
            "decoding {} {}x{} in {where_}",
            video_codec_name(config.codec),
            config.width,
            config.height
        );

        true
    }

    /// Opens, or reopens, the audio decoder for a config message.
    pub fn configure_audio(&mut self, config: &AudioConfig<'_>) -> bool {
        let codec_id = match config.codec {
            AUDIO_CODEC_AAC_LC => av::AV_CODEC_ID_AAC,
            codec => {
                obs_log!(Level::Warning, "unsupported audio codec {codec}");
                return false;
            }
        };

        if self.audio.configured(config.codec, false, config.record) {
            return true;
        }

        self.audio.close();

        // SAFETY: a lookup by id, returning a static codec or null.
        let codec = unsafe { av::avcodec_find_decoder(codec_id) };

        if codec.is_null() {
            obs_log!(Level::Error, "no {} decoder available", audio_codec_name(config.codec));
            return false;
        }

        // The AudioSpecificConfig goes in as extradata, so the decoder knows
        // what the raw access units on the wire hold without an ADTS header in
        // front of each one.
        if !self.audio.begin(codec, config.record) {
            return false;
        }

        // SAFETY: the context was just allocated and is not open yet.
        unsafe {
            let context = self.audio.context;

            (*context).sample_rate = config.sample_rate as i32;
            av::av_channel_layout_default(&mut (*context).ch_layout, i32::from(config.channels));
        }

        if !self.audio.open(codec, config.codec, false, config.record) {
            obs_log!(
                Level::Error,
                "failed to open the {} decoder",
                audio_codec_name(config.codec)
            );
            return false;
        }

        self.logged_sample_format = None;
        self.logged_channels = None;

        obs_log!(
            Level::Info,
            "decoding {} {} Hz {} channel",
            audio_codec_name(config.codec),
            config.sample_rate,
            config.channels
        );

        true
    }

    /// Decodes one video access unit.
    ///
    /// Video is what the source is for, so a stream that cannot be decoded ends
    /// the connection and another one is tried; that is what a false return
    /// means.
    pub fn decode_video(&mut self, frame: &VideoFrame<'_>, sink: &mut dyn Sink) -> bool {
        if !self.video.is_open() {
            return true;
        }

        if !self.got_keyframe {
            if !frame.keyframe {
                return true;
            }

            self.got_keyframe = true;
        }

        if !self.video.send(frame.data, frame.pts_us as i64, frame.keyframe) {
            obs_log!(Level::Warning, "failed to decode a frame, flushing the decoder");
            self.video.flush();
            self.got_keyframe = false;
            return true;
        }

        let received = match self.video.hardware.as_ref() {
            Some(hardware) => hardware.frame,
            None => self.video.frame,
        };

        loop {
            // SAFETY: the context is open and received is a live frame.
            let result = unsafe { av::avcodec_receive_frame(self.video.context, received) };

            if result == averror(libc_eagain()) || result == AVERROR_EOF {
                break;
            }

            if result < 0 {
                return false;
            }

            match self.video.download(received) {
                None => {
                    if !self.logged_transfer_failure {
                        self.logged_transfer_failure = true;
                        obs_log!(Level::Warning, "failed to read a frame back from the hardware decoder");
                    }
                }
                Some(decoded) => self.emit_video(decoded, sink),
            }

            // SAFETY: both frames are live; unreferencing one twice is safe.
            unsafe {
                av::av_frame_unref(received);
                av::av_frame_unref(self.video.frame);
            }
        }

        true
    }

    /// Decodes one audio access unit. Audio that cannot be decoded is dropped,
    /// leaving the video running, so there is nothing for the caller to handle.
    pub fn decode_audio(&mut self, frame: &AudioFrame<'_>, sink: &mut dyn Sink) {
        if !self.audio.is_open() {
            return;
        }

        if !self.audio.send(frame.data, frame.pts_us as i64, true) {
            obs_log!(Level::Warning, "failed to decode audio, flushing the decoder");
            self.audio.flush();
            return;
        }

        loop {
            // SAFETY: the context is open and the frame is live.
            let result = unsafe { av::avcodec_receive_frame(self.audio.context, self.audio.frame) };

            if result == averror(libc_eagain()) || result == AVERROR_EOF {
                break;
            }

            if result < 0 {
                obs_log!(Level::Warning, "failed to decode audio, flushing the decoder");
                self.audio.flush();
                break;
            }

            self.emit_audio(sink);

            // SAFETY: the frame is live.
            unsafe { av::av_frame_unref(self.audio.frame) };
        }
    }

    /// Describes a decoded frame to OBS, in place, and hands it over.
    fn emit_video(&mut self, decoded: *mut av::AVFrame, sink: &mut dyn Sink) {
        // SAFETY: decoded was filled in by the decoder and outlives this call.
        let source = unsafe { &*decoded };

        let Some((format, format_is_full_range)) = media::video_format(source.format) else {
            if self.logged_pixel_format != Some(source.format) {
                self.logged_pixel_format = Some(source.format);
                obs_log!(
                    Level::Warning,
                    "unsupported pixel format {}",
                    pixel_format_name(source.format)
                );
            }

            return;
        };

        let full_range = format_is_full_range || source.color_range == av::AVCOL_RANGE_JPEG;

        let mut frame = Frame {
            format,
            width: source.width as u32,
            height: source.height as u32,
            full_range,
            trc: media::transfer(source),
            ..Default::default()
        };

        for plane in 0..obs::sys::MAX_AV_PLANES as usize {
            frame.data[plane] = source.data[plane];
            frame.linesize[plane] = source.linesize[plane] as u32;
        }

        media::set_color_parameters(&mut frame, media::colorspace(source), full_range);

        sink.video(&mut frame, source.pts as u64);
    }

    /// Describes decoded audio to OBS, in place, and hands it over.
    fn emit_audio(&mut self, sink: &mut dyn Sink) {
        // SAFETY: the frame was filled in by the decoder and outlives this call.
        let source = unsafe { &*self.audio.frame };

        let Some(format) = media::audio_format(source.format) else {
            if self.logged_sample_format != Some(source.format) {
                self.logged_sample_format = Some(source.format);
                obs_log!(
                    Level::Warning,
                    "unsupported sample format {}",
                    sample_format_name(source.format)
                );
            }

            return;
        };

        let channels = source.ch_layout.nb_channels;

        let Some(speakers) = media::speakers(channels) else {
            if self.logged_channels != Some(channels) {
                self.logged_channels = Some(channels);
                obs_log!(Level::Warning, "unsupported channel count {channels}");
            }

            return;
        };

        let mut audio = Audio {
            format,
            speakers,
            frames: source.nb_samples as u32,
            samples_per_sec: source.sample_rate as u32,
            ..Default::default()
        };

        let planes = media::audio_planes(format, speakers).min(obs::sys::MAX_AV_PLANES as usize);

        for plane in 0..planes {
            // SAFETY: extended_data holds at least as many planes as the layout
            // OBS was told about, which is what planes counts.
            audio.data[plane] = unsafe { *source.extended_data.add(plane) };
        }

        sink.audio(&mut audio, source.pts as u64);
    }
}
