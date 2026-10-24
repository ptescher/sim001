//! The game server's tests in the "Towers" section (expectations/09-towers): a whole game between
//! two computer teams, each drone an ArduPlane SITL with the AGENT mode. This driver starts the
//! server (`towers_server --speedup 10`), takes two seats of its own (one that watches Blue, and
//! a spectator, who sees the whole game) and reads the server's report at the end.
//!
//! The fog is checked from outside: each map the Blue seat gets is held against the spectator's
//! whole game at the same moment, with the vision of the rules worked out here again.
//!
//! It needs the AGENT build: `SIM001_BIN` (a directory with `arduplane`), else
//! `.ardupilot-agent/build/sitl/bin`. Without it every test is "not supported yet". The six
//! drones use SITL instances 30 to 35 (ports 6060-6110, 9302-9352), clear of the station's and the
//! runner's.

use expectation_harness::service::{Present, Services};
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use expectation_story::Status;
use recreation_towers::seat::{self, Join, ToSeat, ToServer, Truth};
use recreation_towers::{Phase, Rules, Team, TeamView};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const EVENT: &str = "two computer teams play a game of Towers";
/// The seat port the test's server listens on.
const PORT: u16 = 5955;
/// A whole game takes about a minute and a half at ten times real time.
const PATIENCE: Duration = Duration::from_secs(400);

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

fn agent_dir() -> PathBuf {
    match std::env::var_os("SIM001_BIN") {
        Some(d) => PathBuf::from(d),
        None => root().join(".ardupilot-agent/build/sitl/bin"),
    }
}

fn agent_service() -> Present {
    Present::new("ardupilot-agent", &agent_dir().join("arduplane"))
        .hint("build it with `AP_DIR=.ardupilot-agent AGENT_EXEC=1 ardupilot/build_native.sh`, or set SIM001_BIN")
}

/// What a game left: the server's report and what the two seats got.
#[derive(Default)]
struct Played {
    results: HashMap<String, String>,
    /// Maps the Blue seat got with something in them that Blue did not see.
    leaks: u32,
    /// The Red drones (their numbers) the Blue seat saw.
    red_seen: BTreeSet<usize>,
    /// Blue maps held against the whole game.
    checked: usize,
    /// The last map the Blue seat got, and the rules: the picture of the end of the game.
    last_blue: Option<(TeamView, Rules)>,
}

/// Is `at` in what `team` sees in `truth`?
fn sees(rules: &Rules, truth: &Truth, team: Team, at: glam::Vec2) -> bool {
    let near = |from: glam::Vec2, range: f32| from.distance(at) <= range + 1e-3;
    near(rules.base(team), rules.base_vision)
        || truth.drones.iter().any(|d| d.team == team && d.phase != Phase::Down && near(d.at, rules.drone_vision))
        || truth.towers.iter().any(|t| t.holder == Some(team) && near(t.at, rules.vision(t.kind)))
}

/// What in the Blue map `view` Blue does not see in `truth`: a Red drone shown as seen that Blue
/// cannot see, or not where it is; or one shown as out of sight but where it is now, flying.
fn leaks(rules: &Rules, view: &TeamView, truth: &Truth) -> u32 {
    let n = rules.drones;
    view.enemies
        .iter()
        .filter(|e| {
            let d = &truth.drones[n + e.k];
            if e.seen {
                d.phase == Phase::Down || !sees(rules, truth, Team::Blue, d.at) || e.at != d.at
            } else {
                d.phase == Phase::Flying && e.at == d.at
            }
        })
        .count() as u32
}

/// Takes a seat as `join` and keeps everything the server sends it.
fn take_seat(join: Join, got: Arc<Mutex<Vec<ToSeat>>>) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(60);
    let stream = loop {
        match TcpStream::connect(("127.0.0.1", PORT)) {
            Ok(s) => break s,
            Err(_) if Instant::now() < end => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(format!("no game server on port {PORT}: {e}")),
        }
    };
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    writeln!(w, "{}", seat::encode(&ToServer::Join(join))).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let Ok(msg) = seat::decode::<ToSeat>(&line) {
                got.lock().unwrap().push(msg);
            }
        }
    });
    Ok(())
}

fn play(bin: &Path) -> Result<Played, String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_towers_server"))
        .args(["--speedup", "10", "--timeout", "350", "--seat-port", &PORT.to_string()])
        .env("SIM001_BIN", bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("couldn't start the game server: {e}"))?;
    let out = child.stdout.take().unwrap();
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let l2 = lines.clone();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            println!("server: {line}");
            l2.lock().unwrap().push(line);
        }
    });
    let blue = Arc::new(Mutex::new(vec![]));
    let all = Arc::new(Mutex::new(vec![]));
    let seated = take_seat(Join::Watch(Team::Blue), blue.clone()).and_then(|_| take_seat(Join::Spectate, all.clone()));
    let end = Instant::now() + PATIENCE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            _ if Instant::now() > end || seated.is_err() => break None,
            _ => std::thread::sleep(Duration::from_millis(200)),
        }
    };
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = Command::new("pkill").arg("-f").arg(bin.join("arduplane")).status();
    let _ = reader.join();
    seated?;
    let lines = lines.lock().unwrap().clone();
    let tail = || lines.iter().rev().take(20).rev().cloned().collect::<Vec<_>>().join("\n");
    match status {
        Some(s) if s.success() => {}
        Some(s) => return Err(format!("the game server failed ({s}); its last lines:\n{}", tail())),
        None => return Err(format!("the game server did not finish; its last lines:\n{}", tail())),
    }
    let mut played = Played::default();
    for l in &lines {
        if let Some((k, v)) = l.strip_prefix("RESULT ").and_then(|r| r.split_once('=')) {
            played.results.insert(k.to_string(), v.to_string());
        }
    }
    let rules = blue
        .lock()
        .unwrap()
        .iter()
        .find_map(|m| match m {
            ToSeat::Welcome { rules, .. } => Some(*rules),
            _ => None,
        })
        .ok_or("the Blue seat got no welcome")?;
    let truths: HashMap<u32, Truth> = all
        .lock()
        .unwrap()
        .iter()
        .filter_map(|m| match m {
            ToSeat::Truth(t) => Some((t.time.to_bits(), t.clone())),
            _ => None,
        })
        .collect();
    for m in blue.lock().unwrap().iter() {
        let ToSeat::View(v) = m else { continue };
        played.red_seen.extend(v.enemies.iter().filter(|e| e.seen).map(|e| e.k));
        if let Some(t) = truths.get(&v.time.to_bits()) {
            played.checked += 1;
            let n = leaks(&rules, v, t);
            if n > 0 && played.leaks < 5 {
                for e in &v.enemies {
                    let d = &t.drones[rules.drones + e.k];
                    println!(
                        "fog at {:.2}s: Red{} map {:?} seen {} | truth {:?} {:?} seen by Blue {}",
                        v.time,
                        e.k + 1,
                        e.at,
                        e.seen,
                        d.at,
                        d.phase,
                        sees(&rules, t, Team::Blue, d.at)
                    );
                }
            }
            played.leaks += n;
        }
    }
    played.last_blue = blue.lock().unwrap().iter().rev().find_map(|m| match m {
        ToSeat::View(v) => Some((v.clone(), rules)),
        _ => None,
    });
    if played.checked < 20 {
        return Err(format!("only {} Blue maps could be held against the whole game", played.checked));
    }
    Ok(played)
}

/// The states a fresh game starts in: the section's start, the only one the server can arrange.
const START: &[(&str, &str)] = &[
    ("the game of Towers", "in play"),
    ("the time left", "600"),
    ("the score of Blue", "0"),
    ("the score of Red", "0"),
    ("the towers held by Blue", "1"),
    ("the west tower", "neutral"),
    ("the control of the west tower", "0"),
    ("the Blue drones in the west circle", "0"),
    ("the Red drones in the west circle", "0"),
    ("the Blue drones on the ground in the west circle", "0"),
    ("the east tower", "neutral"),
    ("the control of the east tower", "0"),
    ("the Red drones in the east circle", "0"),
    ("the east tower on the Blue map", "neutral"),
    ("the Red scout", "at its base"),
];

const OBSERVED: &[&str] = &[
    "the drones in AGENT mode",
    "the commands that the drones refused",
    "the drones that crashed",
    "the towers that Blue took",
    "the towers that Red took",
    "the sightings from the fog on the Blue seat",
    "the Red drones that the Blue seat saw",
    "the seconds that the first light drone took to leave its base",
    "the top speed of the light drones",
    "the light drones that fly low",
];

fn is(v: &Variable, subject: &str) -> bool {
    v.subject.eq_ignore_ascii_case(subject)
}

fn text(v: &Value) -> String {
    v.as_f64().map_or_else(|| v.as_str().unwrap_or_default().to_lowercase(), |x| format!("{x}"))
}

struct Server {
    bin: PathBuf,
    played: Option<Result<Played, String>>,
}

impl Driver for Server {
    fn reset(&mut self) -> Result<(), String> {
        self.played = None;
        Ok(())
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        if self.can_arrange(var, value) { Ok(()) } else { Err(format!("a fresh game has another {}", var.subject)) }
    }

    fn can_arrange(&self, var: &Variable, value: &Value) -> bool {
        START.iter().any(|(s, v)| is(var, s) && text(value) == v.to_lowercase())
    }

    fn set(&mut self, var: &Variable, _: &Value) -> Result<(), String> {
        Err(format!("the game server can't set {}", var.subject))
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        if !event.text.eq_ignore_ascii_case(EVENT) {
            return Err(format!("the game server can't make {} happen", event.text));
        }
        let played = play(&self.bin);
        let failed = played.as_ref().err().cloned();
        self.played = Some(played);
        failed.map_or(Ok(()), Err)
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        // Before the game the server starts fresh: its start is the section's.
        if self.played.is_none()
            && let Some((_, v)) = START.iter().find(|(s, _)| is(var, s))
        {
            return Ok(v.parse::<f64>().map_or_else(|_| Value::from(*v), Value::from));
        }
        let Some(Ok(p)) = &self.played else { return Err("no game has been played".into()) };
        let result = |k: &str| p.results.get(k).and_then(|v| v.parse::<f64>().ok()).ok_or(format!("no RESULT {k}"));
        let n = match var.subject.as_str() {
            "the drones in AGENT mode" => result("in_agent")?,
            "the commands that the drones refused" => result("refused")?,
            "the drones that crashed" => result("crashes")?,
            "the towers that Blue took" => result("taken_blue")?,
            "the towers that Red took" => result("taken_red")?,
            "the sightings from the fog on the Blue seat" => p.leaks as f64,
            "the Red drones that the Blue seat saw" => p.red_seen.len() as f64,
            "the seconds that the first light drone took to leave its base" => result("light_away")?,
            "the top speed of the light drones" => result("light_top_speed")?,
            "the light drones that fly low" => result("light_low")?,
            s => return Err(format!("can't observe {s}")),
        };
        Ok(Value::from(n))
    }

    fn advance(&mut self) -> Progress {
        Progress::Static
    }

    /// The Blue seat's map, as the commander app draws it: a fresh game before it is played, the
    /// last map the seat got after.
    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        let (view, rules) = match &self.played {
            Some(Ok(Played { last_blue: Some((v, r)), .. })) => (v.clone(), *r),
            Some(_) => return Ok(None),
            None => {
                let game = recreation_towers::Game::new(Rules::default());
                (game.view(Team::Blue), game.rules)
            }
        };
        Ok(Some(Screenshot::svg(recreation_towers::svg::map_svg(&view, &rules))))
    }

    fn needs(&self) -> Vec<String> {
        vec!["ardupilot-agent".into()]
    }

    fn supports(&self, need: Need<'_>) -> bool {
        match need {
            Need::Arrange(v) => START.iter().any(|(s, _)| is(v, s)),
            Need::Set(_) => false,
            Need::Perform(e) => e.text.eq_ignore_ascii_case(EVENT),
            Need::Observe(v) => OBSERVED.iter().any(|s| is(v, s)) || START.iter().any(|(s, _)| is(v, s)),
        }
    }
}

#[test]
fn the_server_plays_a_game_with_arduplane() {
    let dir = root().join("expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    let section = a.suite.sections.iter().find(|s| s.slug == "towers").expect("a towers section").id;
    let ours = a.suite.subtree(section);
    let mut driver = Server { bin: agent_dir(), played: None };
    let mut services = Services::new().with(agent_service());
    let missing = services.missing(&["ardupilot-agent"]);
    if let Some(why) = missing.first() {
        println!("not supported yet: {why}");
    }
    // With SIM001_DOCS=DIR the results and pictures of the map go into the project's docs.
    let docs = std::env::var("SIM001_DOCS").ok().map(PathBuf::from);
    let opts = RunOptions { docs_dir: docs.clone(), ..Default::default() };
    let suite_time = suite_budget::Budget::from_env();
    let report = expectation_story::Report {
        results: a
            .plans
            .iter()
            .map(|plan| {
                if suite_time.spent() {
                    suite_time.ran_out(&a.suite, plan, &driver)
                } else {
                    expectation_harness::run::run_with(
                        &a.suite,
                        std::slice::from_ref(plan),
                        &mut driver,
                        &opts,
                        &mut services,
                    )
                    .results
                    .remove(0)
                }
            })
            .collect(),
    };
    if let Some(dir) = &docs {
        a.publish(dir, "towers-server", &report).unwrap();
    }
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {} {:?}", r.status, p.title, r.start.error);
    }
    println!("{}", report.failures(&a.suite, &a.plans));
    if !missing.is_empty() {
        return;
    }
    let passed: Vec<_> = ran.iter().filter(|(_, r)| r.status == Status::Passed).map(|(p, _)| p.title.clone()).collect();
    let failed: Vec<_> = ran.iter().filter(|(_, r)| r.status == Status::Failed).map(|(p, _)| p.title.clone()).collect();
    assert!(failed.is_empty(), "failed: {failed:?}\n{}", report.failures(&a.suite, &a.plans));
    assert!(passed.iter().any(|t| t.contains("Watching two computer teams play")), "passed: {passed:?}");
}
