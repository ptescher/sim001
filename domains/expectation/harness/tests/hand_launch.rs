//! The hand-launch example as tests: the suite checks clean, every story and generated test
//! passes against the toy world and documents itself, and a seeded defect is caught by the
//! expectation it breaks.

#[path = "../examples/hand_launch/world.rs"]
mod world;

use expectation_harness::{Acceptance, Bindings, RunOptions};
use expectation_story::{Status, Verdict};
use std::path::Path;

fn suite() -> Acceptance {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hand_launch");
    Acceptance::load(dir).unwrap_or_else(|e| panic!("{e}")).with_coverage()
}

#[test]
fn every_story_passes_and_documents_itself() {
    let a = suite();
    assert!(a.diagnostics.iter().all(|d| !d.is_error()));
    let docs = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hand_launch_docs");
    let opts = RunOptions { docs_dir: Some(docs.clone()), ..Default::default() };
    let report = a.assert_passes(&mut world::driver(false), &opts);

    // Generated tests reach what the stories don't, so everything gets verified.
    assert!(report.verdicts(&a.suite, &a.plans).iter().all(|v| *v == Verdict::Verified));
    let read = |page: &str| std::fs::read_to_string(docs.join(page)).unwrap();
    assert!(read("tests.md").contains("```mermaid"));
    assert!(read("features/hand-launch.md").contains("**Except** when the calibration wizard is open"));
    // The story is a tutorial in the reader's words, and a test report step by step.
    let tutorial = read("tutorials/first-throw--throttle-up-then-pick-it-back-up.md");
    assert!(tutorial.contains("2. **Set the throttle to 0 %. Then press reset.**"));
    let test = read("tests/first-throw--throttle-up-then-pick-it-back-up.md");
    assert!(test.contains("## 2. When the throttle is 0 % and the pilot presses reset"));
    assert!(docs.join("screenshots/first-throw--throttle-up-then-pick-it-back-up/01.svg").exists());
}

#[test]
fn a_seeded_defect_fails_the_expectation_it_breaks() {
    let a = suite();
    let report = a.run(&mut world::driver(true), &RunOptions::default());
    assert!(!report.passed());
    let verdicts = report.verdicts(&a.suite, &a.plans);
    let broken = a.suite.expectations.iter().find(|e| e.name.starts_with("The wizard holds")).unwrap();
    assert_eq!(verdicts[broken.id.0 as usize], Verdict::Failed);
    // Steps after the failure are skipped, not failed.
    let failed = report.results.iter().find(|r| r.status == Status::Failed).unwrap();
    assert!(failed.steps.iter().any(|s| s.status == Status::Skipped));
    assert!(report.failures(&a.suite, &a.plans).contains("observed flying"));
}

#[test]
fn what_the_driver_cant_do_is_not_supported_yet() {
    let a = suite();
    let mut driver = Bindings::new(world::World::new(false)).observe("the aircraft", |w| w.phase_name());
    let report = a.run(&mut driver, &RunOptions::default());
    let first = &report.results[0];
    // Not a failure: nothing ran.
    assert_eq!(first.status, Status::Unsupported);
    assert!(report.passed());
    let error = first.start.error.as_deref().unwrap();
    assert!(error.starts_with("The driver can't arrange the aircraft"), "{error}");
    assert!(error.contains("more things"), "{error}");
    let verdicts = report.verdicts(&a.suite, &a.plans);
    assert!(verdicts.iter().all(|v| *v == Verdict::Unsupported), "{verdicts:?}");
}

#[test]
fn results_save_and_load() {
    let a = suite();
    let report = a.run(&mut world::driver(true), &RunOptions::default());
    let loaded = expectation_story::Report::load(&report.save(&a.plans), &a.plans);
    for (x, y) in report.results.iter().zip(&loaded.results) {
        assert_eq!(x.status, y.status);
        assert_eq!(x.steps.len(), y.steps.len());
        for (p, q) in x.steps.iter().zip(&y.steps) {
            assert_eq!((p.status, p.checks.len()), (q.status, q.checks.len()));
            for (c, d) in p.checks.iter().zip(&q.checks) {
                assert_eq!((c.passed, &c.observed), (d.passed, &d.observed));
            }
        }
    }
    // A plan the results don't mention hasn't run; merging prefers what ran.
    let none = expectation_story::Report::not_run(&a.plans);
    assert!(none.results.iter().all(|r| r.status == Status::NotRun));
    let merged = none.merge(loaded);
    assert_eq!(merged.count(Status::Failed), 1);
    // Results saved for an older version of a plan aren't its results.
    let mut changed = a.plans.clone();
    changed[0].steps[0].checks.clear();
    let stale = expectation_story::Report::load(&report.save(&a.plans), &changed);
    assert_eq!(stale.results[0].status, Status::NotRun);
    assert_eq!(stale.results[1].status, report.results[1].status);
}
