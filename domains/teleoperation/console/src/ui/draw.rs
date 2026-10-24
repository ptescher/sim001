//! Immediate-mode drawing helpers for canvas elements (lines, shapes, text).

use super::theme::{FONT, c};
use gpui::{
    App, Bounds, Hsla, PathBuilder, Pixels, Point, SharedString, TextRun, Window, font, point, px, quad,
    transparent_black,
};

pub fn pt(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

pub fn f(p: Pixels) -> f32 {
    f32::from(p)
}

/// Plain f32 view of a bounds: (x, y, w, h).
pub fn rect(b: Bounds<Pixels>) -> (f32, f32, f32, f32) {
    (f(b.origin.x), f(b.origin.y), f(b.size.width), f(b.size.height))
}

pub fn polyline(window: &mut Window, pts: &[(f32, f32)], width: f32, color: Hsla) {
    if pts.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(pt(pts[0].0, pts[0].1));
    for p in &pts[1..] {
        b.line_to(pt(p.0, p.1));
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

pub fn dashed(window: &mut Window, pts: &[(f32, f32)], width: f32, dash: f32, color: Hsla) {
    if pts.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width)).dash_array(&[px(dash), px(dash * 0.8)]);
    b.move_to(pt(pts[0].0, pts[0].1));
    for p in &pts[1..] {
        b.line_to(pt(p.0, p.1));
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

pub fn line(window: &mut Window, a: (f32, f32), b: (f32, f32), width: f32, color: Hsla) {
    polyline(window, &[a, b], width, color);
}

pub fn fill_poly(window: &mut Window, pts: &[(f32, f32)], color: Hsla) {
    if pts.len() < 3 {
        return;
    }
    let mut b = PathBuilder::fill();
    b.move_to(pt(pts[0].0, pts[0].1));
    for p in &pts[1..] {
        b.line_to(pt(p.0, p.1));
    }
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn circle_pts(cx: f32, cy: f32, r: f32) -> Vec<(f32, f32)> {
    let n = ((r * 0.8) as usize).clamp(16, 96);
    (0..=n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

pub fn circle(window: &mut Window, cx: f32, cy: f32, r: f32, width: f32, color: Hsla) {
    polyline(window, &circle_pts(cx, cy, r), width, color);
}

pub fn dashed_circle(window: &mut Window, cx: f32, cy: f32, r: f32, width: f32, dash: f32, color: Hsla) {
    dashed(window, &circle_pts(cx, cy, r), width, dash, color);
}

pub fn fill_circle(window: &mut Window, cx: f32, cy: f32, r: f32, color: Hsla) {
    fill_poly(window, &circle_pts(cx, cy, r), color);
}

pub fn fill_rect(window: &mut Window, x: f32, y: f32, w: f32, h: f32, color: Hsla) {
    window.paint_quad(quad(
        Bounds { origin: pt(x, y), size: gpui::size(px(w), px(h)) },
        px(0.0),
        color,
        px(0.0),
        transparent_black(),
        Default::default(),
    ));
}

pub fn stroke_rect(window: &mut Window, x: f32, y: f32, w: f32, h: f32, width: f32, color: Hsla) {
    polyline(window, &[(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)], width, color);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Draws one line of monospace text with its vertical centre at `y`. Returns its width.
pub fn text(window: &mut Window, cx: &mut App, s: &str, x: f32, y: f32, size: f32, color: Hsla, align: Align) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    let s: SharedString = s.to_string().into();
    let run =
        TextRun { len: s.len(), font: font(FONT), color, background_color: None, underline: None, strikethrough: None };
    let shaped = window.text_system().shape_line(s, px(size), &[run], None);
    let w = f(shaped.width);
    let x0 = match align {
        Align::Left => x,
        Align::Center => x - w * 0.5,
        Align::Right => x - w,
    };
    let lh = size * 1.25;
    let _ = shaped.paint(pt(x0, y - lh * 0.5), px(lh), window, cx);
    w
}

/// Text on an opaque box, for readouts that sit over moving symbology.
pub fn boxed_text(
    window: &mut Window,
    cx: &mut App,
    s: &str,
    x: f32,
    y: f32,
    size: f32,
    color: Hsla,
    align: Align,
    min_w: f32,
) {
    let w = (s.chars().count() as f32 * size * 0.62).max(min_w);
    let x0 = match align {
        Align::Left => x,
        Align::Center => x - w * 0.5,
        Align::Right => x - w,
    };
    let h = size * 1.5;
    fill_rect(window, x0 - 4.0, y - h * 0.5, w + 8.0, h, c(0x000000));
    stroke_rect(window, x0 - 4.0, y - h * 0.5, w + 8.0, h, 1.2, color);
    text(window, cx, s, x0 + w * 0.5, y, size, color, Align::Center);
}

/// Rotates (x, y) by `a` radians (screen coordinates, y down, positive = clockwise).
pub fn rot(x: f32, y: f32, a: f32) -> (f32, f32) {
    let (s, co) = a.sin_cos();
    (x * co - y * s, x * s + y * co)
}

/// Masks the corners of a canvas outside a rounded rectangle of radius `r` with `color`
/// (GPUI clips children to rectangles, so canvases inside rounded screens need this).
pub fn round_corners(window: &mut Window, b: Bounds<Pixels>, r: f32, color: Hsla) {
    let (x, y, w, h) = rect(b);
    let n = 10;
    for (cx0, cy0, sx, sy) in
        [(x, y, 1.0f32, 1.0f32), (x + w, y, -1.0, 1.0), (x + w, y + h, -1.0, -1.0), (x, y + h, 1.0, -1.0)]
    {
        // The corner square minus the quarter disc: corner point, then the arc.
        let mut pts = vec![(cx0, cy0)];
        for i in 0..=n {
            let a = i as f32 / n as f32 * std::f32::consts::FRAC_PI_2;
            pts.push((cx0 + sx * r * (1.0 - a.cos()), cy0 + sy * r * (1.0 - a.sin())));
        }
        fill_poly(window, &pts, color);
    }
}
