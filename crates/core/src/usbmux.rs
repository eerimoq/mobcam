use plist::{Dictionary, Value};
use std::io::{ErrorKind, Read, Write};
use std::time::Duration;

#[cfg_attr(unix, path = "usbmux/unix.rs")]
#[cfg_attr(windows, path = "usbmux/windows.rs")]
mod socket;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const HEADER_SIZE: usize = 16;
const VERSION_PLIST: u32 = 1;
const TYPE_PLIST: u32 = 8;
const MAX_REPLY_SIZE: u32 = 4 * 1024 * 1024;
const CLIENT_NAME: &str = "obs-mobcam";

pub struct Stream {
    inner: socket::Socket,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Io {
    Aborted,
    Closed,
    Error,
}

pub trait Abort {
    fn aborted(&self) -> bool;
}

impl<F: Fn() -> bool> Abort for F {
    fn aborted(&self) -> bool {
        self()
    }
}

impl Stream {
    pub fn connect_usbmuxd() -> Option<Self> {
        let inner = socket::connect()?;
        inner.set_read_timeout(Some(POLL_INTERVAL)).ok()?;
        Some(Self { inner })
    }

    pub fn write_all(&mut self, data: &[u8]) -> bool {
        self.inner.write_all(data).is_ok()
    }

    pub fn read_exact(&mut self, buffer: &mut [u8], abort: &dyn Abort) -> Result<(), Io> {
        let mut filled = 0;
        while filled < buffer.len() {
            if abort.aborted() {
                return Err(Io::Aborted);
            }
            match self.inner.read(&mut buffer[filled..]) {
                Ok(0) => return Err(Io::Closed),
                Ok(read) => filled += read,
                Err(error) => match error.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => continue,
                    ErrorKind::Interrupted => continue,
                    _ => return Err(Io::Error),
                },
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Error {
    NoDaemon,
    NoDevice,
    Refused,
    Failed,
    Aborted,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Error::NoDaemon => "usbmuxd is not reachable",
            Error::NoDevice => "no device attached over USB",
            Error::Refused => "the device refused the connection",
            Error::Aborted => "aborted",
            Error::Failed => "usbmuxd communication failed",
        }
    }
}

pub struct Device {
    pub device_id: u32,
    pub serial: String,
}

struct Session {
    stream: Stream,
    tag: u32,
}

impl Session {
    fn open() -> Result<Self, Error> {
        let stream = Stream::connect_usbmuxd().ok_or(Error::NoDaemon)?;
        Ok(Self { stream, tag: 0 })
    }

    fn request(&mut self, body: Dictionary, abort: &dyn Abort) -> Result<Dictionary, Error> {
        self.tag += 1;
        let body = encode(body)?;
        let total = HEADER_SIZE
            .checked_add(body.len())
            .and_then(|total| u32::try_from(total).ok())
            .ok_or(Error::Failed)?;
        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(&total.to_le_bytes());
        header[4..8].copy_from_slice(&VERSION_PLIST.to_le_bytes());
        header[8..12].copy_from_slice(&TYPE_PLIST.to_le_bytes());
        header[12..16].copy_from_slice(&self.tag.to_le_bytes());
        if !self.stream.write_all(&header) || !self.stream.write_all(&body) {
            return Err(Error::Failed);
        }
        self.stream.read_exact(&mut header, abort).map_err(Self::io_error)?;
        let total = u32::from_le_bytes(header[0..4].try_into().expect("four bytes"));
        if total < HEADER_SIZE as u32 || total > MAX_REPLY_SIZE {
            return Err(Error::Failed);
        }
        let mut payload = vec![0u8; total as usize - HEADER_SIZE];
        self.stream.read_exact(&mut payload, abort).map_err(Self::io_error)?;
        decode(&payload)
    }

    fn io_error(error: Io) -> Error {
        match error {
            Io::Aborted => Error::Aborted,
            _ => Error::Failed,
        }
    }
}

fn encode(body: Dictionary) -> Result<Vec<u8>, Error> {
    let mut xml = Vec::new();
    Value::Dictionary(body)
        .to_writer_xml(&mut xml)
        .map_err(|_| Error::Failed)?;
    Ok(xml)
}

fn decode(payload: &[u8]) -> Result<Dictionary, Error> {
    match Value::from_reader_xml(payload) {
        Ok(value) => value.into_dictionary().ok_or(Error::Failed),
        Err(_) => Err(Error::Failed),
    }
}

fn create_request(message_type: &str) -> Dictionary {
    let mut body = Dictionary::new();
    body.insert("ClientVersionString".into(), CLIENT_NAME.into());
    body.insert("ProgName".into(), CLIENT_NAME.into());
    body.insert("kLibUSBMuxVersion".into(), 3.into());
    body.insert("MessageType".into(), message_type.into());
    body
}

pub fn list_devices(abort: &dyn Abort) -> Result<Vec<Device>, Error> {
    let mut session = Session::open()?;
    let reply = session.request(create_request("ListDevices"), abort)?;
    Ok(reply
        .get("DeviceList")
        .ok_or(Error::Failed)?
        .as_array()
        .ok_or(Error::Failed)?
        .iter()
        .filter_map(|device| {
            let device = device.as_dictionary()?;
            let properties = device.get("Properties")?.as_dictionary()?;
            let serial = properties.get("SerialNumber")?.as_string()?;
            let device_id = device.get("DeviceID")?.as_signed_integer()?;
            match properties.get("ConnectionType").and_then(Value::as_string) {
                Some(connection) if connection != "USB" => return None,
                _ => {}
            }
            Some(Device {
                device_id: device_id as u32,
                serial: serial.to_string(),
            })
        })
        .collect())
}

pub fn connect_to_device(serial: &str, port: u16, abort: &dyn Abort) -> Result<(Stream, String), Error> {
    let devices = list_devices(abort)?;
    let chosen = devices
        .iter()
        .find(|device| serial.is_empty() || device.serial == serial)
        .ok_or(Error::NoDevice)?;
    let mut session = Session::open()?;
    let mut body = create_request("Connect");
    body.insert("DeviceID".into(), chosen.device_id.into());
    body.insert("PortNumber".into(), port.to_be().into());
    let reply = session.request(body, abort)?;
    if reply.get("Number").and_then(Value::as_signed_integer) != Some(0) {
        return Err(Error::Refused);
    }
    Ok((session.stream, chosen.serial.clone()))
}
