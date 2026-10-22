//! The commander's window: the top bar (the seat, the clock, the scores), the map of the team
//! (the field, its fog, the towers, the drones) and the list of drones with their orders.
//!
//! To give an order: select a drone (click it in the list or on the map, or press 1, 2 or 3),
//! then click a tower (take it) or anywhere else on the field (go there and circle). HOLD and
//! HOME (or H and B) act on the selected drone; Esc lets it go. LOW (L) makes the selected drone
//! fly along the lanes, below the treetops; CAMERAS shows the camera of each drone.

use crate::cameras::Cameras;
use crate::draw::*;
use crate::seat::{Seat, Status};
use crate::theme::*;
use glam::Vec2;
use gpui::{
    App, Bounds, Context, Corners, Div, FocusHandle, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Pixels,
    Render, RenderImage, SharedString, Stateful, Window, canvas, div, prelude::*, px,
};
use recreation_towers::map::{LANE_HALF_WIDTH, Map};
use recreation_towers::seat::Join;
use recreation_towers::{Order, Outcome, Phase, Rules, Team, TeamView};
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
    /// The selected drone (its number in the team).
    pub selected: Option<usize>,
    pub map_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The valley (lanes, lake, clearings), worked out from the game's rules once they arrive.
    valley: Option<Map>,
    /// The pictures from the team's drones, and the one of each that the window has on its atlas.
    pub cameras: Arc<Cameras>,
    shown: Vec<Option<Arc<RenderImage>>>,
    focus: FocusHandle,
}

impl CommanderView {
    pub fn new(seat: Arc<Seat>, cameras: Arc<Cameras>, window: &mut Window, cx: &mut Context<Self>) -> CommanderView {
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
            selected: None,
            map_bounds: Rc::new(Cell::new(Bounds::default())),
            valley: None,
            cameras,
            shown: vec![None; 3],
            focus,
        }
    }

    /// The seat commands its team (it does not only watch).
    fn commands(&self) -> bool {
        matches!(self.seat.state().status, Status::Seated(Join::Command(_)))
    }

    pub fn select(&mut self, k: Option<usize>) {
        self.selected = k;
    }

    /// Whether drone `k`'s order is flown low now, as the server has it.
    fn low_of(&self, k: usize) -> bool {
        self.seat.state().view.as_ref().and_then(|v| v.drones.get(k)).is_some_and(|d| d.low)
    }

    /// Gives the selected drone `order`, flown as the drone flies now (low or high).
    pub fn give(&mut self, order: Order) {
        if let Some(k) = self.selected
            && self.commands()
        {
            let low = self.low_of(k);
            self.seat.order(k, order, low);
        }
    }

    /// LOW: the selected drone flies its order low, along the lanes, or (again) high.
    pub fn toggle_low(&mut self) {
        let Some(k) = self.selected else { return };
        if !self.commands() {
            return;
        }
        let (order, low) = {
            let s = self.seat.state();
            let d = s.view.as_ref().and_then(|v| v.drones.get(k));
            (d.and_then(|d| d.order).unwrap_or(Order::Home), d.is_some_and(|d| d.low))
        };
        self.seat.order(k, order, !low);
    }

    /// A click on the field at `at`: with a drone selected, take the tower there or go there;
    /// else select the drone there.
    pub fn click_field(&mut self, at: Vec2, pick: f32) {
        let (view, rules) = {
            let s = self.seat.state();
            (s.view.clone(), s.rules)
        };
        let (Some(view), Some(rules)) = (view, rules) else { return };
        let drone = view
            .drones
            .iter()
            .filter(|d| d.at.distance(at) <= pick)
            .min_by(|a, b| a.at.distance(at).total_cmp(&b.at.distance(at)));
        match (self.selected, drone) {
            (None, Some(d)) => self.selected = Some(d.k),
            (None, None) => {}
            (Some(_), _) => {
                let tower = view.towers.iter().position(|t| t.at.distance(at) <= rules.circle_radius);
                let order = match tower {
                    Some(t) => Order::Take(t),
                    None => Order::Goto(at.clamp_length_max(rules.field_radius - 40.0)),
                };
                self.give(order);
            }
        }
    }

    fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            k @ ("1" | "2" | "3") => self.select(k.parse::<usize>().ok().map(|n| n - 1)),
            "h" => self.give(Order::Hold),
            "b" => self.give(Order::Home),
            "l" => self.toggle_low(),
            "escape" => self.select(None),
            _ => return,
        }
        cx.notify();
    }

    fn map_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(rules) = self.seat.state().rules else { return };
        let map = MapView::new(self.map_bounds.get(), &rules);
        let at = map.to_field(f(ev.position.x), f(ev.position.y));
        self.click_field(at, PICK / map.scale);
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
    div()
        .flex()
        .flex_col()
        .bg(c(BEZEL))
        .border_1()
        .border_color(c(BEZEL_EDGE))
        .rounded(px(PANEL_RADIUS))
        .p(px(BEZEL_PAD))
        .gap(px(4.0))
        .child(div().px_2().bg(c(TITLE_BG)).text_color(c(GREY)).text_xs().child(title))
        .child(div().flex_1().bg(c(SCREEN)).rounded(px(SCREEN_RADIUS)).overflow_hidden().child(body))
}

impl CommanderView {
    /// The camera of each drone beside its name, and the selected drone's (else the first's) larger.
    fn cameras_panel(&mut self, window: &mut Window, view: &Option<TeamView>, cx: &mut Context<Self>) -> Div {
        // The newest pictures; the ones the window had are freed from its atlas.
        let images: Vec<Option<Arc<RenderImage>>> = (0..3).map(|k| self.cameras.image(k)).collect();
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
        let mut row = div().flex().gap(px(4.0)).p_2();
        for (k, img) in images.iter().enumerate() {
            let label = team.map_or(String::new(), |t| format!("{}{}", &team_name(t)[..1], k + 1));
            row = row.child(
                camera_tile(format!("camera-{k}"), &label, img.clone(), 100.0, 75.0, self.selected == Some(k))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(Some(k));
                        cx.notify();
                    })),
            );
        }
        let k = self.selected.unwrap_or(0);
        let label = team.map_or(String::new(), |t| format!("{}{}", &team_name(t)[..1], k + 1));
        div().flex().flex_col().child(row).child(div().px_2().pb_2().child(camera_tile(
            "camera-selected".to_string(),
            &label,
            images[k].clone(),
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
                        None => text(window, cx, "NO SIGNAL", x + w * 0.5, y + h * 0.5, 10.0, c(GREY), Align::Center),
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
        let bounds = self.map_bounds.clone();
        let selected = self.selected;
        let (v2, r2) = (view.clone(), rules);
        let map_canvas = canvas(
            |_, _, _| {},
            move |b, _, window, cx| {
                bounds.set(b);
                if let (Some(v), Some(r), Some(m)) = (&v2, &r2, &valley) {
                    paint_map(b, v, r, m, selected, window, cx);
                }
                round_corners(window, b, SCREEN_RADIUS - 2.0, c(0x000000));
            },
        )
        .size_full();
        let map = div()
            .id("map")
            .size_full()
            .child(map_canvas)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev, _, cx| this.map_down(ev, cx)));

        let mut drones = div().flex().flex_col().gap(px(4.0)).p_2().text_sm();
        if let Some(v) = &view {
            let tint = theme_team(v.team);
            for d in &v.drones {
                let k = d.k;
                let phase = match d.phase {
                    Phase::Flying => format!("{:.0} m", d.alt),
                    Phase::Ground => "GROUND".into(),
                    Phase::Down => "DOWN".into(),
                };
                let low = if d.low { " LOW" } else { "" };
                let label =
                    format!("{}{}  {:<7} {}{low}", &team_name(v.team)[..1], k + 1, phase, order_text(d.order, v));
                drones = drones.child(
                    button(format!("drone-{k}"), label, selected == Some(k))
                        .border_color(c(if selected == Some(k) { tint } else { BEZEL_EDGE }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select(Some(k));
                            cx.notify();
                        })),
                );
            }
        }
        let actions = div()
            .flex()
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
            .child(button("low", "LOW", selected.is_some_and(|k| self.low_of(k))).on_click(cx.listener(
                |this, _, _, cx| {
                    this.toggle_low();
                    cx.notify();
                },
            )));
        let help = div().p_2().text_xs().text_color(c(GREY)).child(match selected {
            Some(k) => format!(
                "Click a tower to take it, or a point to fly there with drone {}. LOW: along the lanes. Esc: none.",
                k + 1
            ),
            None => "Select a drone: click it, or press 1, 2 or 3.".to_string(),
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
        let right = div()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .w(px(340.0))
            .flex_none()
            .child(screen("DRONES", div().flex().flex_col().child(drones).child(actions).child(help)))
            .child(screen("TOWERS", towers))
            .child(screen("CAMERAS", cameras));

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
    selected: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) {
    let (x, y, w, h) = rect(b);
    let map = MapView::new(b, rules);
    let s = map.scale;
    fill_rect(window, x, y, w, h, c(FOG));
    // What the team sees, as the server would work it out from this map.
    let mut eyes: Vec<(Vec2, f32)> = vec![(rules.base(v.team), rules.base_vision)];
    eyes.extend(v.drones.iter().filter(|d| d.phase != Phase::Down).map(|d| (d.at, rules.drone_vision)));
    eyes.extend(v.towers.iter().filter(|t| t.holder == Some(v.team)).map(|t| (t.at, rules.tower_vision)));
    for (at, range) in &eyes {
        let (sx, sy) = map.to_screen(*at);
        fill_circle(window, sx, sy, range * s, c(FIELD));
    }
    paint_valley(window, map, valley);
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
            dashed(window, &[a, b2], 1.0, 5.0, ca(tint, if selected == Some(d.k) { 0.9 } else { 0.4 }));
        }
    }
    for d in &v.drones {
        let (dx, dy) = map.to_screen(d.at);
        let colour = if d.phase == Phase::Down { ca(GREY, 0.6) } else { c(tint) };
        drone_mark(window, dx, dy, d.vel, colour, true);
        if selected == Some(d.k) {
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
    if let Some(outcome) = v.outcome {
        let banner = match outcome {
            Outcome::Won(t) => format!("{} WINS", team_name(t)),
            Outcome::Draw => "DRAW".into(),
        };
        fill_rect(window, x + w * 0.5 - 140.0, y + h * 0.5 - 30.0, 280.0, 60.0, ca(0x000000, 0.8));
        text(window, cx, &banner, x + w * 0.5, y + h * 0.5, 28.0, c(AMBER), Align::Center);
    }
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
