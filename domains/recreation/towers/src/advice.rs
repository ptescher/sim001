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

/// One entry on a team's board (`plans/advisors.md`): what an advisor posted, for the commander
/// and for the other advisors to read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Its number on the board, from 1 in the order posted (0 before it is posted).
    #[serde(default)]
    pub id: u64,
    /// The advisor that posted it.
    pub author: String,
    /// The game time (s) of the map it was made from.
    pub made: f32,
    /// The game time (s) after which it no longer holds.
    pub until: f32,
    /// How sure its author is, 0 to 1.
    pub confidence: f32,
    pub kind: EntryKind,
}

/// What an entry says. Proposals are orders for a drone; the other kinds come with the next steps
/// of the plan (intents, assessments, critiques).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EntryKind {
    Proposal(Proposal),
    Intent(Intent),
}

/// What an intent says of one tower.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Aim {
    /// Take it, with this weight (0 to 1: how much more it matters than the other towers to take).
    Take(f32),
    /// Keep it, with this weight.
    Hold(f32),
    /// Leave it: do not send drones there.
    Leave,
    /// No view.
    Open,
}

/// A strategy for the next minute or two (`plans/advisors.md`): an aim for each tower, by its
/// number, and the words for a person. A slow thinker posts it; fast thinkers follow it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub aims: Vec<Aim>,
    pub why: String,
}

impl Intent {
    /// The bias that following this intent puts on sending a drone to each tower, in points:
    /// `follow` times the weight for a tower to take or hold, minus `leave` for one to leave.
    pub fn bias(&self, follow: f32, leave: f32) -> Vec<f32> {
        self.aims
            .iter()
            .map(|a| match *a {
                Aim::Take(w) | Aim::Hold(w) => follow * w.clamp(0.0, 1.0),
                Aim::Leave => -leave,
                Aim::Open => 0.0,
            })
            .collect()
    }
}

impl Entry {
    /// A proposal of `author`'s, as an entry: it holds as long as the proposal does.
    pub fn proposal(author: impl Into<String>, p: Proposal) -> Entry {
        Entry {
            id: 0,
            author: author.into(),
            made: p.made,
            until: p.until,
            confidence: 1.0,
            kind: EntryKind::Proposal(p),
        }
    }

    /// The proposal, if the entry is one.
    pub fn as_proposal(&self) -> Option<&Proposal> {
        match &self.kind {
            EntryKind::Proposal(p) => Some(p),
            _ => None,
        }
    }

    /// The intent, if the entry is one.
    pub fn as_intent(&self) -> Option<&Intent> {
        match &self.kind {
            EntryKind::Intent(i) => Some(i),
            _ => None,
        }
    }
}

/// A team's board: every entry posted in a game, in the order posted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    entries: Vec<Entry>,
    /// The proposals the commander took: when (game time, s), and the entry's id.
    taken: Vec<(f32, u64)>,
}

impl Board {
    /// Posts `entry` (its `id` is set here) and returns its id.
    pub fn post(&mut self, mut entry: Entry) -> u64 {
        entry.id = self.entries.len() as u64 + 1;
        self.entries.push(entry);
        self.entries.len() as u64
    }

    /// The id of the last entry posted (0 for none).
    pub fn last(&self) -> u64 {
        self.entries.len() as u64
    }

    /// The entries posted after entry `id`.
    pub fn since(&self, id: u64) -> &[Entry] {
        &self.entries[(id as usize).min(self.entries.len())..]
    }

    /// The entries that still hold at game time `now`.
    pub fn in_force(&self, now: f32) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.until >= now)
    }

    /// Every entry, in the order posted.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The commander took proposal `p` at game time `at`: notes it against the last entry that
    /// posted it (none if it was never posted).
    pub fn took(&mut self, at: f32, p: &Proposal) {
        if let Some(e) = self.entries.iter().rev().find(|e| e.as_proposal() == Some(p)) {
            self.taken.push((at, e.id));
        }
    }

    /// The proposals the commander took, in order: when, and the entry's id.
    pub fn taken(&self) -> &[(f32, u64)] {
        &self.taken
    }
}

/// Looks at the team's map and proposes moves. It may be slow; it is called when the map is shown
/// to it, and what it answers reaches the commander later.
pub trait Advise: std::fmt::Debug + Send {
    /// The proposals on this map, if any. Called with each map the advisor is shown (the
    /// commander shows it one each second); it decides for itself how often it thinks. An advisor
    /// that reads the board answers [`Advise::look`] instead.
    fn advise(&mut self, _view: &TeamView, _hand: &Hand) -> Vec<Proposal> {
        vec![]
    }

    /// What it posts on this map, shown the entries that went on the board since its last look
    /// (its own and the other advisors'). By default its proposals, from [`Advise::advise`].
    fn look(&mut self, view: &TeamView, hand: &Hand, _board: &[Entry]) -> Vec<Entry> {
        let name = self.name();
        self.advise(view, hand).into_iter().map(|p| Entry::proposal(name.clone(), p)).collect()
    }

    /// The name its entries are posted under.
    fn name(&self) -> String {
        "advisor".into()
    }

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
    /// The intent it follows (the last one posted that still holds), with its author.
    intent: Option<(Entry, String)>,
    /// Following an intent: points added per unit of a tower's weight, and taken off a tower to
    /// leave, when it chooses which change to propose (not to the gain it reports).
    follow: f32,
    leave: f32,
}

/// Points a lookahead adds, per unit of weight, to a change that takes a tower an intent says to
/// take or hold.
pub const FOLLOW: f32 = 10.0;
/// Points it takes off a change that sends a drone to a tower an intent says to leave.
pub const LEAVE: f32 = 20.0;

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
            intent: None,
            follow: FOLLOW,
            leave: LEAVE,
        }
    }

    /// A lookahead that follows intents this much (points per unit of weight, and off a tower to
    /// leave); 0 and 0 ignore them.
    pub fn following(mut self, follow: f32, leave: f32) -> Lookahead {
        self.follow = follow;
        self.leave = leave;
        self
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
        if self.last == f32::NEG_INFINITY {
            // Its draws differ from game to game, or one seat would have the luckier ones in all.
            self.belief.reseed(game_key(view));
        }
        self.last = view.time;
        self.moved.resize(hand.orders.len(), f32::NEG_INFINITY);
        let hold: Vec<bool> = self.moved.iter().map(|&m| view.time - m < self.cooldown).collect();
        let intent = self.intent.as_ref().filter(|(e, _)| e.until >= view.time);
        let bias = intent.and_then(|(e, _)| e.as_intent()).map_or(vec![], |i| i.bias(self.follow, self.leave));
        let Some((drone, tower, gain)) = lookahead::best_change_biased(
            &self.rules,
            view,
            &hand.orders,
            &hand.free,
            &hold,
            &hand.away,
            &self.belief,
            self.model,
            &bias,
        ) else {
            return vec![];
        };
        let following = intent.filter(|_| bias.get(tower).is_some_and(|&b| b > 0.0)).map(|(_, a)| a.clone());
        self.moved[drone] = view.time;
        vec![Proposal {
            drone,
            order: Order::Take(tower),
            queue: vec![],
            gain,
            made: view.time,
            until: view.time + self.model.horizon,
            why: format!(
                "Send drone {} to the {} tower: about {gain:.0} points over the next {:.0} s{}.",
                drone + 1,
                view.towers[tower].name,
                self.model.horizon,
                following.map_or(String::new(), |a| format!(", as {a}'s intent says"))
            ),
        }]
    }

    fn look(&mut self, view: &TeamView, hand: &Hand, board: &[Entry]) -> Vec<Entry> {
        // The intent to follow: the last one posted.
        if let Some(e) = board.iter().rev().find(|e| e.as_intent().is_some()) {
            self.intent = Some((e.clone(), e.author.clone()));
        }
        let name = self.name();
        self.advise(view, hand).into_iter().map(|p| Entry::proposal(name.clone(), p)).collect()
    }

    fn name(&self) -> String {
        "lookahead".into()
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}

/// The slow thinker's stand-in (`plans/advisors.md`, step 2): every [`DEEP_EVERY`] s it plays the
/// game [`DEEP_HORIZON`] s forward, for every drone it may send to every tower, and posts an intent
/// for the next [`DEEP_HOLDS`] s, against what the commander means to do: take the towers that
/// sending one more drone to would gain at (weighted by that gain), leave the ones that taking a
/// drone off would gain at. It gives no orders.
#[derive(Clone, Debug)]
pub struct Deep {
    rules: Rules,
    belief: Belief,
    last: f32,
    model: Model,
}

/// A deep thinker looks this often (s)...
pub const DEEP_EVERY: f32 = 15.0;
/// ...this far ahead (s)...
pub const DEEP_HORIZON: f32 = 120.0;
/// ...and its intent holds this long (s).
pub const DEEP_HOLDS: f32 = 60.0;
/// A tower whose best move gains more than this (points) is one to take...
const TAKE_AT: f32 = 1.0;
/// ...and one that taking a drone off gains more than this at is one to leave.
const LEAVE_AT: f32 = 5.0;

impl Deep {
    pub fn new(rules: Rules) -> Deep {
        Deep {
            rules,
            belief: Belief::new(rules, Team::Blue),
            last: f32::NEG_INFINITY,
            model: Model { horizon: DEEP_HORIZON, ..Model::default() },
        }
    }

    /// The intent it would post on this map.
    pub fn intent(&self, view: &TeamView, hand: &Hand) -> Intent {
        let none = vec![false; hand.orders.len()];
        let gains = lookahead::change_gains(
            &self.rules,
            view,
            &hand.orders,
            &hand.free,
            &none,
            &hand.away,
            &self.belief,
            self.model,
        );
        // For each tower: the most that sending a drone to it gains, and the most that taking a
        // drone off it (to any other tower) gains.
        let n = view.towers.len();
        let (mut to, mut off) = (vec![f32::NEG_INFINITY; n], vec![f32::NEG_INFINITY; n]);
        for ((k, t), g) in gains.into_iter().filter(|(_, g)| g.is_finite()) {
            to[t] = to[t].max(g);
            if let Some(Order::Take(from)) = hand.orders[k] {
                off[from] = off[from].max(g);
            }
        }
        let top = to.iter().copied().fold(0.0f32, f32::max);
        let aims: Vec<Aim> = (0..n)
            .map(|t| {
                if view.towers[t].holder == Some(view.team) {
                    Aim::Open
                } else if to[t] > TAKE_AT {
                    Aim::Take((to[t] / top.max(TAKE_AT)).clamp(0.0, 1.0))
                } else if off[t] > LEAVE_AT {
                    Aim::Leave
                } else {
                    Aim::Open
                }
            })
            .collect();
        let names = |pick: fn(&Aim) -> bool| -> Vec<String> {
            aims.iter().zip(&view.towers).filter(|(a, _)| pick(a)).map(|(_, t)| t.name.clone()).collect()
        };
        let take = names(|a| matches!(a, Aim::Take(_)));
        let leave = names(|a| matches!(a, Aim::Leave));
        let why = match (take.is_empty(), leave.is_empty()) {
            (true, true) => "Nothing to take that would gain: hold what we have.".to_string(),
            (false, true) => format!("Take the {}.", take.join(", ")),
            (true, false) => format!("Leave the {}.", leave.join(", ")),
            (false, false) => format!("Take the {}; leave the {}.", take.join(", "), leave.join(", ")),
        };
        Intent { aims, why }
    }
}

impl Advise for Deep {
    fn look(&mut self, view: &TeamView, hand: &Hand, _board: &[Entry]) -> Vec<Entry> {
        if self.belief.team() != view.team {
            self.belief = Belief::new(self.rules, view.team);
        }
        self.belief.update(view);
        if view.time < LOOK_AFTER || view.time - self.last < DEEP_EVERY {
            return vec![];
        }
        self.last = view.time;
        let intent = self.intent(view, hand);
        vec![Entry {
            id: 0,
            author: self.name(),
            made: view.time,
            until: view.time + DEEP_HOLDS,
            confidence: 1.0,
            kind: EntryKind::Intent(intent),
        }]
    }

    fn name(&self) -> String {
        "deep".into()
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}

/// A number that differs from game to game: the team's own drones where they are now (at the
/// first look, after the take-off, they are where each game's draws put them).
pub(crate) fn game_key(view: &TeamView) -> u64 {
    view.drones.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, d| {
        let h = (h ^ d.at.x.to_bits() as u64).wrapping_mul(0x0100_0000_01B3);
        (h ^ d.at.y.to_bits() as u64).wrapping_mul(0x0100_0000_01B3)
    })
}
