//! 3D grass and alpine flowers near the camera.
//!
//! Clumps are alpha-cut cards (raster only, receiving ray-traced lighting) placed per 6 m
//! tile from `terrain_render::ground_cover`, streamed in and out around the camera. Card
//! normals follow the ground so the grass shades like the turf it stands on.

use crate::camera::MainCamera;
use crate::foliage::Canvas;
use crate::terrain;
use crate::terrain_render::ground_cover;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::f32::consts::TAU;

const TILE: f32 = 6.0;
const RADIUS: f32 = 38.0;
const MAX_AGL: f32 = 70.0;
const SPAWNS_PER_FRAME: usize = 8;

#[derive(Resource)]
pub struct GrassAssets {
    material: Handle<StandardMaterial>,
    tiles: HashMap<(i32, i32), Entity>,
}

#[derive(Component)]
pub struct GrassTile;

fn rnd(a: i32, b: i32, k: u32) -> f32 {
    let mut h = (a as u32).wrapping_mul(0x8da6_b343) ^ (b as u32).wrapping_mul(0xd816_3841) ^ k.wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffffff) as f32 / 0xffffff as f32
}

/// 1024x256 atlas: [grass | white daisies | yellow avens | purple lupine].
fn atlas() -> Image {
    let (w, h) = (1024usize, 256usize);
    let mut c = Canvas::new(w, h);
    let mut k = 0u32;
    let mut r = || {
        k += 1;
        rnd(17, 29, k)
    };
    for panel in 0..4 {
        let x0 = (panel * 256) as f32;
        let blades = if panel == 0 { 46 } else { 22 };
        for _ in 0..blades {
            let bx = x0 + 18.0 + r() * 220.0;
            let tall = 0.55 + 0.42 * r();
            let lean = (r() - 0.5) * 70.0;
            let base = Vec2::new(bx, h as f32 - 1.0);
            let mid = base + Vec2::new(lean * 0.35, -tall * h as f32 * 0.55);
            let tip = base + Vec2::new(lean, -tall * h as f32 * 0.97);
            let g = r();
            let col = [0.020 + 0.022 * g, 0.068 + 0.045 * g, 0.008 + 0.004 * g];
            let tipc = [0.055 + 0.035 * g, 0.125 + 0.035 * g, 0.014];
            c.line(base, mid, 5.0, 3.6, col, [col[0] * 1.2, col[1] * 1.15, col[2]]);
            c.line(mid, tip, 3.6, 0.8, [col[0] * 1.2, col[1] * 1.15, col[2]], tipc);
        }
        if panel > 0 {
            for _ in 0..13 {
                let bx = x0 + 30.0 + r() * 196.0;
                let top = Vec2::new(bx + (r() - 0.5) * 30.0, 40.0 + r() * 90.0);
                let stem = [0.05, 0.10, 0.03];
                c.line(Vec2::new(bx, h as f32 - 1.0), top, 2.6, 1.6, stem, stem);
                match panel {
                    1 => {
                        // Oxeye daisy: white petals around a yellow centre.
                        for p in 0..13 {
                            let a = p as f32 / 13.0 * TAU;
                            let d = Vec2::new(a.cos(), a.sin());
                            c.leaf(top + d * 6.5, 6.0, 2.2, a, [0.82, 0.82, 0.78]);
                        }
                        c.leaf(top, 3.6, 3.6, 0.0, [0.78, 0.52, 0.03]);
                    }
                    2 => {
                        // Hawkweed / dandelion: many thin yellow-orange rays.
                        for p in 0..18 {
                            let a = p as f32 / 18.0 * TAU;
                            let d = Vec2::new(a.cos(), a.sin());
                            c.leaf(top + d * 4.8, 5.2, 1.7, a, [0.86, 0.46, 0.02]);
                        }
                        c.leaf(top, 2.8, 2.8, 0.0, [0.72, 0.36, 0.02]);
                    }
                    _ => {
                        // Clover / knapweed: a round purple head of florets.
                        for f in 0..16 {
                            let a = f as f32 * 2.4;
                            let r = (f as f32).sqrt() * 1.9;
                            c.leaf(top + Vec2::new(a.cos(), a.sin()) * r, 2.8, 2.2, a, [0.30, 0.09, 0.36]);
                        }
                    }
                }
            }
        }
    }
    c.into_image(0.5)
}

fn tile_mesh(ix: i32, iz: i32) -> Option<Mesh> {
    let (x0, z0) = (ix as f32 * TILE, iz as f32 * TILE);
    let centre = Vec3::new(x0 + TILE * 0.5, terrain::ground_height(x0 + TILE * 0.5, z0 + TILE * 0.5), z0 + TILE * 0.5);
    let (mut pos, mut nrm, mut uv, mut col, mut idx) = (vec![], vec![], vec![], vec![], vec![]);
    // Sample cover on a 2 m grid within the tile.
    let mut k = 0u32;
    for gz in 0..3 {
        for gx in 0..3 {
            let (cx, cz) = (x0 + gx as f32 * 2.0 + 1.0, z0 + gz as f32 * 2.0 + 1.0);
            let cover = ground_cover(cx, cz);
            let count = (cover.density * 4.0 * (0.85 + 0.3 * rnd(ix, iz, 900 + gx + gz * 3))) as u32;
            for _ in 0..count {
                k += 1;
                let x = cx + (rnd(ix, iz, k * 5) - 0.5) * 2.0;
                let z = cz + (rnd(ix, iz, k * 5 + 1) - 0.5) * 2.0;
                let y = terrain::ground_height(x, z) - 0.02;
                let n = terrain::ground_normal(x, z);
                let flower = rnd(ix, iz, k * 5 + 2) < cover.flowers;
                let hh = cover.height * (0.65 + 0.7 * rnd(ix, iz, k * 5 + 3)) * if flower { 1.2 } else { 1.0 };
                let ww = hh * if cover.height < 0.12 { 2.2 } else { 1.1 };
                // Mixed flowers (as in a real meadow): mostly hawkweed and daisies, some clover.
                let pick = rnd(ix, iz, k * 11) + (cover.flower_kind as f32 - 1.0) * 0.12;
                let kind = if pick < 0.45 { 2 } else if pick < 0.82 { 1 } else { 3 };
                let panel = if flower { kind } else { 0 } as f32;
                let sunny = {
                    let n = terrain::fbm(x / 11.0, z / 11.0, 3);
                    ((n - 0.55) / 0.25).clamp(0.0, 1.0) * 0.6
                };
                let tint = 0.72 + 0.28 * rnd(ix, iz, k * 5 + 4);
                let dry = (cover.dry * 0.5 + 0.5 * rnd(ix, iz, k * 7)) * 0.25;
                let c = if flower {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [tint * (1.0 + dry + sunny * 1.2), tint * (1.0 - dry * 0.15 + sunny * 0.15), tint * (1.0 - dry * 0.4 - sunny * 0.3), 1.0]
                };
                let yaw = rnd(ix, iz, k * 7 + 1) * TAU;
                for q in 0..2 {
                    let a = yaw + q as f32 * std::f32::consts::FRAC_PI_2;
                    let d = Vec3::new(a.cos(), 0.0, a.sin()) * ww * 0.5;
                    let base = pos.len() as u32;
                    let p = Vec3::new(x, y, z) - centre;
                    let up = Vec3::Y * hh;
                    for (v, u, off) in [(1.0, 0.0, -d), (1.0, 1.0, d), (0.0, 1.0, d + up), (0.0, 0.0, -d + up)] {
                        pos.push((p + off).to_array());
                        nrm.push(n.to_array());
                        uv.push([(panel + u) / 4.0, v]);
                        col.push(c);
                    }
                    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        }
    }
    if idx.is_empty() {
        return None;
    }
    Some(
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
            .with_inserted_indices(Indices::U32(idx)),
    )
}

pub fn setup_grass(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(atlas())),
        alpha_mode: AlphaMode::Mask(0.5),
        double_sided: false,
        cull_mode: None,
        // Grass is matte; up-facing card normals would otherwise pick up a sheen.
        perceptual_roughness: 1.0,
        reflectance: 0.05,
        ..default()
    });
    commands.insert_resource(GrassAssets { material, tiles: HashMap::default() });
}

pub fn stream_grass(
    mut commands: Commands,
    mut grass: ResMut<GrassAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&GlobalTransform, With<MainCamera>>,
) {
    let Ok(cam) = camera.single() else { return };
    let p = cam.translation();
    let agl = p.y - terrain::ground_height(p.x, p.z);
    let grass = &mut *grass;
    // Drop tiles that are out of range (or everything when flying high).
    let keep_r = RADIUS + TILE * 1.5;
    grass.tiles.retain(|&(ix, iz), e| {
        let c = Vec2::new((ix as f32 + 0.5) * TILE, (iz as f32 + 0.5) * TILE);
        let keep = agl < MAX_AGL + 10.0 && c.distance(Vec2::new(p.x, p.z)) < keep_r;
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    if agl > MAX_AGL {
        return;
    }
    // Spawn the nearest missing tiles first, a few per frame.
    let (cx, cz) = ((p.x / TILE).floor() as i32, (p.z / TILE).floor() as i32);
    let n = (RADIUS / TILE).ceil() as i32;
    let mut wanted = Vec::new();
    for iz in cz - n..=cz + n {
        for ix in cx - n..=cx + n {
            let c = Vec2::new((ix as f32 + 0.5) * TILE, (iz as f32 + 0.5) * TILE);
            let d = c.distance(Vec2::new(p.x, p.z));
            if d < RADIUS && !grass.tiles.contains_key(&(ix, iz)) {
                wanted.push((d, ix, iz));
            }
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, ix, iz) in wanted.iter().take(SPAWNS_PER_FRAME) {
        let centre_xz = Vec2::new((ix as f32 + 0.5) * TILE, (iz as f32 + 0.5) * TILE);
        let centre = Vec3::new(centre_xz.x, terrain::ground_height(centre_xz.x, centre_xz.y), centre_xz.y);
        let e = match tile_mesh(ix, iz) {
            Some(mesh) => commands
                .spawn((
                    GrassTile,
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(grass.material.clone()),
                    Transform::from_translation(centre),
                    VisibilityRange { start_margin: 0.0..0.0, end_margin: (RADIUS - 12.0)..RADIUS, use_aabb: false },
                ))
                .id(),
            // Bare tile: remember it so it isn't regenerated every frame.
            None => commands.spawn((GrassTile, Transform::from_translation(centre))).id(),
        };
        grass.tiles.insert((ix, iz), e);
    }
}
