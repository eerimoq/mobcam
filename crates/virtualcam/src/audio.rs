#![cfg_attr(any(not(alsa), not(pulse)), allow(dead_code))]

use crate::alsa;
use crate::options::AudioBackend;
use crate::pulse;
use mobcam_core::ffmpeg::{self, sys as av};
use mobcam_core::{Level, log};

pub const LATENCY_US: u32 = 100_000;
pub const DEFAULT_SINK: &str = "Mobcam";
const MAX_CHANNELS: i32 = 8;
const BYTES_PER_SAMPLE: usize = size_of::<i16>();

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Spec {
    pub rate: u32,
    pub channels: u8,
}

impl Spec {
    fn of(frame: &ffmpeg::Frame) -> Option<Self> {
        let rate = u32::try_from(frame.sample_rate()).ok().filter(|rate| *rate > 0)?;
        let channels = frame.channels();
        (1..=MAX_CHANNELS).contains(&channels).then_some(Self {
            rate,
            channels: channels as u8,
        })
    }

    pub fn frame_size(&self) -> usize {
        BYTES_PER_SAMPLE * usize::from(self.channels)
    }

    pub fn bytes_for(&self, microseconds: u32) -> u32 {
        let bytes = u64::from(self.rate) * u64::from(microseconds) / 1_000_000 * self.frame_size() as u64;
        u32::try_from(bytes).unwrap_or(u32::MAX)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Backend {
    Pulse,
    Alsa,
}

impl Backend {
    fn describe(self, device: &str) -> String {
        match self {
            Self::Pulse => format!("the PulseAudio sink {device}"),
            Self::Alsa => format!("the ALSA device {device}"),
        }
    }

    fn hint(self, device: &str) -> String {
        match self {
            Self::Pulse => format!("create it with `pactl load-module module-null-sink sink_name={device}`"),
            Self::Alsa => String::from("load the module with `sudo modprobe snd-aloop`"),
        }
    }
}

enum Playback {
    Pulse(pulse::Stream),
    Alsa(alsa::Device),
}

impl Playback {
    fn open(backend: Backend, device: &str, spec: Spec) -> Result<Self, String> {
        match backend {
            Backend::Pulse => pulse::Stream::open(device, spec).map(Self::Pulse),
            Backend::Alsa => alsa::Device::open(device, spec).map(Self::Alsa),
        }
    }

    fn write(&mut self, pcm: &[u8]) -> Result<(), String> {
        match self {
            Self::Pulse(stream) => stream.write(pcm),
            Self::Alsa(device) => device.write(pcm),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Sample {
    U8,
    S16,
    S32,
    Float,
    Double,
}

impl Sample {
    fn of(format: av::AVSampleFormat) -> Option<Self> {
        match format {
            av::AV_SAMPLE_FMT_U8 | av::AV_SAMPLE_FMT_U8P => Some(Self::U8),
            av::AV_SAMPLE_FMT_S16 | av::AV_SAMPLE_FMT_S16P => Some(Self::S16),
            av::AV_SAMPLE_FMT_S32 | av::AV_SAMPLE_FMT_S32P => Some(Self::S32),
            av::AV_SAMPLE_FMT_FLT | av::AV_SAMPLE_FMT_FLTP => Some(Self::Float),
            av::AV_SAMPLE_FMT_DBL | av::AV_SAMPLE_FMT_DBLP => Some(Self::Double),
            _ => None,
        }
    }

    fn width(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::S16 => 2,
            Self::S32 => 4,
            Self::Float => 4,
            Self::Double => 8,
        }
    }

    fn read(self, bytes: &[u8]) -> i16 {
        match self {
            Self::U8 => i16::from(bytes[0]).wrapping_sub(128) << 8,
            Self::S16 => i16::from_ne_bytes([bytes[0], bytes[1]]),
            Self::S32 => (i32::from_ne_bytes(bytes[..4].try_into().unwrap_or_default()) >> 16) as i16,
            Self::Float => scale(f64::from(f32::from_ne_bytes(bytes[..4].try_into().unwrap_or_default()))),
            Self::Double => scale(f64::from_ne_bytes(bytes[..8].try_into().unwrap_or_default())),
        }
    }
}

fn scale(sample: f64) -> i16 {
    (sample.clamp(-1.0, 1.0) * f64::from(i16::MAX)) as i16
}

fn planes(frame: &ffmpeg::Frame, spec: Spec, sample: Sample) -> Option<Vec<&[u8]>> {
    let samples = usize::try_from(frame.samples()).ok()?;
    let channels = usize::from(spec.channels);
    let planar = frame.audio_planes() > 1;
    let length = sample.width() * samples * if planar { 1 } else { channels };
    (0..frame.audio_planes())
        .map(|index| {
            let data = frame.audio_plane(index);
            (!data.is_null()).then(|| unsafe { std::slice::from_raw_parts(data, length) })
        })
        .collect()
}

fn interleave(out: &mut Vec<u8>, planes: &[&[u8]], samples: usize, channels: usize, sample: Sample) -> bool {
    let width = sample.width();
    out.clear();
    out.reserve(samples * channels * BYTES_PER_SAMPLE);
    for index in 0..samples {
        for channel in 0..channels {
            let (plane, offset) = match planes.len() {
                1 => (0, index * channels + channel),
                _ => (channel, index),
            };
            let Some(bytes) = planes.get(plane).and_then(|plane| plane.get(offset * width..)) else {
                return false;
            };
            if bytes.len() < width {
                return false;
            }
            out.extend(sample.read(bytes).to_le_bytes());
        }
    }
    true
}

pub struct Audio {
    backend: Backend,
    device: String,
    playback: Option<Playback>,
    spec: Option<Spec>,
    pcm: Vec<u8>,
    logged_sample_format: Option<av::AVSampleFormat>,
    failed: bool,
}

impl Audio {
    pub fn open(backend: AudioBackend, device: Option<&str>) -> Option<Self> {
        let (backend, device) = resolve(backend, device)?;
        log!(Level::Info, "playing audio into {}", backend.describe(&device));
        Some(Self {
            backend,
            device,
            playback: None,
            spec: None,
            pcm: Vec::new(),
            logged_sample_format: None,
            failed: false,
        })
    }

    pub fn reset(&mut self) {
        self.failed = false;
    }

    pub fn play(&mut self, frame: &ffmpeg::Frame) {
        if self.failed {
            return;
        }
        let Some(sample) = Sample::of(frame.sample_format()) else {
            if self.logged_sample_format != Some(frame.sample_format()) {
                self.logged_sample_format = Some(frame.sample_format());
                log!(
                    Level::Warning,
                    "unsupported sample format {}",
                    ffmpeg::sample_format_name(frame.sample_format())
                );
            }
            return;
        };
        let Some(spec) = Spec::of(frame) else {
            self.fail(format!(
                "unsupported audio of {} Hz and {} channels",
                frame.sample_rate(),
                frame.channels()
            ));
            return;
        };
        let Some(planes) = planes(frame, spec, sample) else {
            return;
        };
        let samples = frame.samples().max(0) as usize;
        if !interleave(&mut self.pcm, &planes, samples, usize::from(spec.channels), sample) {
            self.fail(String::from("failed to read the decoded audio"));
            return;
        }
        if self.spec != Some(spec) && !self.reopen(spec) {
            return;
        }
        if let Some(playback) = self.playback.as_mut()
            && let Err(error) = playback.write(&self.pcm)
        {
            self.fail(format!("failed to play audio into {}: {error}", self.device));
        }
    }

    fn reopen(&mut self, spec: Spec) -> bool {
        self.playback = None;
        self.spec = None;
        match Playback::open(self.backend, &self.device, spec) {
            Ok(playback) => {
                self.playback = Some(playback);
                self.spec = Some(spec);
                log!(
                    Level::Info,
                    "playing {} Hz {} channel audio into {}",
                    spec.rate,
                    spec.channels,
                    self.device
                );
                true
            }
            Err(error) => {
                self.fail(format!(
                    "failed to open {}: {error}; {}",
                    self.backend.describe(&self.device),
                    self.backend.hint(&self.device)
                ));
                false
            }
        }
    }

    fn fail(&mut self, message: String) {
        log!(Level::Warning, "{message}");
        self.playback = None;
        self.spec = None;
        self.failed = true;
    }
}

fn resolve(backend: AudioBackend, device: Option<&str>) -> Option<(Backend, String)> {
    let pulse = || pulse::available().then(|| (Backend::Pulse, device.unwrap_or(DEFAULT_SINK).to_string()));
    let alsa = || {
        if !alsa::available() {
            return None;
        }
        let name = match device {
            Some(name) => Some(name.to_string()),
            None => alsa::loopback_devices().into_iter().next().map(|(name, _)| name),
        };
        name.map(|name| (Backend::Alsa, name))
    };
    match backend {
        AudioBackend::Auto => pulse().or_else(alsa),
        #[cfg(pulse)]
        AudioBackend::Pulse => pulse(),
        #[cfg(alsa)]
        AudioBackend::Alsa => alsa(),
    }
}

pub fn hint() -> String {
    let mut hints = Vec::new();
    if cfg!(pulse) {
        hints.push(format!(
            "create a sink with `pactl load-module module-null-sink sink_name={DEFAULT_SINK}`"
        ));
    }
    if cfg!(alsa) {
        hints.push(String::from("load the module with `sudo modprobe snd-aloop`"));
    }
    match hints.is_empty() {
        true => String::from("install libpulse-dev or libasound2-dev and build mobcam-virtualcam again"),
        false => hints.join(" or "),
    }
}

pub fn devices() -> Vec<String> {
    let mut devices = Vec::new();
    if pulse::available() {
        devices.push(format!(
            "PulseAudio or PipeWire sinks, {DEFAULT_SINK} by default \
             (list them with `pactl list short sinks`)"
        ));
    }
    devices.extend(
        alsa::loopback_devices()
            .into_iter()
            .map(|(device, name)| format!("{device} ({name})")),
    );
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interleaved(planes: &[&[u8]], samples: usize, channels: usize, sample: Sample) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        interleave(&mut out, planes, samples, channels, sample).then_some(out)
    }

    #[test]
    fn planar_channels_are_woven_together() {
        let left = 1i16.to_ne_bytes();
        let right = 2i16.to_ne_bytes();
        let planes: [&[u8]; 2] = [&left, &right];
        assert_eq!(interleaved(&planes, 1, 2, Sample::S16), Some(vec![1, 0, 2, 0]));
    }

    #[test]
    fn packed_samples_are_copied_as_they_are() {
        let packed: Vec<u8> = [1i16, 2, 3, 4].iter().flat_map(|s| s.to_ne_bytes()).collect();
        let planes: [&[u8]; 1] = [&packed];
        assert_eq!(
            interleaved(&planes, 2, 2, Sample::S16),
            Some(vec![1, 0, 2, 0, 3, 0, 4, 0])
        );
    }

    #[test]
    fn every_sample_format_narrows_to_sixteen_bits() {
        assert_eq!(Sample::U8.read(&[255]), 127 << 8);
        assert_eq!(Sample::U8.read(&[128]), 0);
        assert_eq!(Sample::U8.read(&[0]), -128 << 8);
        assert_eq!(Sample::S32.read(&i32::MIN.to_ne_bytes()), i16::MIN);
        assert_eq!(Sample::Float.read(&1.0f32.to_ne_bytes()), i16::MAX);
        assert_eq!(Sample::Float.read(&(-1.0f32).to_ne_bytes()), -i16::MAX);
        assert_eq!(Sample::Double.read(&0.5f64.to_ne_bytes()), i16::MAX / 2);
    }

    #[test]
    fn samples_beyond_what_the_decoder_gave_are_refused() {
        let plane = 1i16.to_ne_bytes();
        let planes: [&[u8]; 1] = [&plane];
        assert_eq!(interleaved(&planes, 2, 1, Sample::S16), None);
        assert_eq!(interleaved(&planes, 1, 2, Sample::S16), None);
    }

    #[test]
    fn samples_out_of_range_are_clamped_rather_than_wrapped() {
        assert_eq!(Sample::Float.read(&2.0f32.to_ne_bytes()), i16::MAX);
        assert_eq!(Sample::Float.read(&(-2.0f32).to_ne_bytes()), -i16::MAX);
    }

    #[test]
    fn the_buffer_size_follows_the_sample_rate() {
        let spec = Spec {
            rate: 48000,
            channels: 2,
        };
        assert_eq!(spec.frame_size(), 4);
        assert_eq!(spec.bytes_for(LATENCY_US), 48000 / 10 * 4);
    }
}
