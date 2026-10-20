//! The HEEWING T1-S VTOL: the T1-S airframe with its two wing motors on tilt servos and a third
//! rotor in the tail, and a flight controller (an F405) in place of the plain plane's receiver
//! and ESCs. In a hover the wing motors point up and the three rotors hold the plane up; in
//! forward flight the wing motors point forward and the wing lifts it. ArduPilot flies it as a
//! tilt-rotor tri (`ardupilot/t1s-vtol.parm`).
//!
//! HEEWING doesn't publish the VTOL's motors, props, tilt mechanism or weight, so everything
//! that isn't the T1-S's is an estimate: change it to try other hardware.
//!
//! Outputs (SERVO1 to SERVO8): aileron, V-tail left, left motor (Motor 2), V-tail right, right
//! motor (Motor 1), rear motor (Motor 4), left tilt, right tilt.

use crate::t1s_ranger;
use crate::{AircraftParams, ContactPoint, MotorSpec, Output, OutputKind, StartStyle, TiltSpec};
use glam::Vec3;
use std::f32::consts::PI;

pub const DEG: f32 = PI / 180.0;

/// The motors, in `AircraftParams::motors` order.
pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;
pub const REAR: usize = 2;

/// The servo outputs.
pub const MOTOR_LEFT: usize = t1s_ranger::THROTTLE_LEFT;
pub const MOTOR_RIGHT: usize = t1s_ranger::THROTTLE_RIGHT;
pub const MOTOR_REAR: usize = 5;
pub const TILT_LEFT: usize = 6;
pub const TILT_RIGHT: usize = 7;

/// How far a wing motor turns from pointing forward to fully back: vertical, and a little past
/// it so the pair can swing apart to yaw the plane in a hover (ArduPilot's vectored yaw).
const TILT_RANGE: f32 = 105.0 * DEG;
/// The tilt servo output that holds the wing motors vertical.
pub const UP: f32 = -1.0 + 2.0 * (1.0 - (90.0 * DEG) / TILT_RANGE);
/// A servo that turns the motors through 90° in about a third of a second.
const TILT_RATE: f32 = 4.5;

const WING_MOTOR_RADIUS: f32 = 0.0635; // 5 inch props
/// ArduPilot's tri mixer puts the rear rotor twice as far behind the CG as the wing motors are ahead of it.
const REAR_Z: f32 = 0.24;

pub fn params() -> AircraftParams {
    let mut p = t1s_ranger::params();
    let tilt = |channel| TiltSpec { channel, axis: Vec3::X, range: TILT_RANGE, rest: 90.0 * DEG, rate: TILT_RATE };
    let wing_motor = |side: f32, channel: usize, tilt_channel: usize| MotorSpec {
        pos: Vec3::new(side * 0.165, 0.022, -0.12),
        dir: Vec3::NEG_Z,
        static_thrust: 8.0,
        pitch_speed: 38.0,
        max_current: 18.0,
        spool_tau: 0.06,
        disk_area: PI * WING_MOTOR_RADIUS * WING_MOTOR_RADIUS,
        channel,
        forward: true,
        tilt: Some(tilt(tilt_channel)),
        lift: true,
        // The two props turn opposite ways.
        reaction: -side * 0.012,
        tip_speed: 90.0,
    };
    let rear = MotorSpec {
        pos: Vec3::new(0.0, 0.05, REAR_Z),
        dir: Vec3::Y,
        static_thrust: 6.5,
        pitch_speed: 30.0,
        max_current: 15.0,
        spool_tau: 0.06,
        disk_area: PI * WING_MOTOR_RADIUS * WING_MOTOR_RADIUS,
        channel: MOTOR_REAR,
        forward: false,
        tilt: None,
        lift: true,
        reaction: -0.012,
        tip_speed: 85.0,
    };
    p.motors = vec![wing_motor(-1.0, MOTOR_LEFT, TILT_LEFT), wing_motor(1.0, MOTOR_RIGHT, TILT_RIGHT), rear];
    // The T1-S's five outputs (the ESC throttles are now Motor 2 and Motor 1), then the rear
    // motor and the tilt servos.
    p.outputs[MOTOR_LEFT] = Output { kind: OutputKind::Motor, invert: false };
    p.outputs[MOTOR_RIGHT] = Output { kind: OutputKind::Motor, invert: false };
    p.outputs.push(Output { kind: OutputKind::Motor, invert: false });
    // Both tilt servos: minimum output is fully back, maximum is forward.
    p.outputs.push(Output { kind: OutputKind::Servo, invert: false });
    p.outputs.push(Output { kind: OutputKind::Servo, invert: false });

    // Landing legs, so it sits level on a pad: two forward, two aft.
    for (x, z) in [(-0.10, -0.10), (0.10, -0.10), (-0.07, 0.28), (0.07, 0.28)] {
        p.contacts.push(ContactPoint { pos: Vec3::new(x, -0.07, z), prop: None });
    }

    // Weight: the rear motor and its ESC, the tilt servos and brackets, the flight controller,
    // and a bigger pack. The rear motor and its moment arm add to the inertia.
    p.mass = 1.10;
    p.inertia = Vec3::new(0.0226, 0.0376, 0.0170);
    p.fuselage_cda += Vec3::new(0.0, 0.004, 0.002);
    // HEEWING's Hunter 4S HV pack: 3850 mAh, 20C, 265 g, LiHV cells (3.85 V nominal, 4.35 V full).
    p.battery_capacity_mah = 3850.0;
    p.battery_hv = true;
    p.battery_resistance = 0.05;
    p.start = StartStyle::Ground { rest_height: 0.07 };
    p.flight_controller = true;
    // The radio's mode channel (CH8) has six positions: two hover modes (height hold, then
    // position hold), two forward flight modes (stabilized, then holding height and heading), a
    // mission, and the way home.
    p.rc_modes = vec!["QHOVER", "QLOITER", "FBWA", "CRUISE", "AUTO", "QRTL"];
    p
}
