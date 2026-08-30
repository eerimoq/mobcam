use crate::decoder::{Decoder, INPUT_PADDING};
use crate::protocol::{self, DeviceHello};
use crate::usbmux::{self, Abort, Stream};
use crate::{Level, log};

pub use crate::decoder::Sink;

pub trait Handler: Sink {
    fn hello(&mut self, _hello: &DeviceHello) {}
}

pub struct Session {
    decoder: Decoder,
    udid: String,
    port: u16,
    reported_failure: Option<usbmux::Error>,
}

impl Session {
    pub fn new(udid: String, port: u16, hardware: bool, audio: bool) -> Option<Self> {
        Some(Self {
            decoder: Decoder::new(hardware, audio)?,
            udid,
            port,
            reported_failure: None,
        })
    }

    pub fn attempt<H: Handler>(&mut self, abort: &dyn Abort, handler: impl FnOnce(&str) -> H) -> Option<H> {
        let (stream, serial) = match usbmux::connect_to_device(&self.udid, self.port, abort) {
            Ok(connected) => connected,
            Err(error) => {
                if error != usbmux::Error::Aborted && self.reported_failure != Some(error) {
                    self.reported_failure = Some(error);
                    log!(Level::Info, "not connected: {}", error.message());
                }
                return None;
            }
        };
        self.reported_failure = None;
        let mut handler = handler(&serial);
        let connection = Connection {
            decoder: &mut self.decoder,
            stream,
            serial,
        };
        connection.run(&mut handler, abort);
        Some(handler)
    }
}

struct Connection<'a> {
    decoder: &'a mut Decoder,
    stream: Stream,
    serial: String,
}

impl Connection<'_> {
    fn run(mut self, handler: &mut dyn Handler, abort: &dyn Abort) {
        self.receive(handler, abort);
        if !abort.aborted() {
            log!(Level::Info, "disconnected from {}", self.serial);
        }
    }

    fn receive(&mut self, handler: &mut dyn Handler, abort: &dyn Abort) {
        if !self.stream.write_all(&protocol::pack_host_hello()) {
            log!(Level::Warning, "failed to say hello");
            return;
        }
        self.decoder.reset();
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            let mut header = [0u8; protocol::MESSAGE_HEADER_SIZE];
            if self.stream.read_exact(&mut header, abort).is_err() {
                break;
            }
            let (kind, length) = protocol::unpack_message_header(&header);
            if length > protocol::MAX_MESSAGE_SIZE {
                log!(Level::Warning, "message of {length} bytes is too big");
                break;
            }
            let payload_size = length as usize;
            buffer.resize(payload_size + INPUT_PADDING, 0);
            buffer[payload_size..].fill(0);
            if self.stream.read_exact(&mut buffer[..payload_size], abort).is_err() {
                break;
            }
            if !self.handle_message(handler, kind, &buffer[..payload_size]) {
                break;
            }
        }
    }

    fn handle_message(&mut self, handler: &mut dyn Handler, kind: u8, payload: &[u8]) -> bool {
        match kind {
            protocol::MESSAGE_DEVICE_HELLO => self.handle_message_device_hello(handler, payload),
            protocol::MESSAGE_VIDEO_CONFIG => self.handle_message_video_config(payload),
            protocol::MESSAGE_VIDEO_FRAME => self.handle_message_video_frame(handler, payload),
            protocol::MESSAGE_AUDIO_CONFIG => self.handle_message_audio_config(payload),
            protocol::MESSAGE_AUDIO_FRAME => self.handle_message_audio_frame(handler, payload),
            _ => true,
        }
    }

    fn handle_message_device_hello(&mut self, handler: &mut dyn Handler, payload: &[u8]) -> bool {
        let Some(hello) = protocol::unpack_device_hello(payload) else {
            log!(Level::Warning, "malformed device hello");
            return false;
        };
        log!(
            Level::Info,
            "connected to {} (Moblin {}) on {}",
            hello.name,
            hello.app_version,
            self.serial
        );
        handler.hello(&hello);
        true
    }

    fn handle_message_video_config(&mut self, payload: &[u8]) -> bool {
        match protocol::unpack_video_config(payload) {
            Some(config) => self.decoder.configure_video(&config),
            None => {
                log!(Level::Warning, "malformed video config");
                false
            }
        }
    }

    fn handle_message_video_frame(&mut self, handler: &mut dyn Handler, payload: &[u8]) -> bool {
        match protocol::unpack_video_frame(payload) {
            Some(frame) => self.decoder.decode_video(&frame, handler),
            None => {
                log!(Level::Warning, "malformed video frame");
                false
            }
        }
    }

    fn handle_message_audio_config(&mut self, payload: &[u8]) -> bool {
        match protocol::unpack_audio_config(payload) {
            Some(config) => {
                self.decoder.configure_audio(&config);
            }
            None => {
                log!(Level::Warning, "malformed audio config");
                return false;
            }
        }
        true
    }

    fn handle_message_audio_frame(&mut self, handler: &mut dyn Handler, payload: &[u8]) -> bool {
        match protocol::unpack_audio_frame(payload) {
            Some(frame) => self.decoder.decode_audio(&frame, handler),
            None => {
                log!(Level::Warning, "malformed audio frame");
                return false;
            }
        }
        true
    }
}
