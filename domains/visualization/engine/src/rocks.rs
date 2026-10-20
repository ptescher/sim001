//! Granite boulders and cobbles: an icosphere cut by random planes into faces and edges, roughened
//! a little, set on a flat bottom, and coloured in the vertices with moss on the tops, lichen on
//! the sides and dirt where it meets the ground. Each triangle is flat-shaded (a little
//! softened) and mapped to the texture by box projection, so nothing is stretched.

use crate::vegetation::{noise3, rnd};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// The lowest point of every rock is at `y = -BOTTOM`: the placement sinks it by a known amount.
pub const BOTTOM: f32 = 0.35;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    /// A thousand triangles: seen from a few metres.
    Boulder,
    /// A couple of hundred: cobbles and stones.
    Cobble,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn unit_vector(seed: u32, k: u32) -> Vec3 {
    // Uniform on the sphere.
    let z = rnd(seed, k) * 2.0 - 1.0;
    let a = rnd(seed, k + 1) * std::f32::consts::TAU;
    let r = (1.0 - z * z).sqrt();
    Vec3::new(r * a.cos(), z, r * a.sin())
}

pub fn rock(seed: u32, size: Size) -> Mesh {
    let seed = seed.wrapping_mul(7919).wrapping_add(13);
    let subdivisions = match size {
        Size::Boulder => 6,
        Size::Cobble => 2,
    };
    let ico = Sphere::new(1.0).mesh().ico(subdivisions).unwrap();
    let mut pos: Vec<Vec3> = ico.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).unwrap().iter().map(|p| Vec3::from_array(*p)).collect();
    let tris: Vec<u32> = ico.indices().unwrap().iter().map(|i| i as u32).collect();

    // Planes that cut the sphere into faces: a few leaning up (the top), the rest all round, none
    // low enough to lift the bottom.
    let planes = match size {
        Size::Boulder => 7 + (rnd(seed, 1) * 6.0) as u32,
        Size::Cobble => 4 + (rnd(seed, 1) * 3.0) as u32,
    };
    let cuts: Vec<(Vec3, f32)> = (0..planes)
        .map(|k| {
            let mut n = unit_vector(seed, 10 + k * 3);
            if k < planes / 4 {
                n = (n * 0.5 + Vec3::Y * 1.2).normalize();
            }
            n.y = n.y.max(-0.5);
            (n.normalize(), 0.62 + 0.28 * rnd(seed, 11 + k * 3))
        })
        .collect();
    let off = Vec3::splat(seed as f32 * 0.37);
    let scale = Vec3::new(0.9 + 0.35 * rnd(seed, 2), 0.5 + 0.45 * rnd(seed, 3), 0.8 + 0.4 * rnd(seed, 4));
    for p in &mut pos {
        for (n, d) in &cuts {
            let over = p.dot(*n) - d;
            if over > 0.0 {
                *p -= *n * over;
            }
        }
        // Rough, not smooth: grain on the faces.
        let g = noise3(*p * 5.0 + off) - 0.5;
        *p *= 1.0 + 0.025 * g;
        *p *= scale;
        p.y = p.y.max(-BOTTOM);
    }

    // Smooth normals from the shared vertices, to soften the flat ones with.
    let mut smooth = vec![Vec3::ZERO; pos.len()];
    for t in tris.chunks(3) {
        let n = (pos[t[1] as usize] - pos[t[0] as usize]).cross(pos[t[2] as usize] - pos[t[0] as usize]);
        for &i in t {
            smooth[i as usize] += n;
        }
    }

    let (mut p_out, mut n_out, mut uv_out, mut c_out) = (vec![], vec![], vec![], vec![]);
    let moss = 0.25 + 0.75 * rnd(seed, 5);
    let tex_off = Vec2::new(rnd(seed, 6), rnd(seed, 7)) * 0.2;
    for t in tris.chunks(3) {
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        let face = (b - a).cross(c - a).normalize_or(Vec3::Y);
        // Box projection: onto the plane the face is most nearly parallel to.
        let axis = face.abs();
        let project = |p: Vec3| -> Vec2 {
            let q = if axis.x >= axis.y && axis.x >= axis.z {
                Vec2::new(p.z, p.y)
            } else if axis.y >= axis.z {
                Vec2::new(p.x, p.z)
            } else {
                Vec2::new(p.x, p.y)
            };
            q * 0.35 + Vec2::splat(0.5) + tex_off
        };
        for &i in t {
            let p = pos[i as usize];
            let n = (smooth[i as usize].normalize_or(face) * 0.3 + face * 0.7).normalize_or(face);
            let cover = smoothstep(0.55, 0.85, n.y) * smoothstep(0.35, 0.65, noise3(p * 2.5 + off)) * moss;
            let lichen = smoothstep(0.62, 0.78, noise3(p * 6.0 + off * 1.7)) * (1.0 - smoothstep(0.3, 0.8, n.y)) * smoothstep(-0.2, 0.2, p.y);
            let dirt = smoothstep(-BOTTOM + 0.2, -BOTTOM, p.y);
            let mut col = Vec3::ONE;
            col = col.lerp(Vec3::new(0.50, 0.66, 0.28), cover.min(1.0) * 0.9);
            col = col.lerp(Vec3::new(0.88, 0.83, 0.50), lichen * 0.55);
            col = col.lerp(Vec3::new(0.62, 0.50, 0.40), dirt * 0.8);
            p_out.push(p.to_array());
            n_out.push(n.to_array());
            uv_out.push(project(p).to_array());
            c_out.push([col.x, col.y, col.z, 1.0]);
        }
    }
    let n = p_out.len() as u32;
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, p_out)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, n_out)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv_out)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, c_out)
        .with_inserted_indices(Indices::U32((0..n).collect()))
        .with_generated_tangents()
        .expect("rock tangents")
}

/// Granite: grey grain with black and white flecks, pink feldspar blotches, and dark streaks
/// where water runs down it.
pub fn granite_texture() -> Image {
    crate::mesh_util::texture(512, |u, v| {
        let n = |f: f32, z: f32| noise3(Vec3::new(u * f, v * f, z));
        let grain = n(60.0, 1.0) * 0.5 + n(150.0, 2.0) * 0.5;
        let mut g = 0.13 + 0.07 * grain;
        // Flecks of dark hornblende and bright mica.
        let fleck = n(220.0, 3.0);
        if fleck > 0.74 {
            g *= 0.4;
        } else if fleck < 0.2 {
            g *= 1.7;
        }
        // Water stains run down the face.
        let stain = smoothstep(0.55, 0.8, noise3(Vec3::new(u * 4.0, v * 38.0, 5.0)));
        g *= 1.0 - 0.35 * stain;
        let pink = smoothstep(0.5, 0.75, n(7.0, 6.0));
        [g * (1.0 + 0.16 * pink), g * (1.0 - 0.04 * pink), g * (1.0 - 0.1 * pink)]
    })
}

/// The grain and the pits of the same stone, for the light to catch.
pub fn granite_normal() -> Image {
    crate::mesh_util::normal_map(512, 2.2, |u, v| {
        let n = |f: f32, z: f32| noise3(Vec3::new(u * f, v * f, z));
        0.5 * n(24.0, 7.0) + 0.3 * n(70.0, 8.0) + 0.2 * n(180.0, 9.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Soup {
        pos: Vec<Vec3>,
        nrm: Vec<Vec3>,
        uv: Vec<Vec2>,
        col: Vec<[f32; 4]>,
    }

    fn soup(m: &Mesh) -> Soup {
        let f3 = |a| m.attribute(a).and_then(|v| v.as_float3()).expect("attribute").iter().map(|p| Vec3::from_array(*p)).collect::<Vec<_>>();
        let uv = match m.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(v)) => v.iter().map(|p| Vec2::from_array(*p)).collect(),
            _ => panic!("uvs"),
        };
        let col = match m.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(v)) => v.clone(),
            _ => panic!("vertex colours"),
        };
        Soup { pos: f3(Mesh::ATTRIBUTE_POSITION), nrm: f3(Mesh::ATTRIBUTE_NORMAL), uv, col }
    }

    fn tris(m: &Mesh) -> usize {
        m.indices().map_or(0, |i| i.len() / 3)
    }

    fn face_normal(s: &Soup, t: usize, idx: &[u32]) -> Vec3 {
        let (a, b, c) = (s.pos[idx[t * 3] as usize], s.pos[idx[t * 3 + 1] as usize], s.pos[idx[t * 3 + 2] as usize]);
        (b - a).cross(c - a).normalize_or_zero()
    }

    fn indices(m: &Mesh) -> Vec<u32> {
        m.indices().unwrap().iter().map(|i| i as u32).collect()
    }

    /// A granite boulder has flat faces meeting at edges, where a lump of clay has none: many of
    /// the edges between neighbouring triangles are creases.
    #[test]
    fn a_boulder_has_faces_and_edges() {
        for seed in [1, 2, 3, 4] {
            let m = rock(seed, Size::Boulder);
            let (s, idx) = (soup(&m), indices(&m));
            // Triangles that share an edge (by position) and how their faces meet.
            let key = |p: Vec3| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
            let mut edges: HashMap<_, Vec<usize>> = HashMap::new();
            for t in 0..idx.len() / 3 {
                for e in 0..3 {
                    let (a, b) = (key(s.pos[idx[t * 3 + e] as usize]), key(s.pos[idx[t * 3 + (e + 1) % 3] as usize]));
                    edges.entry(if a < b { (a, b) } else { (b, a) }).or_default().push(t);
                }
            }
            let (mut creases, mut total) = (0, 0);
            for ts in edges.values().filter(|v| v.len() == 2) {
                total += 1;
                if face_normal(&s, ts[0], &idx).dot(face_normal(&s, ts[1], &idx)) < 20f32.to_radians().cos() {
                    creases += 1;
                }
            }
            assert!(creases >= 25, "seed {seed}: only {creases} creases in {total} edges");
            // Most of the surface is flat faces, not creases.
            assert!(creases * 4 < total, "seed {seed}: {creases} of {total} edges are creases: it is rubble");
        }
    }

    /// It rests on a flat bottom, as a rock does, at a known height so that it can be sunk into
    /// the ground by a known amount.
    #[test]
    fn a_boulder_sits_on_a_flat_bottom() {
        for seed in [1, 2, 3] {
            let s = soup(&rock(seed, Size::Boulder));
            let low = s.pos.iter().map(|p| p.y).fold(f32::MAX, f32::min);
            assert!((low + BOTTOM).abs() < 1e-3, "lowest point {low}");
            let flat = s.pos.iter().filter(|p| (p.y - low).abs() < 1e-3).count();
            assert!(flat * 10 >= s.pos.len(), "only {flat} of {} points are on the bottom", s.pos.len());
        }
    }

    #[test]
    fn boulders_are_not_all_the_same_shape() {
        let aspects: Vec<f32> = (1..9)
            .map(|seed| {
                let s = soup(&rock(seed, Size::Boulder));
                let (lo, hi) = s.pos.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
                (hi.y - lo.y) / (hi.x - lo.x).max(hi.z - lo.z)
            })
            .collect();
        let spread = aspects.iter().cloned().fold(f32::MIN, f32::max) - aspects.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 0.15, "all about as tall as wide: {aspects:?}");
        assert!(aspects.iter().all(|&a| a > 0.2 && a < 1.3), "{aspects:?}");
    }

    #[test]
    fn rocks_stay_within_their_budgets() {
        for seed in 1..5 {
            let (big, small) = (tris(&rock(seed, Size::Boulder)), tris(&rock(seed, Size::Cobble)));
            assert!(big <= 1500, "a boulder has {big} triangles");
            assert!(small <= 300 && small >= 60, "a cobble has {small} triangles");
        }
    }

    /// Moss on the tops, bare or lichened on the sides, dirt at the bottom.
    #[test]
    fn moss_grows_on_top_and_dirt_collects_below() {
        let (mut top, mut side, mut base) = ((0.0, 0u32), (0.0, 0u32), (0.0, 0u32));
        for seed in 1..7 {
            let s = soup(&rock(seed, Size::Boulder));
            for i in 0..s.pos.len() {
                let (n, c) = (s.nrm[i], s.col[i]);
                let green = c[1] - 0.5 * (c[0] + c[2]);
                let bright = (c[0] + c[1] + c[2]) / 3.0;
                if n.y > 0.7 {
                    top = (top.0 + green, top.1 + 1);
                } else if n.y.abs() < 0.3 {
                    side = (side.0 + green, side.1 + 1);
                }
                if s.pos[i].y < -BOTTOM + 0.12 {
                    base = (base.0 + bright, base.1 + 1);
                }
            }
        }
        let (top, side) = (top.0 / top.1 as f32, side.0 / side.1 as f32);
        assert!(top > side + 0.03, "moss on top {top}, on the sides {side}");
        let base = base.0 / base.1 as f32;
        assert!(base < 0.85, "no dirt at the bottom (brightness {base})");
    }

    /// The texture is not stretched: a box projection keeps a patch of rock about the same size
    /// in the texture wherever it faces.
    #[test]
    fn the_texture_is_not_stretched() {
        let m = rock(3, Size::Boulder);
        let (s, idx) = (soup(&m), indices(&m));
        let mut ratios = Vec::new();
        for t in 0..idx.len() / 3 {
            let i = [idx[t * 3] as usize, idx[t * 3 + 1] as usize, idx[t * 3 + 2] as usize];
            let world = (s.pos[i[1]] - s.pos[i[0]]).cross(s.pos[i[2]] - s.pos[i[0]]).length() * 0.5;
            let tex = ((s.uv[i[1]] - s.uv[i[0]]).perp_dot(s.uv[i[2]] - s.uv[i[0]])).abs() * 0.5;
            if world > 1e-6 {
                ratios.push(tex / world);
            }
        }
        ratios.sort_by(|a, b| a.total_cmp(b));
        let (lo, mid, hi) = (ratios[ratios.len() / 20], ratios[ratios.len() / 2], ratios[ratios.len() * 19 / 20]);
        assert!(hi / mid < 2.6 && mid / lo.max(1e-6) < 6.0, "texture density {lo} / {mid} / {hi}");
    }

    #[test]
    fn a_rock_is_the_same_every_time() {
        let (a, b) = (rock(5, Size::Boulder), rock(5, Size::Boulder));
        assert_eq!(soup(&a).pos, soup(&b).pos);
        assert_ne!(soup(&a).pos, soup(&rock(6, Size::Boulder)).pos);
    }
}
