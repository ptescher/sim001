//! `cognition_league`: the league that keeps making the executor, and Towers' captains, better
//! (plans/league.md). `--game executor` (the default) or `--game captain` picks the contest.
//!
//! ```text
//! play   [--candidate X] [--opponent Y] [--seeds N | --seed-list a,b,c] [--json]
//! judge  --champion X --challenger Y [--seeds N] [--opponent-bin PROGRAM]
//! run    [--generations N] [--strategies es,claude-tuner,claude-engineer] [--dir DIR] [--seeds N]
//! start  --champion X [--dir DIR]   (a new league: what was in DIR goes)
//! status [--dir DIR]
//! captain-seat                     (a captain on stdin and stdout, for the league to play)
//! ```
//!
//! X and Y are `default`, a contest's own (`slow`, `timid`) or a JSON file of an entry. The
//! league's folder is `league` for the executor and `league/captain` for captains.

use cognition_league::contest::{self, Code, Contest};
use cognition_league::strategy::{Claude, Es};
use cognition_league::{Entry, League, Options, Strategy, judge};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        std::process::exit(2)
    };
    if args.first().map(String::as_str) == Some("captain-seat") {
        let stdin = std::io::stdin();
        if let Err(e) = cognition_towers::serve(stdin.lock(), std::io::stdout().lock()) {
            fail(e);
        }
        return;
    }
    let game = arg("--game").unwrap_or("executor".into());
    let contest: Box<dyn Contest> =
        contest::by_name(&game).unwrap_or_else(|| fail(format!("no game called {game} (executor, captain)")));
    let seeds: Vec<u64> = match arg("--seed-list") {
        Some(list) => list.split(',').filter_map(|s| s.trim().parse().ok()).collect(),
        None => (0..arg("--seeds").and_then(|s| s.parse().ok()).unwrap_or(Options::default().seeds as u64)).collect(),
    };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repo root");
    let dir = arg("--dir").map(PathBuf::from).unwrap_or_else(|| repo.join(contest.dir()));
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| repo.join("target/league"));
    let load = |name: &str| Entry::load_for(&*contest, name).unwrap_or_else(|e| fail(e));

    match args.first().map(String::as_str) {
        Some("play") => {
            let entry = load(&arg("--candidate").unwrap_or("default".into()));
            let opponent = load(&arg("--opponent").unwrap_or("default".into()));
            let t = Instant::now();
            let games = contest
                .play((&entry, &Code::Here), (&opponent, &Code::Here), &seeds, &target)
                .unwrap_or_else(|e| fail(e));
            if flag("--json") {
                println!("{}", serde_json::to_string(&games).unwrap());
                return;
            }
            for g in &games {
                println!("{}", contest.line(g));
                for c in &g.crash_log {
                    println!("      crash at {c}");
                }
            }
            let secs = t.elapsed().as_secs_f32();
            let played: f32 = games.iter().map(|g| g.length).sum();
            let mean = games.iter().map(|g| g.score).sum::<f32>() / games.len().max(1) as f32;
            println!(
                "{} ({}): {} games, mean score {mean:+.3}; {played:.0} s of games in {secs:.1} s, {:.0}x real time",
                entry.name,
                entry.hash(),
                games.len(),
                played / secs.max(1e-3)
            );
        }
        Some("judge") => {
            let champion = load(&arg("--champion").unwrap_or("default".into()));
            let challenger = load(&arg("--challenger").unwrap_or("default".into()));
            // The champion's code: this build's, or another program's (a copy of an older build).
            let champion_code = arg("--opponent-bin").map_or(Code::Here, |b| Code::Bin(PathBuf::from(b)));
            let play = |e: &Entry, code: &Code| {
                contest.play((e, code), (&champion, &champion_code), &seeds, &target).unwrap_or_else(|e| fail(e))
            };
            let j = judge(&play(&champion, &champion_code), &play(&challenger, &Code::Here), 1);
            let reason = j.reason.map(|r| format!(": {}", r.label())).unwrap_or_default();
            println!("{}{reason} (mean difference {:+.3}, lower bound {:+.3})", j.verdict.label(), j.mean, j.lower);
            for (who, s) in [("champion", &j.champion), ("challenger", &j.challenger)] {
                println!("  {who:<10} {}", contest.describe(s));
            }
        }
        Some("run") => {
            let gens: u32 = arg("--generations").and_then(|s| s.parse().ok()).unwrap_or(1);
            let names = arg("--strategies").unwrap_or("es".into());
            let mut strategies: Vec<Box<dyn Strategy>> = names
                .split(',')
                .map(|n| -> Box<dyn Strategy> {
                    match n.trim() {
                        "es" => Box::new(Es::new(fastrand::u64(..))),
                        "claude-tuner" => Box::new(Claude::tuner()),
                        "claude-engineer" => Box::new(Claude::engineer()),
                        other => fail(format!("no strategy called {other} (es, claude-tuner, claude-engineer)")),
                    }
                })
                .collect();
            let mut league = League::open(contest, &dir, &repo);
            let opts = Options { seeds: seeds.len() };
            for _ in 0..gens {
                let g = league.generation(&mut strategies, &opts);
                println!("{}", g.summary());
            }
        }
        Some("start") => {
            let entry = load(&arg("--champion").unwrap_or("default".into()));
            let league = League::start(contest, &dir, &repo, entry);
            println!("{}", league.status());
        }
        Some("status") => {
            let league = League::open(contest, &dir, &repo);
            println!("{}", league.status());
        }
        _ => fail("usage: cognition_league play|judge|run|start|status|captain-seat (see the source's header)".into()),
    }
}
