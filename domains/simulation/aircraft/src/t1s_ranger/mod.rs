//! The HEEWING T1-S Ranger: twin-motor V-tail foam plane, 790 mm span, 4S.

pub mod geom;

/// Servo outputs, as ArduPilot has them wired for this plane (SERVO1 to SERVO5).
pub const AILERON: usize = 0;
pub const VTAIL_LEFT: usize = 1;
pub const THROTTLE_LEFT: usize = 2;
pub const VTAIL_RIGHT: usize = 3;
pub const THROTTLE_RIGHT: usize = 4;

const MAX_AILERON: f32 = 16.0 * DEG;
const MAX_RUDDERVATOR: f32 = 25.0 * DEG;

/// Mix a conventional aileron, elevator (nose up +), rudder (nose right +) and a throttle per
/// motor into this plane's outputs: the V-tail's two ruddervators mix elevator and rudder.
pub fn mix(aileron: f32, elevator: f32, rudder: f32, throttle: [f32; 2]) -> ActuatorCommands {
    // Nose up: both trailing edges up. Yaw right: the right panel pushes the tail left
    // (TE down), the left panel trails up. Same handedness as ArduPilot's V-tail mixer.
    let mut c = ActuatorCommands::default();
    c.out[AILERON] = aileron.clamp(-1.0, 1.0);
    c.out[VTAIL_LEFT] = (-elevator - rudder).clamp(-1.0, 1.0);
    c.out[VTAIL_RIGHT] = (-elevator + rudder).clamp(-1.0, 1.0);
    c.out[THROTTLE_LEFT] = throttle[0];
    c.out[THROTTLE_RIGHT] = throttle[1];
    c
}

use crate::{ActuatorCommands, AeroSurface, AircraftParams, ContactPoint, MotorSpec, Output, OutputKind, SurfaceControl};
use geom::*;
use glam::{Quat, Vec3};
use std::f32::consts::PI;

pub fn params() -> AircraftParams {
    let mut surfaces = Vec::new();
    let wing_area = WING_HALF_SPAN * (WING_ROOT_CHORD + WING_TIP_CHORD);
    let wing_ar = (2.0 * WING_HALF_SPAN).powi(2) / wing_area;
    let tail_ar = (2.0 * TAIL_SPAN).powi(2) / (TAIL_SPAN * (TAIL_ROOT_CHORD + TAIL_TIP_CHORD));

    for side in [-1.0f32, 1.0] {
        let sidei = side as i8;
        let span_out = Vec3::new(side * WING_DIHEDRAL.cos(), WING_DIHEDRAL.sin(), 0.0);
        let normal = Vec3::new(-side * WING_DIHEDRAL.sin(), WING_DIHEDRAL.cos(), 0.0);
        // Inner (prop-washed) and outer (aileron) wing sections.
        for (s0, s1, control, wash) in [
            (0.0, AILERON_START, SurfaceControl::None, Some((if side < 0.0 { 0 } else { 1 }, 0.45))),
            (AILERON_START, WING_HALF_SPAN, SurfaceControl::Output { channel: AILERON, gain: -(sidei as f32) * MAX_AILERON }, None),
        ] {
            let sm = 0.5 * (s0 + s1);
            let c = wing_chord(sm);
            let area = (s1 - s0) * 0.5 * (wing_chord(s0) + wing_chord(s1));
            let pos = Vec3::new(0.0, WING_Y, wing_le_z(sm) + 0.25 * c) + span_out * sm;
            surfaces.push(make_surface(
                if side < 0.0 { "wing_l" } else { "wing_r" },
                pos,
                normal,
                WING_INCIDENCE,
                area,
                c,
                wing_ar,
                if matches!(control, SurfaceControl::None) { 0.0 } else { AILERON_CHORD_FRAC },
                control,
                0.22,
                -0.045,
                0.022,
                14.0 * DEG,
                -11.0 * DEG,
                wash,
                false,
            ));
        }
    }

    for side in [-1.0f32, 1.0] {
        let span_out = Vec3::new(side * TAIL_DIHEDRAL.cos(), TAIL_DIHEDRAL.sin(), 0.0);
        let normal = Vec3::new(-side * TAIL_DIHEDRAL.sin(), TAIL_DIHEDRAL.cos(), 0.0);
        let sm = 0.5 * TAIL_SPAN;
        let c = tail_chord(sm);
        let area = TAIL_SPAN * 0.5 * (TAIL_ROOT_CHORD + TAIL_TIP_CHORD);
        let pos = Vec3::new(side * TAIL_ROOT_X, TAIL_Y, tail_le_z(sm) + 0.25 * c) + span_out * sm;
        surfaces.push(make_surface(
            if side < 0.0 { "vtail_l" } else { "vtail_r" },
            pos,
            normal,
            TAIL_INCIDENCE,
            area,
            c,
            tail_ar,
            RUDDERVATOR_CHORD_FRAC,
            SurfaceControl::Output { channel: if side < 0.0 { VTAIL_LEFT } else { VTAIL_RIGHT }, gain: MAX_RUDDERVATOR },
            0.0,
            0.0,
            0.025,
            15.0 * DEG,
            -15.0 * DEG,
            None,
            true,
        ));
    }

    let prop_disk = PI * PROP_RADIUS * PROP_RADIUS;
    let motor = |x: f32, channel: usize| MotorSpec {
        pos: Vec3::new(x, MOTOR_Y, MOTOR_Z),
        dir: Vec3::NEG_Z,
        // FX1406-3150KV on 4S with 3830 props: roughly 480-520 g static each.
        static_thrust: 4.9,
        // 3150 KV * 14.8 V * ~0.8 loaded * 3.0 in pitch.
        pitch_speed: 47.0,
        max_current: 17.0,
        spool_tau: 0.07,
        disk_area: prop_disk,
        channel,
        forward: true,
        tilt: None,
        lift: false,
        // The two props turn opposite ways, so their reaction torques cancel: not modelled.
        reaction: 0.0,
        // ~33,600 rpm on a 3.8 inch prop.
        tip_speed: 170.0,
    };

    let contacts = vec![
        ContactPoint { pos: Vec3::new(0.0, -0.035, NOSE_Z + 0.01), prop: None },
        ContactPoint { pos: Vec3::new(0.0, -0.048, -0.12), prop: None },
        ContactPoint { pos: Vec3::new(0.0, -0.045, 0.06), prop: None },
        ContactPoint { pos: Vec3::new(0.0, -0.005, TAIL_END_Z), prop: None },
        ContactPoint { pos: Vec3::new(-WING_HALF_SPAN, WING_Y + WING_HALF_SPAN * WING_DIHEDRAL.sin(), wing_le_z(WING_HALF_SPAN) + 0.05), prop: None },
        ContactPoint { pos: Vec3::new(WING_HALF_SPAN, WING_Y + WING_HALF_SPAN * WING_DIHEDRAL.sin(), wing_le_z(WING_HALF_SPAN) + 0.05), prop: None },
        ContactPoint { pos: Vec3::new(-0.15, 0.135, 0.40), prop: None },
        ContactPoint { pos: Vec3::new(0.15, 0.135, 0.40), prop: None },
        ContactPoint { pos: Vec3::new(-MOTOR_X, MOTOR_Y - PROP_RADIUS, MOTOR_Z - 0.01), prop: Some(0) },
        ContactPoint { pos: Vec3::new(MOTOR_X, MOTOR_Y - PROP_RADIUS, MOTOR_Z - 0.01), prop: Some(1) },
        ContactPoint { pos: Vec3::new(0.0, 0.05, -0.10), prop: None },
    ];

    AircraftParams {
        mass: 0.80,
        inertia: Vec3::new(0.016, 0.030, 0.014),
        surfaces,
        motors: vec![motor(-MOTOR_X, THROTTLE_LEFT), motor(MOTOR_X, THROTTLE_RIGHT)],
        outputs: vec![
            Output { kind: OutputKind::Servo, invert: false },
            // ArduPilot's V-tail mixer drives non-reversed servos "trailing edge up" for positive output.
            Output { kind: OutputKind::Servo, invert: true },
            Output { kind: OutputKind::Motor, invert: false },
            Output { kind: OutputKind::Servo, invert: true },
            Output { kind: OutputKind::Motor, invert: false },
        ],
        fuselage_cda: Vec3::new(0.018, 0.026, 0.0042),
        contacts,
        servo_rate: 600.0 * DEG,
        battery_cells: 4.0,
        battery_capacity_mah: 1800.0,
        battery_resistance: 0.055,
        start: crate::StartStyle::Hand,
        // The plain PNP plane has a receiver and two ESCs.
        flight_controller: false,
        rc_modes: vec![],
    }
}

#[allow(clippy::too_many_arguments)]
fn make_surface(
    name: &'static str,
    pos: Vec3,
    normal: Vec3,
    incidence: f32,
    area: f32,
    chord: f32,
    aspect_ratio: f32,
    flap_fraction: f32,
    control: SurfaceControl,
    cl0: f32,
    cm0: f32,
    cd0: f32,
    stall_pos: f32,
    stall_neg: f32,
    prop_wash: Option<(usize, f32)>,
    is_tail: bool,
) -> AeroSurface {
    let forward = Vec3::NEG_Z;
    let span_dir = forward.cross(normal).normalize();
    // Positive incidence pitches the surface nose-up about its span axis.
    let inc = Quat::from_axis_angle(span_dir, incidence);
    AeroSurface {
        name,
        pos,
        forward: inc * forward,
        normal: inc * normal,
        span_dir,
        area,
        chord,
        aspect_ratio,
        flap_fraction,
        control,
        cl0,
        cm0,
        cd0,
        stall_pos,
        stall_neg,
        prop_wash,
        is_tail,
    }
}

