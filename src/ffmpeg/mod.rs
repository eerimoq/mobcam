//! The parts of FFmpeg the plugin uses, wrapped just enough to be safe.
//!
//! Every call into libavcodec and libavutil is made from inside this module, so
//! that the decoder above it can be written without an unsafe block of its own.
//! What the wrappers hand out are still raw pointers into decoder memory, since
//! that is what a frame is; the borrows around them are what is made safe.

pub mod sys;

mod codec;
mod format;
mod frame;
mod hardware;
mod packet;

use std::ffi::CStr;
use std::os::raw::c_char;

pub use codec::{Codec, Context};
pub use format::{pixel_format_name, sample_format_name};
pub use frame::Frame;
pub use hardware::Device;
pub use packet::Packet;

/// The zeroed bytes libavcodec expects to find past an access unit it is handed,
/// because it reads a little beyond the end of one.
pub const INPUT_BUFFER_PADDING: usize = sys::AV_INPUT_BUFFER_PADDING_SIZE as usize;

/// libavcodec's end-of-stream error. FFmpeg builds it with the FFERRTAG macro,
/// which bindgen cannot emit, so the same four character tag is spelled out.
const AVERROR_EOF: i32 = -((b'E' as i32) | ((b'O' as i32) << 8) | ((b'F' as i32) << 16) | ((b' ' as i32) << 24));

/// EAGAIN as libavcodec reports it. FFmpeg's AVERROR macro is arithmetic on the
/// platform errno, which bindgen cannot emit either.
const fn eagain() -> i32 {
    // EAGAIN is 11 on Linux and 35 on the BSDs, macOS included; Windows builds
    // of FFmpeg use the same value as the MSVC runtime.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    return -35;

    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    return -11;
}

/// What a send or a receive came back with. The two codes that are not failures
/// are named, since both are ordinary steps in the exchange with a decoder:
/// EAGAIN means the other half of it has to happen first, and EOF that the
/// decoder has been drained.
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

    /// True when the call did what was asked of it, EAGAIN included: a decoder
    /// that wants to be read before it takes more input has not failed.
    pub fn is_ok(self) -> bool {
        matches!(self, Status::Ok | Status::Again)
    }
}

/// A static string libavutil returned, or nothing when it did not recognise
/// what it was asked about.
fn name(name: *const c_char) -> Option<String> {
    if name.is_null() {
        return None;
    }

    // SAFETY: a static NUL terminated string owned by libavutil.
    Some(unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned())
}
