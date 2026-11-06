//! The battlefield and the minimap: what the app draws of the team's map, on the 3D view's picture
//! of the valley (or on the plain map of it), and on the minimap.

use crate::ground::Ground;
use glam::{Vec2, Vec3};
use gpui::{App, Bounds, ContentMask, Corners, Pixels, RenderImage, Window, px};
use recreation_towers::map::{LANE_HALF_WIDTH, Map};
use recreation_towers::{Kind, Order, Outcome, Phase, Rules, Team, TeamView};
use std::sync::Arc;
use std::time::Instant;
use teleoperation_battlefield::draw::*;
use teleoperation_battlefield::theme::*;
use teleoperation_battlefield::{Flat, Lens, paint};

/// A box being dragged: where the button went down, where the mouse is now (screen pixels), and
/// whether Shift was held.
pub type Drag = ((f32, f32), (f32, f32), bool);

/// How high the top of a tower's mast is drawn (m).
const MAST: f32 = 40.0;
/// The side of a cell of the fog (m).
const FOG_CELL: f32 = 15.0;
/// How dark the fog is over the picture.
const FOG_DARK: f32 = 0.62;
/// How long the mark of a right-click lasts (s).
pub const PING: f32 = 0.7;

pub fn team_name(t: Team) -> &'static str {
    match t {
        Team::Blue => "BLUE",
        Team::Red => "RED",
    }
}

/// The colour of team `t`.
pub fn team_colour(t: Team) -> u32 {
    match t {
        Team::Blue => BLUE,
        Team::Red => RED,
    }
}

fn tint(t: Team) -> u32 {
    team_colour(t)
}

/// What the battlefield shows besides the team's map.
pub struct Scene<'a> {
    pub picture: Option<Arc<RenderImage>>,
    pub selected: &'a [usize],
    pub drag: Option<Drag>,
    /// Where the last right-click ordered the drones to, and when.
    pub ping: Option<(Vec2, Instant)>,
    /// The routes of the drones that have one.
    pub routes: &'a std::collections::HashMap<usize, Vec<Vec2>>,
    /// CIRCLE while it is dragged: its centre and its distance.
    pub circle: Option<(Vec2, f32)>,
}

/// The battlefield: the picture of the valley (or the plain map), the fog, the towers, the drones
/// and the orders, through `lens`.
#[allow(clippy::too_many_arguments)]
pub fn paint_battlefield(
    b: Bounds<Pixels>,
    lens: Lens,
    v: &TeamView,
    rules: &Rules,
    valley: &Map,
    scene: &Scene,
    window: &mut Window,
    cx: &mut App,
) {
    let (x, y, w, h) = rect(b);
    window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
        match &scene.picture {
            Some(img) => {
                let _ = window.paint_image(b, Corners::default(), img.clone(), 0, false);
            }
            None => {
                fill_rect(window, x, y, w, h, c(FOG));
                paint_plain_valley(window, lens, rules, valley);
            }
        }
        paint_fog(window, lens, v, rules);
        if let Some(ring) = lens.ring(Vec2::ZERO, rules.field_radius) {
            dashed(window, &ring, 1.0, 8.0, ca(WHITE, 0.35));
        }
        for team in Team::BOTH {
            let base = rules.base(team);
            let pad = [(-25.0, -25.0), (-25.0, 25.0), (25.0, 25.0), (25.0, -25.0)]
                .map(|(n, e)| lens.ground(base + Vec2::new(n, e)));
            if let Some(pts) = pad.into_iter().collect::<Option<Vec<_>>>() {
                fill_poly(window, &pts, ca(tint(team), 0.35));
                let mut closed = pts.clone();
                closed.push(pts[0]);
                polyline(window, &closed, 1.5, ca(tint(team), 0.9));
            }
            if let Some((bx, by)) = lens.ground(base) {
                paint::label(window, cx, &format!("{} BASE", team_name(team)), bx, by + 18.0, ca(tint(team), 0.95));
            }
        }
        paint_tower_grounds(window, lens, v, rules);
        paint_orders(window, lens, v, rules, scene.selected);
        paint_routes(window, cx, lens, v, scene.routes);
        if let Some((centre, r)) = scene.circle {
            paint::circle_preview(window, cx, lens, centre, r, &|_| 0.0);
        }
        paint_drones(window, cx, lens, v, scene.selected);
        paint_towers(window, cx, lens, v);
        if let Some((at, when)) = scene.ping {
            let t = when.elapsed().as_secs_f32() / PING;
            if t < 1.0
                && let Some(ring) = lens.ring(at, 8.0 + 22.0 * t)
            {
                polyline(window, &ring, 2.0, ca(GREEN, 1.0 - t));
            }
        }
        if let Some(((x0, y0), (x1, y1), _)) = scene.drag {
            let (bx, by, bw, bh) = (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
            fill_rect(window, bx, by, bw, bh, ca(GREEN, 0.08));
            stroke_rect(window, bx, by, bw, bh, 1.0, ca(GREEN, 0.9));
        }
        if let Some(outcome) = v.outcome {
            let banner = match outcome {
                Outcome::Won(t) => format!("{} WINS", team_name(t)),
                Outcome::Draw => "DRAW".into(),
            };
            fill_rect(window, x + w * 0.5 - 160.0, y + h * 0.4 - 34.0, 320.0, 68.0, ca(0x000000, 0.8));
            text(window, cx, &banner, x + w * 0.5, y + h * 0.4, 30.0, c(AMBER), Align::Center);
        }
    });
}

/// The plain map of the valley in perspective: the ground, the lake, the lanes and the clearings.
fn paint_plain_valley(window: &mut Window, lens: Lens, rules: &Rules, valley: &Map) {
    if let Some(ring) = lens.ring(Vec2::ZERO, rules.field_radius * 1.15) {
        fill_poly(window, &ring, c(FIELD));
    }
    if let Some(ring) = lens.ring(valley.lake.at, valley.lake.radius) {
        fill_poly(window, &ring, ca(LAKE, 0.9));
    }
    for lane in &valley.lanes {
        for seg in lane.points.windows(2) {
            let (a, z) = (seg[0], seg[1]);
            let n = (z - a).perp().normalize_or_zero() * LANE_HALF_WIDTH;
            if let Some(q) =
                [a + n, z + n, z - n, a - n].map(|p| lens.ground(p)).into_iter().collect::<Option<Vec<_>>>()
            {
                fill_poly(window, &q, ca(LANE, 0.85));
            }
        }
    }
    for cl in &valley.clearings {
        if let Some(ring) = lens.ring(cl.at, cl.radius) {
            fill_poly(window, &ring, ca(LANE, 0.85));
        }
    }
}

/// Where the team sees: round its base, its drones and the towers it holds.
fn eyes(v: &TeamView, rules: &Rules) -> Vec<(Vec2, f32)> {
    let mut eyes: Vec<(Vec2, f32)> = vec![(rules.base(v.team), rules.base_vision)];
    eyes.extend(v.drones.iter().filter(|d| d.phase != Phase::Down).map(|d| (d.at, rules.drone_vision)));
    eyes.extend(v.towers.iter().filter(|t| t.holder == Some(v.team)).map(|t| (t.at, rules.vision(t.kind))));
    eyes
}

/// The fog: the ground that the team does not see is dark, with a soft edge. The cells of a row
/// that are equally dark are drawn as one strip.
fn paint_fog(window: &mut Window, lens: Lens, v: &TeamView, rules: &Rules) {
    let eyes = eyes(v, rules);
    let extent = rules.field_radius * 1.6;
    let n = (2.0 * extent / FOG_CELL).ceil() as i32;
    let level = |p: Vec2| -> u8 {
        let d = eyes.iter().map(|(at, r)| p.distance(*at) - r).fold(f32::MAX, f32::min);
        let t = ((d + 20.0) / 40.0).clamp(0.0, 1.0);
        (t * 8.0).round() as u8
    };
    for i in 0..n {
        let north = -extent + i as f32 * FOG_CELL;
        let mut j = 0;
        while j < n {
            let east = -extent + j as f32 * FOG_CELL;
            let l = level(Vec2::new(north + FOG_CELL * 0.5, east + FOG_CELL * 0.5));
            let mut run = 1;
            while j + run < n
                && run < 8
                && level(Vec2::new(north + FOG_CELL * 0.5, east + (run as f32 + 0.5) * FOG_CELL)) == l
            {
                run += 1;
            }
            if l > 0 {
                let e1 = east + run as f32 * FOG_CELL;
                let corners = [
                    Vec2::new(north, east),
                    Vec2::new(north + FOG_CELL, east),
                    Vec2::new(north + FOG_CELL, e1),
                    Vec2::new(north, e1),
                ];
                if let Some(q) = corners.map(|p| lens.ground(p)).into_iter().collect::<Option<Vec<_>>>() {
                    fill_poly(window, &q, ca(FOG, FOG_DARK * l as f32 / 8.0));
                }
            }
            j += run;
        }
    }
}

/// On the ground at each tower: the radio shadow of an enemy tower, and the circle to hold.
fn paint_tower_grounds(window: &mut Window, lens: Lens, v: &TeamView, rules: &Rules) {
    let flash = (v.time * 3.0) as i64 % 2 == 0;
    for t in &v.towers {
        let colour = t.holder.map_or(GREY, tint);
        let alpha = if t.seen { 1.0 } else { 0.55 };
        if t.holder == Some(v.team.other())
            && let Some(ring) = lens.ring(t.at, rules.shadow(t.kind))
        {
            fill_poly(window, &ring, ca(tint(v.team.other()), 0.08 * alpha));
            dashed(window, &ring, 1.0, 5.0, ca(tint(v.team.other()), 0.6 * alpha));
        }
        if let Some(ring) = lens.ring(t.at, rules.circle_radius) {
            if t.contested && flash {
                polyline(window, &ring, 3.0, c(AMBER));
            } else {
                fill_poly(window, &ring, ca(colour, 0.12 * alpha));
                polyline(window, &ring, 1.5, ca(colour, 0.75 * alpha));
            }
        }
    }
}

/// The towers: a mast with a light at the top, the control bar and the name over it.
fn paint_towers(window: &mut Window, cx: &mut App, lens: Lens, v: &TeamView) {
    for t in &v.towers {
        let (Some(foot), Some(top)) = (lens.ground(t.at), lens.at(t.at.extend(MAST))) else { continue };
        let colour = t.holder.map_or(GREY, tint);
        let alpha = if t.seen { 1.0 } else { 0.55 };
        polyline(window, &[foot, top], 3.0, ca(0x000000, 0.6 * alpha));
        polyline(window, &[foot, top], 1.5, ca(WHITE, 0.85 * alpha));
        fill_circle(window, top.0, top.1, 9.0, ca(colour, 0.35 * alpha));
        fill_circle(window, top.0, top.1, 5.0, ca(colour, alpha));
        circle(window, top.0, top.1, 5.0, 1.0, ca(WHITE, 0.8 * alpha));
        // The control bar: right of the middle towards Blue, left towards Red.
        let (bw, by) = (48.0, top.1 - 20.0);
        fill_rect(window, top.0 - bw * 0.5 - 1.0, by - 1.0, bw + 2.0, 7.0, ca(0x000000, 0.8));
        let filled = t.control.clamp(-1.0, 1.0) * bw * 0.5;
        let (from, bar) = if filled >= 0.0 { (top.0, BLUE) } else { (top.0 + filled, RED) };
        fill_rect(window, from, by, filled.abs(), 5.0, ca(bar, alpha));
        fill_rect(window, top.0 - 0.5, by - 1.0, 1.0, 7.0, c(WHITE));
        let kind = match t.kind {
            Kind::Fort => "FORT",
            Kind::Spire => "SPIRE x2",
            Kind::Depot => "DEPOT",
            Kind::Outpost => "OUTPOST",
        };
        let name = format!("{} {kind}", t.name.to_uppercase());
        let name = if t.seen { name } else { format!("{name} ({:.0} s)", t.age) };
        paint::label(window, cx, &name, top.0, by - 12.0, ca(WHITE, alpha));
    }
}

/// The orders: a dashed line on the ground from each drone to where it is sent.
fn paint_orders(window: &mut Window, lens: Lens, v: &TeamView, rules: &Rules, selected: &[usize]) {
    let colour = tint(v.team);
    for d in v.drones.iter().filter(|d| d.phase != Phase::Down) {
        let to = match d.order {
            Some(Order::Take(t)) => v.towers.get(t).map(|t| t.at),
            Some(Order::Goto(p)) => Some(p),
            Some(Order::Circle(p, r)) => {
                let alpha = if selected.contains(&d.k) { 0.9 } else { 0.4 };
                paint::ground_ring(window, lens, p, r, &|_| 0.0, 1.5, true, ca(colour, alpha));
                Some(p)
            }
            Some(Order::Home) => Some(rules.base(v.team)),
            _ => None,
        };
        let Some(to) = to else { continue };
        if let (Some(a), Some(z)) = (lens.ground(d.at), lens.ground(to)) {
            let alpha = if selected.contains(&d.k) { 0.9 } else { 0.35 };
            dashed(window, &[a, z], 1.5, 6.0, ca(colour, alpha));
            if selected.contains(&d.k) {
                fill_circle(window, z.0, z.1, 3.0, ca(colour, alpha));
            }
        }
    }
}

fn paint_drones(window: &mut Window, cx: &mut App, lens: Lens, v: &TeamView, selected: &[usize]) {
    let own = tint(v.team);
    for d in &v.drones {
        let dim = d.phase == Phase::Down || !d.linked;
        let colour = if d.phase == Phase::Down { ca(GREY, 0.6) } else { ca(own, if dim { 0.5 } else { 1.0 }) };
        let picked = selected.contains(&d.k);
        if let Some((ax, ay)) = paint::vehicle(window, lens, d.at, d.alt, d.vel, colour, !dim, picked) {
            let name = format!("{}{}", &team_name(v.team)[..1], d.k + 1);
            text(window, cx, &name, ax + 14.0, ay - 10.0, 10.0, colour, Align::Left);
        }
    }
    let enemy = tint(v.team.other());
    for e in &v.enemies {
        let name = format!("{}{}", &team_name(v.team.other())[..1], e.k + 1);
        let (colour, name) = if e.seen { (c(enemy), name) } else { (ca(enemy, 0.5), format!("{name} {:.0} s", e.age)) };
        if let Some((ax, ay)) = paint::vehicle(window, lens, e.at, e.alt, e.vel, colour, e.seen, false) {
            text(window, cx, &name, ax + 14.0, ay - 10.0, 10.0, colour, Align::Left);
        }
    }
}

/// The routes: a dashed line from each drone through the points it has still to fly to, numbered.
fn paint_routes(
    window: &mut Window,
    cx: &mut App,
    lens: Lens,
    v: &TeamView,
    routes: &std::collections::HashMap<usize, Vec<Vec2>>,
) {
    for (k, route) in routes {
        let Some(d) = v.drones.get(*k) else { continue };
        let points: Vec<(Vec2, f32, String)> =
            route.iter().enumerate().map(|(i, p)| (*p, d.alt.max(30.0), format!("{}", i + 1))).collect();
        let from = Some(glam::Vec3::new(d.at.x, d.at.y, d.alt));
        paint::route(window, cx, lens, from, &points, &|_| 0.0, ca(tint(v.team), 0.9), true, None);
    }
}

/// Where each drone of the team is on the battlefield (its number, and the screen point).
pub fn drones_on_screen(lens: Lens, v: &TeamView) -> Vec<(usize, (f32, f32))> {
    v.drones.iter().filter_map(|d| Some((d.k, lens.at(Vec3::new(d.at.x, d.at.y, d.alt))?))).collect()
}

/// The minimap: the whole field, north up, with the area that the battlefield shows and the way
/// the camera looks.
pub fn paint_minimap(
    window: &mut Window,
    b: Bounds<Pixels>,
    v: &TeamView,
    rules: &Rules,
    ground: Option<&Ground>,
    lens: Lens,
) {
    let (mx, my, mw, mh) = rect(b);
    let flat = Flat::new(b, rules.field_radius);
    window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
        fill_rect(window, mx, my, mw, mh, c(FOG));
        if let Some(g) = ground {
            let half = g.size_m * 0.5;
            let (left, top) = flat.to_screen(g.centre + Vec2::new(half, -half));
            let side = g.size_m * flat.scale;
            let at = Bounds { origin: pt(left, top), size: gpui::size(px(side), px(side)) };
            let _ = window.paint_image(at, Corners::default(), g.image.clone(), 0, false);
            fill_rect(window, mx, my, mw, mh, ca(FOG, 0.55));
        }
        for (at, range) in eyes(v, rules) {
            let (sx, sy) = flat.to_screen(at);
            fill_circle(window, sx, sy, range * flat.scale, ca(SIGHT, 0.10));
        }
        let (cx0, cy0) = flat.to_screen(Vec2::ZERO);
        circle(window, cx0, cy0, rules.field_radius * flat.scale, 1.0, ca(GREY, 0.6));
        for t in &v.towers {
            let (x, y) = flat.to_screen(t.at);
            fill_rect(window, x - 3.0, y - 3.0, 6.0, 6.0, c(t.holder.map_or(GREY, tint)));
            if t.contested {
                stroke_rect(window, x - 4.5, y - 4.5, 9.0, 9.0, 1.0, c(AMBER));
            }
        }
        for e in v.enemies.iter().filter(|e| e.seen) {
            let (x, y) = flat.to_screen(e.at);
            fill_circle(window, x, y, 2.5, c(tint(v.team.other())));
        }
        for d in v.drones.iter().filter(|d| d.phase != Phase::Down) {
            let (x, y) = flat.to_screen(d.at);
            fill_circle(window, x, y, 2.5, c(tint(v.team)));
        }
        paint::minimap_view(window, b, flat, lens, rules.field_radius * 3.0);
    });
}
