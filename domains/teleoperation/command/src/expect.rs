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
use crate::ui::{self, CommanderView};
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use expectation_story::Status as TestStatus;
use gpui::{
    App, AppContext as _, Context, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, ScrollDelta, ScrollWheelEvent, TouchPhase, Window,
    WindowHandle, point, px,
};
use recreation_towers::seat::Join;
use recreation_towers::{Order, Team, TeamView};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
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

const EVENTS: &[&str] = &[
    "the commander joins the game",
    "the commander waits for the cameras",
    "the commander selects the first Blue drone",
    "the commander sends the selected drone to the west tower",
    "the commander holds the selected drone",
    "the commander sends the selected drone home",
    "the commander flies the selected drone low",
    "the commander clicks the camera of the second Blue drone",
    "the game runs for 2 minutes",
    "the game runs for 3 minutes",
    "the commander waits for the battlefield",
    "the commander clicks the west tower on the minimap",
    "the commander pans the battlefield to the front",
    "the commander moves the mouse to the top edge of the battlefield",
    "the commander turns the battlefield camera to the right",
    "the commander zooms the battlefield in",
    "the commander centres the battlefield on the selected drone",
    "the commander draws a circle of 120 m on the battlefield",
    "the commander adds a point to the route of the selected drone",
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
    "the drones that the app flies in AGENT mode",
    "the position reports that the app heard from its drones",
    "the maps of the game that the game server sent the app",
    "the towers held by Blue in the top bar",
    "the first Blue drone",
    "the radius of the circle of the first Blue drone",
    "the distance of the first Blue drone from the centre of its circle",
    "the points in the route of the first Blue drone",
    "the picture in the portrait",
    "the battlefield picture in the app",
    "the point of the battlefield camera",
    "the distance from the Blue base to the battlefield camera's point",
    "the heading of the battlefield camera",
    "the height of the battlefield camera",
];

/// The states of the battlefield's camera that a test sets up, as the commander would with the keys
/// and the wheel.
const CAMERA: [&str; 3] = [
    "the heading of the battlefield camera",
    "the height of the battlefield camera",
    "the point of the battlefield camera",
];

/// The camera looks at a place when the middle of the picture is this near it (m).
const LOOKS_AT: f32 = 100.0;

/// How long a key is held, or the mouse kept at an edge, to move the camera.
const HOLD: Duration = Duration::from_secs(1);

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
        Some(Order::Circle(..)) => "circle a point".into(),
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

/// A key pressed (`down`) or let go, as the platform would deliver it.
fn key(window: &mut Window, cx: &mut App, key: &str, down: bool) {
    let keystroke = Keystroke::parse(key).expect("a key");
    let e = if down {
        PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false })
    } else {
        PlatformInput::KeyUp(KeyUpEvent { keystroke })
    };
    let _ = window.dispatch_event(e, cx);
}

/// A key pressed and let go.
fn tap(window: &mut Window, cx: &mut App, k: &str) {
    key(window, cx, k, true);
    key(window, cx, k, false);
}

/// The mouse moved to `at`, with no button down.
fn hover(window: &mut Window, cx: &mut App, at: Point<Pixels>) {
    let e = MouseMoveEvent { position: at, pressed_button: None, modifiers: Modifiers::default() };
    let _ = window.dispatch_event(PlatformInput::MouseMove(e), cx);
}

/// A press of the left button at `from`, a drag to `to`, and a let-go there.
fn drag(window: &mut Window, cx: &mut App, from: Point<Pixels>, to: Point<Pixels>) {
    let modifiers = Modifiers::default();
    let mut send = |e: PlatformInput| {
        let _ = window.dispatch_event(e, cx);
    };
    send(PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers }));
    send(PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: from,
        modifiers,
        click_count: 1,
        first_mouse: false,
    }));
    for i in 1..=8 {
        let at = from + (to - from) * (i as f32 / 8.0);
        send(PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: Some(MouseButton::Left),
            modifiers,
        }));
    }
    send(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: to, modifiers, click_count: 1 }));
}

/// A right-click with Shift held.
fn shift_right_click(window: &mut Window, cx: &mut App, at: Point<Pixels>) {
    let modifiers = Modifiers { shift: true, ..Default::default() };
    let mut send = |e: PlatformInput| {
        let _ = window.dispatch_event(e, cx);
    };
    send(PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: None, modifiers }));
    send(PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Right,
        position: at,
        modifiers,
        click_count: 1,
        first_mouse: false,
    }));
    send(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position: at, modifiers, click_count: 1 }));
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
    /// What the driver did in this test, and when (s since the test began): for a failure's report.
    trace: RefCell<Vec<(f32, String)>>,
    began: Cell<Instant>,
    /// The time the whole suite has (`SIM001_SUITE_BUDGET`): no wait outlasts it.
    suite: suite_budget::Budget,
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
        let end = Instant::now() + self.suite.cap(patience);
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
    fn note(&self, what: impl Into<String>) {
        self.trace.borrow_mut().push((self.began.get().elapsed().as_secs_f32(), what.into()));
    }

    fn debug(&self, title: &str) {
        println!("expect:   steps (s since the test began):");
        for (t, what) in self.trace.borrow().iter() {
            println!("expect:     {t:6.1}  {what}");
        }
        let state = self.call(|v, _, _| {
            let s = v.seat.state();
            let game = s.view.as_ref().map_or("no map yet".to_string(), |m| {
                let orders: Vec<_> = m.drones.iter().map(|d| order_name(d.order, m)).collect();
                format!("game time {:.0} s, outcome {:?}, orders {orders:?}", m.time, m.outcome)
            });
            format!("seat {:?}, selected {:?}, {game}", s.status, v.selected)
        });
        println!("expect:   app: {}", state.unwrap_or_else(|e| e));
        let log = crate::server::log_path();
        let Ok(text) = std::fs::read_to_string(&log) else {
            println!("expect:   no server log at {}", log.display());
            return;
        };
        let slug: String =
            title.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let saved = log.with_file_name(format!("failed-{slug}.log"));
        let kept = std::fs::write(&saved, &text).is_ok();
        println!(
            "expect:   server log ({}):",
            if kept { saved.display().to_string() } else { log.display().to_string() }
        );
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
        let end = Instant::now() + self.suite.cap(SEATED);
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
        // Draw first: the map's bounds are those of the last frame, and the layout may have changed
        // since (a selection shows the UNIT panel), which would move the tower under the click.
        self.with_window(|w, cx| w.draw(cx).clear())?;
        let shown = self.call(move |v, _, _| {
            let view = v.seat.state().view.clone().ok_or("the app has no map".to_string())?;
            let tower = view.towers.iter().find(|t| t.name == name).ok_or(format!("no {name} tower"))?;
            let b = v.map_bounds.get();
            Ok::<_, String>(v.screen_of(tower.at).filter(|&(x, y)| b.contains(&point(px(x), px(y)))))
        })??;
        // Off the battlefield: the commander first looks there, with the minimap.
        let (x, y) = match shown {
            Some(at) => at,
            None => {
                self.click_minimap_tower(name)?;
                self.with_window(|w, cx| w.draw(cx).clear())?;
                self.call(move |v, _, _| {
                    let at = v.seat.state().view.as_ref()?.towers.iter().find(|t| t.name == name)?.at;
                    v.screen_of(at)
                })?
                .ok_or(format!("the {name} tower is not on the battlefield"))?
            }
        };
        self.with_window(move |w, cx| press(w, cx, point(px(x), px(y)), MouseButton::Right))
    }

    /// Clicks the tower `name` on the minimap.
    fn click_minimap_tower(&self, name: &'static str) -> Result<(), String> {
        self.with_window(|w, cx| w.draw(cx).clear())?;
        let (x, y) = self.call(move |v, _, _| {
            let view = v.seat.state().view.clone().ok_or("the app has no map")?;
            let tower = view.towers.iter().find(|t| t.name == name).ok_or(format!("no {name} tower"))?;
            v.minimap_screen(tower.at).ok_or("there is no minimap".to_string())
        })??;
        self.with_window(move |w, cx| click(w, cx, point(px(x), px(y))))
    }

    /// Holds the key `k` down for a while, then lets it go.
    fn hold_key(&self, k: &'static str) -> Result<(), String> {
        self.with_window(move |w, cx| key(w, cx, k, true))?;
        std::thread::sleep(HOLD);
        self.with_window(move |w, cx| key(w, cx, k, false))
    }

    /// Presses the key `k` once.
    fn press_key(&self, k: &'static str) -> Result<(), String> {
        self.with_window(move |w, cx| {
            w.draw(cx).clear();
            tap(w, cx, k)
        })
    }

    /// The camera of the battlefield now, and the rules.
    fn eye(&self) -> Result<(teleoperation_shared::battlefield::Eye, recreation_towers::Rules), String> {
        let (eye, rules) = self.call(|v, _, _| (v.rig.eye, v.seat.state().rules))?;
        Ok((eye, rules.ok_or("the app has no rules yet")?))
    }

    /// The ground point of the battlefield for the next point of a route or the centre of a circle:
    /// ahead of the camera's point, apart from the last, away from the towers, and with room for a
    /// circle of `room` round it inside the field (the mountains at its edge crash a drone that
    /// circles into them; a point of a route needs only to be clear of them).
    /// With `far`, the open point furthest from the first drone: the game runs at ten times
    /// real time, and a drone sent to a near point is there before the next step looks.
    fn open_point(&self, room: f32, far: bool) -> Result<glam::Vec2, String> {
        self.call(move |v, _, _| {
            let s = v.seat.state();
            let view = s.view.clone();
            // Inside the field, by more than a circle's room (the app moves a point outside it in).
            let inside = s.rules.as_ref().map_or(f32::MAX, |r| r.field_radius - room - ROOM_MARGIN);
            drop(s);
            let look = v.rig.eye.look;
            let h = v.rig.eye.heading.to_radians();
            let ahead = glam::Vec2::new(h.cos(), h.sin());
            let valley = v.valley().cloned();
            let towers: Vec<glam::Vec2> =
                view.as_ref().map(|m| m.towers.iter().map(|t| t.at).collect()).unwrap_or_default();
            // Away from the first drone and the points it already has, so that each is a new goal.
            let mut taken: Vec<glam::Vec2> = v.route(0).to_vec();
            let drone = view.as_ref().and_then(|m| m.drones.first()).map(|d| d.at);
            taken.extend(drone);
            // Anywhere on the field (the commander looks there first if it is off the picture).
            let grid =
                (-20..=20).flat_map(|i| (-20..=20).map(move |j| glam::Vec2::new(40.0 * i as f32, 40.0 * j as f32)));
            let mut open: Vec<glam::Vec2> = grid
                .filter(|p| {
                    p.length() < inside
                        && towers.iter().all(|t| t.distance(*p) > 120.0)
                        && taken.iter().all(|t| t.distance(*p) > 250.0)
                        && valley.as_ref().is_none_or(|m| room_to_circle(m, *p, room))
                })
                .collect();
            // Nearest to where the camera looks, ahead of it first; or furthest from the drone.
            let ahead_first = |p: &glam::Vec2| (*p - look).length() - 0.5 * (*p - look).dot(ahead);
            open.sort_by(|a, b| ahead_first(a).total_cmp(&ahead_first(b)));
            let found = match (far, drone) {
                (true, Some(d)) => open.iter().copied().max_by(|a, b| a.distance(d).total_cmp(&b.distance(d))),
                _ => open.first().copied(),
            };
            found.ok_or_else(|| "no open ground on the field".to_string())
        })?
    }

    /// CIRCLE (C), then a drag on the battlefield from an open point out to `radius` metres.
    fn draw_circle(&self, radius: f32) -> Result<(), String> {
        let centre = self.open_point(radius, false)?;
        let edge = move |v: &CommanderView| {
            let h = v.rig.eye.heading.to_radians();
            centre + glam::Vec2::new(-h.sin(), h.cos()) * radius
        };
        self.look_at(centre, move |v| vec![centre, edge(v)])?;
        self.press_key("c")?;
        let (from, to) = self
            .call(move |v, _, _| Some((v.screen_of(centre)?, v.screen_of(edge(v))?)))?
            .ok_or("the circle is not on the battlefield")?;
        self.with_window(move |w, cx| drag(w, cx, point(px(from.0), px(from.1)), point(px(to.0), px(to.1))))
    }

    /// Shift and a right-click on an open point of the battlefield.
    fn add_route_point(&self) -> Result<(), String> {
        let at = self.open_point(0.0, true)?;
        self.look_at(at, move |_| vec![at])?;
        let (x, y) = self.call(move |v, _, _| v.screen_of(at))?.ok_or("the point is not on the battlefield")?;
        self.with_window(move |w, cx| shift_right_click(w, cx, point(px(x), px(y))))?;
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    /// Unless the points that `points` gives are well inside the battlefield already, the
    /// commander first looks at `at` with the minimap, as for a tower off the battlefield. (Far
    /// up the picture a point is under the edge, and a drag there is too short to be one.)
    fn look_at(
        &self,
        at: glam::Vec2,
        points: impl Fn(&CommanderView) -> Vec<glam::Vec2> + Send + Clone + 'static,
    ) -> Result<(), String> {
        self.with_window(|w, cx| w.draw(cx).clear())?;
        let inside = move |v: &CommanderView| {
            let b = v.map_bounds.get();
            let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
            let (x0, y0) = (f32::from(b.origin.x), f32::from(b.origin.y));
            points(v).iter().all(|p| {
                v.screen_of(*p).is_some_and(|(x, y)| {
                    x > x0 + 0.15 * w && x < x0 + 0.85 * w && y > y0 + 0.25 * h && y < y0 + 0.85 * h
                })
            })
        };
        let check = inside.clone();
        if self.call(move |v, _, _| check(v))? {
            return Ok(());
        }
        let (x, y) =
            self.call(move |v, _, _| v.minimap_screen(at))?.ok_or_else(|| "there is no minimap".to_string())?;
        self.with_window(move |w, cx| click(w, cx, point(px(x), px(y))))?;
        self.with_window(|w, cx| w.draw(cx).clear())?;
        Ok(())
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
        self.trace.borrow_mut().clear();
        self.began.set(Instant::now());
        self.note("reset");
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
                v.fresh();
            })?;
        }
        self.used = true;
        self.seated()
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        self.note(format!("arrange: {} = {}", var.subject, text(value)));
        let subject = var.subject.as_str();
        if START.iter().any(|(s, _)| is(subject, s)) {
            return if self.can_arrange(var, value) {
                Ok(())
            } else {
                Err(format!("a fresh game has another {subject}"))
            };
        }
        if is(subject, "the selected drone") || CAMERA.iter().any(|c| is(subject, c)) {
            return self.set(var, value);
        }
        if is(subject, "the order of the first Blue drone") && text(value) == "circle a point" {
            // As a commander does: B1 selected, and a circle drawn.
            self.click_button("drone-0")?;
            self.draw_circle(120.0)?;
            return self.echoed(0, "circle a point");
        }
        if is(subject, "the points in the route of the first Blue drone") {
            let want = value.as_f64().ok_or("a number of points")? as usize;
            let have = self.call(|v, _, _| v.route_len(0))?;
            if want < have {
                return Err("the app can't take points out of a route".into());
            }
            if want > have {
                self.click_button("drone-0")?;
                for _ in have..want {
                    self.add_route_point()?;
                }
            }
            return Ok(());
        }
        if is(subject, "the first Blue drone") && text(value) == "in the west circle" {
            // Sent to the west tower, as a commander does, and on its way until it circles it.
            self.call(|v, _, _| {
                let west = v.seat.state().view.as_ref().and_then(|m| m.towers.iter().position(|t| t.name == "west"));
                if let Some(t) = west {
                    v.seat.order(0, Order::Take(t), false);
                }
            })?;
            let circle = self.call(|v, _, _| v.seat.state().rules.map_or(60.0, |r| r.circle_radius))?;
            return self.until(GAME, "the first Blue drone in the west circle", move |v| {
                let west = v.towers.iter().find(|t| t.name == "west");
                west.zip(v.drones.first()).is_some_and(|(t, d)| d.at.distance(t.at) < circle + 20.0)
            });
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
            || CAMERA.iter().any(|c| is(subject, c))
            || (is(subject, "the first Blue drone") && text(value) == "in the west circle")
            || (is(subject, "the order of the first Blue drone")
                && (text(value) == "take the west tower" || text(value) == "circle a point"))
            || is(subject, "the points in the route of the first Blue drone")
            || is(subject, "the flight of the first Blue drone")
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        if is(&var.subject, CAMERA[0]) {
            let heading = value.as_f64().ok_or("a heading is a number")? as f32;
            return self.call(move |v, _, cx| {
                v.rig.eye.heading = heading;
                cx.notify();
            });
        }
        if is(&var.subject, CAMERA[2]) {
            let want = text(value);
            let at = self.call(move |v, _, _| {
                let s = v.seat.state();
                let rules = s.rules?;
                match want.as_str() {
                    "the blue base" => Some(rules.base(Team::Blue)),
                    "the red base" => Some(rules.base(Team::Red)),
                    t => s.view.as_ref()?.towers.iter().find(|x| format!("the {} tower", x.name) == t).map(|x| x.at),
                }
            })?;
            let at = at.ok_or_else(|| format!("no place {}", text(value)))?;
            return self.call(move |v, _, cx| {
                v.rig.eye.look = at;
                cx.notify();
            });
        }
        if is(&var.subject, CAMERA[1]) {
            let height = value.as_f64().ok_or("a height is a number")? as f32;
            return self.call(move |v, _, cx| {
                v.rig.eye.distance = height / v.rig.eye.pitch.to_radians().sin();
                cx.notify();
            });
        }
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
        self.note(format!("perform: {}", event.text));
        match event.text.to_lowercase().as_str() {
            "the commander joins the game" => self.seated(),
            // The check that follows waits for the pictures.
            "the commander waits for the cameras" => Ok(()),
            "the commander selects the first blue drone" => self.click_button("drone-0"),
            "the commander sends the selected drone to the west tower" => self.click_tower("west"),
            "the commander holds the selected drone" => self.click_button("hold"),
            "the commander sends the selected drone home" => self.click_button("home"),
            "the commander flies the selected drone low" => {
                self.click_button("low")?;
                self.until(ANSWER, "the server's answer to LOW", |v| v.drones.first().is_some_and(|d| d.low))
            }
            "the commander clicks the camera of the second blue drone" => self.click_button("camera-1"),
            // The check that follows waits for the picture.
            "the commander waits for the battlefield" => Ok(()),
            "the commander clicks the west tower on the minimap" => self.click_minimap_tower("west"),
            "the commander pans the battlefield to the front" => self.hold_key("w"),
            "the commander moves the mouse to the top edge of the battlefield" => {
                let b = self.call(|v, _, _| v.map_bounds.get())?;
                let (top, middle) = (point(b.center().x, b.origin.y + px(2.0)), b.center());
                self.with_window(move |w, cx| hover(w, cx, top))?;
                std::thread::sleep(HOLD);
                self.with_window(move |w, cx| hover(w, cx, middle))
            }
            "the commander turns the battlefield camera to the right" => self.press_key("e"),
            "the commander zooms the battlefield in" => {
                let at = self.call(|v, _, _| v.map_bounds.get().center())?;
                self.with_window(move |w, cx| {
                    let e = ScrollWheelEvent {
                        position: at,
                        delta: ScrollDelta::Lines(point(0.0, 1.0)),
                        modifiers: Modifiers::default(),
                        touch_phase: TouchPhase::Moved,
                    };
                    let _ = w.dispatch_event(PlatformInput::ScrollWheel(e), cx);
                })
            }
            "the commander centres the battlefield on the selected drone" => self.press_key("space"),
            "the commander draws a circle of 120 m on the battlefield" => self.draw_circle(120.0),
            "the commander adds a point to the route of the selected drone" => self.add_route_point(),
            e @ ("the game runs for 2 minutes" | "the game runs for 3 minutes") => {
                let minutes: f32 = if e.ends_with("2 minutes") { 2.0 } else { 3.0 };
                let from = self.view()?.map_or(0.0, |v| v.time);
                let end = Instant::now() + self.suite.cap(GAME);
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
        self.note(format!("observe: {}", var.subject));
        let subject = var.subject.as_str();
        if let Some((_, v)) = START.iter().find(|(s, _)| is(subject, s)) {
            // Each test starts on a fresh game.
            return Ok(v.parse::<f64>().map_or_else(|_| Value::from(*v), Value::from));
        }
        let lower = subject.to_lowercase();
        if lower == "the drones that the app flies in agent mode" {
            let flown = self.call(|v, _, _| v.seat.flown.load(std::sync::atomic::Ordering::SeqCst))?;
            return Ok(Value::from(flown as f64));
        }
        if lower == "the position reports that the app heard from its drones" {
            let heard = self.call(|v, _, _| v.seat.heard.load(std::sync::atomic::Ordering::SeqCst))?;
            return Ok(Value::from(heard as f64));
        }
        if lower == "the maps of the game that the game server sent the app" {
            let maps = self.call(|v, _, _| v.seat.state().maps)?;
            return Ok(Value::from(maps as f64));
        }
        if lower == "the camera feeds in the app" {
            // The 3D view loads the valley first, which takes a while.
            let end = Instant::now() + self.suite.cap(FEEDS);
            loop {
                let live = self.call(|v, _, _| v.cameras.live())?;
                if live >= 3 || Instant::now() > end {
                    return Ok(Value::from(live as f64));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        if lower == "the battlefield picture in the app" {
            // The 3D view loads the valley first, which takes a while.
            let end = Instant::now() + self.suite.cap(FEEDS);
            loop {
                let live = self.call(|v, _, _| v.battlefield_live())?;
                if live || Instant::now() > end {
                    return Ok(Value::from(if live { "live" } else { "missing" }));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        if CAMERA.iter().any(|c| is(subject, c))
            || lower == "the point of the battlefield camera"
            || lower == "the distance from the blue base to the battlefield camera's point"
        {
            let (eye, rules) = self.eye()?;
            if is(subject, CAMERA[0]) {
                return Ok(Value::from(eye.heading.rem_euclid(360.0).round() as f64 % 360.0));
            }
            if is(subject, CAMERA[1]) {
                return Ok(Value::from(eye.height().round() as f64));
            }
            let base = rules.base(Team::Blue);
            if lower.starts_with("the distance") {
                return Ok(Value::from(eye.look.distance(base).round() as f64));
            }
            let view = self.view()?.ok_or("the app has no map yet")?;
            let mut places = vec![("the Blue base".to_string(), base), ("the Red base".into(), rules.base(Team::Red))];
            places.extend(view.towers.iter().map(|t| (format!("the {} tower", t.name), t.at)));
            let near = places
                .into_iter()
                .filter(|(_, at)| at.distance(eye.look) < LOOKS_AT)
                .min_by(|a, b| a.1.distance(eye.look).total_cmp(&b.1.distance(eye.look)));
            return Ok(Value::from(near.map_or("another place".to_string(), |(name, _)| name)));
        }
        if lower == "the first blue drone" {
            // Only where the app's map puts it in the west circle, or not.
            let circle = self.call(|v, _, _| v.seat.state().rules.map_or(60.0, |r| r.circle_radius))?;
            let view = self.view()?.ok_or("the app has no map yet")?;
            let west = view.towers.iter().find(|t| t.name == "west").ok_or("no west tower")?;
            let inside = view.drones.first().is_some_and(|d| d.at.distance(west.at) < circle + 20.0);
            return Ok(Value::from(if inside { "in the west circle" } else { "somewhere else" }));
        }
        if lower == "the points in the route of the first blue drone" {
            return Ok(Value::from(self.call(|v, _, _| v.route_len(0))? as f64));
        }
        if lower == "the radius of the circle of the first blue drone"
            || lower == "the distance of the first blue drone from the centre of its circle"
        {
            let view = self.view()?.ok_or("the app has no map yet")?;
            let d = view.drones.first().ok_or("no first drone")?;
            let Some(Order::Circle(centre, radius)) = d.order else {
                return Ok(Value::from(0.0));
            };
            let x = if lower.starts_with("the radius") { radius } else { d.at.distance(centre) };
            return Ok(Value::from(x.round() as f64));
        }
        if lower == "the towers held by blue in the top bar" {
            self.with_window(|w, cx| w.draw(cx).clear())?;
            let chips = self.call(|v, _, _| v.chips.borrow().clone())?;
            return Ok(Value::from(chips.iter().filter(|c| **c == Some(Team::Blue)).count() as f64));
        }
        if lower == "the picture in the portrait" {
            // The pictures come when the 3D view has loaded the valley.
            let end = Instant::now() + self.suite.cap(FEEDS);
            loop {
                self.with_window(|w, cx| w.draw(cx).clear())?;
                let (shown, selected) = self.call(|v, _, _| (v.portrait.get(), v.selected.first().copied()))?;
                if shown.is_some() || selected.is_none() || Instant::now() > end {
                    return Ok(Value::from(
                        shown.map_or("nothing".to_string(), |k| format!("the camera of {}", DRONES[k])),
                    ));
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
                    || is(&v.subject, "the first Blue drone")
                    || CAMERA.iter().any(|c| is(&v.subject, c))
                    || is(&v.subject, "the order of the first Blue drone")
                    || is(&v.subject, "the flight of the first Blue drone")
                    || is(&v.subject, "the points in the route of the first Blue drone")
            }
            Need::Set(v) => is(&v.subject, "the selected drone") || CAMERA.iter().any(|c| is(&v.subject, c)),
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
    let mut acceptance = match Acceptance::load_for(&suite, "sim") {
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
        let mut driver = AppDriver {
            tx,
            server: server.clone(),
            server_args,
            cameras,
            used: false,
            dead: Cell::new(false),
            trace: RefCell::default(),
            began: Cell::new(Instant::now()),
            suite: suite_budget::Budget::from_env(),
        };
        // The section's tests of the rules (with no window) and of the server are not the app's.
        let all = acceptance.plans.len();
        let plans: Vec<_> =
            acceptance.plans.iter().filter(|p| driver_support(&acceptance, p, &driver)).cloned().collect();
        println!("expect: {} of the section's {all} tests are the app's", plans.len());
        let mut report = expectation_story::Report::default();
        for plan in &plans {
            let began = Instant::now();
            let r = if driver.suite.spent() {
                driver.suite.ran_out(&acceptance.suite, plan, &driver)
            } else {
                expectation_harness::run(
                    &acceptance.suite,
                    std::slice::from_ref(plan),
                    &mut driver,
                    &RunOptions { docs_dir: docs.clone(), ..Default::default() },
                )
                .results
                .remove(0)
            };
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

/// Whether the app can arrange `var` in `c`: the first Blue drone only in the west circle.
fn arranges(var: &Variable, c: &expectation_model::Constraint) -> bool {
    if !is(&var.subject, "the first Blue drone") {
        return true;
    }
    let expectation_model::VarKind::Enum(values) = &var.kind else { return false };
    let Some(i) = values.iter().position(|v| v == "in the west circle") else { return false };
    matches!(c, expectation_model::Constraint::OneOf(m) if *m == 1 << i)
}

/// Whether the driver can run every step of `plan` (a test of the rules with no window, or of
/// the server, is not the app's).
fn driver_support(a: &Acceptance, plan: &expectation_story::Plan, d: &AppDriver) -> bool {
    let v = &a.suite.vocabulary;
    // A state that the app arranges only in some values (the first Blue drone in the west circle)
    // makes a test the app's only in those.
    plan.start.iter().all(|(x, c)| d.supports(Need::Arrange(v.var(x))) && arranges(v.var(x), c))
        && plan
            .steps
            .iter()
            .flat_map(|s| &s.checks)
            .chain(&plan.start_checks)
            .all(|c| d.supports(Need::Observe(v.var(c.var))))
        && plan.steps.iter().filter_map(|s| s.action.event).all(|e| d.supports(Need::Perform(v.event(e))))
        && plan.steps.iter().flat_map(|s| s.action.changes.vars()).all(|x| d.supports(Need::Set(v.var(x))))
}

/// Past a circle's radius, the room to the mountains (m): a circle is flown a little wide.
const ROOM_MARGIN: f32 = 60.0;

/// Is there room for a circle of `radius` round `p` inside the field: are `p`, and a ring round
/// it a margin wider than the circle, clear of the mountains?
fn room_to_circle(map: &recreation_towers::map::Map, p: glam::Vec2, radius: f32) -> bool {
    let ring = radius + ROOM_MARGIN;
    std::iter::once(p)
        .chain((0..24).map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            p + glam::Vec2::new(a.cos(), a.sin()) * ring
        }))
        .all(|q| map.mountain_height(q) < 1.0)
}

#[cfg(test)]
mod room_tests {
    use super::*;
    use recreation_towers::Rules;
    use recreation_towers::map::Map;

    /// The middle of the field has room for a circle; a centre near its edge doesn't, because
    /// part of the circle would be in the mountains, where a drone crashes.
    #[test]
    fn a_circle_needs_room_inside_the_field() {
        let map = Map::new(&Rules::default());
        assert!(room_to_circle(&map, glam::Vec2::ZERO, 120.0));
        let edge = (0..200)
            .map(|k| glam::Vec2::new(0.0, 10.0 * k as f32))
            .find(|p| map.mountain_height(*p) > 1.0)
            .expect("mountains east of the field");
        let near = edge - glam::Vec2::new(0.0, 100.0);
        assert!(map.mountain_height(near) < 1.0, "the centre itself is on the field");
        assert!(!room_to_circle(&map, near, 120.0), "but a circle round it reaches the mountains");
    }
}
