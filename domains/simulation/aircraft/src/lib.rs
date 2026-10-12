//! What a plane is, to the physics: its lifting surfaces, motors, ground-contact points, mass
//! and battery. Each plane is a function that builds an [`AircraftParams`]; [`AIRCRAFT`] lists
//! them and [`find`] picks one by id. The geometry a plane's model is drawn from lives next to
//! its description (see [`t1s_ranger::geom`]).
//!
//! Body frame (Bevy convention): +X right, +Y up, -Z forward (nose). The origin is the CG.

use glam::Vec3;

pub mod t1s_ranger;

/// A plane the sim can fly.
pub struct Aircraft {
    /// What `--aircraft` takes.
    pub id: &'static str,
    pub title: &'static str,
    pub params: fn() -> AircraftParams,
}

/// Every plane, the default first.
pub const AIRCRAFT: &[Aircraft] =
    &[Aircraft { id: "t1s-ranger", title: "HEEWING T1-S Ranger", params: t1s_ranger::params }];

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

