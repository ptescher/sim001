//! Lake water.
//!
//! The water is a mirror over a coloured bed: the ray tracer has no refraction, so the colour
//! of what is under the surface is painted on it, by depth (pebbly silt in the shallows, dark
//! where it is deep). Fresnel then does the rest: the reflection wins at a grazing angle, and
//! the bed shows where you look down into it, as at a real mountain lake.
//!
//! The clouds are a pass of their own: see `clouds`.

use crate::mesh_util::normal_map;
use crate::model::rt;
use crate::terrain;
use bevy::math::Affine2;
use bevy::prelude::*;
use std::f32::consts::TAU;

#[derive(Component)]
pub struct Water(Handle<StandardMaterial>);

fn hash(k: u32, j: u32) -> f32 {
    let mut h = k.wrapping_mul(0x9E37_79B1) ^ j.wrapping_mul(0x85EB_CA77);
    h = (h ^ (h >> 15)).wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 13;
    (h & 0xffffff) as f32 / 0xffffff as f32
}

pub fn spawn_scenery(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    // Calm-lake ripples: a sum of low-amplitude sine waves with integer wave vectors (tileable).
    let waves: Vec<(f32, f32, f32, f32)> = (0..14)
        .map(|k| {
            let a = hash(k, 1) * TAU;
            let f = 2.0 + (hash(k, 2) * 9.0).floor();
            ((a.cos() * f).round(), (a.sin() * f).round(), hash(k, 3) * TAU, 1.0 / f)
        })
        .collect();
    let ripples = images.add(normal_map(512, 9.0, move |u, v| {
        waves.iter().map(|&(kx, ky, ph, amp)| amp * (TAU * (kx * u + ky * v) + ph).sin()).sum::<f32>() * 0.002
    }));
    let alpine = terrain::real().is_none();
    let water_mat = materials.add(if alpine {
        // The lake: the bed's pebbles on their own UVs (metres / 4), dimmed with depth by the
        // vertex colours, and still ripples (moving them would drag the bed along).
        StandardMaterial {
            base_color_texture: Some(images.add(pebble_texture())),
            base_color_channel: bevy::mesh::UvChannel::Uv1,
            perceptual_roughness: 0.02,
            reflectance: 0.5,
            normal_map_texture: Some(ripples),
            ..default()
        }
    } else {
        StandardMaterial {
            base_color: Color::linear_rgb(0.004, 0.010, 0.010),
            perceptual_roughness: 0.02,
            reflectance: 0.5,
            normal_map_texture: Some(ripples),
            ..default()
        }
    });
    if terrain::real().is_some() {
        // The sea / bay: one big plane at sea level (the terrain under it was pushed down),
        // with ripples tiled every 25 m. Extends well past the terrain to the horizon.
        let half = terrain::half_size() * 4.0;
        let tiles = half * 2.0 / 25.0;
        let mut sea = crate::mesh_util::GeoBuilder::default();
        let ids: Vec<u32> = [(-half, -half), (half, -half), (half, half), (-half, half)]
            .iter()
            .map(|&(x, z)| {
                sea.vert(
                    Vec3::new(x, 0.0, z),
                    Vec2::new((x / half * 0.5 + 0.5) * tiles, (z / half * 0.5 + 0.5) * tiles),
                )
            })
            .collect();
        sea.quad(ids[0], ids[1], ids[2], ids[3], Vec3::Y);
        commands.spawn((
            Water(water_mat.clone()),
            rt(meshes.add(sea.build()), water_mat, Transform::from_xyz(0.0, terrain::water_level(), 0.0)),
        ));
    } else {
        let water = meshes.add(lake_mesh());
        commands.spawn((
            Water(water_mat.clone()),
            rt(water, water_mat, Transform::from_xyz(0.0, terrain::WATER_LEVEL, 0.0)),
        ));
    }
}

/// How much of the lake bed's colour comes back up through `depth` metres of clear water: red
/// goes first, so deeper water is greener and then dark.
pub fn through_water(depth: f32) -> [f32; 3] {
    let t = (-depth.max(0.0) * 0.9).exp();
    [t.powf(1.8), t.powf(1.1), t.powf(1.3)].map(|c| c * 0.95)
}

/// The lake bed: rounded stones of several sizes, mostly grey granite with some darker slate
/// and a little rust, lying in olive silt.
fn pebble_texture() -> Image {
    // One layer of stones: `cells` across the tile, each stone a jittered ellipse in its cell.
    fn stones(u: f32, v: f32, cells: i32, seed: i32) -> Option<([f32; 3], f32)> {
        let (px, py) = (u * cells as f32, v * cells as f32);
        let (cx, cy) = (px.floor() as i32, py.floor() as i32);
        let mut best: Option<([f32; 3], f32, f32)> = None;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (gx, gy) = (cx + dx, cy + dy);
                let (wx, wy) = (gx.rem_euclid(cells), gy.rem_euclid(cells));
                let h = |k: i32| terrain::hash_pub(wx * 31 + k * 7 + seed, wy * 17 - k * 3 + seed * 5);
                if h(9) < 0.25 {
                    continue; // No stone here: silt.
                }
                let centre = Vec2::new(gx as f32 + 0.25 + 0.5 * h(1), gy as f32 + 0.25 + 0.5 * h(2));
                let r = 0.28 + 0.2 * h(3);
                let ang = h(4) * TAU;
                let q = Vec2::new(px, py) - centre;
                let q = Vec2::new(
                    q.x * ang.cos() + q.y * ang.sin(),
                    (-q.x * ang.sin() + q.y * ang.cos()) * (1.2 + 0.5 * h(5)),
                );
                let d = q.length() / r;
                if d < 1.0 && best.is_none_or(|b| d < b.2) {
                    let tone = 0.75 + 0.5 * h(6);
                    let base = match (h(7) * 10.0) as u32 {
                        0..=5 => [0.17, 0.165, 0.15],
                        6..=8 => [0.09, 0.095, 0.09],
                        _ => [0.17, 0.12, 0.08],
                    };
                    // A rounded top: lit from above, darker towards the edge.
                    let shade = (1.0 - d * d).sqrt() * 0.6 + 0.4 + 0.15 * (-q.y / r);
                    best = Some((base.map(|c| c * tone * shade), d, d));
                }
            }
        }
        best.map(|(c, d, _)| (c, d))
    }
    crate::mesh_util::texture(512, |u, v| {
        // Wider mottling is in the lake's vertex colours: noise here would not tile.
        let silt = [0.085, 0.080, 0.045];
        let grain = 0.92 + 0.16 * terrain::hash_pub((u * 512.0) as i32, (v * 512.0) as i32);
        // Small stones lie on the silt, bigger stones on top of them.
        let mut c = silt;
        for (cells, seed) in [(22, 3), (11, 41)] {
            if let Some((stone, d)) = stones(u, v, cells, seed) {
                let edge = terrain::smoothstep(1.0, 0.85, d);
                c = [0, 1, 2].map(|i| c[i] + (stone[i] - c[i]) * edge);
            }
        }
        c.map(|x| x * grain)
    })
}

/// The Alpine Valley's lake: a flat grid at the water level (the shore hides its edges), with
/// the colour under the surface in its vertices.
fn lake_mesh() -> Mesh {
    let half = terrain::lake_radius() * 2.0;
    let c = terrain::lake_center();
    let n = 160u32;
    let mut b = crate::mesh_util::GeoBuilder::default();
    let mut colours = Vec::new();
    let mut bed_uv = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            let (x, z) = (c.x - half + 2.0 * half * u, c.y - half + 2.0 * half * v);
            // Ripples tile every 19 m or so.
            b.vert(Vec3::new(x, 0.0, z), Vec2::new(u, v) * 22.0);
            bed_uv.push([x / 4.0, z / 4.0]);
            let mottle = 0.75 + 0.5 * terrain::fbm(x / 7.0, z / 7.0, 3);
            let [r, g, bl] = through_water(terrain::WATER_LEVEL - terrain::ground_height(x, z)).map(|c| c * mottle);
            colours.push([r, g, bl, 1.0]);
        }
    }
    let row = n + 1;
    for j in 0..n {
        for i in 0..n {
            let k = j * row + i;
            b.quad(k, k + row, k + row + 1, k + 1, Vec3::Y);
        }
    }
    let mut mesh = b.build();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, bed_uv);
    mesh
}

/// Ripples drift across the sea.
pub fn animate_scenery(time: Res<Time>, water: Query<&Water>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let t = time.elapsed_secs();
    for w in water.iter().filter(|_| terrain::real().is_some()) {
        if let Some(mut m) = materials.get_mut(&w.0) {
            m.uv_transform =
                Affine2::from_scale_angle_translation(Vec2::splat(22.0), 0.3, Vec2::new(t * 0.006, t * 0.004));
        }
    }
}
