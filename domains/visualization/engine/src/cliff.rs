//! Cliffs: the steep faces of the terrain, drawn as rock that runs up the face.
//!
//! The terrain's colour is baked per chunk and laid on from above, so on a face steeper than
//! about 60° a texel is stretched down several times its width, and on the walls round the
//! Towers valley (400 m in 100 m) the rock hangs like drapes. So each chunk's steep triangles
//! become a mesh of their own, [`split`] from the rest, whose texture is laid on from the side
//! (from whichever of east or south the face looks to most): a tiling rock, the colour of the
//! baked rock, with a normal map of its cracks and ledges. The shape is the terrain's, to the
//! vertex: what a drone hits is what it sees.

use crate::terrain::{fbm, gradient_noise};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

/// A triangle whose normal is less upright than this is a cliff (60° and steeper).
pub const STEEP: f32 = 0.5;
/// The rock texture covers this many metres, along and up a face.
pub const TILE: f32 = 48.0;

/// Splits a terrain chunk into its ground and its cliffs (none if it has no steep face).
pub fn split(mesh: &Mesh) -> (Mesh, Option<Mesh>) {
    let (Some(VertexAttributeValues::Float32x3(pos)), Some(VertexAttributeValues::Float32x3(nrm))) =
        (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.attribute(Mesh::ATTRIBUTE_NORMAL))
    else {
        return (mesh.clone(), None);
    };
    let idx: Vec<u32> = match mesh.indices() {
        Some(i) => i.iter().map(|k| k as u32).collect(),
        None => (0..pos.len() as u32).collect(),
    };
    let p = |k: u32| Vec3::from_array(pos[k as usize]);
    let (mut ground, mut cliff) = (Vec::with_capacity(idx.len()), vec![]);
    for t in idx.as_chunks::<3>().0 {
        let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).normalize_or_zero();
        if n.y.abs() < STEEP { cliff.push(*t) } else { ground.extend_from_slice(t) }
    }
    if cliff.is_empty() {
        return (mesh.clone(), None);
    }
    let mut g = mesh.clone();
    g.insert_indices(Indices::U32(ground));
    // Each cliff triangle on its own corners, laid on from the side it faces most: from the east
    // (u along z) or from the south (u along x), with v down the face.
    let (mut cp, mut cn, mut cuv) = (vec![], vec![], vec![]);
    for t in cliff {
        let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        let east = n.x.abs() >= n.z.abs();
        for k in t {
            let q = p(k);
            cp.push(q.to_array());
            cn.push(buttressed(q, Vec3::from_array(nrm[k as usize])).to_array());
            cuv.push([if east { q.z } else { q.x } / TILE, -q.y / TILE]);
        }
    }
    let count = cp.len() as u32;
    let c = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, cp)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, cn)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, cuv)
        .with_inserted_indices(Indices::U32((0..count).collect()))
        .with_generated_tangents()
        .expect("cliff tangents");
    (g, Some(c))
}

/// How a cliff's light turns at `q` (normal `n`): as if it stood out in buttresses and fell back
/// in gullies tens of metres across, running up the face. A function of where the point is, so
/// neighbouring triangles agree.
fn buttressed(q: Vec3, n: Vec3) -> Vec3 {
    let along = Vec3::Y.cross(n).normalize_or_zero();
    if along == Vec3::ZERO {
        return n;
    }
    let g = |p: Vec3| {
        gradient_noise(p.x / 34.0 + 3.0, p.z / 34.0 - 8.0)
            + 0.45 * gradient_noise((p.x + p.z) / 13.0 + p.y / 400.0, p.y / 70.0 + 2.0)
    };
    // The slope of the buttresses along the face, over a metre.
    let slope = g(q + along) - g(q - along);
    // At most about 50°: a buttress, not a fin.
    (n - along * (slope * 14.0).clamp(-1.2, 1.2)).normalize()
}

/// `f` made to repeat over the unit square: four copies, shifted by a tile, blended so that
/// each edge is the same as the one across from it.
fn tiled(u: f32, v: f32, f: impl Fn(f32, f32) -> f32) -> f32 {
    let (u, v) = (u.rem_euclid(1.0), v.rem_euclid(1.0));
    f(u, v) * (1.0 - u) * (1.0 - v)
        + f(u - 1.0, v) * u * (1.0 - v)
        + f(u, v - 1.0) * (1.0 - u) * v
        + f(u - 1.0, v - 1.0) * u * v
}

/// The cracks of the rock at (u, v) of a tile: 0 to 1, low in a crack.
fn cracks(u: f32, v: f32) -> f32 {
    tiled(u, v, |u, v| fbm(u * TILE / 5.0 + 7.0, v * TILE / 5.0 - 2.0, 4))
}

/// The ledges: the rock breaks into steps a few metres tall (v is down the face).
fn ledges(u: f32, v: f32) -> f32 {
    tiled(u, v, |u, v| {
        let y = v * TILE / 3.2
            + 2.2 * gradient_noise(u * TILE / 14.0, v * TILE / 20.0 + 4.0)
            + 0.6 * gradient_noise(u * TILE / 4.0 + 9.0, v * TILE / 5.0);
        // A rounded step: no jumps, so it tiles and mips cleanly.
        smoothstep(-0.3, 0.9, (y * std::f32::consts::TAU).sin())
    })
}

pub fn rock_height(u: f32, v: f32) -> f32 {
    2.4 * (cracks(u, v) - 0.5) + 0.4 * ledges(u, v)
}

pub fn rock_albedo(u: f32, v: f32) -> [f32; 3] {
    let tone = tiled(u, v, |u, v| fbm(u * TILE / 11.0 - 5.0, v * TILE / 11.0 + 1.0, 3));
    let hollow = smoothstep(0.42, 0.25, cracks(u, v));
    let lit_ledge = smoothstep(0.75, 0.95, ledges(u, v));
    let t = 0.24 * (0.7 + 0.8 * (tone - 0.5) + 0.25 * lit_ledge) * (1.0 - 0.45 * hollow);
    [t * 1.05, t * 0.97, t * 0.87]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The rock's colour, one tile, repeating.
pub fn rock_texture() -> Image {
    crate::mesh_util::texture(1024, rock_albedo)
}

/// The rock's cracks and ledges as a normal map, one tile, repeating.
pub fn rock_normals() -> Image {
    let res = 1024u32;
    let d = 1.0 / res as f32;
    let metres = d * TILE;
    let texels = (0..res * res)
        .map(|k| {
            let (u, v) = ((k % res) as f32 * d, (k / res) as f32 * d);
            let h = rock_height(u, v);
            let slope = Vec2::new(rock_height(u + d, v) - h, rock_height(u, v + d) - h) / metres;
            let n = Vec3::new(-slope.x, -slope.y, 1.0).normalize();
            [((n.x * 0.5 + 0.5) * 255.0).round() as u8, ((n.y * 0.5 + 0.5) * 255.0).round() as u8]
        })
        .collect();
    crate::terrain_render::normal_image(texels, res, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terrain mesh of `y(x, z)`, 40 m square, in 1 m cells, as the terrain builds them.
    fn ground(y: fn(f32, f32) -> f32) -> Mesh {
        let n = 41;
        let mut pos = vec![];
        let mut uv = vec![];
        for j in 0..n {
            for i in 0..n {
                let (x, z) = (i as f32 - 20.0, j as f32 - 20.0);
                pos.push([x, y(x, z), z]);
                uv.push([i as f32 / 40.0, j as f32 / 40.0]);
            }
        }
        let mut idx = vec![];
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u32;
                let (b, c) = (a + 1, a + n as u32);
                idx.extend_from_slice(&[a, c, b, b, c, c + 1]);
            }
        }
        let normals = vec![[0.0, 1.0, 0.0]; pos.len()];
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
            .with_inserted_indices(Indices::U32(idx))
    }

    /// The triangles of a mesh: their corners, and the corners' uvs.
    fn triangles(m: &Mesh) -> Vec<([Vec3; 3], [Vec2; 3])> {
        let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else { return vec![] };
        let Some(VertexAttributeValues::Float32x2(uv)) = m.attribute(Mesh::ATTRIBUTE_UV_0) else { return vec![] };
        let idx: Vec<usize> = match m.indices() {
            Some(i) => i.iter().collect(),
            None => (0..p.len()).collect(),
        };
        idx.chunks(3)
            .map(|t| ([0, 1, 2].map(|k| Vec3::from_array(p[t[k]])), [0, 1, 2].map(|k| Vec2::from_array(uv[t[k]]))))
            .collect()
    }

    fn upright(t: &[Vec3; 3]) -> f32 {
        (t[1] - t[0]).cross(t[2] - t[0]).normalize().y.abs()
    }

    /// A wall: flat ground, then 4 m up for every metre across (76°, the Towers valley's).
    fn wall(x: f32, _z: f32) -> f32 {
        (x - 5.0).max(0.0) * 4.0
    }

    #[test]
    fn steep_faces_become_cliffs_and_the_rest_stays_ground() {
        let mesh = ground(wall);
        let (ground, cliffs) = split(&mesh);
        let cliffs = cliffs.expect("a wall has cliffs");
        let (g, c) = (triangles(&ground), triangles(&cliffs));
        assert!(!c.is_empty() && !g.is_empty());
        assert_eq!(g.len() + c.len(), triangles(&mesh).len(), "every triangle is drawn once");
        assert!(g.iter().all(|(t, _)| upright(t) >= STEEP), "no cliff left on the ground");
        assert!(c.iter().all(|(t, _)| upright(t) < STEEP), "no ground in the cliffs");
    }

    #[test]
    fn a_hillside_has_no_cliffs() {
        assert!(split(&ground(|x, z| 0.6 * x + 0.3 * z)).1.is_none());
    }

    /// Laid on from the side, the rock is not stretched down the face: a metre is about the same
    /// part of the tile whichever way it runs on the face (laid from above, it is four times as
    /// long down a 76° face as across it).
    #[test]
    fn the_rock_runs_up_the_face_unstretched() {
        for y in [wall as fn(f32, f32) -> f32, |x, z| ((x + z) * 0.707 - 5.0).max(0.0) * 4.0] {
            let (_, cliffs) = split(&ground(y));
            for (t, uv) in triangles(&cliffs.expect("cliffs")) {
                // How many metres a unit of uv covers, along each side of the triangle.
                let per = |a: usize, b: usize| (t[b] - t[a]).length() / (uv[b] - uv[a]).length() / TILE;
                let sides = [per(0, 1), per(1, 2), per(2, 0)];
                let (lo, hi) =
                    (sides.iter().cloned().fold(f32::MAX, f32::min), sides.iter().cloned().fold(0.0, f32::max));
                assert!(hi / lo < 1.8, "stretched {:.2} times on a triangle at {}", hi / lo, t[0]);
                assert!((0.5..2.0).contains(&lo), "the tile is {:.1} m, not {TILE}", lo * TILE);
            }
        }
    }

    /// The rock tiles with no seam: its left edge is its right, its top its bottom, and the step
    /// across an edge is no bigger than the steps inside the tile.
    #[test]
    fn the_rock_tiles_without_a_seam() {
        let lum = |u: f32, v: f32| {
            let c = rock_albedo(u, v);
            0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
        };
        let step = 1.0 / 512.0;
        for f in [&lum as &dyn Fn(f32, f32) -> f32, &rock_height] {
            for k in 0..64 {
                let s = k as f32 / 64.0;
                assert!(
                    (f(0.0, s) - f(1.0, s)).abs() < 1e-4 && (f(s, 0.0) - f(s, 1.0)).abs() < 1e-4,
                    "edges differ at {s}"
                );
                for across in [false, true] {
                    let at = |t: f32| if across { f(s, t) } else { f(t, s) };
                    let inside =
                        (1..511).map(|i| (at((i + 1) as f32 * step) - at(i as f32 * step)).abs()).fold(0.0, f32::max);
                    let seam = (at(0.0) - at(1.0 - step)).abs();
                    assert!(seam <= 1.5 * inside + 1e-4, "a step of {seam:.3} at the edge, at most {inside:.3} inside");
                }
            }
        }
    }

    /// Where a cliff meets the baked rock above or below it, they are one rock: the cliff's
    /// average colour is the baked rock's (0.14 to 0.20, linear), and it is not flat paint.
    #[test]
    fn the_cliff_is_the_colour_of_the_baked_rock() {
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let n = 64;
        let v: Vec<f32> =
            (0..n * n).map(|k| lum(rock_albedo((k % n) as f32 / n as f32, (k / n) as f32 / n as f32))).collect();
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let sd = (v.iter().map(|a| (a - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt();
        assert!((0.12..=0.21).contains(&mean), "the cliff's mean luminance {mean:.3}");
        assert!(sd / mean >= 0.1, "the cliff varies by only {:.0}%", sd / mean * 100.0);
        let relief = (0..n * n).map(|k| rock_height((k % n) as f32 / n as f32, (k / n) as f32 / n as f32));
        let (lo, hi) = relief.fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(h), hi.max(h)));
        assert!(hi - lo >= 0.5, "the cracks and ledges are {:.2} m deep", hi - lo);
    }

    /// A real cliff is not a smooth curtain: buttresses and gullies tens of metres across catch
    /// and lose the light. The shape stays the terrain's (what a drone hits); the cliff's normals
    /// turn as if it had them, and still face out of the rock.
    #[test]
    fn the_cliff_is_shaded_as_buttresses_and_gullies() {
        let big = |x: f32, _z: f32| (x - 5.0).max(0.0) * 4.0;
        let n = 161;
        let mut pos = vec![];
        for j in 0..n {
            for i in 0..n {
                let (x, z) = (i as f32 * 0.25 - 20.0, j as f32 * 2.5 - 200.0);
                pos.push([x, big(x, z), z]);
            }
        }
        let mut idx = vec![];
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u32;
                let (b, c) = (a + 1, a + n as u32);
                idx.extend_from_slice(&[a, c, b, b, c, c + 1]);
            }
        }
        let face = Vec3::new(-4.0, 1.0, 0.0).normalize();
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![face.to_array(); pos.len()])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; pos.len()])
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_indices(Indices::U32(idx));
        let cliffs = split(&mesh).1.expect("cliffs");
        let Some(VertexAttributeValues::Float32x3(normals)) = cliffs.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!()
        };
        let angles: Vec<f32> =
            normals.iter().map(|v| Vec3::from_array(*v).dot(face).clamp(-1.0, 1.0).acos().to_degrees()).collect();
        let rms = (angles.iter().map(|a| a * a).sum::<f32>() / angles.len() as f32).sqrt();
        assert!(rms >= 10.0, "the cliff's light turns by {rms:.1}° (rms)");
        assert!(
            angles.iter().all(|a| *a < 60.0),
            "a normal turns {:.0}° from the face",
            angles.iter().cloned().fold(0.0, f32::max)
        );
    }
}
