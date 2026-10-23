//! The DeepSpace Seeker 3: a 3 inch, deadcat quadcopter. A 139 mm wheelbase, four 1505 4000KV
//! motors on 3 inch tri-blade props, 4S, and a flight controller (an F722); 175 g without the
//! pack, under 250 g with a 650 to 1100 mAh one. It has no wing: ArduPilot flies it as a
//! quadplane with a quad frame (`ardupilot/seeker3.parm`).
//!
//! The retailer gives the weight, wheelbase, motors and props; the arm layout (the front arms
//! shortened so the props stay out of the camera) and everything about the motors' thrust is an
//! estimate.
//!
//! Outputs (SERVO1 to SERVO4): Motor 1 (front right), Motor 2 (rear left), Motor 3 (front left),
//! Motor 4 (rear right), as ArduPilot's quad X has them.

use crate::{AircraftParams, ContactPoint, MotorSpec, Output, OutputKind, StartStyle};
use glam::Vec3;
use std::f32::consts::PI;

/// Half the width between the motors across the frame (m): 139 mm diagonal.
pub const ARM_X: f32 = 0.049;
/// Where the front and the rear motors are from the CG (m, -Z is forward). The real front arms are
/// shorter than the rear ones (a deadcat: the props stay out of the camera), but the CG is under
/// the middle of the four motors: ArduPilot's mixer takes equal thrust at the four for a level
/// hover, and with the CG anywhere else the quad has to lean against a steady pitch torque.
pub const FRONT_Z: f32 = -0.049;
pub const REAR_Z: f32 = 0.049;
pub const MOTOR_Y: f32 = 0.012;
pub const PROP_RADIUS: f32 = 0.0381;

/// (x, z, channel, the prop turns counter-clockwise from above, as ArduPilot's quad X has it)
pub const LAYOUT: [(f32, f32, usize, bool); 4] =
    [(ARM_X, FRONT_Z, 0, true), (-ARM_X, REAR_Z, 1, true), (-ARM_X, FRONT_Z, 2, false), (ARM_X, REAR_Z, 3, false)];

pub fn params() -> AircraftParams {
    let layout = LAYOUT;
    let motors = layout
        .iter()
        .map(|&(x, z, channel, ccw)| MotorSpec {
            pos: Vec3::new(x, MOTOR_Y, z),
            dir: Vec3::Y,
            // 1505 4000KV on 4S with 3 inch tri-blades: about 430 g static each.
            static_thrust: 4.2,
            // 4000 KV * 15.2 V * ~0.75 loaded * 3 in pitch.
            pitch_speed: 58.0,
            max_current: 20.0,
            spool_tau: 0.03,
            disk_area: PI * PROP_RADIUS * PROP_RADIUS,
            channel,
            forward: false,
            tilt: None,
            lift: true,
            reaction: if ccw { 0.008 } else { -0.008 },
            tip_speed: 180.0,
        })
        .collect();
    // Feet under the motors: it sits level on a pad.
    let contacts =
        layout.iter().map(|&(x, z, _, _)| ContactPoint { pos: Vec3::new(x, -0.022, z), prop: None }).collect();
    AircraftParams {
        // 175 g and a 4S pack: 2500 mAh, so that it flies a game of Towers (8 min) on one charge.
        mass: 0.28,
        inertia: Vec3::new(0.00040, 0.00070, 0.00045),
        surfaces: vec![],
        motors,
        outputs: vec![Output { kind: OutputKind::Motor, invert: false }; 4],
        // Flow along the body's X (side), Y (from above) and Z (head on).
        fuselage_cda: Vec3::new(0.005, 0.012, 0.003),
        contacts,
        servo_rate: 600.0 * crate::t1s_ranger::geom::DEG,
        battery_cells: 4.0,
        battery_hv: false,
        battery_capacity_mah: 2500.0,
        battery_resistance: 0.04,
        start: StartStyle::Ground { rest_height: 0.022 },
        flight_controller: true,
        // The mode channel: stabilised hover, position hold, guided, auto, the way home, land.
        rc_modes: vec!["QSTABILIZE", "QLOITER", "GUIDED", "AUTO", "QRTL", "QLAND"],
    }
}
