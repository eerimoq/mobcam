//! The names of the pixel and sample formats, for the log lines that report one
//! the plugin cannot pass on to OBS.

use super::sys;

pub fn pixel_format_name(format: sys::AVPixelFormat) -> String {
    // SAFETY: returns a static string, or null for an unknown format.
    super::name(unsafe { sys::av_get_pix_fmt_name(format) }).unwrap_or_else(|| format.to_string())
}

pub fn sample_format_name(format: sys::AVSampleFormat) -> String {
    // SAFETY: as above.
    super::name(unsafe { sys::av_get_sample_fmt_name(format) }).unwrap_or_else(|| format.to_string())
}
