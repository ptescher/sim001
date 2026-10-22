//! Runs every acceptance runner, each with a time limit, then writes the manual from what they
//! left in `docs/`. The runners: the domains that run with no window (Towers among them), the
//! league, the AGENT mode in ArduPilot SITL, the Towers game server, the sim's own tests, the VTOL
//! on its own and the sim with a flight controller beside it, the launcher, the Towers commander
//! app, and the ground station's. Each saves its results in a file of its own, and every runner
//! that can take screenshots takes them (pictures of a screen for those with no window).
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
        &[
            "test",
            "--release",
            "-p",
            "communication_radio",
            "-p",
            "aviation_computer",
            "-p",
            "visualization_telemetry",
            "-p",
            "cognition_executor",
            "-p",
            "recreation_towers",
            "--test",
            "acceptance",
        ],
        900,
    );
    windowless.env.push(docs_env.clone());
    out.push(windowless);

    // The league plays thousands of games in the fast arena: minutes, with no window.
    let mut league = cargo("the league", root, &["test", "--release", "-p", "cognition_league", "--test", "acceptance"], 900);
    league.env.push(docs_env.clone());
    league.may_fail = true;
    out.push(league);

    let mut agent = cargo("the AGENT mode in ArduPilot SITL", root, &["test", "--release", "-p", "cognition_runner", "--test", "acceptance"], 400);
    agent.env.push(docs_env.clone());
    agent.may_fail = true;
    out.push(agent);

    // A whole game of Towers between two computer teams, six ArduPlane SITL drones.
    let mut towers = cargo("the Towers game server", root, &["test", "--release", "-p", "recreation_towers_server", "--test", "acceptance"], 900);
    towers.env.push(docs_env);
    towers.may_fail = true;
    out.push(towers);

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
    // The launcher's window, with a stand-in for each program it starts.
    out.push(Step {
        name: "the launcher",
        program: "cargo".into(),
        args: s(&["run", "--release", "--", "--expect", "--docs", &docs_arg]),
        dir: root.join("domains/teleoperation/launcher"),
        env: vec![],
        limit: Duration::from_secs(900),
        may_fail: true,
    });
    // The commander app of Towers, each test on a fresh game of its own server.
    out.push(Step {
        name: "the Towers commander app",
        program: "cargo".into(),
        args: s(&["run", "--release", "--", "--expect", "--docs", &docs_arg]),
        dir: root.join("domains/teleoperation/command"),
        env: vec![],
        limit: Duration::from_secs(1800),
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
    out.push(cargo("the manual", root, &["run", "-q", "-p", "expectation", "--", "docs", "expectations", &docs_arg], 600));
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
                "the league",
                "the AGENT mode in ArduPilot SITL",
                "the Towers game server",
                "build the sim",
                "the sim",
                "the VTOL on its own",
                "the sim with a flight controller",
                "the launcher",
                "the Towers commander app",
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

#[cfg(test)]
mod every_runner {
    use super::*;

    /// Every section of the suite has a runner here, so that none of its pages says "not run":
    /// each acceptance test of the workspace, and each app with an `--expect`.
    #[test]
    fn every_acceptance_runner_of_the_repository_is_a_step() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        let all = steps(&root, false);
        let line = |st: &Step| format!("{} {} {}", st.dir.display(), st.program, st.args.join(" "));
        let lines: Vec<String> = all.iter().map(line).collect();
        let runs = |what: &str| lines.iter().any(|l| l.contains(what));
        for domain in std::fs::read_dir(root.join("domains")).unwrap().flatten() {
            for module in std::fs::read_dir(domain.path()).into_iter().flatten().flatten() {
                let path = module.path();
                let name = |p: &Path| {
                    std::fs::read_to_string(p.join("Cargo.toml"))
                        .ok()
                        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("name = \"").map(|n| n.trim_end_matches('"').to_string())))
                };
                if path.join("tests/acceptance.rs").exists() {
                    let n = name(&path).unwrap();
                    assert!(runs(&format!("-p {n} ")) || runs(&format!("-p {n}")), "{n}'s acceptance tests are not run by `tasks docs`");
                }
                let main = path.join("src/main.rs");
                if std::fs::read_to_string(&main).is_ok_and(|m| m.contains("\"--expect\"")) && path.join("Cargo.lock").exists() {
                    let dir = path.display().to_string();
                    assert!(lines.iter().any(|l| l.starts_with(&dir) && l.contains("--expect")), "{dir} has an --expect that `tasks docs` does not run");
                }
            }
        }
    }
}
