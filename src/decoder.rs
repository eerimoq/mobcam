use crate::ffmpeg::{self, sys as av, Codec, Context, Device, Packet, Status};
use crate::obs::{self, media, Audio, Frame, Level};
use crate::obs_log;
use crate::protocol::{
    audio_codec_name, video_codec_name, AudioConfig, AudioFrame, VideoConfig, VideoFrame, AUDIO_CODEC_AAC_LC,
    VIDEO_CODEC_H264, VIDEO_CODEC_HEVC,
};

pub const INPUT_PADDING: usize = 64;

const _: () = assert!(
    INPUT_PADDING >= ffmpeg::INPUT_BUFFER_PADDING,
    "INPUT_PADDING is smaller than what this libavcodec requires"
);

struct Hardware {
    device: Device,
    frame: ffmpeg::Frame,
}

struct Stream {
    context: Option<Context>,
    hardware: Option<Hardware>,
    packet: Packet,
    frame: ffmpeg::Frame,
    decoded: Decoded,
    codec: u8,
    record: Vec<u8>,
    hardware_requested: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Decoded {
    Own,
    Hardware,
}

enum Received {
    Frame,
    Drained,
    NotTransferred,
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

    fn send(&mut self, data: &[u8], pts: i64, keyframe: bool) -> bool {
        let Some(context) = self.context.as_mut() else {
            return false;
        };
        context.send(&mut self.packet, data, pts, keyframe).is_ok()
    }

    fn receive(&mut self) -> Received {
        let Some(context) = self.context.as_mut() else {
            return Received::Drained;
        };
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

    fn decoded(&self) -> &ffmpeg::Frame {
        match self.hardware.as_ref() {
            Some(hardware) if self.decoded == Decoded::Hardware => &hardware.frame,
            _ => &self.frame,
        }
    }

    fn release(&mut self) {
        if let Some(hardware) = self.hardware.as_mut() {
            hardware.frame.unref();
        }
        self.frame.unref();
    }
}

pub trait Sink {
    fn video(&mut self, frame: &mut Frame, pts_us: u64);
    fn audio(&mut self, audio: &mut Audio, pts_us: u64);
}

pub struct Decoder {
    video: Stream,
    audio: Stream,
    hardware: bool,
    got_keyframe: bool,
    logged_pixel_format: Option<av::AVPixelFormat>,
    logged_transfer_failure: bool,
    logged_sample_format: Option<av::AVSampleFormat>,
    logged_channels: Option<i32>,
}

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

    pub fn set_hardware(&mut self, hardware: bool) {
        self.hardware = hardware;
    }

    pub fn reset(&mut self) {
        self.video.flush();
        self.audio.flush();
        self.got_keyframe = false;
    }

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
