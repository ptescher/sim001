//! The meadow close up: blades of grass and flowers as real geometry, for the few metres round
//! the camera where a painted card shows itself to be flat. Beyond `NEAR` the cards of `grass`
//! take over.
//!
//! The builders put triangles into a `Parts` with a colour in every vertex (a blade is dark at the
//! root and light at the tip; a flower's head is its own colour), so one material serves them all.

use crate::vegetation::rnd;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::f32::consts::TAU;

/// Out to here (m) the grass and flowers are solid.
pub const NEAR: f32 = 6.5;

/// The solid meadow, then the painted cards: one hands over to the other at `NEAR`, abruptly (a
/// dithered cross-fade turns see-through under the temporal accumulation).
pub fn near_ranges() -> (VisibilityRange, VisibilityRange) {
    (VisibilityRange::abrupt(0.0, NEAR), VisibilityRange::abrupt(NEAR, crate::grass::RADIUS))
}

#[derive(Default)]
pub struct Parts {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Vec<[f32; 3]>,
    pub col: Vec<[f32; 4]>,
    pub idx: Vec<u32>,
}

impl Parts {
    pub fn tris(&self) -> usize {
        self.idx.len() / 3
    }

    fn vert(&mut self, p: Vec3, n: Vec3, c: [f32; 3]) -> u32 {
        self.pos.push(p.to_array());
        self.nrm.push(n.normalize_or(Vec3::Y).to_array());
        self.col.push([c[0], c[1], c[2], 1.0]);
        (self.pos.len() - 1) as u32
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.idx.extend_from_slice(&[a, b, c]);
    }

    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.tri(a, b, c);
        self.tri(a, c, d);
    }

    pub fn into_mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.col)
            .with_inserted_indices(Indices::U32(self.idx))
    }
}

/// Two horizontal unit vectors at right angles, across `up`.
fn basis(up: Vec3) -> (Vec3, Vec3) {
    let e1 = up.cross(Vec3::Z);
    let e1 = if e1.length() > 0.1 { e1.normalize() } else { Vec3::X };
    (e1, up.cross(e1).normalize())
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// A blade of grass: a ribbon of three sections that narrows to a point and arches over by
/// `bend` (the tip's reach, as a share of the height), dark at the root and light at the tip.
pub fn blade(p: &mut Parts, base: Vec3, up: Vec3, height: f32, width: f32, az: f32, bend: f32, dark: [f32; 3], light: [f32; 3]) {
    let (e1, e2) = basis(up);
    let lean = e1 * az.cos() + e2 * az.sin();
    let across = up.cross(lean).normalize_or(e2);
    let node = |t: f32| base + up * height * t + lean * (bend * height * t * t);
    // Lit as if it were a little rounded, and facing up: what a meadow looks like from above.
    let normal = |t: f32| {
        let tangent = (up + lean * (2.0 * bend * t)).normalize();
        (tangent.cross(across).normalize_or(up) * 0.45 + up * 0.8).normalize()
    };
    let colour = |t: f32| mix3(dark, light, t.powf(0.8));
    let mut prev: Option<(u32, u32)> = None;
    for t in [0.0f32, 0.38, 0.72] {
        let half = across * width * 0.5 * (1.0 - 0.85 * t);
        let c = node(t);
        let (l, r) = (p.vert(c - half, normal(t), colour(t)), p.vert(c + half, normal(t), colour(t)));
        if let Some((pl, pr)) = prev {
            p.tri(pl, pr, l);
            p.tri(pr, r, l);
        }
        prev = Some((l, r));
    }
    let tip = p.vert(node(1.0), normal(1.0), colour(1.0));
    let (l, r) = prev.unwrap();
    p.tri(l, r, tip);
}

/// A tuft of blades: different heights, spread over a disc, fanning outwards.
pub fn clump(p: &mut Parts, centre: Vec3, up: Vec3, seed: u32, height: f32, radius: f32, tint: [f32; 3]) {
    let (e1, e2) = basis(up);
    let n = 10 + (rnd(seed, 1) * 6.0) as u32;
    for k in 0..n {
        let a = rnd(seed, 10 + k * 5) * TAU;
        let r = rnd(seed, 11 + k * 5).sqrt() * radius;
        let h = height * (0.55 + 0.6 * rnd(seed, 12 + k * 5));
        let az = a + (rnd(seed, 13 + k * 5) - 0.5) * 1.6;
        let bend = 0.15 + 0.5 * rnd(seed, 14 + k * 5) * (h / height);
        let width = 0.008 + 0.008 * rnd(seed, 15 + k * 5);
        let shade = 0.8 + 0.4 * rnd(seed, 16 + k * 5);
        let off = (e1 * a.cos() + e2 * a.sin()) * r;
        blade(p, centre + off, up, h, width, az, bend, tint.map(|c| c * 0.35 * shade), tint.map(|c| c * 1.15 * shade));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Flower {
    Daisy,
    Hawkweed,
    Clover,
    Lupine,
    Paintbrush,
}

const GREEN: [f32; 3] = [0.045, 0.12, 0.02];

/// A thin stem of two crossed ribbons, leaning and bending a little. Returns its top and the
/// direction it ends in.
fn stem(p: &mut Parts, base: Vec3, up: Vec3, lean: Vec3, height: f32, bend: f32, width: f32) -> (Vec3, Vec3) {
    let node = |t: f32| base + up * height * t + lean * (bend * height * t * t);
    let across = [up.cross(lean).normalize_or(Vec3::X), lean];
    for a in across {
        let mut prev: Option<(u32, u32)> = None;
        for t in [0.0f32, 0.5, 1.0] {
            let c = node(t);
            let col = mix3(GREEN.map(|v| v * 0.8), GREEN.map(|v| v * 1.3), t);
            let (l, r) = (p.vert(c - a * width * 0.5, up, col), p.vert(c + a * width * 0.5, up, col));
            if let Some((pl, pr)) = prev {
                p.quad(pl, pr, r, l);
            }
            prev = Some((l, r));
        }
    }
    (node(1.0), (up + lean * (2.0 * bend)).normalize())
}

/// A leaf lying out from the base of a plant: a long narrow quad that lifts at the tip.
fn leaf(p: &mut Parts, base: Vec3, up: Vec3, az: f32, length: f32, width: f32, col: [f32; 3]) {
    let (e1, e2) = basis(up);
    let dir = e1 * az.cos() + e2 * az.sin();
    let side = up.cross(dir).normalize_or(e2) * width * 0.5;
    let mid = base + dir * length * 0.5 + up * length * 0.12;
    let tip = base + dir * length + up * length * 0.1;
    let n = (up * 0.9 + dir * 0.2).normalize();
    let (a, b) = (p.vert(base - side * 0.4, n, col.map(|v| v * 0.7)), p.vert(base + side * 0.4, n, col.map(|v| v * 0.7)));
    let (c, d) = (p.vert(mid + side, n, col), p.vert(mid - side, n, col));
    let t = p.vert(tip, n, col.map(|v| v * 1.15));
    p.quad(a, b, c, d);
    p.tri(d, c, t);
}

/// A flower and its leaves, standing at `base` and `height` tall: the stem, then the head (which
/// faces up and a little the way the stem leans).
pub fn flower(p: &mut Parts, kind: Flower, base: Vec3, up: Vec3, height: f32, seed: u32) {
    let (e1, e2) = basis(up);
    let az = rnd(seed, 1) * TAU;
    let lean = e1 * az.cos() + e2 * az.sin();
    let bend = 0.05 + 0.12 * rnd(seed, 2);
    let tweak = |c: [f32; 3]| c.map(|v| (v * (0.93 + 0.14 * rnd(seed, 3))).min(1.0));
    // How much of the height the head takes.
    let head = match kind {
        Flower::Daisy => 0.006,
        Flower::Hawkweed => 0.004,
        Flower::Clover => 0.022,
        Flower::Lupine => 0.012,
        Flower::Paintbrush => 0.03,
    };
    let (top, dir) = stem(p, base, up, lean, (height - head).max(0.02), bend, 0.004);
    let axis = (dir + up * 0.5).normalize();
    let (u, v) = basis(axis);
    let ring = |k: u32, of: u32| {
        let a = k as f32 / of as f32 * TAU;
        u * a.cos() + v * a.sin()
    };
    match kind {
        Flower::Daisy => {
            let centre = p.vert(top + axis * 0.002, axis, [0.9, 0.62, 0.05]);
            let rim: Vec<u32> = (0..6).map(|k| p.vert(top + ring(k, 6) * 0.009, axis, [0.82, 0.52, 0.04])).collect();
            for k in 0..6 {
                p.tri(centre, rim[k], rim[(k + 1) % 6]);
            }
            let white = tweak([0.93, 0.93, 0.89]);
            for k in 0..13 {
                let d = ring(k, 13);
                let side = axis.cross(d).normalize_or(u) * 0.0032;
                let (i, o) = (top + d * 0.007, top + d * 0.027 - axis * 0.005);
                let n = (axis * 0.8 + d * 0.3).normalize();
                let q = [p.vert(i - side, n, white.map(|x| x * 0.9)), p.vert(i + side, n, white.map(|x| x * 0.9)), p.vert(o + side * 1.2, n, white), p.vert(o - side * 1.2, n, white)];
                p.quad(q[0], q[1], q[2], q[3]);
            }
            leaf(p, base, up, az + 0.5, 0.07, 0.014, [0.14, 0.38, 0.09]);
            leaf(p, base, up, az + 3.4, 0.055, 0.012, [0.14, 0.38, 0.09]);
        }
        Flower::Hawkweed => {
            let gold = tweak([0.97, 0.62, 0.06]);
            let centre = p.vert(top + axis * 0.002, axis, [0.82, 0.42, 0.03]);
            let rim: Vec<u32> = (0..6).map(|k| p.vert(top + ring(k, 6) * 0.004, axis, [0.85, 0.46, 0.04])).collect();
            for k in 0..6 {
                p.tri(centre, rim[k], rim[(k + 1) % 6]);
            }
            for k in 0..16 {
                let d = ring(k, 16);
                let side = axis.cross(d).normalize_or(u) * 0.0035;
                let n = (axis * 0.8 + d * 0.3).normalize();
                let tip = top + d * 0.024 - axis * 0.004;
                let (a, b, c) = (p.vert(top + d * 0.003 - side, n, gold.map(|x| x * 0.85)), p.vert(top + d * 0.003 + side, n, gold.map(|x| x * 0.85)), p.vert(tip, n, gold));
                p.tri(a, b, c);
            }
            leaf(p, base, up, az + 1.0, 0.06, 0.01, [0.15, 0.36, 0.09]);
            leaf(p, base, up, az + 4.0, 0.05, 0.01, [0.15, 0.36, 0.09]);
        }
        Flower::Clover => {
            let pink = tweak([0.80, 0.28, 0.52]);
            let low = pink.map(|x| (x * 1.1 + 0.08).min(1.0));
            let a: Vec<u32> = (0..8).map(|k| p.vert(top + ring(k, 8) * 0.011, (ring(k, 8) * 0.6 + axis).normalize(), low)).collect();
            let b: Vec<u32> = (0..8).map(|k| p.vert(top + axis * 0.012 + ring(k, 8) * 0.0105, (ring(k, 8) * 0.4 + axis).normalize(), pink)).collect();
            let cap = p.vert(top + axis * 0.022, axis, pink.map(|x| x * 0.9));
            for k in 0..8 {
                let k2 = (k + 1) % 8;
                p.quad(a[k], a[k2], b[k2], b[k]);
                p.tri(b[k], b[k2], cap);
            }
            // Trifoliate leaves low down: three rounded leaflets on each.
            for l in 0..2 {
                let la = az + 1.4 + l as f32 * 3.0;
                let (d1, d2) = (e1 * la.cos() + e2 * la.sin(), -(e1 * la.sin()) + e2 * la.cos());
                let at = base + d1 * 0.03 + up * (0.04 + 0.03 * l as f32);
                for k in 0..3 {
                    let a = k as f32 / 3.0 * TAU + la;
                    let out = d1 * a.cos() + d2 * a.sin();
                    let side = up.cross(out).normalize_or(d2);
                    let c = [0.17, 0.42, 0.12];
                    let n = (up + out * 0.2).normalize();
                    let q = [p.vert(at, n, c.map(|x| x * 0.7)), p.vert(at + out * 0.014 + side * 0.009, n, c), p.vert(at + out * 0.03, n, c.map(|x| x * 1.15)), p.vert(at + out * 0.014 - side * 0.009, n, c)];
                    p.quad(q[0], q[1], q[2], q[3]);
                }
            }
        }
        Flower::Lupine => {
            // A spike of pea flowers up the top of the stem, violet at the bottom to blue at the tip.
            for k in 0..10 {
                let t = k as f32 / 9.0;
                let at = base + up * (height * (0.62 + 0.36 * t)) + lean * (bend * height * (0.62 + 0.36 * t).powi(2));
                let a = k as f32 * 2.4 + rnd(seed, 20 + k) * 0.6;
                let out = e1 * a.cos() + e2 * a.sin();
                let side = up.cross(out).normalize_or(e2) * 0.006;
                let size = 0.018 * (1.0 - 0.45 * t);
                let col = tweak(mix3([0.42, 0.22, 0.72], [0.55, 0.45, 0.90], t));
                let apex = p.vert(at + out * size + up * 0.003, (out + up * 0.4).normalize(), col.map(|x| (x * 1.1).min(1.0)));
                let ring4 = [at + up * 0.006, at + side, at - up * 0.006, at - side];
                let vs: Vec<u32> = ring4.iter().map(|q| p.vert(*q, (*q - at + out * 0.4).normalize_or(up), col.map(|x| x * 0.85))).collect();
                for i in 0..4 {
                    p.tri(vs[i], vs[(i + 1) % 4], apex);
                }
            }
            let bud = p.vert(base + up * height + lean * (bend * height), up, [0.5, 0.42, 0.88]);
            let b0 = p.vert(base + up * (height - 0.02) + lean * (bend * height) + e1 * 0.004, up, [0.42, 0.30, 0.8]);
            let b1 = p.vert(base + up * (height - 0.02) + lean * (bend * height) - e1 * 0.004, up, [0.42, 0.30, 0.8]);
            p.tri(b0, b1, bud);
            // Palmate leaves low down: five leaflets fanned out.
            for l in 0..2 {
                let la = az + 0.8 + l as f32 * 3.1;
                let at = base + up * (0.03 + 0.02 * l as f32);
                for k in 0..5 {
                    leaf(p, at, up, la + (k as f32 - 2.0) * 0.45, 0.07, 0.012, [0.15, 0.36, 0.1]);
                }
            }
        }
        Flower::Paintbrush => {
            for (ringk, count, reach, rise) in [(0u32, 7u32, 0.012f32, 0.02f32), (1, 6, 0.006, 0.03)] {
                for k in 0..count {
                    let d = ring(k + ringk * 3, count);
                    let side = axis.cross(d).normalize_or(u) * 0.004;
                    let n = (axis * 0.6 + d * 0.5).normalize();
                    let (a, b) = (p.vert(top + d * 0.004 - side, n, [0.62, 0.05, 0.03]), p.vert(top + d * 0.004 + side, n, [0.62, 0.05, 0.03]));
                    let tip = p.vert(top + d * reach + axis * rise, n, tweak([0.90, 0.24, 0.05]));
                    p.tri(a, b, tip);
                }
            }
            for k in 0..3 {
                leaf(p, base, up, az + k as f32 * 2.1, 0.06, 0.008, [0.18, 0.36, 0.10]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(p: &Parts) -> f32 {
        p.pos.iter().map(|v| v[1]).fold(f32::MIN, f32::max)
    }

    /// The mean colour of the vertices in the upper part of the mesh: the heads of the flowers.
    fn head_colour(p: &Parts, from: f32) -> [f32; 3] {
        let (lo, hi) = (p.pos.iter().map(|v| v[1]).fold(f32::MAX, f32::min), top(p));
        let upper: Vec<_> = (0..p.pos.len()).filter(|&i| p.pos[i][1] >= lo + (hi - lo) * from).collect();
        let n = upper.len() as f32;
        [0, 1, 2].map(|c| upper.iter().map(|&i| p.col[i][c]).sum::<f32>() / n)
    }

    /// A blade of grass is a narrowing ribbon that arches over: as tall as asked, a few
    /// triangles, darker at the root than at the tip.
    #[test]
    fn a_blade_arches_and_narrows() {
        let mut p = Parts::default();
        blade(&mut p, Vec3::ZERO, Vec3::Y, 0.3, 0.01, 0.0, 0.5, [0.1, 0.2, 0.05], [0.4, 0.6, 0.2]);
        assert_eq!(p.tris(), 5);
        assert!((top(&p) - 0.3).abs() < 0.04, "top {}", top(&p));
        // It leans over: the tip is well to one side of the root.
        let tip = p.pos.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
        assert!(tip[0].hypot(tip[2]) > 0.05, "the tip is straight above the root");
        let width = |y0: f32, y1: f32| {
            let xs: Vec<f32> = p.pos.iter().filter(|v| v[1] >= y0 && v[1] < y1).map(|v| v[2]).collect();
            xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!(width(0.0, 0.05) > width(0.15, 0.25), "a blade is wider at the root");
        let lum = |c: [f32; 4]| c[0] + c[1] + c[2];
        let (root, tip) = (p.col[0], p.col[p.col.len() - 1]);
        assert!(lum(tip) > lum(root) * 1.5, "the tip is not lighter than the root");
        assert!(p.nrm.iter().all(|n| (Vec3::from_array(*n).length() - 1.0).abs() < 1e-3), "normals are unit");
    }

    /// A clump is a tuft of blades of differing heights, spread round a point, fanning out.
    #[test]
    fn a_clump_is_a_tuft_of_blades() {
        let mut p = Parts::default();
        clump(&mut p, Vec3::new(1.0, 0.0, 2.0), Vec3::Y, 7, 0.3, 0.08, [0.3, 0.5, 0.15]);
        let blades = p.tris() / 5;
        assert!((10..=16).contains(&blades), "{blades} blades");
        let heights: Vec<f32> = (0..blades).map(|b| p.pos[b * 7..b * 7 + 7].iter().map(|v| v[1]).fold(f32::MIN, f32::max)).collect();
        let spread = heights.iter().cloned().fold(f32::MIN, f32::max) - heights.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 0.05, "all the same height ({spread})");
        let far = p.pos.iter().map(|v| (v[0] - 1.0).hypot(v[2] - 2.0)).fold(0.0f32, f32::max);
        assert!(far < 0.08 + 0.3 * 0.6, "a clump is wider than that ({far})");
    }

    /// Each flower is a few dozen triangles, as tall as asked, with a head of its own colour.
    #[test]
    fn each_flower_is_its_own_colour_and_cheap() {
        // (kind, tallest, what the head's colour must satisfy)
        let cases: [(Flower, f32, fn([f32; 3]) -> bool); 5] = [
            (Flower::Daisy, 0.3, |c| c.iter().all(|&v| v > 0.55)),
            (Flower::Hawkweed, 0.35, |c| c[0] > 0.8 && c[1] > 0.4 && c[1] < 0.75 && c[2] < 0.25),
            (Flower::Clover, 0.2, |c| c[0] > 0.55 && c[2] > 0.25 && c[1] < 0.4),
            (Flower::Lupine, 0.5, |c| c[2] > c[0] && c[0] > c[1]),
            (Flower::Paintbrush, 0.3, |c| c[0] > 0.55 && c[1] < 0.35 && c[2] < 0.2),
        ];
        for (kind, height, colour) in cases {
            let mut p = Parts::default();
            flower(&mut p, kind, Vec3::ZERO, Vec3::Y, height, 5);
            assert!(p.tris() >= 20 && p.tris() <= 90, "{kind:?}: {} triangles", p.tris());
            assert!((top(&p) / height - 1.0).abs() < 0.12, "{kind:?}: {} high against {height}", top(&p));
            let c = head_colour(&p, 0.75);
            assert!(colour(c), "{kind:?}: the head is {c:?}");
            assert!(p.nrm.iter().all(|n| (Vec3::from_array(*n).length() - 1.0).abs() < 1e-3), "{kind:?}: normals are unit");
            // It stands on the ground: nothing below it.
            assert!(p.pos.iter().all(|v| v[1] > -0.01), "{kind:?} dips below the ground");
        }
    }

    #[test]
    fn a_flower_is_the_same_every_time_and_not_every_flower_is() {
        let build = |seed| {
            let mut p = Parts::default();
            flower(&mut p, Flower::Daisy, Vec3::ZERO, Vec3::Y, 0.3, seed);
            p.pos
        };
        assert_eq!(build(3), build(3));
        assert_ne!(build(3), build(4));
    }

    /// The solid flowers and grass take over from the painted cards without a cross-fade (a
    /// dithered one turns see-through under the temporal accumulation) and without a gap.
    #[test]
    fn the_solid_meadow_hands_over_to_the_cards_at_one_distance() {
        let (solid, cards) = near_ranges();
        assert!(solid.is_abrupt() && cards.is_abrupt());
        assert_eq!(solid.start_margin.start, 0.0);
        assert_eq!(solid.end_margin.end, cards.start_margin.start, "no gap or overlap");
        assert!(solid.end_margin.end >= 5.0 && solid.end_margin.end <= 14.0, "{}", solid.end_margin.end);
    }
}
