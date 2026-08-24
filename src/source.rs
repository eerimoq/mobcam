use crate::decoder::{Decoder, Sink, INPUT_PADDING};
use crate::obs::{self, sys, text, Audio, Data, Frame, Level, Properties};
use crate::obs_log;
use crate::protocol;
use crate::socket::{self, Abort, Stream};
use crate::usbmux;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

const SETTING_DEVICE: &CStr = c"device";
const SETTING_PORT: &CStr = c"port";
const SETTING_HARDWARE_DECODE: &CStr = c"hardware_decode";
const SETTING_BUFFERING: &CStr = c"buffering";
const SETTING_CLEAR_ON_DISCONNECT: &CStr = c"clear_on_disconnect";
const SETTING_DISCONNECT_WHEN_HIDDEN: &CStr = c"disconnect_when_hidden";
const DEFAULT_PORT: i64 = 7790;
const RECONNECT_DELAY: Duration = Duration::from_millis(1000);
const PTS_DISCONTINUITY_US: u64 = 5 * 1000 * 1000;
const DEVICE_LIST_TIMEOUT: Duration = Duration::from_secs(2);

fn name_cache() -> &'static Mutex<Vec<(String, String)>> {
    static CACHE: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn name_cache_set(serial: &str, name: &str) {
    if serial.is_empty() || name.is_empty() {
        return;
    }
    let Ok(mut cache) = name_cache().lock() else {
        return;
    };
    match cache.iter_mut().find(|(known, _)| known == serial) {
        Some((_, known)) => *known = name.to_string(),
        None => cache.push((serial.to_string(), name.to_string())),
    }
}

fn name_cache_get(serial: &str) -> Option<String> {
    let cache = name_cache().lock().ok()?;
    cache
        .iter()
        .find(|(known, _)| known == serial)
        .map(|(_, name)| name.clone())
}

#[derive(Default)]
struct Clock {
    anchored: bool,
    first_pts_us: u64,
    previous_pts_us: u64,
    anchor_ns: u64,
}

impl Clock {
    fn timestamp(&mut self, pts_us: u64) -> u64 {
        let distance = pts_us.abs_diff(self.previous_pts_us);
        if !self.anchored || distance > PTS_DISCONTINUITY_US {
            self.anchored = true;
            self.first_pts_us = pts_us;
            self.anchor_ns = obs::now_ns();
        } else if pts_us < self.first_pts_us {
            self.anchor_ns -= (self.first_pts_us - pts_us) * 1000;
            self.first_pts_us = pts_us;
        }
        self.previous_pts_us = pts_us;
        self.anchor_ns + (pts_us - self.first_pts_us) * 1000
    }
}

struct Shared {
    source: obs::Source,
    stopping: AtomicBool,
    clear_on_disconnect: AtomicBool,
    width: AtomicU32,
    height: AtomicU32,
    wakeup: (Mutex<bool>, Condvar),
}

impl Shared {
    fn clear_video(&self) {
        if !self.clear_on_disconnect.load(Ordering::Relaxed) {
            return;
        }
        self.source.clear_video();
        self.width.store(0, Ordering::Relaxed);
        self.height.store(0, Ordering::Relaxed);
    }
}

impl Abort for Arc<Shared> {
    fn aborted(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }
}

struct Output {
    shared: Arc<Shared>,
    clock: Clock,
}

impl Sink for Output {
    fn video(&mut self, frame: &mut Frame, pts_us: u64) {
        frame.timestamp = self.clock.timestamp(pts_us);
        self.shared.width.store(frame.width, Ordering::Relaxed);
        self.shared.height.store(frame.height, Ordering::Relaxed);
        self.shared.source.output_video(frame);
    }

    fn audio(&mut self, audio: &mut Audio, pts_us: u64) {
        audio.timestamp = self.clock.timestamp(pts_us);
        self.shared.source.output_audio(audio);
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
        let (stream, serial) = match usbmux::connect(&self.serial, self.port, &self.shared) {
            Ok(connected) => connected,
            Err(error) => {
                if error != usbmux::Error::Aborted && self.reported_failure != Some(error) {
                    self.reported_failure = Some(error);
                    obs_log!(Level::Info, "not connected: {}", error.message());
                }
                return;
            }
        };
        self.reported_failure = None;
        self.stream(stream, &serial);
        if !self.shared.stopping.load(Ordering::Relaxed) {
            obs_log!(Level::Info, "disconnected from {serial}");
        }
        self.shared.clear_video();
    }

    fn stream(&mut self, mut stream: Stream, serial: &str) {
        if !socket::write_all(&mut stream, &protocol::pack_host_hello()) {
            obs_log!(Level::Warning, "failed to say hello to {serial}");
            return;
        }
        let Some(decoder) = self.decoder.as_mut() else {
            return;
        };
        decoder.reset();
        let mut output = Output {
            shared: Arc::clone(&self.shared),
            clock: Clock::default(),
        };
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            let mut header = [0u8; protocol::MESSAGE_HEADER_SIZE];
            if socket::read_exact(&mut stream, &mut header, &self.shared).is_err() {
                break;
            }
            let (kind, length) = protocol::parse_message_header(&header);
            if length > protocol::MAX_MESSAGE_LENGTH {
                obs_log!(Level::Warning, "bad message length {length}");
                break;
            }
            let payload_size = length as usize;
            buffer.clear();
            buffer.resize(payload_size + INPUT_PADDING, 0);
            if socket::read_exact(&mut stream, &mut buffer[..payload_size], &self.shared).is_err() {
                break;
            }
            if !handle_message(decoder, &mut output, kind, &buffer[..payload_size], serial) {
                break;
            }
        }
    }
}

fn handle_message(decoder: &mut Decoder, output: &mut Output, kind: u8, payload: &[u8], serial: &str) -> bool {
    match kind {
        protocol::MESSAGE_DEVICE_HELLO => {
            let Some(hello) = protocol::parse_device_hello(payload) else {
                obs_log!(Level::Warning, "malformed device hello");
                return false;
            };
            obs_log!(
                Level::Info,
                "connected to {} (Moblin {}) on {serial}",
                hello.name,
                hello.app_version
            );
            name_cache_set(serial, &hello.name);
            true
        }
        protocol::MESSAGE_VIDEO_CONFIG => match protocol::parse_video_config(payload) {
            Some(config) => decoder.configure_video(&config),
            None => {
                obs_log!(Level::Warning, "malformed video config");
                false
            }
        },
        protocol::MESSAGE_VIDEO_FRAME => match protocol::parse_video_frame(payload) {
            Some(frame) => decoder.decode_video(&frame, output),
            None => {
                obs_log!(Level::Warning, "malformed video frame");
                false
            }
        },
        protocol::MESSAGE_AUDIO_CONFIG => {
            match protocol::parse_audio_config(payload) {
                Some(config) => {
                    decoder.configure_audio(&config);
                }
                None => {
                    obs_log!(Level::Warning, "malformed audio config");
                    return false;
                }
            }
            true
        }
        protocol::MESSAGE_AUDIO_FRAME => {
            match protocol::parse_audio_frame(payload) {
                Some(frame) => decoder.decode_audio(&frame, output),
                None => {
                    obs_log!(Level::Warning, "malformed audio frame");
                    return false;
                }
            }
            true
        }
        _ => true,
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
            obs_log!(Level::Error, "failed to create the decoder");
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
            .spawn(|| worker.run())
        {
            Ok(thread) => self.thread = Some(thread),
            Err(_) => obs_log!(Level::Error, "failed to start the receive thread"),
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

fn fill_device_list(list: &mut obs::Property) {
    let deadline = std::time::Instant::now() + DEVICE_LIST_TIMEOUT;
    let expired = move || std::time::Instant::now() >= deadline;
    list.clear_list();
    list.add_translated_entry(text(c"Device.Automatic"), "");
    let Ok(devices) = usbmux::list_devices(&expired) else {
        return;
    };
    for device in devices {
        let label = match name_cache_get(&device.serial) {
            Some(name) => format!("{name} ({})", device.serial),
            None => device.serial.clone(),
        };
        list.add_list_entry(&label, &device.serial);
    }
}

unsafe fn source_of<'a>(data: *mut c_void) -> &'a mut Source {
    &mut *(data as *mut Source)
}

extern "C" fn get_name(_type_data: *mut c_void) -> *const c_char {
    crate::panic::guard("get_name", c"MobCam".as_ptr(), || text(c"MobCam").as_ptr())
}

extern "C" fn create(settings: *mut sys::obs_data_t, source: *mut sys::obs_source_t) -> *mut c_void {
    crate::panic::guard("create", std::ptr::null_mut(), || {
        let mut context = Box::new(Source::new(unsafe { obs::Source::from_raw(source) }));
        context.update(&unsafe { Data::from_raw(settings) });
        Box::into_raw(context) as *mut c_void
    })
}

extern "C" fn destroy(data: *mut c_void) {
    crate::panic::guard("destroy", (), || {
        drop(unsafe { Box::from_raw(data as *mut Source) });
    })
}

extern "C" fn update(data: *mut c_void, settings: *mut sys::obs_data_t) {
    crate::panic::guard("update", (), || {
        unsafe { source_of(data) }.update(&unsafe { Data::from_raw(settings) });
    })
}

extern "C" fn show(data: *mut c_void) {
    crate::panic::guard("show", (), || {
        let context = unsafe { source_of(data) };
        if context.disconnect_when_hidden {
            context.start();
        }
    })
}

extern "C" fn hide(data: *mut c_void) {
    crate::panic::guard("hide", (), || {
        let context = unsafe { source_of(data) };
        if context.disconnect_when_hidden {
            context.stop();
        }
    })
}

extern "C" fn get_width(data: *mut c_void) -> u32 {
    crate::panic::guard("get_width", 0, || {
        unsafe { source_of(data) }.shared.width.load(Ordering::Relaxed)
    })
}

extern "C" fn get_height(data: *mut c_void) -> u32 {
    crate::panic::guard("get_height", 0, || {
        unsafe { source_of(data) }.shared.height.load(Ordering::Relaxed)
    })
}

extern "C" fn get_defaults(settings: *mut sys::obs_data_t) {
    crate::panic::guard("get_defaults", (), || {
        let settings = unsafe { Data::from_raw(settings) };
        settings.set_default_string(SETTING_DEVICE, c"");
        settings.set_default_int(SETTING_PORT, DEFAULT_PORT);
        settings.set_default_bool(SETTING_HARDWARE_DECODE, false);
        settings.set_default_bool(SETTING_BUFFERING, false);
        settings.set_default_bool(SETTING_CLEAR_ON_DISCONNECT, true);
        settings.set_default_bool(SETTING_DISCONNECT_WHEN_HIDDEN, false);
    })
}

extern "C" fn refresh_devices_clicked(
    properties: *mut sys::obs_properties_t,
    _property: *mut sys::obs_property_t,
    _data: *mut c_void,
) -> bool {
    crate::panic::guard("refresh_devices", false, || {
        let mut list = unsafe { obs::properties::get(properties, SETTING_DEVICE) };
        fill_device_list(&mut list);
        true
    })
}

extern "C" fn get_properties(_data: *mut c_void) -> *mut sys::obs_properties_t {
    crate::panic::guard("get_properties", std::ptr::null_mut(), || {
        let mut properties = Properties::new();
        let mut list = properties.add_string_list(SETTING_DEVICE, text(c"Device"));
        fill_device_list(&mut list);
        properties.add_button(c"refresh", text(c"RefreshDevices"), Some(refresh_devices_clicked));
        properties.add_int(SETTING_PORT, text(c"Port"), 1, 65535);
        properties.add_bool(SETTING_HARDWARE_DECODE, text(c"HardwareDecode"));
        properties.add_bool(SETTING_BUFFERING, text(c"Buffering"));
        properties.add_bool(SETTING_CLEAR_ON_DISCONNECT, text(c"ClearOnDisconnect"));
        properties.add_bool(SETTING_DISCONNECT_WHEN_HIDDEN, text(c"DisconnectWhenHidden"));
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
        show: Some(show),
        hide: Some(hide),
        get_width: Some(get_width),
        get_height: Some(get_height),
        get_defaults: Some(get_defaults),
        get_properties: Some(get_properties),
        ..Default::default()
    }
}
