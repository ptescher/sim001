//! The launcher window: four panels (controllers, docs, launch, log) in the console's bezels.

use crate::controllers::{self, Device, Mapping, SerialRadio};
use crate::job::{Job, layout};
use crate::theme::*;
use gpui::{
    AnyElement, Bounds, Context, Div, IntoElement, Pixels, Render, SharedString, Stateful, Window, canvas, div,
    prelude::*, px,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use teleoperation_core::debug::{self, Level, LogFile};

thread_local! {
    /// Where each button was last painted, by id, so `--expect` can click it the way a person
    /// would. Written while painting, read between frames; both on the UI thread.
    static BUTTONS: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::new(HashMap::new());
    /// How tall each row of buttons wants to be at its current width, by id (see [`Launcher::fit`]).
    static ROWS: RefCell<HashMap<&'static str, f32>> = RefCell::new(HashMap::new());
}

/// The height of one line of buttons: a row that wants more than this is cut to it, behind MORE.
const ROW_H: f32 = 34.0;

/// Where the button with this id is on screen now, if it is.
pub fn button_bounds(id: &str) -> Option<Bounds<Pixels>> {
    BUTTONS.with(|b| b.borrow().get(id).copied())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// The simulator alone.
    Sim,
    /// The sim, ArduPilot and the ground control console together.
    Station,
    /// A game of Towers: the commander app, which starts the game server and its drones.
    Towers,
}

/// What the LAUNCH screen is set to, and the command lines that follow from it.
#[derive(Clone)]
pub struct Choices {
    pub target: Target,
    /// With the ArduPilot flight controller (the T1-S VTOL) or without (the plain Ranger / sticks direct).
    pub flight_controller: bool,
    /// The plane flown with ArduPilot (one of [`PLANES`]).
    pub plane: &'static str,
    pub raytrace: bool,
    pub fullscreen: bool,
    /// Skip rebuilding before the start.
    pub no_build: bool,
    /// `None` is the Alpine Valley.
    pub map: Option<String>,
    /// Station only: the TX15 on its serial line carries the sticks, telemetry and MAVLink.
    pub use_serial: bool,
    pub serial: Vec<SerialRadio>,
    /// Towers: the computer commands Red (else nobody does: a practice game).
    pub red_computer: bool,
    /// Towers: the 3D view of the game, and the pictures from the drones' cameras.
    pub cameras: bool,
}

/// The planes with a flight controller that the sim can fly, by `--aircraft` id and name.
pub const PLANES: [(&str, &str, &str); 3] = [
    ("t1s-vtol", "T1-S VTOL", "plane-t1s-vtol"),
    ("t2-cruza", "T2 CRUZA (HEAVY)", "plane-t2-cruza"),
    ("seeker3", "SEEKER 3 (LIGHT QUAD)", "plane-seeker3"),
];

impl Default for Choices {
    /// What the launcher opens on: the station with ArduPilot, in the Alpine Valley.
    fn default() -> Choices {
        Choices {
            target: Target::Station,
            flight_controller: true,
            plane: PLANES[0].0,
            raytrace: false,
            fullscreen: false,
            no_build: false,
            map: None,
            use_serial: true,
            serial: vec![],
            red_computer: true,
            cameras: true,
        }
    }
}

impl Choices {
    /// The sim's own arguments.
    pub fn sim_args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec![];
        a.push("--aircraft".into());
        a.push(if self.flight_controller { self.plane } else { "t1s-ranger" }.into());
        if let Some(m) = &self.map {
            a.extend(["--map".into(), m.clone()]);
        }
        if self.raytrace {
            a.push("--raytrace".into());
        }
        if self.fullscreen {
            a.push("--fullscreen".into());
        }
        // A radio on its serial line is not a joystick: the sim's flight computer reads its sticks
        // from the line (CRSF) and sends them to ArduPilot.
        if let Some(r) = self.serial_radio() {
            a.extend(["--flight-computer".into(), r.path.clone()]);
        }
        a
    }

    /// The console's `--station`; without the flight controller the sticks fly the plane
    /// through the sim's own modes and ArduPilot only watches (`--direct`). A bundle has nothing
    /// to build.
    pub fn station_args(&self, bundled: bool) -> Vec<String> {
        let mut a: Vec<String> = vec!["--station".into()];
        if let Some(m) = &self.map {
            a.extend(["--map".into(), m.clone()]);
        }
        if self.raytrace {
            a.push("--raytrace".into());
        }
        if !self.flight_controller {
            a.push("--direct".into());
        } else if self.plane != PLANES[0].0 {
            a.extend(["--aircraft".into(), self.plane.into()]);
        }
        if let Some(r) = self.serial_radio() {
            a.extend(["--tx15-serial".into(), r.path.clone()]);
        }
        if self.no_build || bundled {
            a.push("--no-build".into());
        }
        a
    }

    /// The commander app's: a game of your own, with Blue yours.
    pub fn towers_args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec!["--serve".into()];
        if !self.red_computer {
            a.extend(["--red".into(), "nobody".into()]);
        }
        if !self.cameras {
            a.push("--no-cameras".into());
        }
        a
    }

    /// The TX15 the station will use on its serial line, if one is plugged in and chosen.
    pub fn serial_radio(&self) -> Option<&SerialRadio> {
        if !self.use_serial || !self.flight_controller {
            return None;
        }
        self.serial.iter().find(|r| r.radio)
    }

    /// The title of what LAUNCH starts, for the log.
    pub fn title(&self) -> &'static str {
        match (self.target, self.flight_controller) {
            (Target::Towers, _) => "Towers, you command Blue",
            (Target::Sim, true) => match self.plane {
                "t2-cruza" => "Simulator, T2 Cruza with ArduPilot",
                "seeker3" => "Simulator, Seeker 3 with ArduPilot",
                _ => "Simulator, T1-S VTOL with ArduPilot",
            },
            (Target::Sim, false) => "Simulator, T1-S Ranger",
            (Target::Station, true) => "Ground station with ArduPilot",
            (Target::Station, false) => "Ground station, sticks direct",
        }
    }
}

pub struct Launcher {
    pub choices: Choices,
    maps: Vec<String>,
    devices: Vec<Device>,
    pub mapping: Mapping,
    pub job: Option<Job>,
    notice: String,
    /// The log pane shows the program's output (`None`) or one of the log files of a run: the
    /// console's debug log, what each ArduPlane printed.
    log_tab: Option<String>,
    /// The log files there are, and the last lines of the one that is shown, read as they grow.
    logs: Vec<LogFile>,
    file_lines: Vec<String>,
    /// The rows of buttons (by id) that MORE has opened out to every line.
    expanded: HashSet<&'static str>,
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Normal,
    Active,
    Caution,
    Danger,
}

fn button(id: impl Into<SharedString>, text: impl Into<SharedString>, style: Style) -> Stateful<Div> {
    let id = id.into();
    let recorded = id.clone();
    let (bg, fg, border) = match style {
        Style::Normal => (CONTROL, WHITE, BEZEL_EDGE),
        Style::Active => (ACTIVE_FILL, GREEN, GREEN_DIM),
        Style::Caution => (CAUTION_FILL, AMBER, AMBER),
        Style::Danger => (DANGER_FILL, ON_DANGER, RED),
    };
    div()
        .id(id)
        .relative()
        .px_3()
        .py(px(5.0))
        .bg(c(bg))
        .text_color(c(fg))
        .border_1()
        .border_color(c(border))
        .rounded(px(CONTROL_RADIUS))
        .cursor_pointer()
        .hover(|s| s.bg(c(CONTROL_HOVER)))
        .active(|s| s.opacity(0.7))
        .child(text.into())
        .child(
            canvas(move |b, _, _| BUTTONS.with(|m| m.borrow_mut().insert(recorded, b)), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
}

fn screen(title: &'static str, body: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .gap(px(4.0))
        .bg(c(BEZEL))
        .border_1()
        .border_color(c(BEZEL_EDGE))
        .rounded(px(PANEL_RADIUS))
        .p(px(BEZEL_PAD))
        .child(div().px_1().text_size(px(11.0)).text_color(c(GREY)).child(title))
        .child(
            div()
                .id(title)
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                // What does not fit scrolls, text above all.
                .overflow_scroll()
                .gap_2()
                .p_3()
                .bg(c(SCREEN))
                .border_2()
                .border_color(c(0x000000))
                .rounded(px(SCREEN_RADIUS))
                .child(div().flex().flex_col().flex_none().child(body)),
        )
}

fn note(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.0)).text_color(c(GREY)).child(text.into())
}

fn row() -> Div {
    div().flex().flex_wrap().items_center().gap_2()
}

impl Launcher {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Poll the running program and repaint its log.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                if this.update(cx, |l, cx| l.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut maps: Vec<String> = std::fs::read_dir(layout().home.join("maps"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        maps.sort();
        Launcher {
            choices: Choices { serial: controllers::serial_radios(), ..Choices::default() },
            maps,
            devices: controllers::connected(),
            mapping: controllers::mapping(),
            job: None,
            notice: String::new(),
            log_tab: None,
            logs: vec![],
            file_lines: vec![],
            expanded: HashSet::new(),
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        // The log files of a run, and the end of the one that is shown.
        let logs = debug::list_logs(&layout().data.join("logs"));
        if logs != self.logs {
            self.logs = logs;
        }
        if let Some(name) = &self.log_tab {
            match self.logs.iter().find(|l| &l.name == name) {
                Some(l) => self.file_lines = debug::tail_lines(&l.path, 60),
                None => self.file_lines.clear(),
            }
            cx.notify();
        }
        // Repaint for the rows that read last frame's measure (see `fit`).
        cx.notify();
        if let Some(j) = &mut self.job {
            let was = j.running();
            j.poll();
            if was && !j.running() {
                // The wizard writes controls.cfg; a build may have changed the docs.
                self.mapping = controllers::mapping();
            }
            cx.notify();
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(j) = &mut self.job {
            j.stop();
            std::thread::sleep(Duration::from_millis(300));
            j.kill();
        }
    }

    /// Back to what the launcher opens on, with nothing running (for `--expect`, between tests).
    pub fn reset(&mut self) {
        self.shutdown();
        self.job = None;
        self.choices = Choices { serial: std::mem::take(&mut self.choices.serial), ..Choices::default() };
        self.mapping = controllers::mapping();
        self.notice.clear();
        self.log_tab = None;
    }

    pub fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(Job::running)
    }

    fn run(&mut self, title: &str, program: &str, args: Vec<String>, dir: std::path::PathBuf) {
        if self.busy() {
            return;
        }
        match Job::start(title, program, &args, &dir) {
            Ok(j) => {
                self.notice.clear();
                self.job = Some(j);
            }
            Err(e) => self.notice = e,
        }
    }

    /// Starts a program of the project: from the bundle, or built and run by cargo in a checkout.
    fn program(&self, name: &str, crate_dir: &str, flags: Vec<String>) -> (String, Vec<String>, std::path::PathBuf) {
        let l = layout();
        match l.program(name) {
            Some(p) => (p, flags, l.data.clone()),
            None => {
                let mut args: Vec<String> = vec!["run".into(), "--release".into()];
                // The root package's default binary is this launcher.
                if name == "sim001" {
                    args.extend(["--bin".into(), "sim001".into()]);
                }
                args.push("--".into());
                args.extend(flags);
                ("cargo".into(), args, l.home.join(crate_dir))
            }
        }
    }

    fn launch(&mut self) {
        let title = self.choices.title();
        let (program, args, dir) = match self.choices.target {
            Target::Sim => self.program("sim001", "", self.choices.sim_args()),
            Target::Station => self.program(
                "sim001_gcs",
                "domains/teleoperation/console",
                self.choices.station_args(layout().bundled()),
            ),
            Target::Towers => {
                self.program("sim001_command", "domains/teleoperation/command", self.choices.towers_args())
            }
        };
        self.run(title, &program, args, dir);
    }

    fn calibrate(&mut self) {
        let flags = ["--aircraft", "t1s-ranger", "--calibrate"].map(String::from).to_vec();
        let (program, args, dir) = self.program("sim001", "", flags);
        self.run("Controller setup", &program, args, dir);
    }

    /// Writes the manual from the saved test results (`expectation docs`).
    fn render_docs(&mut self) {
        let args = ["run", "-q", "-p", "expectation", "--", "docs", "expectations", "docs"].map(String::from).to_vec();
        self.run("Render the docs", "cargo", args, layout().home.clone());
    }

    fn open(&mut self, path: std::path::PathBuf) {
        if !path.exists() {
            self.notice = format!("{} does not exist yet. Render the docs first.", path.display());
            return;
        }
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(opener).arg(&path).spawn() {
            self.notice = format!("{opener}: {e}");
        }
    }

    fn choice<F>(&self, id: &'static str, text: &'static str, on: bool, cx: &mut Context<Self>, f: F) -> Stateful<Div>
    where
        F: Fn(&mut Self) + 'static,
    {
        button(id, text, if on { Style::Active } else { Style::Normal }).on_click(cx.listener(move |this, _, _, cx| {
            f(this);
            cx.notify();
        }))
    }

    /// A row of buttons that shows one line and a MORE button when the rest does not fit; MORE
    /// opens it out (and LESS closes it). Whether it fits is read from the last frame.
    fn fit(&self, id: &'static str, content: Div, cx: &mut Context<Self>) -> Div {
        let open = self.expanded.contains(id);
        let natural = ROWS.with(|r| r.borrow().get(id).copied()).unwrap_or(0.0);
        let measure =
            canvas(move |b, _, _| ROWS.with(|r| r.borrow_mut().insert(id, f32::from(b.size.height))), |_, _, _, _| {})
                .absolute()
                .inset_0();
        let mut clip = div().flex_1().min_w(px(0.0)).overflow_hidden();
        if !open {
            clip = clip.max_h(px(ROW_H));
        }
        div().flex().items_start().gap_2().child(clip.child(content.relative().child(measure))).children(
            (natural > ROW_H + 2.0).then(|| {
                button(format!("more-{id}"), if open { "LESS ▴" } else { "MORE ▾" }, Style::Normal).on_click(
                    cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(id) {
                            this.expanded.insert(id);
                        }
                        cx.notify();
                    }),
                )
            }),
        )
    }

    fn controllers_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_1();
        if self.devices.is_empty() && self.choices.serial.is_empty() {
            body = body.child(
                div()
                    .text_color(c(AMBER))
                    .child("No radio or gamepad found. Plug in the TX15 (USB joystick mode) or a controller."),
            );
        }
        for d in &self.devices {
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().text_color(c(GREEN)).child("●"))
                    .child(d.name.clone())
                    .child(div().text_color(c(DARK)).child(format!("{:04x}:{:04x}", d.vendor, d.product))),
            );
        }
        for r in &self.choices.serial {
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x_2()
                    .child(div().text_color(c(if r.radio { GREEN } else { GREY })).child("●"))
                    .child(r.name.clone())
                    .child(div().text_color(c(DARK)).child(format!(
                        "{}{}",
                        r.path,
                        if r.radio { " (serial: telemetry and MAVLink)" } else { "" }
                    ))),
            );
        }
        let m = &self.mapping;
        let mapped = if !m.exists {
            div().text_color(c(AMBER)).child("No controls.cfg yet: the sim guesses the sticks until you set them up.")
        } else {
            div().text_color(c(GREEN)).child(format!(
                "controls.cfg maps {} channels{}",
                m.channels.len(),
                if m.radios.is_empty() { String::new() } else { format!(" for {}", m.radios.join(", ")) }
            ))
        };
        screen(
            "CONTROLLERS",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(body)
                .child(mapped)
                .child(note("Setup opens the sim on the plain T1-S and starts the radio wizard: move each stick and switch when asked. F3 there shows the raw axes."))
                .child(
                    self.fit("r-ctrl", row()
                        .child(button("calibrate", "SET UP CONTROLLER", Style::Caution).on_click(cx.listener(|this, _, _, cx| {
                            if this.devices.is_empty() {
                                this.devices = controllers::connected();
                            }
                            this.calibrate();
                            cx.notify();
                        })))
                        .child(button("rescan", "RESCAN", Style::Normal).on_click(cx.listener(|this, _, _, cx| {
                            this.devices = controllers::connected();
                            this.choices.serial = controllers::serial_radios();
                            this.mapping = controllers::mapping();
                            cx.notify();
                        })))
                        .child(button("cfg", "OPEN CONTROLS.CFG", Style::Normal).on_click(cx.listener(|this, _, _, cx| {
                            this.open(layout().data.join("controls.cfg"));
                            cx.notify();
                        }))), cx),
                ),
        )
    }

    fn docs_panel(&self, cx: &mut Context<Self>) -> Div {
        screen(
            "DOCS",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(note(if layout().bundled() {
                    "The manual in this app was rendered from the specs and test results when the app was built."
                } else {
                    "Renders the manual (docs/ and the HTML site) from the specs in expectations/ and the last saved test results. `tasks docs` also re-runs every test first."
                }))
                .child(
                    self.fit("r-docs", row()
                        .children((!layout().bundled()).then(|| {
                            button("render", "RENDER DOCS", Style::Normal).on_click(cx.listener(|this, _, _, cx| {
                                this.render_docs();
                                cx.notify();
                            }))
                        }))
                        .child(button("open-docs", "OPEN MANUAL", Style::Normal).on_click(cx.listener(|this, _, _, cx| {
                            this.open(layout().home.join("docs/site/index.html"));
                            cx.notify();
                        })))
                        .children((!layout().bundled()).then(|| {
                            button("rerun", "RE-RUN TESTS + RENDER", Style::Caution).on_click(cx.listener(|this, _, _, cx| {
                                this.run(
                                    "Tests and docs",
                                    "cargo",
                                    ["run", "--release", "-p", "sim001_tasks", "--", "docs", "--no-sitl"].map(String::from).to_vec(),
                                    layout().home.clone(),
                                );
                                cx.notify();
                            }))
                        })), cx),
                ),
        )
    }

    fn launch_panel(&self, cx: &mut Context<Self>) -> Div {
        let busy = self.busy();
        let fc = self.choices.flight_controller;
        let target = self.choices.target;
        let explain = match (target, fc) {
            (Target::Towers, _) => {
                "Two teams of three drones fight for five radio towers in a valley with a forest. You command Blue from the map: click a drone, then a tower, and LOW sends it along the lanes under the trees. Starts a game server with six ArduPlane drones, and needs the AGENT build of ArduPlane."
            }
            (Target::Sim, true) => {
                "The T1-S VTOL. The sim starts its own ArduPilot (native build or Docker); fly it with your radio."
            }
            (Target::Sim, false) => "The plain T1-S Ranger: no flight controller, MANUAL only. Throw it and fly.",
            (Target::Station, true) => {
                "The sim, ArduPilot SITL and the ground control console, tiled over your displays. Uses the native ArduPlane when there is one (ardupilot/build_native.sh, or the bundle), else Docker."
            }
            (Target::Station, false) => {
                "The station with the sticks flying the plane through the sim's own modes. ArduPilot still runs for telemetry, but its outputs are ignored."
            }
        };
        let mut maps = row()
            .child(self.choice("map-none", "ALPINE VALLEY", self.choices.map.is_none(), cx, |t| t.choices.map = None));
        for m in &self.maps {
            let name = m.clone();
            let id: SharedString = format!("map-{m}").into();
            maps = maps.child(
                button(
                    id,
                    m.to_uppercase(),
                    if self.choices.map.as_ref() == Some(m) { Style::Active } else { Style::Normal },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choices.map = Some(name.clone());
                    cx.notify();
                })),
            );
        }
        let maps = self.fit("r-maps", maps, cx);
        let go = if busy {
            button("stop", "STOP", Style::Danger).on_click(cx.listener(|this, _, _, cx| {
                if let Some(j) = &mut this.job {
                    j.stop();
                }
                cx.notify();
            }))
        } else {
            button("launch", "LAUNCH", Style::Active).on_click(cx.listener(|this, _, _, cx| {
                this.launch();
                cx.notify();
            }))
        };
        screen(
            "LAUNCH",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    self.fit("r-target", row()
                        .child(
                            self.choice("t-sim", "SIMULATOR ONLY", target == Target::Sim, cx, |t| {
                                t.choices.target = Target::Sim
                            }),
                        )
                        .child(self.choice("t-station", "GROUND STATION", target == Target::Station, cx, |t| {
                            t.choices.target = Target::Station
                        }))
                        // The bundle has no commander app (nor the game server) yet.
                        .children((!layout().bundled()).then(|| {
                            self.choice("t-towers", "TOWERS", target == Target::Towers, cx, |t| {
                                t.choices.target = Target::Towers
                            })
                        })), cx)
                )
                .when(target != Target::Towers, |d| {
                    d.child(self.fit("r-fc", row()
                        .child(self.choice("fc-on", "WITH ARDUPILOT", fc, cx, |t| t.choices.flight_controller = true))
                        .child(self.choice("fc-off", "WITHOUT ARDUPILOT", !fc, cx, |t| t.choices.flight_controller = false)), cx))
                })
                .when(target != Target::Towers && fc, |d| {
                    let planes = PLANES.iter().fold(row(), |l, &(id, name, button)| {
                        l.child(self.choice(button, name, self.choices.plane == id, cx, move |t| {
                            t.choices.plane = id
                        }))
                    });
                    d.child(self.fit("r-plane", planes, cx))
                })
                .child(note(explain))
                .when(target == Target::Towers, |d| {
                    d.child(self.fit("r-towers", row()
                        .child(self.choice("red-computer", "RED: COMPUTER", self.choices.red_computer, cx, |t| {
                            t.choices.red_computer = true
                        }))
                        .child(self.choice("red-nobody", "RED: NOBODY", !self.choices.red_computer, cx, |t| {
                            t.choices.red_computer = false
                        }))
                        .child(self.choice("cams", "DRONE CAMERAS", self.choices.cameras, cx, |t| {
                            t.choices.cameras = !t.choices.cameras
                        })), cx))
                    .child(note("RED: NOBODY is a practice game: Red's drones circle over their base. DRONE CAMERAS starts the 3D view of the game, which sends the camera of each of your drones (the first start takes a while to build it)."))
                })
                .children(
                    self.choices.serial_radio().filter(|_| target != Target::Towers).map(|r| {
                        note(format!("The {} on {} carries the sticks, telemetry and MAVLink.", r.name, r.path))
                    }),
                )
                .when(target != Target::Towers, |d| {
                    d.child(note("Place")).child(maps).child(self.fit("r-opts", row()
                        .child(self.choice("rt", "RAY TRACING", self.choices.raytrace, cx, |t| t.choices.raytrace = !t.choices.raytrace))
                        .children((target == Target::Sim).then(|| {
                            self.choice("fs", "FULL SCREEN", self.choices.fullscreen, cx, |t| t.choices.fullscreen = !t.choices.fullscreen)
                        }))
                        .children((fc && self.choices.serial.iter().any(|r| r.radio)).then(|| {
                            self.choice("ser", "TX15 ON SERIAL", self.choices.use_serial, cx, |t| {
                                t.choices.use_serial = !t.choices.use_serial
                            })
                        }))
                        .children((target == Target::Station).then(|| {
                            self.choice("nb", "SKIP SIM REBUILD", self.choices.no_build, cx, |t| t.choices.no_build = !t.choices.no_build)
                        })), cx))
                })
                .child(row().child(go)),
        )
    }

    /// The log pane: the program's output, or a log file of the run (the console's debug log,
    /// ArduPlane's own output), chosen by the tabs along its top.
    fn log_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = row().child(
            button("log-output", "OUTPUT", if self.log_tab.is_none() { Style::Active } else { Style::Normal })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.log_tab = None;
                    cx.notify();
                })),
        );
        for l in &self.logs {
            let name = l.name.clone();
            let id: SharedString = format!("log-{}", l.name).into();
            tabs = tabs.child(
                button(
                    id,
                    l.name.to_uppercase(),
                    if self.log_tab.as_deref() == Some(l.name.as_str()) { Style::Active } else { Style::Normal },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.log_tab = Some(name.clone());
                    this.file_lines.clear();
                    cx.notify();
                })),
            );
        }
        let tabs = self.fit("r-tabs", tabs, cx);
        let Some(tab) = &self.log_tab else { return self.output_panel(tabs) };
        let colour = |line: &str| match debug::parse_line(line).map(|e| e.level) {
            Some(Level::Error) => RED,
            Some(Level::Warn) => AMBER,
            Some(Level::Info) => WHITE,
            _ => GREY,
        };
        let mut out = div().flex().flex_col().text_size(px(11.0));
        for l in &self.file_lines {
            out = out.child(div().text_color(c(colour(l))).child(l.clone()));
        }
        let path = self.logs.iter().find(|l| &l.name == tab).map(|l| l.path.display().to_string()).unwrap_or_default();
        screen("DEBUG LOG", div().flex().flex_col().gap_1().child(tabs).child(note(path)).child(out))
    }

    fn output_panel(&self, tabs: Div) -> Div {
        let (title, status, color, lines) = match &self.job {
            None => ("Nothing running".to_string(), String::new(), GREY, vec![]),
            Some(j) => {
                let (s, col) = match j.exit {
                    None if j.stopping => ("stopping".to_string(), AMBER),
                    None => ("running".to_string(), GREEN),
                    Some(st) if st.success() => ("finished".to_string(), GREEN),
                    Some(st) => (format!("ended ({st})"), RED),
                };
                let log = j.log.lock().unwrap();
                let from = log.len().saturating_sub(40);
                (j.title.clone(), s, col, log[from..].to_vec())
            }
        };
        let mut out = div().flex().flex_col().text_size(px(11.0)).text_color(c(GREY));
        for l in lines {
            out = out.child(div().child(l));
        }
        screen(
            "LOG",
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(tabs)
                .child(
                    row()
                        .child(div().text_color(c(WHITE)).child(title))
                        .child(div().text_color(c(color)).child(status)),
                )
                .children((!self.notice.is_empty()).then(|| div().text_color(c(AMBER)).child(self.notice.clone())))
                .child(out),
        )
    }
}

impl Render for Launcher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels: [AnyElement; 4] = [
            self.launch_panel(cx).into_any_element(),
            self.controllers_panel(cx).into_any_element(),
            self.docs_panel(cx).into_any_element(),
            self.log_panel(cx).into_any_element(),
        ];
        let [launch, ctrl, docs, log] = panels;
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .p(px(GAP))
            .pt(px(34.0))
            .bg(c(CONSOLE))
            .text_color(c(WHITE))
            .text_size(px(13.0))
            .font_family(FONT)
            .child(div().px_1().text_size(px(18.0)).text_color(c(GREEN)).child("SIM001"))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap(px(GAP))
                    .min_h(px(0.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(GAP))
                            .w(px(520.0))
                            .child(div().flex().flex_col().flex_none().child(launch))
                            .child(div().flex().flex_col().flex_1().min_h(px(0.0)).child(ctrl)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(GAP))
                            .flex_1()
                            .min_w(px(0.0))
                            .child(div().flex().flex_col().flex_none().child(docs))
                            .child(div().flex().flex_col().flex_1().min_h(px(0.0)).child(log)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx15() -> SerialRadio {
        SerialRadio { name: "TX15".into(), path: "/dev/cu.usbmodem1".into(), radio: true }
    }

    #[test]
    fn the_launcher_opens_on_the_station_with_ardupilot_in_the_alpine_valley() {
        let c = Choices::default();
        assert_eq!((c.target, c.flight_controller, c.map.clone()), (Target::Station, true, None));
        assert_eq!(c.station_args(false), ["--station"]);
        assert_eq!(c.title(), "Ground station with ArduPilot");
    }

    #[test]
    fn the_sim_alone_flies_the_vtol_with_ardupilot_and_the_ranger_without() {
        let mut c = Choices { target: Target::Sim, ..Choices::default() };
        assert_eq!(c.sim_args(), ["--aircraft", "t1s-vtol"]);
        c.flight_controller = false;
        assert_eq!(c.sim_args(), ["--aircraft", "t1s-ranger"]);
        assert_eq!(c.title(), "Simulator, T1-S Ranger");
    }

    #[test]
    fn the_heavy_and_the_light_plane_are_flown_with_ardupilot_in_the_sim_and_the_station() {
        let mut c = Choices { target: Target::Sim, plane: "t2-cruza", ..Choices::default() };
        assert_eq!(c.sim_args(), ["--aircraft", "t2-cruza"]);
        assert_eq!(c.title(), "Simulator, T2 Cruza with ArduPilot");
        assert_eq!(c.station_args(false), ["--station", "--aircraft", "t2-cruza"]);
        c.plane = "seeker3";
        assert_eq!(c.sim_args(), ["--aircraft", "seeker3"]);
        c.flight_controller = false;
        assert_eq!(c.sim_args(), ["--aircraft", "t1s-ranger"], "without ArduPilot it is the Ranger");
    }

    #[test]
    fn the_place_ray_tracing_and_full_screen_go_to_the_sim() {
        let c = Choices { map: Some("cypress".into()), raytrace: true, fullscreen: true, ..Choices::default() };
        assert_eq!(c.sim_args(), ["--aircraft", "t1s-vtol", "--map", "cypress", "--raytrace", "--fullscreen"]);
        assert_eq!(
            c.station_args(false),
            ["--station", "--map", "cypress", "--raytrace"],
            "the station has no full screen"
        );
    }

    #[test]
    fn without_ardupilot_the_station_flies_the_sticks_direct() {
        let c = Choices { flight_controller: false, ..Choices::default() };
        assert_eq!(c.station_args(false), ["--station", "--direct"]);
        assert_eq!(c.title(), "Ground station, sticks direct");
    }

    #[test]
    fn a_bundle_or_skip_rebuild_does_not_build_the_sim() {
        assert_eq!(Choices::default().station_args(true), ["--station", "--no-build"]);
        assert_eq!(Choices { no_build: true, ..Choices::default() }.station_args(false), ["--station", "--no-build"]);
    }

    #[test]
    fn a_tx15_on_serial_carries_the_line_only_with_ardupilot() {
        let mut c = Choices { serial: vec![tx15()], ..Choices::default() };
        assert_eq!(c.station_args(false), ["--station", "--tx15-serial", "/dev/cu.usbmodem1"]);
        c.target = Target::Sim;
        assert_eq!(c.sim_args(), ["--aircraft", "t1s-vtol", "--flight-computer", "/dev/cu.usbmodem1"]);
        c.use_serial = false;
        assert_eq!(c.sim_args(), ["--aircraft", "t1s-vtol"]);
        c.use_serial = true;
        c.flight_controller = false;
        assert_eq!(c.sim_args(), ["--aircraft", "t1s-ranger"], "the Ranger has no flight computer");
    }

    #[test]
    fn a_serial_device_that_is_not_a_radio_is_not_used() {
        let other = SerialRadio { name: "Serial device".into(), path: "/dev/cu.usbserial9".into(), radio: false };
        let c = Choices { serial: vec![other], ..Choices::default() };
        assert!(c.serial_radio().is_none());
        assert_eq!(c.station_args(false), ["--station"]);
    }
}
