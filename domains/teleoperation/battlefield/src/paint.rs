//! What the battlefield and the minimap show in both apps: labels, the vehicles (a mark in the air
//! on a line from its shadow on the ground) and, on the minimap, the area that the camera sees with
//! a compass.

use crate::draw::*;
use crate::lens::{Flat, Lens};
use crate::theme::*;
use glam::Vec2;
use gpui::{App, Bounds, Hsla, Pixels, Window};

/// A label with a dark backing, centred on (`x`, `y`).
pub fn label(window: &mut Window, cx: &mut App, s: &str, x: f32, y: f32, colour: Hsla) {
    let w = s.len() as f32 * 6.2 + 8.0;
    fill_rect(window, x - w * 0.5, y - 7.0, w, 14.0, ca(0x000000, 0.55));
    text(window, cx, s, x, y, 10.0, colour, Align::Center);
}

/// A vehicle in the air: its shadow on the ground, the line down to it, and its mark (an arrow the
/// way it flies). Returns where the mark is on screen.
#[allow(clippy::too_many_arguments)]
pub fn vehicle(
    window: &mut Window,
    lens: Lens,
    at: Vec2,
    alt: f32,
    vel: Vec2,
    colour: Hsla,
    filled: bool,
    selected: bool,
) -> Option<(f32, f32)> {
    let air = lens.at(at.extend(alt))?;
    if let Some(shadow) = lens.ring(at, 4.0) {
        fill_poly(window, &shadow, ca(0x000000, 0.45));
    }
    if selected && let Some(ring) = lens.ring(at, 16.0) {
        polyline(window, &ring, 1.5, ca(GREEN, 0.9));
    }
    if let Some(foot) = lens.ground(at)
        && alt > 1.0
    {
        dashed(window, &[foot, air], 1.0, 3.0, ca(0x000000, 0.5));
        dashed(window, &[foot, air], 0.6, 3.0, ca(WHITE, 0.6));
    }
    // The way it flies, on screen.
    let dir = if vel.length() > 1.0 { vel.normalize() } else { lens.eye.axes().0.truncate().normalize() };
    let ahead = lens.at((at + dir * 10.0).extend(alt)).unwrap_or(air);
    let (dx, dy) = (ahead.0 - air.0, ahead.1 - air.1);
    let len = dx.hypot(dy).max(1e-3);
    let (fx, fy) = (dx / len, dy / len);
    let (sx, sy) = (-fy, fx);
    let pts = [
        (air.0 + fx * 11.0, air.1 + fy * 11.0),
        (air.0 - fx * 7.0 + sx * 7.0, air.1 - fy * 7.0 + sy * 7.0),
        (air.0 - fx * 3.0, air.1 - fy * 3.0),
        (air.0 - fx * 7.0 - sx * 7.0, air.1 - fy * 7.0 - sy * 7.0),
    ];
    if filled {
        fill_poly(window, &pts, colour);
        polyline(window, &[pts[0], pts[1], pts[2], pts[3], pts[0]], 1.0, ca(0x000000, 0.7));
    } else {
        polyline(window, &[pts[0], pts[1], pts[2], pts[3], pts[0]], 1.2, colour);
    }
    if selected {
        circle(window, air.0, air.1, 14.0, 1.5, c(GREEN));
    }
    Some(air)
}

/// On the minimap: the area that the battlefield shows (cut `far` metres from the camera), and
/// a compass in the corner that points the way the camera looks.
pub fn minimap_view(window: &mut Window, b: Bounds<Pixels>, flat: Flat, lens: Lens, far: f32) {
    let (mx, my, mw, _) = rect(b);
    // The area the battlefield shows.
    let mut quad: Vec<(f32, f32)> = lens.footprint(far).iter().map(|&p| flat.to_screen(p)).collect();
    quad.push(quad[0]);
    polyline(window, &quad, 1.5, ca(WHITE, 0.9));
    // The compass: the way the camera looks.
    let (ax, ay, r) = (mx + mw - 16.0, my + 16.0, 10.0);
    fill_circle(window, ax, ay, r + 3.0, ca(0x000000, 0.6));
    let hd = lens.eye.heading.to_radians();
    let (fx, fy) = (hd.sin(), -hd.cos());
    fill_poly(
        window,
        &[(ax + fx * r, ay + fy * r), (ax - fy * 4.0, ay + fx * 4.0), (ax + fy * 4.0, ay - fx * 4.0)],
        c(AMBER),
    );
}
