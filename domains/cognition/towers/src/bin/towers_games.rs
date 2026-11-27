//! Plays a batch of Towers games in the arena and keeps a record of each (`plans/game-tooling.md`):
//!
//!   cargo run --release -p cognition_towers --bin towers_games -- [--games N] [--from S]
//!       [--blue SIDE] [--red SIDE] [--rules default|v4] [--swap] [--threads T] [--out DIR]
//!
//! Plays seeds S+1 to S+N (S 0, N 20), Blue's SIDE against Red's, and with `--swap` each seed once
//! more with the sides changed over. A SIDE is `commander` (the commander with the lookahead, the
//! default), `plain` (the commander with no advisor), or `deep` / `deep:DELAY` (the lookahead and the
//! deep thinker, whose intents are DELAY s late, 20 by default). DIR (`towers-games`) gets a record a
//! game, `<seed>.jsonl` (`<seed>-swap.jsonl` swapped), and `summary.tsv`, a line a game. The games run
//! on T threads (all the machine's by default); each game is the same whatever the threads.

use cognition_towers::{Captain, play_recorded};
use recreation_towers::record::{self, Line};
use recreation_towers::{Commander, Deep, Lookahead, Outcome, Rules, Team};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A side: a name for the record, and how to make its captain.
#[derive(Clone, Debug)]
struct Side {
    name: String,
    deep: Option<f32>,
    lookahead: bool,
}

impl Side {
    fn parse(s: &str) -> Result<Side, String> {
        let (kind, arg) = s.split_once(':').map_or((s, None), |(k, a)| (k, Some(a)));
        match kind {
            "commander" => Ok(Side { name: "commander + lookahead".into(), deep: None, lookahead: true }),
            "plain" => Ok(Side { name: "commander".into(), deep: None, lookahead: false }),
            "deep" => {
                let delay: f32 = arg.map_or(Ok(20.0), |a| a.parse().map_err(|_| format!("deep:{a}: a delay in s")))?;
                Ok(Side {
                    name: format!("commander + lookahead + deep ({delay} s late)"),
                    deep: Some(delay),
                    lookahead: true,
                })
            }
            _ => Err(format!("{s}: a side is commander, plain, deep or deep:DELAY")),
        }
    }

    fn captain(&self, rules: Rules) -> Box<dyn Captain> {
        let mut c = Commander::new(rules).with_lookahead(false);
        if self.lookahead {
            c = c.with_advisor(Lookahead::new(rules), 0.0);
        }
        if let Some(delay) = self.deep {
            c = c.with_advisor(Deep::new(rules), delay);
        }
        Box::new(c)
    }
}

struct Game {
    seed: u64,
    swapped: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("towers_games: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let num = |name: &str, d: u64| -> Result<u64, String> {
        arg(name).map_or(Ok(d), |v| v.parse().map_err(|_| format!("{name} {v}: a number")))
    };
    let n = num("--games", 20)?;
    let from = num("--from", 0)?;
    let blue = Side::parse(&arg("--blue").unwrap_or_else(|| "commander".into()))?;
    let red = Side::parse(&arg("--red").unwrap_or_else(|| "commander".into()))?;
    let rules = match arg("--rules").as_deref() {
        None | Some("default") => Rules::default(),
        Some("v4") => Rules::v4(),
        Some(o) => return Err(format!("--rules {o}: default or v4")),
    };
    let swap = args.iter().any(|a| a == "--swap");
    let threads = num("--threads", std::thread::available_parallelism().map_or(4, |n| n.get()) as u64)? as usize;
    let out = PathBuf::from(arg("--out").unwrap_or_else(|| "towers-games".into()));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    let games: Vec<Game> = (from + 1..=from + n)
        .flat_map(|seed| {
            let mut g = vec![Game { seed, swapped: false }];
            if swap {
                g.push(Game { seed, swapped: true });
            }
            g
        })
        .collect();
    println!(
        "towers_games: {} games, Blue {} against Red {}, into {}",
        games.len(),
        blue.name,
        red.name,
        out.display()
    );
    let next = AtomicUsize::new(0);
    let rows: Mutex<Vec<(usize, Result<String, String>)>> = Mutex::new(vec![]);
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(g) = games.get(i) else { break };
                    let row = play(rules, g, &blue, &red, &out);
                    rows.lock().unwrap().push((i, row));
                }
            });
        }
    });
    let mut rows = rows.into_inner().unwrap();
    rows.sort_by_key(|(i, _)| *i);
    let mut summary = String::from(
        "seed\tswapped\tblue\tred\toutcome\tblue_score\tred_score\tlength\tblue_held\tred_held\tblue_down\tred_down\tblue_took\tred_took\trecord\n",
    );
    let mut failed = 0;
    // By side (the `--blue` side and the `--red` side, whichever colour each had), and by colour.
    let (mut first_won, mut second_won, mut drawn, mut blue_won) = (0, 0, 0, 0);
    for (_, row) in rows {
        match row {
            Ok(line) => {
                let cols: Vec<&str> = line.split('\t').collect();
                let swapped = cols.get(1) == Some(&"true");
                match (cols.get(4).copied(), swapped) {
                    (Some("Blue"), false) | (Some("Red"), true) => first_won += 1,
                    (Some("Blue"), true) | (Some("Red"), false) => second_won += 1,
                    _ => drawn += 1,
                }
                blue_won += usize::from(cols.get(4) == Some(&"Blue"));
                summary += &line;
                summary.push('\n');
            }
            Err(e) => {
                failed += 1;
                eprintln!("{e}");
            }
        }
    }
    let path = out.join("summary.tsv");
    std::fs::write(&path, summary).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "{} won {first_won}, {} won {second_won}, drawn {drawn} (the Blue seat won {blue_won}); {failed} failed. {}",
        blue.name,
        red.name,
        path.display()
    );
    if failed > 0 {
        return Err(format!("{failed} games failed"));
    }
    Ok(())
}

/// Plays one game, writes its record, and returns its line of the summary.
fn play(rules: Rules, g: &Game, blue: &Side, red: &Side, out: &Path) -> Result<String, String> {
    let (b, r) = if g.swapped { (red, blue) } else { (blue, red) };
    let name = if g.swapped { format!("{}-swap.jsonl", g.seed) } else { format!("{}.jsonl", g.seed) };
    let path = out.join(&name);
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mut bc, mut rc) = (b.captain(rules), r.captain(rules));
    let res = play_recorded(
        rules,
        bc.as_mut(),
        rc.as_mut(),
        g.seed,
        [b.name.clone(), r.name.clone()],
        Box::new(std::io::BufWriter::new(file)),
    )
    .map_err(|e| format!("seed {}: {e}", g.seed))?;
    drop((bc, rc));
    // What the commanders took, from the record.
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let lines = record::read(&text)?;
    let took = |team: Team| lines.iter().filter(|l| matches!(l, Line::Took { team: t, .. } if *t == team)).count();
    let outcome = match res.outcome {
        Outcome::Won(t) => format!("{t:?}"),
        Outcome::Draw => "draw".into(),
    };
    Ok(format!(
        "{}\t{}\t{}\t{}\t{outcome}\t{:.0}\t{:.0}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{}\t{name}",
        g.seed,
        g.swapped,
        b.name,
        r.name,
        res.score[0],
        res.score[1],
        res.length,
        res.held[0],
        res.held[1],
        res.downs[0],
        res.downs[1],
        took(Team::Blue),
        took(Team::Red),
    ))
}
