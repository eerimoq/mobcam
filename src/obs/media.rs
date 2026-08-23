//! Turning decoded FFmpeg frames into the video and audio OBS takes.
//!
//! No pixel or sample conversion happens here: a format OBS cannot take is
//! dropped and reported rather than converted, which is what keeps the latency
//! down and libswscale out of the plugin.

use crate::ffmpeg::{self, sys as av};

use super::sys;

/// A video frame pointing at the decoder's memory. It borrows for exactly as
/// long as the call that hands it to OBS.
pub type Frame = sys::obs_source_frame;

/// An audio buffer pointing at the decoder's memory, on the same terms.
pub type Audio = sys::obs_source_audio;

/// The OBS video format for a decoded pixel format, and whether that pixel
/// format implies full range regardless of what the frame says.
pub fn video_format(format: av::AVPixelFormat) -> Option<(sys::video_format, bool)> {
    let format = match format {
        av::AV_PIX_FMT_YUVJ420P => return Some((sys::VIDEO_FORMAT_I420, true)),
        av::AV_PIX_FMT_YUV420P => sys::VIDEO_FORMAT_I420,
        av::AV_PIX_FMT_NV12 => sys::VIDEO_FORMAT_NV12,
        av::AV_PIX_FMT_YUV420P10LE => sys::VIDEO_FORMAT_I010,
        av::AV_PIX_FMT_P010LE => sys::VIDEO_FORMAT_P010,
        av::AV_PIX_FMT_YUV444P => sys::VIDEO_FORMAT_I444,
        av::AV_PIX_FMT_YUV422P => sys::VIDEO_FORMAT_I422,
        _ => return None,
    };

    Some((format, false))
}

pub fn colorspace(frame: &ffmpeg::Frame) -> sys::video_colorspace {
    match frame.colorspace() {
        av::AVCOL_SPC_BT470BG | av::AVCOL_SPC_SMPTE170M => sys::VIDEO_CS_601,
        av::AVCOL_SPC_BT2020_NCL => {
            if frame.color_trc() == av::AVCOL_TRC_ARIB_STD_B67 {
                sys::VIDEO_CS_2100_HLG
            } else {
                sys::VIDEO_CS_2100_PQ
            }
        }
        _ => sys::VIDEO_CS_709,
    }
}

pub fn transfer(frame: &ffmpeg::Frame) -> u8 {
    let trc = match frame.color_trc() {
        av::AVCOL_TRC_SMPTE2084 => sys::VIDEO_TRC_PQ,
        av::AVCOL_TRC_ARIB_STD_B67 => sys::VIDEO_TRC_HLG,
        _ => sys::VIDEO_TRC_DEFAULT,
    };

    trc as u8
}

pub fn audio_format(format: av::AVSampleFormat) -> Option<sys::audio_format> {
    let format = match format {
        av::AV_SAMPLE_FMT_U8 => sys::AUDIO_FORMAT_U8BIT,
        av::AV_SAMPLE_FMT_S16 => sys::AUDIO_FORMAT_16BIT,
        av::AV_SAMPLE_FMT_S32 => sys::AUDIO_FORMAT_32BIT,
        av::AV_SAMPLE_FMT_FLT => sys::AUDIO_FORMAT_FLOAT,
        av::AV_SAMPLE_FMT_U8P => sys::AUDIO_FORMAT_U8BIT_PLANAR,
        av::AV_SAMPLE_FMT_S16P => sys::AUDIO_FORMAT_16BIT_PLANAR,
        av::AV_SAMPLE_FMT_S32P => sys::AUDIO_FORMAT_32BIT_PLANAR,
        av::AV_SAMPLE_FMT_FLTP => sys::AUDIO_FORMAT_FLOAT_PLANAR,
        _ => return None,
    };

    Some(format)
}

pub fn speakers(channels: i32) -> Option<sys::speaker_layout> {
    let speakers = match channels {
        1 => sys::SPEAKERS_MONO,
        2 => sys::SPEAKERS_STEREO,
        3 => sys::SPEAKERS_2POINT1,
        4 => sys::SPEAKERS_4POINT0,
        5 => sys::SPEAKERS_4POINT1,
        6 => sys::SPEAKERS_5POINT1,
        8 => sys::SPEAKERS_7POINT1,
        _ => return None,
    };

    Some(speakers)
}

/// Fills in the colour matrix and range OBS needs to render the frame.
pub fn set_color_parameters(frame: &mut Frame, colorspace: sys::video_colorspace, full_range: bool) {
    let range = if full_range {
        sys::VIDEO_RANGE_FULL
    } else {
        sys::VIDEO_RANGE_PARTIAL
    };

    // SAFETY: the three arrays are fields of the frame and have the fixed sizes
    // the function documents.
    unsafe {
        sys::video_format_get_parameters_for_format(
            colorspace,
            range,
            frame.format,
            frame.color_matrix.as_mut_ptr(),
            frame.color_range_min.as_mut_ptr(),
            frame.color_range_max.as_mut_ptr(),
        );
    }
}

/// How many channels a layout carries.
///
/// This and `audio_planes` below are `static inline` in the OBS headers, so
/// there is no symbol for bindgen to bind; they are kept in step with
/// media-io/audio-io.h by the tests at the bottom of this file.
pub fn audio_channels(speakers: sys::speaker_layout) -> usize {
    match speakers {
        sys::SPEAKERS_MONO => 1,
        sys::SPEAKERS_STEREO => 2,
        sys::SPEAKERS_2POINT1 => 3,
        sys::SPEAKERS_4POINT0 => 4,
        sys::SPEAKERS_4POINT1 => 5,
        sys::SPEAKERS_5POINT1 => 6,
        sys::SPEAKERS_7POINT1 => 8,
        _ => 0,
    }
}

fn is_planar(format: sys::audio_format) -> bool {
    matches!(
        format,
        sys::AUDIO_FORMAT_U8BIT_PLANAR
            | sys::AUDIO_FORMAT_16BIT_PLANAR
            | sys::AUDIO_FORMAT_32BIT_PLANAR
            | sys::AUDIO_FORMAT_FLOAT_PLANAR
    )
}

/// How many of the plane pointers OBS will read for this layout. Interleaved
/// audio lives in one plane however many channels it carries.
pub fn audio_planes(format: sys::audio_format, speakers: sys::speaker_layout) -> usize {
    if is_planar(format) {
        audio_channels(speakers)
    } else {
        1
    }
}
