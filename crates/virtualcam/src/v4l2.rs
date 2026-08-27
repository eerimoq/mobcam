#[cfg_attr(target_os = "linux", path = "v4l2/supported.rs")]
#[cfg_attr(not(target_os = "linux"), path = "v4l2/unsupported.rs")]
mod device;

pub use device::{Device, loopback_devices};

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Colorspace {
    Smpte170m,
    Rec709,
    Rec2020,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Quantization {
    FullRange,
    LimitedRange,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Format {
    I420,
    Nv12,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::I420 => "YU12",
            Self::Nv12 => "NV12",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub colorspace: Colorspace,
    pub quantization: Quantization,
}
