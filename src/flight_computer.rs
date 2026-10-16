//! The flight computer (`--flight-computer LINE`): a computer between the TX15 and ArduPilot,
//! which the TX15 talks to on a serial line (UART) in CRSF instead of as a USB controller.
//! See `domains/aviation`.
//!
//! - The TX15's channels become the radio channels ArduPilot reads (`SimCore::uart_rc`). If the
//!   line goes quiet, the throttle channel drops below ArduPilot's failsafe level.
//! - The plane's state goes back down the line as CRSF telemetry, and so does ArduPilot's
//!   MAVLink (it connects to ArduPilot at `--autopilot`, default 127.0.0.1:5760). The
//!   ground station gets its telemetry from the TX15, not from ArduPilot.
//!
//! `LINE` is `tcp:PORT` (the sim listens: a virtual UART, for `--radio-uart` and for tests) or the
//! path of a serial device (a USB-serial adapter to the radio, or a real UART).
//! [`tx15`] is the other end of a virtual line: the TX15 stand-in for `--radio`.

use crate::sim::SimCore;
use aviation_computer::{FlightComputer, Handset, Vehicle};
use communication_traffic::Position;
use simulation_physics as physics;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Core = Arc<Mutex<SimCore>>;

const TICK: Duration = Duration::from_millis(4);

/// One end of a serial line.
pub enum Line {
    Tcp(TcpStream),
    #[cfg(unix)]
    Device(std::fs::File),
}

impl Read for Line {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Line::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Line::Device(f) => f.read(buf),
        }
    }
}

impl Write for Line {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Line::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Line::Device(f) => f.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Line::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Line::Device(f) => f.flush(),
        }
    }
}

/// CRSF runs at 420 000 baud.
pub const BAUD: u32 = 420_000;

#[cfg(target_os = "macos")]
fn open_device(path: &str) -> std::io::Result<Line> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK).open(path)?;
    let fd = f.as_raw_fd();
    // SAFETY: fd is open; termios is plain data; IOSSIOSPEED takes a pointer to a speed_t.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        libc::cfmakeraw(&mut t);
        t.c_cflag |= libc::CLOCAL | libc::CREAD;
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 0;
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        const IOSSIOSPEED: libc::c_ulong = 0x8004_5402;
        let speed: libc::speed_t = BAUD as libc::speed_t;
        // A pseudo-terminal has no speed to set: ignore the failure.
        let _ = libc::ioctl(fd, IOSSIOSPEED, &speed);
    }
    Ok(Line::Device(f))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_device(path: &str) -> std::io::Result<Line> {
    Ok(Line::Device(std::fs::OpenOptions::new().read(true).write(true).open(path)?))
}

#[cfg(not(unix))]
fn open_device(path: &str) -> std::io::Result<Line> {
    Err(std::io::Error::new(ErrorKind::Unsupported, format!("serial devices ({path}) need a Unix system")))
}

fn nonblocking(l: Line) -> Line {
    if let Line::Tcp(s) = &l {
        let _ = (s.set_nonblocking(true), s.set_nodelay(true));
    }
    l
}

/// Reads what is waiting; `Ok(None)` is a line that has hung up.
fn read_some(l: &mut Line, buf: &mut [u8]) -> Option<usize> {
    match l.read(buf) {
        Ok(0) if matches!(l, Line::Tcp(_)) => None,
        Ok(n) => Some(n),
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted | ErrorKind::TimedOut) => Some(0),
        Err(_) => None,
    }
}

fn write_all(l: &mut Line, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        match l.write(bytes) {
            Ok(0) => return false,
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => return false,
        }
    }
    true
}

pub struct Config {
    /// `tcp:PORT` or a serial device.
    pub line: String,
    /// Where ArduPilot's MAVLink is.
    pub autopilot: String,
    /// The pilot box, where the sim's origin is on the earth.
    pub home: Position,
}

/// The plane as the flight computer reports it.
pub fn vehicle(c: &SimCore, home: &Position) -> Vehicle {
    let s = &c.state;
    let (roll, pitch, heading) = physics::euler(s.rot);
    let p = home.moved(-s.pos.z, s.pos.x, s.pos.y);
    let (armed, mode) = c.ap.vehicle().map(|(a, m)| (a, m.to_string())).unwrap_or((false, "-".into()));
    let soc = (1.0 - s.used_mah / c.params.battery_capacity_mah).clamp(0.0, 1.0);
    Vehicle {
        armed,
        mode,
        volts: s.voltage,
        amps: s.current,
        used_mah: s.used_mah.max(0.0) as u32,
        percent: (soc * 100.0).round() as u8,
        roll,
        pitch,
        yaw: heading,
        lat: p.lat,
        lon: p.lon,
        alt: p.alt,
        ground_speed: (s.vel.x.powi(2) + s.vel.z.powi(2)).sqrt(),
        airspeed: s.airspeed,
        climb: s.vel.y,
        heading: heading.to_degrees().rem_euclid(360.0),
        sats: 12,
    }
}

/// Starts the flight computer on its line. On a serial device (the real TX15 on its USB cable),
/// it also listens on a local port for the ground station's relay, and returns its address: what
/// the radio forwards up for the ground station comes out there, and what the ground station
/// sends goes down to the radio, which carries it up to the plane.
pub fn spawn(core: Core, cfg: Config) -> std::io::Result<Option<SocketAddr>> {
    let listener = match cfg.line.strip_prefix("tcp:") {
        Some(port) => {
            let l = TcpListener::bind(("127.0.0.1", port.parse::<u16>().map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))?))?;
            println!("flight computer: waiting for the TX15 on {}", l.local_addr()?);
            Some(l)
        }
        None => None,
    };
    let device = if listener.is_none() { Some(open_device(&cfg.line)?) } else { None };
    let ground_station = if device.is_some() {
        let l = TcpListener::bind("127.0.0.1:0")?;
        l.set_nonblocking(true)?;
        println!("flight computer: the ground station connects to {}", l.local_addr()?);
        Some(l)
    } else {
        None
    };
    let gs_addr = ground_station.as_ref().map(|l| l.local_addr()).transpose()?;
    if device.is_some() {
        println!("flight computer: on {} at {BAUD} baud", cfg.line);
    }
    std::thread::Builder::new().name("flight-computer".into()).spawn(move || {
        let mut device = device;
        loop {
            let line = match (&listener, device.take()) {
                (_, Some(d)) => d,
                (Some(l), None) => match l.accept() {
                    Ok((s, _)) => Line::Tcp(s),
                    Err(_) => continue,
                },
                (None, None) => return,
            };
            run(&core, &cfg, nonblocking(line), ground_station.as_ref());
            core.lock().unwrap().uart_rc = None;
            if listener.is_none() {
                // A serial device that went away (the radio was unplugged and plugged in again)
                // is a dead descriptor: open it again when it is back, and say so.
                println!("flight computer: lost {}; waiting for it to come back", cfg.line);
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    if let Ok(d) = open_device(&cfg.line) {
                        println!("flight computer: on {} again", cfg.line);
                        device = Some(d);
                        break;
                    }
                }
            }
        }
    })?;
    Ok(gs_addr)
}

/// The sim's own pipe to ArduPilot's MAVLink, reconnecting when it drops.
fn connect_autopilot(addr: &str) -> Option<TcpStream> {
    use std::net::ToSocketAddrs;
    let a = addr.to_socket_addrs().ok()?.next()?;
    let s = TcpStream::connect_timeout(&a, Duration::from_millis(500)).ok()?;
    let _ = (s.set_nonblocking(true), s.set_nodelay(true));
    Some(s)
}

/// One TX15 on the line, until it hangs up.
fn run(core: &Core, cfg: &Config, mut line: Line, ground_station: Option<&TcpListener>) {
    let mut gs: Option<TcpStream> = None;
    let start = Instant::now();
    // The widget on the radio decodes this with a small CPU allowance per call: a real radio gets
    // less MAVLink than the virtual one the tests use.
    #[cfg(unix)]
    let on_radio = matches!(line, Line::Device(_));
    #[cfg(not(unix))]
    let on_radio = false;
    let mut fc = FlightComputer::with_mavlink_rate(if on_radio { 1500.0 } else { aviation_computer::MAVLINK_RATE });
    let mut autopilot: Option<TcpStream> = None;
    let mut next_connect = 0.0;
    let mut buf = [0u8; 4096];
    let (mut heard, mut sent, mut next_report, mut was_up) = (0usize, 0usize, 2.0, false);
    loop {
        let now = start.elapsed().as_secs_f64();
        match read_some(&mut line, &mut buf) {
            None => return,
            Some(n) => {
                heard += n;
                fc.from_line(&buf[..n], now)
            }
        }
        // Say how the line is doing: what the TX15 sends, and whether the link is up.
        if now >= next_report {
            let up = fc.link_up(now);
            if up != was_up || heard > 0 || sent > 0 {
                let ch = fc.channels(now);
                println!(
                    "flight computer: link {} ({} bytes heard, {} sent in the last 2 s, quality {} %); roll {} pitch {} throttle {} yaw {} arm {} mode {}",
                    if up { "up" } else { "down" },
                    heard,
                    sent,
                    fc.quality(now),
                    ch[0],
                    ch[1],
                    ch[2],
                    ch[3],
                    ch[4],
                    ch[7]
                );
            }
            (was_up, heard, sent, next_report) = (up, 0, 0, now + 2.0);
        }
        if autopilot.is_none() && now >= next_connect {
            autopilot = connect_autopilot(&cfg.autopilot);
            next_connect = now + 1.0;
        }
        if let Some(ap) = &mut autopilot {
            match ap.read(&mut buf) {
                Ok(0) => autopilot = None,
                Ok(n) => fc.from_autopilot(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(_) => autopilot = None,
            }
        }
        if let Some(ap) = &mut autopilot {
            let up = fc.take_for_autopilot();
            if !up.is_empty() {
                let mut s = &*ap;
                if s.write_all(&up).is_err() {
                    autopilot = None;
                }
            }
        }
        // The ground station's relay: what it says goes down to the radio, and what the radio
        // forwards for it comes out here.
        if let Some(l) = ground_station {
            if gs.is_none() {
                if let Ok((s, _)) = l.accept() {
                    let _ = (s.set_nonblocking(true), s.set_nodelay(true));
                    gs = Some(s);
                }
            }
            if let Some(s) = &mut gs {
                let mut keep = true;
                match s.read(&mut buf) {
                    Ok(0) => keep = false,
                    Ok(n) => fc.from_ground_station(&buf[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(_) => keep = false,
                }
                let up = fc.take_for_ground_station();
                if keep && !up.is_empty() && s.write_all(&up).is_err() {
                    keep = false;
                }
                if !keep {
                    gs = None;
                }
            }
        }
        let v = {
            let mut c = core.lock().unwrap();
            c.uart_rc = Some(fc.channels(now));
            vehicle(&c, &cfg.home)
        };
        // ArduPilot's own heartbeat says the mode best: the console's report only comes when the
        // ground station is connected and hears it.
        let mut v = v;
        if let Some((armed, mode)) = fc.autopilot_mode() {
            (v.armed, v.mode) = (armed, mode.to_string());
        }
        let down = fc.to_line(&v, now);
        sent += down.len();
        if !down.is_empty() && !write_all(&mut line, &down) {
            return;
        }
        std::thread::sleep(TICK);
    }
}

/// The TX15 stand-in for `--radio`: the other end of a virtual line to the flight computer. It
/// listens for the radio relay (which takes the place of the ground station's link to the
/// plane), puts the pilot's controls on the line as the TX15 would, and passes the MAVLink that
/// comes down to the relay, and what the relay sends to the line. Returns where it listens.
pub fn tx15(core: Core, line: &str) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let uart = line.strip_prefix("tcp:").unwrap_or(line).to_string();
    std::thread::Builder::new().name("tx15".into()).spawn(move || {
        for relay in listener.incoming().flatten() {
            let _ = relay.set_nonblocking(true);
            let Some(plane) = (|| {
                use std::net::ToSocketAddrs;
                let a = uart.to_socket_addrs().ok()?.next()?;
                TcpStream::connect_timeout(&a, Duration::from_secs(2)).ok()
            })() else {
                continue; // the relay sees the link close and tries again
            };
            bridge(&core, relay, nonblocking(Line::Tcp(plane)));
        }
    })?;
    Ok(addr)
}

fn bridge(core: &Core, mut relay: TcpStream, mut line: Line) {
    let start = Instant::now();
    let mut handset = Handset::new();
    let mut buf = [0u8; 4096];
    loop {
        let now = start.elapsed().as_secs_f64();
        let pilot = core.lock().unwrap().pilot_channels();
        for (i, us) in pilot.iter().enumerate() {
            handset.channels[i] = *us;
        }
        match relay.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => handset.send_mavlink(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        let up = handset.to_line(now);
        if !up.is_empty() && !write_all(&mut line, &up) {
            return;
        }
        match read_some(&mut line, &mut buf) {
            None => return,
            Some(n) => handset.from_line(&buf[..n], now),
        }
        let down = handset.take_mavlink();
        if !down.is_empty() && !write_all(&mut Line::Tcp(relay.try_clone().unwrap()), &down) {
            return;
        }
        std::thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn read_for(s: &mut TcpStream, want: usize, secs: u64) -> Vec<u8> {
        s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let end = Instant::now() + Duration::from_secs(secs);
        let (mut got, mut buf) = (vec![], [0u8; 4096]);
        while got.len() < want && Instant::now() < end {
            if let Ok(n) = s.read(&mut buf) {
                got.extend_from_slice(&buf[..n]);
            }
        }
        got
    }

    /// The whole chain in the sim: the pilot's sticks leave a TX15 stand-in on a UART, the flight
    /// A TX15 on its USB cable is a serial device. This one is a pseudo-terminal, with a radio on
    /// the master side that does what the radio's script does: it sends the channels, reads the
    /// telemetry, and carries MAVLink for the ground station both ways.
    #[cfg(unix)]
    #[test]
    fn a_tx15_on_a_serial_device_flies_the_plane_and_carries_the_ground_station() {
        use aviation_crsf as crsf;
        use std::os::fd::FromRawFd;
        // SAFETY: openpty fills two descriptors and a name; they are ours to own.
        let (master, name) = unsafe {
            let (mut m, mut s) = (0, 0);
            let mut name = [0 as libc::c_char; 128];
            assert_eq!(libc::openpty(&mut m, &mut s, name.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut()), 0);
            libc::close(s);
            (std::fs::File::from_raw_fd(m), std::ffi::CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned())
        };
        let core: Core =
            Arc::new(Mutex::new(SimCore::new((simulation_aircraft::find("t1s-vtol").unwrap().params)(), vec![], true, 9002)));
        let home = Position::new(-35.363261, 149.16523, 584.0);
        let gs_addr = spawn(core.clone(), Config { line: name, autopilot: "127.0.0.1:1".into(), home }).unwrap().expect("a ground station port");

        let mut radio = master;
        let mut ground_station = TcpStream::connect(gs_addr).unwrap();
        ground_station.write_all(b"a command from the ground station").unwrap();
        // The radio sends its channels, and reads what comes down, for a while.
        let mut channels = [1500u16; 16];
        channels[0] = 1900;
        let (mut framer, mut seen, mut carried) = (crsf::Framer::default(), vec![], vec![]);
        let end = Instant::now() + Duration::from_secs(4);
        let mut buf = [0u8; 4096];
        while Instant::now() < end {
            radio.write_all(&crsf::rc_frame(&channels)).unwrap();
            if carried.is_empty() {
                // The radio carries what the ground station sent on to the plane, and telemetry up.
                radio.write_all(&crsf::envelopes(crsf::ADDR_RADIO, b"telemetry for the ground station").concat()).unwrap();
            }
            std::thread::sleep(Duration::from_millis(20));
            // SAFETY: a plain non-blocking read on a descriptor we own.
            let n = unsafe {
                let fl = libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&radio), libc::F_GETFL);
                libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&radio), libc::F_SETFL, fl | libc::O_NONBLOCK);
                libc::read(std::os::fd::AsRawFd::as_raw_fd(&radio), buf.as_mut_ptr() as *mut _, buf.len())
            };
            if n > 0 {
                for f in framer.push(&buf[..n as usize]) {
                    if f.kind == crsf::TYPE_MAVLINK_ENVELOPE && f.addr == crsf::ADDR_RADIO {
                        carried.extend_from_slice(crsf::envelope_data(&f.payload));
                    }
                    seen.push(f.kind);
                }
            }
            ground_station.set_read_timeout(Some(Duration::from_millis(1))).unwrap();
            let mut got = [0u8; 256];
            if let Ok(n) = ground_station.read(&mut got) {
                if got[..n].windows(16).any(|w| w == b"telemetry for th") {
                    assert!(seen.contains(&crsf::TYPE_ATTITUDE), "attitude came down the cable: {seen:?}");
                    assert!(seen.contains(&crsf::TYPE_BATTERY), "battery came down the cable");
                    assert_eq!(carried, b"a command from the ground station", "the ground station's command went down to the radio");
                    assert!(core.lock().unwrap().rc_channels()[0] >= 1890, "the roll stick went through the cable");
                    return;
                }
            }
        }
        panic!("the radio's telemetry for the ground station never came out (seen {seen:?}, carried {carried:?})");
    }


    /// computer gives them to ArduPilot as radio channels, ArduPilot's MAVLink comes back down the
    /// same UART to the "station", and the station's bytes reach ArduPilot.
    #[test]
    fn sticks_go_up_and_the_stations_telemetry_comes_down_the_uart() {
        let core: Core =
            Arc::new(Mutex::new(SimCore::new((simulation_aircraft::find("t1s-vtol").unwrap().params)(), vec![], true, 9002)));
        // ArduPilot's MAVLink port: a stand-in that sends a message and keeps what it gets.
        let ap = TcpListener::bind("127.0.0.1:0").unwrap();
        let ap_addr = ap.local_addr().unwrap().to_string();
        let received = Arc::new(Mutex::new(vec![]));
        let hello: Vec<u8> = (0..100u8).collect();
        {
            let (received, hello) = (received.clone(), hello.clone());
            std::thread::spawn(move || {
                let (mut s, _) = ap.accept().unwrap();
                s.write_all(&hello).unwrap();
                s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
                let mut buf = [0u8; 256];
                loop {
                    if let Ok(n) = s.read(&mut buf) {
                        received.lock().unwrap().extend_from_slice(&buf[..n]);
                    }
                }
            });
        }
        let port = free_port();
        let home = Position::new(-35.363261, 149.16523, 584.0);
        spawn(core.clone(), Config { line: format!("tcp:{port}"), autopilot: ap_addr, home }).unwrap();
        let tx15_addr = tx15(core.clone(), &format!("tcp:127.0.0.1:{port}")).unwrap();
        // The station connects where the radio relay would.
        let mut station = TcpStream::connect(tx15_addr).unwrap();
        station.write_all(b"from the station").unwrap();

        core.lock().unwrap().sticks.roll = 1.0;
        let down = read_for(&mut station, 100, 5);
        assert_eq!(down, hello, "ArduPilot's bytes arrive whole through the line");
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end && !received.lock().unwrap().windows(16).any(|w| w == b"from the station") {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(received.lock().unwrap().windows(16).any(|w| w == b"from the station"), "the station's bytes reach ArduPilot");
        let c = core.lock().unwrap();
        let rc = c.rc_channels();
        assert!(rc[0] >= 1995, "the roll stick went through the UART: {rc:?}");
        assert!(c.uart_rc.is_some());
    }
}
