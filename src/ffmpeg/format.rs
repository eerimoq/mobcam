use super::sys;

pub fn pixel_format_name(format: sys::AVPixelFormat) -> String {
    super::name(unsafe { sys::av_get_pix_fmt_name(format) }).unwrap_or_else(|| format.to_string())
}

pub fn sample_format_name(format: sys::AVSampleFormat) -> String {
    super::name(unsafe { sys::av_get_sample_fmt_name(format) }).unwrap_or_else(|| format.to_string())
}
