//! Advisors that propose moves to a commander (plans/actors.md): the [`Advise`] trait, the
//! [`Proposal`] it makes, and the first advisor, the [`Lookahead`]. Nothing here does I/O: the
//! arena, the game server and a seat in another process all drive the same trait.
//!
//! An advisor sees what a seat gets, the team's map ([`TeamView`]), and what the commander has in
//! hand ([`Hand`]: the orders it means to give, the drones it may still change, the towers a
//! drone must stay away from). It may keep what state it likes, a belief about the enemy for one.
//! It answers with proposals; it does not give orders. The commander decides whether to take one
//! (`Commander::advise`), and an advisor that is slow is only late: its proposals reach the
//! commander some seconds after the map they were made from.

use crate::lookahead::{self, HORIZON};
use crate::{Belief, Order, Queued, Rules, Team, TeamView};

/// A proposal: one order for one drone, with what the advisor expects to gain by it.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// The drone, by its number in the team.
    pub drone: usize,
    pub order: Order,
    /// The queue that goes with the order (what the drone does out of contact).
    pub queue: Vec<Queued>,
    /// The points the advisor expects the order to gain over what the commander means to do.
    pub gain: f32,
    /// The game time (s) of the map the proposal was made from.
    pub made: f32,
    /// The game time (s) after which the proposal is stale and is not taken.
    pub until: f32,
    /// What the advisor says to a person who is shown the proposal.
    pub why: String,
}

/// What a commander has in hand when an advisor looks: the part of its own state that an advisor
/// needs and that the map does not show. A seat has it, for the seat is the commander.
#[derive(Clone, Debug, PartialEq)]
pub struct Hand {
    /// The order each drone is to fly, by its number in the team.
    pub orders: Vec<Option<Order>>,
    /// The drones that an order may be given to (in contact, up, not on their way to charge).
    pub free: Vec<bool>,
    /// For each drone, the towers it must stay away from (it gave them up).
    pub away: Vec<Vec<bool>>,
}

/// Looks at the team's map and proposes moves. It may be slow; it is called when the map is shown
/// to it, and what it answers reaches the commander later.
pub trait Advise: std::fmt::Debug {
    /// The proposals on this map, if any. Called with each map the advisor is shown (the
    /// commander shows it one each second); it decides for itself how often it thinks.
    fn advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal>;

    /// A copy of the advisor, with its state (a commander can be cloned).
    fn clone_box(&self) -> Box<dyn Advise>;
}

impl Clone for Box<dyn Advise> {
    fn clone(&self) -> Box<dyn Advise> {
        self.clone_box()
    }
}

/// A lookahead thinks ahead this often (s)...
pub const LOOK_EVERY: f32 = 5.0;
/// ...and not before this time in the game (s): at the start the belief is only a prior.
pub const LOOK_AFTER: f32 = 10.0;

/// The first advisor: the lookahead. Every [`LOOK_EVERY`] s it plays the game forward
/// [`HORIZON`] s in a small model (`lookahead`), with the enemy drones guessed from its own
/// [`Belief`], and proposes the one change of orders that gains most. Its proposal is good for
/// the horizon it looked over.
#[derive(Clone, Debug)]
pub struct Lookahead {
    rules: Rules,
    belief: Belief,
    /// The game time of the last look.
    last: f32,
    /// For measuring: the belief is never updated, so the model knows nothing of the enemy.
    blind: bool,
}

impl Lookahead {
    pub fn new(rules: Rules) -> Lookahead {
        Lookahead { rules, belief: Belief::new(rules, Team::Blue), last: f32::NEG_INFINITY, blind: false }
    }

    /// A lookahead whose belief is never updated (for measuring what the belief is worth).
    pub fn blind(mut self, blind: bool) -> Lookahead {
        self.blind = blind;
        self
    }
}

impl Advise for Lookahead {
    fn advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal> {
        if self.belief.team() != view.team {
            self.belief = Belief::new(self.rules, view.team);
        }
        if !self.blind {
            self.belief.update(view);
        }
        if view.time < LOOK_AFTER || view.time - self.last < LOOK_EVERY {
            return vec![];
        }
        self.last = view.time;
        let Some((drone, tower, gain)) =
            lookahead::best_change(&self.rules, view, &hand.orders, &hand.free, &hand.away, &self.belief)
        else {
            return vec![];
        };
        vec![Proposal {
            drone,
            order: Order::Take(tower),
            queue: vec![],
            gain,
            made: view.time,
            until: view.time + HORIZON,
            why: format!(
                "Send drone {} to the {} tower: about {gain:.0} points over the next {HORIZON:.0} s.",
                drone + 1,
                view.towers[tower].name
            ),
        }]
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}
