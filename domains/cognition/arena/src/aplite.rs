//! ap-lite: a small model of what ArduPlane does with the executor's demand in AGENT mode, as in
//! FBWA. The roll and pitch controllers turn an attitude error into a rate, and the rate error
//! into a surface (with more surface when slow, as ArduPilot scales its gains by airspeed). An
//! integrator on each finds the trim. The throttle goes straight through; a negative throttle
//! (the pilot's) is held at a cruise setting.
//!
//! It is not ArduPilot. sim2real-tag.md calibrates it against SITL step responses later; here it
//! only has to fly the demand about as briskly as the T1-S's tuned controllers do.

use cognition_plan::Demand;
use recreation_pilot::Controls;
use simulation_physics::{FlightState, body_rates, euler};

/// The throttle when the demand leaves it to the pilot.
const PILOT_THROTTLE: f32 = 0.5;
const GRAVITY: f32 = 9.81;
/// The gains are for this airspeed (m/s), and scale with its square.
const SCALE_SPEED: f32 = 15.0;
/// Roll: the time constant of the attitude loop (s, `RLL2SRV_TCONST`), the most rate (rad/s), the
/// rate gain, the feed-forward and the integrator.
const ROLL_TAU: f32 = 0.4;
const ROLL_RATE_MAX: f32 = 1.6;
const ROLL_RATE_P: f32 = 0.35;
const ROLL_FF: f32 = 0.25;
const ROLL_I: f32 = 0.3;
/// Pitch: the same.
const PITCH_TAU: f32 = 0.5;
const PITCH_RATE_MAX: f32 = 1.0;
const PITCH_RATE_P: f32 = 0.6;
const PITCH_FF: f32 = 0.3;
const PITCH_I: f32 = 0.6;
/// The most each integrator may hold (of full surface).
const I_MAX: f32 = 0.4;

#[derive(Clone, Copy, Debug, Default)]
pub struct ApLite {
    roll_i: f32,
    pitch_i: f32,
}

impl ApLite {
    pub fn fly(&mut self, s: &FlightState, d: &Demand, dt: f32) -> Controls {
        let (roll, pitch, _) = euler(s.rot);
        let (p, q, _) = body_rates(s.omega);
        let v = s.airspeed.max(8.0);
        // Surfaces bite harder the faster the plane goes.
        let scale = (SCALE_SPEED / v).powi(2).clamp(0.25, 2.5);
        let roll_d = (d.roll_cd as f32 * 0.01).to_radians();
        let pitch_d = (d.pitch_cd as f32 * 0.01).to_radians();

        let roll_err = roll_d - roll;
        let p_d = (roll_err / ROLL_TAU).clamp(-ROLL_RATE_MAX, ROLL_RATE_MAX);
        self.roll_i = (self.roll_i + ROLL_I * roll_err * dt).clamp(-I_MAX, I_MAX);
        let aileron = (ROLL_RATE_P * (p_d - p) + ROLL_FF * p_d) * scale + self.roll_i;

        // In a turn the nose must come round about the pitch axis: feed that rate forward.
        let turn = GRAVITY * roll.sin() * roll.tan().clamp(-3.0, 3.0) / v;
        let pitch_err = pitch_d - pitch;
        let q_d = (pitch_err / PITCH_TAU).clamp(-PITCH_RATE_MAX, PITCH_RATE_MAX) + turn;
        self.pitch_i = (self.pitch_i + PITCH_I * pitch_err * dt).clamp(-I_MAX, I_MAX);
        let elevator = (PITCH_RATE_P * (q_d - q) + PITCH_FF * q_d) * scale + self.pitch_i;

        let throttle = if d.throttle < 0.0 { PILOT_THROTTLE } else { d.throttle.clamp(0.0, 1.0) };
        Controls { aileron: aileron.clamp(-1.0, 1.0), elevator: elevator.clamp(-1.0, 1.0), rudder: 0.0, throttle }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use simulation_aircraft::t1s_ranger;
    use simulation_physics::{Environment, Flat, Phase, Start, step};

    const DT: f32 = 1.0 / 400.0;

    /// A Ranger in level flight at 18 m/s, 40 m up, heading north.
    fn flying() -> (simulation_aircraft::AircraftParams, FlightState) {
        let params = t1s_ranger::params();
        let mut s = FlightState::new(&params, &Start { pos: Vec3::new(0.0, 40.0, 0.0), heading: 0.0 });
        s.phase = Phase::Flying;
        s.vel = Vec3::new(0.0, 0.0, -18.0);
        s.airspeed = 18.0;
        s.on_ground = false;
        (params, s)
    }

    fn demand(roll_deg: f32, pitch_deg: f32, throttle: f32) -> Demand {
        Demand {
            roll_cd: (roll_deg * 100.0) as i32,
            pitch_cd: (pitch_deg * 100.0) as i32,
            throttle,
            ..Demand::default()
        }
    }

    /// Flies `d` for `secs`, and the roll and pitch (deg) over the last second.
    fn hold(d: Demand, secs: f32) -> (Vec<(f32, f32)>, FlightState) {
        let (params, mut s) = flying();
        let mut ap = ApLite::default();
        let mut last = vec![];
        let n = (secs / DT) as usize;
        for i in 0..n {
            let c = ap.fly(&s, &d, DT);
            step(&params, &mut s, &c.mix(), &Environment { wind: Vec3::ZERO }, &Flat, &[], DT);
            if i + (1.0 / DT) as usize >= n {
                let (r, p, _) = euler(s.rot);
                last.push((r.to_degrees(), p.to_degrees()));
            }
        }
        (last, s)
    }

    #[test]
    fn it_holds_a_bank_it_is_asked_for() {
        let (last, s) = hold(demand(40.0, 5.0, 0.7), 4.0);
        assert_eq!(s.phase, Phase::Flying);
        for (r, _) in last {
            assert!((r - 40.0).abs() < 5.0, "roll {r}");
        }
    }

    #[test]
    fn it_rolls_the_other_way_as_briskly() {
        let (last, _) = hold(demand(-40.0, 5.0, 0.7), 4.0);
        for (r, _) in last {
            assert!((r + 40.0).abs() < 5.0, "roll {r}");
        }
    }

    #[test]
    fn it_holds_a_pitch_up_and_down() {
        for want in [8.0, -5.0] {
            let (last, _) = hold(demand(0.0, want, 0.6), 4.0);
            for (r, p) in last {
                assert!((p - want).abs() < 3.0 && r.abs() < 5.0, "asked {want}: roll {r} pitch {p}");
            }
        }
    }
}
