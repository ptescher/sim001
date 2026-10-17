//! Lake water and cumulus clouds.
//!
//! The water is a mirror over a coloured bed: the ray tracer has no refraction, so the colour
//! of what is under the surface is painted on it, by depth (pebbly silt in the shallows, dark
//! where it is deep). Fresnel then does the rest: the reflection wins at a grazing angle, and
//! the bed shows where you look down into it, as at a real mountain lake.
//!
//! Clouds are camera-facing puffs, sorted back to front every frame.

use crate::mesh_util::normal_map;
use crate::model::rt;
use crate::terrain;
use crate::Scene;
use bevy::math::Affine2;
use bevy::prelude::*;
use std::f32::consts::TAU;

#[derive(Component)]
pub struct Water(Handle<StandardMaterial>);

#[derive(Component)]
pub struct Cloud {
    home: Vec3,
    mesh: Handle<Mesh>,
    /// (offset from cloud centre, half size, atlas panel, tint)
    puffs: Vec<(Vec3, f32, u32, [f32; 4])>,
}

/// Clouds wrap around a square this far (m) each way from the field: far enough to fill the
/// sky down to the mountains.
const CLOUD_FIELD: f32 = 16_000.0;

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
            .map(|&(x, z)| sea.vert(Vec3::new(x, 0.0, z), Vec2::new((x / half * 0.5 + 0.5) * tiles, (z / half * 0.5 + 0.5) * tiles)))
            .collect();
        sea.quad(ids[0], ids[1], ids[2], ids[3], Vec3::Y);
        commands.spawn((Water(water_mat.clone()), rt(meshes.add(sea.build()), water_mat, Transform::from_xyz(0.0, terrain::water_level(), 0.0))));
    } else {
        let water = meshes.add(lake_mesh());
        commands.spawn((Water(water_mat.clone()), rt(water, water_mat, Transform::from_xyz(0.0, terrain::WATER_LEVEL, 0.0))));
    }

    // Cumulus: each cloud is one mesh of camera-facing puffs, re-sorted back-to-front every
    // frame on the CPU. (Separate entities would pop in front of each other as the camera
    // moves, flashing the darker base puffs over the bright tops.)
    let atlas = images.add(puff_atlas());
    let cloud_mat = materials.add(StandardMaterial {
        base_color_texture: Some(atlas),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let sun_dir = crate::field::sun_dir();
    // A summer afternoon over the mountains: fair-weather cumulus, flat-bottomed at about the
    // same height, heaped up in the middle, covering about a third of the sky.
    // The ray tracer cannot see these puffs, so clouds neither shade the ground nor show in the
    // lake. (Solid stand-ins reflect as dark rocks, since a solid has none of a cloud's inner
    // glow; emissive ones become lights and starve the sun of samples.)
    for k in 0..520u32 {
        let a = hash(k, 10) * TAU;
        let d = 500.0 + hash(k, 11).sqrt() * (CLOUD_FIELD - 500.0);
        // Far clouds are a few pixels across: fewer puffs do.
        let far = d > 7000.0;
        let size = 300.0 + 650.0 * hash(k, 12).powi(2);
        let base = 1700.0 + 250.0 * hash(k, 13);
        let home = Vec3::new(a.cos() * d, base, a.sin() * d);
        let n = if far { 10 + (hash(k, 14) * 8.0) as u32 } else { 26 + (hash(k, 14) * 26.0) as u32 };
        let stretch = 1.0 + 0.8 * hash(k, 15);
        let mut puffs = Vec::new();
        for p in 0..n {
            let h = |j: u32| hash(k * 100 + p, j);
            let ang = h(1) * TAU;
            let rad = h(2).sqrt() * 0.55;
            // Heaped towards the middle, and never below the flat base.
            let top = (0.5 - rad).max(0.0) * (0.5 + 0.9 * h(3));
            let r = (0.22 + 0.2 * h(4) * (1.0 - rad)) * size;
            let off = Vec3::new(ang.cos() * rad * stretch, top + r / size * 0.6, ang.sin() * rad);
            // Bright sunlit tops and sides, blue-grey bases in their own shadow.
            let up = (off.y / 0.7).clamp(0.0, 1.0);
            let sun_shade = (off.normalize_or_zero().dot(sun_dir) * 0.5 + 0.5) * 0.25;
            let k_shade = (up * 0.8 + 0.1 + sun_shade).clamp(0.0, 1.0);
            let c = Vec3::new(0.72, 0.76, 0.84).lerp(Vec3::new(1.0, 0.99, 0.97), k_shade);
            puffs.push((off * size, r, (h(5) * 3.0) as u32 % 3, [c.x, c.y, c.z, 1.0]));
        }
        let mesh = meshes.add(empty_cloud_mesh());
        commands.spawn((
            Cloud { home, mesh: mesh.clone(), puffs },
            Mesh3d(mesh),
            MeshMaterial3d(cloud_mat.clone()),
            Transform::from_translation(home),
            // Puffs extend well beyond the mesh's initial (empty) bounds.
            bevy::camera::visibility::NoFrustumCulling,
            // Thin vapour: it must not throw a hard sun-shadow onto the ground.
            bevy::light::NotShadowCaster,
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
                let q = Vec2::new(q.x * ang.cos() + q.y * ang.sin(), (-q.x * ang.sin() + q.y * ang.cos()) * (1.2 + 0.5 * h(5)));
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
    let half = terrain::LAKE_RADIUS * 2.0;
    let c = terrain::LAKE_CENTER;
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

fn empty_cloud_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 4])
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

/// Three soft cumulus puffs side by side (768x256).
fn puff_atlas() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let panels: Vec<Image> = (0..3).map(puff_texture).collect();
    let n = 256usize;
    let mut data = vec![0u8; 3 * n * n * 4];
    for (p, img) in panels.iter().enumerate() {
        let src = img.data.as_ref().unwrap();
        for y in 0..n {
            let dst = (y * 3 * n + p * n) * 4;
            data[dst..dst + n * 4].copy_from_slice(&src[y * n * 4..(y + 1) * n * 4]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: 3 * n as u32, height: n as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = bevy::image::ImageSampler::linear();
    img
}

/// Soft cumulus puff: noisy round falloff, white (tinted by material).
fn puff_texture(variant: u32) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let n = 256u32;
    let mut data = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let u = x as f32 / n as f32 * 2.0 - 1.0;
            let v = y as f32 / n as f32 * 2.0 - 1.0;
            let o = variant as f32 * 13.7;
            let edge = 0.62 + 0.22 * terrain::fbm(u * 2.5 + o, v * 2.5 - o, 4) + 0.08 * terrain::fbm(u * 9.0 + o, v * 9.0, 2);
            let d = (u * u + v * v).sqrt();
            // A fairly crisp edge, as a cumulus has in sunlight.
            let a = (1.0 - ((d - edge * 0.72) / (edge * 0.28)).clamp(0.0, 1.0)).powf(1.3);
            // Slightly darker centre-bottom for volume.
            let shade = 1.0 - 0.12 * (v.max(0.0)) * (1.0 - d);
            let c = (shade * 255.0) as u8;
            data.extend_from_slice(&[c, c, c, (a * 255.0) as u8]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = bevy::image::ImageSampler::linear();
    img
}

/// Ripples drift across the lake; clouds drift with the wind and wrap around the field.
pub fn animate_scenery(
    time: Res<Time>,
    scene: Res<Scene>,
    water: Query<&Water>,
    mut clouds: Query<(&Cloud, &mut Transform)>,
    camera: Query<&GlobalTransform, With<crate::camera::MainCamera>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Ok(cam) = camera.single() else { return };
    let (cam_pos, cam_rot) = (cam.translation(), cam.rotation());
    let (right, up) = (cam_rot * Vec3::X, cam_rot * Vec3::Y);
    let t = time.elapsed_secs();
    for w in water.iter().filter(|_| terrain::real().is_some()) {
        if let Some(mut m) = materials.get_mut(&w.0) {
            m.uv_transform = Affine2::from_scale_angle_translation(Vec2::splat(22.0), 0.3, Vec2::new(t * 0.006, t * 0.004));
        }
    }
    let wind = {
        let w = &scene.wind;
        -Vec3::new(w.from_dir.sin(), 0.0, -w.from_dir.cos()) * (w.speed * 2.5 + 3.0)
    };
    for (c, mut tf) in &mut clouds {
        let mut p = c.home + wind * t;
        // Wrap within a square around the field.
        p.x = (p.x + CLOUD_FIELD).rem_euclid(2.0 * CLOUD_FIELD) - CLOUD_FIELD;
        p.z = (p.z + CLOUD_FIELD).rem_euclid(2.0 * CLOUD_FIELD) - CLOUD_FIELD;
        tf.translation = p;

        // Billboard every puff and sort back-to-front within the cloud.
        let mut order: Vec<(f32, usize)> =
            c.puffs.iter().enumerate().map(|(i, (off, ..))| (-(p + *off).distance_squared(cam_pos), i)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let n = order.len();
        let (mut pos, mut uv, mut col) = (Vec::with_capacity(n * 4), Vec::with_capacity(n * 4), Vec::with_capacity(n * 4));
        for &(_, i) in &order {
            let (off, r, panel, tint) = c.puffs[i];
            let u0 = panel as f32 / 3.0;
            for (sx, sy, u, v) in [(-1.0f32, -1.0f32, 0.0f32, 1.0f32), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
                pos.push((off + (right * sx + up * sy) * r).to_array());
                uv.push([u0 + u / 3.0, v]);
                col.push(tint);
            }
        }
        let idx: Vec<u32> = (0..n as u32).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
        if let Some(mut m) = meshes.get_mut(&c.mesh) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; n * 4]);
            m.insert_indices(bevy::mesh::Indices::U32(idx));
        }
    }
}
