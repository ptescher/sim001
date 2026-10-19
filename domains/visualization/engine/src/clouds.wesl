// Volumetric cumulus and their shadows: see clouds.rs.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Clouds {
    world_from_clip: mat4x4<f32>,
    camera: vec3<f32>,
    base: f32,
    sun_dir: vec3<f32>,
    top: f32,
    sun: vec3<f32>,
    coverage: f32,
    sky_top: vec3<f32>,
    extinction: f32,
    sky_bottom: vec3<f32>,
    shape_scale: f32,
    haze_color: vec3<f32>,
    haze_density: f32,
    drift: vec2<f32>,
    haze_scale_height: f32,
    shadow_floor: f32,
    frame: u32,
    _pad: vec3<f32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_2d<f32>;
@group(0) @binding(2) var screen_sampler: sampler;
@group(0) @binding(3) var noise: texture_3d<f32>;
@group(0) @binding(4) var noise_sampler: sampler;
@group(0) @binding(5) var<uniform> clouds: Clouds;

const PI: f32 = 3.14159265;
// Primary steps through the layer, and steps towards the sun from each.
const STEPS: i32 = 64;
const MAX_ITERATIONS: i32 = 192;
const LIGHT_STEPS: i32 = 6;
const MULTIPLE_SCATTERING: f32 = 2.5;
// Clouds further than this are lost in the haze anyway.
const MAX_DISTANCE: f32 = 60000.0;

fn remap(v: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    return c + (v - a) / (b - a) * (d - c);
}

// Where the wind has carried the cloud field: positions are looked up upwind.
fn field(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(p.x - clouds.drift.x, p.y, p.z - clouds.drift.y);
}

// How much cloud a column of air has, 0 to 1: the big cells of the noise at a larger scale,
// cut off so that `coverage` of the sky has some.
fn weather(q: vec3<f32>) -> f32 {
    let uv = q.xz / (clouds.shape_scale * 3.3);
    let w = textureSampleLevel(noise, noise_sampler, vec3<f32>(uv, 0.37), 0.0).r;
    return saturate(remap(w, 1.0 - clouds.coverage, 1.0, 0.0, 1.0));
}

// Cumulus in height: a flat base, rising fast, rounded towards a top that is higher where the
// column has more cloud.
fn height_profile(h: f32, cover: f32) -> f32 {
    let top = mix(0.4, 1.0, sqrt(cover));
    return saturate(h / 0.03) * saturate((top - h) / (top * 0.7));
}

// Cloud density, 0 to 1, at a point. `detail` erodes the edges with the finer noise.
fn density(p: vec3<f32>, detail: bool) -> f32 {
    let h = (p.y - clouds.base) / (clouds.top - clouds.base);
    if h <= 0.0 || h >= 1.0 {
        return 0.0;
    }
    let q = field(p);
    let cover = weather(q);
    if cover <= 0.0 {
        return 0.0;
    }
    let n = textureSampleLevel(noise, noise_sampler, q / clouds.shape_scale, 0.0);
    let cells = n.g * 0.625 + n.b * 0.25 + n.a * 0.125;
    var d = remap(n.r, cells - 1.0, 1.0, 0.0, 1.0) * height_profile(h, cover);
    d = saturate(remap(d, 1.0 - cover, 1.0, 0.0, 1.0)) * cover;
    if d <= 0.0 || !detail {
        return d;
    }
    // Wisps at the base, cauliflower at the tops.
    let m = textureSampleLevel(noise, noise_sampler, q / (clouds.shape_scale * 0.1), 0.0);
    let fine = m.g * 0.625 + m.b * 0.25 + m.a * 0.125;
    let erode = mix(fine, 1.0 - fine, saturate(h * 6.0));
    return saturate(remap(d, erode * 0.5, 1.0, 0.0, 1.0));
}

fn hg(cos_t: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(1.0 + g2 - 2.0 * g * cos_t, 1.5));
}

// Strong forward scattering (the silver lining) and a little back towards the sun.
fn phase(cos_t: f32, k: f32) -> f32 {
    return mix(hg(cos_t, 0.8 * k), hg(cos_t, -0.3 * k), 0.3);
}

// Where a ray from `o` along `d` is between the layer's base and top: (enter, leave), or
// enter >= leave if it never is.
fn slab(o: vec3<f32>, d: vec3<f32>) -> vec2<f32> {
    if abs(d.y) < 1e-5 {
        if o.y > clouds.base && o.y < clouds.top {
            return vec2<f32>(0.0, MAX_DISTANCE);
        }
        return vec2<f32>(1.0, 0.0);
    }
    let a = (clouds.base - o.y) / d.y;
    let b = (clouds.top - o.y) / d.y;
    return vec2<f32>(max(min(a, b), 0.0), max(a, b));
}

// Optical depth of cloud towards the sun from `p`, in steps that grow away from it.
fn sun_depth(p: vec3<f32>, detail: bool) -> f32 {
    let span = slab(p, clouds.sun_dir);
    var t = span.x;
    var step = (span.y - span.x) / 63.0;
    var tau = 0.0;
    for (var i = 0; i < LIGHT_STEPS; i++) {
        let s = p + clouds.sun_dir * (t + step * 0.5);
        tau += density(s, detail) * clouds.extinction * step;
        t += step;
        step *= 2.0;
    }
    return tau;
}

// Optical depth of air along a straight ray, as in haze.wesl.
fn air_depth(o: vec3<f32>, ray: vec3<f32>) -> f32 {
    let dist = length(ray);
    let h0 = o.y / clouds.haze_scale_height;
    let dh = ray.y / clouds.haze_scale_height;
    var thin = exp(-h0);
    if abs(dh) > 1e-3 {
        thin = exp(-h0) * (1.0 - exp(-dh)) / dh;
    }
    return clouds.haze_density * dist * thin;
}

fn interleaved_gradient(px: vec2<f32>, frame: u32) -> f32 {
    let p = px + 5.588238 * f32(frame % 64u);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let colour = textureSample(screen, screen_sampler, in.uv);
    let size = vec2<f32>(textureDimensions(depth));
    let px = in.uv * size;
    let d = textureLoad(depth, vec2<i32>(px), 0).x;
    let ndc_xy = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    // Reversed infinite depth: 0 is the sky. A point on the near plane gives the direction.
    let near = clouds.world_from_clip * vec4<f32>(ndc_xy, 1.0, 1.0);
    let dir = normalize(near.xyz / near.w - clouds.camera);
    var scene = colour.rgb;
    var limit = MAX_DISTANCE;
    if d > 0.0 {
        let w = clouds.world_from_clip * vec4<f32>(ndc_xy, d, 1.0);
        let p = w.xyz / w.w;
        limit = length(p - clouds.camera);
        // The ground in a cloud's shadow keeps only its skylight.
        let lit = exp(-sun_depth(p, false) * 0.8);
        scene *= mix(clouds.shadow_floor, 1.0, lit);
    }

    let span = slab(clouds.camera, dir);
    let t0 = span.x;
    let t1 = min(span.y, limit);
    if t0 >= t1 {
        return vec4<f32>(scene, colour.a);
    }
    let cos_t = dot(dir, clouds.sun_dir);
    let jitter = interleaved_gradient(px, clouds.frame);
    // Big steps through clear air until a cloud is found, then short ones inside it: light
    // gets only a few tens of metres into a cloud, so a step longer than that lands in the
    // cloud's own shadow and greys it.
    let coarse = max((t1 - t0) / f32(STEPS), 80.0);
    var t = t0 + jitter * coarse;
    var inside = false;
    var empty = 0;
    var transmittance = 1.0;
    var light = vec3<f32>(0.0);
    var weighted_t = 0.0;
    for (var i = 0; i < MAX_ITERATIONS; i++) {
        if t >= t1 || transmittance < 0.01 {
            break;
        }
        let p = clouds.camera + dir * t;
        if !inside {
            if density(p, false) > 0.0 {
                inside = true;
                empty = 0;
                t = max(t - coarse, t0);
            } else {
                t += coarse;
            }
            continue;
        }
        // Finer near the camera, where a step covers more of the picture.
        let step = clamp(t * 0.004, 12.0, coarse) * (0.75 + 0.5 * jitter);
        let rho = density(p, true);
        if rho <= 0.0 {
            empty++;
            if empty > 6 {
                inside = false;
            }
            t += step;
            continue;
        }
        empty = 0;
        let sigma = rho * clouds.extinction;
        let tau_sun = sun_depth(p, false);
        // Light bounced around inside the cloud: each octave sees less of the shadow and
        // scatters less sharply forward (Wrenninge's approximation).
        var sun = 0.0;
        var a = 1.0;
        var b = 1.0;
        for (var o = 0; o < 4; o++) {
            sun += b * exp(-tau_sun * a) * phase(cos_t, a);
            a *= 0.5;
            b *= 0.7;
        }
        // The octaves above stand in for many bounces; a cloud's albedo is near 1, so the light
        // they lose out of the sides comes back from its neighbours.
        sun *= MULTIPLE_SCATTERING;
        let h = saturate((p.y - clouds.base) / (clouds.top - clouds.base));
        let sky = mix(clouds.sky_bottom, clouds.sky_top, h);
        // Little light reaches the inside of a thick cloud's near edge yet (the dark
        // edges of a sunlit cumulus).
        let powder = 1.0 - exp(-2.0 * sigma * 60.0);
        let source = (clouds.sun * sun * mix(0.6, 1.0, powder) + sky) * sigma;
        // Integrated exactly over the step (Hillaire).
        let step_t = exp(-sigma * step);
        light += transmittance * source * (1.0 - step_t) / sigma;
        weighted_t += transmittance * (1.0 - step_t) * t;
        transmittance *= step_t;
        t += step;
    }
    let opacity = 1.0 - transmittance;
    if opacity <= 0.0 {
        return vec4<f32>(scene, colour.a);
    }
    // Far clouds fade into the haze along the horizon.
    let t_mean = weighted_t / opacity;
    let seen = exp(-air_depth(clouds.camera, dir * t_mean));
    let cloud = light * seen + clouds.haze_color * (1.0 - seen) * opacity;
    return vec4<f32>(scene * transmittance + cloud, colour.a);
}
