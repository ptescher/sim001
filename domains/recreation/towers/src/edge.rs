//! What a drone knows on board ([`EdgeObs`]): what its own camera sees, its battery and the time
//! since it was last in contact. A drone in a radio shadow has nothing else, so an on-board
//! decision uses this and no more (plans/edge.md).

use crate::{Class, Game, Phase, Team};
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// An enemy drone that the camera sees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnemySeen {
    pub class: Class,
    /// From the drone to the enemy: north, east (m).
    pub offset: Vec2,
    /// Height above the field (m).
    pub alt: f32,
}

/// A tower that the camera sees, as it is now.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TowerSeen {
    /// Its number in the game.
    pub index: usize,
    /// From −1 (Red) to +1 (Blue).
    pub control: f32,
    pub holder: Option<Team>,
    pub contested: bool,
}

/// What one drone knows on board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeObs {
    /// North, east of the centre (m), and the height (m).
    pub at: Vec2,
    pub alt: f32,
    /// The enemy drones in the range of the camera, and not hidden by the trees.
    pub enemies: Vec<EnemySeen>,
    /// The towers in the range of the camera.
    pub towers: Vec<TowerSeen>,
    /// What is left of the battery (s); `None` in a game with no batteries.
    pub battery: Option<f32>,
    /// How long it has been out of contact (s); 0 while it is in contact.
    pub silent: f32,
}

impl Game {
    /// What drone `i` knows on board now.
    pub fn edge_obs(&self, i: usize) -> EdgeObs {
        let d = &self.drones[i];
        let down = d.phase == Phase::Down;
        let enemies = self
            .drone_senses(i)
            .into_iter()
            .map(|j| {
                let e = &self.drones[j];
                EnemySeen { class: e.class, offset: e.at - d.at, alt: e.alt }
            })
            .collect();
        let towers = self
            .towers
            .iter()
            .enumerate()
            .filter(|(_, t)| !down && t.at.distance(d.at) <= self.rules.drone_vision)
            .map(|(index, t)| TowerSeen { index, control: t.control, holder: t.holder, contested: t.contested })
            .collect();
        EdgeObs { at: d.at, alt: d.alt, enemies, towers, battery: self.charge(i), silent: self.silent[i] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rules;

    fn game() -> Game {
        let mut g = Game::new(Rules::default());
        g.look();
        g
    }

    fn fly(g: &mut Game, i: usize, at: Vec2) {
        g.drones[i].at = at;
        g.drones[i].alt = 60.0;
        g.drones[i].phase = Phase::Flying;
    }

    #[test]
    fn a_drone_sees_an_enemy_in_its_range_and_not_one_outside() {
        let mut g = game();
        let (blue, red) = (g.drone(Team::Blue, 0), g.drone(Team::Red, 0));
        fly(&mut g, blue, Vec2::ZERO);
        fly(&mut g, red, Vec2::new(0.0, 100.0));
        assert_eq!(g.edge_obs(blue).enemies.len(), 1);
        assert_eq!(g.edge_obs(blue).enemies[0].offset, Vec2::new(0.0, 100.0));
        fly(&mut g, red, Vec2::new(0.0, 200.0));
        assert!(g.edge_obs(blue).enemies.is_empty());
    }

    #[test]
    fn a_drone_does_not_see_its_own_team() {
        let mut g = game();
        let (a, b) = (g.drone(Team::Blue, 0), g.drone(Team::Blue, 1));
        fly(&mut g, a, Vec2::ZERO);
        fly(&mut g, b, Vec2::new(0.0, 50.0));
        assert!(g.edge_obs(a).enemies.is_empty());
    }

    #[test]
    fn a_drone_sees_the_control_of_a_tower_in_range_only() {
        let mut g = game();
        let west = g.tower("west");
        g.towers[west].control = 0.5;
        let a = g.drone(Team::Blue, 0);
        let at = g.towers[west].at + Vec2::new(40.0, 0.0);
        fly(&mut g, a, at);
        let seen: Vec<_> = g.edge_obs(a).towers.iter().map(|t| (t.index, t.control)).collect();
        assert_eq!(seen, vec![(west, 0.5)]);
        fly(&mut g, a, Vec2::ZERO);
        assert!(g.edge_obs(a).towers.iter().all(|t| t.index != west));
    }

    #[test]
    fn it_carries_the_time_out_of_contact_and_the_battery() {
        let mut g = game();
        let a = g.drone(Team::Blue, 0);
        g.silent[a] = 12.0;
        let obs = g.edge_obs(a);
        assert_eq!(obs.silent, 12.0);
        assert_eq!(obs.battery, g.charge(a));
    }
}
