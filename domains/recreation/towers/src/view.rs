//! What a seat gets: its team's map of the game ([`TeamView`]), and the orders it gives.

use crate::{Game, Kind, Outcome, Phase, Team};
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// What a drone is to do. The server flies it by plan.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Order {
    /// Circle the tower (by its index in the game) inside its circle.
    Take(usize),
    /// Fly to a point (north, east of the centre, m) and circle it.
    Goto(Vec2),
    /// Circle where it is.
    Hold,
    /// Circle over its base.
    Home,
}

/// A tower as the team knows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TowerView {
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    pub at: Vec2,
    /// As the team last saw it.
    pub control: f32,
    pub holder: Option<Team>,
    /// Drones of both teams are in its circle (known only while the team sees it).
    pub contested: bool,
    /// The team sees it now.
    pub seen: bool,
    /// Since the team last saw it (s).
    pub age: f32,
}

/// One of the team's own drones.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OwnDrone {
    /// Its number in the team (0 is the first).
    pub k: usize,
    pub at: Vec2,
    pub alt: f32,
    pub vel: Vec2,
    pub phase: Phase,
    /// The order the server has for it (the referee knows none: the server fills it in).
    #[serde(default)]
    pub order: Option<Order>,
    /// The order is flown low (along the lanes, below the treetops).
    #[serde(default)]
    pub low: bool,
    /// The team hears from the drone. If not, it is in the radio shadow of an enemy tower: the
    /// position, height, speed and phase are from when the team last heard from it, it gets no
    /// order, and it sends no picture.
    #[serde(default = "in_contact")]
    pub linked: bool,
    /// How long the team has not heard from it (s); 0 while it does.
    #[serde(default)]
    pub silent: f32,
    /// What is left of its battery (s), as the drone last said (0 if the drones have none).
    #[serde(default)]
    pub battery: f32,
}

fn in_contact() -> bool {
    true
}

/// An enemy drone the team sees, or has seen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnemyDrone {
    /// Its number in its team.
    pub k: usize,
    pub at: Vec2,
    pub alt: f32,
    pub vel: Vec2,
    /// The team sees it now; if not, it is where the team last saw it.
    pub seen: bool,
    /// Since the team last saw it (s).
    pub age: f32,
}

/// A team's map of the game: all a seat of that team gets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TeamView {
    pub team: Team,
    /// Since the start (s).
    pub time: f32,
    pub time_left: f32,
    /// Blue's, Red's: the scores are known to both.
    pub score: [f32; 2],
    pub outcome: Option<Outcome>,
    pub towers: Vec<TowerView>,
    pub drones: Vec<OwnDrone>,
    /// Only the enemy drones the team has seen.
    pub enemies: Vec<EnemyDrone>,
}

impl Game {
    /// What `team` knows.
    pub fn view(&self, team: Team) -> TeamView {
        let map = &self.maps[team.index()];
        let towers = self
            .towers
            .iter()
            .zip(&map.towers)
            .map(|(t, s)| TowerView {
                name: t.name.clone(),
                kind: t.kind,
                at: t.at,
                control: s.control,
                holder: s.holder,
                contested: s.seen && t.contested,
                seen: s.seen,
                age: self.time - s.time,
            })
            .collect();
        let n = self.rules.drones;
        let k = |i: usize| i % n;
        let drones = (0..n)
            .map(|j| {
                let i = self.drone(team, j);
                let d = &self.drones[i];
                let linked = !self.jammed[i];
                // Out of contact, the team has only what the drone last sent.
                let (at, alt, vel, phase) = match map.drones[i] {
                    Some(s) if !linked => (s.at, s.alt, s.vel, s.phase),
                    _ => (d.at, d.alt, d.vel, d.phase),
                };
                OwnDrone {
                    k: j,
                    at,
                    alt,
                    vel,
                    phase,
                    order: None,
                    low: false,
                    linked,
                    silent: self.silent[i],
                    battery: self.reported[i],
                }
            })
            .collect();
        let enemies = self
            .drones
            .iter()
            .zip(&map.drones)
            .enumerate()
            .filter(|(_, (d, _))| d.team != team)
            .filter_map(|(i, (_, s))| {
                s.map(|s| EnemyDrone {
                    k: k(i),
                    at: s.at,
                    alt: s.alt,
                    vel: s.vel,
                    seen: s.seen,
                    age: self.time - s.time,
                })
            })
            .collect();
        TeamView {
            team,
            time: self.time,
            time_left: self.time_left(),
            score: self.score,
            outcome: self.outcome,
            towers,
            drones,
            enemies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rules;

    #[test]
    fn a_view_has_the_teams_drones_and_only_the_enemies_it_has_seen() {
        let mut g = Game::new(Rules::default());
        let blue = g.drone(Team::Blue, 0);
        let scout = g.drone(Team::Red, 0);
        g.drones[blue].at = Vec2::ZERO;
        g.drones[blue].phase = Phase::Flying;
        g.drones[scout].at = Vec2::new(100.0, 0.0);
        g.drones[scout].phase = Phase::Flying;
        g.step(0.1);
        g.drones[scout].at = Vec2::new(300.0, 250.0);
        g.step(2.0);
        let v = g.view(Team::Blue);
        assert_eq!(v.drones.len(), 3);
        assert_eq!((v.drones[0].k, v.drones[0].at), (0, Vec2::ZERO));
        assert_eq!(v.enemies.len(), 1, "only the scout was seen");
        let e = v.enemies[0];
        assert_eq!((e.k, e.at, e.seen), (0, Vec2::new(100.0, 0.0), false), "where it was last seen");
        assert!((e.age - 2.0).abs() < 1e-3);
    }

    #[test]
    fn a_view_has_the_towers_as_the_team_last_saw_them() {
        let mut g = Game::new(Rules::default());
        let east = g.tower("east");
        g.towers[east].holder = Some(Team::Red);
        g.towers[east].control = -1.0;
        g.step(1.0);
        let v = g.view(Team::Blue);
        assert_eq!(v.towers.len(), 5);
        assert_eq!(v.towers[east].name, "east");
        assert_eq!((v.towers[east].holder, v.towers[east].seen), (None, false));
        assert!((v.towers[east].age - 1.0).abs() < 1e-3);
        assert_eq!((v.towers[0].holder, v.towers[0].seen, v.towers[0].age), (Some(Team::Blue), true, 0.0));
    }

    #[test]
    fn a_view_goes_over_the_wire_and_back() {
        let g = Game::new(Rules::default());
        let v = g.view(Team::Red);
        let line = serde_json::to_string(&v).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<TeamView>(&line).unwrap(), v);
    }
}
