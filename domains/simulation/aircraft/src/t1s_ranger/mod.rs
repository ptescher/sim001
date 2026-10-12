//! The HEEWING T1-S Ranger: twin-motor V-tail foam plane, 790 mm span, 4S.

pub mod geom;

use crate::{AeroSurface, AircraftParams, ContactPoint, MotorSpec, SurfaceControl};
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
            (AILERON_START, WING_HALF_SPAN, SurfaceControl::Aileron(sidei), None),
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
            SurfaceControl::Ruddervator(side as i8),
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
    let motor = |x: f32| MotorSpec {
        pos: Vec3::new(x, MOTOR_Y, MOTOR_Z),
        dir: Vec3::NEG_Z,
        // FX1406-3150KV on 4S with 3830 props: roughly 480-520 g static each.
        static_thrust: 4.9,
        // 3150 KV * 14.8 V * ~0.8 loaded * 3.0 in pitch.
        pitch_speed: 47.0,
        max_current: 17.0,
        spool_tau: 0.07,
        disk_area: prop_disk,
    };

    let contacts = vec![
        ContactPoint { pos: Vec3::new(0.0, -0.035, NOSE_Z + 0.01), fragile: false },
        ContactPoint { pos: Vec3::new(0.0, -0.048, -0.12), fragile: false },
        ContactPoint { pos: Vec3::new(0.0, -0.045, 0.06), fragile: false },
        ContactPoint { pos: Vec3::new(0.0, -0.005, TAIL_END_Z), fragile: false },
        ContactPoint { pos: Vec3::new(-WING_HALF_SPAN, WING_Y + WING_HALF_SPAN * WING_DIHEDRAL.sin(), wing_le_z(WING_HALF_SPAN) + 0.05), fragile: false },
        ContactPoint { pos: Vec3::new(WING_HALF_SPAN, WING_Y + WING_HALF_SPAN * WING_DIHEDRAL.sin(), wing_le_z(WING_HALF_SPAN) + 0.05), fragile: false },
        ContactPoint { pos: Vec3::new(-0.15, 0.135, 0.40), fragile: false },
        ContactPoint { pos: Vec3::new(0.15, 0.135, 0.40), fragile: false },
        ContactPoint { pos: Vec3::new(-MOTOR_X, MOTOR_Y - PROP_RADIUS, MOTOR_Z - 0.01), fragile: true },
        ContactPoint { pos: Vec3::new(MOTOR_X, MOTOR_Y - PROP_RADIUS, MOTOR_Z - 0.01), fragile: true },
        ContactPoint { pos: Vec3::new(0.0, 0.05, -0.10), fragile: false },
    ];

    AircraftParams {
        mass: 0.80,
        inertia: Vec3::new(0.016, 0.030, 0.014),
        surfaces,
        motors: [motor(-MOTOR_X), motor(MOTOR_X)],
        fuselage_cda: Vec3::new(0.018, 0.026, 0.0042),
        contacts,
        max_aileron: 16.0 * DEG,
        max_ruddervator: 25.0 * DEG,
        servo_rate: 600.0 * DEG,
        battery_cells: 4.0,
        battery_capacity_mah: 1800.0,
        battery_resistance: 0.055,
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

