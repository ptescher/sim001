//! 3D grass and alpine flowers near the camera.
//!
//! Clumps are alpha-cut cards (raster only, receiving ray-traced lighting) placed per 6 m
//! tile from `terrain_render::ground_cover`, streamed in and out around the camera. Card
//! normals follow the ground so the grass shades like the turf it stands on.

use crate::camera::MainCamera;
use crate::foliage::Canvas;
use crate::meadow::{self, Flower, Parts};
use crate::terrain;
use crate::terrain_render::ground_cover;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::f32::consts::TAU;

const TILE: f32 = 6.0;
pub const RADIUS: f32 = 38.0;
const MAX_AGL: f32 = 70.0;
const SPAWNS_PER_FRAME: usize = 8;
/// Panels in the atlas: grass, then one for each flower.
const PANELS: u32 = 6;
/// Solid meadows built per frame as the camera comes near.
const NEAR_BUILDS_PER_FRAME: usize = 2;

#[derive(Resource)]
pub struct GrassAssets {
    material: Handle<StandardMaterial>,
    /// For the solid blades and flowers: their colours are in their vertices.
    solid_material: Handle<StandardMaterial>,
    tiles: HashMap<(i32, i32), Tile>,
}

/// A tile of meadow: its painted cards, and the solid grass and flowers while the camera is near.
struct Tile {
    /// The painted grass, which stays under the solid grass at every distance.
    cards: Entity,
    /// The painted flowers, which the solid ones take the place of near the camera.
    flower_cards: Option<Entity>,
    solid: Option<Entity>,
    /// Nothing grows here: there is nothing to build.
    bare: bool,
}

#[derive(Component)]
pub struct GrassTile;

fn rnd(a: i32, b: i32, k: u32) -> f32 {
    let mut h = (a as u32).wrapping_mul(0x8da6_b343) ^ (b as u32).wrapping_mul(0xd816_3841) ^ k.wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffffff) as f32 / 0xffffff as f32
}

/// 1536x256 atlas: [grass | daisies | hawkweed | clover | lupine | paintbrush].
fn atlas() -> Image {
    let (w, h) = (256 * PANELS as usize, 256usize);
    let mut c = Canvas::new(w, h);
    let mut k = 0u32;
    let mut r = || {
        k += 1;
        rnd(17, 29, k)
    };
    for panel in 0..PANELS {
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
                    3 => {
                        // Clover / knapweed: a round purple head of florets.
                        for f in 0..16 {
                            let a = f as f32 * 2.4;
                            let r = (f as f32).sqrt() * 1.9;
                            c.leaf(top + Vec2::new(a.cos(), a.sin()) * r, 2.8, 2.2, a, [0.30, 0.09, 0.36]);
                        }
                    }
                    4 => {
                        // Lupine: a spike of violet-to-blue florets down from the tip.
                        for f in 0..14 {
                            let t = f as f32 / 13.0;
                            let a = f as f32 * 2.1;
                            let p = top + Vec2::new(a.cos() * 3.0, f as f32 * 4.6);
                            let col = [0.16 + 0.05 * t, 0.07 + 0.08 * t, 0.42 - 0.04 * t];
                            c.leaf(p, 5.2 - 2.0 * t, 2.8, a, col);
                        }
                    }
                    _ => {
                        // Paintbrush: red bracts, orange at the tips, in a brush.
                        for f in 0..9 {
                            let a = -1.57 + (f as f32 - 4.0) * 0.32;
                            let d = Vec2::new(a.cos(), a.sin());
                            c.leaf(top + d * 6.0 + Vec2::new(0.0, 5.0), 9.0, 2.6, a, [0.50, 0.04, 0.03]);
                            c.leaf(top + d * 11.0 + Vec2::new(0.0, 3.0), 4.0, 2.2, a, [0.78, 0.22, 0.04]);
                        }
                    }
                }
            }
        }
    }
    c.into_image(0.5)
}

/// One clump of grass or one flower clump in a tile.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub pos: Vec3,
    /// The ground's normal.
    pub up: Vec3,
    pub height: f32,
    pub width: f32,
    pub flower: Option<Flower>,
    /// What the grass colour is multiplied by here: lusher, drier, sunnier.
    pub tint: [f32; 3],
    pub yaw: f32,
    pub seed: u32,
}

/// The panel of the atlas a flower is painted on.
fn card_panel(f: Flower) -> u32 {
    match f {
        Flower::Daisy => 1,
        Flower::Hawkweed => 2,
        Flower::Clover => 3,
        Flower::Lupine => 4,
        Flower::Paintbrush => 5,
    }
}

/// The clumps and flower clumps of a tile, from the ground cover at each place. Both the painted
/// cards and the solid meadow are built from these.
pub fn placements(ix: i32, iz: i32) -> Vec<Placement> {
    let (x0, z0) = (ix as f32 * TILE, iz as f32 * TILE);
    let mut out = Vec::new();
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
                let up = terrain::ground_normal(x, z);
                let is_flower = rnd(ix, iz, k * 5 + 2) < cover.flowers;
                let height = cover.height * (0.65 + 0.7 * rnd(ix, iz, k * 5 + 3)) * if is_flower { 1.2 } else { 1.0 };
                let width = height * if cover.height < 0.12 { 2.2 } else { 1.1 };
                // Mixed flowers (as in a real meadow): mostly hawkweed and daisies, some clover,
                // and a few lupines and paintbrushes standing out of it.
                let pick = rnd(ix, iz, k * 11) + (cover.flower_kind as f32 - 1.0) * 0.12;
                let rare = rnd(ix, iz, k * 13 + 5);
                let kind = if rare < 0.09 {
                    Flower::Lupine
                } else if rare < 0.15 {
                    Flower::Paintbrush
                } else if pick < 0.45 {
                    Flower::Hawkweed
                } else if pick < 0.82 {
                    Flower::Daisy
                } else {
                    Flower::Clover
                };
                let sunny = {
                    let n = terrain::fbm(x / 11.0, z / 11.0, 3);
                    ((n - 0.55) / 0.25).clamp(0.0, 1.0) * 0.6
                };
                let tint = 0.72 + 0.28 * rnd(ix, iz, k * 5 + 4);
                let dry = (cover.dry * 0.5 + 0.5 * rnd(ix, iz, k * 7)) * 0.25;
                out.push(Placement {
                    pos: Vec3::new(x, y, z),
                    up,
                    height,
                    width,
                    flower: is_flower.then_some(kind),
                    tint: [tint * (1.0 + dry + sunny * 1.2), tint * (1.0 - dry * 0.15 + sunny * 0.15), tint * (1.0 - dry * 0.4 - sunny * 0.3)],
                    yaw: rnd(ix, iz, k * 7 + 1) * TAU,
                    seed: ix.wrapping_mul(73856093) as u32 ^ iz.wrapping_mul(19349663) as u32 ^ k.wrapping_mul(83492791),
                });
            }
        }
    }
    out
}

fn tile_centre(ix: i32, iz: i32) -> Vec3 {
    let (x, z) = ((ix as f32 + 0.5) * TILE, (iz as f32 + 0.5) * TILE);
    Vec3::new(x, terrain::ground_height(x, z), z)
}

/// The painted cards of a tile, grass or flowers: two crossed quads per clump.
fn tile_mesh(ix: i32, iz: i32, flowers: bool) -> Option<Mesh> {
    let centre = tile_centre(ix, iz);
    let (mut pos, mut nrm, mut uv, mut col, mut idx) = (vec![], vec![], vec![], vec![], vec![]);
    for pl in placements(ix, iz).into_iter().filter(|pl| pl.flower.is_some() == flowers) {
        let panel = pl.flower.map_or(0, card_panel) as f32;
        let c = if pl.flower.is_some() { [1.0, 1.0, 1.0, 1.0] } else { [pl.tint[0], pl.tint[1], pl.tint[2], 1.0] };
        for q in 0..2 {
            let a = pl.yaw + q as f32 * std::f32::consts::FRAC_PI_2;
            let d = Vec3::new(a.cos(), 0.0, a.sin()) * pl.width * 0.5;
            let base = pos.len() as u32;
            let p = pl.pos - centre;
            let up = Vec3::Y * pl.height;
            for (v, u, off) in [(1.0, 0.0, -d), (1.0, 1.0, d), (0.0, 1.0, d + up), (0.0, 0.0, -d + up)] {
                pos.push((p + off).to_array());
                nrm.push(pl.up.to_array());
                uv.push([(panel + u) / PANELS as f32, v]);
                col.push(c);
            }
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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

/// The solid meadow of a tile: a tuft of blades for each clump, and a handful of real flowers for
/// each flower clump.
fn near_mesh(ix: i32, iz: i32) -> Option<Mesh> {
    let centre = tile_centre(ix, iz);
    let mut parts = Parts::default();
    // Grass colour in linear light (what the sun makes of it is the lighting's business).
    const GRASS: [f32; 3] = [0.07, 0.14, 0.02];
    for pl in placements(ix, iz) {
        let at = pl.pos - centre;
        let tint = [GRASS[0] * pl.tint[0], GRASS[1] * pl.tint[1], GRASS[2] * pl.tint[2]];
        let radius = (pl.width * 0.35).clamp(0.03, 0.14);
        meadow::clump(&mut parts, at, pl.up, pl.seed, pl.height * 0.9, radius, tint);
        if let Some(kind) = pl.flower {
            let n = 2 + (pl.seed % 3);
            for f in 0..n {
                let a = rnd(f as i32, (pl.seed & 0xffff) as i32, 1) * TAU;
                let r = rnd(f as i32, (pl.seed & 0xffff) as i32, 2).sqrt() * pl.width * 0.4;
                let off = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                let ground = terrain::ground_height(pl.pos.x + off.x, pl.pos.z + off.z) - 0.02 - centre.y;
                let height = pl.height * (0.85 + 0.3 * rnd(f as i32, (pl.seed & 0xffff) as i32, 3))
                    * match kind {
                        Flower::Lupine => 1.5,
                        // Clover holds its heads just above its leaves.
                        Flower::Clover => 0.45,
                        _ => 1.0,
                    };
                meadow::flower(&mut parts, kind, Vec3::new(at.x + off.x, ground, at.z + off.z), pl.up, height, pl.seed.wrapping_add(f * 977));
            }
        }
    }
    (parts.tris() > 0).then(|| parts.into_mesh())
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
    let solid_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        double_sided: true,
        cull_mode: None,
        perceptual_roughness: 0.85,
        reflectance: 0.08,
        ..default()
    });
    commands.insert_resource(GrassAssets { material, solid_material, tiles: HashMap::default() });
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
    let eye = Vec2::new(p.x, p.z);
    let (solid_range, card_range) = meadow::near_ranges();
    let everywhere = VisibilityRange::abrupt(0.0, RADIUS);
    // Drop tiles that are out of range (or everything when flying high).
    let keep_r = RADIUS + TILE * 1.5;
    grass.tiles.retain(|&(ix, iz), tile| {
        let c = Vec2::new((ix as f32 + 0.5) * TILE, (iz as f32 + 0.5) * TILE);
        let keep = agl < MAX_AGL + 10.0 && c.distance(eye) < keep_r;
        if !keep {
            commands.entity(tile.cards).despawn();
            for e in [tile.flower_cards, tile.solid].into_iter().flatten() {
                commands.entity(e).despawn();
            }
        } else if tile.solid.is_some() && c.distance(eye) > meadow::NEAR + TILE * 2.5 {
            // Too far for the solid meadow to show: let it go, and let the cards cover the tile.
            commands.entity(tile.solid.take().unwrap()).despawn();
            if let Some(e) = tile.flower_cards {
                commands.entity(e).insert(everywhere.clone());
            }
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
            let d = c.distance(eye);
            if d < RADIUS && !grass.tiles.contains_key(&(ix, iz)) {
                wanted.push((d, ix, iz));
            }
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, ix, iz) in wanted.iter().take(SPAWNS_PER_FRAME) {
        let centre = tile_centre(ix, iz);
        let spawn = |commands: &mut Commands, meshes: &mut Assets<Mesh>, mesh: Mesh| {
            commands.spawn((GrassTile, Mesh3d(meshes.add(mesh)), MeshMaterial3d(grass.material.clone()), Transform::from_translation(centre), everywhere.clone())).id()
        };
        let grass_mesh = tile_mesh(ix, iz, false);
        let flower_mesh = tile_mesh(ix, iz, true);
        let bare = grass_mesh.is_none() && flower_mesh.is_none();
        // A tile with no grass still gets an entity, so that it is remembered.
        let cards = match grass_mesh {
            Some(mesh) => spawn(&mut commands, &mut meshes, mesh),
            None => commands.spawn((GrassTile, Transform::from_translation(centre))).id(),
        };
        let flower_cards = flower_mesh.map(|mesh| spawn(&mut commands, &mut meshes, mesh));
        grass.tiles.insert((ix, iz), Tile { cards, flower_cards, solid: None, bare });
    }
    // The tiles the camera is near get their solid grass and flowers, nearest first.
    let mut near: Vec<_> = grass
        .tiles
        .iter()
        .filter(|(_, t)| t.solid.is_none() && !t.bare)
        .map(|(&(ix, iz), _)| (Vec2::new((ix as f32 + 0.5) * TILE, (iz as f32 + 0.5) * TILE).distance(eye), ix, iz))
        .filter(|&(d, ..)| d < meadow::NEAR + TILE * 1.5)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    // SIM001_MEADOW=0 leaves the solid meadow out (for timing).
    let solid_enabled = std::env::var("SIM001_MEADOW").map_or(true, |v| v != "0");
    for &(_, ix, iz) in near.iter().filter(|_| solid_enabled).take(NEAR_BUILDS_PER_FRAME) {
        let Some(mesh) = near_mesh(ix, iz) else { continue };
        let solid = commands
            .spawn((GrassTile, Mesh3d(meshes.add(mesh)), MeshMaterial3d(grass.solid_material.clone()), Transform::from_translation(tile_centre(ix, iz)), solid_range.clone()))
            .id();
        let tile = grass.tiles.get_mut(&(ix, iz)).unwrap();
        tile.solid = Some(solid);
        // The flower cards of this tile start where the solid meadow stops.
        if let Some(e) = tile.flower_cards {
            commands.entity(e).insert(card_range.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meadow::Flower;

    /// Where the clumps and flowers of a tile are, and what they are: the same for the painted
    /// cards and the solid meadow, so that one hands over to the other without anything jumping.
    #[test]
    fn a_tile_is_the_same_every_time_and_stays_inside_itself() {
        let (a, b) = (placements(10, -20), placements(10, -20));
        assert!(!a.is_empty());
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(p, q)| p.pos == q.pos && p.flower == q.flower));
        for p in &a {
            assert!(p.pos.x >= 10.0 * TILE - 1.1 && p.pos.x <= 11.0 * TILE + 1.1, "{:?} is outside its tile", p.pos);
            assert!(p.pos.z >= -20.0 * TILE - 1.1 && p.pos.z <= -19.0 * TILE + 1.1, "{:?} is outside its tile", p.pos);
            assert!(p.height > 0.04 && p.height < 0.9, "height {}", p.height);
            assert!((p.up.length() - 1.0).abs() < 1e-3 && p.up.y > 0.3);
        }
    }

    /// A meadow has all five flowers in it, the common ones commoner than the rare.
    #[test]
    fn all_five_flowers_grow_in_the_meadow() {
        let mut count = std::collections::HashMap::new();
        for ix in -40..40 {
            for iz in -40..0 {
                for p in placements(ix, iz) {
                    if let Some(f) = p.flower {
                        *count.entry(f).or_insert(0u32) += 1;
                    }
                }
            }
        }
        for f in [Flower::Daisy, Flower::Hawkweed, Flower::Clover, Flower::Lupine, Flower::Paintbrush] {
            assert!(count.get(&f).copied().unwrap_or(0) > 20, "{f:?}: {:?}", count.get(&f));
        }
        let (common, rare) = (count[&Flower::Hawkweed].min(count[&Flower::Daisy]), count[&Flower::Paintbrush].max(count[&Flower::Lupine]));
        assert!(common > rare, "the rare flowers are as common as the common ones: {count:?}");
    }

    /// Every flower has a panel in the atlas of painted cards.
    #[test]
    fn every_flower_has_a_card() {
        for f in [Flower::Daisy, Flower::Hawkweed, Flower::Clover, Flower::Lupine, Flower::Paintbrush] {
            let panel = card_panel(f);
            assert!((1..PANELS).contains(&panel), "{f:?} has panel {panel}");
        }
        let panels: std::collections::HashSet<_> =
            [Flower::Daisy, Flower::Hawkweed, Flower::Clover, Flower::Lupine, Flower::Paintbrush].iter().map(|f| card_panel(*f)).collect();
        assert_eq!(panels.len(), 5, "two flowers share a card");
        assert_eq!(atlas().width(), 256 * PANELS);
    }
}
