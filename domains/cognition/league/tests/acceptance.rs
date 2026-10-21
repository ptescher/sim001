//! The "Making the executor better" section of the suite (expectations/08-league): the real fast
//! arena and the real league, each test in a folder of its own. The Claude strategies run a
//! stand-in for `claude` (`tests/fakes/`), which changes the code in its worktree as an engineer
//! would; what the league does with the change (the check of the files, the tests) is real.

use cognition_arena::{Candidate, GameResult, play, play_many};
use cognition_league::strategy::{Claude, Es};
use cognition_league::{Entry, Generation, Judgement, League, Options, Strategy, judge};
use expectation_harness::{Acceptance, Bindings, Progress, RunOptions};
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
        }
    }

    fn league(&self) -> League {
        League::open(&self.dir, &self.repo)
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

fn entry(v: &expectation_model::Value) -> Entry {
    match v.as_str() {
        Some("the slow executor") => Entry::load("slow").unwrap(),
        _ => Entry::load("default").unwrap(),
    }
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
                League::start(&w.dir, &w.repo, Entry::load("slow").unwrap());
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
        .on_reset(|w| *w = World::new())
        .on_advance(|_| Progress::Static)
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
    let report = a.run(&mut bindings(), &RunOptions::default());
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
