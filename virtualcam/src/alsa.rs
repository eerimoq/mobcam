#[cfg(alsa)]
use crate::audio::LATENCY_US;
use crate::audio::Spec;
#[cfg(alsa)]
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};

const CARDS: &str = "/proc/asound/cards";
const LOOPBACK_CARD: &str = "Loopback";
#[cfg(alsa)]
const STREAM_PLAYBACK: c_int = 0;
#[cfg(alsa)]
const FORMAT_S16_LE: c_int = 2;
#[cfg(alsa)]
const ACCESS_RW_INTERLEAVED: c_int = 3;
#[cfg(alsa)]
const SOFT_RESAMPLE: c_int = 1;
#[cfg(alsa)]
const BLOCKING: c_int = 0;
#[cfg(alsa)]
const WRITE_ATTEMPTS: usize = 3;
#[cfg(not(alsa))]
const UNSUPPORTED: &str = "mobcam-virtualcam was built without ALSA support";

#[cfg(alsa)]
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

#[cfg(alsa)]
fn message(error: c_int) -> String {
    let message = unsafe { snd_strerror(error) };
    match message.is_null() {
        true => format!("error {error}"),
        false => unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned(),
    }
}

pub fn available() -> bool {
    cfg!(alsa)
}

pub fn loopback_devices() -> Vec<(String, String)> {
    if !available() {
        return Vec::new();
    }
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

#[cfg(alsa)]
pub struct Device {
    handle: *mut c_void,
    frame_size: usize,
}

#[cfg(alsa)]
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

#[cfg(alsa)]
impl Drop for Device {
    fn drop(&mut self) {
        unsafe { snd_pcm_close(self.handle) };
    }
}

/// There is no device to open when libasound was not found when building.
#[cfg(not(alsa))]
pub enum Device {}

#[cfg(not(alsa))]
impl Device {
    pub fn open(_name: &str, _spec: Spec) -> Result<Self, String> {
        Err(String::from(UNSUPPORTED))
    }

    pub fn write(&mut self, _pcm: &[u8]) -> Result<(), String> {
        match *self {}
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
