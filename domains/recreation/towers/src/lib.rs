//! Towers: two teams of drones fight for radio towers (see plans/towers.md and
//! expectations/09-towers).
//!
//! A drone counts for a tower while it flies in the tower's circle. Each tower has a control,
//! from −1 (Red) to +1 (Blue), that moves towards the team with more drones in the circle; a team
//! holds the tower when the control gets to its end, and loses it when the control gets back to 0.
//! A held tower scores a point a second. Each team sees round its drones, its base and its towers,
//! and keeps a map of what it last saw: the rest of the field is in fog.
//!
//! The referee only needs where each drone is and whether it flies; the host moves the drones.
//! Positions are on the field: north and east of its centre (m), and a height above it (m).

mod commander;
pub mod flight;
pub mod seat;
pub mod svg;
mod view;

pub use commander::{Commander, CommanderTuning};
pub use view::{EnemyDrone, Order, OwnDrone, TeamView, TowerView};

use glam::Vec2;
use serde::{Deserialize, Serialize};

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

/// The numbers of the game. To tune, not measurements.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// The field: a circle round its centre (m), the AGENT fence.
    pub field_radius: f32,
    /// From the centre to each base (m): Blue's to the south, Red's to the north.
    pub base_distance: f32,
    /// From the centre to each home tower and each flank tower (m).
    pub tower_distance: f32,
    /// Drones in each team.
    pub drones: usize,
    /// A tower's circle: a drone counts within this distance across (m)...
    pub circle_radius: f32,
    /// ...and at most this high (m).
    pub circle_ceiling: f32,
    /// One drone more in the circle takes the control from 0 to its end in this time (s).
    pub capture_time: f32,
    /// The score that wins.
    pub win_score: f32,
    /// How long a game lasts (s).
    pub duration: f32,
    /// How far a team sees round a tower it holds, a drone and its base (m, across).
    pub tower_vision: f32,
    pub drone_vision: f32,
    pub base_vision: f32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            field_radius: 420.0,
            base_distance: 300.0,
            tower_distance: 220.0,
            drones: 3,
            circle_radius: 60.0,
            circle_ceiling: 120.0,
            capture_time: 20.0,
            win_score: 500.0,
            duration: 480.0,
            tower_vision: 200.0,
            drone_vision: 150.0,
            base_vision: 150.0,
        }
    }
}

impl Rules {
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
    /// The game time it was seen (s).
    pub time: f32,
    /// The team sees it now.
    pub seen: bool,
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
}

impl Game {
    /// The start: the five towers, each team holding its home tower, and every drone on the
    /// ground at its base.
    pub fn new(rules: Rules) -> Game {
        let d = rules.tower_distance;
        let tower = |name: &str, at: Vec2, holder: Option<Team>| Tower {
            name: name.to_string(),
            at,
            control: holder.map_or(0.0, Team::sign),
            holder,
            contested: false,
        };
        let towers = vec![
            tower("Blue home", Vec2::new(-d, 0.0), Some(Team::Blue)),
            tower("west", Vec2::new(0.0, -d), None),
            tower("centre", Vec2::ZERO, None),
            tower("east", Vec2::new(0.0, d), None),
            tower("Red home", Vec2::new(d, 0.0), Some(Team::Red)),
        ];
        let drones: Vec<Drone> = Team::BOTH
            .into_iter()
            .flat_map(|team| {
                (0..rules.drones).map(move |k| Drone {
                    team,
                    at: rules.base(team) + Vec2::new(0.0, 10.0 * (k as f32 - 1.0)),
                    alt: 0.0,
                    vel: Vec2::ZERO,
                    phase: Phase::Ground,
                })
            })
            .collect();
        let map = TeamMap {
            towers: towers
                .iter()
                .map(|t| TowerSighting { control: t.control, holder: t.holder, time: 0.0, seen: false })
                .collect(),
            drones: vec![None; drones.len()],
        };
        Game { rules, towers, drones, score: [0.0; 2], time: 0.0, outcome: None, maps: [map.clone(), map] }
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
        self.score(dt);
        self.look();
    }

    /// Is drone `d` in the circle of the tower at `at`?
    fn in_circle(&self, d: &Drone, at: Vec2) -> bool {
        d.phase == Phase::Flying && d.at.distance(at) <= self.rules.circle_radius && d.alt <= self.rules.circle_ceiling
    }

    fn capture(&mut self, dt: f32) {
        for i in 0..self.towers.len() {
            let at = self.towers[i].at;
            let mut count = [0i32; 2];
            for d in self.drones.iter().filter(|d| self.in_circle(d, at)) {
                count[d.team.index()] += 1;
            }
            let t = &mut self.towers[i];
            t.contested = count[0] > 0 && count[1] > 0;
            let before = t.control;
            t.control = (t.control + (count[0] - count[1]) as f32 * dt / self.rules.capture_time).clamp(-1.0, 1.0);
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
            let held = self.towers.iter().filter(|t| t.holder == Some(team)).count();
            let s = &mut self.score[team.index()];
            *s = (*s + held as f32 * dt).min(self.rules.win_score);
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
    /// holds, with how far each sees (m, across).
    fn eyes(&self, team: Team) -> Vec<(Vec2, f32)> {
        let r = &self.rules;
        let drones =
            self.drones.iter().filter(|d| d.team == team && d.phase != Phase::Down).map(|d| (d.at, r.drone_vision));
        let towers = self.towers.iter().filter(|t| t.holder == Some(team)).map(|t| (t.at, r.tower_vision));
        drones.chain(towers).chain([(r.base(team), r.base_vision)]).collect()
    }

    /// Each team looks round and updates its map.
    pub fn look(&mut self) {
        for team in Team::BOTH {
            let eyes = self.eyes(team);
            let sees = |at: Vec2| eyes.iter().any(|&(from, range)| from.distance(at) <= range);
            let time = self.time;
            let map = &mut self.maps[team.index()];
            for (d, s) in self.drones.iter().zip(map.drones.iter_mut()) {
                if d.team == team || (d.phase != Phase::Down && sees(d.at)) {
                    *s = Some(DroneSighting { at: d.at, alt: d.alt, vel: d.vel, time, seen: true });
                } else if let Some(s) = s {
                    s.seen = false;
                }
            }
            for (t, s) in self.towers.iter().zip(map.towers.iter_mut()) {
                if sees(t.at) {
                    *s = TowerSighting { control: t.control, holder: t.holder, time, seen: true };
                } else {
                    s.seen = false;
                }
            }
        }
    }
}

/// How near (in control, or in seconds of the clock) counts as there: the referee sums in floats.
const ENDS: f32 = 1e-4;

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
        g.score = [499.5, 100.0];
        run(&mut g, 1.0);
        assert_eq!(g.outcome, Some(Outcome::Won(Team::Blue)));
        assert_eq!(g.score[0], 500.0, "the score stops at the win");
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
        // Out of the drone's sight, and out of the home tower's (200 m round (-220, 0)).
        g.drones[scout].at = Vec2::new(151.0, 0.0);
        g.look();
        assert!(!sees(&g));
        // Near the Blue home tower, with the Blue drone back at the base.
        g.drones[blue] = Game::new(rules).drones[blue];
        g.drones[scout].at = Vec2::new(-221.0 + 199.0, 0.0);
        g.look();
        assert!(sees(&g));
        // A neutral tower gives no sight.
        g.drones[scout].at = g.towers[g.tower("west")].at + Vec2::new(10.0, 0.0);
        g.look();
        assert!(!sees(&g));
        // Near the Blue base, on the ground.
        g.drones[scout].at = rules.base(Team::Blue) + Vec2::new(0.0, 149.0 + 10.0);
        g.drones[scout].phase = Phase::Ground;
        g.look();
        assert!(sees(&g));
        // A crashed drone is not seen.
        g.drones[scout].phase = Phase::Down;
        g.look();
        assert!(!sees(&g));
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
        fly(&mut g, Team::Blue, 0, at + Vec2::new(-100.0, 0.0));
        g.look();
        let s = g.maps[Team::Blue.index()].towers[east];
        assert_eq!((s.holder, s.seen), (Some(Team::Red), true));
    }
}
