use clap::{ArgAction, Parser};
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 7790;

const ABOUT: &str = "\
Use an iPhone or iPad running Moblin as a camera in any program, over USB.

The device is fed into a v4l2loopback device, which every program that can use a
camera can read. Load the module first, for example with

    sudo modprobe v4l2loopback card_label=Mobcam exclusive_caps=1";

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

    /// decode in software even when the machine can do it in hardware
    #[arg(long = "no-hardware-decode", action = ArgAction::SetFalse)]
    pub hardware_decode: bool,

    /// list the attached iPhones and iPads and the v4l2loopback devices, and exit
    #[arg(short, long)]
    pub list: bool,
}

impl Options {
    pub fn udid(&self) -> &str {
        self.udid.as_deref().unwrap_or_default()
    }
}
