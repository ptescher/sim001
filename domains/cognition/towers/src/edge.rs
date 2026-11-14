//! Decisions on the drone, for when the radio is blocked (plans/edge.md): an [`Edge`] policy looks
//! at what a drone knows on board ([`EdgeObs`]) each second it is out of contact, and may change
//! its order. [`Edged`] puts one on any captain, over the standing order "fly the order on". A policy
//! is not a unit of its own: [`EdgeUnit`] is the default unit (the autopilot) with the policy asked
//! once a second while the drone is out of contact.

use crate::{Captain, Comparison, Persist, Prepared, compare_captains};
use recreation_towers::{
    Autopilot, Commander, EdgeObs, Instruction, OnBoard, Order, Rules, Standing, Team, TeamView, Unit,
};
use std::rc::Rc;

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

impl<E: Edge + ?Sized> Edge for Rc<E> {
    fn name(&self) -> String {
        (**self).name()
    }

    fn act(&self, team: Team, obs: &EdgeObs, order: Order) -> Option<Order> {
        (**self).act(team, obs, order)
    }
}

/// The default unit with an on-board policy: it flies as the autopilot does, and once a second,
/// while the drone is flying out of contact, asks `edge` what to do from what the drone sees. A
/// new order from the policy replaces the queue: the failsafe still holds.
pub struct EdgeUnit<E> {
    team: Team,
    edge: E,
    next: f32,
}

impl<E: Edge> EdgeUnit<E> {
    pub fn new(team: Team, edge: E) -> EdgeUnit<E> {
        EdgeUnit { team, edge, next: 0.0 }
    }
}

impl<E: Edge> Unit for EdgeUnit<E> {
    fn fly(&mut self, on: &OnBoard, told: &mut Instruction) -> Option<Order> {
        Autopilot.fly(on, told);
        let now = on.time() >= self.next;
        if now {
            self.next += 1.0;
        }
        if now
            && !on.in_contact()
            && on.flying()
            && let Some(order) = told.order
            && let Some(new) = self.edge.act(self.team, on.obs(), order)
            && new != order
        {
            told.decide(new);
        }
        told.order
    }
}

/// A captain with an on-board policy: its orders go out with "fly the order on", and the policy
/// decides for a drone that is out of contact.
pub struct Edged<C, E> {
    pub captain: C,
    pub edge: E,
}

impl<C: Captain, E: Edge + Clone + 'static> Captain for Edged<C, E> {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        self.captain.decide(view)
    }

    fn standing(&mut self, _: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        orders.iter().map(|&(k, o)| (k, Standing { after: 0.0, then: o })).collect()
    }

    /// Each drone gets its own copy of the policy: the policy is all the captain gives a drone.
    fn unit(&self, team: Team, _: usize) -> Box<dyn Unit> {
        Box::new(EdgeUnit::new(team, self.edge.clone()))
    }
}

/// `edge` against a captain that flies the order on (the baseline), on paired seeds.
pub fn compare_edge(rules: Rules, edge: Rc<dyn Edge>, seeds: &[u64]) -> Result<Comparison, String> {
    compare_captains(
        rules,
        &|| Box::new(Edged { captain: Commander::new(rules).with_lookahead(false), edge: edge.clone() }),
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
        let c = compare_edge(Rules::default(), Rc::new(Keep), &[1, 2, 3]).unwrap();
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
        let c = compare_edge(Rules::default(), Rc::new(AlwaysHome), &[1, 2, 3]).unwrap();
        assert!(c.margin.abs() > 1e-3, "the policy never ran: {c:?}");
    }
}
