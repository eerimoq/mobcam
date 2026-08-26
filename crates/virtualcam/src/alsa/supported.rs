use crate::audio::{LATENCY_US, Spec};
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};

const STREAM_PLAYBACK: c_int = 0;
const FORMAT_S16_LE: c_int = 2;
const ACCESS_RW_INTERLEAVED: c_int = 3;
const SOFT_RESAMPLE: c_int = 1;
const BLOCKING: c_int = 0;
const WRITE_ATTEMPTS: usize = 3;

unsafe extern "C" {
    fn snd_pcm_open(pcm: *mut *mut c_void, name: *const c_char, stream: c_int, mode: c_int) -> c_int;
    fn snd_pcm_set_params(
        pcm: *mut c_void,
        format: c_int,
        access: c_int,
        channels: c_uint,
        rate: c_uint,
        soft_resample: c_int,
        latency: c_uint,
    ) -> c_int;
    fn snd_pcm_writei(pcm: *mut c_void, buffer: *const c_void, frames: c_ulong) -> c_long;
    fn snd_pcm_recover(pcm: *mut c_void, error: c_int, silent: c_int) -> c_int;
    fn snd_pcm_close(pcm: *mut c_void) -> c_int;
    fn snd_strerror(error: c_int) -> *const c_char;
}

fn message(error: c_int) -> String {
    let message = unsafe { snd_strerror(error) };
    match message.is_null() {
        true => format!("error {error}"),
        false => unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned(),
    }
}

pub struct Device {
    handle: *mut c_void,
    frame_size: usize,
}

impl Device {
    pub fn open(name: &str, spec: Spec) -> Result<Self, String> {
        let name = CString::new(name).map_err(|_| String::from("the device name contains a nul byte"))?;
        let mut handle = std::ptr::null_mut();
        let result = unsafe { snd_pcm_open(&raw mut handle, name.as_ptr(), STREAM_PLAYBACK, BLOCKING) };
        if result < 0 {
            return Err(message(result));
        }
        let device = Self {
            handle,
            frame_size: spec.frame_size(),
        };
        let result = unsafe {
            snd_pcm_set_params(
                handle,
                FORMAT_S16_LE,
                ACCESS_RW_INTERLEAVED,
                c_uint::from(spec.channels),
                spec.rate,
                SOFT_RESAMPLE,
                LATENCY_US,
            )
        };
        match result < 0 {
            true => Err(message(result)),
            false => Ok(device),
        }
    }

    pub fn write(&mut self, pcm: &[u8]) -> Result<(), String> {
        let mut written = 0;
        for _ in 0..WRITE_ATTEMPTS {
            let rest = &pcm[written..];
            if rest.is_empty() {
                return Ok(());
            }
            let frames = (rest.len() / self.frame_size) as c_ulong;
            let result = unsafe { snd_pcm_writei(self.handle, rest.as_ptr().cast(), frames) };
            if result < 0 {
                let recovered = unsafe { snd_pcm_recover(self.handle, result as c_int, 1) };
                if recovered < 0 {
                    return Err(message(recovered));
                }
                continue;
            }
            written += result as usize * self.frame_size;
        }
        match written == pcm.len() {
            true => Ok(()),
            false => Err(format!("wrote {written} of {} bytes", pcm.len())),
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { snd_pcm_close(self.handle) };
    }
}
