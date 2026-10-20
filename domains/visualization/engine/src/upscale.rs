//! Temporal upscaling for the main camera. The ray tracer is the cost of a frame and scales with
//! the number of pixels, so the camera draws a smaller picture and the full-size one is rebuilt
//! over time: each frame the camera is shaken by a different fraction of a pixel, so each small
//! frame samples the scene at different places, and a resolve pass blends them into a history
//! that is the size of the window (reprojected by the motion vectors, and kept honest by
//! clamping it to what the current frame sees nearby). What is added up from a few frames is
//! sharper than any one of them stretched.
//!
//! It takes the place of Bevy's temporal anti-aliasing on the camera that has it (the resolve
//! does that job too). The main pass is told to draw into a corner of the full-size target with
//! `MainPassResolutionOverride`, the way Bevy's DLSS does; every pass between the lighting and
//! the resolve works at that size, and everything after it at the full size.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::{
    FullscreenShader,
    prepass::{DepthPrepass, MotionVectorPrepass, ViewPrepassTextures},
    schedule::{Core3d, Core3dSystems},
};
use bevy::diagnostic::FrameCount;
use bevy::image::ToExtents;
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderStartup, RenderSystems,
    camera::{ExtractedCamera, MipBias, TemporalJitter},
    extract_component::{ExtractComponent, ExtractComponentPlugin},
    render_resource::{
        BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState,
        ColorWrites, FilterMode, FragmentState, Operations, PipelineCache, RenderPassColorAttachment,
        RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
        ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, TextureDescriptor, TextureDimension,
        TextureFormat, TextureSampleType, TextureUsages, UniformBuffer,
        binding_types::{sampler, texture_2d, texture_depth_2d, uniform_buffer},
    },
    renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
    texture::{CachedTexture, TextureCache},
    view::{ExtractedView, Msaa, ViewTarget, prepare_view_targets},
};

/// Put it on a camera to draw its main pass at `scale` times the camera's size, and rebuild the
/// full-size picture over time.
#[derive(Component, ExtractComponent, Clone, Copy, Debug)]
#[extract_app(RenderApp)]
#[require(TemporalJitter, MipBias, DepthPrepass, MotionVectorPrepass)]
pub struct TemporalUpscale {
    /// Each side of the main pass as a fraction of the camera's, 0 to 1.
    pub scale: f32,
    /// Forget the history: the picture it holds is not the one being drawn (a new size).
    pub reset: bool,
}

impl Default for TemporalUpscale {
    fn default() -> Self {
        TemporalUpscale { scale: 1.0, reset: true }
    }
}

/// How many frames the camera's shake takes to come round again.
pub const JITTER_FRAMES: u32 = 16;

fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0, 0.0);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// Where in the pixel (-0.5 to 0.5 each way, in pixels of the main pass) the camera looks on
/// this frame: a low-discrepancy sequence that covers the pixel evenly after any number of frames.
pub fn jitter(frame: u32) -> Vec2 {
    let i = frame % JITTER_FRAMES + 1;
    Vec2::new(halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

/// The main pass's size for a camera `full` big, at `scale`: never bigger than the camera, and
/// never so small that there is too little to work with (64 pixels at the shorter side, unless
/// the camera itself is smaller).
pub fn render_resolution(full: UVec2, scale: f32) -> UVec2 {
    let least = 64.0 / full.min_element().max(1) as f32;
    let scale = scale.max(least);
    (full.as_vec2() * scale).round().as_uvec2().max(UVec2::ONE).min(full)
}

/// How much finer to sample textures when the picture is rendered smaller: a mip level for each
/// halving, so that the frames add up to a picture as sharp as a full-size one.
pub fn mip_bias(full: UVec2, render: UVec2) -> f32 {
    (render.x.max(1) as f32 / full.x.max(1) as f32).log2()
}

pub struct TemporalUpscalePlugin;

impl Plugin for TemporalUpscalePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "upscale.wesl");
        app.add_plugins(ExtractComponentPlugin::<TemporalUpscale>::default()).add_systems(Update, shake);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<SpecializedRenderPipelines<UpscalePipeline>>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                (
                    override_resolution.in_set(RenderSystems::PrepareViews).before(prepare_view_targets),
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_history.in_set(RenderSystems::PrepareResources),
                ),
            )
            // After the passes that work at the main pass's size, before the ones at full size.
            .add_systems(Core3d, temporal_upscale.in_set(Core3dSystems::EarlyPostProcess).after(crate::clouds::CloudsSet));
    }
}

/// Moves the camera by this frame's fraction of a pixel, and tells the textures how much finer to be.
fn shake(
    frame: Res<FrameCount>,
    mut cameras: Query<(&mut TemporalUpscale, &Camera, &mut TemporalJitter, &mut MipBias)>,
    mut last: Local<UVec2>,
) {
    for (mut up, camera, mut jit, mut bias) in &mut cameras {
        jit.offset = jitter(frame.0);
        let Some(full) = camera.physical_viewport_size() else { continue };
        bias.0 = mip_bias(full, render_resolution(full, up.scale));
        up.reset = full != *last;
        *last = full;
    }
}

/// In the render world: the main pass draws into a corner of the target that is `scale` times as big.
fn override_resolution(mut commands: Commands, cameras: Query<(Entity, &ExtractedCamera, &TemporalUpscale)>) {
    for (entity, camera, up) in &cameras {
        let Some(full) = camera.physical_viewport_size else { continue };
        let render = render_resolution(full, up.scale);
        if render == full {
            commands.entity(entity).remove::<MainPassResolutionOverride>();
        } else {
            commands.entity(entity).insert(MainPassResolutionOverride(render));
        }
    }
}

#[derive(Component)]
struct UpscaleHistory {
    write: CachedTexture,
    read: CachedTexture,
}

fn prepare_history(
    mut commands: Commands,
    mut textures: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    frame: Res<FrameCount>,
    cameras: Query<(Entity, &ExtractedView, &ExtractedCamera), With<TemporalUpscale>>,
) {
    for (entity, view, camera) in &cameras {
        let Some(size) = camera.physical_target_size else { continue };
        let mut descriptor = TextureDescriptor {
            label: Some("upscale_history_1"),
            size: size.to_extents(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: view.target_format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let one = textures.get(&device, descriptor.clone());
        descriptor.label = Some("upscale_history_2");
        let two = textures.get(&device, descriptor);
        let history = if frame.0.is_multiple_of(2) { UpscaleHistory { write: one, read: two } } else { UpscaleHistory { write: two, read: one } };
        commands.entity(entity).insert(history);
    }
}

#[derive(ShaderType)]
struct Uniforms {
    render_size: Vec2,
    full_size: Vec2,
    jitter: Vec2,
    reset: u32,
    _pad: u32,
    /// Jitter sign, filter width, the blend of a changed pixel, the history's clip width.
    tune: Vec4,
}

/// SIM001_TAAU="sign,sigma,blend,gamma" overrides the resolve's constants, for experiments.
fn tune() -> Vec4 {
    static TUNE: std::sync::OnceLock<Vec4> = std::sync::OnceLock::new();
    *TUNE.get_or_init(|| {
        let mut v = [1.0, 0.25, 0.1, 2.0];
        if let Ok(s) = std::env::var("SIM001_TAAU") {
            for (slot, part) in v.iter_mut().zip(s.split(',')) {
                if let Ok(x) = part.parse() {
                    *slot = x;
                }
            }
        }
        Vec4::from_array(v)
    })
}

#[derive(Resource)]
struct UpscalePipeline {
    layout: BindGroupLayoutDescriptor,
    linear: Sampler,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen: Res<FullscreenShader>, assets: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "upscale_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                // The main pass's picture, in a corner of a full-size texture.
                texture_2d(TextureSampleType::Float { filterable: false }),
                // The history, full size.
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_depth_2d(),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<Uniforms>(false),
            ),
        ),
    );
    let sampler = |filter| device.create_sampler(&SamplerDescriptor { mag_filter: filter, min_filter: filter, ..default() });
    commands.insert_resource(UpscalePipeline {
        layout,
        linear: sampler(FilterMode::Linear),
        fullscreen: fullscreen.clone(),
        shader: load_embedded_asset!(assets.as_ref(), "upscale.wesl"),
    });
}

impl SpecializedRenderPipeline for UpscalePipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        let target = Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL });
        RenderPipelineDescriptor {
            label: Some("upscale_pipeline".into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen.to_vertex_state(),
            // The picture, and the same colour into the history (with how sure we are of it in alpha).
            fragment: Some(FragmentState { shader: self.shader.clone(), targets: vec![target.clone(), target], ..default() }),
            ..default()
        }
    }
}

#[derive(Component)]
struct UpscalePipelineId(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<UpscalePipeline>>,
    pipeline: Res<UpscalePipeline>,
    views: Query<(Entity, &ExtractedView), With<TemporalUpscale>>,
) {
    for (entity, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(entity).insert(UpscalePipelineId(id));
    }
}

fn temporal_upscale(
    view: ViewQuery<(
        &ExtractedCamera,
        &ViewTarget,
        &UpscaleHistory,
        &ViewPrepassTextures,
        &UpscalePipelineId,
        &TemporalJitter,
        &TemporalUpscale,
        Option<&MainPassResolutionOverride>,
        &Msaa,
    )>,
    pipeline: Res<UpscalePipeline>,
    cache: Res<PipelineCache>,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) {
    let (camera, target, history, prepass, id, jit, up, resolution, msaa) = view.into_inner();
    if *msaa != Msaa::Off {
        return;
    }
    let (Some(render_pipeline), Some(motion), Some(depth)) =
        (cache.get_render_pipeline(id.0), &prepass.motion_vectors, &prepass.depth)
    else {
        return;
    };
    let Some(full) = camera.physical_viewport_size else { return };
    let render = resolution.map_or(full, |r| r.0);

    let mut uniforms = UniformBuffer::from(Uniforms {
        render_size: render.as_vec2(),
        full_size: full.as_vec2(),
        jitter: jit.offset,
        reset: up.reset as u32,
        _pad: 0,
        tune: tune(),
    });
    uniforms.write_buffer(ctx.render_device(), &queue);
    let Some(uniforms) = uniforms.binding() else { return };

    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        "upscale_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            post.source,
            &history.read.default_view,
            &motion.texture.default_view,
            &depth.texture.default_view,
            &pipeline.linear,
            uniforms,
        )),
    );
    let attachment = |view| Some(RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: Operations::default() });
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("temporal_upscale"),
        color_attachments: &[attachment(post.destination), attachment(&history.write.default_view)],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The camera is shaken by a different sub-pixel amount each frame, in a pattern that
    /// covers the pixel evenly and comes round again: that is what lets a few small frames
    /// add up to one big one.
    #[test]
    fn the_jitter_covers_the_pixel_evenly_and_repeats() {
        let all: Vec<Vec2> = (0..JITTER_FRAMES).map(jitter).collect();
        for j in &all {
            assert!(j.x.abs() <= 0.5 && j.y.abs() <= 0.5, "{j} is outside the pixel");
        }
        let mean = all.iter().sum::<Vec2>() / all.len() as f32;
        assert!(mean.length() < 0.05, "centred on the pixel, not {mean}");
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(a.distance(*b) > 0.05, "{a} and {b} are the same spot");
            }
        }
        assert_eq!(jitter(3), jitter(3 + JITTER_FRAMES));
        assert_eq!(jitter(JITTER_FRAMES - 1), jitter(2 * JITTER_FRAMES - 1));
    }

    /// The internal picture is the window's size times the scale, never bigger than the window
    /// and never so small that the ray tracer has nothing to work with.
    #[test]
    fn the_internal_resolution_follows_the_scale() {
        let full = UVec2::new(3200, 1800);
        assert_eq!(render_resolution(full, 0.5), UVec2::new(1600, 900));
        assert_eq!(render_resolution(full, 1.0), full);
        assert_eq!(render_resolution(full, 2.0), full, "no bigger than the window");
        let tiny = render_resolution(full, 0.001);
        assert!(tiny.x >= 64 && tiny.y >= 36, "{tiny} is too small");
        assert_eq!(render_resolution(UVec2::new(50, 50), 0.5), UVec2::new(50, 50), "a tiny window is not shrunk below its own size");
    }

    /// Textures are sampled finer when the picture is rendered smaller, by one mip level for
    /// each halving, so that what the accumulated frames add up to is sharp.
    #[test]
    fn textures_are_sampled_finer_when_rendered_smaller() {
        let full = UVec2::new(3200, 1800);
        assert_eq!(mip_bias(full, full), 0.0);
        assert!((mip_bias(full, UVec2::new(1600, 900)) + 1.0).abs() < 1e-5);
        assert!((mip_bias(full, UVec2::new(800, 450)) + 2.0).abs() < 1e-5);
    }
}
