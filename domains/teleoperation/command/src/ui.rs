//! The commander's window: the top bar (the seat, the clock, the scores), the map of the team
//! (the field, its fog, the towers, the drones) and the list of drones with their orders.
//!
//! The controls are those of a real-time strategy game. Left-click a drone (on the map or in the
//! list) to select it, or drag a box round several; Shift adds to the selection (and takes a
//! drone out of it again), Ctrl/Cmd+A selects all, the number keys select one drone, Esc lets go.
//! Right-click a tower (take it) or anywhere else on the field (go there and circle) to order
//! the selected drones. HOLD and HOME (or H and B) act on the selected drones; LOW (L) makes them
//! fly along the lanes, below the treetops; CAMERAS shows the camera of each drone.

use crate::cameras::Cameras;
use crate::draw::*;
use crate::ground::Ground;
use crate::seat::{Seat, Status};
use crate::theme::*;
use glam::Vec2;
use gpui::{
    App, Bounds, ContentMask, Context, Corners, Div, FocusHandle, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, RenderImage, SharedString, Stateful, Window, canvas,
    div, prelude::*, px,
};
use recreation_towers::map::{LANE_HALF_WIDTH, Map};
use recreation_towers::seat::Join;
use recreation_towers::{Class, Order, Outcome, Phase, Rules, Team, TeamView};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

thread_local! {
    /// Where each button was last painted, by id, so `--expect` can click it the way a person
    /// would. Written while painting, read between frames; both on the UI thread.
    static BUTTONS: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::new(HashMap::new());
}

/// Where the button with this id is on screen now, if it is.
pub fn button_bounds(id: &str) -> Option<Bounds<Pixels>> {
    BUTTONS.with(|b| b.borrow().get(id).copied())
}

/// A drone on the map is clicked when the click is this near it (screen pixels).
const PICK: f32 = 14.0;

/// A box being dragged on the map: where the button went down, where the mouse is now (screen
/// pixels), and whether Shift was held.
type Drag = ((f32, f32), (f32, f32), bool);

/// A left button press that moves less than this (screen pixels) is a click, not a box.
const DRAG: f32 = 4.0;

/// The field on the map: the whole field fits, north up.
#[derive(Clone, Copy)]
pub struct MapView {
    centre: (f32, f32),
    /// Pixels per metre.
    scale: f32,
}

impl MapView {
    pub fn new(b: Bounds<Pixels>, rules: &Rules) -> MapView {
        let (x, y, w, h) = rect(b);
        let scale = 0.5 * w.min(h) / (rules.field_radius * 1.08);
        MapView { centre: (x + w * 0.5, y + h * 0.5), scale }
    }

    pub fn to_screen(self, at: Vec2) -> (f32, f32) {
        (self.centre.0 + at.y * self.scale, self.centre.1 - at.x * self.scale)
    }

    pub fn to_field(self, sx: f32, sy: f32) -> Vec2 {
        Vec2::new((self.centre.1 - sy) / self.scale, (sx - self.centre.0) / self.scale)
    }
}

pub struct CommanderView {
    pub seat: Arc<Seat>,
    /// The selected drones (their numbers in the team), in order.
    pub selected: Vec<usize>,
    /// A box being dragged on the map: where the button went down, where the mouse is now
    /// (screen pixels), and whether Shift was held.
    drag: Option<Drag>,
    pub map_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The valley (lanes, lake, clearings), worked out from the game's rules once they arrive.
    valley: Option<Map>,
    /// The picture of the valley under the map, if it has been exported.
    ground: Option<Arc<Ground>>,
    /// The pictures from the team's drones, and the one of each that the window has on its atlas.
    pub cameras: Arc<Cameras>,
    shown: Vec<Option<Arc<RenderImage>>>,
    focus: FocusHandle,
}

impl CommanderView {
    pub fn new(
        seat: Arc<Seat>,
        cameras: Arc<Cameras>,
        ground: Option<Arc<Ground>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CommanderView {
        // The game arrives on another thread; repaint at about 30 Hz.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus);
        CommanderView {
            seat,
            selected: vec![],
            drag: None,
            map_bounds: Rc::new(Cell::new(Bounds::default())),
            valley: None,
            ground,
            shown: vec![None; cameras.len()],
            cameras,
            focus,
        }
    }

    /// How many drones the team has.
    fn drone_count(&self) -> usize {
        self.seat.state().rules.map_or(0, |r| r.drones)
    }

    /// The seat commands its team (it does not only watch).
    fn commands(&self) -> bool {
        matches!(self.seat.state().status, Status::Seated(Join::Command(_)))
    }

    /// Selects drone `k` alone, or none.
    pub fn select(&mut self, k: Option<usize>) {
        self.selected = k.into_iter().collect();
    }

    /// Selects `ks`: instead of the selection, or (with `add`) added to it, or (with `add` and
    /// all of them in it already) taken out of it.
    pub fn select_many(&mut self, ks: &[usize], add: bool) {
        if !add {
            self.selected = ks.to_vec();
        } else if !ks.is_empty() && ks.iter().all(|k| self.selected.contains(k)) {
            self.selected.retain(|k| !ks.contains(k));
        } else {
            let fresh: Vec<usize> = ks.iter().copied().filter(|k| !self.selected.contains(k)).collect();
            self.selected.extend(fresh);
        }
        self.selected.sort_unstable();
    }

    /// Whether drone `k`'s order is flown low now, as the server has it.
    fn low_of(&self, k: usize) -> bool {
        self.seat.state().view.as_ref().and_then(|v| v.drones.get(k)).is_some_and(|d| d.low)
    }

    /// Gives each selected drone `order`, flown as that drone flies now (low or high).
    pub fn give(&mut self, order: Order) {
        if !self.commands() {
            return;
        }
        for &k in &self.selected {
            let low = self.low_of(k);
            self.seat.order(k, order, low);
        }
    }

    /// LOW: the selected drones fly their orders low, along the lanes, or (when all do) high.
    pub fn toggle_low(&mut self) {
        if !self.commands() || self.selected.is_empty() {
            return;
        }
        let all_low = self.selected.iter().all(|&k| self.low_of(k));
        for &k in &self.selected {
            let order = {
                let s = self.seat.state();
                s.view.as_ref().and_then(|v| v.drones.get(k)).and_then(|d| d.order).unwrap_or(Order::Home)
            };
            self.seat.order(k, order, !all_low);
        }
    }

    /// The drones near `at` (field metres, within `pick`), nearest first.
    fn drones_at(&self, at: Vec2, pick: f32) -> Vec<usize> {
        let s = self.seat.state();
        let Some(view) = &s.view else { return vec![] };
        let mut near: Vec<_> = view.drones.iter().filter(|d| d.at.distance(at) <= pick).collect();
        near.sort_by(|a, b| a.at.distance(at).total_cmp(&b.at.distance(at)));
        near.iter().map(|d| d.k).collect()
    }

    /// A left click on the field at `at`: selects the drone there (the nearest), or with nothing
    /// there lets go of the selection. `add` (Shift) keeps the selection and toggles the drone.
    pub fn click_field(&mut self, at: Vec2, pick: f32, add: bool) {
        match self.drones_at(at, pick).first() {
            Some(&k) => self.select_many(&[k], add),
            None if !add => self.selected.clear(),
            None => {}
        }
    }

    /// A box on the field with corners `a` and `b`: selects the drones inside it.
    pub fn box_select(&mut self, a: Vec2, b: Vec2, add: bool) {
        let (lo, hi) = (a.min(b), a.max(b));
        let inside: Vec<usize> = {
            let s = self.seat.state();
            let Some(view) = &s.view else { return };
            view.drones.iter().filter(|d| d.at.cmpge(lo).all() && d.at.cmple(hi).all()).map(|d| d.k).collect()
        };
        if inside.is_empty() && !add {
            self.selected.clear();
        } else if add {
            let fresh: Vec<usize> = inside.into_iter().filter(|k| !self.selected.contains(k)).collect();
            self.select_many(&fresh, true);
        } else {
            self.select_many(&inside, false);
        }
    }

    /// A right click on the field at `at`: the selected drones take the tower there or go there.
    pub fn order_field(&mut self, at: Vec2) {
        let (view, rules) = {
            let s = self.seat.state();
            (s.view.clone(), s.rules)
        };
        let (Some(view), Some(rules)) = (view, rules) else { return };
        let tower = view.towers.iter().position(|t| t.at.distance(at) <= rules.circle_radius);
        let order = match tower {
            Some(t) => Order::Take(t),
            None => Order::Goto(at.clamp_length_max(rules.field_radius - 40.0)),
        };
        self.give(order);
    }

    fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            k @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                let n = k.parse::<usize>().map_or(0, |n| n - 1);
                if n < self.drone_count() {
                    self.select_many(&[n], ev.keystroke.modifiers.shift)
                }
            }
            "a" if ev.keystroke.modifiers.secondary() || ev.keystroke.modifiers.control => {
                let all: Vec<usize> = (0..self.drone_count()).collect();
                self.select_many(&all, false)
            }
            "h" => self.give(Order::Hold),
            "b" => self.give(Order::Home),
            "l" => self.toggle_low(),
            "escape" => self.selected.clear(),
            _ => return,
        }
        cx.notify();
    }

    fn map_field(&self, p: gpui::Point<Pixels>) -> Option<(MapView, Vec2)> {
        let rules = self.seat.state().rules?;
        let map = MapView::new(self.map_bounds.get(), &rules);
        Some((map, map.to_field(f(p.x), f(p.y))))
    }

    fn map_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let p = (f(ev.position.x), f(ev.position.y));
        match ev.button {
            MouseButton::Left => self.drag = Some((p, p, ev.modifiers.shift)),
            MouseButton::Right => {
                if let Some((_, at)) = self.map_field(ev.position) {
                    self.order_field(at);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn map_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some((_, to, _)) = &mut self.drag {
            if ev.dragging() {
                *to = (f(ev.position.x), f(ev.position.y));
            } else {
                self.drag = None;
            }
            cx.notify();
        }
    }

    /// The button went up: a click selects the drone under it; a drag selects the drones in its box.
    fn map_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some((from, _, add)) = self.drag.take() else { return };
        let to = (f(ev.position.x), f(ev.position.y));
        if let Some((map, at)) = self.map_field(ev.position) {
            if (to.0 - from.0).hypot(to.1 - from.1) < DRAG {
                self.click_field(at, PICK / map.scale, add);
            } else {
                self.box_select(map.to_field(from.0, from.1), at, add);
            }
        }
        cx.notify();
    }
}

/// What an order is, in the list of drones.
pub fn order_text(order: Option<Order>, view: &TeamView) -> String {
    order_name(order, view)
}

fn order_name(order: Option<Order>, view: &TeamView) -> String {
    match order {
        None => "NO ORDER".into(),
        Some(Order::Take(t)) => format!("TAKE {}", view.towers.get(t).map_or("?".into(), |t| t.name.to_uppercase())),
        Some(Order::Goto(_)) => "GO TO A POINT".into(),
        Some(Order::Hold) => "HOLD".into(),
        Some(Order::Home) => "HOME".into(),
    }
}

fn clock(secs: f32) -> String {
    let s = secs.max(0.0).round() as u32;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn team_name(t: Team) -> &'static str {
    match t {
        Team::Blue => "BLUE",
        Team::Red => "RED",
    }
}

fn button(id: impl Into<SharedString>, text: impl Into<SharedString>, active: bool) -> Stateful<Div> {
    let (bg, fg, border) = if active { (ACTIVE_FILL, GREEN, GREEN_DIM) } else { (CONTROL, WHITE, BEZEL_EDGE) };
    let id: SharedString = id.into();
    let recorded = id.clone();
    div()
        .id(id)
        .relative()
        .px_2()
        .py(px(4.0))
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

/// A titled screen in a bezel, as on the console.
fn screen(title: &'static str, body: impl IntoElement) -> Div {
    screen_with(title, body, false)
}

/// A screen whose body scrolls, for text and buttons that may not fit the window (not the map,
/// which fits itself to the room and takes the mouse).
fn scrolling(title: &'static str, body: impl IntoElement) -> Div {
    screen_with(title, body, true)
}

fn screen_with(title: &'static str, body: impl IntoElement, scroll: bool) -> Div {
    let area = div().id(title).flex_1().min_h(px(0.0)).bg(c(SCREEN)).rounded(px(SCREEN_RADIUS));
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
        .child(div().px_2().bg(c(TITLE_BG)).text_color(c(GREY)).text_xs().child(title))
        .child(area.child(body))
}

impl CommanderView {
    /// The details of the first selected drone: its icon and basic stats.
    fn unit_panel(&self, view: &Option<TeamView>, rules: Option<&Rules>) -> Option<Div> {
        let (v, r) = (view.as_ref()?, rules?);
        let k = *self.selected.first()?;
        let d = v.drones.iter().find(|d| d.k == k)?;
        let class = r.class(k);
        let tint = theme_team(v.team);
        let state = match d.phase {
            Phase::Flying => "FLYING",
            Phase::Ground => "GROUND",
            Phase::Down => "DOWN",
        };
        let more = if self.selected.len() > 1 { format!("  (+{} more)", self.selected.len() - 1) } else { String::new() };
        let rows = [
            ("CLASS", class.name().to_uppercase()),
            ("WEIGHT", format!("{} on a tower", class.weight())),
            ("MASS", format!("{:.2} kg", class.mass_kg())),
            ("SPEED", format!("{:.0} m/s", d.vel.length())),
            ("ALTITUDE", format!("{:.0} m", d.alt)),
            ("STATE", format!("{state}{}", if d.low { " LOW" } else { "" })),
        ];
        let mut stats = div().flex().flex_col().flex_1().text_sm();
        stats = stats.child(
            div().text_color(c(tint)).child(format!("{}{}{more}", &team_name(v.team)[..1], k + 1)),
        );
        for (name, value) in rows {
            stats = stats.child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(div().w(px(70.0)).text_color(c(GREY)).child(name))
                    .child(div().text_color(c(WHITE)).child(value)),
            );
        }
        let icon = canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                let (x, y, w, h) = rect(b);
                fill_rect(window, x, y, w, h, c(0x000000));
                unit_icon(window, x + w * 0.5, y + h * 0.5, class, c(tint));
            },
        )
        .size_full();
        Some(
            div()
                .flex()
                .gap(px(8.0))
                .p_2()
                .child(div().w(px(84.0)).h(px(84.0)).flex_none().border_1().border_color(c(BEZEL_EDGE)).child(icon))
                .child(stats),
        )
    }

    /// The camera of each drone beside its name, and the selected drone's (else the first's) larger.
    fn cameras_panel(&mut self, window: &mut Window, view: &Option<TeamView>, cx: &mut Context<Self>) -> Div {
        // The newest pictures; the ones the window had are freed from its atlas.
        let images: Vec<Option<Arc<RenderImage>>> = (0..self.cameras.len()).map(|k| self.cameras.image(k)).collect();
        for (k, img) in images.iter().enumerate() {
            let same = match (&self.shown[k], img) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if !same && let Some(old) = std::mem::replace(&mut self.shown[k], img.clone()) {
                let _ = window.drop_image(old);
            }
        }
        let team = view.as_ref().map(|v| v.team);
        // A drone in the radio shadow of an enemy tower sends no picture.
        let linked = |k: usize| view.as_ref().and_then(|v| v.drones.get(k)).is_none_or(|d| d.linked);
        let mut row = div().flex().gap(px(4.0)).p_2();
        for (k, img) in images.iter().enumerate() {
            let label = team.map_or(String::new(), |t| format!("{}{}", &team_name(t)[..1], k + 1));
            let img = if linked(k) { img.clone() } else { None };
            row = row.child(
                camera_tile(format!("camera-{k}"), &label, img, !linked(k), 100.0, 75.0, self.selected.contains(&k))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(Some(k));
                        cx.notify();
                    })),
            );
        }
        let k = self.selected.first().copied().unwrap_or(0);
        let label = team.map_or(String::new(), |t| format!("{}{}", &team_name(t)[..1], k + 1));
        div().flex().flex_col().child(row).child(div().px_2().pb_2().child(camera_tile(
            "camera-selected".to_string(),
            &label,
            images.get(k).cloned().flatten().filter(|_| linked(k)),
            !linked(k),
            252.0,
            189.0,
            false,
        )))
    }
}

/// One drone's picture (or NO SIGNAL), `w` by `h` pixels, with its name over it.
fn camera_tile(
    id: String,
    label: &str,
    image: Option<Arc<RenderImage>>,
    no_link: bool,
    w: f32,
    h: f32,
    selected: bool,
) -> Stateful<Div> {
    let recorded: SharedString = id.clone().into();
    let label = label.to_string();
    div()
        .id(SharedString::from(id))
        .relative()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .border_1()
        .border_color(c(if selected { WHITE } else { BEZEL_EDGE }))
        .cursor_pointer()
        .child(
            canvas(
                move |b, _, _| BUTTONS.with(|m| m.borrow_mut().insert(recorded, b)),
                move |b, _, window, cx| {
                    let (x, y, w, h) = rect(b);
                    fill_rect(window, x, y, w, h, c(0x000000));
                    match &image {
                        Some(img) => {
                            let _ = window.paint_image(b, Corners::default(), img.clone(), 0, false);
                        }
                        None => {
                            let message = if no_link { "NO LINK" } else { "NO SIGNAL" };
                            text(window, cx, message, x + w * 0.5, y + h * 0.5, 10.0, c(GREY), Align::Center)
                        }
                    }
                    text(window, cx, &label, x + 6.0, y + 9.0, 10.0, c(WHITE), Align::Left);
                },
            )
            .absolute()
            .inset_0(),
        )
}

impl Render for CommanderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, rules, view) = {
            let s = self.seat.state();
            (s.status.clone(), s.rules, s.view.clone())
        };
        let join = self.seat.join();
        let team = match join {
            Join::Command(t) | Join::Watch(t) => Some(t),
            Join::Spectate => None,
        };
        let seat_text = match (&status, join) {
            (Status::Connecting, _) => "CONNECTING TO THE GAME SERVER...".to_string(),
            (Status::Refused(why), _) => format!("REFUSED: {why}"),
            (Status::Seated(Join::Command(t)), _) => format!("{} COMMANDER", team_name(*t)),
            (Status::Seated(Join::Watch(t)), _) => format!("WATCHING {}", team_name(*t)),
            (Status::Seated(Join::Spectate), _) => "SPECTATOR".into(),
        };
        let (game_text, score) = match &view {
            None => (String::new(), [0.0, 0.0]),
            Some(v) => {
                let state = match v.outcome {
                    Some(Outcome::Won(t)) => format!("{} WINS", team_name(t)),
                    Some(Outcome::Draw) => "DRAW".into(),
                    None if v.time == 0.0 => "WAITING FOR GO".into(),
                    None => "PLAYING".into(),
                };
                (format!("{}  {}", clock(v.time_left), state), v.score)
            }
        };
        let top = div()
            .flex()
            .items_center()
            .gap(px(18.0))
            .px_3()
            .py(px(6.0))
            .bg(c(BEZEL))
            .rounded(px(PANEL_RADIUS))
            .text_color(c(WHITE))
            .child(div().text_color(c(GREY)).child("TOWERS"))
            .child(div().text_color(c(team.map_or(WHITE, theme_team))).child(seat_text))
            .child(div().flex_1())
            .child(div().text_color(c(BLUE)).child(format!("BLUE {:.0}", score[0])))
            .child(div().text_color(c(RED)).child(format!("RED {:.0}", score[1])))
            .child(div().text_color(c(AMBER)).child(game_text));

        if self.valley.is_none()
            && let Some(r) = &rules
        {
            self.valley = Some(Map::new(r));
        }
        let valley = self.valley.clone();
        let ground = self.ground.clone();
        let bounds = self.map_bounds.clone();
        let selected = self.selected.clone();
        let painted = selected.clone();
        let drag = self.drag;
        let (v2, r2) = (view.clone(), rules);
        let map_canvas = canvas(
            |_, _, _| {},
            move |b, _, window, cx| {
                bounds.set(b);
                if let (Some(v), Some(r), Some(m)) = (&v2, &r2, &valley) {
                    paint_map(b, v, r, m, ground.as_deref(), &painted, drag, window, cx);
                }
                round_corners(window, b, SCREEN_RADIUS - 2.0, c(0x000000));
            },
        )
        .size_full();
        let map = div()
            .id("map")
            .size_full()
            .child(map_canvas)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_down(ev, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev, _, cx| this.map_down(ev, cx)))
            .on_mouse_move(cx.listener(|this, ev, _, cx| this.map_move(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_up(ev, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_up(ev, cx)));

        let mut drones = div().flex().flex_col().gap(px(4.0)).p_2().text_sm();
        if let Some(v) = &view {
            let tint = theme_team(v.team);
            for d in &v.drones {
                let k = d.k;
                let phase = match d.phase {
                    // No telemetry: the last the drone sent, and how long ago.
                    _ if !d.linked => format!("NO LINK {:.0} s", d.silent),
                    Phase::Flying => format!("{:.0} m", d.alt),
                    Phase::Ground => "GROUND".into(),
                    Phase::Down => "DOWN".into(),
                };
                let low = if d.low { " LOW" } else { "" };
                let label =
                    format!("{}{}  {:<7} {}{low}", &team_name(v.team)[..1], k + 1, phase, order_text(d.order, v));
                drones = drones.child(
                    button(format!("drone-{k}"), label, selected.contains(&k))
                        .border_color(c(if selected.contains(&k) { tint } else { BEZEL_EDGE }))
                        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                            this.select_many(&[k], ev.modifiers().shift);
                            cx.notify();
                        })),
                );
            }
        }
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .p_2()
            .child(button("hold", "HOLD", false).on_click(cx.listener(|this, _, _, cx| {
                this.give(Order::Hold);
                cx.notify();
            })))
            .child(button("home", "HOME", false).on_click(cx.listener(|this, _, _, cx| {
                this.give(Order::Home);
                cx.notify();
            })))
            .child(button("low", "LOW", !selected.is_empty() && selected.iter().all(|&k| self.low_of(k))).on_click(
                cx.listener(|this, _, _, cx| {
                    this.toggle_low();
                    cx.notify();
                }),
            ));
        let help = div().p_2().text_xs().text_color(c(GREY)).child(if selected.is_empty() {
            "Click a drone, or drag a box round several (Shift adds, Ctrl+A all). Then right-click to order them."
                .to_string()
        } else {
            format!(
                "{} selected. Right-click a tower to take it, or a point to fly there. LOW: along the lanes. Esc: none.",
                selected.len()
            )
        });
        let mut towers = div().flex().flex_col().gap(px(2.0)).p_2().text_sm();
        if let Some(v) = &view {
            for t in &v.towers {
                let (holder, colour) = match t.holder {
                    Some(team) => (team_name(team), theme_team(team)),
                    None => ("NEUTRAL", GREY),
                };
                let seen = if t.seen { String::new() } else { format!("  {:.0} s ago", t.age) };
                let contested = if t.contested { "  CONTESTED" } else { "" };
                towers = towers.child(div().text_color(c(colour)).child(format!(
                    "{:<10} {:<7} {:+4.0}%{contested}{seen}",
                    t.name.to_uppercase(),
                    holder,
                    100.0 * t.control
                )));
            }
        }
        let cameras = self.cameras_panel(window, &view, cx);
        let unit = self.unit_panel(&view, rules.as_ref());
        let mut right = div()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .w(px(340.0))
            .flex_none()
            .child(scrolling("DRONES", div().flex().flex_col().child(drones).child(actions).child(help)));
        if let Some(unit) = unit {
            right = right.child(screen("UNIT", unit));
        }
        let right = right.child(scrolling("TOWERS", towers)).child(scrolling("CAMERAS", cameras));

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev, _, cx| this.key(ev, cx)))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .p(px(GAP))
            .pt(px(30.0))
            .bg(c(CONSOLE))
            .font_family(FONT)
            .child(top)
            .child(div().flex().flex_1().gap(px(GAP)).child(screen("FIELD", map).flex_1()).child(right))
    }
}

fn theme_team(t: Team) -> u32 {
    crate::theme::team(t)
}

/// The team's map: the fog, what the team sees, the towers and the drones.
fn paint_map(
    b: Bounds<Pixels>,
    v: &TeamView,
    rules: &Rules,
    valley: &Map,
    ground: Option<&Ground>,
    selected: &[usize],
    drag: Option<Drag>,
    window: &mut Window,
    cx: &mut App,
) {
    let (x, y, w, h) = rect(b);
    let map = MapView::new(b, rules);
    let s = map.scale;
    fill_rect(window, x, y, w, h, c(FOG));
    // The ground: the picture of the valley, dim; what the team sees is lit over it. Without the
    // picture, the plain ground of the vector map.
    if let Some(g) = ground {
        paint_ground(window, b, map, g);
    }
    // What the team sees, as the server would work it out from this map.
    let mut eyes: Vec<(Vec2, f32)> = vec![(rules.base(v.team), rules.base_vision)];
    eyes.extend(v.drones.iter().filter(|d| d.phase != Phase::Down).map(|d| (d.at, rules.drone_vision)));
    eyes.extend(v.towers.iter().filter(|t| t.holder == Some(v.team)).map(|t| (t.at, rules.tower_vision)));
    for (at, range) in &eyes {
        let (sx, sy) = map.to_screen(*at);
        if ground.is_some() {
            fill_circle(window, sx, sy, range * s, ca(SIGHT, 0.07));
            circle(window, sx, sy, range * s, 1.0, ca(SIGHT, 0.25));
        } else {
            fill_circle(window, sx, sy, range * s, c(FIELD));
        }
    }
    if ground.is_none() {
        paint_valley(window, map, valley);
    }
    let (cx0, cy0) = map.to_screen(Vec2::ZERO);
    dashed_circle(window, cx0, cy0, rules.field_radius * s, 1.0, 6.0, ca(GREY, 0.6));
    for team in Team::BOTH {
        let (bx, by) = map.to_screen(rules.base(team));
        fill_rect(window, bx - 10.0, by - 5.0, 20.0, 10.0, ca(theme_team(team), 0.35));
        text(
            window,
            cx,
            &format!("{} BASE", team_name(team)),
            bx,
            by + 14.0,
            10.0,
            ca(theme_team(team), 0.8),
            Align::Center,
        );
    }
    // The towers, as the team last saw them.
    let flash = (v.time * 3.0) as i64 % 2 == 0;
    for t in &v.towers {
        let (tx, ty) = map.to_screen(t.at);
        let colour = t.holder.map_or(GREY, theme_team);
        let alpha = if t.seen { 1.0 } else { 0.55 };
        if t.contested && flash {
            circle(window, tx, ty, rules.circle_radius * s, 2.5, c(AMBER));
        } else {
            dashed_circle(window, tx, ty, rules.circle_radius * s, 1.0, 4.0, ca(colour, 0.6 * alpha));
        }
        fill_rect(window, tx - 7.0, ty - 7.0, 14.0, 14.0, ca(colour, alpha));
        stroke_rect(window, tx - 7.0, ty - 7.0, 14.0, 14.0, 1.0, ca(WHITE, 0.7 * alpha));
        // The control bar: right of the middle towards Blue, left towards Red.
        let (bw, by) = (44.0, ty + 14.0);
        fill_rect(window, tx - bw * 0.5, by, bw, 5.0, ca(DARK, 0.9));
        let filled = t.control.clamp(-1.0, 1.0) * bw * 0.5;
        let (from, colour) = if filled >= 0.0 { (tx, BLUE) } else { (tx + filled, RED) };
        fill_rect(window, from, by, filled.abs(), 5.0, ca(colour, alpha));
        fill_rect(window, tx - 0.5, by - 1.0, 1.0, 7.0, c(WHITE));
        let label = if t.seen { t.name.to_uppercase() } else { format!("{} ({:.0} s)", t.name.to_uppercase(), t.age) };
        text(window, cx, &label, tx, by + 14.0, 10.0, ca(WHITE, alpha), Align::Center);
    }
    let tint = theme_team(v.team);
    // The orders: a dashed line from each drone to where it is sent.
    for d in v.drones.iter().filter(|d| d.phase != Phase::Down) {
        let to = match d.order {
            Some(Order::Take(t)) => v.towers.get(t).map(|t| t.at),
            Some(Order::Goto(p)) => Some(p),
            Some(Order::Home) => Some(rules.base(v.team)),
            _ => None,
        };
        if let Some(to) = to {
            let (a, b2) = (map.to_screen(d.at), map.to_screen(to));
            dashed(window, &[a, b2], 1.0, 5.0, ca(tint, if selected.contains(&d.k) { 0.9 } else { 0.4 }));
        }
    }
    for d in &v.drones {
        let (dx, dy) = map.to_screen(d.at);
        let colour = if d.phase == Phase::Down { ca(GREY, 0.6) } else { c(tint) };
        drone_mark(window, dx, dy, d.vel, colour, true);
        if selected.contains(&d.k) {
            circle(window, dx, dy, 12.0, 1.5, c(WHITE));
        }
        text(
            window,
            cx,
            &format!("{}{}", &team_name(v.team)[..1], d.k + 1),
            dx + 14.0,
            dy - 10.0,
            10.0,
            c(tint),
            Align::Left,
        );
    }
    // The enemy drones the team sees, and where it last saw the others.
    let enemy = theme_team(v.team.other());
    for e in &v.enemies {
        let (ex, ey) = map.to_screen(e.at);
        let label = format!("{}{}", &team_name(v.team.other())[..1], e.k + 1);
        if e.seen {
            drone_mark(window, ex, ey, e.vel, c(enemy), true);
            text(window, cx, &label, ex + 14.0, ey - 10.0, 10.0, c(enemy), Align::Left);
        } else {
            drone_mark(window, ex, ey, e.vel, ca(enemy, 0.45), false);
            text(
                window,
                cx,
                &format!("{label} {:.0} s", e.age),
                ex + 14.0,
                ey - 10.0,
                10.0,
                ca(enemy, 0.6),
                Align::Left,
            );
        }
    }
    if let Some(((x0, y0), (x1, y1), _)) = drag {
        let (bx, by, bw, bh) = (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
        fill_rect(window, bx, by, bw, bh, ca(WHITE, 0.1));
        stroke_rect(window, bx, by, bw, bh, 1.0, ca(WHITE, 0.8));
    }
    if let Some(outcome) = v.outcome {
        let banner = match outcome {
            Outcome::Won(t) => format!("{} WINS", team_name(t)),
            Outcome::Draw => "DRAW".into(),
        };
        fill_rect(window, x + w * 0.5 - 140.0, y + h * 0.5 - 30.0, 280.0, 60.0, ca(0x000000, 0.8));
        text(window, cx, &banner, x + w * 0.5, y + h * 0.5, 28.0, c(AMBER), Align::Center);
    }
}

/// The picture of the valley, where the game has it, under a veil of fog.
fn paint_ground(window: &mut Window, b: Bounds<Pixels>, map: MapView, g: &Ground) {
    let half = g.size_m * 0.5;
    let (left, top) = map.to_screen(g.centre + Vec2::new(half, -half));
    let side = g.size_m * map.scale;
    let at = gpui::Bounds { origin: pt(left, top), size: gpui::size(px(side), px(side)) };
    window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
        let _ = window.paint_image(at, Corners::default(), g.image.clone(), 0, false);
        let (x, y, w, h) = rect(b);
        fill_rect(window, x, y, w, h, ca(FOG, 0.62));
    });
}

/// The ground a team knows: the lake, the lanes cut through the forest and the clearings. It is
/// drawn over the fog and the team's sight both, dim.
fn paint_valley(window: &mut Window, map: MapView, valley: &Map) {
    let s = map.scale;
    let (lx, ly) = map.to_screen(valley.lake.at);
    fill_circle(window, lx, ly, valley.lake.radius * s, ca(LAKE, 0.8));
    for lane in &valley.lanes {
        let pts: Vec<(f32, f32)> = lane.points.iter().map(|&p| map.to_screen(p)).collect();
        polyline(window, &pts, 2.0 * LANE_HALF_WIDTH * s, ca(LANE, 0.75));
    }
    for cl in &valley.clearings {
        let (x, y) = map.to_screen(cl.at);
        fill_circle(window, x, y, cl.radius * s, ca(LANE, 0.75));
    }
}

/// A drone: a triangle pointing the way it flies (north when it is still).
fn drone_mark(window: &mut Window, x: f32, y: f32, vel: Vec2, colour: gpui::Hsla, filled: bool) {
    let dir = if vel.length() > 1.0 { vel.normalize() } else { Vec2::X };
    // North is up the screen and east to the right.
    let (fx, fy) = (dir.y, -dir.x);
    let (sx, sy) = (-fy, fx);
    let pts = [
        (x + fx * 9.0, y + fy * 9.0),
        (x - fx * 6.0 + sx * 6.0, y - fy * 6.0 + sy * 6.0),
        (x - fx * 6.0 - sx * 6.0, y - fy * 6.0 - sy * 6.0),
    ];
    if filled {
        fill_poly(window, &pts, colour);
    } else {
        polyline(window, &[pts[0], pts[1], pts[2], pts[0]], 1.2, colour);
    }
}

/// A top-down silhouette of a drone of `class`, nose up, centred on (`x`, `y`) in an 84 pixel square.
fn unit_icon(window: &mut Window, x: f32, y: f32, class: Class, colour: gpui::Hsla) {
    match class {
        Class::Light => {
            // A quadcopter: two arms crossed, a rotor at each end.
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0)] {
                polyline(window, &[(x - dx * 24.0, y - dy * 24.0), (x + dx * 24.0, y + dy * 24.0)], 3.0, colour);
            }
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                circle(window, x + dx * 24.0, y + dy * 24.0, 11.0, 1.5, colour);
            }
            fill_rect(window, x - 6.0, y - 6.0, 12.0, 12.0, colour);
        }
        Class::Standard | Class::Heavy => {
            // A VTOL aeroplane: fuselage, wing, tailplane; the heavy one is bigger.
            let s = if class == Class::Heavy { 1.0 } else { 0.8 };
            fill_poly(window, &[(x, y - 36.0 * s), (x + 4.0 * s, y - 24.0 * s), (x + 4.0 * s, y + 32.0 * s), (x - 4.0 * s, y + 32.0 * s), (x - 4.0 * s, y - 24.0 * s)], colour);
            fill_poly(window, &[(x - 36.0 * s, y - 2.0 * s), (x + 36.0 * s, y - 2.0 * s), (x + 36.0 * s, y + 8.0 * s), (x - 36.0 * s, y + 8.0 * s)], colour);
            fill_poly(window, &[(x - 14.0 * s, y + 24.0 * s), (x + 14.0 * s, y + 24.0 * s), (x + 14.0 * s, y + 32.0 * s), (x - 14.0 * s, y + 32.0 * s)], colour);
            for dx in [-22.0, 22.0] {
                circle(window, x + dx * s, y - 14.0 * s, 8.0 * s, 1.5, colour);
            }
        }
    }
}
