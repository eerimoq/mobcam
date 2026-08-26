use crate::audio::{LATENCY_US, Spec};
use std::ffi::{CStr, CString, c_char, c_int, c_void};

const APPLICATION: &CStr = c"Mobcam";
const STREAM: &CStr = c"camera";
const STREAM_PLAYBACK: c_int = 1;
const SAMPLE_S16LE: c_int = 3;
const DEFAULT: u32 = u32::MAX;

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

unsafe extern "C" {
    fn pa_simple_new(
        server: *const c_char,
        name: *const c_char,
        direction: c_int,
        device: *const c_char,
        stream: *const c_char,
        spec: *const SampleSpec,
        map: *const c_void,
        attributes: *const BufferAttr,
        error: *mut c_int,
    ) -> *mut c_void;
    fn pa_simple_write(simple: *mut c_void, data: *const c_void, bytes: usize, error: *mut c_int) -> c_int;
    fn pa_simple_free(simple: *mut c_void);
    fn pa_strerror(error: c_int) -> *const c_char;
}

fn message(error: c_int) -> String {
    let message = unsafe { pa_strerror(error) };
    match message.is_null() {
        true => format!("error {error}"),
        false => unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned(),
    }
}

pub struct Stream {
    handle: *mut c_void,
}

impl Stream {
    pub fn open(sink: &str, spec: Spec) -> Result<Self, String> {
        let sink = CString::new(sink).map_err(|_| String::from("the sink name contains a nul byte"))?;
        let sample_spec = SampleSpec {
            format: SAMPLE_S16LE,
            rate: spec.rate,
            channels: spec.channels,
        };
        let attributes = BufferAttr {
            maxlength: DEFAULT,
            tlength: spec.bytes_for(LATENCY_US),
            prebuf: DEFAULT,
            minreq: DEFAULT,
            fragsize: DEFAULT,
        };
        let mut error = 0;
        let handle = unsafe {
            pa_simple_new(
                std::ptr::null(),
                APPLICATION.as_ptr(),
                STREAM_PLAYBACK,
                sink.as_ptr(),
                STREAM.as_ptr(),
                &raw const sample_spec,
                std::ptr::null(),
                &raw const attributes,
                &raw mut error,
            )
        };
        match handle.is_null() {
            true => Err(message(error)),
            false => Ok(Self { handle }),
        }
    }

    pub fn write(&mut self, pcm: &[u8]) -> Result<(), String> {
        let mut error = 0;
        let result = unsafe { pa_simple_write(self.handle, pcm.as_ptr().cast(), pcm.len(), &raw mut error) };
        match result < 0 {
            true => Err(message(error)),
            false => Ok(()),
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { pa_simple_free(self.handle) }
    }
}
