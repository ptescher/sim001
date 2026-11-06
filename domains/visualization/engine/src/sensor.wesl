// A small camera's shutter and grain: see sensor.rs.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Sensor {
    read: f32,
    shot: f32,
    chroma: f32,
    frame: u32,
    exposure: f32,
    blur: f32,
    samples: u32,
    motion_scale: vec2<f32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var motion: texture_2d<f32>;
@group(0) @binding(2) var screen_sampler: sampler;
@group(0) @binding(3) var<uniform> sensor: Sensor;

// Three independent uniform numbers from a pixel and a frame (pcg3d, Jarzynski and Olano 2020).
fn pcg3d(seed: vec3<u32>) -> vec3<f32> {
    var v = seed * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return vec3<f32>(v) / 4294967295.0;
}

// Roughly normal, mean 0 and standard deviation 1: the sum of three uniforms.
fn normal(u: vec3<f32>) -> f32 {
    return (u.x + u.y + u.z - 1.5) * 2.0;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.position.xy);
    // The shutter: average the picture along the pixel's motion (in uvs of the picture) while
    // the shutter is open, centred on the frame.
    let mv = textureLoad(motion, vec2<i32>(in.position.xy * sensor.motion_scale), 0).xy;
    let path = mv * sensor.blur;
    var colour = textureLoad(screen, vec2<i32>(pixel), 0);
    let n = i32(sensor.samples);
    if n > 0 && dot(path, path) > 1e-10 {
        var sum = vec3<f32>(0.0);
        // A different offset in each pixel and frame turns the copies of a sharp edge into a
        // smooth smear (with a little noise, which the grain hides).
        let jitter = pcg3d(vec3<u32>(pixel, sensor.frame * 3u + 11u)).x - 0.5;
        for (var i = -n; i <= n; i++) {
            let t = (f32(i) + jitter) / f32(2 * n + 1);
            let uv = clamp(in.uv - path * t, vec2<f32>(0.0), vec2<f32>(1.0));
            sum += textureSampleLevel(screen, screen_sampler, uv, 0.0).rgb;
        }
        colour = vec4<f32>(sum / f32(2 * n + 1), colour.a);
    }

    // The grain: read noise and shot noise, mostly in the brightness.
    let v = max(dot(colour.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0);
    let sigma = sqrt(sensor.read * sensor.read + sensor.shot * v);
    let f = sensor.frame * 3u;
    let luma = normal(pcg3d(vec3<u32>(pixel, f)));
    let tint = vec3<f32>(
        normal(pcg3d(vec3<u32>(pixel, f + 1u))),
        normal(pcg3d(vec3<u32>(pixel, f + 2u))),
        normal(pcg3d(vec3<u32>(pixel.yx, f + 7u))),
    );
    let noise = (1.0 - sensor.chroma) * vec3<f32>(luma) + sensor.chroma * tint;
    return vec4<f32>(max(colour.rgb + sigma * noise, vec3<f32>(0.0)), colour.a);
}
