// Aerial perspective: see haze.rs.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Haze {
    world_from_clip: mat4x4<f32>,
    camera: vec3<f32>,
    density: f32,
    color: vec3<f32>,
    scale_height: f32,
    sun_dir: vec3<f32>,
    sun_glow: f32,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_2d<f32>;
@group(0) @binding(2) var screen_sampler: sampler;
@group(0) @binding(3) var<uniform> haze: Haze;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let colour = textureSample(screen, screen_sampler, in.uv);
    let size = vec2<f32>(textureDimensions(depth));
    let d = textureLoad(depth, vec2<i32>(in.uv * size), 0).x;
    // Reversed infinite depth: 0 is the sky, which the atmosphere already drew.
    if d <= 0.0 {
        return colour;
    }
    let ndc = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, d, 1.0);
    let w = haze.world_from_clip * ndc;
    let p = w.xyz / w.w;
    let ray = p - haze.camera;
    let dist = length(ray);
    // Optical depth through air that thins with height: the exact integral of an exponential
    // along a straight ray.
    let h0 = haze.camera.y / haze.scale_height;
    let dh = ray.y / haze.scale_height;
    var thin = exp(-h0);
    if abs(dh) > 1e-3 {
        thin = exp(-h0) * (1.0 - exp(-dh)) / dh;
    }
    let tau = haze.density * dist * thin;
    let seen = exp(-tau);
    // Brighter towards the sun (forward scattering), never darker than the plain haze.
    let cos_sun = max(dot(ray / dist, haze.sun_dir), 0.0);
    let glow = 1.0 + haze.sun_glow * pow(cos_sun, 8.0);
    let inscatter = haze.color * glow;
    return vec4<f32>(colour.rgb * seen + inscatter * (1.0 - seen), colour.a);
}
