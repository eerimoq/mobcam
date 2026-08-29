use crate::audio::{self, Audio};
use crate::convert;
use crate::options::{Options, PixelFormat};
use crate::v4l2;
use clap::Parser;
use mobcam_core::clock::Clock;
use mobcam_core::ffmpeg::{self, sys as av};
use mobcam_core::protocol::DeviceHello;
use mobcam_core::session::{Handler, Session, Sink};
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
    log_decoders(options.hardware_decode);
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn log_decoders(hardware: bool) {
    let codecs = [("H.264", av::AV_CODEC_ID_H264), ("HEVC", av::AV_CODEC_ID_HEVC)];
    for (name, id) in codecs {
        log!(Level::Info, "Available {name} decoders, highest priority first:");
        let decoders = ffmpeg::Codec::decoders_for(id, hardware);
        if decoders.is_empty() {
            log!(Level::Info, "  none available");
        }
        for decoder in decoders {
            let name = match decoder.long_name() {
                Some(long_name) => format!("{} ({long_name})", decoder.name()),
                None => decoder.name(),
            };
            log!(Level::Info, "  {name}{}", acceleration_name(decoder));
        }
    }
}

fn acceleration_name(codec: ffmpeg::Codec) -> &'static str {
    match codec.acceleration() {
        ffmpeg::Acceleration::Hardware => " [hardware]",
        ffmpeg::Acceleration::Accelerated => " [hardware accelerated]",
        ffmpeg::Acceleration::Software => " [software]",
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
        println!("  none; {}", audio::hint());
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
    device.set_debug(options.debug);
    let mut audio = options
        .audio
        .then(|| Audio::open(options.audio_backend, options.audio_device.as_deref()))
        .flatten();
    if options.audio && audio.is_none() {
        log!(
            Level::Info,
            "no virtual microphone; {}, and the audio plays into it",
            audio::hint()
        );
    }
    let nv12 = options.pixel_format == PixelFormat::Auto && device.takes_nv12();
    let mut session = Session::new().ok_or("failed to create the decoder")?;
    session.set_hardware(options.hardware_decode);
    session.set_audio(audio.is_some());
    unsafe {
        signal(SIGINT, stop as extern "C" fn(c_int) as usize);
        signal(SIGTERM, stop as extern "C" fn(c_int) as usize);
    }
    log!(
        Level::Info,
        "writing to {} in {}",
        device.path().display(),
        match nv12 {
            true => "NV12 or I420, whichever the decoder produces",
            false => "I420",
        }
    );
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
                    nv12,
                    clock: Clock::default(),
                    serial: serial.clone(),
                    logged_conversion: None,
                    logged_pixel_format: None,
                    failure: None,
                    debug: options.debug,
                    previous_write_frame_timestamp: 0,
                    previous_write_frame_clock: 0,
                };
                session.run(&mut stream, &mut output, &abort);
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
    nv12: bool,
    clock: Clock,
    serial: String,
    logged_conversion: Option<(av::AVPixelFormat, v4l2::Format)>,
    logged_pixel_format: Option<av::AVPixelFormat>,
    failure: Option<String>,
    debug: bool,
    previous_write_frame_timestamp: u64,
    previous_write_frame_clock: u64,
}

impl Sink for Output<'_> {
    fn video(&mut self, frame: &ffmpeg::Frame) {
        let decoded = frame.pixel_format();
        let Some(image) = convert::image(frame, self.buffer, self.nv12) else {
            if self.logged_pixel_format != Some(decoded) {
                self.logged_pixel_format = Some(decoded);
                log!(
                    Level::Warning,
                    "no conversion from {} to anything the camera takes; dropping the frames",
                    ffmpeg::pixel_format_name(decoded)
                );
            }
            return;
        };
        if self.logged_conversion != Some((decoded, image.format)) {
            self.logged_conversion = Some((decoded, image.format));
            match image.format {
                v4l2::Format::Nv12 => log!(
                    Level::Info,
                    "writing {} frames to the camera without converting them",
                    ffmpeg::pixel_format_name(decoded)
                ),
                v4l2::Format::I420 => log!(
                    Level::Info,
                    "converting {} frames to {} for the camera",
                    ffmpeg::pixel_format_name(decoded),
                    image.format.name()
                ),
            }
        }
        let picture = v4l2::Picture {
            width: image.width,
            height: image.height,
            format: image.format,
            colorspace: colorspace(frame),
            quantization: match frame.is_full_range() {
                true => v4l2::Quantization::FullRange,
                false => v4l2::Quantization::LimitedRange,
            },
        };
        let timestamp = self.clock.timestamp(frame.pts() as u64, v4l2::now_ns);
        if self.debug {
            let timestamp_delta = timestamp - self.previous_write_frame_timestamp;
            self.previous_write_frame_timestamp = timestamp;
            let clock = v4l2::now_ns();
            let clock_delta = clock - self.previous_write_frame_clock;
            self.previous_write_frame_clock = clock;
            log!(
                Level::Info,
                "writing frame with timestamp delta {} ms and clock delta {} ms",
                timestamp_delta / 1_000_000,
                clock_delta / 1_000_000
            );
        }
        if let Err(error) = self.device.write_frame(picture, self.buffer, timestamp) {
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

fn colorspace(frame: &ffmpeg::Frame) -> v4l2::Colorspace {
    match frame.colorspace() {
        av::AVCOL_SPC_BT470BG | av::AVCOL_SPC_SMPTE170M => v4l2::Colorspace::Smpte170m,
        av::AVCOL_SPC_BT2020_NCL => v4l2::Colorspace::Rec2020,
        _ => v4l2::Colorspace::Rec709,
    }
}
