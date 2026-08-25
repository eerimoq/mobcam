//! Command line parsing.

use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 7790;

const USAGE: &str = "\
Use an iPhone or iPad running Moblin as a camera in any program, over USB.

The device is fed into a v4l2loopback device, which every program that can use a
camera can read. Load the module first, for example with

    sudo modprobe v4l2loopback card_label=Mobcam exclusive_caps=1

Usage: mobcam-virtualcam [options]

Options:
  -d, --device PATH     v4l2loopback device to write to, the first one found by
                        default
  -u, --udid UDID       iPhone or iPad to read from, the first one attached by
                        default
  -p, --port PORT       port Moblin streams to (default: 7790)
      --no-hardware-decode
                        decode in software even when the machine can do it in
                        hardware
  -l, --list            list the attached iPhones and iPads and the
                        v4l2loopback devices, and exit
  -h, --help            show this text and exit
  -V, --version         show the version and exit
";

pub struct Options {
    pub udid: String,
    pub port: u16,
    pub device: Option<PathBuf>,
    pub hardware_decode: bool,
}

pub enum Parsed {
    Run(Options),
    List,
    Text(String),
}

pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Parsed, String> {
    let mut options = Options {
        udid: String::new(),
        port: DEFAULT_PORT,
        device: None,
        hardware_decode: true,
    };
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{argument} needs a value; see --help"))
        };
        match argument.as_str() {
            "-d" | "--device" => options.device = Some(PathBuf::from(value()?)),
            "-u" | "--udid" => options.udid = value()?,
            "-p" | "--port" => {
                let port = value()?;
                options.port = port.parse().map_err(|_| format!("{port} is not a port number"))?;
            }
            "--no-hardware-decode" => options.hardware_decode = false,
            "-l" | "--list" => return Ok(Parsed::List),
            "-h" | "--help" => return Ok(Parsed::Text(USAGE.to_string())),
            "-V" | "--version" => return Ok(Parsed::Text(format!("{}\n", env!("CARGO_PKG_VERSION")))),
            _ => return Err(format!("unknown argument {argument}; see --help")),
        }
    }
    Ok(Parsed::Run(options))
}
