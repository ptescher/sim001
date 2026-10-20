//! The 3D view: terrain, vegetation, water, sky, the planes and the cameras that look at them.
//!
//! The engine draws a [`Scene`] and knows nothing of where it comes from. A source writes it
//! every frame, before [`EngineSet::Draw`]:
//!
//! - the simulator, from its flight model (`sim001`);
//! - a real plane, from its MAVLink telemetry (`visualization_telemetry`, `sim001_view`).
//!
//! The same view serves pilots and camera-based systems, so it aims to look like the real
//! thing: see the domain's README for the photo comparison.

pub mod camera;
pub mod capture;
pub mod clouds;

pub mod field;
pub mod foliage;
pub mod foam;
pub mod grass;
pub mod haze;
pub mod loading;
pub mod mapgen;
pub mod meadow;
pub mod mesh_util;
pub mod model;
pub mod rocks;
pub mod scenery;
pub mod terrain_render;
pub mod theme;
pub mod upscale;
pub mod treegen;
pub mod vegetation;

pub use visualization_world::{realsite, terrain};

use bevy::light::atmosphere::ScatteringMedium;
use bevy::prelude::*;
use model::PLANES;
use simulation_physics::Obstacle;

#[derive(Resource, Clone)]
pub struct RenderSettings {
    /// Bevy Solari (real-time ray tracing); otherwise rasterized lighting with shadow maps.
    pub raytracing: bool,
}

/// Where a plane is and what its moving parts are doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    /// World frame: +X east, +Y up, -Z north, metres from the world origin.
    pub pos: Vec3,
    /// Body to world; the body's nose is -Z, up is +Y.
    pub rot: Quat,
    /// Each lifting surface's deflection (rad), in the aircraft's surface order.
    pub deflection: Vec<f32>,
    /// Each motor's spool, 0 to 1, in the aircraft's motor order.
    pub spool: Vec<f32>,
    /// Each motor's tilt from pointing forward (rad), in the same order; empty for a plane whose
    /// motors don't tilt.
    pub tilt: Vec<f32>,
    /// A crashed plane's props are stopped.
    pub crashed: bool,
}

/// The wind, for the windsock and the clouds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wind {
    /// Where it blows from, radians clockwise from north.
    pub from_dir: f32,
    /// Mean speed, m/s.
    pub speed: f32,
    /// The wind (m/s, the way it blows) at the windsock, gusts and all.
    pub at_sock: Vec3,
}

/// What the engine draws. Slot 0 is the plane the cameras follow; the others are other aircraft
/// (computer pilots, or traffic). An empty slot is not drawn.
#[derive(Resource, Clone, Debug, Default)]
pub struct Scene {
    pub vehicles: [Option<Pose>; PLANES],
    pub wind: Wind,
}

/// What the pilot asked of the cameras this frame. The source sets it before
/// [`EngineSet::Draw`]; the engine clears the one-shot requests after reading them.
#[derive(Resource, Clone, Debug, Default)]
pub struct ViewInput {
    /// Next view: line of sight, chase, nose camera.
    pub cycle: bool,
    /// Switch the line-of-sight auto-zoom on or off.
    pub toggle_auto_zoom: bool,
    /// A radio's view knob (0 to 1), when it has one.
    pub view_knob: Option<f32>,
    /// A radio's zoom knob (0 to 1), when it has one.
    pub zoom_knob: Option<f32>,
}

/// What the world puts in a plane's way, ready once loading is done. A simulation flies into it.
#[derive(Resource, Clone, Debug, Default)]
pub struct Obstacles(pub Vec<Obstacle>);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum EngineSet {
    /// The scene's entities follow [`Scene`]: planes, windsock, clouds, cameras, grass.
    Draw,
}

/// The engine: world, sky, planes, cameras and the loading screen.
pub struct EnginePlugin {
    pub raytracing: bool,
    /// The colour of each plane slot's markings.
    pub liveries: Vec<Color>,
    /// Shown one at a time on the loading screen.
    pub tips: Vec<&'static str>,
}

impl Default for EnginePlugin {
    fn default() -> Self {
        Self {
            raytracing: false,
            liveries: vec![Color::srgb(1.0, 0.36, 0.05)],
            tips: vec![],
        }
    }
}

#[derive(Resource, Clone)]
struct Liveries(Vec<Color>);

impl Plugin for EnginePlugin {
    fn build(&self, app: &mut App) {
        if self.raytracing {
            app.add_plugins((bevy::solari::SolariPlugins, haze::HazePlugin, upscale::TemporalUpscalePlugin));
        }
        app.add_plugins((bevy::post_process::auto_exposure::AutoExposurePlugin, clouds::CloudsPlugin));
        app.world_mut()
            .resource_mut::<Assets<bevy::post_process::auto_exposure::AutoExposureCompensationCurve>>()
            .insert(&camera::METERING, camera::metering_curve())
            .expect("the metering curve's handle is free");
        app.insert_resource(RenderSettings {
            raytracing: self.raytracing,
        })
        .insert_resource(Liveries(self.liveries.clone()))
        .insert_resource(loading::Tips(self.tips.clone()))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GlobalAmbientLight::NONE)
        .init_resource::<Scene>()
        .init_resource::<ViewInput>()
        .add_systems(Startup, (setup, grass::setup_grass))
        .add_systems(
            Update,
            (
                model::sync_plane,
                field::update_windsock,
                scenery::animate_scenery,
                camera::update_camera,
                grass::stream_grass.run_if(in_state(loading::AppState::Running)),
            )
                .chain()
                .in_set(EngineSet::Draw),
        );
        camera::camera_plugin(app);
        loading::loading_plugin(app);
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut media: ResMut<Assets<ScatteringMedium>>,
    settings: Res<RenderSettings>,
    liveries: Res<Liveries>,
    offscreen: Option<Res<camera::UiOffscreen>>,
) {
    field::spawn_world(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut media,
        &settings,
    );
    for (i, accent) in liveries.0.iter().take(PLANES).enumerate() {
        model::spawn_plane(&mut commands, &mut meshes, &mut materials, &mut images, i, *accent);
    }
    camera::spawn_camera(&mut commands, &settings, &mut images, offscreen.map(|o| *o));
}
