//! The radio shadow as a filter in the link, for where there is no simulated air.
//!
//! In the sim, the air in front of each drone drops the frames of a drone that the referee has in a
//! radio shadow. In the field there is no referee and no such air: the ground radio (or the drone) has
//! this filter. It knows who holds the towers and where the drones are, and it says which drones the
//! radio reaches by the same rule as the referee's (it asks a game of its own: [`Game::look`]).
//! Thus the rule is one rule on both platforms.

use crate::{Game, Phase, Rules, Team};
use glam::Vec2;

pub struct LinkFilter {
    game: Game,
}

impl LinkFilter {
    pub fn new(rules: Rules) -> LinkFilter {
        LinkFilter { game: Game::new(rules) }
    }

    /// Who holds each tower now (by its number in the game).
    pub fn set_holders(&mut self, holders: &[Option<Team>]) {
        for (t, h) in self.game.towers.iter_mut().zip(holders) {
            t.holder = *h;
        }
    }

    /// Where drone `i` (its number in the game) is, how high, and whether it flies.
    pub fn set_drone(&mut self, i: usize, at: Vec2, alt: f32, flying: bool) {
        let d = &mut self.game.drones[i];
        (d.at, d.alt, d.phase) = (at, alt, if flying { Phase::Flying } else { Phase::Ground });
    }

    /// Whether the radio reaches drone `i`: its frames, to and from it, go through.
    pub fn reaches(&mut self, i: usize) -> bool {
        self.game.look();
        !self.game.jammed[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drone_in_the_shadow_of_an_enemy_tower_is_not_reached_and_one_outside_it_is() {
        let rules = Rules::default();
        let mut f = LinkFilter::new(rules);
        let red_home = Game::new(rules).towers[4].at;
        f.set_holders(&[Some(Team::Blue), None, None, None, Some(Team::Red)]);
        f.set_drone(0, red_home - Vec2::new(100.0, 0.0), 50.0, true);
        f.set_drone(1, Vec2::new(-300.0, 0.0), 50.0, true);
        assert!(!f.reaches(0));
        assert!(f.reaches(1));
        f.set_holders(&[Some(Team::Blue), None, None, None, None]);
        assert!(f.reaches(0), "a tower nobody holds blocks no radio");
    }

    #[test]
    fn a_friend_outside_the_shadow_relays_and_the_filter_agrees_with_the_referee() {
        let rules = Rules::default();
        let mut referee = Game::new(rules);
        let mut f = LinkFilter::new(rules);
        let red_home = referee.towers[4].at;
        referee.towers[4].holder = Some(Team::Red);
        f.set_holders(&referee.towers.iter().map(|t| t.holder).collect::<Vec<_>>());
        // Every pair of places for two Blue drones on a grid round the Red home tower: the filter says
        // what the referee says.
        let spots: Vec<Vec2> = (0..8).map(|k| red_home - Vec2::new(60.0 + 30.0 * k as f32, 0.0)).collect();
        for &a in &spots {
            for &b in &spots {
                for (g, i, p) in [(0, 0, a), (0, 1, b)] {
                    let _ = g;
                    referee.drones[i] = crate::Drone { at: p, alt: 50.0, phase: Phase::Flying, ..referee.drones[i] };
                    f.set_drone(i, p, 50.0, true);
                }
                referee.look();
                for i in 0..2 {
                    assert_eq!(f.reaches(i), !referee.jammed[i], "drone {i} with the friend at {a} / {b}");
                }
            }
        }
    }
}
