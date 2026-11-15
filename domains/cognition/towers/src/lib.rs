//! The fast Towers arena: two captains play a seeded game of Towers (`recreation_towers`) with no
//! window, no SITL and no server, in milliseconds. The referee, the vision and the fog are the
//! game's own; each captain sees only `Game::view(team)`. The drones fly their orders on their own,
//! as the server's plans make them fly (see [`Flyer`]).
//!
//! A game is played by a roster ([`Roster`]): a [`Command`] for each team, which turns the team's
//! view into orders, and a [`Unit`] for each drone, which flies on what the drone has on board
//! (`recreation_towers::actors`). A [`Captain`] is the two together, as one object that is the
//! team's commander and makes each drone's unit: a `Commander` in this process, or one in a process
//! of its own ([`SeatProcess`]), so that two captains with different code can play each other.

mod autonomy;
mod edge;
mod formation;

pub use autonomy::{
    After, Autonomy, Chain, Comparison, Failsafe, Persist, Prepared, Surface, Timed, compare, compare_captains,
    compare_with,
};
pub use edge::{Edge, EdgeUnit, Edged, FinishThenReturn, Keep, Outnumbered, compare_edge};
pub use formation::{Formation, Post};

use glam::Vec2;
// Where a drone circles and how high: the game's own numbers, as the server flies them.
use recreation_towers::Commander;
use recreation_towers::flight::{ORBIT_RADIUS, low_height, orbit_height};
use recreation_towers::{
    Autopilot, Command, CommanderTuning, Directive, Game, Instruction, OnBoard, Order, Outcome, Phase, Queued, Rules,
    Standing, Team, TeamView, Unit,
};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};

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

/// A team's commander and the brain of each of its drones, as one object. It splits into a
/// [`Command`] (see [`Directing`]) and a [`Unit`] for each drone ([`Captain::unit`]): nothing but the
/// unit's own state goes on board.
pub trait Captain {
    /// The orders for the team's drones: `(drone, order)` for each.
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String>;

    /// The standing orders that go with `orders`, given just now: `(drone, standing order)`.
    fn standing(&mut self, _view: &TeamView, _orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        vec![]
    }

    /// The queues that go with `orders`, given just now: `(drone, queue)`. A drone with no entry
    /// gets none. By default each standing order is a queue of one.
    fn queue(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Vec<Queued>)> {
        self.standing(view, orders).into_iter().map(|(k, s)| (k, vec![Queued::from(s)])).collect()
    }

    /// The unit of drone `k` of `team`: its own brain on board. By default the [`Autopilot`]. It
    /// is made once, before the game, and shares nothing with the captain that a drone could not
    /// have on board.
    fn unit(&self, _team: Team, _k: usize) -> Box<dyn Unit> {
        Box::new(Autopilot)
    }
}

impl<C: Captain + ?Sized> Captain for &mut C {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        (**self).decide(view)
    }

    fn standing(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        (**self).standing(view, orders)
    }

    fn queue(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Vec<Queued>)> {
        (**self).queue(view, orders)
    }

    fn unit(&self, team: Team, k: usize) -> Box<dyn Unit> {
        (**self).unit(team, k)
    }
}

impl Captain for recreation_towers::Commander {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        Ok(recreation_towers::Commander::decide(self, view))
    }
}

/// A captain as a [`Command`]: its orders go out with the queues it prepares for them.
pub struct Directing<C>(pub C);

impl<C: Captain> Command for Directing<C> {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<Directive>, String> {
        let orders = self.0.decide(view)?;
        let queues = self.0.queue(view, &orders);
        Ok(orders
            .into_iter()
            .map(|(k, order)| {
                let queue = queues.iter().find(|(j, _)| *j == k).map(|(_, q)| q.clone()).unwrap_or_default();
                Directive { drone: k, order, queue }
            })
            .collect())
    }
}

/// Who plays: a [`Command`] for each team and a [`Unit`] for each drone, the drones of Blue first.
pub struct Roster<'a> {
    pub blue: Box<dyn Command + 'a>,
    pub red: Box<dyn Command + 'a>,
    pub units: Vec<Box<dyn Unit + 'a>>,
}

impl<'a> Roster<'a> {
    /// The two commanders, and `unit(team, k)` for drone `k` of each team.
    pub fn new(
        rules: &Rules,
        blue: impl Command + 'a,
        red: impl Command + 'a,
        mut unit: impl FnMut(Team, usize) -> Box<dyn Unit + 'a>,
    ) -> Roster<'a> {
        let units =
            Team::BOTH.into_iter().flat_map(|t| (0..rules.drones).map(move |k| (t, k))).map(|(t, k)| unit(t, k));
        Roster { blue: Box::new(blue), red: Box::new(red), units: units.collect() }
    }

    /// Two captains, each with the units it makes.
    pub fn of_captains(rules: &Rules, blue: impl Captain + 'a, red: impl Captain + 'a) -> Roster<'a> {
        let units = Team::BOTH.into_iter().flat_map(|t| (0..rules.drones).map(move |k| (t, k))).map(|(t, k)| {
            let captain = if t == Team::Blue { &blue as &dyn Captain } else { &red };
            captain.unit(t, k)
        });
        let units = units.collect();
        Roster { blue: Box::new(Directing(blue)), red: Box::new(Directing(red)), units }
    }

    /// Replaces the unit of drone `k` of `team`.
    pub fn with_unit(mut self, rules: &Rules, team: Team, k: usize, unit: impl Unit + 'a) -> Roster<'a> {
        self.units[team.index() * rules.drones + k] = Box::new(unit);
        self
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
            Some(Order::Circle(p, r)) => (p, r),
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
        // On its circle at a tower it takes, it comes down below the treetops: only a low drone
        // takes a tower.
        let taking = matches!(self.order, Some(Order::Take(_))) && d <= radius + 20.0;
        let target = if taking { low_height(self.team, self.k) } else { orbit_height(self.team, self.k) };
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
    /// Drones that went down with an empty battery, for Blue and for Red.
    #[serde(default)]
    pub downs: [usize; 2],
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
pub struct Match<'u> {
    pub game: Game,
    pub flyers: Vec<Flyer>,
    seed: u64,
    wind: Vec2,
    /// How long an order takes to reach its drone (s).
    delay: f32,
    /// Orders on their way: (when the drone flies it, drone index, order).
    pending: Vec<(f32, usize, Order, Vec<Queued>)>,
    /// What each drone was told: its order and its queue.
    told: Vec<Instruction>,
    next_decision: f32,
    dark: [f32; 2],
    /// Each drone is down now, and how many times each team's drones went down.
    down: Vec<bool>,
    downs: [usize; 2],
    /// Each drone has taken off.
    took_off: Vec<bool>,
    /// The unit of each drone: its own brain, run on every step.
    units: Vec<Box<dyn Unit + 'u>>,
}

impl<'u> Match<'u> {
    /// The start of a game on `seed`: the drones on the ground, the wind and the delay drawn.
    pub fn new(seed: u64) -> Match<'u> {
        Match::with_rules(Rules::default(), seed)
    }

    /// The same, with the rules of the game. Every drone has the default unit.
    pub fn with_rules(rules: Rules, seed: u64) -> Match<'u> {
        let units = (0..2 * rules.drones).map(|_| Box::new(Autopilot) as Box<dyn Unit + 'u>).collect();
        Match::with_units(rules, seed, units)
    }

    /// The same, with a unit for each drone, the drones of Blue first.
    pub fn with_units(rules: Rules, seed: u64, units: Vec<Box<dyn Unit + 'u>>) -> Match<'u> {
        assert_eq!(units.len(), 2 * rules.drones, "a unit for each drone");
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
            down: vec![false; 2 * rules.drones],
            downs: [0; 2],
            took_off: vec![false; 2 * rules.drones],
            units,
        }
    }

    /// One step with captains: see [`Match::step_with`]. Each team's commander is its captain's
    /// [`Directing`]; the drones' units are the ones the match was built with.
    pub fn step(&mut self, blue: &mut dyn Captain, red: &mut dyn Captain) -> Result<(), String> {
        self.step_with(&mut Directing(blue), &mut Directing(red))
    }

    /// One step: the commanders decide when it is time, the orders arrive, each unit flies its
    /// drone, the referee steps.
    pub fn step_with(&mut self, blue: &mut dyn Command, red: &mut dyn Command) -> Result<(), String> {
        let (game, flyers) = (&mut self.game, &mut self.flyers);
        let drones = game.rules.drones;
        if game.time >= self.next_decision {
            self.next_decision += DECIDE_EVERY;
            for team in Team::BOTH {
                let commander: &mut dyn Command = if team == Team::Blue { &mut *blue } else { &mut *red };
                for d in commander.decide(&game.view(team))?.into_iter().filter(|d| d.drone < drones) {
                    self.pending.push((game.time + self.delay, game.drone(team, d.drone), d.order, d.queue));
                }
            }
        }
        // An order reaches a drone that is in contact when it arrives; one in a radio shadow
        // never hears it.
        self.pending.retain(|(when, i, order, queue)| {
            if game.time < *when {
                return true;
            }
            if !game.jammed[*i] {
                self.told[*i].give_queue(*order, queue.clone());
            }
            false
        });
        for (i, told) in self.told.iter_mut().enumerate() {
            // Each drone's unit runs on every step, in contact or not, from what the drone has on
            // board and what it was told. What it returns is the order the drone flies.
            let flown = self.units[i].fly(&OnBoard::new(game, i), told);
            if flyers[i].order != flown {
                flyers[i].hold = flyers[i].at;
                flyers[i].order = flown;
            }
        }
        for (i, f) in flyers.iter_mut().enumerate() {
            // The referee puts a drone down when its battery is empty, and stands it at its base
            // again later: a drone that is down does not fly, and one that is back starts again.
            if game.drones[i].phase == Phase::Down {
                if !self.down[i] {
                    self.down[i] = true;
                    f.flying = false;
                    self.downs[game.drones[i].team.index()] += 1;
                    self.pending.retain(|p| p.1 != i);
                }
                continue;
            }
            if self.down[i] {
                self.down[i] = false;
                let d = game.drones[i];
                let heading = if d.team == Team::Blue { 0.0 } else { std::f32::consts::PI };
                *f = Flyer { at: d.at, alt: 0.0, heading, flying: false, order: None, hold: d.at, ..*f };
                self.told[i] = Instruction::default();
                self.units[i].restart();
            }
            f.step(game, self.wind, TAKEOFF_APART * (i % drones) as f32, DT);
            self.took_off[i] |= f.flying;
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
            flew: self.took_off.iter().filter(|&&t| t).count(),
            held: [held(Team::Blue), held(Team::Red)],
            dark: self.dark,
            downs: self.downs,
        })
    }
}

/// The rules the league plays by: the defaults, on the layout that `SIM001_LAYOUT` names
/// (`chain`; anything else is the valley). A captain's own process reads the same variable.
pub fn league_rules() -> Rules {
    let layout = match std::env::var("SIM001_LAYOUT").as_deref() {
        Ok("chain") => recreation_towers::map::LayoutKind::Chain,
        _ => recreation_towers::map::LayoutKind::Valley,
    };
    Rules { layout, ..Rules::default() }
}

/// Plays one game: `blue` against `red`, on `seed`.
pub fn play(blue: &mut dyn Captain, red: &mut dyn Captain, seed: u64) -> Result<TowersResult, String> {
    play_with(league_rules(), blue, red, seed)
}

/// The same, with the rules of the game.
pub fn play_with(
    rules: Rules,
    blue: &mut dyn Captain,
    red: &mut dyn Captain,
    seed: u64,
) -> Result<TowersResult, String> {
    let units = Team::BOTH
        .into_iter()
        .flat_map(|t| (0..rules.drones).map(move |k| (t, k)))
        .map(|(t, k)| if t == Team::Blue { blue.unit(t, k) } else { red.unit(t, k) })
        .collect();
    let mut m = Match::with_units(rules, seed, units);
    loop {
        if let Some(result) = m.result() {
            return Ok(result);
        }
        m.step(blue, red)?;
    }
}

/// Plays one game by the rules, with a roster: a commander for each team and a unit for each drone.
pub fn play_roster(rules: Rules, roster: Roster<'_>, seed: u64) -> Result<TowersResult, String> {
    let Roster { mut blue, mut red, units } = roster;
    let mut m = Match::with_units(rules, seed, units);
    loop {
        if let Some(result) = m.result() {
            return Ok(result);
        }
        m.step_with(&mut blue, &mut red)?;
    }
}

/// A captain in a process of its own: the seat's JSON lines on its stdin and stdout. It gets
/// `{"Start": {"team": ..., "tuning": [...]}}` before each game and a `TeamView` for each
/// decision, and answers each with a line: `"ok"`, or a list of `[drone, order]` or
/// `[drone, order, queue]` (the queue that goes with the order, as a list of [`Queued`]). Only
/// `decide` crosses: the drones' units are the default.
pub struct SeatProcess {
    child: Child,
    to: ChildStdin,
    from: BufReader<ChildStdout>,
    /// The queues of the last reply.
    queues: Vec<(usize, Vec<Queued>)>,
}

/// One order of a captain's reply: the old reply is `[drone, order]`, a reply with a queue
/// `[drone, order, queue]`. Both are read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reply {
    Plain(usize, Order),
    Queued(usize, Order, Vec<Queued>),
}

impl From<&Directive> for Reply {
    /// An order with no queue is sent as it always was.
    fn from(d: &Directive) -> Reply {
        if d.queue.is_empty() {
            Reply::Plain(d.drone, d.order)
        } else {
            Reply::Queued(d.drone, d.order, d.queue.clone())
        }
    }
}

impl From<Reply> for Directive {
    fn from(r: Reply) -> Directive {
        match r {
            Reply::Plain(drone, order) => Directive::new(drone, order),
            Reply::Queued(drone, order, queue) => Directive::with_queue(drone, order, queue),
        }
    }
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
        let mut child = std::process::Command::new(command)
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("{command}: {e}"))?;
        let to = child.stdin.take().unwrap();
        let from = BufReader::new(child.stdout.take().unwrap());
        Ok(SeatProcess { child, to, from, queues: vec![] })
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
        let reply: Vec<Reply> = serde_json::from_str(&line).map_err(|e| format!("the captain said {line:?}: {e}"))?;
        let directives: Vec<Directive> = reply.into_iter().map(Directive::from).collect();
        self.queues = directives.iter().filter(|d| !d.queue.is_empty()).map(|d| (d.drone, d.queue.clone())).collect();
        Ok(directives.into_iter().map(|d| (d.drone, d.order)).collect())
    }

    fn queue(&mut self, _: &TeamView, _: &[(usize, Order)]) -> Vec<(usize, Vec<Queued>)> {
        std::mem::take(&mut self.queues)
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
pub fn serve(input: impl BufRead, output: impl Write) -> Result<(), String> {
    serve_with(input, output, |tuning| Box::new(Commander::with_tuning(league_rules(), tuning)))
}

/// The same with any [`Command`], made from the tuning each game starts with. Each reply carries
/// the queue of each order that has one.
pub fn serve_with(
    input: impl BufRead,
    mut output: impl Write,
    make: impl Fn(CommanderTuning) -> Box<dyn Command>,
) -> Result<(), String> {
    let mut captain: Option<Box<dyn Command>> = None;
    for line in input.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let reply = match serde_json::from_str::<ToCaptain>(&line).map_err(|e| e.to_string())? {
            ToCaptain::Start { tuning, .. } => {
                let tuning = <[f32; CommanderTuning::LEN]>::try_from(tuning)
                    .ok()
                    .and_then(|a| CommanderTuning::from_array(&a))
                    .ok_or("a tuning out of bounds")?;
                captain = Some(make(tuning));
                "\"ok\"".to_string()
            }
            ToCaptain::View(view) => {
                let c = captain.as_mut().ok_or("a view before the start")?;
                let reply: Vec<Reply> = c.decide(&view)?.iter().map(Reply::from).collect();
                serde_json::to_string(&reply).unwrap()
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
    fn a_drone_flies_to_its_tower_and_circles_it_low_in_the_circle() {
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
        // Low, below the treetops: only a low drone takes a tower.
        assert!((f.alt - low_height(Team::Blue, 0)).abs() < 1.0 && f.alt < game.rules.treetop, "{}", f.alt);
    }

    #[test]
    fn a_game_between_two_captains_ends_with_every_drone_flown() {
        let r = play(&mut default(), &mut default(), 1).unwrap();
        assert_eq!(r.flew, 2 * Rules::default().drones);
        assert!(r.length <= Rules::default().duration + DT && r.length > 60.0, "{r:?}");
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
            downs: [0; 2],
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
        run(&mut m, 44.0, &mut Silent);
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
        assert_eq!(m.told[i].standing(), Some(standing));
        // Into the shadow: out of contact. Not the failsafe's 20 s, but the standing order's 5 s.
        put_blue(&mut m, true);
        run(&mut m, 10.0, &mut Silent);
        assert_eq!(m.flyers[i].order, Some(Order::Home));
    }

    #[test]
    fn a_queue_given_in_contact_is_flown_down_in_the_shadow() {
        let mut m = Match::new(1);
        let i = put_blue(&mut m, true);
        let here = m.game.drones[i].at;
        let queue = vec![Queued::silent(1.0, Order::Goto(here)), Queued::done(Order::Home)];
        m.told[i].give_queue(Order::Hold, queue);
        run(&mut m, 0.5, &mut Silent);
        assert_eq!(m.flyers[i].order, Some(Order::Hold), "the first instruction waits for the silence");
        run(&mut m, 3.0, &mut Silent);
        assert_eq!(m.flyers[i].order, Some(Order::Home), "it is there, so the second instruction starts");
    }

    #[test]
    fn a_policy_with_a_queue_of_three_gives_the_next_tower_and_home() {
        let m = Match::new(1);
        let view = m.game.view(Team::Blue);
        let west = m.game.tower("west");
        let q = Chain.queue(&view, 0, Order::Take(west));
        assert_eq!(q.len(), 3);
        assert_eq!(q[0], Queued::silent(0.0, Order::Take(west)));
        assert!(matches!(q[1], Queued { when: recreation_towers::When::Done, order: Order::Take(t) } if t != west));
        assert_eq!(q[2], Queued::done(Order::Home));
    }

    fn tired() -> Rules {
        Rules { light_battery: 150.0, standard_battery: 240.0, heavy_battery: 300.0, ..Rules::default() }
    }

    #[test]
    fn a_drone_with_an_empty_battery_is_down_and_flies_again_after_the_respawn() {
        let mut m = Match::with_rules(tired(), 1);
        let i = put_blue(&mut m, false);
        m.game.battery[i] = 5.0;
        m.told[i].give(Order::Hold, None);
        run(&mut m, 6.0, &mut Silent);
        assert_eq!(m.game.drones[i].phase, Phase::Down);
        assert!(!m.flyers[i].flying, "a drone that is down does not fly");
        let down_at = m.game.drones[i].at;
        run(&mut m, 90.0, &mut Silent);
        assert_eq!(m.game.drones[i].phase, Phase::Down);
        assert_eq!(m.game.drones[i].at, down_at, "it stays where it fell");
        run(&mut m, 12.0, &mut Silent);
        assert!(m.game.drones[i].at.distance(m.game.rules.base(Team::Blue)) < 60.0, "back at its base");
        assert_eq!(m.game.battery[i], 240.0);
        run(&mut m, 10.0, &mut Silent);
        assert!(m.flyers[i].flying, "it takes off again");
        assert_eq!(m.flyers[i].order, None, "and has to be told what to do");
    }

    #[test]
    fn the_result_counts_the_drones_that_went_down() {
        let mut m = Match::with_rules(tired(), 1);
        let i = put_blue(&mut m, false);
        m.game.battery[i] = 2.0;
        m.told[i].give(Order::Hold, None);
        m.game.time = m.game.rules.duration - 10.0;
        run(&mut m, 11.0, &mut Silent);
        assert_eq!(m.result().unwrap().downs, [1, 0]);
    }

    // ---- The roster: a commander for each team, a unit for each drone ----

    use std::cell::RefCell;
    use std::rc::Rc;

    /// What a counting unit saw, for each drone: steps it ran, steps it ran flying in contact, and
    /// flying out of contact.
    type Calls = Rc<RefCell<Vec<[usize; 3]>>>;

    /// A unit that counts its calls and flies as the autopilot does.
    struct Counting(usize, Calls);

    impl Unit for Counting {
        fn fly(&mut self, on: &OnBoard, told: &mut Instruction) -> Option<Order> {
            let mut calls = self.1.borrow_mut();
            calls[self.0][0] += 1;
            if on.flying() {
                calls[self.0][if on.in_contact() { 1 } else { 2 }] += 1;
            }
            Autopilot.fly(on, told)
        }
    }

    /// A unit that always flies home, whatever it was told, in contact or not.
    struct Recall;

    impl Unit for Recall {
        fn fly(&mut self, _: &OnBoard, told: &mut Instruction) -> Option<Order> {
            told.decide(Order::Home);
            told.order
        }
    }

    fn commander() -> Commander {
        Commander::new(Rules::default()).with_lookahead(false)
    }

    #[test]
    fn a_unit_runs_on_every_step_for_every_drone_in_contact_or_not() {
        let rules = Rules::default();
        let calls: Calls = Rc::new(RefCell::new(vec![[0; 3]; 2 * rules.drones]));
        let units = (0..2 * rules.drones).map(|i| Box::new(Counting(i, calls.clone())) as Box<dyn Unit>).collect();
        let mut m = Match::with_units(rules, 1, units);
        // Blue's first drone flies in a radio shadow; the others take off and fly in contact.
        let shadowed = put_blue(&mut m, true);
        let steps = 200;
        for _ in 0..steps {
            m.step_with(&mut Directing(Silent), &mut Directing(Silent)).unwrap();
        }
        let calls = calls.borrow();
        assert!(calls.iter().all(|c| c[0] == steps), "each drone's unit ran on every step: {calls:?}");
        assert!(calls[shadowed][2] > 50, "a unit flying out of contact runs: {:?}", calls[shadowed]);
        let linked: usize = calls.iter().enumerate().filter(|(i, _)| *i != shadowed).map(|(_, c)| c[1]).sum();
        assert!(linked > 50, "units flying in contact run too: {calls:?}");
    }

    #[test]
    fn a_roster_of_default_units_plays_the_game_that_play_does() {
        let rules = Rules::default();
        let by_roster = play_roster(rules, Roster::of_captains(&rules, commander(), commander()), 4).unwrap();
        assert_eq!(by_roster, play_with(rules, &mut commander(), &mut commander(), 4).unwrap());
    }

    /// Plays Blue's commander against Red's, with `unit(team, k)` for each drone.
    fn mixed(seed: u64, unit: impl FnMut(Team, usize) -> Box<dyn Unit>) -> TowersResult {
        let rules = Rules::default();
        play_roster(rules, Roster::new(&rules, commander(), commander(), unit), seed).unwrap()
    }

    fn default_units(_: Team, _: usize) -> Box<dyn Unit> {
        Box::new(Autopilot)
    }

    #[test]
    fn the_two_teams_can_fly_different_units() {
        // Blue's drones on the autopilot; Red's go home when one enemy is near, as they take a tower.
        let red_edge = |t: Team, _| -> Box<dyn Unit> {
            if t == Team::Blue { Box::new(Autopilot) } else { Box::new(EdgeUnit::new(t, Outnumbered { n: 1 })) }
        };
        let mut differs = 0;
        for seed in 1..=6 {
            let (a, b) = (mixed(seed, red_edge), mixed(seed, red_edge));
            assert_eq!(a, b, "the same seed plays the same game");
            differs += (a != mixed(seed, default_units)) as usize;
        }
        assert!(differs > 0, "Red's unit never changed a game");
    }

    #[test]
    fn one_drone_can_fly_a_unit_unlike_its_team_mates() {
        // Blue's first drone ignores its orders and flies home; its team-mates fly them.
        let recall = |t: Team, k: usize| -> Box<dyn Unit> {
            if (t, k) == (Team::Blue, 0) { Box::new(Recall) } else { Box::new(Autopilot) }
        };
        let (a, b) = (mixed(2, recall), mixed(2, recall));
        assert_eq!(a, b);
        assert_ne!(a, mixed(2, default_units), "the drone that flew home changed the game");
    }

    #[test]
    fn a_roster_can_swap_the_unit_of_one_drone() {
        let rules = Rules::default();
        let play = || {
            let roster = Roster::of_captains(&rules, commander(), commander()).with_unit(&rules, Team::Red, 1, Recall);
            play_roster(rules, roster, 2).unwrap()
        };
        assert_eq!(play(), play());
        assert_ne!(play(), play_with(rules, &mut commander(), &mut commander(), 2).unwrap());
    }

    #[test]
    fn an_old_reply_with_no_queue_is_read_and_a_queue_comes_with_the_order() {
        let old: Vec<Reply> = serde_json::from_str(r#"[[0,"Hold"],[3,{"Take":2}]]"#).unwrap();
        assert_eq!(old, vec![Reply::Plain(0, Order::Hold), Reply::Plain(3, Order::Take(2))]);
        let queue = vec![Queued::silent(5.0, Order::Home), Queued::done(Order::Hold)];
        let new = Reply::from(&Directive::with_queue(1, Order::Take(2), queue.clone()));
        let line = serde_json::to_string(&vec![new.clone()]).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Reply>>(&line).unwrap(), vec![new]);
        // An order with no queue goes out as before.
        let plain = serde_json::to_string(&vec![Reply::from(&Directive::new(0, Order::Hold))]).unwrap();
        assert_eq!(plain, r#"[[0,"Hold"]]"#);
        assert_eq!(Directive::from(Reply::Queued(1, Order::Home, queue.clone())).queue, queue);
    }

    #[test]
    fn a_commander_served_in_a_process_sends_each_order_with_its_queue() {
        struct Queueing;
        impl Command for Queueing {
            fn decide(&mut self, _: &TeamView) -> Result<Vec<Directive>, String> {
                Ok(vec![Directive::with_queue(0, Order::Hold, vec![Queued::silent(5.0, Order::Home)])])
            }
        }
        let (to_r, mut to_w) = std::io::pipe().unwrap();
        let (from_r, from_w) = std::io::pipe().unwrap();
        std::thread::spawn(move || serve_with(BufReader::new(to_r), from_w, |_| Box::new(Queueing)));
        let mut from = BufReader::new(from_r);
        let start = ToCaptain::Start { team: Team::Blue, tuning: CommanderTuning::DEFAULT.to_array().to_vec() };
        let view = ToCaptain::View(Box::new(Game::new(Rules::default()).view(Team::Blue)));
        let mut line = String::new();
        for msg in [start, view] {
            writeln!(to_w, "{}", serde_json::to_string(&msg).unwrap()).unwrap();
            line.clear();
            from.read_line(&mut line).unwrap();
        }
        let reply: Vec<Reply> = serde_json::from_str(&line).unwrap();
        assert_eq!(reply, vec![Reply::Queued(0, Order::Hold, vec![Queued::silent(5.0, Order::Home)])]);
    }

    #[test]
    fn a_seat_process_hands_the_queues_of_its_reply_to_the_match() {
        // A shell that answers every line with one order and a queue, and one order with none.
        let reply = r#"[[0,"Hold",[{"when":{"Silent":5.0},"order":"Home"}]],[1,"Hold"]]"#;
        let script = format!("while read l; do echo '{reply}'; done");
        let mut seat = SeatProcess::spawn("sh", &["-c", &script], std::path::Path::new(".")).unwrap();
        let view = Game::new(Rules::default()).view(Team::Blue);
        let ds = Directing(&mut seat).decide(&view).unwrap();
        assert_eq!((ds[0].order, ds[1].order), (Order::Hold, Order::Hold));
        assert_eq!(ds[0].queue, vec![Queued::silent(5.0, Order::Home)]);
        assert!(ds[1].queue.is_empty());
    }
}
