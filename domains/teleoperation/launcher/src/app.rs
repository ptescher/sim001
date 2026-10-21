//! The launcher window: four panels (controllers, docs, launch, log) in the console's bezels.

use crate::controllers::{self, Device, Mapping, SerialRadio};
use crate::job::{Job, layout};
use crate::theme::*;
use teleoperation_core::debug::{self, Level, LogFile};
use gpui::{AnyElement, Context, Div, IntoElement, Render, SharedString, Stateful, Window, div, prelude::*, px};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Target {
    /// The simulator alone.
    Sim,
    /// The sim, ArduPilot and the ground control console together.
    Station,
}

pub struct Launcher {
    target: Target,
    /// With the ArduPilot flight controller (the T1-S VTOL) or without (the plain Ranger / sticks direct).
    flight_controller: bool,
    raytrace: bool,
    fullscreen: bool,
    /// Skip rebuilding before the start.
    no_build: bool,
    /// `None` is the Alpine Valley.
    map: Option<String>,
    maps: Vec<String>,
    devices: Vec<Device>,
    serial: Vec<SerialRadio>,
    /// Station only: the TX15 on its serial line carries the sticks, telemetry and MAVLink.
    use_serial: bool,
    mapping: Mapping,
    job: Option<Job>,
    notice: String,
    /// The log pane shows the program's output (`None`) or one of the log files of a run: the
    /// console's debug log, what each ArduPlane printed.
    log_tab: Option<String>,
    /// The log files there are, and the last lines of the one that is shown, read as they grow.
    logs: Vec<LogFile>,
    file_lines: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Normal,
    Active,
    Caution,
    Danger,
}

fn button(id: impl Into<SharedString>, text: impl Into<SharedString>, style: Style) -> Stateful<Div> {
    let (bg, fg, border) = match style {
        Style::Normal => (CONTROL, WHITE, BEZEL_EDGE),
        Style::Active => (ACTIVE_FILL, GREEN, GREEN_DIM),
        Style::Caution => (CAUTION_FILL, AMBER, AMBER),
        Style::Danger => (DANGER_FILL, ON_DANGER, RED),
    };
    div()
        .id(id.into())
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
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .gap_2()
                .p_3()
                .bg(c(SCREEN))
                .border_2()
                .border_color(c(0x000000))
                .rounded(px(SCREEN_RADIUS))
                .child(body),
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
            target: Target::Station,
            flight_controller: true,
            raytrace: false,
            fullscreen: false,
            no_build: false,
            map: None,
            maps,
            devices: controllers::connected(),
            serial: controllers::serial_radios(),
            use_serial: true,
            mapping: controllers::mapping(),
            job: None,
            notice: String::new(),
            log_tab: None,
            logs: vec![],
            file_lines: vec![],
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

    fn busy(&self) -> bool {
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
                let mut args: Vec<String> = vec!["run".into(), "--release".into(), "--".into()];
                args.extend(flags);
                ("cargo".into(), args, l.home.join(crate_dir))
            }
        }
    }

    /// The sim's own arguments.
    fn sim_args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec![];
        a.push("--aircraft".into());
        a.push(if self.flight_controller { "t1s-vtol" } else { "t1s-ranger" }.into());
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
    /// through the sim's own modes and ArduPilot only watches (`--direct`).
    fn station_args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec!["--station".into()];
        if let Some(m) = &self.map {
            a.extend(["--map".into(), m.clone()]);
        }
        if self.raytrace {
            a.push("--raytrace".into());
        }
        if !self.flight_controller {
            a.push("--direct".into());
        }
        if let Some(r) = self.serial_radio() {
            a.extend(["--tx15-serial".into(), r.path.clone()]);
        }
        // A bundle has nothing to build.
        if self.no_build || layout().bundled() {
            a.push("--no-build".into());
        }
        a
    }

    /// The TX15 the station will use on its serial line, if one is plugged in and chosen.
    fn serial_radio(&self) -> Option<&SerialRadio> {
        if !self.use_serial || !self.flight_controller {
            return None;
        }
        self.serial.iter().find(|r| r.radio)
    }

    fn launch(&mut self) {
        match self.target {
            Target::Sim => {
                let title = if self.flight_controller {
                    "Simulator, T1-S VTOL with ArduPilot"
                } else {
                    "Simulator, T1-S Ranger"
                };
                let (program, args, dir) = self.program("sim001", "", self.sim_args());
                self.run(title, &program, args, dir);
            }
            Target::Station => {
                let title = if self.flight_controller {
                    "Ground station with ArduPilot"
                } else {
                    "Ground station, sticks direct"
                };
                let (program, args, dir) =
                    self.program("sim001_gcs", "domains/teleoperation/console", self.station_args());
                self.run(title, &program, args, dir);
            }
        }
    }

    fn calibrate(&mut self) {
        let flags = ["--aircraft", "t1s-ranger", "--calibrate"].map(String::from).to_vec();
        let (program, args, dir) = self.program("sim001", "", flags);
        self.run("Controller setup", &program, args, dir);
    }

    /// Writes the manual from the saved test results (`expectation docs`).
    fn render_docs(&mut self) {
        let args =
            ["run", "-q", "-p", "expectation", "--", "docs", "expectations", "docs"].map(String::from).to_vec();
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

    fn controllers_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_1();
        if self.devices.is_empty() && self.serial.is_empty() {
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
        for r in &self.serial {
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
                    row()
                        .child(button("calibrate", "SET UP CONTROLLER", Style::Caution).on_click(cx.listener(|this, _, _, cx| {
                            if this.devices.is_empty() {
                                this.devices = controllers::connected();
                            }
                            this.calibrate();
                            cx.notify();
                        })))
                        .child(button("rescan", "RESCAN", Style::Normal).on_click(cx.listener(|this, _, _, cx| {
                            this.devices = controllers::connected();
                            this.serial = controllers::serial_radios();
                            this.mapping = controllers::mapping();
                            cx.notify();
                        })))
                        .child(button("cfg", "OPEN CONTROLS.CFG", Style::Normal).on_click(cx.listener(|this, _, _, cx| {
                            this.open(layout().data.join("controls.cfg"));
                            cx.notify();
                        }))),
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
                    row()
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
                                    ["run", "--release", "-p", "sim001_tasks", "--", "docs", "--no-station"].map(String::from).to_vec(),
                                    layout().home.clone(),
                                );
                                cx.notify();
                            }))
                        })),
                ),
        )
    }

    fn launch_panel(&self, cx: &mut Context<Self>) -> Div {
        let busy = self.busy();
        let fc = self.flight_controller;
        let target = self.target;
        let explain = match (target, fc) {
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
        let mut maps = row().child(self.choice("map-none", "ALPINE VALLEY", self.map.is_none(), cx, |t| t.map = None));
        for m in &self.maps {
            let name = m.clone();
            let id: SharedString = format!("map-{m}").into();
            maps = maps.child(
                button(id, m.to_uppercase(), if self.map.as_ref() == Some(m) { Style::Active } else { Style::Normal })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.map = Some(name.clone());
                        cx.notify();
                    })),
            );
        }
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
                    row()
                        .child(
                            self.choice("t-sim", "SIMULATOR ONLY", target == Target::Sim, cx, |t| {
                                t.target = Target::Sim
                            }),
                        )
                        .child(self.choice("t-station", "GROUND STATION", target == Target::Station, cx, |t| {
                            t.target = Target::Station
                        })),
                )
                .child(
                    row()
                        .child(self.choice("fc-on", "WITH ARDUPILOT", fc, cx, |t| t.flight_controller = true))
                        .child(self.choice("fc-off", "WITHOUT ARDUPILOT", !fc, cx, |t| t.flight_controller = false)),
                )
                .child(note(explain))
                .children(
                    self.serial_radio().map(|r| {
                        note(format!("The {} on {} carries the sticks, telemetry and MAVLink.", r.name, r.path))
                    }),
                )
                .child(note("Place"))
                .child(maps)
                .child(
                    row()
                        .child(self.choice("rt", "RAY TRACING", self.raytrace, cx, |t| t.raytrace = !t.raytrace))
                        .children((target == Target::Sim).then(|| {
                            self.choice("fs", "FULL SCREEN", self.fullscreen, cx, |t| t.fullscreen = !t.fullscreen)
                        }))
                        .children((fc && self.serial.iter().any(|r| r.radio)).then(|| {
                            self.choice("ser", "TX15 ON SERIAL", self.use_serial, cx, |t| t.use_serial = !t.use_serial)
                        }))
                        .children((target == Target::Station).then(|| {
                            self.choice("nb", "SKIP SIM REBUILD", self.no_build, cx, |t| t.no_build = !t.no_build)
                        })),
                )
                .child(row().child(go)),
        )
    }

    /// The log pane: the program's output, or a log file of the run (the console's debug log,
    /// ArduPlane's own output), chosen by the tabs along its top.
    fn log_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = row().child(button("log-output", "OUTPUT", if self.log_tab.is_none() { Style::Active } else { Style::Normal }).on_click(cx.listener(|this, _, _, cx| {
            this.log_tab = None;
            cx.notify();
        })));
        for l in &self.logs {
            let name = l.name.clone();
            let id: SharedString = format!("log-{}", l.name).into();
            tabs = tabs.child(button(id, l.name.to_uppercase(), if self.log_tab.as_deref() == Some(l.name.as_str()) { Style::Active } else { Style::Normal }).on_click(cx.listener(move |this, _, _, cx| {
                this.log_tab = Some(name.clone());
                this.file_lines.clear();
                cx.notify();
            })));
        }
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
                    .child(div().flex().flex_col().gap(px(GAP)).w(px(520.0)).child(div().flex().flex_col().flex_none().child(launch)).child(div().flex().flex_col().flex_1().min_h(px(0.0)).child(ctrl)))
                    .child(div().flex().flex_col().gap(px(GAP)).flex_1().min_w(px(0.0)).child(div().flex().flex_col().flex_none().child(docs)).child(div().flex().flex_col().flex_1().min_h(px(0.0)).child(log))),
            )
    }
}
