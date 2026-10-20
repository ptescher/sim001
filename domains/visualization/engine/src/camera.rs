//! Cameras: line-of-sight (pilot on the field), chase, and FPV (nose camera).

use crate::terrain;
use crate::{RenderSettings, Scene, ViewInput};
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::{CameraMainTextureUsages, Exposure};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::AtmosphereEnvironmentMapLight;
use bevy::pbr::AtmosphereSettings;
use bevy::post_process::auto_exposure::{AutoExposure, AutoExposureCompensationCurve};
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
    /// The camera draws a part of the picture and the rest is rebuilt over time (see `upscale`),
    /// rather than the picture being smaller and stretched. SIM001_UPSCALE=0 turns it off.
    pub upscale: bool,
}

#[derive(Component)]
struct SceneView;

/// Draw the window's UI (the scene and the HUD over it) into an image of this size instead of
/// the window: for pictures of the whole view that do not need the window on screen (macOS does
/// not draw a window that is covered or on another Space). Insert it before the engine starts.
#[derive(Resource, Clone, Copy)]
pub struct UiOffscreen(pub UVec2);

/// The image that the UI is drawn into with [`UiOffscreen`].
#[derive(Resource, Clone)]
pub struct UiImage(pub Handle<Image>);

pub fn spawn_camera(commands: &mut Commands, settings: &RenderSettings, images: &mut Assets<Image>, offscreen: Option<UiOffscreen>) {
    use bevy::camera::RenderTarget;
    use bevy::image::ImageSampler;
    use bevy::render::render_resource::TextureFormat;

    let mut target = Image::new_target_texture(1920, 1080, TextureFormat::Bgra8UnormSrgb, None);
    target.sampler = ImageSampler::linear();
    let image = images.add(target);
    let upscale = settings.raytracing && std::env::var("SIM001_UPSCALE").map_or(true, |v| v != "0");
    commands.insert_resource(RenderScale { scale: if settings.raytracing { 0.0 } else { 1.0 }, auto: settings.raytracing, image: image.clone(), suspended: true, upscale });

    // Window camera: draws the upscaled scene image and the HUD at full resolution.
    let mut ui_camera = commands.spawn((Camera2d, Camera { order: 1, ..default() }, bevy::ui::IsDefaultUiCamera));
    if let Some(UiOffscreen(size)) = offscreen {
        let mut target = Image::new_target_texture(size.x, size.y, TextureFormat::Bgra8UnormSrgb, None);
        target.sampler = ImageSampler::linear();
        let target = images.add(target);
        ui_camera.insert(RenderTarget::Image(target.clone().into()));
        commands.insert_resource(UiImage(target));
    }
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
    if upscale {
        // The ray tracer draws a smaller picture (see `RenderScale`) and the full-size one is
        // rebuilt over time, which takes the place of the temporal anti-aliasing.
        cam.remove::<TemporalAntiAliasing>().insert(crate::upscale::TemporalUpscale::default());
    }
}

/// Bevy's auto exposure brings the scene's average to 1.0; a camera brings it to mid grey.
pub const METERING: Handle<AutoExposureCompensationCurve> = bevy::asset::uuid_handle!("6f1d2c3e-7a0b-4b8e-9a51-3c0e2d9b7a14");

/// About 2.5 stops down, at every brightness: the average lands on 18% grey.
pub fn metering_curve() -> AutoExposureCompensationCurve {
    let down = -1.5f32;
    AutoExposureCompensationCurve::from_curve(bevy::curve::cubic_splines::LinearSpline::new([
        Vec2::new(-8.0, down),
        Vec2::new(8.0, down),
    ]))
    .expect("a flat line is a curve")
}

/// Sky, exposure, grading and anti-aliasing shared by every camera that renders the scene
/// (the main view and the aircraft's sensor camera), so they look the same.
pub fn scene_look() -> impl Bundle {
    (
        AtmosphereSettings::default(),
        AtmosphereEnvironmentMapLight::default(),
        // Sets the pre-exposure; auto exposure then meters the picture and follows the light.
        Exposure { ev100: 14.0 },
        AutoExposure {
            range: -6.0..=6.0,
            speed_brighten: 1.5,
            speed_darken: 1.5,
            compensation_curve: METERING,
            ..default()
        },
        // A filmic curve and no grading on top: highlights roll off to white as on film or a
        // sensor, and colours are as lit, not pushed.
        Tonemapping::AgX,
        bevy::render::view::ColorGrading::default(),
        // Aerial perspective: far ridges fade into a pale blue haze. Solari does not apply it:
        // ray-traced cameras get `haze::Haze` instead (see `add_raytracing`).
        DistanceFog {
            color: Color::srgb(0.62, 0.72, 0.85),
            directional_light_color: Color::srgba(1.0, 0.95, 0.85, 0.3),
            directional_light_exponent: 20.0,
            falloff: FogFalloff::from_visibility_colors(25_000.0, Color::srgb(0.40, 0.48, 0.60), Color::srgb(0.75, 0.83, 0.95)),
        },
        crate::clouds::Clouds::default(),
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
    cam.insert((solari_settings(), CameraMainTextureUsages::default().with(TextureUsages::STORAGE_BINDING), crate::haze::Haze::default()));
}

/// A camera placed by hand, for pictures of the scenery: heights are above the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub eye: Vec3,
    pub target: Vec3,
    /// Vertical field of view, degrees.
    pub fov: f32,
}

impl View {
    /// `x,agl,z,tx,tagl,tz[,fov]`: the eye and the point it looks at, each a height above the
    /// ground, and the field of view (default 60 degrees).
    pub fn parse(s: &str) -> Option<View> {
        let n: Vec<f32> = s.split(',').map(|s| s.trim().parse().ok()).collect::<Option<_>>()?;
        (n.len() >= 6).then(|| View { eye: Vec3::new(n[0], n[1], n[2]), target: Vec3::new(n[3], n[4], n[5]), fov: n.get(6).copied().unwrap_or(60.0) })
    }

    pub fn transform(&self) -> Transform {
        let gy = |p: Vec3| Vec3::new(p.x, terrain::ground_height(p.x, p.z) + p.y, p.z);
        Transform::from_translation(gy(self.eye)).looking_at(gy(self.target), Vec3::Y)
    }
}

/// When set, the main camera stays at this view.
#[derive(Resource, Default, Clone, Copy)]
pub struct PinnedView(pub Option<View>);

pub fn camera_plugin(app: &mut App) {
    app.init_resource::<PinnedView>().insert_resource(CamState { mode: CamMode::LineOfSight, auto_zoom: true, knob_view: None, chase_pos: Vec3::ZERO, fov: 50.0 })
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
    mut upscale: Query<&mut crate::upscale::TemporalUpscale, With<MainCamera>>,
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
        // SIM001_SCALE pins it (for timing experiments).
        if let Some(pinned) = std::env::var("SIM001_SCALE").ok().and_then(|v| v.parse::<f32>().ok()) {
            rs.scale = pinned.clamp(0.1, 1.0);
            rs.auto = false;
        }
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
    // With temporal upscaling the picture is always the window's size and the camera draws a
    // part of it; without, the picture itself is smaller and the screen stretches it.
    let want = if rs.suspended {
        UVec2::new(64, 36)
    } else if rs.upscale {
        phys
    } else {
        (phys.as_vec2() * rs.scale).round().as_uvec2().max(UVec2::splat(64))
    };
    for mut u in &mut upscale {
        u.scale = rs.scale;
    }
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
    mut input: ResMut<ViewInput>,
    scene: Res<Scene>,
    mut cs: ResMut<CamState>,
    pinned: Res<PinnedView>,
    mut cam: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    if std::mem::take(&mut input.cycle) {
        cs.mode = match cs.mode {
            CamMode::LineOfSight => CamMode::Chase,
            CamMode::Chase => CamMode::Fpv,
            CamMode::Fpv => CamMode::LineOfSight,
        };
    }
    if std::mem::take(&mut input.toggle_auto_zoom) {
        cs.auto_zoom = !cs.auto_zoom;
    }
    match input.view_knob.map(knob_view) {
        Some(v) if cs.knob_view != Some(v) => {
            cs.knob_view = Some(v);
            cs.mode = v;
        }
        None => cs.knob_view = None,
        _ => {}
    }
    let (pos, rot) = scene.vehicles[0].as_ref().map_or((Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY), |p| (p.pos, p.rot));
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
            if let Some(k) = input.zoom_knob {
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
    // A pinned view (SIM001_VIEW, or a capture) wins over the pilot's.
    let mut target_fov = target_fov;
    if let Some(v) = pinned.0.or_else(|| std::env::var("SIM001_VIEW").ok().and_then(|v| View::parse(&v))) {
        *t = v.transform();
        target_fov = v.fov;
        cs.fov = target_fov;
    }
    cs.fov += (target_fov - cs.fov) * (1.0 - (-dt * 4.0).exp());
    if let Projection::Perspective(p) = &mut *proj {
        p.fov = cs.fov.to_radians();
        p.near = if cs.mode == CamMode::Fpv { 0.01 } else { 0.05 };
    }
}

/// Solari tuned for an outdoor scene lit by one sun plus the sky. The defaults (8 direct-light
/// samples, 3 bounces, 15 cm cache cells) target interiors with many lights. Each value can be
/// overridden with an env var (SIM001_DI, SIM001_DI2, SIM001_BOUNCES, SIM001_CELL) for experiments.
fn solari_settings() -> SolariLighting {
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
    SolariLighting {
        primary_di_samples: env("SIM001_DI").map(|v| v as u32).unwrap_or(2),
        secondary_di_samples: env("SIM001_DI2").map(|v| v as u32).unwrap_or(1),
        max_bounces: env("SIM001_BOUNCES").map(|v| v as u32).unwrap_or(2),
        world_cache_position_base_cell_size: env("SIM001_CELL").unwrap_or(0.4),
        world_cache_max_gi_ray_distance: 120.0,
        ..SolariLighting::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::view::ColorGrading;

    /// The view should look like a camera's picture, not a painting: a filmic curve that rolls
    /// highlights off towards white, no extra saturation or contrast, and an exposure that
    /// follows the light.
    #[test]
    fn the_scene_looks_like_a_photo() {
        let mut world = World::new();
        let e = world.spawn(scene_look()).id();
        assert_eq!(world.get::<Tonemapping>(e), Some(&Tonemapping::AgX));
        let g = world.get::<ColorGrading>(e).expect("a colour grading");
        assert_eq!(g.global.post_saturation, 1.0, "no saturation boost");
        for s in [&g.shadows, &g.midtones, &g.highlights] {
            assert_eq!((s.contrast, s.saturation), (1.0, 1.0), "no contrast or saturation boost");
        }
        assert!(world.get::<AutoExposure>(e).is_some(), "the exposure follows the light");
    }

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
    fn a_view_parses_with_and_without_its_field_of_view() {
        let v = View::parse("1,2,3,4,5,6").unwrap();
        assert_eq!((v.eye, v.target, v.fov), (Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0), 60.0));
        assert_eq!(View::parse("1,2,3,4,5,6,30").unwrap().fov, 30.0);
        assert_eq!(View::parse("1,2,3"), None);
        assert_eq!(View::parse("a,b,c,d,e,f"), None);
    }

    #[test]
    fn the_view_knob_picks_a_view_in_thirds() {
        assert_eq!(knob_view(0.0), CamMode::LineOfSight);
        assert_eq!(knob_view(0.32), CamMode::LineOfSight);
        assert_eq!(knob_view(0.5), CamMode::Chase);
        assert_eq!(knob_view(0.68), CamMode::Fpv);
        assert_eq!(knob_view(1.0), CamMode::Fpv);
    }
}
