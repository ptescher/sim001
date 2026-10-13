//! Cameras: line-of-sight (pilot on the field), chase, and FPV (nose camera).

use crate::input::{Action, PilotInput};
use crate::terrain;
use crate::{RenderSettings, SharedSim};
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::{CameraMainTextureUsages, Exposure};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::AtmosphereEnvironmentMapLight;
use bevy::pbr::AtmosphereSettings;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::TextureUsages;
use bevy::solari::prelude::SolariLighting;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamMode {
    LineOfSight,
    Chase,
    Fpv,
}

impl CamMode {
    pub fn label(self) -> &'static str {
        match self {
            CamMode::LineOfSight => "LINE OF SIGHT",
            CamMode::Chase => "CHASE",
            CamMode::Fpv => "FPV",
        }
    }
}

/// The field of view (degrees) a zoom knob (0 to 1) picks for the line-of-sight camera: from the
/// widest, 55, to the narrowest, 14, which are the limits of the automatic zoom too.
pub fn knob_fov(k: f32) -> f32 {
    55.0 + k.clamp(0.0, 1.0) * (14.0 - 55.0)
}

/// The view a knob (0 to 1) picks, in thirds.
pub fn knob_view(k: f32) -> CamMode {
    if k < 1.0 / 3.0 {
        CamMode::LineOfSight
    } else if k < 2.0 / 3.0 {
        CamMode::Chase
    } else {
        CamMode::Fpv
    }
}

#[derive(Resource)]
pub struct CamState {
    pub mode: CamMode,
    pub auto_zoom: bool,
    /// The view the radio's view knob last picked: the knob only changes the view when it moves
    /// to another third, so C still works.
    knob_view: Option<CamMode>,
    chase_pos: Vec3,
    fov: f32,
}

#[derive(Component)]
pub struct MainCamera;

/// The 3D view renders into an offscreen image at `scale` x window resolution, which is
/// shown full-screen behind the HUD. Ray tracing cost scales with pixel count, so this is
/// the main performance knob. With `auto` (the default when ray tracing) the scale follows
/// the frame time to hold [`TARGET_FPS`]; F5 / F6 switch to manual, F7 back to auto.
#[derive(Resource)]
pub struct RenderScale {
    pub scale: f32,
    pub auto: bool,
    pub image: Handle<Image>,
    /// While loading: render a tiny frame so the GPU is free (deactivating the camera
    /// trips Bevy's atmosphere probe, which needs a live view).
    pub suspended: bool,
}

#[derive(Component)]
struct SceneView;

pub fn spawn_camera(commands: &mut Commands, settings: &RenderSettings, images: &mut Assets<Image>) {
    use bevy::camera::RenderTarget;
    use bevy::image::ImageSampler;
    use bevy::render::render_resource::TextureFormat;

    let mut target = Image::new_target_texture(1920, 1080, TextureFormat::Bgra8UnormSrgb, None);
    target.sampler = ImageSampler::linear();
    let image = images.add(target);
    commands.insert_resource(RenderScale { scale: if settings.raytracing { 0.0 } else { 1.0 }, auto: settings.raytracing, image: image.clone(), suspended: true });

    // Window camera: draws the upscaled scene image and the HUD at full resolution.
    commands.spawn((Camera2d, Camera { order: 1, ..default() }, bevy::ui::IsDefaultUiCamera));
    commands.spawn((
        SceneView,
        ImageNode::new(image.clone()),
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), ..default() },
        GlobalZIndex(i32::MIN),
    ));

    let p = terrain::pilot_pos();
    let mut cam = commands.spawn((
        MainCamera,
        Camera3d::default(),
        RenderTarget::Image(image.into()),
        Projection::Perspective(PerspectiveProjection { fov: 50f32.to_radians(), near: 0.05, ..default() }),
        Transform::from_xyz(p.x, 1.7, p.z).looking_at(Vec3::new(0.0, 5.0, -60.0), Vec3::Y),
        scene_look(),
    ));
    if settings.raytracing {
        add_raytracing(&mut cam);
    }
}

/// Sky, exposure, grading and anti-aliasing shared by every camera that renders the scene
/// (the main view and the aircraft's sensor camera), so they look the same.
pub fn scene_look() -> impl Bundle {
    (
        AtmosphereSettings::default(),
        AtmosphereEnvironmentMapLight::default(),
        Exposure { ev100: 14.0 },
        Tonemapping::TonyMcMapface,
        bevy::render::view::ColorGrading {
            global: bevy::render::view::ColorGradingGlobal { post_saturation: 1.15, ..default() },
            midtones: bevy::render::view::ColorGradingSection { contrast: 1.12, ..default() },
            shadows: bevy::render::view::ColorGradingSection { contrast: 1.05, saturation: 1.05, ..default() },
            ..default()
        },
        Bloom::NATURAL,
        Msaa::Off,
        TemporalAntiAliasing::default(),
    )
}

/// Lighter ray tracing for the 720p sensor feed: one light sample and one bounce.
pub fn feed_solari_settings() -> SolariLighting {
    SolariLighting { primary_di_samples: 1, secondary_di_samples: 1, max_bounces: 1, ..solari_settings() }
}

/// Solari needs storage access to the main texture and MSAA off.
pub fn add_raytracing(cam: &mut EntityCommands) {
    cam.insert((solari_settings(), CameraMainTextureUsages::default().with(TextureUsages::STORAGE_BINDING)));
}

pub fn camera_plugin(app: &mut App) {
    app.insert_resource(CamState { mode: CamMode::LineOfSight, auto_zoom: true, knob_view: None, chase_pos: Vec3::ZERO, fov: 50.0 })
        .add_systems(Update, update_render_scale);
}

/// Frame rate the automatic render scale aims for.
pub const TARGET_FPS: f32 = 60.0;
const MIN_SCALE: f32 = 0.35;

/// Dynamic-resolution state: frame times are averaged over short windows, the scale drops at
/// once when a window misses the target and creeps back up after a while at the target.
#[derive(Default)]
struct AutoScale {
    sum: f32,
    frames: u32,
    /// Windows to ignore after a resize (reallocating the ray-tracing buffers hitches).
    settle: u32,
    /// Seconds spent holding the target since the last change.
    holding: f32,
    /// Scale a probe upwards started from; if the probe misses, go back and wait longer.
    probe_from: Option<f32>,
    hold_off: f32,
}

/// Keeps the offscreen scene image at `scale` x the window's physical size.
fn update_render_scale(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut rs: ResMut<RenderScale>,
    mut images: ResMut<Assets<Image>>,
    mut auto: Local<AutoScale>,
) {
    let Ok(window) = windows.single() else { return };
    let phys = UVec2::new(window.physical_width(), window.physical_height());
    if phys.x == 0 || phys.y == 0 {
        return;
    }
    if rs.scale <= 0.0 {
        // Starting point for ray tracing: about 1600 pixels across.
        rs.scale = (1600.0 / phys.x as f32).clamp(MIN_SCALE, 1.0);
    }
    if keys.just_pressed(KeyCode::F5) {
        rs.auto = false;
        rs.scale = (rs.scale - 0.1).max(0.3);
    }
    if keys.just_pressed(KeyCode::F6) {
        rs.auto = false;
        rs.scale = (rs.scale + 0.1).min(1.0);
    }
    if keys.just_pressed(KeyCode::F7) {
        rs.auto = true;
        *auto = AutoScale::default();
    }
    if rs.auto && !rs.suspended {
        if let Some(scale) = auto_scale(&mut auto, rs.scale, time.delta_secs()) {
            rs.scale = scale;
        }
    }
    let want = if rs.suspended { UVec2::new(64, 36) } else { (phys.as_vec2() * rs.scale).round().as_uvec2().max(UVec2::splat(64)) };
    let handle = rs.image.clone();
    // Only take a mutable borrow when resizing: `get_mut` flags the asset as modified.
    if images.get(&handle).is_some_and(|img| img.size() != want) {
        if let Some(mut img) = images.get_mut(&handle) {
            img.resize(bevy::render::render_resource::Extent3d { width: want.x, height: want.y, depth_or_array_layers: 1 });
        }
        auto.settle = 2;
    }
}

/// One step of the dynamic-resolution controller; returns a new scale when it changes.
/// Ray-tracing cost goes with pixel count, i.e. with scale squared.
fn auto_scale(a: &mut AutoScale, scale: f32, dt: f32) -> Option<f32> {
    const WINDOW: f32 = 0.5;
    let target = 1.0 / TARGET_FPS;
    a.sum += dt;
    a.frames += 1;
    a.hold_off = (a.hold_off - dt).max(0.0);
    if a.sum < WINDOW {
        return None;
    }
    let mean = a.sum / a.frames as f32;
    (a.sum, a.frames) = (0.0, 0);
    if a.settle > 0 {
        a.settle -= 1;
        return None;
    }
    if mean > target * 1.1 {
        a.holding = 0.0;
        if let Some(from) = a.probe_from.take() {
            // The last step up was too much: go back and leave it there for a while.
            a.hold_off = 30.0;
            return Some(from);
        }
        let next = (scale * (target / mean).sqrt() * 0.97).clamp(MIN_SCALE, 1.0).max(scale * 0.75);
        return (next < scale - 0.005).then_some(next);
    }
    a.probe_from = None;
    // Well under the target (a faster display, or load went away): step up straight away.
    if mean < target * 0.8 {
        let next = (scale * (target / mean).sqrt() * 0.95).min(scale * 1.15).min(1.0);
        return (next > scale + 0.005).then_some(next);
    }
    // At the target (vsync hides any headroom): probe upwards now and then.
    a.holding += WINDOW;
    if a.holding >= 4.0 && a.hold_off <= 0.0 && scale < 1.0 {
        a.holding = 0.0;
        a.probe_from = Some(scale);
        return Some((scale * 1.06).min(1.0));
    }
    None
}
pub fn update_camera(
    time: Res<Time>,
    input: Res<PilotInput>,
    sim: Res<SharedSim>,
    mut cs: ResMut<CamState>,
    mut cam: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    if input.has(Action::Camera) {
        cs.mode = match cs.mode {
            CamMode::LineOfSight => CamMode::Chase,
            CamMode::Chase => CamMode::Fpv,
            CamMode::Fpv => CamMode::LineOfSight,
        };
    }
    if input.has(Action::Zoom) {
        cs.auto_zoom = !cs.auto_zoom;
    }
    match input.radio_view.map(knob_view) {
        Some(v) if cs.knob_view != Some(v) => {
            cs.knob_view = Some(v);
            cs.mode = v;
        }
        None => cs.knob_view = None,
        _ => {}
    }
    let (pos, rot) = {
        let c = sim.0.lock().unwrap();
        (c.state.pos, c.state.rot)
    };
    let dt = time.delta_secs().min(0.1);
    let Ok((mut t, mut proj)) = cam.single_mut() else { return };

    let target_fov = match cs.mode {
        CamMode::LineOfSight => {
            let p = terrain::pilot_pos();
            let eye = Vec3::new(p.x, terrain::ground_height(p.x, p.z) + 1.7, p.z);
            t.translation = eye;
            let look = pos - eye;
            if look.length() > 0.5 {
                t.look_at(pos, Vec3::Y);
            } else {
                t.look_at(eye + Vec3::NEG_Z, Vec3::Y);
            }
            if let Some(k) = input.radio_zoom {
                knob_fov(k)
            } else if cs.auto_zoom {
                // Keep the plane at a comfortable apparent size, like your eyes "zooming" in.
                (2.0 * (5.0 / look.length().max(1.0)).atan()).to_degrees().clamp(14.0, 55.0)
            } else {
                55.0
            }
        }
        CamMode::Chase => {
            let fwd = rot * Vec3::NEG_Z;
            let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
            let desired = pos - flat * 3.2 + Vec3::Y * 0.9;
            if cs.chase_pos == Vec3::ZERO || cs.chase_pos.distance(desired) > 50.0 {
                cs.chase_pos = desired;
            }
            let k = 1.0 - (-dt * 5.0).exp();
            cs.chase_pos = cs.chase_pos.lerp(desired, k);
            let ground = terrain::ground_height(cs.chase_pos.x, cs.chase_pos.z) + 0.3;
            cs.chase_pos.y = cs.chase_pos.y.max(ground);
            t.translation = cs.chase_pos;
            t.look_at(pos + Vec3::Y * 0.2, Vec3::Y);
            60.0
        }
        CamMode::Fpv => {
            // Nose camera, tilted up 20°, as on a typical FPV setup.
            t.translation = pos + rot * Vec3::new(0.0, 0.016, -0.27);
            t.rotation = rot * Quat::from_rotation_x(20f32.to_radians());
            // ~120° diagonal on 16:9 is roughly 75° vertical.
            75.0
        }
    };
    // T1S_VIEW="x,agl,z,tx,tagl,tz[,fov]": pin the camera (for screenshots of the scenery).
    let mut target_fov = target_fov;
    if let Ok(v) = std::env::var("T1S_VIEW") {
        let n: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if n.len() >= 6 {
            // Heights are above ground level.
            let gy = |x: f32, z: f32| terrain::ground_height(x, z);
            *t = Transform::from_xyz(n[0], gy(n[0], n[2]) + n[1], n[2])
                .looking_at(Vec3::new(n[3], gy(n[3], n[5]) + n[4], n[5]), Vec3::Y);
            target_fov = n.get(6).copied().unwrap_or(60.0);
            cs.fov = target_fov;
        }
    }
    cs.fov += (target_fov - cs.fov) * (1.0 - (-dt * 4.0).exp());
    if let Projection::Perspective(p) = &mut *proj {
        p.fov = cs.fov.to_radians();
        p.near = if cs.mode == CamMode::Fpv { 0.01 } else { 0.05 };
    }
}

/// Solari tuned for an outdoor scene lit by one sun plus the sky. The defaults (8 direct-light
/// samples, 3 bounces, 15 cm cache cells) target interiors with many lights. Each value can be
/// overridden with an env var (T1S_DI, T1S_DI2, T1S_BOUNCES, T1S_CELL) for experiments.
fn solari_settings() -> SolariLighting {
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
    SolariLighting {
        primary_di_samples: env("T1S_DI").map(|v| v as u32).unwrap_or(2),
        secondary_di_samples: env("T1S_DI2").map(|v| v as u32).unwrap_or(1),
        max_bounces: env("T1S_BOUNCES").map(|v| v as u32).unwrap_or(2),
        world_cache_position_base_cell_size: env("T1S_CELL").unwrap_or(0.4),
        world_cache_max_gi_ray_distance: 120.0,
        ..SolariLighting::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P1 zooms the line-of-sight camera: all the way down is the widest view, all the way up the
    /// narrowest, the same range the automatic zoom works in.
    #[test]
    fn the_zoom_knob_runs_from_wide_to_narrow() {
        assert_eq!(knob_fov(0.0), 55.0);
        assert_eq!(knob_fov(1.0), 14.0);
        assert!(knob_fov(0.5) < 55.0 && knob_fov(0.5) > 14.0);
        assert_eq!(knob_fov(-3.0), 55.0, "out of range is clamped");
    }

    /// P2 picks the view in thirds: line of sight, chase, then the nose camera.
    #[test]
    fn the_view_knob_picks_a_view_in_thirds() {
        assert_eq!(knob_view(0.0), CamMode::LineOfSight);
        assert_eq!(knob_view(0.32), CamMode::LineOfSight);
        assert_eq!(knob_view(0.5), CamMode::Chase);
        assert_eq!(knob_view(0.68), CamMode::Fpv);
        assert_eq!(knob_view(1.0), CamMode::Fpv);
    }
}
