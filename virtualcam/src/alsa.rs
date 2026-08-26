use crate::audio::{LATENCY_US, Spec};
use crate::dynlib::Library;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::c_char;
use std::ffi::c_int;
use std::ffi::c_long;
use std::ffi::c_uint;
use std::ffi::c_ulong;
use std::ffi::c_void;
use std::sync::OnceLock;

const LIBRARY: &str = "libasound.so.2";
const CARDS: &str = "/proc/asound/cards";
const LOOPBACK_CARD: &str = "Loopback";
const STREAM_PLAYBACK: c_int = 0;
const FORMAT_S16_LE: c_int = 2;
const ACCESS_RW_INTERLEAVED: c_int = 3;
const SOFT_RESAMPLE: c_int = 1;
const BLOCKING: c_int = 0;
const WRITE_ATTEMPTS: usize = 3;

type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type SetParams = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type WriteInterleaved = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
type Close = unsafe extern "C" fn(*mut c_void) -> c_int;
type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

struct Api {
    open: Open,
    set_params: SetParams,
    write: WriteInterleaved,
    recover: Recover,
    close: Close,
    strerror: Option<StrError>,
}

impl Api {
    fn load() -> Option<Self> {
        let library = Library::open(LIBRARY)?;
        Some(unsafe {
            Self {
                open: library.symbol(c"snd_pcm_open")?,
                set_params: library.symbol(c"snd_pcm_set_params")?,
                write: library.symbol(c"snd_pcm_writei")?,
                recover: library.symbol(c"snd_pcm_recover")?,
                close: library.symbol(c"snd_pcm_close")?,
                strerror: library.symbol(c"snd_strerror"),
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

pub fn available() -> bool {
    api().is_some()
}

pub fn loopback_devices() -> Vec<(String, String)> {
    let Ok(cards) = std::fs::read_to_string(CARDS) else {
        return Vec::new();
    };
    loopback_cards(&cards)
        .into_iter()
        .map(|(id, name)| (format!("plughw:CARD={id},DEV=0"), name))
        .collect()
}

fn loopback_cards(cards: &str) -> Vec<(String, String)> {
    cards
        .lines()
        .filter_map(card)
        .filter(|(id, _)| id.starts_with(LOOPBACK_CARD))
        .collect()
}

fn card(line: &str) -> Option<(String, String)> {
    let (index, rest) = line.split_once('[')?;
    index.trim().parse::<u32>().ok()?;
    let (id, rest) = rest.split_once(']')?;
    let name = rest.split_once(':')?.1;
    Some((id.trim().to_string(), name.trim().to_string()))
}

pub struct Device {
    api: &'static Api,
    handle: *mut c_void,
    frame_size: usize,
}

impl Device {
    pub fn open(name: &str, spec: Spec) -> Result<Self, String> {
        let api = api().ok_or_else(|| format!("{LIBRARY} is not installed"))?;
        let name = CString::new(name).map_err(|_| String::from("the device name contains a nul byte"))?;
        let mut handle = std::ptr::null_mut();
        let result = unsafe { (api.open)(&raw mut handle, name.as_ptr(), STREAM_PLAYBACK, BLOCKING) };
        if result < 0 {
            return Err(api.message(result));
        }
        let device = Self {
            api,
            handle,
            frame_size: spec.frame_size(),
        };
        let result = unsafe {
            (api.set_params)(
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
            true => Err(api.message(result)),
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
            let result = unsafe { (self.api.write)(self.handle, rest.as_ptr().cast(), frames) };
            if result < 0 {
                let recovered = unsafe { (self.api.recover)(self.handle, result as c_int, 1) };
                if recovered < 0 {
                    return Err(self.api.message(recovered));
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
        unsafe { (self.api.close)(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARDS: &str = concat!(
        " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n",
        "                      HDA Intel PCH at 0xf7f10000 irq 33\n",
        " 1 [Loopback       ]: Loopback - Loopback\n",
        "                      Loopback 1\n",
    );

    #[test]
    fn only_the_loopback_cards_are_kept() {
        assert_eq!(
            loopback_cards(CARDS),
            vec![(String::from("Loopback"), String::from("Loopback - Loopback"))]
        );
    }

    #[test]
    fn the_lines_of_details_are_not_cards() {
        assert_eq!(card("                      Loopback 1"), None);
        assert_eq!(
            card(" 0 [PCH            ]: HDA-Intel - HDA Intel PCH").unwrap().0,
            "PCH"
        );
    }

    #[test]
    fn no_cards_at_all_is_no_devices() {
        assert!(loopback_cards("--- no soundcards ---\n").is_empty());
    }
}
