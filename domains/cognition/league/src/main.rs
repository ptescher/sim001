//! `cognition_league`: the league that keeps making the executor better (plans/league.md).
//!
//! ```text
//! play   [--candidate default|slow|FILE] [--seeds N | --seed-list a,b,c] [--json]
//! judge  --champion X --challenger Y [--seeds N]
//! run    [--generations N] [--strategies es,claude-tuner,claude-engineer] [--dir league] [--seeds N]
//! start  --champion default|slow|FILE [--dir league]   (a new league: what was in DIR goes)
//! status [--dir league]
//! ```

use cognition_arena::{GameResult, play_many};
use cognition_league::strategy::{Claude, Es};
use cognition_league::{Entry, League, Options, Strategy, judge};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let seeds: Vec<u64> = match arg("--seed-list") {
        Some(list) => list.split(',').filter_map(|s| s.trim().parse().ok()).collect(),
        None => (0..arg("--seeds")
            .and_then(|s| s.parse().ok())
            .unwrap_or(cognition_league::Options::default().seeds as u64))
            .collect(),
    };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repo root");
    let dir = arg("--dir").map(PathBuf::from).unwrap_or_else(|| repo.join("league"));
    let load = |name: &str| {
        Entry::load(name).and_then(|e| e.candidate().map(|c| (e, c))).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        })
    };

    match args.first().map(String::as_str) {
        Some("play") => {
            let (entry, c) = load(&arg("--candidate").unwrap_or("default".into()));
            let t = Instant::now();
            let games = play_many(&c, &seeds);
            if flag("--json") {
                println!("{}", serde_json::to_string(&games).unwrap());
                return;
            }
            for g in &games {
                println!("{}", line(g));
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
            let (_, a) = load(&arg("--champion").unwrap_or("default".into()));
            let (_, b) = load(&arg("--challenger").unwrap_or("default".into()));
            let j = judge(&play_many(&a, &seeds), &play_many(&b, &seeds), 1);
            let reason = j.reason.map(|r| format!(": {}", r.label())).unwrap_or_default();
            println!("{}{reason} (mean difference {:+.3}, lower bound {:+.3})", j.verdict.label(), j.mean, j.lower);
            for (who, s) in [("champion", &j.champion), ("challenger", &j.challenger)] {
                println!("  {who:<10} {}", cognition_league::judge::describe(s));
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
                        other => {
                            eprintln!("no strategy called {other} (es, claude-tuner, claude-engineer)");
                            std::process::exit(2)
                        }
                    }
                })
                .collect();
            let mut league = League::open(&dir, &repo);
            let opts = Options { seeds: seeds.len() };
            for _ in 0..gens {
                let g = league.generation(&mut strategies, &opts);
                println!("{}", g.summary());
            }
        }
        Some("start") => {
            let (entry, _) = load(&arg("--champion").unwrap_or("default".into()));
            let league = League::start(&dir, &repo, entry);
            println!("{}", league.status());
        }
        Some("status") => {
            let league = League::open(&dir, &repo);
            println!("{}", league.status());
        }
        _ => {
            eprintln!("usage: cognition_league play|judge|run|status (see the source's header)");
            std::process::exit(2);
        }
    }
}

fn line(g: &GameResult) -> String {
    format!(
        "seed {:>4}: score {:+.3}  place {}  it {:>5.1} s  tags {}  tagged {}  exits {} ({} in launch)  crashes {} ({} in launch)  by plan {:>5.1} s  envelope {:>4.1} %  closest chase {:.0} m",
        g.seed,
        g.score,
        g.place,
        g.it_time,
        g.tags,
        g.tagged,
        g.exits,
        g.launch_exits,
        g.crashes,
        g.launch_crashes,
        g.by_plan,
        100.0 * g.interventions as f32 / g.steps.max(1) as f32,
        g.closest_chase
    )
}
