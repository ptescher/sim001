//! The expectation domain end to end, on a toy model of the sim's hand launch:
//!
//! ```text
//! cargo run -p expectation_harness --example hand_launch -- [docs-dir] [--bug]
//! ```
//!
//! It loads the `.expect` suite in this directory and checks it: vocabulary, conflicts, the
//! override DAG, and that every story only relies on specified behaviour. Then it runs each
//! story, plus generated tests for anything no story reaches, against `world::World`, and
//! writes the docs with screenshots. `--bug` seeds a defect (the calibration wizard lets the
//! throttle through) to show what a failing run looks like.

mod world;

use expectation_harness::{Acceptance, RunOptions};
use expectation_story::Status;
use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bug = args.iter().any(|a| a == "--bug");
    let out = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/expectation-docs/hand_launch".into());

    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hand_launch");
    let acceptance = match Acceptance::load(&suite) {
        Ok(a) => a.with_coverage(),
        Err(e) => {
            eprint!("{e}");
            exit(1);
        }
    };
    if !acceptance.diagnostics.is_empty() {
        eprint!("{}", acceptance.render_diagnostics());
    }

    let mut driver = world::driver(bug);
    let report = acceptance.run(&mut driver, &RunOptions { docs_dir: Some(out.clone()), ..Default::default() });
    if let Err(e) = acceptance.docs(Some(&report)).write(&out) {
        eprintln!("couldn't write the docs to {}: {e}", out.display());
        exit(1);
    }

    for (plan, r) in acceptance.plans.iter().zip(&report.results) {
        let mark = if r.status == Status::Passed { "✓" } else { "✗" };
        println!("{mark} {}", plan.title);
    }
    println!("docs: {}", out.join("README.md").display());
    if !report.passed() {
        eprint!("\n{}", report.failures(&acceptance.suite, &acceptance.plans));
        exit(1);
    }
}
