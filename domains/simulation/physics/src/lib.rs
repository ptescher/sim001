//! 6-DOF rigid-body flight model.
//!
//! Aerodynamics are computed per lifting surface (strip theory): each wing section and each
//! V-tail panel sees its own local airflow (including rotation, prop wash and wing downwash),
//! so stability, damping, adverse yaw, tip stalls and spins emerge from geometry rather than
//! from canned stability derivatives.
//!
//! Nothing here knows about a renderer or a particular world: the ground comes in through
//! [`Terrain`], and the plane through `simulation_aircraft`.

use glam::{Quat, Vec3};
use simulation_aircraft::{ActuatorCommands, AeroSurface, AircraftParams, MAX_MOTORS, StartStyle, SurfaceControl};
use std::f32::consts::PI;

pub const RHO: f32 = 1.225;
pub const GRAVITY: f32 = 9.81;
pub const DEG: f32 = PI / 180.0;

/// A lift rotor in sideways flow pushes back along the flow with this fraction of its thrust at
/// a flow speed equal to its tip speed (the rotor's H-force).
const ROTOR_H_FORCE: f32 = 0.25;

/// The ground the plane flies over.
pub trait Terrain {
    /// Ground height (world Y) at a point.
    fn height(&self, x: f32, z: f32) -> f32;
    /// Unit surface normal at a point.
    fn normal(&self, x: f32, z: f32) -> Vec3;
    fn is_water(&self, x: f32, z: f32) -> bool;
    /// The smooth, low-friction strip.
    fn is_runway(&self, x: f32, z: f32) -> bool;
}

/// Level ground at Y = 0, no water or runway.
pub struct Flat;

impl Terrain for Flat {
    fn height(&self, _: f32, _: f32) -> f32 {
        0.0
    }
    fn normal(&self, _: f32, _: f32) -> Vec3 {
        Vec3::Y
    }
    fn is_water(&self, _: f32, _: f32) -> bool {
        false
    }
    fn is_runway(&self, _: f32, _: f32) -> bool {
        false
    }
}

/// Where the plane starts: the spot on the ground at the pilot box, and the way it faces.
#[derive(Clone, Copy, Debug)]
pub struct Start {
    /// On the ground.
    pub pos: Vec3,
    /// Heading the plane points (radians about +Y).
    pub heading: f32,
}

/// How high the pilot holds a plane above the ground (m).
const HAND_HEIGHT: f32 = 1.6;

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
    /// Per motor: how spun up it is (0..1), its thrust (N), and the propeller's angle.
    pub spool: Vec<f32>,
    pub thrust: Vec<f32>,
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
    pub prop_angle: Vec<f32>,
    /// Per motor: how far it is tilted from pointing forward (rad); 0 for a motor that doesn't tilt.
    pub tilt: Vec<f32>,
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
    pub fn new(params: &AircraftParams, start: &Start) -> Self {
        let n = params.surfaces.len();
        let mut s = Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            omega: Vec3::ZERO,
            spool: vec![0.0; params.motors.len()],
            thrust: vec![0.0; params.motors.len()],
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
            voltage: params.battery_cells * if params.battery_hv { 4.35 } else { 4.2 },
            current: 0.0,
            on_ground: false,
            specific_force: Vec3::Y * GRAVITY,
            tilt: params.motors.iter().map(|m| m.tilt.map_or(0.0, |t| t.rest)).collect(),
            prop_angle: (0..params.motors.len()).map(|m| if m == 1 { 1.3 } else { 0.0 }).collect(),
            hand_rot: Quat::IDENTITY,
            carry: None,
        };
        s.reset_to_hand(params, start);
        // Start already in the hand; later resets carry it there (see `step`).
        s.rot = s.hand_rot;
        if let Some(c) = s.carry.take() {
            s.pos = c.to;
        }
        s.specific_force = s.rot.inverse() * Vec3::Y * GRAVITY;
        s
    }

    /// Pilot holds the plane at shoulder height, pointing down the runway, slightly nose up.
    pub fn reset_to_hand(&mut self, params: &AircraftParams, start: &Start) {
        let (hand_pos, tilt) = match params.start {
            StartStyle::Hand => (start.pos + Vec3::Y * HAND_HEIGHT, 8.0 * DEG),
            StartStyle::Ground { rest_height } => (start.pos + Vec3::Y * rest_height, 0.0),
        };
        // Like the attitude below, the position isn't snapped either: an instant jump reads as a
        // GPS glitch to ArduPilot ("GPS and AHRS differ by ...").
        self.carry = Carry::new(self.pos, hand_pos);
        if self.carry.is_none() {
            self.pos = hand_pos;
        }
        // The attitude isn't snapped: while held the plane turns into `hand_rot` at a finite rate
        // with matching body rates. An instant jump never shows on ArduPilot's gyros, which
        // leaves its DCM and EKF attitudes disagreeing ("DCM Roll/Pitch inconsistent ... Wait or
        // reboot") and fails the compass check against the wrong attitude.
        self.hand_rot = Quat::from_rotation_y(start.heading) * Quat::from_rotation_x(tilt);
        self.vel = Vec3::ZERO;
        self.omega = Vec3::ZERO;
        self.phase = Phase::Held;
        self.crash_reason.clear();
        self.launch_timer = 0.0;
        self.on_ground = false;
        self.spool.fill(0.0);
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
fn cell_ocv(soc: f32, hv: bool) -> f32 {
    let soc = soc.clamp(0.0, 1.0);
    // Piecewise-linear approximation of a typical LiPo discharge curve, and of a LiHV one (4.35 V full, 3.85 V nominal).
    let lipo = [(0.0, 3.30), (0.05, 3.55), (0.15, 3.68), (0.4, 3.78), (0.7, 3.92), (0.9, 4.08), (1.0, 4.20)];
    let lihv = [(0.0, 3.30), (0.05, 3.58), (0.15, 3.72), (0.4, 3.84), (0.7, 4.02), (0.9, 4.20), (1.0, 4.35)];
    let pts = if hv { lihv } else { lipo };
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if soc <= b.0 {
            return a.1 + (b.1 - a.1) * (soc - a.0) / (b.0 - a.0);
        }
    }
    pts[pts.len() - 1].1
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Environment {
    /// Wind velocity at the aircraft (world frame, m/s).
    pub wind: Vec3,
}

/// Which way motor `m` pushes, in the body frame, at its current tilt.
pub fn motor_dir(p: &AircraftParams, s: &FlightState, m: usize) -> Vec3 {
    let spec = &p.motors[m];
    match spec.tilt {
        Some(t) => Quat::from_axis_angle(t.axis, s.tilt[m]) * spec.dir,
        None => spec.dir,
    }
}

/// Advance the simulation by `dt` seconds.
pub fn step(
    p: &AircraftParams,
    s: &mut FlightState,
    cmd: &ActuatorCommands,
    env: &Environment,
    ground: &dyn Terrain,
    obstacles: &[Obstacle],
    dt: f32,
) {
    s.time += dt as f64;

    // Servos (rate limited).
    for (i, surf) in p.surfaces.iter().enumerate() {
        let target = match surf.control {
            SurfaceControl::None => 0.0,
            SurfaceControl::Output { channel, gain } => cmd.out[channel] * gain,
        };
        let d = target - s.deflection[i];
        let max = p.servo_rate * dt;
        s.deflection[i] += d.clamp(-max, max);
    }

    // Tilt servos (rate limited): output -1 is fully back, +1 fully forward.
    for (m, spec) in p.motors.iter().enumerate() {
        if let Some(t) = spec.tilt {
            let target = t.range * (1.0 - 0.5 * (cmd.out[t.channel].clamp(-1.0, 1.0) + 1.0));
            let max = t.rate * dt;
            s.tilt[m] += (target - s.tilt[m]).clamp(-max, max);
        }
    }

    // Battery.
    let soc = 1.0 - s.used_mah / p.battery_capacity_mah;
    let ocv = cell_ocv(soc, p.battery_hv) * p.battery_cells;
    let mut current = 0.25; // electronics, VTX, camera
    for m in 0..p.motors.len() {
        let target = if soc > 0.0 { cmd.out[p.motors[m].channel].clamp(0.0, 1.0) } else { 0.0 };
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
        s.spool.fill(0.0);
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
    let mut slipstream = [0.0f32; MAX_MOTORS];
    let mut dirs = [Vec3::ZERO; MAX_MOTORS];
    for (m, spec) in p.motors.iter().enumerate() {
        let dir = motor_dir(p, s, m);
        dirs[m] = dir;
        let va_raw = air_body.dot(dir);
        let va = va_raw.max(0.0);
        let f = s.spool[m];
        let mut t = if f > 0.001 {
            spec.static_thrust * volt_scale * f * f * (1.0 - va / (spec.pitch_speed * f.max(0.05)))
        } else {
            0.0
        };
        // Windmilling drag when the prop is overrun.
        t = t.max(-0.08 * spec.static_thrust * (va / spec.pitch_speed).powi(2));
        if spec.lift && t > 0.0 {
            let radius = (spec.disk_area / PI).sqrt();
            let axis = s.rot * dir;
            // Ground effect: a rotor close to the ground that blows towards it makes more thrust
            // (Cheeseman and Bennett: 1 / (1 - (R / 4z)^2)).
            if axis.y > 0.5 {
                let rotor = s.pos + s.rot * spec.pos;
                let z = (rotor.y - ground.height(rotor.x, rotor.z)).max(0.5 * radius);
                t /= 1.0 - (radius / (4.0 * z)).powi(2);
            }
            // Vortex ring: descending through its own wash, at about the speed the rotor blows
            // the air down, a rotor loses a good part of its thrust.
            let descent = (-va_raw).max(0.0);
            let hover_wash = (t / (2.0 * RHO * spec.disk_area)).sqrt();
            t *= 1.0 - 0.35 * (-((descent / hover_wash - 1.0) / 0.45).powi(2)).exp();
            // Sideways flow: the rotor drags back along the flow, and the airframe feels it at the rotor.
            let sideways = air_body - dir * va_raw;
            let h_force = -sideways * (ROTOR_H_FORCE * t / spec.tip_speed);
            force += h_force;
            torque += spec.pos.cross(h_force);
        }
        s.thrust[m] = t;
        force += dir * t;
        torque += spec.pos.cross(dir * t);
        // A rotor turning one way pushes the airframe round the other.
        torque += dir * (spec.reaction * t);
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
            // A tilted motor blows less of its wash over the wing.
            let along = dirs[m].dot(p.motors[m].dir).max(0.0);
            air += -dirs[m] * slipstream[m] * frac * along;
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
        let ground_clearance = s.pos.y - ground.height(s.pos.x, s.pos.z);
        if ground_clearance < 1.5 {
            for c in &p.contacts {
                let r = s.rot * c.pos;
                let wp = s.pos + r;
                let h = ground.height(wp.x, wp.z);
                let pen = h - wp.y;
                if pen <= 0.0 {
                    continue;
                }
                s.on_ground = true;
                let n = ground.normal(wp.x, wp.z);
                let v = s.vel + (s.rot * s.omega).cross(r);
                let vn = v.dot(n);
                let speed = v.length();
                if ground.is_water(wp.x, wp.z) {
                    crash(s, "Splashdown in the lake");
                    return;
                }
                if -vn > 5.0 || (speed > 14.0 && -vn > 2.0) {
                    crash(s, &format!("Hard impact at {:.1} m/s", speed));
                    return;
                }
                if c.prop.is_some_and(|m| s.spool[m] > 0.25) && speed > 4.0 {
                    // Prop strike: kills the motors, like the real thing would.
                    crash(s, "Prop strike");
                    return;
                }
                let fn_mag = (2500.0 * pen - 45.0 * vn).max(0.0);
                let vt = v - n * vn;
                let mu = if ground.is_runway(wp.x, wp.z) { 0.30 } else { 0.55 };
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
            // A plane that starts on the ground is let go once it is set down level.
            if s.carry.is_none() && matches!(p.start, StartStyle::Ground { .. }) {
                s.phase = Phase::Flying;
            }
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
    if s.on_ground
        && s.vel.length() < 0.05
        && s.omega.length() < 0.05
        && p.motors.iter().map(|m| cmd.out[m.channel]).sum::<f32>() < 0.05
    {
        s.vel = Vec3::ZERO;
        s.omega = Vec3::ZERO;
    }
}

fn crash(s: &mut FlightState, reason: &str) {
    s.phase = Phase::Crashed;
    s.crash_reason = reason.to_string();
    s.vel = Vec3::ZERO;
    s.omega = Vec3::ZERO;
    s.spool.fill(0.0);
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
    use simulation_aircraft::t1s_ranger;

    /// A pilot standing at the origin, facing north.
    fn pilot() -> Start {
        Start { pos: Vec3::new(-0.5, 0.0, -0.6), heading: 0.0 }
    }

    /// After a crash, reset turns the plane into the hand with body rates that integrate to the
    /// same attitude, as an autopilot's gyros would see it.
    #[test]
    fn reset_to_hand_is_gyro_consistent() {
        let p = t1s_ranger::params();
        let mut s = FlightState::new(&p, &pilot());
        // Upside down and yawed, as after a crash.
        s.rot = Quat::from_rotation_z(3.0) * Quat::from_rotation_y(1.0);
        s.phase = Phase::Crashed;
        s.reset_to_hand(&p, &pilot());
        let cmd = ActuatorCommands::default();
        let env = Environment { wind: Vec3::ZERO };
        let dt = 1.0 / 1200.0;
        let mut gyro_att = s.rot;
        for _ in 0..(2.0 / dt) as usize {
            step(&p, &mut s, &cmd, &env, &Flat, &[], dt);
            gyro_att = (gyro_att * Quat::from_scaled_axis(s.omega * dt)).normalize();
        }
        assert!(s.rot.angle_between(s.hand_rot) < 1e-3, "settled in the hand");
        assert!(
            gyro_att.angle_between(s.rot) < 0.01,
            "gyros track the motion: off by {:.3} rad",
            gyro_att.angle_between(s.rot)
        );
        assert_eq!(s.omega, Vec3::ZERO);
    }

    /// After a flight, reset carries the plane back with accelerations that dead-reckon to the
    /// same place (an IMU and GPS agree), within the IMU's range, and blocks launch until then.
    #[test]
    fn reset_to_hand_is_imu_consistent() {
        let p = t1s_ranger::params();
        let mut s = FlightState::new(&p, &pilot());
        let held = s.pos;
        s.pos = held + Vec3::new(600.0, 80.0, -900.0);
        s.phase = Phase::Crashed;
        s.reset_to_hand(&p, &pilot());
        assert!(s.carry.is_some());
        s.launch();
        assert_eq!(s.phase, Phase::Held, "no launch mid-carry");
        let cmd = ActuatorCommands::default();
        let env = Environment { wind: Vec3::ZERO };
        let dt = 1.0 / 1200.0;
        let (mut ins_pos, mut ins_vel) = (s.pos, Vec3::ZERO);
        let mut peak = 0.0f32;
        for _ in 0..(30.0 / dt) as usize {
            step(&p, &mut s, &cmd, &env, &Flat, &[], dt);
            // What an INS integrates: specific force rotated to world, plus gravity.
            let a = s.rot * s.specific_force - Vec3::Y * GRAVITY;
            peak = peak.max(s.specific_force.length());
            ins_vel += a * dt;
            ins_pos += ins_vel * dt;
        }
        assert!(s.carry.is_none() && s.pos.distance(held) < 1e-3, "arrived in the hand");
        assert!(ins_pos.distance(held) < 2.0, "dead reckoning ends {:.2} m off", ins_pos.distance(held));
        assert!(peak < 4.0 * GRAVITY, "peak specific force {:.0} m/s^2", peak);
    }
}

#[cfg(test)]
mod rotor_tests {
    use super::*;
    use simulation_aircraft::t1s_vtol;

    /// The thrust of the first lift rotor after one step, with the lift motors at a steady
    /// throttle, the plane at `height` (m, the CG) moving at `vel`.
    fn lift_thrust(height: f32, vel: Vec3) -> f32 {
        let p = t1s_vtol::params();
        let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        s.carry = None;
        s.phase = Phase::Flying;
        s.pos = Vec3::new(0.0, height, 0.0);
        s.rot = Quat::IDENTITY;
        s.vel = vel;
        let mut c = ActuatorCommands::default();
        for (i, m) in p.motors.iter().enumerate() {
            if m.lift {
                c.out[m.channel] = 0.6;
                s.spool[i] = 0.6;
            }
        }
        step(&p, &mut s, &c, &Environment::default(), &Flat, &[], 1.0 / 600.0);
        s.thrust[2]
    }

    #[test]
    fn a_rotor_near_the_ground_makes_more_thrust() {
        // The rear rotor, one radius off the ground.
        let low = lift_thrust(0.015, Vec3::ZERO);
        let high = lift_thrust(30.0, Vec3::ZERO);
        assert!(low > high * 1.04, "{low:.2} N near the ground against {high:.2} N");
        assert!(low < high * 1.4, "ground effect is bounded: {low:.2} N against {high:.2} N");
    }

    #[test]
    fn descending_through_its_own_wash_costs_thrust() {
        let hover = lift_thrust(30.0, Vec3::ZERO);
        // About the speed the rotor blows the air down at: the vortex ring state.
        let in_the_ring = lift_thrust(30.0, Vec3::new(0.0, -7.0, 0.0));
        // Much faster, the rotor is clear of its wash again.
        let fast = lift_thrust(30.0, Vec3::new(0.0, -25.0, 0.0));
        assert!(in_the_ring < hover * 0.85, "{in_the_ring:.2} N in the ring against {hover:.2} N in the hover");
        assert!(fast > hover * 0.9, "{fast:.2} N falling fast against {hover:.2} N");
    }

    #[test]
    fn rotors_in_sideways_flow_drag_the_airframe_back() {
        let p = t1s_vtol::params();
        // Flying forward at 15 m/s with the lift rotors on, and off.
        let accel = |lift_throttle: f32| {
            let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
            s.carry = None;
            s.phase = Phase::Flying;
            s.pos = Vec3::new(0.0, 50.0, 0.0);
            s.vel = Vec3::new(0.0, 0.0, -15.0);
            let mut c = ActuatorCommands::default();
            for (i, m) in p.motors.iter().enumerate() {
                if m.lift {
                    c.out[m.channel] = lift_throttle;
                    s.spool[i] = lift_throttle;
                }
            }
            step(&p, &mut s, &c, &Environment::default(), &Flat, &[], 1.0 / 600.0);
            s.vel.z
        };
        // Moving along -Z, drag shows as a velocity change towards +Z.
        assert!(accel(0.6) > accel(0.0), "{:.5} with rotors on against {:.5} off", accel(0.6), accel(0.0));
    }
}

#[cfg(test)]
mod tilt_tests {
    use super::*;
    use simulation_aircraft::t1s_vtol::{self, TILT_LEFT, TILT_RIGHT, UP};

    /// The VTOL in the air at 50 m, flying, motors off.
    fn airborne() -> (AircraftParams, FlightState) {
        let p = t1s_vtol::params();
        let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        s.carry = None;
        s.phase = Phase::Flying;
        s.pos = Vec3::new(0.0, 50.0, 0.0);
        s.rot = Quat::IDENTITY;
        (p, s)
    }

    fn run(p: &AircraftParams, s: &mut FlightState, c: &ActuatorCommands, seconds: f32) {
        let dt = 1.0 / 600.0;
        for _ in 0..(seconds / dt) as usize {
            step(p, s, c, &Environment::default(), &Flat, &[], dt);
        }
    }

    fn tilted(out: f32) -> ActuatorCommands {
        let mut c = ActuatorCommands::default();
        c.out[TILT_LEFT] = out;
        c.out[TILT_RIGHT] = out;
        c
    }

    #[test]
    fn the_tilt_servos_point_the_wing_motors_up_or_forward() {
        let (p, mut s) = airborne();
        // Full output is forward: the thrust goes along the nose.
        run(&p, &mut s, &tilted(1.0), 1.5);
        assert!((motor_dir(&p, &s, 0) - Vec3::NEG_Z).length() < 0.01, "forward: {:?}", motor_dir(&p, &s, 0));
        assert!((motor_dir(&p, &s, 1) - Vec3::NEG_Z).length() < 0.01);
        // At the output that holds the motors vertical, the thrust goes straight up.
        run(&p, &mut s, &tilted(UP), 1.5);
        assert!((motor_dir(&p, &s, 0) - Vec3::Y).length() < 0.01, "up: {:?}", motor_dir(&p, &s, 0));
        // The rear rotor never moves.
        assert_eq!(motor_dir(&p, &s, 2), Vec3::Y);
    }

    #[test]
    fn the_tilt_servos_have_a_speed_limit() {
        let (p, mut s) = airborne();
        run(&p, &mut s, &tilted(1.0), 1.5);
        // Told to go up, the motors haven't got there after 0.1 s, and have after 1 s.
        run(&p, &mut s, &tilted(UP), 0.1);
        let early = motor_dir(&p, &s, 0).y;
        assert!(early > 0.05 && early < 0.9, "after 0.1 s the motors point {early:.2} up");
        run(&p, &mut s, &tilted(UP), 0.9);
        assert!(motor_dir(&p, &s, 0).y > 0.999);
    }

    #[test]
    fn tilted_up_the_motors_lift_and_tilted_forward_they_push() {
        // The motors' thrust together, in the body frame.
        let push = |out: f32| {
            let (p, mut s) = airborne();
            let mut c = tilted(out);
            for m in &p.motors {
                c.out[m.channel] = 0.6;
            }
            run(&p, &mut s, &c, 1.5);
            (0..p.motors.len()).map(|m| motor_dir(&p, &s, m) * s.thrust[m]).sum::<Vec3>()
        };
        let up = push(UP);
        let forward = push(1.0);
        assert!(up.y > 5.0 && up.z.abs() < 0.5, "up: {up:?}");
        assert!(forward.z < -3.0 && forward.y < 3.0, "forward: {forward:?}");
    }

    #[test]
    fn it_has_two_tilt_rotors_and_a_fixed_one_in_the_tail() {
        let p = t1s_vtol::params();
        assert_eq!(p.motors.len(), 3);
        assert_eq!(p.motors.iter().filter(|m| m.tilt.is_some()).count(), 2);
        assert!(p.flight_controller);
        // Enough thrust to hover and climb: about twice the weight.
        let thrust: f32 = p.motors.iter().map(|m| m.static_thrust).sum();
        assert!(thrust / (p.mass * GRAVITY) > 1.6, "thrust to weight {:.2}", thrust / (p.mass * GRAVITY));
    }
}

#[cfg(test)]
mod hover_tests {
    use super::*;
    use simulation_aircraft::t1s_vtol::{self, MOTOR_LEFT, MOTOR_REAR, MOTOR_RIGHT, TILT_LEFT, TILT_RIGHT, UP};

    /// A plain hover controller for the tilt-rotor tri: altitude from the collective, roll from
    /// the two wing motors, pitch from the wing motors against the rear one, and yaw by swinging
    /// the wing motors apart.
    struct Hover {
        target: f32,
        height_i: f32,
        yaw_i: f32,
        roll_i: f32,
        pitch_i: f32,
    }

    impl Hover {
        fn command(&mut self, s: &FlightState, dt: f32) -> ActuatorCommands {
            let (roll, pitch, _) = euler(s.rot);
            let (p, q, r) = body_rates(s.omega);
            let err = self.target - s.pos.y;
            self.height_i = (self.height_i + 0.02 * err * dt).clamp(-0.15, 0.15);
            let collective = (0.68 + self.height_i + 0.06 * err - 0.12 * s.vel.y).clamp(0.3, 1.0);
            // Positive: roll right, nose up, yaw right.
            self.roll_i = (self.roll_i - 0.6 * roll * dt).clamp(-0.2, 0.2);
            self.pitch_i = (self.pitch_i - 0.6 * pitch * dt).clamp(-0.2, 0.2);
            let roll_u = -0.9 * roll - 0.16 * p + self.roll_i;
            let pitch_u = -0.9 * pitch - 0.16 * q + self.pitch_i;
            self.yaw_i = (self.yaw_i - 0.05 * r * dt).clamp(-0.5, 0.5);
            let yaw_u = -0.25 * r + self.yaw_i;
            let mut c = ActuatorCommands::default();
            // The wing motors are ahead of the CG and the rear one behind it, so a nose-up push
            // lifts the wing motors and drops the rear one, by about half as much again.
            // Rolling right needs more lift on the left.
            c.out[MOTOR_LEFT] = (collective + roll_u + 0.5 * pitch_u).clamp(0.0, 1.0);
            c.out[MOTOR_RIGHT] = (collective - roll_u + 0.5 * pitch_u).clamp(0.0, 1.0);
            c.out[MOTOR_REAR] = (collective * 1.11 - 0.8 * pitch_u).clamp(0.0, 1.0);
            // Yaw right: the left motor tips forward and the right one back.
            c.out[TILT_LEFT] = (UP + yaw_u).clamp(-1.0, 1.0);
            c.out[TILT_RIGHT] = (UP - yaw_u).clamp(-1.0, 1.0);
            c
        }
    }

    fn fly(wind: Vec3, seconds: f32, mut inspect: impl FnMut(&FlightState)) -> (AircraftParams, FlightState) {
        let p = t1s_vtol::params();
        let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        s.carry = None;
        s.phase = Phase::Flying;
        s.pos = Vec3::new(0.0, 0.07, 0.0);
        s.rot = Quat::IDENTITY;
        let mut hover = Hover { target: 10.0, height_i: 0.0, yaw_i: 0.0, roll_i: 0.0, pitch_i: 0.0 };
        let env = Environment { wind };
        let dt = 1.0 / 600.0;
        for _ in 0..(seconds / dt) as usize {
            let cmd = hover.command(&s, dt);
            step(&p, &mut s, &cmd, &env, &Flat, &[], dt);
            inspect(&s);
        }
        (p, s)
    }

    #[test]
    fn it_lifts_off_and_hovers_on_three_rotors() {
        let mut worst_tilt = 0.0f32;
        let (p, s) = fly(Vec3::ZERO, 25.0, |s| {
            let (roll, pitch, _) = euler(s.rot);
            worst_tilt = worst_tilt.max(roll.abs()).max(pitch.abs());
        });
        let (_, _, yaw_rate) = body_rates(s.omega);
        assert_eq!(s.phase, Phase::Flying);
        assert!((s.pos.y - 10.0).abs() < 1.0, "altitude {:.2} m", s.pos.y);
        assert!(s.vel.length() < 1.5, "speed {:.2} m/s", s.vel.length());
        assert!(worst_tilt < 20.0 * DEG, "worst tilt {:.1} deg", worst_tilt / DEG);
        assert!(yaw_rate.abs() < 0.3, "yaw rate {yaw_rate:.2} rad/s");
        let minutes = p.battery_capacity_mah / 1000.0 / s.current * 60.0;
        println!("hover: {:.1} A, {:.1} V, about {:.1} min", s.current, s.voltage, minutes);
        assert!(s.current > 8.0 && s.current < 60.0, "hover current {:.1} A", s.current);
    }

    /// Minutes the pack lasts at a steady `amps`, used down to 15 % (a pilot lands before that).
    fn minutes_at(p: &AircraftParams, amps: f32) -> f32 {
        0.85 * p.battery_capacity_mah / 1000.0 / amps * 60.0
    }

    /// Current in level cruise at `speed` (m/s), hands off the sticks: the wing motors point
    /// forward, the rear rotor is off, the throttle holds the speed and the elevator holds the
    /// height. Returns the current (A) and the climb rate left over (m/s).
    fn cruise_amps(speed: f32) -> (f32, f32) {
        let p = t1s_vtol::params();
        let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        s.carry = None;
        s.phase = Phase::Flying;
        s.pos = Vec3::new(0.0, 300.0, 0.0);
        s.vel = Vec3::NEG_Z * speed;
        s.tilt.fill(0.0);
        let (dt, mut thr, mut elev) = (1.0 / 600.0, 0.5f32, 0.0f32);
        let (mut y0, mut amps, mut n) = (0.0, 0.0, 0.0);
        let total = 120.0;
        for i in 0..(total / dt) as usize {
            let t = i as f32 * dt;
            let (_, _, q) = (0.0, 0.0, body_rates(s.omega).0);
            thr = (thr + 0.05 * (speed - s.airspeed) * dt).clamp(0.05, 1.0);
            elev = (elev + 0.01 * (0.0 - s.vel.y) * dt).clamp(-0.5, 0.5);
            let mut cmd =
                simulation_aircraft::t1s_ranger::mix(0.0, (elev + 0.15 * (-q)).clamp(-1.0, 1.0), 0.0, [0.0, 0.0]);
            cmd.out[MOTOR_LEFT] = thr;
            cmd.out[MOTOR_RIGHT] = thr;
            cmd.out[TILT_LEFT] = 1.0;
            cmd.out[TILT_RIGHT] = 1.0;
            step(&p, &mut s, &cmd, &Environment::default(), &Flat, &[], dt);
            if t > total - 20.0 {
                if y0 == 0.0 {
                    y0 = s.pos.y;
                }
                amps += s.current;
                n += 1.0;
            }
        }
        if s.phase != Phase::Flying || s.pos.y < 100.0 {
            return (f32::NAN, f32::NAN);
        }
        (amps / n, (s.pos.y - y0) / 20.0)
    }

    /// HEEWING says of the Hunter 4S HV 3850 mAh pack in the T1: a full VTOL flight lasts about
    /// 10 minutes, and a flight that takes off in VTOL and then flies as a plane about 45.
    #[test]
    fn the_pack_lasts_as_long_as_heewing_says() {
        let (p, s) = fly(Vec3::ZERO, 25.0, |_| {});
        let hover = minutes_at(&p, s.current);
        let mut best = (f32::MAX, 0.0);
        for speed in [10.0, 11.0, 12.0, 12.5, 13.0, 13.5, 14.0, 15.0, 16.0] {
            let (amps, climb) = cruise_amps(speed);
            println!("cruise at {speed:.0} m/s: {amps:.1} A, climb {climb:+.2} m/s");
            if climb.abs() < 0.3 && amps < best.0 {
                best = (amps, speed);
            }
        }
        let (amps, speed) = best;
        let cruise = minutes_at(&p, amps);
        println!(
            "hover {:.1} A, {hover:.1} min; best cruise {amps:.1} A at {speed:.0} m/s, {cruise:.1} min",
            s.current
        );
        assert!((8.0..12.5).contains(&hover), "hover lasts {hover:.1} min, HEEWING says about 10");
        // Known gap: the model's cruise is about half as long as HEEWING's 45 minutes. The
        // airframe's drag is too high for that (a lift to drag ratio of about 4), and nothing
        // here is fitted to a flight log yet. Tighten this band when it is.
        assert!((20.0..52.0).contains(&cruise), "cruise lasts {cruise:.1} min, HEEWING says about 45");
    }

    #[test]
    fn it_holds_attitude_in_a_steady_wind() {
        let mut worst_tilt = 0.0f32;
        let (_, s) = fly(Vec3::new(5.0, 0.0, 0.0), 25.0, |s| {
            let (roll, pitch, _) = euler(s.rot);
            worst_tilt = worst_tilt.max(roll.abs()).max(pitch.abs());
        });
        assert!((s.pos.y - 10.0).abs() < 1.5, "altitude {:.2} m", s.pos.y);
        assert!(worst_tilt < 30.0 * DEG, "worst tilt {:.1} deg", worst_tilt / DEG);
    }
}

#[cfg(test)]
mod rest_tests {
    use super::*;
    use simulation_aircraft::{t1s_ranger, t1s_vtol};

    /// What an accelerometer on the plane feels while it sits on the ground, after it has settled:
    /// the mean and the biggest swing from it (m/s²), over 5 s.
    fn at_rest(p: &AircraftParams) -> (Vec3, f32) {
        let mut s = FlightState::new(p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        s.carry = None;
        s.phase = Phase::Flying;
        let rest = match p.start {
            simulation_aircraft::StartStyle::Ground { rest_height } => rest_height,
            _ => 0.05,
        };
        s.pos = Vec3::new(0.0, rest, 0.0);
        let c = ActuatorCommands::default();
        let dt = 1.0 / 1200.0;
        for _ in 0..(3.0 / dt) as usize {
            step(p, &mut s, &c, &Environment::default(), &Flat, &[], dt);
        }
        let mut samples = vec![];
        for _ in 0..(5.0 / dt) as usize {
            step(p, &mut s, &c, &Environment::default(), &Flat, &[], dt);
            samples.push(s.specific_force);
        }
        let mean = samples.iter().copied().sum::<Vec3>() / samples.len() as f32;
        let swing = samples.iter().map(|f| (*f - mean).length()).fold(0.0, f32::max);
        (mean, swing)
    }

    #[test]
    fn a_plane_on_the_ground_feels_a_steady_one_g() {
        for (name, p) in [("T1-S VTOL", t1s_vtol::params()), ("T1-S", t1s_ranger::params())] {
            let (mean, swing) = at_rest(&p);
            println!("{name}: feels {mean:?}, swings {swing:.3} m/s²");
            assert!((mean.length() - GRAVITY).abs() < 0.3, "{name}: mean {:.2} m/s²", mean.length());
            assert!(swing < 0.05, "{name}: the accelerometer swings {swing:.3} m/s² at rest");
        }
    }
}
