use crate::v4l2::{Colorspace, Format, Picture, Quantization};
use std::os::raw::{c_int, c_void};
use std::path::Path;
use std::path::PathBuf;
use std::ptr;
use v4l::buffer::Type;
use v4l::capability::Flags;
use v4l::context;
use v4l::format::{FieldOrder, FourCC};
use v4l::memory::Memory;
use v4l::v4l_sys::{timeval, v4l2_buffer, v4l2_requestbuffers};
use v4l::v4l2;
use v4l::video::Output;

const LOOPBACK_DRIVER: &str = "v4l2 loopback";
const PROBE_SIZE: (u32, u32) = (640, 480);
const BUFFERS: u32 = 4;
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

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

impl From<Picture> for v4l::format::Format {
    fn from(picture: Picture) -> Self {
        let size = picture.width as usize * picture.height as usize * 3 / 2;
        Self {
            field_order: FieldOrder::Progressive,
            stride: picture.width,
            size: size as u32,
            colorspace: picture.colorspace.into(),
            quantization: picture.quantization.into(),
            ..Self::new(picture.width, picture.height, picture.format.into())
        }
    }
}

pub fn now_ns() -> u64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
        return 0;
    }
    now.tv_sec as u64 * NANOSECONDS_PER_SECOND + now.tv_nsec as u64
}

fn descriptor(index: u32) -> v4l2_buffer {
    v4l2_buffer {
        index,
        type_: Type::VideoOutput as u32,
        memory: Memory::Mmap as u32,
        ..unsafe { std::mem::zeroed() }
    }
}

fn call(fd: c_int, request: v4l2::vidioc::_IOC_TYPE, argument: *mut c_void, what: &str) -> Result<(), String> {
    unsafe { v4l2::ioctl(fd, request, argument) }.map_err(|error| format!("{what} failed: {error}"))
}

struct Mapping {
    data: *mut u8,
    length: usize,
}

struct Buffers {
    fd: c_int,
    mappings: Vec<Mapping>,
    next: usize,
    queued: usize,
}

impl Buffers {
    fn open(fd: c_int) -> Result<Self, String> {
        let mut request = v4l2_requestbuffers {
            count: BUFFERS,
            type_: Type::VideoOutput as u32,
            memory: Memory::Mmap as u32,
            ..unsafe { std::mem::zeroed() }
        };
        call(
            fd,
            v4l2::vidioc::VIDIOC_REQBUFS,
            &mut request as *mut _ as *mut c_void,
            "requesting buffers",
        )?;
        let mut buffers = Self {
            fd,
            mappings: Vec::new(),
            next: 0,
            queued: 0,
        };
        for index in 0..request.count {
            buffers.mappings.push(buffers.map(index)?);
        }
        if buffers.mappings.is_empty() {
            return Err(String::from("no buffers to write frames into"));
        }
        let mut kind = Type::VideoOutput as c_int;
        call(
            fd,
            v4l2::vidioc::VIDIOC_STREAMON,
            &mut kind as *mut _ as *mut c_void,
            "starting the stream",
        )?;
        Ok(buffers)
    }

    fn map(&self, index: u32) -> Result<Mapping, String> {
        let mut buffer = descriptor(index);
        call(
            self.fd,
            v4l2::vidioc::VIDIOC_QUERYBUF,
            &mut buffer as *mut _ as *mut c_void,
            "querying a buffer",
        )?;
        let length = buffer.length as usize;
        let data = unsafe {
            v4l2::mmap(
                ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                self.fd,
                buffer.m.offset as libc::off_t,
            )
        }
        .map_err(|error| format!("mapping a buffer failed: {error}"))?;
        Ok(Mapping {
            data: data as *mut u8,
            length,
        })
    }

    fn write(&mut self, data: &[u8], timestamp_ns: u64) -> Result<(), String> {
        if self.queued == self.mappings.len() {
            self.dequeue()?;
            self.queued -= 1;
        }
        let index = self.next;
        let mapping = &self.mappings[index];
        if data.len() > mapping.length {
            return Err(format!("a frame of {} bytes does not fit a buffer", data.len()));
        }
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), mapping.data, data.len()) };
        self.queue(index, data.len(), timestamp_ns)?;
        self.next = (index + 1) % self.mappings.len();
        self.queued += 1;
        Ok(())
    }

    fn queue(&self, index: usize, bytes: usize, timestamp_ns: u64) -> Result<(), String> {
        let mut buffer = descriptor(index as u32);
        buffer.bytesused = bytes as u32;
        buffer.field = FieldOrder::Progressive as u32;
        buffer.timestamp = timeval {
            tv_sec: (timestamp_ns / NANOSECONDS_PER_SECOND) as libc::time_t,
            tv_usec: (timestamp_ns % NANOSECONDS_PER_SECOND / 1000) as libc::suseconds_t,
        };
        call(
            self.fd,
            v4l2::vidioc::VIDIOC_QBUF,
            &mut buffer as *mut _ as *mut c_void,
            "queueing a buffer",
        )
    }

    fn dequeue(&self) -> Result<(), String> {
        let mut buffer = descriptor(0);
        call(
            self.fd,
            v4l2::vidioc::VIDIOC_DQBUF,
            &mut buffer as *mut _ as *mut c_void,
            "dequeueing a buffer",
        )
    }
}

impl Drop for Buffers {
    fn drop(&mut self) {
        let mut kind = Type::VideoOutput as c_int;
        unsafe {
            let _ = v4l2::ioctl(
                self.fd,
                v4l2::vidioc::VIDIOC_STREAMOFF,
                &mut kind as *mut _ as *mut c_void,
            );
            for mapping in &self.mappings {
                let _ = v4l2::munmap(mapping.data as *mut c_void, mapping.length);
            }
            let mut request = v4l2_requestbuffers {
                count: 0,
                type_: Type::VideoOutput as u32,
                memory: Memory::Mmap as u32,
                ..std::mem::zeroed()
            };
            let _ = v4l2::ioctl(
                self.fd,
                v4l2::vidioc::VIDIOC_REQBUFS,
                &mut request as *mut _ as *mut c_void,
            );
        }
    }
}

pub struct Device {
    buffers: Option<Buffers>,
    device: v4l::Device,
    path: PathBuf,
    picture: Option<Picture>,
    nv12: bool,
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
            buffers: None,
            device,
            path: path.to_path_buf(),
            picture: None,
            nv12,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn takes_nv12(&self) -> bool {
        self.nv12
    }

    pub fn write_frame(&mut self, picture: Picture, data: &[u8], timestamp_ns: u64) -> Result<(), String> {
        if self.picture != Some(picture) {
            self.buffers = None;
            self.set_picture(picture)?;
            self.buffers = Some(Buffers::open(self.device.handle().fd())?);
            self.picture = Some(picture);
        }
        match self.buffers.as_mut() {
            Some(buffers) => buffers.write(data, timestamp_ns),
            None => Err(String::from("no buffers to write frames into")),
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
