//! `--expect [suite]`: runs the acceptance tests of "The launcher" (expectations/10-launcher)
//! against the live window, and with `--docs DIR` writes their results and screenshots there.
//!
//! The runner works on a thread of its own, through a driver that turns every call into a job for
//! the UI thread (polled between frames), as the console's and the commander app's do. Events are
//! real mouse events dispatched where the button was painted, so they take the path a click does.
//! States are read back from the launcher: its choices, the job it started (the command line is
//! the first line of the log) and what it read from controls.cfg.
//!
//! The launcher starts a stand-in of itself in place of each program (see `job::use_stand_in`),
//! and keeps controls.cfg in a data folder of the run's own (`SIM001_DATA`).

use crate::app::{self, Launcher, Target};
use crate::job::layout;
use expectation_harness::{Acceptance, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use expectation_story::Status as TestStatus;
use gpui::{
    App, AppContext as _, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    PlatformInput, Point, Window, WindowHandle,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

/// The process exit code once the run is over (0 when everything that could run passed).
pub static EXIT: AtomicI32 = AtomicI32::new(0);

pub struct Config {
    pub suite: PathBuf,
    pub only: Option<String>,
    pub docs: Option<PathBuf>,
}

type Job = Box<dyn FnOnce(&mut App, WindowHandle<Launcher>) + Send>;

/// The launcher polls its program four times a second, and repaints.
const SETTLE: Duration = Duration::from_millis(700);
const POLL: Duration = Duration::from_millis(250);

const STATES: &[&str] = &[
    "the launch target",
    "the flight controller choice",
    "the place choice",
    "the ray tracing choice",
    "the red team",
    "the drone cameras choice",
    "the launch button",
    "the program in the log",
    "the program",
    "the plane of the started sim",
    "the sticks of the started station",
    "the place of the started program",
    "the ray tracing of the started program",
    "the red team of the started game",
    "the drone cameras of the started game",
    "the controls file",
];

/// The states a Given can arrange.
const ARRANGED: &[&str] = &[
    "the launch target",
    "the red team",
    "the drone cameras choice",
    "the flight controller choice",
    "the place choice",
    "the ray tracing choice",
    "the launch button",
    "the program in the log",
    "the program",
    "the controls file",
];

/// The choices a step can set.
const SETTABLE: &[&str] = &[
    "the launch target",
    "the flight controller choice",
    "the place choice",
    "the ray tracing choice",
    "the red team",
    "the drone cameras choice",
];

const EVENTS: &[&str] = &[
    "the pilot launches",
    "the pilot stops the program",
    "the pilot sets up the controller",
    "the radio wizard saves controls.cfg and closes",
    "the pilot renders the docs",
];

fn is(v: &str, subject: &str) -> bool {
    v.eq_ignore_ascii_case(subject)
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// A left click at `at`, as the platform would deliver it.
fn click(window: &mut Window, cx: &mut App, at: Point<Pixels>) {
    let modifiers = Modifiers::default();
    let mut send = |e: PlatformInput| {
        let _ = window.dispatch_event(e, cx);
    };
    send(PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: None, modifiers }));
    send(PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers,
        click_count: 1,
        first_mouse: false,
    }));
    send(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1 }));
}

/// What the launcher shows now, read on the UI thread.
#[derive(Clone)]
struct Seen {
    target: Target,
    flight_controller: bool,
    map: Option<String>,
    raytrace: bool,
    red_computer: bool,
    cameras: bool,
    busy: bool,
    /// The job's title, command line and whether it still runs.
    job: Option<(String, String, bool)>,
    mapped: bool,
}

impl Seen {
    fn of(l: &Launcher) -> Seen {
        Seen {
            target: l.choices.target,
            flight_controller: l.choices.flight_controller,
            map: l.choices.map.clone(),
            raytrace: l.choices.raytrace,
            red_computer: l.choices.red_computer,
            cameras: l.choices.cameras,
            busy: l.busy(),
            job: l.job.as_ref().map(|j| (j.title.clone(), j.command(), j.running())),
            mapped: l.mapping.exists,
        }
    }

    fn command(&self) -> &str {
        self.job.as_ref().map_or("", |(_, c, _)| c.as_str())
    }

    /// The command line's words.
    fn has(&self, words: &str) -> bool {
        format!(" {} ", self.command()).contains(&format!(" {words} "))
    }

    fn read(&self, subject: &str) -> Result<Value, String> {
        let s = |v: &str| Ok(Value::from(v));
        match subject.to_lowercase().as_str() {
            "the launch target" => s(match self.target {
                Target::Sim => "SIMULATOR ONLY",
                Target::Station => "GROUND STATION",
                Target::Towers => "TOWERS",
            }),
            "the red team" => s(if self.red_computer { "COMPUTER" } else { "NOBODY" }),
            "the drone cameras choice" => s(if self.cameras { "on" } else { "off" }),
            "the flight controller choice" => {
                s(if self.flight_controller { "WITH ARDUPILOT" } else { "WITHOUT ARDUPILOT" })
            }
            "the place choice" => Ok(Value::from(self.map.as_deref().unwrap_or("alpine valley").to_uppercase())),
            "the ray tracing choice" => s(if self.raytrace { "on" } else { "off" }),
            "the launch button" => s(if self.busy { "STOP" } else { "LAUNCH" }),
            "the program in the log" => s(match self.job.as_ref().map(|(t, _, _)| t.as_str()) {
                None => "nothing",
                Some(t) if t.starts_with("Simulator") => "the simulator",
                Some(t) if t.starts_with("Ground station") => "the ground station",
                Some(t) if t.starts_with("Towers") => "the Towers game",
                Some("Controller setup") => "the radio wizard",
                Some("Render the docs") => "the manual build",
                Some(t) => return Err(format!("the log names {t}")),
            }),
            "the program" => s(match &self.job {
                None => "idle",
                Some((_, _, true)) => "running",
                Some(_) => "stopped",
            }),
            "the plane of the started sim" => s(if self.has("--aircraft t1s-vtol") {
                "the T1-S VTOL"
            } else if self.has("--aircraft t1s-ranger") {
                "the T1-S Ranger"
            } else {
                "none"
            }),
            "the sticks of the started station" => {
                if !self.has("--station") {
                    return Err(format!("the started program is not the station: {}", self.command()));
                }
                s(if self.has("--direct") { "direct" } else { "through ArduPilot" })
            }
            "the place of the started program" => s(if self.has("--map cypress") {
                "Cypress"
            } else if self.has("--map berkeley") {
                "Berkeley"
            } else {
                "the Alpine Valley"
            }),
            "the red team of the started game" => {
                if !self.has("--serve") {
                    return Err(format!("the started program is not the Towers app: {}", self.command()));
                }
                s(if self.has("nobody") { "nobody" } else { "the computer" })
            }
            "the drone cameras of the started game" => s(if self.has("--no-cameras") { "off" } else { "on" }),
            "the ray tracing of the started program" => s(if self.has("--raytrace") { "on" } else { "off" }),
            "the controls file" => s(if self.mapped { "mapped" } else { "missing" }),
            other => Err(format!("can't observe {other}")),
        }
    }
}

struct LauncherDriver {
    tx: Sender<Job>,
}

impl LauncherDriver {
    /// Runs `f` on the UI thread, with the launcher, and returns what it returns.
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Launcher, &mut Window, &mut Context<Launcher>) -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = channel();
        let job: Job = Box::new(move |cx, app| {
            let _ = app.update(cx, |v, w, cx| tx.send(f(v, w, cx)));
        });
        self.tx.send(job).map_err(|_| "the launcher has quit".to_string())?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "the launcher did not answer".to_string())
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
        self.tx.send(job).map_err(|_| "the launcher has quit".to_string())?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "the launcher did not answer".to_string())
    }

    fn seen(&self) -> Result<Seen, String> {
        self.call(|l, _, _| Seen::of(l))
    }

    /// Clicks the button with this id where it is painted, and lets the window repaint. The window
    /// is drawn first: a button that has just appeared (STOP after LAUNCH) is only painted on the
    /// next frame, and a window the system doesn't show (the screen locked or asleep, or the window
    /// covered) gets no frames at all, so a click would land on the frame before. (GPUI draws a
    /// dirty window before a key press the same way.)
    fn click_button(&self, id: String) -> Result<(), String> {
        self.with_window(move |w, cx| {
            w.draw(cx).clear();
            let b = app::button_bounds(&id).ok_or_else(|| format!("the {id} button isn't on screen"))?;
            click(w, cx, b.center());
            Ok::<(), String>(())
        })??;
        std::thread::sleep(Duration::from_millis(150));
        Ok(())
    }

    /// Waits until `done` holds of what the launcher shows.
    fn until(&self, what: &str, done: impl Fn(&Seen) -> bool) -> Result<(), String> {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if done(&self.seen()?) {
                return Ok(());
            }
            if Instant::now() > end {
                return Err(format!("timed out waiting for {what}"));
            }
            std::thread::sleep(POLL);
        }
    }

    fn controls_cfg() -> PathBuf {
        layout().data.join("controls.cfg")
    }

    /// The radio wizard's controls.cfg: the example's mapping, a TX15 in joystick mode.
    fn write_controls() -> Result<(), String> {
        std::fs::copy(layout().home.join("controls.cfg.example"), Self::controls_cfg())
            .map(|_| ())
            .map_err(|e| format!("couldn't write controls.cfg: {e}"))
    }

    fn choose(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        let want = text(value);
        let id = match var.subject.to_lowercase().as_str() {
            "the launch target" => match want.as_str() {
                "SIMULATOR ONLY" => "t-sim",
                "TOWERS" => "t-towers",
                _ => "t-station",
            }
            .to_string(),
            "the red team" => if want == "COMPUTER" { "red-computer" } else { "red-nobody" }.to_string(),
            "the drone cameras choice" => {
                if self.seen()?.cameras == (want == "on") {
                    return Ok(());
                }
                "cams".to_string()
            }
            "the flight controller choice" => if want == "WITH ARDUPILOT" { "fc-on" } else { "fc-off" }.to_string(),
            "the place choice" if want == "ALPINE VALLEY" => "map-none".to_string(),
            "the place choice" => format!("map-{}", want.to_lowercase()),
            "the ray tracing choice" => {
                if self.seen()?.raytrace == (want == "on") {
                    return Ok(());
                }
                "rt".to_string()
            }
            s => return Err(format!("the launcher can't set {s}")),
        };
        self.click_button(id)?;
        let subject = var.subject.clone();
        self.until(&format!("{subject} to be {want}"), |s| s.read(&subject).is_ok_and(|v| text(&v) == want))
    }
}

impl Driver for LauncherDriver {
    fn reset(&mut self) -> Result<(), String> {
        let _ = std::fs::remove_file(Self::controls_cfg());
        self.call(|l, _, cx| {
            l.reset();
            cx.notify();
        })?;
        std::thread::sleep(SETTLE);
        Ok(())
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        let subject = var.subject.to_lowercase();
        let want = text(value);
        match subject.as_str() {
            s if SETTABLE.contains(&s) => self.choose(var, value),
            "the controls file" => {
                if want == "mapped" {
                    Self::write_controls()?;
                } else {
                    let _ = std::fs::remove_file(Self::controls_cfg());
                }
                self.call(|l, _, cx| {
                    l.mapping = crate::controllers::mapping();
                    cx.notify();
                })
            }
            // What runs: a fresh launcher runs nothing; a program is started as a person would.
            "the program" | "the program in the log" | "the launch button" => {
                let now = self.seen()?;
                if now.read(&subject).is_ok_and(|v| text(&v) == want) {
                    return Ok(());
                }
                let button = match (subject.as_str(), want.as_str()) {
                    ("the program", "running") | ("the launch button", "STOP") => "launch",
                    ("the program in the log", "the radio wizard") => "calibrate",
                    ("the program in the log", "the manual build") => "render",
                    _ => return Err(format!("the launcher can't arrange that {subject} is {want}")),
                };
                if now.busy {
                    return Err(format!("a program already runs, so {button} can't start another"));
                }
                self.click_button(button.into())?;
                self.until(&format!("{subject} to be {want}"), |s| s.read(&subject).is_ok_and(|v| text(&v) == want))
            }
            s => Err(format!("the launcher can't arrange {s}")),
        }
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        self.choose(var, value)
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        match event.text.to_lowercase().as_str() {
            "the pilot launches" => self.click_button("launch".into()),
            "the pilot stops the program" => self.click_button("stop".into()),
            "the pilot sets up the controller" => self.click_button("calibrate".into()),
            "the pilot renders the docs" => self.click_button("render".into()),
            "the radio wizard saves controls.cfg and closes" => {
                Self::write_controls()?;
                // The sim closes: the stand-in ends as the sim does when its window is closed.
                let pid = self.call(|l, _, _| l.job.as_ref().filter(|j| j.running()).map(|j| j.pid()))?;
                let pid = pid.ok_or("no program runs: the wizard isn't open")?;
                #[cfg(unix)]
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                self.until("the wizard to close", |s| s.job.as_ref().is_some_and(|(_, _, running)| !running))
            }
            e => Err(format!("the launcher can't make {e} happen")),
        }
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        self.seen()?.read(&var.subject)
    }

    fn settle(&mut self) {
        std::thread::sleep(SETTLE);
    }

    fn advance(&mut self) -> Progress {
        std::thread::sleep(POLL);
        Progress::Advanced
    }

    /// The launcher's window as the system draws it.
    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        Ok(self.with_window(crate::snapshot::window_jpeg)?.map(Screenshot::jpeg))
    }

    fn supports(&self, need: Need<'_>) -> bool {
        let subject = |v: &Variable| v.subject.to_lowercase();
        match need {
            Need::Observe(v) => STATES.contains(&subject(v).as_str()),
            Need::Arrange(v) => ARRANGED.contains(&subject(v).as_str()),
            Need::Set(v) => SETTABLE.contains(&subject(v).as_str()),
            Need::Perform(e) => EVENTS.iter().any(|s| is(&e.text, s)),
        }
    }
}

/// Loads the suite, then runs the section's tests on a thread of their own and quits.
pub fn start(cx: &mut App, window: WindowHandle<Launcher>, config: Config) {
    let Config { suite, only, docs } = config;
    let mut acceptance = match Acceptance::load_for(&suite, "desktop") {
        Ok(a) => a.with_coverage(),
        Err(e) => {
            eprint!("{e}");
            std::process::exit(1);
        }
    };
    let section = acceptance.suite.sections.iter().find(|s| s.slug == "launcher").map(|s| s.id);
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
                let _ = cx.update(|cx| job(cx, window));
            }
        }
    })
    .detach();
    std::thread::spawn(move || {
        let mut driver = LauncherDriver { tx };
        // The window paints once before the first click.
        std::thread::sleep(Duration::from_secs(1));
        println!("expect: {} tests of the launcher", acceptance.plans.len());
        let opts = RunOptions { docs_dir: docs.clone(), patience: 20, ..Default::default() };
        let mut report = expectation_story::Report::default();
        let suite_time = suite_budget::Budget::from_env();
        for plan in &acceptance.plans {
            let began = Instant::now();
            let r = if suite_time.spent() {
                suite_time.ran_out(&acceptance.suite, plan, &driver)
            } else {
                expectation_harness::run(&acceptance.suite, std::slice::from_ref(plan), &mut driver, &opts)
                    .results
                    .remove(0)
            };
            match r.status {
                TestStatus::Passed => println!("expect: ✓ {} ({:.0} s)", plan.title, began.elapsed().as_secs_f32()),
                _ => println!("expect: ✗ {} {:?} ({:.0} s)", plan.title, r.status, began.elapsed().as_secs_f32()),
            }
            report.results.push(r);
        }
        if let Some(dir) = &docs {
            // Saved as the launcher's results, and merged with the other runners' when the manual is written.
            match acceptance.publish(dir, "launcher", &report) {
                Ok(_) => println!("expect: wrote the results to {}", dir.display()),
                Err(e) => eprintln!("expect: couldn't write the results to {}: {e}", dir.display()),
            }
        }
        if !report.passed() {
            eprint!("{}", report.failures(&acceptance.suite, &acceptance.plans));
            EXIT.store(1, Ordering::SeqCst);
        }
        let _ = driver.call(|l, _, cx| {
            l.shutdown();
            cx.quit()
        });
    });
}
