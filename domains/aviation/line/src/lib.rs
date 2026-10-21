//! The TX15's end of the serial line, as a driver.
//!
//! A [`Tx15`] is a radio on a [`Wire`]: the pilot's sticks and switches go in ([`Tx15::fly`]),
//! its screen and the MAVLink that comes down are read out. The wire is a TCP stream (the sim's
//! virtual UART), a serial device, or [`mock_pair`], an in-memory line whose other end a test
//! plays: the flight computer, or nothing at all.
//!
//! Time is passed in (seconds), as for the flight computer, so a test steps the radio itself and
//! the station runs it on a thread ([`Tx15::run`]).

mod hosted;
mod wire;

pub use hosted::{Hosted, Lamps};
pub use wire::{Mock, TcpWire, Wire, mock_pair};

use aviation_computer::{Handset, Screen};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the pilot does with the radio: sticks from -1 to 1 (throttle 0 down to 1 up), the arm
/// switch, and which of the six positions of the mode switch (channel 8) it is in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pilot {
    /// Right is positive.
    pub roll: f32,
    /// Stick back (nose up) is positive.
    pub pitch: f32,
    /// Right is positive.
    pub yaw: f32,
    /// 0 is all the way down, 1 all the way up.
    pub throttle: f32,
    pub arm: bool,
    pub mode: u8,
}

impl Pilot {
    /// A pilot who holds the radio with the sticks centred, the throttle down and the switches out.
    pub const AT_REST: Pilot = Pilot { roll: 0.0, pitch: 0.0, yaw: 0.0, throttle: 0.0, arm: false, mode: 0 };

    /// The microseconds the first eight channels show, as the TX15's model sends them: AETR, the arm
    /// switch on channel 5 (low or high) and the mode switch on channel 8.
    pub fn channels(&self) -> [u16; 8] {
        let stick = |x: f32| (1500.0 + 500.0 * x.clamp(-1.0, 1.0)).round() as u16;
        [
            stick(self.roll),
            stick(self.pitch),
            (1000.0 + 1000.0 * self.throttle.clamp(0.0, 1.0)).round() as u16,
            stick(self.yaw),
            if self.arm { 1900 } else { 1100 },
            1500,
            1500,
            simulation_aircraft::rc_mode_pwm(self.mode.min(5) as usize),
        ]
    }
}

/// A radio on a line.
pub struct Tx15 {
    handset: Handset,
    wire: Box<dyn Wire>,
    alive: bool,
}

impl Tx15 {
    pub fn new(wire: impl Wire + 'static) -> Tx15 {
        let mut handset = Handset::new();
        handset.channels[..8].copy_from_slice(&Pilot::AT_REST.channels());
        Tx15 { handset, wire: Box::new(wire), alive: true }
    }

    /// Move the sticks and switches.
    pub fn fly(&mut self, pilot: Pilot) {
        self.handset.channels[..8].copy_from_slice(&pilot.channels());
    }

    /// Plug the radio in, or unplug it: an unplugged radio says nothing, and the flight computer
    /// fails safe after its timeout.
    pub fn connect(&mut self, on: bool) {
        self.handset.set_connected(on);
    }

    /// Let this share (0 to 1) of the channel frames have a wrong checksum.
    pub fn corrupt(&mut self, share: f32) {
        self.handset.corrupt = share;
    }

    /// What the screen shows now.
    pub fn screen(&self) -> &Screen {
        &self.handset.screen
    }

    /// The uplink quality the screen shows, 0 to 100.
    pub fn quality(&self, now: f64) -> u8 {
        self.handset.quality(now)
    }

    /// MAVLink from a ground station, to go up to the plane.
    pub fn send_mavlink(&mut self, bytes: &[u8]) {
        self.handset.send_mavlink(bytes);
    }

    /// The MAVLink that has come down, for a ground station.
    pub fn take_mavlink(&mut self) -> Vec<u8> {
        self.handset.take_mavlink()
    }

    /// False once the other end has hung up.
    pub fn alive(&self) -> bool {
        self.alive
    }

    /// One turn of the radio at time `now`: what it has to say goes up the line, and what is
    /// waiting on it is read.
    pub fn step(&mut self, now: f64) {
        let up = self.handset.to_line(now);
        if !up.is_empty() && !self.wire.write_all(&up) {
            self.alive = false;
            return;
        }
        let mut buf = [0u8; 4096];
        match self.wire.read_some(&mut buf) {
            None => self.alive = false,
            Some(n) => self.handset.from_line(&buf[..n], now),
        }
    }

    /// Run the radio on a thread of its own until the line hangs up, stepping it every few
    /// milliseconds in real time. The radio stays reachable through the returned handle.
    pub fn run(self) -> Shared {
        let shared = Shared(Arc::new(Mutex::new(self)));
        let again = shared.clone();
        std::thread::Builder::new()
            .name("tx15".into())
            .spawn(move || {
                let start = Instant::now();
                loop {
                    {
                        let mut radio = again.0.lock().unwrap();
                        radio.step(start.elapsed().as_secs_f64());
                        if !radio.alive {
                            return;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
            })
            .expect("a thread for the radio");
        shared
    }
}

/// A radio that runs on its own thread.
#[derive(Clone)]
pub struct Shared(Arc<Mutex<Tx15>>);

impl Shared {
    pub fn with<R>(&self, f: impl FnOnce(&mut Tx15) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }
}
