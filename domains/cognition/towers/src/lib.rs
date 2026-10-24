//! The fast Towers arena: two captains play a seeded game of Towers (`recreation_towers`) with no
//! window, no SITL and no server, in milliseconds. The referee, the vision and the fog are the
//! game's own; each captain sees only `Game::view(team)`. The drones fly their orders on their own,
//! as the server's plans make them fly (see [`Flyer`]).
//!
//! A captain is anything that turns a team's view into orders ([`Captain`]): a `Commander` in this
//! process, or one in a process of its own ([`SeatProcess`]), so that two captains with different
//! code can play each other.

mod autonomy;

pub use autonomy::{After, Autonomy, Comparison, Failsafe, Persist, Prepared, compare};

use glam::Vec2;
// Where a drone circles and how high: the game's own numbers, as the server flies them.
use recreation_towers::flight::{ORBIT_RADIUS, orbit_height};
use recreation_towers::{
    Commander, CommanderTuning, Game, Instruction, Order, Outcome, Phase, Rules, Standing, Team, TeamView,
};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// The arena's step (s), and how often each captain decides (s).
const DT: f32 = 0.1;
const DECIDE_EVERY: f32 = 1.0;
/// The take-off: each drone climbs at this rate (m/s) to this height (m), then flies; the drones of
/// a team leave this many seconds apart.
const CLIMB_RATE: f32 = 3.0;
const TAKEOFF_HEIGHT: f32 = 15.0;
const TAKEOFF_APART: f32 = 2.0;
/// A T1-S in AGENT mode: about this airspeed (m/s), a bank of at most 50°, and a climb or sink of
/// at most this (m/s).
const CRUISE: f32 = 16.0;
const BANK: f32 = 50.0 * std::f32::consts::PI / 180.0;
const VERTICAL: f32 = 3.0;
const G: f32 = 9.81;
/// The most wind (m/s), and the delay (s) of an order before the drone flies it, per seed.
const WIND_MAX: f32 = 5.0;
const DELAY: (f32, f32) = (0.3, 2.0);

/// What turns a team's view into orders.
pub trait Captain {
    /// The orders for the team's drones: `(drone, order)` for each.
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String>;

    /// The standing orders that go with `orders`, given just now: `(drone, standing order)`.
    fn standing(&mut self, _view: &TeamView, _orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        vec![]
    }
}

impl<C: Captain + ?Sized> Captain for &mut C {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        (**self).decide(view)
    }

    fn standing(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        (**self).standing(view, orders)
    }
}

impl Captain for Commander {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        Ok(Commander::decide(self, view))
    }
}

/// A drone flying its order: where it is, its heading, and the order it flies.
#[derive(Clone, Copy, Debug)]
pub struct Flyer {
    pub team: Team,
    pub k: usize,
    pub at: Vec2,
    pub alt: f32,
    /// Radians from north, clockwise.
    pub heading: f32,
    pub speed: f32,
    pub flying: bool,
    pub order: Option<Order>,
    /// Where it circles on Hold: where it was when it got the order.
    hold: Vec2,
}

impl Flyer {
    /// The point it circles for its order, and the radius.
    fn circle(&self, game: &Game) -> (Vec2, f32) {
        match self.order {
            Some(Order::Take(t)) => (game.towers[t].at, ORBIT_RADIUS),
            Some(Order::Goto(p)) => (p, ORBIT_RADIUS),
            Some(Order::Home) => (game.rules.base(self.team), ORBIT_RADIUS),
            Some(Order::Hold) => (self.hold, ORBIT_RADIUS),
            // As the server: a drone its seat has not ordered yet circles over its base.
            None => (game.rules.base(self.team), ORBIT_RADIUS),
        }
    }

    /// One step: climbs out, then flies round its circle (a vector field, as the executor's
    /// orbit), its turn limited by the bank, carried by the wind.
    pub fn step(&mut self, game: &Game, wind: Vec2, takeoff_at: f32, dt: f32) {
        if !self.flying {
            if game.time < takeoff_at {
                return;
            }
            self.alt += CLIMB_RATE * dt;
            if self.alt >= TAKEOFF_HEIGHT {
                self.flying = true;
                self.hold = self.at;
            }
            return;
        }
        let (centre, radius) = self.circle(game);
        let r = self.at - centre;
        let d = r.length();
        let phi = if d > 1.0 { r.y.atan2(r.x) } else { self.heading - std::f32::consts::FRAC_PI_2 };
        // Clockwise round the circle, bending towards it from outside or inside.
        let want = phi + std::f32::consts::FRAC_PI_2 + (2.0 * (d - radius) / radius).atan();
        let err = (want - self.heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let max_rate = G * BANK.tan() / self.speed;
        self.heading += (1.5 * err).clamp(-max_rate, max_rate) * dt;
        let air = Vec2::new(self.heading.cos(), self.heading.sin()) * self.speed;
        self.at += (air + wind) * dt;
        let target = orbit_height(self.team, self.k);
        self.alt += (target - self.alt).clamp(-VERTICAL * dt, VERTICAL * dt);
    }
}

/// How one game went.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TowersResult {
    pub seed: u64,
    pub outcome: Outcome,
    /// Blue's, Red's.
    pub score: [f32; 2],
    /// How long it lasted (s).
    pub length: f32,
    /// Drones that took off.
    pub flew: usize,
    /// Towers each team held at the end.
    pub held: [usize; 2],
    /// Drone-seconds out of contact in the radio shadows, for Blue and for Red.
    #[serde(default)]
    pub dark: [f32; 2],
}

impl TowersResult {
    /// The score of the game for `team`: +1 for a win, -1 for a loss, 0 for a draw, plus the points
    /// it scored more than the other, over the score that wins.
    pub fn score_for(&self, team: Team, win: f32) -> f32 {
        let result = match self.outcome {
            Outcome::Won(t) if t == team => 1.0,
            Outcome::Won(_) => -1.0,
            Outcome::Draw => 0.0,
        };
        result + (self.score[team.index()] - self.score[team.other().index()]) / win
    }
}

/// A game in progress: the referee, the drones flying their orders, and the orders on their way.
pub struct Match {
    pub game: Game,
    pub flyers: Vec<Flyer>,
    seed: u64,
    wind: Vec2,
    /// How long an order takes to reach its drone (s).
    delay: f32,
    /// Orders on their way: (when the drone flies it, drone index, order).
    pending: Vec<(f32, usize, Order, Option<Standing>)>,
    /// What each drone was told: its order and its standing order.
    told: Vec<Instruction>,
    next_decision: f32,
    dark: [f32; 2],
}

impl Match {
    /// The start of a game on `seed`: the drones on the ground, the wind and the delay drawn.
    pub fn new(seed: u64) -> Match {
        let rules = Rules::default();
        let mut rng = fastrand::Rng::with_seed(seed);
        let mut game = Game::new(rules);
        let wind = {
            let (speed, dir) = (rng.f32() * WIND_MAX, rng.f32() * std::f32::consts::TAU);
            Vec2::new(dir.cos(), dir.sin()) * speed
        };
        let delay = DELAY.0 + rng.f32() * (DELAY.1 - DELAY.0);
        let flyers = game
            .drones
            .iter()
            .enumerate()
            .map(|(i, d)| Flyer {
                team: d.team,
                k: i % rules.drones,
                at: d.at,
                alt: 0.0,
                heading: if d.team == Team::Blue { 0.0 } else { std::f32::consts::PI },
                speed: CRUISE * (0.95 + 0.1 * rng.f32()),
                flying: false,
                order: None,
                hold: d.at,
            })
            .collect();
        game.look();
        Match {
            game,
            flyers,
            seed,
            wind,
            delay,
            pending: vec![],
            told: vec![Instruction::default(); 2 * rules.drones],
            next_decision: 0.0,
            dark: [0.0; 2],
        }
    }

    /// One step: the captains decide when it is time, the orders arrive, the drones fly, the
    /// referee steps.
    pub fn step(&mut self, blue: &mut dyn Captain, red: &mut dyn Captain) -> Result<(), String> {
        let (game, flyers) = (&mut self.game, &mut self.flyers);
        let drones = game.rules.drones;
        if game.time >= self.next_decision {
            self.next_decision += DECIDE_EVERY;
            for team in Team::BOTH {
                let captain: &mut dyn Captain = if team == Team::Blue { &mut *blue } else { &mut *red };
                let view = game.view(team);
                let orders = captain.decide(&view)?;
                let standing = captain.standing(&view, &orders);
                for &(k, order) in orders.iter().filter(|(k, _)| *k < drones) {
                    let s = standing.iter().find(|(j, _)| *j == k).map(|&(_, s)| s);
                    self.pending.push((game.time + self.delay, game.drone(team, k), order, s));
                }
            }
        }
        // An order reaches a drone that is in contact when it arrives; one in a radio shadow
        // never hears it.
        self.pending.retain(|&(when, i, order, standing)| {
            if game.time < when {
                return true;
            }
            if !game.jammed[i] {
                self.told[i].give(order, standing);
            }
            false
        });
        for (i, told) in self.told.iter_mut().enumerate() {
            // Out of contact long enough, a drone flies its standing order, or home.
            told.tick(game.silent[i], game.rules.link_timeout, game.charge(i), game.rules.bingo);
            if flyers[i].order != told.order {
                flyers[i].hold = flyers[i].at;
                flyers[i].order = told.order;
            }
        }
        for (i, f) in flyers.iter_mut().enumerate() {
            f.step(game, self.wind, TAKEOFF_APART * (i % drones) as f32, DT);
            let d = &mut game.drones[i];
            d.at = f.at;
            d.alt = f.alt;
            d.vel = Vec2::new(f.heading.cos(), f.heading.sin()) * f.speed + self.wind;
            d.phase = if f.flying { Phase::Flying } else { Phase::Ground };
        }
        game.step(DT);
        for (_, d) in game.drones.iter().enumerate().filter(|(i, _)| game.jammed[*i]) {
            self.dark[d.team.index()] += DT;
        }
        Ok(())
    }

    /// How it went, once it is over.
    pub fn result(&self) -> Option<TowersResult> {
        let outcome = self.game.outcome?;
        let held = |team| self.game.towers.iter().filter(|t| t.holder == Some(team)).count();
        Some(TowersResult {
            seed: self.seed,
            outcome,
            score: self.game.score,
            length: self.game.time,
            flew: self.flyers.iter().filter(|f| f.flying).count(),
            held: [held(Team::Blue), held(Team::Red)],
            dark: self.dark,
        })
    }
}

/// Plays one game: `blue` against `red`, on `seed`.
pub fn play(blue: &mut dyn Captain, red: &mut dyn Captain, seed: u64) -> Result<TowersResult, String> {
    let mut m = Match::new(seed);
    loop {
        if let Some(result) = m.result() {
            return Ok(result);
        }
        m.step(blue, red)?;
    }
}

/// A captain in a process of its own: the seat's JSON lines on its stdin and stdout. It gets
/// `{"Start": {"team": ..., "tuning": [...]}}` before each game and a `TeamView` for each
/// decision, and answers each with a line: `"ok"`, or a list of `[drone, order]`.
pub struct SeatProcess {
    child: Child,
    to: ChildStdin,
    from: BufReader<ChildStdout>,
}

/// A line to a captain's process.
#[derive(Serialize, Deserialize)]
pub enum ToCaptain {
    Start { team: Team, tuning: Vec<f32> },
    View(Box<TeamView>),
}

impl SeatProcess {
    /// Starts `command` (with `args`, in `dir`), a program that runs [`serve`].
    pub fn spawn(command: &str, args: &[&str], dir: &std::path::Path) -> Result<SeatProcess, String> {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("{command}: {e}"))?;
        let to = child.stdin.take().unwrap();
        let from = BufReader::new(child.stdout.take().unwrap());
        Ok(SeatProcess { child, to, from })
    }

    fn ask(&mut self, msg: &ToCaptain) -> Result<String, String> {
        writeln!(self.to, "{}", serde_json::to_string(msg).unwrap()).map_err(|e| e.to_string())?;
        let mut line = String::new();
        self.from.read_line(&mut line).map_err(|e| e.to_string())?;
        if line.is_empty() {
            return Err("the captain's process ended".into());
        }
        Ok(line)
    }

    /// A new game, as `team`, with `tuning`.
    pub fn start(&mut self, team: Team, tuning: &CommanderTuning) -> Result<(), String> {
        self.ask(&ToCaptain::Start { team, tuning: tuning.to_array().to_vec() }).map(|_| ())
    }
}

impl Captain for SeatProcess {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        let line = self.ask(&ToCaptain::View(Box::new(view.clone())))?;
        serde_json::from_str(&line).map_err(|e| format!("the captain said {line:?}: {e}"))
    }
}

impl Drop for SeatProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Serves a captain on `input` and `output` (a [`SeatProcess`]'s other end): this build's
/// `Commander`, with the tuning each game starts with.
pub fn serve(input: impl BufRead, mut output: impl Write) -> Result<(), String> {
    let mut captain: Option<Commander> = None;
    for line in input.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let reply = match serde_json::from_str::<ToCaptain>(&line).map_err(|e| e.to_string())? {
            ToCaptain::Start { tuning, .. } => {
                let tuning = <[f32; CommanderTuning::LEN]>::try_from(tuning)
                    .ok()
                    .and_then(|a| CommanderTuning::from_array(&a))
                    .ok_or("a tuning out of bounds")?;
                captain = Some(Commander::with_tuning(Rules::default(), tuning));
                "\"ok\"".to_string()
            }
            ToCaptain::View(view) => {
                let c = captain.as_mut().ok_or("a view before the start")?;
                serde_json::to_string(&Commander::decide(c, &view)).unwrap()
            }
        };
        writeln!(output, "{reply}").map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default() -> Commander {
        Commander::new(Rules::default())
    }

    #[test]
    fn a_drone_flies_to_its_tower_and_circles_it_in_the_circle() {
        let game = Game::new(Rules::default());
        let mut f = Flyer {
            team: Team::Blue,
            k: 0,
            at: game.rules.base(Team::Blue),
            alt: 0.0,
            heading: 0.0,
            speed: CRUISE,
            flying: false,
            order: Some(Order::Take(game.tower("west"))),
            hold: Vec2::ZERO,
        };
        let west = game.towers[game.tower("west")].at;
        let mut inside = 0;
        for i in 0..1200 {
            f.step(&game, Vec2::new(3.0, 0.0), 0.0, DT);
            if i >= 600 && f.at.distance(west) <= game.rules.circle_radius && f.flying {
                inside += 1;
            }
        }
        assert!(inside > 550, "in the west tower's circle for {inside} of the last 600 steps");
        assert!((f.alt - orbit_height(Team::Blue, 0)).abs() < 1.0, "{}", f.alt);
    }

    #[test]
    fn a_game_between_two_captains_ends_with_every_drone_flown() {
        let r = play(&mut default(), &mut default(), 1).unwrap();
        assert_eq!(r.flew, 2 * Rules::default().drones);
        assert!(r.length <= 480.0 + DT && r.length > 60.0, "{r:?}");
        assert!(r.score.iter().any(|s| *s > 0.0), "{r:?}");
    }

    #[test]
    fn the_same_seed_plays_the_same_game() {
        let a = play(&mut default(), &mut default(), 7).unwrap();
        let b = play(&mut default(), &mut default(), 7).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_captain_in_a_process_plays_as_it_would_here() {
        // The other end of the pipe is `serve`, on a thread.
        let (to_r, to_w) = std::io::pipe().unwrap();
        let (from_r, from_w) = std::io::pipe().unwrap();
        std::thread::spawn(move || serve(BufReader::new(to_r), from_w));
        struct Piped(std::io::PipeWriter, BufReader<std::io::PipeReader>);
        impl Captain for Piped {
            fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
                let msg = serde_json::to_string(&ToCaptain::View(Box::new(view.clone()))).unwrap();
                writeln!(self.0, "{msg}").unwrap();
                let mut line = String::new();
                self.1.read_line(&mut line).unwrap();
                serde_json::from_str(&line).map_err(|e| e.to_string())
            }
        }
        let mut piped = Piped(to_w, BufReader::new(from_r));
        let start = ToCaptain::Start { team: Team::Red, tuning: CommanderTuning::DEFAULT.to_array().to_vec() };
        writeln!(piped.0, "{}", serde_json::to_string(&start).unwrap()).unwrap();
        let mut ok = String::new();
        piped.1.read_line(&mut ok).unwrap();
        assert_eq!(ok.trim(), "\"ok\"");
        let here = play(&mut default(), &mut default(), 3).unwrap();
        let there = play(&mut default(), &mut piped, 3).unwrap();
        assert_eq!(here, there);
    }

    #[test]
    fn a_drone_with_no_order_circles_over_its_base_as_the_server_flies_it() {
        struct Silent;
        impl Captain for Silent {
            fn decide(&mut self, _: &TeamView) -> Result<Vec<(usize, Order)>, String> {
                Ok(vec![])
            }
        }
        let r = play(&mut Silent, &mut Silent, 5).unwrap();
        // Neither team leaves its base: nothing is taken, and each scores from its home tower.
        assert_eq!(r.held, [1, 1], "{r:?}");
        assert_eq!(r.outcome, Outcome::Draw);
    }

    #[test]
    fn the_score_is_the_result_and_the_margin() {
        let r = TowersResult {
            seed: 0,
            outcome: Outcome::Won(Team::Blue),
            score: [500.0, 250.0],
            length: 300.0,
            flew: 6,
            held: [3, 1],
            dark: [0.0; 2],
        };
        assert_eq!(r.score_for(Team::Blue, 500.0), 1.5);
        assert_eq!(r.score_for(Team::Red, 500.0), -1.5);
    }

    /// A captain that gives Blue's first drone the same order at every decision, in contact or
    /// not, with a standing order.
    struct First(Order, Option<Standing>);

    impl Captain for First {
        fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
            Ok(if view.team == Team::Blue { vec![(0, self.0)] } else { vec![] })
        }

        fn standing(&mut self, _: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
            orders.iter().filter_map(|&(k, _)| self.1.map(|s| (k, s))).collect()
        }
    }

    struct Silent;

    impl Captain for Silent {
        fn decide(&mut self, _: &TeamView) -> Result<Vec<(usize, Order)>, String> {
            Ok(vec![])
        }
    }

    /// Where Blue's first drone circles: deep in the shadow of the Red home tower (a circle of 40 m
    /// stays inside it), or outside it.
    fn put_blue(m: &mut Match, shadow: bool) -> usize {
        let i = m.game.drone(Team::Blue, 0);
        let red = m.game.tower("Red home");
        m.game.towers[red].holder = Some(Team::Red);
        let at = m.game.towers[red].at + if shadow { Vec2::new(-30.0, 0.0) } else { Vec2::new(-300.0, 0.0) };
        let f = &mut m.flyers[i];
        (f.at, f.alt, f.flying, f.hold) = (at, 50.0, true, at);
        m.game.drones[i].at = at;
        m.game.drones[i].alt = 50.0;
        m.game.drones[i].phase = Phase::Flying;
        m.game.look();
        i
    }

    fn run(m: &mut Match, secs: f32, blue: &mut dyn Captain) {
        for _ in 0..(secs / DT).round() as usize {
            m.step(blue, &mut Silent).unwrap();
        }
    }

    #[test]
    fn a_drone_in_a_radio_shadow_hears_no_order() {
        let mut m = Match::new(1);
        let i = put_blue(&mut m, true);
        let west = m.game.tower("west");
        run(&mut m, 3.0, &mut First(Order::Take(west), None));
        assert!(m.game.jammed[i], "the drone is in the shadow");
        assert_eq!(m.flyers[i].order, None, "an order to a drone out of contact is lost");
    }

    #[test]
    fn a_drone_that_loses_its_link_flies_home() {
        // It holds where it is, in the shadow: the drone that flies out of the shadow is in contact.
        let mut m = Match::new(1);
        let i = put_blue(&mut m, true);
        m.told[i].give(Order::Hold, None);
        run(&mut m, 19.0, &mut Silent);
        assert!(m.game.jammed[i], "it holds in the shadow");
        assert_eq!(m.flyers[i].order, Some(Order::Hold));
        run(&mut m, 2.0, &mut Silent);
        assert_eq!(m.flyers[i].order, Some(Order::Home));
    }

    #[test]
    fn a_standing_order_given_in_contact_is_flown_in_the_shadow() {
        let mut m = Match::new(1);
        let i = put_blue(&mut m, false);
        let standing = Standing { after: 5.0, then: Order::Home };
        let mut captain = First(Order::Hold, Some(standing));
        run(&mut m, 3.0, &mut captain);
        assert_eq!(m.flyers[i].order, Some(Order::Hold));
        assert_eq!(m.told[i].standing, Some(standing));
        // Into the shadow: out of contact. Not the failsafe's 20 s, but the standing order's 5 s.
        put_blue(&mut m, true);
        run(&mut m, 10.0, &mut Silent);
        assert_eq!(m.flyers[i].order, Some(Order::Home));
    }
}
