//! The connection to usbmuxd, and the cancellable reads on top of it.
//!
//! usbmuxd listens on a unix socket on macOS and Linux and on a TCP port on
//! Windows, where the Apple Mobile Device Service provides it. Both are stream
//! sockets that the standard library already knows how to open, so there is no
//! platform socket code here.

use std::io::{ErrorKind, Read, Write};
use std::time::Duration;

#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

/// How long a blocking read waits before it asks whether to give up.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(unix)]
const USBMUXD_PATH: &str = "/var/run/usbmuxd";

#[cfg(windows)]
const USBMUXD_ADDRESS: &str = "127.0.0.1:27015";

#[cfg(unix)]
pub type Stream = UnixStream;

#[cfg(windows)]
pub type Stream = TcpStream;

/// What a read stopped for.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Io {
    /// The caller asked to give up, which is how a source being stopped gets
    /// its worker thread back.
    Aborted,
    /// The far end closed the connection.
    Closed,
    Error,
}

/// Asked while a read is blocked. Returning true gives up on it.
pub trait Abort {
    fn aborted(&self) -> bool;
}

impl<F: Fn() -> bool> Abort for F {
    fn aborted(&self) -> bool {
        self()
    }
}

/// Opens a connection to usbmuxd, or reports that it is not reachable.
pub fn connect_usbmuxd() -> Option<Stream> {
    #[cfg(unix)]
    let stream = UnixStream::connect(USBMUXD_PATH).ok()?;

    #[cfg(windows)]
    let stream = {
        let stream = TcpStream::connect(USBMUXD_ADDRESS).ok()?;
        let _ = stream.set_nodelay(true);
        stream
    };

    // The read timeout is what makes a blocked read notice an abort, so it has
    // to be in place before anything is read.
    stream.set_read_timeout(Some(POLL_INTERVAL)).ok()?;

    Some(stream)
}

pub fn write_all(stream: &mut Stream, data: &[u8]) -> bool {
    stream.write_all(data).is_ok()
}

/// Fills `buffer`, giving up whenever `abort` says to.
///
/// The read timeout turns into a chance to ask the abort callback rather than a
/// failure, so a stopped source waits at most one interval.
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
                // What the read timeout above produces; platforms disagree on
                // which of the two they report.
                ErrorKind::WouldBlock | ErrorKind::TimedOut => continue,
                ErrorKind::Interrupted => continue,
                _ => return Err(Io::Error),
            },
        }
    }

    Ok(())
}
