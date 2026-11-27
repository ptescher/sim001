//! Reads recorded games of Towers and reports on them (`plans/game-tooling.md`, step 5):
//!
//!   cargo run --release -p cognition_towers --bin towers_report -- RECORD_OR_DIR... [--out FILE.md]
//!
//! A batch first (for each side, whichever colour it had: games, wins, the mean margin of score;
//! and by colour), then the same numbers per game and per team: the towers' control (seconds each
//! team held each tower), the drones down, how much of its flying the enemy saw (and how soon it
//! first saw each drone), and the advice: what each advisor posted, what the commander took, what
//! the taken proposals said they would gain and how the team's lead moved over their horizon (not a
//! counterfactual: it says how things went, not what the proposal was worth), and how many of the
//! lookahead's proposals followed an intent. The report is Markdown, to stdout or FILE.

use recreation_towers::record::{self, Header, Line, Tick};
use recreation_towers::{EntryKind, Outcome, Phase, Team};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(e) = run() {
        eprintln!("towers_report: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let inputs: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--out"))
        .map(|(_, a)| a)
        .collect();
    if inputs.is_empty() {
        return Err("usage: towers_report RECORD_OR_DIR... [--out FILE.md]".into());
    }
    let mut files: Vec<PathBuf> = vec![];
    for i in inputs {
        let p = Path::new(i);
        if p.is_dir() {
            let mut in_dir: Vec<PathBuf> = std::fs::read_dir(p)
                .map_err(|e| format!("{i}: {e}"))?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .collect();
            in_dir.sort();
            files.extend(in_dir);
        } else {
            files.push(p.to_path_buf());
        }
    }
    let mut games = vec![];
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let lines = record::read(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        games.push(GameReport::of(f, &lines).map_err(|e| format!("{}: {e}", f.display()))?);
    }
    let report = report(&games);
    match out {
        Some(p) => std::fs::write(&p, report).map_err(|e| format!("{}: {e}", p.display()))?,
        None => print!("{report}"),
    }
    Ok(())
}

/// What one team did in one game.
#[derive(Default, Clone)]
struct TeamReport {
    side: String,
    score: f32,
    won: bool,
    /// Seconds it held each tower.
    held: Vec<f32>,
    downs: usize,
    /// Seconds its drones flew, and of those the seconds the enemy saw them.
    flown: f32,
    seen: f32,
    /// How soon (s) the enemy first saw each of its drones that it saw.
    first_seen: Vec<f32>,
    /// Per advisor: entries posted, proposals posted, proposals taken, the gain the taken ones
    /// said, the lead's move over their horizon.
    advisors: BTreeMap<String, Advice>,
    /// The lookahead's proposals, and those that followed an intent.
    proposals: usize,
    following: usize,
}

#[derive(Default, Clone)]
struct Advice {
    entries: usize,
    proposals: usize,
    intents: usize,
    taken: usize,
    said: f32,
    moved: f32,
}

struct GameReport {
    name: String,
    seed: u64,
    length: f32,
    outcome: String,
    teams: [TeamReport; 2],
}

impl GameReport {
    fn of(path: &Path, lines: &[Line]) -> Result<GameReport, String> {
        let Some(Line::Header(h)) = lines.first() else { return Err("no header".into()) };
        let h: &Header = h;
        let ticks: Vec<&Tick> =
            lines.iter().filter_map(|l| if let Line::Tick(t) = l { Some(t) } else { None }).collect();
        let last = *ticks.last().ok_or("no ticks")?;
        let end = lines.iter().find_map(|l| if let Line::End(e) = l { Some(e) } else { None });
        let mut teams: [TeamReport; 2] = Default::default();
        for (k, t) in teams.iter_mut().enumerate() {
            t.side = h.sides[k].clone();
            t.score = last.score[k];
            t.held = vec![0.0; h.towers.len()];
            t.won = matches!(end.map(|e| e.outcome), Some(Outcome::Won(w)) if w.index() == k);
        }
        // Control and flying, tick by tick; who the enemy saw, from the events.
        let n = h.drones.len();
        let mut seen = [vec![false; n], vec![false; n]];
        let mut first = vec![None; n];
        let mut li = 0;
        for w in ticks.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dt = b.t - a.t;
            // The lines up to this tick, in order: stop at the first that is later.
            while li < lines.len() {
                let at = match &lines[li] {
                    Line::Spotted { t, .. } | Line::Lost { t, .. } => Some(*t),
                    Line::Tick(t) => Some(t.t),
                    _ => None,
                };
                if at.is_some_and(|at| at > a.t) {
                    break;
                }
                match &lines[li] {
                    Line::Spotted { t, by, drone } => {
                        seen[by.index()][*drone] = true;
                        first[*drone].get_or_insert(*t);
                    }
                    Line::Lost { by, drone, .. } => seen[by.index()][*drone] = false,
                    _ => {}
                }
                li += 1;
            }
            for (k, (_, holder)) in a.towers.iter().enumerate() {
                if let Some(team) = holder {
                    teams[team.index()].held[k] += dt;
                }
            }
            for (d, x) in a.drones.iter().enumerate() {
                if x.phase != Phase::Flying {
                    continue;
                }
                let ti = h.drones[d].0.index();
                teams[ti].flown += dt;
                if seen[1 - ti][d] {
                    teams[ti].seen += dt;
                }
            }
        }
        for (d, f) in first.iter().enumerate() {
            if let Some(t) = f {
                teams[h.drones[d].0.index()].first_seen.push(*t);
            }
        }
        for l in lines {
            if let Line::Down { drone, .. } = l {
                teams[h.drones[*drone].0.index()].downs += 1;
            }
        }
        // The advice: each board's entries, and what was taken.
        let mut by_id: [BTreeMap<u64, &recreation_towers::Entry>; 2] = Default::default();
        for l in lines {
            if let Line::Board { team, entry, .. } = l {
                let tr = &mut teams[team.index()];
                let a = tr.advisors.entry(entry.author.clone()).or_default();
                a.entries += 1;
                match &entry.kind {
                    EntryKind::Proposal(p) => {
                        a.proposals += 1;
                        if entry.author == "lookahead" {
                            tr.proposals += 1;
                            if p.why.contains("intent says") {
                                tr.following += 1;
                            }
                        }
                    }
                    EntryKind::Intent(_) => a.intents += 1,
                }
                by_id[team.index()].insert(entry.id, entry);
            }
        }
        let lead_at = |team: Team, t: f32| {
            let k = ticks.partition_point(|x| x.t <= t).saturating_sub(1);
            let s = ticks[k].score;
            s[team.index()] - s[team.other().index()]
        };
        for l in lines {
            if let Line::Took { t, team, entry } = l
                && let Some(e) = by_id[team.index()].get(entry)
                && let EntryKind::Proposal(p) = &e.kind
            {
                let a = teams[team.index()].advisors.entry(e.author.clone()).or_default();
                a.taken += 1;
                a.said += p.gain;
                let horizon = (p.until - p.made).max(0.0);
                a.moved += lead_at(*team, (t + horizon).min(last.t)) - lead_at(*team, *t);
            }
        }
        let outcome = match end.map(|e| e.outcome) {
            Some(Outcome::Won(t)) => format!("{t:?} won"),
            Some(Outcome::Draw) => "draw".into(),
            None => "not over".into(),
        };
        Ok(GameReport {
            name: path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
            seed: h.seed,
            length: last.t,
            outcome,
            teams,
        })
    }
}

fn pct(a: f32, b: f32) -> String {
    if b > 0.0 { format!("{:.0} %", 100.0 * a / b) } else { "-".into() }
}

fn report(games: &[GameReport]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Towers games: {} recorded\n", games.len());
    // By side, whichever colour it had.
    #[derive(Default)]
    struct Side {
        games: usize,
        won: usize,
        margin: f32,
        seen: f32,
        flown: f32,
        downs: usize,
        taken: usize,
        said: f32,
        moved: f32,
    }
    let mut sides: BTreeMap<String, Side> = BTreeMap::new();
    let mut blue_won = 0;
    for g in games {
        for k in 0..2 {
            let t = &g.teams[k];
            let o = &g.teams[1 - k];
            let side = sides.entry(t.side.clone()).or_default();
            side.games += 1;
            side.won += usize::from(t.won);
            side.margin += t.score - o.score;
            side.seen += t.seen;
            side.flown += t.flown;
            side.downs += t.downs;
            for a in t.advisors.values() {
                side.taken += a.taken;
                side.said += a.said;
                side.moved += a.moved;
            }
        }
        blue_won += usize::from(g.teams[0].won);
    }
    let _ = writeln!(s, "## By side\n");
    let _ = writeln!(
        s,
        "| Side | Games | Won | Mean margin | Seen by the enemy | Drones down a game | Proposals taken a game | Said gain a proposal | Lead's move a proposal |"
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|---|---|---|");
    for (name, x) in &sides {
        let n = x.games.max(1) as f32;
        let _ = writeln!(
            s,
            "| {name} | {} | {} ({}) | {:+.0} | {} | {:.1} | {:.1} | {} | {} |",
            x.games,
            x.won,
            pct(x.won as f32, n),
            x.margin / n,
            pct(x.seen, x.flown),
            x.downs as f32 / n,
            x.taken as f32 / n,
            if x.taken > 0 { format!("{:.1}", x.said / x.taken as f32) } else { "-".into() },
            if x.taken > 0 { format!("{:+.1}", x.moved / x.taken as f32) } else { "-".into() },
        );
    }
    let _ = writeln!(
        s,
        "\nThe Blue seat won {blue_won} of {} ({}). \"Lead's move\" is how the team's lead changed over a taken proposal's horizon: how things went, not what the proposal was worth.\n",
        games.len(),
        pct(blue_won as f32, games.len() as f32)
    );
    let _ = writeln!(s, "## Games\n");
    let _ = writeln!(
        s,
        "| Record | Seed | Outcome | Length | Blue | Red | Towers held (s), Blue / Red | Seen by the enemy, Blue / Red | Down, Blue / Red | Proposals taken, Blue / Red |"
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|---|---|---|---|");
    for g in games {
        let [b, r] = &g.teams;
        let held = |t: &TeamReport| t.held.iter().map(|h| format!("{h:.0}")).collect::<Vec<_>>().join(" ");
        let taken = |t: &TeamReport| t.advisors.values().map(|a| a.taken).sum::<usize>();
        let _ = writeln!(
            s,
            "| {} | {} | {} | {:.0} s | {:.0} | {:.0} | {} / {} | {} / {} | {} / {} | {} / {} |",
            g.name,
            g.seed,
            g.outcome,
            g.length,
            b.score,
            r.score,
            held(b),
            held(r),
            pct(b.seen, b.flown),
            pct(r.seen, r.flown),
            b.downs,
            r.downs,
            taken(b),
            taken(r)
        );
    }
    if !games.is_empty() {
        let _ = writeln!(s, "\n## Advice, game by game\n");
        let _ = writeln!(
            s,
            "| Record | Team | Advisor | Entries | Proposals | Intents | Taken | Said gain | Lead's move | Lookahead following an intent |"
        );
        let _ = writeln!(s, "|---|---|---|---|---|---|---|---|---|---|");
        for g in games {
            for (k, t) in g.teams.iter().enumerate() {
                for (name, a) in &t.advisors {
                    let _ = writeln!(
                        s,
                        "| {} | {} | {name} | {} | {} | {} | {} | {:.0} | {:+.0} | {} |",
                        g.name,
                        if k == 0 { "Blue" } else { "Red" },
                        a.entries,
                        a.proposals,
                        a.intents,
                        a.taken,
                        a.said,
                        a.moved,
                        if name == "lookahead" { pct(t.following as f32, t.proposals as f32) } else { "-".into() }
                    );
                }
            }
        }
    }
    s
}
