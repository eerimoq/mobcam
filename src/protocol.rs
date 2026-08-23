//! The Moblin USB stream protocol, as described in moblin/docs/usb-protocol.md.
//!
//! Every message is a big endian u32 length covering the type byte and the
//! payload, then the type byte, then the payload. Everything here reads from a
//! borrowed slice: the payloads are large and land in the receive buffer, and
//! copying them again would be the plugin's largest cost per frame.

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
    /// The avcC or hvcC record.
    pub record: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame<'a> {
    pub pts_us: u64,
    pub keyframe: bool,
    /// One access unit in AVCC form.
    pub data: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioConfig<'a> {
    pub codec: u8,
    pub sample_rate: u32,
    pub channels: u8,
    /// The AudioSpecificConfig.
    pub record: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame<'a> {
    pub pts_us: u64,
    /// One raw access unit.
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

/// Writes the hello the host must send first.
pub fn pack_host_hello() -> [u8; HOST_HELLO_SIZE] {
    let mut buffer = [0u8; HOST_HELLO_SIZE];

    // The length covers the type byte, "MOBL" and the version byte.
    buffer[0..4].copy_from_slice(&6u32.to_be_bytes());
    buffer[4] = MESSAGE_HOST_HELLO;
    buffer[5..9].copy_from_slice(b"MOBL");
    buffer[9] = PROTOCOL_VERSION;

    buffer
}

pub fn parse_message_header(header: &[u8; MESSAGE_HEADER_SIZE]) -> (u32, u8) {
    (u32_be(header), header[4])
}

/// The device's name and app version arrive as JSON, read with the parser
/// libobs already carries.
pub fn parse_device_hello(payload: &[u8]) -> Option<DeviceHello> {
    if payload.len() < 5 {
        return None;
    }

    let json_size = u32_be(&payload[1..]) as usize;
    let json = payload.get(5..5usize.checked_add(json_size)?)?;

    // A NUL inside the JSON would end it early, and it has to be NUL terminated
    // to be handed to the C parser at all.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_the_host_hello() {
        assert_eq!(pack_host_hello(), [0, 0, 0, 6, 0x01, b'M', b'O', b'B', b'L', 1]);
    }

    #[test]
    fn reads_a_message_header() {
        assert_eq!(parse_message_header(&[0, 0, 0x10, 0x00, 0x04]), (0x1000, 0x04));
    }

    #[test]
    fn reads_a_video_config() {
        let mut payload = vec![VIDEO_CODEC_HEVC, 0x07, 0x80, 0x04, 0x38, 0, 0, 0, 3];

        payload.extend_from_slice(&[0xAA, 0xBB, 0xCC]);

        let config = parse_video_config(&payload).expect("parses");

        assert_eq!(config.codec, VIDEO_CODEC_HEVC);
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert_eq!(config.record, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn reads_a_video_frame() {
        let mut payload = 1_234_567u64.to_be_bytes().to_vec();

        payload.push(1);
        payload.extend_from_slice(&[1, 2, 3, 4]);

        let frame = parse_video_frame(&payload).expect("parses");

        assert_eq!(frame.pts_us, 1_234_567);
        assert!(frame.keyframe);
        assert_eq!(frame.data, &[1, 2, 3, 4]);
    }

    #[test]
    fn a_frame_without_the_keyframe_bit_is_not_one() {
        let mut payload = 0u64.to_be_bytes().to_vec();

        payload.push(0);

        assert!(!parse_video_frame(&payload).expect("parses").keyframe);
    }

    #[test]
    fn reads_an_audio_config() {
        let mut payload = vec![AUDIO_CODEC_AAC_LC];

        payload.extend_from_slice(&48_000u32.to_be_bytes());
        payload.push(2);
        payload.extend_from_slice(&2u32.to_be_bytes());
        payload.extend_from_slice(&[0x11, 0x90]);

        let config = parse_audio_config(&payload).expect("parses");

        assert_eq!(config.codec, AUDIO_CODEC_AAC_LC);
        assert_eq!(config.sample_rate, 48_000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.record, &[0x11, 0x90]);
    }

    #[test]
    fn reads_an_audio_frame() {
        let mut payload = 42u64.to_be_bytes().to_vec();

        payload.extend_from_slice(&[9, 9]);

        let frame = parse_audio_frame(&payload).expect("parses");

        assert_eq!(frame.pts_us, 42);
        assert_eq!(frame.data, &[9, 9]);
    }

    /// Everything above arrives over a cable from a device the plugin does not
    /// control, so a short or lying message must be rejected rather than read
    /// past the end of the buffer.
    #[test]
    fn short_messages_are_rejected() {
        for size in 0..9 {
            assert_eq!(parse_video_config(&vec![0; size]), None, "video config of {size}");
            assert_eq!(parse_video_frame(&vec![0; size]), None, "video frame of {size}");
        }

        for size in 0..10 {
            assert_eq!(parse_audio_config(&vec![0; size]), None, "audio config of {size}");
        }

        for size in 0..8 {
            assert_eq!(parse_audio_frame(&vec![0; size]), None, "audio frame of {size}");
        }
    }

    #[test]
    fn a_record_longer_than_the_message_is_rejected() {
        let mut video = vec![VIDEO_CODEC_H264, 0, 0, 0, 0];

        video.extend_from_slice(&64u32.to_be_bytes());
        video.extend_from_slice(&[0; 4]);

        assert_eq!(parse_video_config(&video), None);

        let mut audio = vec![AUDIO_CODEC_AAC_LC, 0, 0, 0, 0, 2];

        audio.extend_from_slice(&64u32.to_be_bytes());
        audio.extend_from_slice(&[0; 4]);

        assert_eq!(parse_audio_config(&audio), None);
    }

    /// A record size near the top of a u32 would overflow the end offset if it
    /// were added without care.
    #[test]
    fn a_record_size_that_would_overflow_is_rejected() {
        let mut video = vec![VIDEO_CODEC_H264, 0, 0, 0, 0];

        video.extend_from_slice(&u32::MAX.to_be_bytes());
        video.extend_from_slice(&[0; 4]);

        assert_eq!(parse_video_config(&video), None);
    }

    #[test]
    fn an_empty_record_is_read_as_one() {
        let payload = vec![VIDEO_CODEC_H264, 0, 0, 0, 0, 0, 0, 0, 0];

        assert_eq!(parse_video_config(&payload).expect("parses").record, &[] as &[u8]);
    }

    #[test]
    fn codec_names_cover_the_unknown_case() {
        assert_eq!(video_codec_name(VIDEO_CODEC_H264), "H.264");
        assert_eq!(video_codec_name(VIDEO_CODEC_HEVC), "HEVC");
        assert_eq!(video_codec_name(200), "unknown");
        assert_eq!(audio_codec_name(AUDIO_CODEC_AAC_LC), "AAC-LC");
        assert_eq!(audio_codec_name(200), "unknown");
    }
}
