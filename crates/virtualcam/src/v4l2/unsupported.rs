use crate::v4l2::Picture;
use std::path::Path;
use std::path::PathBuf;

const UNSUPPORTED: &str = "v4l2loopback devices are only available on Linux";

pub enum Device {}

impl Device {
    pub fn open(path: &Path) -> Result<Self, String> {
        Err(format!("{}: {UNSUPPORTED}", path.display()))
    }

    pub fn path(&self) -> &Path {
        match *self {}
    }

    pub fn write_frame(&mut self, _picture: Picture, _data: &[u8]) -> Result<(), String> {
        match *self {}
    }
}

pub fn loopback_devices() -> Vec<(PathBuf, String)> {
    Vec::new()
}
