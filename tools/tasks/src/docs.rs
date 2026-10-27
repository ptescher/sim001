//! Runs every acceptance runner, each within two minutes, then writes the manual from what they
//! left in `docs/`. The runners: the domains that run with no window (Towers among them), the
//! league, the AGENT mode in ArduPilot SITL, the Towers game server, the sim's own tests, the VTOL
//! on its own and the sim with a flight controller beside it, the launcher, the Towers commander
//! app, and the ground station's. Each saves its results in a file of its own, and every runner
//! that can take screenshots takes them (pictures of a screen for those with no window).
//!
//! What a runner needs from outside (ArduPilot, Docker) it asks the expectation framework for: a
//! runner whose dependency cannot be had says so, and its tests are "not supported yet".

use crate::limit::{Ended, run as run_limited};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long one suite may run (`SIM001_SUITE_BUDGET`): every runner stops starting tests when it is
/// spent and marks the rest failed, so that the suites can run side by side on a big box and all be
/// over in two minutes (the league excepted: see [`LEAGUE_SECS`]).
pub const SUITE_SECS: u64 = 120;
/// What the league gets instead: the one suite that cannot be done in two minutes.
///
/// The league does not play a few games, it measures. Its «The evolution strategy improves on a
/// slow start» plays 5 generations; a generation is the champion and 8 challengers, 256 games
/// each (2304 games), and then the champion and the slow executor play 512 games each on new
/// seeds: about 12 500 games of 120 s of flight in the fast arena. That is ~190 s on a 10-core
/// laptop, and the plan runs twice (as «Reaching» and as «Visiting the state after»), so those two
/// plans are ~380 s of the suite's ~635 s. The games are CPU-bound and already use every core, so
/// splitting the suite into tests that run side by side does not shorten it (it made it slower).
/// Fewer games would measure a smaller difference from noise: the 0.05 the plan wants is only a
/// few standard errors at 512 games. The suite took 737 s with two plans left to run, since the plans of the captain whose code changes
/// build a worktree and play it (they were refused at once before): 900 s is that, the two plans, and room
/// for a slower box.
/// Revisit this when the games get cheaper, or the plan can use fewer of them.
pub const LEAGUE_SECS: u64 = 900;
/// What the Towers commander app gets: each test of it is on a game server of its own, and the
/// 3D view of a game takes about 2.5 minutes to load the valley, which the test of the cameras
/// waits for. The suite takes ~305 s; 420 s leaves room for a slower box.
pub const COMMAND_SECS: u64 = 420;
/// What a suite gets on top, to write its results and shut down; past it the runner is killed.
const GRACE_SECS: u64 = 30;

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

/// A chore of the build (no time limit worth the name: a cold build is slow).
fn cargo(name: &'static str, root: &Path, args: &[&str], secs: u64) -> Step {
    Step {
        name,
        program: "cargo".into(),
        args: s(args),
        dir: root.to_path_buf(),
        env: vec![],
        limit: Duration::from_secs(secs),
        may_fail: false,
    }
}

/// A suite: it gets the budget and the grace, and its failures are tests that failed.
fn suite(step: Step) -> Step {
    suite_for(step, SUITE_SECS)
}

/// A suite with a budget of its own (see [`LEAGUE_SECS`]).
fn suite_for(mut step: Step, secs: u64) -> Step {
    step.env.push(("SIM001_SUITE_BUDGET".into(), secs.to_string()));
    step.limit = Duration::from_secs(secs + GRACE_SECS);
    step.may_fail = true;
    step
}

/// The packages with a `tests/acceptance.rs` that `tasks docs` runs, and what each is.
const WINDOWLESS: &[&str] =
    &["communication_radio", "aviation_computer", "visualization_telemetry", "cognition_executor", "recreation_towers"];

fn acceptance(name: &'static str, root: &Path, docs_env: &(String, String), packages: &[&str], secs: u64) -> Step {
    let mut args = vec!["test", "--release"];
    for p in packages {
        args.extend(["-p", p]);
    }
    args.extend(["--test", "acceptance"]);
    let mut step = cargo(name, root, &args, 0);
    step.env.push(docs_env.clone());
    suite_for(step, secs)
}

/// The steps of `tasks docs`: everything is built first, so that each suite's clock runs on the
/// tests and not on the compiler; then the suites, each within its budget; then the manual.
pub fn steps(root: &Path, no_station: bool) -> Vec<Step> {
    let docs = root.join("docs");
    let docs_arg = docs.display().to_string();
    let docs_env = ("SIM001_DOCS".to_string(), docs_arg.clone());
    let sim = root.join("target/release/sim001").display().to_string();
    let mut out = vec![];

    let mut test_build = vec!["test", "--release", "--no-run"];
    for p in WINDOWLESS.iter().chain(&["cognition_league", "cognition_runner", "recreation_towers_server"]) {
        test_build.extend(["-p", p]);
    }
    test_build.extend(["--test", "acceptance"]);
    out.push(cargo("build the tests", root, &test_build, 3600));
    out.push(cargo("build the sim", root, &["build", "--release"], 3600));
    for (name, dir) in [
        ("build the launcher", "launcher"),
        ("build the Towers commander app", "command"),
        ("build the console", "console"),
    ] {
        if dir == "console" && no_station {
            continue;
        }
        out.push(cargo(name, &root.join("domains/teleoperation").join(dir), &["build", "--release"], 3600));
    }

    out.push(acceptance("the domains with no window", root, &docs_env, WINDOWLESS, SUITE_SECS));
    // The league plays its games in the fast arena: it is the one suite with a budget of its own.
    out.push(acceptance("the league", root, &docs_env, &["cognition_league"], LEAGUE_SECS));
    out.push(acceptance("the AGENT mode in ArduPilot SITL", root, &docs_env, &["cognition_runner"], SUITE_SECS));
    // A whole game of Towers between two computer teams, six ArduPlane SITL drones.
    out.push(acceptance("the Towers game server", root, &docs_env, &["recreation_towers_server"], SUITE_SECS));

    let sim_step = |name: &'static str, extra: &[&str]| {
        let mut args = s(extra);
        args.extend(s(&["--expect", "--docs", &docs_arg]));
        suite(Step {
            name,
            program: sim.clone(),
            args,
            dir: root.to_path_buf(),
            env: vec![],
            limit: Duration::ZERO,
            may_fail: true,
        })
    };
    out.push(sim_step("the sim", &[]));
    // The VTOL on its own: ArduPilot flies it (the sim asks the harness for ArduPilot).
    out.push(sim_step("the VTOL on its own", &["--aircraft", "t1s-vtol"]));
    // The same on the Seeker 3, a quadcopter: the hover tests run on either.
    out.push(sim_step("the Seeker 3 on its own", &["--aircraft", "seeker3"]));
    // The sim with a flight controller: it flies from its sticks while ArduPilot steps the physics beside it.
    out.push(sim_step("the sim with a flight controller", &["--aircraft", "t1s-vtol", "--direct"]));

    let app = |name: &'static str, dir: &str, extra: &[&str], secs: u64| {
        let mut args = s(&["run", "--release", "--"]);
        args.extend(s(extra));
        args.extend(s(&["--expect", "--docs", &docs_arg]));
        suite_for(
            Step {
                name,
                program: "cargo".into(),
                args,
                dir: root.join("domains/teleoperation").join(dir),
                env: vec![],
                limit: Duration::ZERO,
                may_fail: true,
            },
            secs,
        )
    };
    // The launcher's window, with a stand-in for each program it starts.
    out.push(app("the launcher", "launcher", &[], SUITE_SECS));
    // The commander app of Towers, each test on a fresh game of its own server.
    out.push(app("the Towers commander app", "command", &[], COMMAND_SECS));
    if !no_station {
        out.push(app("the console (the station)", "console", &["--station"], SUITE_SECS));
    }
    out.push(cargo(
        "the manual",
        root,
        &["run", "-q", "-p", "expectation", "--", "docs", "expectations", &docs_arg],
        600,
    ));
    out
}

fn command(step: &Step) -> Command {
    let mut c = Command::new(&step.program);
    c.args(&step.args).current_dir(&step.dir).envs(step.env.iter().map(|(k, v)| (k, v)));
    c
}

/// What the runners that fly the VTOL need and the checkout lacks: a native ArduPlane (see
/// `ardupilot/build_native.sh`; a fresh git worktree has no `.ardupilot` until it is linked to the
/// main checkout's) or Docker. Without it their tests would only say "not supported yet", an hour
/// into the run.
pub fn missing_sitl(root: &Path, docker_up: bool) -> Option<String> {
    let native = root.join(".ardupilot/build/sitl/bin/arduplane");
    if native.exists() || docker_up {
        return None;
    }
    Some(format!(
        "there is no ArduPlane SITL to fly the VTOL: {} is missing and Docker is not running.\n\
         Build it (ardupilot/build_native.sh), link .ardupilot to the main checkout's if this is a worktree, \
         or start Docker. `tasks docs --no-station` leaves out the console's tests.",
        native.display()
    ))
}

fn docker_up() -> bool {
    Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn run(root: &Path, no_station: bool) -> i32 {
    if let Some(why) = missing_sitl(root, docker_up()).filter(|_| !no_station) {
        eprintln!("== not starting: {why}");
        return 1;
    }
    for step in steps(root, no_station) {
        println!("== {}", step.name);
        let ended = run_limited(&mut command(&step), step.limit);
        match (&ended, step.may_fail) {
            (Ended::Exited(0), _) => {}
            (Ended::TimedOut, true) => {
                println!("== {} ran out of time ({} s): its results so far stand", step.name, step.limit.as_secs())
            }
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
                "build the tests",
                "build the sim",
                "build the launcher",
                "build the Towers commander app",
                "build the console",
                "the domains with no window",
                "the league",
                "the AGENT mode in ArduPilot SITL",
                "the Towers game server",
                "the sim",
                "the VTOL on its own",
                "the Seeker 3 on its own",
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
        let quad = find("the Seeker 3 on its own");
        assert!(quad.args.contains(&"seeker3".to_string()) && !quad.args.contains(&"--direct".to_string()));
        let beside = find("the sim with a flight controller");
        assert!(beside.args.contains(&"--direct".to_string()));
    }

    /// Every suite has the same budget, and is killed a little after it, except the league and the
    /// Towers commander app, which have the longer ones that LEAGUE_SECS and COMMAND_SECS explain.
    #[test]
    fn every_suite_has_two_minutes_and_a_grace() {
        for step in steps(&root(), false).iter().filter(|s| s.may_fail) {
            let secs = match step.name {
                "the league" => LEAGUE_SECS,
                "the Towers commander app" => COMMAND_SECS,
                _ => SUITE_SECS,
            };
            assert!(step.env.contains(&("SIM001_SUITE_BUDGET".to_string(), secs.to_string())), "{}", step.name);
            assert_eq!(step.limit, Duration::from_secs(secs + GRACE_SECS), "{}", step.name);
        }
    }

    /// A runner that fails its tests goes on to the next one; building the sim and writing the
    /// manual do not.
    /// A checkout with no ArduPlane and no Docker stops the run before it starts, saying how to fix it.
    #[test]
    fn a_run_with_no_ardupilot_stops_before_it_starts() {
        let dir = std::env::temp_dir().join(format!("sim001-tasks-sitl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let why = missing_sitl(&dir, false).expect("nothing to fly the VTOL with");
        assert!(why.contains("build_native.sh") && why.contains("--no-station"), "{why}");
        assert!(missing_sitl(&dir, true).is_none(), "Docker is enough");
        std::fs::create_dir_all(dir.join(".ardupilot/build/sitl/bin")).unwrap();
        std::fs::write(dir.join(".ardupilot/build/sitl/bin/arduplane"), "").unwrap();
        assert!(missing_sitl(&dir, false).is_none(), "a native build is enough");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_tests_may_fail() {
        for step in steps(&root(), false) {
            let chore = step.name.starts_with("build ") || step.name == "the manual";
            assert_eq!(step.may_fail, !chore, "{}", step.name);
        }
    }

    #[test]
    fn the_runners_write_their_results_where_the_manual_reads_them() {
        for step in steps(&root(), false) {
            let writes = step.args.iter().any(|a| a == "/checkout/docs")
                || step.env.iter().any(|(k, v)| k == "SIM001_DOCS" && v == "/checkout/docs");
            let is_runner = !step.name.starts_with("build ");
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
                    std::fs::read_to_string(p.join("Cargo.toml")).ok().and_then(|t| {
                        t.lines().find_map(|l| l.strip_prefix("name = \"").map(|n| n.trim_end_matches('"').to_string()))
                    })
                };
                if path.join("tests/acceptance.rs").exists() {
                    let n = name(&path).unwrap();
                    assert!(
                        runs(&format!("-p {n} ")) || runs(&format!("-p {n}")),
                        "{n}'s acceptance tests are not run by `tasks docs`"
                    );
                }
                let main = path.join("src/main.rs");
                if std::fs::read_to_string(&main).is_ok_and(|m| m.contains("\"--expect\""))
                    && path.join("Cargo.lock").exists()
                {
                    let dir = path.display().to_string();
                    assert!(
                        lines.iter().any(|l| l.starts_with(&dir) && l.contains("--expect")),
                        "{dir} has an --expect that `tasks docs` does not run"
                    );
                }
            }
        }
    }
}
