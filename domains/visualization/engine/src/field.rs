//! The flying field: terrain, runway, lake, trees, buildings, windsock, sun and sky.

use crate::mesh_util::{GeoBuilder, cuboid, finish, texture};
use crate::model::rt;
use crate::terrain::{self, fbm};
use crate::{RenderSettings, Scene};
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, SunDisk, light_consts::lux};
use bevy::prelude::*;

#[derive(Component)]
pub struct Windsock;

pub fn spawn_world(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    media: &mut Assets<ScatteringMedium>,
    settings: &RenderSettings,
) {
    // Sky + sun. Raw sunlight is filtered by the atmosphere for sky colour; Solari uses the
    // directional light directly and the atmosphere's environment map for sky lighting.
    commands.spawn(Atmosphere::earth(media.add(ScatteringMedium::earth(256, 256))));
    commands.spawn((
        DirectionalLight { illuminance: lux::RAW_SUNLIGHT, shadow_maps_enabled: !settings.raytracing, ..default() },
        SunDisk::EARTH,
        Transform::IDENTITY.looking_to(-sun_dir(), Vec3::Y),
    ));

    // Terrain, trees and boulders are generated in the background (see `loading`).

    // ---- runway texture ----
    let asphalt = images.add(texture(512, |u, v| {
        let n = fbm(u * 128.0, v * 128.0, 3);
        let patch = fbm(u * 6.0 + 3.0, v * 6.0, 3);
        let c = 0.07 + 0.04 * n + 0.03 * (patch - 0.5);
        [c, c, c * 1.03]
    }));
    let asphalt_mat = materials.add(StandardMaterial {
        base_color_texture: Some(asphalt),
        perceptual_roughness: 0.85,
        reflectance: 0.3,
        ..default()
    });

    let alpine = terrain::alpine();
    // ---- runway (the Alpine Valley's; a real place's runway is in its aerial photo) ----
    let rc = terrain::RUNWAY_CENTER;
    let mut rw = GeoBuilder::default();
    let (hw, hl) = (terrain::RUNWAY_HALF_WIDTH, terrain::RUNWAY_HALF_LENGTH);
    let corners = [(-hw, -hl), (hw, -hl), (hw, hl), (-hw, hl)];
    let ids: Vec<u32> = corners
        .iter()
        .map(|&(x, z)| rw.vert(Vec3::new(rc.x + x, 0.01, rc.y + z), Vec2::new(x / 8.0, z / 8.0)))
        .collect();
    rw.quad(ids[0], ids[1], ids[2], ids[3], Vec3::Y);
    if alpine {
        commands.spawn(rt(meshes.add(rw.build()), asphalt_mat, Transform::IDENTITY));
    }
    let paint = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.85, 0.82),
        perceptual_roughness: 0.7,
        ..default()
    });
    let dash = meshes.add(cuboid(Vec3::new(0.3, 0.01, 6.0)));
    let mut z = -hl + 14.0;
    while alpine && z < hl - 14.0 {
        commands.spawn(rt(dash.clone(), paint.clone(), Transform::from_xyz(rc.x, 0.015, rc.y + z)));
        z += 12.0;
    }
    let bar = meshes.add(cuboid(Vec3::new(0.45, 0.01, 5.0)));
    for end in if alpine { [-1.0f32, 1.0].as_slice() } else { &[] } {
        let end = *end;
        for k in 0..8 {
            let x = -hw + 1.2 + k as f32 * ((2.0 * hw - 2.4) / 7.0);
            commands.spawn(rt(
                bar.clone(),
                paint.clone(),
                Transform::from_xyz(rc.x + x, 0.015, rc.y + end * (hl - 4.0)),
            ));
        }
    }

    // ---- lake and clouds ----
    crate::scenery::spawn_scenery(commands, meshes, materials, images);

    // ---- buildings ----
    // Plastered walls with windows under a pitched roof (`building`); houses of the same size
    // share their meshes.
    let plaster = images.add(crate::building::wall_texture());
    // Matte roofs of clay tile, slate and old wood, and walls of white, cream and ochre plaster,
    // picked per house.
    let roofs: Vec<Handle<StandardMaterial>> = [[0.36, 0.13, 0.08], [0.16, 0.16, 0.17], [0.27, 0.17, 0.11]]
        .map(|c| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(c[0], c[1], c[2]),
                perceptual_roughness: 0.8,
                ..default()
            })
        })
        .into();
    let walls: Vec<Handle<StandardMaterial>> = [[0.95, 0.93, 0.88], [0.93, 0.87, 0.74], [0.90, 0.80, 0.62]]
        .map(|c| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(c[0], c[1], c[2]),
                base_color_texture: Some(plaster.clone()),
                perceptual_roughness: 0.85,
                ..default()
            })
        })
        .into();
    // At a real place, roofs take their colour from the aerial photo (it sees them from above),
    // quantized so similar roofs share a material; walls are a light neutral.
    let mut roof_mats: std::collections::HashMap<[u8; 3], Handle<StandardMaterial>> = Default::default();
    let real_wall = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.36, 0.35, 0.33),
        base_color_texture: Some(plaster),
        perceptual_roughness: 0.85,
        ..default()
    });
    let mut houses: std::collections::HashMap<[i32; 3], (Handle<Mesh>, Handle<Mesh>)> = Default::default();
    for b in terrain::buildings() {
        let rot = Quat::from_rotation_y(b.yaw);
        let (wall, roof) = match terrain::real() {
            Some(site) => {
                let c = site.color(b.center.x, b.center.z);
                let key = c.map(|v| (v.sqrt() * 12.0).round() as u8);
                let mat = roof_mats
                    .entry(key)
                    .or_insert_with(|| {
                        let lin = key.map(|k| (k as f32 / 12.0).powi(2) * 0.8);
                        materials.add(StandardMaterial {
                            base_color: Color::linear_rgb(lin[0], lin[1], lin[2]),
                            perceptual_roughness: 0.75,
                            ..default()
                        })
                    })
                    .clone();
                (real_wall.clone(), mat)
            }
            None => {
                let pick = (b.center.x * 0.37 + b.center.z * 0.61).abs() as usize;
                (walls[pick % walls.len()].clone(), roofs[(pick / 3) % roofs.len()].clone())
            }
        };
        let key = (b.size * 10.0).round().as_ivec3().to_array();
        let (walls, top) = houses
            .entry(key)
            .or_insert_with(|| {
                let h = crate::building::house(key.map(|k| k as f32 / 10.0).into());
                (meshes.add(h.walls), meshes.add(h.roof))
            })
            .clone();
        let at = Transform::from_translation(b.center).with_rotation(rot);
        commands.spawn(rt(walls, wall, at));
        commands.spawn(rt(top, roof, at));
    }

    // ---- field furniture: pylons, pilot, windsock ----
    let orange = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.35, 0.02),
        perceptual_roughness: 0.5,
        ..default()
    });
    let pylon = meshes.add(finish(Cone { radius: 0.6, height: 2.2 }.mesh().resolution(16).build()));
    for &(x, z) in
        if alpine { [(-60.0, -160.0), (60.0, -160.0), (-60.0, 100.0), (60.0, 100.0)].as_slice() } else { &[] }
    {
        let h = terrain::ground_height(x, z);
        commands.spawn(rt(pylon.clone(), orange.clone(), Transform::from_xyz(x, h + 1.1, z)));
    }

    let p = terrain::pilot_pos();
    let ph = terrain::ground_height(p.x, p.z);
    let shirt = materials.add(StandardMaterial {
        base_color: Color::srgb(0.1, 0.25, 0.55),
        perceptual_roughness: 0.8,
        ..default()
    });
    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.6, 0.48),
        perceptual_roughness: 0.7,
        ..default()
    });
    let body = meshes.add(finish(Capsule3d::new(0.22, 1.0).mesh().build()));
    let head = meshes.add(finish(Sphere::new(0.12).mesh().ico(2).unwrap()));
    // The pilot stands by the pilot box (not in the middle of the Towers field).
    if terrain::arena().is_none() {
        commands.spawn(rt(body, shirt, Transform::from_xyz(p.x + 0.3, ph + 0.72, p.z + 0.3)));
        commands.spawn(rt(head, skin, Transform::from_xyz(p.x + 0.3, ph + 1.58, p.z + 0.3)));
    }

    let pole_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.7, 0.7, 0.72),
        metallic: 1.0,
        perceptual_roughness: 0.35,
        ..default()
    });
    let pole = meshes.add(finish(Cylinder::new(0.04, 5.0).mesh().build()));
    // Beside the pilot box.
    let (wx, wz) = (p.x + 8.0, p.z - 14.0);
    let wh = terrain::ground_height(wx, wz);
    if terrain::arena().is_none() {
        commands.spawn(rt(pole, pole_mat, Transform::from_xyz(wx, wh + 2.5, wz)));
    }
    if terrain::arena().is_some() {
        return;
    }
    let sock = meshes.add(finish(ConicalFrustum { radius_top: 0.1, radius_bottom: 0.25, height: 1.6 }.mesh().build()));
    let sock_root = commands.spawn((Windsock, Transform::from_xyz(wx, wh + 4.85, wz), Visibility::default())).id();
    let sock_mesh = commands
        .spawn(rt(
            sock,
            orange,
            // Frustum axis is +Y; lay it along +Z (downwind direction of the pivot).
            Transform::from_xyz(0.0, 0.0, 0.8).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ))
        .id();
    commands.entity(sock_root).add_child(sock_mesh);
}

pub fn update_windsock(scene: Res<Scene>, mut socks: Query<&mut Transform, With<Windsock>>) {
    let w = scene.wind.at_sock;
    for mut t in &mut socks {
        let horiz = Vec3::new(w.x, 0.0, w.z);
        let speed = horiz.length();
        let yaw = if speed > 0.05 { horiz.x.atan2(horiz.z) } else { 0.0 };
        // Fully extended at ~8 m/s, hanging down in calm air.
        let droop = (1.0 - (speed / 8.0).min(1.0)) * 1.3;
        t.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(droop);
    }
}

/// Direction *towards* the sun: a mid-afternoon sun (38 deg) in the west-south-west. The views
/// up the valley (north) are side-lit, so trees, buildings and ridges throw their shadows across
/// the picture, and the long shadows model the ground.
pub fn sun_dir() -> Vec3 {
    let elevation = 38f32.to_radians();
    let azimuth = 250f32.to_radians(); // clockwise from north (-Z)
    Vec3::new(azimuth.sin() * elevation.cos(), elevation.sin(), -azimuth.cos() * elevation.cos()).normalize()
}

#[cfg(test)]
mod tests {
    use super::sun_dir;

    /// Elevation and azimuth (clockwise from north) of the sun, in degrees.
    fn sun() -> (f32, f32) {
        let s = sun_dir();
        let elevation = s.y.asin().to_degrees();
        let azimuth = s.x.atan2(-s.z).to_degrees().rem_euclid(360.0);
        (elevation, azimuth)
    }

    /// A photo looks real when the light models the ground: a sun low enough to cast long
    /// shadows, and off to the side of the views up the valley (north), so the shadows fall
    /// across the picture and not behind what casts them.
    #[test]
    fn an_afternoon_sun_lights_the_valley_from_the_side() {
        let (elevation, azimuth) = sun();
        assert!((30.0..=45.0).contains(&elevation), "the sun is {elevation:.0} deg high");
        assert!((235.0..=285.0).contains(&azimuth), "the sun is at {azimuth:.0} deg, not in the west");
    }
}
