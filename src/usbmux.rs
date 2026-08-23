//! A small usbmuxd client, enough to find attached devices and open a tunnel to
//! a TCP port on one of them.
//!
//! The wire format is a 16 byte little endian header followed by an XML
//! property list. Once a Connect request succeeds the same socket becomes the
//! data tunnel, which is why the stream is handed back to the caller.

use plist::{Dictionary, Value};

use crate::socket::{self, Abort, Io, Stream};

const HEADER_SIZE: usize = 16;
const VERSION_PLIST: u32 = 1;
const TYPE_PLIST: u32 = 8;
/// usbmuxd replies are small; anything larger means the stream is out of sync.
const MAX_REPLY_SIZE: u32 = 4 * 1024 * 1024;
/// How many dictionaries and arrays one reply may open. usbmuxd opens two per
/// device and one for the list around them; the limit is here because a value
/// is dropped recursively, so a deeply nested reply would otherwise run the
/// thread out of stack once the parser had happily built it.
const MAX_COLLECTIONS: usize = 1024;
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
    fn request(&mut self, body: Dictionary, abort: &dyn Abort) -> Result<Value, Error> {
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

        if !socket::write_all(&mut self.stream, &header) || !socket::write_all(&mut self.stream, &body) {
            return Err(Error::Failed);
        }

        socket::read_exact(&mut self.stream, &mut header, abort).map_err(Self::io_error)?;

        let total = u32::from_le_bytes(header[0..4].try_into().expect("four bytes"));

        if total < HEADER_SIZE as u32 || total > MAX_REPLY_SIZE {
            return Err(Error::Failed);
        }

        let mut payload = vec![0u8; total as usize - HEADER_SIZE];

        socket::read_exact(&mut self.stream, &mut payload, abort).map_err(Self::io_error)?;

        decode(&payload)
    }

    fn io_error(error: Io) -> Error {
        match error {
            Io::Aborted => Error::Aborted,
            _ => Error::Failed,
        }
    }
}

/// Writes a request body out as the XML property list usbmuxd reads.
fn encode(body: Dictionary) -> Result<Vec<u8>, Error> {
    let mut xml = Vec::new();

    Value::Dictionary(body)
        .to_writer_xml(&mut xml)
        .map_err(|_| Error::Failed)?;

    Ok(xml)
}

/// Reads a reply, refusing one that nests deeper than it has any reason to.
fn decode(payload: &[u8]) -> Result<Value, Error> {
    if collections(payload) > MAX_COLLECTIONS {
        return Err(Error::Failed);
    }

    Value::from_reader_xml(payload).map_err(|_| Error::Failed)
}

/// Counts the dictionaries and arrays a reply opens. Nothing in it can nest
/// deeper than that, so the count bounds the depth without parsing anything.
fn collections(payload: &[u8]) -> usize {
    (0..payload.len())
        .filter(|index| {
            let rest = &payload[*index..];

            rest.starts_with(b"<dict") || rest.starts_with(b"<array")
        })
        .count()
}

/// Looks one key up in a reply. Anything that is not a dictionary has no keys,
/// which keeps callers from having to check the kind first.
fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_dictionary()?.get(key)
}

/// Starts a request body with the keys usbmuxd expects from every client.
fn request_begin(message_type: &str) -> Dictionary {
    let mut body = Dictionary::new();

    body.insert("ClientVersionString".into(), CLIENT_NAME.into());
    body.insert("ProgName".into(), CLIENT_NAME.into());
    body.insert("kLibUSBMuxVersion".into(), 3.into());
    body.insert("MessageType".into(), message_type.into());

    body
}

/// Picks the devices out of a ListDevices reply.
///
/// Wi-Fi paired devices show up in it too and cannot carry this stream, so only
/// the ones attached over USB are kept.
fn devices_from_reply(reply: &Value) -> Vec<Device> {
    let Some(list) = get(reply, "DeviceList").and_then(Value::as_array) else {
        return Vec::new();
    };

    list.iter()
        .filter_map(|device| {
            let properties = get(device, "Properties")?;
            let serial = get(properties, "SerialNumber")?.as_string()?;
            let device_id = get(device, "DeviceID")?.as_signed_integer()?;

            match get(properties, "ConnectionType").and_then(Value::as_string) {
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
    let reply = session.request(request_begin("ListDevices"), abort)?;
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

    body.insert("DeviceID".into(), chosen.device_id.into());
    // usbmuxd wants the port in network byte order, as an integer.
    body.insert("PortNumber".into(), port.to_be().into());

    let reply = session.request(body, abort)?;

    // Number 3 is a refused connection, which is what a device that is not
    // streaming replies. Everything else is a real failure, but neither is
    // worth a distinct message here.
    if get(&reply, "Number").and_then(Value::as_signed_integer) != Some(0) {
        return Err(Error::Refused);
    }

    Ok((session.stream, chosen.serial.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
    <plist version="1.0"><dict>
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

    fn parse(xml: &str) -> Value {
        decode(xml.as_bytes()).expect("parses")
    }

    /// Wraps `depth` nested arrays in a plist, the shape a reply takes when it
    /// is out to overflow a stack rather than to list devices.
    fn nested(depth: usize) -> String {
        let mut xml = String::from(r#"<plist version="1.0">"#);

        xml.push_str(&"<array>".repeat(depth));
        xml.push_str(&"</array>".repeat(depth));
        xml.push_str("</plist>");

        xml
    }

    #[test]
    fn keeps_usb_devices_and_drops_wifi_paired_ones() {
        let devices = devices_from_reply(&parse(LIST));

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
        let reply = parse(r#"<plist version="1.0"><dict></dict></plist>"#);

        assert!(devices_from_reply(&reply).is_empty());
    }

    /// A device missing the fields that identify it is skipped rather than
    /// taken with a zero id or an empty serial.
    #[test]
    fn incomplete_devices_are_skipped() {
        let reply = parse(
            r#"<plist version="1.0"><dict><key>DeviceList</key><array>
          <dict><key>DeviceID</key><integer>1</integer></dict>
          <dict><key>Properties</key><dict><key>SerialNumber</key><string>x</string></dict></dict>
        </array></dict></plist>"#,
        );

        assert!(devices_from_reply(&reply).is_empty());
    }

    #[test]
    fn every_request_carries_the_keys_usbmuxd_expects() {
        let body = encode(request_begin("ListDevices")).expect("a request is written");
        let parsed = Value::from_reader_xml(body.as_slice()).expect("its own output parses");

        assert_eq!(
            get(&parsed, "ClientVersionString").and_then(Value::as_string),
            Some(CLIENT_NAME)
        );
        assert_eq!(get(&parsed, "ProgName").and_then(Value::as_string), Some(CLIENT_NAME));
        assert_eq!(
            get(&parsed, "kLibUSBMuxVersion").and_then(Value::as_signed_integer),
            Some(3)
        );
        assert_eq!(
            get(&parsed, "MessageType").and_then(Value::as_string),
            Some("ListDevices")
        );
    }

    /// usbmuxd reads the port as a byte swapped integer, so 7790 has to go out
    /// as 0x6E1E rather than 0x1E6E.
    #[test]
    fn the_port_is_sent_in_network_byte_order() {
        let mut body = request_begin("Connect");

        body.insert("PortNumber".into(), 7790u16.to_be().into());

        let parsed = Value::from_reader_xml(encode(body).expect("a request is written").as_slice()).expect("parses");

        assert_eq!(
            get(&parsed, "PortNumber").and_then(Value::as_signed_integer),
            Some(0x6E1E)
        );
    }

    /// The daemon is local but its replies are still input.
    #[test]
    fn a_malformed_reply_is_rejected() {
        assert_eq!(decode(b""), Err(Error::Failed));
        assert_eq!(decode(br#"<plist version="1.0"><dict>"#), Err(Error::Failed));
        assert_eq!(
            decode(br#"<plist version="1.0"><dict><key>a</key>"#),
            Err(Error::Failed)
        );
    }

    /// A value is dropped recursively, so one nested past the limit has to be
    /// turned away before it is built. The reply at the limit is parsed here
    /// too, to show that what is allowed through is what a thread can carry.
    #[test]
    fn a_reply_nested_past_the_limit_is_rejected() {
        assert!(decode(nested(MAX_COLLECTIONS).as_bytes()).is_ok());
        assert_eq!(decode(nested(MAX_COLLECTIONS + 1).as_bytes()), Err(Error::Failed));
    }

    #[test]
    fn a_device_list_is_nowhere_near_the_limit() {
        assert!(collections(LIST.as_bytes()) < MAX_COLLECTIONS);
    }
}
