//! The league's folder, and the generations.
//!
//! ```text
//! DIR/champion.json          the champion (an Entry)
//! DIR/state.json             how many generations have run
//! DIR/history.jsonl          one line per challenger: its entry and its judgement
//! DIR/generations/NNN.md     a report per generation, for people and for the Claude strategies
//! DIR/work/                  the strategies' scratch folders and worktrees (left for inspection)
//! ```

use crate::judge::{Judgement, RULES, Reason, Verdict, describe, judge};
use crate::strategy::{Context, Proposal};
use crate::{Entry, Strategy};
use cognition_arena::{GameResult, play_many};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Options {
    /// Games per judgement. The difference in score between two executors has a standard
    /// deviation of about 0.7 a game, so 256 games tell a difference of about 0.1 from noise.
    pub seeds: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { seeds: 256 }
    }
}

/// The files a Claude engineer may change (from the top of the repository).
pub const MAY_CHANGE: [&str; 3] = [
    "domains/cognition/executor/src/primitives.rs",
    "domains/cognition/executor/src/tuning.rs",
    "domains/cognition/agent/src/scripted.rs",
];

/// The files that are the safety envelope, or what it rests on.
pub const ENVELOPE: [&str; 3] =
    ["domains/cognition/executor/src/lib.rs", "domains/cognition/executor/src/math.rs", "domains/cognition/plan/"];

/// The tests a code change must pass.
pub const TESTS: [&str; 4] = ["-p", "cognition_executor", "-p", "cognition_agent"];

/// One generation, as written down.
pub struct Generation {
    pub number: u32,
    /// Each challenger and its judgement, in the order the strategies gave them.
    pub judged: Vec<(Entry, Judgement)>,
    pub promoted: Option<String>,
    pub report: String,
}

impl Generation {
    pub fn summary(&self) -> String {
        let counts = |v: Verdict| self.judged.iter().filter(|(_, j)| j.verdict == v).count();
        format!(
            "generation {}: {} challengers ({} kept out, {} refused); {}",
            self.number,
            self.judged.len(),
            counts(Verdict::KeptOut),
            counts(Verdict::Refused),
            match &self.promoted {
                Some(name) => format!("promoted {name}"),
                None => "the champion stays".into(),
            }
        )
    }

    pub fn verdicts(&self) -> Vec<Verdict> {
        self.judged.iter().map(|(_, j)| j.verdict).collect()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    generation: u32,
}

/// One line of `history.jsonl`.
#[derive(Serialize, Deserialize)]
pub struct Record {
    pub generation: u32,
    pub entry: Entry,
    pub judgement: Judgement,
}

pub struct League {
    pub dir: PathBuf,
    pub repo: PathBuf,
    pub champion: Entry,
    pub generation: u32,
}

impl League {
    /// The league in `dir`, or a new one whose champion is the default executor.
    pub fn open(dir: &Path, repo: &Path) -> League {
        let champion = std::fs::read_to_string(dir.join("champion.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| Entry::load("default").unwrap());
        let state: State = std::fs::read_to_string(dir.join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        League { dir: dir.into(), repo: repo.into(), champion, generation: state.generation }
    }

    /// A new league in `dir` (what was there is removed), with `champion`.
    pub fn start(dir: &Path, repo: &Path, champion: Entry) -> League {
        let _ = std::fs::remove_dir_all(dir);
        let league = League { dir: dir.into(), repo: repo.into(), champion, generation: 0 };
        league.save();
        league
    }

    fn save(&self) {
        std::fs::create_dir_all(self.dir.join("generations")).expect("the league's folder");
        let json = serde_json::to_string_pretty(&self.champion).unwrap();
        std::fs::write(self.dir.join("champion.json"), json).expect("champion.json");
        let state = serde_json::to_string(&State { generation: self.generation }).unwrap();
        std::fs::write(self.dir.join("state.json"), state).expect("state.json");
    }

    pub fn status(&self) -> String {
        let mut out = format!(
            "champion {} ({}) from {}, after {} generations",
            self.champion.name,
            self.champion.hash(),
            if self.champion.strategy.is_empty() { "the start" } else { &self.champion.strategy },
            self.generation
        );
        if let Some(c) = &self.champion.commit {
            let _ = write!(out, "; its code is commit {c} (branch league/champion)");
        }
        out
    }

    /// The report of the last generation, if any.
    pub fn last_report(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join(format!("generations/{:03}.md", self.generation))).ok()
    }

    /// Every challenger so far.
    pub fn history(&self) -> Vec<Record> {
        std::fs::read_to_string(self.dir.join("history.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// The seeds of generation `n`: new games each generation, so that nothing is tuned to a few.
    pub fn seeds(n: u32, count: usize) -> Vec<u64> {
        let base = n as u64 * 1_000_000;
        (base..base + count as u64).collect()
    }

    /// Runs one generation with `strategies`.
    pub fn generation(&mut self, strategies: &mut [Box<dyn Strategy>], opts: &Options) -> Generation {
        let number = self.generation + 1;
        let work = self.dir.join(format!("work/{number:03}"));
        std::fs::create_dir_all(&work).expect("the work folder");
        let report = self.last_report();
        let history = self.history();
        let champion = self.champion.clone();

        // Every strategy proposes at once (the Claude ones take minutes).
        let proposals: Vec<(String, Vec<Proposal>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = strategies
                .iter_mut()
                .map(|s| {
                    let ctx = Context {
                        generation: number,
                        champion: &champion,
                        last_report: report.as_deref(),
                        history: &history,
                        dir: &self.dir,
                        work: &work,
                        repo: &self.repo,
                    };
                    scope.spawn(move || (s.name().to_string(), s.propose(&ctx)))
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("a strategy panicked")).collect()
        });

        let seeds = League::seeds(number, opts.seeds);
        let champion_games = self.play(&self.champion, None, &seeds);
        let mut judged: Vec<(Entry, Judgement, Option<Vec<GameResult>>)> = vec![];
        let mut per_strategy: Vec<usize> = vec![];
        for (strategy, props) in proposals {
            per_strategy.push(props.len());
            for (k, p) in props.into_iter().enumerate() {
                let name = format!("g{number:03}-{strategy}-{}", k + 1);
                let mut entry = match p.entry {
                    Ok(e) => e,
                    Err(reason) => {
                        let mut e = self.champion.clone();
                        e.notes = p.notes;
                        judged.push((
                            Entry { name, strategy: strategy.clone(), ..e },
                            Judgement::refused(reason),
                            None,
                        ));
                        continue;
                    }
                };
                entry.name = name;
                entry.strategy = strategy.clone();
                entry.parent = Some(self.champion.name.clone());
                if entry.notes.is_empty() {
                    entry.notes = p.notes;
                }
                let checked = match &p.worktree {
                    Some(wt) => self.check_code(wt, &mut entry),
                    None => Ok(()),
                };
                let games = checked
                    .and_then(|()| entry.candidate().map(|_| ()).map_err(|_| Reason::Invalid))
                    .and_then(|()| self.play(&entry, p.worktree.as_deref(), &seeds).map_err(|_| Reason::BuildFails));
                match games {
                    Ok(g) => judged.push((entry, Judgement::refused(Reason::NoCandidate), Some(g))),
                    Err(reason) => judged.push((entry, Judgement::refused(reason), None)),
                }
            }
        }
        let champion_games = match champion_games {
            Ok(g) => g,
            Err(e) => panic!("the champion {} does not play: {e}", self.champion.name),
        };
        let rivals = judged.iter().filter(|(_, _, g)| g.is_some()).count();
        for (_, j, games) in judged.iter_mut() {
            if let Some(g) = games {
                *j = judge(&champion_games, g, rivals);
            }
        }
        // The best of the promoted wins; the others were not the best.
        let best = judged
            .iter()
            .enumerate()
            .filter(|(_, (_, j, _))| j.verdict == Verdict::Promoted)
            .max_by(|a, b| a.1.1.mean.total_cmp(&b.1.1.mean))
            .map(|(i, _)| i);
        for (i, (_, j, _)) in judged.iter_mut().enumerate() {
            if j.verdict == Verdict::Promoted && Some(i) != best {
                j.verdict = Verdict::KeptOut;
            }
        }

        // Each strategy hears how its own proposals fared.
        let mut at = 0;
        for (s, n) in strategies.iter_mut().zip(per_strategy) {
            let mine: Vec<Judgement> = judged[at..at + n].iter().map(|(_, j, _)| j.clone()).collect();
            let entries: Vec<Entry> = judged[at..at + n].iter().map(|(e, _, _)| e.clone()).collect();
            s.feedback(&entries, &mine);
            at += n;
        }

        let promoted = best.map(|i| judged[i].0.clone());
        let report = self.report(
            number,
            &seeds,
            &champion_games,
            &judged.iter().map(|(e, j, g)| (e, j, g.as_deref())).collect::<Vec<_>>(),
            promoted.as_ref(),
        );
        let mut history = String::new();
        for (entry, judgement, _) in &judged {
            let r = Record { generation: number, entry: entry.clone(), judgement: judgement.clone() };
            history.push_str(&serde_json::to_string(&r).unwrap());
            history.push('\n');
        }
        use std::io::Write;
        std::fs::create_dir_all(&self.dir).expect("the league's folder");
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("history.jsonl")).unwrap();
        f.write_all(history.as_bytes()).unwrap();

        // Each code challenger's commit stays reachable (refs/league/NAME); the worktrees go.
        for (entry, _, _) in &judged {
            if let Some(c) = &entry.commit {
                let _ = git(&self.repo, &["update-ref", &format!("refs/league/{}", entry.name), c]);
            }
        }
        let listed = git(&self.repo, &["worktree", "list", "--porcelain"]).unwrap_or_default();
        let work = work.canonicalize().unwrap_or(work);
        for path in listed.lines().filter_map(|l| l.strip_prefix("worktree ")) {
            if Path::new(path).starts_with(&work) {
                let _ = git(&self.repo, &["worktree", "remove", "--force", path]);
            }
        }

        if let Some(p) = &promoted {
            if let Some(commit) = &p.commit {
                // The champion's code lives on its own branch; nothing is merged into master.
                let _ = git(&self.repo, &["branch", "-f", "league/champion", commit]);
            }
            self.champion = p.clone();
        }
        self.generation = number;
        self.save();
        std::fs::write(self.dir.join(format!("generations/{number:03}.md")), &report).expect("the report");
        Generation {
            number,
            judged: judged.into_iter().map(|(e, j, _)| (e, j)).collect(),
            promoted: promoted.map(|p| p.name),
            report,
        }
    }

    /// Checks a code change in its worktree: commits what the strategy left, refuses a change to
    /// a file it may not change, and runs the tests. Sets the entry's commit.
    fn check_code(&self, wt: &Path, entry: &mut Entry) -> Result<(), Reason> {
        let base = git(wt, &["rev-parse", "HEAD"]).map_err(|_| Reason::BuildFails)?;
        let _ = git(wt, &["add", "-A", "--", ".", ":!.league"]);
        let changed = git(wt, &["diff", "--cached", "--name-only", &base]).unwrap_or_default();
        let changed: Vec<&str> = changed.lines().filter(|l| !l.is_empty()).collect();
        if changed.iter().any(|f| ENVELOPE.iter().any(|e| f.starts_with(e))) {
            return Err(Reason::EnvelopeChanged);
        }
        let code = changed;
        if code.iter().any(|f| !MAY_CHANGE.contains(f)) {
            return Err(Reason::OtherFiles);
        }
        if !code.is_empty() {
            let msg = format!("League {}: {}", entry.name, entry.notes.lines().next().unwrap_or("a challenger"));
            git(
                wt,
                &["-c", "user.name=cognition_league", "-c", "user.email=league@localhost", "commit", "-q", "-m", &msg],
            )
            .map_err(|_| Reason::BuildFails)?;
            let tests = Command::new("cargo")
                .args(["test", "--release", "-q"])
                .args(TESTS)
                .current_dir(wt)
                .env("CARGO_TARGET_DIR", self.target_dir())
                .output();
            let passed = tests.as_ref().is_ok_and(|o| o.status.success());
            if !passed {
                let log = tests
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr))
                    .unwrap_or_default();
                let tail: Vec<&str> = log.lines().rev().take(30).collect();
                entry.notes.push_str("\n\nTests failed:\n");
                for l in tail.into_iter().rev() {
                    entry.notes.push_str(l);
                    entry.notes.push('\n');
                }
                return Err(Reason::TestsFail);
            }
            entry.commit = Some(git(wt, &["rev-parse", "HEAD"]).map_err(|_| Reason::BuildFails)?);
        }
        Ok(())
    }

    /// Where worktrees build: one folder for all, so each build is incremental.
    fn target_dir(&self) -> PathBuf {
        self.repo.join("target/league")
    }

    /// Plays `entry` on `seeds`: here, if its code is this build's; else with the arena built in
    /// its worktree (`wt`, or one made at its commit).
    fn play(&self, entry: &Entry, wt: Option<&Path>, seeds: &[u64]) -> Result<Vec<GameResult>, String> {
        let candidate = entry.candidate()?;
        let Some(commit) = &entry.commit else { return Ok(play_many(&candidate, seeds)) };
        let wt = match wt {
            Some(w) => w.to_path_buf(),
            None => {
                let w = self.dir.join(format!("work/code-{}", &commit[..commit.len().min(12)]));
                if !w.exists() {
                    git(&self.repo, &["worktree", "add", "--detach", &w.to_string_lossy(), commit])?;
                }
                w
            }
        };
        let file = wt.join(".league/candidate-played.json");
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&file, serde_json::to_string(entry).unwrap()).map_err(|e| e.to_string())?;
        let list: Vec<String> = seeds.iter().map(|s| s.to_string()).collect();
        let out = Command::new("cargo")
            .args(["run", "--release", "-q", "-p", "cognition_league", "--", "play", "--json", "--candidate"])
            .arg(&file)
            .args(["--seed-list", &list.join(",")])
            .current_dir(&wt)
            .env("CARGO_TARGET_DIR", self.target_dir())
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
    }

    /// The report of a generation, in Markdown.
    fn report(
        &self,
        number: u32,
        seeds: &[u64],
        champion: &[GameResult],
        judged: &[(&Entry, &Judgement, Option<&[GameResult]>)],
        promoted: Option<&Entry>,
    ) -> String {
        let mut r = String::new();
        let _ = writeln!(r, "# Generation {number}\n");
        let _ = writeln!(
            r,
            "Champion: **{}** ({}), from {}. {} games, seeds {}..{}.\n",
            self.champion.name,
            self.champion.hash(),
            if self.champion.strategy.is_empty() { "the start" } else { &self.champion.strategy },
            seeds.len(),
            seeds.first().unwrap_or(&0),
            seeds.last().unwrap_or(&0)
        );
        let _ = writeln!(r, "Champion's games: {}.\n", describe(&crate::judge::Summary::of(champion)));
        let _ = writeln!(
            r,
            "Each challenger plays the same games. It is promoted if the mean difference in score is above 0 with 95 % confidence (shared among the challengers), and it does not crash, give bad input, leave the arena more often, or need the envelope more than {} times as often.\n",
            crate::judge::INTERVENTION_ALLOWANCE
        );
        let _ = writeln!(
            r,
            "| Challenger | Strategy | Verdict | Mean difference | Lower bound | Score | It (s) | Tags | Tagged | Exits | Crashes | Envelope |"
        );
        let _ = writeln!(r, "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
        for (e, j, _) in judged {
            let verdict = match j.reason {
                Some(reason) => format!("{} ({})", j.verdict.label(), reason.label()),
                None => j.verdict.label().to_string(),
            };
            let s = &j.challenger;
            let _ = writeln!(
                r,
                "| {} | {} | {} | {:+.3} | {:+.3} | {:+.3} | {:.1} | {} | {} | {} | {} | {:.1} % |",
                e.name,
                e.strategy,
                verdict,
                j.mean,
                j.lower,
                s.mean_score,
                s.it_time,
                s.tags,
                s.tagged,
                s.exits,
                s.crashes,
                100.0 * s.intervention_rate
            );
        }
        let _ = writeln!(r);
        match promoted {
            Some(p) => {
                let _ = writeln!(r, "**Promoted: {}.**\n", p.name);
            }
            None => {
                let _ = writeln!(r, "**No challenger was promoted. The champion stays.**\n");
            }
        }
        let _ = writeln!(r, "## The challengers\n");
        for (e, j, games) in judged {
            let _ = writeln!(r, "### {}\n", e.name);
            let changes = changes(&self.champion, e);
            if !changes.is_empty() {
                let _ = writeln!(r, "Changed: {}.\n", changes.join(", "));
            }
            if let Some(c) = &e.commit {
                let _ = writeln!(r, "Code: commit `{c}`.\n");
            }
            if !e.notes.trim().is_empty() {
                let _ = writeln!(r, "Notes from the strategy:\n\n> {}\n", e.notes.trim().replace('\n', "\n> "));
            }
            if games.is_some() {
                let _ = writeln!(r, "Its games: {}.\n", describe(&j.challenger));
            }
            for g in games.iter().flat_map(|g| g.iter()) {
                for c in &g.crash_log {
                    let _ = writeln!(r, "- Crash in seed {}: {c}", g.seed);
                }
            }
            if games.is_some_and(|g| g.iter().any(|g| !g.crash_log.is_empty())) {
                let _ = writeln!(
                    r,
                    "\n(`ARENA_TRACE=SEED cargo run --release -p cognition_league -- play --candidate FILE --seed-list SEED` traces a game.)\n"
                );
            }
        }
        let _ = writeln!(r, "Envelope rules, per executor step: {}.", RULES.join(", "));
        r
    }
}

/// The numbers `e` changed from `base`, as `name a -> b`.
pub fn changes(base: &Entry, e: &Entry) -> Vec<String> {
    let mut out = vec![];
    for (kind, a, b) in [("", &base.tuning, &e.tuning), ("tactics.", &base.tactics, &e.tactics)] {
        for (name, v) in b {
            let was = a.get(name).copied();
            if was.is_none_or(|w| (w - v).abs() > 1e-6) {
                out.push(format!("{kind}{name} {} -> {v:.4}", was.map_or("-".into(), |w| format!("{w:.4}"))));
            }
        }
    }
    out
}

/// Runs git in `dir`; its trimmed output, or its error.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").args(args).current_dir(dir).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}
