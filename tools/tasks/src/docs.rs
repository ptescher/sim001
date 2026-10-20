//! Runs every acceptance runner, each with a time limit, then writes the manual from what they
//! left in `docs/`. The runners in the order their results are merged: the domains that run with
//! no window, the AGENT mode in ArduPilot SITL, the sim's own tests, the VTOL on its own and the
//! sim with a flight controller beside it, and the ground station's.
//!
//! What a runner needs from outside (ArduPilot, Docker) it asks the expectation framework for: a
//! runner whose dependency cannot be had says so, and its tests are "not supported yet".

use crate::limit::{run as run_limited, Ended};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// One runner: what to run, how long it may take, and whether the rest goes on if it fails.
#[derive(Debug, PartialEq)]
pub struct Step {
    pub name: &'static str,
    pub program: String,
    pub args: Vec<String>,
    pub dir: PathBuf,
    pub env: Vec<(String, String)>,
    pub limit: Duration,
    /// Its failures are tests that failed, not the run: the next one goes on.
    pub may_fail: bool,
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|a| a.to_string()).collect()
}

fn cargo(name: &'static str, root: &Path, args: &[&str], secs: u64) -> Step {
    Step { name, program: "cargo".into(), args: s(args), dir: root.to_path_buf(), env: vec![], limit: Duration::from_secs(secs), may_fail: false }
}

/// The steps of `tasks docs`.
pub fn steps(root: &Path, no_station: bool) -> Vec<Step> {
    let docs = root.join("docs");
    let docs_arg = docs.display().to_string();
    let docs_env = ("SIM001_DOCS".to_string(), docs_arg.clone());
    let sim = root.join("target/release/sim001").display().to_string();
    let mut out = vec![];

    let mut windowless = cargo(
        "the domains with no window",
        root,
        &["test", "--release", "-p", "communication_radio", "-p", "aviation_computer", "-p", "visualization_telemetry", "-p", "cognition_executor", "--test", "acceptance"],
        900,
    );
    windowless.env.push(docs_env.clone());
    out.push(windowless);

    let mut agent = cargo("the AGENT mode in ArduPilot SITL", root, &["test", "--release", "-p", "cognition_runner", "--test", "acceptance"], 400);
    agent.env.push(docs_env);
    agent.may_fail = true;
    out.push(agent);

    out.push(cargo("build the sim", root, &["build", "--release"], 900));
    let sim_plain = Step { name: "the sim", program: sim.clone(), args: s(&["--expect", "--docs", &docs_arg]), dir: root.to_path_buf(), env: vec![], limit: Duration::from_secs(480), may_fail: true };
    out.push(sim_plain);
    // The VTOL on its own: ArduPilot flies it (the sim asks the harness for ArduPilot).
    out.push(Step {
        name: "the VTOL on its own",
        program: sim.clone(),
        args: s(&["--aircraft", "t1s-vtol", "--expect", "--docs", &docs_arg]),
        dir: root.to_path_buf(),
        env: vec![("SIM001_EXPECT_TIMEOUT".into(), "400".into())],
        limit: Duration::from_secs(1500),
        may_fail: true,
    });
    // The sim with a flight controller: it flies from its sticks while ArduPilot steps the physics beside it.
    out.push(Step {
        name: "the sim with a flight controller",
        program: sim,
        args: s(&["--aircraft", "t1s-vtol", "--direct", "--expect", "--docs", &docs_arg]),
        dir: root.to_path_buf(),
        env: vec![],
        limit: Duration::from_secs(400),
        may_fail: true,
    });
    if !no_station {
        out.push(Step {
            name: "the console (the station)",
            program: "cargo".into(),
            args: s(&["run", "--release", "--", "--station", "--expect", "--docs", &docs_arg]),
            dir: root.join("domains/teleoperation/console"),
            env: vec![],
            limit: Duration::from_secs(2400),
            may_fail: true,
        });
    }
    out.push(cargo("the manual", root, &["run", "-q", "-p", "expectation_cli", "--", "docs", "expectations", &docs_arg], 600));
    out
}

fn command(step: &Step) -> Command {
    let mut c = Command::new(&step.program);
    c.args(&step.args).current_dir(&step.dir).envs(step.env.iter().map(|(k, v)| (k, v)));
    c
}

pub fn run(root: &Path, no_station: bool) -> i32 {
    for step in steps(root, no_station) {
        println!("== {}", step.name);
        let ended = run_limited(&mut command(&step), step.limit);
        match (&ended, step.may_fail) {
            (Ended::Exited(0), _) => {}
            (Ended::TimedOut, true) => println!("== {} ran out of time ({} s): its results so far stand", step.name, step.limit.as_secs()),
            (e, true) => println!("== {}: {e:?} (its failed tests are in the manual)", step.name),
            (e, false) => {
                eprintln!("== {} failed: {e:?}", step.name);
                return 1;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/checkout")
    }

    fn names(steps: &[Step]) -> Vec<&str> {
        steps.iter().map(|s| s.name).collect()
    }

    /// The runners go in the order their results are merged, and the manual is written last.
    #[test]
    fn the_runners_go_in_order_and_the_manual_is_last() {
        let all = steps(&root(), false);
        assert_eq!(
            names(&all),
            [
                "the domains with no window",
                "the AGENT mode in ArduPilot SITL",
                "build the sim",
                "the sim",
                "the VTOL on its own",
                "the sim with a flight controller",
                "the console (the station)",
                "the manual"
            ]
        );
    }

    /// The console's tests need Docker and take a while: they can be left out.
    #[test]
    fn the_stations_tests_can_be_left_out() {
        let all = steps(&root(), true);
        assert!(!names(&all).iter().any(|n| n.contains("console")));
        assert_eq!(names(&all).last(), Some(&"the manual"));
    }

    /// Nothing is a script: every step is a program of the build, and ArduPilot is not started
    /// here (the runner asks the framework for it), so no step names a shell or `.sh`.
    #[test]
    fn no_step_is_a_shell_script() {
        for step in steps(&root(), false) {
            let line = format!("{} {}", step.program, step.args.join(" "));
            assert!(!line.contains(".sh"), "{}: {line}", step.name);
            assert!(!["sh", "bash", "zsh"].contains(&step.program.as_str()), "{}", step.name);
        }
    }

    /// The VTOL is flown by ArduPilot with the sticks idle; the flight-controller run flies from
    /// the sticks (`--direct`) while ArduPilot runs beside it.
    #[test]
    fn the_vtol_runs_are_the_two_ways_of_flying_it() {
        let all = steps(&root(), true);
        let find = |n: &str| all.iter().find(|s| s.name == n).unwrap();
        let alone = find("the VTOL on its own");
        assert!(alone.args.contains(&"t1s-vtol".to_string()) && !alone.args.contains(&"--direct".to_string()));
        assert!(alone.limit >= Duration::from_secs(1200), "ArduPilot flies in real time");
        let beside = find("the sim with a flight controller");
        assert!(beside.args.contains(&"--direct".to_string()));
    }

    /// A runner that fails its tests goes on to the next one; building the sim and writing the
    /// manual do not.
    #[test]
    fn only_the_tests_may_fail() {
        for step in steps(&root(), false) {
            let chore = step.name == "build the sim" || step.name == "the manual" || step.name == "the domains with no window";
            assert_eq!(step.may_fail, !chore, "{}", step.name);
        }
    }

    #[test]
    fn the_runners_write_their_results_where_the_manual_reads_them() {
        for step in steps(&root(), false) {
            let writes = step.args.iter().any(|a| a == "/checkout/docs") || step.env.iter().any(|(k, v)| k == "SIM001_DOCS" && v == "/checkout/docs");
            let is_runner = !matches!(step.name, "build the sim");
            assert_eq!(writes, is_runner, "{}", step.name);
        }
    }
}
