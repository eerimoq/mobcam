use crate::ffmpeg::{self, Codec, Context, Device, Packet, Status, sys as av};
use crate::protocol::{
    AUDIO_CODEC_AAC_LC, AudioConfig, AudioFrame, VIDEO_CODEC_H264, VIDEO_CODEC_HEVC, VideoConfig, VideoFrame,
};
use crate::{Level, log};

pub const INPUT_PADDING: usize = 64;

const _: () = assert!(
    INPUT_PADDING >= ffmpeg::INPUT_BUFFER_PADDING,
    "INPUT_PADDING is smaller than what this libavcodec requires"
);

/// How the image of a decoded frame is got out of the device.
#[derive(Clone, Copy)]
enum Access {
    /// Not settled yet; the first frame decides.
    Unknown,
    /// Pointed at where it lies, in this pixel format.
    Map(av::AVPixelFormat),
    /// Copied into memory of our own.
    Copy,
}

struct Stream {
    context: Option<Context>,
    /// The device the decoder was asked to work in, if it took one.
    device: Option<Device>,
    packet: Packet,
    /// What the decoder writes into, which may live in the device.
    decoded: ffmpeg::Frame,
    /// What the sink is given, always in memory it can read.
    frame: ffmpeg::Frame,
    access: Access,
    codec: u8,
    name: String,
    record: Vec<u8>,
}

enum Received {
    Frame,
    Drained,
    Failed,
}

impl Stream {
    fn new() -> Option<Self> {
        Some(Self {
            context: None,
            device: None,
            packet: Packet::new()?,
            decoded: ffmpeg::Frame::new()?,
            frame: ffmpeg::Frame::new()?,
            access: Access::Unknown,
            codec: 0,
            name: String::new(),
            record: Vec::new(),
        })
    }

    fn decoder_name(&self) -> String {
        match self.device.as_ref() {
            Some(device) => format!("{} on {}", self.name, device.name()),
            None => self.name.clone(),
        }
    }

    fn is_open(&self) -> bool {
        self.context.as_ref().is_some_and(Context::is_open)
    }

    fn configured(&self, codec: u8, record: &[u8]) -> bool {
        self.is_open() && self.codec == codec && self.record == record
    }

    fn close(&mut self) {
        self.context = None;
        self.device = None;
        self.access = Access::Unknown;
        self.name.clear();
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

    fn open(&mut self, codec: Codec, wire_codec: u8, record: &[u8]) -> bool {
        let Some(context) = self.context.as_mut() else {
            return false;
        };
        if !context.open(codec) {
            self.close();
            return false;
        }
        self.codec = wire_codec;
        self.name = codec.name();
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
        context.set_hardware_device(&device);
        self.device = Some(device);
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
        match context.receive(&mut self.decoded) {
            Status::Again | Status::Eof => return Received::Drained,
            Status::Error(_) => return Received::Failed,
            Status::Ok => (),
        }
        // A hardware decoder hands over a frame that lives in the device
        // whether or not it was given one to work in, so what has to be done
        // with the frame follows from the frame and not from what we asked for.
        if !self.decoded.is_hardware() {
            self.frame.move_from(&mut self.decoded);
            return Received::Frame;
        }
        if !self.fetch() {
            return Received::Failed;
        }
        Received::Frame
    }

    /// Get at the image of a frame that lives in the device, mapping it where
    /// that is cheaper than copying and copying it out everywhere else.
    fn fetch(&mut self) -> bool {
        if let Access::Unknown = self.access {
            // Only a device we opened ourselves is known to be one worth
            // mapping, so a frame from any other is copied out.
            let format = self
                .device
                .as_ref()
                .filter(|device| device.maps_cheaply())
                .and_then(|_| self.decoded.transfer_format());
            self.access = match format {
                Some(format) => {
                    log!(
                        Level::Info,
                        "mapping the {} frames rather than copying them",
                        self.device_name()
                    );
                    Access::Map(format)
                }
                None => Access::Copy,
            };
        }
        if let Access::Map(format) = self.access {
            if self.frame.map(&self.decoded, format) {
                return true;
            }
            log!(
                Level::Info,
                "the {} frames cannot be mapped, copying them instead",
                self.device_name()
            );
            self.access = Access::Copy;
        }
        self.frame.download(&self.decoded)
    }

    /// What to call the device the frames come from, in a message.
    fn device_name(&self) -> String {
        match self.device.as_ref() {
            Some(device) => device.name(),
            None => String::from("hardware"),
        }
    }

    fn release(&mut self) {
        self.decoded.unref();
        self.frame.unref();
    }
}

pub trait Sink {
    fn video(&mut self, frame: &ffmpeg::Frame);
    fn audio(&mut self, frame: &ffmpeg::Frame);
}

pub struct Decoder {
    video: Stream,
    audio: Stream,
    hardware: bool,
    audio_wanted: bool,
    got_keyframe: bool,
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        Some(Self {
            video: Stream::new()?,
            audio: Stream::new()?,
            hardware: false,
            audio_wanted: true,
            got_keyframe: false,
        })
    }

    pub fn set_hardware(&mut self, hardware: bool) {
        self.hardware = hardware;
    }

    pub fn set_audio(&mut self, audio: bool) {
        self.audio_wanted = audio;
        if !audio {
            self.audio.close();
        }
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
                log!(Level::Warning, "unsupported video codec {codec}");
                return false;
            }
        };
        if self.video.configured(config.codec, config.record) {
            return true;
        }
        self.video.close();
        self.got_keyframe = false;
        for codec in Codec::decoders_for(codec_id, self.hardware) {
            let Some(context) = self.video.begin(codec, config.record) else {
                return false;
            };
            context.set_size(i32::from(config.width), i32::from(config.height));
            context.set_low_latency();
            if self.hardware {
                self.video.attach_hardware(codec);
            }
            if !self.video.open(codec, config.codec, config.record) {
                log!(Level::Warning, "failed to open the {} decoder", codec.name());
                continue;
            }
            log!(
                Level::Info,
                "decoding {} {}x{} in {}",
                config.video_codec_name(),
                config.width,
                config.height,
                self.video.decoder_name()
            );
            return true;
        }
        log!(
            Level::Error,
            "no working {} decoder available",
            config.video_codec_name()
        );
        false
    }

    pub fn configure_audio(&mut self, config: &AudioConfig<'_>) -> bool {
        if !self.audio_wanted {
            return true;
        }
        let codec_id = match config.codec {
            AUDIO_CODEC_AAC_LC => av::AV_CODEC_ID_AAC,
            codec => {
                log!(Level::Warning, "unsupported audio codec {codec}");
                return false;
            }
        };
        if self.audio.configured(config.codec, config.record) {
            return true;
        }
        self.audio.close();
        for codec in Codec::decoders_for(codec_id, false) {
            let Some(context) = self.audio.begin(codec, config.record) else {
                return false;
            };
            context.set_audio(config.sample_rate, i32::from(config.channels));
            if !self.audio.open(codec, config.codec, config.record) {
                log!(Level::Warning, "failed to open the {} decoder", codec.name());
                continue;
            }
            log!(
                Level::Info,
                "decoding {} {} Hz {} channel in {}",
                config.audio_codec_name(),
                config.sample_rate,
                config.channels,
                self.audio.decoder_name()
            );
            return true;
        }
        log!(
            Level::Error,
            "no working {} decoder available",
            config.audio_codec_name()
        );
        false
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
            log!(Level::Warning, "failed to decode a frame, flushing the decoder");
            self.video.flush();
            self.got_keyframe = false;
            return true;
        }
        loop {
            match self.video.receive() {
                Received::Drained => break,
                Received::Failed => {
                    log!(Level::Warning, "failed to receive a frame, reopening the decoder");
                    self.video.close();
                    return false;
                }
                Received::Frame => sink.video(&self.video.frame),
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
            log!(Level::Warning, "failed to decode audio, flushing the decoder");
            self.audio.flush();
            return;
        }
        loop {
            match self.audio.receive() {
                Received::Drained => break,
                Received::Failed => {
                    log!(Level::Warning, "failed to decode audio, flushing the decoder");
                    self.audio.flush();
                    break;
                }
                Received::Frame => sink.audio(&self.audio.frame),
            }
            self.audio.release();
        }
    }
}
