//! A small closed-loop plane for testing the executor: shared by the unit tests (`src/tests.rs`)
//! and the acceptance tests (`tests/acceptance.rs`), so both fly the same model.

#![allow(dead_code)]

use cognition_plan::{Demand, State, Track};

/// The control step (s): ArduPlane's AGENT mode steps the executor at 50 Hz.
pub const DT: f32 = 0.02;
/// The executor's own gravity (`math::G`).
const G: f32 = 9.80665;

/// Centidegrees to radians.
pub fn rad(cd: i32) -> f32 {
    (cd as f32 / 100.0).to_radians()
}

pub fn track(id: u8, pos: [f32; 3], vel: [f32; 3]) -> Track {
    Track { id, valid: 1, pos, vel, age: 0.0 }
}

/// A coordinated-turn point mass: roll and pitch follow the demand with first-order lags, the
/// flight path follows the pitch, airspeed from thrust, drag (more in turns) and gravity. Flat
/// ground at home's height.
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub pos: [f32; 3],
    pub psi: f32,
    pub v: f32,
    pub roll: f32,
    pub pitch: f32,
    pub wind: [f32; 2],
    pub time: f32,
}

impl Plane {
    pub fn new(n: f32, e: f32, h: f32, psi: f32, v: f32) -> Plane {
        Plane { pos: [n, e, -h], psi, v, roll: 0.0, pitch: 0.0, wind: [0.0, 0.0], time: 0.0 }
    }

    pub fn vel(&self) -> [f32; 3] {
        let hz = self.v * self.pitch.cos();
        [hz * self.psi.cos() + self.wind[0], hz * self.psi.sin() + self.wind[1], -self.v * self.pitch.sin()]
    }

    pub fn h(&self) -> f32 {
        -self.pos[2]
    }

    pub fn state(&self) -> State {
        State {
            pos: self.pos,
            vel: self.vel(),
            roll: self.roll,
            pitch: self.pitch,
            yaw: self.psi,
            airspeed: self.v,
            agl: self.h(),
            dt: DT,
            time: self.time,
        }
    }

    pub fn as_track(&self, id: u8) -> Track {
        track(id, self.pos, self.vel())
    }

    /// Flies attitude and throttle demands (rad, 0..1) for one step.
    pub fn fly(&mut self, roll_d: f32, pitch_d: f32, throttle: f32) {
        let thr = if throttle < 0.0 { 0.45 } else { throttle };
        let dr = ((roll_d - self.roll) * DT / 0.3).clamp(-1.5 * DT, 1.5 * DT);
        self.roll += dr;
        self.pitch += (pitch_d - self.pitch) * DT / 0.4;
        let n = 1.0 / self.roll.cos();
        let dv = 6.0 * thr - 2.7 * (self.v / 18.0).powi(2) - 0.8 * (n * n - 1.0) - G * self.pitch.sin();
        self.v = (self.v + dv * DT).max(3.0);
        self.psi += G * self.roll.tan() / self.v * DT;
        let vel = self.vel();
        for (p, v) in self.pos.iter_mut().zip(vel) {
            *p += v * DT;
        }
        self.time += DT;
    }

    pub fn fly_demand(&mut self, d: &Demand) {
        self.fly(rad(d.roll_cd), rad(d.pitch_cd), d.throttle);
    }

    pub fn dist2(&self, p: [f32; 2]) -> f32 {
        ((self.pos[0] - p[0]).powi(2) + (self.pos[1] - p[1]).powi(2)).sqrt()
    }
}

pub fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// A scripted target: a constant-bank circle at a speed and height.
pub fn circling_target(n: f32, e: f32, h: f32, v: f32, bank: f32) -> impl FnMut() -> Plane {
    let mut p = Plane::new(n, e, h, 0.0, v);
    p.roll = bank;
    p.pitch = 0.0;
    move || {
        let v0 = p.v;
        p.fly(bank, 0.0, 0.45);
        p.v = v0;
        p.pos[2] = -h;
        p
    }
}
