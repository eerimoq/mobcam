use clap::{ArgAction, Parser, ValueEnum};
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 7790;

const ABOUT: &str = "\
Use an iPhone or iPad running Moblin as a camera in any program, over USB.

The video is fed into a v4l2loopback device and the audio into a PulseAudio or
PipeWire sink, which every program that can use a camera and a microphone can
read. Load the module and create the sink first, for example with

    sudo modprobe v4l2loopback card_label=Mobcam exclusive_caps=1
    pactl load-module module-null-sink sink_name=Mobcam

Programs then record from the monitor of that sink. An ALSA loopback device,
from `sudo modprobe snd-aloop`, is used instead on machines without PulseAudio
or PipeWire.";

#[derive(Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum AudioBackend {
    /// the first of the ones below that the machine has
    Auto,
    /// a PulseAudio or PipeWire sink
    #[cfg(pulse)]
    Pulse,
    /// an ALSA loopback device
    #[cfg(alsa)]
    Alsa,
}

#[derive(Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum PixelFormat {
    /// the one the decoder produces, when the camera takes it
    Auto,
    /// I420 always, for programs that read nothing else
    I420,
}

#[derive(Parser)]
#[command(version, about = ABOUT)]
pub struct Options {
    /// v4l2loopback device to write to, the first one found by default
    #[arg(short, long, value_name = "PATH")]
    pub device: Option<PathBuf>,

    /// iPhone or iPad to read from, the first one attached by default
    #[arg(short, long, value_name = "UDID")]
    pub udid: Option<String>,

    /// port Moblin streams to
    #[arg(short, long, value_name = "PORT", default_value_t = DEFAULT_PORT)]
    pub port: u16,

    /// do not play the audio into a virtual microphone
    #[arg(long = "no-audio", action = ArgAction::SetFalse)]
    pub audio: bool,

    /// virtual microphone to play the audio into
    #[arg(long, value_name = "BACKEND", default_value = "auto")]
    pub audio_backend: AudioBackend,

    /// PulseAudio sink or ALSA device to play the audio into, Mobcam and the
    /// first loopback device by default
    #[arg(long, value_name = "NAME")]
    pub audio_device: Option<String>,

    /// decode in software even when the machine can do it in hardware
    #[arg(long = "no-hardware-decode", action = ArgAction::SetFalse)]
    pub hardware_decode: bool,

    /// pixel format to write to the camera
    #[arg(long, value_name = "FORMAT", default_value = "auto")]
    pub pixel_format: PixelFormat,

    /// log a line for every frame written to the camera
    #[arg(long)]
    pub debug: bool,

    /// list the attached iPhones and iPads, the v4l2loopback devices and the
    /// virtual microphones, and exit
    #[arg(short, long)]
    pub list: bool,
}

impl Options {
    pub fn udid(&self) -> &str {
        self.udid.as_deref().unwrap_or_default()
    }
}
