use crate::v4l2::{Colorspace, Format, Picture, Quantization};
use std::mem;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use v4l::capability::Flags;
use v4l::context;
use v4l::device::Handle;
use v4l::format::{FieldOrder, FourCC};
use v4l::v4l_sys::*;
use v4l::v4l2;
use v4l::video::Output;

const LOOPBACK_DRIVER: &str = "v4l2 loopback";
const BUFFER_COUNT: u32 = 4;
const BUFFERS_OF: u32 = v4l2_buf_type_V4L2_BUF_TYPE_VIDEO_OUTPUT;
const MAPPED: u32 = v4l2_memory_V4L2_MEMORY_MMAP;
const PROBE_SIZE: (u32, u32) = (640, 480);

impl From<Format> for FourCC {
    fn from(format: Format) -> Self {
        Self {
            repr: match format {
                Format::I420 => *b"YU12",
                Format::Nv12 => *b"NV12",
            },
        }
    }
}

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

fn size_of(picture: Picture) -> usize {
    picture.width as usize * picture.height as usize * 3 / 2
}

impl From<Picture> for v4l::format::Format {
    fn from(picture: Picture) -> Self {
        Self {
            field_order: FieldOrder::Progressive,
            stride: picture.width,
            size: size_of(picture) as u32,
            colorspace: picture.colorspace.into(),
            quantization: picture.quantization.into(),
            ..Self::new(picture.width, picture.height, picture.format.into())
        }
    }
}

struct Mapping {
    data: *mut u8,
    length: usize,
}

struct Buffers {
    handle: Arc<Handle>,
    mappings: Vec<Mapping>,
    ours: Vec<u32>,
    streaming: bool,
}

impl Buffers {
    fn map(device: &v4l::Device, path: &Path) -> Result<Self, String> {
        let mut buffers = Self {
            handle: device.handle(),
            mappings: Vec::new(),
            ours: Vec::new(),
            streaming: false,
        };
        let count = buffers.request(BUFFER_COUNT)?;
        if count == 0 {
            return Err(format!("{} gave no buffers to write through", path.display()));
        }
        for index in 0..count {
            let mapping = buffers.query(index)?;
            buffers.mappings.push(mapping);
        }
        buffers.ours = (0..count).rev().collect();
        buffers.stream(v4l2::vidioc::VIDIOC_STREAMON, "starting the stream")?;
        buffers.streaming = true;
        Ok(buffers)
    }

    fn call<T>(&self, request: v4l2::vidioc::_IOC_TYPE, argument: &mut T, what: &str) -> Result<(), String> {
        let argument = (argument as *mut T).cast();
        unsafe { v4l2::ioctl(self.handle.fd(), request, argument) }.map_err(|error| format!("{what}: {error}"))
    }

    fn request(&self, count: u32) -> Result<u32, String> {
        let mut request = v4l2_requestbuffers {
            count,
            type_: BUFFERS_OF,
            memory: MAPPED,
            ..unsafe { mem::zeroed() }
        };
        self.call(v4l2::vidioc::VIDIOC_REQBUFS, &mut request, "asking for buffers")?;
        Ok(request.count)
    }

    fn query(&self, index: u32) -> Result<Mapping, String> {
        let mut buffer = v4l2_buffer {
            index,
            type_: BUFFERS_OF,
            memory: MAPPED,
            ..unsafe { mem::zeroed() }
        };
        self.call(v4l2::vidioc::VIDIOC_QUERYBUF, &mut buffer, "looking a buffer up")?;
        let length = buffer.length as usize;
        let offset = unsafe { buffer.m.offset }.into();
        let data = unsafe {
            v4l2::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                self.handle.fd(),
                offset,
            )
        }
        .map_err(|error| format!("mapping a buffer: {error}"))?;
        Ok(Mapping {
            data: data.cast(),
            length,
        })
    }

    fn stream(&self, request: v4l2::vidioc::_IOC_TYPE, what: &str) -> Result<(), String> {
        let mut kind = BUFFERS_OF;
        self.call(request, &mut kind, what)
    }

    fn take(&mut self) -> Result<u32, String> {
        if let Some(index) = self.ours.pop() {
            return Ok(index);
        }
        let mut buffer = v4l2_buffer {
            type_: BUFFERS_OF,
            memory: MAPPED,
            ..unsafe { mem::zeroed() }
        };
        self.call(v4l2::vidioc::VIDIOC_DQBUF, &mut buffer, "taking a buffer back")?;
        Ok(buffer.index)
    }

    fn room(&mut self, index: u32, size: usize) -> Option<&mut [u8]> {
        let mapping = self.mappings.get(index as usize)?;
        (mapping.length >= size).then(|| unsafe { std::slice::from_raw_parts_mut(mapping.data, size) })
    }

    fn give(&mut self, index: u32, size: usize) -> Result<(), String> {
        let mut buffer = v4l2_buffer {
            index,
            type_: BUFFERS_OF,
            memory: MAPPED,
            bytesused: size as u32,
            field: v4l2_field_V4L2_FIELD_NONE,
            ..unsafe { mem::zeroed() }
        };
        self.call(v4l2::vidioc::VIDIOC_QBUF, &mut buffer, "handing a buffer over")
    }

    fn keep(&mut self, index: u32) {
        self.ours.push(index);
    }
}

impl Drop for Buffers {
    fn drop(&mut self) {
        if self.streaming {
            let _ = self.stream(v4l2::vidioc::VIDIOC_STREAMOFF, "stopping the stream");
        }
        for mapping in self.mappings.drain(..) {
            let _ = unsafe { v4l2::munmap(mapping.data.cast(), mapping.length) };
        }
        let _ = self.request(0);
    }
}

pub struct Device {
    device: v4l::Device,
    path: PathBuf,
    picture: Option<Picture>,
    nv12: bool,
    buffers: Option<Buffers>,
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
        let nv12 = probe(&device, Format::Nv12);
        Ok(Self {
            device,
            path: path.to_path_buf(),
            picture: None,
            nv12,
            buffers: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn takes_nv12(&self) -> bool {
        self.nv12
    }

    pub fn write_frame(&mut self, picture: Picture, lay_out: impl FnOnce(&mut [u8]) -> bool) -> Result<(), String> {
        if self.picture != Some(picture) {
            self.buffers = None;
            self.picture = None;
            self.set_picture(picture)?;
            self.buffers = Some(Buffers::map(&self.device, &self.path)?);
            self.picture = Some(picture);
        }
        let size = size_of(picture);
        let Some(buffers) = self.buffers.as_mut() else {
            return Err(format!("{} has no buffers to write through", self.path.display()));
        };
        let index = buffers.take()?;
        let Some(room) = buffers.room(index, size) else {
            buffers.keep(index);
            return Err(format!(
                "{} gave a buffer smaller than {size} bytes",
                self.path.display()
            ));
        };
        if !lay_out(room) {
            buffers.keep(index);
            return Err(String::from("failed to lay the frame out"));
        }
        buffers.give(index, size)
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
        if accepted.fourcc != picture.format.into() {
            return Err(format!(
                "{} does not accept {} images",
                self.path.display(),
                picture.format.name()
            ));
        }
        Ok(())
    }
}

fn probe(device: &v4l::Device, format: Format) -> bool {
    let (width, height) = PROBE_SIZE;
    let picture = Picture {
        width,
        height,
        format,
        colorspace: Colorspace::Rec709,
        quantization: Quantization::LimitedRange,
    };
    let found = device.format().ok();
    let takes = device
        .set_format(&picture.into())
        .is_ok_and(|accepted| accepted.fourcc == format.into());
    if let Some(found) = found {
        let _ = device.set_format(&found);
    }
    takes
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
