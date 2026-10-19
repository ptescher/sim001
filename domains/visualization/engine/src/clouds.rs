//! Volumetric cumulus, for the rasterized and the ray-traced view alike: a pass after the
//! lighting that marches each pixel's ray through a layer of cloud, lights the cloud by the sun
//! (with a cheap stand-in for light bouncing around inside it) and the sky, and lays it over
//! the scene in front of whatever the pixel hit. The same cloud field darkens the ground under
//! it: each lit pixel looks up at the sun through the layer and loses the sun's share of its
//! light by how much cloud is in the way.
//!
//! The clouds are a 3D noise, tiled every few kilometres and thinned by a coverage map at a
//! larger scale, and drift with the wind. Neither the ray tracer nor the shadow maps see them,
//! so they do not show in the lake.

use crate::Wind;
use bevy::anti_alias::taa::temporal_anti_alias;
use bevy::asset::{RenderAssetUsages, embedded_asset, load_embedded_asset};
use bevy::core_pipeline::{
    FullscreenShader,
    schedule::{Core3d, Core3dSystems},
};
use bevy::diagnostic::FrameCount;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderStartup, RenderSystems,
    extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin},
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_asset::RenderAssets,
    render_resource::{
        BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState,
        ColorWrites, Extent3d, FragmentState, LoadOp, Operations, PipelineCache, RenderPassColorAttachment,
        RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
        ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp, TextureDimension, TextureFormat,
        TextureSampleType, TextureUsages,
        binding_types::{sampler, texture_2d, texture_3d, uniform_buffer},
    },
    renderer::{RenderContext, RenderDevice, ViewQuery},
    texture::GpuImage,
    view::{ExtractedView, ViewDepthStencilTexture, ViewTarget, prepare_view_targets},
};

/// Put it on a camera to draw clouds in what it sees. `update_clouds` keeps the camera's
/// matrices, the wind's drift and the frame number current.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType)]
#[extract_app(RenderApp)]
pub struct Clouds {
    pub world_from_clip: Mat4,
    pub camera: Vec3,
    /// Flat cloud bases at this height (m).
    pub base: f32,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The tallest tops (m).
    pub top: f32,
    /// Sunlight reaching the clouds, as the view's (pre-exposed) illuminance.
    pub sun: Vec3,
    /// How much of the sky has cloud, 0 to 1.
    pub coverage: f32,
    /// Skylight on the clouds' tops, pre-exposed radiance.
    pub sky_top: Vec3,
    /// Extinction of the densest cloud (per metre).
    pub extinction: f32,
    /// Light from the ground under the clouds, pre-exposed radiance.
    pub sky_bottom: Vec3,
    /// Metres across one tile of the cloud shapes.
    pub shape_scale: f32,
    /// Air between the camera and a far cloud, as in `haze::Haze`.
    pub haze_color: Vec3,
    pub haze_density: f32,
    /// How far the wind has carried the clouds (m, east and south).
    pub drift: Vec2,
    pub haze_scale_height: f32,
    /// The share of a lit surface's light that is left in a cloud's full shadow (the skylight).
    pub shadow_floor: f32,
    /// Changes the ray-march jitter every frame, for the anti-aliasing to average out.
    pub frame: u32,
    pub _pad: Vec3,
}

impl Default for Clouds {
    fn default() -> Self {
        let haze = crate::haze::Haze::default();
        Clouds {
            world_from_clip: Mat4::IDENTITY,
            camera: Vec3::ZERO,
            // Fair-weather cumulus over the mountains on a summer afternoon: bases at about the
            // same height, tops a kilometre or so higher, a third of the sky covered.
            base: 1700.0,
            top: 3300.0,
            coverage: 0.5,
            sun_dir: crate::field::sun_dir(),
            sun: Vec3::new(5.6, 5.4, 5.0),
            sky_top: Vec3::new(0.34, 0.42, 0.56),
            sky_bottom: Vec3::new(0.10, 0.11, 0.11),
            extinction: 0.03,
            shape_scale: 3600.0,
            haze_color: haze.color,
            haze_density: haze.density,
            haze_scale_height: haze.scale_height,
            drift: Vec2::ZERO,
            shadow_floor: 0.3,
            frame: 0,
            _pad: Vec3::ZERO,
        }
    }
}

/// How far the clouds have drifted after `t` seconds: downwind, faster than the wind at the
/// ground (it blows harder up there), and never quite still.
pub fn drift(wind: &Wind, t: f32) -> Vec2 {
    let towards = -Vec2::new(wind.from_dir.sin(), -wind.from_dir.cos());
    towards * (wind.speed * 2.5 + 3.0) * t
}

/// Side of the noise volume, in texels.
const NOISE_SIZE: u32 = 128;

fn hash(c: IVec3, seed: u32) -> Vec3 {
    let mut h = (c.x as u32).wrapping_mul(0x8da6_b343) ^ (c.y as u32).wrapping_mul(0xd816_3841) ^ (c.z as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x9E37_79B1);
    let mut next = || {
        h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
    };
    Vec3::new(next(), next(), next())
}

/// Inverted cellular noise over a unit tile with `period` cells each way: 1 on a feature
/// point, falling to 0 a cell away. It wraps at the tile's edges.
pub fn worley(p: Vec3, period: u32) -> f32 {
    let q = p * period as f32;
    let cell = q.floor().as_ivec3();
    let n = period as i32;
    let mut best = f32::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = cell + IVec3::new(dx, dy, dz);
                let feature = c.as_vec3() + hash(c.rem_euclid(IVec3::splat(n)), 7);
                best = best.min(feature.distance_squared(q));
            }
        }
    }
    1.0 - best.sqrt().min(1.0)
}

fn gradient(c: IVec3, period: i32) -> Vec3 {
    (hash(c.rem_euclid(IVec3::splat(period)), 3) * 2.0 - Vec3::ONE).normalize_or(Vec3::X)
}

/// Gradient noise over a unit tile with `period` lattice cells each way, about -1 to 1.
fn perlin(p: Vec3, period: u32) -> f32 {
    let q = p * period as f32;
    let i = q.floor();
    let f = q - i;
    let u = f * f * f * (f * (f * 6.0 - Vec3::splat(15.0)) + Vec3::splat(10.0));
    let i = i.as_ivec3();
    let n = period as i32;
    let corner = |dx: i32, dy: i32, dz: i32| {
        let o = IVec3::new(dx, dy, dz);
        gradient(i + o, n).dot(f - o.as_vec3())
    };
    let x00 = corner(0, 0, 0) + (corner(1, 0, 0) - corner(0, 0, 0)) * u.x;
    let x10 = corner(0, 1, 0) + (corner(1, 1, 0) - corner(0, 1, 0)) * u.x;
    let x01 = corner(0, 0, 1) + (corner(1, 0, 1) - corner(0, 0, 1)) * u.x;
    let x11 = corner(0, 1, 1) + (corner(1, 1, 1) - corner(0, 1, 1)) * u.x;
    let y0 = x00 + (x10 - x00) * u.y;
    let y1 = x01 + (x11 - x01) * u.y;
    (y0 + (y1 - y0) * u.z) * 1.6
}

/// Octaves of gradient noise, each twice as fine, in 0..1. Wraps at the tile's edges.
pub fn perlin_fbm(p: Vec3, period: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += perlin(p, period << o) * amp;
        norm += amp;
        amp *= 0.5;
    }
    (sum / norm * 0.5 + 0.5).clamp(0.0, 1.0)
}

fn worley_fbm(p: Vec3, period: u32) -> f32 {
    worley(p, period) * 0.625 + worley(p, period * 2) * 0.25 + worley(p, period * 4) * 0.125
}

/// The cloud noise, `size`³ RGBA8: billowy Perlin-Worley for the big shapes in red, and cells
/// at three finer scales in green, blue and alpha to break up and erode their edges.
pub fn cloud_noise(size: u32) -> Vec<u8> {
    let n = size as usize;
    let mut data = vec![0u8; n * n * n * 4];
    let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    std::thread::scope(|s| {
        for (z, slab) in data.chunks_mut(n * n * 4).enumerate() {
            s.spawn(move || {
                for y in 0..n {
                    for x in 0..n {
                        let p = (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)) / size as f32;
                        let cells = worley_fbm(p, 4);
                        let perlin = perlin_fbm(p, 4, 4);
                        // Perlin-Worley: the gradient noise, pushed up where the cells are.
                        let pw = cells + (perlin - 0.0) * (1.0 - cells);
                        let i = (y * n + x) * 4;
                        slab[i..i + 4].copy_from_slice(&[
                            unorm(pw * pw.sqrt()),
                            unorm(worley_fbm(p, 4)),
                            unorm(worley_fbm(p, 8)),
                            unorm(worley_fbm(p, 16)),
                        ]);
                    }
                }
            });
        }
    });
    data
}

fn noise_image() -> Image {
    let mut image = Image::new(
        Extent3d { width: NOISE_SIZE, height: NOISE_SIZE, depth_or_array_layers: NOISE_SIZE },
        TextureDimension::D3,
        cloud_noise(NOISE_SIZE),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
struct CloudNoise(Handle<Image>);

pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "clouds.wesl");
        app.add_plugins((
            ExtractComponentPlugin::<Clouds>::default(),
            UniformComponentPlugin::<Clouds>::default(),
            ExtractResourcePlugin::<CloudNoise>::default(),
        ))
        .add_systems(Startup, |mut commands: Commands, mut images: ResMut<Assets<Image>>| {
            commands.insert_resource(CloudNoise(images.add(noise_image())));
        })
        .add_systems(PostUpdate, update_clouds.after(bevy::transform::TransformSystems::Propagate));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<SpecializedRenderPipelines<CloudsPipeline>>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    depth_readable.in_set(RenderSystems::PrepareViews).after(prepare_view_targets).ambiguous_with_all(),
                ),
            )
            // Before the anti-aliasing, which averages the ray march's jitter away.
            .add_systems(Core3d, clouds.in_set(Core3dSystems::EarlyPostProcess).before(temporal_anti_alias));
    }
}

fn update_clouds(
    time: Res<Time>,
    frame: Res<FrameCount>,
    scene: Res<crate::Scene>,
    mut cameras: Query<(&mut Clouds, &Camera, &GlobalTransform)>,
) {
    for (mut c, cam, t) in &mut cameras {
        c.world_from_clip = t.to_matrix() * cam.clip_from_view().inverse();
        c.camera = t.translation();
        c.drift = drift(&scene.wind, time.elapsed_secs());
        c.frame = frame.0;
    }
}

#[derive(Resource)]
struct CloudsPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen: Res<FullscreenShader>, assets: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "clouds_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                sampler(SamplerBindingType::Filtering),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<Clouds>(true),
            ),
        ),
    );
    commands.insert_resource(CloudsPipeline {
        layout,
        sampler: device.create_sampler(&SamplerDescriptor::default()),
        fullscreen: fullscreen.clone(),
        shader: load_embedded_asset!(assets.as_ref(), "clouds.wesl"),
    });
}

impl SpecializedRenderPipeline for CloudsPipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("clouds_pipeline".into()),
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
struct CloudsPipelineId(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<CloudsPipeline>>,
    pipeline: Res<CloudsPipeline>,
    views: Query<(Entity, &ExtractedView), With<Clouds>>,
) {
    for (entity, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(entity).insert(CloudsPipelineId(id));
    }
}

/// The pass reads the depth buffer.
fn depth_readable(mut cameras: Query<&mut Camera3d, With<Clouds>>) {
    for mut c in &mut cameras {
        c.depth_texture_usages.0 |= TextureUsages::TEXTURE_BINDING.bits();
    }
}

fn clouds(
    view: ViewQuery<(&ViewTarget, &CloudsPipelineId, &ViewDepthStencilTexture, &DynamicUniformIndex<Clouds>)>,
    pipeline: Res<CloudsPipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<Clouds>>,
    noise: Option<Res<CloudNoise>>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, id, depth, index) = view.into_inner();
    let Some(noise) = noise.and_then(|n| images.get(&n.0)) else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let Some(render_pipeline) = cache.get_render_pipeline(id.0) else { return };
    let Some(settings) = uniforms.uniforms().binding() else { return };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        Some("clouds_bind_group"),
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, depth_view, &pipeline.sampler, &noise.texture_view, &noise.sampler, settings)),
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("clouds"),
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

    /// The noise is sampled with repeat addressing, so a cloud never shows a seam: it repeats
    /// exactly one period on, and stays in 0..1.
    #[test]
    fn the_cloud_noise_tiles() {
        for &(x, y, z) in &[(0.1f32, 0.7f32, 0.3f32), (0.95, 0.02, 0.5), (0.5, 0.5, 0.99)] {
            let p = Vec3::new(x, y, z);
            for period in [4u32, 8] {
                let a = worley(p, period);
                let b = worley(p + Vec3::X, period);
                let c = worley(p + Vec3::new(0.0, 1.0, 1.0), period);
                assert!((a - b).abs() < 1e-4 && (a - c).abs() < 1e-4, "worley({p}, {period}) has a seam");
                let a = perlin_fbm(p, period, 4);
                let b = perlin_fbm(p + Vec3::Z, period, 4);
                assert!((a - b).abs() < 1e-4, "perlin({p}, {period}) has a seam");
            }
        }
        let v = cloud_noise(16);
        assert_eq!(v.len(), 16 * 16 * 16 * 4);
        let spread = v.iter().max().unwrap() - v.iter().min().unwrap();
        assert!(spread > 128, "the noise uses its range ({spread} of 255)");
    }

    /// Clouds blow with the wind: from the north means they move south (+Z), faster when it is
    /// windier, and they never stand still.
    #[test]
    fn the_clouds_drift_downwind() {
        let calm = drift(&Wind { from_dir: 0.0, speed: 0.0, ..default() }, 10.0);
        let north = drift(&Wind { from_dir: 0.0, speed: 8.0, ..default() }, 10.0);
        let west = drift(&Wind { from_dir: -std::f32::consts::FRAC_PI_2, speed: 8.0, ..default() }, 10.0);
        assert!(calm.length() > 0.0, "a calm day's clouds still drift");
        assert!(north.y > 0.0 && north.x.abs() < 1e-3, "from the north: south, {north}");
        assert!(west.x > 0.0 && west.y.abs() < 1e-3, "from the west: east, {west}");
        assert!(north.length() > calm.length());
    }

    /// Every camera that draws the scene sees the same sky, clouds and all.
    #[test]
    fn every_scene_camera_has_clouds() {
        let mut world = World::new();
        let e = world.spawn(crate::camera::scene_look()).id();
        assert!(world.get::<Clouds>(e).is_some());
    }
}
