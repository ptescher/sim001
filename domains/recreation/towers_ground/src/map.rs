//! The team's map as a ground station makes it: its own drones are where their telemetry last put
//! them, and an enemy is on it only if a drone, a tower or the base reported it. Nothing is added
//! that nobody told the station.

use glam::Vec2;
use recreation_towers::{EnemyDrone, Game, Order, OwnDrone, Phase, Rules, Sites, TeamView, Umpire};

/// A drone is in contact if its last position report is no older than this (s of the game).
pub const LINKED_WITHIN: f32 = 3.0;
/// A drone higher than this (m) flies.
pub const FLYING_ABOVE: f32 = 2.0;
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

/// The map of the team, from the umpire's word, what the team's `sites` report, the last report of
/// each of the team's drones (`own[k]`) and what the drones' sensors reported (`seen`). `orders[k]`
/// is the order the commander gave drone `k` and whether it is low. A drone nobody has heard from
/// yet stands at its base.
pub fn build(
    umpire: &Umpire,
    sites: &Sites,
    own: &[Option<OwnReport>],
    seen: &[Seen],
    orders: &[(Option<Order>, bool)],
    rules: &Rules,
) -> TeamView {
    let team = sites.team;
    let start = Game::new(*rules);
    let drones = (0..rules.drones)
        .map(|k| {
            let base = &start.drones[team.index() * rules.drones + k];
            let (order, low) = orders.get(k).copied().unwrap_or((None, false));
            let d = OwnDrone {
                k,
                at: base.at,
                alt: 0.0,
                vel: Vec2::ZERO,
                phase: Phase::Ground,
                order,
                low,
                linked: true,
                silent: 0.0,
                battery: 0.0,
            };
            match own.get(k).copied().flatten() {
                Some(r) => {
                    let linked = r.age <= LINKED_WITHIN;
                    OwnDrone {
                        at: r.at,
                        alt: r.alt,
                        vel: r.vel,
                        phase: if r.alt > FLYING_ABOVE { Phase::Flying } else { Phase::Ground },
                        linked,
                        silent: if linked { 0.0 } else { r.age },
                        ..d
                    }
                }
                None => d,
            }
        })
        .collect();
    let mut view = TeamView {
        team,
        time: umpire.time,
        time_left: umpire.time_left,
        score: umpire.score,
        outcome: umpire.outcome,
        towers: sites.towers.clone(),
        drones,
        enemies: sites.enemies.clone(),
    };
    let n = rules.drones;
    for s in seen {
        let k = s.enemy % n;
        // An enemy of the other team, by its number in the game.
        if (s.enemy / n == team.index()) || s.enemy >= 2 * n {
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
    use recreation_towers::Team;

    fn word() -> (Umpire, Sites, Rules) {
        let rules = Rules::default();
        let g = Game::new(rules);
        (g.umpire(), g.sites(Team::Blue), rules)
    }

    fn report(x: f32, age: f32) -> Option<OwnReport> {
        Some(OwnReport { at: Vec2::new(x, 5.0), alt: 50.0, vel: Vec2::new(1.0, 0.0), age })
    }

    fn make(own: &[Option<OwnReport>], seen: &[Seen]) -> TeamView {
        let (u, s, rules) = word();
        build(&u, &s, own, seen, &[], &rules)
    }

    #[test]
    fn a_drone_is_where_its_last_report_puts_it_and_dim_when_it_has_not_been_heard_lately() {
        let n = Rules::default().drones;
        let mut own = vec![None; n];
        own[0] = report(100.0, 0.5);
        own[1] = report(200.0, 12.0);
        let v = make(&own, &[]);
        assert_eq!((v.drones[0].at, v.drones[0].linked, v.drones[0].silent), (Vec2::new(100.0, 5.0), true, 0.0));
        assert_eq!(v.drones[0].phase, Phase::Flying);
        assert_eq!((v.drones[1].at, v.drones[1].linked, v.drones[1].silent), (Vec2::new(200.0, 5.0), false, 12.0));
        let base = Game::new(Rules::default()).drones[2].at;
        assert_eq!((v.drones[2].at, v.drones[2].phase), (base, Phase::Ground), "no report: at its base");
    }

    #[test]
    fn the_map_has_the_umpires_word_and_the_orders_the_commander_gave() {
        let (mut u, s, rules) = word();
        u.score = [12.0, 7.0];
        u.time_left = 480.0;
        let orders = [(Some(Order::Hold), true)];
        let v = build(&u, &s, &vec![None; rules.drones], &[], &orders, &rules);
        assert_eq!((v.score, v.time_left), ([12.0, 7.0], 480.0));
        assert_eq!((v.drones[0].order, v.drones[0].low), (Some(Order::Hold), true));
        assert_eq!(v.drones[1].order, None);
        assert_eq!(v.towers.len(), 5);
    }

    #[test]
    fn nothing_reported_shows_no_enemy() {
        let v = make(&vec![None; Rules::default().drones], &[]);
        assert!(v.enemies.is_empty());
    }

    #[test]
    fn an_enemy_a_drone_reported_is_on_the_map_seen_while_the_report_is_fresh_and_dim_after() {
        let n = Rules::default().drones;
        let own = vec![None; n];
        let at = Vec2::new(300.0, 20.0);
        let fresh = Seen { enemy: n + 2, at, alt: 60.0, vel: Vec2::ZERO, age: 1.0 };
        let v = make(&own, &[fresh]);
        assert_eq!(v.enemies.len(), 1);
        assert_eq!((v.enemies[0].k, v.enemies[0].at, v.enemies[0].seen), (2, at, true));
        let v = make(&own, &[Seen { age: 9.0, ..fresh }]);
        assert_eq!((v.enemies[0].seen, v.enemies[0].age), (false, 9.0), "last reported, with its age");
    }

    #[test]
    fn a_report_of_the_teams_own_drone_is_not_an_enemy() {
        let own_drone = Seen { enemy: 1, at: Vec2::ZERO, alt: 50.0, vel: Vec2::ZERO, age: 0.0 };
        let v = make(&vec![None; Rules::default().drones], &[own_drone]);
        assert!(v.enemies.is_empty());
    }

    #[test]
    fn the_fresher_report_wins_between_a_drone_and_a_tower() {
        let (u, mut s, rules) = word();
        let n = rules.drones;
        s.enemies.push(EnemyDrone {
            k: 0,
            at: Vec2::new(1.0, 1.0),
            alt: 10.0,
            vel: Vec2::ZERO,
            seen: false,
            age: 30.0,
        });
        let drone = Seen { enemy: n, at: Vec2::new(2.0, 2.0), alt: 20.0, vel: Vec2::ZERO, age: 0.5 };
        let v = build(&u, &s, &vec![None; n], &[drone], &[], &rules);
        assert_eq!((v.enemies[0].at, v.enemies[0].seen), (Vec2::new(2.0, 2.0), true), "the drone saw it later");
        s.enemies[0] = EnemyDrone { seen: true, age: 0.0, ..s.enemies[0] };
        let v = build(&u, &s, &vec![None; n], &[Seen { age: 1.0, ..drone }], &[], &rules);
        assert_eq!(v.enemies[0].at, Vec2::new(1.0, 1.0), "the tower sees it now");
    }
}
