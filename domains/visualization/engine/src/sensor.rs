//! A small camera's sensor, after the lens: the shutter and the noise.
//!
//! - **The shutter** is open for part of each frame, so what moves across the picture smears
//!   along its path: the trees as the drone banks, the ground rushing under it. A still scene
//!   stays sharp. The smear follows the motion vectors; they are read where the main pass drew
//!   them, which with the ray tracer's upscaler is a corner of the textures. (Bevy's own motion
//!   blur reads them as if they filled the picture.)
//! - **The grain** moves every frame: faint in the sky and in sunlit grass, plain in the
//!   shadows. It is two noises added: the read noise of the sensor's electronics, the same at
//!   every brightness, and the shot noise of the light itself, which grows with the square root
//!   of the light. Relative to the picture, both are strongest in the dark.
//!
//! One pass after the tone mapping does both, to the picture of a camera that has a [`Sensor`].

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::{
    FullscreenShader,
    prepass::{MotionVectorPrepass, ViewPrepassTextures},
    schedule::{Core3d, Core3dSystems},
    tonemapping::tonemapping,
};
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderStartup, RenderSystems,
    extract_component::{
        ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
    },
    render_resource::{
        BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState,
        ColorWrites, FilterMode, FragmentState, LoadOp, Operations, PipelineCache, RenderPassColorAttachment,
        RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
        ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp, TextureFormat, TextureSampleType,
        binding_types::{sampler, texture_2d, uniform_buffer},
    },
    renderer::{RenderContext, RenderDevice, ViewQuery},
    view::{ExtractedView, ViewTarget},
};

/// Put it on a camera to give it a small camera's shutter and grain.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, ShaderType)]
#[extract_app(RenderApp)]
#[require(MotionVectorPrepass)]
pub struct Sensor {
    /// The read noise: its standard deviation, in the picture's (linear, 0 to 1) units.
    pub read: f32,
    /// The shot noise: its variance per unit of light.
    pub shot: f32,
    /// How much of the noise is colour, not brightness (0 to 1).
    pub chroma: f32,
    /// Counts the frames, so the grain moves.
    pub frame: u32,
    /// How long the shutter is open (s).
    pub exposure: f32,
    /// The smear this frame, as a part of the pixel's motion since the last frame (0 to 1): the
    /// exposure over the frame's time.
    pub blur: f32,
    /// Samples of the smear on each side of a pixel.
    pub samples: u32,
    /// Where the motion vectors are: the main pass's size over the picture's.
    pub motion_scale: Vec2,
}

impl Sensor {
    /// A small FPV camera in daylight.
    pub fn fpv() -> Sensor {
        Sensor {
            read: 0.002,
            shot: 1.4e-4,
            chroma: 0.25,
            frame: 0,
            exposure: 1.0 / 150.0,
            blur: 0.0,
            samples: 3,
            motion_scale: Vec2::ONE,
        }
    }

    /// The noise's standard deviation where the picture is `v` (linear, 0 to 1).
    pub fn sigma(&self, v: f32) -> f32 {
        (self.read * self.read + self.shot * v.max(0.0)).sqrt()
    }

    /// The smear of a frame that took `dt` seconds: what the shutter saw of the motion, the same
    /// per second whatever the frame rate.
    pub fn blur(&self, dt: f32) -> f32 {
        if dt > 0.0 { (self.exposure / dt).min(1.0) } else { 0.0 }
    }

    /// Where the motion vectors of a picture `full` big are, when the main pass drew `render`.
    pub fn motion_scale(full: UVec2, render: Option<UVec2>) -> Vec2 {
        render.map_or(Vec2::ONE, |r| r.as_vec2() / full.max(UVec2::ONE).as_vec2())
    }
}

pub struct SensorPlugin;

impl Plugin for SensorPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sensor.wesl");
        app.add_plugins((ExtractComponentPlugin::<Sensor>::default(), UniformComponentPlugin::<Sensor>::default()))
            .add_systems(PostUpdate, next_frame);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<SpecializedRenderPipelines<SensorPipeline>>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            // After the tone mapping: the smear and the noise are in the picture, not the light.
            .add_systems(Core3d, sensor.after(tonemapping).in_set(Core3dSystems::PostProcess));
    }
}

/// Moves the grain on, and finds the motion vectors for this frame's main pass.
fn next_frame(time: Res<Time>, mut cameras: Query<(&mut Sensor, &Camera, Option<&crate::upscale::TemporalUpscale>)>) {
    for (mut s, cam, up) in &mut cameras {
        s.frame = s.frame.wrapping_add(1);
        s.blur = s.blur(time.delta_secs());
        let Some(full) = cam.physical_viewport_size() else { continue };
        let render = up.map(|u| crate::upscale::render_resolution(full, u.scale)).filter(|r| *r != full);
        s.motion_scale = Sensor::motion_scale(full, render);
    }
}

#[derive(Resource)]
struct SensorPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "sensor_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<Sensor>(true),
            ),
        ),
    );
    commands.insert_resource(SensorPipeline {
        layout,
        sampler: device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
        fullscreen: fullscreen.clone(),
        shader: load_embedded_asset!(assets.as_ref(), "sensor.wesl"),
    });
}

impl SpecializedRenderPipeline for SensorPipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("sensor_pipeline".into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

#[derive(Component)]
struct SensorPipelineId(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<SensorPipeline>>,
    pipeline: Res<SensorPipeline>,
    views: Query<(Entity, &ExtractedView), With<Sensor>>,
) {
    for (entity, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(entity).insert(SensorPipelineId(id));
    }
}

fn sensor(
    view: ViewQuery<(&ViewTarget, &SensorPipelineId, &ViewPrepassTextures, &DynamicUniformIndex<Sensor>)>,
    pipeline: Res<SensorPipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<Sensor>>,
    mut ctx: RenderContext,
) {
    let (target, id, prepass, index) = view.into_inner();
    let Some(motion) = prepass.motion_vectors_view() else { return };
    let Some(render_pipeline) = cache.get_render_pipeline(id.0) else { return };
    let Some(settings) = uniforms.uniforms().binding() else { return };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        Some("sensor_bind_group"),
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, motion, &pipeline.sampler, settings)),
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("sensor"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Clear(Default::default()), store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[index.index()]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MID_GREY: f32 = 0.18;

    fn relative(v: f32) -> f32 {
        Sensor::fpv().sigma(v) / v
    }

    /// In sunlit grass the grain is there if you look for it, not a snowstorm.
    #[test]
    fn the_grain_is_faint_in_the_light() {
        let r = relative(MID_GREY);
        assert!((0.015..=0.05).contains(&r), "{:.1}% at mid grey", r * 100.0);
        assert!(relative(0.8) < r, "brighter is cleaner: {:.1}% at 0.8", relative(0.8) * 100.0);
    }

    /// Under the trees and in the shade of a building the picture gets noisy.
    #[test]
    fn the_grain_is_plain_in_the_shadows() {
        let shade = relative(0.02);
        assert!(shade > 3.0 * relative(MID_GREY), "{:.1}% in the shade", shade * 100.0);
        assert!(shade < 0.4, "the shade is still a picture ({:.0}%)", shade * 100.0);
    }

    /// More light is more noise, though less of the picture: the shot noise.
    #[test]
    fn the_noise_grows_with_the_light() {
        let s = Sensor::fpv();
        assert!(s.sigma(0.8) > 1.5 * s.sigma(0.02));
        assert!(s.chroma > 0.0 && s.chroma < 0.5, "mostly brightness, a little colour");
    }

    /// The shutter is open for the same time whatever the frame rate: a slow frame sees a smaller
    /// part of the motion since the last one, and never more than all of it.
    #[test]
    fn the_smear_is_the_same_at_any_frame_rate() {
        let s = Sensor { exposure: 1.0 / 120.0, ..Sensor::fpv() };
        assert!((s.blur(1.0 / 60.0) - 0.5).abs() < 1e-6);
        assert!((s.blur(1.0 / 10.0) - 1.0 / 12.0).abs() < 1e-6);
        assert_eq!(s.blur(1.0 / 240.0), 1.0);
        assert_eq!(s.blur(0.0), 0.0);
    }

    /// With the upscaler, the main pass draws a smaller picture into a corner of the textures:
    /// the smear reads each pixel's motion there, not off to the side.
    #[test]
    fn the_smear_reads_the_motion_where_the_main_pass_drew_it() {
        let full = UVec2::new(3200, 1800);
        assert_eq!(Sensor::motion_scale(full, Some(UVec2::new(1600, 900))), Vec2::splat(0.5));
        assert_eq!(Sensor::motion_scale(full, None), Vec2::ONE);
    }
}
