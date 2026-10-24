//! The computer commander: a team's orders, once a second, from its team's map alone.
//!
//! Its numbers are [`CommanderTuning`]: each has its default (the constant of the same name),
//! its bounds, and may be set per commander ([`Commander::with_tuning`]). This file is the one a
//! strategy that improves commanders may change; the rules, the referee, the fog and the view
//! are elsewhere.

use crate::{Kind, Order, Phase, Rules, TeamView};
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
    /// gets the drone nearest to it, if no drone is on it already. Then each tower the team does not hold, nearest the
    /// base first, gets the nearest drone with nothing to do; a drone keeps its tower until the
    /// team holds it. Drones left over join the nearest tower the team does not hold. A drone that
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
            let need = self.rules.bingo + home / CRUISE + REFUEL_MARGIN;
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
        let mut targets: Vec<usize> = (0..view.towers.len()).filter(|&t| !ours(t)).collect();
        targets.sort_by(|&a, &b| view.towers[a].at.distance(base).total_cmp(&view.towers[b].at.distance(base)));
        for &t in &targets {
            if !on(&self.orders, t)
                && let Some(k) = nearest(&self.orders, t, false)
            {
                self.orders[k] = Some(Order::Take(t));
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
        for (k, order) in charge.into_iter().enumerate() {
            if order.is_some() {
                self.orders[k] = order;
            }
        }
        self.orders.iter().enumerate().filter(|(k, _)| linked[*k]).filter_map(|(k, o)| o.map(|o| (k, o))).collect()
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
            Commander::with_tuning(g.rules, CommanderTuning { give_up: 60.0, ..CommanderTuning::DEFAULT });
        patient.assign(0, c.order(0));
        let west = Some(Order::Take(g.tower("west")));
        let mut g2 = g.clone();
        assert_ne!(play(&mut g, &mut c, 15), west);
        assert_eq!(play(&mut g2, &mut patient, 15), west);
    }

    #[test]
    fn at_the_start_it_spreads_its_drones_over_the_neutral_towers_nearest_first() {
        let g = Game::new(Rules::default());
        let mut c = Commander::new(g.rules);
        let (west, centre, east) = (g.tower("west"), g.tower("centre"), g.tower("east"));
        let orders = c.decide(&g.view(Team::Blue));
        assert_eq!(orders, vec![(0, Order::Take(west)), (1, Order::Take(centre)), (2, Order::Take(east))]);
    }

    #[test]
    fn a_drone_keeps_its_tower_until_the_team_holds_it_then_goes_on() {
        let mut g = Game::new(Rules::default());
        let mut c = Commander::new(g.rules);
        let (west, centre) = (g.tower("west"), g.tower("centre"));
        decide(&mut c, &g);
        // The drone for the centre flies off towards it: still its order, though the west tower is
        // now nearer to it than the centre is to the others.
        let i = g.drone(Team::Blue, 1);
        g.drones[i] = crate::Drone { at: Vec2::new(-50.0, -150.0), alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
        g.step(0.1);
        assert_eq!(decide(&mut c, &g)[1], Some(Order::Take(centre)));
        // Blue takes the centre: that drone goes on to the nearest tower nobody is sent to.
        g.towers[centre].holder = Some(Team::Blue);
        g.towers[centre].control = 1.0;
        g.drones[i].at = Vec2::new(40.0, 0.0);
        g.step(0.1);
        let orders = decide(&mut c, &g);
        assert_eq!(orders[0], Some(Order::Take(west)));
        assert_eq!(orders[1], Some(Order::Take(g.tower("Red home"))));
    }

    #[test]
    fn a_crashed_drone_gets_no_order_and_the_others_keep_theirs() {
        let mut g = Game::new(Rules::default());
        let mut c = Commander::new(g.rules);
        let before = decide(&mut c, &g);
        let i = g.drone(Team::Blue, 1);
        g.drones[i].phase = Phase::Down;
        let orders = decide(&mut c, &g);
        assert_eq!(orders, vec![before[0], None, before[2]]);
        // Back at its base, it gets the tower nobody is on.
        g.drones[i].phase = Phase::Ground;
        assert_eq!(decide(&mut c, &g)[1], Some(Order::Take(g.tower("centre"))));
    }

    #[test]
    fn spare_drones_join_the_nearest_tower_the_team_does_not_hold() {
        let mut g = Game::new(Rules::default());
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
        assert_eq!(orders, vec![red, centre, centre]);
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
                crate::Drone { at: at + Vec2::new(0.0, 40.0), alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
        }
        let mut c = Commander::new(g.rules);
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
}
