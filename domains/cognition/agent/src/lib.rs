//! The ground control agent: what the station decides for each of its planes, about once a second,
//! and the MAVLink link that tells them (see plans/sim2real-tag.md).
//!
//! - A [`Brain`] takes the [`Picture`] the ground has (its own planes, the game) and gives one plan
//!   ([`cognition_plan::WirePlan`]) per flying plane. [`ScriptedBrain`] is the computer pilots'
//!   strategy (`recreation_games`) re-expressed as plans.
//! - A [`link::Link`] per plane, over MAVLink, sends the plan (`COMMAND_INT` 31010), the game
//!   state (31011), the switch to AGENT mode and the other planes as `ADSB_VEHICLE`, and reads
//!   back the heartbeat's mode, `COMMAND_ACK` and the agent's `NAMED_VALUE_INT` status.
//!
//! Frames: positions in a picture are local NED in metres from home (north, east, down), as the
//! plane's EKF has them. Plane ids are the ids the plane knows the others by: a plan's `target` and
//! the low byte of an uplinked `ADSB_VEHICLE`'s ICAO address.

pub mod link;
pub mod scripted;

pub use cognition_plan::{Primitive, WireGame, WirePlan, wire::Role};
pub use scripted::{AgentSkill, ScriptedBrain, Tactics};

use glam::Vec3;

/// One of the station's planes, as the ground sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Craft {
    /// Its id (1..=255): the target of a plan, the low byte of its ADS-B ICAO address.
    pub id: u8,
    /// Local NED from home (m).
    pub pos: Vec3,
    /// NED (m/s).
    pub vel: Vec3,
    /// In the air (not on the ground, being carried or crashed).
    pub flying: bool,
}

impl Craft {
    /// Height above home (m, up positive).
    pub fn alt(&self) -> f32 {
        -self.pos.z
    }
}

/// The game as the ground knows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameView {
    /// The id of the plane that is it (0: none of ours).
    pub it: u8,
    /// Seconds left in the game.
    pub time_left: f32,
    /// The id the plane that is it may not tag back yet (0 for none).
    pub no_tag_back: u8,
    /// The arena: centre (north, east, m from home), radius (m), ceiling and floor above home (m).
    pub centre: [f32; 2],
    pub radius: f32,
    pub ceiling: f32,
    pub floor: f32,
}

impl Default for GameView {
    /// The sim's tag arena (`recreation_tag::Arena::around`): 90 m round a point 70 m north of
    /// the pilot box, under 60 m, with the executor's floor of 12 m.
    fn default() -> GameView {
        GameView {
            it: 0,
            time_left: 0.0,
            no_tag_back: 0,
            centre: [70.0, 0.0],
            radius: 90.0,
            ceiling: 60.0,
            floor: 12.0,
        }
    }
}

/// What the ground has to decide with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picture {
    /// Seconds (any clock that runs with the planes'; the brain only looks at differences).
    pub time: f32,
    /// Home (degrees): where local NED (0, 0, 0) is.
    pub home: (f64, f64),
    pub planes: Vec<Craft>,
    pub game: GameView,
}

impl Picture {
    pub fn plane(&self, id: u8) -> Option<&Craft> {
        self.planes.iter().find(|c| c.id == id)
    }

    /// The game state as plane `id` is to be told it (`CMD_GAME`).
    pub fn game_for(&self, id: u8) -> WireGame {
        let role = if self.game.it == 0 {
            Role::Free
        } else if self.game.it == id {
            Role::It
        } else {
            Role::Runner
        };
        let no_tag_back = if role == Role::It { self.game.no_tag_back } else { 0 };
        WireGame { role, time_left: self.game.time_left.max(0.0), no_tag_back }
    }

    /// A point in local NED as latitude and longitude (degrees).
    pub fn global(&self, north: f32, east: f32) -> (f64, f64) {
        cognition_plan::to_global(self.home.0, self.home.1, north, east)
    }
}

/// A plan for one plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Order {
    pub plane: u8,
    pub plan: WirePlan,
}

/// What decides the plans: scripted, trained or otherwise.
pub trait Brain {
    /// One plan per flying plane of the picture's (none for a plane on the ground). Called about
    /// once a second; each plan carries a ttl, so a plane whose ground goes quiet falls back to a
    /// loiter on its own.
    fn decide(&mut self, picture: &Picture) -> Vec<Order>;
}

/// World (the sim's: +X east, +Y up, -Z north) to local NED, with `home` the world point at NED
/// (0, 0, 0): the mapping of `simulation_sitl::sensor_json`.
pub fn world_to_ned(world: Vec3, home: Vec3) -> Vec3 {
    let v = world - home;
    Vec3::new(-v.z, v.x, -v.y)
}

/// The same for a velocity.
pub fn world_vel_to_ned(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, -v.y)
}

/// A `"lat,lon,alt,heading"` home string (SITL's `--home`) as latitude, longitude and altitude.
pub fn parse_home(home: &str) -> Option<(f64, f64, f32)> {
    let mut it = home.split(',').map(|s| s.trim().parse::<f64>());
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()? as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_maps_to_ned_as_sitl_has_it() {
        // 10 m east, 20 m up, 30 m north of home.
        assert_eq!(world_to_ned(Vec3::new(10.0, 20.0, -30.0), Vec3::ZERO), Vec3::new(30.0, 10.0, -20.0));
        assert_eq!(world_vel_to_ned(Vec3::new(1.0, 2.0, -3.0)), Vec3::new(3.0, 1.0, -2.0));
    }

    #[test]
    fn each_plane_hears_its_own_role() {
        let p = Picture {
            game: GameView { it: 2, no_tag_back: 3, time_left: 40.0, ..Default::default() },
            ..Default::default()
        };
        assert_eq!(p.game_for(2), WireGame { role: Role::It, time_left: 40.0, no_tag_back: 3 });
        assert_eq!(p.game_for(1), WireGame { role: Role::Runner, time_left: 40.0, no_tag_back: 0 });
        let free = Picture::default();
        assert_eq!(free.game_for(1).role, Role::Free);
    }

    #[test]
    fn the_sitl_home_parses() {
        let (lat, lon, alt) = parse_home("-35.363261,149.165230,584,353").unwrap();
        assert_eq!((lat, lon, alt), (-35.363261, 149.16523, 584.0));
        assert!(parse_home("x").is_none());
    }
}
