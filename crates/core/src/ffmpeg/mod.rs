mod codec;
mod format;
mod frame;
mod hardware;
mod packet;
pub mod sys;
pub mod version;
pub use codec::{Acceleration, Codec, Context};
pub use format::{pixel_format_name, sample_format_name};
pub use frame::Frame;
pub use hardware::Device;
pub use packet::Packet;
use std::ffi::CStr;
use std::os::raw::c_char;

pub const INPUT_BUFFER_PADDING: usize = sys::AV_INPUT_BUFFER_PADDING_SIZE as usize;

const AVERROR_EOF: i32 = -((b'E' as i32) | ((b'O' as i32) << 8) | ((b'F' as i32) << 16) | ((b' ' as i32) << 24));

const fn eagain() -> i32 {
    match cfg!(any(target_os = "macos", target_os = "ios")) {
        true => -35,
        false => -11,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Ok,
    Again,
    Eof,
    Error(i32),
}

impl Status {
    fn of(result: i32) -> Self {
        match result {
            result if result >= 0 => Status::Ok,
            result if result == eagain() => Status::Again,
            AVERROR_EOF => Status::Eof,
            result => Status::Error(result),
        }
    }

    pub fn is_ok(self) -> bool {
        matches!(self, Status::Ok | Status::Again)
    }
}

fn name(name: *const c_char) -> Option<String> {
    if name.is_null() {
        return None;
    }

    Some(unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned())
}
