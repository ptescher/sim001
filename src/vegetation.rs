//! Procedural spruce, aspen and granite boulder meshes, and their placement.
//!
//! All meshes are unit-sized and shared between instances (one BLAS each for Solari);
//! variety comes from several variants, per-instance scale/rotation and material tints.

use crate::mesh_util::{texture, GeoBuilder};
use crate::model::rt;
use crate::terrain::{self, fbm, gradient_noise};
use bevy::prelude::*;
use std::f32::consts::TAU;

fn hash3(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Smooth 3D value noise in [0, 1].
fn noise3(p: Vec3) -> f32 {
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

fn rnd(seed: u32, k: u32) -> f32 {
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

/// Lumpy blob cluster (aspen canopy or boulder): displaced icospheres merged.
pub fn lumpy(seed: u32, blobs: usize, spread: Vec3, blob_r: f32, bump: f32, flat_bottom: Option<f32>) -> Mesh {
    let ico = Sphere::new(1.0).mesh().ico(3).unwrap();
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
            Vec3::new(rnd(seed, 3 * k as u32) - 0.5, rnd(seed, 3 * k as u32 + 1) - 0.5, rnd(seed, 3 * k as u32 + 2) - 0.5)
                * spread
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

/// Generated trees are built at these heights and instanced with a scale factor.
const SPRUCE_REF: f32 = 20.0;
const ASPEN_REF: f32 = 14.0;

/// CPU-side vegetation assets, built on a background thread while the loading screen shows.
pub struct VegetationBuild {
    spray_tex: Vec<Image>,
    leaf_tex: Vec<Image>,
    bark_tex: Image,
    aspen_bark_tex: Image,
    granite_tex: Image,
    spruce_gen: Vec<(Mesh, Mesh)>,
    aspen_gen: Vec<(Mesh, Mesh)>,
    spruce_solid: Vec<Mesh>,
    aspen_solid: Vec<Mesh>,
    aspen_trunk: Mesh,
    spruce_proxy: Mesh,
    boulder_meshes: Vec<Mesh>,
    /// (variant index, transform)
    boulders: Vec<(usize, Transform)>,
}

/// GPU handles shared by every tree instance.
#[derive(Resource, Clone)]
pub struct VegetationHandles {
    spruce_gen: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    aspen_gen: Vec<(Handle<Mesh>, Handle<Mesh>)>,
    spruce_solid: Vec<Handle<Mesh>>,
    aspen_solid: Vec<Handle<Mesh>>,
    aspen_trunk: Handle<Mesh>,
    spruce_proxy: Handle<Mesh>,
    spruce_solid_mats: Vec<Handle<StandardMaterial>>,
    aspen_solid_mats: Vec<Handle<StandardMaterial>>,
    spray_mats: Vec<Handle<StandardMaterial>>,
    leaf_mats: Vec<Handle<StandardMaterial>>,
    bark: Handle<StandardMaterial>,
    aspen_bark: Handle<StandardMaterial>,
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
    let grow = |sp, seed, h| {
        let t = crate::treegen::generate(sp, seed, h);
        (t.wood, t.foliage)
    };
    let spruce_gen: Vec<_> = (0..4).map(|s| grow(crate::treegen::Species::Spruce, 11 + s, SPRUCE_REF)).collect();
    let aspen_gen: Vec<_> = (0..4).map(|s| grow(crate::treegen::Species::Aspen, 51 + s, ASPEN_REF)).collect();
    let spruce_solid: Vec<_> = (0..5).map(spruce_foliage).collect();
    let aspen_solid: Vec<_> = (0..4).map(|s| lumpy(100 + s, 6, Vec3::new(0.45, 0.9, 0.45), 0.42, 0.25, None)).collect();
    let spruce_proxy = spruce_proxy();
    let aspen_trunk = trunk(0.02, 0.85, 0.35);

    step("Placing granite boulders");
    let granite_tex = texture(512, |u, v| {
        let speck = fbm(u * 180.0, v * 180.0, 2);
        let grain = fbm(u * 40.0, v * 40.0, 3);
        let lichen = ((fbm(u * 9.0 + 4.0, v * 9.0, 4) - 0.55) * 4.0).clamp(0.0, 1.0);
        let g = 0.20 + 0.12 * grain + 0.10 * (speck - 0.5);
        let rock = [g * 1.02, g, g * 0.97];
        let lich = [0.16, 0.18, 0.08];
        [
            rock[0] + (lich[0] - rock[0]) * lichen * 0.6,
            rock[1] + (lich[1] - rock[1]) * lichen * 0.6,
            rock[2] + (lich[2] - rock[2]) * lichen * 0.6,
        ]
    });
    let boulder_meshes: Vec<_> =
        (0..6).map(|s| lumpy(300 + s, 1 + (s as usize % 3), Vec3::new(0.9, 0.3, 0.9), 0.75, 0.22, Some(-0.35))).collect();
    let mut boulders = Vec::new();
    let mut place = |x: f32, z: f32, size: f32, sink: f32, k: u32| {
        let h = terrain::ground_height(x, z);
        let n = terrain::ground_normal(x, z);
        let rot = Quat::from_rotation_arc(Vec3::Y, n.lerp(Vec3::Y, 0.5).normalize()) * Quat::from_rotation_y(rnd(k, 1) * TAU);
        let s = Vec3::new(size * (0.8 + 0.4 * rnd(k, 2)), size * (0.55 + 0.35 * rnd(k, 3)), size * (0.8 + 0.4 * rnd(k, 4)));
        boulders.push((k as usize, Transform::from_xyz(x, h - sink * s.y, z).with_rotation(rot).with_scale(s)));
    };
    // Lake shore boulders, some standing in the water.
    let lc = terrain::LAKE_CENTER;
    for k in 0..70u32 {
        let a = rnd(k, 20) * TAU;
        // Walk out from the centre to the shoreline, then scatter around it.
        let dir = Vec2::new(a.cos(), a.sin());
        let mut d = 0.0;
        while terrain::lake_factor(lc.x + dir.x * d, lc.y + dir.y * d) < 1.0 && d < 400.0 {
            d += 2.0;
        }
        let d = d * (0.88 + 0.22 * rnd(k, 21));
        let size = 1.2 + 4.5 * rnd(k, 22).powi(2);
        place(lc.x + dir.x * d, lc.y + dir.y * d, size, 0.15, k);
    }
    // Outcrops on steep hillsides, and scattered erratics in the meadows.
    let spacing = 34.0;
    let n = (1700.0 / spacing) as i32;
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
            let rocky = ((slope - 0.12) * 5.0).clamp(0.0, 1.0) * 0.6 + gradient_noise(x / 200.0, z / 200.0).max(0.0) * 0.25;
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
        spruce_gen,
        aspen_gen,
        spruce_solid,
        aspen_solid,
        aspen_trunk,
        spruce_proxy,
        boulder_meshes,
        boulders,
    }
}

/// Main thread: upload assets, create materials, spawn boulders. Trees are spawned in
/// batches afterwards with `spawn_tree`.
pub fn upload_vegetation(
    b: VegetationBuild,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> VegetationHandles {
    let mat = |m: &mut Assets<StandardMaterial>, c: Color, rough: f32| {
        m.add(StandardMaterial { base_color: c, perceptual_roughness: rough, reflectance: 0.3, ..default() })
    };
    // Solid crowns: used far away (raster) and as the ray-tracing proxy (shadows/reflections).
    let spruce_solid_mats: Vec<_> = [(0.026, 0.058, 0.032), (0.031, 0.066, 0.032), (0.024, 0.054, 0.036), (0.036, 0.074, 0.032)]
        .iter()
        .map(|&(r, g, b)| mat(materials, Color::linear_rgb(r, g, b), 0.85))
        .collect();
    let aspen_solid_mats: Vec<_> = [(0.060, 0.120, 0.028), (0.078, 0.135, 0.030), (0.050, 0.100, 0.026)]
        .iter()
        .map(|&(r, g, b)| mat(materials, Color::linear_rgb(r, g, b), 0.75))
        .collect();
    // Near: alpha-cut foliage cards with painted needles / leaves (raster only).
    let card_mat = |m: &mut Assets<StandardMaterial>, tex: Handle<Image>, tint: Vec3| {
        m.add(StandardMaterial {
            base_color: Color::linear_rgb(tint.x, tint.y, tint.z),
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
    let tints = [Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.1, 1.08, 0.95), Vec3::new(0.88, 0.92, 1.05), Vec3::new(1.2, 1.15, 0.9)];
    // Spruce needles a little darker and bluer than the painted texture (Engelmann / blue spruce).
    let spray_mats: Vec<_> = (0..4).map(|k| card_mat(materials, spray_tex[k % 3].clone(), tints[k] * Vec3::new(0.72, 0.76, 0.86))).collect();
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
    let bark = bark_mat(materials, images.add(b.bark_tex));
    let aspen_bark = bark_mat(materials, images.add(b.aspen_bark_tex));

    // Boulders.
    let granite = images.add(b.granite_tex);
    let rock_mats: Vec<_> = [1.0f32, 0.85, 1.12]
        .iter()
        .map(|&k| {
            materials.add(StandardMaterial {
                base_color: Color::linear_rgb(k, k, k * 0.98),
                base_color_texture: Some(granite.clone()),
                perceptual_roughness: 0.75,
                reflectance: 0.35,
                ..default()
            })
        })
        .collect();
    let boulder_meshes: Vec<_> = b.boulder_meshes.into_iter().map(|m| meshes.add(m)).collect();
    for (k, tf) in &b.boulders {
        commands.spawn(rt(boulder_meshes[k % boulder_meshes.len()].clone(), rock_mats[k % rock_mats.len()].clone(), *tf));
    }
    info!("placed {} boulders", b.boulders.len());

    VegetationHandles {
        spruce_gen: b.spruce_gen.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        aspen_gen: b.aspen_gen.into_iter().map(|(w, f)| (meshes.add(w), meshes.add(f))).collect(),
        spruce_solid: b.spruce_solid.into_iter().map(|m| meshes.add(m)).collect(),
        aspen_solid: b.aspen_solid.into_iter().map(|m| meshes.add(m)).collect(),
        aspen_trunk: meshes.add(b.aspen_trunk),
        spruce_proxy: meshes.add(b.spruce_proxy),
        spruce_solid_mats,
        aspen_solid_mats,
        spray_mats,
        leaf_mats,
        bark,
        aspen_bark,
    }
}

/// Spawns one tree (wood + foliage cards near, solid crown far, ray-tracing proxy).
pub fn spawn_tree(commands: &mut Commands, v: &VegetationHandles, i: usize, t: &terrain::Tree) {
    use bevy::camera::visibility::VisibilityRange;
    use bevy::solari::prelude::RaytracingMesh3d;
    // LOD switch distance (metres), with a dithered cross-fade band.
    let near = VisibilityRange { start_margin: 0.0..0.0, end_margin: 260.0..340.0, use_aabb: false };
    let far = VisibilityRange { start_margin: 260.0..340.0, end_margin: 1.0e7..1.0e7, use_aabb: false };

    let keep = std::env::var("T1S_TREE_FRAC").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    if rnd(i as u32, 99) > keep {
        return;
    }
    let h = t.height;
    let yaw = Quat::from_rotation_y(rnd(i as u32, 7) * TAU);
    let tilt = Quat::from_rotation_x((rnd(i as u32, 8) - 0.5) * 0.06) * Quat::from_rotation_z((rnd(i as u32, 9) - 0.5) * 0.06);
    let base = t.pos - Vec3::Y * 0.3;
    let shade = t.shade as usize;
    if t.conifer {
        let w = h * (0.85 + 0.3 * rnd(i as u32, 11));
        let (wood, cards) = &v.spruce_gen[i % v.spruce_gen.len()];
        let k = h / SPRUCE_REF;
        let gen_tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::new(k * w / h, k, k * w / h));
        commands.spawn((RaytracingMesh3d(wood.clone()), MeshMaterial3d(v.bark.clone()), gen_tf));
        commands.spawn((Mesh3d(wood.clone()), MeshMaterial3d(v.bark.clone()), gen_tf, near.clone()));
        commands.spawn((Mesh3d(cards.clone()), MeshMaterial3d(v.spray_mats[shade % 4].clone()), gen_tf, near));
        let tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::new(w, h, w));
        let solid = v.spruce_solid[i % v.spruce_solid.len()].clone();
        let solid_mat = v.spruce_solid_mats[shade % 4].clone();
        commands.spawn((Mesh3d(solid), MeshMaterial3d(solid_mat.clone()), tf, far));
        // Ray-tracing proxy for the needle mass: a smooth cone a bit inside the visible crown.
        let proxy = tf.with_scale(Vec3::new(w * 0.8, h * 0.96, w * 0.8));
        commands.spawn((RaytracingMesh3d(v.spruce_proxy.clone()), MeshMaterial3d(solid_mat), proxy));
    } else {
        let h = h * 0.8;
        let (wood, cards) = &v.aspen_gen[i % v.aspen_gen.len()];
        let gen_tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::splat(h / ASPEN_REF));
        commands.spawn((RaytracingMesh3d(wood.clone()), MeshMaterial3d(v.aspen_bark.clone()), gen_tf));
        commands.spawn((Mesh3d(wood.clone()), MeshMaterial3d(v.aspen_bark.clone()), gen_tf, near.clone()));
        commands.spawn((Mesh3d(cards.clone()), MeshMaterial3d(v.leaf_mats[shade % 3].clone()), gen_tf, near));
        let tf = Transform::from_translation(base).with_rotation(tilt * yaw).with_scale(Vec3::splat(h));
        commands.spawn((Mesh3d(v.aspen_trunk.clone()), MeshMaterial3d(v.aspen_bark.clone()), tf, far.clone()));
        let canopy = Transform::from_translation(base + Vec3::Y * h * 0.68)
            .with_rotation(yaw)
            .with_scale(Vec3::new(h * 0.36, h * 0.5, h * 0.36));
        let solid = v.aspen_solid[i % v.aspen_solid.len()].clone();
        let solid_mat = v.aspen_solid_mats[shade % 3].clone();
        commands.spawn((Mesh3d(solid.clone()), MeshMaterial3d(solid_mat.clone()), canopy, far));
        let proxy = canopy.with_scale(canopy.scale * 0.75);
        commands.spawn((RaytracingMesh3d(solid), MeshMaterial3d(solid_mat), proxy));
    }
}
