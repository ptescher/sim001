//! A captain that reacts with the v4 tools (plans/towers-rules-v4.md, results "Where the v4 rules
//! stand"): the default commander chooses the towers, and the tactician uses what each drone
//! carries. A tagger hunts, a drone attacking a tower its team does not hold goes quiet, a jammer is
//! on near the tower it attacks, and a drone sent home lands for a swap.

use crate::Captain;
use recreation_towers::{Advise, Commander, Order, Payload, Posture, Rules, TeamView};

/// How near the tower it attacks (m) a jammer switches on.
pub const JAM_NEAR: f32 = 250.0;

pub struct Tactician {
    pub commander: Commander,
    rules: Rules,
    /// Attackers go quiet; if not, they stay loud.
    pub quiet_attacks: bool,
    /// A held tower that an enemy drone comes near gets a defender.
    pub defend: bool,
}

/// An enemy drone this near a held tower (m) calls a defender.
pub const DEFEND_NEAR: f32 = 250.0;

impl Tactician {
    pub fn new(rules: Rules) -> Tactician {
        Tactician { commander: Commander::new(rules), rules, quiet_attacks: true, defend: false }
    }

    pub fn loud(self) -> Tactician {
        Tactician { quiet_attacks: false, ..self }
    }

    /// The same, sending a drone to a held tower that an enemy drone comes near.
    pub fn defending(self) -> Tactician {
        Tactician { defend: true, ..self }
    }

    /// Sends a defender to each held tower that an enemy drone the team sees is near, if none of
    /// its drones is on it: the nearest drone that is not a tagger, and not going home.
    fn defend(&self, view: &TeamView, orders: &mut [(usize, Order)]) {
        for (t, tower) in view.towers.iter().enumerate() {
            let threatened = tower.holder == Some(view.team)
                && view.enemies.iter().any(|e| e.seen && e.at.distance(tower.at) <= DEFEND_NEAR);
            if !threatened || orders.iter().any(|&(_, o)| o == Order::Take(t)) {
                continue;
            }
            let free =
                |k: usize| view.drones.iter().find(|d| d.k == k).filter(|d| d.payload != Payload::Tagger && d.linked);
            let nearest = orders
                .iter_mut()
                .filter(|(k, o)| !matches!(o, Order::Home | Order::Land) && free(*k).is_some())
                .min_by(|a, b| {
                    let dist = |k| free(k).map_or(f32::MAX, |d| d.at.distance(tower.at));
                    dist(a.0).total_cmp(&dist(b.0))
                });
            if let Some(o) = nearest {
                o.1 = Order::Take(t);
            }
        }
    }
}

impl Captain for Tactician {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        let swap = self.rules.swap_time > 0.0;
        let mut orders = self.commander.decide(view);
        if self.defend {
            self.defend(view, &mut orders);
        }
        Ok(orders
            .into_iter()
            .map(|(k, o)| {
                let d = view.drones.iter().find(|d| d.k == k);
                let order = match (o, d.map(|d| d.payload)) {
                    // Home to recharge is a landing for a swap, where there is one.
                    (Order::Home, _) if swap => Order::Land,
                    (Order::Home, _) => Order::Home,
                    // A tagger counts for nothing in a circle: it hunts instead.
                    (_, Some(Payload::Tagger)) => crate::formation::hunt(view, d.map_or(glam::Vec2::ZERO, |d| d.at)),
                    (o, _) => o,
                };
                (k, order)
            })
            .collect())
    }

    fn posture(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Posture)> {
        orders
            .iter()
            .filter_map(|&(k, o)| {
                let d = view.drones.iter().find(|d| d.k == k)?;
                let target = attacks(view, o);
                let p = match target {
                    Some(at) if d.payload == Payload::Jammer && d.at.distance(at) <= JAM_NEAR => Posture::Jam,
                    Some(_) if self.quiet_attacks => Posture::Quiet,
                    _ => Posture::Loud,
                };
                Some((k, p))
            })
            .collect()
    }

    fn advised_by(&mut self, advisor: Box<dyn Advise>, delay: f32) -> bool {
        self.commander.add_advisor(advisor, delay);
        true
    }
}

/// Where the tower is that `order` attacks: a tower the team does not hold.
fn attacks(view: &TeamView, order: Order) -> Option<glam::Vec2> {
    match order {
        Order::Take(t) if view.towers[t].holder != Some(view.team) => Some(view.towers[t].at),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use recreation_towers::{EnemyDrone, Game, Phase, Team};

    fn view() -> TeamView {
        let mut g = Game::new(Rules::v4());
        g.look();
        let mut v = g.view(Team::Blue);
        for d in &mut v.drones {
            d.phase = Phase::Flying;
            d.battery = 200.0;
        }
        v
    }

    #[test]
    fn a_tagger_hunts_what_its_team_sees_and_never_stands_in_a_circle() {
        let mut t = Tactician::new(Rules::v4());
        let mut v = view();
        v.drones[4].payload = Payload::Tagger;
        let o = t.decide(&v).unwrap();
        assert!(!matches!(o.iter().find(|(k, _)| *k == 4).unwrap().1, Order::Take(_)), "{o:?}");
        let at = v.drones[4].at + Vec2::new(100.0, 0.0);
        v.enemies =
            vec![EnemyDrone { k: 3, class: None, payload: None, at, alt: 50.0, vel: Vec2::ZERO, seen: true, age: 0.0 }];
        let o = t.decide(&v).unwrap();
        assert_eq!(o.iter().find(|(k, _)| *k == 4).unwrap().1, Order::Intercept(3));
    }

    #[test]
    fn an_attacker_goes_quiet_a_defender_stays_loud_and_a_jammer_is_on_near_its_target() {
        let mut t = Tactician::new(Rules::v4());
        let mut v = view();
        v.drones[2].payload = Payload::Jammer;
        let (west, home) = (v.towers[1].at, v.towers[0].at);
        v.drones[2].at = west + Vec2::new(200.0, 0.0);
        let orders = vec![(0, Order::Take(1)), (1, Order::Take(0)), (2, Order::Take(1)), (3, Order::Hold)];
        let p = t.posture(&v, &orders);
        let of = |k| p.iter().find(|(j, _)| *j == k).map(|&(_, p)| p);
        assert_eq!(of(0), Some(Posture::Quiet), "it attacks the west tower");
        assert_eq!(of(1), Some(Posture::Loud), "it defends its home tower");
        assert_eq!(of(2), Some(Posture::Jam), "a jammer 200 m from the tower it attacks");
        v.drones[2].at = home;
        assert_eq!(t.posture(&v, &orders).iter().find(|(j, _)| *j == 2).unwrap().1, Posture::Quiet, "far from it");
        assert!(t.loud().posture(&v, &orders).iter().all(|&(_, p)| p != Posture::Quiet));
    }

    #[test]
    fn a_defender_sends_its_nearest_drone_to_a_held_tower_with_an_enemy_near_it() {
        let mut t = Tactician::new(Rules::v4()).defending();
        let mut v = view();
        let home = v.towers[0].at;
        v.drones[4].payload = Payload::Tagger;
        v.drones[4].at = home;
        v.drones[1].at = home + Vec2::new(30.0, 0.0);
        let near = |k| EnemyDrone {
            k,
            class: None,
            payload: None,
            at: home + Vec2::new(150.0, 0.0),
            alt: 17.0,
            vel: Vec2::ZERO,
            seen: true,
            age: 0.0,
        };
        v.enemies = vec![near(0)];
        let o = t.decide(&v).unwrap();
        let of = |k| o.iter().find(|(j, _)| *j == k).unwrap().1;
        assert_eq!(of(1), Order::Take(0), "the nearest drone that is not a tagger: {o:?}");
        assert_ne!(of(4), Order::Take(0), "a tagger hunts, it does not hold");
        let mut quiet = Tactician::new(Rules::v4());
        assert_ne!(
            quiet.decide(&v).unwrap().iter().find(|(j, _)| *j == 1).unwrap().1,
            Order::Take(0),
            "one that does not defend"
        );
    }

    #[test]
    fn a_drone_sent_home_lands_for_a_swap() {
        let mut t = Tactician::new(Rules::v4());
        let mut v = view();
        v.drones[0].battery = 20.0;
        let o = t.decide(&v).unwrap();
        assert_eq!(o.iter().find(|(k, _)| *k == 0).unwrap().1, Order::Land, "{o:?}");
    }
}
