use std::os::unix::net::UnixStream;

const USBMUXD_PATH: &str = "/var/run/usbmuxd";

pub type Socket = UnixStream;

pub fn connect() -> Option<Socket> {
    UnixStream::connect(USBMUXD_PATH).ok()
}
