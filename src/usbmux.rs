//! A small usbmuxd client, enough to find attached devices and open a tunnel to
//! a TCP port on one of them.
//!
//! The wire format is a 16 byte little endian header followed by an XML
//! property list. Once a Connect request succeeds the same socket becomes the
//! data tunnel, which is why the stream is handed back to the caller.

use crate::plist::{self, Value, Writer};
use crate::socket::{self, Abort, Io, Stream};

const HEADER_SIZE: usize = 16;
const VERSION_PLIST: u32 = 1;
const TYPE_PLIST: u32 = 8;
/// usbmuxd replies are small; anything larger means the stream is out of sync.
const MAX_REPLY_SIZE: u32 = 4 * 1024 * 1024;
const CLIENT_NAME: &str = "obs-mobcam";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Error {
    /// usbmuxd is not running, or not installed.
    NoDaemon,
    /// No device is attached over USB, or none with the wanted serial.
    NoDevice,
    /// The device is there but nothing listens on the port.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub device_id: u32,
    pub serial: String,
}

/// A connection to usbmuxd, used for one or two requests and then either closed
/// or handed on as the tunnel.
struct Session {
    stream: Stream,
    tag: u32,
}

impl Session {
    fn open() -> Result<Self, Error> {
        let stream = socket::connect_usbmuxd().ok_or(Error::NoDaemon)?;

        Ok(Self { stream, tag: 0 })
    }

    /// Sends a request body and returns the reply it was answered with.
    fn request(&mut self, body: &str, abort: &dyn Abort) -> Result<Value, Error> {
        self.tag += 1;

        let total = HEADER_SIZE
            .checked_add(body.len())
            .and_then(|total| u32::try_from(total).ok())
            .ok_or(Error::Failed)?;

        let mut header = [0u8; HEADER_SIZE];

        header[0..4].copy_from_slice(&total.to_le_bytes());
        header[4..8].copy_from_slice(&VERSION_PLIST.to_le_bytes());
        header[8..12].copy_from_slice(&TYPE_PLIST.to_le_bytes());
        header[12..16].copy_from_slice(&self.tag.to_le_bytes());

        if !socket::write_all(&mut self.stream, &header) || !socket::write_all(&mut self.stream, body.as_bytes()) {
            return Err(Error::Failed);
        }

        socket::read_exact(&mut self.stream, &mut header, abort).map_err(Self::io_error)?;

        let total = u32::from_le_bytes(header[0..4].try_into().expect("four bytes"));

        if total < HEADER_SIZE as u32 || total > MAX_REPLY_SIZE {
            return Err(Error::Failed);
        }

        let mut payload = vec![0u8; total as usize - HEADER_SIZE];

        socket::read_exact(&mut self.stream, &mut payload, abort).map_err(Self::io_error)?;

        plist::parse(&payload).ok_or(Error::Failed)
    }

    fn io_error(error: Io) -> Error {
        match error {
            Io::Aborted => Error::Aborted,
            _ => Error::Failed,
        }
    }
}

/// Starts a request body with the keys usbmuxd expects from every client.
fn request_begin(message_type: &str) -> Writer {
    let mut body = Writer::new();

    body.string("ClientVersionString", CLIENT_NAME);
    body.string("ProgName", CLIENT_NAME);
    body.integer("kLibUSBMuxVersion", 3);
    body.string("MessageType", message_type);

    body
}

/// Picks the devices out of a ListDevices reply.
///
/// Wi-Fi paired devices show up in it too and cannot carry this stream, so only
/// the ones attached over USB are kept.
fn devices_from_reply(reply: &Value) -> Vec<Device> {
    let Some(list) = reply.get("DeviceList") else {
        return Vec::new();
    };

    list.array()
        .iter()
        .filter_map(|device| {
            let properties = device.get("Properties")?;
            let serial = properties.get_string("SerialNumber")?;
            let device_id = device.get_integer("DeviceID")?;

            match properties.get_string("ConnectionType") {
                Some(connection) if connection != "USB" => return None,
                _ => {}
            }

            Some(Device {
                device_id: device_id as u32,
                serial: serial.to_string(),
            })
        })
        .collect()
}

/// Lists the devices attached over USB.
pub fn list_devices(abort: &dyn Abort) -> Result<Vec<Device>, Error> {
    let mut session = Session::open()?;
    let reply = session.request(&request_begin("ListDevices").finish(), abort)?;
    let devices = devices_from_reply(&reply);

    if devices.is_empty() {
        return Err(Error::NoDevice);
    }

    Ok(devices)
}

/// Opens a connection to a TCP port on a device.
///
/// An empty serial takes the first device. On success the stream is the tunnel
/// and the serial names the device it landed on.
pub fn connect(serial: &str, port: u16, abort: &dyn Abort) -> Result<(Stream, String), Error> {
    let devices = list_devices(abort)?;

    let chosen = devices
        .iter()
        .find(|device| serial.is_empty() || device.serial == serial)
        .ok_or(Error::NoDevice)?;

    let mut session = Session::open()?;
    let mut body = request_begin("Connect");

    body.integer("DeviceID", i64::from(chosen.device_id));
    // usbmuxd wants the port in network byte order, as an integer.
    body.integer("PortNumber", i64::from(port.to_be()));

    let reply = session.request(&body.finish(), abort)?;

    // Number 3 is a refused connection, which is what a device that is not
    // streaming replies. Everything else is a real failure, but neither is
    // worth a distinct message here.
    if reply.get_integer("Number") != Some(0) {
        return Err(Error::Refused);
    }

    Ok((session.stream, chosen.serial.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"<plist version="1.0"><dict>
      <key>DeviceList</key>
      <array>
        <dict>
          <key>DeviceID</key><integer>7</integer>
          <key>Properties</key>
          <dict>
            <key>ConnectionType</key><string>USB</string>
            <key>SerialNumber</key><string>usb-device</string>
          </dict>
        </dict>
        <dict>
          <key>DeviceID</key><integer>9</integer>
          <key>Properties</key>
          <dict>
            <key>ConnectionType</key><string>Network</string>
            <key>SerialNumber</key><string>wifi-device</string>
          </dict>
        </dict>
      </array>
    </dict></plist>"#;

    #[test]
    fn keeps_usb_devices_and_drops_wifi_paired_ones() {
        let reply = plist::parse(LIST.as_bytes()).expect("parses");
        let devices = devices_from_reply(&reply);

        assert_eq!(
            devices,
            vec![Device {
                device_id: 7,
                serial: "usb-device".to_string(),
            }]
        );
    }

    #[test]
    fn a_reply_without_a_device_list_yields_nothing() {
        let reply = plist::parse(b"<plist version=\"1.0\"><dict></dict></plist>").expect("parses");

        assert!(devices_from_reply(&reply).is_empty());
    }

    /// A device missing the fields that identify it is skipped rather than
    /// taken with a zero id or an empty serial.
    #[test]
    fn incomplete_devices_are_skipped() {
        let xml = r#"<plist version="1.0"><dict><key>DeviceList</key><array>
          <dict><key>DeviceID</key><integer>1</integer></dict>
          <dict><key>Properties</key><dict><key>SerialNumber</key><string>x</string></dict></dict>
        </array></dict></plist>"#;

        let reply = plist::parse(xml.as_bytes()).expect("parses");

        assert!(devices_from_reply(&reply).is_empty());
    }

    #[test]
    fn every_request_carries_the_keys_usbmuxd_expects() {
        let body = request_begin("ListDevices").finish();
        let parsed = plist::parse(body.as_bytes()).expect("parses");

        assert_eq!(parsed.get_string("ClientVersionString"), Some(CLIENT_NAME));
        assert_eq!(parsed.get_string("ProgName"), Some(CLIENT_NAME));
        assert_eq!(parsed.get_integer("kLibUSBMuxVersion"), Some(3));
        assert_eq!(parsed.get_string("MessageType"), Some("ListDevices"));
    }

    /// usbmuxd reads the port as a byte swapped integer, so 7790 has to go out
    /// as 0x6E1E rather than 0x1E6E.
    #[test]
    fn the_port_is_sent_in_network_byte_order() {
        let mut body = request_begin("Connect");

        body.integer("PortNumber", i64::from(7790u16.to_be()));

        let parsed = plist::parse(body.finish().as_bytes()).expect("parses");

        assert_eq!(parsed.get_integer("PortNumber"), Some(0x6E1E));
    }
}
