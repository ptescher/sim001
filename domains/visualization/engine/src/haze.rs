//! Aerial perspective for the ray-traced view: the air between the camera and a far ridge
//! scatters skylight into the line of sight and dims what is behind it, so distant mountains
//! fade into a pale blue. Bevy's `DistanceFog` does this in the rasterized view, but Solari does
//! not apply it, so this is a pass of its own after the lighting: it reads the depth, works out
//! how much air each pixel looks through (thinner higher up), and blends towards the haze.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::{
    FullscreenShader,
    schedule::{Core3d, Core3dSystems},
};
use bevy::camera::MainPassResolutionOverride;
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderStartup, RenderSystems,
    extract_component::{ComponentUniforms, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin},
    render_resource::{
        BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState,
        ColorWrites, FragmentState, LoadOp, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
        RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType,
        SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp, TextureFormat, TextureSampleType, TextureUsages,
        binding_types::{sampler, texture_2d, uniform_buffer},
    },
    renderer::{RenderContext, RenderDevice, ViewQuery},
    view::{ExtractedView, ViewDepthStencilTexture, ViewTarget, prepare_view_targets},
};

/// Put it on a camera to haze what it sees. `update_haze` keeps the camera's matrices current.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType)]
#[extract_app(RenderApp)]
pub struct Haze {
    pub world_from_clip: Mat4,
    pub camera: Vec3,
    /// Extinction at sea level of the scene (per metre): visibility is about 3.9 / density.
    pub density: f32,
    /// Skylight scattered towards the camera, as the view's (pre-exposed) colour.
    pub color: Vec3,
    /// The air thins by e every this many metres up.
    pub scale_height: f32,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// How much brighter the haze is looking towards the sun.
    pub sun_glow: f32,
}

impl Default for Haze {
    fn default() -> Self {
        Haze {
            world_from_clip: Mat4::IDENTITY,
            camera: Vec3::ZERO,
            // About 25 km of visibility: a clear summer day in the mountains, with a little haze.
            density: 3.9 / 25_000.0,
            color: Vec3::new(0.30, 0.40, 0.56),
            scale_height: 1800.0,
            sun_dir: crate::field::sun_dir(),
            sun_glow: 0.6,
        }
    }
}

pub struct HazePlugin;

impl Plugin for HazePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "haze.wesl");
        app.add_plugins((ExtractComponentPlugin::<Haze>::default(), UniformComponentPlugin::<Haze>::default()))
            .add_systems(PostUpdate, update_haze.after(bevy::transform::TransformSystems::Propagate));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<SpecializedRenderPipelines<HazePipeline>>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    depth_readable.in_set(RenderSystems::PrepareViews).after(prepare_view_targets).ambiguous_with_all(),
                ),
            )
            // Early, with the passes that work at the main pass's size (a temporal upscale comes after).
            .add_systems(Core3d, haze.in_set(Core3dSystems::EarlyPostProcess).before(crate::clouds::CloudsSet));
    }
}

fn update_haze(mut cameras: Query<(&mut Haze, &Camera, &GlobalTransform)>) {
    for (mut h, cam, t) in &mut cameras {
        h.world_from_clip = t.to_matrix() * cam.clip_from_view().inverse();
        h.camera = t.translation();
    }
}

#[derive(Resource)]
struct HazePipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen: Res<FullscreenShader>, assets: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "haze_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<Haze>(true),
            ),
        ),
    );
    commands.insert_resource(HazePipeline {
        layout,
        sampler: device.create_sampler(&SamplerDescriptor::default()),
        fullscreen: fullscreen.clone(),
        shader: load_embedded_asset!(assets.as_ref(), "haze.wesl"),
    });
}

impl SpecializedRenderPipeline for HazePipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("haze_pipeline".into()),
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
struct HazePipelineId(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<HazePipeline>>,
    pipeline: Res<HazePipeline>,
    views: Query<(Entity, &ExtractedView), With<Haze>>,
) {
    for (entity, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(entity).insert(HazePipelineId(id));
    }
}

/// The pass reads the depth buffer.
fn depth_readable(mut cameras: Query<&mut Camera3d, With<Haze>>) {
    for mut c in &mut cameras {
        c.depth_texture_usages.0 |= TextureUsages::TEXTURE_BINDING.bits();
    }
}

fn haze(
    view: ViewQuery<(
        &ViewTarget,
        &HazePipelineId,
        &ViewDepthStencilTexture,
        &bevy::render::extract_component::DynamicUniformIndex<Haze>,
        Option<&MainPassResolutionOverride>,
    )>,
    pipeline: Res<HazePipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<Haze>>,
    mut ctx: RenderContext,
) {
    let (target, id, depth, index, resolution) = view.into_inner();
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let Some(render_pipeline) = cache.get_render_pipeline(id.0) else { return };
    let Some(settings) = uniforms.uniforms().binding() else { return };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        Some("haze_bind_group"),
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, depth_view, &pipeline.sampler, settings)),
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("haze"),
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
    // The main pass may fill only a corner of the textures.
    if let Some(MainPassResolutionOverride(size)) = resolution {
        pass.set_viewport(0.0, 0.0, size.x as f32, size.y as f32, 0.0, 1.0);
    }
    pass.draw(0..3, 0..1);
}
