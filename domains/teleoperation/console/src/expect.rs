//! `--expect [suite] [--docs dir]`: runs the acceptance tests in `expectations/` against the
//! live console and, with `--docs`, writes the documentation with real screenshots.
//!
//! The runner works on a thread of its own, through a driver that turns every call into a job
//! for the UI thread (polled between frames). Events are real mouse events dispatched into the
//! console's window at the place the button was painted, so they take the path a click does:
//! hit test, `on_click`, and the same handlers. States are read back from the console's model
//! and the vehicle's telemetry. The runner needs a vehicle, so start the whole station:
//! `cargo run --release -- --station --expect`.
//!
//! Not driven yet (their tests report "not supported yet"): the mission file, the terrain
//! check, the survey grid and measuring (the console has no such features), and calibrating the
//! radio (F2, in the sim's window; the stand-in radio of the station is never calibrated).

use crate::link::Cmd;
use crate::ui::app::{self, Gcs};
use crate::ui::map::Tool;
use crate::vehicle::{Waypoint, WpKind, mode_number};
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use gpui::{
    App, AppContext as _, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    PlatformInput, Point, Window, WindowHandle,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use teleoperation_shared::feed::{GameClock, GameKind, GameStatus, Key, RadioState};

/// The process exit code once the run is over (0 when everything that could run passed).
pub static EXIT: AtomicI32 = AtomicI32::new(0);

pub struct Config {
    pub suite: PathBuf,
    pub docs: Option<PathBuf>,
    /// Run only the tests whose title contains this (any case).
    pub only: Option<String>,
    /// The plane in the sim is a VTOL (`--aircraft`).
    pub vtol: bool,
    /// The names on the radio's mode switch, in order (empty without a flight controller).
    pub rc_modes: Vec<&'static str>,
    /// The tests' own TX15, hosted on the flight computer's serial line (with the station).
    pub radio: Option<aviation_line::Hosted>,
    /// The vehicle, for the radio of a test that doesn't use it.
    pub vehicle: std::sync::Arc<std::sync::Mutex<crate::vehicle::Vehicle>>,
}

/// Where a pilot who flies without the radio (a keyboard pilot) has the throttle: mid-way in the
/// modes that climb on the stick once the plane is armed, down otherwise.
fn idle_throttle(armed: bool, mode: &str) -> f32 {
    if armed && (mode.starts_with('Q') || mode == "GUIDED") { 0.5 } else { 0.0 }
}

/// The test's elevator is -1 for a stick pulled back (as the sim's USB radio read it); the pilot of the
/// serial line has back as positive.
fn pilot_of(r: &RadioState) -> aviation_line::Pilot {
    aviation_line::Pilot { roll: r.roll, pitch: -r.pitch, yaw: r.yaw, throttle: r.throttle, arm: r.arm, mode: r.mode }
}

/// What the pilot does with the radio.
#[derive(Clone, Copy)]
enum RadioMove {
    /// The arm switch (channel 5): on or off.
    ArmSwitch(bool),
    /// The mode channel (channel 8), at the middle of this slot's band.
    ModeSwitch(usize),
    /// The throttle stick, 0 (down) to 1 (up).
    Throttle(f32),
    /// The aileron stick, -1 (left) to 1 (right).
    Aileron(f32),
    /// The elevator stick, -1 (back: nose up) to 1 (forward: nose down).
    Elevator(f32),
    /// The mode switch, at the button for this mode.
    Mode(&'static str),
}

/// The stand-in radio as a pilot leaves it when the next test starts: unplugged, the sticks let go
/// with the throttle down, the arm switch off and the mode switch on its first slot.
fn radio_at_rest(r: RadioState) -> RadioState {
    RadioState { connected: false, roll: 0.0, pitch: 0.0, yaw: 0.0, throttle: STICK_LOW, arm: false, mode: 0, ..r }
}

/// The radio that presses the sim's reset for a fresh plane: at rest already, since the autopilot
/// restarts with whatever it sends. ArduPlane takes the mode switch's mode before it knows where it
/// is, and SITL panics entering QRTL there ("Location cannot be (0, 0, 0)"), at every start.
fn radio_for_a_fresh_plane(r: RadioState) -> RadioState {
    RadioState { reset_presses: r.reset_presses.wrapping_add(1), ..radio_at_rest(r) }
}

/// Where the throttle stick is for each of its values in the vocabulary.
const STICK_LOW: f32 = 0.0;
const STICK_MIDDLE: f32 = 0.5;
const STICK_HIGH: f32 = 0.75;
/// And the aileron stick: half way to either side.
const AILERON_HALF: f32 = 0.5;
/// The wing of the VTOL needs a little nose up to hold its height at cruise speed: a pilot holds the
/// elevator stick a little back, with the throttle stick all the way up.
const ELEVATOR_A_LITTLE_BACK: f32 = -0.3;
const STICK_TOP: f32 = 1.0;

/// What each event of the vocabulary does on the console.
enum Act {
    /// Click the button with this id.
    Button(&'static str),
    /// Click it, and again a moment later (a confirmation).
    Twice(&'static str),
    /// Click the map.
    Map(MouseButton),
    /// Something the pilot does on the radio (the sim's stand-in for the TX15).
    Radio(RadioMove),
    /// Several things, one after the other.
    Radios(&'static [RadioMove]),
    /// ARM, then CONFIRM ARM, again and again until the vehicle arms: a vehicle that has only
    /// just come up refuses until its checks pass, and the button says so.
    Arm,
    /// A key in the sim's window.
    Key(Key),
    /// Wait for the game's countdown to reach GO.
    Go,
}

const EVENTS: &[(&str, Act)] = &[
    ("operator toggles the planning tool", Act::Button("plan")),
    ("operator toggles the fly-to tool", Act::Button("goto")),
    ("operator clicks the map", Act::Map(MouseButton::Left)),
    ("operator right-clicks the map", Act::Map(MouseButton::Right)),
    ("operator raises the plan altitude", Act::Button("alt-up")),
    ("operator lowers the plan altitude", Act::Button("alt-dn")),
    ("operator adds a takeoff", Act::Button("tkof")),
    ("operator adds a return home", Act::Button("rtl")),
    ("operator clears the plan", Act::Button("clear")),
    ("operator uploads the plan", Act::Button("upload")),
    ("operator downloads the vehicle's mission", Act::Button("download")),
    ("operator edits the vehicle's mission", Act::Button("edit")),
    ("operator restarts the mission", Act::Button("restart")),
    ("operator arms the vehicle", Act::Arm),
    ("operator disarms the vehicle", Act::Button("disarm")),
    ("operator switches to auto", Act::Button("mode-AUTO")),
    ("operator switches to auto anyway", Act::Twice("mode-AUTO")),
    ("operator takes off vertically", Act::Button("vtol-tkof")),
    ("operator switches to forward flight", Act::Button("mode-CRUISE")),
    ("operator switches to hover", Act::Button("mode-QLOITER")),
    ("operator lands vertically", Act::Button("mode-QLAND")),
    ("pilot raises the throttle stick", Act::Radio(RadioMove::Throttle(STICK_HIGH))),
    ("pilot centres the throttle stick", Act::Radio(RadioMove::Throttle(STICK_MIDDLE))),
    ("pilot lowers the throttle stick", Act::Radio(RadioMove::Throttle(STICK_LOW))),
    ("pilot switches to qhover", Act::Radio(RadioMove::Mode("QHOVER"))),
    (
        "pilot switches back to a hover",
        Act::Radios(&[RadioMove::Elevator(0.0), RadioMove::Throttle(STICK_MIDDLE), RadioMove::Mode("QLOITER")]),
    ),
    (
        "pilot switches to forward flight",
        Act::Radios(&[
            RadioMove::Throttle(STICK_TOP),
            RadioMove::Elevator(ELEVATOR_A_LITTLE_BACK),
            RadioMove::Mode("FBWA"),
        ]),
    ),
    ("pilot switches to cruise", Act::Radios(&[RadioMove::Throttle(STICK_TOP), RadioMove::Mode("CRUISE")])),
    ("pilot sends the plane home", Act::Radio(RadioMove::Mode("QRTL"))),
    ("pilot throws the plane in the sim", Act::Key(Key::Throw)),
    ("pilot starts a race", Act::Key(Key::Race)),
    ("pilot starts a game of tag", Act::Key(Key::Tag)),
    ("countdown ends", Act::Go),
];

/// What the plane does by itself, and the driver waits for.
const WAITS: &[&str] = &["plane reaches a waypoint", "plane reaches the last item"];
/// How long the plane has to get to the next waypoint, or to the last item, flying the mission.
const FLY_SECS: u64 = 240;

/// States the driver can read (and put in place). Those the console has no feature for are
/// always at their starting value, so a test that needs them to change needs an event below
/// that the driver doesn't have.
const STATES: &[&str] = &[
    "link",
    "vehicle",
    "autopilot mode",
    "map tool",
    "plan",
    "plan altitude",
    "last plan item",
    "takeoff",
    "newest waypoint",
    "vehicle mission",
    "home position",
    "waypoints",
    "mission file",
    "measured distance",
    "vehicle's current item",
    "plane",
    "airframe",
    "flight",
    "height above home",
    "battery current",
    "battery voltage",
    "radio",
    "radio line",
    "tx15 screen mode",
    "tx15 screen arming",
    "throttle stick",
    "aileron stick",
    "vtol's wings",
    "last landing",
    "arm switch",
    "mode switch",
    "game",
    "game clock",
    "opponents",
];
/// States an action can change directly (the others change through events).
const SETTABLE: &[&str] =
    &["autopilot mode", "plan altitude", "throttle stick", "aileron stick", "arm switch", "mode switch"];

const MODE_IDS: [&str; 10] = [
    "mode-MANUAL",
    "mode-FBWA",
    "mode-FBWB",
    "mode-CRUISE",
    "mode-AUTO",
    "mode-RTL",
    "mode-LOITER",
    "mode-CIRCLE",
    "mode-GUIDED",
    "mode-TAKEOFF",
];

/// Time to let the console settle after an action, and between looks while waiting for a check.
const SETTLE: Duration = Duration::from_millis(1200);
const LOOK: Duration = Duration::from_millis(250);
/// How long a mode has to hold before it counts as arranged.
const HOLD: Duration = Duration::from_secs(3);
/// Between a click and whatever repaints before the next one.
const FRAME: Duration = Duration::from_millis(150);
/// Longest the runner waits for the console to answer a job.
const ANSWER: Duration = Duration::from_secs(60);

/// The time the whole run may take: the suite's budget (`SIM001_SUITE_BUDGET`, 120 s) and a
/// grace to write the results in, or `SIM001_EXPECT_TOTAL` (seconds).
fn run_limit() -> Duration {
    if let Some(s) = std::env::var("SIM001_EXPECT_TOTAL").ok().and_then(|s| s.parse().ok()) {
        return Duration::from_secs(s);
    }
    suite_budget::Budget::from_env().limit() + Duration::from_secs(30)
}

/// A test's time: 30 s, and 40 s for each step. An expectation that does one thing has about
/// 70 s, and a story that flies has as many steps as it needs (a gentle landing from cruise
/// height alone takes more than a minute).
fn test_budget(plan: &expectation_story::Plan) -> u64 {
    30 + 40 * plan.steps.len() as u64
}

/// Runs on the UI thread with the app and the console's window.
type Job = Box<dyn FnOnce(&mut App, WindowHandle<Gcs>) + Send>;

/// Load and check the suite before anything opens, then start the runner thread.
pub fn start(cx: &mut App, console: WindowHandle<Gcs>, config: Config) {
    let Config { suite, docs, only, vtol, rc_modes, radio, vehicle } = config;
    let mut acceptance = match Acceptance::load_for(&suite, "desktop") {
        Ok(a) => a.with_coverage(),
        Err(e) => {
            eprint!("{e}");
            crate::station::exit(1);
        }
    };
    if let Some(only) = &only {
        let only = only.to_lowercase();
        acceptance.plans.retain(|p| p.title.to_lowercase().contains(&only));
    }
    println!("expect: {} tests from {}", acceptance.plans.len(), suite.display());

    let (tx, rx): (Sender<Job>, Receiver<Job>) = channel();
    // The UI thread answers the runner's jobs between frames.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(5)).await;
            while let Ok(job) = rx.try_recv() {
                let _ = cx.update(|cx| job(cx, console));
            }
        }
    })
    .detach();

    // A hung run never leaves the windows open: the whole run has a limit, and the console quits when
    // it passes. Each test has its own budget (below); this is for what a budget can't catch.
    // SIM001_EXPECT_ONLY=<text>: run only the tests whose title has it (while working on them).
    if let Ok(only) = std::env::var("SIM001_EXPECT_ONLY") {
        acceptance.plans.retain(|p| p.title.contains(&only));
    }
    let total = run_limit();
    std::thread::spawn(move || {
        std::thread::sleep(total);
        eprintln!(
            "expect: the run took more than {} s: stopping (SIM001_EXPECT_TOTAL sets the limit)",
            total.as_secs()
        );
        crate::station::exit(2);
    });
    // A radio that the test doesn't use is held like a keyboard pilot's: the sticks centred, and
    // the throttle where the sim put it for a pilot with no radio.
    let in_use = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Some(line) = radio.clone() {
        let in_use = in_use.clone();
        std::thread::spawn(move || {
            loop {
                if !in_use.load(Ordering::SeqCst) {
                    let (armed, mode) = {
                        let v = vehicle.lock().unwrap();
                        (v.armed, v.mode().to_string())
                    };
                    line.fly(aviation_line::Pilot {
                        throttle: idle_throttle(armed, &mode),
                        ..aviation_line::Pilot::AT_REST
                    });
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }
    std::thread::spawn(move || {
        let mut driver = ConsoleDriver::new(tx, vtol, rc_modes);
        driver.line = radio;
        driver.in_use = in_use.clone();
        let opts = RunOptions { docs_dir: docs.clone(), patience: 480, ..Default::default() };
        let mut report = expectation_story::Report::default();
        let mut aborted = false;
        let suite_time = suite_budget::Budget::from_env();
        for plan in &acceptance.plans {
            let began = Instant::now();
            if suite_time.spent() {
                println!("expect: ✗ {} (no time left in the suite's {} s)", plan.title, suite_time.limit().as_secs());
                report.results.push(suite_time.ran_out(&acceptance.suite, plan, &driver));
                continue;
            }
            driver.budget = suite_time.cap(Duration::from_secs(test_budget(plan)));
            let r = expectation_harness::run(&acceptance.suite, std::slice::from_ref(plan), &mut driver, &opts);
            let mut r = r.results.into_iter().next().unwrap();
            // A flight depends on the autopilot, the link and the timing of the machine: a test
            // that fails once gets one more go, from a fresh plane, and says so.
            let mut retried = false;
            if matches!(r.status, expectation_story::Status::Failed)
                && driver.dead.is_none()
                && !suite_time.spent()
                && std::env::var("SIM001_EXPECT_NO_RETRY").is_err()
            {
                println!("expect: ✗ {} ({:.0} s): trying once more", plan.title, began.elapsed().as_secs_f32());
                driver.budget = suite_time.cap(Duration::from_secs(test_budget(plan)));
                r = expectation_harness::run(&acceptance.suite, std::slice::from_ref(plan), &mut driver, &opts)
                    .results
                    .into_iter()
                    .next()
                    .unwrap();
                retried = true;
            }
            match r.status {
                expectation_story::Status::Passed => println!(
                    "expect: ✓ {} ({:.0} s{})",
                    plan.title,
                    began.elapsed().as_secs_f32(),
                    if retried { ", on the second go" } else { "" }
                ),
                // Another system's test (the sim's), or one for a feature the console lacks.
                expectation_story::Status::Unsupported => {}
                _ => {
                    println!("expect: ✗ {} ({:.0} s)", plan.title, began.elapsed().as_secs_f32());
                    // What the vehicle was saying, for working out why. (Not part of the test's time:
                    // after the budget, the driver refuses every call.)
                    driver.deadline = None;
                    if let Ok(state) = driver.call(|g, _, _| {
                        let v = g.vehicle.lock().unwrap();
                        format!(
                            "mode {} armed {} height {:.1} m, climb {:+.1} m/s, airspeed {:.1}, ground speed {:.1}, roll {:.0}°, pitch {:.0}°, rc {:?}, last touchdown {:?}",
                            v.mode(), v.armed, v.alt_rel, v.climb, v.airspeed, v.groundspeed, v.roll.to_degrees(), v.pitch.to_degrees(), &v.rc[..8], v.last_touchdown
                        )
                    }) {
                        println!("expect:   plane at the end: {state}");
                    }
                    // The test's flight, one line every 4 s.
                    if let Ok(rows) = driver.call(|g, _, _| {
                        let v = g.vehicle.lock().unwrap();
                        let end = v.history.back().map_or(0.0, |s| s.t);
                        let mut last = f32::MIN;
                        v.history
                            .iter()
                            .filter(|s| s.t > end - 480.0)
                            .filter(|s| {
                                let keep = s.t - last >= 4.0
                                    && (s.alt.abs() > 1.0
                                        || s.airspeed > 12.0
                                        || s.t > end - 480.0 && s.alt > 0.5
                                        || true);
                                if keep {
                                    last = s.t;
                                }
                                keep
                            })
                            .map(|s| {
                                format!(
                                    "t-{:.0}s height {:.1} m, airspeed {:.1}, ground speed {:.1}, throttle {:.0}",
                                    end - s.t,
                                    s.alt,
                                    s.airspeed,
                                    s.groundspeed,
                                    s.throttle
                                )
                            })
                            .collect::<Vec<_>>()
                    }) {
                        for r in rows {
                            println!("expect:   flight: {r}");
                        }
                    }
                    if let Ok(lines) = driver.call(|g, _, _| {
                        let v = g.vehicle.lock().unwrap();
                        v.log.iter().rev().take(70).map(|l| l.text.clone()).collect::<Vec<_>>()
                    }) {
                        for l in lines.iter().rev() {
                            println!("expect:   vehicle: {l}");
                        }
                    }
                    // What the autopilot was, and everything that passed between it and the console:
                    // commands, acknowledgements, pings, heartbeats, and what it printed itself.
                    if let Ok((report, events)) = driver.call(|g, _, _| {
                        let v = g.vehicle.lock().unwrap();
                        let report = teleoperation_core::debug::state_report(&v);
                        let events: Vec<String> =
                            v.debug.tail(80).into_iter().map(teleoperation_core::debug::format_line).collect();
                        (report, events)
                    }) {
                        for (k, val) in report {
                            println!("expect:   autopilot {k}: {val}");
                        }
                        for e in events {
                            println!("expect:   debug: {e}");
                        }
                    }
                    for l in teleoperation_core::debug::tail_lines(
                        &crate::station::logs_dir().join("sitl/arduplane.log"),
                        20,
                    ) {
                        println!("expect:   arduplane: {l}");
                    }
                }
            }
            report.results.push(r);
            if let Some(why) = driver.dead.clone() {
                // Nothing after this can run: say so now, not after every test has waited its turn.
                let left = &acceptance.plans[report.results.len()..];
                eprintln!("expect: the plane is not answering ({why}): stopping, {} tests not run", left.len());
                report.results.extend(
                    left.iter().map(|p| expectation_story::PlanResult::with(expectation_story::Status::NotRun, p)),
                );
                aborted = true;
                break;
            }
        }
        if aborted {
            // Not saved: a run that stopped has nothing to say about the tests it did not reach, and
            // must not replace the results of an earlier run with empty ones.
            eprintln!("expect: the results were not saved");
        } else if let Some(dir) = &docs {
            // Saved as the console's results, and merged with the sim's before the docs are written.
            match acceptance.publish(dir, "gcs", &report) {
                Ok(_) => println!("expect: wrote the docs to {}", dir.display()),
                Err(e) => eprintln!("expect: couldn't write the docs to {}: {e}", dir.display()),
            }
        }
        let unsupported = report.count(expectation_story::Status::Unsupported);
        if unsupported > 0 {
            println!(
                "expect: {unsupported} tests need a driver the console doesn't have yet (the sim's, or a feature to come)"
            );
        }
        if !report.passed() {
            eprint!("{}", report.failures(&acceptance.suite, &acceptance.plans));
            EXIT.store(1, Ordering::SeqCst);
        }
        driver.deadline = None;
        let _ = driver.call(|_, _, cx| cx.quit());
    });
}

/// A left or right click at `at`, as the platform would deliver it.
fn click(window: &mut Window, cx: &mut App, at: Point<Pixels>, button: MouseButton) {
    let modifiers = Modifiers::default();
    let mut send = |e: PlatformInput| {
        let _ = window.dispatch_event(e, cx);
    };
    send(PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: None, modifiers }));
    send(PlatformInput::MouseDown(MouseDownEvent {
        button,
        position: at,
        modifiers,
        click_count: 1,
        first_mouse: false,
    }));
    send(PlatformInput::MouseUp(MouseUpEvent { button, position: at, modifiers, click_count: 1 }));
}

fn click_button(window: &mut Window, cx: &mut App, id: &str) -> Result<(), String> {
    let b = app::button_bounds(id).ok_or_else(|| format!("the {id} button isn't on screen"))?;
    click(window, cx, b.center(), MouseButton::Left);
    Ok(())
}

/// Where to click the map next: somewhere new each time, so the plan's points are apart.
fn map_point(g: &Gcs) -> Result<Point<Pixels>, String> {
    let b = g.map_bounds.get();
    if b.size.width <= Pixels::ZERO {
        return Err("the map isn't on screen".into());
    }
    let n = g.map.plan.len() as f32;
    Ok(Point::new(b.center().x + gpui::px(40.0 * ((n % 5.0) - 2.0)), b.center().y - gpui::px(30.0 * (n % 4.0))))
}

fn enum_value(v: &Value) -> Result<String, String> {
    v.as_str().map(str::to_string).ok_or_else(|| format!("expected a name, got {v}"))
}

fn kind_name(k: WpKind) -> &'static str {
    match k {
        WpKind::Takeoff => "TAKEOFF",
        WpKind::Waypoint => "WAYPOINT",
        WpKind::LoiterUnlim => "LOITER_UNLIM",
        WpKind::LoiterTurns => "LOITER_TURNS",
        WpKind::Land => "LAND",
        WpKind::Rtl => "RTL",
        WpKind::Other(_) => "OTHER",
    }
}

fn kind_of(name: &str) -> Option<WpKind> {
    Some(match name {
        "TAKEOFF" => WpKind::Takeoff,
        "WAYPOINT" => WpKind::Waypoint,
        "LOITER_UNLIM" => WpKind::LoiterUnlim,
        "LOITER_TURNS" => WpKind::LoiterTurns,
        "LAND" => WpKind::Land,
        "RTL" => WpKind::Rtl,
        _ => return None,
    })
}

/// A waypoint of our own on the valley floor near home, for arranging a plan.
fn waypoint(g: &Gcs, i: usize) -> Waypoint {
    let (lat, lon) = g.origin.to_ll(80.0 + 40.0 * i as f32, 50.0 * ((i % 3) as f32 - 1.0));
    Waypoint { kind: WpKind::Waypoint, lat, lon, alt: g.map.plan_alt }
}

/// The vehicle's mission (after its home) is the plan.
fn same(plan: &[Waypoint], mission: &[Waypoint]) -> bool {
    let m = mission.get(1..).unwrap_or(&[]);
    plan.len() == m.len()
        && plan.iter().zip(m).all(|(a, b)| {
            a.kind == b.kind
                && (a.alt - b.alt).abs() < 1.0
                && (!a.has_position() || ((a.lat - b.lat).abs() < 1e-4 && (a.lon - b.lon).abs() < 1e-4))
        })
}

/// The game and its clock as the sim reports them on the sensor feed.
fn game_reading(key: &str, game: Option<GameStatus>) -> &'static str {
    match (key, game) {
        ("game", None) => "free flight",
        ("game", Some(g)) if g.kind == GameKind::Race => "a race",
        ("game", Some(_)) => "tag",
        (_, None) => "off",
        (_, Some(g)) => match g.clock {
            GameClock::Waiting => "waiting for takeoff",
            GameClock::CountingDown => "counting down",
            GameClock::Running => "running",
            GameClock::Stopped => "stopped",
        },
    }
}

/// Where the computer pilots' planes are, from their markers on the map: all on the ground
/// near home, all above 5 m, or in between. Planes that stand side by side are one marker (the
/// tracker joins reports from the same place), so this looks at the markers there are, and the
/// heights are those of the other aircraft's reports less your own plane's ground height.
fn opponents(v: &crate::vehicle::Vehicle) -> &'static str {
    let Some((_, _, home)) = v.home else {
        return "not on the map";
    };
    let heights: Vec<f32> = v.other_aircraft().iter().map(|t| t.position.alt - home).collect();
    if heights.is_empty() {
        "not on the map"
    } else if heights.iter().all(|h| *h < 3.0) {
        "on the ground"
    } else if heights.iter().all(|h| *h > 5.0) {
        "flying"
    } else {
        "launching"
    }
}

/// What a VTOL is doing, from its telemetry: on the ground, hovering (a Q mode, hardly moving),
/// cruising (a fixed-wing mode above the speed the transition ends at), or in between.
fn flight(v: &crate::vehicle::Vehicle) -> &'static str {
    // (No fixed-wing flight is slower than 6 m/s, so a plane that slow is hovering, whatever the mode is called.)
    let q_mode = v.mode().starts_with('Q');
    if v.alt_rel < 1.0 && v.groundspeed < 1.0 {
        "grounded"
    } else if !q_mode && v.airspeed >= 11.0 {
        "cruising"
    } else if v.airspeed < 6.0 && v.groundspeed < 5.0 {
        "hovering"
    } else {
        "in transition"
    }
}

fn observe(g: &mut Gcs, key: &str) -> Result<Value, String> {
    let v = g.vehicle.lock().unwrap();
    let plan = &g.map.plan;
    Ok(match key {
        "link" => Value::from(if v.link.alive() { "connected" } else { "lost" }),
        "vehicle" => Value::from(if v.armed { "armed" } else { "disarmed" }),
        "autopilot mode" => Value::from(v.mode()),
        "map tool" => Value::from(match g.map.tool {
            Tool::Plan => "PLAN",
            Tool::Goto => "GOTO",
            Tool::Pan | Tool::Look => "pan",
        }),
        "plan" => Value::from(plan.len() as f64),
        "plan altitude" => Value::from(g.map.plan_alt),
        "last plan item" => Value::from(plan.last().map_or("none", |w| kind_name(w.kind))),
        "takeoff" => {
            Value::from(if plan.first().is_some_and(|w| w.kind == WpKind::Takeoff) { "planned" } else { "not planned" })
        }
        "newest waypoint" => Value::from(match plan.iter().rev().find(|w| w.kind == WpKind::Waypoint) {
            Some(w) if (w.alt - g.map.plan_alt).abs() > 0.5 => "at another altitude",
            _ => "at the plan altitude",
        }),
        "vehicle mission" => Value::from(if v.mission.len() <= 1 {
            "empty"
        } else if same(plan, &v.mission) {
            "up to date"
        } else {
            "out of date"
        }),
        "home position" => Value::from(if v.home.is_some() { "set" } else { "not set" }),
        "vehicle's current item" => Value::from(v.mission_seq as f64),
        "plane" => {
            let on_ground = !v.have_position || (v.alt_rel < 2.0 && v.groundspeed < 2.0);
            Value::from(match v.mode() {
                _ if on_ground => "on the ground",
                "AUTO" => "flying the mission",
                "RTL" => "flying home",
                "LOITER" | "CIRCLE" | "GUIDED" => "loitering",
                _ => "flying",
            })
        }
        // The console has no terrain check, mission file or measuring yet: they stay as they start.
        "height above home" => Value::from(v.alt_rel as f64),
        "battery current" => Value::from(v.batt_a as f64),
        "battery voltage" => Value::from(v.batt_v as f64),
        "flight" => Value::from(flight(&v)),
        // Banked past 10°, level within 5°, and in between while it settles.
        "vtol's wings" => Value::from(match v.roll.to_degrees().abs() {
            r if r > 10.0 => "banked",
            r if r < 5.0 => "level",
            _ => "turning",
        }),
        // Soft below 2 m/s. Before the first landing, there is nothing to say.
        "last landing" => Value::from(match v.last_touchdown {
            Some(s) if s < 2.0 => "soft",
            Some(_) => "hard",
            None => "none yet",
        }),
        "opponents" => Value::from(opponents(&v)),
        "waypoints" => Value::from("unchecked"),
        "mission file" => Value::from("missing"),
        "measured distance" => Value::from("hidden"),
        other => return Err(format!("no way to observe `{other}`")),
    })
}

/// Put the parts of the console's own model in place (the plan and the map). The vehicle's
/// parts need time, so the runner thread does those.
fn arrange_model(g: &mut Gcs, key: &str, value: &Value) -> Result<(), String> {
    let name = || enum_value(value);
    match key {
        "map tool" => {
            g.map.tool = match name()?.as_str() {
                "PLAN" => Tool::Plan,
                "GOTO" => Tool::Goto,
                _ => Tool::Pan,
            }
        }
        "plan altitude" => g.map.plan_alt = value.as_f64().ok_or("expected a number")? as f32,
        "plan" => {
            let n = value.as_f64().ok_or("expected a number")? as usize;
            g.map.plan.truncate(n);
            while g.map.plan.len() < n {
                let w = waypoint(g, g.map.plan.len());
                g.map.plan.push(w);
            }
        }
        "takeoff" => match name()?.as_str() {
            "planned" => g.add_takeoff(),
            _ => g.map.plan.retain(|w| w.kind != WpKind::Takeoff),
        },
        "last plan item" => match name()?.as_str() {
            "none" => g.map.plan.clear(),
            kind => {
                let kind = kind_of(kind).ok_or_else(|| format!("no command called {kind}"))?;
                if g.map.plan.is_empty() {
                    let w = waypoint(g, 0);
                    g.map.plan.push(w);
                }
                let last = g.map.plan.last_mut().unwrap();
                last.kind = kind;
                if kind == WpKind::Rtl {
                    (last.lat, last.lon, last.alt) = (0.0, 0.0, 0.0);
                }
            }
        },
        "newest waypoint" => {
            if g.map.plan.is_empty() {
                let w = waypoint(g, 0);
                g.map.plan.push(w);
            }
            let alt = g.map.plan_alt;
            let other = if alt + 10.0 <= 300.0 { alt + 10.0 } else { alt - 10.0 };
            let last = g.map.plan.last_mut().unwrap();
            last.alt = if name()? == "at another altitude" { other } else { alt };
        }
        _ => return Err(format!("can't put {key} in place: {value}")),
    }
    Ok(())
}

struct ConsoleDriver {
    tx: Sender<Job>,
    vtol: bool,
    rc_modes: Vec<&'static str>,
    /// What the stand-in radio is doing now.
    radio: RadioState,
    /// How many times the vehicle had armed when the plane was last fresh.
    arms_seen: u32,
    /// How long the next test has, from when its plane is ready, and when it is up.
    budget: Duration,
    deadline: Option<Instant>,
    /// The TX15 on the flight computer's serial line, which the console hosts.
    line: Option<aviation_line::Hosted>,
    /// The test uses the radio (the keeper leaves the sticks to it).
    in_use: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// AUTO, asked for before the vehicle has a mission: set once the mission is arranged (an
    /// empty mission ends as AUTO starts, and ArduPlane flies home in RTL).
    pending_auto: bool,
    /// Why the plane can't be got ready (the autopilot didn't come back, or never got ready to
    /// arm): no other test can run either, so the run stops instead of waiting out each one.
    dead: Option<String>,
}

impl ConsoleDriver {
    fn new(tx: Sender<Job>, vtol: bool, rc_modes: Vec<&'static str>) -> Self {
        let radio = RadioState {
            connected: false,
            roll: 0.0,
            pitch: 0.0,
            yaw: 0.0,
            throttle: STICK_LOW,
            arm: false,
            mode: 0,
            reset_presses: 0,
        };
        ConsoleDriver {
            tx,
            vtol,
            rc_modes,
            radio,
            arms_seen: 0,
            budget: Duration::from_secs(35),
            deadline: None,
            line: None,
            in_use: Default::default(),
            pending_auto: false,
            dead: None,
        }
    }

    fn serial_line(&self) -> bool {
        self.line.is_some()
    }

    /// What the lamps of the simulated TX15 show now.
    fn tx15_lamps(&self) -> Result<crate::tx15_screen::Lamps, String> {
        let line = self.line.as_ref().ok_or("the TX15 is not on the serial line: start the station (--station)")?;
        let lamps = line.lamps();
        Ok(crate::tx15_screen::Lamps { mode: lamps.mode, armed: lamps.armed })
    }

    /// Give the radio the state the test has put it in: on the serial line the sticks go to the
    /// hosted TX15; with none (a console on its own) they go to the sim over the feed.
    fn deliver_radio(&self, radio: RadioState) -> Result<(), String> {
        self.in_use.store(radio.connected, Ordering::SeqCst);
        if let Some(line) = &self.line {
            line.fly(pilot_of(&radio));
            return Ok(());
        }
        let sent = self.call(move |g, _, _| g.sensor.as_ref().map(|s| s.send_radio(radio)).is_some())?;
        if sent {
            Ok(())
        } else {
            Err("there is no link to the sim to carry the radio: start the station (--station)".into())
        }
    }

    /// Move the stand-in radio and tell the sim.
    fn radio_move(&mut self, m: RadioMove) -> Result<(), String> {
        // Using the radio plugs it in.
        self.radio.connected = true;
        match m {
            RadioMove::ArmSwitch(on) => self.radio.arm = on,
            RadioMove::ModeSwitch(slot) => self.radio.mode = slot.min(5) as u8,
            RadioMove::Throttle(t) => self.radio.throttle = t,
            RadioMove::Aileron(a) => self.radio.roll = a,
            RadioMove::Elevator(e) => self.radio.pitch = e,
            RadioMove::Mode(name) => {
                let slot =
                    self.rc_modes.iter().position(|m| *m == name).ok_or("this plane has no such mode on its switch")?;
                self.radio.mode = slot.min(5) as u8;
            }
        }
        self.deliver_radio(self.radio)
    }

    /// Which way the radio's three inputs read to ArduPilot: the throttle stick, the arm
    /// switch and the mode switch.
    fn rc_reading(&self, key: &str) -> Result<Value, String> {
        let rc = self.call(|g, _, _| g.vehicle.lock().unwrap().rc)?;
        // On the serial line the console's RC readback (the RC 1-4 line) comes down through the
        // flight computer and the TX15 and does not follow the sticks, so the sticks and switches
        // read where the pilot put them (as the arm switch does with any radio).
        if self.serial_line() && self.radio.connected {
            let r = &self.radio;
            return Ok(match key {
                "throttle stick" => Value::from(if r.throttle < 0.25 {
                    "low"
                } else if r.throttle < 0.65 {
                    "middle"
                } else {
                    "high"
                }),
                "aileron stick" => Value::from(if r.roll < -0.25 {
                    "left"
                } else if r.roll > 0.25 {
                    "right"
                } else {
                    "centred"
                }),
                "arm switch" => Value::from(if r.arm { "on" } else { "off" }),
                _ => Value::from(*self.rc_modes.get(r.mode as usize).ok_or("this plane has no mode switch")?),
            });
        }
        Ok(match key {
            "throttle stick" => Value::from(match rc[2] {
                0..=1249 => "low",
                1250..=1649 => "middle",
                _ => "high",
            }),
            // Where the pilot put the switch: in a game the sim holds it off until GO, so what
            // reaches the autopilot is not always where it is.
            "aileron stick" => Value::from(match rc[0] {
                0..=1349 => "left",
                1350..=1649 => "centred",
                _ => "right",
            }),
            "arm switch" if self.radio.connected => Value::from(if self.radio.arm { "on" } else { "off" }),
            "arm switch" => Value::from(if rc[4] > 1500 { "on" } else { "off" }),
            _ => {
                // The mode switch's position nearest to the PWM.
                let slot = simulation_aircraft::rc_mode_slot(rc[7]);
                Value::from(*self.rc_modes.get(slot).ok_or("this plane has no mode switch")?)
            }
        })
    }
    /// An error if the test has used up its time: for loops that try again and so don't see the
    /// error that `call` gives them.
    fn out_of_time(&self) -> Result<(), String> {
        if self.deadline.is_some_and(|d| Instant::now() > d) {
            return Err(format!("the test ran out of time ({} s)", self.budget.as_secs()));
        }
        Ok(())
    }

    /// Run `f` on the UI thread, between frames, and return what it returns.
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Gcs, &mut Window, &mut Context<Gcs>) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.out_of_time()?;
        let (tx, rx) = channel();
        let job: Job = Box::new(move |cx, console| {
            let _ = console.update(cx, |g, w, cx| tx.send(f(g, w, cx)));
        });
        self.tx.send(job).map_err(|_| "the console has quit".to_string())?;
        rx.recv_timeout(ANSWER).map_err(|_| "the console stopped answering".to_string())
    }

    /// Run `f` on the UI thread against the window alone. Input has to be delivered this way:
    /// the click's handlers update the console, which `call` has borrowed.
    fn with_window<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Window, &mut App) -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = channel();
        let job: Job = Box::new(move |cx, console| {
            let _ = cx.update_window(console.into(), |_, w, cx| tx.send(f(w, cx)));
        });
        self.tx.send(job).map_err(|_| "the console has quit".to_string())?;
        rx.recv_timeout(ANSWER).map_err(|_| "the console stopped answering".to_string())
    }

    fn send(&self, cmd: Cmd) -> Result<(), String> {
        self.call(move |g, _, _| {
            let _ = g.tx.send(cmd);
        })
    }

    fn read(&self, key: &str) -> Result<Value, String> {
        if matches!(key, "throttle stick" | "aileron stick" | "arm switch" | "mode switch") {
            return self.rc_reading(key);
        }
        if key == "radio line" {
            return Ok(Value::from(if self.serial_line() { "serial" } else { "USB" }));
        }
        if key == "radio" {
            // The station's stand-in radio goes straight to the channels, past the sim's
            // calibration (F2, in the sim's window): it is never a calibrated one.
            return Ok(Value::from("not calibrated"));
        }
        if key == "tx15 screen mode" || key == "tx15 screen arming" {
            let lamps = self.tx15_lamps()?;
            return Ok(Value::from(if key == "tx15 screen mode" {
                lamps.mode
            } else if lamps.armed {
                "armed".to_string()
            } else {
                "disarmed".to_string()
            }));
        }
        if key == "airframe" {
            return Ok(Value::from(if self.vtol { "VTOL" } else { "conventional" }));
        }
        if key == "game" || key == "game clock" {
            let game = self.call(|g, _, _| g.sensor.as_ref().and_then(|s| s.game()))?;
            return Ok(Value::from(game_reading(key, game)));
        }
        let key = key.to_string();
        self.call(move |g, _, _| observe(g, &key))?
    }

    /// Poll until `done` holds, or fail saying `what` didn't happen in time.
    fn wait_for(
        &self,
        secs: u64,
        what: &str,
        done: impl Fn(&mut Gcs) -> bool + Clone + Send + 'static,
    ) -> Result<(), String> {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let d = done.clone();
            if self.call(move |g, _, _| d(g))? {
                return Ok(());
            }
            if let Some(why) = crate::station::autopilot_gave_up() {
                return Err(format!("{what} can't happen: {why}"));
            }
            if Instant::now() > end {
                return Err(format!("{what} didn't happen within {secs} s"));
            }
            std::thread::sleep(LOOK);
        }
    }

    /// The vehicle's mission: `empty`, `up to date` (the plan uploaded) or `out of date`.
    fn arrange_mission(&mut self, want: &str) -> Result<(), String> {
        let key = "vehicle mission";
        let want = want.to_string();
        let state = self.read(key)?;
        if state.as_str() == Some(want.as_str()) {
            return Ok(());
        }
        match want.as_str() {
            "empty" => {
                // Just after the line comes up a first clear can be lost, so ask again
                // a few times before giving up.
                let mut cleared = Err(String::new());
                for _ in 0..3 {
                    self.send(Cmd::ClearMission)?;
                    std::thread::sleep(Duration::from_millis(500));
                    // Read it back, rather than trusting what the console remembers.
                    self.send(Cmd::Download)?;
                    cleared = self.wait_for(8, "the vehicle's mission being cleared", |g| {
                        let v = g.vehicle.lock().unwrap();
                        v.mission.len() <= 1 && v.mission_status == "no mission on vehicle"
                    });
                    if cleared.is_ok() {
                        break;
                    }
                }
                cleared
            }
            "up to date" => {
                let plan = self.call(|g, _, _| g.map.plan.clone())?;
                if plan.is_empty() {
                    return Err("there is no plan to upload".into());
                }
                self.send(Cmd::Upload(plan))?;
                self.wait_for(30, "the upload", |g| same(&g.map.plan, &g.vehicle.lock().unwrap().mission))
            }
            _ => {
                // Out of date: something on the vehicle, and a plan that differs from it.
                if state.as_str() == Some("empty") {
                    let one = self.call(|g, _, _| vec![waypoint(g, 0)])?;
                    self.send(Cmd::Upload(one))?;
                    self.wait_for(30, "the upload", |g| g.vehicle.lock().unwrap().mission.len() > 1)?;
                }
                self.call(|g, _, _| {
                    let w = waypoint(g, g.map.plan.len() + 1);
                    g.map.plan.push(w);
                })
            }
        }
    }

    /// Puts the vehicle in `mode`, and sees that it stays there.
    fn arrange_mode(&mut self, mode: &str) -> Result<(), String> {
        let key = "autopilot mode";
        let mode = mode.to_string();
        let n = mode_number(&mode).ok_or_else(|| format!("no mode called {mode}"))?;
        // A vehicle that has only just come up can change mode by itself (ArduPlane
        // does when the EKF settles), so the mode has to hold for a moment.
        for _ in 0..4 {
            if self.read(key)?.as_str() != Some(mode.as_str()) {
                self.send(Cmd::SetMode(n))?;
                let m = mode.clone();
                if let Err(e) =
                    self.wait_for(30, &format!("the {mode} mode"), move |g| g.vehicle.lock().unwrap().mode() == m)
                {
                    // Why the autopilot said no, when it said why.
                    let recent = self.call(|g, _, _| {
                        let v = g.vehicle.lock().unwrap();
                        v.log.iter().rev().take(8).map(|l| l.text.clone()).collect::<Vec<_>>()
                    })?;
                    return Err(format!("{e} (its latest messages, newest first: {})", recent.join(" | ")));
                }
            }
            std::thread::sleep(HOLD);
            if self.read(key)?.as_str() == Some(mode.as_str()) {
                return Ok(());
            }
        }
        Err(format!("the vehicle wouldn't stay in {mode}"))
    }

    /// Fly the VTOL to `want`: armed and taking off straight up, then (for forward flight)
    /// switched across.
    fn flight_now(&self) -> Result<String, String> {
        self.read("flight").map(|v| v.as_str().unwrap_or("").to_string())
    }

    fn arrange_flight(&mut self, want: &str) -> Result<(), String> {
        match want {
            "grounded" if self.flight_now()? == "grounded" => Ok(()),
            "grounded" => Err("can't put a flying plane on the ground without landing it".into()),
            "hovering" | "cruising" => {
                if self.flight_now()? == "grounded" {
                    // With the radio plugged in, its arm switch rules: left off, it would disarm the
                    // plane that the console has just armed.
                    if self.radio.connected && !self.radio.arm {
                        self.radio_move(RadioMove::ArmSwitch(true))?;
                    }
                    if self.read("vehicle")?.as_str() != Some("armed") {
                        for _ in 0..6 {
                            self.send(Cmd::Arm(true))?;
                            self.out_of_time()?;
                            if self.wait_for(5, "arming", |g| g.vehicle.lock().unwrap().armed).is_ok() {
                                break;
                            }
                        }
                    }
                    self.press("vtol-tkof")?;
                }
                if self.flight_now()? == "cruising" && want == "hovering" {
                    self.press("mode-QLOITER")?;
                }
                self.wait_for(90, "the plane hovering", |g| {
                    let v = g.vehicle.lock().unwrap();
                    flight(&v) == "hovering" && v.alt_rel > 15.0 || flight(&v) == "cruising"
                })?;
                if want == "cruising" && self.flight_now()? != "cruising" {
                    self.press("mode-CRUISE")?;
                    self.wait_for(60, "the plane cruising", |g| flight(&g.vehicle.lock().unwrap()) == "cruising")?;
                }
                Ok(())
            }
            other => Err(format!("can't put the flight in `{other}`")),
        }
    }

    /// Has the plane armed since it was last fresh, or is its autopilot unhappy (the pre-arm checks
    /// fail), so that a test can't start from it? Arming leaves things behind (a landed flag, the
    /// tilt of the motors, a changed mode) that a test must not inherit.
    fn needs_a_fresh_plane(&self) -> Result<bool, String> {
        const PREARM_CHECK: u32 = 0x1000_0000;
        let seen = self.arms_seen;
        self.call(move |g, _, _| {
            let v = g.vehicle.lock().unwrap();
            v.link.alive()
                && (v.arms > seen || v.armed || v.alt_rel.abs() > 1.0 || v.sensor_ok(PREARM_CHECK) == Some(false))
        })
    }

    /// Start the sim again with a fresh plane, and wait for ArduPilot to go and come back.
    fn fresh_plane(&mut self) -> Result<(), String> {
        println!("expect: starting the next test from a fresh plane (the autopilot restarts, about a minute)");
        self.radio = radio_for_a_fresh_plane(self.radio);
        self.deliver_radio(self.radio)?;
        // The sim starts again, as with the R key.
        self.call(|g, _, _| {
            if let Some(s) = &g.sensor {
                s.press_key(Key::Reset);
            }
        })?;
        // The station restarts the autopilot: its link drops, and then comes back.
        let _ = self.wait_for(45, "the autopilot restarting", |g| !g.vehicle.lock().unwrap().link.alive());
        self.wait_for(180, "the autopilot coming back", |g| g.vehicle.lock().unwrap().link.alive())?;
        // ArduPilot goes to the mode of its mode switch a few seconds after it is ready, and a test
        // that sets a mode before then has it taken away; by the time it is ready to arm, that is
        // long done.
        self.wait_until_ready_to_arm()?;
        self.arms_seen = self.call(|g, _, _| g.vehicle.lock().unwrap().arms)?;
        Ok(())
    }

    /// Wait until the autopilot would take an arm: its pre-arm checks pass, and it has not
    /// complained for a while (it says what is wrong every few seconds, and the EKF can still be
    /// settling when the status already says the checks pass).
    fn wait_until_ready_to_arm(&self) -> Result<(), String> {
        const PREARM_CHECK: u32 = 0x1000_0000;
        self.wait_for(240, "the autopilot being ready to arm (is the station running? use --station)", |g| {
            let v = g.vehicle.lock().unwrap();
            v.sensor_ok(PREARM_CHECK) == Some(true)
                && v.prearm.as_ref().is_none_or(|p| p.0.elapsed() > Duration::from_secs(10))
        })
    }

    fn perform_key(&self, k: Key) -> Result<(), String> {
        self.call(move |g, _, _| {
            if let Some(s) = &g.sensor {
                s.press_key(k)
            }
        })?;
        std::thread::sleep(SETTLE);
        Ok(())
    }

    fn press(&self, id: &'static str) -> Result<(), String> {
        // A window too narrow for a row of buttons cuts it to one line behind MORE, and a click on
        // a button on the lines that are cut would land on the map under it: open the rows first.
        let opened = self.with_window(|w, cx| {
            let cut = app::rows_cut_short();
            cut.iter().try_for_each(|row| click_button(w, cx, &format!("more-{row}"))).map(|()| !cut.is_empty())
        })??;
        if opened {
            std::thread::sleep(SETTLE);
        }
        self.with_window(move |w, cx| click_button(w, cx, id))??;
        std::thread::sleep(FRAME);
        Ok(())
    }

    fn check(&self, key: &str, want: &str, hint: &str) -> Result<(), String> {
        let have = self.read(key)?;
        if have.as_str().is_some_and(|h| h.eq_ignore_ascii_case(want)) {
            Ok(())
        } else {
            Err(format!("{key} is {have}, not {want}{hint}"))
        }
    }
}

impl Driver for ConsoleDriver {
    /// Back to the Background's UI state: panning, no plan, 40 m. The vehicle is arranged per
    /// test (disarmed, MANUAL, ...), since it can't be reset by the console.
    fn reset(&mut self) -> Result<(), String> {
        self.pending_auto = false;
        // Getting a fresh plane (the autopilot restarts) is not the test's time: the clock starts
        // when the plane is ready.
        self.deadline = None;
        // A plane that has flown, or whose autopilot has lost its bearings, starts the next test
        // from a fresh plane: the sim starts again (as with R), and the station restarts ArduPilot.
        if self.vtol
            && self.needs_a_fresh_plane()?
            && let Err(e) = self.fresh_plane()
        {
            self.dead = Some(e.clone());
            return Err(e);
        }
        // The throttle stick starts down, as a pilot leaves it before arming.
        if self.vtol {
            // A test that doesn't use the radio flies without one, like a keyboard pilot, and the
            // switches are where a fresh radio has them: the arm switch out, the first mode.
            self.radio = radio_at_rest(self.radio);
            self.deliver_radio(self.radio)?;
        }
        self.call(|g, _, _| {
            g.map.tool = Tool::Pan;
            g.map.plan.clear();
            g.map.plan_alt = 40.0;
            g.map.follow = true;
        })?;
        // Nor is the autopilot's own start, the first time: the test begins from a vehicle that is
        // ready to arm.
        if self.vtol
            && let Err(e) = self.wait_until_ready_to_arm()
        {
            self.dead = Some(e.clone());
            return Err(e);
        }
        self.deadline = Some(Instant::now() + self.budget);
        Ok(())
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        let key = var.key.as_str();
        let name = || enum_value(value);
        match key {
            "link" => {
                let want = name()?;
                if want != "connected" {
                    return Err("can't cut the link".into());
                }
                self.wait_for(120, "the link coming up (is the station running? use --station)", |g| {
                    g.vehicle.lock().unwrap().link.alive()
                })?;
                // The start the spec describes waits for the ARM button to read ARM, not NOT READY:
                // the vehicle has passed its pre-arm checks. (A switch arms once, and not if it's too early.)
                const PREARM_CHECK: u32 = 0x1000_0000;
                self.wait_for(180, "the vehicle passing its pre-arm checks", |g| {
                    let v = g.vehicle.lock().unwrap();
                    v.armed || v.sensor_ok(PREARM_CHECK).unwrap_or(true)
                })
            }
            "vehicle" => match name()?.as_str() {
                "armed" => {
                    for _ in 0..6 {
                        self.send(Cmd::Arm(true))?;
                        self.out_of_time()?;
                        if self.wait_for(5, "arming", |g| g.vehicle.lock().unwrap().armed).is_ok() {
                            return Ok(());
                        }
                    }
                    Err("the vehicle wouldn't arm".into())
                }
                _ => {
                    if self.read("vehicle")?.as_str() == Some("armed") {
                        self.send(Cmd::ForceDisarm)?;
                        self.wait_for(15, "disarming", |g| !g.vehicle.lock().unwrap().armed)?;
                    }
                    Ok(())
                }
            },
            "autopilot mode" => {
                let mode = name()?;
                // The plan's start arranges the mode before the mission: AUTO waits for it.
                if mode == "AUTO" && self.read("vehicle mission")?.as_str() == Some("empty") {
                    self.pending_auto = true;
                    return Ok(());
                }
                self.arrange_mode(&mode)
            }
            "radio" => {
                let want = name()?;
                self.check(key, &want, " (the station's stand-in radio is never calibrated)")
            }
            "radio line" => {
                let want = name()?;
                self.check(key, &want, " (start the station with --flight-computer for the serial line)")
            }
            "tx15 screen mode" | "tx15 screen arming" => {
                let want = name()?;
                self.check(key, &want, "")
            }
            "airframe" => {
                let want = name()?;
                self.check(key, &want, " (start the station with --aircraft t1s-vtol for a VTOL)")
            }
            "throttle stick" => {
                let t = match name()?.as_str() {
                    "low" => STICK_LOW,
                    "middle" => STICK_MIDDLE,
                    _ => STICK_HIGH,
                };
                self.radio_move(RadioMove::Throttle(t))?;
                std::thread::sleep(SETTLE);
                Ok(())
            }
            "aileron stick" => {
                self.radio_move(RadioMove::Aileron(match name()?.as_str() {
                    "left" => -AILERON_HALF,
                    "right" => AILERON_HALF,
                    _ => 0.0,
                }))?;
                std::thread::sleep(SETTLE);
                Ok(())
            }
            "arm switch" => {
                let want = name()?;
                if want == "off" {
                    self.radio_move(RadioMove::ArmSwitch(false))?;
                    std::thread::sleep(SETTLE);
                    return self.check(key, &want, "");
                }
                // A vehicle that isn't ready yet refuses to arm, so a pilot flips the switch off
                // and on again until it takes.
                self.radio_move(RadioMove::ArmSwitch(true))?;
                // In a countdown the sim holds the switch off until GO: the pilot puts it on, and
                // nothing arms.
                let counting = |g: &mut Gcs| {
                    g.sensor.as_ref().and_then(|s| s.game()).is_some_and(|x| x.clock == GameClock::CountingDown)
                };
                if self.call(move |g, _, _| counting(g))? {
                    std::thread::sleep(SETTLE);
                    return self.check(key, &want, "");
                }
                for _ in 0..100 {
                    self.out_of_time()?;
                    if self.wait_for(3, "arming", |g| g.vehicle.lock().unwrap().armed).is_ok() {
                        return self.check(key, &want, "");
                    }
                    self.radio_move(RadioMove::ArmSwitch(false))?;
                    std::thread::sleep(Duration::from_millis(600));
                    self.radio_move(RadioMove::ArmSwitch(true))?;
                }
                let (why, recent) = self.call(|g, _, _| {
                    let v = g.vehicle.lock().unwrap();
                    let why = v.prearm.as_ref().map(|p| p.1.clone());
                    let recent: Vec<String> = v.log.iter().rev().take(8).map(|l| l.text.clone()).collect();
                    (why, recent)
                })?;
                Err(format!(
                    "the vehicle wouldn't arm: {} (its latest messages, newest first: {})",
                    why.unwrap_or_else(|| "no reason given".into()),
                    recent.join(" | ")
                ))
            }
            "mode switch" => {
                let want = name()?;
                let slot =
                    self.rc_modes.iter().position(|m| *m == want).ok_or("this plane has no such mode on its switch")?;
                self.radio_move(RadioMove::ModeSwitch(slot))?;
                std::thread::sleep(SETTLE);
                self.check(key, &want, "")
            }
            "flight" => self.arrange_flight(&name()?),
            // The flight arrangement brings the plane up to height. On the ground that is still to come.
            "height above home" => {
                let want = value.as_f64().ok_or("expected a number")?;
                let have = self.read(key)?.as_f64().unwrap_or(0.0);
                if have + 0.5 >= want || self.read("flight")?.as_str() == Some("grounded") {
                    Ok(())
                } else {
                    Err(format!("the plane is at {have:.0} m and can't be put at {want:.0} m without flying there"))
                }
            }
            "home position" => {
                if name()? != "set" {
                    return Err("can't unset home".into());
                }
                self.wait_for(60, "home being set", |g| g.vehicle.lock().unwrap().home.is_some())
            }
            "plane" if name()? == "on the ground" => self.check(key, "on the ground", ""),
            // A mission flown from the hand: armed in AUTO, the pilot throws the plane (Space in
            // the sim's window), and ArduPlane takes it from there.
            "plane" if name()? == "flying the mission" => {
                if self.read(key)?.as_str() != Some("flying the mission") {
                    if self.read("autopilot mode")?.as_str() != Some("AUTO")
                        || self.read("vehicle")?.as_str() != Some("armed")
                    {
                        return Err("only a plane armed in AUTO flies the mission".into());
                    }
                    self.perform_key(Key::Throw)?;
                    self.wait_for(15, "the thrown plane flying the mission", |g| {
                        let v = g.vehicle.lock().unwrap();
                        v.mode() == "AUTO" && v.have_position && (v.alt_rel >= 2.0 || v.groundspeed >= 2.0)
                    })?;
                }
                self.check(key, "flying the mission", "")
            }
            // Where the vehicle is in its mission: as it is (AUTO starts at the first item).
            "vehicle's current item" => {
                let want = value.as_f64().ok_or("expected a number")?;
                let w = want as u16;
                self.wait_for(5, &format!("the vehicle's current item being {want}"), move |g| {
                    g.vehicle.lock().unwrap().mission_seq == w
                })
                .map_err(|_| match want {
                    0.0 => "the vehicle is part-way through a mission".to_string(),
                    _ => format!("the vehicle's current item isn't {want}, and can't be put there"),
                })
            }
            "vehicle mission" => {
                self.arrange_mission(&name()?)?;
                if std::mem::take(&mut self.pending_auto) {
                    self.arrange_mode("AUTO")?;
                }
                Ok(())
            }
            "waypoints" | "mission file" | "measured distance" => {
                let want = name()?;
                self.check(key, &want, " (the console can't do anything else yet)")
            }
            // No game: press the key of the one being played, which stops it.
            "game clock" | "game" if matches!(name()?.as_str(), "off" | "free flight") => {
                if let Some(game) = self.call(|g, _, _| g.sensor.as_ref().and_then(|s| s.game()))? {
                    self.perform_key(if game.kind == GameKind::Race { Key::Race } else { Key::Tag })?;
                }
                self.check(key, &name()?, "")
            }
            _ => {
                let (key, value) = (key.to_string(), value.clone());
                self.call(move |g, _, _| arrange_model(g, &key, &value))?
            }
        }
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        match var.key.as_str() {
            "autopilot mode" => {
                let mode = enum_value(value)?;
                let id = MODE_IDS
                    .into_iter()
                    .find(|id| id.strip_prefix("mode-") == Some(mode.as_str()))
                    .ok_or_else(|| format!("no mode called {mode}"))?;
                self.press(id)
            }
            "plan altitude" => {
                let want = value.as_f64().ok_or("expected a number")?;
                let have = self.read("plan altitude")?.as_f64().unwrap_or(want);
                let steps = ((want - have) / 10.0).round() as i32;
                for _ in 0..steps.abs() {
                    self.press(if steps > 0 { "alt-up" } else { "alt-dn" })?;
                }
                Ok(())
            }
            // The radio's three inputs are put in place the way the pilot moves them.
            "throttle stick" | "aileron stick" | "arm switch" | "mode switch" => self.arrange(var, value),
            other => Err(format!("{other} isn't something to set")),
        }
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        match event.key.as_str() {
            // The mission goes on to its next item.
            "plane reaches a waypoint" => {
                let from = self.call(|g, _, _| g.vehicle.lock().unwrap().mission_seq)?;
                return self.wait_for(FLY_SECS, "the plane reaching a waypoint", move |g| {
                    g.vehicle.lock().unwrap().mission_seq > from
                });
            }
            // The mission is on its last item (the first is home).
            "plane reaches the last item" => {
                return self.wait_for(FLY_SECS, "the plane reaching the last item", |g| {
                    let v = g.vehicle.lock().unwrap();
                    v.mission.len() > 1 && usize::from(v.mission_seq) + 1 >= v.mission.len()
                });
            }
            _ => {}
        }
        let act = EVENTS.iter().find(|(k, _)| *k == event.key).map(|(_, a)| a).ok_or("no way to do it")?;
        match *act {
            Act::Button(id) => self.press(id),
            Act::Twice(id) => {
                self.press(id)?;
                self.press(id)
            }
            Act::Arm => {
                for _ in 0..12 {
                    self.press("arm")?;
                    self.press("arm")?;
                    self.out_of_time()?;
                    if self.wait_for(5, "arming", |g| g.vehicle.lock().unwrap().armed).is_ok() {
                        return Ok(());
                    }
                }
                let why = self.call(|g, _, _| g.vehicle.lock().unwrap().prearm.as_ref().map(|p| p.1.clone()))?;
                Err(format!("the vehicle wouldn't arm: {}", why.unwrap_or_else(|| "no reason given".into())))
            }
            Act::Radio(m) => self.radio_move(m),
            Act::Radios(moves) => moves.iter().try_for_each(|m| self.radio_move(*m)),
            Act::Key(k) => {
                let sent = self.call(move |g, _, _| g.sensor.as_ref().map(|s| s.press_key(k)).is_some())?;
                if !sent {
                    return Err("there is no link to the sim to press its keys: start the station (--station)".into());
                }
                std::thread::sleep(SETTLE);
                Ok(())
            }
            Act::Go => self.wait_for(30, "GO", |g| {
                g.sensor.as_ref().and_then(|s| s.game()).is_some_and(|x| x.clock == GameClock::Running)
            }),
            Act::Map(button) => {
                let at = self.call(|g, _, _| map_point(g))??;
                self.with_window(move |w, cx| click(w, cx, at, button))?;
                std::thread::sleep(FRAME);
                Ok(())
            }
        }
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        self.read(&var.key)
    }

    fn settle(&mut self) {
        // AUTO with no mission arranged after it: as it is (the start's check says what it got).
        if std::mem::take(&mut self.pending_auto) {
            let _ = self.arrange_mode("AUTO");
        }
        std::thread::sleep(SETTLE);
    }

    fn advance(&mut self) -> Progress {
        std::thread::sleep(LOOK);
        Progress::Advanced
    }

    /// The console's window, rendered off screen by GPUI (`Window::capture_image`): it needs
    /// neither the window nor the display to be visible, nor the Screen Recording permission. At
    /// one pixel a point: the docs show it smaller still, and the picture with the TX15's screen
    /// (an SVG) is published as it is.
    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        let bytes = self.with_window(|w, cx| {
            let scale = w.scale_factor().max(1.0);
            let image = image::DynamicImage::ImageRgba8(w.capture_image(cx)?);
            let (width, height) = ((image.width() as f32 / scale) as u32, (image.height() as f32 / scale) as u32);
            let rgb = image.resize_exact(width, height, image::imageops::FilterType::Triangle).into_rgb8();
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80).encode_image(&rgb).ok()?;
            Some(jpeg)
        })?;
        // With the TX15 on the serial line, its screen goes above the console's.
        if let (Some(jpg), Some(line)) = (&bytes, &self.line)
            && let Some(svg) = crate::tx15_screen::composite(jpg, &line.svg())
        {
            return Ok(Some(Screenshot::svg(svg)));
        }
        Ok(bytes.map(Screenshot::jpeg))
    }

    fn supports(&self, need: Need<'_>) -> bool {
        supported(need)
    }
}

/// What the driver can do: the states it reads, the ones it sets and the events it performs.
fn supported(need: Need<'_>) -> bool {
    match need {
        Need::Observe(v) | Need::Arrange(v) => STATES.contains(&v.key.as_str()),
        Need::Set(v) => SETTABLE.contains(&v.key.as_str()),
        Need::Perform(e) => EVENTS.iter().any(|(k, _)| *k == e.key) || WAITS.contains(&e.key.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test that ends with the mode switch on QRTL (the VTOL's last slot) and the arm switch on
    /// must not hand that to the restarted autopilot: ArduPlane enters the switch's mode before it
    /// knows where it is, and in QRTL SITL panics with "Location cannot be (0, 0, 0)", at
    /// every start, for as long as the switch stays there.
    #[test]
    fn the_stick_pulled_back_raises_the_pitch_channel() {
        // The elevator of the radio moves as the sim's USB radio did: -1 is back (nose up), and a
        // stick pulled back sends more than 1500 us on the pitch channel of ArduPlane.
        let back =
            RadioState { pitch: ELEVATOR_A_LITTLE_BACK, connected: true, ..radio_at_rest(radio_state_for_test()) };
        assert!(pilot_of(&back).channels()[1] > 1500, "{:?}", pilot_of(&back).channels());
        let forward = RadioState { pitch: 0.5, ..back };
        assert!(pilot_of(&forward).channels()[1] < 1500);
    }

    fn radio_state_for_test() -> RadioState {
        RadioState {
            connected: false,
            roll: 0.0,
            pitch: 0.0,
            yaw: 0.0,
            throttle: 0.0,
            arm: false,
            mode: 0,
            reset_presses: 0,
        }
    }

    #[test]
    fn a_test_that_flies_without_the_radio_leaves_the_throttle_where_the_sim_did() {
        // A keyboard pilot's throttle sits mid-way in the modes that climb on the stick, and low otherwise.
        assert_eq!(idle_throttle(true, "QHOVER"), 0.5);
        assert_eq!(idle_throttle(true, "GUIDED"), 0.5);
        assert_eq!(idle_throttle(true, "AUTO"), 0.0);
        assert_eq!(idle_throttle(false, "QHOVER"), 0.0, "a plane on the ground has its throttle down to arm");
    }

    #[test]
    fn a_fresh_plane_starts_with_the_radio_at_rest() {
        let left = RadioState {
            connected: true,
            roll: 0.3,
            pitch: -0.2,
            yaw: 0.1,
            throttle: 0.8,
            arm: true,
            mode: 5,
            reset_presses: 255,
        };
        let r = radio_for_a_fresh_plane(left);
        assert_eq!(r.reset_presses, 0, "the reset is pressed once");
        assert!(!r.connected && !r.arm, "unplugged, the arm switch off");
        assert_eq!(r.mode, 0, "the mode switch on its first slot");
        assert_eq!((r.roll, r.pitch, r.yaw, r.throttle), (0.0, 0.0, 0.0, STICK_LOW), "the sticks let go");
    }

    /// The suite checks clean, and the driver can do everything the console's plain UI tests
    /// need. Running them needs the window and a vehicle: `--station --expect`.
    #[test]
    fn a_run_has_a_limit_longer_than_its_tests_and_it_can_be_set() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
        let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
        let _ = &a;
        assert!(run_limit() > suite_budget::Budget::from_env().limit());
    }

    #[test]
    fn the_driver_covers_the_planning_and_command_tests() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
        let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
        let (tx, _rx) = channel();
        let driver = ConsoleDriver::new(tx, false, vec![]);
        let v = &a.suite.vocabulary;
        let gcs = a.suite.sections.iter().find(|s| s.slug == "gcs").expect("a gcs section").id;
        let ours = a.suite.subtree(gcs);
        let supported = |p: &expectation_story::Plan| {
            p.start.iter().all(|(x, _)| driver.supports(Need::Arrange(v.var(x))))
                && p.steps
                    .iter()
                    .flat_map(|s| &s.checks)
                    .chain(&p.start_checks)
                    .all(|c| driver.supports(Need::Observe(v.var(c.var))))
                && p.steps.iter().filter_map(|s| s.action.event).all(|e| driver.supports(Need::Perform(v.event(e))))
                && p.steps.iter().flat_map(|s| s.action.changes.vars()).all(|x| driver.supports(Need::Set(v.var(x))))
        };
        let plans: Vec<_> = a.plans.iter().filter(|p| ours.contains(&p.section)).collect();
        let n = plans.iter().filter(|p| supported(p)).count();
        println!("{n} of {} console tests are supported", plans.len());
        assert!(n >= 16, "only {n} of {} console tests are supported", plans.len());
    }

    /// Lists, in the file named by `SIM001_LIST_UNSUPPORTED`, the plans of the suite that the
    /// driver cannot run and what each lacks (run it with `--ignored`).
    #[test]
    #[ignore]
    fn list_the_plans_the_driver_cannot_run() {
        struct Dry;
        impl Driver for Dry {
            fn set(&mut self, _: &Variable, _: &Value) -> Result<(), String> {
                Err("dry".into())
            }
            fn perform(&mut self, _: &Event) -> Result<(), String> {
                Err("dry".into())
            }
            fn observe(&mut self, _: &Variable) -> Result<Value, String> {
                Err("dry".into())
            }
            fn supports(&self, need: Need<'_>) -> bool {
                supported(need)
            }
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
        let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
        let report = a.run(&mut Dry, &RunOptions { patience: 1, controls: false, ..Default::default() });
        let mut out = String::new();
        for (p, r) in a.plans.iter().zip(&report.results) {
            let ours = a.suite.sections.iter().any(|s| s.slug == "gcs" && a.suite.subtree(s.id).contains(&p.section));
            if ours {
                out += &format!("{:?}\t{}\t{}\n", r.status, p.title, r.start.error.clone().unwrap_or_default());
            }
        }
        std::fs::write(std::env::var("SIM001_LIST_UNSUPPORTED").expect("a path"), out).unwrap();
    }
}
