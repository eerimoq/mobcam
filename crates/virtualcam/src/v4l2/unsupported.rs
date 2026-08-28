use crate::v4l2::Picture;
use std::path::Path;
use std::path::PathBuf;

const UNSUPPORTED: &str = "v4l2loopback devices are only available on Linux";

pub enum Device {}

impl Device {
    pub fn open(path: &Path) -> Result<Self, String> {
        Err(format!("{}: {UNSUPPORTED}", path.display()))
    }

    pub fn set_debug(&mut self, _debug: bool) {
        match *self {}
    }

    pub fn path(&self) -> &Path {
        match *self {}
    }

    pub fn takes_nv12(&self) -> bool {
        match *self {}
    }

    pub fn write_frame(&mut self, _picture: Picture, _data: &[u8], _timestamp_ns: u64) -> Result<(), String> {
        match *self {}
    }
}

pub fn now_ns() -> u64 {
    0
}

pub fn loopback_devices() -> Vec<(PathBuf, String)> {
    Vec::new()
}
