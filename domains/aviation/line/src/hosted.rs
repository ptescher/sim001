//! A radio hosted by a process of its own (the ground station's tests): it finds the flight
//! computer's line and keeps the pilot's sticks when the line comes back, and a relay carries a
//! ground station's MAVLink through it.

use crate::{Pilot, Shared, TcpWire, Tx15};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// What the lamps along the top of the radio's screen show.
#[derive(Clone, Debug, PartialEq)]
pub struct Lamps {
    pub mode: String,
    pub armed: bool,
}

struct State {
    pilot: Pilot,
    plugged_in: bool,
    radio: Option<Shared>,
    started: Instant,
}

/// A radio that stays on its line. Cloning gives another handle to the same radio.
#[derive(Clone)]
pub struct Hosted(Arc<Mutex<State>>);

impl Hosted {
    /// Host a radio on the line at `uart` (`127.0.0.1:5751`: the sim's virtual UART, found as soon
    /// as it listens, and again if it starts over). Returns where a ground station connects to
    /// reach the plane through it.
    pub fn host(uart: &str) -> std::io::Result<(Hosted, SocketAddr)> {
        let relay = TcpListener::bind("127.0.0.1:0")?;
        relay.set_nonblocking(true)?;
        let addr = relay.local_addr()?;
        let state = Arc::new(Mutex::new(State {
            pilot: Pilot::AT_REST,
            plugged_in: true,
            radio: None,
            started: Instant::now(),
        }));
        let keeper = Arc::downgrade(&state);
        let uart = uart.to_string();
        std::thread::Builder::new().name("tx15-line".into()).spawn(move || keep_the_line(keeper, &uart))?;
        let relaying = Arc::downgrade(&state);
        std::thread::Builder::new()
            .name("tx15-relay".into())
            .spawn(move || relay_for_ground_station(relaying, relay))?;
        Ok((Hosted(state), addr))
    }

    /// Move the sticks and switches.
    pub fn fly(&self, pilot: Pilot) {
        let mut s = self.0.lock().unwrap();
        s.pilot = pilot;
        if let Some(r) = &s.radio {
            r.with(|r| r.fly(pilot));
        }
    }

    /// Plug the radio in, or unplug it.
    pub fn connect(&self, on: bool) {
        let mut s = self.0.lock().unwrap();
        s.plugged_in = on;
        if let Some(r) = &s.radio {
            r.with(|r| r.connect(on));
        }
    }

    /// The radio has a line to the flight computer.
    pub fn on_line(&self) -> bool {
        self.0.lock().unwrap().radio.as_ref().is_some_and(|r| r.with(|r| r.alive()))
    }

    /// What the lamps show: `NO LINK` until the flight computer has said something.
    pub fn lamps(&self) -> Lamps {
        let s = self.0.lock().unwrap();
        let live = s
            .radio
            .as_ref()
            .and_then(|r| r.with(|r| r.screen().last_frame.map(|_| (r.screen().mode.clone(), r.screen().armed))));
        match live {
            Some((mode, armed)) => Lamps { mode, armed },
            None => Lamps { mode: "NO LINK".into(), armed: false },
        }
    }

    /// The screen as a picture.
    pub fn svg(&self) -> String {
        let s = self.0.lock().unwrap();
        match &s.radio {
            Some(r) => r.with(|r| r.screen().to_svg()),
            None => aviation_computer::Screen::default().to_svg(),
        }
    }

    /// Seconds since the radio was hosted.
    pub fn now(&self) -> f64 {
        self.0.lock().unwrap().started.elapsed().as_secs_f64()
    }
}

/// Connect to the line, and again whenever it drops, for as long as the radio is wanted.
fn keep_the_line(radio: Weak<Mutex<State>>, uart: &str) {
    loop {
        let Some(state) = radio.upgrade() else { return };
        let wanted = {
            let s = state.lock().unwrap();
            !s.radio.as_ref().is_some_and(|r| r.with(|r| r.alive()))
        };
        if wanted && let Ok(wire) = TcpWire::connect(uart) {
            let mut tx15 = Tx15::new(wire);
            let mut s = state.lock().unwrap();
            tx15.fly(s.pilot);
            tx15.connect(s.plugged_in);
            s.radio = Some(tx15.run());
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Carry a ground station's MAVLink up through the radio, and what comes down back out.
fn relay_for_ground_station(radio: Weak<Mutex<State>>, listener: TcpListener) {
    loop {
        if radio.upgrade().is_none() {
            return;
        }
        let Ok((mut station, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        let _ = (station.set_nonblocking(true), station.set_nodelay(true));
        let mut buf = [0u8; 4096];
        loop {
            let Some(state) = radio.upgrade() else { return };
            match station.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(r) = &state.lock().unwrap().radio {
                        r.with(|r| r.send_mavlink(&buf[..n]));
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                Err(_) => break,
            }
            let down = state.lock().unwrap().radio.as_ref().map(|r| r.with(|r| r.take_mavlink())).unwrap_or_default();
            drop(state);
            if !down.is_empty() && station.write_all(&down).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
    }
}
