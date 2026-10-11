//! Lake water and cumulus clouds.
//!
//! Clouds are opaque, lumpy meshes rather than transparent billboards so they are part of the
//! ray-traced scene: lit by the sun and sky, casting shadows and reflecting in the lake.

use crate::mesh_util::{finish, normal_map};
use crate::model::rt;
use crate::terrain;
use crate::SharedSim;
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

const CLOUD_FIELD: f32 = 5200.0;

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
        waves.iter().map(|&(kx, ky, ph, amp)| amp * (TAU * (kx * u + ky * v) + ph).sin()).sum::<f32>() * 0.012
    }));
    let water_mat = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.004, 0.010, 0.010),
        perceptual_roughness: 0.02,
        reflectance: 0.5,
        normal_map_texture: Some(ripples),
        ..default()
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
        let radius = terrain::LAKE_RADIUS * 2.0;
        let water = meshes.add(finish(Circle::new(radius).mesh().resolution(96).build()));
        commands.spawn((
            Water(water_mat.clone()),
            rt(
                water,
                water_mat,
                Transform::from_xyz(terrain::LAKE_CENTER.x, terrain::WATER_LEVEL, terrain::LAKE_CENTER.y)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            ),
        ));
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
    let sun_dir = crate::world::sun_dir();
    for k in 0..44u32 {
        let a = hash(k, 10) * TAU;
        let d = 900.0 + hash(k, 11) * (CLOUD_FIELD - 900.0);
        let size = 260.0 + 420.0 * hash(k, 12);
        let base = 1500.0 + 500.0 * hash(k, 13);
        let home = Vec3::new(a.cos() * d, base, a.sin() * d);
        let n = 16 + (hash(k, 14) * 14.0) as u32;
        let mut puffs = Vec::new();
        for p in 0..n {
            // Flattened dome: wide base, heaped centre.
            let ang = hash(k * 100 + p, 1) * TAU;
            let rad = hash(k * 100 + p, 2).sqrt() * 0.5;
            let off = Vec3::new(ang.cos() * rad * 1.3, (0.42 - rad).max(0.0) * (0.6 + 0.8 * hash(k * 100 + p, 3)), ang.sin() * rad);
            let r = (0.28 + 0.22 * hash(k * 100 + p, 4) * (1.0 - rad)) * size;
            // Shading: bright sunlit tops, soft blue-grey bases.
            let height_shade = (off.y / 0.5).clamp(0.0, 1.0) * 0.65 + 0.25;
            let sun_shade = (off.normalize_or_zero().dot(sun_dir) * 0.5 + 0.5) * 0.3;
            let k_shade = (height_shade + sun_shade).clamp(0.0, 1.0);
            let c = Vec3::new(0.66, 0.70, 0.78).lerp(Vec3::new(1.0, 0.99, 0.97), k_shade);
            puffs.push((off * size, r, (hash(k * 100 + p, 5) * 3.0) as u32 % 3, [c.x, c.y, c.z, 1.0]));
        }
        let mesh = meshes.add(empty_cloud_mesh());
        commands.spawn((
            Cloud { home, mesh: mesh.clone(), puffs },
            Mesh3d(mesh),
            MeshMaterial3d(cloud_mat.clone()),
            Transform::from_translation(home),
            // Puffs extend well beyond the mesh's initial (empty) bounds.
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
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
            let a = (1.0 - ((d - edge * 0.55) / (edge * 0.45)).clamp(0.0, 1.0)).powf(1.6);
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
    sim: Res<SharedSim>,
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
    for w in &water {
        if let Some(mut m) = materials.get_mut(&w.0) {
            m.uv_transform = Affine2::from_scale_angle_translation(Vec2::splat(22.0), 0.3, Vec2::new(t * 0.006, t * 0.004));
        }
    }
    let wind = {
        let c = sim.0.lock().unwrap();
        let w = &c.wind;
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
