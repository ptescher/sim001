//! What a drone does on its own when a radio shadow cuts it off ([`Autonomy`]), and a paired
//! comparison of two such policies in the arena ([`compare`]).
//!
//! A commander cannot reach a drone in a shadow, so it prepares the drone while it is in contact:
//! each order goes out with a standing order ([`recreation_towers::Standing`]) chosen from the
//! team's map. An [`Autonomy`] chooses it. [`Prepared`] puts one on any captain.

use crate::{Captain, TowersResult, play};
use recreation_towers::{Commander, Order, Rules, Standing, Team, TeamView};

/// Chooses the standing order that goes with an order.
pub trait Autonomy {
    fn name(&self) -> String;

    /// The standing order for drone `k`, given `order` now, from what the team's map shows now.
    /// `None` is the failsafe: home after the link is lost.
    fn standing(&self, view: &TeamView, k: usize, order: Order) -> Option<Standing>;
}

impl<A: Autonomy + ?Sized> Autonomy for &A {
    fn name(&self) -> String {
        (**self).name()
    }

    fn standing(&self, view: &TeamView, k: usize, order: Order) -> Option<Standing> {
        (**self).standing(view, k, order)
    }
}

/// No standing order: out of contact, a drone flies its order, and home when the link is lost.
pub struct Failsafe;

impl Autonomy for Failsafe {
    fn name(&self) -> String {
        "failsafe".into()
    }

    fn standing(&self, _: &TeamView, _: usize, _: Order) -> Option<Standing> {
        None
    }
}

/// A drone out of contact flies its order on, as long as the shadow lasts.
pub struct Persist;

impl Autonomy for Persist {
    fn name(&self) -> String {
        "persist".into()
    }

    fn standing(&self, _: &TeamView, _: usize, order: Order) -> Option<Standing> {
        Some(Standing { after: 0.0, then: order })
    }
}

/// A drone out of contact for `after` s does `then`.
pub struct After {
    pub after: f32,
    pub then: Order,
}

impl Autonomy for After {
    fn name(&self) -> String {
        format!("{:?} after {} s", self.then, self.after)
    }

    fn standing(&self, _: &TeamView, _: usize, _: Order) -> Option<Standing> {
        Some(Standing { after: self.after, then: self.then })
    }
}

/// A captain with an autonomy: its orders go out with the autonomy's standing orders.
pub struct Prepared<C, A> {
    pub captain: C,
    pub autonomy: A,
}

impl<C: Captain, A: Autonomy> Captain for Prepared<C, A> {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        self.captain.decide(view)
    }

    fn standing(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        orders.iter().filter_map(|&(k, o)| self.autonomy.standing(view, k, o).map(|s| (k, s))).collect()
    }
}

/// How `a` did against `b` over some seeds, each played on both sides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comparison {
    pub games: usize,
    pub won: usize,
    pub lost: usize,
    pub drawn: usize,
    /// The mean of `a`'s score over `b`'s (see [`TowersResult::score_for`]), and its standard error.
    pub margin: f32,
    pub error: f32,
    /// Drone-seconds out of contact in a game, for `a` and for `b`.
    pub dark: [f32; 2],
}

/// Plays `a` against `b` on each seed, as Blue and as Red, each with the default commander.
pub fn compare(a: &dyn Autonomy, b: &dyn Autonomy, seeds: &[u64]) -> Result<Comparison, String> {
    let rules = Rules::default();
    let seat = |autonomy| Prepared { captain: Commander::new(rules), autonomy };
    let mut c = Comparison::default();
    let mut scores = vec![];
    for &seed in seeds {
        let first: TowersResult = play(&mut seat(a), &mut seat(b), seed)?;
        let second: TowersResult = play(&mut seat(b), &mut seat(a), seed)?;
        for (r, mine) in [(&first, Team::Blue), (&second, Team::Red)] {
            let s = r.score_for(mine, rules.win_score);
            scores.push(s);
            c.games += 1;
            match r.outcome {
                recreation_towers::Outcome::Won(t) if t == mine => c.won += 1,
                recreation_towers::Outcome::Won(_) => c.lost += 1,
                recreation_towers::Outcome::Draw => c.drawn += 1,
            }
            c.dark[0] += r.dark[mine.index()];
            c.dark[1] += r.dark[mine.other().index()];
        }
    }
    let n = scores.len().max(1) as f32;
    c.margin = scores.iter().sum::<f32>() / n;
    let var = scores.iter().map(|s| (s - c.margin).powi(2)).sum::<f32>() / (n - 1.0).max(1.0);
    c.error = (var / n).sqrt();
    c.dark = c.dark.map(|d| d / n);
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds(n: u64) -> Vec<u64> {
        (1..=n).collect()
    }

    #[test]
    fn a_policy_against_itself_is_level() {
        let c = compare(&Persist, &Persist, &seeds(4)).unwrap();
        assert_eq!(c.games, 8);
        assert!(c.margin.abs() < 1e-6, "{c:?}");
        assert_eq!(c.won, c.lost);
    }

    #[test]
    fn a_comparison_is_the_other_way_round_when_the_policies_swap() {
        let ab = compare(&Persist, &Failsafe, &seeds(4)).unwrap();
        let ba = compare(&Failsafe, &Persist, &seeds(4)).unwrap();
        assert!((ab.margin + ba.margin).abs() < 1e-5, "{ab:?} {ba:?}");
        assert_eq!((ab.won, ab.lost), (ba.lost, ba.won));
        assert!((ab.dark[0] - ba.dark[1]).abs() < 1e-2 && (ab.dark[1] - ba.dark[0]).abs() < 1e-2, "{ab:?} {ba:?}");
    }

    #[test]
    fn the_drones_are_dark_for_some_of_a_game() {
        let c = compare(&Failsafe, &Failsafe, &seeds(4)).unwrap();
        assert!(c.dark[0] > 0.0, "no drone ever went dark: {c:?}");
    }

    /// The baseline a learned autonomy has to beat. Measured on 100 seeds (200 games): persist
    /// +0.74 ± 0.08 against the failsafe, and 76 % of the games won. An attacker is always in the
    /// shadow of the tower it takes (the shadow is wider than the circle), so a failsafe that
    /// sends it home after 20 s, as long as the capture takes, pulls it off the tower.
    #[test]
    fn flying_the_order_on_beats_the_failsafe_by_a_clear_margin() {
        let c = compare(&Persist, &Failsafe, &seeds(30)).unwrap();
        assert!(c.margin > 3.0 * c.error, "{c:?}");
        assert!(c.dark[0] < c.dark[1], "persisting ends the shadows sooner: {c:?}");
    }
}
