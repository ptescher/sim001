//! Where challengers come from.
//!
//! - [`Es`]: a (1+λ) evolution strategy on the champion's numbers, in the space where each
//!   number's bounds are 0 and 1. Its step size grows after a generation where one of its
//!   challengers was promoted and shrinks after one where none was (the 1/5 rule, roughly).
//! - [`Claude`]: a headless `claude -p` run in a worktree of its own, at the champion's code. As a
//!   tuner it proposes numbers; as an engineer it may also change the code of the primitives and
//!   the tactics (`league::MAY_CHANGE`). It can play the arena itself before it proposes. What it
//!   says is not trusted: the league checks the files, runs the tests and plays the games.

use crate::Entry;
pub use crate::contest::Role;
use crate::contest::{Contest, Group};
use crate::judge::{Judgement, Reason, Verdict};
use crate::league::{Record, git};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a strategy knows when it proposes.
pub struct Context<'a> {
    /// What the league improves.
    pub contest: &'a dyn Contest,
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
    /// The runs of an agent since this was last asked (a Claude strategy's: what each cost).
    fn take_runs(&mut self) -> Vec<Run> {
        vec![]
    }
}

/// One run of an agent, as the league keeps it (`runs.jsonl`, and the report).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub strategy: String,
    #[serde(default)]
    pub generation: u32,
    /// What it cost and how many turns it took, as `claude --output-format json` says; unknown
    /// when it printed nothing (it was stopped, or it failed).
    pub cost_usd: Option<f64>,
    pub turns: Option<u32>,
    /// The wall-clock time it took.
    pub seconds: f64,
    pub timed_out: bool,
    /// Where its prompt, its output and the files it wrote are kept, from the league's folder.
    #[serde(default)]
    pub folder: String,
    /// The last thing it said.
    #[serde(default)]
    pub said: String,
}

impl Run {
    /// A run from what `claude --output-format json` printed (`out`).
    pub fn read(strategy: &str, out: &[u8], seconds: f64, timed_out: bool) -> Run {
        let v: serde_json::Value = serde_json::from_slice(out).unwrap_or_default();
        Run {
            strategy: strategy.into(),
            cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()),
            turns: v.get("num_turns").and_then(|t| t.as_u64()).map(|t| t as u32),
            seconds,
            timed_out,
            said: v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string(),
            ..Run::default()
        }
    }
}

/// Runs `cmd` for at most `time`: true if it finished, false if it was stopped. On Unix it runs
/// in a process group of its own, and the whole group is stopped: what it started (a build, a
/// test) would otherwise go on, and hold its output open. Being in a group of its own, it does
/// not get the terminal's Ctrl-C: a league stopped that way leaves it to its own time and budget.
pub fn run_for(cmd: &mut Command, time: std::time::Duration) -> std::io::Result<bool> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let mut child = cmd.spawn()?;
    let deadline = std::time::Instant::now() + time;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            #[cfg(unix)]
            // SAFETY: a signal to the group this child leads (its id is the group's).
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = child.kill();
            child.wait()?;
            return Ok(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Copies the files under `from` into `to`, but programs the league built (`bin`).
fn keep(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    let _ = std::fs::create_dir_all(to);
    for e in entries.flatten() {
        let (path, name) = (e.path(), e.file_name());
        if path.is_dir() {
            if name != "bin" {
                keep(&path, &to.join(&name));
            }
        } else {
            let _ = std::fs::copy(&path, to.join(&name));
        }
    }
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

    fn bounds(groups: &[Group]) -> Vec<(f32, f32)> {
        groups.iter().flat_map(|g| g.bounds.iter().copied()).collect()
    }
}

impl Strategy for Es {
    fn name(&self) -> &str {
        "es"
    }

    fn propose(&mut self, ctx: &Context) -> Vec<Proposal> {
        let groups = ctx.contest.groups();
        let Ok(values) = ctx.champion.check(ctx.contest) else {
            return vec![Proposal {
                entry: Err(Reason::Invalid),
                worktree: None,
                notes: "the champion is invalid".into(),
            }];
        };
        let bounds = Es::bounds(&groups);
        let hash = ctx.champion.hash();
        if self.centre.as_ref().is_none_or(|(h, _)| *h != hash) {
            let x = values.iter().flatten().zip(&bounds).map(|(v, (lo, hi))| (v - lo) / (hi - lo).max(1e-9)).collect();
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
                let mut v = x.iter().zip(&bounds).map(|(x, (lo, hi))| lo + x * (hi - lo));
                let split: Vec<Vec<f32>> = groups.iter().map(|g| v.by_ref().take(g.names.len()).collect()).collect();
                let mut e = Entry::with("", &groups, &split);
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

/// A headless Claude run.
pub struct Claude {
    pub role: Role,
    /// The `claude` command (or a stand-in, in the tests). `SIM001_CLAUDE` overrides it.
    pub command: PathBuf,
    /// The most each run may spend (US dollars).
    pub budget_usd: f32,
    pub model: Option<String>,
    /// The longest a run may take.
    pub time: std::time::Duration,
    /// The runs not yet taken by the league.
    runs: Vec<Run>,
}

impl Claude {
    fn new(role: Role) -> Claude {
        let command = std::env::var_os("SIM001_CLAUDE").map(PathBuf::from).unwrap_or_else(|| "claude".into());
        Claude {
            role,
            command,
            budget_usd: 5.0,
            model: None,
            time: std::time::Duration::from_secs(30 * 60),
            runs: vec![],
        }
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

    pub fn with_time(self, time: std::time::Duration) -> Claude {
        Claude { time, ..self }
    }

    /// Runs `claude --model NAME`.
    pub fn with_model(self, name: &str) -> Claude {
        Claude { model: Some(name.to_string()), ..self }
    }

    /// The prompt, with the generation's facts filled in.
    fn prompt(&self, ctx: &Context, wt: &Path) -> String {
        let template = ctx.contest.prompt(self.role);
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
            .replace(
                "{{may_change}}",
                &ctx.contest.may_change().iter().map(|f| format!("- `{f}`")).collect::<Vec<_>>().join("\n"),
            )
    }
}

/// What Claude leaves in `.league/candidates.json`: one of these, or a list of them.
#[derive(Deserialize)]
struct Suggested {
    #[serde(default)]
    notes: String,
    /// Each group's numbers (`tuning`, `tactics`, `captain`...).
    #[serde(flatten)]
    params: BTreeMap<String, BTreeMap<String, f32>>,
}

impl Strategy for Claude {
    fn name(&self) -> &str {
        match self.role {
            Role::Tuner => "claude-tuner",
            Role::Engineer => "claude-engineer",
        }
    }

    fn take_runs(&mut self) -> Vec<Run> {
        std::mem::take(&mut self.runs)
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
        // The run's record, beside the worktree: the league removes the worktree after the
        // generation, and keeps this.
        let kept = ctx.work.join(format!("{}.run", self.name()));
        let _ = std::fs::create_dir_all(&kept);
        let prompt = self.prompt(ctx, &wt);
        let _ = std::fs::write(league_dir.join("prompt.md"), &prompt);

        let tools = match self.role {
            Role::Tuner => {
                "Read Glob Grep Edit(.league/**) Bash(cargo run --release -p cognition_league:*) Bash(cargo run --release -q -p cognition_league:*) Bash(cargo build:*)"
            }
            Role::Engineer => {
                "Read Glob Grep Edit Write Bash(cargo test:*) Bash(cargo run --release -p cognition_league:*) Bash(cargo run --release -q -p cognition_league:*) Bash(cargo build:*) Bash(git diff:*) Bash(git status:*)"
            }
        };
        let mut cmd = Command::new(&self.command);
        cmd.arg("-p")
            .arg(&prompt)
            .args(["--output-format", "json", "--permission-mode", "dontAsk", "--no-session-persistence"])
            .args(["--allowedTools", tools])
            .args(["--max-budget-usd", &self.budget_usd.to_string()])
            .current_dir(&wt)
            .stdin(std::process::Stdio::null())
            .env("CARGO_TARGET_DIR", ctx.repo.join("target/league"));
        // Started from inside a Claude session (the desktop app, its terminal, Claude Code), the
        // environment points at that session's own auth and endpoint; `claude` must use its own
        // login instead, or it is refused (401).
        for (name, _) in std::env::vars_os() {
            let n = name.to_string_lossy();
            if n.starts_with("CLAUDE") || n == "ANTHROPIC_BASE_URL" {
                cmd.env_remove(&name);
            }
        }
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }
        let (out_path, err_path) = (kept.join("claude-output.json"), kept.join("claude-stderr.txt"));
        let started = std::time::Instant::now();
        let ran = std::fs::File::create(&out_path)
            .and_then(|out| Ok((out, std::fs::File::create(&err_path)?)))
            .and_then(|(out, err)| run_for(cmd.stdout(out).stderr(err), self.time));
        let finished = match ran {
            Ok(f) => f,
            Err(e) => return refuse(Reason::NoCandidate, format!("could not run {}: {e}", self.command.display())),
        };
        let out = std::fs::read(&out_path).unwrap_or_default();
        let mut run = Run::read(self.name(), &out, started.elapsed().as_secs_f64(), !finished);
        run.generation = ctx.generation;
        run.folder = kept.strip_prefix(ctx.dir).unwrap_or(&kept).to_string_lossy().into_owned();
        keep(&league_dir, &kept);
        let said = run.said.clone();
        self.runs.push(run);
        if !finished {
            let minutes = self.time.as_secs_f32() / 60.0;
            return refuse(Reason::TimedOut, format!("stopped after {minutes:.1} minutes, its time"));
        }

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
            for (group, numbers) in &s.params {
                e.params.entry(group.clone()).or_default().extend(numbers.clone());
            }
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
                let s =
                    suggested.into_iter().next().unwrap_or(Suggested { notes: String::new(), params: BTreeMap::new() });
                vec![Proposal { entry: Ok(make(&s)), worktree: Some(wt), notes: said }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A git repository with one commit, for a strategy to make worktrees in.
    fn repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for args in [
            &["init", "-q"][..],
            &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "start"],
        ] {
            assert!(Command::new("git").args(args).current_dir(dir).status().unwrap().success());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_run_past_its_time_is_stopped_with_what_it_started() {
        let dir = std::env::temp_dir().join(format!("league-stop-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let pid_file = dir.join("child.pid");
        // A shell that waits on a child of its own, as Claude waits on a build.
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!("sleep 600 & echo $! > {}; wait", pid_file.display()));
        let started = std::time::Instant::now();
        let finished = run_for(&mut cmd, std::time::Duration::from_secs(1)).unwrap();
        assert!(!finished, "it said the run finished");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "it waited {:?}", started.elapsed());
        let child = std::fs::read_to_string(&pid_file).unwrap().trim().to_string();
        // Gone, or a zombie that nobody has reaped yet: either way, it no longer runs.
        let state = Command::new("ps").args(["-o", "stat=", "-p", &child]).output().unwrap().stdout;
        let state = String::from_utf8_lossy(&state);
        assert!(
            state.trim().is_empty() || state.starts_with('Z'),
            "the shell's child {child} is still running: {state}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_that_finishes_in_time_is_not_stopped() {
        assert!(run_for(&mut Command::new("true"), std::time::Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn a_run_reads_its_cost_and_turns_from_what_claude_printed() {
        let out = br#"{"type":"result","result":"done","total_cost_usd":1.25,"num_turns":12,"duration_ms":60000}"#;
        let run = Run::read("claude-tuner", out, 61.0, false);
        assert_eq!((run.cost_usd, run.turns, run.seconds, run.timed_out), (Some(1.25), Some(12), 61.0, false));
        assert_eq!(run.said, "done");
        // Nothing printed (it was stopped, or it failed): the time is still known.
        let run = Run::read("claude-engineer", b"", 1800.0, true);
        assert_eq!((run.cost_usd, run.turns, run.seconds, run.timed_out), (None, None, 1800.0, true));
    }

    #[test]
    fn claude_runs_with_its_own_login_not_the_session_it_was_started_from() {
        // As under the desktop app: a base URL and the session's own Claude variables.
        // SAFETY: set before any other thread of this test reads the environment.
        unsafe {
            std::env::set_var("ANTHROPIC_BASE_URL", "http://localhost:1");
            std::env::set_var("CLAUDE_CODE_OAUTH_SCOPES", "x");
            std::env::set_var("CLAUDECODE", "1");
        }
        let root = std::env::temp_dir().join(format!("league-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (repo_dir, work) = (root.join("repo"), root.join("work"));
        repo(&repo_dir);
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes/records-env.sh");
        let mut s = Claude::tuner().with_command(fake);
        let champion = Entry::load("default").unwrap();
        let ctx = Context {
            contest: &crate::contest::executor::ExecutorContest,
            generation: 1,
            champion: &champion,
            last_report: None,
            history: &[],
            dir: &root,
            work: &work,
            repo: &repo_dir,
        };
        let _ = s.propose(&ctx);
        let seen = std::fs::read_to_string(work.join("claude-tuner/.league/env.txt")).unwrap();
        assert_eq!(seen.trim(), "", "claude was given: {seen}");
        let _ = Command::new("git").args(["worktree", "prune"]).current_dir(&repo_dir).status();
        let _ = std::fs::remove_dir_all(&root);
    }
}
