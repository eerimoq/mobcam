use crate::audio::Spec;

const UNSUPPORTED: &str = "mobcam-virtualcam was built without ALSA support";

pub enum Device {}

impl Device {
    pub fn open(_name: &str, _spec: Spec) -> Result<Self, String> {
        Err(String::from(UNSUPPORTED))
    }

    pub fn write(&mut self, _pcm: &[u8]) -> Result<(), String> {
        match *self {}
    }
}
