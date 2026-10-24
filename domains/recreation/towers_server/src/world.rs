//! The world of a game: each drone's plane on the flight model, its pilot's radio for the
//! take-off, and the referee. Each SITL drone's physics thread steps its own plane here; drone 0's
//! also steps the referee, by the same time.
//!
//! Frames: the referee's field is north and east of its centre, and the centre is SITL's home.
//! The flight model's world is x east, y up, z south (`cognition_agent::world_to_ned`).

use crate::terrain::Valley;
use crate::trees::Forest;
use glam::{Vec2, Vec3};
use recreation_games::{Controls, Preflight, Radio, flying_slot, mode_slot};
use recreation_towers::{Game, Instruction, Link, Order, Phase as Referee, Rules, Standing, Team};
use recreation_towers_ground::leg_for;
use simulation_aircraft::{ActuatorCommands, AircraftParams};
use simulation_physics::Obstacle;
use simulation_physics::{Environment, FlightState, Phase, Start, step};

/// A crashed drone is back at its base this long after the crash (s).
pub const RESPAWN: f32 = 15.0;

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

/// The fastest a hovering drone is flown (m/s): its QLOITER's full stick (`Q_LOIT_SPEED`).
const HOVER_SPEED: f32 = 5.0;
/// How fast it climbs at full throttle stick (m/s): `Q_PILOT_SPD_UP`.
const HOVER_CLIMB: f32 = 3.0;

/// The sticks that fly a drone that hovers (no wing, in QLOITER) to a point: the roll and pitch
/// sticks ask for a speed relative to its nose, and the throttle stick for a climb, as a pilot's do.
/// `at`: north, east of the field's centre; `climbing`: its climb rate (m/s); `heading`: clockwise
/// from north.
pub fn hover_sticks(at: Vec2, alt: f32, climbing: f32, heading: f32, to: Vec2, to_alt: f32) -> Controls {
    let err = to - at;
    // Slow down as it comes in: the speed it can lose in the distance left.
    let speed = (0.3 * err.length()).min(HOVER_SPEED);
    let want = if err.length() > 0.1 { err.normalize() * speed } else { Vec2::ZERO };
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
        let planes = (0..n).map(|i| FlightState::new(&params[i], &World::start_of(&game, i))).collect();
        World {
            game,
            params,
            planes,
            preflight: vec![Preflight::default(); n],
            down_for: vec![0.0; n],
            orders: vec![Some(Order::Home); n],
            lows: vec![false; n],
            standing: vec![None; n],
            told: vec![(Some(Order::Home), false); n],
            terrain,
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

    /// The radio reaches drone `i` now: the air carries its frames. A drone in a radio shadow, or
    /// out of the radio's range of its team's base, is out of reach.
    pub fn in_reach(&self, i: usize) -> bool {
        let d = &self.game.drones[i];
        !self.game.jammed[i]
            && d.at.distance(self.game.rules.base(d.team)) < communication_air::Technology::MavlinkNetwork.range()
    }

    /// The team gives drone `i` an order. The team does not know whether it is heard: the order is
    /// the team's, and the air carries it to the drone, or drops it (see `air`).
    pub fn command(&mut self, i: usize, order: Order, low: bool) {
        self.command_standing(i, order, low, None)
    }

    /// The team gives drone `i` an order and a standing order.
    pub fn command_standing(&mut self, i: usize, order: Order, low: bool, standing: Option<Standing>) {
        self.orders[i] = Some(order);
        self.lows[i] = low;
        self.standing[i] = standing;
    }

    /// What the ground sees of the game: all of it.
    pub fn situation(&self) -> recreation_towers_ground::Situation<'_> {
        recreation_towers_ground::Situation {
            now: self.now(),
            game: &self.game,
            orders: &self.orders,
            lows: &self.lows,
            standing: &self.standing,
            wing: (0..self.orders.len()).map(|i| self.on_the_wing(i)).collect(),
        }
    }

    /// The drone has no wing for the on-board executor to fly (a quadcopter), so its order is flown from
    /// here by its radio, and the standing order is carried out here too.
    pub fn flown_from_here(&self, i: usize) -> bool {
        self.game.drones[i].class == recreation_towers::Class::Light
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
            // A drone on the wing flies its standing order itself (the ground sends it ahead:
            // `ground.rs`). Only a drone with no wing, flown from here by its radio, needs this.
            if !self.flown_from_here(i) {
                continue;
            }
            let mut flown = Instruction { order: self.orders[i], standing: self.standing[i] };
            flown.tick(self.game.silent[i], self.game.rules.link_timeout, self.game.charge(i), self.game.rules.bingo);
            if flown.order != self.orders[i] {
                println!("failsafe: drone {i} out of contact, and flies {:?}", flown.order);
                self.orders[i] = flown.order;
                self.lows[i] = false;
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
                hover_sticks(at, alt, s.vel.y, heading, leg.point, leg.alt)
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
    use cognition_plan::Primitive;
    use recreation_towers::flight;
    use recreation_towers_ground::plan_for;

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
        let s = hover_sticks(Vec2::ZERO, 50.0, 0.0, 0.0, Vec2::new(100.0, 100.0), 80.0);
        assert!(s.elevator < -0.3 && s.aileron > 0.3 && s.throttle > 0.6, "{s:?}");
        // Facing east, the same point is on its left and ahead.
        let s = hover_sticks(Vec2::ZERO, 50.0, 0.0, std::f32::consts::FRAC_PI_2, Vec2::new(100.0, 100.0), 50.0);
        assert!(s.elevator < -0.3 && s.aileron < -0.3 && (s.throttle - 0.5).abs() < 1e-3, "{s:?}");
        // On the point and at height: the sticks are at rest.
        let s = hover_sticks(Vec2::new(5.0, 5.0), 60.0, 0.0, 1.0, Vec2::new(5.0, 5.0), 60.0);
        assert_eq!((s.aileron, s.elevator, s.throttle), (0.0, 0.0, 0.5));
    }

    /// Blue's drone `k` flies in the radio shadow of the Red home tower.
    fn shadowed_k(k: usize) -> (World, usize) {
        let mut w = world();
        let i = w.game.drone(Team::Blue, k);
        w.game.drones[i].at = w.game.towers[4].at - Vec2::new(100.0, 0.0);
        w.game.drones[i].alt = 50.0;
        w.game.drones[i].phase = Referee::Flying;
        w.game.look();
        (w, i)
    }

    /// A drone on the wing in a shadow.
    fn shadowed() -> (World, usize) {
        shadowed_k(1)
    }

    /// A drone with no wing in a shadow: the server flies its order.
    fn shadowed_light() -> (World, usize) {
        let k = (0..Rules::default().drones)
            .find(|&k| Rules::default().class(k) == recreation_towers::Class::Light)
            .expect("a light drone");
        shadowed_k(k)
    }

    #[test]
    fn the_air_carries_to_a_drone_in_contact_and_not_to_one_in_a_radio_shadow() {
        let (mut w, i) = shadowed();
        assert!(!w.in_reach(i));
        let j = w.game.drone(Team::Blue, 0);
        assert!(w.in_reach(j));
        // Out of the radio's range, a drone in no shadow is out of reach too.
        w.game.drones[j].at = Vec2::new(-10_000.0, 0.0);
        w.game.look();
        assert!(!w.in_reach(j));
    }

    #[test]
    fn an_order_is_the_teams_whether_or_not_the_drone_hears_it() {
        let (mut w, i) = shadowed();
        let west = w.game.tower("west");
        w.command(i, Order::Take(west), true);
        assert_eq!((w.orders[i], w.lows[i]), (Some(Order::Take(west)), true));
        assert_eq!(w.told[i].0, Some(Order::Home), "its team is told only what the drone heard");
    }

    #[test]
    fn a_standing_order_given_in_contact_replaces_the_failsafe() {
        let (mut w, i) = shadowed_light();
        let west = w.game.tower("west");
        w.game.drones[i].at = Vec2::new(-300.0, 0.0);
        w.game.look();
        let standing = Standing { after: 5.0, then: Order::Hold };
        w.command_standing(i, Order::Take(west), false, Some(standing));
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
    fn a_drone_on_the_wing_keeps_the_order_the_team_gave_it_for_it_flies_its_standing_order_itself() {
        let (mut w, i) = shadowed();
        let west = w.game.tower("west");
        w.game.drones[i].at = Vec2::new(-300.0, 0.0);
        w.game.look();
        w.command(i, Order::Take(west), false);
        w.game.drones[i].at = w.game.towers[4].at - Vec2::new(100.0, 0.0);
        for _ in 0..600 {
            w.game.step(0.1);
            w.link_up();
        }
        assert_eq!(w.orders[i], Some(Order::Take(west)), "60 s out of contact: the ground only shows what it sent");
    }

    #[test]
    fn a_drone_that_lost_its_link_flies_home_and_its_team_still_sees_its_old_order() {
        let (mut w, i) = shadowed_light();
        let west = w.game.tower("west");
        // Told while it was in contact...
        w.game.drones[i].at = Vec2::new(-300.0, 0.0);
        w.game.look();
        w.command(i, Order::Take(west), false);
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
        w.command(i, Order::Take(west), false);
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
