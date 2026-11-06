//! Cliffs: the steep faces of the terrain, drawn as rock that runs up the face.
//!
//! The terrain's colour is baked per chunk and laid on from above, so on a face steeper than
//! about 60° a texel is stretched down several times its width, and on the walls round the
//! Towers valley (400 m in 100 m) the rock hangs like drapes. So each chunk's steep triangles
//! become a mesh of their own, [`split`] from the rest, whose texture is laid on from the side
//! (from whichever of east or south the face looks to most): a tiling rock, the colour of the
//! baked rock, with a normal map of its cracks and ledges. The shape is the terrain's, to the
//! vertex: what a drone hits is what it sees.

use bevy::prelude::*;

/// A triangle whose normal is less upright than this is a cliff (60° and steeper).
pub const STEEP: f32 = 0.5;
/// The rock texture covers this many metres, along and up a face.
pub const TILE: f32 = 24.0;

/// Splits a terrain chunk into its ground and its cliffs (none if it has no steep face).
pub fn split(mesh: &Mesh) -> (Mesh, Option<Mesh>) {
    (mesh.clone(), None)
}

/// The cliff rock's albedo at (u, v) of its tile (0 to 1 each way; it repeats).
pub fn rock_albedo(u: f32, v: f32) -> [f32; 3] {
    let _ = (u, v);
    [0.0; 3]
}

/// The height of the cliff rock's cracks and ledges at (u, v) of its tile (m; it repeats).
pub fn rock_height(u: f32, v: f32) -> f32 {
    let _ = (u, v);
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};

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

    /// The rock tiles with no seam: its left edge meets its right, its top its bottom.
    #[test]
    fn the_rock_tiles_without_a_seam() {
        let step = 1.0 / 512.0;
        let mut worst = 0.0f32;
        for k in 0..64 {
            let s = k as f32 / 64.0;
            for (a, b) in [((0.0, s), (1.0 - step, s)), ((s, 0.0), (s, 1.0 - step))] {
                let (ca, cb) = (rock_albedo(a.0, a.1), rock_albedo(b.0, b.1));
                worst = worst.max((0..3).map(|i| (ca[i] - cb[i]).abs()).fold(0.0, f32::max));
                worst = worst.max((rock_height(a.0, a.1) - rock_height(b.0, b.1)).abs() / 2.0);
            }
        }
        assert!(worst < 0.02, "a seam of {worst:.3}");
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
}
