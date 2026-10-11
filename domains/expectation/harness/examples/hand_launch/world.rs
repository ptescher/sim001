//! A toy of the sim's hand launch, calibration wizard and flight modes, faithful to
//! src/sim.rs and src/input.rs where it matters: the throttle has to be held above 55 % for
//! 0.6 s before the plane is thrown, a throw takes 0.22 s, and while the calibration wizard
//! is open the radio's sticks and buttons don't reach the plane. Screenshots are SVG.

use expectation_harness::{Bindings, Progress, Screenshot};
use std::fmt::Write;

pub const MODES: [&str; 3] = ["MANUAL", "ANGLE", "ACRO"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Held,
    Launching { t: f64 },
    Flying,
    Crashed,
}

#[derive(Clone, Debug)]
pub struct World {
    pub phase: Phase,
    /// The throttle stick, in %.
    pub throttle: f64,
    pub mode: usize,
    pub wizard: bool,
    pub altitude: f64,
    pub time: f64,
    launch_hold: f64,
    /// A seeded defect for demos: the wizard lets the throttle through.
    pub wizard_leaks: bool,
}

impl World {
    pub fn new(wizard_leaks: bool) -> Self {
        World {
            phase: Phase::Held,
            throttle: 0.0,
            mode: 0,
            wizard: false,
            altitude: 0.0,
            time: 0.0,
            launch_hold: 0.0,
            wizard_leaks,
        }
    }

    pub fn tick(&mut self, dt: f64) {
        self.time += dt;
        // Radio sticks don't reach the sim while the wizard is open.
        let thr = if self.wizard && !self.wizard_leaks { 0.0 } else { self.throttle };
        match self.phase {
            Phase::Held if thr > 55.0 => {
                self.launch_hold += dt;
                if self.launch_hold > 0.6 {
                    self.phase = Phase::Launching { t: 0.0 };
                    self.launch_hold = 0.0;
                }
            }
            Phase::Held => self.launch_hold = 0.0,
            Phase::Launching { t } => {
                self.altitude = 6.0;
                self.phase = if t + dt >= 0.22 { Phase::Flying } else { Phase::Launching { t: t + dt } };
            }
            Phase::Flying => self.altitude = (self.altitude + dt * (thr / 100.0 * 24.0 - 6.0)).clamp(4.0, 60.0),
            Phase::Crashed => {}
        }
    }

    pub fn launch(&mut self) {
        if !self.wizard && self.phase == Phase::Held {
            self.phase = Phase::Launching { t: 0.0 };
        }
    }

    pub fn reset(&mut self) {
        self.phase = Phase::Held;
        self.launch_hold = 0.0;
        self.altitude = 0.0;
    }

    pub fn press_mode(&mut self) {
        if !self.wizard {
            self.mode = (self.mode + 1) % MODES.len();
        }
    }

    pub fn press_calibrate(&mut self) {
        self.wizard = !self.wizard;
    }

    pub fn hit_ground(&mut self) {
        if matches!(self.phase, Phase::Flying | Phase::Launching { .. }) {
            self.phase = Phase::Crashed;
            self.altitude = 0.0;
        }
    }

    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Held => "in hand",
            Phase::Launching { .. } => "launching",
            Phase::Flying => "flying",
            Phase::Crashed => "crashed",
        }
    }

    /// The scene and a HUD, roughly as the sim's line-of-sight view shows it.
    pub fn svg(&self) -> String {
        let mut s = String::new();
        let _ = write!(
            s,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Helvetica, Arial, sans-serif">
<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#4f86c9"/><stop offset="1" stop-color="#d4e6f5"/></linearGradient></defs>
<rect width="640" height="360" fill="url(#sky)"/>
<path d="M0 252 L60 200 L105 226 L185 148 L255 214 L330 168 L402 226 L468 156 L540 212 L598 182 L640 204 L640 272 L0 272 Z" fill="#7a8ca1"/>
<path d="M185 148 L204 166 L194 163 L186 170 L176 162 L168 164 Z M468 156 L486 172 L476 170 L468 176 L460 168 L452 171 Z" fill="#f3f6f9"/>
<path d="M0 262 Q160 250 320 258 T640 256 L640 360 L0 360 Z" fill="#4f7d3f"/>
<path d="M0 300 Q320 290 640 300 L640 360 L0 360 Z" fill="#6c9b52"/>
<rect x="96" y="292" width="34" height="10" rx="2" fill="#c9b27c"/>
"##
        );

        let (x, y, r) = match self.phase {
            Phase::Held => (150.0, 288.0, -6.0),
            Phase::Launching { t } => (180.0 + 200.0 * t, 284.0 - 40.0 * t, -12.0),
            Phase::Flying => (380.0, (250.0 - self.altitude * 3.5).max(70.0), -10.0),
            Phase::Crashed => (430.0, 304.0, 28.0),
        };
        if self.phase == Phase::Held {
            let _ =
                write!(s, r##"<ellipse cx="{x}" cy="{}" rx="13" ry="7" fill="#e3b08f" stroke="#9c6d52"/>"##, y + 9.0);
        }
        let _ = write!(
            s,
            r##"<g transform="translate({x} {y}) rotate({r})">
<path d="M-34 0 Q-30 -6 0 -6 L26 -3 Q33 0 26 3 L0 5 Q-30 5 -34 0 Z" fill="#f4f4f1" stroke="#2c2c2c" stroke-width="1.2"/>
<path d="M-10 -1 L12 -1 L18 2 L-14 2 Z" fill="#e5452c"/>
<path d="M-28 -3 L-41 -16 L-36 -16 L-23 -4 Z" fill="#e5452c" stroke="#2c2c2c" stroke-width="0.8"/>
<circle cx="4" cy="-7" r="3" fill="#2c2c2c"/>
</g>
"##
        );

        // HUD: mode and phase, throttle bar.
        let status = match self.phase {
            Phase::Held => "IN HAND",
            Phase::Launching { .. } => "THROWN",
            Phase::Flying => "FLYING",
            Phase::Crashed => "CRASHED",
        };
        let status_color = if self.phase == Phase::Crashed { "#ff6b5b" } else { "#ffffff" };
        let _ = write!(
            s,
            r##"<rect x="12" y="12" width="176" height="52" rx="8" fill="#000" fill-opacity="0.45"/>
<text x="24" y="34" font-size="13" fill="#b8c7d9">MODE</text><text x="72" y="34" font-size="15" font-weight="700" fill="#fff">{}</text>
<text x="24" y="55" font-size="13" fill="#b8c7d9">STATE</text><text x="72" y="55" font-size="15" font-weight="700" fill="{status_color}">{status}</text>
<rect x="18" y="92" width="16" height="160" rx="3" fill="#000" fill-opacity="0.35" stroke="#fff" stroke-opacity="0.7"/>
<rect x="18" y="{:.1}" width="16" height="{:.1}" rx="3" fill="#ffd34d"/>
<line x1="14" x2="38" y1="{:.1}" y2="{:.1}" stroke="#fff" stroke-dasharray="3 2"/>
<text x="44" y="252" font-size="12" fill="#fff">THR {:.0}%</text>
"##,
            MODES[self.mode],
            252.0 - 1.6 * self.throttle,
            1.6 * self.throttle,
            252.0 - 1.6 * 55.0,
            252.0 - 1.6 * 55.0,
            self.throttle
        );

        if self.wizard {
            let _ = write!(
                s,
                r##"<rect x="150" y="104" width="340" height="112" rx="10" fill="#101820" fill-opacity="0.82" stroke="#ffd34d"/>
<text x="320" y="136" font-size="16" font-weight="700" fill="#ffd34d" text-anchor="middle">RADIO CALIBRATION</text>
<text x="320" y="162" font-size="13" fill="#e8eef4" text-anchor="middle">Move each stick when asked</text>
<text x="320" y="182" font-size="13" fill="#e8eef4" text-anchor="middle">Sticks and buttons don't fly the plane</text>
<text x="320" y="202" font-size="12" fill="#9fb0c2" text-anchor="middle">F2 to finish</text>
"##
            );
        }
        if self.phase == Phase::Crashed {
            let _ = write!(
                s,
                r##"<text x="320" y="96" font-size="15" fill="#fff" text-anchor="middle" font-weight="700">Press R to pick it back up</text>"##
            );
        }
        let _ = write!(
            s,
            r##"<text x="628" y="350" font-size="11" fill="#fff" fill-opacity="0.8" text-anchor="end">t = {:.1} s</text></svg>"##,
            self.time
        );
        s
    }
}

/// The vocabulary bound to the world.
pub fn driver(wizard_leaks: bool) -> Bindings<World> {
    Bindings::new(World::new(wizard_leaks))
        .observe("the aircraft", |w| w.phase_name())
        .observe("the throttle", |w| w.throttle)
        .observe("the flight mode", |w| MODES[w.mode])
        .observe("the calibration wizard", |w| if w.wizard { "open" } else { "closed" })
        .set("the throttle", |w, v| w.throttle = v.as_f64().unwrap())
        .arrange("the aircraft", |w, v| match v.as_str() {
            Some("in hand") => w.reset(),
            Some("launching") => w.phase = Phase::Launching { t: 0.0 },
            Some("flying") => {
                w.phase = Phase::Flying;
                w.altitude = 30.0;
            }
            _ => w.phase = Phase::Crashed,
        })
        .arrange("the flight mode", |w, v| w.mode = MODES.iter().position(|m| Some(*m) == v.as_str()).unwrap())
        .arrange("the calibration wizard", |w, v| w.wizard = v.as_str() == Some("open"))
        .on("the pilot presses launch", World::launch)
        .on("the pilot presses reset", World::reset)
        .on("the pilot presses flight mode", World::press_mode)
        .on("the pilot presses calibrate", World::press_calibrate)
        .on("the aircraft hits the ground", World::hit_ground)
        .on_reset(|w| *w = World::new(w.wizard_leaks))
        // A second of sim time after each action, then 0.1 s per extra look.
        .on_settle(|w| (0..20).for_each(|_| w.tick(0.05)))
        .on_advance(|w| {
            w.tick(0.1);
            Progress::Advanced
        })
        .on_screenshot(|w| Screenshot::svg(w.svg()))
}
