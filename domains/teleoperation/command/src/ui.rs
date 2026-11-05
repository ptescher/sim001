//! The commander's window, laid out as a real-time strategy game: a thin top bar (the seat, a chip
//! for each tower, the scores and the clock), the battlefield that fills the window (the valley in
//! 3D from the commander's camera, with the team's map drawn on it), and the bottom bar (the
//! minimap, a card for each drone with its camera, the PORTRAIT and UNIT of the selected drone,
//! and the command card).
//!
//! Left-click a drone (on the battlefield or its card) to select it, or drag a box round several;
//! Shift adds to the selection (and takes a drone out of it again), Ctrl/Cmd+A selects all, the
//! number keys select one drone, Esc lets go. Right-click a tower (take it) or anywhere else on the
//! field (go there and circle) to order the selected drones. HOLD and HOME (or H and B) act on the
//! selected drones; LOW (L) makes them fly along the lanes, below the treetops.
//!
//! The camera: W, A, S, D or the arrow keys move it, and so does the mouse at an edge of the
//! battlefield; Q and E turn it by 45°; the wheel takes it nearer to the ground or further; a click
//! on the minimap looks there; Space looks at the selected drones.

use crate::battlefield::Battlefield;
use crate::cameras::Cameras;
use crate::draw::*;
use crate::field::{self, Drag, PING, Scene, team_name};
use crate::ground::Ground;
use crate::lens::{Flat, Lens};
use crate::seat::{Seat, Status};
use crate::theme::*;
use glam::Vec2;
use gpui::{
    Bounds, Context, Corners, Div, FocusHandle, IntoElement, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Render, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Stateful,
    Window, canvas, div, prelude::*, px,
};
use recreation_towers::battlefield::Eye;
use recreation_towers::map::Map;
use recreation_towers::seat::Join;
use recreation_towers::{Class, Order, Phase, Rules, Team, TeamView};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

thread_local! {
    /// Where each button was last painted, by id, so `--expect` can click it the way a person
    /// would. Written while painting, read between frames; both on the UI thread.
    static BUTTONS: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::new(HashMap::new());
}

/// Where the button with this id is on screen now, if it is.
pub fn button_bounds(id: &str) -> Option<Bounds<Pixels>> {
    BUTTONS.with(|b| b.borrow().get(id).copied())
}

/// A drone on the battlefield is clicked when the click is this near it (screen pixels).
const PICK: f32 = 16.0;

/// A left button press that moves less than this (screen pixels) is a click, not a box.
const DRAG: f32 = 4.0;

/// The mouse this near an edge of the battlefield moves the camera (screen pixels).
const EDGE: f32 = 14.0;

/// How the camera looks down, and how near and how far from the ground it goes (m).
const PITCH: f32 = 50.0;
const HEIGHT_MIN: f32 = 20.0;
const HEIGHT_MAX: f32 = 2500.0;
/// One step of the wheel takes the camera this much nearer.
const WHEEL_STEP: f32 = 0.8;
/// Q and E turn the camera this far (deg).
const TURN: f32 = 45.0;

/// The height of the bottom bar, and the side of its minimap (px).
const BAR: f32 = 212.0;
const MINIMAP: f32 = 176.0;

/// How long a piece of the advisor's advice stays on the battlefield.
const ADVICE_SHOWN: Duration = Duration::from_secs(20);

pub struct CommanderView {
    pub seat: Arc<Seat>,
    /// The selected drones (their numbers in the team), in order.
    pub selected: Vec<usize>,
    drag: Option<Drag>,
    /// The left button is down on the minimap, moving the camera.
    minimap_drag: bool,
    /// The battlefield and the minimap, as last painted.
    pub map_bounds: Rc<Cell<Bounds<Pixels>>>,
    minimap_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The valley (lanes, lake, clearings), worked out from the game's rules once they arrive.
    valley: Option<Map>,
    /// The picture of the valley under the minimap, if it has been exported.
    ground: Option<Arc<Ground>>,
    /// The pictures from the team's drones, and the one of each that the window has on its atlas.
    pub cameras: Arc<Cameras>,
    shown: Vec<Option<Arc<RenderImage>>>,
    /// The picture of the battlefield from the 3D view, and the one the window has on its atlas.
    battlefield: Arc<Battlefield>,
    picture: Option<(Arc<RenderImage>, Eye)>,
    focus: FocusHandle,
    /// The commander's camera over the battlefield.
    pub eye: Eye,
    /// The camera has been put over the team's base.
    eye_placed: bool,
    /// The keys that move the camera, held down now.
    held: HashSet<String>,
    /// The mouse on the battlefield, for the edges (screen pixels).
    hover: Option<(f32, f32)>,
    last_tick: Instant,
    /// Where the last right-click sent the drones, and when.
    ping: Option<(Vec2, Instant)>,
    /// The tower chips of the top bar, as last drawn: the team that holds each.
    pub chips: RefCell<Vec<Option<Team>>>,
    /// The drone whose camera the PORTRAIT showed when last drawn.
    pub portrait: Cell<Option<usize>>,
}

impl CommanderView {
    pub fn new(
        seat: Arc<Seat>,
        cameras: Arc<Cameras>,
        battlefield: Arc<Battlefield>,
        ground: Option<Arc<Ground>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CommanderView {
        // The game arrives on another thread; the camera moves and the window repaints at about 30 Hz.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                let ticked = this.update(cx, |v, cx| {
                    v.tick();
                    cx.notify()
                });
                if ticked.is_err() {
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
            minimap_drag: false,
            map_bounds: Rc::new(Cell::new(Bounds::default())),
            minimap_bounds: Rc::new(Cell::new(Bounds::default())),
            valley: None,
            ground,
            shown: vec![None; cameras.len()],
            cameras,
            battlefield,
            picture: None,
            focus,
            eye: Eye { pitch: PITCH, ..Eye::default() },
            eye_placed: false,
            held: HashSet::new(),
            hover: None,
            last_tick: Instant::now(),
            ping: None,
            chips: RefCell::default(),
            portrait: Cell::new(None),
        }
    }

    /// As on a fresh start of the app: nothing selected, and the camera over the team's base.
    pub fn fresh(&mut self) {
        self.selected.clear();
        self.eye_placed = false;
        self.held.clear();
        self.hover = None;
    }

    /// The battlefield shows the picture of the 3D view (not the plain map).
    pub fn battlefield_live(&self) -> bool {
        self.battlefield.picture().is_some()
    }

    /// The battlefield through the camera now.
    fn lens(&self) -> Lens {
        Lens::new(self.eye, self.map_bounds.get())
    }

    /// Where the field point `at` is on the battlefield now.
    pub fn screen_of(&self, at: Vec2) -> Option<(f32, f32)> {
        self.lens().ground(at)
    }

    fn flat(&self) -> Option<Flat> {
        let rules = self.seat.state().rules?;
        Some(Flat::new(self.minimap_bounds.get(), rules.field_radius))
    }

    /// Where the field point `at` is on the minimap now.
    pub fn minimap_screen(&self, at: Vec2) -> Option<(f32, f32)> {
        Some(self.flat()?.to_screen(at))
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

    /// The drones near the screen point `p` on the battlefield (within `PICK`), nearest first.
    fn drones_at(&self, p: (f32, f32)) -> Vec<usize> {
        let Some(view) = self.seat.state().view.clone() else { return vec![] };
        let mut near: Vec<(usize, f32)> = field::drones_on_screen(self.lens(), &view)
            .into_iter()
            .map(|(k, s)| (k, (s.0 - p.0).hypot(s.1 - p.1)))
            .filter(|(_, d)| *d <= PICK)
            .collect();
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        near.into_iter().map(|(k, _)| k).collect()
    }

    /// A left click on the battlefield at `p`: selects the drone there (the nearest), or with
    /// nothing there lets go of the selection. `add` (Shift) keeps the selection and toggles the drone.
    fn click_field(&mut self, p: (f32, f32), add: bool) {
        match self.drones_at(p).first() {
            Some(&k) => self.select_many(&[k], add),
            None if !add => self.selected.clear(),
            None => {}
        }
    }

    /// A box on the battlefield with corners `a` and `b` (screen): selects the drones inside it.
    fn box_select(&mut self, a: (f32, f32), b: (f32, f32), add: bool) {
        let (x0, x1, y0, y1) = (a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1));
        let Some(view) = self.seat.state().view.clone() else { return };
        let inside: Vec<usize> = field::drones_on_screen(self.lens(), &view)
            .into_iter()
            .filter(|(_, (x, y))| *x >= x0 && *x <= x1 && *y >= y0 && *y <= y1)
            .map(|(k, _)| k)
            .collect();
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
        let (order, to) = match tower {
            Some(t) => (Order::Take(t), view.towers[t].at),
            None => {
                let p = at.clamp_length_max(rules.field_radius - 40.0);
                (Order::Goto(p), p)
            }
        };
        if !self.selected.is_empty() && self.commands() {
            self.ping = Some((to, Instant::now()));
        }
        self.give(order);
    }

    // The camera.

    /// Puts the camera over the team's base, looking towards the enemy, once the rules arrive.
    fn place_eye(&mut self) {
        if self.eye_placed {
            return;
        }
        let Some(rules) = self.seat.state().rules else { return };
        let team = match self.seat.join() {
            Join::Command(t) | Join::Watch(t) => t,
            Join::Spectate => Team::Blue,
        };
        self.eye = Eye {
            look: rules.base(team),
            // Blue's base is in the south: Blue looks north, Red south.
            heading: if rules.base(team).x < 0.0 { 0.0 } else { 180.0 },
            pitch: PITCH,
            distance: 450.0,
            ..Eye::default()
        };
        self.eye_placed = true;
    }

    /// Moves the camera by the keys held and the mouse at an edge.
    fn tick(&mut self) {
        let dt = self.last_tick.elapsed().as_secs_f32().min(0.1);
        self.last_tick = Instant::now();
        let held = |k: &[&str]| k.iter().any(|k| self.held.contains(*k));
        let (mut fwd, mut right) = (0.0f32, 0.0f32);
        if held(&["w", "up"]) {
            fwd += 1.0;
        }
        if held(&["s", "down"]) {
            fwd -= 1.0;
        }
        if held(&["d", "right"]) {
            right += 1.0;
        }
        if held(&["a", "left"]) {
            right -= 1.0;
        }
        if let Some((x, y)) = self.hover {
            let (bx, by, bw, bh) = rect(self.map_bounds.get());
            if x < bx + EDGE {
                right -= 1.0;
            }
            if x > bx + bw - EDGE {
                right += 1.0;
            }
            if y < by + EDGE {
                fwd += 1.0;
            }
            if y > by + bh - EDGE {
                fwd -= 1.0;
            }
        }
        if fwd == 0.0 && right == 0.0 {
            return;
        }
        let h = self.eye.heading.to_radians();
        let (ahead, side) = (Vec2::new(h.cos(), h.sin()), Vec2::new(-h.sin(), h.cos()));
        // Faster the higher the camera is, as in a strategy game.
        let speed = (self.eye.height() * 1.3).max(120.0);
        let to = self.eye.look + (ahead * fwd + side * right).normalize_or_zero() * speed * dt;
        self.look_at(to);
    }

    /// The camera looks at `at`, kept over the field.
    fn look_at(&mut self, at: Vec2) {
        let limit = self.seat.state().rules.map_or(1000.0, |r| r.field_radius * 1.3);
        self.eye.look = at.clamp_length_max(limit);
    }

    /// Q and E: the camera turns `by` degrees, to a multiple of 45°.
    fn turn(&mut self, by: f32) {
        self.eye.heading = ((self.eye.heading + by) / TURN).round() * TURN;
        self.eye.heading = self.eye.heading.rem_euclid(360.0);
    }

    /// The camera goes nearer to the ground (`factor` < 1) or further.
    fn zoom(&mut self, factor: f32) {
        let s = self.eye.pitch.to_radians().sin();
        self.eye.distance = (self.eye.distance * factor).clamp(HEIGHT_MIN / s, HEIGHT_MAX / s);
    }

    /// Space: the camera looks at the middle of the selected drones.
    fn focus_selection(&mut self) {
        let Some(view) = self.seat.state().view.clone() else { return };
        let at: Vec<Vec2> = view.drones.iter().filter(|d| self.selected.contains(&d.k)).map(|d| d.at).collect();
        if !at.is_empty() {
            let mid = at.iter().copied().sum::<Vec2>() / at.len() as f32;
            self.look_at(mid);
        }
    }

    fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let m = ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        match key {
            k @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                let n = k.parse::<usize>().map_or(0, |n| n - 1);
                if n < self.drone_count() {
                    self.select_many(&[n], m.shift)
                }
            }
            "a" if m.secondary() || m.control => {
                let all: Vec<usize> = (0..self.drone_count()).collect();
                self.select_many(&all, false)
            }
            "w" | "a" | "s" | "d" | "up" | "down" | "left" | "right" => {
                self.held.insert(key.to_string());
            }
            "q" => self.turn(-TURN),
            "e" => self.turn(TURN),
            "space" => self.focus_selection(),
            "h" => self.give(Order::Hold),
            "b" => self.give(Order::Home),
            "l" => self.toggle_low(),
            "escape" => self.selected.clear(),
            _ => return,
        }
        cx.notify();
    }

    fn key_up(&mut self, ev: &KeyUpEvent) {
        self.held.remove(ev.keystroke.key.as_str());
    }

    /// The wheel takes the camera nearer to the ground, or further.
    fn map_scroll(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let factor = match ev.delta {
            ScrollDelta::Pixels(p) => (-f(p.y) * 0.004).exp(),
            ScrollDelta::Lines(l) => WHEEL_STEP.powf(l.y),
        };
        self.zoom(factor);
        cx.notify();
    }

    fn map_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let p = (f(ev.position.x), f(ev.position.y));
        match ev.button {
            MouseButton::Left => self.drag = Some((p, p, ev.modifiers.shift)),
            MouseButton::Right => {
                if let Some(at) = self.lens().to_field(p.0, p.1) {
                    self.order_field(at);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn map_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let p = (f(ev.position.x), f(ev.position.y));
        self.hover = self.map_bounds.get().contains(&ev.position).then_some(p);
        if let Some((_, to, _)) = &mut self.drag {
            if ev.dragging() {
                *to = p;
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
        if (to.0 - from.0).hypot(to.1 - from.1) < DRAG {
            self.click_field(to, add);
        } else {
            self.box_select(from, to, add);
        }
        cx.notify();
    }

    /// The camera looks at the point of the minimap under `p`.
    fn minimap_look(&mut self, p: gpui::Point<Pixels>) {
        let b = self.minimap_bounds.get();
        let Some(flat) = self.flat() else { return };
        let (x, y, w, h) = rect(b);
        self.look_at(flat.to_field(f(p.x).clamp(x, x + w), f(p.y).clamp(y, y + h)));
    }
}

/// What an order is, on the card of a drone.
pub fn order_text(order: Option<Order>, view: &TeamView) -> String {
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

/// Records where the element with `id` is painted, for `--expect`.
fn recorder(id: SharedString) -> impl IntoElement {
    canvas(move |b, _, _| BUTTONS.with(|m| m.borrow_mut().insert(id, b)), |_, _, _, _| {}).absolute().inset_0()
}

/// A button of the command card: a square with its name, and its key in the corner.
fn command(id: &'static str, name: &'static str, key: &'static str, active: bool) -> Stateful<Div> {
    let (bg, fg, border) = if active { (ACTIVE_FILL, GREEN, GREEN_DIM) } else { (CONTROL, WHITE, BEZEL_EDGE) };
    div()
        .id(id)
        .relative()
        .w(px(62.0))
        .h(px(52.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(c(bg))
        .text_color(c(fg))
        .text_sm()
        .border_1()
        .border_color(c(border))
        .rounded(px(CONTROL_RADIUS))
        .cursor_pointer()
        .hover(|s| s.bg(c(CONTROL_HOVER)))
        .active(|s| s.opacity(0.7))
        .child(name)
        .child(div().absolute().top(px(2.0)).right(px(4.0)).text_xs().text_color(c(AMBER)).child(key))
        .child(recorder(id.into()))
}

/// A titled panel of the bars, as on the console.
fn panel(title: &'static str, body: impl IntoElement) -> Div {
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
        .child(div().flex_1().min_h(px(0.0)).bg(c(SCREEN)).rounded(px(SCREEN_RADIUS)).overflow_hidden().child(body))
}

fn theme_team(t: Team) -> u32 {
    crate::theme::team(t)
}

impl CommanderView {
    /// The newest pictures of the drones' cameras; the ones the window had are freed from its atlas.
    fn camera_images(&mut self, window: &mut Window) -> Vec<Option<Arc<RenderImage>>> {
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
        images
    }

    /// The newest picture of the battlefield; the one the window had is freed from its atlas.
    fn battlefield_picture(&mut self, window: &mut Window) -> Option<(Arc<RenderImage>, Eye)> {
        let new = self.battlefield.picture();
        let same = match (&self.picture, &new) {
            (Some((a, _)), Some((b, _))) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same && let Some((old, _)) = std::mem::replace(&mut self.picture, new.clone()) {
            let _ = window.drop_image(old);
        }
        new
    }

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
        let more =
            if self.selected.len() > 1 { format!("  (+{} more)", self.selected.len() - 1) } else { String::new() };
        let rows = [
            ("CLASS", class.name().to_uppercase()),
            ("WEIGHT", format!("{} on a tower", class.weight())),
            ("MASS", format!("{:.2} kg", class.mass_kg())),
            ("SPEED", format!("{:.0} m/s", d.vel.length())),
            ("ALTITUDE", format!("{:.0} m", d.alt)),
            ("STATE", format!("{state}{}", if d.low { " LOW" } else { "" })),
        ];
        let mut stats = div().flex().flex_col().flex_1().text_xs();
        stats = stats.child(div().text_sm().text_color(c(tint)).child(format!(
            "{}{}{more}",
            &team_name(v.team)[..1],
            k + 1
        )));
        for (name, value) in rows {
            stats = stats.child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(div().w(px(58.0)).text_color(c(GREY)).child(name))
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

    /// The card of each drone: its camera, its name, its altitude and its order.
    fn cards(&self, images: &[Option<Arc<RenderImage>>], view: &Option<TeamView>, cx: &mut Context<Self>) -> Div {
        let mut cards = div().flex().flex_wrap().content_start().gap(px(6.0)).p(px(6.0));
        let Some(v) = view else { return cards };
        let tint = theme_team(v.team);
        let battery = self.seat.state().rules.is_some_and(|r| r.battery(r.class(0)) > 0.0);
        for d in &v.drones {
            let k = d.k;
            let picked = self.selected.contains(&k);
            let name = format!("{}{}", &team_name(v.team)[..1], k + 1);
            let state = match d.phase {
                // No telemetry: the last the drone sent, and how long ago.
                _ if !d.linked => format!("NO LINK {:.0} s", d.silent),
                Phase::Flying => format!("{:.0} m", d.alt),
                Phase::Ground => "GROUND".into(),
                Phase::Down => "DOWN".into(),
            };
            let mut tags = vec![];
            if d.low {
                tags.push("LOW".to_string());
            }
            if battery {
                tags.push(format!("BAT {:.0} s", d.battery));
            }
            let img = images.get(k).cloned().flatten().filter(|_| d.linked);
            let tile = camera_tile(format!("camera-{k}"), &name, img, !d.linked, 118.0, 66.0, picked).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.select(Some(k));
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
            let card = div()
                .id(SharedString::from(format!("drone-{k}")))
                .relative()
                .w(px(122.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .p(px(1.0))
                .bg(c(if picked { ACTIVE_FILL } else { CONTROL }))
                .border_1()
                .border_color(c(if picked { tint } else { BEZEL_EDGE }))
                .rounded(px(CONTROL_RADIUS))
                .cursor_pointer()
                .hover(|s| s.bg(c(CONTROL_HOVER)))
                .text_xs()
                .child(tile)
                .child(
                    div()
                        .flex()
                        .px_1()
                        .child(div().text_color(c(tint)).child(name))
                        .child(div().flex_1())
                        .child(div().text_color(c(WHITE)).child(state)),
                )
                .child(div().px_1().text_color(c(GREY)).child(order_text(d.order, v)))
                .child(div().px_1().h(px(14.0)).text_color(c(AMBER)).child(tags.join("  ")))
                .child(recorder(format!("drone-{k}").into()))
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                    this.select_many(&[k], ev.modifiers().shift);
                    cx.notify();
                }));
            cards = cards.child(card);
        }
        cards
    }
}

impl Render for CommanderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.place_eye();
        let (status, rules, view) = {
            let s = self.seat.state();
            (s.status.clone(), s.rules, s.view.clone())
        };
        let join = self.seat.join();
        let team = match join {
            Join::Command(t) | Join::Watch(t) => Some(t),
            Join::Spectate => None,
        };
        let seat_text = match &status {
            Status::Connecting => "CONNECTING TO THE GAME SERVER...".to_string(),
            Status::Refused(why) => format!("REFUSED: {why}"),
            Status::Seated(Join::Command(t)) => format!("{} COMMANDER", team_name(*t)),
            Status::Seated(Join::Watch(t)) => format!("WATCHING {}", team_name(*t)),
            Status::Seated(Join::Spectate) => "SPECTATOR".into(),
        };

        // The top bar: the seat, a chip for each tower, the scores and the clock.
        let mut chips = div().flex().gap(px(4.0));
        let mut held = vec![];
        let flash = view.as_ref().is_some_and(|v| (v.time * 3.0) as i64 % 2 == 0);
        if let Some(v) = &view {
            for t in &v.towers {
                held.push(t.holder);
                let colour = t.holder.map_or(GREY, theme_team);
                let edge = if t.contested && flash { AMBER } else { colour };
                chips = chips.child(
                    div()
                        .px(px(6.0))
                        .py(px(1.0))
                        .text_xs()
                        .bg(ca(colour, if t.seen { 0.35 } else { 0.15 }))
                        .border_1()
                        .border_color(c(edge))
                        .rounded(px(CONTROL_RADIUS))
                        .text_color(c(WHITE))
                        .child(t.name.to_uppercase()),
                );
            }
        }
        *self.chips.borrow_mut() = held.clone();
        let count = |t: Team| held.iter().filter(|h| **h == Some(t)).count();
        let (game_text, score) = match &view {
            None => (String::new(), [0.0, 0.0]),
            Some(v) => {
                let state = match v.outcome {
                    Some(recreation_towers::Outcome::Won(t)) => format!("{} WINS", team_name(t)),
                    Some(recreation_towers::Outcome::Draw) => "DRAW".into(),
                    None if v.time == 0.0 => "WAITING FOR GO".into(),
                    None => "PLAYING".into(),
                };
                (format!("{}  {}", clock(v.time_left), state), v.score)
            }
        };
        let top = div()
            .flex()
            .items_center()
            .gap(px(16.0))
            .px_3()
            .h(px(30.0))
            .flex_none()
            .bg(c(BEZEL))
            .border_b_1()
            .border_color(c(BEZEL_EDGE))
            .text_sm()
            .text_color(c(WHITE))
            .child(div().text_color(c(GREY)).child("TOWERS"))
            .child(div().text_color(c(team.map_or(WHITE, theme_team))).child(seat_text))
            .child(chips)
            .child(div().flex_1())
            .child(div().text_color(c(BLUE)).child(format!("BLUE {:.0}  ({} held)", score[0], count(Team::Blue))))
            .child(div().text_color(c(RED)).child(format!("RED {:.0}  ({} held)", score[1], count(Team::Red))))
            .child(div().text_color(c(AMBER)).child(game_text));

        if self.valley.is_none()
            && let Some(r) = &rules
        {
            self.valley = Some(Map::new(r));
        }

        // The battlefield.
        let picture = self.battlefield_picture(window);
        let battlefield = self.battlefield.clone();
        let bounds = self.map_bounds.clone();
        let selected = self.selected.clone();
        let (drag, ping, eye) = (self.drag, self.ping.filter(|(_, t)| t.elapsed().as_secs_f32() < PING), self.eye);
        let (v2, r2, valley) = (view.clone(), rules, self.valley.clone());
        let map_canvas = canvas(
            |_, _, _| {},
            move |b, _, window, cx| {
                bounds.set(b);
                let (_, _, w, h) = rect(b);
                battlefield.ask(eye, w.round() as u16, h.round() as u16);
                // The picture's own eye, so that the map lies on it where it was rendered.
                let (image, lens) = match &picture {
                    Some((img, at)) => (Some(img.clone()), Lens::new(*at, b)),
                    None => (None, Lens::new(eye, b)),
                };
                if let (Some(v), Some(r), Some(m)) = (&v2, &r2, &valley) {
                    let scene = Scene { picture: image, selected: &selected, drag, ping };
                    field::paint_battlefield(b, lens, v, r, m, &scene, window, cx);
                } else {
                    let (x, y, w, h) = rect(b);
                    fill_rect(window, x, y, w, h, c(FOG));
                }
            },
        )
        .size_full();
        // The advisor speaks when something matters, and its words stay a while, over the battlefield.
        let advice: Vec<_> = self
            .seat
            .state()
            .advice
            .iter()
            .rev()
            .filter(|(_, at)| at.elapsed() < ADVICE_SHOWN)
            .map(|(a, _)| a.text.clone())
            .collect();
        let mut toasts = div().absolute().top(px(10.0)).left(px(10.0)).flex().flex_col().gap(px(4.0)).max_w(px(460.0));
        for line in advice {
            toasts = toasts.child(
                div()
                    .px_2()
                    .py(px(3.0))
                    .bg(ca(0x000000, 0.7))
                    .border_l_2()
                    .border_color(c(AMBER))
                    .text_sm()
                    .text_color(c(AMBER))
                    .child(format!("ADVISOR  {line}")),
            );
        }
        let map = div()
            .id("map")
            .relative()
            .flex_1()
            .min_h(px(0.0))
            .child(map_canvas)
            .child(toasts)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_down(ev, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev, _, cx| this.map_down(ev, cx)))
            .on_scroll_wheel(cx.listener(|this, ev, _, cx| this.map_scroll(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_up(ev, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_up(ev, cx)));

        // The bottom bar: the minimap.
        let mini_bounds = self.minimap_bounds.clone();
        let (v3, r3, ground, mini_lens) = (view.clone(), rules, self.ground.clone(), self.lens());
        let minimap = div()
            .id("minimap")
            .w(px(MINIMAP))
            .h(px(MINIMAP))
            .cursor_crosshair()
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        mini_bounds.set(b);
                        if let (Some(v), Some(r)) = (&v3, &r3) {
                            field::paint_minimap(window, b, v, r, ground.as_deref(), mini_lens);
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.minimap_drag = true;
                    this.minimap_look(ev.position);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.minimap_drag = false;
                }),
            );

        // The cards of the drones, the PORTRAIT and UNIT of the selected one, and the command card.
        let images = self.camera_images(window);
        let cards = self.cards(&images, &view, cx);
        let first = self.selected.first().copied();
        let linked = |k: usize| view.as_ref().and_then(|v| v.drones.get(k)).is_none_or(|d| d.linked);
        let portrait_img = first.and_then(|k| images.get(k).cloned().flatten().filter(|_| linked(k)));
        self.portrait.set(first.filter(|_| portrait_img.is_some()));
        let unit = self.unit_panel(&view, rules.as_ref());
        let any = !self.selected.is_empty();
        let all_low = any && self.selected.iter().all(|&k| self.low_of(k));
        let card = div()
            .flex()
            .flex_wrap()
            .w(px(3.0 * 62.0 + 2.0 * 6.0 + 12.0))
            .gap(px(6.0))
            .p(px(6.0))
            .child(command("hold", "HOLD", "H", false).on_click(cx.listener(|this, _, _, cx| {
                this.give(Order::Hold);
                cx.notify();
            })))
            .child(command("home", "HOME", "B", false).on_click(cx.listener(|this, _, _, cx| {
                this.give(Order::Home);
                cx.notify();
            })))
            .child(command("low", "LOW", "L", all_low).on_click(cx.listener(|this, _, _, cx| {
                this.toggle_low();
                cx.notify();
            })))
            .child(command("all", "ALL", "^A", false).on_click(cx.listener(|this, _, _, cx| {
                let all: Vec<usize> = (0..this.drone_count()).collect();
                this.select_many(&all, false);
                cx.notify();
            })))
            .child(command("focus", "FOCUS", "SPC", false).on_click(cx.listener(|this, _, _, cx| {
                this.focus_selection();
                cx.notify();
            })))
            .child(command("none", "NONE", "ESC", false).on_click(cx.listener(|this, _, _, cx| {
                this.selected.clear();
                cx.notify();
            })))
            .child(div().w_full().text_xs().text_color(c(GREY)).child(if any {
                "Right-click: take a tower, or fly to a point."
            } else {
                "Select a drone: click it, or drag a box."
            }));
        let mut bottom = div()
            .flex()
            .flex_none()
            .h(px(BAR))
            .gap(px(GAP))
            .p(px(GAP))
            .bg(c(CONSOLE))
            .border_t_1()
            .border_color(c(BEZEL_EDGE))
            .child(panel("MAP", minimap).flex_none())
            .child(panel("DRONES", cards).flex_1().min_w(px(0.0)));
        if let Some(k) = first {
            let label = team.map_or(String::new(), |t| format!("{}{}", &team_name(t)[..1], k + 1));
            let portrait = div().p(px(4.0)).child(camera_tile(
                "camera-selected".to_string(),
                &label,
                portrait_img,
                !linked(k),
                200.0,
                150.0,
                false,
            ));
            bottom = bottom.child(panel("PORTRAIT", portrait).flex_none());
        }
        if let Some(unit) = unit {
            bottom = bottom.child(panel("UNIT", unit).w(px(250.0)).flex_none());
        }
        let bottom = bottom.child(panel("COMMANDS", card).flex_none());

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev, _, cx| this.key(ev, cx)))
            .on_key_up(cx.listener(|this, ev, _, _| this.key_up(ev)))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.minimap_drag {
                    if ev.dragging() {
                        this.minimap_look(ev.position);
                    } else {
                        this.minimap_drag = false;
                    }
                }
                this.map_move(ev, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .pt(px(28.0))
            .bg(c(CONSOLE))
            .font_family(FONT)
            .child(top)
            .child(map)
            .child(bottom)
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
            fill_poly(
                window,
                &[
                    (x, y - 36.0 * s),
                    (x + 4.0 * s, y - 24.0 * s),
                    (x + 4.0 * s, y + 32.0 * s),
                    (x - 4.0 * s, y + 32.0 * s),
                    (x - 4.0 * s, y - 24.0 * s),
                ],
                colour,
            );
            fill_poly(
                window,
                &[
                    (x - 36.0 * s, y - 2.0 * s),
                    (x + 36.0 * s, y - 2.0 * s),
                    (x + 36.0 * s, y + 8.0 * s),
                    (x - 36.0 * s, y + 8.0 * s),
                ],
                colour,
            );
            fill_poly(
                window,
                &[
                    (x - 14.0 * s, y + 24.0 * s),
                    (x + 14.0 * s, y + 24.0 * s),
                    (x + 14.0 * s, y + 32.0 * s),
                    (x - 14.0 * s, y + 32.0 * s),
                ],
                colour,
            );
            for dx in [-22.0, 22.0] {
                circle(window, x + dx * s, y - 14.0 * s, 8.0 * s, 1.5, colour);
            }
        }
    }
}
