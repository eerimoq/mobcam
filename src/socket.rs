use std::io::{ErrorKind, Read, Write};
use std::time::Duration;

#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(unix)]
const USBMUXD_PATH: &str = "/var/run/usbmuxd";

#[cfg(windows)]
const USBMUXD_ADDRESS: &str = "127.0.0.1:27015";

#[cfg(unix)]
pub type Stream = UnixStream;

#[cfg(windows)]
pub type Stream = TcpStream;

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

pub fn connect_usbmuxd() -> Option<Stream> {
    #[cfg(unix)]
    let stream = UnixStream::connect(USBMUXD_PATH).ok()?;

    #[cfg(windows)]
    let stream = {
        let stream = TcpStream::connect(USBMUXD_ADDRESS).ok()?;
        let _ = stream.set_nodelay(true);
        stream
    };

    stream.set_read_timeout(Some(POLL_INTERVAL)).ok()?;

    Some(stream)
}

pub fn write_all(stream: &mut Stream, data: &[u8]) -> bool {
    stream.write_all(data).is_ok()
}

pub fn read_exact(stream: &mut Stream, buffer: &mut [u8], abort: &dyn Abort) -> Result<(), Io> {
    let mut filled = 0;

    while filled < buffer.len() {
        if abort.aborted() {
            return Err(Io::Aborted);
        }

        match stream.read(&mut buffer[filled..]) {
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
