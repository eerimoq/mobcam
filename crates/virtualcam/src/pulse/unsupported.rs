use crate::audio::Spec;

const UNSUPPORTED: &str = "mobcam-virtualcam was built without PulseAudio support";

pub enum Stream {}

impl Stream {
    pub fn open(_sink: &str, _spec: Spec) -> Result<Self, String> {
        Err(String::from(UNSUPPORTED))
    }

    pub fn write(&mut self, _pcm: &[u8]) -> Result<(), String> {
        match *self {}
    }
}
