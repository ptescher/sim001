//! Where the field is on screen: the battlefield through the commander's camera (`Lens`), and the
//! minimap from straight above, north up (`Flat`).

use crate::draw::rect;
use glam::{Vec2, Vec3};
use gpui::{Bounds, Pixels};
use teleoperation_shared::battlefield::Eye;

/// The battlefield: the field as the camera `eye` sees it, in the screen rectangle of the picture.
#[derive(Clone, Copy)]
pub struct Lens {
    pub eye: Eye,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Lens {
    pub fn new(eye: Eye, b: Bounds<Pixels>) -> Lens {
        let (x, y, w, h) = rect(b);
        Lens { eye, x, y, w: w.max(1.0), h: h.max(1.0) }
    }

    /// The screen point of `p` (north, east, up), or `None` behind the camera.
    pub fn at(self, p: Vec3) -> Option<(f32, f32)> {
        self.eye.project(p, self.w, self.h).map(|(sx, sy)| (self.x + sx, self.y + sy))
    }

    /// The screen point of the ground at `at`.
    pub fn ground(self, at: Vec2) -> Option<(f32, f32)> {
        self.at(at.extend(0.0))
    }

    /// The ground under the screen point (`sx`, `sy`), or `None` in the sky.
    pub fn to_field(self, sx: f32, sy: f32) -> Option<Vec2> {
        self.eye.ground(sx - self.x, sy - self.y, self.w, self.h)
    }

    /// A circle of radius `r` on the ground round `at`, on screen (closed), if all of it is in
    /// front of the camera.
    pub fn ring(self, at: Vec2, r: f32) -> Option<Vec<(f32, f32)>> {
        let n = 48;
        (0..=n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                self.ground(at + Vec2::new(a.cos(), a.sin()) * r)
            })
            .collect()
    }

    /// The four ground corners of the battlefield (top left, top right, bottom right, bottom
    /// left); a corner in the sky is cut to `far` metres from the camera.
    pub fn footprint(self, far: f32) -> Vec<Vec2> {
        let corners = [(0.0, 0.0), (self.w, 0.0), (self.w, self.h), (0.0, self.h)];
        let from = self.eye.position().truncate();
        corners
            .iter()
            .map(|&(cx, cy)| {
                let g = self.eye.ground(cx, cy, self.w, self.h);
                match g {
                    Some(g) if g.distance(from) < far => g,
                    _ => {
                        // Along the corner's heading, as far as `far`.
                        let a = self.eye.ground(cx, self.h * 0.5, self.w, self.h).unwrap_or(self.eye.look);
                        from + (a - from).normalize_or(Vec2::X) * far
                    }
                }
            })
            .collect()
    }
}

/// The whole field from straight above, north up, fitted to a square.
#[derive(Clone, Copy)]
pub struct Flat {
    centre: (f32, f32),
    /// Pixels per metre.
    pub scale: f32,
}

impl Flat {
    /// The field of radius `radius` fitted to `b`.
    pub fn new(b: Bounds<Pixels>, radius: f32) -> Flat {
        let (x, y, w, h) = rect(b);
        Flat { centre: (x + w * 0.5, y + h * 0.5), scale: 0.5 * w.min(h) / (radius * 1.08) }
    }

    pub fn to_screen(self, at: Vec2) -> (f32, f32) {
        (self.centre.0 + at.y * self.scale, self.centre.1 - at.x * self.scale)
    }

    pub fn to_field(self, sx: f32, sy: f32) -> Vec2 {
        Vec2::new((self.centre.1 - sy) / self.scale, (sx - self.centre.0) / self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    fn bounds() -> Bounds<Pixels> {
        Bounds { origin: point(px(100.0), px(40.0)), size: size(px(1000.0), px(600.0)) }
    }

    #[test]
    fn the_battlefield_and_its_ground_agree() {
        let lens = Lens::new(Eye { look: Vec2::new(-400.0, 30.0), heading: 20.0, ..Eye::default() }, bounds());
        let (sx, sy) = lens.ground(Vec2::new(-350.0, 80.0)).unwrap();
        assert!(sx > 100.0 && sy > 40.0);
        assert!(lens.to_field(sx, sy).unwrap().distance(Vec2::new(-350.0, 80.0)) < 0.05);
        // The point it looks at is in the middle of the battlefield.
        let (mx, my) = lens.ground(lens.eye.look).unwrap();
        assert!((mx - 600.0).abs() < 0.01 && (my - 340.0).abs() < 0.01);
    }

    #[test]
    fn the_footprint_is_round_the_point_the_camera_looks_at() {
        let lens = Lens::new(Eye::default(), bounds());
        let f = lens.footprint(5000.0);
        let middle = f.iter().copied().sum::<Vec2>() / 4.0;
        // Further ahead than the point, as a trapezoid wider at the top.
        assert!(middle.x > 0.0 && middle.y.abs() < 1.0, "{middle}");
        assert!(f[0].distance(f[1]) > f[2].distance(f[3]));
    }

    #[test]
    fn the_minimap_is_north_up() {
        let flat = Flat::new(bounds(), 600.0);
        let (_, ny) = flat.to_screen(Vec2::new(100.0, 0.0));
        let (ex, _) = flat.to_screen(Vec2::new(0.0, 100.0));
        assert!(ny < 340.0 && ex > 600.0);
        assert!(flat.to_field(ex, 340.0).distance(Vec2::new(0.0, 100.0)) < 0.01);
    }
}
