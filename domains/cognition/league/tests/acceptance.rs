//! The "Making the executor better" section of the suite (expectations/08-league): the real fast
//! arenas (tag and Towers) and the real league, each test in a folder of its own. The Claude strategies run a
//! stand-in for `claude` (`tests/fakes/`), which changes the code in its worktree as an engineer
//! would; what the league does with the change (the check of the files, the tests) is real.

use cognition_arena::{Candidate, GameResult, play, play_many};
use cognition_league::contest::captain::CaptainContest;
use cognition_league::contest::executor::ExecutorContest;
use cognition_league::contest::{Code, Contest};
use cognition_league::strategy::{Claude, Es};
use cognition_league::{Entry, Generation, Judgement, League, Options, Strategy, judge};
use expectation_harness::{Acceptance, Bindings, Progress, RunOptions, Screenshot};
use expectation_story::Status;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// The games the champion is scored on against the slow executor, at the end of a run: new ones,
/// and enough that the measure is not noise (about ±0.03).
const SCORED: usize = 512;

struct World {
    repo: PathBuf,
    dir: PathBuf,
    champion: Entry,
    challenger: Entry,
    /// The challenger's games are marked with a crash before they are judged.
    crash: bool,
    games: Vec<GameResult>,
    same: Option<bool>,
    speed: f64,
    judgement: Option<Judgement>,
    last: Option<Generation>,
    /// The champion's mean score less the slow executor's, on the same seeds.
    against_slow: f64,
    /// Towers: the champion and challenger captains, the last captain game and the default
    /// captain's score against itself.
    captain_champion: Entry,
    captain_challenger: Entry,
    towers: Option<cognition_towers::TowersResult>,
    against_itself: f64,
}

static FOLDERS: AtomicU32 = AtomicU32::new(0);

impl World {
    fn new() -> World {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap();
        let n = FOLDERS.fetch_add(1, Ordering::Relaxed);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("league-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        World {
            repo,
            dir,
            champion: Entry::load("default").unwrap(),
            challenger: Entry::load("default").unwrap(),
            crash: false,
            games: vec![],
            same: None,
            speed: 0.0,
            judgement: None,
            last: None,
            against_slow: 0.0,
            captain_champion: CaptainContest.fixture("default").unwrap(),
            captain_challenger: CaptainContest.fixture("default").unwrap(),
            towers: None,
            against_itself: f64::NAN,
        }
    }

    fn league(&self) -> League {
        League::open(Box::new(ExecutorContest), &self.dir, &self.repo)
    }

    /// The captains' league, in a folder of its own beside the executor's.
    fn captain_league(&self) -> League {
        League::open(Box::new(CaptainContest), &self.dir.join("captain"), &self.repo)
    }

    /// A Claude engineer for captains, with a stand-in for `claude`.
    fn claude_captain(&mut self, fake: &str) {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes").join(fake);
        let mut s: Vec<Box<dyn Strategy>> = vec![Box::new(Claude::engineer().with_command(script))];
        let mut league = self.captain_league();
        self.last = Some(league.generation(&mut s, &Options::default()));
    }

    fn captain_games(&self, a: &Entry, b: &Entry) -> Vec<GameResult> {
        let seeds: Vec<u64> = (0..Options::default().seeds as u64).collect();
        let target = self.repo.join("target/league");
        CaptainContest.play((a, &Code::Here), (b, &Code::Here), &seeds, &target).unwrap()
    }

    fn generations(&mut self, n: usize, strategies: &mut [Box<dyn Strategy>]) {
        let mut league = self.league();
        for _ in 0..n {
            self.last = Some(league.generation(strategies, &Options::default()));
        }
        let seeds: Vec<u64> = (9_000_000..9_000_000 + SCORED as u64).collect();
        let mean = |c: &Candidate| play_many(c, &seeds).iter().map(|g| g.score as f64).sum::<f64>() / SCORED as f64;
        self.against_slow = mean(&league.champion.candidate().unwrap()) - mean(&Candidate::slow());
    }

    fn claude(&mut self, fake: &str) {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes").join(fake);
        let mut s: Vec<Box<dyn Strategy>> = vec![Box::new(Claude::engineer().with_command(script))];
        self.generations(1, &mut s);
    }

    /// The verdict of the last judgement: the one asked for, or the one challenger of the last
    /// generation.
    fn judged(&self) -> Option<&Judgement> {
        self.judgement.as_ref().or_else(|| self.last.as_ref().and_then(|g| g.judged.first().map(|(_, j)| j)))
    }
}

impl World {
    /// What the league has to say now, one line each: the games played, the judgement, the
    /// generation and the champion, as `cognition_league` reports them.
    fn report(&self) -> Vec<String> {
        let mut out = vec![format!("champion: {}   challenger: {}", self.champion.name, self.challenger.name)];
        if let [g] = self.games.as_slice() {
            out.push(format!(
                "game {}: score {:+.3}, {:.0} s, {} players, by plan {:.0} s, {} crashes",
                g.seed, g.score, g.length, g.players, g.by_plan, g.crashes
            ));
        } else if !self.games.is_empty() {
            let mean = self.games.iter().map(|g| g.score as f64).sum::<f64>() / self.games.len() as f64;
            out.push(format!("{} games: mean score {mean:+.3}, {:.0} times real time", self.games.len(), self.speed));
        }
        if let Some(same) = self.same {
            out.push(format!("the same seed twice: {}", if same { "the same game" } else { "different games" }));
        }
        if let Some(j) = self.judged() {
            out.push(format!(
                "{}{}: mean difference {:+.3} (lower bound {:+.3}) over {} games",
                j.verdict.label(),
                j.reason.map_or(String::new(), |r| format!(" ({})", r.label())),
                j.mean,
                j.lower,
                j.challenger.games
            ));
        }
        if let Some(g) = &self.last {
            out.push(g.summary());
        }
        if self.against_slow != 0.0 {
            out.push(format!("champion against the slow executor: {:+.3}", self.against_slow));
        }
        if let Some(t) = &self.towers {
            out.push(format!(
                "Towers, seed {}: {:?}, Blue {:.0} Red {:.0} after {:.0} s, {} drones flew",
                t.seed, t.outcome, t.score[0], t.score[1], t.length, t.flew
            ));
        }
        if self.against_itself.is_finite() {
            out.push(format!("the default captain against itself: {:.3}", self.against_itself));
        }
        out
    }

    /// The report as a screen of the console's theme.
    fn svg(&self) -> String {
        let lines = self.report();
        let h = 44 + 18 * lines.len();
        let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let mut s = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 720 {h}" width="720" height="{h}" font-family="Menlo, ui-monospace, monospace" font-size="12"><rect width="720" height="{h}" fill="#050806"/><text x="14" y="22" fill="#919c91">COGNITION_LEAGUE</text>"##
        );
        for (i, l) in lines.iter().enumerate() {
            let colour = if l.starts_with("promoted") {
                "#4cff7a"
            } else if l.starts_with("refused") {
                "#ff5a4f"
            } else {
                "#dfe8df"
            };
            s += &format!(r##"<text x="14" y="{}" fill="{colour}">{}</text>"##, 46 + 18 * i, esc(l));
        }
        s + "</svg>"
    }
}

fn entry(v: &expectation_model::Value) -> Entry {
    match v.as_str() {
        Some("the slow executor") => Entry::load("slow").unwrap(),
        _ => Entry::load("default").unwrap(),
    }
}

fn captain(v: &expectation_model::Value) -> Entry {
    let name = if v.as_str() == Some("the timid captain") { "timid" } else { "default" };
    CaptainContest.fixture(name).unwrap()
}

fn captain_name(e: &Entry) -> &'static str {
    if e.name == "timid" { "the timid captain" } else { "the default captain" }
}

fn bindings() -> Bindings<World> {
    Bindings::new(World::new())
        .arrange("the champion", |w, v| w.champion = entry(v))
        .observe(
            "the champion",
            |w| if w.champion.name == "slow" { "the slow executor" } else { "the default executor" },
        )
        .arrange("the challenger", |w, v| w.challenger = entry(v))
        .observe("the challenger", |w| {
            if w.challenger.name == "slow" { "the slow executor" } else { "the default executor" }
        })
        .arrange("the league", |w, v| {
            let _ = std::fs::remove_dir_all(&w.dir);
            if v.as_str() == Some("started from the slow executor") {
                League::start(Box::new(ExecutorContest), &w.dir, &w.repo, Entry::load("slow").unwrap());
            }
        })
        .observe("the league", |w| {
            let slow = w.league().champion.name == "slow";
            if w.dir.join("champion.json").exists() && slow { "started from the slow executor" } else { "new" }
        })
        // A game, and many.
        .observe("the length of the game", |w| w.games.first().map_or(0.0, |g| g.length as f64))
        .observe("the players in the game", |w| w.games.first().map_or(0.0, |g| g.players as f64))
        .observe("the time the candidate flew by plan", |w| w.games.first().map_or(0.0, |g| g.by_plan as f64))
        .observe("the crashes of the candidate", |w| {
            if w.crash { 1.0 } else { w.games.iter().map(|g| g.crashes as f64).sum() }
        })
        .arrange("the crashes of the candidate", |w, v| w.crash = v.as_f64().unwrap_or(0.0) > 0.0)
        .observe("the two games", |w| match w.same {
            Some(true) => "the same",
            _ => "different",
        })
        .observe("the speed of the arena", |w| w.speed)
        .on("the arena plays a game with the default executor", |w| {
            w.games = vec![play(&Candidate::default_executor(), 1)]
        })
        .on("the arena plays the same seed twice", |w| {
            let (a, b) = (play(&Candidate::default_executor(), 1), play(&Candidate::default_executor(), 1));
            w.same = Some(a == b && a.score.to_bits() == b.score.to_bits());
            w.games = vec![a];
        })
        .on("the arena plays 64 games with the default executor", |w| {
            let seeds: Vec<u64> = (0..64).collect();
            let t = Instant::now();
            w.games = play_many(&Candidate::default_executor(), &seeds);
            let played: f64 = w.games.iter().map(|g| g.length as f64).sum();
            w.speed = played / t.elapsed().as_secs_f64();
        })
        // Judgement.
        .on("the league judges the challenger", |w| {
            let seeds: Vec<u64> = (0..Options::default().seeds as u64).collect();
            let champion = play_many(&w.champion.candidate().unwrap(), &seeds);
            let mut challenger = play_many(&w.challenger.candidate().unwrap(), &seeds);
            if w.crash {
                challenger[0].crashes += 1;
            }
            w.judgement = Some(judge(&champion, &challenger, 1));
        })
        .observe("the verdict", |w| w.judged().map_or("none", |j| j.verdict.label()))
        .observe("the reason", |w| match w.judged().and_then(|j| j.reason).map(|r| r.label()) {
            Some("crashed") => "it crashed",
            Some("tests fail") => "its tests fail",
            Some("envelope changed") => "it changes the envelope",
            Some("rules changed") => "it changes the rules",
            Some(other) => other,
            None => "none",
        })
        // The loop.
        .on("the league runs one generation with the evolution strategy", |w| {
            w.generations(1, &mut [Box::new(Es::new(1)) as Box<dyn Strategy>])
        })
        .on("the league runs 5 generations with the evolution strategy", |w| {
            w.generations(5, &mut [Box::new(Es::new(1)) as Box<dyn Strategy>])
        })
        .on("a Claude strategy gives a challenger whose tests fail", |w| w.claude("breaks-tests.sh"))
        .on("a Claude strategy gives a challenger that changes the envelope", |w| w.claude("changes-envelope.sh"))
        .observe("the challengers in the history", |w| {
            std::fs::read_to_string(w.dir.join("history.jsonl")).map_or(0.0, |s| s.lines().count() as f64)
        })
        .observe("the report of the generation", |w| {
            if w.dir.join("generations/001.md").exists() { "written" } else { "missing" }
        })
        .observe("the champion file", |w| if w.dir.join("champion.json").exists() { "written" } else { "missing" })
        .observe("the score of the champion against the slow executor", |w| w.against_slow)
        // Captains for Towers.
        .arrange("the champion captain", |w, v| w.captain_champion = captain(v))
        .observe("the champion captain", |w| captain_name(&w.captain_champion))
        .arrange("the challenger captain", |w, v| w.captain_challenger = captain(v))
        .observe("the challenger captain", |w| captain_name(&w.captain_challenger))
        .on("the arena plays a game of Towers between two default captains", |w| {
            let c = || recreation_towers::Commander::new(recreation_towers::Rules::default());
            w.towers = Some(cognition_towers::play(&mut c(), &mut c(), 1).unwrap());
        })
        .observe("the captain game", |w| match w.towers.as_ref().map(|r| r.outcome) {
            Some(recreation_towers::Outcome::Won(recreation_towers::Team::Blue)) => "won by Blue",
            Some(recreation_towers::Outcome::Won(recreation_towers::Team::Red)) => "won by Red",
            Some(recreation_towers::Outcome::Draw) => "a draw",
            None => "not over",
        })
        .observe("the drones that flew in the captain game", |w| w.towers.as_ref().map_or(0.0, |r| r.flew as f64))
        .on("the default captain plays itself from both sides", |w| {
            let d = CaptainContest.fixture("default").unwrap();
            let games = w.captain_games(&d, &d);
            w.against_itself = games.iter().map(|g| g.score.abs() as f64).fold(0.0, f64::max);
        })
        .observe("the score of the captain against itself", |w| w.against_itself)
        .on("the league judges the captain challenger", |w| {
            let champion = w.captain_games(&w.captain_champion, &w.captain_champion);
            let challenger = w.captain_games(&w.captain_challenger, &w.captain_champion);
            w.judgement = Some(judge(&champion, &challenger, 1));
        })
        .on("a Claude strategy gives a captain whose code sends every drone home", |w| {
            w.claude_captain("captain-goes-home.sh")
        })
        .on("a Claude strategy gives a captain that changes the referee", |w| {
            w.claude_captain("captain-changes-rules.sh")
        })
        .observe("the mean difference of the captain challenger", |w| w.judged().map_or(f64::NAN, |j| j.mean as f64))
        .on_reset(|w| *w = World::new())
        .on_advance(|_| Progress::Static)
        .on_screenshot(|w| Screenshot::svg(w.svg()))
}

fn suite() -> Acceptance {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage()
}

#[test]
fn every_test_of_the_section_passes() {
    let a = suite();
    let section = a.suite.sections.iter().find(|s| s.slug == "league").expect("a league section").id;
    let ours = a.suite.subtree(section);
    // With SIM001_DOCS=DIR the results and pictures of the league's report go into the project's docs.
    let docs = std::env::var("SIM001_DOCS").ok().map(PathBuf::from);
    let opts = RunOptions { docs_dir: docs.clone(), ..Default::default() };
    let mut driver = bindings();
    let suite_time = suite_budget::Budget::from_env();
    let report = expectation_story::Report {
        results: a
            .plans
            .iter()
            .map(|plan| {
                if suite_time.spent() {
                    suite_time.ran_out(&a.suite, plan, &driver)
                } else {
                    expectation_harness::run(&a.suite, std::slice::from_ref(plan), &mut driver, &opts).results.remove(0)
                }
            })
            .collect(),
    };
    if let Some(dir) = &docs {
        a.publish(dir, "league", &report).unwrap();
    }
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {} {:?}", r.status, p.title, r.start.error);
        for (step, sr) in p.steps.iter().zip(&r.steps) {
            for (c, cr) in step.checks.iter().zip(&sr.checks) {
                let var = a.suite.vocabulary.var(c.var);
                println!("    {}: {:?}", var.describe(&c.expect), cr.observed.as_ref().map(|v| v.to_string()));
            }
        }
    }
    let bad: Vec<_> = ran
        .iter()
        .filter(|(_, r)| r.status != Status::Passed)
        .map(|(p, r)| format!("{}: {:?}", p.title, r.status))
        .collect();
    assert!(!ran.is_empty(), "no tests of the section ran");
    assert!(bad.is_empty(), "{}\n{}", bad.join("\n"), report.failures(&a.suite, &a.plans));
}
