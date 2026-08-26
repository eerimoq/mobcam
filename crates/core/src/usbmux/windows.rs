use std::net::TcpStream;

const USBMUXD_ADDRESS: &str = "127.0.0.1:27015";

pub type Socket = TcpStream;

pub fn connect() -> Option<Socket> {
    let socket = TcpStream::connect(USBMUXD_ADDRESS).ok()?;
    let _ = socket.set_nodelay(true);
    Some(socket)
}
