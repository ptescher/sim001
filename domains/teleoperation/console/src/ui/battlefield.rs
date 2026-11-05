//! The battlefield view of the console: the place in 3D from the operator's camera (the sim's
//! picture, or `sim001_view`'s for a real flight; see `teleoperation_battlefield`), with what the
//! tactical map shows drawn on it in perspective: home, the mission on the vehicle, the plan, the
//! GUIDED target, the ground track, the aircraft and the traffic. Heights are above home.

use super::map::{LOITER_RADIUS, MapData};
use crate::geo::TerrainMap;
use crate::vehicle::{Waypoint, WpKind};
use glam::Vec2;
use gpui::{App, Bounds, ContentMask, Corners, Hsla, Pixels, RenderImage, Window};
use std::sync::Arc;
use teleoperation_battlefield::draw::*;
use teleoperation_battlefield::theme::{CONSOLE, c, ca};
use teleoperation_battlefield::{Flat, Lens, paint};

use super::theme::{AMBER, CYAN, GREEN, GREY, MAGENTA, SPI, WHITE};

/// The rings round home on the battlefield (m).
const RINGS: [f32; 3] = [250.0, 500.0, 1000.0];

/// What the battlefield draws, besides the map's snapshot.
pub struct Scene<'a> {
    pub picture: Option<Arc<RenderImage>>,
    /// The aircraft's height above home (m), and its velocity (north, east; m/s).
    pub alt: f32,
    pub velocity: Vec2,
    /// The plan being edited (north, east of the origin, and the height above home).
    pub plan: &'a [(WpKind, Vec2, f32)],
    pub measure: &'a [(f32, f32)],
    pub survey: &'a [(f32, f32)],
}

/// The ground under a field point, above home (m): from the sim's exported heights.
pub fn ground(terrain: Option<&TerrainMap>, at: Vec2) -> f32 {
    terrain.and_then(|t| t.height(at.x, at.y)).unwrap_or(0.0)
}

fn v2(p: (f32, f32)) -> Vec2 {
    Vec2::new(p.0, p.1)
}

/// The plan as the battlefield draws it.
pub fn plan_points(plan: &[Waypoint], origin: crate::geo::Origin) -> Vec<(WpKind, Vec2, f32)> {
    plan.iter().map(|w| (w.kind, v2(origin.to_ne(w.lat, w.lon)), w.alt)).collect()
}

pub fn paint_battlefield(
    b: Bounds<Pixels>,
    lens: Lens,
    data: &MapData,
    scene: &Scene,
    terrain: Option<&TerrainMap>,
    window: &mut Window,
    cx: &mut App,
) {
    let (x, y, w, h) = rect(b);
    let g = |at: Vec2| ground(terrain, at);
    let on_ground = |at: Vec2| lens.at(at.extend(g(at)));
    window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
        match &scene.picture {
            Some(img) => {
                let _ = window.paint_image(b, Corners::default(), img.clone(), 0, false);
            }
            None => {
                // The plain ground: a grid of 100 m squares round home, in the same perspective.
                fill_rect(window, x, y, w, h, c(CONSOLE));
                for k in -15..=15 {
                    let d = k as f32 * 100.0;
                    for (a, z) in
                        [(Vec2::new(d, -1500.0), Vec2::new(d, 1500.0)), (Vec2::new(-1500.0, d), Vec2::new(1500.0, d))]
                    {
                        let pts: Vec<_> = (0..=30).filter_map(|i| lens.ground(a.lerp(z, i as f32 / 30.0))).collect();
                        polyline(window, &pts, 1.0, ca(GREY, if k == 0 { 0.35 } else { 0.15 }));
                    }
                }
            }
        }
        // Home, and the rings round it.
        if let Some(home) = data.home.map(v2) {
            for r in RINGS {
                let ring: Option<Vec<_>> = (0..=64)
                    .map(|i| {
                        let a = i as f32 / 64.0 * std::f32::consts::TAU;
                        on_ground(home + Vec2::new(a.cos(), a.sin()) * r)
                    })
                    .collect();
                if let Some(ring) = ring {
                    dashed(window, &ring, 1.0, 6.0, ca(WHITE, 0.3));
                }
            }
            if let Some((hx, hy)) = on_ground(home) {
                fill_circle(window, hx, hy, 6.0, ca(GREEN, 0.9));
                circle(window, hx, hy, 9.0, 1.5, c(WHITE));
                paint::label(window, cx, "HOME", hx, hy + 18.0, c(GREEN));
            }
        }
        // The ground track.
        let trail: Vec<_> = data.trail.iter().filter_map(|&p| on_ground(v2(p))).collect();
        polyline(window, &trail, 1.5, ca(CYAN, 0.5));
        // The mission on the vehicle: a solid line at height, a pin for each item.
        let items: Vec<(usize, WpKind, Vec2, f32)> =
            data.mission.iter().enumerate().filter_map(|(i, (k, at, alt))| at.map(|a| (i, *k, v2(a), *alt))).collect();
        route(window, cx, lens, terrain, &items, ca(WHITE, 0.85), false, Some(data.mission_seq as usize));
        // The plan being edited: dashed.
        let plan: Vec<(usize, WpKind, Vec2, f32)> =
            scene.plan.iter().enumerate().map(|(i, (k, at, alt))| (i + 1, *k, *at, *alt)).collect();
        route(window, cx, lens, terrain, &plan, ca(AMBER, 0.95), true, None);
        // The GUIDED target: a circle at the aircraft's height.
        if let Some(t) = data.guided.map(v2) {
            loiter(window, lens, t, scene.alt.max(g(t) + 10.0), ca(MAGENTA, 0.9));
            pin(window, lens, terrain, t, scene.alt, c(MAGENTA));
        }
        // What the camera of the aircraft looks at.
        for (p, colour) in [(data.spi, SPI), (data.track, MAGENTA)] {
            if let Some((sx, sy)) = p.and_then(|p| on_ground(v2(p))) {
                polyline(window, &[(sx - 8.0, sy), (sx + 8.0, sy)], 1.5, c(colour));
                polyline(window, &[(sx, sy - 8.0), (sx, sy + 8.0)], 1.5, c(colour));
            }
        }
        // Measuring and the area of a survey grid, on the ground.
        let pts = |v: &[(f32, f32)]| -> Vec<(f32, f32)> { v.iter().filter_map(|&p| on_ground(v2(p))).collect() };
        if scene.measure.len() == 2 {
            let m = pts(scene.measure);
            polyline(window, &m, 2.0, c(AMBER));
            if let [a, z] = scene.measure {
                let d = v2(*a).distance(v2(*z));
                if let Some((mx, my)) = on_ground((v2(*a) + v2(*z)) * 0.5) {
                    paint::label(window, cx, &format!("{d:.0} m"), mx, my - 12.0, c(AMBER));
                }
            }
        }
        if !scene.survey.is_empty() {
            let mut s = pts(scene.survey);
            if s.len() > 2 {
                s.push(s[0]);
            }
            dashed(window, &s, 1.5, 5.0, c(AMBER));
        }
        // The other aircraft that the radio tells the station about.
        for t in &data.traffic {
            let colour = if t.stale { ca(GREY, 0.6) } else { c(AMBER) };
            let at = v2(t.at);
            if let Some((ax, ay)) =
                paint::vehicle(window, lens, at, g(at) + 30.0, v2(t.velocity), colour, !t.stale, false)
            {
                text(window, cx, &t.name, ax + 14.0, ay - 10.0, 10.0, colour, Align::Left);
            }
        }
        // The aircraft.
        if let Some(at) = data.aircraft.map(v2) {
            let colour = if data.alive { c(CYAN) } else { ca(GREY, 0.7) };
            if let Some((ax, ay)) = paint::vehicle(window, lens, at, scene.alt, scene.velocity, colour, true, true) {
                text(
                    window,
                    cx,
                    &format!("{} {:.0} m", data.mode, scene.alt),
                    ax + 16.0,
                    ay - 10.0,
                    10.0,
                    colour,
                    Align::Left,
                );
            }
        }
    });
}

/// A route: a line between its items at their heights, and a pin for each (the current one magenta).
#[allow(clippy::too_many_arguments)]
fn route(
    window: &mut Window,
    cx: &mut App,
    lens: Lens,
    terrain: Option<&TerrainMap>,
    items: &[(usize, WpKind, Vec2, f32)],
    colour: Hsla,
    dashes: bool,
    current: Option<usize>,
) {
    let line: Vec<_> = items.iter().filter_map(|(_, _, at, alt)| lens.at(at.extend(*alt))).collect();
    if dashes {
        dashed(window, &line, 2.0, 7.0, colour);
    } else {
        polyline(window, &line, 2.0, colour);
    }
    for &(i, kind, at, alt) in items {
        let here = if current == Some(i) { c(MAGENTA) } else { colour };
        if matches!(kind, WpKind::LoiterUnlim | WpKind::LoiterTurns) {
            loiter(window, lens, at, alt, here);
        }
        if let Some((px_, py_)) = pin(window, lens, terrain, at, alt, here) {
            let name = match kind {
                WpKind::Takeoff => "TKOF".to_string(),
                WpKind::Rtl => "RTL".into(),
                WpKind::Land => "LAND".into(),
                _ => format!("{i}"),
            };
            text(window, cx, &name, px_ + 8.0, py_ - 8.0, 10.0, here, Align::Left);
        }
    }
}

/// A point in the air on a line down to the ground. Returns where the point is on screen.
fn pin(
    window: &mut Window,
    lens: Lens,
    terrain: Option<&TerrainMap>,
    at: Vec2,
    alt: f32,
    colour: Hsla,
) -> Option<(f32, f32)> {
    let top = lens.at(at.extend(alt))?;
    if let Some(foot) = lens.at(at.extend(ground(terrain, at))) {
        polyline(window, &[foot, top], 1.0, ca(0x000000, 0.5));
        dashed(window, &[foot, top], 0.8, 3.0, colour);
        fill_circle(window, foot.0, foot.1, 2.5, ca(0x000000, 0.6));
    }
    fill_circle(window, top.0, top.1, 4.5, colour);
    circle(window, top.0, top.1, 4.5, 1.0, ca(0x000000, 0.8));
    Some(top)
}

/// A loiter circle at height `alt` round `at`.
fn loiter(window: &mut Window, lens: Lens, at: Vec2, alt: f32, colour: Hsla) {
    let ring: Option<Vec<_>> = (0..=48)
        .map(|i| {
            let a = i as f32 / 48.0 * std::f32::consts::TAU;
            lens.at((at + Vec2::new(a.cos(), a.sin()) * LOITER_RADIUS).extend(alt))
        })
        .collect();
    if let Some(ring) = ring {
        dashed(window, &ring, 1.5, 5.0, colour);
    }
}

/// The minimap: the orthophoto, home, the mission, the aircraft, and the area that the camera sees.
pub fn paint_minimap(
    window: &mut Window,
    b: Bounds<Pixels>,
    flat: Flat,
    data: &MapData,
    terrain: Option<&TerrainMap>,
    image: Option<&Arc<RenderImage>>,
    lens: Lens,
) {
    let (mx, my, mw, mh) = rect(b);
    window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
        fill_rect(window, mx, my, mw, mh, c(CONSOLE));
        if let (Some(t), Some(img)) = (terrain, image) {
            let half = t.size_m * 0.5;
            let (left, top) = flat.to_screen(Vec2::new(t.centre_n + half, t.centre_e - half));
            let side = t.size_m * flat.scale;
            let at = Bounds { origin: pt(left, top), size: gpui::size(gpui::px(side), gpui::px(side)) };
            let _ = window.paint_image(at, Corners::default(), img.clone(), 0, false);
        }
        let line: Vec<_> = data.mission.iter().filter_map(|(_, at, _)| at.map(|a| flat.to_screen(v2(a)))).collect();
        polyline(window, &line, 1.5, ca(WHITE, 0.8));
        if let Some((hx, hy)) = data.home.map(|h| flat.to_screen(v2(h))) {
            fill_circle(window, hx, hy, 3.0, c(GREEN));
        }
        if let Some((ax, ay)) = data.aircraft.map(|a| flat.to_screen(v2(a))) {
            fill_circle(window, ax, ay, 3.5, c(CYAN));
        }
        paint::minimap_view(window, b, flat, lens, 3000.0);
    });
}
