//! The team's map as a ground station makes it: its own drones are where their telemetry last put
//! them, and an enemy is on it only if a drone, a tower or the base reported it. Nothing is added
//! that nobody told the station.

use glam::Vec2;
use recreation_towers::{EnemyDrone, Rules, TeamView};

/// A drone is in contact if its last position report is no older than this (s of the game).
pub const LINKED_WITHIN: f32 = 3.0;
/// An enemy a drone reported is seen now if the report is no older than this (s of the game): the
/// sensor reports once a second.
pub const SEEN_WITHIN: f32 = 2.5;

/// What a drone last said of itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnReport {
    pub at: Vec2,
    pub alt: f32,
    pub vel: Vec2,
    /// Since the report came (s of the game).
    pub age: f32,
}

/// An enemy that a drone's sensor reported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seen {
    /// Its number in the game.
    pub enemy: usize,
    pub at: Vec2,
    pub alt: f32,
    pub vel: Vec2,
    /// Since the report came (s of the game).
    pub age: f32,
}

/// The map of the team: `site` is what the server (the towers and the base, the clock and the score)
/// tells the station; `own[k]` is the last report of the team's drone `k`, and `seen` is what the
/// drones' sensors reported.
pub fn build(site: &TeamView, own: &[Option<OwnReport>], seen: &[Seen], rules: &Rules) -> TeamView {
    let mut view = site.clone();
    for (d, report) in view.drones.iter_mut().zip(own) {
        if let Some(r) = report {
            (d.at, d.alt, d.vel) = (r.at, r.alt, r.vel);
            d.linked = r.age <= LINKED_WITHIN;
            d.silent = if d.linked { 0.0 } else { r.age };
        }
    }
    let n = rules.drones;
    for s in seen {
        let k = s.enemy % n;
        // An enemy of the other team, by its number in the game.
        if (s.enemy / n == site.team.index()) || s.enemy >= 2 * n {
            continue;
        }
        let mine = EnemyDrone { k, at: s.at, alt: s.alt, vel: s.vel, seen: s.age <= SEEN_WITHIN, age: s.age };
        match view.enemies.iter_mut().find(|e| e.k == k) {
            // The fresher report wins: the sites' picture of it, or the drone's.
            Some(e) if e.seen && !mine.seen || e.age <= mine.age && (e.seen || !mine.seen) => {}
            Some(e) => *e = mine,
            None => view.enemies.push(mine),
        }
    }
    view.enemies.sort_by_key(|e| e.k);
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use recreation_towers::{Game, Phase, Team};

    fn site() -> (TeamView, Rules) {
        let rules = Rules::default();
        (Game::new(rules).site_view(Team::Blue), rules)
    }

    fn report(x: f32, age: f32) -> Option<OwnReport> {
        Some(OwnReport { at: Vec2::new(x, 5.0), alt: 50.0, vel: Vec2::new(1.0, 0.0), age })
    }

    #[test]
    fn a_drone_is_where_its_last_report_puts_it_and_dim_when_it_has_not_been_heard_lately() {
        let (site, rules) = site();
        let mut own = vec![None; rules.drones];
        own[0] = report(100.0, 0.5);
        own[1] = report(200.0, 12.0);
        let v = build(&site, &own, &[], &rules);
        assert_eq!((v.drones[0].at, v.drones[0].linked, v.drones[0].silent), (Vec2::new(100.0, 5.0), true, 0.0));
        assert_eq!((v.drones[1].at, v.drones[1].linked, v.drones[1].silent), (Vec2::new(200.0, 5.0), false, 12.0));
        assert_eq!(v.drones[2].at, site.drones[2].at, "no report, nothing new");
        assert_eq!(v.drones[0].phase, Phase::Ground);
    }

    #[test]
    fn nothing_reported_shows_no_enemy() {
        let (site, rules) = site();
        let v = build(&site, &vec![None; rules.drones], &[], &rules);
        assert!(v.enemies.is_empty());
    }

    #[test]
    fn an_enemy_a_drone_reported_is_on_the_map_seen_while_the_report_is_fresh_and_dim_after() {
        let (site, rules) = site();
        let n = rules.drones;
        let own = vec![None; n];
        let red2 = n + 2;
        let at = Vec2::new(300.0, 20.0);
        let fresh = Seen { enemy: red2, at, alt: 60.0, vel: Vec2::ZERO, age: 1.0 };
        let v = build(&site, &own, &[fresh], &rules);
        assert_eq!(v.enemies.len(), 1);
        assert_eq!((v.enemies[0].k, v.enemies[0].at, v.enemies[0].seen), (2, at, true));
        let old = Seen { age: 9.0, ..fresh };
        let v = build(&site, &own, &[old], &rules);
        assert_eq!((v.enemies[0].seen, v.enemies[0].age), (false, 9.0), "last reported, with its age");
    }

    #[test]
    fn a_report_of_the_teams_own_drone_is_not_an_enemy() {
        let (site, rules) = site();
        let own_drone = Seen { enemy: 1, at: Vec2::ZERO, alt: 50.0, vel: Vec2::ZERO, age: 0.0 };
        let v = build(&site, &vec![None; rules.drones], &[own_drone], &rules);
        assert!(v.enemies.is_empty());
    }

    #[test]
    fn the_fresher_report_wins_between_a_drone_and_a_tower() {
        let (mut site, rules) = site();
        let n = rules.drones;
        site.enemies.push(EnemyDrone {
            k: 0,
            at: Vec2::new(1.0, 1.0),
            alt: 10.0,
            vel: Vec2::ZERO,
            seen: false,
            age: 30.0,
        });
        let drone = Seen { enemy: n, at: Vec2::new(2.0, 2.0), alt: 20.0, vel: Vec2::ZERO, age: 0.5 };
        let v = build(&site, &vec![None; n], &[drone], &rules);
        assert_eq!((v.enemies[0].at, v.enemies[0].seen), (Vec2::new(2.0, 2.0), true), "the drone saw it later");
        site.enemies[0] = EnemyDrone { seen: true, age: 0.0, ..site.enemies[0] };
        let v = build(&site, &vec![None; n], &[Seen { age: 1.0, ..drone }], &rules);
        assert_eq!(v.enemies[0].at, Vec2::new(1.0, 1.0), "the tower sees it now");
    }
}
