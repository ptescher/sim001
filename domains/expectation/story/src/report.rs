//! Results of running plans, and what they say about each expectation.

use crate::plan::{Because, Check, Plan};
use expectation_model::{ExpId, Suite, Value};
use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Passed,
    Failed,
    /// Not run, because an earlier step failed.
    Skipped,
    /// Not run, because the driver can't do something it needs yet.
    Unsupported,
    /// No result: it hasn't been run.
    NotRun,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::Skipped => "skipped",
            Status::Unsupported => "unsupported",
            Status::NotRun => "not-run",
        }
    }

    fn parse(s: &str) -> Option<Status> {
        [Status::Passed, Status::Failed, Status::Skipped, Status::Unsupported, Status::NotRun]
            .into_iter()
            .find(|x| x.name() == s)
    }

    /// Did it run (pass or fail), as opposed to not running at all?
    pub fn ran(self) -> bool {
        matches!(self, Status::Passed | Status::Failed)
    }
}

#[derive(Clone, Debug)]
pub struct CheckResult {
    pub observed: Option<Value>,
    pub passed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StepResult {
    pub status: Status,
    /// Same order as the plan step's checks; empty if skipped.
    pub checks: Vec<CheckResult>,
    /// Relative to the docs directory.
    pub screenshot: Option<String>,
    pub error: Option<String>,
    /// How many times the checks were observed before they passed or it gave up.
    pub polls: u32,
}

impl StepResult {
    pub fn skipped() -> Self {
        Self::with(Status::Skipped)
    }

    pub fn with(status: Status) -> Self {
        StepResult { status, checks: vec![], screenshot: None, error: None, polls: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct PlanResult {
    pub status: Status,
    pub start: StepResult,
    pub steps: Vec<StepResult>,
}

impl PlanResult {
    /// The same status throughout, with nothing observed.
    pub fn with(status: Status, plan: &Plan) -> Self {
        PlanResult {
            status,
            start: StepResult::with(status),
            steps: plan.steps.iter().map(|_| StepResult::with(status)).collect(),
        }
    }
}

/// One result per plan, in plan order.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub results: Vec<PlanResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A test checked its promises and they held.
    Verified,
    /// A test checked its promises and one didn't hold.
    Failed,
    /// No test that ran checked it.
    Untested,
    /// The tests that would check it can't run yet: the driver doesn't support them.
    Unsupported,
    /// The tests that would check it haven't run.
    NotRun,
}

impl Report {
    /// Nothing failed. Tests that aren't supported yet, or weren't run, don't count against it.
    pub fn passed(&self) -> bool {
        !self.results.iter().any(|r| r.status == Status::Failed)
    }

    pub fn count(&self, status: Status) -> usize {
        self.results.iter().filter(|r| r.status == status).count()
    }

    /// For plans that haven't run.
    pub fn not_run(plans: &[Plan]) -> Report {
        Report { results: plans.iter().map(|p| PlanResult::with(Status::NotRun, p)).collect() }
    }

    /// Plan by plan, the more informative of the two: a result that ran over one that's
    /// unsupported, and either over none. Merges the runs of different drivers.
    pub fn merge(mut self, other: Report) -> Report {
        let rank = |s: Status| match s {
            Status::NotRun => 0,
            Status::Unsupported => 1,
            _ => 2,
        };
        for (mine, theirs) in self.results.iter_mut().zip(other.results) {
            if rank(theirs.status) > rank(mine.status) {
                *mine = theirs;
            }
        }
        self
    }

    /// Each expectation's verdict, from the checks it contributed.
    pub fn verdicts(&self, suite: &Suite, plans: &[Plan]) -> Vec<Verdict> {
        let mut out = vec![Verdict::Untested; suite.expectations.len()];
        let mut mark = |checks: &[Check], results: &[CheckResult]| {
            for (c, r) in checks.iter().zip(results) {
                let Because::Expectation(e) = c.because else { continue };
                let v = &mut out[e.0 as usize];
                if !r.passed {
                    *v = Verdict::Failed;
                } else if *v == Verdict::Untested {
                    *v = Verdict::Verified;
                }
            }
        };
        for (plan, result) in plans.iter().zip(&self.results) {
            mark(&plan.start_checks, &result.start.checks);
            for (step, r) in plan.steps.iter().zip(&result.steps) {
                mark(&step.checks, &r.checks);
            }
        }
        // Untested only because the tests that would check it can't run yet, or haven't.
        for (status, verdict) in [(Status::Unsupported, Verdict::Unsupported), (Status::NotRun, Verdict::NotRun)] {
            for (plan, result) in plans.iter().zip(&self.results) {
                if result.status != status {
                    continue;
                }
                for e in plan.start_invariants.iter().copied().chain(plan.steps.iter().flat_map(|s| s.exercises())) {
                    if out[e.0 as usize] == Verdict::Untested {
                        out[e.0 as usize] = verdict;
                    }
                }
            }
        }
        out
    }

    /// The results as text, one line per plan, step and check, keyed by each plan's slug so
    /// they load back onto a suite that has changed since.
    pub fn save(&self, plans: &[Plan]) -> String {
        let field = |s: &str| s.replace(['\t', '\n'], " ");
        let opt = |s: &Option<String>| s.as_deref().map(field).unwrap_or_else(|| "-".into());
        let mut out = String::from("expectation-results 1\n");
        for (plan, r) in plans.iter().zip(&self.results) {
            if r.status == Status::NotRun {
                continue;
            }
            let _ = writeln!(out, "plan\t{}\t{}\t{}", plan.slug, r.status.name(), fingerprint(plan));
            for (n, step) in std::iter::once(&r.start).chain(&r.steps).enumerate() {
                let _ = writeln!(
                    out,
                    "step\t{n}\t{}\t{}\t{}\t{}",
                    step.status.name(),
                    step.polls,
                    opt(&step.screenshot),
                    opt(&step.error)
                );
                for c in &step.checks {
                    let observed = match &c.observed {
                        Some(Value::Number(v)) => format!("n:{v}"),
                        Some(Value::Enum(s)) => format!("e:{}", field(s)),
                        None => "-".into(),
                    };
                    let _ = writeln!(out, "check\t{}\t{observed}\t{}", c.passed as u8, opt(&c.error));
                }
            }
        }
        out
    }

    /// Results saved by [`Report::save`], lined up with `plans`. Plans it has nothing for, or
    /// whose steps no longer line up, haven't been run.
    pub fn load(text: &str, plans: &[Plan]) -> Report {
        let opt = |s: &str| (s != "-").then(|| s.to_string());
        let mut found: std::collections::HashMap<String, PlanResult> = Default::default();
        let mut current: Option<(String, PlanResult)> = None;
        // What each saved plan was when it ran; a plan that has changed since hasn't run.
        let mut prints: std::collections::HashMap<String, String> = Default::default();
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["plan", slug, status, rest @ ..] => {
                    found.extend(current.take());
                    if let [print] = rest {
                        prints.insert(slug.to_string(), print.to_string());
                    }
                    let status = Status::parse(status).unwrap_or(Status::NotRun);
                    current =
                        Some((slug.to_string(), PlanResult { status, start: StepResult::with(status), steps: vec![] }));
                }
                ["step", n, status, polls, shot, error] => {
                    let Some((_, r)) = current.as_mut() else { continue };
                    let step = StepResult {
                        status: Status::parse(status).unwrap_or(Status::NotRun),
                        checks: vec![],
                        screenshot: opt(shot),
                        error: opt(error),
                        polls: polls.parse().unwrap_or(0),
                    };
                    if *n == "0" { r.start = step } else { r.steps.push(step) }
                }
                ["check", passed, observed, error] => {
                    let Some((_, r)) = current.as_mut() else { continue };
                    let observed = match observed.split_once(':') {
                        Some(("n", v)) => v.parse().ok().map(Value::Number),
                        Some(("e", v)) => Some(Value::Enum(v.to_string())),
                        _ => None,
                    };
                    let check = CheckResult { observed, passed: *passed == "1", error: opt(error) };
                    r.steps.last_mut().unwrap_or(&mut r.start).checks.push(check);
                }
                _ => {}
            }
        }
        found.extend(current);
        let results = plans
            .iter()
            .map(|p| match found.remove(&p.slug) {
                Some(r)
                    if r.steps.len() == p.steps.len()
                        && prints.get(&p.slug).is_none_or(|print| *print == fingerprint(p)) =>
                {
                    r
                }
                _ => PlanResult::with(Status::NotRun, p),
            })
            .collect();
        Report { results }
    }

    /// What went wrong, for a test failure message.
    pub fn failures(&self, suite: &Suite, plans: &[Plan]) -> String {
        let mut s = String::new();
        for (plan, result) in plans.iter().zip(&self.results) {
            if result.status != Status::Failed {
                continue;
            }
            let _ = writeln!(s, "✗ {}", plan.title);
            let steps = std::iter::once((None, &plan.start_checks, &result.start))
                .chain(plan.steps.iter().zip(&result.steps).map(|(p, r)| (Some(p), &p.checks, r)));
            for (step, checks, r) in steps {
                if r.status != Status::Failed {
                    continue;
                }
                let what = match step {
                    None => "arranging the start".to_string(),
                    Some(p) => p
                        .text
                        .iter()
                        .map(|t| format!("{} {}", t.keyword.as_str(), t.text))
                        .collect::<Vec<_>>()
                        .join(" "),
                };
                let _ = writeln!(s, "    at: {what}");
                if let Some(e) = &r.error {
                    let _ = writeln!(s, "    error: {e}");
                }
                for (c, cr) in checks.iter().zip(&r.checks).filter(|(_, cr)| !cr.passed) {
                    let var = suite.vocabulary.var(c.var);
                    let why = match &c.because {
                        Because::Arranged => "arranged".to_string(),
                        Because::Expectation(e) => format!("«{}»", suite.exp(*e).name),
                        Because::Story(span) => format!("the story, {span}"),
                    };
                    let seen = match (&cr.observed, &cr.error) {
                        (_, Some(e)) => format!("couldn't observe it: {e}"),
                        (Some(v), _) => format!("observed {v}"),
                        (None, _) => "not observed".into(),
                    };
                    let _ = writeln!(s, "    expected {} ({why}); {seen}", var.describe(&c.expect));
                }
            }
        }
        s
    }
}

/// Every `*.tsv` results file in `dir` (one per driver that has run), merged and lined up
/// with `plans`. Nothing there: nothing has run.
/// A short hash of what a plan does and checks, so that results saved for an older version
/// of it aren't shown as its results.
pub fn fingerprint(plan: &Plan) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |s: &str| {
        for b in s.bytes().chain([0]) {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(&format!("{:?}", plan.start));
    for step in &plan.steps {
        for t in &step.text {
            eat(&t.text);
        }
        for c in &step.checks {
            eat(&format!("{:?} {:?}", c.var, c.expect));
        }
    }
    format!("{h:016x}")
}

pub fn load_results(dir: &std::path::Path, plans: &[Plan]) -> Report {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "tsv"))
        .collect();
    files.sort();
    let mut report = Report::not_run(plans);
    for f in files {
        if let Ok(text) = std::fs::read_to_string(&f) {
            report = report.merge(Report::load(&text, plans));
        }
    }
    report
}

/// An expectation's verdict is about the expectation; this is the id it's indexed by.
pub fn verdict_of(verdicts: &[Verdict], e: ExpId) -> Verdict {
    verdicts.get(e.0 as usize).copied().unwrap_or(Verdict::Untested)
}
