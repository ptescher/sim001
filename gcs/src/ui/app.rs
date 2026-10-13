//! Root view: the console layout and its panels.

use super::map::{MapData, MapState, Tool};
use super::draw::round_corners;
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
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

thread_local! {
    /// Where each button was last painted, by id, so `--expect` can click it the way a
    /// person would. Written while painting, read between frames; both on the UI thread.
    static BUTTONS: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::new(HashMap::new());
}

/// Where the button with this id is on screen now, if it is.
pub fn button_bounds(id: &str) -> Option<Bounds<Pixels>> {
    BUTTONS.with(|b| b.borrow().get(id).copied())
}

/// A VTOL's own modes (ArduPlane QuadPlane), shown when the plane has lift rotors.
const Q_MODE_BUTTONS: [&str; 5] = ["QSTABILIZE", "QHOVER", "QLOITER", "QLAND", "QRTL"];

const MODE_BUTTONS: [&str; 10] = ["MANUAL", "FBWA", "FBWB", "CRUISE", "AUTO", "RTL", "LOITER", "CIRCLE", "GUIDED", "TAKEOFF"];

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
            (format!("LINK {}  ·  {}  ·  SYS {}", v.link.address, v.link.status, v.sysid), GREEN)
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
                    .child(div().text_color(c(WHITE)).child("T1-S RANGER  GROUND CONTROL STATION"))
                    .child(div().text_color(c(GREY)).child("PILOT / PAYLOAD CONSOLE")),
            )
            .child(div().text_color(c(link_col)).child(link_txt))
            .child(div().text_color(c(WHITE)).child(format!("{hh:02}:{mm:02}:{ss:02}Z")))
    }

    fn command_panel(&mut self, v: &Vehicle, cx: &mut Context<Self>) -> Div {
        let confirming = self.arm_confirm.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
        if !confirming {
            self.arm_confirm = None;
        }
        let alive = v.link.alive();
        let armed = v.armed;
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
            button("disarm", if force { "FORCE DISARM" } else { "DISARM" }, Style::Danger).on_click(cx.listener(move |this, _, _, _| {
                if force {
                    this.disarm_at = None;
                    this.send(Cmd::ForceDisarm);
                } else {
                    this.disarm_at = Some(Instant::now());
                    this.send(Cmd::Arm(false));
                }
            }))
        } else if confirming {
            button("arm", "CONFIRM ARM", Style::Danger).on_click(cx.listener(|this, _, _, _| {
                this.arm_confirm = None;
                this.send(Cmd::Arm(true));
            }))
        } else {
            let (text, style) = if not_ready_reason.is_some() { ("ARM: NOT READY", Style::Normal) } else { ("ARM", Style::Caution) };
            button("arm", text, style).on_click(cx.listener(|this, _, _, _| this.arm_confirm = Some(Instant::now())))
        };
        // A VTOL takes off straight up, to the plan altitude.
        let takeoff = self.vtol.then(|| {
            button("vtol-tkof", "VTOL TKOF", Style::Normal).on_click(cx.listener(move |this, _, _, _| {
                if armed {
                    this.send(Cmd::Takeoff { alt: this.map.plan_alt });
                } else {
                    this.log(4, "Arm the vehicle first: VTOL TKOF lifts an armed VTOL straight up");
                }
            }))
        });
        let rebooting = self.reboot_confirm.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
        let reboot = button("reboot", if rebooting { "CONFIRM REBOOT" } else { "REBOOT FC" }, if rebooting { Style::Danger } else { Style::Normal })
            .on_click(cx.listener(move |this, _, _, _| {
                if rebooting {
                    this.reboot_confirm = None;
                    this.send(Cmd::Reboot);
                } else {
                    this.reboot_confirm = Some(Instant::now());
                }
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
        let q_modes: Vec<_> = if self.vtol { Q_MODE_BUTTONS.iter().map(|&m| mode_button(m, cx)).collect() } else { vec![] };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .size_full()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(label("VEHICLE"))
                    .child(arm)
                    .child(reboot)
                    .children(takeoff),
            )
            // The reason the vehicle isn't ready (or a confirmation) has a line of its own, so
            // the buttons keep their places.
            .child(hint)
            .child(div().flex().flex_wrap().gap_1().items_center().child(label("MODE")).children(modes))
            .children((!q_modes.is_empty()).then(|| div().flex().flex_wrap().gap_1().items_center().child(label("VTOL")).children(q_modes)))
            .children(direct.then(|| {
                div()
                    .text_color(c(CYAN))
                    .child("DIRECT: the sim's sticks are flying (J / D-pad down in the sim). ArduPilot is only watching.")
            }))
    }

    fn map_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let st = &self.map;
        let tool_btn = |id: &'static str, text: &'static str, tool: Tool| {
            button(id, text, if st.tool == tool { Style::Active } else { Style::Normal })
                .on_click(cx.listener(move |this, _, _, _| this.map.tool = if this.map.tool == tool { Tool::Pan } else { tool }))
        };
        let plan_len = st.plan.len();
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .items_center()
            .child(
                button("follow", "FOLLOW", if st.follow { Style::Active } else { Style::Normal })
                    .on_click(cx.listener(|this, _, _, _| this.map.follow = !this.map.follow)),
            )
            .child(tool_btn("goto", "GOTO", Tool::Goto))
            .children(self.sensor.is_some().then(|| tool_btn("look", "LOOK", Tool::Look)))
            .child(tool_btn("plan", "PLAN", Tool::Plan))
            .child(button("alt-dn", "ALT-", Style::Normal).on_click(cx.listener(|this, _, _, _| this.map.plan_alt = (this.map.plan_alt - 10.0).max(10.0))))
            .child(div().text_color(c(AMBER)).min_w(px(44.0)).child(format!("{:.0} m", st.plan_alt)))
            .child(button("alt-up", "ALT+", Style::Normal).on_click(cx.listener(|this, _, _, _| this.map.plan_alt = (this.map.plan_alt + 10.0).min(300.0))))
            .child(button("tkof", "+TKOF", Style::Normal).on_click(cx.listener(|this, _, _, _| this.add_takeoff())))
            .child(button("rtl", "+RTL", Style::Normal).on_click(cx.listener(|this, _, _, _| {
                this.map.plan.push(Waypoint { kind: WpKind::Rtl, lat: 0.0, lon: 0.0, alt: 0.0 });
            })))
            .child(button("clear", "CLEAR", Style::Normal).on_click(cx.listener(|this, _, _, _| this.map.plan.clear())))
            .child(
                button("upload", format!("UPLOAD {plan_len}"), if plan_len > 0 { Style::Caution } else { Style::Normal })
                    .on_click(cx.listener(|this, _, _, _| {
                        if this.map.plan.is_empty() {
                            this.log(4, "Nothing to upload: use PLAN to click waypoints on the map");
                        } else {
                            this.send(Cmd::Upload(this.map.plan.clone()));
                            this.map.tool = Tool::Pan;
                        }
                    })),
            )
            .child(button("download", "DOWNLOAD", Style::Normal).on_click(cx.listener(|this, _, _, _| this.send(Cmd::Download))))
            .child(button("edit", "EDIT", Style::Normal).on_click(cx.listener(|this, _, _, _| this.edit_vehicle_mission())))
            .child(button("restart", "WP1", Style::Normal).on_click(cx.listener(|this, _, _, _| this.send(Cmd::SetCurrentWp(1)))))
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
        let flash = (v.now() * 2.0) as u32 % 2 == 0;
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
            ("LINK", if alive { Ok } else if v.link.connected { Caution } else { Warn }),
            ("GPS", if !alive { Off } else if v.gps_fix >= 3 { Ok } else if v.gps_fix == 2 { Caution } else { Warn }),
            ("EKF", if !alive { Off } else if ekf_var > 0.8 { Warn } else if !ekf_good || ekf_var > 0.5 { Caution } else { Ok }),
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
            ("ARMED", if !alive { Off } else if v.armed { Caution } else { Ok }),
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
            ("STALL", if flying && v.airspeed < 9.5 { Warn } else if alive { Ok } else { Off }),
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
            ("XTRACK", if alive && v.mode() == "AUTO" && v.xtrack.abs() > 25.0 { Caution } else if alive { Ok } else { Off }),
        ];
        div().flex().flex_wrap().gap_1().p_2().children(tiles.into_iter().map(|(name, a)| annunciator(name, a, flash, tile_w)))
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
                format!("{}S {:.2} V {:.1} A {} mAh{}", v.batt_cells, v.batt_v, v.batt_a, v.batt_mah, if v.batt_pct >= 0 { format!(" {}%", v.batt_pct) } else { String::new() }),
                AMBER,
            ))
            .child(row("GPS", format!("{fix} {} SV HDOP {:.1}", v.sats, v.hdop), GREEN))
            .child(row("WIND (EST)", if v.wind_valid { format!("{:03.0}° {:.1} m/s", v.wind_dir, v.wind_speed) } else { "--".into() }, CYAN))
            .child(row("HOME", home.map_or("--".into(), |(d, b)| format!("{d:.0} m @ {b:03.0}°")), AMBER))
            .child(row("NAV WP / DIST", if v.nav_valid { format!("{} / {:.0} m", v.mission_seq, v.wp_dist) } else { "--".into() }, MAGENTA))
            .child(row("XTRACK / ALT ERR", if v.nav_valid { format!("{:+.1} / {:+.1} m", v.xtrack, v.alt_error) } else { "--".into() }, MAGENTA))
            .child(row("MISSION", if v.mission_status.is_empty() { "--".into() } else { v.mission_status.clone() }, CYAN))
            .child(row("EKF VEL/POS/HGT/MAG", format!("{:.2} {:.2} {:.2} {:.2}", v.ekf_vel_var, v.ekf_pos_var, v.ekf_hgt_var, v.ekf_mag_var), GREEN))
            .child(row("RC 1-4  5/8", format!("{} {} {} {}  {}/{}", v.rc[0], v.rc[1], v.rc[2], v.rc[3], v.rc[4], v.rc[7]), GREY))
            .child(row("SERVO 1-5", format!("{} {} {} {} {}", v.servo[0], v.servo[1], v.servo[2], v.servo[3], v.servo[4]), GREY))
    }

    fn messages(&self, v: &Vehicle) -> Div {
        let lines = v.log.iter().rev().take(60).map(|l| {
            let age = l.time.duration_since(v.start).as_secs();
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().whitespace_nowrap().text_color(c(DARK)).child(format!("T+{:02}:{:02}:{:02}", age / 3600, (age / 60) % 60, age % 60)))
                .child(div().flex_none().w(px(26.0)).text_color(c(if l.source == "GCS" { CYAN } else { GREY })).child(l.source))
                // Long messages (e.g. ArduPilot's pre-arm reasons) wrap rather than clip.
                .child(div().flex_1().min_w(px(0.0)).text_color(c(severity(l.severity))).child(l.text.clone()))
        });
        div().flex().flex_col().p_2().gap(px(1.0)).overflow_hidden().children(lines)
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
        }
        let history: Vec<_> = v.history.iter().copied().collect();
        let now = v.now();
        let top = self.top_bar(&v);
        let systems = self.systems(&v, agl);
        let ann = self.annunciators(&v, agl, (right_w - 30.0) / 3.0);
        let msgs = self.messages(&v);
        let command = self.command_panel(&v, cx);
        drop(v);
        let toolbar = self.map_toolbar(cx);

        let hud_canvas = canvas(|_, _, _| {}, move |b, _, window, cx| {
            hud::paint(b, &hud_data, hud::HudOpts::default(), window, cx);
            round_corners(window, b, SCREEN_RADIUS - 2.0, c(0x000000));
        })
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
        let charts_canvas = canvas(|_, _, _| {}, move |b, _, window, cx| {
            charts::paint(b, &history, now, window, cx);
            round_corners(window, b, SCREEN_RADIUS - 2.0, c(0x000000));
        })
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

        let map_screen = screen("TACTICAL SITUATION", Some(toolbar.into_any_element()), map_area.into_any_element()).flex_1();
        let pfd = screen("PRIMARY FLIGHT DISPLAY", None, hud_canvas.into_any_element());
        let command = screen("VEHICLE COMMAND", None, command.into_any_element());
        let trends = screen("TELEMETRY TRENDS", None, charts_canvas.into_any_element());
        let right = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .w(px(right_w))
            .flex_none()
            .child(screen("CAUTION / WARNING", None, ann.into_any_element()))
            .child(screen("SYSTEMS", None, systems.into_any_element()))
            .child(screen("MESSAGES", None, msgs.into_any_element()).flex_1());

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
                .child(div().flex().gap(px(6.0)).h(px(if self.vtol { 230.0 } else { 170.0 })).child(command.w(relative(0.5))).child(trends.flex_1().min_w(px(0.0))));
            div().flex().flex_1().min_h(px(0.0)).gap(px(6.0)).child(main).child(right)
        } else {
            div()
                .flex()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(6.0))
                // Left station: flight display and vehicle commands.
                .child(div().flex().flex_col().gap(px(6.0)).w(relative(0.36)).child(pfd.flex_1()).child(command.h(px(150.0))))
                // Centre: tactical map and trends.
                .child(div().flex().flex_col().gap(px(6.0)).flex_1().min_w(px(0.0)).child(map_screen).child(trends.h(px(170.0))))
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
            drag: None,
            cursor: m.cursor,
            bounds: m.bounds,
        })
    }
}

/// A bezelled monitor with a title strip.
fn screen(title: &'static str, tools: Option<AnyElement>, body: AnyElement) -> Div {
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
                .child(div().text_size(px(11.0)).text_color(c(GREY)).child(title))
                .children(tools),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.0))
                .bg(c(SCREEN))
                .border_2()
                .border_color(c(0x000000))
                .rounded(px(SCREEN_RADIUS))
                .overflow_hidden()
                .child(body),
        )
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
            canvas(
                move |b, _, _| BUTTONS.with(|m| m.borrow_mut().insert(recorded, b)),
                |_, _, _, _| {},
            )
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
