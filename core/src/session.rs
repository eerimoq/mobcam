//! The Moblin side of a connection: say hello, then read messages until the
//! device goes away. Shared by the OBS plugin and the virtual camera.

use crate::decoder::{Decoder, INPUT_PADDING, Sink};
use crate::protocol::{self, DeviceHello};
use crate::usbmux::{Abort, Stream};
use crate::{Level, log};

/// What a caller has to provide to run a session: somewhere to put the decoded
/// frames, plus the device hello.
pub trait Handler: Sink {
    fn hello(&mut self, hello: &DeviceHello);
}

/// Says hello and dispatches messages until the device disconnects, the stream
/// breaks or `abort` says to stop.
pub fn stream(stream: &mut Stream, decoder: &mut Decoder, handler: &mut dyn Handler, abort: &dyn Abort) {
    if !stream.write_all(&protocol::pack_host_hello()) {
        log!(Level::Warning, "failed to say hello");
        return;
    }
    decoder.reset();
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let mut header = [0u8; protocol::MESSAGE_HEADER_SIZE];
        if stream.read_exact(&mut header, abort).is_err() {
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
        if stream.read_exact(&mut buffer[..payload_size], abort).is_err() {
            break;
        }
        if !handle_message(decoder, handler, kind, &buffer[..payload_size]) {
            break;
        }
    }
}

fn handle_message(decoder: &mut Decoder, handler: &mut dyn Handler, kind: u8, payload: &[u8]) -> bool {
    match kind {
        protocol::MESSAGE_DEVICE_HELLO => handle_message_device_hello(handler, payload),
        protocol::MESSAGE_VIDEO_CONFIG => handle_message_video_config(decoder, payload),
        protocol::MESSAGE_VIDEO_FRAME => handle_message_video_frame(decoder, handler, payload),
        protocol::MESSAGE_AUDIO_CONFIG => handle_message_audio_config(decoder, payload),
        protocol::MESSAGE_AUDIO_FRAME => handle_message_audio_frame(decoder, handler, payload),
        _ => true,
    }
}

fn handle_message_device_hello(handler: &mut dyn Handler, payload: &[u8]) -> bool {
    let Some(hello) = protocol::unpack_device_hello(payload) else {
        log!(Level::Warning, "malformed device hello");
        return false;
    };
    handler.hello(&hello);
    true
}

fn handle_message_video_config(decoder: &mut Decoder, payload: &[u8]) -> bool {
    match protocol::unpack_video_config(payload) {
        Some(config) => decoder.configure_video(&config),
        None => {
            log!(Level::Warning, "malformed video config");
            false
        }
    }
}

fn handle_message_video_frame(decoder: &mut Decoder, handler: &mut dyn Handler, payload: &[u8]) -> bool {
    match protocol::unpack_video_frame(payload) {
        Some(frame) => decoder.decode_video(&frame, handler),
        None => {
            log!(Level::Warning, "malformed video frame");
            false
        }
    }
}

fn handle_message_audio_config(decoder: &mut Decoder, payload: &[u8]) -> bool {
    match protocol::unpack_audio_config(payload) {
        Some(config) => {
            decoder.configure_audio(&config);
        }
        None => {
            log!(Level::Warning, "malformed audio config");
            return false;
        }
    }
    true
}

fn handle_message_audio_frame(decoder: &mut Decoder, handler: &mut dyn Handler, payload: &[u8]) -> bool {
    match protocol::unpack_audio_frame(payload) {
        Some(frame) => decoder.decode_audio(&frame, handler),
        None => {
            log!(Level::Warning, "malformed audio frame");
            return false;
        }
    }
    true
}
