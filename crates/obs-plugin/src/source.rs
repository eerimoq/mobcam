use crate::devices::{Device, Devices};
use crate::obs::{self, Audio, Data, Frame, Properties, media, sys, text};
use mobcam_core::clock::Clock;
use mobcam_core::decoder::{Decoder, Sink};
use mobcam_core::ffmpeg::{self, sys as av};
use mobcam_core::protocol::DeviceHello;
use mobcam_core::session::{self, Handler};
use mobcam_core::usbmux::{self, Abort, Stream};
use mobcam_core::{Level, log, panic};
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const SETTING_DEVICE: &CStr = c"device";
const SETTING_PORT: &CStr = c"port";
const SETTING_HARDWARE_DECODE: &CStr = c"hardware_decode";
const SETTING_BUFFERING: &CStr = c"buffering";
const SETTING_CLEAR_ON_DISCONNECT: &CStr = c"clear_on_disconnect";
const SETTING_DISCONNECT_WHEN_HIDDEN: &CStr = c"disconnect_when_hidden";
const DEFAULT_PORT: i64 = 7790;
const RECONNECT_DELAY: Duration = Duration::from_millis(1000);
const DEVICE_LIST_TIMEOUT: Duration = Duration::from_secs(2);

struct Shared {
    source: obs::Source,
    stopping: AtomicBool,
    clear_on_disconnect: AtomicBool,
    width: AtomicU32,
    height: AtomicU32,
    wakeup: (Mutex<bool>, Condvar),
    devices: Mutex<Devices>,
}

impl Shared {
    fn remember(&self, serial: &str, name: &str) {
        if let Ok(mut devices) = self.devices.lock() {
            devices.remember(serial, name);
        }
    }

    fn clear_video(&self) {
        if !self.clear_on_disconnect.load(Ordering::Relaxed) {
            return;
        }
        self.source.clear_video();
        self.width.store(0, Ordering::Relaxed);
        self.height.store(0, Ordering::Relaxed);
    }

    fn load(&self, settings: &Data) {
        if let Ok(mut devices) = self.devices.lock() {
            devices.load(settings);
        }
    }

    fn save(&self, settings: &Data) {
        if let Ok(devices) = self.devices.lock() {
            devices.save(settings);
        }
    }
}

impl Abort for Shared {
    fn aborted(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }
}

struct Output {
    shared: Arc<Shared>,
    clock: Clock,
    serial: String,
    logged_pixel_format: Option<av::AVPixelFormat>,
    logged_sample_format: Option<av::AVSampleFormat>,
    logged_channels: Option<i32>,
}

impl Output {
    fn new(shared: Arc<Shared>, serial: &str) -> Self {
        Self {
            shared,
            clock: Clock::default(),
            serial: serial.to_string(),
            logged_pixel_format: None,
            logged_sample_format: None,
            logged_channels: None,
        }
    }
}

impl Sink for Output {
    fn video(&mut self, source: &ffmpeg::Frame) {
        let Some((format, format_is_full_range)) = media::video_format(source.pixel_format()) else {
            if self.logged_pixel_format != Some(source.pixel_format()) {
                self.logged_pixel_format = Some(source.pixel_format());
                log!(
                    Level::Warning,
                    "unsupported pixel format {}",
                    ffmpeg::pixel_format_name(source.pixel_format())
                );
            }
            return;
        };
        let full_range = format_is_full_range || source.is_full_range();
        let mut frame = Frame {
            format,
            width: source.width() as u32,
            height: source.height() as u32,
            full_range,
            trc: media::transfer(source),
            timestamp: self.clock.timestamp(source.pts() as u64, obs::now_ns),
            ..Default::default()
        };
        for plane in 0..(sys::MAX_AV_PLANES as usize).min(ffmpeg::Frame::PLANES) {
            let (data, linesize) = source.plane(plane);
            frame.data[plane] = data;
            frame.linesize[plane] = linesize as u32;
        }
        media::set_color_parameters(&mut frame, media::colorspace(source), full_range);
        self.shared.width.store(frame.width, Ordering::Relaxed);
        self.shared.height.store(frame.height, Ordering::Relaxed);
        self.shared.source.output_video(&frame);
    }

    fn audio(&mut self, source: &ffmpeg::Frame) {
        let Some(format) = media::audio_format(source.sample_format()) else {
            if self.logged_sample_format != Some(source.sample_format()) {
                self.logged_sample_format = Some(source.sample_format());
                log!(
                    Level::Warning,
                    "unsupported sample format {}",
                    ffmpeg::sample_format_name(source.sample_format())
                );
            }
            return;
        };
        let channels = source.channels();
        let Some(speakers) = media::speakers(channels) else {
            if self.logged_channels != Some(channels) {
                self.logged_channels = Some(channels);
                log!(Level::Warning, "unsupported channel count {channels}");
            }
            return;
        };
        let mut audio = Audio {
            format,
            speakers,
            frames: source.samples() as u32,
            samples_per_sec: source.sample_rate() as u32,
            timestamp: self.clock.timestamp(source.pts() as u64, obs::now_ns),
            ..Default::default()
        };
        let planes = media::audio_planes(format, speakers).min(sys::MAX_AV_PLANES as usize);
        for plane in 0..planes {
            audio.data[plane] = source.audio_plane(plane);
        }
        self.shared.source.output_audio(&audio);
    }
}

impl Handler for Output {
    fn hello(&mut self, hello: &DeviceHello) {
        log!(
            Level::Info,
            "connected to {} (Moblin {}) on {}",
            hello.name,
            hello.app_version,
            self.serial
        );
        self.shared.remember(&self.serial, &hello.name);
    }
}

struct Worker {
    shared: Arc<Shared>,
    decoder: Option<Decoder>,
    serial: String,
    port: u16,
    reported_failure: Option<usbmux::Error>,
}

impl Worker {
    fn run(mut self) {
        while !self.shared.stopping.load(Ordering::Relaxed) {
            self.connect();
            if self.shared.stopping.load(Ordering::Relaxed) {
                break;
            }
            self.wait_before_reconnecting();
        }
    }

    fn wait_before_reconnecting(&self) {
        let (lock, condvar) = &self.shared.wakeup;
        let Ok(signalled) = lock.lock() else {
            return;
        };
        let _ = condvar.wait_timeout_while(signalled, RECONNECT_DELAY, |signalled| !*signalled);
    }

    fn connect(&mut self) {
        let (stream, serial) = match usbmux::connect_to_device(&self.serial, self.port, self.shared.as_ref()) {
            Ok(connected) => connected,
            Err(error) => {
                if error != usbmux::Error::Aborted && self.reported_failure != Some(error) {
                    self.reported_failure = Some(error);
                    log!(Level::Info, "not connected: {}", error.message());
                }
                return;
            }
        };
        self.reported_failure = None;
        self.stream(stream, &serial);
        if !self.shared.stopping.load(Ordering::Relaxed) {
            log!(Level::Info, "disconnected from {serial}");
        }
        self.shared.clear_video();
    }

    fn stream(&mut self, mut stream: Stream, serial: &str) {
        let Some(decoder) = self.decoder.as_mut() else {
            return;
        };
        let mut output = Output::new(Arc::clone(&self.shared), serial);
        session::stream(&mut stream, decoder, &mut output, self.shared.as_ref());
    }
}

struct Source {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    serial: String,
    port: u16,
    hardware_decode: bool,
    disconnect_when_hidden: bool,
}

impl Source {
    fn new(source: obs::Source) -> Self {
        Self {
            shared: Arc::new(Shared {
                source,
                stopping: AtomicBool::new(false),
                clear_on_disconnect: AtomicBool::new(true),
                width: AtomicU32::new(0),
                height: AtomicU32::new(0),
                wakeup: (Mutex::new(false), Condvar::new()),
                devices: Mutex::new(Devices::default()),
            }),
            thread: None,
            serial: String::new(),
            port: DEFAULT_PORT as u16,
            hardware_decode: false,
            disconnect_when_hidden: false,
        }
    }

    fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let Some(mut decoder) = Decoder::new() else {
            log!(Level::Error, "failed to create the decoder");
            return;
        };
        decoder.set_hardware(self.hardware_decode);
        self.shared.stopping.store(false, Ordering::Relaxed);
        if let Ok(mut signalled) = self.shared.wakeup.0.lock() {
            *signalled = false;
        }
        let worker = Worker {
            shared: Arc::clone(&self.shared),
            decoder: Some(decoder),
            serial: self.serial.clone(),
            port: self.port,
            reported_failure: None,
        };
        match std::thread::Builder::new()
            .name(String::from("mobcam"))
            .spawn(|| panic::guard("the receive thread", (), || worker.run()))
        {
            Ok(thread) => self.thread = Some(thread),
            Err(_) => log!(Level::Error, "failed to start the receive thread"),
        }
    }

    fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.shared.stopping.store(true, Ordering::Relaxed);
        if let Ok(mut signalled) = self.shared.wakeup.0.lock() {
            *signalled = true;
            self.shared.wakeup.1.notify_all();
        }
        let _ = thread.join();
        self.shared.clear_video();
    }

    fn update(&mut self, settings: &Data) {
        let serial = settings.string(SETTING_DEVICE);
        let port = settings.int(SETTING_PORT) as u16;
        let hardware_decode = settings.bool(SETTING_HARDWARE_DECODE);
        let buffering = settings.bool(SETTING_BUFFERING);
        let disconnect_when_hidden = settings.bool(SETTING_DISCONNECT_WHEN_HIDDEN);
        self.shared
            .clear_on_disconnect
            .store(settings.bool(SETTING_CLEAR_ON_DISCONNECT), Ordering::Relaxed);
        self.shared.source.set_async_unbuffered(!buffering);
        let restart = port != self.port || hardware_decode != self.hardware_decode || serial != self.serial;
        if restart {
            self.stop();
            self.serial = serial;
            self.port = port;
            self.hardware_decode = hardware_decode;
        }
        self.disconnect_when_hidden = disconnect_when_hidden;
        let showing = self.shared.source.showing();
        if !disconnect_when_hidden || showing {
            self.start();
        } else {
            self.stop();
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.stop();
    }
}

fn fill_device_list(list: &mut obs::Property, shared: Option<&Shared>) {
    let deadline = std::time::Instant::now() + DEVICE_LIST_TIMEOUT;
    let expired = move || std::time::Instant::now() >= deadline;
    list.clear_list();
    list.add_translated_list_entry(text(c"Device.Automatic"), "");
    let attached = usbmux::list_devices(&expired).unwrap_or_default();
    let mut without_source = Devices::default();
    let mut locked = shared.and_then(|shared| shared.devices.lock().ok());
    let known = locked.as_deref_mut().unwrap_or(&mut without_source);
    for device in &attached {
        known.remember(&device.serial, "");
    }
    let connected = |device: &Device| attached.iter().any(|found| found.serial == device.serial);
    for device in known.all().iter().filter(|device| connected(device)) {
        list.add_list_entry(&device.label(), &device.serial);
    }
    let disconnected = text(c"Device.Disconnected").to_string_lossy();
    for device in known.all().iter().filter(|device| !connected(device)) {
        let label = format!("{} - {disconnected}", device.label());
        let index = list.add_list_entry(&label, &device.serial);
        list.disable_list_entry(index, true);
    }
}

fn source_of<'a>(data: *mut c_void) -> &'a mut Source {
    unsafe { &mut *(data as *mut Source) }
}

fn shared_of<'a>(data: *mut c_void) -> Option<&'a Shared> {
    if data.is_null() {
        return None;
    }
    Some(&unsafe { &*(data as *const Source) }.shared)
}

extern "C" fn get_name(_type_data: *mut c_void) -> *const c_char {
    panic::guard("get_name", c"Mobcam".as_ptr(), || text(c"Mobcam").as_ptr())
}

extern "C" fn create(settings: *mut sys::obs_data_t, source: *mut sys::obs_source_t) -> *mut c_void {
    panic::guard("create", std::ptr::null_mut(), || {
        let mut context = Box::new(Source::new(obs::Source::from_raw(source)));
        let settings = Data::from_raw(settings);
        context.shared.load(&settings);
        context.update(&settings);
        Box::into_raw(context) as *mut c_void
    })
}

extern "C" fn destroy(data: *mut c_void) {
    panic::guard("destroy", (), || {
        drop(unsafe { Box::from_raw(data as *mut Source) });
    })
}

extern "C" fn update(data: *mut c_void, settings: *mut sys::obs_data_t) {
    panic::guard("update", (), || {
        source_of(data).update(&Data::from_raw(settings));
    })
}

extern "C" fn save(data: *mut c_void, settings: *mut sys::obs_data_t) {
    panic::guard("save", (), || {
        source_of(data).shared.save(&Data::from_raw(settings));
    })
}

extern "C" fn show(data: *mut c_void) {
    panic::guard("show", (), || {
        let context = source_of(data);
        if context.disconnect_when_hidden {
            context.start();
        }
    })
}

extern "C" fn hide(data: *mut c_void) {
    panic::guard("hide", (), || {
        let context = source_of(data);
        if context.disconnect_when_hidden {
            context.stop();
        }
    })
}

extern "C" fn get_width(data: *mut c_void) -> u32 {
    panic::guard("get_width", 0, || source_of(data).shared.width.load(Ordering::Relaxed))
}

extern "C" fn get_height(data: *mut c_void) -> u32 {
    panic::guard("get_height", 0, || {
        source_of(data).shared.height.load(Ordering::Relaxed)
    })
}

extern "C" fn get_defaults(settings: *mut sys::obs_data_t) {
    panic::guard("get_defaults", (), || {
        let settings = Data::from_raw(settings);
        settings.set_default_string(SETTING_DEVICE, c"");
        settings.set_default_int(SETTING_PORT, DEFAULT_PORT);
        settings.set_default_bool(SETTING_HARDWARE_DECODE, true);
        settings.set_default_bool(SETTING_BUFFERING, false);
        settings.set_default_bool(SETTING_CLEAR_ON_DISCONNECT, true);
        settings.set_default_bool(SETTING_DISCONNECT_WHEN_HIDDEN, false);
    })
}

extern "C" fn refresh_devices_clicked(
    properties: *mut sys::obs_properties_t,
    _property: *mut sys::obs_property_t,
    data: *mut c_void,
) -> bool {
    panic::guard("refresh_devices", false, || {
        let mut list = unsafe { obs::properties::get(properties, SETTING_DEVICE) };
        fill_device_list(&mut list, shared_of(data));
        true
    })
}

extern "C" fn get_properties(data: *mut c_void) -> *mut sys::obs_properties_t {
    panic::guard("get_properties", std::ptr::null_mut(), || {
        let mut properties = Properties::new();
        let mut list = properties.add_string_list(SETTING_DEVICE, text(c"Device"));
        fill_device_list(&mut list, shared_of(data));
        unsafe { properties.add_button(c"refresh", text(c"RefreshDevices"), Some(refresh_devices_clicked), data) };
        properties.add_bool(SETTING_HARDWARE_DECODE, text(c"HardwareDecode"));
        properties.add_bool(SETTING_BUFFERING, text(c"Buffering"));
        properties.add_bool(SETTING_CLEAR_ON_DISCONNECT, text(c"ClearOnDisconnect"));
        properties.add_bool(SETTING_DISCONNECT_WHEN_HIDDEN, text(c"DisconnectWhenHidden"));
        let mut port = properties.add_int(SETTING_PORT, text(c"Port"), 1, 65535);
        port.set_long_description(text(c"Port.Description"));
        properties.into_raw()
    })
}

pub fn info() -> sys::obs_source_info {
    sys::obs_source_info {
        id: c"mobcam_source".as_ptr(),
        type_: sys::OBS_SOURCE_TYPE_INPUT,
        output_flags: sys::OBS_SOURCE_ASYNC_VIDEO | sys::OBS_SOURCE_AUDIO | sys::OBS_SOURCE_DO_NOT_DUPLICATE,
        icon_type: sys::OBS_ICON_TYPE_CAMERA,
        get_name: Some(get_name),
        create: Some(create),
        destroy: Some(destroy),
        update: Some(update),
        save: Some(save),
        show: Some(show),
        hide: Some(hide),
        get_width: Some(get_width),
        get_height: Some(get_height),
        get_defaults: Some(get_defaults),
        get_properties: Some(get_properties),
        ..Default::default()
    }
}
