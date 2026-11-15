//! What the lookahead does to a game, and whether the model or the taking is at fault
//! (plans/actors.md, step 4):
//!
//! `cargo run --release -p cognition_towers --example lookahead -- calibrate [SEEDS]`
//!   every proposal the lookahead makes in a game of Blue (advised) against Red (no advisor), and
//!   every one the commander takes, with what the model promised and what the real arena game
//!   gave over the next 45 s: the same game replayed from the seed, once with the proposal taken
//!   and once without it, and nothing else proposed in between.
//!
//! `cargo run --release -p cognition_towers --example lookahead -- variants [SEEDS]`
//!   each variant of the advisor or of the taking against a commander with no advisor, on paired
//!   seeds, both sides.
//!
//! Nothing here is in the commander's hot path: the probes are advisors and captains of their own.

use cognition_towers::{Captain, Match, TowersResult};
use glam::Vec2;
use recreation_towers::{
    Advise, Commander, EnemyDrone, Hand, Lookahead, Model, Order, Outcome, Phase, Proposal, Rules, Team, TeamView,
};
use std::sync::{Arc, Mutex};

const HORIZON: f32 = 45.0;

/// What the model said, when.
#[derive(Clone, Copy, Debug)]
struct Made {
    time: f32,
    drone: usize,
    tower: usize,
    gain: f32,
}

/// What the commander did with it.
#[derive(Clone, Copy, Debug)]
struct Taken {
    made: Made,
    /// What the drone was told to do before.
    before: Before,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Before {
    /// Nothing.
    Idle,
    /// On its way to a tower, not in its circle yet.
    Flying,
    /// In the circle of the tower it was told to take, which the team does not hold yet.
    Capturing,
    /// In the circle of the tower it was told to take, which the team holds (a guard).
    Guarding,
}

/// What to do with the lookahead's proposals in a replay.
#[derive(Clone, Copy, Debug, Default)]
struct Plan {
    /// No proposal made in this window (time after, time up to and including) is handed on.
    silence: Option<(f32, f32)>,
    /// The proposal made at this time is not handed on.
    veto: Option<f32>,
}

/// The lookahead, watched: it logs what it proposes and hands on what the plan lets through.
#[derive(Clone, Debug)]
struct Probe {
    inner: Lookahead,
    log: Arc<Mutex<Vec<Made>>>,
    plan: Plan,
}

impl Advise for Probe {
    fn advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal> {
        let proposals = self.inner.advise(view, hand);
        for p in &proposals {
            if let Order::Take(tower) = p.order {
                self.log.lock().unwrap().push(Made { time: view.time, drone: p.drone, tower, gain: p.gain });
            }
        }
        let t = view.time;
        let silent = self.plan.silence.is_some_and(|(from, to)| t > from && t <= to);
        let vetoed = self.plan.veto.is_some_and(|v| (t - v).abs() < 1e-3);
        if silent || vetoed { vec![] } else { proposals }
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}

/// A commander that notes which proposals it took.
struct Spy {
    commander: Commander,
    log: Arc<Mutex<Vec<Made>>>,
    taken: Arc<Mutex<Vec<Taken>>>,
    seen: usize,
    circle: f32,
}

impl Captain for Spy {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        let out = self.commander.decide(view);
        let log = self.log.lock().unwrap();
        for &made in &log[self.seen..] {
            let order = Order::Take(made.tower);
            let before = self.commander.hand().and_then(|h| h.orders[made.drone]);
            if self.commander.order(made.drone) == Some(order) && before != Some(order) {
                let how = match before {
                    Some(Order::Take(t)) => {
                        let near = view.drones[made.drone].at.distance(view.towers[t].at) <= self.circle;
                        match (near, view.towers[t].holder == Some(view.team)) {
                            (false, _) => Before::Flying,
                            (true, false) => Before::Capturing,
                            (true, true) => Before::Guarding,
                        }
                    }
                    _ => Before::Idle,
                };
                self.taken.lock().unwrap().push(Taken { made, before: how });
            }
        }
        self.seen = log.len();
        Ok(out)
    }

    fn queue(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Vec<recreation_towers::Queued>)> {
        Captain::queue(&mut self.commander, view, orders)
    }
}

fn plain(rules: Rules) -> Commander {
    Commander::new(rules).with_lookahead(false)
}

/// What a team has to its name at a moment: the points it is ahead, and that with the bars of the
/// towers counted as the model counts them (10 a point of a tower's score).
#[derive(Clone, Copy, Debug, Default)]
struct Standing {
    margin: f32,
    value: f32,
}

fn standing(m: &Match<'_>, team: Team, rules: &Rules) -> Standing {
    let g = &m.game;
    let margin = g.score[team.index()] - g.score[team.other().index()];
    let bars: f32 = g.towers.iter().map(|t| t.control * team.sign() * rules.points(t.kind)).sum();
    Standing { margin, value: margin + 10.0 * bars }
}

/// One replay of a game of `side` (advised by the lookahead under `plan`) against a plain commander,
/// until `stop` (s) or the end of the game.
struct Replay {
    made: Vec<Made>,
    taken: Vec<Taken>,
    at_stop: Standing,
    result: Option<TowersResult>,
}

fn replay(rules: Rules, model: Model, seed: u64, side: Team, plan: Plan, stop: f32) -> Replay {
    let log = Arc::new(Mutex::new(vec![]));
    let taken = Arc::new(Mutex::new(vec![]));
    let probe = Probe { inner: Lookahead::new(rules).with_model(model), log: log.clone(), plan };
    let mut spy = Spy {
        commander: plain(rules).with_advisor(probe, 0.0),
        log: log.clone(),
        taken: taken.clone(),
        seen: 0,
        circle: rules.circle_radius,
    };
    let mut other = plain(rules);
    let mut m = Match::with_rules(rules, seed);
    while m.result().is_none() && m.game.time < stop {
        let _ = if side == Team::Blue { m.step(&mut spy, &mut other) } else { m.step(&mut other, &mut spy) };
    }
    let (made, taken) = (log.lock().unwrap().clone(), taken.lock().unwrap().clone());
    Replay { made, taken, at_stop: standing(&m, side, &rules), result: m.result() }
}

/// One bucket of the calibration: what the model promised and what the games gave.
#[derive(Clone, Copy, Default)]
struct Bucket {
    n: usize,
    promised: f32,
    margin: f32,
    value: f32,
    margin_sq: f32,
}

impl Bucket {
    fn add(&mut self, promised: f32, margin: f32, value: f32) {
        self.n += 1;
        self.promised += promised;
        self.margin += margin;
        self.value += value;
        self.margin_sq += margin * margin;
    }

    fn row(&self, name: &str) -> String {
        let n = self.n.max(1) as f32;
        let sd = (self.margin_sq / n - (self.margin / n).powi(2)).max(0.0).sqrt();
        format!(
            "| {name} | {} | {:+.1} | {:+.1} ± {:.1} | {:+.1} |",
            self.n,
            self.promised / n,
            self.margin / n,
            sd / n.sqrt(),
            self.value / n
        )
    }
}

fn calibrate(seeds: &[u64], model: Model) {
    let rules = Rules::default();
    let header = "| | taken | promised | realised points | realised value |\n|---|---|---|---|---|";
    let (mut games, mut made, mut taken, mut won, mut ended) = (0, 0, 0, 0, 0);
    let (mut by_gain, mut by_before) = (vec![Bucket::default(); 6], vec![Bucket::default(); 4]);
    let mut all = Bucket::default();
    let mut by_kind: Vec<(String, Bucket)> = vec![];
    let edges = [2.0, 5.0, 10.0, 20.0, 40.0, f32::INFINITY];
    for &seed in seeds {
        for side in Team::BOTH {
            let real = replay(rules, model, seed, side, Plan::default(), f32::INFINITY);
            games += 1;
            made += real.made.len();
            taken += real.taken.len();
            ended += 1;
            if let Some(r) = &real.result
                && matches!(r.outcome, Outcome::Won(t) if t == side)
            {
                won += 1;
            }
            for t in &real.taken {
                let when = t.made.time;
                let until = when + HORIZON;
                let with = replay(rules, model, seed, side, Plan { silence: Some((when, until)), veto: None }, until);
                let without =
                    replay(rules, model, seed, side, Plan { silence: Some((when, until)), veto: Some(when) }, until);
                let (dm, dv) =
                    (with.at_stop.margin - without.at_stop.margin, with.at_stop.value - without.at_stop.value);
                let g = t.made.gain;
                let bucket = edges.iter().position(|&e| g <= e).unwrap_or(5);
                by_gain[bucket].add(g, dm, dv);
                let b = match t.before {
                    Before::Idle => 0,
                    Before::Flying => 1,
                    Before::Capturing => 2,
                    Before::Guarding => 3,
                };
                by_before[b].add(g, dm, dv);
                all.add(g, dm, dv);
                let kind = format!("drone {}", t.made.drone + 1);
                match by_kind.iter_mut().find(|(k, _)| *k == kind) {
                    Some((_, b)) => b.add(g, dm, dv),
                    None => {
                        let mut b = Bucket::default();
                        b.add(g, dm, dv);
                        by_kind.push((kind, b));
                    }
                }
            }
        }
    }
    println!("calibrate: {} seeds, {games} games of the advised side (both sides), {HORIZON} s horizon", seeds.len());
    println!(
        "per game: {:.1} proposals made, {:.1} taken; the advised side won {won} of {ended}\n",
        made as f32 / games as f32,
        taken as f32 / games as f32
    );
    println!("by the gain promised\n{header}");
    let names = ["2 to 5", "5 to 10", "10 to 20", "20 to 40", "over 40"];
    println!("{}", by_gain[0].row("up to 2 (not taken in general)"));
    for (b, n) in by_gain[1..].iter().zip(names) {
        println!("{}", b.row(n));
    }
    println!("\nby what the drone was doing\n{header}");
    for (b, n) in by_before.iter().zip(["no order", "flying to a tower", "capturing a tower", "guarding a held tower"])
    {
        println!("{}", b.row(n));
    }
    println!("\nby drone\n{header}");
    by_kind.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, b) in &by_kind {
        println!("{}", b.row(k));
    }
    println!("\nall\n{header}\n{}", all.row("all taken"));
}

// ---------------------------------------------------------------- variants

/// Which drones the lookahead may move.
#[derive(Clone, Copy, Debug, Default)]
struct Only {
    /// Only drones with no order.
    idle: bool,
    /// Not a drone that is capturing a tower.
    not_capturing: bool,
}

/// The lookahead, with some drones taken out of its hand.
#[derive(Clone, Debug)]
struct Restrict {
    inner: Lookahead,
    only: Only,
    circle: f32,
}

impl Advise for Restrict {
    fn advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal> {
        let mut hand = hand.clone();
        for k in 0..hand.free.len() {
            let order = hand.orders[k];
            if self.only.idle && order.is_some() {
                hand.free[k] = false;
            }
            if self.only.not_capturing
                && let Some(Order::Take(t)) = order
                && view.drones[k].at.distance(view.towers[t].at) <= self.circle
                && view.towers[t].holder != Some(view.team)
            {
                hand.free[k] = false;
            }
        }
        self.inner.advise(view, &hand)
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}

/// The lookahead, shown where the enemy really is.
#[derive(Clone, Debug)]
struct Oracle {
    inner: Lookahead,
    truth: Arc<Mutex<[Vec<EnemyDrone>; 2]>>,
}

impl Advise for Oracle {
    fn advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal> {
        let mut view = view.clone();
        view.enemies = self.truth.lock().unwrap()[view.team.index()].clone();
        self.inner.advise(&view, hand)
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}

type Truth = Arc<Mutex<[Vec<EnemyDrone>; 2]>>;

fn publish(m: &Match<'_>, truth: &Truth) {
    let g = &m.game;
    let n = g.rules.drones;
    let mut t = truth.lock().unwrap();
    for team in Team::BOTH {
        t[team.index()] = (0..n)
            .map(|k| (k, &g.drones[g.drone(team.other(), k)]))
            .filter(|(_, d)| d.phase != Phase::Down)
            .map(|(k, d)| EnemyDrone {
                k,
                at: d.at,
                alt: d.alt,
                vel: Vec2::new(d.vel.x, d.vel.y),
                seen: true,
                age: 0.0,
            })
            .collect();
    }
}

fn play(rules: Rules, seed: u64, blue: &mut dyn Captain, red: &mut dyn Captain, truth: &Truth) -> TowersResult {
    let mut m = Match::with_rules(rules, seed);
    loop {
        if let Some(r) = m.result() {
            return r;
        }
        publish(&m, truth);
        m.step(blue, red).unwrap();
    }
}

struct Row {
    margin: f32,
    error: f32,
    won: usize,
    lost: usize,
    games: usize,
}

/// `advised` against a commander with no advisor, on each seed on both sides.
fn compare(rules: Rules, seeds: &[u64], advised: &dyn Fn(&Truth) -> Commander) -> Row {
    let truth: Truth = Arc::new(Mutex::new([vec![], vec![]]));
    let mut scores = vec![];
    let (mut won, mut lost) = (0, 0);
    for &seed in seeds {
        for side in Team::BOTH {
            let (mut a, mut b) = (advised(&truth), plain(rules));
            let r = if side == Team::Blue {
                play(rules, seed, &mut a, &mut b, &truth)
            } else {
                play(rules, seed, &mut b, &mut a, &truth)
            };
            scores.push(r.score_for(side, rules.win_score));
            match r.outcome {
                Outcome::Won(t) if t == side => won += 1,
                Outcome::Won(_) => lost += 1,
                Outcome::Draw => {}
            }
        }
    }
    let n = scores.len() as f32;
    let margin = scores.iter().sum::<f32>() / n;
    let var = scores.iter().map(|s| (s - margin).powi(2)).sum::<f32>() / (n - 1.0);
    Row { margin, error: (var / n).sqrt(), won, lost, games: scores.len() }
}

fn model_named(spec: &str) -> Model {
    let mut tokens = spec.split('+').filter(|t| !t.is_empty()).peekable();
    let mut m = match tokens.peek() {
        Some(&("first" | "flight" | "drift" | "charging" | "redeploy")) => Model::first(),
        _ => Model::default(),
    };
    for t in tokens {
        match t {
            "first" => m = Model::first(),
            "all" => m = Model::default(),
            "flight" => m.flight = true,
            "drift" => m.drift = true,
            "charging" => m.charging = true,
            "redeploy" => m.redeploy = true,
            "spare" => m.spare_capturing = true,
            "no-flight" => m.flight = false,
            "no-drift" => m.drift = false,
            "no-charging" => m.charging = false,
            "no-redeploy" => m.redeploy = false,
            s if s.starts_with('h') => m.horizon = s[1..].parse().unwrap_or(45.0),
            s if s.starts_with('f') => m.futures = s[1..].parse().unwrap_or(8),
            other => eprintln!("unknown model token {other}"),
        }
    }
    m
}

fn variants(seeds: &[u64], set: &str, model: Model) {
    let rules = Rules::default();
    println!(
        "variants: the commander with an advisor against the commander with none, {} seeds, both sides\n",
        seeds.len()
    );
    println!("| variant | margin | won | lost |\n|---|---|---|---|");
    let show = |name: &str, advised: &dyn Fn(&Truth) -> Commander| {
        let r = compare(rules, seeds, advised);
        println!("| {name} | {:+.2} ± {:.2} | {} | {} |", r.margin, r.error, r.won, r.lost);
        let _ = r.games;
    };
    let look = || Lookahead::new(rules).with_model(model);
    if set == "models" {
        for name in std::env::var("MODELS").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
            let m = model_named(name);
            show(name, &|_| plain(rules).with_advisor(Lookahead::new(rules).with_model(m), 0.0));
        }
        return;
    }
    show("lookahead, margin 2 (as it is)", &|_| plain(rules).with_advisor(look(), 0.0));
    for m in [5.0, 10.0, 20.0, 40.0] {
        show(&format!("margin {m}"), &|_| plain(rules).with_advisor(look(), 0.0).with_advice_margin(m));
    }
    let restrict = |only: Only| {
        move |_: &Truth| plain(rules).with_advisor(Restrict { inner: look(), only, circle: rules.circle_radius }, 0.0)
    };
    show("only drones with no order", &restrict(Only { idle: true, not_capturing: false }));
    show("not a drone that is capturing", &|_| {
        plain(rules).with_advisor(look().with_model(Model { spare_capturing: true, ..model }), 0.0)
    });
    for c in [10.0, 20.0, 40.0] {
        show(&format!("cooldown {c} s per drone"), &|_| plain(rules).with_advisor(look().with_cooldown(c), 0.0));
    }
    show("not capturing, margin 10", &|_| {
        plain(rules)
            .with_advisor(look().with_model(Model { spare_capturing: true, ..model }), 0.0)
            .with_advice_margin(10.0)
    });
    show("blind (the belief never updated)", &|_| plain(rules).with_advisor(look().blind(true), 0.0));
    show("oracle (enemy drones where they are)", &|truth| {
        plain(rules).with_advisor(Oracle { inner: look(), truth: truth.clone() }, 0.0)
    });
    show("no advisor (should be level)", &|_| plain(rules));
}

fn main() {
    let mut args = std::env::args().skip(1);
    let what = args.next().unwrap_or_else(|| "calibrate".into());
    let n: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(40);
    let seeds: Vec<u64> = (1..=n).collect();
    match what.as_str() {
        "calibrate" => calibrate(&seeds, model_named(&args.next().unwrap_or_default())),
        "variants" => {
            let set = args.next().unwrap_or_else(|| "taking".into());
            variants(&seeds, &set, model_named(&args.next().unwrap_or_default()))
        }
        other => eprintln!("calibrate or variants, not {other}"),
    }
}
