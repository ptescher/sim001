//! The world of a game: each drone's plane on the flight model, its pilot's radio for the
//! take-off, and the referee. Each SITL drone's physics thread steps its own plane here; drone 0's
//! also steps the referee, by the same time.
//!
//! Frames: the referee's field is north and east of its centre, and the centre is SITL's home.
//! The flight model's world is x east, y up, z south (`cognition_agent::world_to_ned`).

use crate::trees::Forest;
use cognition_plan::{Primitive, WirePlan, to_global};
use glam::{Vec2, Vec3};
use recreation_games::{Preflight, Radio, mode_slot};
use recreation_towers::flight;
use recreation_towers::{Game, Order, Phase as Referee, Rules, Team};
use simulation_aircraft::{ActuatorCommands, AircraftParams};
use simulation_physics::Obstacle;
use simulation_physics::{Environment, Flat, FlightState, Phase, Start, step};

/// A crashed drone is back at its base this long after the crash (s).
pub const RESPAWN: f32 = 15.0;
/// Each plan is good for this long (s); the ground sends a new one about once a second.
pub const PLAN_TTL: f32 = 3.0;

/// The world point (x east, y up, z south) of a field point at `alt` above the ground.
pub fn to_world(at: Vec2, alt: f32) -> Vec3 {
    Vec3::new(at.y, alt, -at.x)
}

/// The field point (north, east) and the height of a world point.
pub fn to_field(pos: Vec3) -> (Vec2, f32) {
    (Vec2::new(-pos.z, pos.x), pos.y)
}

/// The field velocity (north, east) of a world velocity.
pub fn vel_to_field(vel: Vec3) -> Vec2 {
    Vec2::new(-vel.z, vel.x)
}

/// In the air: what the referee counts as flying.
pub fn airborne(s: &FlightState) -> bool {
    matches!(s.phase, Phase::Launching | Phase::Flying) && s.carry.is_none() && !s.on_ground
}

/// Where drone `k` of `team` stands at its base: in a line across the field, facing *away* from
/// the centre. The autopilot turns a drone about half way round while it climbs in QHOVER (its
/// yaw estimate jumps as it leaves the ground: six drones in six, measured), so it ends up facing
/// the centre, and the take-off transition carries it out over the lane and the home tower's
/// clearing and not into the forest behind the base.
pub fn base_start(rules: &Rules, team: Team, k: usize) -> Start {
    let at = rules.base(team) + Vec2::new(0.0, 10.0 * (k as f32 - 2.0));
    let heading = if team == Team::Blue { std::f32::consts::PI } else { 0.0 };
    Start { pos: to_world(at, 0.0), heading }
}

/// The plan for drone `i` (its index in the game) to carry out `order`, flown `low` or high, with
/// home at (`lat`, `lon`).
pub fn plan_for(game: &Game, i: usize, order: Order, low: bool, home: (f64, f64), id: u16) -> WirePlan {
    let d = &game.drones[i];
    let (team, k) = (d.team, i % game.rules.drones);
    let target = match order {
        Order::Take(t) => game.towers[t].at,
        Order::Goto(p) => p,
        Order::Hold => d.at,
        Order::Home => game.rules.base(team),
    };
    let leg = if order == Order::Hold {
        // Circle where it is: low only if it is over open ground.
        let low = low && game.map.open(d.at);
        let alt = if low { flight::low_height(team, k) } else { flight::orbit_height(team, k) };
        flight::Leg { orbit: false, point: d.at, alt, radius: 0.0 }
    } else {
        let plan = flight::plan(&game.map, d.at, target, low);
        flight::leg(&game.map, &plan, team, k, d.at, d.alt)
    };
    let (primitive, radius) = match (order, leg.orbit) {
        (Order::Hold, _) => (Primitive::Loiter, 0.0),
        (_, true) => (Primitive::Orbit, leg.radius),
        (_, false) => (Primitive::Goto, 0.0),
    };
    let (lat, lon) = to_global(home.0, home.1, leg.point.x, leg.point.y);
    WirePlan { id, primitive, target: 0, lat, lon, alt: leg.alt, radius, ttl: PLAN_TTL }
}

/// The plane that a drone of `class` flies.
pub fn aircraft(class: recreation_towers::Class) -> AircraftParams {
    (simulation_aircraft::find(class.aircraft()).expect("a plane for every class").params)()
}

/// The game and the drones' planes.
pub struct World {
    pub game: Game,
    /// Each drone's plane, by its index in the game.
    pub params: Vec<AircraftParams>,
    pub planes: Vec<FlightState>,
    preflight: Vec<Preflight>,
    down_for: Vec<f32>,
    /// Each drone's order (by its index in the game). Until its seat gives it one, a drone circles
    /// over its base.
    pub orders: Vec<Option<Order>>,
    /// Each drone's order is flown low, along the lanes (by its index in the game).
    pub lows: Vec<bool>,
    /// The trees, for the flight model, and the ones near the plane being stepped.
    forest: Forest,
    near: Vec<Obstacle>,
    /// GO: when the drones' own time (drone 0's, s) gets here, the game starts.
    pub go: f64,
    pub crashes: u32,
    /// Towers each team took (Blue's, Red's).
    pub taken: [u32; 2],
    /// Physics packets from each drone's SITL.
    pub packets: Vec<u64>,
}

impl World {
    pub fn new(rules: Rules, go: f64) -> World {
        let game = Game::new(rules);
        let n = game.drones.len();
        let forest = Forest::new(&game.map.trees());
        let params: Vec<AircraftParams> = game.drones.iter().map(|d| aircraft(d.class)).collect();
        let planes = (0..n).map(|i| FlightState::new(&params[i], &World::start_of(&game, i))).collect();
        World {
            game,
            params,
            planes,
            preflight: vec![Preflight::default(); n],
            down_for: vec![0.0; n],
            orders: vec![Some(Order::Home); n],
            lows: vec![false; n],
            forest,
            near: vec![],
            go,
            crashes: 0,
            taken: [0; 2],
            packets: vec![0; n],
        }
    }

    fn start_of(game: &Game, i: usize) -> Start {
        let d = &game.drones[i];
        base_start(&game.rules, d.team, i % game.rules.drones)
    }

    /// The drones' own time (drone 0's, s).
    pub fn now(&self) -> f64 {
        self.planes[0].time
    }

    pub fn started(&self) -> bool {
        self.now() >= self.go
    }

    pub fn on_the_wing(&self, i: usize) -> bool {
        self.preflight[i].on_the_wing() && airborne(&self.planes[i])
    }

    /// One physics step of drone `i` from its autopilot's outputs, then the referee (by drone 0).
    /// Returns what its radio sends.
    pub fn step_drone(&mut self, i: usize, pwm: &[u16; 16], dt: f32) -> Radio {
        self.packets[i] += 1;
        let cmd = ActuatorCommands::from_pwm(&self.params[i], pwm);
        self.forest.near(self.planes[i].pos, &mut self.near);
        step(&self.params[i], &mut self.planes[i], &cmd, &Environment { wind: Vec3::ZERO }, &Flat, &self.near, dt);
        let radio = self.radio(i, dt);
        self.sync(i);
        if i == 0 && self.started() {
            let before: Vec<_> = self.game.towers.iter().map(|t| t.holder).collect();
            self.game.step(dt);
            for (t, was) in self.game.towers.iter().zip(before) {
                if let Some(team) = t.holder
                    && was != Some(team)
                {
                    self.taken[team.index()] += 1;
                }
            }
        }
        radio
    }

    /// What drone `i`'s radio sends: off before GO; the take-off; then FBWA with the sticks
    /// centred, for the AGENT mode to take over. A crashed drone is back at its base after
    /// [`RESPAWN`] s and takes off again.
    fn radio(&mut self, i: usize, dt: f32) -> Radio {
        let p = &self.params[i];
        let off = Radio { arm: false, mode: mode_slot(p, "QHOVER"), ..Default::default() };
        if self.planes[i].phase == Phase::Crashed {
            if self.down_for[i] == 0.0 {
                self.crashes += 1;
                let (at, alt) = to_field(self.planes[i].pos);
                println!("crash: drone {i}: {} at {at} (north, east), {alt:.1} m", self.planes[i].crash_reason);
            }
            self.down_for[i] += dt;
            if self.down_for[i] > RESPAWN {
                self.down_for[i] = 0.0;
                let start = World::start_of(&self.game, i);
                self.planes[i].reset_to_hand(p, &start);
                self.preflight[i].reset();
            }
            return off;
        }
        let s = &self.planes[i];
        if !self.started() || s.carry.is_some() || s.phase == Phase::Held {
            self.preflight[i].reset();
            return off;
        }
        let agl = s.pos.y;
        self.preflight[i].radio(p, s, agl, dt).unwrap_or(Radio {
            sticks: recreation_games::Controls { throttle: 0.5, ..Default::default() },
            arm: true,
            mode: mode_slot(p, "FBWA"),
        })
    }

    /// Drone `i` in the referee's game as its plane is.
    fn sync(&mut self, i: usize) {
        let s = &self.planes[i];
        let (at, alt) = to_field(s.pos);
        let d = &mut self.game.drones[i];
        d.at = at;
        d.alt = alt;
        d.vel = vel_to_field(s.vel);
        d.phase = if s.phase == Phase::Crashed {
            Referee::Down
        } else if airborne(s) {
            Referee::Flying
        } else {
            Referee::Ground
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World::new(Rules::default(), 0.0)
    }

    #[test]
    fn the_field_and_the_world_agree_with_the_ned_of_the_ground() {
        let p = to_world(Vec2::new(100.0, 20.0), 30.0);
        assert_eq!(cognition_agent::world_to_ned(p, Vec3::ZERO), Vec3::new(100.0, 20.0, -30.0));
        assert_eq!(to_field(p), (Vec2::new(100.0, 20.0), 30.0));
        assert_eq!(vel_to_field(Vec3::new(3.0, 0.0, -5.0)), Vec2::new(5.0, 3.0));
    }

    #[test]
    fn the_drones_start_on_the_ground_at_their_bases() {
        let w = world();
        let rules = w.game.rules;
        for (i, p) in w.planes.iter().enumerate() {
            let team = w.game.drones[i].team;
            let (at, alt) = to_field(p.pos);
            assert!(at.distance(rules.base(team)) <= 10.5, "drone {i} at {at}");
            assert!(alt < 1.0);
            assert!(!airborne(p));
        }
    }

    #[test]
    fn a_take_order_circles_the_tower_inside_its_circle_at_the_drones_own_height() {
        let mut w = world();
        let rules = w.game.rules;
        let home = (-35.363261, 149.165230);
        let west = w.game.tower("west");
        // A drone that is up (a drone on the ground climbs over its base first).
        let red = w.game.drone(Team::Red, 1);
        w.game.drones[red].alt = 60.0;
        w.game.drones[red].phase = Referee::Flying;
        let plan = plan_for(&w.game, red, Order::Take(west), false, home, 7);
        assert_eq!((plan.primitive, plan.radius, plan.id, plan.alt), (Primitive::Orbit, flight::ORBIT_RADIUS, 7, 67.0));
        assert!(plan.radius < rules.circle_radius && plan.alt < rules.circle_ceiling);
        let (n, e) = cognition_plan::to_local(home.0, home.1, plan.lat, plan.lon);
        let at = w.game.towers[west].at;
        assert!((n - at.x).abs() < 0.1 && (e - at.y).abs() < 0.1, "{n} {e}");
    }

    #[test]
    fn a_drone_on_the_ground_climbs_over_its_base_before_it_goes_to_its_order() {
        let w = world();
        let home = (-35.363261, 149.165230);
        let red = w.game.drone(Team::Red, 1);
        let plan = plan_for(&w.game, red, Order::Take(w.game.tower("west")), false, home, 1);
        assert_eq!((plan.primitive, plan.alt), (Primitive::Orbit, 67.0));
        let (n, e) = cognition_plan::to_local(home.0, home.1, plan.lat, plan.lon);
        let base = w.game.rules.base(Team::Red);
        assert!((n - base.x).abs() < 0.1 && (e - base.y).abs() < 0.1, "it circles the base, at {n} {e}");
    }

    #[test]
    fn a_low_order_is_flown_along_the_lane_below_the_treetops_and_a_high_one_straight() {
        let mut w = world();
        let home = (-35.363261, 149.165230);
        let blue = w.game.drone(Team::Blue, 1);
        // The drone is flying over the Blue home tower.
        w.game.drones[blue].at = w.game.towers[0].at;
        w.game.drones[blue].alt = 15.0;
        w.game.drones[blue].phase = Referee::Flying;
        let west = w.game.tower("west");
        let low = plan_for(&w.game, blue, Order::Take(west), true, home, 1);
        assert_eq!((low.primitive, low.alt), (Primitive::Goto, 17.5));
        let (n, e) = cognition_plan::to_local(home.0, home.1, low.lat, low.lon);
        assert!(w.game.map.open(Vec2::new(n, e)), "it aims along the lane");
        let high = plan_for(&w.game, blue, Order::Take(west), false, home, 2);
        assert_eq!((high.primitive, high.alt), (Primitive::Orbit, 63.0));
        assert!(low.alt < w.game.rules.treetop);
    }

    #[test]
    fn a_drone_with_no_order_from_its_seat_circles_over_its_base() {
        let w = world();
        assert!(w.orders.iter().all(|o| *o == Some(Order::Home)), "{:?}", w.orders);
    }

    #[test]
    fn nothing_happens_before_go() {
        let mut w = world();
        w.go = 100.0;
        let r = w.step_drone(0, &[1000; 16], 0.0025);
        assert!(!r.arm);
        assert_eq!(w.game.time, 0.0);
    }
}
