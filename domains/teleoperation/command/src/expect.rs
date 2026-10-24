//! `--expect [suite]`: runs the acceptance tests of the "Towers" section against the live app,
//! each on a fresh practice game of its own server (`towers_server --blue person --red nobody
//! --speedup 10`, six ArduPlane SITL drones; it needs the AGENT build).
//!
//! The runner works on a thread of its own, through a driver that turns every call into a job for
//! the UI thread (polled between frames), as the console's does. Events are real mouse events
//! dispatched into the window where the button or the tower was painted, so they take the path a
//! click does. States are read back from the app's seat: the map the server sent it.

use crate::seat::Status;
use crate::server::Server;
use crate::ui::{self, CommanderView, MapView};
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use expectation_story::Status as TestStatus;
use gpui::{
    App, AppContext as _, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    PlatformInput, Point, Window, WindowHandle, point, px,
};
use recreation_towers::seat::Join;
use recreation_towers::{Order, Team, TeamView};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::cell::Cell;
use std::time::{Duration, Instant};

/// The process exit code once the run is over (0 when everything that could run passed).
pub static EXIT: AtomicI32 = AtomicI32::new(0);

pub struct Config {
    pub suite: PathBuf,
    pub only: Option<String>,
    /// Where the results and the screenshots go (`--docs`).
    pub docs: Option<PathBuf>,
    pub server: Arc<Mutex<Option<Server>>>,
    pub server_args: Vec<String>,
    /// The games have the 3D view, with the cameras of the drones.
    pub cameras: bool,
}

type Job = Box<dyn FnOnce(&mut App, WindowHandle<CommanderView>) + Send>;

/// How long the app waits for its seat on a fresh game, for the server's answer to an order,
/// and for the clock of a game.
const SEATED: Duration = Duration::from_secs(90);
const ANSWER: Duration = Duration::from_secs(5);
const GAME: Duration = Duration::from_secs(300);

const ORDERS: [&str; 3] =
    ["the order of the first Blue drone", "the order of the second Blue drone", "the order of the third Blue drone"];
const DRONES: [&str; 3] = ["the first Blue drone", "the second Blue drone", "the third Blue drone"];

/// The states a fresh game starts in: the section's start, the only one the app can arrange.
const START: &[(&str, &str)] = &[
    ("the game of Towers", "in play"),
    ("the time left", "480"),
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

const EVENTS: &[&str] = &[
    "the commander joins the game",
    "the commander selects the first Blue drone",
    "the commander sends the selected drone to the west tower",
    "the commander holds the selected drone",
    "the commander sends the selected drone home",
    "the commander flies the selected drone low",
    "the commander clicks the camera of the second Blue drone",
    "the game runs for 2 minutes",
    "the game runs for 3 minutes",
];

const OBSERVED: &[&str] = &[
    "the seat of the app",
    "the towers on the app's map",
    "the selected drone",
    "the unit panel of the app",
    "the class in the unit panel",
    "the mass in the unit panel",
    "the west tower on the Blue map",
    "the order of the first Blue drone",
    "the order of the second Blue drone",
    "the order of the third Blue drone",
    "the flight of the first Blue drone",
    "the camera feeds in the app",
];

/// The cameras need the 3D view to load the valley first.
const FEEDS: Duration = Duration::from_secs(180);

fn is(v: &str, subject: &str) -> bool {
    v.eq_ignore_ascii_case(subject)
}

fn text(v: &Value) -> String {
    v.as_f64().map_or_else(|| v.as_str().unwrap_or_default().to_lowercase(), |x| format!("{x}"))
}

/// An order as the spec words it.
fn order_name(order: Option<Order>, view: &TeamView) -> String {
    match order {
        None => "none".into(),
        Some(Order::Take(t)) => format!("take the {} tower", view.towers[t].name),
        Some(Order::Hold) => "hold".into(),
        Some(Order::Home) => "go home".into(),
        Some(Order::Goto(_)) => "go to a point".into(),
    }
}

fn holder_name(h: Option<Team>) -> &'static str {
    match h {
        Some(Team::Blue) => "held by Blue",
        Some(Team::Red) => "held by Red",
        None => "neutral",
    }
}

/// A left click at `at`, as the platform would deliver it.
fn click(window: &mut Window, cx: &mut App, at: Point<Pixels>) {
    press(window, cx, at, MouseButton::Left);
}

/// A click with `button`.
fn press(window: &mut Window, cx: &mut App, at: Point<Pixels>, button: MouseButton) {
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

struct AppDriver {
    tx: Sender<Job>,
    server: Arc<Mutex<Option<Server>>>,
    server_args: Vec<String>,
    cameras: bool,
    /// The game has been used: the next test needs a fresh one.
    used: bool,
    /// The game server died: no test can run, so the run stops.
    dead: Cell<bool>,
}

impl AppDriver {
    /// Runs `f` on the UI thread, with the app's view, and returns what it returns.
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut CommanderView, &mut Window, &mut Context<CommanderView>) -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = channel();
        let job: Job = Box::new(move |cx, app| {
            let _ = app.update(cx, |v, w, cx| tx.send(f(v, w, cx)));
        });
        self.tx.send(job).map_err(|_| "the app has quit".to_string())?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "the app did not answer".to_string())
    }

    /// Runs `f` on the UI thread with the window but not the view: a click dispatched here reaches
    /// the view's handlers, which update it.
    fn with_window<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Window, &mut App) -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = channel();
        let job: Job = Box::new(move |cx, app| {
            let _ = cx.update_window(app.into(), |_, w, cx| tx.send(f(w, cx)));
        });
        self.tx.send(job).map_err(|_| "the app has quit".to_string())?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "the app did not answer".to_string())
    }

    /// The map the app has now.
    fn view(&self) -> Result<Option<TeamView>, String> {
        self.call(|v, _, _| v.seat.state().view.clone())
    }

    /// Waits until `done` holds of the app's map, or `patience` runs out.
    fn until(&self, patience: Duration, what: &str, done: impl Fn(&TeamView) -> bool) -> Result<(), String> {
        let end = Instant::now() + patience;
        loop {
            if let Some(v) = self.view()?
                && done(&v)
            {
                return Ok(());
            }
            if Instant::now() > end {
                return Err(format!("timed out waiting for {what}"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Where to start looking after a failed test: the app's state, the end of the server's log,
    /// and the whole log saved beside it (the next test's server overwrites it).
    fn debug(&self, title: &str) {
        let state = self.call(|v, _, _| {
            let s = v.seat.state();
            let game = s.view.as_ref().map_or("no map yet".to_string(), |m| {
                let orders: Vec<_> = m.drones.iter().map(|d| order_name(d.order, m)).collect();
                format!("game time {:.0} s, outcome {:?}, orders {orders:?}", m.time, m.outcome)
            });
            format!("seat {:?}, selected {:?}, {game}", s.status, v.selected)
        });
        println!("expect:   app: {}", state.unwrap_or_else(|e| e));
        let log = crate::server::root().join("logs/towers/server.log");
        let Ok(text) = std::fs::read_to_string(&log) else {
            println!("expect:   no server log at {}", log.display());
            return;
        };
        let slug: String = title.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let saved = log.with_file_name(format!("failed-{slug}.log"));
        let kept = std::fs::write(&saved, &text).is_ok();
        println!("expect:   server log ({}):", if kept { saved.display().to_string() } else { log.display().to_string() });
        let lines: Vec<&str> = text.lines().collect();
        for l in &lines[lines.len().saturating_sub(12)..] {
            println!("expect:     {l}");
        }
    }

    /// Whether the game server has exited. The run cannot go on: the tests after this one stop.
    fn server_exited(&self) -> bool {
        self.dead.get() || self.server.lock().unwrap().as_mut().is_some_and(|s| s.exited().is_some())
    }

    fn died(&self) -> String {
        self.dead.set(true);
        self.server.lock().unwrap().as_mut().and_then(Server::exited).unwrap_or_else(|| "the game server exited".into())
    }

    fn seated(&self) -> Result<(), String> {
        let end = Instant::now() + SEATED;
        loop {
            let (status, view) = self.call(|v, _, _| {
                let s = v.seat.state();
                (s.status.clone(), s.view.is_some())
            })?;
            match status {
                Status::Seated(_) if view => return Ok(()),
                Status::Refused(why) => return Err(format!("the server refused the seat: {why}")),
                _ if self.server_exited() => return Err(self.died()),
                _ if Instant::now() > end => return Err("the app got no seat on the game server".into()),
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    /// Clicks the button `id` where it is painted. The window is drawn first, as GPUI does before a
    /// key press: a window that gets no frames (the screen locked or asleep, or the window
    /// covered) would otherwise take the click on the frame before.
    fn click_button(&self, id: &'static str) -> Result<(), String> {
        self.with_window(move |w, cx| {
            w.draw(cx).clear();
            let b = ui::button_bounds(id).ok_or_else(|| format!("the {id} button isn't on screen"))?;
            click(w, cx, b.center());
            Ok::<(), String>(())
        })?
    }

    /// Right-clicks the tower `name` on the map: an order for the selected drones.
    fn click_tower(&self, name: &'static str) -> Result<(), String> {
        let (x, y) = self.call(move |v, _, _| {
            let (view, rules) = {
                let s = v.seat.state();
                (s.view.clone(), s.rules)
            };
            let (Some(view), Some(rules)) = (view, rules) else { return Err("the app has no map".to_string()) };
            let tower = view.towers.iter().find(|t| t.name == name).ok_or(format!("no {name} tower"))?;
            Ok(MapView::new(v.map_bounds.get(), &rules).to_screen(tower.at))
        })??;
        self.with_window(move |w, cx| {
            w.draw(cx).clear();
            press(w, cx, point(px(x), px(y)), MouseButton::Right)
        })
    }

    /// Waits for the server's echo of drone `k`'s order.
    fn echoed(&self, k: usize, want: &'static str) -> Result<(), String> {
        let want = want.to_string();
        self.until(ANSWER, "the server's answer to the order", move |v| {
            v.drones.get(k).is_some_and(|d| order_name(d.order, v) == want)
        })
    }
}

impl Driver for AppDriver {
    fn reset(&mut self) -> Result<(), String> {
        if self.used {
            // A fresh game: a new server, and the seat taken again. The seat hangs up first, or
            // it would come back to the new server as it starts, and take the seat itself.
            self.call(|v, _, _| v.seat.hang_up())?;
            let mut s = self.server.lock().unwrap();
            s.take();
            *s = Some(Server::start(&self.server_args, self.cameras)?);
            drop(s);
            self.call(|v, _, _| {
                v.seat.reconnect();
                v.select(None);
            })?;
        }
        self.used = true;
        self.seated()
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        let subject = var.subject.as_str();
        if START.iter().any(|(s, _)| is(subject, s)) {
            return if self.can_arrange(var, value) {
                Ok(())
            } else {
                Err(format!("a fresh game has another {subject}"))
            };
        }
        if is(subject, "the selected drone") {
            return self.set(var, value);
        }
        if is(subject, "the order of the first Blue drone") && text(value) == "take the west tower" {
            self.call(|v, _, _| {
                let west = v.seat.state().view.as_ref().and_then(|m| m.towers.iter().position(|t| t.name == "west"));
                if let Some(t) = west {
                    v.seat.order(0, Order::Take(t), false);
                }
            })?;
            return self.echoed(0, "take the west tower");
        }
        if is(subject, "the flight of the first Blue drone") {
            // A fresh game flies high. For low: select B1 and click LOW, as a commander does.
            if text(value) == "high" {
                return Ok(());
            }
            self.click_button("drone-0")?;
            self.click_button("low")?;
            return self.until(ANSWER, "the server's answer to LOW", |v| v.drones.first().is_some_and(|d| d.low));
        }
        Err(format!("the app can't arrange {subject}"))
    }

    fn can_arrange(&self, var: &Variable, value: &Value) -> bool {
        let subject = var.subject.as_str();
        START.iter().any(|(s, v)| is(subject, s) && text(value) == v.to_lowercase())
            || is(subject, "the selected drone")
            || (is(subject, "the order of the first Blue drone") && text(value) == "take the west tower")
            || is(subject, "the flight of the first Blue drone")
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        if !is(&var.subject, "the selected drone") {
            return Err(format!("the app can't set {}", var.subject));
        }
        let want = text(value);
        match DRONES.iter().position(|d| d.to_lowercase() == want) {
            Some(k) => self.click_button(["drone-0", "drone-1", "drone-2"][k]),
            None => self.call(|v, _, _| v.select(None)),
        }
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        match event.text.to_lowercase().as_str() {
            "the commander joins the game" => self.seated(),
            "the commander selects the first blue drone" => self.click_button("drone-0"),
            "the commander sends the selected drone to the west tower" => self.click_tower("west"),
            "the commander holds the selected drone" => self.click_button("hold"),
            "the commander sends the selected drone home" => self.click_button("home"),
            "the commander flies the selected drone low" => {
                self.click_button("low")?;
                self.until(ANSWER, "the server's answer to LOW", |v| v.drones.first().is_some_and(|d| d.low))
            }
            "the commander clicks the camera of the second blue drone" => self.click_button("camera-1"),
            e @ ("the game runs for 2 minutes" | "the game runs for 3 minutes") => {
                let minutes: f32 = if e.ends_with("2 minutes") { 2.0 } else { 3.0 };
                let from = self.view()?.map_or(0.0, |v| v.time);
                let end = Instant::now() + GAME;
                loop {
                    let v = self.view()?;
                    if v.as_ref().is_some_and(|v| v.time >= from + 60.0 * minutes || v.outcome.is_some()) {
                        return Ok(());
                    }
                    if Instant::now() > end {
                        return Err(format!("the game's clock did not run for {minutes} minutes"));
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            e => Err(format!("the app can't make {e} happen")),
        }
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        let subject = var.subject.as_str();
        if let Some((_, v)) = START.iter().find(|(s, _)| is(subject, s)) {
            // Each test starts on a fresh game.
            return Ok(v.parse::<f64>().map_or_else(|_| Value::from(*v), Value::from));
        }
        let lower = subject.to_lowercase();
        if lower == "the camera feeds in the app" {
            // The 3D view loads the valley first, which takes a while.
            let end = Instant::now() + FEEDS;
            loop {
                let live = self.call(|v, _, _| v.cameras.live())?;
                if live >= 3 || Instant::now() > end {
                    return Ok(Value::from(live as f64));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        let (status, view, selected) = self.call(|v, _, _| {
            let s = v.seat.state();
            (s.status.clone(), s.view.clone(), v.selected.first().copied())
        })?;
        if lower == "the seat of the app" {
            return Ok(Value::from(match status {
                Status::Seated(Join::Command(Team::Blue)) => "Blue",
                Status::Refused(_) => "refused",
                _ => "none",
            }));
        }
        if lower == "the selected drone" {
            return Ok(Value::from(selected.map_or("none", |k| DRONES[k])));
        }
        let view = view.ok_or("the app has no map yet")?;
        if lower.starts_with("the unit panel") || lower.ends_with("in the unit panel") {
            let rules = self.call(|v, _, _| v.seat.state().rules)?.ok_or("the app has no rules yet")?;
            let class = selected.filter(|&k| k < view.drones.len()).map(|k| rules.class(k));
            return Ok(match lower.as_str() {
                "the unit panel of the app" => Value::from(if class.is_some() { "shown" } else { "hidden" }),
                "the class in the unit panel" => Value::from(class.map_or("none", |c| c.name())),
                _ => Value::from(class.map_or(0.0, |c| (c.mass_kg() * 100.0).round() as f64 / 100.0)),
            });
        }
        if lower == "the towers on the app's map" {
            return Ok(Value::from(view.towers.len() as f64));
        }
        if lower == "the west tower on the blue map" {
            let west = view.towers.iter().find(|t| t.name == "west").ok_or("no west tower")?;
            return Ok(Value::from(holder_name(west.holder)));
        }
        if lower == "the flight of the first blue drone" {
            return Ok(Value::from(if view.drones[0].low { "low" } else { "high" }));
        }
        if let Some(k) = ORDERS.iter().position(|o| o.to_lowercase() == lower) {
            return Ok(Value::from(order_name(view.drones[k].order, &view)));
        }
        Err(format!("can't observe {subject}"))
    }

    fn settle(&mut self) {
        // The server answers an order in its next map, a quarter of a second later.
        std::thread::sleep(Duration::from_millis(700));
    }

    fn advance(&mut self) -> Progress {
        Progress::Static
    }

    /// The app's window as the system draws it.
    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        Ok(self.with_window(crate::snapshot::window_jpeg)?.map(Screenshot::jpeg))
    }

    fn supports(&self, need: Need<'_>) -> bool {
        match need {
            Need::Arrange(v) => {
                START.iter().any(|(s, _)| is(&v.subject, s))
                    || is(&v.subject, "the selected drone")
                    || is(&v.subject, "the order of the first Blue drone")
                    || is(&v.subject, "the flight of the first Blue drone")
            }
            Need::Set(v) => is(&v.subject, "the selected drone"),
            Need::Perform(e) => EVENTS.iter().any(|s| is(&e.text, s)),
            Need::Observe(v) => {
                OBSERVED.iter().any(|s| is(&v.subject, s)) || START.iter().any(|(s, _)| is(&v.subject, s))
            }
        }
    }
}

/// Loads the suite, then runs the section's tests on a thread of their own and quits.
pub fn start(cx: &mut App, app: WindowHandle<CommanderView>, config: Config) {
    let Config { suite, only, docs, server, server_args, cameras } = config;
    let mut acceptance = match Acceptance::load_for(&suite, "desktop") {
        Ok(a) => a.with_coverage(),
        Err(e) => {
            eprint!("{e}");
            std::process::exit(1);
        }
    };
    let section = acceptance.suite.sections.iter().find(|s| s.slug == "towers").map(|s| s.id);
    let ours = section.map(|s| acceptance.suite.subtree(s)).unwrap_or_default();
    acceptance.plans.retain(|p| ours.contains(&p.section));
    if let Some(only) = &only {
        let only = only.to_lowercase();
        acceptance.plans.retain(|p| p.title.to_lowercase().contains(&only));
    }
    let (tx, rx) = channel::<Job>();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(5)).await;
            while let Ok(job) = rx.try_recv() {
                let _ = cx.update(|cx| job(cx, app));
            }
        }
    })
    .detach();
    std::thread::spawn(move || {
        let mut driver = AppDriver { tx, server: server.clone(), server_args, cameras, used: false, dead: Cell::new(false) };
        // The section's tests of the rules (with no window) and of the server are not the app's.
        let all = acceptance.plans.len();
        let plans: Vec<_> =
            acceptance.plans.iter().filter(|p| driver_support(&acceptance, p, &driver)).cloned().collect();
        println!("expect: {} of the section's {all} tests are the app's", plans.len());
        let mut report = expectation_story::Report::default();
        for plan in &plans {
            let began = Instant::now();
            let r = expectation_harness::run(
                &acceptance.suite,
                std::slice::from_ref(plan),
                &mut driver,
                &RunOptions { docs_dir: docs.clone(), ..Default::default() },
            );
            let r = r.results.into_iter().next().unwrap();
            match r.status {
                TestStatus::Passed => println!("expect: ✓ {} ({:.0} s)", plan.title, began.elapsed().as_secs_f32()),
                _ => {
                    println!("expect: ✗ {} {:?} ({:.0} s)", plan.title, r.status, began.elapsed().as_secs_f32());
                    driver.debug(&plan.title);
                }
            }
            report.results.push(r);
            if driver.dead.get() {
                println!("expect: stopping, the remaining tests need the game server");
                break;
            }
        }
        let acceptance = Acceptance { plans, ..acceptance };
        if let Some(dir) = &docs {
            // Saved as the app's results, and merged with the other runners' when the manual is written.
            match acceptance.publish(dir, "towers-app", &report) {
                Ok(_) => println!("expect: wrote the results to {}", dir.display()),
                Err(e) => eprintln!("expect: couldn't write the results to {}: {e}", dir.display()),
            }
        }
        if !report.passed() {
            eprint!("{}", report.failures(&acceptance.suite, &acceptance.plans));
            EXIT.store(1, Ordering::SeqCst);
        }
        server.lock().unwrap().take();
        let _ = driver.call(|_, _, cx| cx.quit());
    });
}

/// Whether the driver can run every step of `plan` (a test of the rules with no window, or of
/// the server, is not the app's).
fn driver_support(a: &Acceptance, plan: &expectation_story::Plan, d: &AppDriver) -> bool {
    let v = &a.suite.vocabulary;
    plan.start.iter().all(|(x, _)| d.supports(Need::Arrange(v.var(x))))
        && plan
            .steps
            .iter()
            .flat_map(|s| &s.checks)
            .chain(&plan.start_checks)
            .all(|c| d.supports(Need::Observe(v.var(c.var))))
        && plan.steps.iter().filter_map(|s| s.action.event).all(|e| d.supports(Need::Perform(v.event(e))))
        && plan.steps.iter().flat_map(|s| s.action.changes.vars()).all(|x| d.supports(Need::Set(v.var(x))))
}
