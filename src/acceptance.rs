//! `--expect [suite] [--docs dir]`: runs the acceptance tests in `expectations/` against the
//! live sim and, with `--docs`, writes the documentation with real screenshots.
//!
//! The runner works on a thread of its own, through a driver that turns every call into a
//! request for the `serve` system. Events are real key presses, injected just before the input
//! system reads the keyboard, so they take the same path as a pilot's. The throttle and pitch
//! stick are held the way a radio holds them, written over the pilot's input every frame.
//! Waiting (settling after an action, polling a check) counts real time, frame by frame.

use crate::camera::{CamMode, CamState};
use crate::hud::HudState;
use crate::input::PilotInput;
use crate::loading::AppState;
use simulation_physics::Phase;
use crate::sim::{FlightMode, SimCore, Sticks};
use crate::{SharedSim, SimSet};
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::WindowOccluded;
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions};
use expectation_model::{Event, Value, Variable};
use recreation_games::{Clock, Errand, Kind, StandIn};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// What each event of the vocabulary presses.
const KEYS: &[(&str, KeyCode)] = &[
    ("pilot throws the plane", KeyCode::Space),
    ("pilot resets the plane", KeyCode::KeyR),
    ("pilot changes flight mode", KeyCode::KeyM),
    ("pilot changes rates", KeyCode::KeyL),
    ("pilot toggles differential thrust", KeyCode::KeyX),
    ("pilot changes the view", KeyCode::KeyC),
    ("pilot toggles auto-zoom", KeyCode::KeyZ),
    ("pilot toggles the help", KeyCode::KeyH),
    ("pilot closes the help", KeyCode::Escape),
    ("pilot toggles pause", KeyCode::KeyP),
    ("pilot fits a fresh battery", KeyCode::KeyV),
    ("pilot starts a race", KeyCode::KeyG),
    ("pilot starts a game of tag", KeyCode::KeyT),
];

/// Events that aren't one key: the stand-in pilot flies them, or the test waits for them.
const FLOWN: &[&str] = &[
    "pilot ends the game",
    "countdown ends",
    "pilot flies a lap",
    "pilot cuts a pylon",
    "pilot tags an opponent",
    "chaser tags the plane",
    "pilot leaves the arena",
    "clock runs out",
];

/// States the driver can read (all of them), and the ones it can put in place directly.
const OBSERVED: &[&str] = &[
    "plane",
    "throttle",
    "pitch stick",
    "roll stick",
    "wings",
    "flight mode",
    "rates",
    "differential thrust",
    "view",
    "auto-zoom",
    "help screen",
    "sim",
    "battery",
    "flight controller",
    "game",
    "game clock",
    "opponents",
    "lap count",
    "cut count",
    "chaser",
];
const STICKS: &[&str] = &["throttle", "pitch stick", "roll stick"];
/// Wings within this many degrees of level count as level.
const LEVEL: f32 = 10.0;

/// Time to let the sim settle after an action, and between looks while waiting for a check.
const SETTLE: f32 = 1.5;
const LOOK: f32 = 0.25;
/// Longest the runner waits for the sim to answer (the first request waits for loading).
const ANSWER: Duration = Duration::from_secs(600);
/// Screenshots are scaled to this width and saved as JPEG, to keep the docs small.
const SHOT_WIDTH: u32 = 1280;
/// How many times (a second apart) to retry a screenshot that came out black.
const BLACK_RETRIES: u32 = 120;

type Job = Box<dyn FnOnce(&mut World) + Send>;

enum Request {
    /// Run against the world this frame.
    Now(Job),
    /// Answer once this much time has passed.
    Wait(f32, Sender<()>),
    /// Answer with a JPEG of the next frame.
    Shot(Sender<Shot>),
}

/// What a screenshot request gets back.
enum Shot {
    Jpeg(Vec<u8>),
    /// The frame came out black: the window wasn't being drawn (covered, or on another Space).
    Black,
    Failed,
}

#[derive(Resource)]
struct Remote {
    rx: Mutex<Receiver<Request>>,
    waiting: Option<(f32, Sender<()>)>,
    /// A screenshot waiting for the window to be visible, and how long it has been.
    shot: Option<(Sender<Shot>, f32)>,
    told_hidden: bool,
}

/// Whether the window is hidden from view. macOS doesn't draw a hidden window, so a
/// screenshot then would be black; screenshots wait until it's visible again.
#[derive(Resource, Default)]
struct Occluded(bool);

/// Keys pressed this frame, released the next.
#[derive(Resource, Default)]
struct Pressed(Vec<KeyCode>);

/// The sticks as the test holds them.
#[derive(Resource, Default)]
struct Held(Sticks);

/// In a game, a computer pilot flies the plane for the test once it is in the air, through
/// the sticks, as a pilot would: what it is doing now (by default racing, or loitering in tag).
#[derive(Resource, Default)]
struct Helper {
    errand: Option<Errand>,
    pilot: StandIn,
}

/// Load and check the suite before anything opens, then start the runner thread.
pub fn plugin(app: &mut App, suite: PathBuf, docs: Option<PathBuf>, flight_controller: bool) {
    let acceptance = match Acceptance::load(&suite) {
        Ok(a) => a.with_coverage(),
        Err(e) => {
            eprint!("{e}");
            std::process::exit(1);
        }
    };
    // T1S_EXPECT_ONLY=<text>: run only the tests whose title has it (while working on them).
    let mut acceptance = acceptance;
    if let Ok(only) = std::env::var("T1S_EXPECT_ONLY") {
        acceptance.plans.retain(|p| p.title.contains(&only));
    }
    println!("expect: {} tests from {}", acceptance.plans.len(), suite.display());

    let (tx, rx) = channel();
    app.insert_resource(Remote { rx: Mutex::new(rx), waiting: None, shot: None, told_hidden: false })
        .init_resource::<Pressed>()
        .init_resource::<Held>()
        .init_resource::<Helper>()
        .init_resource::<Occluded>()
        .add_systems(Update, track_occlusion)
        .add_systems(Update, serve.before(SimSet::Input).run_if(in_state(AppState::Running)))
        .add_systems(Update, hold.after(SimSet::Input).before(SimSet::Logic).run_if(in_state(AppState::Running)));

    std::thread::spawn(move || {
        let mut driver = SimDriver { tx, flight_controller };
        let opts = RunOptions { docs_dir: docs.clone(), patience: 100 };
        let mut report = expectation_story::Report::default();
        for plan in &acceptance.plans {
            let r = expectation_harness::run(&acceptance.suite, std::slice::from_ref(plan), &mut driver, &opts);
            let r = r.results.into_iter().next().unwrap();
            match r.status {
                expectation_story::Status::Passed => println!("expect: ✓ {}", plan.title),
                // Another system's test: counted at the end rather than listed.
                expectation_story::Status::Unsupported => {}
                _ => println!("expect: ✗ {}", plan.title),
            }
            report.results.push(r);
        }
        if let Some(dir) = &docs {
            // Saved as the sim's results, and merged with any other runner's (the ground
            // station's, one day) before the docs are written.
            match acceptance.publish(dir, "sim", &report) {
                Ok(_) => println!("expect: wrote the docs to {}", dir.display()),
                Err(e) => eprintln!("expect: couldn't write the docs to {}: {e}", dir.display()),
            }
        }
        let unsupported = report.count(expectation_story::Status::Unsupported);
        if unsupported > 0 {
            println!("expect: {unsupported} tests need a driver the sim doesn't have yet (the ground station's)");
        }
        if !report.passed() {
            eprint!("{}", report.failures(&acceptance.suite, &acceptance.plans));
        }
        let exit = if report.passed() { AppExit::Success } else { AppExit::from_code(1) };
        let _ = driver.call(move |w| w.write_message(exit));
    });
}

/// Answers the runner's requests, between frames' input handling.
fn serve(world: &mut World) {
    for key in std::mem::take(&mut world.resource_mut::<Pressed>().0) {
        world.resource_mut::<ButtonInput<KeyCode>>().release(key);
    }
    let dt = world.resource::<Time<Real>>().delta_secs();
    let hidden = world.resource::<Occluded>().0;
    world.resource_scope(|world, mut remote: Mut<Remote>| {
        if let Some((tx, visible)) = remote.shot.take() {
            if hidden {
                if !remote.told_hidden {
                    println!("expect: the sim window is hidden; screenshots wait until it's on screen");
                    remote.told_hidden = true;
                }
                remote.shot = Some((tx, 0.0));
            } else if visible < 0.5 {
                // Give a window that just came back a moment to draw.
                remote.shot = Some((tx, visible + dt));
            } else {
                remote.told_hidden = false;
                let mut tx = Some(tx);
                world.spawn(Screenshot::primary_window()).observe(move |shot: On<ScreenshotCaptured>| {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(jpeg(&shot.image));
                    }
                });
            }
            return;
        }
        if let Some((left, tx)) = remote.waiting.take() {
            if left > dt {
                remote.waiting = Some((left - dt, tx));
                return;
            }
            let _ = tx.send(());
        }
        loop {
            let next = remote.rx.lock().unwrap().try_recv();
            match next {
                Ok(Request::Now(job)) => job(world),
                Ok(Request::Wait(secs, tx)) => {
                    remote.waiting = Some((secs, tx));
                    return;
                }
                Ok(Request::Shot(tx)) => {
                    remote.shot = Some((tx, 0.5));
                    return;
                }
                Err(_) => return,
            }
        }
    });
}

fn track_occlusion(mut events: MessageReader<WindowOccluded>, mut occluded: ResMut<Occluded>) {
    for e in events.read() {
        occluded.0 = e.occluded;
    }
}

fn hold(held: Res<Held>, mut helper: ResMut<Helper>, sim: Res<SharedSim>, time: Res<Time>, mut input: ResMut<PilotInput>) {
    input.sticks = held.0;
    let c = sim.0.lock().unwrap();
    let Some(game) = &c.game else {
        helper.errand = None;
        return;
    };
    if !matches!(c.state.phase, Phase::Launching | Phase::Flying) {
        return;
    }
    let errand = helper.errand.unwrap_or(match game.kind {
        Kind::Race => Errand::Race,
        Kind::Tag => Errand::Loiter,
    });
    let dt = time.delta_secs().clamp(0.001, 0.1);
    let hands = helper.pilot.fly(game, &c.state, errand, &crate::terrain::WorldTerrain, dt);
    // MANUAL moves the surfaces by the stick through the rates and an expo: undo both.
    let rate = if c.high_rates { 1.0 } else { 0.65 };
    input.sticks = Sticks {
        roll: unexpo(hands.aileron / rate, 0.3),
        pitch: unexpo(hands.elevator / rate, 0.3),
        yaw: unexpo(hands.rudder / rate, 0.2),
        throttle: hands.throttle.clamp(0.0, 1.0),
    };
}

/// The stick that gives `y` through the sim's expo `e·x³ + (1 − e)·x`.
fn unexpo(y: f32, e: f32) -> f32 {
    let y = y.clamp(-1.0, 1.0);
    let mut x = y;
    for _ in 0..8 {
        x -= (e * x * x * x + (1.0 - e) * x - y) / (3.0 * e * x * x + 1.0 - e);
    }
    x.clamp(-1.0, 1.0)
}

/// The frame, scaled down, as JPEG. All black means the window wasn't being drawn.
fn jpeg(image: &Image) -> Shot {
    let Ok(dynamic) = image.clone().try_into_dynamic() else { return Shot::Failed };
    let rgb = dynamic.to_rgb8();
    if rgb.pixels().all(|p| p.0.iter().all(|&c| c < 4)) {
        return Shot::Black;
    }
    let (w, h) = rgb.dimensions();
    let width = SHOT_WIDTH.min(w);
    let small = image::imageops::resize(&rgb, width, h * width / w, image::imageops::FilterType::Triangle);
    let mut out = vec![];
    match image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&small) {
        Ok(()) => Shot::Jpeg(out),
        Err(_) => Shot::Failed,
    }
}

struct SimDriver {
    tx: Sender<Request>,
    /// The plane has a flight controller (`--aircraft`).
    flight_controller: bool,
}

impl SimDriver {
    /// Run `f` against the world on the next frame and return what it returns.
    fn call<T: Send + 'static>(&self, f: impl FnOnce(&mut World) -> T + Send + 'static) -> Result<T, String> {
        let (tx, rx) = channel();
        let job: Job = Box::new(move |w| {
            let _ = tx.send(f(w));
        });
        self.tx.send(Request::Now(job)).map_err(|_| "the sim has quit".to_string())?;
        rx.recv_timeout(ANSWER).map_err(|_| "the sim stopped answering".to_string())
    }

    fn sim<T: Send + 'static>(&self, f: impl FnOnce(&mut SimCore) -> T + Send + 'static) -> Result<T, String> {
        self.call(move |w| f(&mut w.resource::<SharedSim>().0.lock().unwrap()))
    }

    fn wait(&self, secs: f32) -> Result<(), String> {
        let (tx, rx) = channel();
        self.tx.send(Request::Wait(secs, tx)).map_err(|_| "the sim has quit".to_string())?;
        rx.recv_timeout(ANSWER).map_err(|_| "the sim stopped answering".to_string())
    }

    /// Wait up to `secs` of sim time for `done`, saying what it waited for if it never came.
    fn until(&self, secs: f32, what: &str, done: impl Fn(&SimCore) -> bool + Send + Clone + 'static) -> Result<(), String> {
        let mut waited = 0.0;
        while waited < secs {
            let d = done.clone();
            if self.sim(move |c| d(c))? {
                return Ok(());
            }
            self.wait(LOOK)?;
            waited += LOOK;
        }
        Err(format!("waited {secs:.0} s for {what}"))
    }

    /// Give the stand-in pilot a job.
    fn errand(&self, errand: Errand) -> Result<(), String> {
        self.call(move |w| w.resource_mut::<Helper>().errand = Some(errand))
    }

    /// The events the stand-in pilot flies, or the test waits for: each returns when it has
    /// happened.
    fn fly(&self, event: &str) -> Result<(), String> {
        let laps = |c: &SimCore| c.game.as_ref().map_or(0, |g| g.your_laps());
        let cuts = |c: &SimCore| c.game.as_ref().map_or(0, |g| g.your_cuts());
        let chaser = |c: &SimCore| c.game.as_ref().and_then(|g| g.chaser());
        match event {
            "pilot ends the game" => {
                let key = match self.sim(|c| c.game.as_ref().map(|g| g.kind))? {
                    Some(Kind::Race) => KeyCode::KeyG,
                    Some(Kind::Tag) => KeyCode::KeyT,
                    None => return Err("there is no game to end".into()),
                };
                self.press(key)
            }
            "countdown ends" => self.until(10.0, "GO", |c| c.game.as_ref().is_some_and(|g| !matches!(g.clock, Clock::CountingDown { .. }))),
            "pilot flies a lap" => {
                let before = self.sim(move |c| laps(c))?;
                self.errand(Errand::Race)?;
                self.until(90.0, "a lap", move |c| laps(c) > before)
            }
            "pilot cuts a pylon" => {
                let before = self.sim(move |c| cuts(c))?;
                self.errand(Errand::Cut)?;
                let r = self.until(90.0, "a cut", move |c| cuts(c) > before);
                self.errand(Errand::Race)?;
                r
            }
            "pilot tags an opponent" => {
                self.errand(Errand::Tag)?;
                let r = self.until(120.0, "a tag", move |c| chaser(c).is_some_and(|i| i != 0));
                self.errand(Errand::Loiter)?;
                r
            }
            "chaser tags the plane" => {
                self.errand(Errand::Loiter)?;
                self.until(150.0, "the chaser to tag the plane", move |c| chaser(c) == Some(0))
            }
            "pilot leaves the arena" => {
                self.errand(Errand::LeaveArena)?;
                let r = self.until(60.0, "the plane to leave the arena", move |c| chaser(c) == Some(0));
                self.errand(Errand::Loiter)?;
                r
            }
            "clock runs out" => self.until(150.0, "the end of the game", |c| c.game.as_ref().is_some_and(|g| matches!(g.clock, Clock::Stopped { .. }))),
            other => Err(format!("no way to do `{other}`")),
        }
    }

    fn press(&self, key: KeyCode) -> Result<(), String> {
        self.call(move |w| {
            w.resource_mut::<ButtonInput<KeyCode>>().press(key);
            w.resource_mut::<Pressed>().0.push(key);
        })
    }
}

fn enum_value(v: &Value) -> Result<String, String> {
    v.as_str().map(str::to_string).ok_or_else(|| format!("expected a name, got {v}"))
}

fn on_off(on: bool, yes: &'static str, no: &'static str) -> Value {
    Value::from(if on { yes } else { no })
}

impl Driver for SimDriver {
    /// Back to the Background: in hand, sticks centred, MANUAL, high rates, differential
    /// thrust on, line of sight with auto-zoom, no help, running, a full battery. Controllers
    /// are ignored from here on, so a connected gamepad can't change what the keys do.
    fn reset(&mut self) -> Result<(), String> {
        self.call(|w| {
            {
                let sim = w.resource::<SharedSim>().0.clone();
                let mut c = sim.lock().unwrap();
                c.reset();
                c.state.snap_to_hand();
                c.state.recharge();
                c.mode = FlightMode::Manual;
                c.high_rates = true;
                c.diff_thrust = true;
                c.paused = false;
                c.game = None;
            }
            w.resource_mut::<Helper>().errand = None;
            w.resource_mut::<Held>().0 = Sticks::default();
            let mut cam = w.resource_mut::<CamState>();
            cam.mode = CamMode::LineOfSight;
            cam.auto_zoom = true;
            w.resource_mut::<HudState>().show_help = false;
            let mut input = w.resource_mut::<PilotInput>();
            input.wizard = None;
            input.keyboard_only = true;
        })
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        let name = || enum_value(value);
        match var.key.as_str() {
            "throttle" | "pitch stick" | "roll stick" => self.set(var, value),
            // Anything else would mean flying it there: the Background already has it in hand,
            // and level.
            "plane" if name()? == "in hand" => Ok(()),
            "wings" if name()? == "level" => Ok(()),
            "battery" if value.as_f64().is_some_and(|b| b >= 99.0) => self.sim(|c| c.state.recharge()),
            "flight mode" => {
                let mode = match name()?.as_str() {
                    "MANUAL" => FlightMode::Manual,
                    "ANGLE" => FlightMode::Angle,
                    _ => FlightMode::Acro,
                };
                self.sim(move |c| c.mode = mode)
            }
            "rates" => {
                let high = name()? == "high";
                self.sim(move |c| c.high_rates = high)
            }
            "differential thrust" => {
                let on = name()? == "on";
                self.sim(move |c| c.diff_thrust = on)
            }
            "flight controller" => {
                let want = name()?;
                let have = if self.flight_controller { "fitted" } else { "missing" };
                if want == have { Ok(()) } else { Err(format!("the plane's flight controller is {have}, not {want}")) }
            }
            "sim" => {
                let paused = name()? == "paused";
                self.sim(move |c| c.paused = paused)
            }
            "view" => {
                let mode = match name()?.as_str() {
                    "line of sight" => CamMode::LineOfSight,
                    "chase" => CamMode::Chase,
                    _ => CamMode::Fpv,
                };
                self.call(move |w| w.resource_mut::<CamState>().mode = mode)
            }
            "auto-zoom" => {
                let on = name()? == "on";
                self.call(move |w| w.resource_mut::<CamState>().auto_zoom = on)
            }
            "help screen" => {
                let shown = name()? == "shown";
                self.call(move |w| w.resource_mut::<HudState>().show_help = shown)
            }
            "game" => {
                let want = match name()?.as_str() {
                    "a race" => Some(Kind::Race),
                    "tag" => Some(Kind::Tag),
                    _ => None,
                };
                self.sim(move |c| match want {
                    None => c.game = None,
                    Some(kind) if c.game.as_ref().is_none_or(|g| g.kind != kind) => c.toggle_game(kind),
                    Some(_) => {}
                })
            }
            // These follow from the game; they can only be what they already are.
            "game clock" | "lap count" | "cut count" | "opponents" | "chaser" => {
                let now = self.observe(var)?;
                if now == *value { Ok(()) } else { Err(format!("{} is {now}, and only the game can make it {value}", var.subject)) }
            }
            _ => Err(format!("can't put {} in place: {value}", var.subject)),
        }
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        match var.key.as_str() {
            "throttle" => {
                let t = value.as_f64().ok_or("expected a number")? as f32 / 100.0;
                self.call(move |w| w.resource_mut::<Held>().0.throttle = t)
            }
            "pitch stick" => {
                // Stick forward is nose down.
                let p = match enum_value(value)?.as_str() {
                    "forward" => -1.0,
                    "back" => 1.0,
                    _ => 0.0,
                };
                self.call(move |w| w.resource_mut::<Held>().0.pitch = p)
            }
            "roll stick" => {
                // Stick right is positive.
                let r = match enum_value(value)?.as_str() {
                    "left" => -1.0,
                    "right" => 1.0,
                    _ => 0.0,
                };
                self.call(move |w| w.resource_mut::<Held>().0.roll = r)
            }
            _ => Err(format!("{} isn't a stick", var.subject)),
        }
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        if FLOWN.contains(&event.key.as_str()) {
            return self.fly(&event.key);
        }
        let key = KEYS.iter().find(|(k, _)| *k == event.key).map(|(_, k)| *k).ok_or("no key for it")?;
        self.press(key)
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        let key = var.key.clone();
        self.call(move |w| {
            let cam = w.resource::<CamState>();
            let (cam_mode, auto_zoom) = (cam.mode, cam.auto_zoom);
            let help = w.resource::<HudState>().show_help;
            let c = w.resource::<SharedSim>().0.lock().unwrap();
            let s = &c.state;
            Ok(match key.as_str() {
                "plane" => Value::from(match s.phase {
                    Phase::Held if s.carry.is_some() => "being carried back",
                    Phase::Held => "in hand",
                    Phase::Launching => "launching",
                    Phase::Flying if s.on_ground => "on the ground",
                    Phase::Flying => "flying",
                    Phase::Crashed => "crashed",
                }),
                "throttle" => Value::from(c.sticks.throttle as f64 * 100.0),
                "pitch stick" => Value::from(match c.sticks.pitch {
                    p if p < -0.5 => "forward",
                    p if p > 0.5 => "back",
                    _ => "centred",
                }),
                "roll stick" => Value::from(match c.sticks.roll {
                    r if r < -0.5 => "left",
                    r if r > 0.5 => "right",
                    _ => "centred",
                }),
                "wings" => {
                    let roll = simulation_physics::euler(s.rot).0.to_degrees();
                    Value::from(if roll.abs() < LEVEL { "level" } else { "banked" })
                }
                "flight mode" => Value::from(c.mode.label()),
                "rates" => on_off(c.high_rates, "high", "low"),
                "differential thrust" => on_off(c.diff_thrust, "on", "off"),
                "view" => Value::from(match cam_mode {
                    CamMode::LineOfSight => "line of sight",
                    CamMode::Chase => "chase",
                    CamMode::Fpv => "FPV",
                }),
                "auto-zoom" => on_off(auto_zoom, "on", "off"),
                "help screen" => on_off(help, "shown", "hidden"),
                "sim" => on_off(c.paused, "paused", "running"),
                "flight controller" => Value::from(if c.params.flight_controller { "fitted" } else { "missing" }),
                "battery" => Value::from((1.0 - s.used_mah / c.params.battery_capacity_mah).max(0.0) as f64 * 100.0),
                "game" => Value::from(match c.game.as_ref().map(|g| g.kind) {
                    None => "free flight",
                    Some(Kind::Race) => "a race",
                    Some(Kind::Tag) => "tag",
                }),
                "game clock" => Value::from(match c.game.as_ref().map(|g| g.clock) {
                    None => "off",
                    Some(Clock::Waiting) => "waiting for takeoff",
                    Some(Clock::CountingDown { .. }) => "counting down",
                    Some(Clock::Running { .. }) => "running",
                    Some(Clock::Stopped { .. }) => "stopped",
                }),
                "opponents" => {
                    let Some(g) = &c.game else { return Err("there are no opponents without a game".into()) };
                    let all = |f: &dyn Fn(&simulation_physics::FlightState) -> bool| g.opponents.iter().all(|o| f(&o.state));
                    Value::from(if all(&|s| s.phase == Phase::Held) {
                        "in hand"
                    } else if g.opponents.iter().any(|o| o.state.phase == Phase::Crashed) {
                        "down"
                    } else if all(&|s| s.phase == Phase::Flying && !s.on_ground) {
                        "flying"
                    } else {
                        "launching"
                    })
                }
                "lap count" => Value::from(c.game.as_ref().map_or(0, |g| g.your_laps()) as f64),
                "cut count" => Value::from(c.game.as_ref().map_or(0, |g| g.your_cuts()) as f64),
                "chaser" => match c.game.as_ref().and_then(|g| g.chaser()) {
                    Some(0) => Value::from("you"),
                    Some(_) => Value::from("an opponent"),
                    None => return Err("nobody is it outside a game of tag".into()),
                },
                other => return Err(format!("no way to observe `{other}`")),
            })
        })?
    }

    fn settle(&mut self) {
        let _ = self.wait(SETTLE);
    }

    fn advance(&mut self) -> Progress {
        match self.wait(LOOK) {
            Ok(()) => Progress::Advanced,
            Err(_) => Progress::Static,
        }
    }

    /// A black frame means the window is covered or on another Space, where macOS doesn't
    /// draw it: wait for it to come back (saying so once) rather than lose the picture.
    fn screenshot(&mut self) -> Result<Option<expectation_harness::Screenshot>, String> {
        for attempt in 0..BLACK_RETRIES {
            let (tx, rx) = channel();
            self.tx.send(Request::Shot(tx)).map_err(|_| "the sim has quit".to_string())?;
            match rx.recv_timeout(ANSWER).map_err(|_| "no screenshot arrived".to_string())? {
                Shot::Jpeg(bytes) => return Ok(Some(expectation_harness::Screenshot::jpeg(bytes))),
                Shot::Failed => return Ok(None),
                Shot::Black => {
                    if attempt == 0 {
                        println!("expect: the sim window isn't being drawn (covered?); waiting for it to show");
                    }
                    self.wait(1.0)?;
                }
            }
        }
        println!("expect: gave up waiting for the window; leaving this screenshot out");
        Ok(None)
    }

    /// The plane in the sim is the one it was started with: a test that starts with a flight
    /// controller can't be arranged on a plane without one.
    fn can_arrange(&self, var: &Variable, value: &Value) -> bool {
        var.key != "flight controller" || value.as_str() == Some(if self.flight_controller { "fitted" } else { "missing" })
    }

    fn supports(&self, need: Need<'_>) -> bool {
        match need {
            Need::Observe(v) | Need::Arrange(v) => OBSERVED.contains(&v.key.as_str()),
            Need::Set(v) => STICKS.contains(&v.key.as_str()),
            Need::Perform(e) => KEYS.iter().any(|(k, _)| *k == e.key) || FLOWN.contains(&e.key.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stand-in pilot's sticks give the surfaces it wants, through MANUAL's expo.
    #[test]
    fn unexpo_undoes_the_expo() {
        for y in [-1.0f32, -0.6, -0.1, 0.0, 0.25, 0.8, 1.0] {
            let x = unexpo(y, 0.3);
            assert!((0.3 * x * x * x + 0.7 * x - y).abs() < 1e-4, "{y}: {x}");
        }
        assert_eq!(unexpo(3.0, 0.3), 1.0);
    }

    /// The suite checks clean (no conflicts, no story relying on unspecified behaviour), and
    /// the driver can do everything its tests need. Running it needs the window: `--expect`.
    #[test]
    fn the_suite_checks_clean_and_the_driver_covers_it() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expectations");
        let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
        let (tx, _rx) = channel();
        let driver = SimDriver { tx, flight_controller: false };
        let v = &a.suite.vocabulary;
        let sim = a.suite.sections.iter().find(|s| s.slug == "sim").expect("a sim section").id;
        let ours = a.suite.subtree(sim);
        let plans: Vec<_> = a.plans.iter().filter(|p| ours.contains(&p.section)).collect();
        assert!(!plans.is_empty());
        for plan in &plans {
            for (x, _) in plan.start.iter() {
                assert!(driver.supports(Need::Arrange(v.var(x))), "{}: can't arrange {}", plan.title, v.var(x).subject);
            }
            for c in plan.steps.iter().flat_map(|s| &s.checks).chain(&plan.start_checks) {
                assert!(
                    driver.supports(Need::Observe(v.var(c.var))),
                    "{}: can't observe {}",
                    plan.title,
                    v.var(c.var).subject
                );
            }
            for e in plan.steps.iter().filter_map(|s| s.action.event) {
                assert!(driver.supports(Need::Perform(v.event(e))), "{}: no key for {}", plan.title, v.event(e).text);
            }
        }
        for plan in &plans {
            for step in &plan.steps {
                for x in step.action.changes.vars() {
                    assert!(driver.supports(Need::Set(v.var(x))), "{}: can't set {}", plan.title, v.var(x).subject);
                }
            }
        }
    }
}
