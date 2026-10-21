//! Where challengers come from.
//!
//! - [`Es`]: a (1+λ) evolution strategy on the champion's numbers, in the space where each
//!   number's bounds are 0 and 1. Its step size grows after a generation where one of its
//!   challengers was promoted and shrinks after one where none was (the 1/5 rule, roughly).
//! - [`Claude`]: a headless `claude -p` run in a worktree of its own, at the champion's code. As a
//!   tuner it proposes numbers; as an engineer it may also change the code of the primitives and
//!   the tactics (`league::MAY_CHANGE`). It can play the arena itself before it proposes. What it
//!   says is not trusted: the league checks the files, runs the tests and plays the games.

use crate::judge::{Judgement, Reason, Verdict};
use crate::league::{Record, git};
use crate::{Entry, league};
use cognition_arena::{Tactics, Tuning};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a strategy knows when it proposes.
pub struct Context<'a> {
    pub generation: u32,
    pub champion: &'a Entry,
    /// The report of the last generation (Markdown), if there was one.
    pub last_report: Option<&'a str>,
    /// Every challenger before this generation.
    pub history: &'a [Record],
    /// The league's folder, this generation's scratch folder, and the repository it improves.
    pub dir: &'a Path,
    pub work: &'a Path,
    pub repo: &'a Path,
}

/// A challenger as a strategy gives it: an entry (its name, strategy and parent are the
/// league's to set), or the reason it has none.
pub struct Proposal {
    pub entry: Result<Entry, Reason>,
    /// A code change: the worktree it is in. The league commits it there.
    pub worktree: Option<PathBuf>,
    /// What the strategy said.
    pub notes: String,
}

pub trait Strategy: Send {
    fn name(&self) -> &str;
    fn propose(&mut self, ctx: &Context) -> Vec<Proposal>;
    /// How each of its proposals fared, in the order given.
    fn feedback(&mut self, _entries: &[Entry], _judged: &[Judgement]) {}
}

/// The evolution strategy: a (μ/μ, λ) ES with its own search centre. Each generation it samples
/// λ challengers round the centre, and moves the centre to the mean of its best μ (ranked by
/// their difference from the champion on the generation's games), whether or not any was
/// promoted: small gains add up until one is clear enough to promote. When the champion changes
/// (by this strategy or another), the centre starts again from it.
pub struct Es {
    rng: fastrand::Rng,
    /// The step, as a fraction of each number's range.
    pub sigma: f32,
    /// Challengers per generation, and how many of the best the centre moves to.
    pub lambda: usize,
    pub mu: usize,
    /// The chance that each number is changed (at least one always is).
    pub rate: f32,
    /// The centre, in the space where each number's bounds are 0 and 1, and the champion it
    /// started from (its hash).
    centre: Option<(String, Vec<f32>)>,
    /// What it proposed last, in the same space.
    last: Vec<Vec<f32>>,
}

impl Es {
    pub fn new(seed: u64) -> Es {
        Es {
            rng: fastrand::Rng::with_seed(seed),
            sigma: 0.15,
            lambda: 8,
            mu: 2,
            rate: 0.25,
            centre: None,
            last: vec![],
        }
    }

    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.rng.f32().max(1e-7), self.rng.f32());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }

    fn bounds() -> Vec<(f32, f32)> {
        Tuning::BOUNDS.iter().chain(Tactics::BOUNDS.iter()).copied().collect()
    }
}

impl Strategy for Es {
    fn name(&self) -> &str {
        "es"
    }

    fn propose(&mut self, ctx: &Context) -> Vec<Proposal> {
        let Ok(base) = ctx.champion.candidate() else {
            return vec![Proposal {
                entry: Err(Reason::Invalid),
                worktree: None,
                notes: "the champion is invalid".into(),
            }];
        };
        let bounds = Es::bounds();
        let hash = ctx.champion.hash();
        if self.centre.as_ref().is_none_or(|(h, _)| *h != hash) {
            let x: Vec<f32> = base
                .tuning
                .to_array()
                .iter()
                .chain(base.tactics.to_array().iter())
                .zip(&bounds)
                .map(|(v, (lo, hi))| (v - lo) / (hi - lo))
                .collect();
            self.centre = Some((hash, x));
        }
        let centre = self.centre.as_ref().unwrap().1.clone();
        self.last.clear();
        (0..self.lambda)
            .map(|_| {
                let mut x = centre.clone();
                let forced = self.rng.usize(0..x.len());
                for (i, xi) in x.iter_mut().enumerate() {
                    if i == forced || self.rng.f32() < self.rate {
                        *xi = (*xi + self.sigma * self.gauss()).clamp(0.0, 1.0);
                    }
                }
                self.last.push(x.clone());
                let v: Vec<f32> = x.iter().zip(&bounds).map(|(x, (lo, hi))| lo + x * (hi - lo)).collect();
                let mut c = base;
                c.tuning = Tuning::from_array(&v[..Tuning::LEN].try_into().unwrap()).unwrap_or(base.tuning);
                c.tactics = Tactics::from_array(&v[Tuning::LEN..].try_into().unwrap()).unwrap_or(base.tactics);
                let mut e = Entry::new("", &c);
                e.commit = ctx.champion.commit.clone();
                let notes = format!("A step of {:.3} of each range, round the strategy's centre.", self.sigma);
                Proposal { entry: Ok(e), worktree: None, notes }
            })
            .collect()
    }

    fn feedback(&mut self, _entries: &[Entry], judged: &[Judgement]) {
        // Rank what was played by its difference from the champion; refused ones last.
        let mut order: Vec<usize> = (0..judged.len().min(self.last.len())).collect();
        let key = |j: &Judgement| if j.verdict == Verdict::Refused { f32::MIN } else { j.mean };
        order.sort_by(|&a, &b| key(&judged[b]).total_cmp(&key(&judged[a])));
        let best: Vec<usize> =
            order.iter().copied().filter(|&i| judged[i].verdict != Verdict::Refused).take(self.mu).collect();
        if let (Some((_, centre)), false) = (&mut self.centre, best.is_empty()) {
            for (d, c) in centre.iter_mut().enumerate() {
                *c = best.iter().map(|&i| self.last[i][d]).sum::<f32>() / best.len() as f32;
            }
        }
        // Wider when its best beat the champion, narrower when none did.
        let gained = best.first().is_some_and(|&i| judged[i].mean > 0.0);
        self.sigma = (self.sigma * if gained { 1.2 } else { 0.85 }).clamp(0.02, 0.4);
    }
}

/// What Claude does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Proposes numbers.
    Tuner,
    /// May change the code of the primitives and the tactics too.
    Engineer,
}

/// A headless Claude run.
pub struct Claude {
    pub role: Role,
    /// The `claude` command (or a stand-in, in the tests). `SIM001_CLAUDE` overrides it.
    pub command: PathBuf,
    /// The most each run may spend (US dollars).
    pub budget_usd: f32,
    pub model: Option<String>,
}

impl Claude {
    fn new(role: Role) -> Claude {
        let command = std::env::var_os("SIM001_CLAUDE").map(PathBuf::from).unwrap_or_else(|| "claude".into());
        Claude { role, command, budget_usd: 5.0, model: None }
    }

    pub fn tuner() -> Claude {
        Claude::new(Role::Tuner)
    }

    pub fn engineer() -> Claude {
        Claude::new(Role::Engineer)
    }

    pub fn with_command(self, command: PathBuf) -> Claude {
        Claude { command, ..self }
    }

    /// The prompt, with the generation's facts filled in.
    fn prompt(&self, ctx: &Context, wt: &Path) -> String {
        let template = match self.role {
            Role::Tuner => include_str!("../prompts/tuner.md"),
            Role::Engineer => include_str!("../prompts/engineer.md"),
        };
        let tried: Vec<String> = ctx
            .history
            .iter()
            .rev()
            .take(40)
            .map(|r| {
                let reason = r.judgement.reason.map(|x| format!(" ({})", x.label())).unwrap_or_default();
                format!(
                    "- generation {}, {} ({}): {}{reason}, mean difference {:+.3}. {}",
                    r.generation,
                    r.entry.name,
                    r.entry.strategy,
                    r.judgement.verdict.label(),
                    r.judgement.mean,
                    r.entry.notes.lines().next().unwrap_or("")
                )
            })
            .collect();
        template
            .replace("{{generation}}", &ctx.generation.to_string())
            .replace("{{worktree}}", &wt.to_string_lossy())
            .replace("{{champion}}", &serde_json::to_string_pretty(ctx.champion).unwrap())
            .replace("{{report}}", ctx.last_report.unwrap_or("(This is the first generation: there is no report yet.)"))
            .replace("{{history}}", &if tried.is_empty() { "(none yet)".into() } else { tried.join("\n") })
            .replace("{{may_change}}", &league::MAY_CHANGE.map(|f| format!("- `{f}`")).join("\n"))
    }
}

/// What Claude leaves in `.league/candidates.json`: one of these, or a list of them.
#[derive(Deserialize)]
struct Suggested {
    #[serde(default)]
    tuning: BTreeMap<String, f32>,
    #[serde(default)]
    tactics: BTreeMap<String, f32>,
    #[serde(default)]
    notes: String,
}

impl Strategy for Claude {
    fn name(&self) -> &str {
        match self.role {
            Role::Tuner => "claude-tuner",
            Role::Engineer => "claude-engineer",
        }
    }

    fn propose(&mut self, ctx: &Context) -> Vec<Proposal> {
        let refuse = |reason: Reason, notes: String| vec![Proposal { entry: Err(reason), worktree: None, notes }];
        let base = match &ctx.champion.commit {
            Some(c) => c.clone(),
            None => match git(ctx.repo, &["rev-parse", "HEAD"]) {
                Ok(c) => c,
                Err(e) => return refuse(Reason::NoCandidate, format!("no commit to start from: {e}")),
            },
        };
        let wt = ctx.work.join(self.name());
        if let Err(e) = git(ctx.repo, &["worktree", "add", "--detach", &wt.to_string_lossy(), &base]) {
            return refuse(Reason::NoCandidate, format!("no worktree: {e}"));
        }
        let league_dir = wt.join(".league");
        let _ = std::fs::create_dir_all(&league_dir);
        let prompt = self.prompt(ctx, &wt);
        let _ = std::fs::write(league_dir.join("prompt.md"), &prompt);

        let tools = match self.role {
            Role::Tuner => {
                "Read Glob Grep Write(.league/**) Bash(cargo run --release -p cognition_league:*) Bash(cargo build:*)"
            }
            Role::Engineer => {
                "Read Glob Grep Edit Write Bash(cargo test:*) Bash(cargo run --release -p cognition_league:*) Bash(cargo build:*) Bash(git diff:*) Bash(git status:*)"
            }
        };
        let mut cmd = Command::new(&self.command);
        cmd.arg("-p")
            .arg(&prompt)
            .args(["--output-format", "json", "--permission-mode", "dontAsk", "--no-session-persistence"])
            .args(["--allowedTools", tools])
            .args(["--max-budget-usd", &self.budget_usd.to_string()])
            .current_dir(&wt)
            .env("CARGO_TARGET_DIR", ctx.repo.join("target/league"));
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }
        let said = match cmd.output() {
            Ok(o) => {
                let _ = std::fs::write(league_dir.join("claude-output.json"), &o.stdout);
                let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
                v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string()
            }
            Err(e) => return refuse(Reason::NoCandidate, format!("could not run {}: {e}", self.command.display())),
        };

        let suggested: Vec<Suggested> = std::fs::read_to_string(league_dir.join("candidates.json"))
            .ok()
            .and_then(|s| {
                serde_json::from_str::<Vec<Suggested>>(&s)
                    .ok()
                    .or_else(|| serde_json::from_str::<Suggested>(&s).ok().map(|x| vec![x]))
            })
            .unwrap_or_default();
        let changed_code = git(&wt, &["status", "--porcelain"])
            .unwrap_or_default()
            .lines()
            .any(|l| !l.get(3..).unwrap_or("").starts_with(".league"));
        let make = |s: &Suggested| {
            let mut e = ctx.champion.clone();
            e.tuning.extend(s.tuning.clone());
            e.tactics.extend(s.tactics.clone());
            e.notes = if s.notes.is_empty() { said.clone() } else { s.notes.clone() };
            e
        };
        match self.role {
            Role::Tuner => {
                if changed_code {
                    return refuse(Reason::OtherFiles, format!("the tuner changed code. It said: {said}"));
                }
                if suggested.is_empty() {
                    return refuse(Reason::NoCandidate, format!("no .league/candidates.json. It said: {said}"));
                }
                suggested
                    .iter()
                    .take(3)
                    .map(|s| Proposal { entry: Ok(make(s)), worktree: None, notes: said.clone() })
                    .collect()
            }
            Role::Engineer => {
                if !changed_code && suggested.is_empty() {
                    return refuse(Reason::NoCandidate, format!("no change and no candidate. It said: {said}"));
                }
                let s = suggested.into_iter().next().unwrap_or(Suggested {
                    tuning: BTreeMap::new(),
                    tactics: BTreeMap::new(),
                    notes: String::new(),
                });
                vec![Proposal { entry: Ok(make(&s)), worktree: Some(wt), notes: said }]
            }
        }
    }
}
