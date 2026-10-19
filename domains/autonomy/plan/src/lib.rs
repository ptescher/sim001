//! The plan: what the ground control agent tells a plane, about once a second, and what the
//! plane's on-board executor (`autonomy_executor`) takes to fly it.
//!
//! No new MAVLink dialect: a plan is a `COMMAND_INT` with `MAV_CMD_USER_1`, and the game state
//! another with `MAV_CMD_USER_2`. [`CommandInt`] holds just the fields of that message, so this
//! crate needs no MAVLink library; the station and the plane put them into and read them out of
//! the real message. All the `#[repr(C)]` types here are the executor's C ABI
//! (`executor/include/agent_executor.h` must match them).
//!
//! Frames: positions and velocities handed to the executor are local NED in metres from home
//! (north, east, down), as ArduPilot's EKF has them. On the wire a point is latitude and
//! longitude in 1e-7 degrees and an altitude in metres above home.

#![cfg_attr(feature = "ffi-static", no_std)]

/// `MAV_CMD_USER_1`: a plan.
pub const CMD_PLAN: u16 = 31010;
/// `MAV_CMD_USER_2`: the state of the game.
pub const CMD_GAME: u16 = 31011;

/// Names of the `NAMED_VALUE_INT` / `NAMED_VALUE_FLOAT` messages the plane reports with.
pub mod status_names {
    /// The plan being flown (its id), 0 for none.
    pub const PLAN: &str = "AG_PLAN";
    /// The primitive being flown (`Primitive as i32`).
    pub const PRIMITIVE: &str = "AG_PRIM";
    /// Interventions by the safety envelope since AGENT mode was entered.
    pub const INTERVENTIONS: &str = "AG_ENV";
    /// Why the executor fell back (`Fallback as i32`), 0 for not at all.
    pub const FALLBACK: &str = "AG_FALL";
}

/// What the plane is to fly.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Primitive {
    /// No plan: wings level, hold height.
    #[default]
    None = 0,
    /// Close on the track `target` and stay on its tail.
    Pursue = 1,
    /// Break away from the track `target`.
    Evade = 2,
    /// Fly to the point.
    Goto = 3,
    /// Circle the point at `radius`.
    Orbit = 4,
    /// Hold the height band `[z_low, z_high]` and wander in the arena (the point is its centre).
    Band = 5,
    /// Circle where you are.
    Loiter = 6,
}

impl Primitive {
    pub fn from_u8(v: u8) -> Option<Primitive> {
        Some(match v {
            0 => Primitive::None,
            1 => Primitive::Pursue,
            2 => Primitive::Evade,
            3 => Primitive::Goto,
            4 => Primitive::Orbit,
            5 => Primitive::Band,
            6 => Primitive::Loiter,
            _ => return None,
        })
    }
}

/// Why the executor stopped flying the plan (reported as `AG_FALL`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fallback {
    #[default]
    NotAtAll = 0,
    /// The plan ran out before another arrived.
    PlanExpired = 1,
    /// The state or a track had a NaN or an impossible value.
    BadInput = 2,
    /// Under the stall margin or below the floor: recovering.
    Recovery = 3,
}

/// A plan as the executor takes it (the C ABI). Positions are local NED, in metres.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plan {
    /// Increases with each plan, so a repeated one is ignored.
    pub id: u16,
    /// A [`Primitive`].
    pub primitive: u8,
    /// MAVLink system id of the track pursued or fled from.
    pub target: u8,
    /// The point (north, east, down, m) for Goto, Orbit and Band.
    pub point: [f32; 3],
    /// Orbit radius (m).
    pub radius: f32,
    /// Band: height above home, lowest and highest (m, up positive).
    pub band: [f32; 2],
    /// Seconds it is good for, from when the plane received it.
    pub ttl: f32,
}

/// The plane's own state (the C ABI): the EKF's, in local NED.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    /// Radians.
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
    /// True airspeed (m/s).
    pub airspeed: f32,
    /// Height above the ground (m); the same as height above home on a flat field.
    pub agl: f32,
    /// Seconds since the last step.
    pub dt: f32,
    /// Seconds since AGENT mode was entered.
    pub time: f32,
}

/// Another aircraft, from the plane's receivers or the ground's uplink (the C ABI).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Track {
    /// MAVLink system id (what a plan's `target` names).
    pub id: u8,
    /// 1 when this entry is in use.
    pub valid: u8,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    /// Seconds since this was heard.
    pub age: f32,
}

/// The most tracks the executor looks at.
pub const MAX_TRACKS: usize = 8;

/// The safety envelope's limits and the plane's performance (the C ABI). Set from ArduPilot's
/// parameters and the geofence; the policy can't change them.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Fence: centre (north, east, m) and radius (m). Radius 0: no fence.
    pub fence_centre: [f32; 2],
    pub fence_radius: f32,
    /// Ceiling above home (m) and floor above the ground (m).
    pub ceiling: f32,
    pub floor: f32,
    /// Largest bank (rad), pitch up and down (rad, both positive).
    pub bank_max: f32,
    pub pitch_max: f32,
    pub pitch_min: f32,
    /// Slowest airspeed the executor may let the plane fly at (m/s): `ARSPD_FBW_MIN` plus a margin.
    pub airspeed_min: f32,
    /// Cruise airspeed and the top (m/s).
    pub airspeed_cruise: f32,
    pub airspeed_max: f32,
    /// Never closer than this to another aircraft (m).
    pub separation: f32,
    /// Seconds a plan stays good past its ttl before the plane loiters (0: the ttl is it).
    pub grace: f32,
}

impl Default for Limits {
    /// The T1-S in a 90 m arena: the sim's tag field.
    fn default() -> Limits {
        Limits {
            fence_centre: [0.0, 0.0],
            fence_radius: 0.0,
            ceiling: 60.0,
            floor: 12.0,
            bank_max: 0.87,
            pitch_max: 0.44,
            pitch_min: -0.44,
            airspeed_min: 13.0,
            airspeed_cruise: 18.0,
            airspeed_max: 28.0,
            separation: 4.0,
            grace: 0.0,
        }
    }
}

/// What the executor asks the stock controllers for (the C ABI).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Demand {
    /// Roll demand (cd, positive right wing down).
    pub roll_cd: i32,
    /// Pitch demand (cd, positive nose up).
    pub pitch_cd: i32,
    /// Throttle 0 to 1, or a negative number to leave it to the pilot.
    pub throttle: f32,
    /// A [`Fallback`].
    pub fallback: u8,
    /// 1 if the envelope changed the demand on this step (the caller counts them).
    pub intervened: u8,
    pub _pad: [u8; 2],
}

/// The wire side: the station's (and the tests') end. The plane decodes the commands in C++, so
/// the on-board build leaves this out (it needs `f32::round` and `cos`, which `core` has not).
#[cfg(not(feature = "ffi-static"))]
pub mod wire {
    use super::*;

    /// The fields of `COMMAND_INT` a plan uses.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct CommandInt {
        pub command: u16,
        pub param1: f32,
        pub param2: f32,
        pub param3: f32,
        pub param4: f32,
        /// Latitude, 1e-7 degrees.
        pub x: i32,
        /// Longitude, 1e-7 degrees.
        pub y: i32,
        /// Altitude above home (m).
        pub z: f32,
    }

    /// A plan as it goes over the air: a primitive aimed at a point on the globe. Latitude and
    /// longitude are in degrees, `alt` is above home.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct WirePlan {
        pub id: u16,
        pub primitive: Primitive,
        pub target: u8,
        pub lat: f64,
        pub lon: f64,
        pub alt: f32,
        pub radius: f32,
        /// Seconds it is good for.
        pub ttl: f32,
    }

    impl WirePlan {
        /// `param1` primitive, `param2` target, `param3` id, `param4` ttl (s), `x`/`y` the point, and
        /// `z` the altitude above home and the radius packed in one float (a float holds 24 bits
        /// exactly: 12 for the altitude to 0.25 m, 10 for the radius to 1 m). For Band, the radius
        /// is the thickness of the band, which starts at `alt`.
        pub fn to_command(&self) -> CommandInt {
            CommandInt {
                command: CMD_PLAN,
                param1: self.primitive as u8 as f32,
                param2: self.target as f32,
                param3: self.id as f32,
                param4: self.ttl,
                x: (self.lat * 1e7).round() as i32,
                y: (self.lon * 1e7).round() as i32,
                z: pack_alt_radius(self.alt, self.radius),
            }
        }

        pub fn from_command(c: &CommandInt) -> Option<WirePlan> {
            if c.command != CMD_PLAN {
                return None;
            }
            let (alt, radius) = unpack_alt_radius(c.z);
            Some(WirePlan {
                id: finite_u16(c.param3)?,
                primitive: Primitive::from_u8(finite_u16(c.param1)? as u8)?,
                target: finite_u16(c.param2)? as u8,
                lat: c.x as f64 * 1e-7,
                lon: c.y as f64 * 1e-7,
                alt,
                radius,
                ttl: if c.param4.is_finite() { c.param4.max(0.0) } else { return None },
            })
        }
    }

    /// Altitude (-512 to 511.75 m, to 0.25 m) and radius (0 to 1023 m, to 1 m) in one `f32`: a
    /// float holds 24 bits exactly, 12 for each.
    fn pack_alt_radius(alt: f32, radius: f32) -> f32 {
        let a = ((alt.clamp(-512.0, 511.75) + 512.0) * 4.0).round() as u32; // 0..4095
        let r = radius.clamp(0.0, 1023.0).round() as u32; // 0..1023 (10 bits)
        ((a << 10) | r) as f32
    }

    fn unpack_alt_radius(z: f32) -> (f32, f32) {
        let v = z as u32;
        (((v >> 10) & 0xFFF) as f32 / 4.0 - 512.0, (v & 0x3FF) as f32)
    }

    fn finite_u16(v: f32) -> Option<u16> {
        (v.is_finite() && (0.0..=65535.0).contains(&v)).then_some(v as u16)
    }

    /// Which role the plane has in the game (`CMD_GAME` `param1`).
    #[repr(u8)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Role {
        #[default]
        Free = 0,
        /// It: chase.
        It = 1,
        /// Not it: keep away.
        Runner = 2,
    }

    /// The state of the game as the plane is told it (`CMD_GAME`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct WireGame {
        pub role: Role,
        /// Seconds left in the game.
        pub time_left: f32,
        /// The system id that this plane can't tag back yet (0 for none).
        pub no_tag_back: u8,
    }

    impl WireGame {
        pub fn to_command(&self) -> CommandInt {
            CommandInt {
                command: CMD_GAME,
                param1: self.role as u8 as f32,
                param2: self.time_left,
                param3: self.no_tag_back as f32,
                ..Default::default()
            }
        }

        pub fn from_command(c: &CommandInt) -> Option<WireGame> {
            if c.command != CMD_GAME || !c.param2.is_finite() {
                return None;
            }
            let role = match finite_u16(c.param1)? {
                0 => Role::Free,
                1 => Role::It,
                2 => Role::Runner,
                _ => return None,
            };
            Some(WireGame { role, time_left: c.param2, no_tag_back: finite_u16(c.param3)? as u8 })
        }
    }

    /// A point on the globe as local north-east metres from `home`, on a flat earth: good to a
    /// fraction of a metre over a few hundred metres.
    pub fn to_local(home_lat: f64, home_lon: f64, lat: f64, lon: f64) -> (f32, f32) {
        const R: f64 = 6_371_000.0;
        let north = (lat - home_lat).to_radians() * R;
        let east = (lon - home_lon).to_radians() * R * home_lat.to_radians().cos();
        (north as f32, east as f32)
    }

    /// The inverse of [`to_local`].
    pub fn to_global(home_lat: f64, home_lon: f64, north: f32, east: f32) -> (f64, f64) {
        const R: f64 = 6_371_000.0;
        let lat = home_lat + (north as f64 / R).to_degrees();
        let lon = home_lon + (east as f64 / (R * home_lat.to_radians().cos())).to_degrees();
        (lat, lon)
    }

    impl WirePlan {
        /// The plan the executor takes, with the point in local NED metres from `home`
        /// (`down` is minus the altitude).
        pub fn to_plan(&self, home_lat: f64, home_lon: f64) -> Plan {
            let (n, e) = to_local(home_lat, home_lon, self.lat, self.lon);
            Plan {
                id: self.id,
                primitive: self.primitive as u8,
                target: self.target,
                point: [n, e, -self.alt],
                radius: self.radius,
                band: [self.alt, self.alt + self.radius],
                ttl: self.ttl,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_plan_survives_the_wire() {
            let p = WirePlan { id: 4321, primitive: Primitive::Orbit, target: 3, lat: -35.363261, lon: 149.16523, alt: 37.5, radius: 60.0, ttl: 2.5 };
            let c = p.to_command();
            assert_eq!(c.command, CMD_PLAN);
            let q = WirePlan::from_command(&c).unwrap();
            assert_eq!((q.id, q.primitive, q.target, q.alt, q.radius, q.ttl), (4321, Primitive::Orbit, 3, 37.5, 60.0, 2.5));
            assert!((q.lat - p.lat).abs() < 1e-7 && (q.lon - p.lon).abs() < 1e-7);
        }

        #[test]
        fn altitude_and_radius_share_a_float_exactly() {
            for (alt, r) in [(0.0, 0.0), (12.25, 1.0), (-30.0, 1023.0), (511.75, 500.0)] {
                assert_eq!(unpack_alt_radius(pack_alt_radius(alt, r)), (alt, r));
            }
        }

        #[test]
        fn nonsense_is_refused() {
            let ok = WirePlan { id: 1, primitive: Primitive::Goto, ..Default::default() }.to_command();
            assert!(WirePlan::from_command(&ok).is_some());
            assert!(WirePlan::from_command(&CommandInt { command: 1, ..ok }).is_none());
            assert!(WirePlan::from_command(&CommandInt { param1: 99.0, ..ok }).is_none());
            assert!(WirePlan::from_command(&CommandInt { param1: f32::NAN, ..ok }).is_none());
            assert!(WirePlan::from_command(&CommandInt { param4: f32::INFINITY, ..ok }).is_none());
            assert!(WirePlan::from_command(&CommandInt { param3: -1.0, ..ok }).is_none());
        }

        #[test]
        fn the_game_state_survives_the_wire() {
            let g = WireGame { role: Role::It, time_left: 83.5, no_tag_back: 2 };
            assert_eq!(WireGame::from_command(&g.to_command()), Some(g));
            assert!(WireGame::from_command(&CommandInt { param1: 7.0, ..g.to_command() }).is_none());
        }

        #[test]
        fn local_and_global_agree() {
            let (lat, lon) = to_global(-35.36, 149.16, 120.0, -80.0);
            let (n, e) = to_local(-35.36, 149.16, lat, lon);
            assert!((n - 120.0).abs() < 0.01 && (e + 80.0).abs() < 0.01, "{n} {e}");
        }

        #[test]
        fn the_executor_gets_down_not_up() {
            let p = WirePlan { primitive: Primitive::Goto, lat: -35.36, lon: 149.16, alt: 30.0, ..Default::default() };
            assert_eq!(p.to_plan(-35.36, 149.16).point[2], -30.0);
        }
    }
}

#[cfg(not(feature = "ffi-static"))]
pub use wire::*;
