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

use crate::lookahead::{self, Model};
use crate::{Belief, Order, Queued, Rules, Team, TeamView};
use serde::{Deserialize, Serialize};

/// A proposal: one order for one drone, with what the advisor expects to gain by it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
pub trait Advise: std::fmt::Debug + Send {
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
/// 45 s in a small model (`lookahead`), with the enemy drones guessed from its own
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
    /// The rules of the model it plays forward.
    model: Model,
    /// A drone it proposed a move for is left alone this long (s), and when it proposed it last.
    cooldown: f32,
    moved: Vec<f32>,
}

impl Lookahead {
    pub fn new(rules: Rules) -> Lookahead {
        Lookahead {
            rules,
            belief: Belief::new(rules, Team::Blue),
            last: f32::NEG_INFINITY,
            blind: false,
            model: Model::default(),
            cooldown: 0.0,
            moved: vec![],
        }
    }

    /// A lookahead whose model has these rules (for measuring what each is worth).
    pub fn with_model(mut self, model: Model) -> Lookahead {
        self.model = model;
        self
    }

    /// A lookahead whose belief is never updated (for measuring what the belief is worth).
    /// A lookahead that does not propose a move for a drone this long (s) after it proposed one.
    pub fn with_cooldown(mut self, secs: f32) -> Lookahead {
        self.cooldown = secs;
        self
    }

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
        self.moved.resize(hand.orders.len(), f32::NEG_INFINITY);
        let hold: Vec<bool> = self.moved.iter().map(|&m| view.time - m < self.cooldown).collect();
        let Some((drone, tower, gain)) = lookahead::best_change_with(
            &self.rules,
            view,
            &hand.orders,
            &hand.free,
            &hold,
            &hand.away,
            &self.belief,
            self.model,
        ) else {
            return vec![];
        };
        self.moved[drone] = view.time;
        vec![Proposal {
            drone,
            order: Order::Take(tower),
            queue: vec![],
            gain,
            made: view.time,
            until: view.time + self.model.horizon,
            why: format!(
                "Send drone {} to the {} tower: about {gain:.0} points over the next {:.0} s.",
                drone + 1,
                view.towers[tower].name,
                self.model.horizon
            ),
        }]
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}
