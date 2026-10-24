//! The world of a game: each drone's plane on the flight model, its pilot's radio for the
//! take-off, and the referee. Each SITL drone's physics thread steps its own plane here; drone 0's
//! also steps the referee, by the same time.
//!
//! Frames: the referee's field is north and east of its centre, and the centre is SITL's home.
//! The flight model's world is x east, y up, z south (`cognition_agent::world_to_ned`).

use crate::terrain::Valley;
use crate::trees::Forest;
use cognition_plan::{Primitive, WirePlan, to_global};
use glam::{Vec2, Vec3};
use recreation_games::{Controls, Preflight, Radio, flying_slot, mode_slot};
use recreation_towers::flight;
use recreation_towers::{Class, Game, Instruction, Link, Order, Phase as Referee, Rules, Standing, Team};
use simulation_aircraft::{ActuatorCommands, AircraftParams};
use simulation_physics::Obstacle;
use simulation_physics::{Environment, FlightState, Phase, Start, step};

/// A crashed drone is back at its base this long after the crash (s).
pub const RESPAWN: f32 = 15.0;
/// A drone this far from where it started (m) has left its base.
const AWAY: f32 = 100.0;
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

/// Where drone `i` flies to carry out `order`, flown `low` or high.
pub fn leg_for(game: &Game, i: usize, order: Order, low: bool) -> flight::Leg {
    let d = &game.drones[i];
    let (team, k) = (d.team, i % game.rules.drones);
    let target = match order {
        Order::Take(t) => game.towers[t].at,
        Order::Goto(p) => p,
        Order::Hold => d.at,
        Order::Home => game.rules.base(team),
    };
    if order == Order::Hold {
        // Circle where it is: low only if it is over open ground.
        let low = low && game.map.open(d.at);
        let alt = if low { flight::low_height(team, k) } else { flight::orbit_height(team, k) };
        flight::Leg { orbit: false, point: d.at, alt, radius: 0.0 }
    } else {
        let plan = flight::plan(&game.map, d.at, target, low);
        flight::leg(&game.map, &plan, team, k, d.at, d.alt)
    }
}

/// The plan for drone `i` (its index in the game) to carry out `order`, flown `low` or high, with
/// home at (`lat`, `lon`).
pub fn plan_for(game: &Game, i: usize, order: Order, low: bool, home: (f64, f64), id: u16) -> WirePlan {
    let leg = leg_for(game, i, order, low);
    let (primitive, radius) = match (order, leg.orbit) {
        (Order::Hold, _) => (Primitive::Loiter, 0.0),
        (_, true) => (Primitive::Orbit, leg.radius),
        (_, false) => (Primitive::Goto, 0.0),
    };
    let (lat, lon) = to_global(home.0, home.1, leg.point.x, leg.point.y);
    WirePlan { id, primitive, target: 0, lat, lon, alt: leg.alt, radius, ttl: PLAN_TTL }
}

/// The fastest a hovering drone is flown (m/s): its QLOITER's full stick (`Q_LOIT_SPEED_MS` in
/// `ardupilot/seeker3.parm`).
const HOVER_SPEED: f32 = 25.0;
/// How fast it climbs at full throttle stick (m/s): `Q_PILOT_SPD_UP`.
const HOVER_CLIMB: f32 = 5.0;
/// Flown low, under the treetops, in a lane 60 m wide, it is asked for this speed at most (m/s).
const LANE_SPEED: f32 = 18.0;
/// It plans to brake this hard for its point (m/s/s): under QLOITER's `Q_LOIT_BRK_ACC_M`.
const HOVER_BRAKE: f32 = 4.0;
/// QLOITER takes seconds to reach the speed the sticks ask for, so the sticks ask for this much
/// more of the change than they want.
const HOVER_LEAD: f32 = 1.0;
/// A drone whose way turns away from where it flies slows to this share of its speed, at most.
const HOVER_TURN: f32 = 0.25;

/// The sticks that fly a drone that hovers (no wing, in QLOITER) to a point: the roll and pitch
/// sticks ask for a speed relative to its nose, and the throttle stick for a climb, as a pilot's do.
/// `at`: north, east of the field's centre; `vel`: its velocity (m/s, north, east); `climbing`:
/// its climb rate (m/s); `heading`: clockwise from north; `top`: the most speed to ask for (m/s).
#[allow(clippy::too_many_arguments)]
pub fn hover_sticks(
    at: Vec2,
    vel: Vec2,
    top: f32,
    alt: f32,
    climbing: f32,
    heading: f32,
    to: Vec2,
    to_alt: f32,
) -> Controls {
    let err = to - at;
    let dist = err.length();
    // Slow down as it comes in: the speed it can lose in the distance left.
    let mut speed = (2.0 * HOVER_BRAKE * dist).sqrt().min(0.5 * dist).min(top);
    // And for a corner: the more the way turns from where it flies, the slower.
    if vel.length() > 3.0 && dist > 0.1 {
        speed *= vel.normalize().dot(err / dist).max(HOVER_TURN);
    }
    let want = if dist > 0.1 { err / dist * speed } else { Vec2::ZERO };
    let want = (want + (want - vel) * HOVER_LEAD).clamp_length_max(top);
    let nose = Vec2::new(heading.cos(), heading.sin());
    let right = Vec2::new(-heading.sin(), heading.cos());
    // The climb it asks for is what is left of the height, less the climb it has: no overshoot.
    let climb = (0.3 * (to_alt - alt) - 0.6 * climbing).clamp(-HOVER_CLIMB, HOVER_CLIMB);
    Controls {
        aileron: (want.dot(right) / HOVER_SPEED).clamp(-1.0, 1.0),
        // Pitch stick forward is a negative elevator.
        elevator: (-want.dot(nose) / HOVER_SPEED).clamp(-1.0, 1.0),
        rudder: 0.0,
        throttle: 0.5 + 0.5 * climb / HOVER_CLIMB,
    }
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
    /// Each drone's standing order: what it does when it is out of contact (by its index in the game).
    pub standing: Vec<Option<Standing>>,
    /// The order each team last knew of for each drone, and whether it is low: what the drone
    /// was told while it was in contact. A drone out of contact flies on, and its team is not told
    /// what it flies now (the order is the team's, and the drone's own failsafe is not).
    pub told: Vec<(Option<Order>, bool)>,
    /// The ground: flat, with the mountains.
    terrain: Valley,
    /// The trees, for the flight model, and the ones near the plane being stepped.
    forest: Forest,
    near: Vec<Obstacle>,
    /// GO: when the drones' own time (drone 0's, s) gets here, the game starts.
    pub go: f64,
    pub crashes: u32,
    /// Towers each team took (Blue's, Red's).
    pub taken: [u32; 2],
    /// When each drone was first 100 m from where it started (the game's time, s).
    pub away: Vec<Option<f32>>,
    /// The fastest each drone has flown over the ground (m/s).
    pub top_speed: Vec<f32>,
    /// Physics packets from each drone's SITL.
    pub packets: Vec<u64>,
}

impl World {
    pub fn new(rules: Rules, go: f64) -> World {
        let game = Game::new(rules);
        let n = game.drones.len();
        let forest = Forest::new(&game.map.trees());
        let terrain = Valley(game.map.clone());
        let params: Vec<AircraftParams> = game.drones.iter().map(|d| aircraft(d.class)).collect();
        let lows: Vec<bool> = game.drones.iter().map(|d| d.class == Class::Light).collect();
        let planes = (0..n).map(|i| FlightState::new(&params[i], &World::start_of(&game, i))).collect();
        World {
            game,
            params,
            planes,
            preflight: vec![Preflight::default(); n],
            down_for: vec![0.0; n],
            orders: vec![Some(Order::Home); n],
            told: lows.iter().map(|&low| (Some(Order::Home), low)).collect(),
            lows,
            standing: vec![None; n],
            terrain,
            forest,
            near: vec![],
            go,
            crashes: 0,
            taken: [0; 2],
            away: vec![None; n],
            top_speed: vec![0.0; n],
            packets: vec![0; n],
        }
    }

    /// Drone `i` flies its orders low unless it is told otherwise: a light drone, which is small
    /// and quick, and hides under the treetops.
    pub fn low_by_default(&self, i: usize) -> bool {
        self.game.drones[i].class == Class::Light
    }

    /// The drones of `class`, by their index in the game.
    pub fn of_class(&self, class: Class) -> impl Iterator<Item = usize> + '_ {
        (0..self.game.drones.len()).filter(move |&i| self.game.drones[i].class == class)
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
        step(
            &self.params[i],
            &mut self.planes[i],
            &cmd,
            &Environment { wind: Vec3::ZERO },
            &self.terrain,
            &self.near,
            dt,
        );
        let radio = self.radio(i, dt);
        self.sync(i);
        if i == 0 {
            // Before GO the game stands still, but the drones still hear their orders (and the
            // commander sees them).
            if !self.started() {
                self.link_up();
                return radio;
            }
            let before: Vec<_> = self.game.towers.iter().map(|t| t.holder).collect();
            self.game.step(dt);
            self.link_up();
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

    /// The seats may give drone `i` an order: it is in contact.
    pub fn can_command(&self, i: usize) -> bool {
        !self.game.jammed[i]
    }

    /// Gives drone `i` an order, if it is in contact. Whether it was heard.
    pub fn command(&mut self, i: usize, order: Order, low: bool) -> bool {
        self.command_standing(i, order, low, None)
    }

    /// Gives drone `i` an order and a standing order, if it is in contact. Whether it was heard.
    pub fn command_standing(&mut self, i: usize, order: Order, low: bool, standing: Option<Standing>) -> bool {
        if !self.can_command(i) {
            return false;
        }
        self.orders[i] = Some(order);
        self.lows[i] = low;
        self.standing[i] = standing;
        true
    }

    /// What the drones' radios do after a step of the referee: a drone in contact tells its team
    /// its order; one that is out of contact long enough flies its standing order, or, with none,
    /// home (the failsafe), and stays on that order until the team can give it another.
    fn link_up(&mut self) {
        for i in 0..self.orders.len() {
            if self.game.link(i) == Link::Up {
                self.told[i] = (self.orders[i], self.lows[i]);
                continue;
            }
            let mut flown = Instruction { order: self.orders[i], standing: self.standing[i] };
            flown.tick(self.game.silent[i], self.game.rules.link_timeout, self.game.charge(i), self.game.rules.bingo);
            if flown.order != self.orders[i] {
                println!("failsafe: drone {i} out of contact, and flies {:?}", flown.order);
                self.orders[i] = flown.order;
                self.lows[i] = self.low_by_default(i);
            }
        }
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
        self.preflight[i].radio(p, s, agl, dt).unwrap_or_else(|| {
            // A drone on the wing is flown by its plan. One with no wing is flown by the sticks
            // to where its order sends it.
            let sticks = if self.game.drones[i].class == recreation_towers::Class::Light {
                let order = self.orders[i].unwrap_or(Order::Home);
                let leg = leg_for(&self.game, i, order, self.lows[i]);
                let (at, alt) = to_field(s.pos);
                let heading = simulation_physics::euler(s.rot).2;
                let top = if self.lows[i] { LANE_SPEED } else { HOVER_SPEED };
                hover_sticks(at, vel_to_field(s.vel), top, alt, s.vel.y, heading, leg.point, leg.alt)
            } else {
                Controls { throttle: 0.5, ..Default::default() }
            };
            Radio { sticks, arm: true, mode: flying_slot(p) }
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
        let vel = d.vel;
        if self.game.time > 0.0 {
            if airborne(&self.planes[i]) {
                self.top_speed[i] = self.top_speed[i].max(vel.length());
            }
            let (start, _) = to_field(World::start_of(&self.game, i).pos);
            if self.away[i].is_none() && at.distance(start) > AWAY {
                self.away[i] = Some(self.game.time);
            }
        }
        let d = &mut self.game.drones[i];
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
        World::new(Rules::default().without_batteries(), 0.0)
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
            assert!(at.distance(rules.base(team)) <= 20.5, "drone {i} at {at}");
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
    fn a_hovering_drone_is_flown_towards_its_point_by_its_sticks() {
        // Facing north, with the point to the north east and higher: forward, right, and up.
        let s = hover_sticks(Vec2::ZERO, Vec2::ZERO, HOVER_SPEED, 50.0, 0.0, 0.0, Vec2::new(100.0, 100.0), 80.0);
        assert!(s.elevator < -0.3 && s.aileron > 0.3 && s.throttle > 0.6, "{s:?}");
        // Facing east, the same point is on its left and ahead.
        let s = hover_sticks(
            Vec2::ZERO,
            Vec2::ZERO,
            HOVER_SPEED,
            50.0,
            0.0,
            std::f32::consts::FRAC_PI_2,
            Vec2::new(100.0, 100.0),
            50.0,
        );
        assert!(s.elevator < -0.3 && s.aileron < -0.3 && (s.throttle - 0.5).abs() < 1e-3, "{s:?}");
        // On the point and at height: the sticks are at rest.
        let s = hover_sticks(Vec2::new(5.0, 5.0), Vec2::ZERO, HOVER_SPEED, 60.0, 0.0, 1.0, Vec2::new(5.0, 5.0), 60.0);
        assert_eq!((s.aileron, s.elevator, s.throttle), (0.0, 0.0, 0.5));
    }

    /// Far from its point, a light drone asks for the full speed of its QLOITER, the speed a
    /// Seeker 3 flies; near it, it asks for less than it flies, to brake in time.
    #[test]
    fn a_hovering_drone_flies_at_full_speed_and_brakes_for_its_point() {
        let s = hover_sticks(Vec2::ZERO, Vec2::ZERO, HOVER_SPEED, 50.0, 0.0, 0.0, Vec2::new(500.0, 0.0), 50.0);
        assert_eq!(s.elevator, -1.0, "full stick forward: {s:?}");
        const { assert!(HOVER_SPEED >= 20.0) };
        // At full speed, 30 m from the point: it asks for less than half its speed.
        let s = hover_sticks(
            Vec2::ZERO,
            Vec2::new(HOVER_SPEED, 0.0),
            HOVER_SPEED,
            50.0,
            0.0,
            0.0,
            Vec2::new(30.0, 0.0),
            50.0,
        );
        assert!(-s.elevator * HOVER_SPEED < 0.5 * HOVER_SPEED, "{s:?}");
    }

    /// Under the treetops a lane is 60 m wide, and a drone that leans over to turn at 25 m/s
    /// leaves it: a light drone flying low is asked for less than its full speed.
    #[test]
    fn a_hovering_drone_is_held_to_the_speed_it_is_given() {
        let s = hover_sticks(
            Vec2::ZERO,
            Vec2::new(LANE_SPEED, 0.0),
            LANE_SPEED,
            18.0,
            0.0,
            0.0,
            Vec2::new(500.0, 0.0),
            18.0,
        );
        assert!((-s.elevator * HOVER_SPEED - LANE_SPEED).abs() < 1.0, "{s:?}");
        let s = hover_sticks(Vec2::ZERO, Vec2::ZERO, LANE_SPEED, 18.0, 0.0, 0.0, Vec2::new(500.0, 0.0), 18.0);
        assert!(-s.elevator * HOVER_SPEED <= LANE_SPEED + 1e-3, "no more than the cap at the start: {s:?}");
        const { assert!(LANE_SPEED < HOVER_SPEED && LANE_SPEED >= 16.0) };
    }

    /// A light drone in a lane that turns slows for the corner: flying north at full speed with
    /// its point off to the east, it asks for less speed than it has.
    #[test]
    fn a_hovering_drone_slows_for_a_corner() {
        let v = Vec2::new(HOVER_SPEED, 0.0);
        let s = hover_sticks(Vec2::ZERO, v, HOVER_SPEED, 18.0, 0.0, 0.0, Vec2::new(0.0, 70.0), 18.0);
        assert!(s.elevator > 0.0, "it brakes: {s:?}");
        assert!(s.aileron * HOVER_SPEED < 0.6 * HOVER_SPEED, "it does not ask for full speed to the east: {s:?}");
    }

    /// The light drones fly their orders low by default, the others high.
    #[test]
    fn a_light_drone_flies_low_by_default() {
        let mut w = world();
        for i in 0..w.game.drones.len() {
            let light = w.game.drones[i].class == Class::Light;
            assert_eq!(w.lows[i], light, "drone {i}");
            assert_eq!(w.low_by_default(i), light);
        }
        let light = w.of_class(Class::Light).next().unwrap();
        w.command(light, Order::Home, w.low_by_default(light));
        assert!(w.lows[light]);
    }

    /// Blue's drone 1 flies in the radio shadow of the Red home tower.
    fn shadowed() -> (World, usize) {
        let mut w = world();
        let i = w.game.drone(Team::Blue, 1);
        w.game.drones[i].at = w.game.towers[4].at - Vec2::new(100.0, 0.0);
        w.game.drones[i].alt = 50.0;
        w.game.drones[i].phase = Referee::Flying;
        w.game.look();
        (w, i)
    }

    #[test]
    fn a_drone_in_the_radio_shadow_does_not_hear_an_order_and_one_in_contact_does() {
        let (mut w, i) = shadowed();
        let west = w.game.tower("west");
        assert!(!w.command(i, Order::Take(west), true));
        assert_eq!((w.orders[i], w.lows[i]), (Some(Order::Home), false));
        let j = w.game.drone(Team::Blue, 0);
        assert!(w.command(j, Order::Take(west), true));
        assert_eq!((w.orders[j], w.lows[j]), (Some(Order::Take(west)), true));
    }

    #[test]
    fn a_standing_order_given_in_contact_replaces_the_failsafe() {
        let (mut w, i) = shadowed();
        let west = w.game.tower("west");
        w.game.drones[i].at = Vec2::new(-300.0, 0.0);
        w.game.look();
        let standing = Standing { after: 5.0, then: Order::Hold };
        assert!(w.command_standing(i, Order::Take(west), false, Some(standing)));
        w.game.drones[i].at = w.game.towers[4].at - Vec2::new(100.0, 0.0);
        for _ in 0..49 {
            w.game.step(0.1);
            w.link_up();
        }
        assert_eq!(w.orders[i], Some(Order::Take(west)), "4.9 s");
        for _ in 0..251 {
            w.game.step(0.1);
            w.link_up();
        }
        assert_eq!(w.orders[i], Some(Order::Hold), "30 s: the standing order, and not home");
    }

    #[test]
    fn a_drone_that_lost_its_link_flies_home_and_its_team_still_sees_its_old_order() {
        let (mut w, i) = shadowed();
        let west = w.game.tower("west");
        // Told while it was in contact...
        w.game.drones[i].at = Vec2::new(-300.0, 0.0);
        w.game.look();
        assert!(w.command(i, Order::Take(west), false));
        w.link_up();
        assert_eq!(w.told[i], (Some(Order::Take(west)), false));
        // ...then the shadow: it flies on for the timeout, and then the failsafe takes it home.
        w.game.drones[i].at = w.game.towers[4].at - Vec2::new(100.0, 0.0);
        for _ in 0..449 {
            w.game.step(0.1);
            w.link_up();
        }
        assert_eq!(w.orders[i], Some(Order::Take(west)), "44.9 s");
        w.game.step(0.1);
        w.link_up();
        assert_eq!(w.orders[i], Some(Order::Home));
        assert!(!w.lows[i]);
        assert_eq!(w.told[i], (Some(Order::Take(west)), false), "its team is not told");
        // Out of the shadow, it hears the team again and gets its order back.
        w.game.drones[i].at = Vec2::new(-300.0, 0.0);
        w.game.step(0.1);
        assert!(w.command(i, Order::Take(west), false));
    }

    #[test]
    fn a_drone_that_flies_into_the_mountains_crashes() {
        let mut w = world();
        let edge = Vec2::new(0.0, 600.0);
        let height = w.game.map.mountain_height(edge);
        assert!(height > 100.0);
        // Flying at 50 m, below the ridge, at the foot of it, with its motors at hover.
        w.planes[0].pos = to_world(edge, 50.0);
        w.planes[0].phase = Phase::Flying;
        let pwm = [1500u16; 16];
        for _ in 0..400 {
            w.step_drone(0, &pwm, 0.0025);
        }
        assert_eq!(w.planes[0].phase, Phase::Crashed, "{:?}", w.planes[0].crash_reason);
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
