//! The air between the ground and each drone (plans/unified-link.md, step 2).
//!
//! The ground sends MAVLink to a port of the air, and the air carries it to the drone's SITL, and
//! back. Every frame that goes either way is held against the world at that moment: a drone that
//! the radio cannot reach (a radio shadow, the mesh, or past the range of the radio) hears
//! nothing and is heard from not at all, as in the field. The referee decides who is in contact
//! (`Game::jam`); the air enforces it on the wire, so that nothing else has to.
//!
//! The air carries whole frames: it cuts the stream with the radio's framer, and drops a frame or
//! passes it.

use communication_radio::frame::Framer;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Whether the radio reaches the drone right now.
pub type Contact = Arc<dyn Fn() -> bool + Send + Sync>;

/// What the air did with the frames of one drone.
#[derive(Default)]
pub struct Counts {
    pub carried: AtomicU64,
    pub dropped: AtomicU64,
}

impl Counts {
    pub fn carried(&self) -> u64 {
        self.carried.load(Ordering::Relaxed)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The air in front of one drone's MAVLink port.
pub struct Channel {
    /// Where the ground connects.
    pub port: u16,
    pub counts: Arc<Counts>,
}

/// How long the air waits for a drone's SITL to listen.
const PATIENCE: Duration = Duration::from_secs(120);

/// Opens the air in front of the MAVLink port `upstream` of a drone. `contact` says, for each
/// frame, whether the radio reaches the drone.
pub fn open(upstream: u16, contact: Contact) -> io::Result<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let counts = Arc::new(Counts::default());
    let c = counts.clone();
    std::thread::Builder::new().name(format!("air-{upstream}")).spawn(move || {
        for ground in listener.incoming().flatten() {
            carry(ground, upstream, &contact, &c);
        }
    })?;
    Ok(Channel { port, counts })
}

fn connect(upstream: u16) -> Option<TcpStream> {
    let end = Instant::now() + PATIENCE;
    while Instant::now() < end {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", upstream)) {
            return Some(s);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    None
}

/// One frame stream, from `from` to `to`: whole frames that the contact lets through.
fn pump(mut from: TcpStream, mut to: TcpStream, contact: Contact, counts: Arc<Counts>, done: Arc<AtomicBool>) {
    let mut framer = Framer::default();
    let mut buf = [0u8; 4096];
    while let Ok(n) = from.read(&mut buf) {
        if n == 0 {
            break;
        }
        for frame in framer.push(&buf[..n]) {
            if !contact() {
                counts.dropped.fetch_add(1, Ordering::Relaxed);
            } else if to.write_all(&frame).is_ok() {
                counts.carried.fetch_add(1, Ordering::Relaxed);
            } else {
                done.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
    done.store(true, Ordering::SeqCst);
}

/// The ground and the drone, until one hangs up.
fn carry(ground: TcpStream, upstream: u16, contact: &Contact, counts: &Arc<Counts>) {
    let Some(drone) = connect(upstream) else { return };
    let _ = (ground.set_nodelay(true), drone.set_nodelay(true));
    let done = Arc::new(AtomicBool::new(false));
    let (g2, d2) = (ground.try_clone().unwrap(), drone.try_clone().unwrap());
    let up = {
        let (contact, counts, done) = (contact.clone(), counts.clone(), done.clone());
        std::thread::spawn(move || pump(g2, d2, contact, counts, done))
    };
    let down = {
        let (g, d) = (ground.try_clone().unwrap(), drone.try_clone().unwrap());
        let (contact, counts, done) = (contact.clone(), counts.clone(), done.clone());
        std::thread::spawn(move || pump(d, g, contact, counts, done))
    };
    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = (ground.shutdown(Shutdown::Both), drone.shutdown(Shutdown::Both));
    let _ = (up.join(), down.join());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    /// A MAVLink 2 frame with a payload of `n` bytes.
    fn frame(n: u8) -> Vec<u8> {
        let mut f = vec![0xFD, n, 0, 0, 0, 1, 1, 77, 0, 0];
        f.extend(std::iter::repeat_n(7, n as usize));
        f.extend([0, 0]);
        f
    }

    /// A stand-in SITL that reports what it hears and sends one frame down when told.
    fn sitl() -> (u16, std::sync::mpsc::Receiver<Vec<u8>>, std::sync::mpsc::Sender<Vec<u8>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (heard_tx, heard_rx) = channel();
        let (send_tx, send_rx) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = s.try_clone().unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 64];
                while let Ok(n) = r.read(&mut buf) {
                    if n == 0 || heard_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            });
            for f in send_rx {
                let _ = s.write_all(&f);
            }
        });
        (port, heard_rx, send_tx)
    }

    #[test]
    fn a_frame_goes_both_ways_while_the_radio_reaches_the_drone() {
        let (port, heard, send) = sitl();
        let air = open(port, Arc::new(|| true)).unwrap();
        let mut ground = TcpStream::connect(("127.0.0.1", air.port)).unwrap();
        ground.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        ground.write_all(&frame(3)).unwrap();
        assert_eq!(heard.recv_timeout(Duration::from_secs(5)).unwrap(), frame(3));
        send.send(frame(5)).unwrap();
        let mut got = vec![0u8; frame(5).len()];
        ground.read_exact(&mut got).unwrap();
        assert_eq!(got, frame(5));
        assert_eq!((air.counts.carried(), air.counts.dropped()), (2, 0));
    }

    #[test]
    fn no_frame_goes_either_way_while_the_drone_is_in_a_shadow_and_they_go_again_after() {
        let (port, heard, send) = sitl();
        let reach = Arc::new(AtomicBool::new(false));
        let r = reach.clone();
        let air = open(port, Arc::new(move || r.load(Ordering::SeqCst))).unwrap();
        let mut ground = TcpStream::connect(("127.0.0.1", air.port)).unwrap();
        ground.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        ground.write_all(&frame(3)).unwrap();
        send.send(frame(5)).unwrap();
        assert!(heard.recv_timeout(Duration::from_millis(500)).is_err(), "the drone heard the ground");
        assert!(ground.read(&mut [0u8; 64]).is_err(), "the ground heard the drone");
        assert_eq!(air.counts.dropped(), 2);
        reach.store(true, Ordering::SeqCst);
        ground.write_all(&frame(4)).unwrap();
        assert_eq!(heard.recv_timeout(Duration::from_secs(5)).unwrap(), frame(4));
        assert_eq!((air.counts.carried(), air.counts.dropped()), (1, 2));
    }

    #[test]
    fn a_frame_in_two_pieces_is_carried_whole() {
        let (port, heard, _send) = sitl();
        let air = open(port, Arc::new(|| true)).unwrap();
        let mut ground = TcpStream::connect(("127.0.0.1", air.port)).unwrap();
        let f = frame(9);
        ground.write_all(&f[..6]).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        ground.write_all(&f[6..]).unwrap();
        assert_eq!(heard.recv_timeout(Duration::from_secs(5)).unwrap(), f);
    }
}
