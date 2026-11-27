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
use recreation_towers::seat::{Mission, STICKS_TIMEOUT, Sticks};
use recreation_towers::{
    Class, Game, Instruction, Link, Order, Phase as Referee, Queued, Rules, Standing, Team, queue_of,
};
use recreation_towers_ground::{drone_name, leg_for};
use simulation_aircraft::{ActuatorCommands, AircraftParams};
use simulation_physics::Obstacle;
use simulation_physics::{Environment, FlightState, Phase, Start, step};
use std::collections::VecDeque;
use std::time::Instant;

/// A new pilot's sticks are the drone's this long (wall seconds) after the pilot took the seat: the
/// ground station of the team is told at once, and has this long to leave the drone alone.
pub const HANDOVER: f32 = 1.0;
/// A pilot who takes a drone that flies in AGENT sends it a radio mode that is not FBWA this long
/// (s of the game) before FBWA: ArduPilot only changes mode when the switch moves.
const MODE_PULSE: f32 = 1.5;

/// A crashed drone is back at its base this long after the crash (s).
pub const RESPAWN: f32 = 15.0;
/// The track dump of a crash holds this many seconds of the drone before it, at `TRAIL_EVERY`.
const TRAIL_SECONDS: f32 = 10.0;
const TRAIL_EVERY: f32 = 0.1;
/// The plan's leg shown in a sample is worked out this often (s): it is the plan the ground sends.
const LEG_EVERY: f32 = 0.5;

/// What a drone was doing at one moment: a line of a crash's track dump.
#[derive(Clone, Debug)]
pub struct Sample {
    pub t: f32,
    pub at: Vec2,
    pub alt: f32,
    pub vel: Vec2,
    pub airspeed: f32,
    pub roll: f32,
    pub pitch: f32,
    pub heading: f32,
    /// The executor's `AG_PLAN`, `AG_ENV` and `AG_FALL` (-1: not said).
    pub env: [i32; 3],
    /// The leg of the plan the ground sends for its order: orbit or go, the point, height, radius.
    pub leg: Option<recreation_towers::flight::Leg>,
    pub order: Option<Order>,
    pub low: bool,
    /// The nearest other flying drone: its number, how far across, and how far above (m, below if
    /// negative). A separation escape of the envelope needs one near.
    pub near: Option<(usize, f32, f32)>,
    /// How long it has been out of contact (the radio shadow of an enemy tower), s; 0 in contact.
    pub silent: f32,
}

/// A drone this far from where it started (m) has left its base.
const AWAY: f32 = 100.0;

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
    let at = recreation_towers::base_spot(rules, team, k);
    let heading = if team == Team::Blue { std::f32::consts::PI } else { 0.0 };
    Start { pos: to_world(at, 0.0), heading }
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

/// A person who flies a drone by the sticks (a seat that is `Join::Unit`).
pub struct Pilot {
    /// The seat's connection.
    pub id: u64,
    pub sticks: Controls,
    /// When the sticks were last heard, and when the pilot took the drone (wall time).
    heard: Instant,
    since: Instant,
    /// The game seconds left of the switch that takes a drone out of AGENT (`MODE_PULSE`); `None`
    /// until the sticks are the pilot's. A drone that takes off under its pilot needs none.
    pulse: Option<f32>,
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
    /// Each drone's queue of instructions: what it does when it is out of contact (by its index in
    /// the game), and how many of them it has started. The server flies the whole queue for a
    /// drone with no wing (`link_up`).
    pub queue: Vec<Vec<Queued>>,
    pub queue_taken: Vec<usize>,
    /// Each drone's standing order: the first instruction of its queue that waits for the silence.
    /// This is all that the flight controller has (one standing plan), so it is all that the
    /// ground uploads to a drone on the wing; the rest of the queue is flown from here.
    pub standing: Vec<Option<Standing>>,
    /// The order each team last knew of for each drone, and whether it is low: what the drone
    /// was told while it was in contact. A drone out of contact flies on, and its team is not told
    /// what it flies now (the order is the team's, and the drone's own failsafe is not).
    pub told: Vec<(Option<Order>, bool)>,
    /// The person who flies each drone, if one does (by its index in the game).
    pub pilots: Vec<Option<Pilot>>,
    /// The mission each drone's pilot is shown: the order, whether it is low, and the queue that
    /// last reached the drone, which is the commander's last word while the radio reached it.
    pub shown: Vec<(Option<Order>, bool, Vec<Queued>)>,
    /// The ground: flat, with the mountains.
    terrain: Valley,
    /// The trees, for the flight model, and the ones near the plane being stepped.
    forest: Forest,
    near: Vec<Obstacle>,
    /// GO: when the drones' own time (drone 0's, s) gets here, the game starts.
    pub go: f64,
    pub crashes: u32,
    /// The last seconds of each drone, for the track dump of a crash, and when it was last added to.
    trail: Vec<VecDeque<Sample>>,
    next_trail: Vec<f64>,
    leg: Vec<(f32, Option<recreation_towers::flight::Leg>)>,
    /// What each drone's executor reports (`AG_PLAN`, `AG_ENV`, `AG_FALL`), set by the server.
    pub env: Vec<[i32; 3]>,
    /// The flight recorder (`--record`): every sample of every drone, for the whole game.
    pub recording: Option<Recording>,
    /// The game's record (`--record`, `plans/game-tooling.md`): what the replay page, the video
    /// and the report read, as from the arena.
    pub record: Option<recreation_towers::record::Recorder>,
    /// Towers each team took (Blue's, Red's).
    pub taken: [u32; 2],
    /// When each drone was first 100 m from where it started (the game's time, s).
    pub away: Vec<Option<f32>>,
    /// The fastest each drone has flown over the ground (m/s).
    pub top_speed: Vec<f32>,
    /// Physics packets from each drone's SITL.
    pub packets: Vec<u64>,
    /// The game time at which each drone's sensor next reports (s), and the sequence of its frames.
    next_sense: Vec<f32>,
    sense_seq: Vec<u8>,
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
            queue: vec![vec![]; n],
            queue_taken: vec![0; n],
            standing: vec![None; n],
            pilots: (0..n).map(|_| None).collect(),
            shown: vec![(Some(Order::Home), false, vec![]); n],
            terrain,
            forest,
            near: vec![],
            go,
            crashes: 0,
            trail: vec![VecDeque::new(); n],
            next_trail: vec![0.0; n],
            leg: vec![(f32::MIN, None); n],
            env: vec![[-1; 3]; n],
            recording: None,
            record: None,
            taken: [0; 2],
            away: vec![None; n],
            top_speed: vec![0.0; n],
            packets: vec![0; n],
            next_sense: vec![0.0; n],
            sense_seq: vec![0; n],
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
        self.sync(i);
        self.record(i);
        let radio = self.radio(i, dt);
        if i == 0 {
            // Before GO the game stands still, but the drones still hear their orders (and the
            // commander sees them).
            if !self.started() {
                self.link_up();
                return radio;
            }
            let before: Vec<_> = self.game.towers.iter().map(|t| t.holder).collect();
            self.game.step(dt);
            if let Some(rec) = &mut self.record {
                // The computer commanders' boards are at their seats, not here (yet).
                rec.step(&self.game, &self.orders, [None, None]);
            }
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

    /// Adds a sample of drone `i` to its trail, ten times a second of the game.
    fn record(&mut self, i: usize) {
        let s = &self.planes[i];
        let crashed = s.phase == Phase::Crashed;
        if (crashed && self.down_for[i] > 0.0) || (!crashed && !airborne(s)) {
            self.trail[i].clear();
            return;
        }
        // The step that crashes the drone is the last sample, whenever it falls.
        if s.time < self.next_trail[i] && !(crashed && self.down_for[i] == 0.0) {
            return;
        }
        self.next_trail[i] = s.time + TRAIL_EVERY as f64;
        let t = self.game.time;
        if t - self.leg[i].0 >= LEG_EVERY || t < self.leg[i].0 {
            let order = self.orders[i].unwrap_or(Order::Home);
            let leg = (self.game.drones[i].phase == Referee::Flying && !self.is_quad(i))
                .then(|| leg_for(&self.game, i, order, self.lows[i]));
            self.leg[i] = (t, leg);
        }
        let (roll, pitch, heading) = {
            let (r, p, y) = simulation_physics::euler(s.rot);
            (r.to_degrees(), p.to_degrees(), y.to_degrees())
        };
        let (at, alt) = to_field(s.pos);
        let near = (0..self.game.drones.len())
            .filter(|&j| j != i && self.game.drones[j].phase == Referee::Flying)
            .map(|j| {
                let d = &self.game.drones[j];
                (j, d.at.distance(at), d.alt - alt)
            })
            .min_by(|a, b| a.1.hypot(a.2).total_cmp(&b.1.hypot(b.2)));
        let sample = Sample {
            silent: self.game.silent[i],
            near,
            t,
            at,
            alt,
            vel: vel_to_field(s.vel),
            airspeed: s.airspeed,
            roll,
            pitch,
            heading,
            env: self.env[i],
            leg: self.leg[i].1,
            order: self.orders[i],
            low: self.lows[i],
        };
        if let Some(r) = &mut self.recording {
            r.write(i, &sample);
        }
        let trail = &mut self.trail[i];
        trail.push_back(sample);
        while trail.len() as f32 * TRAIL_EVERY > TRAIL_SECONDS {
            trail.pop_front();
        }
    }

    fn is_quad(&self, i: usize) -> bool {
        self.game.drones[i].class == Class::Light
    }

    /// The track dump of drone `i`'s crash: how it was flying, and the tree it hit.
    fn dump_crash(&self, i: usize) {
        let d = &self.game.drones[i];
        let (at, alt) = to_field(self.planes[i].pos);
        let tree = self
            .near
            .iter()
            .filter(|o| o.top > self.planes[i].pos.y)
            .map(|o| {
                let (tat, _) = to_field(o.pos);
                (tat.distance(at) - o.radius, tat, o.top, o.radius)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let towers: Vec<String> =
            self.game.towers.iter().enumerate().map(|(k, t)| format!("{k}:{:.0}", t.at.distance(at))).collect();
        let clearing = self.game.map.nearest_clearing(at);
        println!(
            "crashinfo: drone {i} {:?} {} t {:.1} reason {:?} order {:?} low {} at ({:.0}, {:.0}) alt {alt:.1} | tower dist {} | nearest clearing at ({:.0}, {:.0}) r {:.0} dist {:.0} in_clearing {} open {} | tree {}",
            d.class,
            drone_name(&self.game.rules, i),
            self.game.time,
            self.planes[i].crash_reason,
            self.orders[i],
            self.lows[i],
            at.x,
            at.y,
            towers.join(" "),
            clearing.at.x,
            clearing.at.y,
            clearing.radius,
            clearing.at.distance(at),
            self.game.map.in_clearing(at),
            self.game.map.open(at),
            tree.map_or("none".into(), |(gap, tat, top, r)| format!(
                "gap {gap:.1} m at ({:.0}, {:.0}) top {top:.1} r {r:.1}",
                tat.x, tat.y
            )),
        );
        for s in &self.trail[i] {
            let tower = match s.order {
                Some(Order::Take(k)) => Some((k, self.game.towers[k].at)),
                _ => None,
            };
            let clearing = self.game.map.nearest_clearing(s.at);
            println!(
                "track {i} t {:.1} at {:.0} {:.0} alt {:.1} gs {:.1} as {:.1} vel {:.1} {:.1} roll {:.0} pitch {:.0} hdg {:.0} env {:?} order {:?} low {} tower {} leg {} clearing_edge {:.0} near {} silent {:.1}",
                s.t,
                s.at.x,
                s.at.y,
                s.alt,
                s.vel.length(),
                s.airspeed,
                s.vel.x,
                s.vel.y,
                s.roll,
                s.pitch,
                s.heading,
                s.env,
                s.order,
                s.low,
                tower.map_or("-".into(), |(k, p)| format!("{k} d {:.0}", p.distance(s.at))),
                s.leg.map_or("-".into(), |l| format!(
                    "{} to ({:.0}, {:.0}) alt {:.1} r {:.0} dist {:.0}",
                    if l.orbit { "ORBIT" } else { "GO" },
                    l.point.x,
                    l.point.y,
                    l.alt,
                    l.radius,
                    l.point.distance(s.at)
                )),
                clearing.radius - clearing.at.distance(s.at),
                s.near.map_or("-".into(), |(j, across, above)| format!("{j} {across:.0} {above:+.0}")),
                s.silent,
            );
        }
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
    #[cfg(test)]
    pub fn command(&mut self, i: usize, order: Order, low: bool) {
        self.command_standing(i, order, low, None)
    }

    /// The team gives drone `i` an order and a standing order.
    #[cfg(test)]
    pub fn command_standing(&mut self, i: usize, order: Order, low: bool, standing: Option<Standing>) {
        self.command_queue(i, order, low, queue_of(standing, vec![]))
    }

    /// The team gives drone `i` an order and a queue (cut to the most that a queue holds). The
    /// order and the queue replace the old ones.
    pub fn command_queue(&mut self, i: usize, order: Order, low: bool, queue: Vec<Queued>) {
        let queue = queue_of(None, queue);
        self.orders[i] = Some(order);
        self.lows[i] = low;
        self.standing[i] = Instruction { order: None, queue: queue.clone(), taken: 0 }.standing();
        self.queue[i] = queue;
        self.queue_taken[i] = 0;
    }

    /// What drone `i`'s sensor reports now, once a second of game time: each enemy drone in its
    /// range, as an ADSB_VEHICLE frame from the drone (see `cognition_agent::link::sighting_frame`).
    pub fn sense(&mut self, i: usize) -> Vec<Vec<u8>> {
        if !self.started() || self.game.time < self.next_sense[i] {
            return vec![];
        }
        self.next_sense[i] = self.game.time + 1.0;
        let home = recreation_towers_ground::sitl_home();
        self.game
            .drone_senses(i)
            .into_iter()
            .map(|j| {
                let d = &self.game.drones[j];
                let craft = cognition_agent::Craft {
                    id: j as u8 + 1,
                    pos: Vec3::new(d.at.x, d.at.y, -d.alt),
                    vel: Vec3::new(d.vel.x, d.vel.y, 0.0),
                    flying: true,
                };
                let t = cognition_agent::link::Traffic::from_craft(&craft, home, "SEEN");
                self.sense_seq[i] = self.sense_seq[i].wrapping_add(1);
                cognition_agent::link::sighting_frame(i as u8 + 1, self.sense_seq[i], j, &t)
            })
            .collect()
    }

    /// The drone has no wing for the on-board executor to fly (a quadcopter), so its order is flown from
    /// here by its radio, and the standing order is carried out here too.
    pub fn flown_from_here(&self, i: usize) -> bool {
        self.game.drones[i].class == recreation_towers::Class::Light
    }

    /// What the drones' radios do after a step of the referee: a drone in contact tells its team
    /// its order; one that is out of contact long enough flies its standing order, or, with none,
    /// home (the failsafe), and stays on that order until the team can give it another.
    pub(crate) fn link_up(&mut self) {
        for i in 0..self.orders.len() {
            // The mission on a pilot's screen is what reached the drone: it does not change in a
            // radio shadow, and catches up when the radio reaches the drone again.
            if self.in_reach(i) {
                self.shown[i] = (self.orders[i], self.lows[i], self.queue[i].clone());
            }
            if self.game.link(i) == Link::Up {
                self.told[i] = (self.orders[i], self.lows[i]);
                continue;
            }
            // A drone on the wing flies its standing order itself (the ground sends it ahead:
            // `ground.rs`). Only a drone with no wing, flown from here by its radio, needs this.
            if !self.flown_from_here(i) {
                continue;
            }
            let mut flown =
                Instruction { order: self.orders[i], queue: self.queue[i].clone(), taken: self.queue_taken[i] };
            let game = &self.game;
            flown.tick_with(game.silent[i], game.rules.link_timeout, game.charge(i), game.rules.bingo, |o| {
                game.order_done(i, o)
            });
            self.queue_taken[i] = flown.taken;
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
                if let Some(r) = &mut self.recording {
                    r.crash(i, self.game.time, &self.planes[i].crash_reason, at, alt);
                }
                self.dump_crash(i);
                // What it was doing, and who was near: to tell a mid-air from a tree, a drone in a
                // shadow from one in contact.
                let here = glam::Vec3::new(at.x, at.y, alt);
                let near = (0..self.game.drones.len())
                    .filter(|&j| j != i && self.game.drones[j].phase == Referee::Flying)
                    .map(|j| {
                        let d = &self.game.drones[j];
                        (j, glam::Vec3::new(d.at.x, d.at.y, d.alt).distance(here))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                println!(
                    "crash: drone {i}: t {:.0}s order {:?}{} speed {:.1} m/s, jammed {}, silent {:.1}s, nearest flying drone {}",
                    self.game.time,
                    self.orders[i],
                    if self.lows[i] { " LOW" } else { " HIGH" },
                    self.planes[i].vel.length(),
                    self.game.jammed[i],
                    self.game.silent[i],
                    near.map_or("none".into(), |(j, d)| format!("{j} at {d:.0} m")),
                );
            }
            self.down_for[i] += dt;
            if self.down_for[i] > RESPAWN {
                self.down_for[i] = 0.0;
                let start = World::start_of(&self.game, i);
                self.planes[i].reset_to_hand(p, &start);
                // Back at its base it is as it was at the start of the game: with its battery full
                // (the plane's own pack, which sags: 17.4 V at rest and 15.0 V under load at the
                // take-off transition after 400 mAh spent). "Hard impact" at the base after a
                // respawn was 9 of 33 crashes in 14 games (and 1 in 16 later, none with the old
                // code in 16 quieter ones): the take-off transition dives (pitch -33 degrees at 9
                // m/s) and a respawned drone did not always come out of it. Not shown to be the pack.
                self.planes[i].recharge();
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
        match self.preflight[i].radio(p, s, agl, dt) {
            Some(radio) => {
                // A pilot who has the drone before it is on the wing needs no switch to get it out
                // of AGENT: it has not been in it.
                if let Some(pilot) = self.pilots[i].as_mut() {
                    pilot.pulse.get_or_insert(0.0);
                }
                radio
            }
            None => self.on_the_wing(i, dt),
        }
    }

    /// What drone `i`'s radio sends once it is on the wing: the pilot's sticks, if a person flies
    /// it; else the sticks of the server's quad, or the sticks centred for the AGENT mode.
    fn on_the_wing(&mut self, i: usize, dt: f32) -> Radio {
        if let Some(radio) = self.pilot_radio(i, dt) {
            return radio;
        }
        let p = &self.params[i];
        let s = &self.planes[i];
        {
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
        }
    }

    /// Takes the wing of drone `i` for the pilot `id`, or says why not.
    pub fn claim_pilot(&mut self, i: usize, id: u64) -> Result<(), String> {
        let name = drone_name(&self.game.rules, i);
        if self.game.drones[i].class == Class::Light {
            return Err(format!("{name} is a quad: the server flies it, and only a winged drone has a pilot"));
        }
        if self.pilots[i].is_some() {
            return Err(format!("{name} has its pilot"));
        }
        let now = Instant::now();
        self.pilots[i] = Some(Pilot {
            id,
            sticks: Controls { throttle: 0.5, ..Default::default() },
            heard: now,
            since: now,
            pulse: None,
        });
        println!("pilot: {name} is flown by a person");
        Ok(())
    }

    /// The pilot `id` sends its sticks. A pilot whose sticks had stopped, and whose drone nobody
    /// else has taken, has it again.
    pub fn fly(&mut self, i: usize, id: u64, sticks: Sticks) {
        if self.pilots[i].is_none() {
            let _ = self.claim_pilot(i, id);
        }
        if let Some(pilot) = self.pilots[i].as_mut().filter(|p| p.id == id) {
            let s = sticks.clamped();
            pilot.sticks = Controls { aileron: s.roll, elevator: s.pitch, rudder: s.yaw, throttle: s.throttle };
            pilot.heard = Instant::now();
        }
    }

    /// The pilot `id` has gone: the drone is the computer's again.
    pub fn release_pilot(&mut self, i: usize, id: u64) {
        if self.pilots[i].as_ref().is_some_and(|p| p.id == id) {
            self.pilots[i] = None;
            println!("pilot: {} is given back to the computer: the seat is gone", drone_name(&self.game.rules, i));
        }
    }

    /// Gives back the drones whose pilots' sticks have stopped for [`STICKS_TIMEOUT`] s.
    pub fn expire_pilots(&mut self) {
        for i in 0..self.pilots.len() {
            if self.pilots[i].as_ref().is_some_and(|p| p.heard.elapsed().as_secs_f32() > STICKS_TIMEOUT) {
                self.pilots[i] = None;
                println!(
                    "pilot: {} is given back to the computer: the sticks stopped",
                    drone_name(&self.game.rules, i)
                );
            }
        }
    }

    /// The drones of `team` (by their number in the team) that a person flies now.
    pub fn piloted(&self, team: Team) -> Vec<usize> {
        let n = self.game.rules.drones;
        (0..n).filter(|&k| self.pilots[self.game.drone(team, k)].is_some()).collect()
    }

    /// What the radio of drone `i` sends when its pilot flies it: the pilot's sticks, the mode
    /// switch at FBWA; `None` while the drone is the computer's. For [`HANDOVER`] s after the pilot
    /// takes it the computer's station still has it; then, if it is in AGENT, the mode switch moves
    /// to another mode for a moment, so that the switch's return to FBWA is a change ArduPilot takes.
    fn pilot_radio(&mut self, i: usize, dt: f32) -> Option<Radio> {
        self.expire_pilots();
        let params = &self.params[i];
        let pilot = self.pilots[i].as_mut()?;
        if pilot.since.elapsed().as_secs_f32() < HANDOVER {
            return None;
        }
        let pulse = pilot.pulse.get_or_insert(MODE_PULSE);
        let fbwa = flying_slot(params);
        if *pulse > 0.0 {
            *pulse -= dt;
            let other =
                params.rc_modes.iter().position(|m| *m == "CRUISE").unwrap_or((fbwa + 1) % params.rc_modes.len());
            return Some(Radio { sticks: Controls { throttle: 0.5, ..Default::default() }, arm: true, mode: other });
        }
        Some(Radio { sticks: pilot.sticks, arm: true, mode: fbwa })
    }

    /// What drone `i`'s pilot is shown: the mission and what the drone itself knows.
    pub fn mission(&self, i: usize) -> Mission {
        let game = &self.game;
        let d = &game.drones[i];
        Mission {
            drone: i % game.rules.drones,
            order: self.shown[i].0,
            low: self.shown[i].1,
            queue: self.shown[i].2.clone(),
            in_contact: self.in_reach(i),
            obs: game.edge_obs(i),
            vel: d.vel,
            attitude: self.planes[i].rot.to_array(),
            towers: game.towers.iter().map(|t| (t.name.clone(), t.at)).collect(),
            time: game.time,
            time_left: game.time_left(),
            outcome: game.outcome,
        }
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

/// The flight recorder of a game (`--record DIR`): `tracks.tsv`, a line for every drone ten times a
/// second from take-off to the end (what the crash dump shows of the last 10 s, for the whole game),
/// and `crashes.tsv`, a line for each crash. ArduPilot's own log of each drone, with the executor's
/// every step (`AGNT`: its demand, the envelope's rules and the escape), is copied beside them at the
/// end (see `main.rs`).
pub struct Recording {
    tracks: std::io::BufWriter<std::fs::File>,
    crashes: std::io::BufWriter<std::fs::File>,
}

impl Recording {
    pub const TRACKS: &str = "t\tdrone\tx\ty\talt\tvx\tvy\tairspeed\troll\tpitch\theading\tplan\tenv\tfall\torder\tlow\tleg_orbit\tleg_x\tleg_y\tleg_alt\tleg_r\tnear\tnear_d\tnear_dz\tsilent";

    pub fn create(dir: &std::path::Path) -> std::io::Result<Recording> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        let mut tracks = std::io::BufWriter::new(std::fs::File::create(dir.join("tracks.tsv"))?);
        writeln!(tracks, "{}", Self::TRACKS)?;
        let mut crashes = std::io::BufWriter::new(std::fs::File::create(dir.join("crashes.tsv"))?);
        writeln!(crashes, "t\tdrone\treason\tx\ty\talt")?;
        Ok(Recording { tracks, crashes })
    }

    fn write(&mut self, i: usize, s: &Sample) {
        use std::io::Write;
        let leg = s.leg.map_or("\t\t\t\t".to_string(), |l| {
            format!("{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}", l.orbit as u8, l.point.x, l.point.y, l.alt, l.radius)
        });
        let near = s.near.map_or("\t\t".to_string(), |(j, d, dz)| format!("{j}\t{d:.1}\t{dz:.1}"));
        let _ = writeln!(
            self.tracks,
            "{:.1}\t{i}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.0}\t{:.0}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{leg}\t{near}\t{:.1}",
            s.t,
            s.at.x,
            s.at.y,
            s.alt,
            s.vel.x,
            s.vel.y,
            s.airspeed,
            s.roll,
            s.pitch,
            s.heading,
            s.env[0],
            s.env[1],
            s.env[2],
            s.order.map_or("-".to_string(), |o| format!("{o:?}")),
            s.low as u8,
            s.silent,
        );
    }

    fn crash(&mut self, i: usize, t: f32, reason: &str, at: Vec2, alt: f32) {
        use std::io::Write;
        let _ = writeln!(self.crashes, "{t:.1}\t{i}\t{reason}\t{:.1}\t{:.1}\t{alt:.1}", at.x, at.y);
    }

    pub fn flush(&mut self) {
        use std::io::Write;
        let _ = self.tracks.flush();
        let _ = self.crashes.flush();
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
    fn a_pilots_sticks_are_the_drones_radio_after_the_handover_and_a_drone_in_agent_is_taken_out_of_it() {
        let mut w = world();
        let i = w.game.drone(Team::Blue, 1);
        let fbwa = flying_slot(&w.params[i]);
        let sticks = Sticks { roll: 0.5, pitch: -0.25, throttle: 0.8, yaw: -0.5 };
        w.claim_pilot(i, 7).unwrap();
        w.fly(i, 7, sticks);
        // For the first second the computer's station still has the drone: sticks centred, FBWA.
        let r = w.on_the_wing(i, 0.1);
        assert_eq!((r.sticks.aileron, r.mode), (0.0, fbwa));
        // Then the sticks are the pilot's. The drone may be in AGENT: the mode switch leaves FBWA
        // for a moment (1.5 s of the game) and comes back, which is a change that ArduPilot takes.
        w.pilots[i].as_mut().unwrap().since = Instant::now() - std::time::Duration::from_secs(2);
        let r = w.on_the_wing(i, 0.5);
        assert_ne!(r.mode, fbwa, "the switch moves");
        assert_eq!(r.sticks.aileron, 0.0, "the sticks are centred meanwhile");
        for _ in 0..3 {
            w.on_the_wing(i, 0.5);
        }
        let r = w.on_the_wing(i, 0.5);
        assert_eq!(r.mode, fbwa);
        assert!(r.arm);
        assert_eq!((r.sticks.aileron, r.sticks.elevator, r.sticks.throttle, r.sticks.rudder), (0.5, -0.25, 0.8, -0.5));
        // A pilot whose sticks have stopped for good gives the drone back: centred again.
        w.pilots[i].as_mut().unwrap().heard = Instant::now() - std::time::Duration::from_secs(3);
        let r = w.on_the_wing(i, 0.1);
        assert_eq!((r.sticks.aileron, r.sticks.throttle, r.mode), (0.0, 0.5, fbwa));
        assert!(w.pilots[i].is_none());
    }

    #[test]
    fn a_pilot_who_took_the_drone_before_the_wing_needs_no_switch() {
        let mut w = world();
        let i = w.game.drone(Team::Blue, 0);
        w.claim_pilot(i, 1).unwrap();
        w.fly(i, 1, Sticks { roll: 1.0, ..Sticks::CENTRED });
        w.pilots[i].as_mut().unwrap().since = Instant::now() - std::time::Duration::from_secs(2);
        // The drone is still taking off: the preflight flies it, and the pilot has not been in AGENT.
        w.pilots[i].as_mut().unwrap().pulse.get_or_insert(0.0);
        let r = w.on_the_wing(i, 0.1);
        assert_eq!((r.sticks.aileron, r.mode), (1.0, flying_slot(&w.params[i])));
    }

    #[test]
    fn a_quad_has_no_pilot_and_a_drone_has_one() {
        let mut w = world();
        let quad = w.game.drone(Team::Red, 4);
        assert!(w.claim_pilot(quad, 1).unwrap_err().contains("quad"));
        let i = w.game.drone(Team::Red, 0);
        assert!(w.claim_pilot(i, 1).is_ok());
        assert!(w.claim_pilot(i, 2).unwrap_err().contains("has its pilot"));
        // Another seat's sticks are not the pilot's.
        w.fly(i, 2, Sticks { roll: 1.0, ..Sticks::CENTRED });
        assert_eq!(w.pilots[i].as_ref().unwrap().sticks.aileron, 0.0);
        w.release_pilot(i, 2);
        assert!(w.pilots[i].is_some(), "a seat gives up only its own drone");
        w.release_pilot(i, 1);
        assert_eq!(w.piloted(Team::Red), Vec::<usize>::new());
    }

    #[test]
    fn the_mission_a_pilot_is_shown_is_the_last_order_that_reached_the_drone() {
        let mut w = world();
        let i = w.game.drone(Team::Blue, 0);
        w.command(i, Order::Take(1), false);
        w.link_up();
        assert_eq!(w.mission(i).order, Some(Order::Take(1)));
        // Out of reach (a shadow): the team gives another order, and the mission stays.
        let red_home = w.game.tower("Red home");
        w.game.towers[red_home].holder = Some(Team::Red);
        w.game.drones[i].at = w.game.towers[red_home].at;
        w.game.drones[i].phase = Referee::Flying;
        w.game.look();
        w.command(i, Order::Hold, true);
        w.link_up();
        let m = w.mission(i);
        assert_eq!((m.order, m.low, m.in_contact), (Some(Order::Take(1)), false, false));
        assert!(m.obs.silent >= 0.0);
        w.game.drones[i].at = w.game.rules.base(Team::Blue);
        w.game.look();
        w.link_up();
        let m = w.mission(i);
        assert_eq!((m.order, m.low, m.in_contact), (Some(Order::Hold), true, true));
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
    fn a_take_order_circles_the_tower_inside_its_circle_low_at_the_drones_own_height() {
        let mut w = world();
        let rules = w.game.rules;
        let home = (-35.363261, 149.165230);
        let west = w.game.tower("west");
        // A drone that is up (a drone on the ground climbs over its base first).
        let red = w.game.drone(Team::Red, 1);
        w.game.drones[red].alt = 60.0;
        w.game.drones[red].phase = Referee::Flying;
        let plan = plan_for(&w.game, red, Order::Take(west), false, home, 7);
        assert_eq!(
            (plan.primitive, plan.radius, plan.id, plan.alt),
            (Primitive::Orbit, flight::orbit_radius(Team::Red), 7, 67.0)
        );
        assert!(plan.radius < rules.circle_radius);
        let (n, e) = cognition_plan::to_local(home.0, home.1, plan.lat, plan.lon);
        let at = w.game.towers[west].at;
        assert!((n - at.x).abs() < 0.1 && (e - at.y).abs() < 0.1, "{n} {e}");
        // On its circle at the tower it comes down below the treetops: only a low drone takes it.
        w.game.drones[red].at = at + Vec2::new(flight::orbit_radius(Team::Red), 0.0);
        let plan = plan_for(&w.game, red, Order::Take(west), false, home, 8);
        assert_eq!(plan.alt, flight::low_height(Team::Red, 1));
        assert!(plan.alt < rules.circle_ceiling);
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
        assert_eq!((low.primitive, low.alt), (Primitive::Goto, flight::low_height(Team::Blue, 1)));
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
    fn a_queue_given_in_contact_is_flown_down_in_the_shadow_and_a_longer_one_is_cut() {
        let (mut w, i) = shadowed_light();
        let west = w.game.tower("west");
        w.game.drones[i].at = Vec2::new(-300.0, 0.0);
        w.game.look();
        let queue = vec![
            Queued::silent(5.0, Order::Hold),
            Queued::silent(10.0, Order::Take(west)),
            Queued::silent(20.0, Order::Home),
        ];
        w.command_queue(i, Order::Take(west), false, queue.clone());
        assert_eq!(w.standing[i], Some(Standing { after: 5.0, then: Order::Hold }), "the flight controller's one slot");
        w.game.drones[i].at = w.game.towers[4].at - Vec2::new(100.0, 0.0);
        for (secs, order) in
            [(4.9, Order::Take(west)), (5.2, Order::Hold), (10.2, Order::Take(west)), (20.2, Order::Home)]
        {
            while w.game.silent[i] < secs - 0.05 {
                w.game.step(0.1);
                w.link_up();
            }
            assert_eq!(w.orders[i], Some(order), "{secs} s");
        }
        w.command_queue(i, Order::Hold, false, vec![Queued::done(Order::Home); 7]);
        assert_eq!(w.queue[i].len(), recreation_towers::MAX_QUEUE);
        assert_eq!(w.queue_taken[i], 0, "a new order starts the queue again");
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
        assert_eq!(w.lows[i], w.low_by_default(i), "a light drone flies home low");
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
    fn a_crashed_drone_is_back_at_its_base_with_a_full_battery() {
        let mut w = world();
        w.planes[0].phase = Phase::Crashed;
        w.planes[0].used_mah = 600.0;
        // Down for as long as it takes to be brought back.
        w.down_for[0] = RESPAWN;
        w.radio(0, 0.05);
        assert_ne!(w.planes[0].phase, Phase::Crashed);
        assert_eq!(w.planes[0].used_mah, 0.0);
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
