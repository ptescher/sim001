//! Decisions on the drone, for when the radio is blocked (plans/edge.md): an [`Edge`] policy looks
//! at what a drone knows on board ([`EdgeObs`]) each second it is out of contact, and may change
//! its order. [`Edged`] puts one on any captain, over the standing order "fly the order on".

use crate::{Captain, Comparison, Persist, Prepared, compare_captains};
use recreation_towers::{Commander, EdgeObs, Order, Rules, Standing, Team, TeamView};

/// What a cut-off drone decides on its own.
pub trait Edge {
    fn name(&self) -> String;

    /// A new order for a drone that flies `order` and is out of contact, or `None` to go on.
    fn act(&self, team: Team, obs: &EdgeObs, order: Order) -> Option<Order>;
}

/// Goes on with the order: the baseline that any other policy has to beat.
pub struct Keep;

impl Edge for Keep {
    fn name(&self) -> String {
        "keep the order".into()
    }

    fn act(&self, _: Team, _: &EdgeObs, _: Order) -> Option<Order> {
        None
    }
}

/// Goes home when `n` enemy drones or more are in its camera, and it is taking a tower.
pub struct Outnumbered {
    pub n: usize,
}

impl Edge for Outnumbered {
    fn name(&self) -> String {
        format!("home when {} enemies are near", self.n)
    }

    fn act(&self, _: Team, obs: &EdgeObs, order: Order) -> Option<Order> {
        (matches!(order, Order::Take(_)) && obs.enemies.len() >= self.n).then_some(Order::Home)
    }
}

/// Goes home to recharge when the tower in its camera is held by its team, at the end of the bar,
/// and no enemy is near: the work is done, and a drone that goes on circling burns its battery.
pub struct FinishThenReturn;

impl Edge for FinishThenReturn {
    fn name(&self) -> String {
        "finish the capture, then return".into()
    }

    fn act(&self, team: Team, obs: &EdgeObs, order: Order) -> Option<Order> {
        let Order::Take(t) = order else { return None };
        let tower = obs.towers.iter().find(|s| s.index == t)?;
        (tower.holder == Some(team) && tower.control.abs() >= 1.0 && obs.enemies.is_empty()).then_some(Order::Home)
    }
}

/// A captain with an on-board policy: its orders go out with "fly the order on", and the policy
/// decides for a drone that is out of contact.
pub struct Edged<C, E> {
    pub captain: C,
    pub edge: E,
}

impl<C: Captain, E: Edge> Captain for Edged<C, E> {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        self.captain.decide(view)
    }

    fn standing(&mut self, _: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        orders.iter().map(|&(k, o)| (k, Standing { after: 0.0, then: o })).collect()
    }

    fn edge(&mut self, team: Team, obs: &EdgeObs, order: Order) -> Option<Order> {
        self.edge.act(team, obs, order)
    }
}

/// `edge` against a captain that flies the order on (the baseline), on paired seeds.
pub fn compare_edge(rules: Rules, edge: &dyn Edge, seeds: &[u64]) -> Result<Comparison, String> {
    struct Dyn<'a>(&'a dyn Edge);
    impl Edge for Dyn<'_> {
        fn name(&self) -> String {
            self.0.name()
        }

        fn act(&self, team: Team, obs: &EdgeObs, order: Order) -> Option<Order> {
            self.0.act(team, obs, order)
        }
    }
    compare_captains(
        rules,
        &|| Box::new(Edged { captain: Commander::new(rules).with_lookahead(false), edge: Dyn(edge) }),
        &|| Box::new(Prepared { captain: Commander::new(rules).with_lookahead(false), autonomy: Persist }),
        seeds,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use recreation_towers::{Class, EnemySeen, TowerSeen};

    fn obs(enemies: usize, tower: Option<TowerSeen>) -> EdgeObs {
        let enemy = EnemySeen { class: Class::Standard, offset: Vec2::ZERO, alt: 60.0 };
        EdgeObs {
            at: Vec2::ZERO,
            alt: 60.0,
            enemies: vec![enemy; enemies],
            towers: tower.into_iter().collect(),
            battery: Some(200.0),
            silent: 5.0,
        }
    }

    fn mine(control: f32) -> TowerSeen {
        TowerSeen { index: 2, control, holder: Some(Team::Blue), contested: false }
    }

    #[test]
    fn keep_goes_on() {
        assert_eq!(Keep.act(Team::Blue, &obs(3, None), Order::Take(2)), None);
    }

    #[test]
    fn outnumbered_goes_home_with_enough_enemies_near_and_only_when_taking_a_tower() {
        let p = Outnumbered { n: 2 };
        assert_eq!(p.act(Team::Blue, &obs(2, None), Order::Take(2)), Some(Order::Home));
        assert_eq!(p.act(Team::Blue, &obs(1, None), Order::Take(2)), None);
        assert_eq!(p.act(Team::Blue, &obs(3, None), Order::Hold), None);
    }

    #[test]
    fn finish_then_return_goes_home_when_the_tower_is_held_and_quiet() {
        let p = FinishThenReturn;
        assert_eq!(p.act(Team::Blue, &obs(0, Some(mine(1.0))), Order::Take(2)), Some(Order::Home));
        assert_eq!(p.act(Team::Blue, &obs(0, Some(mine(0.6))), Order::Take(2)), None);
        assert_eq!(p.act(Team::Blue, &obs(1, Some(mine(1.0))), Order::Take(2)), None);
        assert_eq!(p.act(Team::Blue, &obs(0, None), Order::Take(2)), None);
    }

    /// A team that keeps its order plays the same game as one with the standing order "fly on".
    #[test]
    fn an_edge_that_goes_on_plays_like_persist() {
        let c = compare_edge(Rules::default(), &Keep, &[1, 2, 3]).unwrap();
        assert!(c.margin.abs() < 1e-6, "{c:?}");
        assert_eq!(c.won, c.lost);
    }

    /// The hook is called: a policy that always sends a cut-off drone home changes the game.
    #[test]
    fn a_policy_that_sends_cut_off_drones_home_changes_the_game() {
        struct AlwaysHome;
        impl Edge for AlwaysHome {
            fn name(&self) -> String {
                "always home".into()
            }

            fn act(&self, _: Team, _: &EdgeObs, _: Order) -> Option<Order> {
                Some(Order::Home)
            }
        }
        let c = compare_edge(Rules::default(), &AlwaysHome, &[1, 2, 3]).unwrap();
        assert!(c.margin.abs() > 1e-3, "the policy never ran: {c:?}");
    }
}
