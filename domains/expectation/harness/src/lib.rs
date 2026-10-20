//! Runs a suite's stories as acceptance tests against the system under test, and writes the
//! documentation from what happened.
//!
//! ```ignore
//! let acceptance = Acceptance::load("expectations")?.with_coverage();
//! let mut driver = Bindings::new(world).observe(...).set(...).on(...);
//! acceptance.assert_passes(&mut driver, &RunOptions { docs_dir: Some("docs/behaviour".into()), ..Default::default() });
//! ```
//!
//! The system under test plugs in through [`Driver`], or [`Bindings`] for closures.

pub mod bindings;
pub mod driver;
pub mod run;

pub use bindings::Bindings;
pub use driver::{Driver, Need, Progress, Screenshot};
pub use run::{RunOptions, run};

use expectation_docs::Docs;
use expectation_graph::Analysis;
use expectation_language::Loaded;
use expectation_model::diagnostic::{has_errors, render_all};
use expectation_model::{Diagnostic, Suite};
use expectation_story::{Plan, Report};
use std::path::Path;

/// A suite that has been checked and planned, ready to run.
pub struct Acceptance {
    pub suite: Suite,
    pub analysis: Analysis,
    pub plans: Vec<Plan>,
    /// Warnings (and notes) from checking it; errors stop it loading.
    pub diagnostics: Vec<Diagnostic>,
    /// The directory it was loaded from, if it was.
    pub dir: Option<std::path::PathBuf>,
    /// Set by [`Acceptance::load_for`]: the platform this is the suite of. Its results are
    /// its own, and its docs are one tree of several (see [`Acceptance::tree`]).
    pub platform: Option<String>,
}

impl Acceptance {
    /// Load, check and plan the suite in `dir`. Anything that's an error — a step outside the
    /// vocabulary, an unsettled conflict, a story relying on unspecified behaviour — fails
    /// it, with every diagnostic rendered.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, String> {
        Self::read(dir.as_ref(), None)
    }

    /// As [`Acceptance::load`], for one platform of a suite that declares several: the
    /// variants written for it replace the plain lines, and what is `On:` elsewhere is
    /// left out. Run the same suite on each platform with its own driver.
    pub fn load_for(dir: impl AsRef<Path>, platform: &str) -> Result<Self, String> {
        let mut a = Self::read(dir.as_ref(), Some(platform))?;
        a.platform = a.suite.platform.clone();
        Ok(a)
    }

    fn read(dir: &Path, platform: Option<&str>) -> Result<Self, String> {
        let loaded = expectation_language::load_dir_for(dir, platform)
            .map_err(|e| format!("couldn't read {}: {e}", dir.display()))?;
        let mut a = Self::from_loaded(loaded)?;
        a.dir = Some(dir.to_path_buf());
        Ok(a)
    }

    /// Where this platform's docs go under the docs directory `dir`: the default platform's
    /// (and a suite without platforms) are in `dir` itself, the others' in `dir/platforms/<name>`.
    pub fn tree(&self, dir: &Path) -> std::path::PathBuf {
        match self.suite.platform.as_deref() {
            Some(p) if self.suite.platforms.first().map(String::as_str) != Some(p) => {
                dir.join("platforms").join(p.to_lowercase())
            }
            _ => dir.to_path_buf(),
        }
    }

    pub fn from_loaded(loaded: Loaded) -> Result<Self, String> {
        let Loaded { suite, mut diagnostics } = loaded;
        if has_errors(&diagnostics) {
            return Err(render_all(&diagnostics, &suite.sources));
        }
        let analysis = expectation_graph::analyze(&suite);
        diagnostics.extend(analysis.diagnostics.iter().cloned());
        let planned = expectation_story::plan_stories(&suite, &analysis);
        diagnostics.extend(planned.diagnostics);
        diagnostics.extend(expectation_story::untested(&suite, &analysis, &planned.plans));
        if has_errors(&diagnostics) {
            return Err(render_all(&diagnostics, &suite.sources));
        }
        Ok(Acceptance { suite, analysis, plans: planned.plans, diagnostics, dir: None, platform: None })
    }

    /// Add generated tests for the expectations no story exercises.
    pub fn with_coverage(mut self) -> Self {
        let extra = expectation_story::coverage_plans(&self.suite, &self.analysis, &self.plans);
        self.plans.extend(extra);
        self
    }

    pub fn render_diagnostics(&self) -> String {
        render_all(&self.diagnostics, &self.suite.sources)
    }

    pub fn run(&self, driver: &mut dyn Driver, opts: &RunOptions) -> Report {
        run::run(&self.suite, &self.plans, driver, opts)
    }

    pub fn docs<'a>(&'a self, report: Option<&'a Report>) -> Docs<'a> {
        Docs {
            suite: &self.suite,
            analysis: &self.analysis,
            plans: &self.plans,
            report,
            suite_dir: self.dir.as_deref(),
            siblings: &[],
        }
    }

    /// Save `report` as `runner`'s results in `dir/results/` (a platform's runner saves its
    /// own: `gcs@iOS.tsv`), merge them with the results that apply to this platform, and write
    /// its docs (Markdown and the HTML site) from the lot. Returns the merged report.
    pub fn publish(&self, dir: &Path, runner: &str, report: &Report) -> std::io::Result<Report> {
        let results = dir.join("results");
        std::fs::create_dir_all(&results)?;
        let file = expectation_story::results_file(runner, self.platform.as_deref());
        std::fs::write(results.join(file), report.save(&self.plans))?;
        let merged = expectation_story::load_results_for(&results, &self.plans, self.suite.platform.as_deref());
        self.docs(Some(&merged)).write(&self.tree(dir))?;
        Ok(merged)
    }

    /// Run every plan, write the docs to `opts.docs_dir` (if set) whatever happened, and panic
    /// with what failed. For `#[test]`s.
    pub fn assert_passes(&self, driver: &mut dyn Driver, opts: &RunOptions) -> Report {
        let report = self.run(driver, opts);
        if let Some(dir) = &opts.docs_dir {
            self.docs(Some(&report)).write(dir).expect("writing the docs");
        }
        if !report.passed() {
            panic!("acceptance tests failed:\n{}", report.failures(&self.suite, &self.plans));
        }
        report
    }
}
