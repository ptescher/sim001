//! The advisor of a team: what it says to a person who commands, and only when it matters. It
//! looks at the team's map once a second (as the computer commander does) and uses nothing else,
//! so it knows nothing of the fog.

use crate::commander::DEFEND_RANGE;
use crate::{Phase, Rules, Team, TeamView};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What the advice is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdviceKind {
    /// An enemy drone is near a tower that the team holds.
    TowerInDanger,
    /// The team lost a tower it held.
    TowerLost,
    /// One of the team's drones is down.
    DroneDown,
    /// The clock runs out and the team is behind.
    Behind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Advice {
    pub kind: AdviceKind,
    pub text: String,
}

/// The advisor says a thing again, if it is still true, after this long (s).
const REPEAT_AFTER: f32 = 30.0;
/// In the last part of the game (s) a team that is behind hears of it.
const ENDGAME: f32 = 60.0;

pub struct Advisor {
    /// Who held each tower at the last look (empty before the first).
    held: Vec<Option<Team>>,
    /// Which drones were down at the last look.
    down: Vec<bool>,
    /// When each thing was last said (game time, s), for the things that are said again.
    said: HashMap<(AdviceKind, usize), f32>,
}

impl Advisor {
    pub fn new(_rules: Rules) -> Advisor {
        Advisor { held: vec![], down: vec![], said: HashMap::new() }
    }

    /// Looks at the map: what the advisor has to say now, if anything. Call it about once a second.
    pub fn look(&mut self, view: &TeamView) -> Vec<Advice> {
        let me = view.team;
        let mut say = vec![];
        let tower = |t: &crate::TowerView| format!("the {} tower", t.name);

        // A tower the team lost: said once, when it happens.
        if !self.held.is_empty() {
            for (t, v) in view.towers.iter().enumerate() {
                if self.held.get(t) == Some(&Some(me)) && v.holder != Some(me) {
                    say.push(Advice { kind: AdviceKind::TowerLost, text: format!("You lost {}.", tower(v)) });
                }
            }
        }
        self.held = view.towers.iter().map(|t| t.holder).collect();

        // An enemy drone near a tower the team holds.
        for (t, v) in view.towers.iter().enumerate().filter(|(_, v)| v.holder == Some(me)) {
            let near = view
                .enemies
                .iter()
                .filter(|e| e.seen)
                .map(|e| e.at.distance(v.at))
                .filter(|&d| d <= DEFEND_RANGE)
                .min_by(f32::total_cmp);
            if let Some(d) = near
                && self.due(AdviceKind::TowerInDanger, t, view.time)
            {
                let text = format!("{} is in danger: a {:?} drone is {d:.0} m away.", tower(v), me.other());
                say.push(Advice { kind: AdviceKind::TowerInDanger, text: capital(&text) });
            }
        }

        // A drone that is down: said once, until it is back.
        self.down.resize(view.drones.len(), false);
        for d in &view.drones {
            let down = d.phase == Phase::Down;
            if down && !self.down[d.k] {
                say.push(Advice {
                    kind: AdviceKind::DroneDown,
                    text: format!("{}{} is down.", &format!("{me:?}")[..1], d.k + 1),
                });
            }
            self.down[d.k] = down;
        }

        // The clock runs out and the team is behind.
        let (mine, theirs) = (view.score[me.index()], view.score[me.other().index()]);
        if view.outcome.is_none()
            && view.time_left > 0.0
            && view.time_left <= ENDGAME
            && mine < theirs
            && self.due(AdviceKind::Behind, 0, view.time)
        {
            say.push(Advice {
                kind: AdviceKind::Behind,
                text: format!("You are behind, {mine:.0} to {theirs:.0}, with {:.0} s left.", view.time_left),
            });
        }
        say
    }

    /// Whether a thing may be said now; if so, it is said.
    fn due(&mut self, kind: AdviceKind, subject: usize, now: f32) -> bool {
        match self.said.get(&(kind, subject)) {
            Some(&at) if now - at < REPEAT_AFTER => false,
            _ => {
                self.said.insert((kind, subject), now);
                true
            }
        }
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Game, Phase, Team};
    use glam::Vec2;

    fn setup() -> (Game, Advisor) {
        let g = Game::new(Rules::default());
        let a = Advisor::new(g.rules);
        (g, a)
    }

    fn kinds(a: Vec<Advice>) -> Vec<AdviceKind> {
        a.into_iter().map(|a| a.kind).collect()
    }

    #[test]
    fn an_enemy_near_a_held_tower_is_a_danger_with_the_towers_name() {
        let (mut g, mut a) = setup();
        let scout = g.drone(Team::Red, 0);
        g.drones[scout].at = g.towers[0].at + Vec2::new(0.0, 100.0);
        g.drones[scout].phase = Phase::Flying;
        g.drones[scout].alt = 50.0;
        g.step(0.1);
        let said = a.look(&g.view(Team::Blue));
        assert_eq!(kinds(said.clone()), [AdviceKind::TowerInDanger]);
        assert!(said[0].text.contains(&g.towers[0].name), "{}", said[0].text);
    }

    #[test]
    fn a_danger_is_said_again_only_after_the_cooldown() {
        let (mut g, mut a) = setup();
        let scout = g.drone(Team::Red, 0);
        g.drones[scout].at = g.towers[0].at + Vec2::new(0.0, 100.0);
        g.drones[scout].phase = Phase::Flying;
        g.drones[scout].alt = 50.0;
        g.step(0.1);
        assert_eq!(a.look(&g.view(Team::Blue)).len(), 1);
        g.step(10.0);
        assert!(a.look(&g.view(Team::Blue)).is_empty());
        g.step(25.0);
        assert_eq!(a.look(&g.view(Team::Blue)).len(), 1);
    }

    #[test]
    fn a_drone_that_is_down_is_said_once_until_it_is_back() {
        let (mut g, mut a) = setup();
        let i = g.drone(Team::Blue, 1);
        g.drones[i].phase = Phase::Down;
        assert_eq!(kinds(a.look(&g.view(Team::Blue))), [AdviceKind::DroneDown]);
        g.step(60.0);
        assert!(a.look(&g.view(Team::Blue)).is_empty());
        g.drones[i].phase = Phase::Ground;
        assert!(a.look(&g.view(Team::Blue)).is_empty());
        g.drones[i].phase = Phase::Down;
        assert_eq!(kinds(a.look(&g.view(Team::Blue))), [AdviceKind::DroneDown]);
    }

    #[test]
    fn a_tower_that_the_team_sees_go_to_the_enemy_is_lost() {
        let (mut g, mut a) = setup();
        let west = g.tower("west");
        g.towers[west].holder = Some(Team::Blue);
        g.towers[west].control = 1.0;
        let d = g.drone(Team::Blue, 0);
        g.drones[d].at = g.towers[west].at + Vec2::new(0.0, 130.0);
        g.drones[d].phase = Phase::Flying;
        g.drones[d].alt = 50.0;
        g.look();
        assert!(a.look(&g.view(Team::Blue)).is_empty());
        g.towers[west].holder = Some(Team::Red);
        g.towers[west].control = -1.0;
        g.look();
        let said = a.look(&g.view(Team::Blue));
        assert_eq!(kinds(said.clone()), [AdviceKind::TowerLost]);
        assert!(said[0].text.contains("west"), "{}", said[0].text);
    }

    #[test]
    fn the_first_look_does_not_call_a_held_tower_lost() {
        let (g, mut a) = setup();
        assert!(a.look(&g.view(Team::Blue)).is_empty());
    }

    #[test]
    fn a_team_that_is_ahead_hears_nothing_at_the_end() {
        let (mut g, mut a) = setup();
        g.time = g.rules.duration - 30.0;
        g.score = [200.0, 100.0];
        assert!(a.look(&g.view(Team::Blue)).is_empty());
        g.score = [100.0, 200.0];
        assert_eq!(kinds(a.look(&g.view(Team::Blue))), [AdviceKind::Behind]);
    }
}
