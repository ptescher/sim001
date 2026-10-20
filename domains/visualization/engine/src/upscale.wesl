// Temporal upscaling: see upscale.rs. Modelled on Bevy's TAA (Playdead's reprojection, Karis'
// reversible tonemap for the blend, Salvi's variance clipping), with the current frame read at
// the main pass's size and the history kept at the full size.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Uniforms {
    render_size: vec2<f32>,
    full_size: vec2<f32>,
    // Where the camera was shaken to this frame, in pixels of the main pass.
    jitter: vec2<f32>,
    reset: u32,
    tune: vec4<f32>,
}

@group(0) @binding(0) var current: texture_2d<f32>;
@group(0) @binding(1) var history: texture_2d<f32>;
@group(0) @binding(2) var motion_vectors: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var linear_sampler: sampler;
@group(0) @binding(5) var<uniform> u: Uniforms;

struct Output {
    @location(0) view_target: vec4<f32>,
    @location(1) history: vec4<f32>,
}

// The share of the current frame in the blend: as little as MIN where the picture holds still
// (the history is the average of many frames), at most tune.z where it has just changed.
const MIN_BLEND: f32 = 0.02;

fn max3(x: vec3<f32>) -> f32 { return max(x.r, max(x.g, x.b)); }
fn tonemap(c: vec3<f32>) -> vec3<f32> { return c / (max3(c) + 1.0); }
fn untonemap(c: vec3<f32>) -> vec3<f32> { return c / (1.0 - max3(c)); }

fn rgb_to_ycocg(rgb: vec3<f32>) -> vec3<f32> {
    return vec3(rgb.r / 4.0 + rgb.g / 2.0 + rgb.b / 4.0, rgb.r / 2.0 - rgb.b / 2.0, -rgb.r / 4.0 + rgb.g / 2.0 - rgb.b / 4.0);
}
fn ycocg_to_rgb(c: vec3<f32>) -> vec3<f32> {
    return saturate(vec3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z));
}

fn clip_towards_aabb_center(history_color: vec3<f32>, aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> vec3<f32> {
    let p = 0.5 * (aabb_max + aabb_min);
    let e = 0.5 * (aabb_max - aabb_min) + 0.00000001;
    let v = history_color - p;
    let m = max3(abs(v / e));
    if m > 1.0 {
        return p + v / m;
    }
    return history_color;
}

// A colour that is not a number (or is absurdly bright) becomes black. One such pixel left in
// the history would be spread to its neighbours by the resampling (NaN times zero is NaN) and
// grow until the picture was black.
fn finite(c: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0), clamp(c, vec3<f32>(0.0), vec3<f32>(60000.0)), c == c);
}

fn load_current(p: vec2<i32>) -> vec3<f32> {
    let c = clamp(p, vec2<i32>(0), vec2<i32>(u.render_size) - vec2<i32>(1));
    return rgb_to_ycocg(tonemap(finite(textureLoad(current, c, 0).rgb)));
}

fn sample_history(uv: vec2<f32>) -> vec3<f32> {
    return finite(textureSampleLevel(history, linear_sampler, uv, 0.0).rgb);
}

@fragment
fn upscale(in: FullscreenVertexOutput) -> Output {
    let uv = in.position.xy / u.full_size;
    // This output pixel's place among the main pass's pixels. The camera was shaken by `jitter`,
    // so main-pass pixel i looked at the scene where pixel i + 0.5 + jitter would have.
    let q = uv * u.render_size;
    let jitter = u.jitter * u.tune.x;
    let centre = vec2<i32>(floor(q - jitter));

    // The current frame around here, filtered to this output pixel's place: a Gaussian over the
    // 3x3 nearest main-pass samples, and the spread of those samples to judge the history by.
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var closest = centre;
    var closest_depth = -1.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let i = centre + vec2<i32>(dx, dy);
            let c = load_current(i);
            let at = vec2<f32>(i) + 0.5 + jitter - q;
            let w = exp(-dot(at, at) / (2.0 * u.tune.y * u.tune.y));
            sum += c * w;
            weight += w;
            m1 += c;
            m2 += c * c;
            // The motion of the nearest surface around here, so that an edge moves as its front does.
            let ic = clamp(i, vec2<i32>(0), vec2<i32>(u.render_size) - vec2<i32>(1));
            let d = textureLoad(depth, ic, 0);
            if d > closest_depth {
                closest_depth = d;
                closest = ic;
            }
        }
    }
    let filtered = sum / max(weight, 1e-5);
    let alpha = textureLoad(current, clamp(centre, vec2<i32>(0), vec2<i32>(u.render_size) - vec2<i32>(1)), 0).a;
    var colour = filtered;
    var confidence = 1.0 / MIN_BLEND;

    if u.reset == 0u {
        let mean = m1 / 9.0;
        let deviation = sqrt(max(m2 / 9.0 - mean * mean, vec3<f32>(0.0)));
        let motion = textureLoad(motion_vectors, closest, 0).rg;

        // Where this was a frame ago, read from the full-size history with Catmull-Rom (the five
        // taps that count).
        let history_uv = uv - motion;
        let texel = 1.0 / u.full_size;
        let p = history_uv * u.full_size;
        let tc = floor(p - 0.5) + 0.5;
        let f = p - tc;
        let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
        let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
        let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
        let w3 = f * f * (-0.5 + 0.5 * f);
        let w12 = w1 + w2;
        let p0 = (tc - 1.0) * texel;
        let p3 = (tc + 2.0) * texel;
        let p12 = (tc + w2 / w12) * texel;
        var past = sample_history(vec2(p12.x, p0.y)) * w12.x * w0.y;
        past += sample_history(vec2(p0.x, p12.y)) * w0.x * w12.y;
        past += sample_history(vec2(p12.x, p12.y)) * w12.x * w12.y;
        past += sample_history(vec2(p3.x, p12.y)) * w3.x * w12.y;
        past += sample_history(vec2(p12.x, p3.y)) * w12.x * w3.y;

        // How long the picture has held still here: the longer, the less of the new frame.
        confidence = textureSampleLevel(history, linear_sampler, uv, 0.0).a;
        let pixels = abs(motion) * u.full_size;
        if pixels.x < 0.01 && pixels.y < 0.01 {
            confidence += 10.0;
        } else {
            confidence = 1.0;
        }
        // The history may stray far from the neighbourhood only where nothing moves (it is
        // averaging noise there); where things change, it is held to what is seen nearby, which
        // is what stops a moving thing leaving a trail.
        let still = saturate((confidence - 1.0) / 6.0);
        let gamma = mix(1.0, u.tune.w, still);
        let before = rgb_to_ycocg(past);
        let after = clip_towards_aabb_center(before, mean - gamma * deviation, mean + gamma * deviation);
        // How far the history had to be pulled back, against how much the neighbourhood varies
        // (the noise): the history of a noisy patch is often a little outside the box, and that
        // is no reason to forget it, but a history many times further out than the noise was
        // a different picture.
        let noise = length(deviation * vec3(2.0, 1.0, 1.0)) + 0.02;
        let pulled = length((before - after) * vec3(2.0, 1.0, 1.0));
        past = ycocg_to_rgb(after);
        // `filtered` is in YCoCg too: put both in RGB to blend.
        let now = ycocg_to_rgb(filtered);

        var factor = clamp(1.0 / confidence, MIN_BLEND, u.tune.z);
        factor = min(factor + saturate(pulled / noise - 1.0) * 0.4, 1.0);
        // Where the history and the new frame disagree by more than noise could (what a spinning
        // blade uncovered, which the neighbourhood is too wide to catch), trust the new frame.
        let disagreement = length((rgb_to_ycocg(past) - filtered) * vec3(2.0, 1.0, 1.0));
        factor = max(factor, saturate((disagreement - 0.1 - 3.0 * noise) * 4.0) * 0.6);
        // Nothing to go on from outside the picture.
        if any(saturate(history_uv) != history_uv) {
            factor = 1.0;
            confidence = 1.0;
        }
        colour = rgb_to_ycocg(mix(past, now, factor));
    }

    let rgb = finite(ycocg_to_rgb(colour));
    var out: Output;
    out.history = vec4(rgb, confidence);
    out.view_target = vec4(untonemap(rgb), alpha);
    return out;
}
