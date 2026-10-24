//! Towers: two teams of drones fight for radio towers (see plans/towers.md and
//! expectations/09-towers).
//!
//! A drone counts for a tower while it flies in the tower's circle. Each tower has a control,
//! from −1 (Red) to +1 (Blue), that moves towards the team with more drones in the circle; a team
//! holds the tower when the control gets to its end, and loses it when the control gets back to 0.
//! A held tower scores a point a second. Each team sees round its drones, its base and its towers,
//! and keeps a map of what it last saw: the rest of the field is in fog. The field is a forest with
//! lanes ([`map`]): the trees hide a drone that flies below them. A tower that a team holds blocks
//! the radio of the enemy within [`Rules::jam_radius`]: a drone there is out of contact. Tall mountains
//! stand round the edge of the field.
//!
//! The referee only needs where each drone is and whether it flies; the host moves the drones.
//! Positions are on the field: north and east of its centre (m), and a height above it (m).

mod advisor;
mod commander;
pub mod flight;
mod instruction;
pub mod map;
pub mod seat;
pub mod svg;
mod view;

pub use advisor::{Advice, AdviceKind, Advisor};
pub use commander::{Commander, CommanderTuning};
pub use instruction::{Instruction, Standing};
pub use map::Map;
pub use view::{EnemyDrone, Order, OwnDrone, Sites, TeamView, TowerView, Umpire};

use glam::Vec2;
use serde::{Deserialize, Serialize};

/// What a tower is: the map fixes it. The kinds differ in how far their radio shadow goes, how
/// far they see and how many points they give ([`Rules::shadow`], [`Rules::vision`],
/// [`Rules::points`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// A home tower.
    #[default]
    Fort,
    /// The centre tower: the widest shadow, the longest sight and the most points.
    Spire,
    /// A flank tower: the narrowest shadow.
    Depot,
}

/// A side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Team {
    Blue,
    Red,
}

impl Team {
    pub const BOTH: [Team; 2] = [Team::Blue, Team::Red];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn other(self) -> Team {
        match self {
            Team::Blue => Team::Red,
            Team::Red => Team::Blue,
        }
    }

    /// The end of the control bar that is this team's: +1 for Blue, −1 for Red.
    pub fn sign(self) -> f32 {
        match self {
            Team::Blue => 1.0,
            Team::Red => -1.0,
        }
    }
}

/// What a drone is: its weight in a tower's circle, and the plane it flies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Class {
    /// The DeepSpace Seeker 3 quadcopter.
    Light,
    /// The HEEWING T1-S VTOL.
    Standard,
    /// The HEEWING T2 Cruza VTOL.
    Heavy,
}

impl Class {
    /// How many drones one of this class counts for, taking a tower.
    pub fn weight(self) -> f32 {
        match self {
            Class::Light => 0.5,
            Class::Standard => 1.0,
            Class::Heavy => 2.0,
        }
    }

    /// The plane (`simulation_aircraft`) a drone of this class flies.
    pub fn aircraft(self) -> &'static str {
        match self {
            Class::Light => "seeker3",
            Class::Standard => "t1s-vtol",
            Class::Heavy => "t2-cruza",
        }
    }

    /// What the aircraft weighs (kg), as the simulation flies it.
    pub fn mass_kg(self) -> f32 {
        match self {
            Class::Light => 0.28,
            Class::Standard => 1.10,
            Class::Heavy => 3.0,
        }
    }

    /// A word for the commander's map.
    pub fn name(self) -> &'static str {
        match self {
            Class::Light => "light",
            Class::Standard => "standard",
            Class::Heavy => "heavy",
        }
    }
}

/// The numbers of the game. To tune, not measurements.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// The field: a circle round its centre (m), the AGENT fence.
    pub field_radius: f32,
    /// From the centre to each base (m): Blue's to the south, Red's to the north.
    pub base_distance: f32,
    /// From the centre to each home tower (m), towards its base.
    pub home_distance: f32,
    /// From the centre to the flank towers, about (m): the map puts each a little off.
    pub tower_distance: f32,
    /// The height of the trees' crowns, about (m): a drone lower than this is hidden by them.
    pub treetop: f32,
    /// How far an eye above the treetops sees a drone below them, looking down through a lane or
    /// a clearing (m, across).
    pub look_down: f32,
    /// Drones in each team.
    pub drones: usize,
    /// The class of each team's drone `k`: three standard drones, a heavy one and a light one.
    pub squad: [Class; 5],
    /// A tower's circle: a drone counts within this distance across (m)...
    pub circle_radius: f32,
    /// ...and at most this high (m).
    pub circle_ceiling: f32,
    /// One drone more in the circle takes the control from 0 to its end in this time (s).
    pub capture_time: f32,
    /// A circle with nobody in it drifts back across the whole bar in this time (s); 0 for no drift.
    pub decay_time: f32,
    /// The score that wins.
    pub win_score: f32,
    /// The points each second for a held tower: a fort or a depot, and the spire.
    pub tower_score: f32,
    pub spire_score: f32,
    /// How long a game lasts (s).
    pub duration: f32,
    /// A fort that a team holds blocks the radio of the enemy this far round it (m, across), at
    /// all heights. A spire and a depot block it as far as their own radii.
    pub jam_radius: f32,
    pub spire_jam_radius: f32,
    pub depot_jam_radius: f32,
    /// A drone in a radio shadow is in contact if a friend in contact is this near (m, across); 0 for
    /// no relay.
    pub mesh_range: f32,
    /// A relay may be a chain: a drone is in contact through at most this many friends in a row (1:
    /// only a friend that is in contact on its own relays).
    pub mesh_hops: u32,
    /// A drone out of contact this long has lost its link (s): it flies home by itself.
    pub link_timeout: f32,
    /// The battery of a light, a standard and a heavy drone (s of flight); 0 for no battery.
    pub light_battery: f32,
    pub standard_battery: f32,
    pub heavy_battery: f32,
    /// A drone in a tower circle uses this much more battery each second (s) for each unit of
    /// enemy weight in the circle.
    pub contest_drain: f32,
    /// A drone recharges this much each second (s) over its own base (within `pad_radius`, m
    /// across), and in the circle of a depot its team holds, if the circle is not contested.
    pub base_recharge: f32,
    pub depot_recharge: f32,
    pub pad_radius: f32,
    /// A drone with an empty battery is back at its base after this long (s).
    pub respawn_time: f32,
    /// A drone out of contact with no standing order flies home when its battery gets to this (s).
    pub bingo: f32,
    /// The highest a drone flies (m): the AGENT ceiling. The mountains are higher.
    pub ceiling: f32,
    /// How far a team sees round a tower it holds, a drone and its base (m, across).
    pub tower_vision: f32,
    pub spire_vision: f32,
    pub drone_vision: f32,
    pub base_vision: f32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            field_radius: 600.0,
            base_distance: 520.0,
            home_distance: 430.0,
            tower_distance: 360.0,
            treetop: map::TREE_MIN,
            look_down: 75.0,
            drones: 5,
            squad: [Class::Standard, Class::Standard, Class::Standard, Class::Heavy, Class::Light],
            circle_radius: 60.0,
            circle_ceiling: 120.0,
            capture_time: 20.0,
            decay_time: 100.0,
            win_score: 600.0,
            tower_score: 1.0,
            spire_score: 2.0,
            duration: 600.0,
            jam_radius: 160.0,
            spire_jam_radius: 200.0,
            depot_jam_radius: 120.0,
            mesh_range: 70.0,
            mesh_hops: 3,
            link_timeout: 45.0,
            light_battery: 150.0,
            standard_battery: 240.0,
            heavy_battery: 300.0,
            contest_drain: 1.5,
            base_recharge: 10.0,
            depot_recharge: 3.0,
            pad_radius: 60.0,
            respawn_time: 100.0,
            bingo: 60.0,
            ceiling: 100.0,
            tower_vision: 200.0,
            spire_vision: 250.0,
            drone_vision: 150.0,
            base_vision: 150.0,
        }
    }
}

impl Rules {
    /// The rules as they were before v2 (plans/towers-rules-v1.md): one shadow of 140 m for every
    /// tower, one point for every tower, 500 points in 8 minutes, a link lost after 20 s, no
    /// drift, no mesh and no batteries. For measurements to compare with.
    pub fn v1() -> Rules {
        let d = Rules::default();
        Rules {
            jam_radius: 140.0,
            spire_jam_radius: 140.0,
            depot_jam_radius: 140.0,
            spire_vision: d.tower_vision,
            spire_score: 1.0,
            win_score: 500.0,
            duration: 480.0,
            link_timeout: 20.0,
            decay_time: 0.0,
            mesh_range: 0.0,
            mesh_hops: 1,
            ..d.without_batteries()
        }
    }

    /// The same rules with no batteries: for a host that cannot put a drone down.
    pub fn without_batteries(self) -> Rules {
        Rules { light_battery: 0.0, standard_battery: 0.0, heavy_battery: 0.0, ..self }
    }

    /// The class of a team's drone `k`.
    pub fn class(&self, k: usize) -> Class {
        self.squad[k]
    }

    /// The battery of a drone of this class (s of flight); 0 if the drones have none.
    pub fn battery(&self, class: Class) -> f32 {
        match class {
            Class::Light => self.light_battery,
            Class::Standard => self.standard_battery,
            Class::Heavy => self.heavy_battery,
        }
    }

    /// How far a held tower of this kind blocks the enemy's radio (m, across).
    pub fn shadow(&self, kind: Kind) -> f32 {
        match kind {
            Kind::Fort => self.jam_radius,
            Kind::Spire => self.spire_jam_radius,
            Kind::Depot => self.depot_jam_radius,
        }
    }

    /// How far a team sees round a tower of this kind that it holds (m, across).
    pub fn vision(&self, kind: Kind) -> f32 {
        match kind {
            Kind::Spire => self.spire_vision,
            _ => self.tower_vision,
        }
    }

    /// The points each second a held tower of this kind gives.
    pub fn points(&self, kind: Kind) -> f32 {
        match kind {
            Kind::Spire => self.spire_score,
            _ => self.tower_score,
        }
    }

    /// Where a team's base is (north, east of the centre).
    pub fn base(&self, team: Team) -> Vec2 {
        Vec2::new(-team.sign() * self.base_distance, 0.0)
    }
}

/// What a drone is doing, as far as the referee cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// In the air.
    Flying,
    /// On the ground, its motors running or not.
    Ground,
    /// Crashed: out of the game until it is back at its base.
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drone {
    pub team: Team,
    pub class: Class,
    /// North, east of the centre (m).
    pub at: Vec2,
    /// Height above the field (m).
    pub alt: f32,
    /// North, east (m/s).
    pub vel: Vec2,
    pub phase: Phase,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tower {
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// North, east of the centre (m).
    pub at: Vec2,
    /// From −1 (Red) to +1 (Blue).
    pub control: f32,
    pub holder: Option<Team>,
    /// Drones of both teams are in its circle.
    pub contested: bool,
}

/// How a game ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Won(Team),
    Draw,
}

/// A tower as a team last saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TowerSighting {
    pub control: f32,
    pub holder: Option<Team>,
    /// The game time it was seen (s).
    pub time: f32,
    /// The team sees it now.
    pub seen: bool,
}

/// An enemy drone as a team last saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroneSighting {
    pub at: Vec2,
    pub alt: f32,
    pub vel: Vec2,
    pub phase: Phase,
    /// The game time it was seen (s).
    pub time: f32,
    /// The team sees it now.
    pub seen: bool,
}

/// How a drone is linked to its team.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// The team hears from it and can give it orders.
    Up,
    /// It is in the radio shadow of an enemy tower.
    Cut,
    /// It has been out of contact for [`Rules::link_timeout`]: it flies home by itself.
    Lost,
}

/// What a team knows of the field: each tower and each drone of the game (by index), as it last
/// saw them. Its own drones are always seen.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamMap {
    pub towers: Vec<TowerSighting>,
    pub drones: Vec<Option<DroneSighting>>,
}

/// A game of Towers.
#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    pub rules: Rules,
    /// The valley: its lanes, its lake, its clearings and its trees.
    pub map: Map,
    pub towers: Vec<Tower>,
    /// Blue's drones, then Red's.
    pub drones: Vec<Drone>,
    /// Blue's, Red's.
    pub score: [f32; 2],
    /// Since the start (s).
    pub time: f32,
    pub outcome: Option<Outcome>,
    /// Blue's, Red's.
    pub maps: [TeamMap; 2],
    /// What each team's sites saw (its towers and its base, not its drones) of each drone, as it
    /// last saw it: the part of the fog that a ground station cannot build from its drones' reports.
    pub site_drones: [Vec<Option<DroneSighting>>; 2],
    /// Each drone is in the radio shadow of an enemy tower now.
    pub jammed: Vec<bool>,
    /// How long each drone has been out of contact (s); 0 while it is in contact.
    pub silent: Vec<f32>,
    /// What is left of each drone's battery (s), and what its team last heard of it.
    pub battery: Vec<f32>,
    pub reported: Vec<f32>,
    /// How long each drone with an empty battery has been down (s); `None` while it is not.
    pub down_for: Vec<Option<f32>>,
}

impl Game {
    /// The start: the five towers, each team holding its home tower, and every drone on the
    /// ground at its base.
    pub fn new(rules: Rules) -> Game {
        let map = Map::new(&rules);
        let tower = |name: &str, kind: Kind, at: Vec2, holder: Option<Team>| Tower {
            name: name.to_string(),
            kind,
            at,
            control: holder.map_or(0.0, Team::sign),
            holder,
            contested: false,
        };
        let towers = vec![
            tower("Blue home", Kind::Fort, map.towers[0], Some(Team::Blue)),
            tower("west", Kind::Depot, map.towers[1], None),
            tower("centre", Kind::Spire, map.towers[2], None),
            tower("east", Kind::Depot, map.towers[3], None),
            tower("Red home", Kind::Fort, map.towers[4], Some(Team::Red)),
        ];
        let drones: Vec<Drone> = Team::BOTH
            .into_iter()
            .flat_map(|team| {
                (0..rules.drones).map(move |k| Drone {
                    team,
                    class: rules.class(k),
                    at: rules.base(team) + Vec2::new(0.0, 10.0 * (k as f32 - 2.0)),
                    alt: 0.0,
                    vel: Vec2::ZERO,
                    phase: Phase::Ground,
                })
            })
            .collect();
        let team_map = TeamMap {
            towers: towers
                .iter()
                .map(|t| TowerSighting { control: t.control, holder: t.holder, time: 0.0, seen: false })
                .collect(),
            drones: vec![None; drones.len()],
        };
        let n = drones.len();
        let battery: Vec<f32> = drones.iter().map(|d| rules.battery(d.class)).collect();
        Game {
            rules,
            map,
            towers,
            drones,
            score: [0.0; 2],
            time: 0.0,
            outcome: None,
            site_drones: [vec![None; n], vec![None; n]],
            maps: [team_map.clone(), team_map],
            jammed: vec![false; n],
            silent: vec![0.0; n],
            reported: battery.clone(),
            battery,
            down_for: vec![None; n],
        }
    }

    /// What is left of drone `i`'s battery (s); `None` if the drones have none.
    pub fn charge(&self, i: usize) -> Option<f32> {
        (self.rules.battery(self.drones[i].class) > 0.0).then_some(self.battery[i])
    }

    /// Every drone's battery full, as the rules have it now.
    pub fn fill_batteries(&mut self) {
        for (b, d) in self.battery.iter_mut().zip(&self.drones) {
            *b = self.rules.battery(d.class);
        }
        self.reported = self.battery.clone();
        self.down_for.fill(None);
    }

    pub fn tower(&self, name: &str) -> usize {
        self.towers.iter().position(|t| t.name == name).unwrap_or_else(|| panic!("no tower {name}"))
    }

    /// The index of a team's `k`th drone.
    pub fn drone(&self, team: Team, k: usize) -> usize {
        team.index() * self.rules.drones + k
    }

    pub fn time_left(&self) -> f32 {
        (self.rules.duration - self.time).max(0.0)
    }

    /// The game goes on by `dt` (s), with the drones where they are now. A game that has ended
    /// does not change.
    pub fn step(&mut self, dt: f32) {
        if self.outcome.is_some() {
            return;
        }
        let dt = dt.min(self.time_left());
        self.time += dt;
        self.capture(dt);
        self.power(dt);
        self.jam(dt);
        self.score(dt);
        self.look();
    }

    /// How drone `i` is linked to its team.
    pub fn link(&self, i: usize) -> Link {
        if !self.jammed[i] {
            Link::Up
        } else if link_lost(self.silent[i], self.rules.link_timeout) {
            Link::Lost
        } else {
            Link::Cut
        }
    }

    /// Which drones are out of contact, and for how long. A drone in the radio shadow of a tower
    /// of the enemy is, unless a friend that flies, and is in contact on its own, is within
    /// [`Rules::mesh_range`] of it, or of a chain of such friends at most [`Rules::mesh_hops`] long (a
    /// relayed drone relays for the next). A drone that is down is not.
    fn jam(&mut self, dt: f32) {
        let shadow: Vec<bool> =
            self.drones
                .iter()
                .map(|d| {
                    d.phase != Phase::Down
                        && self.towers.iter().any(|t| {
                            t.holder == Some(d.team.other()) && t.at.distance(d.at) <= self.rules.shadow(t.kind)
                        })
                })
                .collect();
        // Hop by hop: a drone out of any shadow is in contact on its own (0 hops); each pass lets a
        // drone in a shadow join if a flying friend that joined the pass before is in range.
        let mut hops: Vec<Option<u32>> = shadow.iter().map(|&s| (!s).then_some(0)).collect();
        if self.rules.mesh_range > 0.0 {
            for hop in 1..=self.rules.mesh_hops {
                let joined: Vec<usize> = (0..self.drones.len())
                    .filter(|&i| hops[i].is_none())
                    .filter(|&i| {
                        self.drones.iter().enumerate().any(|(j, f)| {
                            f.team == self.drones[i].team
                                && hops[j] == Some(hop - 1)
                                && f.phase == Phase::Flying
                                && f.at.distance(self.drones[i].at) <= self.rules.mesh_range
                        })
                    })
                    .collect();
                if joined.is_empty() {
                    break;
                }
                for i in joined {
                    hops[i] = Some(hop);
                }
            }
        }
        for (i, hop) in hops.iter().enumerate() {
            let jammed = hop.is_none();
            self.jammed[i] = jammed;
            self.silent[i] = if jammed { self.silent[i] + dt } else { 0.0 };
        }
        // A drone in contact tells its team how much battery it has.
        for i in (0..self.drones.len()).filter(|&i| !self.jammed[i]) {
            self.reported[i] = self.battery[i];
        }
    }

    /// The batteries: a flying drone uses a second a second, and more for each unit of enemy
    /// weight in its circle; a drone recharges over its base, and in the circle of a depot its
    /// team holds that is not contested. A drone with an empty battery is down, and back at its
    /// base, full, after [`Rules::respawn_time`].
    fn power(&mut self, dt: f32) {
        let r = self.rules;
        for i in 0..self.drones.len() {
            let (d, full) = (self.drones[i], r.battery(self.drones[i].class));
            if full <= 0.0 {
                continue;
            }
            if let Some(down) = self.down_for[i] {
                if down + dt >= r.respawn_time {
                    self.respawn(i);
                } else {
                    self.down_for[i] = Some(down + dt);
                }
                continue;
            }
            if d.phase == Phase::Down {
                continue;
            }
            let flying = d.phase == Phase::Flying;
            let circle = self.towers.iter().find(|t| self.in_circle(&d, t.at));
            let mut change = 0.0;
            if flying {
                let enemy: f32 = circle.map_or(0.0, |t| {
                    self.drones
                        .iter()
                        .filter(|e| e.team != d.team && self.in_circle(e, t.at))
                        .map(|e| e.class.weight())
                        .sum()
                });
                change -= 1.0 + r.contest_drain * enemy;
            }
            if d.at.distance(r.base(d.team)) <= r.pad_radius {
                change += r.base_recharge;
            } else if let Some(t) = circle
                && t.kind == Kind::Depot
                && t.holder == Some(d.team)
                && !t.contested
            {
                change += r.depot_recharge;
            }
            self.battery[i] = (self.battery[i] + change * dt).clamp(0.0, full);
            if flying && self.battery[i] <= 0.0 {
                self.drones[i].phase = Phase::Down;
                self.down_for[i] = Some(0.0);
            }
        }
    }

    /// Drone `i` is on the ground at its base again, with a full battery.
    fn respawn(&mut self, i: usize) {
        let team = self.drones[i].team;
        let k = i % self.rules.drones;
        self.drones[i] = Drone {
            at: self.rules.base(team) + Vec2::new(0.0, 10.0 * (k as f32 - 2.0)),
            alt: 0.0,
            vel: Vec2::ZERO,
            phase: Phase::Ground,
            ..self.drones[i]
        };
        self.battery[i] = self.rules.battery(self.drones[i].class);
        self.down_for[i] = None;
    }

    /// Is drone `d` in the circle of the tower at `at`?
    fn in_circle(&self, d: &Drone, at: Vec2) -> bool {
        d.phase == Phase::Flying && d.at.distance(at) <= self.rules.circle_radius && d.alt <= self.rules.circle_ceiling
    }

    fn capture(&mut self, dt: f32) {
        for i in 0..self.towers.len() {
            let at = self.towers[i].at;
            // Drones in the circle, each by its weight.
            let mut count = [0.0f32; 2];
            let mut present = [0usize; 2];
            for d in self.drones.iter().filter(|d| self.in_circle(d, at)) {
                count[d.team.index()] += d.class.weight();
                present[d.team.index()] += 1;
            }
            let t = &mut self.towers[i];
            t.contested = present[0] > 0 && present[1] > 0;
            let before = t.control;
            let mut change = (count[0] - count[1]) * dt / self.rules.capture_time;
            // A circle with nobody in it drifts back: towards its holder's end, or towards 0.
            if present == [0, 0] && self.rules.decay_time > 0.0 {
                let end = t.holder.map_or(0.0, Team::sign);
                change = (end - t.control).clamp(-dt / self.rules.decay_time, dt / self.rules.decay_time);
            }
            t.control = (t.control + change).clamp(-1.0, 1.0);
            // The control is summed in floats: an end or the middle within a hair is reached.
            if (1.0 - t.control.abs()) < ENDS {
                t.control = t.control.signum();
            }
            if t.control.abs() < ENDS && before.abs() >= ENDS {
                t.control = 0.0;
            }
            if let Some(h) = t.holder
                && t.control * h.sign() <= 0.0
            {
                t.holder = None;
            }
            if t.control.abs() == 1.0 {
                t.holder = Some(if t.control > 0.0 { Team::Blue } else { Team::Red });
            }
        }
    }

    fn score(&mut self, dt: f32) {
        for team in Team::BOTH {
            let points: f32 =
                self.towers.iter().filter(|t| t.holder == Some(team)).map(|t| self.rules.points(t.kind)).sum();
            let s = &mut self.score[team.index()];
            *s = (*s + points * dt).min(self.rules.win_score);
        }
        let [blue, red] = self.score;
        let win = self.rules.win_score;
        if blue >= win || red >= win || self.time_left() < ENDS {
            self.outcome = Some(if blue > red {
                Outcome::Won(Team::Blue)
            } else if red > blue {
                Outcome::Won(Team::Red)
            } else {
                Outcome::Draw
            });
        }
    }

    /// What `team` can see from: each of its drones that is not down, its base and each tower it
    /// holds, with how far each sees (m, across) and whether it looks from above the trees.
    fn eyes(&self, team: Team) -> Vec<Eye> {
        self.eyes_of(team, true)
    }

    /// The same, with or without the team's drones: its sites alone are its towers and its base.
    fn eyes_of(&self, team: Team, with_drones: bool) -> Vec<Eye> {
        let r = &self.rules;
        // A drone out of contact sends no picture: it sees nothing for its team.
        let drones = self
            .drones
            .iter()
            .enumerate()
            .filter(|(i, d)| with_drones && d.team == team && d.phase != Phase::Down && !self.jammed[*i])
            .map(|(_, d)| Eye {
                at: d.at,
                range: r.drone_vision,
                high: d.phase == Phase::Flying && d.alt >= r.treetop,
            });
        let towers = self.towers.iter().filter(|t| t.holder == Some(team)).map(|t| Eye {
            at: t.at,
            range: r.vision(t.kind),
            high: true,
        });
        let base = Eye { at: r.base(team), range: r.base_vision, high: false };
        drones.chain(towers).chain([base]).collect()
    }

    /// Does an eye see a drone at `at`, at `alt`? A drone below the treetops is hidden by the
    /// trees from an eye that does not look from above: it is seen only along open ground.
    fn sees_drone(&self, eyes: &[Eye], at: Vec2, alt: f32, flying: bool) -> bool {
        let above = flying && alt >= self.rules.treetop;
        eyes.iter().any(|e| {
            let d = e.at.distance(at);
            d <= e.range && (above || (e.high && d <= self.rules.look_down) || self.map.line_open(e.at, at))
        })
    }

    /// The enemy drones (their numbers in the game) that drone `i` sees with its sensor alone: in
    /// its sensor range, and not hidden by the trees from where it looks. What it reports to its
    /// team, when the team can hear it.
    pub fn drone_senses(&self, i: usize) -> Vec<usize> {
        let d = &self.drones[i];
        if d.phase == Phase::Down {
            return vec![];
        }
        let eye = [Eye {
            at: d.at,
            range: self.rules.drone_vision,
            high: d.phase == Phase::Flying && d.alt >= self.rules.treetop,
        }];
        (0..self.drones.len())
            .filter(|&j| {
                let e = &self.drones[j];
                e.team != d.team
                    && e.phase != Phase::Down
                    && self.sees_drone(&eye, e.at, e.alt, e.phase == Phase::Flying)
            })
            .collect()
    }

    /// Each team looks round and updates its map.
    pub fn look(&mut self) {
        self.jam(0.0);
        for team in Team::BOTH {
            let eyes = self.eyes(team);
            let sees_tower = |at: Vec2| eyes.iter().any(|e| e.at.distance(at) <= e.range);
            let time = self.time;
            let seen: Vec<bool> = self
                .drones
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    if d.team == team {
                        // A team hears from its own drones while they are in contact.
                        !self.jammed[i]
                    } else {
                        d.phase != Phase::Down && self.sees_drone(&eyes, d.at, d.alt, d.phase == Phase::Flying)
                    }
                })
                .collect();
            // The sites alone (towers and base): what a ground station is told besides its drones' reports.
            let site_eyes = self.eyes_of(team, false);
            let site_seen: Vec<bool> = self
                .drones
                .iter()
                .map(|d| {
                    d.team != team
                        && d.phase != Phase::Down
                        && self.sees_drone(&site_eyes, d.at, d.alt, d.phase == Phase::Flying)
                })
                .collect();
            for ((d, s), seen) in self.drones.iter().zip(self.site_drones[team.index()].iter_mut()).zip(site_seen) {
                if seen {
                    *s = Some(DroneSighting { at: d.at, alt: d.alt, vel: d.vel, phase: d.phase, time, seen: true });
                } else if let Some(s) = s {
                    s.seen = false;
                }
            }
            let map = &mut self.maps[team.index()];
            for ((d, s), seen) in self.drones.iter().zip(map.drones.iter_mut()).zip(seen) {
                if seen {
                    *s = Some(DroneSighting { at: d.at, alt: d.alt, vel: d.vel, phase: d.phase, time, seen: true });
                } else if let Some(s) = s {
                    s.seen = false;
                }
            }
            for (t, s) in self.towers.iter().zip(map.towers.iter_mut()) {
                if sees_tower(t.at) {
                    *s = TowerSighting { control: t.control, holder: t.holder, time, seen: true };
                } else {
                    s.seen = false;
                }
            }
        }
    }
}

/// Something a team sees from.
struct Eye {
    at: Vec2,
    range: f32,
    /// It looks from above the trees (a tower, a drone above the treetops).
    high: bool,
}

/// How near (in control, or in seconds of the clock) counts as there: the referee sums in floats.
const ENDS: f32 = 1e-4;

/// A drone that has been out of contact for `silent` s has lost its link at `timeout` s. The time is
/// summed in floats, many small steps: a hair under the timeout counts.
pub(crate) fn link_lost(silent: f32, timeout: f32) -> bool {
    silent >= timeout * (1.0 - 1e-4)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 0.1;

    fn run(g: &mut Game, secs: f32) {
        for _ in 0..(secs / DT).round() as usize {
            g.step(DT);
        }
    }

    fn fly(g: &mut Game, team: Team, k: usize, at: Vec2) {
        let i = g.drone(team, k);
        g.drones[i] = Drone { at, alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
    }

    fn fly_low(g: &mut Game, team: Team, k: usize, at: Vec2) {
        let i = g.drone(team, k);
        g.drones[i] = Drone { at, alt: 12.0, phase: Phase::Flying, ..g.drones[i] };
    }

    #[test]
    fn the_start_has_each_home_tower_held_and_the_rest_neutral() {
        let g = Game::new(Rules::default());
        let holders: Vec<_> = g.towers.iter().map(|t| t.holder).collect();
        assert_eq!(holders, [Some(Team::Blue), None, None, None, Some(Team::Red)]);
        assert_eq!(g.towers[0].control, 1.0);
        assert!(g.drones.iter().all(|d| d.phase == Phase::Ground));
        assert_eq!(g.drones.iter().filter(|d| d.team == Team::Red).count(), 3);
    }

    #[test]
    fn the_control_moves_by_the_difference_in_drones() {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        let at = g.towers[west].at;
        fly(&mut g, Team::Blue, 0, at + Vec2::new(40.0, 0.0));
        fly(&mut g, Team::Blue, 1, at + Vec2::new(-40.0, 0.0));
        fly(&mut g, Team::Red, 0, at + Vec2::new(0.0, 40.0));
        run(&mut g, 4.0);
        assert!((g.towers[west].control - 0.2).abs() < 1e-3, "{}", g.towers[west].control);
        assert!(g.towers[west].contested);
    }

    #[test]
    fn a_drone_outside_the_circle_or_above_it_does_not_count() {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        let at = g.towers[west].at;
        fly(&mut g, Team::Blue, 0, at + Vec2::new(61.0, 0.0));
        fly(&mut g, Team::Blue, 1, at);
        let i = g.drone(Team::Blue, 1);
        g.drones[i].alt = 121.0;
        run(&mut g, 5.0);
        assert_eq!(g.towers[west].control, 0.0);
    }

    #[test]
    fn the_home_towers_are_forts_the_centre_is_the_spire_and_the_flanks_are_depots() {
        let g = Game::new(Rules::default());
        let kinds: Vec<(&str, Kind)> = g.towers.iter().map(|t| (t.name.as_str(), t.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("Blue home", Kind::Fort),
                ("west", Kind::Depot),
                ("centre", Kind::Spire),
                ("east", Kind::Depot),
                ("Red home", Kind::Fort)
            ]
        );
        let r = g.rules;
        assert_eq!([r.shadow(Kind::Fort), r.shadow(Kind::Spire), r.shadow(Kind::Depot)], [160.0, 200.0, 120.0]);
        assert_eq!([r.points(Kind::Fort), r.points(Kind::Spire), r.points(Kind::Depot)], [1.0, 2.0, 1.0]);
    }

    #[test]
    fn a_team_sees_250_m_round_the_spire_it_holds_and_200_m_round_a_depot() {
        let mut g = Game::new(Rules::default());
        let (centre, west) = (g.tower("centre"), g.tower("west"));
        let red = g.drone(Team::Red, 0);
        // A Red drone above the trees, 240 m from the one tower that Blue holds.
        for (tower, seen) in [(centre, true), (west, false)] {
            for t in [centre, west] {
                g.towers[t].holder = (t == tower).then_some(Team::Blue);
            }
            g.drones[red] = Drone {
                at: g.towers[tower].at + Vec2::new(0.0, 240.0),
                alt: 60.0,
                phase: Phase::Flying,
                ..g.drones[red]
            };
            g.maps = Game::new(Rules::default()).maps;
            g.look();
            let s = g.maps[Team::Blue.index()].drones[red].map(|s| s.seen);
            assert_eq!(s.unwrap_or(false), seen, "{}", g.towers[tower].name);
        }
    }

    /// A game with the batteries on (150, 240 and 300 s), Blue's first drone (a standard one)
    /// flying at `at`, with `left` s in its battery.
    fn charged(at: Vec2, left: f32) -> (Game, usize) {
        let rules = Rules { light_battery: 150.0, standard_battery: 240.0, heavy_battery: 300.0, ..Rules::default() };
        let mut g = Game::new(rules);
        fly(&mut g, Team::Blue, 0, at);
        let i = g.drone(Team::Blue, 0);
        g.battery[i] = left;
        g.look();
        (g, i)
    }

    const MID: Vec2 = Vec2::new(-220.0, 0.0);

    #[test]
    fn a_game_with_no_batteries_never_runs_one_down() {
        let mut g = Game::new(Rules::default().without_batteries());
        fly(&mut g, Team::Blue, 0, MID);
        run(&mut g, 600.0);
        let i = g.drone(Team::Blue, 0);
        assert_eq!((g.drones[i].phase, g.battery[i]), (Phase::Flying, 0.0));
    }

    #[test]
    fn a_flying_drone_uses_a_second_of_battery_each_second_and_a_drone_on_the_ground_none() {
        let (mut g, i) = charged(MID, 100.0);
        run(&mut g, 10.0);
        assert!((g.battery[i] - 90.0).abs() < 0.01, "{}", g.battery[i]);
        let j = g.drone(Team::Blue, 1);
        g.battery[j] = 100.0;
        // On the ground, away from the base's pad.
        g.drones[j].at = Vec2::new(-100.0, 0.0);
        run(&mut g, 10.0);
        assert!((g.battery[j] - 100.0).abs() < 0.01, "{}", g.battery[j]);
    }

    #[test]
    fn a_drone_recharges_10_s_each_second_over_its_base_and_never_over_the_full_battery() {
        let (mut g, i) = charged(Vec2::new(-520.0, 0.0), 100.0);
        run(&mut g, 10.0);
        assert!((g.battery[i] - 190.0).abs() < 0.01, "flying uses 1: {}", g.battery[i]);
        run(&mut g, 10.0);
        assert_eq!(g.battery[i], 240.0, "the most a standard drone holds");
    }

    #[test]
    fn enemy_weight_in_the_circle_makes_a_drone_use_more() {
        // Enemy classes: standard 1, heavy 2, light 0.5. Red drone 0 is standard, 3 heavy, 4 light.
        for (k, per_second) in [(0, 2.5), (3, 4.0), (4, 1.75)] {
            let (mut g, i) = charged(Vec2::ZERO + Vec2::new(40.0, 0.0), 100.0);
            fly(&mut g, Team::Red, k, Vec2::new(-40.0, 0.0));
            run(&mut g, 10.0);
            assert!((g.battery[i] - (100.0 - 10.0 * per_second)).abs() < 0.01, "enemy {k}: {}", g.battery[i]);
        }
    }

    #[test]
    fn a_depot_the_team_holds_recharges_unless_the_circle_is_contested_and_a_fort_never_does() {
        let (mut g, i) = charged(Vec2::new(45.0, -390.0) + Vec2::new(40.0, 0.0), 100.0);
        let west = g.tower("west");
        g.towers[west].holder = Some(Team::Blue);
        g.towers[west].control = 1.0;
        run(&mut g, 10.0);
        assert!((g.battery[i] - 120.0).abs() < 0.01, "+3 -1: {}", g.battery[i]);
        // An enemy in the circle: contested, no recharge, and the drain is higher.
        fly(&mut g, Team::Red, 0, Vec2::new(45.0, -390.0));
        g.battery[i] = 100.0;
        run(&mut g, 10.0);
        assert!((g.battery[i] - 75.0).abs() < 0.01, "{}", g.battery[i]);
        // A depot the enemy holds, a fort the team holds.
        let (mut g, i) = charged(g.towers[g.tower("Blue home")].at + Vec2::new(40.0, 0.0), 100.0);
        run(&mut g, 10.0);
        assert!((g.battery[i] - 90.0).abs() < 0.01, "a fort: {}", g.battery[i]);
    }

    #[test]
    fn a_drone_with_an_empty_battery_is_down_and_is_back_full_at_its_base_after_the_respawn_time() {
        let (mut g, i) = charged(MID, 5.0);
        run(&mut g, 5.5);
        assert_eq!(g.drones[i].phase, Phase::Down);
        run(&mut g, 99.0);
        assert_eq!(g.drones[i].phase, Phase::Down, "100 s from the empty battery");
        run(&mut g, 2.0);
        let d = g.drones[i];
        assert_eq!(d.phase, Phase::Ground);
        assert_eq!(d.alt, 0.0);
        assert!(d.at.distance(g.rules.base(Team::Blue)) < 30.0, "{:?}", d.at);
        assert_eq!(g.battery[i], 240.0);
    }

    #[test]
    fn a_drone_that_is_down_counts_for_no_circle_and_scores_no_points_for_the_enemy() {
        let (mut g, i) = charged(Vec2::ZERO, 1.0);
        let centre = g.tower("centre");
        run(&mut g, 2.0);
        assert_eq!(g.drones[i].phase, Phase::Down);
        let control = g.towers[centre].control;
        assert!(control > 0.0, "it moved the bar while it flew");
        run(&mut g, 30.0);
        assert_eq!(
            g.towers[centre].control, 0.0,
            "a drone that is down moves nothing up: the empty circle drifts back"
        );
        assert!((g.score[Team::Red.index()] - 32.0).abs() < 0.2, "only Red's home tower scores");
    }

    #[test]
    fn the_team_knows_the_battery_a_drone_out_of_contact_last_reported() {
        let (mut g, i) = charged(MID, 100.0);
        let red = g.tower("Red home");
        g.towers[red].holder = Some(Team::Red);
        g.towers[red].control = -1.0;
        run(&mut g, 5.0);
        assert!((g.view(Team::Blue).drones[0].battery - 95.0).abs() < 0.2, "in contact");
        let shadow = g.towers[red].at + Vec2::new(-30.0, 0.0);
        fly(&mut g, Team::Blue, 0, shadow);
        g.look();
        run(&mut g, 10.0);
        let seen = g.view(Team::Blue).drones[0];
        assert!(!seen.linked);
        assert!((seen.battery - 95.0).abs() < 0.2, "the last report, not the 85 s it has: {}", seen.battery);
        assert!((g.battery[i] - 85.0).abs() < 0.2);
    }

    #[test]
    fn an_empty_circle_drifts_back_towards_0_and_a_held_tower_towards_its_holders_end() {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        g.towers[west].control = 0.5;
        run(&mut g, 10.0);
        assert!((g.towers[west].control - 0.4).abs() < 1e-3, "{}", g.towers[west].control);
        run(&mut g, 100.0);
        assert_eq!(g.towers[west].control, 0.0, "it stops at 0");
        // Held by Red, half taken back by Blue, then left: back to Red's end, and no further.
        g.towers[west].holder = Some(Team::Red);
        g.towers[west].control = -0.5;
        run(&mut g, 10.0);
        assert!((g.towers[west].control + 0.6).abs() < 1e-3, "{}", g.towers[west].control);
        run(&mut g, 100.0);
        assert_eq!((g.towers[west].control, g.towers[west].holder), (-1.0, Some(Team::Red)));
    }

    #[test]
    fn a_circle_with_a_drone_in_it_does_not_drift() {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        g.towers[west].control = 0.5;
        let at = g.towers[west].at;
        // One Blue and one Red: contested, nothing moves.
        fly(&mut g, Team::Blue, 0, at + Vec2::new(40.0, 0.0));
        fly(&mut g, Team::Red, 1, at + Vec2::new(-40.0, 0.0));
        run(&mut g, 10.0);
        assert_eq!(g.towers[west].control, 0.5);
    }

    /// Blue's first and second drones, `first` and `second` m east of the Red home tower (a fort
    /// held by Red, with a shadow of 160 m).
    fn pickets(first: f32, second: f32) -> Game {
        let mut g = Game::new(Rules::default());
        let red = g.towers[g.tower("Red home")].at;
        fly(&mut g, Team::Blue, 0, red + Vec2::new(0.0, first));
        fly(&mut g, Team::Blue, 1, red + Vec2::new(0.0, second));
        g.look();
        g
    }

    #[test]
    fn a_drone_in_a_shadow_is_in_contact_through_a_friend_in_contact_within_70_m() {
        // 150 m is in the shadow; 210 m is not, and is 60 m away.
        let g = pickets(150.0, 210.0);
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Up);
        assert_eq!(g.silent[g.drone(Team::Blue, 0)], 0.0);
        assert_eq!(link_of(&g, Team::Blue, 1), Link::Up);
    }

    #[test]
    fn a_friend_that_is_too_far_in_a_shadow_on_the_ground_or_down_does_not_relay() {
        assert_eq!(link_of(&pickets(150.0, 230.0), Team::Blue, 0), Link::Cut, "80 m");
        assert_eq!(link_of(&pickets(150.0, 130.0), Team::Blue, 0), Link::Cut, "in the shadow too");
        for phase in [Phase::Ground, Phase::Down] {
            let mut g = pickets(150.0, 210.0);
            let j = g.drone(Team::Blue, 1);
            g.drones[j].phase = phase;
            g.look();
            assert_eq!(link_of(&g, Team::Blue, 0), Link::Cut, "{phase:?}");
        }
    }

    /// A chain of Blue drones from the edge of the Red home shadow inwards, 60 m apart, at 210 m,
    /// 150 m, 90 m and 30 m: the first is out of the shadow, the rest are in it.
    fn chain(hops: u32) -> Game {
        let mut g = Game::new(Rules { mesh_hops: hops, ..Rules::default() });
        let red = g.towers[g.tower("Red home")].at;
        for (i, d) in [210.0, 150.0, 90.0, 30.0].into_iter().enumerate() {
            fly(&mut g, Team::Blue, i, red + Vec2::new(0.0, d));
        }
        g.look();
        g
    }

    #[test]
    fn a_relayed_drone_relays_for_the_next_in_a_chain() {
        let g = chain(3);
        for i in 0..4 {
            assert_eq!(link_of(&g, Team::Blue, i), Link::Up, "drone {i}");
        }
    }

    #[test]
    fn a_chain_is_as_long_as_the_hops_and_no_longer() {
        let g = chain(2);
        assert_eq!(link_of(&g, Team::Blue, 2), Link::Up);
        assert_eq!(link_of(&g, Team::Blue, 3), Link::Cut, "three hops from contact");
        let g = chain(1);
        assert_eq!(link_of(&g, Team::Blue, 1), Link::Up);
        assert_eq!(link_of(&g, Team::Blue, 2), Link::Cut, "one hop: a relayed drone does not relay");
    }

    #[test]
    fn a_chain_breaks_where_a_link_is_down_or_too_far() {
        let mut g = chain(3);
        let j = g.drone(Team::Blue, 1);
        g.drones[j].phase = Phase::Down;
        g.look();
        assert_eq!(link_of(&g, Team::Blue, 2), Link::Cut);
        assert_eq!(link_of(&g, Team::Blue, 3), Link::Cut);
    }

    #[test]
    fn a_game_with_no_mesh_range_has_no_relay() {
        let mut g = pickets(150.0, 210.0);
        g.rules.mesh_range = 0.0;
        g.look();
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Cut);
    }

    #[test]
    fn a_tower_is_held_at_the_end_of_the_bar_and_lost_at_the_middle() {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        let at = g.towers[west].at;
        fly(&mut g, Team::Red, 1, at);
        run(&mut g, 19.9);
        assert_eq!(g.towers[west].holder, None);
        run(&mut g, 0.1);
        assert_eq!(g.towers[west].holder, Some(Team::Red));
        assert_eq!(g.towers[west].control, -1.0, "the control stops at the end");
        let i = g.drone(Team::Red, 1);
        g.drones[i].phase = Phase::Down;
        fly(&mut g, Team::Blue, 0, at);
        run(&mut g, 19.9);
        assert_eq!(g.towers[west].holder, Some(Team::Red));
        run(&mut g, 0.1);
        assert_eq!(g.towers[west].holder, None);
    }

    #[test]
    fn each_held_tower_scores_a_point_a_second() {
        let mut g = Game::new(Rules::default());
        run(&mut g, 10.0);
        assert!((g.score[0] - 10.0).abs() < 1e-3 && (g.score[1] - 10.0).abs() < 1e-3, "{:?}", g.score);
    }

    #[test]
    fn the_first_to_the_win_score_wins_and_the_game_stops() {
        let mut g = Game::new(Rules::default());
        g.score = [599.5, 100.0];
        run(&mut g, 1.0);
        assert_eq!(g.outcome, Some(Outcome::Won(Team::Blue)));
        assert_eq!(g.score[0], 600.0, "the score stops at the win");
        let (time, score) = (g.time, g.score);
        run(&mut g, 5.0);
        assert_eq!((g.time, g.score), (time, score), "a finished game does not change");
    }

    #[test]
    fn at_the_end_of_the_clock_more_points_win_and_equal_is_a_draw() {
        let mut g = Game::new(Rules::default());
        g.time = g.rules.duration - 1.0;
        g.score = [10.0, 20.0];
        run(&mut g, 2.0);
        assert_eq!(g.outcome, Some(Outcome::Won(Team::Red)));
        assert_eq!(g.time_left(), 0.0);
        let mut g = Game::new(Rules::default());
        g.time = g.rules.duration - 1.0;
        run(&mut g, 2.0);
        assert_eq!(g.outcome, Some(Outcome::Draw));
    }

    #[test]
    fn a_team_sees_round_its_drones_its_base_and_its_towers_only() {
        let rules = Rules::default();
        let mut g = Game::new(rules);
        let scout = g.drone(Team::Red, 0);
        let blue = g.drone(Team::Blue, 0);
        let sees = |g: &Game| g.maps[Team::Blue.index()].drones[scout].is_some_and(|s| s.seen);
        // Near a Blue drone in the middle of the field.
        fly(&mut g, Team::Blue, 0, Vec2::ZERO);
        fly(&mut g, Team::Red, 0, Vec2::new(149.0, 0.0));
        g.look();
        assert!(sees(&g));
        // Out of the drone's sight, and out of the home tower's (200 m round (-430, 0)).
        g.drones[scout].at = Vec2::new(151.0, 0.0);
        g.look();
        assert!(!sees(&g));
        // Near the Blue home tower, with the Blue drone back at the base.
        g.drones[blue] = Game::new(rules).drones[blue];
        g.drones[scout].at = Vec2::new(-430.0 + 199.0, 0.0);
        g.look();
        assert!(sees(&g));
        // A neutral tower gives no sight.
        g.drones[scout].at = g.towers[g.tower("west")].at + Vec2::new(10.0, 0.0);
        g.look();
        assert!(!sees(&g));
        // Near the Blue base, on the ground, in its clearing.
        g.drones[scout].at = rules.base(Team::Blue) + Vec2::new(0.0, 60.0);
        g.drones[scout].phase = Phase::Ground;
        g.look();
        assert!(sees(&g));
        // A crashed drone is not seen.
        g.drones[scout].phase = Phase::Down;
        g.look();
        assert!(!sees(&g));
    }

    #[test]
    fn the_trees_hide_a_drone_below_them_from_an_eye_that_looks_along_the_ground() {
        let mut g = Game::new(Rules::default());
        let scout = g.drone(Team::Red, 0);
        let sees = |g: &Game| g.maps[Team::Blue.index()].drones[scout].is_some_and(|s| s.seen);
        // Blue's home tower sees down its lanes: leave it out of this.
        g.towers[0].holder = None;
        // Blue's drone is low in the mid lane; Red's is low in the east lane, 130 m off.
        fly_low(&mut g, Team::Blue, 0, Vec2::new(-200.0, 0.0));
        let east = &g.map.lanes.iter().find(|l| l.name == map::LaneName::East).unwrap().points;
        let p = east[0].lerp(east[1], 0.35);
        fly_low(&mut g, Team::Red, 0, p);
        assert!(p.distance(Vec2::new(-200.0, 0.0)) < 150.0 && !g.map.line_open(p, Vec2::new(-200.0, 0.0)));
        g.look();
        assert!(!sees(&g), "the trees are between them");
        // The same drone above the trees is seen.
        g.drones[scout].alt = 50.0;
        g.look();
        assert!(sees(&g));
        // Low in the same lane, it is seen along the lane.
        fly_low(&mut g, Team::Red, 0, Vec2::new(-100.0, 0.0));
        g.look();
        assert!(sees(&g));
        // A drone above the trees looks down on a low drone near it, but only near.
        fly(&mut g, Team::Blue, 0, Vec2::new(-200.0, 40.0));
        fly_low(&mut g, Team::Red, 0, p);
        g.look();
        assert!(!sees(&g), "{} m is past looking down", Vec2::new(-200.0, 40.0).distance(p));
        fly_low(&mut g, Team::Red, 0, Vec2::new(-250.0, 0.0));
        g.look();
        assert!(sees(&g), "about 64 m below");
    }

    #[test]
    fn a_tower_sees_a_low_drone_down_its_lanes_and_in_its_clearing_but_not_in_the_forest() {
        let mut g = Game::new(Rules::default());
        let scout = g.drone(Team::Red, 0);
        let sees = |g: &Game| g.maps[Team::Blue.index()].drones[scout].is_some_and(|s| s.seen);
        // Blue's home tower is held: it sees from above the trees, 200 m round.
        fly_low(&mut g, Team::Red, 0, Vec2::new(-430.0 + 150.0, 0.0));
        g.look();
        assert!(sees(&g), "down the mid lane");
        fly_low(&mut g, Team::Red, 0, Vec2::new(-430.0 + 40.0, 0.0));
        g.look();
        assert!(sees(&g), "in the clearing");
        g.drones[scout].phase = Phase::Down;
        g.look();
        assert!(!sees(&g));
    }

    fn link_of(g: &Game, team: Team, k: usize) -> Link {
        g.link(g.drone(team, k))
    }

    #[test]
    fn a_drone_near_a_tower_of_the_enemy_is_cut_and_after_the_timeout_lost() {
        let mut g = Game::new(Rules::default());
        let red_home = g.towers[4].at;
        fly(&mut g, Team::Blue, 0, red_home + Vec2::new(-159.0, 0.0));
        g.look();
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Cut);
        run(&mut g, 44.9);
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Cut);
        run(&mut g, 0.1);
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Lost);
        // Out of the shadow, in contact at once, and the clock starts again.
        let i = g.drone(Team::Blue, 0);
        g.drones[i].at = red_home + Vec2::new(-161.0, 0.0);
        run(&mut g, 0.1);
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Up);
        assert_eq!(g.silent[i], 0.0);
    }

    #[test]
    fn only_a_tower_the_enemy_holds_blocks_the_radio_at_any_height() {
        let mut g = Game::new(Rules::default());
        let red_home = g.towers[4].at;
        fly(&mut g, Team::Blue, 0, red_home + Vec2::new(-100.0, 0.0));
        let i = g.drone(Team::Blue, 0);
        g.drones[i].alt = 100.0;
        g.look();
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Cut, "the shadow is as tall as the sky");
        g.towers[4].holder = None;
        g.look();
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Up, "a tower that nobody holds");
        g.towers[4].holder = Some(Team::Blue);
        g.look();
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Up, "a tower of its own team");
        g.towers[4].holder = Some(Team::Red);
        g.drones[i].phase = Phase::Down;
        g.look();
        assert_eq!(link_of(&g, Team::Blue, 0), Link::Up, "a crashed drone sends what it can");
    }

    #[test]
    fn a_drone_out_of_contact_sends_no_telemetry_and_sees_nothing_for_its_team() {
        let mut g = Game::new(Rules::default());
        let red_home = g.towers[4].at;
        let scout = g.drone(Team::Red, 0);
        // In contact first, high over the middle: the map has it there.
        fly(&mut g, Team::Blue, 0, Vec2::new(-300.0, 0.0));
        g.look();
        let blue = g.drone(Team::Blue, 0);
        assert!(g.view(Team::Blue).drones[0].linked);
        // Then into the shadow, with a scout beside it.
        fly(&mut g, Team::Blue, 0, red_home + Vec2::new(-100.0, 0.0));
        fly(&mut g, Team::Red, 0, red_home + Vec2::new(-100.0, 50.0));
        run(&mut g, 5.0);
        let view = g.view(Team::Blue);
        let d = view.drones[0];
        assert!(!d.linked && (d.silent - 5.0).abs() < 1e-3, "{d:?}");
        assert_eq!(d.at, Vec2::new(-300.0, 0.0), "the telemetry is from when it was last heard");
        assert!(!g.maps[Team::Blue.index()].drones[scout].is_some_and(|s| s.seen), "no picture, no sight");
        assert!(g.maps[Team::Blue.index()].drones[blue].is_some_and(|s| !s.seen));
        // The enemy's map is not affected: Red's tower sees the drone.
        assert!(g.maps[Team::Red.index()].drones[blue].is_some_and(|s| s.seen));
    }

    #[test]
    fn a_drone_out_of_sight_stays_where_it_was_last_seen() {
        let mut g = Game::new(Rules::default());
        let scout = g.drone(Team::Red, 0);
        fly(&mut g, Team::Blue, 0, Vec2::ZERO);
        fly(&mut g, Team::Red, 0, Vec2::new(100.0, 0.0));
        run(&mut g, 1.0);
        g.drones[scout].at = Vec2::new(300.0, 250.0);
        run(&mut g, 3.0);
        let s = g.maps[Team::Blue.index()].drones[scout].unwrap();
        assert!(!s.seen);
        assert_eq!(s.at, Vec2::new(100.0, 0.0));
        assert!((g.time - s.time - 3.0).abs() < 1e-3);
    }

    #[test]
    fn a_team_knows_its_own_drones_and_towers() {
        let mut g = Game::new(Rules::default());
        g.look();
        let blue = &g.maps[Team::Blue.index()];
        assert!(blue.drones[..3].iter().all(|s| s.is_some_and(|s| s.seen)));
        assert!(blue.drones[3..].iter().all(|s| s.is_none()), "Red's base is far from Blue's");
        assert!(blue.towers[0].seen && !blue.towers[2].seen && !blue.towers[4].seen);
    }

    #[test]
    fn a_tower_on_a_map_changes_only_when_the_team_sees_it() {
        let mut g = Game::new(Rules::default());
        let east = g.tower("east");
        let at = g.towers[east].at;
        fly(&mut g, Team::Red, 1, at);
        run(&mut g, 20.0);
        assert_eq!(g.towers[east].holder, Some(Team::Red));
        assert_eq!(g.maps[Team::Blue.index()].towers[east].holder, None);
        assert_eq!(g.maps[Team::Red.index()].towers[east].holder, Some(Team::Red));
        fly(&mut g, Team::Blue, 0, at + Vec2::new(-145.0, 0.0));
        g.look();
        let s = g.maps[Team::Blue.index()].towers[east];
        assert_eq!((s.holder, s.seen), (Some(Team::Red), true));
    }

    #[test]
    fn a_drone_senses_the_enemy_in_its_range_and_the_site_view_leaves_out_what_only_drones_saw() {
        let mut g = Game::new(Rules::default());
        let east = Vec2::new(0.0, 300.0);
        fly(&mut g, Team::Blue, 0, east);
        fly(&mut g, Team::Red, 0, east + Vec2::new(0.0, 50.0));
        fly(&mut g, Team::Red, 1, east + Vec2::new(0.0, 900.0));
        g.step(0.1);
        let (blue, scout, far) = (g.drone(Team::Blue, 0), g.drone(Team::Red, 0), g.drone(Team::Red, 1));
        assert_eq!(g.drone_senses(blue), vec![scout], "the near one, not the far one ({far})");
        assert!(g.view(Team::Blue).enemies.iter().any(|e| e.k == 0 && e.seen), "the team sees it through its drone");
        assert!(g.site_view(Team::Blue).enemies.is_empty(), "its sites see nothing of it");
    }
}
