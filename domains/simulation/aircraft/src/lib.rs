//! What a plane is, to the physics: its lifting surfaces, motors, ground-contact points, mass
//! and battery. Each plane is a function that builds an [`AircraftParams`]; [`AIRCRAFT`] lists
//! them and [`find`] picks one by id. The geometry a plane's model is drawn from lives next to
//! its description (see [`t1s_ranger::geom`]).
//!
//! Body frame (Bevy convention): +X right, +Y up, -Z forward (nose). The origin is the CG.

use glam::Vec3;

pub mod t1s_ranger;
pub mod t1s_vtol;

/// The PWM in the middle of slot `i` of the radio's mode switch (ArduPilot's FLTMODE1 to FLTMODE6).
pub fn rc_mode_pwm(i: usize) -> u16 {
    1165 + 130 * i as u16
}

/// The slot of the radio's mode switch that a PWM on ArduPilot's FLTMODE_CH picks: six bands, with
/// the edges at 1230, 1360, 1490, 1620 and 1749 µs.
pub fn rc_mode_slot(pwm: u16) -> usize {
    match pwm {
        0..=1230 => 0,
        1231..=1360 => 1,
        1361..=1490 => 2,
        1491..=1620 => 3,
        1621..=1749 => 4,
        _ => 5,
    }
}

/// A plane the sim can fly.
pub struct Aircraft {
    /// What `--aircraft` takes.
    pub id: &'static str,
    pub title: &'static str,
    pub params: fn() -> AircraftParams,
}

/// Every plane, the default first.
pub const AIRCRAFT: &[Aircraft] = &[
    Aircraft { id: "t1s-vtol", title: "HEEWING T1-S VTOL", params: t1s_vtol::params },
    Aircraft { id: "t1s-ranger", title: "HEEWING T1-S Ranger", params: t1s_ranger::params },
];

/// The plane to fly when none is asked for.
pub fn default() -> &'static Aircraft {
    &AIRCRAFT[0]
}

/// The plane with this id.
pub fn find(id: &str) -> Option<&'static Aircraft> {
    AIRCRAFT.iter().find(|a| a.id == id)
}

/// The ids of all planes, for messages.
pub fn ids() -> Vec<&'static str> {
    AIRCRAFT.iter().map(|a| a.id).collect()
}

/// How many servo outputs a plane has: ArduPilot's 16 (and the SITL link's).
pub const MAX_OUTPUTS: usize = 16;
/// The most motors a plane can have.
pub const MAX_MOTORS: usize = 8;

/// How a plane starts, and starts again after a reset.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StartStyle {
    /// Carried to the pilot's hand, turned nose up a little, and thrown.
    Hand,
    /// Set down level on the ground, resting on its legs with the body this high above it (m).
    Ground { rest_height: f32 },
}

/// What drives a lifting surface's flap.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SurfaceControl {
    None,
    /// Output `channel` (-1..1) times `gain` is the deflection in radians, trailing edge down
    /// positive. A surface's side and the plane's mixing are in the gain and the output.
    Output { channel: usize, gain: f32 },
}

/// How a servo output's value (0..1 for a motor, -1..1 for a servo) is read from a PWM.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputKind {
    /// 1500 µs is 0, ±500 µs is ±1.
    Servo,
    /// 1000 µs is 0, 2000 µs is 1.
    Motor,
}

/// One servo output of the plane.
#[derive(Clone, Copy, Debug)]
pub struct Output {
    pub kind: OutputKind,
    /// ArduPilot drives this output the other way round from the physics' convention.
    pub invert: bool,
}

/// Normalized actuator commands, one per servo output, after any mixing / flight controller:
/// -1..1 for a servo, 0..1 for a motor. What each output does is in the plane's description
/// (`AeroSurface::control`, `MotorSpec::channel`).
#[derive(Clone, Copy, Debug)]
pub struct ActuatorCommands {
    pub out: [f32; MAX_OUTPUTS],
}

impl Default for ActuatorCommands {
    fn default() -> Self {
        Self { out: [0.0; MAX_OUTPUTS] }
    }
}

impl ActuatorCommands {
    /// From ArduPilot's servo PWMs (0 means the output is off).
    pub fn from_pwm(p: &AircraftParams, pwm: &[u16; MAX_OUTPUTS]) -> Self {
        let mut c = Self::default();
        for (i, o) in p.outputs.iter().enumerate() {
            let v = pwm[i];
            let x = match o.kind {
                _ if v == 0 => 0.0,
                OutputKind::Servo => ((v as f32 - 1500.0) / 500.0).clamp(-1.0, 1.0),
                OutputKind::Motor => ((v as f32 - 1000.0) / 1000.0).clamp(0.0, 1.0),
            };
            c.out[i] = if o.invert { -x } else { x };
        }
        c
    }

    /// The average throttle of the motors that fly the plane forward (the HUD's throttle and
    /// the hand-launch trigger).
    pub fn forward_throttle(&self, p: &AircraftParams) -> f32 {
        let (sum, n) = p.motors.iter().filter(|m| m.forward).fold((0.0, 0), |(s, n), m| (s + self.out[m.channel], n + 1));
        if n == 0 { 0.0 } else { sum / n as f32 }
    }
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

/// A motor on a tilt servo. The servo output (-1..1) turns it from fully back (-1, past vertical
/// by `range - 90°`) to fully forward (+1, the motor's `dir`), about `axis`. Positive turns
/// (right-hand rule about `axis`) swing `dir` from forward up.
#[derive(Clone, Copy, Debug)]
pub struct TiltSpec {
    /// The servo output that tilts it.
    pub channel: usize,
    pub axis: Vec3,
    /// The whole travel, from forward to fully back (rad).
    pub range: f32,
    /// The angle from forward it sits at before the servo is driven (rad): straight up.
    pub rest: f32,
    /// How fast the servo turns it (rad/s).
    pub rate: f32,
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
    /// The servo output that throttles it.
    pub channel: usize,
    /// It pushes the plane forward in cruise (the HUD's throttle, and the hand-launch trigger).
    pub forward: bool,
    /// It can turn between pointing up and pointing forward.
    pub tilt: Option<TiltSpec>,
    /// It can lift the plane in a hover (and so has the aerodynamics of a rotor: ground effect,
    /// vortex ring, a sideways drag).
    pub lift: bool,
    /// Reaction torque on the airframe about the thrust axis, per newton of thrust (m): the
    /// sign is the direction (right-hand rule about `dir`), 0 for none.
    pub reaction: f32,
    /// Blade tip speed at full throttle (m/s): how fast the rotor turns, for the force it
    /// makes in sideways flow.
    pub tip_speed: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ContactPoint {
    pub pos: Vec3,
    /// A propeller tip: touching the ground with this motor spinning is a prop strike.
    pub prop: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct AircraftParams {
    pub mass: f32,
    /// Principal moments of inertia about body X (pitch), Y (yaw), Z (roll).
    pub inertia: Vec3,
    pub surfaces: Vec<AeroSurface>,
    pub motors: Vec<MotorSpec>,
    /// The servo outputs, by channel.
    pub outputs: Vec<Output>,
    /// Fuselage drag area (Cd*A) for flow along body X, Y, Z.
    pub fuselage_cda: Vec3,
    pub contacts: Vec<ContactPoint>,
    pub servo_rate: f32,
    pub battery_cells: f32,
    /// High-voltage LiPo cells (LiHV): 4.35 V full instead of 4.2 V.
    pub battery_hv: bool,
    pub battery_capacity_mah: f32,
    pub battery_resistance: f32,
    pub start: StartStyle,
    /// The plane carries a flight controller. Without one there is only a receiver and ESCs: no
    /// ArduPilot, no telemetry, no self-levelling modes, just the sticks moving the surfaces.
    pub flight_controller: bool,
    /// The ArduPilot modes on the radio's mode switch, in the order of FLTMODE1 to FLTMODE6 (and
    /// so the PWMs 1165, 1295, 1425, 1555, 1685 and 1815 µs). Empty without a flight controller.
    pub rc_modes: Vec<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The positions of the radio's mode channel, as ArduPilot's FLTMODE1 to FLTMODE6 have them:
    /// two hover modes, forward flight (stabilized, then holding height and heading), a mission,
    /// and the way home.
    #[test]
    fn the_vtol_has_six_radio_modes_and_the_plain_plane_none() {
        let vtol = t1s_vtol::params();
        assert_eq!(vtol.rc_modes, ["QHOVER", "QLOITER", "FBWA", "CRUISE", "AUTO", "QRTL"]);
        let at = |pwm| vtol.rc_modes[rc_mode_slot(pwm)];
        assert_eq!((at(1165), at(1295), at(1425), at(1555), at(1685), at(1815)), ("QHOVER", "QLOITER", "FBWA", "CRUISE", "AUTO", "QRTL"));
        assert!(t1s_ranger::params().rc_modes.is_empty(), "no flight controller, no modes");
    }

    /// ArduPilot's FLTMODE_CH bands, and the PWM in the middle of each.
    #[test]
    fn the_six_slots_are_ardupilots_bands() {
        for (pwm, slot) in [(900, 0), (1230, 0), (1231, 1), (1360, 1), (1361, 2), (1490, 2), (1491, 3), (1620, 3), (1621, 4), (1749, 4), (1750, 5), (2100, 5)] {
            assert_eq!(rc_mode_slot(pwm), slot, "{pwm} µs");
        }
        for slot in 0..6 {
            assert_eq!(rc_mode_slot(rc_mode_pwm(slot)), slot, "the middle of slot {slot}");
        }
    }
}
