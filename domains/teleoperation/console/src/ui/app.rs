//! Root view: the console layout and its panels.

use super::draw::round_corners;
use super::map::{MapData, MapState, Tool};
use super::theme::*;
use super::{charts, hud, map};
use crate::feed::Sensor;
use crate::geo::{Origin, TerrainMap, wrap360};
use crate::link::{Cmd, Shared};
use crate::vehicle::{Vehicle, Waypoint, WpKind, mode_number};
use gpui::{
    AnyElement, Bounds, Context, Div, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Render, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Stateful, Window, canvas, div, prelude::*, px,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use teleoperation_core::debug::{self, Level, Source};

thread_local! {
    /// Where each button was last painted, by id, so `--expect` can click it the way a
    /// person would. Written while painting, read between frames; both on the UI thread.
    static BUTTONS: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::new(HashMap::new());
    /// How tall each row of buttons wants to be at its current width, by id (see [`fit`]).
    static ROWS: RefCell<HashMap<&'static str, f32>> = RefCell::new(HashMap::new());
    /// The rows of buttons that MORE has opened out to every line.
    static EXPANDED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
}

/// The height of one line of buttons: a row that wants more than this is cut to it, behind MORE.
const ROW_H: f32 = 28.0;

/// A wrapping row of buttons that shows one line and a MORE button when the rest does not fit;
/// MORE opens it out (and LESS closes it). Whether it fits is read from the last frame.
fn fit(id: &'static str, content: Div) -> Div {
    let open = EXPANDED.with(|e| e.borrow().contains(id));
    let natural = ROWS.with(|r| r.borrow().get(id).copied()).unwrap_or(0.0);
    let measure =
        canvas(move |b, _, _| ROWS.with(|r| r.borrow_mut().insert(id, f32::from(b.size.height))), |_, _, _, _| {})
            .absolute()
            .inset_0();
    let mut clip = div().flex_1().min_w(px(0.0)).overflow_hidden();
    if !open {
        clip = clip.max_h(px(ROW_H));
    }
    div().flex().items_start().gap_1().child(clip.child(content.relative().child(measure))).children(
        (natural > ROW_H + 2.0).then(move || {
            button(format!("more-{id}"), if open { "LESS ▴" } else { "MORE ▾" }, Style::Normal).on_click(
                move |_, window, _| {
                    EXPANDED.with(|e| {
                        let mut e = e.borrow_mut();
                        if !e.remove(id) {
                            e.insert(id);
                        }
                    });
                    window.refresh();
                },
            )
        }),
    )
}

/// The rows of buttons that are cut to one line now, behind a MORE button: their other buttons are
/// painted, but out of sight, and a click there lands on what is beneath them.
pub fn rows_cut_short() -> Vec<&'static str> {
    let open = EXPANDED.with(|e| e.borrow().clone());
    ROWS.with(|r| {
        let mut cut: Vec<_> =
            r.borrow().iter().filter(|(id, h)| **h > ROW_H + 2.0 && !open.contains(*id)).map(|(id, _)| *id).collect();
        cut.sort_unstable();
        cut
    })
}

/// Where the button with this id is on screen now, if it is.
pub fn button_bounds(id: &str) -> Option<Bounds<Pixels>> {
    BUTTONS.with(|b| b.borrow().get(id).copied())
}

/// The lines of a survey grid are this far apart (m).
const SURVEY_SPACING: f32 = 40.0;
/// A height check raises a waypoint to at least this far above the ground below it (m).
const TERRAIN_CLEARANCE: f32 = 30.0;
/// The highest a plan altitude goes (ALT+ stops here).
const MAX_PLAN_ALT: f32 = 300.0;

/// A VTOL's own modes (ArduPlane QuadPlane), shown when the plane has lift rotors.
const Q_MODE_BUTTONS: [&str; 5] = ["QSTABILIZE", "QHOVER", "QLOITER", "QLAND", "QRTL"];

const MODE_BUTTONS: [&str; 10] =
    ["MANUAL", "FBWA", "FBWB", "CRUISE", "AUTO", "RTL", "LOITER", "CIRCLE", "GUIDED", "TAKEOFF"];

pub struct Gcs {
    pub(crate) vehicle: Shared,
    pub(crate) tx: Sender<Cmd>,
    pub(crate) origin: Origin,
    terrain: Option<Arc<TerrainMap>>,
    map_image: Option<Arc<RenderImage>>,
    pub(crate) map: MapState,
    pub(crate) map_bounds: Rc<Cell<Bounds<Pixels>>>,
    arm_confirm: Option<Instant>,
    /// When DISARM was last clicked: clicking again while still armed forces it.
    disarm_at: Option<Instant>,
    reboot_confirm: Option<Instant>,
    /// AUTO clicked with no mission on the vehicle: a second click goes ahead.
    auto_confirm: Option<Instant>,
    /// The aircraft camera, when the sensor feed is in use (`--station` / `--sensor`).
    pub(crate) sensor: Option<Arc<Sensor>>,
    /// The plane is a VTOL: show its Q modes.
    vtol: bool,
    /// The MESSAGES screen shows the debug view: the autopilot's state, its process log and
    /// everything that happened on the link.
    pub(crate) debug_view: bool,
    /// The last lines of the autopilot's own output, read once a second.
    proc_log: (Option<Instant>, Vec<String>),
}

/// Below this window width the console drops to two columns; with a sensor window open the
/// flight display is left to the sensor feed's HUD.
const COMPACT_WIDTH: f32 = 1500.0;

impl Gcs {
    pub fn new(
        vehicle: Shared,
        tx: Sender<Cmd>,
        origin: Origin,
        terrain: Option<Arc<TerrainMap>>,
        sensor: Option<Arc<Sensor>>,
        vtol: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let map_image = terrain.as_deref().and_then(map::load_image);
        if terrain.is_some() && map_image.is_none() {
            eprintln!("terrain map metadata found but the PNG failed to load");
        }
        // Telemetry arrives on another thread; repaint at ~30 Hz.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            vehicle,
            tx,
            origin,
            terrain,
            map_image,
            map: MapState::new(),
            map_bounds: Rc::new(Cell::new(Bounds::default())),
            arm_confirm: None,
            disarm_at: None,
            reboot_confirm: None,
            auto_confirm: None,
            sensor,
            vtol,
            // SIM001_DEBUG_VIEW=1 opens on the debug view (for a screenshot of it, and while debugging).
            debug_view: std::env::var_os("SIM001_DEBUG_VIEW").is_some(),
            proc_log: (None, vec![]),
        }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    fn agl(&self, v: &Vehicle) -> Option<f32> {
        let t = self.terrain.as_ref()?;
        if !v.have_position {
            return None;
        }
        let (n, e) = self.origin.to_ne(v.lat, v.lon);
        // Map heights are relative to the origin's ground.
        Some(v.alt_msl - self.origin.alt - t.height(n, e)?)
    }

    fn log(&self, sev: u8, s: impl Into<String>) {
        self.vehicle.lock().unwrap().push_log("GCS", sev, s);
    }

    // --- map interaction ----------------------------------------------------------------

    fn map_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        self.map.bounds = self.map_bounds.get();
        let (sx, sy) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let (n, e) = self.map.screen_to_ne(sx, sy);
        match (ev.button, self.map.tool) {
            (MouseButton::Left, Tool::Goto) => {
                let (lat, lon) = self.origin.to_ll(n, e);
                let alt = {
                    let v = self.vehicle.lock().unwrap();
                    if v.alt_rel > 15.0 { v.alt_rel.round() } else { self.map.plan_alt }
                };
                self.send(Cmd::Goto { lat, lon, alt });
                self.map.tool = Tool::Pan;
            }
            (MouseButton::Left, Tool::Survey) => self.map.survey.push((n, e)),
            (MouseButton::Right, Tool::Survey) => {
                self.map.survey.pop();
            }
            (MouseButton::Left, Tool::Measure) => {
                // The first click starts a measurement (and forgets the last one), the second ends
                // it and lets the map pan again, with the distance still shown.
                if self.map.measure.len() >= 2 {
                    self.map.measure.clear();
                }
                self.map.measure.push((n, e));
                if self.map.measure.len() == 2 {
                    self.map.tool = Tool::Pan;
                }
            }
            (MouseButton::Left, Tool::Look) => {
                if let Some(s) = &self.sensor {
                    s.track_point(n, e);
                }
                self.map.tool = Tool::Pan;
            }
            (MouseButton::Left, Tool::Plan) => {
                let (lat, lon) = self.origin.to_ll(n, e);
                self.map.plan.push(Waypoint { kind: WpKind::Waypoint, lat, lon, alt: self.map.plan_alt });
            }
            (MouseButton::Right, Tool::Plan) => {
                self.map.plan.pop();
            }
            (MouseButton::Left, Tool::Pan) => {
                self.map.drag = Some(((sx, sy), self.map.centre));
            }
            _ => {}
        }
        cx.notify();
    }

    fn map_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        self.map.bounds = self.map_bounds.get();
        let (sx, sy) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(((x0, y0), c0)) = self.map.drag {
            if ev.dragging() {
                let (dx, dy) = (sx - x0, sy - y0);
                if dx.abs() + dy.abs() > 3.0 {
                    self.map.follow = false;
                }
                self.map.centre = (c0.0 + dy * self.map.mpp, c0.1 - dx * self.map.mpp);
            } else {
                self.map.drag = None;
            }
        }
        let b = self.map.bounds;
        self.map.cursor = b.contains(&ev.position).then(|| self.map.screen_to_ne(sx, sy));
        cx.notify();
    }

    fn map_scroll(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.map.bounds = self.map_bounds.get();
        let dy = match ev.delta {
            ScrollDelta::Pixels(p) => f32::from(p.y),
            ScrollDelta::Lines(l) => l.y * 20.0,
        };
        let (sx, sy) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if self.map.follow {
            // Zoom about the aircraft while following.
            self.map.mpp = (self.map.mpp * (-dy * 0.004).exp()).clamp(0.08, 12.0);
        } else {
            self.map.zoom_at(sx, sy, (-dy * 0.004).exp());
        }
        cx.notify();
    }

    // --- panels -------------------------------------------------------------------------

    fn top_bar(&self, v: &Vehicle) -> Div {
        let utc = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (hh, mm, ss) = ((utc / 3600) % 24, (utc / 60) % 60, utc % 60);
        let (link_txt, link_col) = if v.link.alive() {
            (
                format!(
                    "LINK {}  ·  {}  ·  SYS {}{}",
                    v.link.address,
                    v.link.status,
                    v.sysid,
                    v.ping.last_ms().map_or(String::new(), |ms| format!("  ·  PING {ms:.0} ms"))
                ),
                GREEN,
            )
        } else if v.link.connected {
            (format!("LINK {}  ·  {}", v.link.address, v.link.status), AMBER)
        } else {
            (format!("LINK DOWN  ·  {}", v.link.status), RED)
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(30.0))
            .px_3()
            .bg(c(TITLE_BG))
            // The window has no title bar of its own: drag this strip to move it.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .border_1()
            .border_color(c(BEZEL_EDGE))
            .rounded(px(PANEL_RADIUS))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(div().text_color(c(WHITE)).child("SIM001  GROUND CONTROL STATION"))
                    .child(div().text_color(c(GREY)).child("PILOT / PAYLOAD CONSOLE")),
            )
            .child(div().text_color(c(link_col)).child(link_txt))
            .child(div().text_color(c(WHITE)).child(format!("{hh:02}:{mm:02}:{ss:02}Z")))
    }

    fn command_panel(&mut self, v: &Vehicle, cx: &mut Context<Self>) -> Div {
        let confirming = self.arm_confirm.is_some_and(|t| t.elapsed() < ARM_CONFIRM);
        if !confirming {
            self.arm_confirm = None;
        }
        let alive = v.link.alive();
        let _armed = v.armed;
        // ArduPilot reports whether its pre-arm checks pass; until then (e.g. the first ~30 s
        // while the EKF initialises) say so, with the latest reason. Arming is still allowed.
        const PREARM_CHECK: u32 = 0x1000_0000;
        let ready = v.armed || v.sensor_ok(PREARM_CHECK).unwrap_or(true);
        let not_ready_reason = (!ready && alive).then(|| {
            v.prearm
                .as_ref()
                .filter(|p| p.0.elapsed() < Duration::from_secs(40))
                .map(|p| p.1.trim_start_matches("PreArm: ").trim_start_matches("Arm: ").to_string())
                .unwrap_or_else(|| "waiting for pre-arm checks".into())
        });
        // A disarm ArduPilot refused (it thinks it's flying) can be forced with a second click.
        let force = v.armed && self.disarm_at.is_some_and(|t| (1..8).contains(&t.elapsed().as_secs()));
        let arm = if v.armed {
            button("disarm", if force { "FORCE DISARM" } else { "DISARM" }, Style::Danger).on_click(cx.listener(
                move |this, _, _, cx| {
                    match disarm_click(&mut this.disarm_at, Instant::now()) {
                        Disarm::Force => this.send(Cmd::ForceDisarm),
                        Disarm::Ask => this.send(Cmd::Arm(false)),
                    }
                    cx.notify();
                },
            ))
        } else {
            let (text, style) = if confirming {
                ("CONFIRM ARM", Style::Danger)
            } else if not_ready_reason.is_some() {
                ("ARM: NOT READY", Style::Normal)
            } else {
                ("ARM", Style::Caution)
            };
            button("arm", text, style).on_click(cx.listener(|this, _, _, cx| {
                if arm_click(&mut this.arm_confirm, Instant::now()) {
                    this.send(Cmd::Arm(true));
                }
                cx.notify();
            }))
        };
        // A VTOL takes off straight up, to the plan altitude.
        let takeoff = self.vtol.then(|| {
            button("vtol-tkof", "VTOL TKOF", Style::Normal).on_click(cx.listener(move |this, _, _, _| {
                // Whether it is armed now, not when the screen was last drawn: a console that is slow
                // to redraw would refuse a take-off for an arm it had not drawn yet.
                if this.vehicle.lock().unwrap().armed {
                    this.send(Cmd::Takeoff { alt: this.map.plan_alt });
                } else {
                    this.log(4, "Arm the vehicle first: VTOL TKOF lifts an armed VTOL straight up");
                }
            }))
        });
        let rebooting = self.reboot_confirm.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
        let reboot = button(
            "reboot",
            if rebooting { "CONFIRM REBOOT" } else { "REBOOT FC" },
            if rebooting { Style::Danger } else { Style::Normal },
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if arm_click(&mut this.reboot_confirm, Instant::now()) {
                this.send(Cmd::Reboot);
            }
            cx.notify();
        }));
        let hint = if confirming || rebooting {
            div().text_color(c(GREY)).child("click again within 3 s")
        } else if let Some(reason) = not_ready_reason {
            div().text_color(c(AMBER)).child(reason)
        } else {
            div().child(" ")
        };
        // When the sim's sticks are flying, ArduPilot's mode buttons don't steer the plane.
        let direct = self.sensor.as_ref().is_some_and(|s| s.direct());
        // AUTO with nothing uploaded just RTLs (low, right after a launch): ask first.
        let no_mission = v.mission.len() <= 1;
        let auto_pending = self.auto_confirm.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        let mode_button = |m: &'static str, cx: &mut Context<Self>| {
            let active = alive && v.mode() == m;
            let (label, style) = if m == "AUTO" && auto_pending && no_mission {
                ("AUTO: NO MISSION?", Style::Caution)
            } else {
                (m, if active { Style::Active } else { Style::Normal })
            };
            button(SharedString::from(format!("mode-{m}")), label, style).on_click(cx.listener(move |this, _, _, _| {
                let no_mission = this.vehicle.lock().unwrap().mission.len() <= 1;
                if m == "AUTO" && no_mission && !this.auto_confirm.is_some_and(|t| t.elapsed() < Duration::from_secs(4)) {
                    this.auto_confirm = Some(Instant::now());
                    this.log(4, "No mission on the vehicle: AUTO would go straight to RTL. Plan one (PLAN, +TKOF, UPLOAD), or click AUTO again to do it anyway");
                    return;
                }
                this.auto_confirm = None;
                if let Some(n) = mode_number(m) {
                    this.send(Cmd::SetMode(n));
                }
            }))
        };
        let modes: Vec<_> = MODE_BUTTONS.iter().map(|&m| mode_button(m, cx)).collect();
        let q_modes: Vec<_> =
            if self.vtol { Q_MODE_BUTTONS.iter().map(|&m| mode_button(m, cx)).collect() } else { vec![] };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .size_full()
            .child(fit(
                "r-vehicle",
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .child(label("VEHICLE"))
                    .child(arm)
                    .child(reboot)
                    .children(takeoff),
            ))
            // The reason the vehicle isn't ready (or a confirmation) has a line of its own, so
            // the buttons keep their places.
            .child(hint)
            .child(fit("r-modes", div().flex().flex_wrap().gap_1().items_center().child(label("MODE")).children(modes)))
            .children((!q_modes.is_empty()).then(|| {
                fit("r-qmodes", div().flex().flex_wrap().gap_1().items_center().child(label("VTOL")).children(q_modes))
            }))
            .children(direct.then(|| {
                div().text_color(c(CYAN)).child(
                    "DIRECT: the sim's sticks are flying (J / D-pad down in the sim). ArduPilot is only watching.",
                )
            }))
    }

    fn map_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let st = &self.map;
        let tool_btn = |id: &'static str, text: &'static str, tool: Tool| {
            button(id, text, if st.tool == tool { Style::Active } else { Style::Normal }).on_click(
                cx.listener(move |this, _, _, _| this.map.tool = if this.map.tool == tool { Tool::Pan } else { tool }),
            )
        };
        let plan_len = st.plan.len();
        let bar = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap_1()
            .items_center()
            .child(
                button("follow", "FOLLOW", if st.follow { Style::Active } else { Style::Normal })
                    .on_click(cx.listener(|this, _, _, _| this.map.follow = !this.map.follow)),
            )
            .child(tool_btn("goto", "GOTO", Tool::Goto))
            .children(self.sensor.is_some().then(|| tool_btn("look", "LOOK", Tool::Look)))
            .child(tool_btn("plan", "PLAN", Tool::Plan))
            .child(tool_btn("measure", "MEASURE", Tool::Measure))
            .child(
                button("alt-dn", "ALT-", Style::Normal)
                    .on_click(cx.listener(|this, _, _, _| this.map.plan_alt = (this.map.plan_alt - 10.0).max(10.0))),
            )
            .child(div().text_color(c(AMBER)).min_w(px(44.0)).child(format!("{:.0} m", st.plan_alt)))
            .child(
                button("alt-up", "ALT+", Style::Normal)
                    .on_click(cx.listener(|this, _, _, _| this.map.plan_alt = (this.map.plan_alt + 10.0).min(300.0))),
            )
            .child(button("tkof", "+TKOF", Style::Normal).on_click(cx.listener(|this, _, _, _| this.add_takeoff())))
            .child(button("rtl", "+RTL", Style::Normal).on_click(cx.listener(|this, _, _, _| {
                this.map.plan.push(Waypoint { kind: WpKind::Rtl, lat: 0.0, lon: 0.0, alt: 0.0 });
            })))
            .child(button("clear", "CLEAR", Style::Normal).on_click(cx.listener(|this, _, _, _| this.map.plan.clear())))
            .child(tool_btn("survey", "GRID", Tool::Survey))
            .child(
                button("fill-grid", "FILL GRID", Style::Normal).on_click(cx.listener(|this, _, _, _| this.fill_grid())),
            )
            .child(
                button("check-heights", "CHECK HT", Style::Normal)
                    .on_click(cx.listener(|this, _, _, _| this.check_heights())),
            )
            .child(button("cmd", "CMD", Style::Normal).on_click(cx.listener(|this, _, _, _| this.next_command())))
            .child(button("save-plan", "SAVE", Style::Normal).on_click(cx.listener(|this, _, _, _| this.save_plan())))
            .child(button("load-plan", "LOAD", Style::Normal).on_click(cx.listener(|this, _, _, _| this.load_plan())))
            .child(
                button(
                    "upload",
                    format!("UPLOAD {plan_len}"),
                    if plan_len > 0 { Style::Caution } else { Style::Normal },
                )
                .on_click(cx.listener(|this, _, _, _| {
                    if this.map.plan.is_empty() {
                        this.log(4, "Nothing to upload: use PLAN to click waypoints on the map");
                    } else {
                        this.send(Cmd::Upload(this.map.plan.clone()));
                        this.map.tool = Tool::Pan;
                    }
                })),
            )
            .child(
                button("download", "DOWNLOAD", Style::Normal)
                    .on_click(cx.listener(|this, _, _, _| this.send(Cmd::Download))),
            )
            .child(
                button("edit", "EDIT", Style::Normal)
                    .on_click(cx.listener(|this, _, _, _| this.edit_vehicle_mission())),
            )
            .child(
                button("restart", "WP1", Style::Normal)
                    .on_click(cx.listener(|this, _, _, _| this.send(Cmd::SetCurrentWp(1)))),
            );
        fit("r-map", bar).flex_1().min_w(px(0.0))
    }

    /// Covers the area of GRID with lines of waypoints at the plan altitude (Auto WP, Grid).
    fn fill_grid(&mut self) {
        let area = std::mem::take(&mut self.map.survey);
        let points = super::survey::grid(&area, SURVEY_SPACING);
        if points.is_empty() {
            self.log(4, "A grid needs an area: use GRID and click at least three corners");
            return;
        }
        for (n, e) in points {
            let (lat, lon) = self.origin.to_ll(n, e);
            self.map.plan.push(Waypoint { kind: WpKind::Waypoint, lat, lon, alt: self.map.plan_alt });
        }
        self.map.tool = Tool::Pan;
    }

    /// Raises the waypoints that are too near the ground below them; the plan is then checked
    /// (until it changes), or too low somewhere if a waypoint would have to go above 300 m.
    fn check_heights(&mut self) {
        let Some(terrain) = self.terrain.clone() else {
            self.log(4, "No terrain map to check the heights against");
            return;
        };
        let mut clear = true;
        for wp in self.map.plan.iter_mut().filter(|w| w.has_position()) {
            let (n, e) = self.origin.to_ne(wp.lat, wp.lon);
            let Some(ground) = terrain.height(n, e) else { continue };
            let wanted = (ground + TERRAIN_CLEARANCE).max(0.0);
            if wp.alt < wanted {
                wp.alt = wanted.min(MAX_PLAN_ALT);
                clear &= wanted <= MAX_PLAN_ALT;
            }
        }
        self.map.checked = Some((self.map.plan.len(), clear));
    }

    /// Changes the command of the last item of the plan to the next one, in its place.
    fn next_command(&mut self) {
        let Some(last) = self.map.plan.last().copied() else {
            self.log(4, "No plan item to change");
            return;
        };
        let kind = match last.kind {
            WpKind::Waypoint => WpKind::LoiterUnlim,
            WpKind::LoiterUnlim => WpKind::LoiterTurns,
            WpKind::LoiterTurns => WpKind::Land,
            WpKind::Land => WpKind::Rtl,
            WpKind::Rtl | WpKind::Takeoff | WpKind::Other(_) => WpKind::Waypoint,
        };
        // An RTL has no place of its own: the item that takes its place goes where the one before it is.
        let (lat, lon) = match (last.lat == 0.0 && last.lon == 0.0, self.map.plan.len().checked_sub(2)) {
            (true, Some(before)) => (self.map.plan[before].lat, self.map.plan[before].lon),
            _ => (last.lat, last.lon),
        };
        let item = match kind {
            WpKind::Rtl => Waypoint { kind, lat: 0.0, lon: 0.0, alt: 0.0 },
            _ => Waypoint { kind, lat, lon, alt: if last.alt > 0.0 { last.alt } else { self.map.plan_alt } },
        };
        *self.map.plan.last_mut().expect("a last item") = item;
        self.map.checked = None;
    }

    /// Writes the plan to the mission file (Mission Planner's Save WP File).
    fn save_plan(&self) {
        let home = {
            let v = self.vehicle.lock().unwrap();
            v.home.map_or_else(|| self.origin.to_ll(0.0, 0.0), |h| (h.0, h.1))
        };
        let path = crate::wpfile::path();
        match crate::wpfile::save(&path, &self.map.plan, home) {
            Ok(()) => self.log(6, format!("Plan saved to {}", path.display())),
            Err(e) => self.log(4, format!("Could not save the plan: {e}")),
        }
    }

    /// Brings the plan back from the mission file (Load WP File). With no file, nothing changes.
    fn load_plan(&mut self) {
        let path = crate::wpfile::path();
        if !path.exists() {
            self.log(4, "No mission file to load: use SAVE first");
            return;
        }
        match crate::wpfile::load(&path) {
            Ok(plan) => {
                self.log(6, format!("Plan loaded from {} ({} items)", path.display(), plan.len()));
                self.map.plan = plan;
            }
            Err(e) => self.log(4, format!("Could not load the plan: {e}")),
        }
    }

    pub(crate) fn add_takeoff(&mut self) {
        if self.map.plan.first().is_some_and(|w| w.kind == WpKind::Takeoff) {
            return;
        }
        let (lat, lon) = {
            let v = self.vehicle.lock().unwrap();
            v.home.map(|h| (h.0, h.1)).unwrap_or(self.origin.to_ll(0.0, 0.0))
        };
        self.map.plan.insert(0, Waypoint { kind: WpKind::Takeoff, lat, lon, alt: self.map.plan_alt.min(40.0) });
    }

    /// Copy the vehicle's mission (minus home) into the editable plan.
    fn edit_vehicle_mission(&mut self) {
        let mission = self.vehicle.lock().unwrap().mission.clone();
        self.map.plan = mission.into_iter().skip(1).collect();
        self.map.tool = Tool::Plan;
    }

    fn annunciators(&self, v: &Vehicle, agl: Option<f32>, tile_w: f32) -> Div {
        use Ann::*;
        let alive = v.link.alive();
        let flash = ((v.now() * 2.0) as u32).is_multiple_of(2);
        let s = |bit: u32| -> Ann {
            match v.sensor_ok(bit) {
                None => Off,
                Some(true) => Ok,
                Some(false) => Warn,
            }
        };
        const AHRS: u32 = 0x20_0000;
        const FENCE: u32 = 0x10_0000;
        const RC: u32 = 0x1_0000;
        const PREARM: u32 = 0x1000_0000;
        let ekf_good = v.ekf_flags & (1 | 2 | 16) == (1 | 2 | 16) && v.ekf_flags & (128 | 1024) == 0;
        let ekf_var = v.ekf_vel_var.max(v.ekf_pos_var).max(v.ekf_hgt_var).max(v.ekf_mag_var);
        let flying = v.armed && v.alt_rel > 3.0;
        let tiles: Vec<(&str, Ann)> = vec![
            (
                "LINK",
                if alive {
                    Ok
                } else if v.link.connected {
                    Caution
                } else {
                    Warn
                },
            ),
            (
                "GPS",
                if !alive {
                    Off
                } else if v.gps_fix >= 3 {
                    Ok
                } else if v.gps_fix == 2 {
                    Caution
                } else {
                    Warn
                },
            ),
            (
                "EKF",
                if !alive {
                    Off
                } else if ekf_var > 0.8 {
                    Warn
                } else if !ekf_good || ekf_var > 0.5 {
                    Caution
                } else {
                    Ok
                },
            ),
            ("AHRS", if alive { s(AHRS) } else { Off }),
            (
                "BATT",
                match v.cell_v() {
                    _ if !alive => Off,
                    None => Off,
                    Some(cv) if cv < 3.3 || (0..10).contains(&v.batt_pct) => Warn,
                    Some(cv) if cv < 3.5 || (0..20).contains(&v.batt_pct) => Caution,
                    Some(_) => Ok,
                },
            ),
            (
                "ARMED",
                if !alive {
                    Off
                } else if v.armed {
                    Caution
                } else {
                    Ok
                },
            ),
            (
                "PREARM",
                if !alive || v.armed {
                    Off
                } else if v.prearm.as_ref().is_some_and(|p| p.0.elapsed().as_secs() < 10) {
                    Caution
                } else {
                    s(PREARM)
                },
            ),
            (
                "STALL",
                if flying && v.airspeed < 9.5 {
                    Warn
                } else if alive {
                    Ok
                } else {
                    Off
                },
            ),
            (
                "TERRAIN",
                match agl {
                    Some(a) if flying && a < 8.0 && v.climb < -0.5 => Warn,
                    Some(a) if flying && a < 20.0 => Caution,
                    Some(_) if alive => Ok,
                    _ => Off,
                },
            ),
            ("FENCE", if alive { s(FENCE) } else { Off }),
            ("RC", if alive { s(RC) } else { Off }),
            (
                "XTRACK",
                if alive && v.mode() == "AUTO" && v.xtrack.abs() > 25.0 {
                    Caution
                } else if alive {
                    Ok
                } else {
                    Off
                },
            ),
        ];
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .p_2()
            .children(tiles.into_iter().map(|(name, a)| annunciator(name, a, flash, tile_w)))
    }

    fn systems(&self, v: &Vehicle, agl: Option<f32>) -> Div {
        let row = |k: &str, val: String, col: u32| {
            div()
                .flex()
                .justify_between()
                .child(div().text_color(c(GREY)).child(k.to_string()))
                .child(div().text_color(c(col)).child(val))
        };
        let home = v.home.map(|h| {
            let (n, e) = self.origin.to_ne(h.0, h.1);
            let (an, ae) = self.origin.to_ne(v.lat, v.lon);
            let (dn, de) = (n - an, e - ae);
            ((dn * dn + de * de).sqrt(), wrap360(de.atan2(dn).to_degrees()))
        });
        let ft = v.flight_secs();
        let fix = match v.gps_fix {
            0 | 1 => "NO FIX",
            2 => "2D",
            3 => "3D",
            4 => "DGPS",
            5 => "RTK FLT",
            _ => "RTK FIX",
        };
        div()
            .flex()
            .flex_col()
            .p_2()
            .gap(px(1.0))
            .child(row("MODE", v.mode().to_string(), WHITE))
            .child(row("FLIGHT TIME", format!("{:02}:{:02}", (ft / 60.0) as u32, (ft % 60.0) as u32), GREEN))
            .child(row("ALT REL / AMSL", format!("{:.1} / {:.1} m", v.alt_rel, v.alt_msl), GREEN))
            .child(row("AGL (SIM TERRAIN)", agl.map_or("--".into(), |a| format!("{a:.1} m")), GREEN))
            .child(row("IAS / GS", format!("{:.1} / {:.1} m/s", v.airspeed, v.groundspeed), GREEN))
            .child(row("CLIMB", format!("{:+.1} m/s", v.climb), GREEN))
            .child(row("HDG / THR", format!("{:03.0}° / {}%", wrap360(v.yaw.to_degrees()), v.throttle), GREEN))
            .child(row(
                "BATTERY",
                format!(
                    "{}S {:.2} V {:.1} A {} mAh{}",
                    v.batt_cells,
                    v.batt_v,
                    v.batt_a,
                    v.batt_mah,
                    if v.batt_pct >= 0 { format!(" {}%", v.batt_pct) } else { String::new() }
                ),
                AMBER,
            ))
            .child(row("GPS", format!("{fix} {} SV HDOP {:.1}", v.sats, v.hdop), GREEN))
            .child(row(
                "WIND (EST)",
                if v.wind_valid { format!("{:03.0}° {:.1} m/s", v.wind_dir, v.wind_speed) } else { "--".into() },
                CYAN,
            ))
            .child(row("HOME", home.map_or("--".into(), |(d, b)| format!("{d:.0} m @ {b:03.0}°")), AMBER))
            .child(row(
                "NAV WP / DIST",
                if v.nav_valid { format!("{} / {:.0} m", v.mission_seq, v.wp_dist) } else { "--".into() },
                MAGENTA,
            ))
            .child(row(
                "XTRACK / ALT ERR",
                if v.nav_valid { format!("{:+.1} / {:+.1} m", v.xtrack, v.alt_error) } else { "--".into() },
                MAGENTA,
            ))
            .child(row(
                "MISSION",
                if v.mission_status.is_empty() { "--".into() } else { v.mission_status.clone() },
                CYAN,
            ))
            .child(row(
                "EKF VEL/POS/HGT/MAG",
                format!("{:.2} {:.2} {:.2} {:.2}", v.ekf_vel_var, v.ekf_pos_var, v.ekf_hgt_var, v.ekf_mag_var),
                GREEN,
            ))
            .child(row(
                "RC 1-4  5/8",
                format!("{} {} {} {}  {}/{}", v.rc[0], v.rc[1], v.rc[2], v.rc[3], v.rc[4], v.rc[7]),
                GREY,
            ))
            .child(row(
                "SERVO 1-5",
                format!("{} {} {} {} {}", v.servo[0], v.servo[1], v.servo[2], v.servo[3], v.servo[4]),
                GREY,
            ))
    }

    /// The autopilot's own output, the last lines, read from its file once a second.
    fn refresh_proc_log(&mut self) {
        if self.proc_log.0.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.proc_log =
            (Some(Instant::now()), debug::tail_lines(&crate::station::logs_dir().join("sitl/arduplane.log"), 8));
    }

    /// What a debugger wants: the autopilot's state, its own output, and everything that
    /// happened on the link, newest first.
    fn debug_panel(&self, v: &Vehicle) -> Div {
        let level_colour = |l: Level| match l {
            Level::Error => RED,
            Level::Warn => AMBER,
            Level::Info => WHITE,
            Level::Debug | Level::Trace => GREY,
        };
        let state = debug::state_report(v).into_iter().map(|(k, val)| {
            div().flex().gap_2().child(div().flex_none().w(px(84.0)).text_color(c(GREY)).child(k.to_uppercase())).child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_color(c(if val.contains("LOST") || val.starts_with("no (") || val.starts_with("unhealthy") {
                        AMBER
                    } else {
                        GREEN
                    }))
                    .child(val),
            )
        });
        let process = self
            .proc_log
            .1
            .iter()
            .map(|l| div().text_color(c(GREY)).whitespace_nowrap().overflow_hidden().child(l.clone()));
        let events = v.debug.tail(80).into_iter().rev().map(|e| {
            let secs = e.t as u64;
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().whitespace_nowrap().text_color(c(DARK)).child(format!(
                    "{:02}:{:02}.{:01}",
                    secs / 60,
                    secs % 60,
                    ((e.t.fract()) * 10.0) as u8
                )))
                .child(
                    div()
                        .flex_none()
                        .w(px(38.0))
                        .text_color(c(if e.source == Source::Fc { GREY } else { CYAN }))
                        .child(e.source.tag()),
                )
                .child(div().flex_1().min_w(px(0.0)).text_color(c(level_colour(e.level))).child(e.text.clone()))
        });
        let heading = |t: &'static str| div().pt_1().text_color(c(MAGENTA)).child(t);
        div()
            .flex()
            .flex_col()
            .p_2()
            .gap(px(1.0))
            .children(state)
            .child(heading("ARDUPLANE OUTPUT"))
            .children(process)
            .child(heading("EVENTS"))
            .children(events)
    }

    fn messages(&self, v: &Vehicle) -> Div {
        let lines = v.log.iter().rev().take(60).map(|l| {
            let age = l.time.duration_since(v.start).as_secs();
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().whitespace_nowrap().text_color(c(DARK)).child(format!(
                    "T+{:02}:{:02}:{:02}",
                    age / 3600,
                    (age / 60) % 60,
                    age % 60
                )))
                .child(
                    div()
                        .flex_none()
                        .w(px(26.0))
                        .text_color(c(if l.source == "GCS" { CYAN } else { GREY }))
                        .child(l.source),
                )
                // Long messages (e.g. ArduPilot's pre-arm reasons) wrap rather than clip.
                .child(div().flex_1().min_w(px(0.0)).text_color(c(severity(l.severity))).child(l.text.clone()))
        });
        div().flex().flex_col().p_2().gap(px(1.0)).children(lines)
    }
}

impl Render for Gcs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = f32::from(window.viewport_size().width) < COMPACT_WIDTH;
        let right_w = if compact { 340.0 } else { 400.0 };
        let vehicle = self.vehicle.clone();
        let v = vehicle.lock().unwrap();
        let agl = self.agl(&v);
        if self.map.follow && v.have_position {
            self.map.centre = self.origin.to_ne(v.lat, v.lon);
        }
        let hud_data = hud::HudData::from(&v, agl);
        let mut map_data = MapData::from(&v, self.origin);
        if let Some(s) = &self.sensor {
            map_data.spi = s.spi(&v).map(|p| (p.north, p.east));
            map_data.track = s.track.lock().unwrap().map(|t| (t.0, t.1));
            if let Some(a) = s.frame.lock().unwrap().as_ref().and_then(|f| f.header.arena) {
                map_data.cones = a.cones();
            }
        }
        let history: Vec<_> = v.history.iter().copied().collect();
        let now = v.now();
        let top = self.top_bar(&v);
        let systems = self.systems(&v, agl);
        let ann = self.annunciators(&v, agl, (right_w - 30.0) / 3.0);
        let msgs = if self.debug_view {
            self.refresh_proc_log();
            self.debug_panel(&v)
        } else {
            self.messages(&v)
        };
        let command = self.command_panel(&v, cx);
        drop(v);
        let toolbar = self.map_toolbar(cx);
        let debug_toggle = button(
            "debug-toggle",
            if self.debug_view { "MESSAGES" } else { "DEBUG" },
            if self.debug_view { Style::Active } else { Style::Normal },
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.debug_view = !this.debug_view;
            cx.notify();
        }));

        let hud_canvas = canvas(
            |_, _, _| {},
            move |b, _, window, cx| {
                hud::paint(b, &hud_data, hud::HudOpts::default(), window, cx);
                round_corners(window, b, SCREEN_RADIUS - 2.0, c(0x000000));
            },
        )
        .size_full();
        let map_state = MapStateSnapshot::from(&self.map);
        let terrain = self.terrain.clone();
        let image = self.map_image.clone();
        let bounds_cell = self.map_bounds.clone();
        let map_canvas = canvas(
            |_, _, _| {},
            move |b, _, window, cx| {
                bounds_cell.set(b);
                let mut st = map_state.0;
                st.bounds = b;
                map::paint(b, &st, &map_data, terrain.as_deref(), image.as_ref(), window, cx);
                round_corners(window, b, SCREEN_RADIUS - 2.0, c(0x000000));
            },
        )
        .size_full();
        let charts_canvas = canvas(
            |_, _, _| {},
            move |b, _, window, cx| {
                charts::paint(b, &history, now, window, cx);
                round_corners(window, b, SCREEN_RADIUS - 2.0, c(0x000000));
            },
        )
        .size_full();

        let map_area = div()
            .id("map")
            .size_full()
            .child(map_canvas)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_mouse_down(ev, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev, _, cx| this.map_mouse_down(ev, cx)))
            .on_mouse_move(cx.listener(|this, ev, _, cx| this.map_mouse_move(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.map.drag = None))
            .on_scroll_wheel(cx.listener(|this, ev, _, cx| this.map_scroll(ev, cx)));

        let map_screen =
            screen("TACTICAL SITUATION", Some(toolbar.into_any_element()), map_area.into_any_element()).flex_1();
        let pfd = screen("PRIMARY FLIGHT DISPLAY", None, hud_canvas.into_any_element());
        let command = scrolling("VEHICLE COMMAND", None, command.into_any_element());
        let trends = screen("TELEMETRY TRENDS", None, charts_canvas.into_any_element());
        let messages_screen = scrolling(
            if self.debug_view { "DEBUG" } else { "MESSAGES" },
            Some(debug_toggle.into_any_element()),
            msgs.into_any_element(),
        )
        .flex_1();
        // The debug view takes the whole right column: the autopilot's state, its output and the
        // events on the link need the room, and the state it shows covers what SYSTEMS does.
        let right = if self.debug_view {
            div().flex().flex_col().gap(px(6.0)).w(px(right_w + 140.0)).flex_none().child(messages_screen)
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .w(px(right_w))
                .flex_none()
                .child(scrolling("CAUTION / WARNING", None, ann.into_any_element()))
                .child(scrolling("SYSTEMS", None, systems.into_any_element()))
                .child(messages_screen)
        };

        let body = if compact {
            // Map with command + trends under it; the PFD only if there's no sensor window
            // (whose HUD shows the same flight data).
            let main = div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .flex_1()
                .min_w(px(0.0))
                .children(self.sensor.is_none().then(|| pfd.h(relative(0.42))))
                .child(map_screen)
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .h(px(if self.vtol { 230.0 } else { 170.0 }))
                        .child(command.w(relative(0.5)))
                        .child(trends.flex_1().min_w(px(0.0))),
                );
            div().flex().flex_1().min_h(px(0.0)).gap(px(6.0)).child(main).child(right)
        } else {
            div()
                .flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(6.0))
                // Left station: flight display and vehicle commands.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .w(relative(0.36))
                        .child(pfd.flex_1())
                        .child(command.h(px(150.0))),
                )
                // Centre: tactical map and trends.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .flex_1()
                        .min_w(px(0.0))
                        .child(map_screen)
                        .child(trends.h(px(170.0))),
                )
                // Right: health and messages.
                .child(right)
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .p(px(GAP))
            .bg(c(CONSOLE))
            .font_family(FONT)
            .text_size(px(12.0))
            .text_color(c(WHITE))
            .child(top)
            .child(body)
    }
}

/// A click on ARM: the first asks for confirmation, a second within `ARM_CONFIRM` sends the arm. Decided from
/// the state when the click lands, not from what the screen showed when it was last drawn: a
/// console that is slow to redraw (or a click that comes at once) must not turn two clicks into
/// two first clicks.
const ARM_CONFIRM: Duration = Duration::from_secs(3);

fn arm_click(confirm: &mut Option<Instant>, now: Instant) -> bool {
    if confirm.is_some_and(|t| now.saturating_duration_since(t) < ARM_CONFIRM) {
        *confirm = None;
        true
    } else {
        *confirm = Some(now);
        false
    }
}

/// A click on DISARM: the first asks the autopilot to disarm; if it is still armed a second, between
/// 1 and 8 s later (it refused, thinking it is flying), forces it. Decided when the click lands.
#[derive(Debug, PartialEq)]
enum Disarm {
    Ask,
    Force,
}

fn disarm_click(at: &mut Option<Instant>, now: Instant) -> Disarm {
    let force = at.is_some_and(|t| (1..8).contains(&now.saturating_duration_since(t).as_secs()));
    if force {
        *at = None;
        Disarm::Force
    } else {
        *at = Some(now);
        Disarm::Ask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disarm_that_was_refused_is_forced_by_a_second_click_a_little_later() {
        let now = Instant::now();
        let mut at = None;
        assert_eq!(disarm_click(&mut at, now), Disarm::Ask);
        assert_eq!(
            disarm_click(&mut at, now + Duration::from_millis(300)),
            Disarm::Ask,
            "too soon: it is the same click"
        );
        assert_eq!(disarm_click(&mut at, now + Duration::from_secs(3)), Disarm::Force);
        assert!(at.is_none());
        assert_eq!(disarm_click(&mut at, now + Duration::from_secs(20)), Disarm::Ask);
        assert_eq!(disarm_click(&mut at, now + Duration::from_secs(40)), Disarm::Ask, "too late");
    }

    #[test]
    fn a_second_click_arms_whatever_was_drawn_between() {
        let now = Instant::now();
        let mut confirm = None;
        assert!(!arm_click(&mut confirm, now), "the first click only asks");
        assert!(confirm.is_some());
        assert!(arm_click(&mut confirm, now + Duration::from_millis(5)), "the second, at once, arms");
        assert!(confirm.is_none(), "and the confirmation is spent");
        assert!(!arm_click(&mut confirm, now + Duration::from_millis(10)), "a third starts again");
    }

    #[test]
    fn a_confirmation_that_has_waited_too_long_is_gone() {
        let now = Instant::now();
        let mut confirm = None;
        assert!(!arm_click(&mut confirm, now));
        assert!(!arm_click(&mut confirm, now + Duration::from_secs(4)), "after 3 s it asks again");
        assert!(arm_click(&mut confirm, now + Duration::from_secs(5)));
    }
}

fn relative(f: f32) -> gpui::DefiniteLength {
    gpui::relative(f)
}

/// MapState minus the parts that only matter to event handlers.
struct MapStateSnapshot(MapState);

impl MapStateSnapshot {
    fn from(m: &MapState) -> Self {
        MapStateSnapshot(MapState {
            centre: m.centre,
            mpp: m.mpp,
            follow: m.follow,
            tool: m.tool,
            plan: m.plan.clone(),
            plan_alt: m.plan_alt,
            measure: m.measure.clone(),
            survey: m.survey.clone(),
            checked: m.checked,
            drag: None,
            cursor: m.cursor,
            bounds: m.bounds,
        })
    }
}

/// A bezelled monitor with a title strip.
fn screen(title: &'static str, tools: Option<AnyElement>, body: AnyElement) -> Div {
    screen_with(title, tools, body, false)
}

/// A screen whose body scrolls, for text and buttons that may not fit (not for the drawn
/// instruments, which fit themselves to the room and take the mouse wheel).
fn scrolling(title: &'static str, tools: Option<AnyElement>, body: AnyElement) -> Div {
    screen_with(title, tools, body, true)
}

fn screen_with(title: &'static str, tools: Option<AnyElement>, body: AnyElement, scroll: bool) -> Div {
    let area = div()
        .id(title)
        .flex_1()
        .min_h(px(0.0))
        .bg(c(SCREEN))
        .border_2()
        .border_color(c(0x000000))
        .rounded(px(SCREEN_RADIUS));
    let area = if scroll { area.overflow_scroll() } else { area.overflow_hidden() };
    div()
        .flex()
        .flex_col()
        .min_h(px(0.0))
        .bg(c(BEZEL))
        .border_1()
        .border_color(c(BEZEL_EDGE))
        .rounded(px(PANEL_RADIUS))
        .p(px(BEZEL_PAD))
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_1()
                .child(div().flex_none().text_size(px(11.0)).text_color(c(GREY)).child(title))
                .children(tools),
        )
        .child(area.child(body))
}

fn label(s: &'static str) -> Div {
    div().text_color(c(GREY)).min_w(px(56.0)).child(s)
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
    let id: SharedString = id.into();
    let recorded = id.clone();
    div()
        .id(id)
        .relative()
        .px_2()
        .py(px(3.0))
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

#[derive(Clone, Copy, PartialEq)]
enum Ann {
    Off,
    Ok,
    Caution,
    Warn,
}

fn annunciator(name: &'static str, a: Ann, flash: bool, w: f32) -> Div {
    let (bg, fg, border) = match a {
        Ann::Off => (LAMP_OFF, DARK, LAMP_OFF_EDGE),
        Ann::Ok => (LAMP_OK, GREEN, GREEN_DIM),
        Ann::Caution => (AMBER, 0x000000, AMBER),
        Ann::Warn if flash => (RED, 0xffffff, RED),
        Ann::Warn => (LAMP_WARN, 0xffffff, RED),
    };
    div()
        .w(px(w))
        .h(px(30.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(c(bg))
        .text_color(c(fg))
        .border_1()
        .border_color(c(border))
        .rounded(px(CONTROL_RADIUS))
        .child(name)
}
