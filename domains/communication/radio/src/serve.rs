//! The TX15 stand-in on the network: a TCP relay between the plane's autopilot and the station.
//!
//! The station connects to the relay as it would to the autopilot. For each station that
//! connects, the relay connects to the autopilot, copies the station's bytes up to it as they
//! are, and passes what the autopilot sends down through the [`Relay`]: at the rate of the
//! link, with the traffic that the plane's receivers heard added in.

use crate::relay::Relay;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The relay and its clock, shared between the sockets and whoever feeds the receivers.
pub struct Shared {
    pub relay: Relay,
    start: Instant,
}

impl Shared {
    pub fn new(relay: Relay) -> Shared {
        Shared { relay, start: Instant::now() }
    }

    /// Seconds since the relay started: the clock of the link.
    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

pub type Handle = Arc<Mutex<Shared>>;

const TICK: Duration = Duration::from_millis(20);

/// Listens for stations on `listen` and relays them to the autopilot at `upstream`. Returns
/// the address it listens on (so that `127.0.0.1:0` works).
pub fn spawn(listen: &str, upstream: String, shared: Handle) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(listen)?;
    let addr = listener.local_addr()?;
    std::thread::Builder::new().name("radio".into()).spawn(move || {
        for station in listener.incoming().flatten() {
            session(station, &upstream, &shared);
        }
    })?;
    Ok(addr)
}

fn connect(upstream: &str) -> Option<TcpStream> {
    let addr = upstream.to_socket_addrs().ok()?.next()?;
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()
}

/// One station, until it or the autopilot hangs up.
fn session(mut station: TcpStream, upstream: &str, shared: &Handle) {
    let Some(plane) = connect(upstream) else {
        // No autopilot: the station sees the link close and tries again.
        return;
    };
    let _ = (station.set_nodelay(true), plane.set_nodelay(true));
    let done = Arc::new(AtomicBool::new(false));

    // The station to the autopilot, byte for byte.
    let up = {
        let (mut from, mut to, done) = (station.try_clone().unwrap(), plane.try_clone().unwrap(), done.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = from.read(&mut buf) {
                if n == 0 || to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    // The autopilot to the link.
    let down = {
        let (mut from, shared, done) = (plane.try_clone().unwrap(), shared.clone(), done.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = from.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut s = shared.lock().unwrap();
                let now = s.now();
                s.relay.from_plane(&buf[..n], now);
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    // The link to the station.
    let mut last = Instant::now();
    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        let bytes = {
            let mut s = shared.lock().unwrap();
            let now = s.now();
            s.relay.step(last.elapsed().as_secs_f64(), now)
        };
        last = Instant::now();
        if !bytes.is_empty() && station.write_all(&bytes).is_err() {
            break;
        }
    }
    let _ = (station.shutdown(Shutdown::Both), plane.shutdown(Shutdown::Both));
    let _ = (up.join(), down.join());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downlink::Downlink;
    use communication_air::{Payload, Reception, Technology};
    use communication_telemetry::{Beacon, Decoder, frame, global_position};
    use communication_traffic::Position;
    use std::sync::mpsc::channel;

    fn read_until(s: &mut TcpStream, want: usize) -> Vec<u8> {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut got = vec![];
        let mut buf = [0u8; 4096];
        while got.len() < want {
            match s.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
        got
    }

    #[test]
    fn a_station_talks_to_the_autopilot_through_the_relay_and_hears_the_other_planes() {
        // A stand-in autopilot: sends a reply that must arrive and a position, and reports what it receives.
        let plane = TcpListener::bind("127.0.0.1:0").unwrap();
        let plane_addr = plane.local_addr().unwrap().to_string();
        let (got_tx, got_rx) = channel();
        let ack = vec![0xFD, 3, 0, 0, 1, 1, 1, 77, 0, 0, 1, 2, 3, 0, 0];
        let me = Beacon {
            sysid: 1,
            position: Position::new(37.87, -122.27, 50.0),
            velocity: [0.0; 3],
            heading: 0.0,
            uas_id: "me".into(),
            mac: [0; 6],
        };
        let position = frame(1, 1, 0, &global_position(&me, 1));
        let (ack2, position2) = (ack.clone(), position.clone());
        std::thread::spawn(move || {
            let (mut s, _) = plane.accept().unwrap();
            s.write_all(&ack2).unwrap();
            s.write_all(&position2).unwrap();
            let mut buf = [0u8; 16];
            let n = s.read(&mut buf).unwrap();
            got_tx.send(buf[..n].to_vec()).unwrap();
            std::thread::sleep(Duration::from_secs(3));
        });

        let shared = Arc::new(Mutex::new(Shared::new(Relay::new(1, Downlink::DEFAULT_RATE))));
        let addr = spawn("127.0.0.1:0", plane_addr, shared.clone()).unwrap();
        let mut station = TcpStream::connect(addr).unwrap();

        // What the autopilot sent comes through whole.
        let got = read_until(&mut station, ack.len() + position.len());
        assert_eq!(got, [ack, position].concat());

        // What the station sends goes up as it is.
        station.write_all(b"hello").unwrap();
        assert_eq!(got_rx.recv_timeout(Duration::from_secs(5)).unwrap(), b"hello");

        // A plane the receivers hear comes down with the rest.
        let other = Beacon { sysid: 7, position: Position::new(37.871, -122.27, 60.0), ..me.clone() };
        let f = frame(7, 1, 0, &global_position(&other, 1));
        {
            let mut s = shared.lock().unwrap();
            let now = s.now();
            s.relay.pass_on(
                vec![Reception {
                    listener: 0,
                    emitter: 0,
                    technology: Technology::MavlinkNetwork,
                    time: now,
                    payload: Payload::Mavlink(f.clone()),
                }],
                now,
            );
        }
        let got = read_until(&mut station, f.len());
        let heard = Decoder::new(1).push(&got, 1.0);
        assert_eq!(heard.traffic.len(), 1);
        assert_eq!(heard.traffic[0].identity.key, "7");
    }
}
