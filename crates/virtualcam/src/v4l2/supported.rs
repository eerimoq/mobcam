use crate::v4l2::{Colorspace, Picture, Quantization};
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use v4l::capability::Flags;
use v4l::context;
use v4l::format::{FieldOrder, Format, FourCC};
use v4l::video::Output;

const LOOPBACK_DRIVER: &str = "v4l2 loopback";
const PIXEL_FORMAT: FourCC = FourCC { repr: *b"YU12" };

impl From<Colorspace> for v4l::format::Colorspace {
    fn from(colorspace: Colorspace) -> Self {
        match colorspace {
            Colorspace::Smpte170m => Self::SMPTE170M,
            Colorspace::Rec709 => Self::Rec709,
            Colorspace::Rec2020 => Self::Rec2020,
        }
    }
}

impl From<Quantization> for v4l::format::Quantization {
    fn from(quantization: Quantization) -> Self {
        match quantization {
            Quantization::FullRange => Self::FullRange,
            Quantization::LimitedRange => Self::LimitedRange,
        }
    }
}

impl From<Picture> for Format {
    fn from(picture: Picture) -> Self {
        let size = picture.width as usize * picture.height as usize * 3 / 2;
        Self {
            field_order: FieldOrder::Progressive,
            stride: picture.width,
            size: size as u32,
            colorspace: picture.colorspace.into(),
            quantization: picture.quantization.into(),
            ..Self::new(picture.width, picture.height, PIXEL_FORMAT)
        }
    }
}

pub struct Device {
    device: v4l::Device,
    path: PathBuf,
    picture: Option<Picture>,
}

impl Device {
    pub fn open(path: &Path) -> Result<Self, String> {
        let device = v4l::Device::with_path(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let capability = device
            .query_caps()
            .map_err(|_| format!("{} is not a V4L2 device", path.display()))?;
        if !capability.capabilities.contains(Flags::VIDEO_OUTPUT) {
            return Err(format!("{} cannot be written to as a camera", path.display()));
        }
        Ok(Self {
            device,
            path: path.to_path_buf(),
            picture: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write_frame(&mut self, picture: Picture, data: &[u8]) -> Result<(), String> {
        if self.picture != Some(picture) {
            self.set_picture(picture)?;
            self.picture = Some(picture);
        }
        match self.device.write(data) {
            Ok(written) if written == data.len() => Ok(()),
            Ok(written) => Err(format!("wrote {written} of {} bytes", data.len())),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set_picture(&mut self, picture: Picture) -> Result<(), String> {
        let accepted = self.device.set_format(&picture.into()).map_err(|error| {
            format!(
                "{} rejected {}x{}: {error}",
                self.path.display(),
                picture.width,
                picture.height
            )
        })?;
        if accepted.width != picture.width || accepted.height != picture.height {
            return Err(format!(
                "{} wants {}x{} rather than {}x{}",
                self.path.display(),
                accepted.width,
                accepted.height,
                picture.width,
                picture.height
            ));
        }
        if accepted.fourcc != PIXEL_FORMAT {
            return Err(format!("{} does not accept YU12 images", self.path.display()));
        }
        Ok(())
    }
}

pub fn loopback_devices() -> Vec<(PathBuf, String)> {
    let mut numbered: Vec<(u32, PathBuf)> = context::enum_devices()
        .into_iter()
        .filter_map(|node| Some((number(node.path())?, node.path().to_path_buf())))
        .collect();
    numbered.sort();
    numbered
        .into_iter()
        .filter_map(|(_, path)| Some((path.clone(), loopback_name(&path)?)))
        .collect()
}

fn number(path: &Path) -> Option<u32> {
    path.file_name()?.to_str()?.strip_prefix("video")?.parse().ok()
}

fn loopback_name(path: &Path) -> Option<String> {
    let capability = v4l::Device::with_path(path).ok()?.query_caps().ok()?;
    if !capability.capabilities.contains(Flags::VIDEO_OUTPUT) || capability.driver != LOOPBACK_DRIVER {
        return None;
    }
    Some(capability.card)
}
