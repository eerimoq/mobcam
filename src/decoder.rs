//! Decoding the video and audio the phone sends, with libavcodec.
//!
//! Access units are handed to the decoder in place, out of the receive buffer,
//! which is why that buffer carries the trailing padding libavcodec requires.
//! Nothing is converted here: a format OBS cannot take is dropped and reported,
//! which keeps libswscale out of the plugin and a frame of latency out of the
//! stream.
//!
//! Everything libavcodec is asked to do goes through the wrappers in
//! `crate::ffmpeg`, so the only unsafe left in this file is the promise that a
//! decoder stays on the thread it was moved to.

use crate::ffmpeg::{self, sys as av, Codec, Context, Device, Packet, Status};
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
    INPUT_PADDING >= ffmpeg::INPUT_BUFFER_PADDING,
    "INPUT_PADDING is smaller than what this libavcodec requires"
);

/// The hardware a stream decodes on, when it has any, and the frame its output
/// lands in before being brought down into system memory.
struct Hardware {
    device: Device,
    frame: ffmpeg::Frame,
}

/// One decoder and the config message it was opened for.
struct Stream {
    /// Allocated when a config message arrives and opened once it has been
    /// configured, so a context here is not necessarily one that decodes.
    ///
    /// Declared before `hardware` because the context holds a reference of its
    /// own on the device and has to let go of it first.
    context: Option<Context>,
    hardware: Option<Hardware>,

    packet: Packet,
    /// Where a frame decoded in software lands, and where one decoded on the
    /// hardware is brought down to.
    frame: ffmpeg::Frame,
    /// Which of the two frames the last receive left its output in.
    decoded: Decoded,

    codec: u8,
    record: Vec<u8>,
    /// What the caller asked for, which is not always what was opened.
    hardware_requested: bool,
}

/// Which of a stream's two frames holds what it last decoded.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Decoded {
    /// The stream's own frame: either it decoded in software, or the frame was
    /// brought down into it from the hardware.
    Own,
    /// The hardware frame, which is also where a hardware decoder that fell
    /// back to software mid-stream leaves its output.
    Hardware,
}

/// What asking a stream for one decoded frame came back with.
enum Received {
    /// A frame is waiting in `decoded()`.
    Frame,
    /// The decoder has handed over everything this access unit held.
    Drained,
    /// A frame was decoded but could not be read back from the hardware, and
    /// has been dropped.
    NotTransferred,
    /// The decoder failed.
    Failed,
}

impl Stream {
    fn new() -> Option<Self> {
        Some(Self {
            context: None,
            hardware: None,
            packet: Packet::new()?,
            frame: ffmpeg::Frame::new()?,
            decoded: Decoded::Own,
            codec: 0,
            record: Vec::new(),
            hardware_requested: false,
        })
    }

    fn is_open(&self) -> bool {
        self.context.as_ref().is_some_and(Context::is_open)
    }

    /// True when the stream is already decoding exactly this configuration, so
    /// that a repeated config message costs nothing.
    fn configured(&self, codec: u8, hardware: bool, record: &[u8]) -> bool {
        self.is_open() && self.codec == codec && self.hardware_requested == hardware && self.record == record
    }

    fn close(&mut self) {
        self.context = None;
        self.hardware = None;
        self.record.clear();
    }

    fn flush(&mut self) {
        if let Some(context) = self.context.as_mut() {
            context.flush();
        }
    }

    /// Allocates a context for a codec and gives it the configuration record.
    /// What comes back is the context to configure the rest of the way before
    /// opening it.
    fn begin(&mut self, codec: Codec, record: &[u8]) -> Option<&mut Context> {
        let mut context = Context::new(codec)?;

        if !context.set_extradata(record) {
            return None;
        }

        Some(self.context.insert(context))
    }

    fn open(&mut self, codec: Codec, wire_codec: u8, hardware: bool, record: &[u8]) -> bool {
        let Some(context) = self.context.as_mut() else {
            return false;
        };

        if !context.open(codec) {
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
    fn attach_hardware(&mut self, codec: Codec) {
        let Some(context) = self.context.as_mut() else {
            return;
        };

        let Some(device) = Device::open(codec) else {
            return;
        };

        let Some(frame) = ffmpeg::Frame::new() else {
            return;
        };

        context.set_hardware_device(&device);
        self.hardware = Some(Hardware { device, frame });
    }

    /// Hands one access unit to the decoder. The receive buffer is padded, so
    /// it can be referenced in place; the decoder copies what it keeps.
    fn send(&mut self, data: &[u8], pts: i64, keyframe: bool) -> bool {
        let Some(context) = self.context.as_mut() else {
            return false;
        };

        context.send(&mut self.packet, data, pts, keyframe).is_ok()
    }

    /// Takes one decoded frame out of the decoder, bringing it down into system
    /// memory when it was decoded on the hardware, since that is the only place
    /// OBS can read it from. A frame already there is left alone, which covers a
    /// hardware decoder falling back to software mid-stream.
    fn receive(&mut self) -> Received {
        let Some(context) = self.context.as_mut() else {
            return Received::Drained;
        };

        // The decoder puts its output in the hardware frame whenever there is
        // one, whether or not this particular frame stayed on the hardware.
        let status = match self.hardware.as_mut() {
            Some(hardware) => context.receive(&mut hardware.frame),
            None => context.receive(&mut self.frame),
        };

        match status {
            Status::Again | Status::Eof => return Received::Drained,
            Status::Error(_) => return Received::Failed,
            Status::Ok => (),
        }

        let Some(hardware) = self.hardware.as_ref() else {
            self.decoded = Decoded::Own;
            return Received::Frame;
        };

        // A frame that is already in system memory - which is what a hardware
        // decoder falling back to software mid-stream looks like - is passed on
        // from where it is.
        if hardware.frame.pixel_format() != hardware.device.format() {
            self.decoded = Decoded::Hardware;
            return Received::Frame;
        }

        if !self.frame.download(&hardware.frame) {
            return Received::NotTransferred;
        }

        self.decoded = Decoded::Own;

        Received::Frame
    }

    /// The frame the last `receive` left its output in.
    fn decoded(&self) -> &ffmpeg::Frame {
        match self.hardware.as_ref() {
            Some(hardware) if self.decoded == Decoded::Hardware => &hardware.frame,
            _ => &self.frame,
        }
    }

    /// Lets go of what the last `receive` produced, in both frames, so that the
    /// decoder can fill them in again.
    fn release(&mut self) {
        if let Some(hardware) = self.hardware.as_mut() {
            hardware.frame.unref();
        }

        self.frame.unref();
    }
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

        let Some(codec) = Codec::find(codec_id) else {
            obs_log!(Level::Error, "no {} decoder available", video_codec_name(config.codec));
            return false;
        };

        // The avcC or hvcC record goes in unmodified, which is what makes the
        // decoder expect the length prefixed access units that arrive on the
        // wire rather than Annex-B.
        let Some(context) = self.video.begin(codec, config.record) else {
            return false;
        };

        context.set_size(i32::from(config.width), i32::from(config.height));
        context.set_low_latency();

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
            Some(hardware) => hardware.device.name(),
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

        let Some(codec) = Codec::find(codec_id) else {
            obs_log!(Level::Error, "no {} decoder available", audio_codec_name(config.codec));
            return false;
        };

        // The AudioSpecificConfig goes in as extradata, so the decoder knows
        // what the raw access units on the wire hold without an ADTS header in
        // front of each one.
        let Some(context) = self.audio.begin(codec, config.record) else {
            return false;
        };

        context.set_audio(config.sample_rate as i32, i32::from(config.channels));

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

        loop {
            match self.video.receive() {
                Received::Drained => break,
                Received::Failed => return false,
                Received::NotTransferred => {
                    if !self.logged_transfer_failure {
                        self.logged_transfer_failure = true;
                        obs_log!(Level::Warning, "failed to read a frame back from the hardware decoder");
                    }
                }
                Received::Frame => self.emit_video(sink),
            }

            self.video.release();
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
            match self.audio.receive() {
                Received::Drained => break,
                Received::Failed | Received::NotTransferred => {
                    obs_log!(Level::Warning, "failed to decode audio, flushing the decoder");
                    self.audio.flush();
                    break;
                }
                Received::Frame => self.emit_audio(sink),
            }

            self.audio.release();
        }
    }

    /// Describes a decoded frame to OBS, in place, and hands it over.
    fn emit_video(&mut self, sink: &mut dyn Sink) {
        let source = self.video.decoded();

        let Some((format, format_is_full_range)) = media::video_format(source.pixel_format()) else {
            if self.logged_pixel_format != Some(source.pixel_format()) {
                self.logged_pixel_format = Some(source.pixel_format());
                obs_log!(
                    Level::Warning,
                    "unsupported pixel format {}",
                    ffmpeg::pixel_format_name(source.pixel_format())
                );
            }

            return;
        };

        let full_range = format_is_full_range || source.is_full_range();

        let mut frame = Frame {
            format,
            width: source.width() as u32,
            height: source.height() as u32,
            full_range,
            trc: media::transfer(source),
            ..Default::default()
        };

        for plane in 0..(obs::sys::MAX_AV_PLANES as usize).min(ffmpeg::Frame::PLANES) {
            let (data, linesize) = source.plane(plane);

            frame.data[plane] = data;
            frame.linesize[plane] = linesize as u32;
        }

        media::set_color_parameters(&mut frame, media::colorspace(source), full_range);

        let pts = source.pts();

        sink.video(&mut frame, pts as u64);
    }

    /// Describes decoded audio to OBS, in place, and hands it over.
    fn emit_audio(&mut self, sink: &mut dyn Sink) {
        let source = self.audio.decoded();

        let Some(format) = media::audio_format(source.sample_format()) else {
            if self.logged_sample_format != Some(source.sample_format()) {
                self.logged_sample_format = Some(source.sample_format());
                obs_log!(
                    Level::Warning,
                    "unsupported sample format {}",
                    ffmpeg::sample_format_name(source.sample_format())
                );
            }

            return;
        };

        let channels = source.channels();

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
            frames: source.samples() as u32,
            samples_per_sec: source.sample_rate() as u32,
            ..Default::default()
        };

        let planes = media::audio_planes(format, speakers).min(obs::sys::MAX_AV_PLANES as usize);

        for plane in 0..planes {
            audio.data[plane] = source.audio_plane(plane);
        }

        let pts = source.pts();

        sink.audio(&mut audio, pts as u64);
    }
}
