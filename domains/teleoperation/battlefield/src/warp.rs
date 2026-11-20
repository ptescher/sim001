//! The last picture, moved to the camera now: a pan or a zoom shows in the same frame, though the
//! picture is a few frames old.
//!
//! The app asks for a picture with a margin ([`margin`]), most of all below, where the near ground
//! moves fastest on screen. It draws the picture as the camera now sees the ground ([`strips`]): the
//! ground is flat at height 0, as the map on it is. A camera that only moved or zoomed sees the
//! ground's rows as rows, so each band of the battlefield is the picture moved and scaled, which
//! GPUI draws ([`paint`]). A turn cannot be drawn this way: the battlefield turns when its picture
//! comes ([`shown`]).

use crate::lens::Lens;
use gpui::{Bounds, ContentMask, Corners, RenderImage, Window, point, px, size};
use std::sync::Arc;
use teleoperation_shared::battlefield::{Eye, MAX_SIZE};

/// The margin of the picture round the battlefield, as parts of its height (above and below) and
/// of its width (each side). At full speed the camera pans 0.15 s of it before the picture is
/// short: the near ground at the bottom moves furthest.
const ABOVE: f32 = 0.08;
const BELOW: f32 = 0.35;
const SIDES: f32 = 0.10;

/// The bands the battlefield is drawn in: each is off the flat ground by under a pixel.
pub const STRIPS: usize = 64;

/// The picture to ask for, for the camera `eye` on a battlefield `w` by `h`: its eye and its size.
/// It is as sharp as the battlefield in the middle, and sees the margin round it: the camera looks
/// a little lower and wider from where it is.
pub fn margin(eye: Eye, w: f32, h: f32) -> (Eye, f32, f32) {
    let f = h * 0.5 / (eye.vfov.to_radians() * 0.5).tan();
    let (up, down) = (((0.5 + ABOVE) * h / f).atan(), ((0.5 + BELOW) * h / f).atan());
    let ph = (2.0 * f * ((up + down) * 0.5).tan()).round();
    let vfov = 2.0 * (ph * 0.5 / f).atan();
    let pitch = eye.pitch + ((down - up) * 0.5).to_degrees();
    // From where the camera is, to the ground in the middle of the new picture.
    let at = eye.position();
    let distance = at.z / pitch.to_radians().sin();
    let wide = Eye { pitch, vfov: vfov.to_degrees(), distance, ..eye };
    let look = (at - wide.axes().0 * -distance).truncate();
    let pw = (w * (1.0 + 2.0 * SIDES)).round();
    // The 3D view renders no larger than MAX_SIZE: less sharp, but it sees as much.
    let k = (MAX_SIZE.0 as f32 / pw).min(MAX_SIZE.1 as f32 / ph).min(1.0);
    (Eye { look, ..wide }, (pw * k).floor(), (ph * k).floor())
}

/// The camera the battlefield is drawn from: the camera now, turned as its picture is (a turn
/// shows when its picture comes).
pub fn shown(now: Eye, picture: Eye) -> Eye {
    Eye { heading: picture.heading, ..now }
}

/// A band of the battlefield, from `top` to `bottom` on screen, and where the whole picture is
/// drawn for it: (x, y, w, h).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    pub top: f32,
    pub bottom: f32,
    pub at: (f32, f32, f32, f32),
}

/// The picture from `from` (`pw` by `ph`) as the battlefield `lens` sees the flat ground, in `n`
/// bands; `None` if the camera turned since, or a band shows the sky.
pub fn strips(from: Eye, pw: f32, ph: f32, lens: Lens, n: usize) -> Option<Vec<Strip>> {
    if (lens.eye.heading - from.heading).rem_euclid(360.0).min((from.heading - lens.eye.heading).rem_euclid(360.0))
        > 1e-3
    {
        return None;
    }
    let (x, y, w, h) = lens.rect();
    // Where the screen point (sx, sy) of the ground is in the picture.
    let picture = |sx: f32, sy: f32| from.project(lens.to_field(sx, sy)?.extend(0.0), pw, ph);
    (0..n)
        .map(|i| {
            let (top, bottom) = (y + h * i as f32 / n as f32, y + h * (i + 1) as f32 / n as f32);
            let mid = 0.5 * (top + bottom);
            // Across: the row in the middle of the band, at two points.
            let (xa, xb) = (x + w * 0.25, x + w * 0.75);
            let (a, b) = (picture(xa, mid)?, picture(xb, mid)?);
            let sx = (xb - xa) / (b.0 - a.0);
            // Down: the band's top and bottom.
            let (t, u) = (picture(x + w * 0.5, top)?.1, picture(x + w * 0.5, bottom)?.1);
            let sy = (bottom - top) / (u - t);
            Some(Strip { top, bottom, at: (xa - a.0 * sx, top - t * sy, pw * sx, ph * sy) })
        })
        .collect()
}

/// Draws `image`, the picture from `from`, on the battlefield `lens` sees: moved band by band, or
/// as it is if the camera turned since.
pub fn paint(window: &mut Window, image: &Arc<RenderImage>, from: Eye, lens: Lens) {
    let s = image.size(0);
    let (pw, ph) = (s.width.0 as f32, s.height.0 as f32);
    let (x, y, w, h) = lens.rect();
    let rect = |x: f32, y: f32, w: f32, h: f32| Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) };
    match strips(from, pw, ph, lens, STRIPS) {
        Some(strips) => {
            for s in strips {
                let mask = ContentMask { bounds: rect(x, s.top, w, s.bottom - s.top) };
                window.with_content_mask(Some(mask), |window| {
                    let _ = window.paint_image(
                        rect(s.at.0, s.at.1, s.at.2, s.at.3),
                        Corners::default(),
                        image.clone(),
                        0,
                        false,
                    );
                });
            }
        }
        None => {
            let _ = window.paint_image(rect(x, y, w, h), Corners::default(), image.clone(), 0, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use gpui::{Bounds, point, px, size};

    const W: f32 = 1400.0;
    const H: f32 = 630.0;

    fn eye() -> Eye {
        Eye { look: Vec2::new(-300.0, 40.0), heading: 30.0, pitch: 50.0, distance: 450.0, vfov: 40.0 }
    }

    fn lens(eye: Eye) -> Lens {
        Lens::new(eye, Bounds { origin: point(px(100.0), px(40.0)), size: size(px(W), px(H)) })
    }

    /// Where the strips draw the ground point `g` of the picture from `from` (`pw` by `ph`): by the
    /// band it falls in (at a seam between two bands, a point may fall a hair outside both).
    fn drawn(strips: &[Strip], from: Eye, pw: f32, ph: f32, g: Vec2) -> Option<(f32, f32)> {
        let (u, v) = from.project(g.extend(0.0), pw, ph)?;
        let outside = |s: &Strip, y: f32| (s.top - y).max(y - s.bottom).max(0.0);
        strips
            .iter()
            .map(|s| ((s.at.0 + u * s.at.2 / pw, s.at.1 + v * s.at.3 / ph), s))
            .min_by(|(a, s), (b, t)| outside(s, a.1).total_cmp(&outside(t, b.1)))
            .filter(|((_, y), s)| outside(s, *y) < 0.5)
            .map(|(p, _)| p)
    }

    /// Ground points all over the battlefield, from the camera `eye`.
    fn grid(eye: Eye) -> Vec<Vec2> {
        let l = lens(eye);
        let mut pts = vec![];
        for i in 0..=14 {
            for j in 0..=8 {
                let (x, y) = (100.0 + W * (0.02 + 0.96 * i as f32 / 14.0), 40.0 + H * (0.02 + 0.96 * j as f32 / 8.0));
                pts.push(l.to_field(x, y).unwrap());
            }
        }
        pts
    }

    /// The largest distance between where the strips draw the ground and where the map is on it.
    fn worst(from: Eye, pw: f32, ph: f32, now: Eye) -> f32 {
        let l = lens(now);
        let strips = strips(from, pw, ph, l, STRIPS).expect("no turn");
        grid(now)
            .into_iter()
            .map(|g| {
                let (dx, dy) = drawn(&strips, from, pw, ph, g).expect("drawn");
                let (mx, my) = l.ground(g).unwrap();
                (dx - mx).hypot(dy - my)
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_picture_asked_for_has_a_margin_most_of_all_below() {
        let (from, pw, ph) = margin(eye(), W, H);
        let l = lens(eye());
        let at = |x: f32, y: f32| from.project(l.to_field(x, y).unwrap().extend(0.0), pw, ph).unwrap();
        let (top, bottom) = (at(100.0 + W * 0.5, 40.0).1, ph - at(100.0 + W * 0.5, 40.0 + H).1);
        assert!(top > 0.05 * H && bottom > 0.25 * H && bottom > 2.5 * top, "top {top}, bottom {bottom}");
        for (x, y) in [(100.0, 40.0), (100.0 + W, 40.0), (100.0, 40.0 + H), (100.0 + W, 40.0 + H)] {
            let (u, _) = at(x, y);
            assert!(u > 0.05 * W && u < pw - 0.05 * W, "a corner at {u} of {pw}");
        }
        // As sharp as the battlefield in the middle: one pixel of it is one of the picture.
        let (a, b) = (at(100.0 + W * 0.5, 40.0 + H * 0.5), at(101.0 + W * 0.5, 40.0 + H * 0.5));
        assert!((b.0 - a.0 - 1.0).abs() < 0.05, "{}", b.0 - a.0);
    }

    #[test]
    fn a_picture_too_large_for_the_view_is_smaller_but_sees_as_much() {
        let (from, pw, ph) = margin(eye(), 2400.0, 1500.0);
        assert!(pw <= MAX_SIZE.0 as f32 && ph <= MAX_SIZE.1 as f32, "{pw} by {ph}");
        let (wide, ..) = margin(eye(), W, H);
        assert!((from.vfov - wide.vfov).abs() < 0.5 && (from.pitch - wide.pitch).abs() < 0.5);
    }

    #[test]
    fn at_rest_the_picture_lies_on_the_map_exactly() {
        let (from, pw, ph) = margin(eye(), W, H);
        assert!(worst(from, pw, ph, eye()) < 0.5);
    }

    #[test]
    fn after_a_pan_or_a_zoom_it_lies_on_the_map_within_a_pixel() {
        let (from, pw, ph) = margin(eye(), W, H);
        // 0.15 s of a pan at full speed, each way, and a step of the wheel.
        let speed = eye().height() * 1.3 * 0.15;
        let h = eye().heading.to_radians();
        let (ahead, side) = (Vec2::new(h.cos(), h.sin()), Vec2::new(-h.sin(), h.cos()));
        for moved in [ahead, -ahead, side, -side] {
            let now = Eye { look: eye().look + moved * speed, ..eye() };
            assert!(worst(from, pw, ph, now) < 1.0, "{moved}: {}", worst(from, pw, ph, now));
        }
        let nearer = Eye { distance: eye().distance * 0.8, ..eye() };
        assert!(worst(from, pw, ph, nearer) < 1.0);
    }

    #[test]
    fn the_strips_cover_the_battlefield_after_a_pan() {
        let (from, pw, ph) = margin(eye(), W, H);
        let h = eye().heading.to_radians();
        let back = -glam::Vec2::new(h.cos(), h.sin()) * eye().height() * 1.3 * 0.15;
        let now = Eye { look: eye().look + back, ..eye() };
        let strips = strips(from, pw, ph, lens(now), STRIPS).unwrap();
        assert_eq!((strips[0].top, strips.last().unwrap().bottom), (40.0, 40.0 + H));
        for (a, b) in strips.iter().zip(&strips[1..]) {
            assert_eq!(a.bottom, b.top);
        }
        for s in &strips {
            let (x, y, w, h) = s.at;
            assert!(x <= 100.0 && x + w >= 100.0 + W && y <= s.top && y + h >= s.bottom, "{s:?}");
        }
    }

    #[test]
    fn a_turn_waits_for_its_picture() {
        let now = Eye { heading: 75.0, look: Vec2::new(-280.0, 40.0), ..eye() };
        let shown = shown(now, eye());
        assert_eq!((shown.heading, shown.look), (eye().heading, now.look));
        assert!(strips(eye(), W, H, lens(now), STRIPS).is_none(), "a turn is not drawn moved");
    }
}
