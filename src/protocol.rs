use crate::obs::OwnedData;

pub const MESSAGE_HEADER_SIZE: usize = 5;
pub const MAX_MESSAGE_LENGTH: u32 = 4 * 1024 * 1024;
pub const MESSAGE_HOST_HELLO: u8 = 0x01;
pub const MESSAGE_DEVICE_HELLO: u8 = 0x02;
pub const MESSAGE_VIDEO_CONFIG: u8 = 0x03;
pub const MESSAGE_VIDEO_FRAME: u8 = 0x04;
pub const MESSAGE_AUDIO_CONFIG: u8 = 0x05;
pub const MESSAGE_AUDIO_FRAME: u8 = 0x06;
const PROTOCOL_VERSION: u8 = 1;
pub const HOST_HELLO_SIZE: usize = 10;
pub const VIDEO_CODEC_H264: u8 = 0;
pub const VIDEO_CODEC_HEVC: u8 = 1;
pub const AUDIO_CODEC_AAC_LC: u8 = 0;

pub fn video_codec_name(codec: u8) -> &'static str {
    match codec {
        VIDEO_CODEC_H264 => "H.264",
        VIDEO_CODEC_HEVC => "HEVC",
        _ => "unknown",
    }
}

pub fn audio_codec_name(codec: u8) -> &'static str {
    match codec {
        AUDIO_CODEC_AAC_LC => "AAC-LC",
        _ => "unknown",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceHello {
    pub version: u8,
    pub name: String,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfig<'a> {
    pub codec: u8,
    pub width: u16,
    pub height: u16,
    pub record: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame<'a> {
    pub pts_us: u64,
    pub keyframe: bool,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioConfig<'a> {
    pub codec: u8,
    pub sample_rate: u32,
    pub channels: u8,
    pub record: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame<'a> {
    pub pts_us: u64,
    pub data: &'a [u8],
}

fn u16_be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn u64_be(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes[..8].try_into().expect("eight bytes"))
}

pub fn pack_host_hello() -> [u8; HOST_HELLO_SIZE] {
    let mut buffer = [0u8; HOST_HELLO_SIZE];
    buffer[0..4].copy_from_slice(&6u32.to_be_bytes());
    buffer[4] = MESSAGE_HOST_HELLO;
    buffer[5..9].copy_from_slice(b"MOBL");
    buffer[9] = PROTOCOL_VERSION;
    buffer
}

pub fn parse_message_header(header: &[u8; MESSAGE_HEADER_SIZE]) -> (u32, u8) {
    (u32_be(header), header[4])
}

pub fn parse_device_hello(payload: &[u8]) -> Option<DeviceHello> {
    if payload.len() < 5 {
        return None;
    }
    let json_size = u32_be(&payload[1..]) as usize;
    let json = payload.get(5..5usize.checked_add(json_size)?)?;
    let json = std::ffi::CString::new(json).ok()?;
    let data = OwnedData::from_json(&json)?;
    Some(DeviceHello {
        version: payload[0],
        name: data.string(c"name"),
        app_version: data.string(c"version"),
    })
}

pub fn parse_video_config(payload: &[u8]) -> Option<VideoConfig<'_>> {
    if payload.len() < 9 {
        return None;
    }
    let record_size = u32_be(&payload[5..]) as usize;
    Some(VideoConfig {
        codec: payload[0],
        width: u16_be(&payload[1..]),
        height: u16_be(&payload[3..]),
        record: payload.get(9..9usize.checked_add(record_size)?)?,
    })
}

pub fn parse_video_frame(payload: &[u8]) -> Option<VideoFrame<'_>> {
    if payload.len() < 9 {
        return None;
    }
    Some(VideoFrame {
        pts_us: u64_be(payload),
        keyframe: (payload[8] & 1) != 0,
        data: &payload[9..],
    })
}

pub fn parse_audio_config(payload: &[u8]) -> Option<AudioConfig<'_>> {
    if payload.len() < 10 {
        return None;
    }
    let record_size = u32_be(&payload[6..]) as usize;
    Some(AudioConfig {
        codec: payload[0],
        sample_rate: u32_be(&payload[1..]),
        channels: payload[5],
        record: payload.get(10..10usize.checked_add(record_size)?)?,
    })
}

pub fn parse_audio_frame(payload: &[u8]) -> Option<AudioFrame<'_>> {
    if payload.len() < 8 {
        return None;
    }
    Some(AudioFrame {
        pts_us: u64_be(payload),
        data: &payload[8..],
    })
}
