//! The line the radio is on.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One end of a serial line.
pub trait Wire: Send {
    /// Reads what is waiting (`Some(0)` is nothing yet); `None` is a line that has hung up.
    fn read_some(&mut self, buf: &mut [u8]) -> Option<usize>;
    /// Writes all of `bytes`; false if the line has hung up.
    fn write_all(&mut self, bytes: &[u8]) -> bool;
}

/// A TCP stream: the sim's virtual UART.
pub struct TcpWire(TcpStream);

impl TcpWire {
    pub fn new(stream: TcpStream) -> std::io::Result<TcpWire> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(TcpWire(stream))
    }

    /// Connect to the flight computer's line at `addr` (`127.0.0.1:5751`).
    pub fn connect(addr: &str) -> std::io::Result<TcpWire> {
        use std::net::ToSocketAddrs;
        let a = addr.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, addr))?;
        TcpWire::new(TcpStream::connect_timeout(&a, Duration::from_secs(2))?)
    }
}

impl Wire for TcpWire {
    fn read_some(&mut self, buf: &mut [u8]) -> Option<usize> {
        match self.0.read(buf) {
            Ok(0) => None,
            Ok(n) => Some(n),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted | ErrorKind::TimedOut) => {
                Some(0)
            }
            Err(_) => None,
        }
    }

    fn write_all(&mut self, mut bytes: &[u8]) -> bool {
        while !bytes.is_empty() {
            match self.0.write(bytes) {
                Ok(0) => return false,
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                Err(_) => return false,
            }
        }
        true
    }
}

#[derive(Default)]
struct Pipe {
    bytes: VecDeque<u8>,
    closed: bool,
}

/// One end of an in-memory line. What one end writes, the other reads; a test plays the other end.
#[derive(Clone)]
pub struct Mock {
    incoming: Arc<Mutex<Pipe>>,
    outgoing: Arc<Mutex<Pipe>>,
}

/// Two ends of an in-memory line.
pub fn mock_pair() -> (Mock, Mock) {
    let (a, b) = (Arc::new(Mutex::new(Pipe::default())), Arc::new(Mutex::new(Pipe::default())));
    (Mock { incoming: a.clone(), outgoing: b.clone() }, Mock { incoming: b, outgoing: a })
}

impl Mock {
    /// Hang up: the other end sees the line close once it has read what was written.
    pub fn hang_up(&self) {
        self.outgoing.lock().unwrap().closed = true;
    }

    /// Everything waiting on this end, taken.
    pub fn take(&self) -> Vec<u8> {
        self.incoming.lock().unwrap().bytes.drain(..).collect()
    }
}

impl Wire for Mock {
    fn read_some(&mut self, buf: &mut [u8]) -> Option<usize> {
        let mut pipe = self.incoming.lock().unwrap();
        if pipe.bytes.is_empty() {
            return if pipe.closed { None } else { Some(0) };
        }
        let n = buf.len().min(pipe.bytes.len());
        for slot in buf.iter_mut().take(n) {
            *slot = pipe.bytes.pop_front().unwrap();
        }
        Some(n)
    }

    fn write_all(&mut self, bytes: &[u8]) -> bool {
        let mut pipe = self.outgoing.lock().unwrap();
        if pipe.closed {
            return false;
        }
        pipe.bytes.extend(bytes);
        true
    }
}
