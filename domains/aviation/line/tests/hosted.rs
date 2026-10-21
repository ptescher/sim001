//! A radio hosted by a process of its own: it finds the flight computer's line, keeps the pilot's
//! sticks across a reconnect, and carries a ground station's MAVLink through a relay.

use aviation_computer::{FlightComputer, Vehicle};
use aviation_line::{Hosted, Pilot};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// The flight computer's end of a virtual UART, as the sim has it.
struct Uart {
    listener: TcpListener,
    fc: FlightComputer,
    line: Option<TcpStream>,
    started: Instant,
}

impl Uart {
    fn new() -> Uart {
        Uart { listener: TcpListener::bind("127.0.0.1:0").unwrap(), fc: FlightComputer::new(), line: None, started: Instant::now() }
    }

    fn addr(&self) -> String {
        self.listener.local_addr().unwrap().to_string()
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Wait for the radio to connect.
    fn accept(&mut self) {
        self.listener.set_nonblocking(true).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if let Ok((s, _)) = self.listener.accept() {
                s.set_nonblocking(true).unwrap();
                self.line = Some(s);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the radio never connected");
    }

    /// Read what the radio sent for a while, and answer with the state of a plane.
    fn run(&mut self, secs: f64) {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        let mut buf = [0u8; 4096];
        while Instant::now() < end {
            let now = self.now();
            if let Some(line) = &mut self.line {
                if let Ok(n) = line.read(&mut buf) {
                    self.fc.from_line(&buf[..n], now);
                }
                let down = self.fc.to_line(&Vehicle { mode: "FBWA".into(), armed: true, ..Default::default() }, now);
                let _ = line.write_all(&down);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn the_hosted_radio_finds_the_line_and_the_flight_computer_hears_the_pilot() {
    let mut uart = Uart::new();
    let (radio, _relay) = Hosted::host(&uart.addr()).unwrap();
    radio.fly(Pilot { throttle: 0.5, arm: true, ..Pilot::AT_REST });
    uart.accept();
    uart.run(0.5);
    let ch = uart.fc.channels(uart.now());
    assert_eq!(ch[2], 1500, "throttle");
    assert_eq!(ch[4], 1900, "arm");
    assert!(radio.on_line());
}

#[test]
fn the_pilot_is_kept_when_the_line_comes_back() {
    let mut uart = Uart::new();
    let (radio, _relay) = Hosted::host(&uart.addr()).unwrap();
    uart.accept();
    uart.run(0.2);
    radio.fly(Pilot { throttle: 0.75, mode: 3, ..Pilot::AT_REST });
    // The sim starts again: the line drops, and the radio finds it again.
    uart.line = None;
    uart.fc = FlightComputer::new();
    uart.accept();
    uart.run(0.5);
    let ch = uart.fc.channels(uart.now());
    assert_eq!(ch[2], 1750, "throttle");
    assert_eq!(ch[7], simulation_aircraft::rc_mode_pwm(3), "mode");
}

#[test]
fn the_screen_lamps_follow_the_plane() {
    let mut uart = Uart::new();
    let (radio, _relay) = Hosted::host(&uart.addr()).unwrap();
    assert_eq!(radio.lamps().mode, "NO LINK", "before there is a line");
    uart.accept();
    uart.run(1.5);
    let lamps = radio.lamps();
    assert_eq!((lamps.mode.as_str(), lamps.armed), ("FBWA", true));
    assert!(radio.svg().contains(">FBWA<"));
}

#[test]
fn a_ground_stations_mavlink_goes_up_through_the_relay() {
    let mut uart = Uart::new();
    let (radio, relay) = Hosted::host(&uart.addr()).unwrap();
    uart.accept();
    let mut station = TcpStream::connect(relay).unwrap();
    station.write_all(&[0xFD, 7, 7, 7]).unwrap();
    uart.run(0.5);
    assert_eq!(uart.fc.take_for_autopilot(), vec![0xFD, 7, 7, 7]);
    // And what comes down goes back out to the station.
    uart.fc.from_autopilot(&[0xFD, 0x09, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 3, 0, 0, 3, 0, 0]);
    uart.run(1.0);
    station.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut buf = [0u8; 256];
    assert!(station.read(&mut buf).unwrap_or(0) > 0, "nothing came down to the station");
    drop(radio);
}
