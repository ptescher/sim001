//! The "Flying by plan" section of the suite (expectations/07-cognition), in real ArduPlane SITL
//! with the AGENT mode: the runner (`cognition_runner --agent --console`) starts three SITL planes,
//! the scripted ground control agent plays a game of tag with them, and then this driver flies the
//! first plane through the runner's console: the mode switch, plans, `COMMAND_ACK`s and the
//! AG_* status are ArduPlane's own.
//!
//! It needs the AGENT build: `SIM001_BIN` (a directory with `arduplane`), else
//! `.ardupilot-agent/build/sitl/bin` (`AP_DIR=.ardupilot-agent AGENT_EXEC=1
//! ardupilot/build_native.sh`). The harness asks for it as the service `ardupilot-agent`: without it
//! every test is "not supported yet", with how to build it. The SITL instances
//! use fixed ports (5770-5790, 9012-9032), so nothing else may run SITL at the same time; this is
//! the only test here, so cargo runs it alone. A game and the tests take about 90 s.
//!
//! What this driver cannot see (where the plane flies) the windowless driver measures, in
//! `domains/cognition/executor/tests/acceptance.rs`. Where both run a test, the docs show this one.

use expectation_harness::service::{Present, Services};
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use expectation_story::Status;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

/// The directory of the AGENT build of ArduPlane, or why there is none.
fn agent_build() -> Result<PathBuf, String> {
    let dir = match std::env::var_os("SIM001_BIN") {
        Some(d) => PathBuf::from(d),
        None => root().join(".ardupilot-agent/build/sitl/bin"),
    };
    if dir.join("arduplane").exists() {
        Ok(dir)
    } else {
        Err(format!(
            "no ArduPlane with the AGENT mode in {} (build it with `AP_DIR=.ardupilot-agent AGENT_EXEC=1 ardupilot/build_native.sh`, or set SIM001_BIN)",
            dir.display()
        ))
    }
}

/// The AGENT build as a service of the run: there or not, and how to build it when it is not.
fn agent_service() -> Present {
    let dir = match std::env::var_os("SIM001_BIN") {
        Some(d) => PathBuf::from(d),
        None => root().join(".ardupilot-agent/build/sitl/bin"),
    };
    Present::new("ardupilot-agent", &dir.join("arduplane"))
        .hint("build it with `AP_DIR=.ardupilot-agent AGENT_EXEC=1 ardupilot/build_native.sh`, or set SIM001_BIN")
}

/// A parameter of `ardupilot/t1s.parm`, which the runner gives every plane as its defaults.
fn parameter(name: &str) -> f64 {
    let parm = std::fs::read_to_string(root().join("ardupilot/t1s.parm")).unwrap_or_default();
    parm.lines()
        .filter_map(|l| l.split_once(char::is_whitespace))
        .find(|(k, _)| *k == name)
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(f64::NAN)
}

/// The runner, its console, and everything it has printed.
struct Runner {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    log: Vec<String>,
    bin: PathBuf,
}

impl Runner {
    fn start(bin: &Path) -> Result<Runner, String> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cognition_runner"))
            .args(["--agent", "--console", "--speedup", "10", "--timeout", "300"])
            .env("SIM001_BIN", bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't start the runner: {e}"))?;
        let (tx, lines) = channel();
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take().unwrap();
        let tx2 = tx.clone();
        std::thread::spawn(move || BufReader::new(out).lines().map_while(Result::ok).try_for_each(|l| tx.send(l)));
        std::thread::spawn(move || BufReader::new(err).lines().map_while(Result::ok).try_for_each(|l| tx2.send(l)));
        let stdin = child.stdin.take().unwrap();
        Ok(Runner { child, stdin, lines, log: vec![], bin: bin.to_path_buf() })
    }

    /// Reads lines until one passes `done`, and returns the lines read.
    fn until(&mut self, timeout: Duration, done: impl Fn(&str) -> bool) -> Result<Vec<String>, String> {
        let end = Instant::now() + timeout;
        let mut got = vec![];
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) => {
                    self.log.push(l.clone());
                    let stop = done(&l);
                    got.push(l);
                    if stop {
                        return Ok(got);
                    }
                }
                Err(e) => {
                    let why = if e == RecvTimeoutError::Timeout { "timed out" } else { "the runner stopped" };
                    let tail: Vec<&String> = self.log.iter().rev().take(15).collect();
                    return Err(format!("{why}; its last lines: {tail:?}"));
                }
            }
        }
    }

    /// Sends a console command, and returns its lines up to and with its `status` line.
    fn command(&mut self, command: &str, timeout: Duration) -> Result<Vec<String>, String> {
        writeln!(self.stdin, "{command}").and_then(|_| self.stdin.flush()).map_err(|e| e.to_string())?;
        self.until(timeout, |l| l.starts_with("status "))
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // `quit` stops the planes; if the runner hangs, kill it and its planes.
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        let end = Instant::now() + Duration::from_secs(15);
        while Instant::now() < end {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = Command::new("pkill").arg("-f").arg(self.bin.join("arduplane")).status();
    }
}

/// What the game showed: the `RESULT` lines and the reports of the hand-overs.
#[derive(Default)]
struct Game {
    in_agent: f64,
    refused: f64,
    pursue_not_it: f64,
    reports_complete: bool,
}

impl Game {
    fn read(lines: &[String]) -> Game {
        let result = |key: &str| {
            lines.iter().find_map(|l| l.strip_prefix(&format!("RESULT {key}=")).map(|v| v.trim().to_string()))
        };
        let num = |key: &str| result(key).and_then(|v| v.parse().ok()).unwrap_or(f64::NAN);
        // Each hand-over, in order: "A tagged B", or "B left the arena and is it (was A)". They
        // must chain from Kestrel (it at the start) to the plane that is it at the end.
        let mut it = "Kestrel".to_string();
        let mut chained = true;
        for l in lines.iter().filter(|l| l.contains("[game ")) {
            let text = l.split("] ").nth(1).unwrap_or("");
            let (from, to) = if let Some((a, b)) = text.split_once(" tagged ") {
                (a.to_string(), b.to_string())
            } else if let Some((b, rest)) = text.split_once(" left the arena and is it (was ") {
                (rest.trim_end_matches(')').to_string(), b.to_string())
            } else {
                chained = false;
                continue;
            };
            chained &= from == it;
            it = to;
        }
        Game {
            in_agent: num("in_agent"),
            refused: num("refused"),
            pursue_not_it: num("pursue_not_it"),
            reports_complete: chained && result("it").as_deref() == Some(it.as_str()),
        }
    }
}

/// The SITL driver. Observations come from the console's `ack`, `reports` and `status` lines.
struct Sitl {
    build: Result<PathBuf, String>,
    runner: Option<Runner>,
    game: Option<Game>,
    mode: String,
    reply: String,
    /// The time to live of the plan the plane accepted last (the plane does not report it; its
    /// `COMMAND_ACK` says that it took that plan).
    ttl: f64,
    plan: f64,
    primitive: String,
    fallback: String,
    interventions: f64,
    recoveries: f64,
    reports: f64,
    /// The console as the reader sees it: the commands typed and what came back.
    transcript: Vec<String>,
}

const ARRANGED: &[&str] = &[
    "the flight mode of the plane",
    "the separation limit",
    "the fence radius",
    "the reply to the plan",
    "the time to live of the plan",
    "the reported plan",
    "the reported primitive",
    "the reported fallback",
];
const OBSERVED: &[&str] = &[
    "the flight mode of the plane",
    "the separation limit",
    "the fence radius",
    "the reply to the plan",
    "the time to live of the plan",
    "the reported plan",
    "the reported primitive",
    "the reported fallback",
    "the reported interventions",
    "the messages about a recovery",
    "the status reports in 10 seconds",
    "the planes in AGENT mode",
    "the refused commands",
    "the pursue plans for the planes not it",
    "the reports of the hand-overs",
];
/// Each event and the console command for it (`None`: nothing to send).
const EVENTS: &[(&str, Option<&str>)] = &[
    ("the ground sends a go-to plan", Some("plan goto 80 10 40 0 600")),
    ("the ground sends a short go-to plan", Some("plan goto 80 10 40 0 5")),
    ("the ground sends an orbit plan", Some("plan orbit 70 0 45 45 600")),
    ("the ground sends a band plan", Some("plan band 70 0 35 10 600")),
    ("the ground sends a loiter plan", Some("plan loiter 0 0 0 0 600")),
    ("the ground sends a malformed plan", Some("plan 9 80 10 40 0 600")),
    ("the ground sends the plan id again with an orbit", Some("again orbit 70 0 45 45 600")),
    ("the plane flies on for 1 second", Some("wait 1")),
    ("the plane flies on for 5 seconds", Some("wait 5")),
    ("the plane flies on for 10 seconds", Some("wait 10")),
    ("the plane flies on for 30 seconds", Some("wait 30")),
    ("the operator watches the status for 10 seconds", Some("watch 10")),
    ("the planes play a game of tag", None),
];

fn key(s: &str) -> String {
    s.trim().trim_start_matches("the ").to_ascii_lowercase()
}

fn is(var: &Variable, subject: &str) -> bool {
    key(&var.subject) == key(subject)
}

impl Sitl {
    fn new() -> Sitl {
        Sitl {
            build: agent_build(),
            runner: None,
            game: None,
            mode: "AGENT".into(),
            reply: "none".into(),
            ttl: 0.0,
            plan: 0.0,
            primitive: "none".into(),
            fallback: "none".into(),
            interventions: 0.0,
            recoveries: 0.0,
            reports: 0.0,
            transcript: vec![],
        }
    }

    /// The runner, after its game, with its console ready.
    fn console(&mut self) -> Result<&mut Runner, String> {
        if self.runner.is_none() {
            let bin = self.build.clone()?;
            let mut r = Runner::start(&bin)?;
            let lines = r.until(Duration::from_secs(400), |l| l == "console ready")?;
            self.game = Some(Game::read(&lines));
            self.runner = Some(r);
        }
        Ok(self.runner.as_mut().unwrap())
    }

    fn command(&mut self, command: &str) -> Result<(), String> {
        let secs: f64 = command.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let lines = self.console()?.command(command, Duration::from_secs_f64(60.0 + secs))?;
        self.transcript.push(format!("> {command}"));
        self.transcript.extend(lines.iter().cloned());
        for l in &lines {
            let fields = |l: &str| -> Vec<(String, String)> {
                l.split_whitespace().filter_map(|w| w.split_once('=')).map(|(k, v)| (k.into(), v.into())).collect()
            };
            let get = |f: &[(String, String)], k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
            if let Some(e) = l.strip_prefix("error ") {
                return Err(format!("the console said: {e}"));
            } else if l.starts_with("ack ") {
                let f = fields(l);
                let (result, reason) = (get(&f, "result").unwrap_or_default(), get(&f, "reason").unwrap_or_default());
                self.reply = match (result.as_str(), reason.as_str()) {
                    ("0", _) => "accepted".into(),
                    ("2", "1") => "refused for the mode".into(),
                    ("2", "2") => "refused as malformed".into(),
                    (r, x) => format!("result {r} reason {x}"),
                };
                if result == "0" {
                    self.ttl = command.split_whitespace().nth(6).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
                }
            } else if let Some(n) = l.strip_prefix("reports ") {
                self.reports = n.trim().parse().unwrap_or(f64::NAN);
            } else if l.starts_with("status ") {
                let f = fields(l);
                let n = |k: &str| get(&f, k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(f64::NAN);
                self.mode = get(&f, "mode").unwrap_or_default();
                self.plan = n("plan");
                self.primitive = match n("prim") as i32 {
                    1 => "pursue",
                    2 => "evade",
                    3 => "go to",
                    4 => "orbit",
                    5 => "band",
                    6 => "loiter",
                    0 => "none",
                    _ => "?",
                }
                .into();
                self.fallback = match n("fall") as i32 {
                    0 => "none",
                    1 => "plan expired",
                    2 => "bad input",
                    3 => "recovery",
                    _ => "?",
                }
                .into();
                self.interventions = n("env");
                self.recoveries = n("recoveries");
            }
        }
        Ok(())
    }
}

impl Driver for Sitl {
    fn reset(&mut self) -> Result<(), String> {
        self.command("reset")?;
        // Each test's pictures start with the reset that the Background asks for.
        self.transcript.drain(..self.transcript.len().saturating_sub(2));
        self.reply = "none".into();
        self.ttl = 0.0;
        self.reports = 0.0;
        Ok(())
    }

    fn can_arrange(&self, var: &Variable, value: &Value) -> bool {
        // Only the start that `reset` gives.
        let s = |v: &str| value.as_str() == Some(v);
        let n = |v: f64| value.as_f64() == Some(v);
        match key(&var.subject).as_str() {
            "flight mode of the plane" => true,
            "separation limit" => n(parameter("AGENT_SEP")),
            "fence radius" => n(parameter("AGENT_FENCE_R")),
            "reply to the plan" | "reported primitive" => s("none"),
            "time to live of the plan" | "reported plan" => n(0.0),
            _ => false,
        }
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        if is(var, "the flight mode of the plane") && value.as_str() == Some("FBWA") {
            return self.command("mode FBWA");
        }
        Ok(())
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        if is(var, "the flight mode of the plane") {
            return self.command(&format!("mode {}", value.as_str().unwrap_or("AGENT")));
        }
        Err(format!("can't set {}", var.subject))
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        match EVENTS.iter().find(|(e, _)| key(e) == key(&event.text)) {
            Some((_, Some(command))) => self.command(command),
            Some((_, None)) => {
                self.console()?;
                // The game's report, as the runner printed it before `console ready`.
                let report: Vec<String> = self.runner.as_ref().map_or(vec![], |r| {
                    r.log.iter().filter(|l| l.starts_with("RESULT") || l.contains("[game ")).cloned().collect()
                });
                self.transcript = report;
                Ok(())
            }
            None => Err(format!("no console command for {}", event.text)),
        }
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        let game = |f: fn(&Game) -> Value| self.game.as_ref().map(f).ok_or("no game yet".to_string());
        Ok(match key(&var.subject).as_str() {
            "flight mode of the plane" => self.mode.as_str().into(),
            "separation limit" => parameter("AGENT_SEP").into(),
            "fence radius" => parameter("AGENT_FENCE_R").into(),
            "reply to the plan" => self.reply.as_str().into(),
            "time to live of the plan" => self.ttl.into(),
            "reported plan" => self.plan.into(),
            "reported primitive" => self.primitive.as_str().into(),
            "reported fallback" => self.fallback.as_str().into(),
            "reported interventions" => self.interventions.into(),
            "messages about a recovery" => self.recoveries.into(),
            "status reports in 10 seconds" => self.reports.into(),
            "planes in agent mode" => game(|g| g.in_agent.into())?,
            "refused commands" => game(|g| g.refused.into())?,
            "pursue plans for the planes not it" => game(|g| g.pursue_not_it.into())?,
            "reports of the hand-overs" => game(|g| if g.reports_complete { "complete" } else { "incomplete" }.into())?,
            _ => return Err(format!("can't observe {}", var.subject)),
        })
    }

    fn advance(&mut self) -> Progress {
        // Each console command waits for the status it changes.
        Progress::Static
    }

    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        Ok(Some(Screenshot::svg(terminal(&self.transcript))))
    }

    fn needs(&self) -> Vec<String> {
        vec!["ardupilot-agent".into()]
    }

    fn supports(&self, need: Need<'_>) -> bool {
        match need {
            Need::Arrange(v) => ARRANGED.iter().any(|s| is(v, s)),
            Need::Set(v) => is(v, "the flight mode of the plane"),
            Need::Perform(e) => EVENTS.iter().any(|(s, _)| key(s) == key(&e.text)),
            Need::Observe(v) => OBSERVED.iter().any(|s| is(v, s)),
        }
    }
}

/// The last lines of the console, as a terminal shows them.
fn terminal(transcript: &[String]) -> String {
    const ROWS: usize = 12;
    let lines = &transcript[transcript.len().saturating_sub(ROWS)..];
    let (w, h) = (720, 24 + 18 * ROWS);
    let mut s = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {w} {h}' width='{w}' height='{h}' font-family='Menlo, monospace' font-size='12'><rect width='{w}' height='{h}' rx='6' fill='#1e1f22'/>"
    );
    for (i, l) in lines.iter().enumerate() {
        let colour = if l.starts_with('>') {
            "#8fd18f"
        } else if l.starts_with("ack") {
            "#f0c674"
        } else {
            "#d8d8d8"
        };
        let text = l.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        s += &format!("<text x='12' y='{}' fill='{colour}' xml:space='preserve'>{text}</text>", 22 + 18 * i);
    }
    s + "</svg>"
}

/// Tests that fail because ArduPlane does not do what the spec says yet (see the windowless
/// acceptance test for why). Remove one from here when it passes.
const KNOWN_FAILURES: &[&str] = &["A repeated plan id changes nothing"];

#[test]
fn the_section_runs_in_sitl() {
    let dir = root().join("expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    let section = a.suite.sections.iter().find(|s| s.slug == "cognition").expect("an cognition section").id;
    let ours = a.suite.subtree(section);
    let mut driver = Sitl::new();
    let mut services = Services::new().with(agent_service());
    if let Some(why) = services.missing(&["ardupilot-agent"]).first() {
        println!("not supported yet: {why}");
    }
    let published = std::env::var("SIM001_DOCS").ok().map(PathBuf::from);
    let report =
        a.run_with(&mut driver, &RunOptions { docs_dir: published.clone(), ..Default::default() }, &mut services);
    drop(driver);
    if let Some(out) = &published {
        a.publish(out, "cognition-sitl", &report).unwrap();
    }
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {} {:?}", r.status, p.title, r.start.error);
    }
    println!("{}", report.failures(&a.suite, &a.plans));
    if agent_build().is_err() {
        assert!(ran.iter().all(|(_, r)| r.status == Status::Unsupported));
        return;
    }
    let known = |title: &str| KNOWN_FAILURES.iter().any(|k| title.contains(k));
    let passed = ran.iter().filter(|(_, r)| r.status == Status::Passed).count();
    assert!(passed >= 3, "only {passed} tests of the section passed in SITL");
    let bad: Vec<_> = ran
        .iter()
        .filter(|(p, r)| r.status == Status::Failed && !known(&p.title))
        .map(|(p, r)| format!("{}: {:?}", p.title, r.status))
        .collect();
    assert!(bad.is_empty(), "{}\n{}", bad.join("\n"), report.failures(&a.suite, &a.plans));
    let fixed: Vec<_> = ran
        .iter()
        .filter(|(p, r)| known(&p.title) && r.status == Status::Passed)
        .map(|(p, _)| p.title.clone())
        .collect();
    assert!(fixed.is_empty(), "these pass now; take them out of KNOWN_FAILURES: {fixed:?}");
}
