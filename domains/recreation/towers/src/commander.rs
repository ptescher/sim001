//! The computer commander: a team's orders, once a second, from its team's map alone.
//!
//! Its numbers are [`CommanderTuning`]: each has its default (the constant of the same name),
//! its bounds, and may be set per commander ([`Commander::with_tuning`]). This file is the one a
//! strategy that improves commanders may change; the rules, the referee, the fog and the view
//! are elsewhere.

use crate::{Advise, Hand, Kind, Lookahead, Order, Phase, Proposal, Queued, Rules, TeamView};
use serde::{Deserialize, Serialize};

/// Declares the tunable numbers: for each, a constant (its default) and a field of
/// [`CommanderTuning`], with its bounds (the least and the most a tuning may set).
macro_rules! tunable {
    ($($(#[$doc:meta])* $field:ident / $name:ident = $default:expr, $lo:expr, $hi:expr;)*) => {
        $($(#[$doc])* pub const $name: f32 = $default;)*

        /// The numbers of one commander (see the constants of the same names, in capitals, for
        /// what each does).
        #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
        pub struct CommanderTuning {
            $(pub $field: f32,)*
        }

        impl CommanderTuning {
            /// The constants: how the commander plays when nothing tunes it.
            pub const DEFAULT: CommanderTuning = CommanderTuning { $($field: $name,)* };
            /// How many numbers there are.
            pub const LEN: usize = [$(stringify!($field)),*].len();
            /// Their names, in the order of [`CommanderTuning::to_array`].
            pub const NAMES: [&'static str; CommanderTuning::LEN] = [$(stringify!($field)),*];
            /// Their bounds (least, most), in the same order.
            pub const BOUNDS: [(f32, f32); CommanderTuning::LEN] = [$(($lo, $hi)),*];

            pub fn to_array(&self) -> [f32; CommanderTuning::LEN] {
                [$(self.$field),*]
            }

            /// From numbers in the order of [`CommanderTuning::NAMES`]. `None` unless each is
            /// finite and within its bounds.
            pub fn from_array(a: &[f32; CommanderTuning::LEN]) -> Option<CommanderTuning> {
                let ok = a
                    .iter()
                    .zip(CommanderTuning::BOUNDS.iter())
                    .all(|(v, (lo, hi))| v.is_finite() && *v >= *lo && *v <= *hi);
                let mut it = a.iter().copied();
                let t = CommanderTuning { $($field: it.next().unwrap_or($name),)* };
                ok.then_some(t)
            }
        }

        impl Default for CommanderTuning {
            fn default() -> CommanderTuning {
                CommanderTuning::DEFAULT
            }
        }
    };
}

tunable! {
    /// A drone behind at a contested tower for this long gives it up (s)...
    give_up / GIVE_UP = 10.0, 1.0, 120.0;
    /// ...and is not sent back to it for this long (s).
    stay_away / STAY_AWAY = 30.0, 0.0, 300.0;
    /// A held tower is in danger when the team sees an enemy drone this near it (m): two circles,
    /// so that an enemy circling the next tower (220 m away) is not.
    defend_range / DEFEND_RANGE = 120.0, 0.0, 400.0;
    /// A drone counts as behind at a contested tower when the control is more than this on the
    /// other team's side (0..1 of the bar).
    behind_by / BEHIND_BY = 0.0, 0.0, 0.9;
}

/// A drone flies about this fast (m/s), for the time it takes to get home.
const CRUISE: f32 = 16.0;
/// A drone goes to charge when its battery is down to the bingo level, the time to fly home and
/// this margin (s); it goes back to work when the battery is this much full.
const REFUEL_MARGIN: f32 = 15.0;
const BACK_TO_WORK: f32 = 0.9;
/// A commander takes a proposal of an advisor only if it gains more than this many points.
pub const ADVICE_MARGIN: f32 = 2.0;

/// A computer in a team's seat.
#[derive(Clone, Debug)]
pub struct Commander {
    rules: Rules,
    tuning: CommanderTuning,
    /// Each drone's order, by its number in the team.
    orders: Vec<Option<Order>>,
    /// How long each drone has been behind at a contested tower (s).
    behind: Vec<f32>,
    /// The tower each drone gave up, and until when (game time, s) it stays away.
    away: Vec<Option<(usize, f32)>>,
    /// Each drone is on its way to charge, or charging.
    recharging: Vec<bool>,
    /// The game time of the last decision.
    last: Option<f32>,
    /// The advisors that look at the map each time it decides, each with the delay (s of game
    /// time) between the map it is shown and the moment its proposals reach the commander. A
    /// commander with none does not think ahead.
    advisors: Vec<(Box<dyn Advise>, f32)>,
    /// Proposals on their way: when each arrives (game time, s).
    late: Vec<(f32, Proposal)>,
    /// Proposals in hand, not yet taken and not yet stale.
    proposals: Vec<Proposal>,
    /// A proposal must gain more than this many points to be taken.
    advice_margin: f32,
    /// The queue that goes with the order a proposal gave a drone, while it still flies it.
    queues: Vec<Option<(Order, Vec<Queued>)>>,
}

impl Commander {
    pub fn new(rules: Rules) -> Commander {
        Commander::with_tuning(rules, CommanderTuning::DEFAULT)
    }

    pub fn with_tuning(rules: Rules, tuning: CommanderTuning) -> Commander {
        let n = rules.drones;
        Commander {
            rules,
            tuning,
            orders: vec![None; n],
            behind: vec![0.0; n],
            away: vec![None; n],
            recharging: vec![false; n],
            last: None,
            advisors: vec![],
            late: vec![],
            proposals: vec![],
            advice_margin: ADVICE_MARGIN,
            queues: vec![None; n],
        }
        .with_lookahead(std::env::var("SIM001_LOOKAHEAD").as_deref() != Ok("0"))
    }

    /// Thinks ahead or not: with the [`Lookahead`] advisor, at no delay, or with no advisor at all
    /// (this takes every advisor away). A commander has the lookahead unless `SIM001_LOOKAHEAD` is
    /// `0`; with it set to `blind`, the lookahead never updates its belief.
    pub fn with_lookahead(mut self, on: bool) -> Commander {
        self.advisors.clear();
        self.late.clear();
        if on {
            let blind = std::env::var("SIM001_LOOKAHEAD").as_deref() == Ok("blind");
            self.advisors.push((Box::new(Lookahead::new(self.rules).blind(blind)), 0.0));
        }
        self
    }

    /// Adds an advisor. Its proposals reach the commander `delay` s of game time after the map it
    /// was shown (the map of each decision), so a slow advisor is a late one.
    pub fn with_advisor(mut self, advisor: impl Advise + 'static, delay: f32) -> Commander {
        self.advisors.push((Box::new(advisor), delay.max(0.0)));
        self
    }

    /// Adds an advisor that is already boxed (see [`Commander::with_advisor`]).
    pub fn add_advisor(&mut self, advisor: Box<dyn Advise>, delay: f32) {
        self.advisors.push((advisor, delay.max(0.0)));
    }

    /// How many advisors the commander has (none: it does not think ahead).
    pub fn advisors(&self) -> usize {
        self.advisors.len()
    }

    /// A proposal must gain more than `points` to be taken (default [`ADVICE_MARGIN`]).
    pub fn with_advice_margin(mut self, points: f32) -> Commander {
        self.advice_margin = points;
        self
    }

    /// Puts proposals in hand, as if they had just arrived: from an advisor outside this process, or
    /// one that a person accepted. The commander takes them at its next decision, or lets them go
    /// if they are stale or do not fit then.
    pub fn advise(&mut self, proposals: impl IntoIterator<Item = Proposal>) {
        self.proposals.extend(proposals);
    }

    /// The queue that goes with drone `k`'s order: the one a proposal gave it, while it flies that
    /// order; else none.
    pub fn advised_queue(&self, k: usize) -> Vec<Queued> {
        match (self.queues.get(k), self.order(k)) {
            (Some(Some((o, q))), Some(now)) if *o == now => q.clone(),
            _ => vec![],
        }
    }

    pub fn tuning(&self) -> CommanderTuning {
        self.tuning
    }

    /// Gives drone `k` an order (or none), as if the commander had given it.
    pub fn assign(&mut self, k: usize, order: Option<Order>) {
        self.orders[k] = order;
    }

    /// The order drone `k` has.
    pub fn order(&self, k: usize) -> Option<Order> {
        self.orders.get(k).copied().flatten()
    }

    /// Looks at the team's map and gives every drone its order: `(drone, order)` for each.
    ///
    /// First, a tower the team holds with an enemy drone in sight near it (within [`DEFEND_RANGE`])
    /// gets the drone nearest to it, if no drone is on it already. Then the heavy drone, which
    /// counts double, goes to the spire if the team does not hold it and nobody is on it. Then the
    /// drones with nothing to do and the towers the team does not hold that nobody is on are
    /// matched greedily, the pair that wins its tower soonest first (the flight, and the capture at
    /// the drone's weight, per point the tower scores); a drone keeps its tower until the team
    /// holds it. Drones left over join the nearest tower the team does not hold. A drone that
    /// has been behind at a contested tower for [`GIVE_UP`] gives it up, and is not sent back to
    /// it for [`STAY_AWAY`].
    pub fn decide(&mut self, view: &TeamView) -> Vec<(usize, Order)> {
        let dt = self.last.map_or(0.0, |t| (view.time - t).max(0.0));
        self.last = Some(view.time);
        for k in 0..self.orders.len() {
            if let Some(Order::Take(t)) = self.orders[k] {
                let tower = &view.towers[t];
                let fighting = tower.seen
                    && tower.contested
                    && tower.control * view.team.sign() < -self.tuning.behind_by
                    && view.drones[k].at.distance(tower.at) <= self.rules.circle_radius;
                self.behind[k] = if fighting { self.behind[k] + dt } else { 0.0 };
                if self.behind[k] >= self.tuning.give_up {
                    self.behind[k] = 0.0;
                    self.away[k] = Some((t, view.time + self.tuning.stay_away));
                    self.orders[k] = None;
                }
            }
        }
        let away = |k: usize, t: usize| self.away[k].is_some_and(|(a, until)| a == t && view.time < until);
        let away: Vec<Vec<bool>> =
            (0..self.orders.len()).map(|k| (0..view.towers.len()).map(|t| away(k, t)).collect()).collect();
        let (near, base) = (self.tuning.defend_range, self.rules.base(view.team));
        let ours = |t: usize| view.towers[t].holder == Some(view.team);
        let threatened =
            |t: usize| ours(t) && view.enemies.iter().any(|e| e.seen && e.at.distance(view.towers[t].at) <= near);
        // A drone out of contact gets no order: it flies the one it has, and stays out of the
        // sums below.
        let linked: Vec<bool> = view.drones.iter().map(|d| d.linked).collect();
        // A drone whose battery is low goes to charge: home, or to a depot the team holds that is
        // nearer. It is left out of the sums below until it is nearly full.
        let mut charge: Vec<Option<Order>> = vec![None; self.orders.len()];
        for d in view.drones.iter().filter(|d| d.linked && d.phase != Phase::Down) {
            let full = self.rules.battery(self.rules.class(d.k));
            if full <= 0.0 {
                self.recharging[d.k] = false;
                continue;
            }
            let home = d.at.distance(base);
            // In a tower's circle with enemies in it the battery drains faster (by their weight):
            // the margin is kept in seconds at that rate.
            let r = self.rules.circle_radius;
            let enemy: f32 = view.towers.iter().find(|t| d.at.distance(t.at) <= r).map_or(0.0, |t| {
                view.enemies
                    .iter()
                    .filter(|e| e.seen && e.at.distance(t.at) <= r)
                    .map(|e| self.rules.class(e.k).weight())
                    .sum()
            });
            let rate = 1.0 + self.rules.contest_drain * enemy;
            let need = self.rules.bingo + home / CRUISE + REFUEL_MARGIN * rate;
            if self.recharging[d.k] {
                self.recharging[d.k] = d.battery < BACK_TO_WORK * full;
            } else {
                self.recharging[d.k] = d.phase == Phase::Flying && d.battery <= need;
            }
            if self.recharging[d.k] {
                let depot = (0..view.towers.len())
                    .filter(|&t| view.towers[t].kind == Kind::Depot && ours(t))
                    .min_by(|&a, &b| d.at.distance(view.towers[a].at).total_cmp(&d.at.distance(view.towers[b].at)))
                    .filter(|&t| d.at.distance(view.towers[t].at) < home);
                charge[d.k] = Some(depot.map_or(Order::Home, Order::Take));
            }
        }
        let up: Vec<bool> =
            view.drones.iter().map(|d| d.phase != Phase::Down && d.linked && !self.recharging[d.k]).collect();
        // Keep the orders that still make sense.
        for (k, o) in self.orders.iter_mut().enumerate() {
            let keep = match *o {
                _ if !linked[k] => true,
                Some(Order::Take(t)) => up[k] && (!ours(t) || threatened(t)),
                _ => false,
            };
            if !keep {
                *o = None;
            }
        }
        let on = |orders: &[Option<Order>], t: usize| orders.contains(&Some(Order::Take(t)));
        let nearest = |orders: &[Option<Order>], t: usize, any: bool| {
            view.drones
                .iter()
                .filter(|d| up[d.k] && !away[d.k][t] && (any || orders[d.k].is_none()))
                .min_by(|a, b| a.at.distance(view.towers[t].at).total_cmp(&b.at.distance(view.towers[t].at)))
                .map(|d| d.k)
        };
        for t in (0..view.towers.len()).filter(|&t| threatened(t)) {
            if !on(&self.orders, t)
                && let Some(k) = nearest(&self.orders, t, true)
            {
                self.orders[k] = Some(Order::Take(t));
            }
        }
        // The heavy, which counts double, goes to the spire, the tower most worth fighting for.
        for t in (0..view.towers.len()).filter(|&t| view.towers[t].kind == Kind::Spire && !ours(t)) {
            if let Some(d) = view.drones.iter().find(|d| {
                self.rules.class(d.k) == crate::Class::Heavy && up[d.k] && !away[d.k][t] && self.orders[d.k].is_none()
            }) && !on(&self.orders, t)
            {
                self.orders[d.k] = Some(Order::Take(t));
            }
        }
        let targets: Vec<usize> = (0..view.towers.len()).filter(|&t| !ours(t)).collect();
        // Free drones and the towers nobody is sent to are matched greedily by the time the drone
        // takes to win the tower (the flight and the capture at its weight) per point the tower
        // scores: the quickest points first. A drone is not sent where its battery cannot cover
        // the flight, the capture and the way home.
        let time = |k: usize, t: usize| {
            let tower = &view.towers[t];
            let flight = view.drones[k].at.distance(tower.at) / CRUISE;
            let need = 1.0 - tower.control * view.team.sign();
            (flight, self.rules.capture_time * need / self.rules.class(k).weight())
        };
        let lasts = |k: usize, t: usize| {
            let (flight, capture) = time(k, t);
            self.rules.battery(self.rules.class(k)) <= 0.0
                || view.drones[k].battery
                    >= self.rules.bingo + flight + capture + view.towers[t].at.distance(base) / CRUISE + REFUEL_MARGIN
        };
        let cost = |k: usize, t: usize| {
            let (flight, capture) = time(k, t);
            (flight + capture) / self.rules.points(view.towers[t].kind)
        };
        loop {
            let mut best: Option<(usize, usize)> = None;
            for &t in targets.iter().filter(|&&t| !on(&self.orders, t)) {
                for k in 0..self.orders.len() {
                    if up[k]
                        && !away[k][t]
                        && self.orders[k].is_none()
                        && lasts(k, t)
                        && best.is_none_or(|(bk, bt)| cost(k, t) < cost(bk, bt))
                    {
                        best = Some((k, t));
                    }
                }
            }
            match best {
                Some((k, t)) => self.orders[k] = Some(Order::Take(t)),
                None => break,
            }
        }
        for (k, order) in self.orders.iter_mut().enumerate().filter(|(k, o)| up[*k] && o.is_none()) {
            let at = view.drones[k].at;
            let spare = targets
                .iter()
                .filter(|&&t| !away[k][t])
                .min_by(|&&a, &&b| view.towers[a].at.distance(at).total_cmp(&view.towers[b].at.distance(at)));
            *order = spare.map(|&t| Order::Take(t));
        }
        if !self.advisors.is_empty() || !self.late.is_empty() || !self.proposals.is_empty() {
            let free: Vec<bool> = (0..self.orders.len()).map(|k| up[k] && charge[k].is_none()).collect();
            self.take_advice(view, free, away);
        }
        for (k, order) in charge.into_iter().enumerate() {
            if order.is_some() {
                self.orders[k] = order;
            }
        }
        self.orders.iter().enumerate().filter(|(k, _)| linked[*k]).filter_map(|(k, o)| o.map(|o| (k, o))).collect()
    }

    /// Shows the advisors the map, collects the proposals that have reached the commander, and takes
    /// the best one that can be taken: not stale, gaining more than the margin, and still fitting
    /// (see [`Commander::fits`]). A proposal that is stale or does not fit is dropped; one that gains too
    /// little waits for its time to run out.
    fn take_advice(&mut self, view: &TeamView, free: Vec<bool>, away: Vec<Vec<bool>>) {
        let hand = Hand { orders: self.orders.clone(), free, away };
        for (advisor, delay) in &mut self.advisors {
            for p in advisor.advise(view, &hand) {
                self.late.push((view.time + *delay, p));
            }
        }
        let now = view.time;
        let (arrived, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.late).into_iter().partition(|(at, _)| *at <= now);
        self.late = waiting;
        self.proposals.extend(arrived.into_iter().map(|(_, p)| p));
        self.proposals.retain(|p| p.until >= now);
        let mut best: Option<usize> = None;
        let mut keep = vec![true; self.proposals.len()];
        for (i, p) in self.proposals.iter().enumerate() {
            if !self.fits(p, view, &hand) {
                keep[i] = false;
            } else if p.gain > self.advice_margin && best.is_none_or(|b| p.gain > self.proposals[b].gain) {
                best = Some(i);
            }
        }
        if let Some(b) = best {
            let p = self.proposals[b].clone();
            self.orders[p.drone] = Some(p.order);
            self.queues[p.drone] = Some((p.order, p.queue.clone()));
            for (i, q) in self.proposals.iter().enumerate() {
                keep[i] &= q.drone != p.drone;
            }
        }
        let mut keep = keep.into_iter();
        self.proposals.retain(|_| keep.next().unwrap_or(false));
    }

    /// Whether a proposal still makes sense: its drone is one of the team's and free, the order
    /// is not the one the drone has, a tower it sends the drone to is not one the drone gave up, and
    /// (for a proposal made on an earlier map) is not one the team holds now.
    fn fits(&self, p: &Proposal, view: &TeamView, hand: &Hand) -> bool {
        if p.drone >= self.orders.len() || !hand.free[p.drone] || hand.orders[p.drone] == Some(p.order) {
            return false;
        }
        match p.order {
            Order::Take(t) => {
                t < view.towers.len()
                    && !hand.away[p.drone][t]
                    && !(p.made < view.time && view.towers[t].holder == Some(view.team))
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Game, Team};
    use glam::Vec2;

    fn decide(c: &mut Commander, g: &Game) -> Vec<Option<Order>> {
        c.decide(&g.view(Team::Blue));
        (0..3).map(|k| c.order(k)).collect()
    }

    #[test]
    fn a_tuning_goes_to_numbers_and_back_and_refuses_what_is_out_of_bounds() {
        let d = CommanderTuning::DEFAULT;
        assert_eq!(CommanderTuning::from_array(&d.to_array()), Some(d));
        assert_eq!(CommanderTuning::NAMES.len(), CommanderTuning::BOUNDS.len());
        assert!(d.to_array().iter().zip(CommanderTuning::BOUNDS).all(|(v, (lo, hi))| *v >= lo && *v <= hi));
        let mut a = d.to_array();
        a[0] = f32::NAN;
        assert_eq!(CommanderTuning::from_array(&a), None);
        a[0] = CommanderTuning::BOUNDS[0].1 + 1.0;
        assert_eq!(CommanderTuning::from_array(&a), None);
        let line = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<CommanderTuning>(&line).unwrap(), d);
    }

    #[test]
    fn a_tuning_changes_how_it_plays() {
        let (mut g, mut c) = fight(-0.2);
        let mut patient =
            Commander::with_tuning(g.rules, CommanderTuning { give_up: 60.0, ..CommanderTuning::DEFAULT })
                .with_lookahead(false);
        patient.assign(0, c.order(0));
        let west = Some(Order::Take(g.tower("west")));
        let mut g2 = g.clone();
        assert_ne!(play(&mut g, &mut c, 15), west);
        assert_eq!(play(&mut g2, &mut patient, 15), west);
    }

    #[test]
    fn at_the_start_it_spreads_its_drones_over_the_towers_it_can_win_soonest() {
        let g = Game::new(Rules { drones: 3, ..Rules::default() });
        let mut c = Commander::new(g.rules);
        let (west, centre, east) = (g.tower("west"), g.tower("centre"), g.tower("east"));
        let orders = c.decide(&g.view(Team::Blue));
        // The quickest win first, each drone and tower once: the centre's drone is the third.
        assert_eq!(orders, vec![(0, Order::Take(west)), (1, Order::Take(east)), (2, Order::Take(centre))]);
    }

    #[test]
    fn at_the_start_the_heavy_goes_to_the_spire() {
        let g = Game::new(Rules::default());
        let mut c = Commander::new(g.rules);
        let centre = g.tower("centre");
        assert_eq!(g.rules.class(3), crate::Class::Heavy);
        c.decide(&g.view(Team::Blue));
        assert_eq!(c.order(3), Some(Order::Take(centre)));
    }

    /// Drone 0 flies over the west tower with `battery` s left; the others wait at the base.
    fn over_the_west_tower(battery: f32) -> (Game, usize) {
        let mut g = Game::new(Rules { drones: 3, ..Rules::default() });
        let west = g.tower("west");
        let i = g.drone(Team::Blue, 0);
        g.drones[i] = crate::Drone { at: g.towers[west].at, alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
        g.battery[i] = battery;
        g.reported[i] = battery;
        (g, west)
    }

    #[test]
    fn a_drone_over_a_tower_is_sent_to_the_tower_it_wins_soonest() {
        // Nearest-first would send it to the centre, which is nearer the base. The west tower is
        // won in a capture time; the centre needs a long flight first.
        let (g, west) = over_the_west_tower(240.0);
        let mut c = Commander::new(g.rules);
        assert_eq!(decide(&mut c, &g)[0], Some(Order::Take(west)));
    }

    #[test]
    fn a_drone_keeps_its_tower_until_the_team_holds_it_then_goes_on() {
        let mut g = Game::new(Rules { drones: 3, ..Rules::default() });
        let mut c = Commander::new(g.rules);
        let (west, centre) = (g.tower("west"), g.tower("centre"));
        decide(&mut c, &g);
        // The drone for the centre flies off towards it: still its order, though the west tower is
        // now nearer to it than the centre is to the others.
        let i = g.drone(Team::Blue, 2);
        g.drones[i] = crate::Drone { at: Vec2::new(-50.0, -150.0), alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
        g.step(0.1);
        assert_eq!(decide(&mut c, &g)[2], Some(Order::Take(centre)));
        // Blue takes the centre: that drone goes on to the nearest tower nobody is sent to.
        g.towers[centre].holder = Some(Team::Blue);
        g.towers[centre].control = 1.0;
        g.drones[i].at = Vec2::new(40.0, 0.0);
        g.step(0.1);
        let orders = decide(&mut c, &g);
        assert_eq!(orders[0], Some(Order::Take(west)));
        assert_eq!(orders[2], Some(Order::Take(g.tower("Red home"))));
    }

    #[test]
    fn a_crashed_drone_gets_no_order_and_the_others_keep_theirs() {
        let mut g = Game::new(Rules { drones: 3, ..Rules::default() });
        let mut c = Commander::new(g.rules);
        let before = decide(&mut c, &g);
        let i = g.drone(Team::Blue, 1);
        g.drones[i].phase = Phase::Down;
        let orders = decide(&mut c, &g);
        assert_eq!(orders, vec![before[0], None, before[2]]);
        // Back at its base, it gets the tower nobody is on.
        g.drones[i].phase = Phase::Ground;
        assert_eq!(decide(&mut c, &g)[1], Some(Order::Take(g.tower("east"))));
    }

    #[test]
    fn spare_drones_join_the_nearest_tower_the_team_does_not_hold() {
        let mut g = Game::new(Rules { drones: 3, ..Rules::default() });
        for name in ["west", "east"] {
            let t = g.tower(name);
            g.towers[t].holder = Some(Team::Blue);
            g.towers[t].control = 1.0;
        }
        g.step(0.1);
        // Two towers to take, the centre and Red's home, for three drones: the third joins the
        // nearer one.
        let orders = decide(&mut Commander::new(g.rules), &g);
        let (centre, red) = (Some(Order::Take(g.tower("centre"))), Some(Order::Take(g.tower("Red home"))));
        assert_eq!(orders, vec![centre, red, centre]);
    }

    /// Blue's first drone and Red's second fight over the west tower, with its control at
    /// `control`; Blue's commander has sent its first drone there.
    fn fight(control: f32) -> (Game, Commander) {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        g.towers[west].control = control;
        let at = g.towers[west].at;
        for (team, k) in [(Team::Blue, 0), (Team::Red, 1)] {
            let i = g.drone(team, k);
            g.drones[i] =
                crate::Drone { at: at + Vec2::new(0.0, 40.0), alt: 15.0, phase: Phase::Flying, ..g.drones[i] };
        }
        let mut c = Commander::new(g.rules).with_lookahead(false);
        c.assign(0, Some(Order::Take(west)));
        (g, c)
    }

    /// Decides once a second for `secs`, and returns drone 0's order.
    fn play(g: &mut Game, c: &mut Commander, secs: usize) -> Option<Order> {
        for _ in 0..secs {
            c.decide(&g.view(Team::Blue));
            g.step(1.0);
        }
        decide(c, g)[0]
    }

    #[test]
    fn a_drone_behind_in_a_fight_gives_up_after_10_seconds_and_stays_away_for_30() {
        let (mut g, mut c) = fight(-0.2);
        let west = Some(Order::Take(g.tower("west")));
        assert_eq!(play(&mut g, &mut c, 9), west, "not yet");
        let away = play(&mut g, &mut c, 2);
        assert_ne!(away, west);
        assert!(away.is_some());
        // Still behind there, but it is not sent back while it stays away.
        assert_ne!(play(&mut g, &mut c, 25), west);
    }

    #[test]
    fn a_drone_ahead_in_a_fight_stays() {
        let (mut g, mut c) = fight(0.2);
        assert_eq!(play(&mut g, &mut c, 30), Some(Order::Take(g.tower("west"))));
    }

    #[test]
    fn an_enemy_past_the_defence_range_is_no_threat() {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        g.towers[west].holder = Some(Team::Blue);
        g.towers[west].control = 1.0;
        // A Red drone 150 m from the west tower: in its sight (200 m), out of the defence range.
        let red = g.drone(Team::Red, 1);
        let at = g.towers[west].at + Vec2::new(0.0, 150.0);
        g.drones[red] = crate::Drone { at, alt: 50.0, phase: Phase::Flying, ..g.drones[red] };
        g.step(0.1);
        assert!(g.view(Team::Blue).enemies.iter().any(|e| e.seen));
        let orders = decide(&mut Commander::new(g.rules), &g);
        assert!(!orders.contains(&Some(Order::Take(west))), "{orders:?}");
    }

    #[test]
    fn the_nearest_drone_defends_a_held_tower_with_an_enemy_near() {
        let mut g = Game::new(Rules::default());
        let mut c = Commander::new(g.rules);
        decide(&mut c, &g);
        let home = g.tower("Blue home");
        // The drone for the west tower is nearest the home tower now.
        let i = g.drone(Team::Blue, 0);
        g.drones[i] = crate::Drone { at: Vec2::new(-400.0, -30.0), alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
        let scout = g.drone(Team::Red, 0);
        g.drones[scout] =
            crate::Drone { at: Vec2::new(-330.0, 0.0), alt: 50.0, phase: Phase::Flying, ..g.drones[scout] };
        g.step(0.1);
        let orders = decide(&mut c, &g);
        assert_eq!(orders[0], Some(Order::Take(home)));
        // The enemy goes; so does the need to defend.
        g.drones[scout].at = Vec2::new(300.0, 250.0);
        g.step(5.0);
        assert_ne!(decide(&mut c, &g)[0], Some(Order::Take(home)));
    }

    /// A game with the batteries on, Blue's first drone flying at `at` with `left` s of battery.
    fn tired_game(at: Vec2, left: f32) -> Game {
        let rules = Rules { light_battery: 150.0, standard_battery: 240.0, heavy_battery: 300.0, ..Rules::default() };
        let mut g = Game::new(rules);
        let i = g.drone(Team::Blue, 0);
        g.drones[i] = crate::Drone { at, alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
        g.battery[i] = left;
        g.look();
        g
    }

    #[test]
    fn a_drone_whose_battery_is_low_goes_home_and_is_sent_out_again_when_it_is_full() {
        // 300 m from its base: 19 s home, so 60 s of bingo and the margin come first.
        let mut g = tired_game(Vec2::new(-220.0, 0.0), 100.0);
        let mut c = Commander::new(g.rules);
        assert_ne!(decide(&mut c, &g)[0], Some(Order::Home), "100 s is enough");
        let i = g.drone(Team::Blue, 0);
        g.battery[i] = 80.0;
        g.look();
        assert_eq!(decide(&mut c, &g)[0], Some(Order::Home), "80 s is not");
        // Home and part charged: it stays on the charge.
        g.battery[i] = 150.0;
        g.look();
        assert_eq!(decide(&mut c, &g)[0], Some(Order::Home));
        g.battery[i] = 230.0;
        g.look();
        assert_ne!(decide(&mut c, &g)[0], Some(Order::Home), "nearly full: back to work");
    }

    #[test]
    fn a_drone_whose_battery_is_low_goes_to_a_depot_its_team_holds_if_that_is_nearer_than_its_base() {
        let mut g = tired_game(Vec2::new(40.0, -330.0), 40.0);
        let west = g.tower("west");
        g.towers[west].holder = Some(Team::Blue);
        g.towers[west].control = 1.0;
        g.look();
        let mut c = Commander::new(g.rules);
        assert_eq!(decide(&mut c, &g)[0], Some(Order::Take(west)));
        // The team does not hold it any more: home.
        g.towers[west].holder = None;
        g.towers[west].control = 0.0;
        g.look();
        let mut c = Commander::new(g.rules);
        assert_eq!(decide(&mut c, &g)[0], Some(Order::Home));
    }

    #[test]
    fn a_game_with_no_batteries_never_sends_a_drone_to_charge() {
        let g = tired_game(Vec2::new(-220.0, 0.0), 0.0);
        let mut g = g;
        g.rules = Rules::default().without_batteries();
        let mut c = Commander::new(g.rules);
        assert_ne!(decide(&mut c, &g)[0], Some(Order::Home));
    }

    /// An advisor that proposes the same thing on every map.
    #[derive(Clone, Debug)]
    struct Fixed(Proposal);

    impl Advise for Fixed {
        fn advise(&mut self, view: &TeamView, _: &Hand) -> Vec<Proposal> {
            vec![Proposal { made: view.time, until: view.time + 5.0, ..self.0.clone() }]
        }

        fn clone_box(&self) -> Box<dyn Advise> {
            Box::new(self.clone())
        }
    }

    fn proposal(drone: usize, order: Order, gain: f32) -> Proposal {
        Proposal { drone, order, queue: vec![], gain, made: 0.0, until: 10.0, why: "test".into() }
    }

    fn bare(g: &Game) -> Commander {
        Commander::new(g.rules).with_lookahead(false)
    }

    #[test]
    fn a_proposal_with_enough_gain_is_taken() {
        let g = Game::new(Rules::default());
        let red_home = Order::Take(g.tower("Red home"));
        let mut c = bare(&g);
        c.advise([proposal(0, red_home, 5.0)]);
        c.decide(&g.view(Team::Blue));
        assert_eq!(c.order(0), Some(red_home));
    }

    #[test]
    fn a_proposal_that_gains_little_is_not_taken() {
        let g = Game::new(Rules::default());
        let mut c = bare(&g);
        c.advise([proposal(0, Order::Take(g.tower("Red home")), ADVICE_MARGIN)]);
        let advised = c.decide(&g.view(Team::Blue));
        assert_eq!(advised, bare(&g).decide(&g.view(Team::Blue)));
    }

    #[test]
    fn a_stale_proposal_is_not_taken() {
        let mut g = Game::new(Rules::default());
        let red_home = Order::Take(g.tower("Red home"));
        let mut c = bare(&g);
        c.decide(&g.view(Team::Blue));
        let before = c.order(0);
        g.step(10.0);
        c.advise([Proposal { made: 0.0, until: 5.0, ..proposal(0, red_home, 9.0) }]);
        c.decide(&g.view(Team::Blue));
        assert_eq!(c.order(0), before);
    }

    #[test]
    fn a_proposal_for_a_drone_that_is_not_free_is_dropped() {
        let mut g = Game::new(Rules::default());
        let red_home = Order::Take(g.tower("Red home"));
        let i = g.drone(Team::Blue, 0);
        g.drones[i].phase = Phase::Down;
        let mut c = bare(&g);
        c.advise([proposal(0, Order::Hold, 9.0), proposal(99, red_home, 9.0)]);
        c.decide(&g.view(Team::Blue));
        assert_ne!(c.order(0), Some(Order::Hold));
        // It is not kept for later either: the drone is up again, and the proposal is gone.
        g.drones[i].phase = Phase::Ground;
        c.decide(&g.view(Team::Blue));
        assert_ne!(c.order(0), Some(Order::Hold));
    }

    #[test]
    fn a_proposal_made_on_an_old_map_is_not_taken_for_a_tower_that_the_team_holds_now() {
        let mut g = Game::new(Rules::default());
        let west = g.tower("west");
        let mut c = bare(&g);
        c.decide(&g.view(Team::Blue));
        g.step(1.0);
        g.towers[west].holder = Some(Team::Blue);
        g.towers[west].control = 1.0;
        g.look();
        c.advise([proposal(1, Order::Take(west), 9.0)]);
        c.decide(&g.view(Team::Blue));
        assert_ne!(c.order(1), Some(Order::Take(west)));
    }

    #[test]
    fn the_best_proposal_is_taken_first_and_one_at_a_decision() {
        let g = Game::new(Rules::default());
        let (a, b) = (Order::Take(g.tower("Red home")), Order::Take(g.tower("east")));
        let mut c = bare(&g);
        c.advise([proposal(0, a, 4.0), proposal(0, b, 6.0), proposal(2, Order::Hold, 3.0)]);
        c.decide(&g.view(Team::Blue));
        assert_eq!((c.order(0), c.order(2) == Some(Order::Hold)), (Some(b), false));
        c.decide(&g.view(Team::Blue));
        assert_eq!(c.order(2), Some(Order::Hold));
    }

    #[test]
    fn a_taken_proposal_gives_its_queue_to_the_drone() {
        let g = Game::new(Rules::default());
        let red_home = Order::Take(g.tower("Red home"));
        let mut c = bare(&g);
        let q = vec![Queued::done(Order::Home)];
        c.advise([Proposal { queue: q.clone(), ..proposal(0, red_home, 5.0) }]);
        c.decide(&g.view(Team::Blue));
        assert_eq!(c.advised_queue(0), q);
        assert!(c.advised_queue(1).is_empty());
    }

    #[test]
    fn an_advisor_that_is_late_is_taken_when_its_proposal_arrives() {
        let mut g = Game::new(Rules::default());
        let red_home = Order::Take(g.tower("Red home"));
        let mut c = bare(&g).with_advisor(Fixed(proposal(0, red_home, 5.0)), 3.0);
        for _ in 0..3 {
            c.decide(&g.view(Team::Blue));
            assert_ne!(c.order(0), Some(red_home), "at {}", g.time);
            g.step(1.0);
        }
        c.decide(&g.view(Team::Blue));
        assert_eq!(c.order(0), Some(red_home));
    }

    #[test]
    fn an_advisor_with_no_delay_is_taken_at_once_and_one_that_is_later_than_its_proposal_lasts_is_not() {
        let mut g = Game::new(Rules::default());
        let red_home = Order::Take(g.tower("Red home"));
        let mut now = bare(&g).with_advisor(Fixed(proposal(0, red_home, 5.0)), 0.0);
        now.decide(&g.view(Team::Blue));
        assert_eq!(now.order(0), Some(red_home));
        // Good for 5 s, and 8 s late: stale when it arrives.
        let mut late = bare(&g).with_advisor(Fixed(proposal(0, red_home, 5.0)), 8.0);
        for _ in 0..30 {
            late.decide(&g.view(Team::Blue));
            assert_ne!(late.order(0), Some(red_home), "at {}", g.time);
            g.step(1.0);
        }
    }

    #[test]
    fn the_lookahead_as_an_advisor_is_the_commander_that_thinks_ahead() {
        let mut g = Game::new(Rules::default());
        let mut inside = Commander::new(g.rules);
        let mut advised = bare(&g).with_advisor(Lookahead::new(g.rules), 0.0);
        for _ in 0..120 {
            let view = g.view(Team::Blue);
            let (a, b) = (inside.decide(&view), advised.decide(&view));
            assert_eq!(a, b, "at {}", g.time);
            g.step(1.0);
        }
    }
}
