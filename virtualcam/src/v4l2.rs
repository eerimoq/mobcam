//! Just enough Video4Linux2 to push frames into a v4l2loopback device.
//!
//! Only the video output side is used, in the read/write mode v4l2loopback
//! offers, so a frame is one `write` of a whole I420 image.

use std::ffi::c_int;
use std::ffi::c_ulong;
use std::ffi::c_void;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::path::PathBuf;

const LOOPBACK_DRIVER: &str = "v4l2 loopback";
const CAP_VIDEO_OUTPUT: u32 = 0x0000_0002;
const BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const FIELD_NONE: u32 = 1;
const PIX_FMT_YUV420: u32 = u32::from_le_bytes(*b"YU12");
// The values of the enums in linux/videodev2.h.
pub const COLORSPACE_SMPTE170M: u32 = 1;
pub const COLORSPACE_REC709: u32 = 3;
pub const COLORSPACE_BT2020: u32 = 10;
pub const QUANTIZATION_FULL_RANGE: u32 = 1;
pub const QUANTIZATION_LIM_RANGE: u32 = 2;

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, argument: *mut c_void) -> c_int;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    encoding: u32,
    quantization: u32,
    transfer_function: u32,
}

/// The kernel's `struct v4l2_format` payload: 200 bytes, aligned like the
/// largest member of its union, which holds pointers.
#[repr(C)]
#[derive(Clone, Copy)]
union FormatPayload {
    pix: PixFormat,
    alignment: [*mut c_void; 0],
    raw: [u8; 200],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Format {
    type_: u32,
    payload: FormatPayload,
}

const _: () = assert!(size_of::<Capability>() == 104);
const _: () = assert!(size_of::<PixFormat>() == 48);

const DIRECTION_WRITE: c_ulong = 1;
const DIRECTION_READ: c_ulong = 2;

/// The `_IOC` encoding from the kernel's asm-generic/ioctl.h.
const fn request(direction: c_ulong, number: c_ulong, size: usize) -> c_ulong {
    (direction << 30) | ((size as c_ulong) << 16) | ((b'V' as c_ulong) << 8) | number
}

const QUERYCAP: c_ulong = request(DIRECTION_READ, 0, size_of::<Capability>());
const S_FMT: c_ulong = request(DIRECTION_READ | DIRECTION_WRITE, 5, size_of::<Format>());

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(QUERYCAP == 0x8068_5600);
    assert!(S_FMT == 0xc0d0_5605);
};

/// The picture a frame is written as. Setting it again is only needed when it
/// changes, which normally never happens after the first frame.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub colorspace: u32,
    pub quantization: u32,
}

impl Picture {
    pub fn size(&self) -> usize {
        self.width as usize * self.height as usize * 3 / 2
    }
}

pub struct Device {
    file: File,
    path: PathBuf,
    picture: Option<Picture>,
}

impl Device {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let capability = query_capability(&file).ok_or_else(|| format!("{} is not a V4L2 device", path.display()))?;
        if capability.capabilities & CAP_VIDEO_OUTPUT == 0 {
            return Err(format!("{} cannot be written to as a camera", path.display()));
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
            picture: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes one I420 image, telling the driver about the picture first if it
    /// is not the one already set.
    pub fn write_frame(&mut self, picture: Picture, data: &[u8]) -> Result<(), String> {
        if self.picture != Some(picture) {
            self.set_picture(picture)?;
            self.picture = Some(picture);
        }
        match self.file.write(data) {
            Ok(written) if written == data.len() => Ok(()),
            Ok(written) => Err(format!("wrote {written} of {} bytes", data.len())),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set_picture(&mut self, picture: Picture) -> Result<(), String> {
        let mut format = Format {
            type_: BUF_TYPE_VIDEO_OUTPUT,
            payload: FormatPayload { raw: [0; 200] },
        };
        format.payload.pix = PixFormat {
            width: picture.width,
            height: picture.height,
            pixelformat: PIX_FMT_YUV420,
            field: FIELD_NONE,
            bytesperline: picture.width,
            sizeimage: picture.size() as u32,
            colorspace: picture.colorspace,
            private: 0,
            flags: 0,
            encoding: 0,
            quantization: picture.quantization,
            transfer_function: 0,
        };
        let result = unsafe { ioctl(self.file.as_raw_fd(), S_FMT, (&raw mut format).cast()) };
        if result < 0 {
            return Err(format!(
                "{} rejected {}x{}: {}",
                self.path.display(),
                picture.width,
                picture.height,
                std::io::Error::last_os_error()
            ));
        }
        let accepted = unsafe { format.payload.pix };
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
        if accepted.pixelformat != PIX_FMT_YUV420 {
            return Err(format!("{} does not accept YU12 images", self.path.display()));
        }
        Ok(())
    }
}

fn query_capability(file: &File) -> Option<Capability> {
    let mut capability: Capability = unsafe { std::mem::zeroed() };
    let result = unsafe { ioctl(file.as_raw_fd(), QUERYCAP, (&raw mut capability).cast()) };
    (result >= 0).then_some(capability)
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Every v4l2loopback device that can be written to, lowest number first.
pub fn loopback_devices() -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut numbered: Vec<(u32, PathBuf)> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let number = path.file_name()?.to_str()?.strip_prefix("video")?.parse().ok()?;
            Some((number, path))
        })
        .collect();
    numbered.sort();
    numbered
        .into_iter()
        .filter_map(|(_, path)| Some((path.clone(), loopback_name(&path)?)))
        .collect()
}

fn loopback_name(path: &Path) -> Option<String> {
    let file = OpenOptions::new().read(true).write(true).open(path).ok()?;
    let capability = query_capability(&file)?;
    if capability.capabilities & CAP_VIDEO_OUTPUT == 0 || text(&capability.driver) != LOOPBACK_DRIVER {
        return None;
    }
    Some(text(&capability.card))
}
