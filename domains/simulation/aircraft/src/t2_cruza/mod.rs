//! The HEEWING T2 Cruza VTOL: a tilt-rotor tri like the T1-S VTOL, a good half again bigger. It
//! has a 1200 mm span and a 1110 mm body, three FX-3110 680KV motors on 1060 props (two on tilt
//! servos, one fixed at the tail) and a 6S pack (HEEWING suggests 7800 mAh HV), and takes off at
//! 2.0 to 3.5 kg.
//!
//! HEEWING doesn't publish the airframe's shape, so it is the T1-S VTOL's scaled up to the
//! published span (the same outputs, the same mixing, `ardupilot/t2-cruza.parm`). The thrust,
//! current and tip speed of the motors are estimated from the motor, the props and the cells.

use crate::{AircraftParams, t1s_ranger, t1s_vtol};
use std::f32::consts::PI;

/// The Cruza's span over the T1-S's (1200 mm / 790 mm).
pub const SCALE: f32 = 1200.0 / 790.0;

/// 1060 props: 10.6 inches across.
const PROP_RADIUS: f32 = 0.1346;

pub fn params() -> AircraftParams {
    let mut p = t1s_vtol::params();
    p.scale(SCALE);
    // 3 kg with the 6S 7800 mAh pack (the pack alone is about 1 kg).
    p.mass = 3.0;
    // Mass and size: moments of inertia go with the mass and the square of the size.
    let k = p.mass / t1s_vtol::params().mass * SCALE * SCALE;
    p.inertia = t1s_vtol::params().inertia * k;
    for (i, m) in p.motors.iter_mut().enumerate() {
        // FX-3110 680KV on 6S, 1060 props: about 2 kg static each; the tail motor a little less.
        m.static_thrust = if i == t1s_vtol::REAR { 18.0 } else { 21.0 };
        // 680 KV * 22.2 V * ~0.8 loaded * 6 in pitch.
        m.pitch_speed = 31.0;
        m.max_current = 40.0;
        m.spool_tau = 0.12;
        m.disk_area = PI * PROP_RADIUS * PROP_RADIUS;
        m.reaction = m.reaction.signum() * 0.02;
        m.tip_speed = 165.0;
        if let Some(t) = &mut m.tilt {
            t.rate = 3.0;
        }
    }
    p.battery_cells = 6.0;
    p.battery_hv = true;
    p.battery_capacity_mah = 7800.0;
    p.battery_resistance = 0.03;
    p.servo_rate = 400.0 * t1s_ranger::geom::DEG;
    p
}
