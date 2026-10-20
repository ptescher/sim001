//! Documentation from a suite, written as a manual: stories become step-by-step tutorials,
//! features become topic pages, and sections say what to do before you begin. The test
//! output (each test's steps, checks and verdicts, and the map of the expectation DAG) goes
//! into collapsed "How this is tested" blocks and a separate test report. Two outputs from the
//! same facts:
//!
//! ```text
//! README.md, tutorials/, features/,      Markdown that GitHub renders
//!   glossary.md, tests.md, tests/
//! site/                                   the same as an HTML site with nested navigation
//! screenshots/…                           written by the harness while the tests run
//! ```
//!
//! The wording comes from the specs: `Say:` on an event or phrase tells the reader what to do
//! ("press Space"), `Set:` on a state tells them how to change it, and a story's text under its
//! narrative is the tutorial's introduction. [`voice`] turns steps into instructions with them.
//! A step without a screenshot (its test isn't supported yet, didn't run, or failed before it
//! got there) shows a placeholder instead.

mod html;
mod manual;
mod map;
mod markdown;
mod voice;

use expectation_graph::Analysis;
use expectation_model::{ExpId, Feature, StepText, Suite, VarKind, Variable};
use expectation_story::{Because, Plan, Report, Status, Verdict, coverage};
use std::path::{Path, PathBuf};

pub struct Docs<'a> {
    pub suite: &'a Suite,
    pub analysis: &'a Analysis,
    pub plans: &'a [Plan],
    /// Results of running `plans`, if they ran.
    pub report: Option<&'a Report>,
    /// Where the suite's files are, so each page can link to the one to edit.
    pub suite_dir: Option<&'a Path>,
    /// The other platforms' docs (when the suite has several), so each page can offer the same
    /// page there. Their trees are `platforms/<name>/` (the default platform's is the root).
    pub siblings: &'a [Sibling],
}

/// The pages of the same manual for another platform.
pub struct Sibling {
    pub platform: String,
    /// Its HTML pages, relative to its tree: `site/feature-x.html`.
    pub pages: std::collections::HashSet<String>,
}

pub struct Page {
    /// Relative to the docs directory.
    pub path: String,
    pub content: String,
}

/// Where the placeholders live, relative to the docs directory.
pub const NOT_SUPPORTED: &str = "site/assets/not-supported.svg";
pub const NO_SCREENSHOT: &str = "site/assets/no-screenshot.svg";

impl Docs<'_> {
    /// Write the Markdown pages and the HTML site under `dir`. Pages for stories or features
    /// that no longer exist are left alone; the caller owns the directory.
    pub fn write(&self, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
        // A screenshot the results name but the directory doesn't have (an interrupted run
        // cleared it) is a placeholder, not a broken image.
        let checked = self.report.map(|r| {
            let mut r = r.clone();
            for result in &mut r.results {
                for step in std::iter::once(&mut result.start).chain(&mut result.steps) {
                    if step.screenshot.as_ref().is_some_and(|p| !dir.join(p).exists()) {
                        step.screenshot = None;
                    }
                }
            }
            r
        });
        let docs = Docs { report: checked.as_ref(), ..*self };
        std::fs::create_dir_all(dir)?;
        let source = self.suite_dir.and_then(|s| relative(dir, s));
        let mut written = vec![];
        for page in docs.pages_with(source) {
            let path = dir.join(&page.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, page.content)?;
            written.push(path);
        }
        Ok(written)
    }

    pub fn pages(&self) -> Vec<Page> {
        self.pages_with(None)
    }

    /// This manual's HTML pages, for the other platforms' dropdowns.
    pub fn sibling(&self) -> Sibling {
        Sibling {
            platform: self.suite.platform.clone().unwrap_or_default(),
            pages: self.pages().into_iter().map(|p| p.path).filter(|p| p.ends_with(".html")).collect(),
        }
    }

    /// The pages, with `source` the suite's directory relative to the docs directory.
    fn pages_with(&self, source: Option<String>) -> Vec<Page> {
        let mut facts = Facts::new(self);
        facts.source = source;
        let mut pages = markdown::pages(self, &facts);
        pages.extend(html::pages(self, &facts));
        pages
    }
}

/// What the pages say about each expectation and test, worked out once.
struct Facts {
    /// Per expectation, if anything ran.
    verdicts: Option<Vec<Verdict>>,
    /// Per expectation, the (plan, step) pairs that exercise it; step `None` is the start.
    coverage: Vec<Vec<(usize, Option<usize>)>>,
    /// Per plan, its status (`NotRun` without a report).
    status: Vec<Status>,
    /// The suite's directory relative to the docs directory, for "Edit this page" links.
    source: Option<String>,
}

impl Facts {
    fn new(d: &Docs) -> Self {
        let status = (0..d.plans.len())
            .map(|i| d.report.and_then(|r| r.results.get(i)).map_or(Status::NotRun, |r| r.status))
            .collect();
        Facts {
            verdicts: d.report.map(|r| r.verdicts(d.suite, d.plans)),
            coverage: coverage(d.suite, d.plans),
            status,
            source: None,
        }
    }

    fn verdict(&self, e: ExpId) -> Option<Verdict> {
        self.verdicts.as_ref().map(|v| expectation_story::report::verdict_of(v, e))
    }

    /// The tests that exercise `e`, each once.
    fn tested_by(&self, e: ExpId) -> Vec<usize> {
        let mut by: Vec<usize> = self.coverage[e.0 as usize].iter().map(|(p, _)| *p).collect();
        by.dedup();
        by
    }

    /// The best picture of `e` in action: the screenshot after a step whose checks for it
    /// passed, else after any step that exercises it. Returns (plan, step) with the path.
    fn shot_of(&self, d: &Docs, e: ExpId) -> Option<(usize, Option<usize>, String)> {
        let report = d.report?;
        let mut fallback = None;
        for &(p, step) in &self.coverage[e.0 as usize] {
            let result = &report.results[p];
            let (r, checks) = match step {
                None => (&result.start, &d.plans[p].start_checks),
                Some(s) => (&result.steps[s], &d.plans[p].steps[s].checks),
            };
            let Some(shot) = &r.screenshot else { continue };
            let verified =
                checks.iter().zip(&r.checks).any(|(c, cr)| c.because == Because::Expectation(e) && cr.passed);
            if verified {
                return Some((p, step, shot.clone()));
            }
            fallback.get_or_insert((p, step, shot.clone()));
        }
        fallback
    }
}

/// `to` relative to `from`, both directories: "../expectations".
fn relative(from: &Path, to: &Path) -> Option<String> {
    let (from, to) = (from.canonicalize().ok()?, to.canonicalize().ok()?);
    let common = from.components().zip(to.components()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); from.components().count() - common];
    parts.extend(to.components().skip(common).map(|c| c.as_os_str().to_string_lossy().into_owned()));
    Some(if parts.is_empty() { ".".into() } else { parts.join("/") })
}

/// A feature's slug, unique in the suite: under its section's.
fn feature_slug(suite: &Suite, f: &Feature) -> String {
    let section = &suite.section(f.section).slug;
    if section.is_empty() { f.slug.clone() } else { format!("{section}--{}", f.slug) }
}

/// "As a pilot / I want … / So that …" as one sentence.
fn narrative(lines: &[String]) -> String {
    let parts: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let l = l.trim_end_matches(['.', ',']);
            let mut c = l.chars();
            match c.next() {
                Some(f) if i > 0 && !l.starts_with("I ") => f.to_lowercase().chain(c).collect(),
                _ => l.to_string(),
            }
        })
        .collect();
    format!("{}.", parts.join(", "))
}

/// "When the throttle is 0 % and the pilot presses reset": the action, without its Thens.
fn action_heading(steps: &[StepText]) -> String {
    use expectation_model::Keyword;
    let mut out = String::new();
    for t in steps.iter().take_while(|t| t.keyword != Keyword::Then) {
        match t.keyword {
            Keyword::And => out.push_str(" and "),
            Keyword::But => out.push_str(" but "),
            k => out.push_str(&format!("{} ", k.as_str())),
        }
        out.push_str(&t.text);
    }
    out
}

fn values(v: &Variable) -> String {
    match &v.kind {
        VarKind::Enum(names) => names.join(" · "),
        VarKind::Number { min, max, unit } => {
            let u = unit.as_deref().map(|u| format!(" {u}")).unwrap_or_default();
            format!(
                "a number from {} to {}{u}",
                expectation_model::text::number(*min),
                expectation_model::text::number(*max)
            )
        }
    }
}

fn verdict_label(v: Option<Verdict>) -> &'static str {
    match v {
        Some(Verdict::Verified) => "verified",
        Some(Verdict::Failed) => "failed",
        Some(Verdict::Untested) => "untested",
        Some(Verdict::Unsupported) => "not supported yet",
        Some(Verdict::NotRun) | None => "not run",
    }
}

fn status_label(s: Status) -> &'static str {
    match s {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::Skipped => "skipped",
        Status::Unsupported => "not supported yet",
        Status::NotRun => "not run",
    }
}

/// Why a step has no screenshot, for the placeholder's caption.
fn missing_shot(status: Status) -> &'static str {
    match status {
        Status::Unsupported => "Screenshot to come: the tests can't run this step yet.",
        Status::Skipped => "No screenshot: the test stopped at an earlier step.",
        Status::NotRun => "No screenshot yet: the tests haven't run.",
        Status::Failed => "No screenshot: the test failed before this step.",
        Status::Passed => "No screenshot of this step.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expectation_model::{Keyword, Span};

    #[test]
    fn narratives_read_as_one_sentence() {
        let lines = ["As a pilot", "I want to throw the plane.", "So that it flies"].map(String::from);
        assert_eq!(narrative(&lines), "As a pilot, I want to throw the plane, so that it flies.");
    }

    #[test]
    fn headings_show_the_whole_action() {
        let step =
            |keyword, text: &str| StepText { keyword, text: text.into(), span: Span::new("t", 1), meaning: None };
        let steps = [
            step(Keyword::When, "the throttle is 0 %"),
            step(Keyword::And, "the pilot presses reset"),
            step(Keyword::Then, "the aircraft is in hand"),
        ];
        assert_eq!(action_heading(&steps), "When the throttle is 0 % and the pilot presses reset");
    }
}
