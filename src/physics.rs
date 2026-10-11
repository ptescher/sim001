//! 6-DOF rigid-body flight model for the T1-S.
//!
//! Aerodynamics are computed per lifting surface (strip theory): each wing section and each
//! V-tail panel sees its own local airflow (including rotation, prop wash and wing downwash),
//! so stability, damping, adverse yaw, tip stalls and spins emerge from geometry rather than
//! from canned stability derivatives.

use crate::geom::*;
use crate::terrain;
use bevy::math::{Quat, Vec3};
use std::f32::consts::PI;

pub const RHO: f32 = 1.225;
pub const GRAVITY: f32 = 9.81;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfaceControl {
    None,
    /// Aileron on the given side (-1 left, +1 right).
    Aileron(i8),
    /// V-tail ruddervator on the given side (-1 left, +1 right).
    Ruddervator(i8),
}

#[derive(Clone, Debug)]
pub struct AeroSurface {
    pub name: &'static str,
    /// Aerodynamic centre (quarter chord) in body frame.
    pub pos: Vec3,
    pub forward: Vec3,
    pub normal: Vec3,
    pub span_dir: Vec3,
    pub area: f32,
    pub chord: f32,
    pub aspect_ratio: f32,
    pub flap_fraction: f32,
    pub control: SurfaceControl,
    pub cl0: f32,
    pub cm0: f32,
    pub cd0: f32,
    pub stall_pos: f32,
    pub stall_neg: f32,
    /// (motor index, fraction of the surface immersed in that prop's slipstream)
    pub prop_wash: Option<(usize, f32)>,
    pub is_tail: bool,
}

#[derive(Clone, Debug)]
pub struct MotorSpec {
    pub pos: Vec3,
    pub dir: Vec3,
    /// Static thrust at full throttle on a fresh pack (N).
    pub static_thrust: f32,
    /// Prop pitch speed at full throttle (m/s).
    pub pitch_speed: f32,
    /// Current at full throttle, static (A).
    pub max_current: f32,
    pub spool_tau: f32,
    pub disk_area: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ContactPoint {
    pub pos: Vec3,
    pub fragile: bool,
}

#[derive(Clone, Debug)]
pub struct AircraftParams {
    pub mass: f32,
    /// Principal moments of inertia about body X (pitch), Y (yaw), Z (roll).
    pub inertia: Vec3,
    pub surfaces: Vec<AeroSurface>,
    pub motors: [MotorSpec; 2],
    /// Fuselage drag area (Cd*A) for flow along body X, Y, Z.
    pub fuselage_cda: Vec3,
    pub contacts: Vec<ContactPoint>,
    pub max_aileron: f32,
    pub max_ruddervator: f32,
    pub servo_rate: f32,
    pub battery_cells: f32,
    pub battery_capacity_mah: f32,
    pub battery_resistance: f32,
}

/// Normalized actuator commands, after any mixing / flight controller.
/// Surfaces use "trailing edge down is positive" per surface; throttles are 0..1.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActuatorCommands {
    /// Roll right positive.
    pub aileron: f32,
    /// Ruddervator trailing-edge-down commands, -1..1, (left, right).
    pub vtail_left: f32,
    pub vtail_right: f32,
    pub throttle: [f32; 2],
}

impl ActuatorCommands {
    /// Mix conventional elevator (nose up +) and rudder (nose right +) into V-tail panels.
    pub fn from_controls(aileron: f32, elevator: f32, rudder: f32, throttle: [f32; 2]) -> Self {
        // Nose up: both trailing edges up. Yaw right: the right panel pushes the tail left
        // (TE down), the left panel trails up. Same handedness as ArduPilot's V-tail mixer.
        Self {
            aileron: aileron.clamp(-1.0, 1.0),
            vtail_left: (-elevator - rudder).clamp(-1.0, 1.0),
            vtail_right: (-elevator + rudder).clamp(-1.0, 1.0),
            throttle,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Obstacle {
    pub pos: Vec3,
    /// Horizontal radius of the solid part.
    pub radius: f32,
    /// Top height (world Y).
    pub top: f32,
    /// Bottom of the solid part (world Y).
    pub bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Held in the pilot's hand, waiting for launch.
    Held,
    /// Being thrown (short forward push).
    Launching,
    Flying,
    Crashed,
}

#[derive(Clone, Debug)]
pub struct FlightState {
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    /// Angular velocity in body frame (rad/s).
    pub omega: Vec3,
    pub spool: [f32; 2],
    pub thrust: [f32; 2],
    pub deflection: Vec<f32>,
    pub alpha: Vec<f32>,
    pub stalled: Vec<bool>,
    pub phase: Phase,
    pub crash_reason: String,
    pub launch_timer: f32,
    pub time: f64,
    pub airspeed: f32,
    pub wing_cl: f32,
    pub used_mah: f32,
    pub voltage: f32,
    pub current: f32,
    pub on_ground: bool,
    /// Specific force (accelerometer reading) in body frame, m/s^2.
    pub specific_force: Vec3,
    pub prop_angle: [f32; 2],
    /// Attitude the pilot's hand turns the plane to while it's held.
    pub hand_rot: Quat,
    /// After a reset, the plane is carried back to the hand along a smooth path rather than
    /// teleported, so an autopilot's IMU dead-reckons the same trip its GPS reports.
    pub carry: Option<Carry>,
}

/// A minimum-jerk trip from where a reset found the plane back to the pilot's hand, with a hop
/// over the trees. Gentle (about 2.5 g peak) so an autopilot's attitude estimators, which lean
/// on GPS-derived acceleration, keep up.
#[derive(Clone, Copy, Debug)]
pub struct Carry {
    pub from: Vec3,
    pub to: Vec3,
    pub dur: f32,
    pub t: f32,
    hop: f32,
}

impl Carry {
    const PEAK_ACCEL: f32 = 25.0;

    fn new(from: Vec3, to: Vec3) -> Option<Carry> {
        let d = from.distance(to);
        if d < 0.05 {
            return None;
        }
        let hop = (d * 0.15).min(30.0);
        // Peak of the min-jerk profile is 5.77 d / T^2; the hop adds a little.
        let dur = (5.77 * (d + hop) / Self::PEAK_ACCEL).sqrt().clamp(1.0, 25.0);
        Some(Carry { from, to, dur, t: 0.0, hop })
    }

    /// Position, velocity and acceleration at time `t`.
    pub fn at(&self, t: f32) -> (Vec3, Vec3, Vec3) {
        let u = (t / self.dur).clamp(0.0, 1.0);
        let (u2, u3, u4, u5) = (u * u, u * u * u, u * u * u * u, u * u * u * u * u);
        let d = self.to - self.from;
        let (t1, t2) = (self.dur, self.dur * self.dur);
        let s = 10.0 * u3 - 15.0 * u4 + 6.0 * u5;
        let ds = (30.0 * u2 - 60.0 * u3 + 30.0 * u4) / t1;
        let dds = (60.0 * u - 180.0 * u2 + 120.0 * u3) / t2;
        // Hop: 16 h u^2 (1-u)^2, zero height and climb rate at both ends.
        let h = 16.0 * self.hop * (u2 - 2.0 * u3 + u4);
        let dh = 16.0 * self.hop * (2.0 * u - 6.0 * u2 + 4.0 * u3) / t1;
        let ddh = 16.0 * self.hop * (2.0 - 12.0 * u + 12.0 * u2) / t2;
        (self.from + d * s + Vec3::Y * h, d * ds + Vec3::Y * dh, d * dds + Vec3::Y * ddh)
    }
}

pub fn t1s_params() -> AircraftParams {
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

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Section coefficients: returns (cl, cd, cm, stall_blend).
pub fn section_coefficients(s: &AeroSurface, alpha: f32, flap: f32) -> (f32, f32, f32, f32) {
    let ar = s.aspect_ratio;
    let cla = 2.0 * PI * ar / (ar + 2.0 * (ar + 4.0) / (ar + 2.0));

    let (dcl, tau) = if s.flap_fraction > 0.0 {
        let theta = (2.0 * s.flap_fraction - 1.0).acos();
        let tau = 1.0 - (theta - theta.sin()) / PI;
        let eta = 0.8 - 0.4 * smoothstep(10.0 * DEG, 50.0 * DEG, flap.abs());
        (cla * tau * eta * flap, tau)
    } else {
        (0.0, 0.0)
    };

    let cl_lin = cla * alpha + s.cl0 + dcl;
    let a_eff = cl_lin / cla;
    let stall = if a_eff >= 0.0 { s.stall_pos } else { -s.stall_neg };
    let blend = smoothstep(stall, stall + 7.0 * DEG, a_eff.abs());

    // Post-stall: flat plate with the flap acting as a camber change.
    let a_fp = alpha + 0.5 * tau * flap;
    let cl_fp = 0.95 * (2.0 * a_fp).sin();
    let cd_fp = s.cd0 + 1.25 * a_fp.sin().powi(2);

    let cd_lin = s.cd0 + cl_lin * cl_lin / (PI * ar * 0.8) + 0.25 * tau * flap.abs().sin().powi(2);

    let cl = cl_lin + (cl_fp - cl_lin) * blend;
    let cd = cd_lin + (cd_fp - cd_lin) * blend;
    let cm = s.cm0 * (1.0 - blend) - 0.25 * tau * flap * (1.0 - blend) * 0.5;
    (cl, cd, cm, blend)
}

impl FlightState {
    pub fn new(params: &AircraftParams) -> Self {
        let n = params.surfaces.len();
        let mut s = Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            omega: Vec3::ZERO,
            spool: [0.0; 2],
            thrust: [0.0; 2],
            deflection: vec![0.0; n],
            alpha: vec![0.0; n],
            stalled: vec![false; n],
            phase: Phase::Held,
            crash_reason: String::new(),
            launch_timer: 0.0,
            time: 0.0,
            airspeed: 0.0,
            wing_cl: 0.0,
            used_mah: 0.0,
            voltage: 16.8,
            current: 0.0,
            on_ground: false,
            specific_force: Vec3::Y * GRAVITY,
            prop_angle: [0.0, 1.3],
            hand_rot: Quat::IDENTITY,
            carry: None,
        };
        s.reset_to_hand();
        // Start already in the hand; later resets carry it there (see `step`).
        s.rot = s.hand_rot;
        if let Some(c) = s.carry.take() {
            s.pos = c.to;
        }
        s.specific_force = s.rot.inverse() * Vec3::Y * GRAVITY;
        s
    }

    /// Pilot holds the plane at shoulder height, pointing down the runway, slightly nose up.
    pub fn reset_to_hand(&mut self) {
        let p = terrain::pilot_pos();
        let hand = Vec3::new(p.x - 0.5, terrain::ground_height(p.x, p.z) + 1.6, p.z - 0.6);
        // Like the attitude below, the position isn't snapped either: an instant jump reads as a
        // GPS glitch to ArduPilot ("GPS and AHRS differ by ...").
        self.carry = Carry::new(self.pos, hand);
        if self.carry.is_none() {
            self.pos = hand;
        }
        // The attitude isn't snapped: while held the plane turns into `hand_rot` at a finite rate
        // with matching body rates. An instant jump never shows on ArduPilot's gyros, which
        // leaves its DCM and EKF attitudes disagreeing ("DCM Roll/Pitch inconsistent ... Wait or
        // reboot") and fails the compass check against the wrong attitude.
        self.hand_rot = Quat::from_rotation_y(terrain::launch_heading()) * Quat::from_rotation_x(8.0 * DEG);
        self.vel = Vec3::ZERO;
        self.omega = Vec3::ZERO;
        self.phase = Phase::Held;
        self.crash_reason.clear();
        self.launch_timer = 0.0;
        self.on_ground = false;
        self.spool = [0.0; 2];
        self.specific_force = self.rot.inverse() * Vec3::Y * GRAVITY;
    }

    /// Finish a reset instantly (no carry back, no turning into the hand).
    pub fn snap_to_hand(&mut self) {
        if let Some(c) = self.carry.take() {
            self.pos = c.to;
        }
        self.rot = self.hand_rot;
        self.vel = Vec3::ZERO;
        self.omega = Vec3::ZERO;
        self.specific_force = self.rot.inverse() * Vec3::Y * GRAVITY;
    }

    pub fn recharge(&mut self) {
        self.used_mah = 0.0;
    }

    pub fn launch(&mut self) {
        // Not while still being carried back to the hand.
        if self.phase == Phase::Held && self.carry.is_none() {
            self.phase = Phase::Launching;
            self.launch_timer = 0.22;
        }
    }
}

/// Open-circuit LiPo cell voltage vs state of charge (0..1).
fn cell_ocv(soc: f32) -> f32 {
    let soc = soc.clamp(0.0, 1.0);
    // Piecewise-linear approximation of a typical LiPo discharge curve.
    let pts = [(0.0, 3.30), (0.05, 3.55), (0.15, 3.68), (0.4, 3.78), (0.7, 3.92), (0.9, 4.08), (1.0, 4.20)];
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if soc <= b.0 {
            return a.1 + (b.1 - a.1) * (soc - a.0) / (b.0 - a.0);
        }
    }
    4.2
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Environment {
    /// Wind velocity at the aircraft (world frame, m/s).
    pub wind: Vec3,
}

/// Advance the simulation by `dt` seconds.
pub fn step(
    p: &AircraftParams,
    s: &mut FlightState,
    cmd: &ActuatorCommands,
    env: &Environment,
    obstacles: &[Obstacle],
    dt: f32,
) {
    s.time += dt as f64;

    // Servos (rate limited).
    for (i, surf) in p.surfaces.iter().enumerate() {
        let target = match surf.control {
            SurfaceControl::None => 0.0,
            // Roll right: right aileron trailing edge up, left down.
            SurfaceControl::Aileron(side) => -(side as f32) * cmd.aileron * p.max_aileron,
            SurfaceControl::Ruddervator(side) => {
                if side < 0 {
                    cmd.vtail_left * p.max_ruddervator
                } else {
                    cmd.vtail_right * p.max_ruddervator
                }
            }
        };
        let d = target - s.deflection[i];
        let max = p.servo_rate * dt;
        s.deflection[i] += d.clamp(-max, max);
    }

    // Battery.
    let soc = 1.0 - s.used_mah / p.battery_capacity_mah;
    let ocv = cell_ocv(soc) * p.battery_cells;
    let mut current = 0.25; // electronics, VTX, camera
    for m in 0..2 {
        let target = if soc > 0.0 { cmd.throttle[m].clamp(0.0, 1.0) } else { 0.0 };
        s.spool[m] += (target - s.spool[m]) * (dt / p.motors[m].spool_tau).min(1.0);
        current += p.motors[m].max_current * s.spool[m].powf(2.6);
    }
    let v_bat = (ocv - current * p.battery_resistance).max(0.0);
    s.voltage = v_bat;
    s.current = current;
    s.used_mah += current * dt * 1000.0 / 3600.0;
    let volt_scale = (v_bat / (4.2 * p.battery_cells)).powi(2);

    if s.phase == Phase::Crashed {
        s.specific_force = Vec3::ZERO;
        for m in 0..2 {
            s.spool[m] = 0.0;
        }
        return;
    }

    let inv_rot = s.rot.inverse();
    let v_body = inv_rot * s.vel;
    let wind_body = inv_rot * env.wind;
    let air_body = v_body - wind_body; // aircraft velocity relative to the air mass
    s.airspeed = air_body.length();

    let mut force = Vec3::ZERO; // body frame, excluding gravity
    let mut torque = Vec3::ZERO;

    // Motors and slipstream.
    let mut slipstream = [0.0f32; 2];
    for (m, spec) in p.motors.iter().enumerate() {
        let va = air_body.dot(spec.dir).max(0.0);
        let f = s.spool[m];
        let mut t = if f > 0.001 {
            spec.static_thrust * volt_scale * f * f * (1.0 - va / (spec.pitch_speed * f.max(0.05)))
        } else {
            0.0
        };
        // Windmilling drag when the prop is overrun.
        t = t.max(-0.08 * spec.static_thrust * (va / spec.pitch_speed).powi(2));
        s.thrust[m] = t;
        force += spec.dir * t;
        torque += spec.pos.cross(spec.dir * t);
        if t > 0.0 {
            let vi = 0.5 * (-va + (va * va + 2.0 * t / (RHO * spec.disk_area)).sqrt());
            slipstream[m] = 2.0 * vi;
        }
        let rps = f * 560.0; // ~33,600 rpm at full throttle
        s.prop_angle[m] = (s.prop_angle[m] + rps * 2.0 * PI * dt * if m == 0 { 1.0 } else { -1.0 }) % (2.0 * PI);
    }

    // Wing downwash at the tail, from last step's wing lift coefficient.
    let downwash = (2.0 * s.wing_cl / (PI * 5.4)).clamp(-0.3, 0.3) * 0.9;

    let mut wing_lift_sum = 0.0;
    let mut wing_area_sum = 0.0;
    for (i, surf) in p.surfaces.iter().enumerate() {
        let v_point = air_body + s.omega.cross(surf.pos);
        // Velocity of the air relative to the surface.
        let mut air = -v_point;
        if let Some((m, frac)) = surf.prop_wash {
            air += -p.motors[m].dir * slipstream[m] * frac;
        }
        if surf.is_tail {
            air += Vec3::NEG_Y * air.length() * downwash;
        }
        let air_plane = air - surf.span_dir * air.dot(surf.span_dir);
        let speed2 = air_plane.length_squared();
        if speed2 < 1e-6 {
            continue;
        }
        let speed = speed2.sqrt();
        let d = air_plane / speed;
        let alpha = d.dot(surf.normal).atan2(d.dot(-surf.forward));
        let (cl, cd, cm, blend) = section_coefficients(surf, alpha, s.deflection[i]);
        s.alpha[i] = alpha;
        s.stalled[i] = blend > 0.5;

        let q = 0.5 * RHO * speed2;
        let lift_dir = d.cross(surf.span_dir);
        let f = (lift_dir * cl + d * cd) * q * surf.area;
        force += f;
        torque += surf.pos.cross(f) + surf.span_dir * (cm * q * surf.area * surf.chord);

        if !surf.is_tail {
            wing_lift_sum += cl * surf.area;
            wing_area_sum += surf.area;
        }
    }
    if wing_area_sum > 0.0 {
        s.wing_cl = wing_lift_sum / wing_area_sum;
    }

    // Fuselage parasitic drag (anisotropic).
    let q = 0.5 * RHO * air_body.length();
    force += -air_body * p.fuselage_cda * q;
    // A little extra rotational damping from the fuselage and booms.
    torque += -s.omega * Vec3::new(0.0004, 0.0006, 0.0002) * (1.0 + s.airspeed * 0.05);

    // Ground contacts (computed in world frame).
    let mut world_force = s.rot * force;
    let mut world_torque_extra = Vec3::ZERO;
    s.on_ground = false;
    if s.phase != Phase::Held {
        let ground_clearance = s.pos.y - terrain::ground_height(s.pos.x, s.pos.z);
        if ground_clearance < 1.5 {
            for c in &p.contacts {
                let r = s.rot * c.pos;
                let wp = s.pos + r;
                let h = terrain::ground_height(wp.x, wp.z);
                let pen = h - wp.y;
                if pen <= 0.0 {
                    continue;
                }
                s.on_ground = true;
                let n = terrain::ground_normal(wp.x, wp.z);
                let v = s.vel + (s.rot * s.omega).cross(r);
                let vn = v.dot(n);
                let speed = v.length();
                if terrain::is_water(wp.x, wp.z) {
                    crash(s, "Splashdown in the lake");
                    return;
                }
                if -vn > 5.0 || (speed > 14.0 && -vn > 2.0) {
                    crash(s, &format!("Hard impact at {:.1} m/s", speed));
                    return;
                }
                if c.fragile && s.spool[if c.pos.x < 0.0 { 0 } else { 1 }] > 0.25 && speed > 4.0 {
                    // Prop strike: kills the motors, like the real thing would.
                    crash(s, "Prop strike");
                    return;
                }
                let fn_mag = (2500.0 * pen - 45.0 * vn).max(0.0);
                let vt = v - n * vn;
                let mu = if terrain::is_runway(wp.x, wp.z) { 0.30 } else { 0.55 };
                let ft = -vt / vt.length().max(0.3) * mu * fn_mag;
                let fc = n * fn_mag + ft;
                world_force += fc;
                world_torque_extra += r.cross(fc);
            }
        }

        // Trees and buildings.
        if ground_clearance < 40.0 {
            for o in obstacles {
                let dx = s.pos.x - o.pos.x;
                let dz = s.pos.z - o.pos.z;
                if s.pos.y < o.top && s.pos.y > o.bottom && dx * dx + dz * dz < o.radius * o.radius {
                    crash(s, "Hit an obstacle");
                    return;
                }
            }
        }
    }

    // Hand launch: a short, smooth throw so accelerometers see a realistic pulse.
    if s.phase == Phase::Launching {
        let dir = s.rot * Quat::from_rotation_x(10.0 * DEG) * Vec3::NEG_Z;
        world_force += dir * p.mass * 45.0 + Vec3::Y * p.mass * GRAVITY;
        s.launch_timer -= dt;
        if s.launch_timer <= 0.0 {
            s.phase = Phase::Flying;
        }
    }

    if s.phase == Phase::Held {
        // Carried back to the hand (after a reset), or held still.
        let mut accel = Vec3::ZERO;
        s.vel = Vec3::ZERO;
        if let Some(c) = &mut s.carry {
            c.t = (c.t + dt).min(c.dur);
            let (p, v, a) = c.at(c.t);
            s.pos = p;
            s.vel = v;
            accel = a;
            if c.t >= c.dur {
                s.pos = c.to;
                s.vel = Vec3::ZERO;
                accel = Vec3::ZERO;
                s.carry = None;
            }
        }
        // Turn into the hand attitude like a pilot picking the plane up (about 1 s from upside
        // down), reporting the body rates so an autopilot's gyros integrate the same motion.
        let mut delta = s.rot.inverse() * s.hand_rot;
        if delta.w < 0.0 {
            delta = -delta;
        }
        let (axis, angle) = delta.to_axis_angle();
        const PICKUP_RATE: f32 = 3.0; // rad/s
        if angle > 1e-4 {
            let step = angle.min(PICKUP_RATE * dt);
            s.omega = axis * (step / dt);
            s.rot = (s.rot * Quat::from_axis_angle(axis, step)).normalize();
        } else {
            s.omega = Vec3::ZERO;
            s.rot = s.hand_rot;
        }
        s.specific_force = s.rot.inverse() * (accel + Vec3::Y * GRAVITY);
        return;
    }

    // Integrate (semi-implicit Euler).
    let accel = world_force / p.mass + Vec3::NEG_Y * GRAVITY;
    s.specific_force = inv_rot * (world_force / p.mass);
    s.vel += accel * dt;
    s.pos += s.vel * dt;

    let body_torque = torque + inv_rot * world_torque_extra;
    let i = p.inertia;
    let w = s.omega;
    let gyro = w.cross(w * i);
    let alpha = (body_torque - gyro) / i;
    s.omega += alpha * dt;
    s.omega = s.omega.clamp_length_max(40.0);
    s.rot = (s.rot * Quat::from_scaled_axis(s.omega * dt)).normalize();

    // Settle when resting on the ground to avoid jitter.
    if s.on_ground && s.vel.length() < 0.05 && s.omega.length() < 0.05 && cmd.throttle[0] + cmd.throttle[1] < 0.05 {
        s.vel = Vec3::ZERO;
        s.omega = Vec3::ZERO;
    }
}

fn crash(s: &mut FlightState, reason: &str) {
    s.phase = Phase::Crashed;
    s.crash_reason = reason.to_string();
    s.vel = Vec3::ZERO;
    s.omega = Vec3::ZERO;
    s.spool = [0.0; 2];
}

/// Roll (right wing down +), pitch (nose up +) and heading (clockwise from north, i.e. -Z).
pub fn euler(rot: Quat) -> (f32, f32, f32) {
    let fwd = rot * Vec3::NEG_Z;
    let right = rot * Vec3::X;
    let up = rot * Vec3::Y;
    let pitch = fwd.y.clamp(-1.0, 1.0).asin();
    let roll = (-right.y).atan2(up.y);
    let heading = fwd.x.atan2(-fwd.z);
    (roll, pitch, heading)
}

/// Body rates: (roll rate right +, pitch rate up +, yaw rate right +).
pub fn body_rates(omega: Vec3) -> (f32, f32, f32) {
    (-omega.z, omega.x, -omega.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After a crash, reset turns the plane into the hand with body rates that integrate to the
    /// same attitude, as an autopilot's gyros would see it.
    #[test]
    fn reset_to_hand_is_gyro_consistent() {
        let p = t1s_params();
        let mut s = FlightState::new(&p);
        // Upside down and yawed, as after a crash.
        s.rot = Quat::from_rotation_z(3.0) * Quat::from_rotation_y(1.0);
        s.phase = Phase::Crashed;
        s.reset_to_hand();
        let cmd = ActuatorCommands { aileron: 0.0, vtail_left: 0.0, vtail_right: 0.0, throttle: [0.0; 2] };
        let env = Environment { wind: Vec3::ZERO };
        let dt = 1.0 / 1200.0;
        let mut gyro_att = s.rot;
        for _ in 0..(2.0 / dt) as usize {
            step(&p, &mut s, &cmd, &env, &[], dt);
            gyro_att = (gyro_att * Quat::from_scaled_axis(s.omega * dt)).normalize();
        }
        assert!(s.rot.angle_between(s.hand_rot) < 1e-3, "settled in the hand");
        assert!(gyro_att.angle_between(s.rot) < 0.01, "gyros track the motion: off by {:.3} rad", gyro_att.angle_between(s.rot));
        assert_eq!(s.omega, Vec3::ZERO);
    }

    /// After a flight, reset carries the plane back with accelerations that dead-reckon to the
    /// same place (an IMU and GPS agree), within the IMU's range, and blocks launch until then.
    #[test]
    fn reset_to_hand_is_imu_consistent() {
        let p = t1s_params();
        let mut s = FlightState::new(&p);
        let hand = s.pos;
        s.pos = hand + Vec3::new(600.0, 80.0, -900.0);
        s.phase = Phase::Crashed;
        s.reset_to_hand();
        assert!(s.carry.is_some());
        s.launch();
        assert_eq!(s.phase, Phase::Held, "no launch mid-carry");
        let cmd = ActuatorCommands { aileron: 0.0, vtail_left: 0.0, vtail_right: 0.0, throttle: [0.0; 2] };
        let env = Environment { wind: Vec3::ZERO };
        let dt = 1.0 / 1200.0;
        let (mut ins_pos, mut ins_vel) = (s.pos, Vec3::ZERO);
        let mut peak = 0.0f32;
        for _ in 0..(30.0 / dt) as usize {
            step(&p, &mut s, &cmd, &env, &[], dt);
            // What an INS integrates: specific force rotated to world, plus gravity.
            let a = s.rot * s.specific_force - Vec3::Y * GRAVITY;
            peak = peak.max(s.specific_force.length());
            ins_vel += a * dt;
            ins_pos += ins_vel * dt;
        }
        assert!(s.carry.is_none() && s.pos.distance(hand) < 1e-3, "arrived in the hand");
        assert!(ins_pos.distance(hand) < 2.0, "dead reckoning ends {:.2} m off", ins_pos.distance(hand));
        assert!(peak < 4.0 * GRAVITY, "peak specific force {:.0} m/s^2", peak);
    }
}
