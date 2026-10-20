//! Expanded polypropylene, the foam the plane is made of: a mid-grey, fine and granular, from
//! the beads it is moulded from, which show as a speckle and as a grain in the surface.
//!
//! The grain is the same size wherever it is: `world_uv` maps a mesh to the texture by its size
//! in metres, onto the plane each face is nearest to (a wing, a pod and a tail made from
//! lofts and panels have UVs of their own that mean nothing about size).

use crate::mesh_util::{normal_map, texture};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;

/// Metres of foam in one tile of the texture.
pub const TILE: f32 = 0.1;
const SIZE: u32 = 512;
/// Beads across a tile: a bead is about 1.4 mm.
const CELLS: i32 = 72;
/// The average of the grain texture: a material's colour is its albedo divided by this.
pub const GRAIN_MEAN: f32 = 0.45;

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Cellular noise over the tile, wrapping at its edges: (distance to the nearest bead centre,
/// how much nearer that is than the next, a number for the bead). Distances are in beads.
fn beads(u: f32, v: f32) -> (f32, f32, f32) {
    let p = Vec2::new(u, v) * CELLS as f32;
    let cell = p.floor().as_ivec2();
    let (mut d1, mut d2, mut id) = (f32::MAX, f32::MAX, 0.0);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let c = cell + IVec2::new(dx, dy);
            let (wx, wy) = (c.x.rem_euclid(CELLS), c.y.rem_euclid(CELLS));
            let feature = c.as_vec2() + Vec2::new(hash(wx, wy, 1), hash(wx, wy, 2));
            let d = feature.distance(p);
            if d < d1 {
                d2 = d1;
                d1 = d;
                id = hash(wx, wy, 3);
            } else if d < d2 {
                d2 = d;
            }
        }
    }
    (d1, d2 - d1, id)
}

/// The grain of the foam, as a multiple of its colour (linear, about `GRAIN_MEAN` on average, so
/// there is headroom for the bright beads): each bead its own shade, darker where beads meet, a
/// fine noise, and the odd dark fleck. The material's colour says how dark the part is.
pub fn foam_texture() -> Image {
    texture(SIZE, |u, v| {
        let (d1, edge, id) = beads(u, v);
        let gap = 1.0 - smoothstep(0.0, 0.45, edge);
        // Each bead is a little domed: brighter in the middle.
        let dome = 1.14 - 0.36 * d1.min(1.0);
        let (px, py) = ((u * SIZE as f32) as i32, (v * SIZE as f32) as i32);
        let fine = 0.98 + 0.04 * hash(px, py, 4);
        let fleck = if hash(px, py, 5) > 0.996 { 0.45 } else { 1.0 };
        let g = GRAIN_MEAN * 1.2 * (0.82 + 0.3 * id) * dome * (1.0 - 0.35 * gap) * fine * fleck;
        [g, g, g * 1.02]
    })
}

/// The grain of the same foam for the light to catch: domed beads with pits between them.
pub fn foam_normal() -> Image {
    normal_map(SIZE, 0.8, |u, v| {
        let (d1, edge, id) = beads(u, v);
        let gap = 1.0 - smoothstep(0.0, 0.25, edge);
        0.5 * (1.0 - d1.min(1.0)) - 0.5 * gap + 0.12 * id
    })
}

/// The mesh with UVs by size in metres: each face projected onto the axis plane it is nearest to,
/// `TILE` metres to the texture. Faces are separated so that the projection can change from one
/// to the next; normals are kept as they were, tangents made again.
pub fn world_uv(mesh: Mesh) -> Mesh {
    let pos: Vec<Vec3> = mesh.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).expect("positions").iter().map(|p| Vec3::from(*p)).collect();
    let nrm: Option<Vec<Vec3>> = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(|a| a.as_float3()).map(|n| n.iter().map(|p| Vec3::from(*p)).collect());
    let idx: Vec<u32> = match mesh.indices() {
        Some(i) => i.iter().map(|i| i as u32).collect(),
        None => (0..pos.len() as u32).collect(),
    };
    let (mut p_out, mut n_out, mut uv_out) = (Vec::new(), Vec::new(), Vec::new());
    for t in idx.chunks(3) {
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        let face = (b - a).cross(c - a).normalize_or(Vec3::Y);
        let axis = face.abs();
        for &i in t {
            let p = pos[i as usize];
            let q = if axis.x >= axis.y && axis.x >= axis.z {
                Vec2::new(p.z, p.y)
            } else if axis.y >= axis.z {
                Vec2::new(p.x, p.z)
            } else {
                Vec2::new(p.x, p.y)
            };
            p_out.push(p.to_array());
            n_out.push(nrm.as_ref().map_or(face, |n| n[i as usize]).to_array());
            uv_out.push((q / TILE).to_array());
        }
    }
    let n = p_out.len() as u32;
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, p_out)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, n_out)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv_out)
        .with_inserted_indices(Indices::U32((0..n).collect()))
        .with_generated_tangents()
        .expect("tangents")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    fn grey(t: &Image) -> Vec<f32> {
        // Mip 0, sRGB bytes to linear luminance.
        let (w, h) = (t.width() as usize, t.height() as usize);
        let data = t.data.as_ref().unwrap();
        let lin = |b: u8| {
            let s = b as f32 / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        (0..w * h).map(|i| (lin(data[i * 4]) + lin(data[i * 4 + 1]) + lin(data[i * 4 + 2])) / 3.0).collect()
    }

    /// Normalised correlation of the picture with itself shifted `lag` pixels across.
    fn autocorrelation(v: &[f32], w: usize, lag: usize) -> f32 {
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32;
        let h = v.len() / w;
        let mut acc = 0.0;
        for y in 0..h {
            for x in 0..w {
                acc += (v[y * w + x] - mean) * (v[y * w + (x + lag) % w] - mean);
            }
        }
        acc / v.len() as f32 / var
    }

    /// The foam's grain is neutral, with the speckle of its fused beads and the odd dark fleck,
    /// not a flat colour, and about `GRAIN_MEAN` on average so that the material's colour can be
    /// the albedo of the part divided by that.
    #[test]
    fn foam_is_a_speckled_grey_around_its_mean() {
        let t = foam_texture();
        let g = grey(&t);
        let mean = g.iter().sum::<f32>() / g.len() as f32;
        let std = (g.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / g.len() as f32).sqrt();
        assert!((mean / GRAIN_MEAN - 1.0).abs() < 0.12, "mean {mean} against {GRAIN_MEAN}");
        assert!(g.iter().all(|&x| x < 0.95), "a bright bead is clipped");
        assert!(std / mean > 0.12 && std / mean < 0.45, "speckle contrast {}", std / mean);
        let rgb = t.data.as_ref().unwrap();
        let (r, b) = (rgb[0] as i32, rgb[2] as i32);
        assert!((r - b).abs() < 24, "the foam is not a grey: {r} {b}");
    }

    /// The beads are about a millimetre or two: the picture is alike a little way off and
    /// unrelated further on. (The texture tile is TILE metres across.)
    #[test]
    fn the_beads_are_a_millimetre_or_two_across() {
        let t = foam_texture();
        let g = grey(&t);
        let w = t.width() as usize;
        let px_per_mm = w as f32 / (TILE * 1000.0);
        let near = autocorrelation(&g, w, (0.4 * px_per_mm).round().max(1.0) as usize);
        let far = autocorrelation(&g, w, (6.0 * px_per_mm).round() as usize);
        assert!(near > 0.35, "no structure at 0.4 mm ({near})");
        assert!(far.abs() < 0.15, "still correlated at 6 mm ({far})");
    }

    #[test]
    fn the_foam_texture_tiles() {
        let t = foam_texture();
        let g = grey(&t);
        let w = t.width() as usize;
        // The wrap-around step is no rougher than any other step.
        let step = |x0: usize, x1: usize| (0..w).map(|y| (g[y * w + x0] - g[y * w + x1]).abs()).sum::<f32>() / w as f32;
        let seam = step(w - 1, 0);
        let inside = step(w / 2, w / 2 + 1);
        assert!(seam < inside * 1.8 + 0.01, "a seam: {seam} against {inside}");
    }

    /// Mostly flat, with grain: the normals lean a little, and balanced.
    #[test]
    fn the_foam_normals_are_grainy_but_flat() {
        let n = foam_normal();
        let data = n.data.as_ref().unwrap();
        let count = (n.width() * n.height()) as usize;
        let (mut z, mut x2, mut xs) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..count {
            let nx = data[i * 4] as f32 / 255.0 * 2.0 - 1.0;
            z += data[i * 4 + 2] as f32 / 255.0 * 2.0 - 1.0;
            x2 += nx * nx;
            xs += nx;
        }
        let (z, rms, bias) = (z / count as f32, (x2 / count as f32).sqrt(), xs / count as f32);
        assert!(z > 0.85, "the surface is not flat: {z}");
        assert!(rms > 0.05, "no grain in the normals ({rms})");
        assert!(bias.abs() < 0.03, "the grain leans one way ({bias})");
    }

    /// Foam grain is the same size on every part, whatever its UVs were: the projection is by
    /// size in metres, onto the plane the face is nearest to.
    #[test]
    fn the_grain_is_the_same_size_on_every_surface() {
        // A 0.2 m box: a texture tile is 0.1 m, so a face spans two tiles of texture.
        let m = world_uv(Cuboid::new(0.2, 0.2, 0.2).mesh().build().with_generated_tangents().unwrap());
        let uv = match m.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(v)) => v.clone(),
            _ => panic!("uvs"),
        };
        let pos = m.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).unwrap().to_vec();
        let idx: Vec<u32> = m.indices().unwrap().iter().map(|i| i as u32).collect();
        for t in idx.chunks(3) {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let world = (Vec3::from(pos[b]) - Vec3::from(pos[a])).cross(Vec3::from(pos[c]) - Vec3::from(pos[a])).length() * 0.5;
            let tex = (Vec2::from(uv[b]) - Vec2::from(uv[a])).perp_dot(Vec2::from(uv[c]) - Vec2::from(uv[a])).abs() * 0.5;
            let ratio = tex * TILE * TILE / world;
            assert!((ratio - 1.0).abs() < 0.05, "a face is {ratio} times its true size in the texture");
        }
        assert!(m.attribute(Mesh::ATTRIBUTE_TANGENT).is_some(), "the normal map needs tangents");
        assert_eq!(m.indices().unwrap().len() / 3, 12, "same triangles");
    }

    /// Tilted faces stretch the texture by at most what the plane they are projected on allows.
    #[test]
    fn a_slanted_face_is_not_stretched_much() {
        let sphere = Sphere::new(0.1).mesh().ico(3).unwrap().with_generated_tangents().unwrap();
        let m = world_uv(sphere);
        let uv = match m.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(v)) => v.clone(),
            _ => panic!("uvs"),
        };
        let pos = m.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).unwrap().to_vec();
        let idx: Vec<u32> = m.indices().unwrap().iter().map(|i| i as u32).collect();
        let mut worst = 0.0f32;
        for t in idx.chunks(3) {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let world = (Vec3::from(pos[b]) - Vec3::from(pos[a])).cross(Vec3::from(pos[c]) - Vec3::from(pos[a])).length() * 0.5;
            let tex = (Vec2::from(uv[b]) - Vec2::from(uv[a])).perp_dot(Vec2::from(uv[c]) - Vec2::from(uv[a])).abs() * 0.5;
            worst = worst.max((world / (tex * TILE * TILE)).max(tex * TILE * TILE / world));
        }
        assert!(worst < 1.8, "a face is stretched {worst} times");
    }
}
