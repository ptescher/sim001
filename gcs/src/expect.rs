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
//! check, the survey grid and measuring (the console has no such features), and anything in the
//! sim's window (throwing the plane).

use crate::link::Cmd;
use crate::ui::app::{self, Gcs};
use crate::ui::map::Tool;
use crate::vehicle::{Waypoint, WpKind, mode_number};
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use t1s_station::feed::RadioState;
use gpui::{
    AppContext as _,
    App, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point,
    Window, WindowHandle,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

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
}

/// Where the throttle stick is for each of its values in the vocabulary.
const STICK_LOW: f32 = 0.0;
const STICK_MIDDLE: f32 = 0.5;
const STICK_HIGH: f32 = 0.75;

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
    /// ARM, then CONFIRM ARM, again and again until the vehicle arms: a vehicle that has only
    /// just come up refuses until its checks pass, and the button says so.
    Arm,
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
    ("operator switches to AUTO", Act::Button("mode-AUTO")),
    ("operator switches to AUTO anyway", Act::Twice("mode-AUTO")),
    ("operator takes off vertically", Act::Button("vtol-tkof")),
    ("operator switches to forward flight", Act::Button("mode-CRUISE")),
    ("operator switches to hover", Act::Button("mode-QLOITER")),
    ("operator lands vertically", Act::Button("mode-QLAND")),
    ("pilot raises the throttle stick", Act::Radio(RadioMove::Throttle(STICK_HIGH))),
    ("pilot centres the throttle stick", Act::Radio(RadioMove::Throttle(STICK_MIDDLE))),
    ("pilot lowers the throttle stick", Act::Radio(RadioMove::Throttle(STICK_LOW))),
];

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
    "throttle stick",
    "arm switch",
    "mode switch",
];
/// States an action can change directly (the others change through events).
const SETTABLE: &[&str] = &["autopilot mode", "plan altitude", "throttle stick", "arm switch", "mode switch"];

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

/// Runs on the UI thread with the app and the console's window.
type Job = Box<dyn FnOnce(&mut App, WindowHandle<Gcs>) + Send>;

/// Load and check the suite before anything opens, then start the runner thread.
pub fn start(cx: &mut App, console: WindowHandle<Gcs>, config: Config) {
    let Config { suite, docs, only, vtol, rc_modes } = config;
    let mut acceptance = match Acceptance::load(&suite) {
        Ok(a) => a.with_coverage(),
        Err(e) => {
            eprint!("{e}");
            std::process::exit(1);
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

    std::thread::spawn(move || {
        let mut driver = ConsoleDriver::new(tx, vtol, rc_modes);
        let opts = RunOptions { docs_dir: docs.clone(), patience: 480 };
        let mut report = expectation_story::Report::default();
        for plan in &acceptance.plans {
            let r = expectation_harness::run(&acceptance.suite, std::slice::from_ref(plan), &mut driver, &opts);
            let r = r.results.into_iter().next().unwrap();
            match r.status {
                expectation_story::Status::Passed => println!("expect: ✓ {}", plan.title),
                // Another system's test (the sim's), or one for a feature the console lacks.
                expectation_story::Status::Unsupported => {}
                _ => {
                    println!("expect: ✗ {}", plan.title);
                    // What the vehicle was saying, for working out why.
                    if let Ok(lines) = driver.call(|g, _, _| {
                        let v = g.vehicle.lock().unwrap();
                        v.log.iter().rev().take(70).map(|l| l.text.clone()).collect::<Vec<_>>()
                    }) {
                        for l in lines.iter().rev() {
                            println!("expect:   vehicle: {l}");
                        }
                    }
                }
            }
            report.results.push(r);
        }
        if let Some(dir) = &docs {
            // Saved as the console's results, and merged with the sim's before the docs are written.
            match acceptance.publish(dir, "gcs", &report) {
                Ok(_) => println!("expect: wrote the docs to {}", dir.display()),
                Err(e) => eprintln!("expect: couldn't write the docs to {}: {e}", dir.display()),
            }
        }
        let unsupported = report.count(expectation_story::Status::Unsupported);
        if unsupported > 0 {
            println!("expect: {unsupported} tests need a driver the console doesn't have yet (the sim's, or a feature to come)");
        }
        if !report.passed() {
            eprint!("{}", report.failures(&acceptance.suite, &acceptance.plans));
            EXIT.store(1, Ordering::SeqCst);
        }
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
    send(PlatformInput::MouseDown(MouseDownEvent { button, position: at, modifiers, click_count: 1, first_mouse: false }));
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
        "takeoff" => Value::from(if plan.first().is_some_and(|w| w.kind == WpKind::Takeoff) { "planned" } else { "not planned" }),
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
        "flight" => Value::from(flight(&v)),
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
        ConsoleDriver { tx, vtol, rc_modes, radio }
    }

    /// Move the stand-in radio and tell the sim.
    fn radio_move(&mut self, m: RadioMove) -> Result<(), String> {
        // Using the radio plugs it in.
        self.radio.connected = true;
        match m {
            RadioMove::ArmSwitch(on) => self.radio.arm = on,
            RadioMove::ModeSwitch(slot) => self.radio.mode = slot.min(5) as u8,
            RadioMove::Throttle(t) => self.radio.throttle = t,
        }
        let radio = self.radio;
        let sent = self.call(move |g, _, _| g.sensor.as_ref().map(|s| s.send_radio(radio)).is_some())?;
        if sent { Ok(()) } else { Err("there is no link to the sim to carry the radio: start the station (--station)".into()) }
    }

    /// Which way the radio's three inputs read to ArduPilot: the throttle stick, the arm
    /// switch and the mode switch.
    fn rc_reading(&self, key: &str) -> Result<Value, String> {
        let rc = self.call(|g, _, _| g.vehicle.lock().unwrap().rc)?;
        Ok(match key {
            "throttle stick" => Value::from(match rc[2] {
                0..=1249 => "low",
                1250..=1649 => "middle",
                _ => "high",
            }),
            "arm switch" => Value::from(if rc[4] > 1500 { "on" } else { "off" }),
            _ => {
                // The mode switch's position nearest to the PWM.
                let slot = simulation_aircraft::rc_mode_slot(rc[7]);
                Value::from(*self.rc_modes.get(slot).ok_or("this plane has no mode switch")?)
            }
        })
    }
    /// Run `f` on the UI thread, between frames, and return what it returns.
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Gcs, &mut Window, &mut Context<Gcs>) -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = channel();
        let job: Job = Box::new(move |cx, console| {
            let _ = console.update(cx, |g, w, cx| tx.send(f(g, w, cx)));
        });
        self.tx.send(job).map_err(|_| "the console has quit".to_string())?;
        rx.recv_timeout(ANSWER).map_err(|_| "the console stopped answering".to_string())
    }

    /// Run `f` on the UI thread against the window alone. Input has to be delivered this way:
    /// the click's handlers update the console, which `call` has borrowed.
    fn with_window<T: Send + 'static>(&self, f: impl FnOnce(&mut Window, &mut App) -> T + Send + 'static) -> Result<T, String> {
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
        if matches!(key, "throttle stick" | "arm switch" | "mode switch") {
            return self.rc_reading(key);
        }
        if key == "airframe" {
            return Ok(Value::from(if self.vtol { "VTOL" } else { "conventional" }));
        }
        let key = key.to_string();
        self.call(move |g, _, _| observe(g, &key))?
    }

    /// Poll until `done` holds, or fail saying `what` didn't happen in time.
    fn wait_for(&self, secs: u64, what: &str, done: impl Fn(&mut Gcs) -> bool + Clone + Send + 'static) -> Result<(), String> {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let d = done.clone();
            if self.call(move |g, _, _| d(g))? {
                return Ok(());
            }
            if Instant::now() > end {
                return Err(format!("{what} didn't happen within {secs} s"));
            }
            std::thread::sleep(LOOK);
        }
    }

    /// Fly the VTOL to `want`: armed and taking off straight up, then (for forward flight)
    /// switched across.
    fn arrange_flight(&self, want: &str) -> Result<(), String> {
        let now = || self.read("flight").map(|v| v.as_str().unwrap_or("").to_string());
        match want {
            "grounded" if now()? == "grounded" => Ok(()),
            "grounded" => Err("can't put a flying plane on the ground without landing it".into()),
            "hovering" | "cruising" => {
                if now()? == "grounded" {
                    if self.read("vehicle")?.as_str() != Some("armed") {
                        for _ in 0..6 {
                            self.send(Cmd::Arm(true))?;
                            if self.wait_for(5, "arming", |g| g.vehicle.lock().unwrap().armed).is_ok() {
                                break;
                            }
                        }
                    }
                    self.press("vtol-tkof")?;
                }
                if now()? == "cruising" && want == "hovering" {
                    self.press("mode-QLOITER")?;
                }
                self.wait_for(90, "the plane hovering", |g| {
                    let v = g.vehicle.lock().unwrap();
                    flight(&v) == "hovering" && v.alt_rel > 15.0 || flight(&v) == "cruising"
                })?;
                if want == "cruising" && now()? != "cruising" {
                    self.press("mode-CRUISE")?;
                    self.wait_for(60, "the plane cruising", |g| flight(&g.vehicle.lock().unwrap()) == "cruising")?;
                }
                Ok(())
            }
            other => Err(format!("can't put the flight in `{other}`")),
        }
    }

    /// Is the plane up, armed, or is its autopilot unhappy (the pre-arm checks fail), so that a
    /// test can't start from it?
    fn needs_a_fresh_plane(&self) -> Result<bool, String> {
        const PREARM_CHECK: u32 = 0x1000_0000;
        self.call(|g, _, _| {
            let v = g.vehicle.lock().unwrap();
            v.link.alive() && (v.armed || v.alt_rel.abs() > 1.0 || v.sensor_ok(PREARM_CHECK) == Some(false))
        })
    }

    /// Start the sim again with a fresh plane, and wait for ArduPilot to go and come back.
    fn fresh_plane(&mut self) -> Result<(), String> {
        println!("expect: starting the next test from a fresh plane (the autopilot restarts, about a minute)");
        self.radio.reset_presses = self.radio.reset_presses.wrapping_add(1);
        let radio = self.radio;
        self.call(move |g, _, _| {
            if let Some(s) = &g.sensor {
                s.send_radio(radio);
            }
        })?;
        // The station restarts the autopilot: its link drops, and then comes back.
        let _ = self.wait_for(45, "the autopilot restarting", |g| !g.vehicle.lock().unwrap().link.alive());
        self.wait_for(180, "the autopilot coming back", |g| g.vehicle.lock().unwrap().link.alive())
    }

    fn press(&self, id: &'static str) -> Result<(), String> {
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
        // A plane that has flown, or whose autopilot has lost its bearings, starts the next test
        // from a fresh plane: the sim starts again (as with R), and the station restarts ArduPilot.
        if self.vtol && self.needs_a_fresh_plane()? {
            self.fresh_plane()?;
        }
        // The throttle stick starts down, as a pilot leaves it before arming.
        if self.vtol {
            // A test that doesn't use the radio flies without one, like a keyboard pilot.
            self.radio.connected = false;
            self.radio.throttle = STICK_LOW;
            let radio = self.radio;
            self.call(move |g, _, _| {
                if let Some(s) = &g.sensor {
                    s.send_radio(radio);
                }
            })?;
        }
        self.call(|g, _, _| {
            g.map.tool = Tool::Pan;
            g.map.plan.clear();
            g.map.plan_alt = 40.0;
            g.map.follow = true;
        })
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
                let n = mode_number(&mode).ok_or_else(|| format!("no mode called {mode}"))?;
                // A vehicle that has only just come up can change mode by itself (ArduPlane
                // does when the EKF settles), so the mode has to hold for a moment.
                for _ in 0..4 {
                    if self.read(key)?.as_str() != Some(mode.as_str()) {
                        self.send(Cmd::SetMode(n))?;
                        let m = mode.clone();
                        self.wait_for(15, &format!("the {mode} mode"), move |g| g.vehicle.lock().unwrap().mode() == m)?;
                    }
                    std::thread::sleep(HOLD);
                    if self.read(key)?.as_str() == Some(mode.as_str()) {
                        return Ok(());
                    }
                }
                Err(format!("the vehicle wouldn't stay in {mode}"))
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
                for _ in 0..100 {
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
                let slot = self.rc_modes.iter().position(|m| *m == want).ok_or("this plane has no such mode on its switch")?;
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
            "vehicle's current item" if value.as_f64() == Some(0.0) => {
                if self.read(key)?.as_f64() == Some(0.0) { Ok(()) } else { Err("the vehicle is part-way through a mission".into()) }
            }
            "vehicle mission" => {
                let want = name()?;
                let state = self.read(key)?;
                if state.as_str() == Some(want.as_str()) {
                    return Ok(());
                }
                match want.as_str() {
                    "empty" => {
                        self.send(Cmd::ClearMission)?;
                        std::thread::sleep(Duration::from_millis(500));
                        // Read it back, rather than trusting what the console remembers.
                        self.send(Cmd::Download)?;
                        self.wait_for(20, "the vehicle's mission being cleared", |g| {
                            let v = g.vehicle.lock().unwrap();
                            v.mission.len() <= 1 && v.mission_status == "no mission on vehicle"
                        })
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
            "waypoints" | "mission file" | "measured distance" => {
                let want = name()?;
                self.check(key, &want, " (the console can't do anything else yet)")
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
            "throttle stick" | "arm switch" | "mode switch" => self.arrange(var, value),
            other => Err(format!("{other} isn't something to set")),
        }
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
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
                    if self.wait_for(5, "arming", |g| g.vehicle.lock().unwrap().armed).is_ok() {
                        return Ok(());
                    }
                }
                let why = self.call(|g, _, _| g.vehicle.lock().unwrap().prearm.as_ref().map(|p| p.1.clone()))?;
                Err(format!("the vehicle wouldn't arm: {}", why.unwrap_or_else(|| "no reason given".into())))
            }
            Act::Radio(m) => self.radio_move(m),
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
        std::thread::sleep(SETTLE);
    }

    fn advance(&mut self) -> Progress {
        std::thread::sleep(LOOK);
        Progress::Advanced
    }

    /// The console's window as the system draws it (macOS `screencapture`; it needs the
    /// window on screen and the Screen Recording permission).
    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        let Some(number) = self.with_window(|w, _| crate::screens::window_number(w))? else { return Ok(None) };
        let path = std::env::temp_dir().join(format!("t1s_gcs_expect_{}.jpg", std::process::id()));
        let ok = std::process::Command::new("screencapture")
            .args(["-x", "-o", "-t", "jpg", "-l", &number.to_string()])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        let bytes = if ok { std::fs::read(&path).ok() } else { None };
        let _ = std::fs::remove_file(&path);
        Ok(bytes.filter(|b| !b.is_empty()).map(Screenshot::jpeg))
    }

    fn supports(&self, need: Need<'_>) -> bool {
        match need {
            Need::Observe(v) | Need::Arrange(v) => STATES.contains(&v.key.as_str()),
            Need::Set(v) => SETTABLE.contains(&v.key.as_str()),
            Need::Perform(e) => EVENTS.iter().any(|(k, _)| *k == e.key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The suite checks clean, and the driver can do everything the console's plain UI tests
    /// need. Running them needs the window and a vehicle: `--station --expect`.
    #[test]
    fn the_driver_covers_the_planning_and_command_tests() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../expectations");
        let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
        let (tx, _rx) = channel();
        let driver = ConsoleDriver::new(tx, false, vec![]);
        let v = &a.suite.vocabulary;
        let gcs = a.suite.sections.iter().find(|s| s.slug == "gcs").expect("a gcs section").id;
        let ours = a.suite.subtree(gcs);
        let supported = |p: &expectation_story::Plan| {
            p.start.iter().all(|(x, _)| driver.supports(Need::Arrange(v.var(x))))
                && p.steps.iter().flat_map(|s| &s.checks).chain(&p.start_checks).all(|c| driver.supports(Need::Observe(v.var(c.var))))
                && p.steps.iter().filter_map(|s| s.action.event).all(|e| driver.supports(Need::Perform(v.event(e))))
                && p.steps.iter().flat_map(|s| s.action.changes.vars()).all(|x| driver.supports(Need::Set(v.var(x))))
        };
        let plans: Vec<_> = a.plans.iter().filter(|p| ours.contains(&p.section)).collect();
        let n = plans.iter().filter(|p| supported(p)).count();
        println!("{n} of {} console tests are supported", plans.len());
        assert!(n >= 16, "only {n} of {} console tests are supported", plans.len());
    }
}
