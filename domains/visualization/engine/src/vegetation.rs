//! Procedural spruce, aspen and granite boulder meshes, and their placement.
//!
//! All meshes are unit-sized and shared between instances (one BLAS each for Solari);
//! variety comes from several variants, per-instance scale/rotation and material tints.

use crate::mesh_util::GeoBuilder;
use crate::model::rt;
use crate::terrain::{self, gradient_noise};
use bevy::prelude::*;
use std::f32::consts::TAU;

fn hash3(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Smooth 3D value noise in [0, 1].
pub(crate) fn noise3(p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let (ix, iy, iz) = (i.x as i32, i.y as i32, i.z as i32);
    let mut acc = 0.0;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let w = (if dx == 1 { u.x } else { 1.0 - u.x })
                    * (if dy == 1 { u.y } else { 1.0 - u.y })
                    * (if dz == 1 { u.z } else { 1.0 - u.z });
                acc += w * hash3(ix + dx, iy + dy, iz + dz);
            }
        }
    }
    acc
}

pub(crate) fn rnd(seed: u32, k: u32) -> f32 {
    hash3(seed as i32, k as i32, 911)
}

/// Spruce/fir foliage: stacked drooping whorls with star-shaped (branch tip) edges.
/// Unit height, base at y = 0.12 (trunk below).
fn spruce_foliage(seed: u32) -> Mesh {
    let mut b = GeoBuilder::default();
    let tiers = 9u32;
    let segs = 18usize;
    let base_r = 0.17 + 0.04 * rnd(seed, 1);
    for t in 0..tiers {
        let f = t as f32 / tiers as f32;
        let y_bottom = 0.12 + f * 0.82;
        let y_top = y_bottom + 0.82 / tiers as f32 * 1.9;
        let r = base_r * (1.0 - f).powf(0.85) + 0.012;
        let rot = rnd(seed, 10 + t) * TAU;
        let apex = b.vert(Vec3::new(0.0, y_top.min(1.0), 0.0), Vec2::new(0.5, 1.0));
        let mut ring = Vec::new();
        for s in 0..=segs {
            let a = rot + s as f32 / segs as f32 * TAU;
            // Alternate long/short spokes for a ragged branch-tip silhouette.
            let spoke = if s % 2 == 0 { 1.0 } else { 0.72 };
            let jitter = 0.85 + 0.3 * rnd(seed, 100 + t * 40 + s as u32);
            let rr = r * spoke * jitter;
            let droop = 0.025 * spoke;
            let p = Vec3::new(a.cos() * rr, y_bottom - droop, a.sin() * rr);
            ring.push(b.vert(p, Vec2::new(s as f32 / segs as f32, 0.0)));
        }
        for s in 0..segs {
            let mid = (b.pos[ring[s] as usize] + b.pos[ring[s + 1] as usize]) * 0.5;
            let out = Vec3::new(mid.x, 0.35 * r, mid.z);
            b.tri(apex, ring[s], ring[s + 1], out);
        }
        // Underside so the tier isn't see-through from below.
        let under = b.vert(Vec3::new(0.0, y_bottom + 0.02, 0.0), Vec2::new(0.5, 0.0));
        for s in 0..segs {
            b.tri(under, ring[s + 1], ring[s], Vec3::NEG_Y);
        }
    }
    b.build()
}

/// Smooth bell-shaped cone (unit height, crown from y = 0.12 to the tip): the ray-tracing
/// proxy for a spruce's needle mass. A smooth shape avoids the deep self-shadowed pockets a
/// tiered shape gets under ray-traced sunlight (which made distant spruces render as black
/// triangles).
fn spruce_proxy() -> Mesh {
    let mut b = GeoBuilder::default();
    let segs = 14;
    let rings = 8;
    let mut grid = Vec::new();
    for r in 0..=rings {
        let t = r as f32 / rings as f32; // 0 at the bottom of the crown, 1 at the tip
        let y = 0.12 + 0.88 * t;
        let radius = 0.19 * (1.0 - t).powf(0.85);
        let row: Vec<u32> = (0..=segs)
            .map(|k| {
                let a = k as f32 / segs as f32 * TAU;
                b.vert(Vec3::new(a.cos() * radius, y, a.sin() * radius), Vec2::new(k as f32 / segs as f32, t))
            })
            .collect();
        grid.push(row);
    }
    for r in 0..rings {
        for k in 0..segs {
            let mid = b.pos[grid[r][k] as usize];
            b.quad(grid[r][k], grid[r][k + 1], grid[r + 1][k + 1], grid[r + 1][k], Vec3::new(mid.x, 0.3, mid.z));
        }
    }
    let centre = b.vert(Vec3::new(0.0, 0.12, 0.0), Vec2::ZERO);
    for k in 0..segs {
        b.tri(centre, grid[0][k + 1], grid[0][k], Vec3::NEG_Y);
    }
    b.build()
}

/// Distant spruce (beyond ~900 m, a few pixels tall): a plain 7-sided cone.
fn spruce_distant() -> Mesh {
    let mut b = GeoBuilder::default();
    let segs = 7;
    let tip = b.vert(Vec3::new(0.0, 1.0, 0.0), Vec2::new(0.5, 1.0));
    let ring: Vec<u32> = (0..=segs)
        .map(|k| {
            let a = k as f32 / segs as f32 * TAU;
            b.vert(Vec3::new(a.cos() * 0.19, 0.08, a.sin() * 0.19), Vec2::new(k as f32 / segs as f32, 0.0))
        })
        .collect();
    let centre = b.vert(Vec3::new(0.0, 0.08, 0.0), Vec2::ZERO);
    for k in 0..segs {
        let mid = (b.pos[ring[k] as usize] + b.pos[ring[k + 1] as usize]) * 0.5;
        b.tri(tip, ring[k], ring[k + 1], Vec3::new(mid.x, 0.2, mid.z));
        b.tri(centre, ring[k + 1], ring[k], Vec3::NEG_Y);
    }
    b.build()
}

fn trunk(radius: f32, height: f32, taper: f32) -> Mesh {
    let mut b = GeoBuilder::default();
    let segs = 8;
    let mut rings = Vec::new();
    for (k, y) in [0.0f32, height].iter().enumerate() {
        let r = if k == 0 { radius } else { radius * taper };
        let ring: Vec<u32> = (0..=segs)
            .map(|s| {
                let a = s as f32 / segs as f32 * TAU;
                b.vert(Vec3::new(a.cos() * r, *y, a.sin() * r), Vec2::new(s as f32 / segs as f32, k as f32))
            })
            .collect();
        rings.push(ring);
    }
    for s in 0..segs {
        let mid = (b.pos[rings[0][s] as usize] + b.pos[rings[0][s + 1] as usize]) * 0.5;
        b.quad(rings[0][s], rings[0][s + 1], rings[1][s + 1], rings[1][s], Vec3::new(mid.x, 0.0, mid.z));
    }
    b.build()
}

/// Lumpy blob cluster (aspen canopy or boulder): displaced icospheres (subdivision `detail`)
/// merged.
pub fn lumpy(
    seed: u32,
    blobs: usize,
    spread: Vec3,
    blob_r: f32,
    bump: f32,
    flat_bottom: Option<f32>,
    detail: u32,
) -> Mesh {
    let ico = Sphere::new(1.0).mesh().ico(detail).unwrap();
    let base_pos: Vec<Vec3> = match ico.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.iter().map(|p| Vec3::from_array(*p)).collect(),
        _ => unreachable!(),
    };
    let base_idx: Vec<u32> = ico.indices().unwrap().iter().map(|i| i as u32).collect();
    let mut b = GeoBuilder::default();
    for k in 0..blobs {
        let c = if k == 0 {
            Vec3::ZERO
        } else {
            Vec3::new(
                rnd(seed, 3 * k as u32) - 0.5,
                rnd(seed, 3 * k as u32 + 1) - 0.5,
                rnd(seed, 3 * k as u32 + 2) - 0.5,
            ) * spread
        };
        let r = blob_r * (0.75 + 0.5 * rnd(seed, 500 + k as u32));
        let off = Vec3::splat(seed as f32 * 3.7 + k as f32 * 11.0);
        let start = b.pos.len() as u32;
        for p in &base_pos {
            let n = noise3(*p * 2.2 + off) * 0.65 + noise3(*p * 5.0 + off) * 0.35;
            let mut q = c + *p * r * (1.0 - bump + 2.0 * bump * n);
            if let Some(fb) = flat_bottom {
                q.y = q.y.max(fb);
            }
            let uv = Vec2::new(p.x.atan2(p.z) / TAU + 0.5, p.y * 0.5 + 0.5);
            b.vert(q, uv);
        }
        for t in base_idx.chunks(3) {
            let (i0, i1, i2) = (start + t[0], start + t[1], start + t[2]);
            let mid = (b.pos[i0 as usize] + b.pos[i1 as usize] + b.pos[i2 as usize]) / 3.0;
            b.tri(i0, i1, i2, mid - c);
        }
    }
    b.build()
}

/// Trees within this distance of the field (m) cast ray-traced branch shadows.
const RT_WOOD_RADIUS: f32 = 400.0;

/// Generated trees are built at these heights and instanced with a scale factor.
const SPRUCE_REF: f32 = 20.0;
/// How much light leaves pass through, as emission in nits (full sun on a leaf is about 5000).
const LEAF_GLOW: f32 = 2500.0;

/// The ray tracer's stand-ins for the crowns block the sun completely, so the crowns' drawn
/// surfaces glow a little in their own colour to stand for the light through the leaves. The
/// rasterized view has shadow maps that the cards do not fully block, so a crown is lit as it
/// is and a glow would only blow it out.
fn leaf_glow(raytracing: bool, over: Option<f32>) -> f32 {
    over.unwrap_or(if raytracing { LEAF_GLOW } else { 0.0 })
}
const SAPLING_REF: f32 = 4.0;
const SHRUB_REF: f32 = 1.5;
/// How many different trees of each kind are grown; instances pick one by index.
const SPRUCE_VARIANTS: u32 = 10;
const ASPEN_VARIANTS: u32 = 6;
const ASPEN_REF: f32 = 14.0;

/// CPU-side vegetation assets, built on a background thread while the loading screen shows.
pub struct VegetationBuild {
    spray_tex: Vec<Image>,
    leaf_tex: Vec<Image>,
    bark_tex: Image,
    aspen_bark_tex: Image,
    granite_tex: Image,
    granite_normal: Image,
    cobble_meshes: Vec<Mesh>,
    spruce_gen: Vec<(Mesh, Mesh)>,
    aspen_gen: Vec<(Mesh, Mesh)>,
    spruce_mid: Vec<(Mesh, Mesh)>,
    aspen_mid: Vec<(Mesh, Mesh)>,
    snag_gen: Vec<(Mesh, Mesh)>,
    snag_mid: Vec<(Mesh, Mesh)>,
    sapling_gen: Vec<(Mesh, Mesh)>,
    shrub_gen: Vec<(Mesh, Mesh)>,
    spruce_solid: Vec<Mesh>,
    aspen_solid: Vec<Mesh>,
    aspen_trunk: Mesh,
    spruce_proxy: Mesh,
    spruce_distant: Mesh,
    aspen_distant: Mesh,
    boulder_meshes: Vec<Mesh>,
    /// (variant index, transform)
    boulders: Vec<(usize, Transform)>,
}

/// GPU handles shared by every tree instance.
#[derive(Resource, Clone)]
pub struct VegetationHandles {
    spruce_gen: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    aspen_gen: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    /// The same trees with a tenth of the branches, for the middle distance.
    spruce_mid: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    aspen_mid: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    snag_gen: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    snag_mid: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    sapling_gen: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    shrub_gen: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    spruce_solid: Vec<Handle<Mesh>>,
    aspen_solid: Vec<Handle<Mesh>>,
    aspen_trunk: Handle<Mesh>,
    spruce_proxy: Handle<Mesh>,
    spruce_distant: Handle<Mesh>,
    aspen_distant: Handle<Mesh>,
    spruce_solid_mats: Vec<Handle<StandardMaterial>>,
    /// The needle mass as reflections and bounced light see it: a smooth cone has none of the
    /// shadowed gaps between branches, so it is darker than the crown it stands in for.
    spruce_proxy_mats: Vec<Handle<StandardMaterial>>,
    aspen_solid_mats: Vec<Handle<StandardMaterial>>,
    /// The aspen crowns as the ray tracer sees them: the same colours, and no glow.
    aspen_proxy_mats: Vec<Handle<StandardMaterial>>,
    spray_mats: Vec<Handle<StandardMaterial>>,
    leaf_mats: Vec<Handle<StandardMaterial>>,
    bark: Handle<StandardMaterial>,
    aspen_bark: Handle<StandardMaterial>,
    /// Weathered grey wood for dead trees.
    snag_bark: Handle<StandardMaterial>,
}

/// Pure CPU work: textures, tree meshes and boulder placement. `step` reports progress.
pub fn build_vegetation(mut step: impl FnMut(&str)) -> VegetationBuild {
    use crate::foliage;
    step("Painting needles and leaves");
    let spray_tex: Vec<_> = (0..3).map(foliage::spruce_spray_texture).collect();
    let leaf_tex: Vec<_> = (0..2).map(foliage::aspen_leaf_texture).collect();
    let bark_tex = foliage::bark_spruce_texture();
    let aspen_bark_tex = foliage::bark_aspen_texture();
    step("Growing spruce and aspen");
    use crate::treegen::{Lod, Species};
    let grow = |sp, seed, h, lod| {
        let t = crate::treegen::generate_lod(sp, seed, h, lod);
        (t.wood, t.foliage)
    };
    let spruce_gen: Vec<_> =
        (0..SPRUCE_VARIANTS).map(|s| grow(Species::Spruce, 11 + s, SPRUCE_REF, Lod::Full)).collect();
    let aspen_gen: Vec<_> = (0..ASPEN_VARIANTS).map(|s| grow(Species::Aspen, 51 + s, ASPEN_REF, Lod::Full)).collect();
    // The same seeds, so that a tree keeps its shape as it comes closer.
    let spruce_mid: Vec<_> =
        (0..SPRUCE_VARIANTS).map(|s| grow(Species::Spruce, 11 + s, SPRUCE_REF, Lod::Mid)).collect();
    let aspen_mid: Vec<_> = (0..ASPEN_VARIANTS).map(|s| grow(Species::Aspen, 51 + s, ASPEN_REF, Lod::Mid)).collect();
    let snag_gen: Vec<_> = (0..4).map(|s| grow(Species::Snag, 31 + s, SPRUCE_REF, Lod::Full)).collect();
    let snag_mid: Vec<_> = (0..4).map(|s| grow(Species::Snag, 31 + s, SPRUCE_REF, Lod::Mid)).collect();
    let sapling_gen: Vec<_> = (0..6).map(|s| grow(Species::Spruce, 71 + s, SAPLING_REF, Lod::Low)).collect();
    let shrub_gen: Vec<_> = (0..6).map(|s| grow(Species::Shrub, 91 + s, SHRUB_REF, Lod::Low)).collect();
    let spruce_solid: Vec<_> = (0..5).map(spruce_foliage).collect();
    // Far crowns are seen from 200 m out: a coarse icosphere per blob is plenty.
    let aspen_solid: Vec<_> =
        (0..4).map(|s| lumpy(100 + s, 6, Vec3::new(0.45, 0.9, 0.45), 0.42, 0.25, None, 1)).collect();
    let aspen_distant = lumpy(150, 1, Vec3::ZERO, 0.55, 0.2, None, 0);
    let spruce_distant = spruce_distant();
    let spruce_proxy = spruce_proxy();
    let aspen_trunk = trunk(0.02, 0.85, 0.35);

    step("Placing granite boulders");
    let granite_tex = crate::rocks::granite_texture();
    let granite_normal = crate::rocks::granite_normal();
    use crate::rocks::{self, Size};
    let boulder_meshes: Vec<_> = (1..=10).map(|k| rocks::rock(k, Size::Boulder)).collect();
    let cobble_meshes: Vec<_> = (1..=10).map(|k| rocks::rock(100 + k, Size::Cobble)).collect();
    let mut boulders = Vec::new();
    let mut place = |x: f32, z: f32, size: f32, sink: f32, k: u32| {
        let h = terrain::ground_height(x, z);
        let n = terrain::ground_normal(x, z);
        let rot =
            Quat::from_rotation_arc(Vec3::Y, n.lerp(Vec3::Y, 0.5).normalize()) * Quat::from_rotation_y(rnd(k, 1) * TAU);
        let s =
            Vec3::new(size * (0.8 + 0.4 * rnd(k, 2)), size * (0.55 + 0.35 * rnd(k, 3)), size * (0.8 + 0.4 * rnd(k, 4)));
        boulders.push((k as usize, Transform::from_xyz(x, h - sink * s.y, z).with_rotation(rot).with_scale(s)));
        // A big boulder has smaller ones round it, where pieces have broken off and rolled.
        if size > 2.2 && k < 1000 {
            for j in 0..(2 + (rnd(k, 60) * 3.0) as u32) {
                let a = rnd(k, 61 + j) * TAU;
                let d = size * (0.9 + 0.8 * rnd(k, 70 + j));
                let (sx, sz) = (x + a.cos() * d, z + a.sin() * d);
                let small = size * (0.12 + 0.28 * rnd(k, 80 + j));
                let sh = terrain::ground_height(sx, sz);
                let spin = Quat::from_rotation_y(rnd(k, 90 + j) * TAU);
                let sv = Vec3::new(small, small * (0.5 + 0.4 * rnd(k, 100 + j)), small);
                boulders.push((
                    5000 + (k as usize) * 8 + j as usize,
                    Transform::from_xyz(sx, sh - 0.3 * sv.y, sz).with_rotation(spin).with_scale(sv),
                ));
            }
        }
    };
    // Lake shore boulders, some standing in the water (the Alpine Valley only).
    let alpine = terrain::real().is_none();
    let lc = terrain::LAKE_CENTER;
    for k in 0..if alpine { 70u32 } else { 0 } {
        let a = rnd(k, 20) * TAU;
        // Walk out from the centre to the shoreline, then scatter around it.
        let dir = Vec2::new(a.cos(), a.sin());
        let mut d = 0.0;
        while terrain::lake_factor(lc.x + dir.x * d, lc.y + dir.y * d) < 1.0 && d < 400.0 {
            d += 2.0;
        }
        let d = d * (0.88 + 0.22 * rnd(k, 21));
        let size = 0.8 + 2.4 * rnd(k, 22).powi(2);
        place(lc.x + dir.x * d, lc.y + dir.y * d, size, 0.15, k);
    }
    // Cobbles along the waterline, some under the water, as a stream or a lake leaves them.
    for k in 0..if alpine { 900u32 } else { 0 } {
        let a = rnd(k, 40) * TAU;
        let dir = Vec2::new(a.cos(), a.sin());
        let mut d = 0.0;
        while terrain::ground_height(lc.x + dir.x * d, lc.y + dir.y * d) < terrain::WATER_LEVEL && d < 400.0 {
            d += 0.5;
        }
        let d = d + (rnd(k, 41) - 0.6) * 5.0;
        let size = 0.12 + 0.55 * rnd(k, 42).powi(3);
        place(lc.x + dir.x * d, lc.y + dir.y * d, size, 0.3, 1000 + k);
    }
    // Outcrops on steep hillsides, and scattered erratics in the meadows.
    let spacing = 34.0;
    let n = if alpine { (1700.0 / spacing) as i32 } else { -1 };
    for iz in -n..=n {
        for ix in -n..=n {
            let k = ((ix + 500) * 2003 + iz + 500) as u32;
            let x = (ix as f32 + rnd(k, 30) - 0.5) * spacing;
            let z = (iz as f32 + rnd(k, 31) - 0.5) * spacing;
            let r = (x * x + z * z).sqrt();
            if r < 240.0 || r > 1700.0 || terrain::lake_factor(x, z) < 1.2 {
                continue;
            }
            let slope = 1.0 - terrain::ground_normal(x, z).y;
            let rocky =
                ((slope - 0.12) * 5.0).clamp(0.0, 1.0) * 0.6 + gradient_noise(x / 200.0, z / 200.0).max(0.0) * 0.25;
            if rnd(k, 32) < rocky {
                let size = 2.5 + 9.0 * rnd(k, 33).powi(2) * (0.5 + slope);
                place(x, z, size, 0.35, k);
            }
        }
    }
    VegetationBuild {
        spray_tex,
        leaf_tex,
        bark_tex,
        aspen_bark_tex,
        granite_tex,
        granite_normal,
        cobble_meshes,
        spruce_gen,
        aspen_gen,
        spruce_mid,
        aspen_mid,
        snag_gen,
        snag_mid,
        sapling_gen,
        shrub_gen,
        spruce_solid,
        aspen_solid,
        aspen_trunk,
        spruce_proxy,
        spruce_distant,
        aspen_distant,
        boulder_meshes,
        boulders,
    }
}

/// Main thread: upload assets, create materials, spawn boulders. Trees are spawned in
/// batches afterwards with `spawn_tree`.
pub fn upload_vegetation(
    b: VegetationBuild,
    raytracing: bool,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> VegetationHandles {
    let mat = |m: &mut Assets<StandardMaterial>, c: Color, rough: f32| {
        m.add(StandardMaterial { base_color: c, perceptual_roughness: rough, reflectance: 0.3, ..default() })
    };
    // The solid crowns seen far away let the sun through as the cards do.
    let glow = leaf_glow(raytracing, std::env::var("SIM001_LEAF_GLOW").ok().and_then(|v| v.parse::<f32>().ok()));
    let lit_mat = |m: &mut Assets<StandardMaterial>, c: Color, rough: f32| {
        let l = c.to_linear();
        m.add(StandardMaterial {
            base_color: c,
            emissive: LinearRgba::rgb(l.red * glow, l.green * glow, l.blue * glow),
            perceptual_roughness: rough,
            reflectance: 0.3,
            ..default()
        })
    };
    // Solid crowns: used far away (raster) and as the ray-tracing proxy (shadows/reflections).
    let spruce_solid_mats: Vec<_> =
        [(0.026, 0.058, 0.032), (0.031, 0.066, 0.032), (0.024, 0.054, 0.036), (0.036, 0.074, 0.032)]
            .iter()
            .map(|&(r, g, b)| lit_mat(materials, Color::linear_rgb(r, g, b), 0.85))
            .collect();
    let spruce_proxy_mats: Vec<_> =
        [(0.026, 0.058, 0.032), (0.031, 0.066, 0.032), (0.024, 0.054, 0.036), (0.036, 0.074, 0.032)]
            .iter()
            .map(|&(r, g, b)| mat(materials, Color::linear_rgb(r * 0.3, g * 0.3, b * 0.3), 0.9))
            .collect();
    let aspen_solid_mats: Vec<_> = [(0.060, 0.120, 0.028), (0.078, 0.135, 0.030), (0.050, 0.100, 0.026)]
        .iter()
        .map(|&(r, g, b)| lit_mat(materials, Color::linear_rgb(r, g, b), 0.75))
        .collect();
    let aspen_proxy_mats: Vec<_> = [(0.060, 0.120, 0.028), (0.078, 0.135, 0.030), (0.050, 0.100, 0.026)]
        .iter()
        .map(|&(r, g, b)| mat(materials, Color::linear_rgb(r, g, b), 0.75))
        .collect();
    // Near: alpha-cut foliage cards with painted needles / leaves (raster only).
    let card_mat = |m: &mut Assets<StandardMaterial>, tex: Handle<Image>, tint: Vec3| {
        // Leaves and needles pass some of the sun's light through: the side of a crown in its own
        // shadow is not black but glows, dimly, in the leaves' colour. (The ray tracer's
        // stand-ins for the crowns block the sun completely, so the glow is added here.)
        let glow = leaf_glow(raytracing, std::env::var("SIM001_LEAF_GLOW").ok().and_then(|v| v.parse::<f32>().ok()));
        m.add(StandardMaterial {
            base_color: Color::linear_rgb(tint.x, tint.y, tint.z),
            emissive: LinearRgba::rgb(tint.x * glow, tint.y * glow, tint.z * glow),
            emissive_texture: Some(tex.clone()),
            base_color_texture: Some(tex),
            alpha_mode: AlphaMode::Mask(0.5),
            // Bent normals are authored per vertex; keep them on both sides.
            double_sided: false,
            cull_mode: None,
            perceptual_roughness: 0.8,
            reflectance: 0.25,
            ..default()
        })
    };
    let spray_tex: Vec<_> = b.spray_tex.into_iter().map(|i| images.add(i)).collect();
    let leaf_tex: Vec<_> = b.leaf_tex.into_iter().map(|i| images.add(i)).collect();
    let tints =
        [Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.1, 1.08, 0.95), Vec3::new(0.88, 0.92, 1.05), Vec3::new(1.2, 1.15, 0.9)];
    // Spruce needles a little darker and bluer than the painted texture (Engelmann / blue spruce).
    let spray_mats: Vec<_> =
        (0..4).map(|k| card_mat(materials, spray_tex[k % 3].clone(), tints[k] * Vec3::new(0.72, 0.76, 0.86))).collect();
    let leaf_mats: Vec<_> = (0..3).map(|k| card_mat(materials, leaf_tex[k % 2].clone(), tints[k])).collect();
    let bark_mat = |m: &mut Assets<StandardMaterial>, tex: Handle<Image>| {
        m.add(StandardMaterial {
            base_color_texture: Some(tex),
            perceptual_roughness: 0.9,
            reflectance: 0.25,
            uv_transform: bevy::math::Affine2::from_scale(Vec2::new(2.0, 10.0)),
            ..default()
        })
    };
    let bark_image = images.add(b.bark_tex);
    let bark = bark_mat(materials, bark_image.clone());
    let aspen_bark = bark_mat(materials, images.add(b.aspen_bark_tex));
    // Dead wood is silver-grey, bleached by sun and snow.
    let snag_bark = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.34, 0.32, 0.30),
        base_color_texture: Some(bark_image),
        perceptual_roughness: 0.95,
        reflectance: 0.2,
        uv_transform: bevy::math::Affine2::from_scale(Vec2::new(2.0, 10.0)),
        ..default()
    });

    // Boulders.
    let granite = images.add(b.granite_tex);
    let granite_normal = images.add(b.granite_normal);
    let rock_mats: Vec<_> = [1.0f32, 0.85, 1.12]
        .iter()
        .map(|&k| {
            materials.add(StandardMaterial {
                base_color: Color::linear_rgb(k, k, k * 0.98),
                base_color_texture: Some(granite.clone()),
                normal_map_texture: Some(granite_normal.clone()),
                perceptual_roughness: 0.82,
                reflectance: 0.3,
                ..default()
            })
        })
        .collect();
    let boulder_meshes: Vec<_> = b.boulder_meshes.into_iter().map(|m| meshes.add(m)).collect();
    let cobble_meshes: Vec<_> = b.cobble_meshes.into_iter().map(|m| meshes.add(m)).collect();
    for (k, tf) in &b.boulders {
        // Cobbles and the small pieces round a boulder are the cheap meshes.
        let mesh = if *k >= 1000 {
            &cobble_meshes[k % cobble_meshes.len()]
        } else {
            &boulder_meshes[k % boulder_meshes.len()]
        };
        commands.spawn(rt(mesh.clone(), rock_mats[k % rock_mats.len()].clone(), *tf));
    }
    info!("placed {} boulders", b.boulders.len());

    VegetationHandles {
        spruce_gen: b.spruce_gen.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        aspen_gen: b.aspen_gen.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        spruce_mid: b.spruce_mid.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        snag_gen: b.snag_gen.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        snag_mid: b.snag_mid.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        sapling_gen: b.sapling_gen.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        shrub_gen: b.shrub_gen.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        aspen_mid: b.aspen_mid.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        spruce_solid: b.spruce_solid.into_iter().map(|m| meshes.add(m)).collect(),
        aspen_solid: b.aspen_solid.into_iter().map(|m| meshes.add(m)).collect(),
        aspen_trunk: meshes.add(b.aspen_trunk),
        spruce_proxy: meshes.add(b.spruce_proxy),
        spruce_distant: meshes.add(b.spruce_distant),
        aspen_distant: meshes.add(b.aspen_distant),
        spruce_solid_mats,
        spruce_proxy_mats,
        aspen_solid_mats,
        aspen_proxy_mats,
        spray_mats,
        leaf_mats,
        bark,
        aspen_bark,
        snag_bark,
    }
}

/// Where each version of a tree is drawn (metres from the camera): the full tree, a reduced one,
/// a solid crown, and a plain cone. They hand over without a fade: a cross-fade is dithered, and
/// the temporal accumulation turns that into a see-through tree.
pub fn tree_ranges() -> [bevy::camera::visibility::VisibilityRange; 4] {
    use bevy::camera::visibility::VisibilityRange as R;
    [R::abrupt(0.0, 140.0), R::abrupt(140.0, 700.0), R::abrupt(700.0, 1000.0), R::abrupt(1000.0, 1.0e7)]
}

impl VegetationHandles {
    /// The materials of everything the crowns put into the ray-traced scene.
    pub fn ray_traced_materials(&self) -> Vec<Handle<StandardMaterial>> {
        self.spruce_proxy_mats.iter().chain(&self.aspen_proxy_mats).cloned().collect()
    }
}

/// Spawns one tree (wood + foliage cards near, solid crown far, ray-tracing proxy).
pub fn spawn_tree(commands: &mut Commands, v: &VegetationHandles, i: usize, t: &terrain::Tree) {
    use bevy::solari::prelude::RaytracingMesh3d;
    // The full trees are 6-20k triangles each and every camera draws them, so they stop at 140 m;
    // a reduced tree (a few hundred triangles) carries on to 700 m, then a solid crown.
    let [near, mid, far, distant] = tree_ranges();
    // Branch shadows only matter close to where the aircraft flies; the proxies carry the
    // crowns' shadows everywhere.
    let rt_wood = Vec2::new(t.pos.x, t.pos.z).length() < RT_WOOD_RADIUS;

    // SIM001_PROXY=0 leaves the crowns out of the ray-traced scene (for looking at what they do).
    let proxies = std::env::var("SIM001_PROXY").map_or(true, |v| v != "0");
    let keep = std::env::var("SIM001_TREE_FRAC").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    if rnd(i as u32, 99) > keep {
        return;
    }
    let h = t.height;
    let yaw = Quat::from_rotation_y(rnd(i as u32, 7) * TAU);
    let tilt =
        Quat::from_rotation_x((rnd(i as u32, 8) - 0.5) * 0.06) * Quat::from_rotation_z((rnd(i as u32, 9) - 0.5) * 0.06);
    let base = t.pos - Vec3::Y * 0.3;
    let shade = t.shade as usize;
    if t.conifer && t.dead {
        // A snag: bare trunk and stubs, with nothing out past the mid range (a few pixels).
        let k = h / SPRUCE_REF;
        let gen_tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::splat(k));
        let (wood, cards) = &v.snag_gen[i % v.snag_gen.len()];
        if rt_wood {
            commands.spawn((RaytracingMesh3d(wood.clone()), MeshMaterial3d(v.snag_bark.clone()), gen_tf));
        }
        commands.spawn((Mesh3d(wood.clone()), MeshMaterial3d(v.snag_bark.clone()), gen_tf, near.clone()));
        commands.spawn((Mesh3d(cards.clone()), MeshMaterial3d(v.spray_mats[shade % 4].clone()), gen_tf, near));
        let (mid_wood, mid_cards) = &v.snag_mid[i % v.snag_mid.len()];
        commands.spawn((Mesh3d(mid_wood.clone()), MeshMaterial3d(v.snag_bark.clone()), gen_tf, mid.clone()));
        commands.spawn((Mesh3d(mid_cards.clone()), MeshMaterial3d(v.spray_mats[shade % 4].clone()), gen_tf, mid));
        return;
    }
    if t.conifer {
        // Narrow spires, as subalpine fir and Engelmann spruce grow where the snow is deep.
        let w = h * (0.55 + 0.3 * rnd(i as u32, 11));
        let (wood, cards) = &v.spruce_gen[i % v.spruce_gen.len()];
        let k = h / SPRUCE_REF;
        let gen_tf =
            Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::new(k * w / h, k, k * w / h));
        if rt_wood {
            commands.spawn((RaytracingMesh3d(wood.clone()), MeshMaterial3d(v.bark.clone()), gen_tf));
        }
        commands.spawn((Mesh3d(wood.clone()), MeshMaterial3d(v.bark.clone()), gen_tf, near.clone()));
        commands.spawn((Mesh3d(cards.clone()), MeshMaterial3d(v.spray_mats[shade % 4].clone()), gen_tf, near));
        let (mid_wood, mid_cards) = &v.spruce_mid[i % v.spruce_mid.len()];
        commands.spawn((Mesh3d(mid_wood.clone()), MeshMaterial3d(v.bark.clone()), gen_tf, mid.clone()));
        commands.spawn((Mesh3d(mid_cards.clone()), MeshMaterial3d(v.spray_mats[shade % 4].clone()), gen_tf, mid));
        let tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::new(w, h, w));
        let solid = v.spruce_solid[i % v.spruce_solid.len()].clone();
        let solid_mat = v.spruce_solid_mats[shade % 4].clone();
        commands.spawn((Mesh3d(solid), MeshMaterial3d(solid_mat.clone()), tf, far));
        commands.spawn((Mesh3d(v.spruce_distant.clone()), MeshMaterial3d(solid_mat.clone()), tf, distant));
        // Ray-tracing proxy for the needle mass: a smooth cone a bit inside the visible crown.
        let proxy = tf.with_scale(Vec3::new(w * 0.62, h * 0.92, w * 0.62));
        if proxies {
            commands.spawn((
                RaytracingMesh3d(v.spruce_proxy.clone()),
                MeshMaterial3d(v.spruce_proxy_mats[shade % 4].clone()),
                proxy,
            ));
        }
    } else {
        let h = h * 0.8;
        let (wood, cards) = &v.aspen_gen[i % v.aspen_gen.len()];
        let gen_tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::splat(h / ASPEN_REF));
        if rt_wood {
            commands.spawn((RaytracingMesh3d(wood.clone()), MeshMaterial3d(v.aspen_bark.clone()), gen_tf));
        }
        commands.spawn((Mesh3d(wood.clone()), MeshMaterial3d(v.aspen_bark.clone()), gen_tf, near.clone()));
        commands.spawn((Mesh3d(cards.clone()), MeshMaterial3d(v.leaf_mats[shade % 3].clone()), gen_tf, near));
        let (mid_wood, mid_cards) = &v.aspen_mid[i % v.aspen_mid.len()];
        commands.spawn((Mesh3d(mid_wood.clone()), MeshMaterial3d(v.aspen_bark.clone()), gen_tf, mid.clone()));
        commands.spawn((Mesh3d(mid_cards.clone()), MeshMaterial3d(v.leaf_mats[shade % 3].clone()), gen_tf, mid));
        let tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::splat(h));
        commands.spawn((Mesh3d(v.aspen_trunk.clone()), MeshMaterial3d(v.aspen_bark.clone()), tf, far.clone()));
        let canopy = Transform::from_translation(base + Vec3::Y * h * 0.68).with_rotation(yaw).with_scale(Vec3::new(
            h * 0.36,
            h * 0.5,
            h * 0.36,
        ));
        let solid = v.aspen_solid[i % v.aspen_solid.len()].clone();
        let solid_mat = v.aspen_solid_mats[shade % 3].clone();
        commands.spawn((Mesh3d(solid.clone()), MeshMaterial3d(solid_mat.clone()), canopy, far));
        commands.spawn((Mesh3d(v.aspen_distant.clone()), MeshMaterial3d(solid_mat.clone()), canopy, distant));
        let proxy = canopy.with_scale(canopy.scale * 0.75);
        if proxies {
            commands.spawn((RaytracingMesh3d(solid), MeshMaterial3d(v.aspen_proxy_mats[shade % 3].clone()), proxy));
        }
    }
}

/// Spawns one sapling or shrub: raster only, drawn out to a couple of hundred metres.
pub fn spawn_plant(commands: &mut Commands, v: &VegetationHandles, i: usize, p: &terrain::Plant) {
    use bevy::camera::visibility::VisibilityRange;
    // SIM001_PLANT_FRAC keeps only a share of the saplings and shrubs (for timing).
    let keep = std::env::var("SIM001_PLANT_FRAC").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    if rnd(i as u32, 98) > keep {
        return;
    }
    let yaw = Quat::from_rotation_y(rnd(i as u32, 17) * TAU);
    let tilt = Quat::from_rotation_x((rnd(i as u32, 18) - 0.5) * 0.12)
        * Quat::from_rotation_z((rnd(i as u32, 19) - 0.5) * 0.12);
    let base = p.pos - Vec3::Y * 0.1;
    let shade = p.shade as usize;
    let (wood, cards) = match p.kind {
        terrain::PlantKind::Sapling => &v.sapling_gen[i % v.sapling_gen.len()],
        terrain::PlantKind::Shrub => &v.shrub_gen[i % v.shrub_gen.len()],
    };
    let (reference, reach) = match p.kind {
        terrain::PlantKind::Sapling => (SAPLING_REF, 220.0),
        terrain::PlantKind::Shrub => (SHRUB_REF, 160.0),
    };
    let tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::splat(p.height / reference));
    let range = VisibilityRange::abrupt(0.0, reach);
    let (wood_mat, card_mat) = match p.kind {
        terrain::PlantKind::Sapling => (v.bark.clone(), v.spray_mats[shade % 4].clone()),
        terrain::PlantKind::Shrub => (v.aspen_bark.clone(), v.leaf_mats[shade % 3].clone()),
    };
    commands.spawn((Mesh3d(wood.clone()), MeshMaterial3d(wood_mat), tf, range.clone()));
    commands.spawn((Mesh3d(cards.clone()), MeshMaterial3d(card_mat), tf, range));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The leaves' glow stands in for light the ray tracer's solid crowns block; the rasterized
    /// view has none of that problem, and a glow there only blows the trees out white.
    #[test]
    fn leaves_glow_only_in_the_ray_traced_view() {
        assert_eq!(leaf_glow(false, None), 0.0);
        assert_eq!(leaf_glow(true, None), LEAF_GLOW);
        assert_eq!(leaf_glow(false, Some(900.0)), 900.0, "an override still works");
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let mut images = Assets::<Image>::default();
        let mut world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let handles =
            upload_vegetation(build_vegetation(|_| {}), false, &mut commands, &mut meshes, &mut materials, &mut images);
        queue.apply(&mut world);
        for h in handles.leaf_mats.iter().chain(&handles.spray_mats) {
            assert_eq!(materials.get(h).unwrap().emissive, LinearRgba::BLACK, "a raster leaf glows");
        }
    }

    /// The ray tracer treats a glowing mesh in its scene as a light and spends the sun's samples
    /// on it, which darkens everything else: so nothing the crowns put in the ray-traced scene
    /// may glow. (The glow that stands for light through leaves is on the drawn crowns only.)
    #[test]
    fn what_the_ray_tracer_sees_of_a_tree_does_not_glow() {
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let mut images = Assets::<Image>::default();
        let mut commands_world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &commands_world);
        let handles =
            upload_vegetation(build_vegetation(|_| {}), true, &mut commands, &mut meshes, &mut materials, &mut images);
        queue.apply(&mut commands_world);
        let ray_traced = handles.ray_traced_materials();
        assert!(!ray_traced.is_empty());
        for h in ray_traced {
            let m = materials.get(&h).expect("a material");
            assert_eq!(m.emissive, LinearRgba::BLACK, "a ray-traced crown glows");
        }
        // And the drawn crowns do glow: that is the light through the leaves.
        let drawn = materials.get(&handles.leaf_mats[0]).expect("leaf");
        assert!(drawn.emissive.green > 0.0, "the leaves let no light through");
    }

    /// Each version of a tree takes over where the one before it leaves off, with no band in
    /// which both are drawn (a cross-fade is dithered, and the temporal accumulation turns that
    /// into a see-through tree).
    #[test]
    fn the_tree_versions_hand_over_without_a_cross_fade() {
        let ranges = tree_ranges();
        assert_eq!(ranges.len(), 4, "full, mid, far, distant");
        assert_eq!(ranges[0].start_margin.start, 0.0);
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end_margin.end, pair[1].start_margin.start, "no gap or overlap");
        }
        for r in &ranges {
            assert!(r.is_abrupt(), "{:?} fades", (&r.start_margin, &r.end_margin));
        }
        assert!(
            ranges[0].end_margin.end >= 100.0 && ranges[0].end_margin.end <= 200.0,
            "the full tree is for the near range"
        );
        assert!(ranges[1].end_margin.end > 400.0, "the mid tree reaches well out");
    }
}
