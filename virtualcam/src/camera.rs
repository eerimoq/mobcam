use crate::audio::{self, Audio};
use crate::convert;
use crate::options::Options;
use crate::v4l2;
use clap::Parser;
use mobcam_core::decoder::{Decoder, Sink};
use mobcam_core::ffmpeg::{self, sys as av};
use mobcam_core::protocol::DeviceHello;
use mobcam_core::session::{self, Handler};
use mobcam_core::usbmux;
use mobcam_core::{Level, log};
use std::ffi::c_int;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const RECONNECT_DELAY: Duration = Duration::from_millis(1000);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DEVICE_LIST_TIMEOUT: Duration = Duration::from_secs(2);
const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;

static STOPPING: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn signal(signal: c_int, handler: usize) -> usize;
}

extern "C" fn stop(_signal: c_int) {
    STOPPING.store(true, Ordering::Relaxed);
}

fn stopping() -> bool {
    STOPPING.load(Ordering::Relaxed)
}

fn write_log(level: Level, message: &str) {
    let level = match level {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Info => "info",
    };
    eprintln!("{level}: {message}");
}

pub fn main() -> ExitCode {
    mobcam_core::set_logger(write_log);
    let options = Options::parse();
    if options.list {
        return list();
    }
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn list() -> ExitCode {
    let deadline = std::time::Instant::now() + DEVICE_LIST_TIMEOUT;
    let expired = move || std::time::Instant::now() >= deadline;
    println!("iPhones and iPads:");
    match usbmux::list_devices(&expired) {
        Ok(devices) if devices.is_empty() => println!("  none attached"),
        Ok(devices) => devices.iter().for_each(|device| println!("  {}", device.serial)),
        Err(error) => println!("  {}", error.message()),
    }
    println!("Virtual cameras:");
    let cameras = v4l2::loopback_devices();
    if cameras.is_empty() {
        println!("  none; load the module with `sudo modprobe v4l2loopback`");
    }
    for (path, name) in &cameras {
        println!("  {} ({name})", path.display());
    }
    println!("Virtual microphones:");
    let microphones = audio::devices();
    if microphones.is_empty() {
        println!(
            "  none; create a sink with `pactl load-module module-null-sink sink_name={}` \
             or load the module with `sudo modprobe snd-aloop`",
            audio::DEFAULT_SINK
        );
    }
    for microphone in &microphones {
        println!("  {microphone}");
    }
    ExitCode::SUCCESS
}

fn choose_device(chosen: Option<&Path>) -> Result<v4l2::Device, String> {
    if let Some(path) = chosen {
        return v4l2::Device::open(path);
    }
    let (path, _) = v4l2::loopback_devices().into_iter().next().ok_or(
        "no v4l2loopback device found; load the module with `sudo modprobe v4l2loopback` \
         (Debian and Ubuntu have it in v4l2loopback-dkms)",
    )?;
    v4l2::Device::open(&path)
}

fn run(options: Options) -> Result<(), String> {
    let mut device = choose_device(options.device.as_deref())?;
    let mut audio = options
        .audio
        .then(|| Audio::open(options.audio_backend, options.audio_device.as_deref()))
        .flatten();
    if options.audio && audio.is_none() {
        log!(
            Level::Info,
            "no virtual microphone; create a sink with \
             `pactl load-module module-null-sink sink_name={}` or load the module with \
             `sudo modprobe snd-aloop`, and the audio plays into it",
            audio::DEFAULT_SINK
        );
    }
    let mut decoder = Decoder::new().ok_or("failed to create the decoder")?;
    decoder.set_hardware(options.hardware_decode);
    decoder.set_audio(audio.is_some());
    unsafe {
        signal(SIGINT, stop as extern "C" fn(c_int) as usize);
        signal(SIGTERM, stop as extern "C" fn(c_int) as usize);
    }
    log!(Level::Info, "writing to {}", device.path().display());
    let abort = || stopping();
    let mut reported_failure = None;
    let mut buffer = Vec::new();
    while !stopping() {
        match usbmux::connect_to_device(options.udid(), options.port, &abort) {
            Ok((mut stream, serial)) => {
                reported_failure = None;
                if let Some(audio) = audio.as_mut() {
                    audio.reset();
                }
                let mut output = Output {
                    device: &mut device,
                    audio: audio.as_mut(),
                    buffer: &mut buffer,
                    serial: serial.clone(),
                    logged_pixel_format: None,
                    failure: None,
                };
                session::stream(&mut stream, &mut decoder, &mut output, &abort);
                if let Some(failure) = output.failure {
                    return Err(failure);
                }
                if !stopping() {
                    log!(Level::Info, "disconnected from {serial}");
                }
            }
            Err(error) => {
                if error != usbmux::Error::Aborted && reported_failure != Some(error) {
                    reported_failure = Some(error);
                    log!(Level::Info, "not connected: {}", error.message());
                }
            }
        }
        wait_before_reconnecting();
    }
    log!(Level::Info, "stopped");
    Ok(())
}

fn wait_before_reconnecting() {
    let deadline = std::time::Instant::now() + RECONNECT_DELAY;
    while !stopping() && std::time::Instant::now() < deadline {
        std::thread::sleep(STOP_POLL_INTERVAL);
    }
}

struct Output<'a> {
    device: &'a mut v4l2::Device,
    audio: Option<&'a mut Audio>,
    buffer: &'a mut Vec<u8>,
    serial: String,
    logged_pixel_format: Option<av::AVPixelFormat>,
    failure: Option<String>,
}

impl Sink for Output<'_> {
    fn video(&mut self, frame: &ffmpeg::Frame) {
        let Some((width, height)) = convert::to_i420(frame, self.buffer) else {
            if self.logged_pixel_format != Some(frame.pixel_format()) {
                self.logged_pixel_format = Some(frame.pixel_format());
                log!(
                    Level::Warning,
                    "unsupported pixel format {}",
                    ffmpeg::pixel_format_name(frame.pixel_format())
                );
            }
            return;
        };
        let picture = v4l2::Picture {
            width,
            height,
            colorspace: colorspace(frame),
            quantization: match frame.is_full_range() {
                true => v4l2::QUANTIZATION_FULL_RANGE,
                false => v4l2::QUANTIZATION_LIM_RANGE,
            },
        };
        if let Err(error) = self.device.write_frame(picture, self.buffer) {
            self.failure = Some(format!("failed to write a frame: {error}"));
            STOPPING.store(true, Ordering::Relaxed);
        }
    }

    fn audio(&mut self, frame: &ffmpeg::Frame) {
        if let Some(audio) = self.audio.as_deref_mut() {
            audio.play(frame);
        }
    }
}

impl Handler for Output<'_> {
    fn hello(&mut self, hello: &DeviceHello) {
        log!(
            Level::Info,
            "connected to {} (Moblin {}) on {}",
            hello.name,
            hello.app_version,
            self.serial
        );
    }
}

fn colorspace(frame: &ffmpeg::Frame) -> u32 {
    match frame.colorspace() {
        av::AVCOL_SPC_BT470BG | av::AVCOL_SPC_SMPTE170M => v4l2::COLORSPACE_SMPTE170M,
        av::AVCOL_SPC_BT2020_NCL => v4l2::COLORSPACE_BT2020,
        _ => v4l2::COLORSPACE_REC709,
    }
}
