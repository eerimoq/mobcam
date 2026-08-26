use crate::audio::{LATENCY_US, Spec};
use crate::dynlib::Library;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::c_char;
use std::ffi::c_int;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;

const LIBRARY: &str = "libpulse-simple.so.0";
const APPLICATION: &CStr = c"Mobcam";
const STREAM: &CStr = c"camera";
const STREAM_PLAYBACK: c_int = 1;
const SAMPLE_S16LE: c_int = 3;
const DEFAULT: u32 = u32::MAX;

unsafe extern "C" {
    fn getuid() -> u32;
}

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

type New = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
    *const c_char,
    *const c_char,
    *const SampleSpec,
    *const c_void,
    *const BufferAttr,
    *mut c_int,
) -> *mut c_void;
type Write = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int;
type Free = unsafe extern "C" fn(*mut c_void);
type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

struct Api {
    new: New,
    write: Write,
    free: Free,
    strerror: Option<StrError>,
}

impl Api {
    fn load() -> Option<Self> {
        let library = Library::open(LIBRARY)?;
        Some(unsafe {
            Self {
                new: library.symbol(c"pa_simple_new")?,
                write: library.symbol(c"pa_simple_write")?,
                free: library.symbol(c"pa_simple_free")?,
                strerror: library.symbol(c"pa_strerror"),
            }
        })
    }

    fn message(&self, error: c_int) -> String {
        let Some(strerror) = self.strerror else {
            return format!("error {error}");
        };
        let message = unsafe { strerror(error) };
        match message.is_null() {
            true => format!("error {error}"),
            false => unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned(),
        }
    }
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(Api::load).as_ref()
}

fn socket() -> PathBuf {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { getuid() })));
    runtime_dir.join("pulse").join("native")
}

pub fn available() -> bool {
    api().is_some() && (std::env::var_os("PULSE_SERVER").is_some() || socket().exists())
}

pub struct Stream {
    api: &'static Api,
    handle: *mut c_void,
}

impl Stream {
    pub fn open(sink: &str, spec: Spec) -> Result<Self, String> {
        let api = api().ok_or_else(|| format!("{LIBRARY} is not installed"))?;
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
            (api.new)(
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
            true => Err(api.message(error)),
            false => Ok(Self { api, handle }),
        }
    }

    pub fn write(&mut self, pcm: &[u8]) -> Result<(), String> {
        let mut error = 0;
        let result = unsafe { (self.api.write)(self.handle, pcm.as_ptr().cast(), pcm.len(), &raw mut error) };
        match result < 0 {
            true => Err(self.api.message(error)),
            false => Ok(()),
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { (self.api.free)(self.handle) }
    }
}
