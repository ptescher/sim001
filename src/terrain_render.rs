//! Chunked terrain with baked, per-chunk surface textures.
//!
//! Solari only ray traces `StandardMaterial` with a single UV set, so material blending
//! (grass, mown field, dirt, forest floor, rock, snow, shore) is baked into one colour
//! texture per chunk. Chunks near the field get dense geometry and ~14 cm texels; chunks
//! toward the horizon get coarser ones. Skirts hide cracks between LOD levels.

use crate::model::rt;
use crate::terrain::{self, fbm, gradient_noise};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const CHUNKS: i32 = 9; // odd, so the field sits in the middle of the centre chunk
/// Mesh cells and texture size per ring (Chebyshev distance from the centre chunk).
/// Each ring's count divides the next-inner ring's, so shared chunk edges line up vertex
/// for vertex and can be stitched watertight (no skirts, which the ray tracer would see).
const RING_CELLS: [usize; 5] = [384, 192, 192, 96, 96];
const RING_TEX: [u32; 5] = [4096, 2048, 1024, 512, 512];

/// Worker threads for world generation: leave headroom so the app keeps rendering.
pub fn worker_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(3).max(2)
}

/// Lower the calling thread's scheduling priority (macOS "utility" QoS), so the main and
/// render threads stay responsive while the world is generated.
pub fn background_priority() {
    #[cfg(target_os = "macos")]
    // SAFETY: plain syscall wrapper that only affects the calling thread.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

// Slowly varying surface fields (indices into a [f32; NF] array). They are evaluated on a
// coarse grid (~1 m near the field) and interpolated; only fine detail runs per texel.
const F_SLOPE: usize = 0;
const F_H: usize = 1;
const F_N1: usize = 2;
const F_N2: usize = 3;
const F_FIELD: usize = 4;
const F_FOREST: usize = 5;
const F_MOSS: usize = 6;
const F_DRIFT: usize = 7;
const F_KIND: usize = 8;
const F_LAKE: usize = 9;
const F_ROCK: usize = 10;
const F_ROCK_TONE: usize = 11;
const F_STRATA: usize = 12;
const F_SNOW: usize = 13;
const NF: usize = 14;

fn coarse_fields(x: f32, z: f32, h: f32, n: Vec3) -> [f32; NF] {
    let mut f = [0.0f32; NF];
    let slope = 1.0 - n.y.clamp(0.0, 1.0);
    let r = (x * x + z * z).sqrt();
    f[F_SLOPE] = slope;
    f[F_H] = h;
    f[F_N1] = fbm(x / 70.0 + 7.0, z / 70.0 - 3.0, 4);
    f[F_N2] = fbm(x / 11.0, z / 11.0, 3);
    let field_edge = 205.0 + 15.0 * gradient_noise(x / 40.0, z / 40.0);
    f[F_FIELD] = 1.0 - smoothstep(field_edge - 6.0, field_edge, r);
    f[F_FOREST] = if r > 140.0 { smoothstep(0.1, 0.6, terrain::forest_density(x, z)) * smoothstep(140.0, 180.0, r) } else { 0.0 };
    f[F_MOSS] = if f[F_FOREST] > 0.0 { smoothstep(0.55, 0.75, fbm(x / 6.0 + 9.0, z / 6.0, 3)) } else { 0.0 };
    f[F_DRIFT] = smoothstep(0.50, 0.75, fbm(x / 18.0 + 3.0, z / 18.0 - 8.0, 3));
    f[F_KIND] = fbm(x / 40.0 - 2.0, z / 40.0 + 5.0, 2);
    f[F_LAKE] = if Vec2::new(x, z).distance(terrain::LAKE_CENTER) < terrain::LAKE_RADIUS * 2.0 { terrain::lake_factor(x, z) } else { 9.0 };
    if slope > 0.2 || h > 140.0 {
        f[F_ROCK] = (smoothstep(0.30, 0.48, slope + 0.08 * gradient_noise(x / 25.0, z / 25.0))
            + smoothstep(180.0, 260.0, h + 30.0 * gradient_noise(x / 120.0, z / 120.0)))
        .clamp(0.0, 1.0);
        f[F_ROCK_TONE] = fbm(x / 60.0, z / 60.0, 3);
        f[F_STRATA] = 0.93 + 0.07 * (h * 0.12 + gradient_noise(x / 30.0, z / 30.0) * 4.0).sin();
    }
    if h > 230.0 {
        f[F_SNOW] = smoothstep(290.0, 330.0, h + 45.0 * gradient_noise(x / 150.0, z / 150.0) + 12.0 * gradient_noise(x / 23.0, z / 23.0))
            * (1.0 - smoothstep(0.45, 0.65, slope));
    }
    f
}

/// Per-texel detail on top of the interpolated coarse fields.
fn fine_color(x: f32, z: f32, f: &[f32; NF]) -> [f32; 3] {
    // Grass: four-tone alpine meadow green (deep, lush, sunny yellow-green, dry) mixed at
    // several scales, plus per-texel blade noise.
    let deep = [0.020, 0.070, 0.009];
    let lush = [0.038, 0.118, 0.011];
    let sunny = [0.090, 0.142, 0.014];
    let dry = [0.105, 0.115, 0.032];
    let n3 = fbm(x / 3.3 + 5.0, z / 3.3, 2);
    let mut c = mix(deep, lush, smoothstep(0.30, 0.62, f[F_N1] * 0.7 + n3 * 0.3));
    c = mix(c, sunny, smoothstep(0.55, 0.80, f[F_N2]) * 0.75);
    c = mix(c, dry, smoothstep(0.66, 0.86, f[F_N1]) * 0.45);
    let blade = gradient_noise(x * 3.1, z * 3.1) * 0.5 + gradient_noise(x * 9.7, z * 9.7) * 0.5;
    c = scale(c, 0.86 + 0.22 * blade.abs().min(1.0) + 0.06 * blade);

    // Mown flying field with stripes along the runway.
    let field = f[F_FIELD];
    if field > 0.0 {
        let tri = ((x / 6.0).rem_euclid(2.0) - 1.0).abs(); // 0..1 triangle wave across 12 m
        let stripe = smoothstep(0.35, 0.65, tri);
        let mown = scale([0.040, 0.112, 0.016], 0.88 + 0.24 * stripe);
        let mown = scale(mown, 0.92 + 0.12 * blade.abs());
        c = mix(c, mown, field);
    }

    // Forest floor darkens the ground under woods, with patchy moss.
    let forest = f[F_FOREST];
    if forest > 0.0 {
        let floor = mix([0.032, 0.036, 0.020], [0.060, 0.085, 0.022], f[F_MOSS]);
        c = mix(c, floor, forest * 0.8);
    }

    // Alpine flowers in meadow drifts (single-texel speckles read as flowers from the air).
    if (1.0 - field) * (1.0 - forest) > 0.2 {
        let drift = f[F_DRIFT] * (1.0 - field) * (1.0 - forest);
        let cell_x = (x * 7.0).floor() as i32;
        let cell_z = (z * 7.0).floor() as i32;
        let hsh = terrain::hash_pub(cell_x, cell_z);
        if hsh < (0.03 + 0.16 * drift) * (1.0 - smoothstep(90.0, 150.0, f[F_H])) {
            let pick = terrain::hash_pub(cell_x + 7, cell_z - 3) + (f[F_KIND] - 0.5) * 0.4;
            let flower = if pick < 0.45 { [0.70, 0.40, 0.02] } else if pick < 0.8 { [0.66, 0.66, 0.60] } else { [0.28, 0.08, 0.34] };
            c = mix(c, flower, 0.85);
        }
    }

    // Worn dirt (pilot box, path to the hangar, runway shoulders) and the lake shore:
    // only evaluated where they can appear.
    let p = Vec2::new(x, z);
    let near_field = p.distance(Vec2::new(terrain::pilot_pos().x, terrain::pilot_pos().z)) < 60.0
        || ((x - terrain::RUNWAY_CENTER.x).abs() < terrain::RUNWAY_HALF_WIDTH + 3.0
            && (z - terrain::RUNWAY_CENTER.y).abs() < terrain::RUNWAY_HALF_LENGTH + 3.0);
    if near_field || f[F_LAKE] < 1.4 {
        let wobble = gradient_noise(x * 0.9, z * 0.9) * 0.4;
        if near_field {
            let pilot = Vec2::new(terrain::pilot_pos().x, terrain::pilot_pos().z);
            let path = 1.0 - smoothstep(0.6, 1.1, seg_dist(p, pilot, Vec2::new(34.0, 36.0)) + wobble);
            let pad = 1.0 - smoothstep(2.5, 4.5, p.distance(pilot) + wobble * 2.0);
            let rx = (x - terrain::RUNWAY_CENTER.x).abs() - terrain::RUNWAY_HALF_WIDTH;
            let rz = (z - terrain::RUNWAY_CENTER.y).abs() - terrain::RUNWAY_HALF_LENGTH;
            let shoulder = if rz < 0.0 { 1.0 - smoothstep(0.0, 0.9, rx + wobble * 0.5) } else { 0.0 };
            let dirt = [0.110, 0.080, 0.050];
            let dirt_amt = path.max(pad).max(shoulder * 0.8) * (0.8 + 0.2 * blade);
            c = mix(c, scale(dirt, 0.9 + 0.2 * blade.abs()), dirt_amt.clamp(0.0, 1.0));
        }
        if f[F_LAKE] < 1.4 {
            let shore = 1.0 - smoothstep(1.02, 1.22, f[F_LAKE] + wobble * 0.03);
            c = mix(c, [0.16, 0.13, 0.09], shore * 0.85);
        }
    }

    // Rock on steep slopes and high ground, with strata; snow above, not on cliffs.
    if f[F_ROCK] > 0.0 {
        let rock_col = scale(mix([0.14, 0.13, 0.12], [0.20, 0.17, 0.14], f[F_ROCK_TONE]), f[F_STRATA]);
        c = mix(c, rock_col, f[F_ROCK]);
    }
    if f[F_SNOW] > 0.0 {
        c = mix(c, [0.78, 0.80, 0.84], f[F_SNOW]);
    }
    c
}

/// Linear albedo of the ground at (x, z), given height and surface normal.
#[allow(dead_code)]
pub fn surface_color(x: f32, z: f32, h: f32, n: Vec3) -> [f32; 3] {
    fine_color(x, z, &coarse_fields(x, z, h, n))
}

/// A real place: the aerial photo, with fine noise so it doesn't look smeared up close.
fn bake_real(site: &crate::realsite::RealSite, x0: f32, z0: f32, size: f32, res: u32) -> Vec<[f32; 3]> {
    let res = res as usize;
    let mut out = vec![[0.0f32; 3]; res * res];
    let rows_per = res.div_ceil(worker_threads());
    std::thread::scope(|s| {
        for (t, chunk) in out.chunks_mut(rows_per * res).enumerate() {
            s.spawn(move || {
                background_priority();
                for (k, px) in chunk.iter_mut().enumerate() {
                    let (row, col) = (t * rows_per + k / res, k % res);
                    let x = x0 + (col as f32 + 0.5) / res as f32 * size;
                    let z = z0 + (row as f32 + 0.5) / res as f32 * size;
                    let c = site.color(x, z);
                    let grain = 0.92 + 0.10 * gradient_noise(x * 2.3, z * 2.3) + 0.05 * gradient_noise(x * 7.9, z * 7.9);
                    *px = c.map(|v| v * grain);
                }
            });
        }
    });
    out
}

/// Bakes one chunk's texture. Slow-varying fields are evaluated on a coarse grid (in
/// parallel) and interpolated; only the cheap detail runs per texel. Rows are baked in parallel.
fn bake_chunk(x0: f32, z0: f32, size: f32, res: u32) -> Vec<[f32; 3]> {
    if let Some(site) = terrain::real() {
        return bake_real(site, x0, z0, size, res);
    }
    let g = (res / 8).max(16) as usize; // coarse grid cells
    let threads = worker_threads();
    let gw = g + 1;
    let mut coarse = vec![[0.0f32; NF]; gw * gw];
    let rows_per = gw.div_ceil(threads);
    std::thread::scope(|s| {
        for (t, rows) in coarse.chunks_mut(rows_per * gw).enumerate() {
            s.spawn(move || {
                background_priority();
                for (k, cell) in rows.iter_mut().enumerate() {
                    let (i, j) = (k % gw, t * rows_per + k / gw);
                    let x = x0 + size * i as f32 / g as f32;
                    let z = z0 + size * j as f32 / g as f32;
                    *cell = coarse_fields(x, z, terrain::ground_height(x, z), terrain::ground_normal(x, z));
                }
            });
        }
    });
    let res = res as usize;
    let mut out = vec![[0.0f32; 3]; res * res];
    let rows_per = res.div_ceil(threads);
    std::thread::scope(|s| {
        for (t, chunk) in out.chunks_mut(rows_per * res).enumerate() {
            let coarse = &coarse;
            s.spawn(move || {
                background_priority();
                let mut f = [0.0f32; NF];
                for (k, px) in chunk.iter_mut().enumerate() {
                    let row = t * rows_per + k / res;
                    let col = k % res;
                    let u = (col as f32 + 0.5) / res as f32;
                    let v = (row as f32 + 0.5) / res as f32;
                    let (gx, gz) = (u * g as f32, v * g as f32);
                    let (ix, iz) = ((gx as usize).min(g - 1), (gz as usize).min(g - 1));
                    let (fx, fz) = (gx - ix as f32, gz - iz as f32);
                    let (a, b, c, d) = (&coarse[iz * gw + ix], &coarse[iz * gw + ix + 1], &coarse[(iz + 1) * gw + ix], &coarse[(iz + 1) * gw + ix + 1]);
                    for q in 0..NF {
                        f[q] = (a[q] * (1.0 - fx) + b[q] * fx) * (1.0 - fz) + (c[q] * (1.0 - fx) + d[q] * fx) * fz;
                    }
                    *px = fine_color(x0 + u * size, z0 + v * size, &f);
                }
            });
        }
    });
    out
}

fn to_image(texels: Vec<[f32; 3]>, res: u32) -> Image {
    let to_srgb = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    let mut level = texels;
    let mut s = res as usize;
    let mut data = Vec::with_capacity(s * s * 4 * 4 / 3 + 16);
    let mut mips = 0;
    loop {
        for c in &level {
            data.extend_from_slice(&[to_srgb(c[0]), to_srgb(c[1]), to_srgb(c[2]), 255]);
        }
        mips += 1;
        if s == 1 {
            break;
        }
        let ns = s / 2;
        let mut next = Vec::with_capacity(ns * ns);
        for y in 0..ns {
            for x in 0..ns {
                let mut acc = [0.0f32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = level[(y * 2 + dy) * s + x * 2 + dx];
                    for i in 0..3 {
                        acc[i] += p[i] * 0.25;
                    }
                }
                next.push(acc);
            }
        }
        level = next;
        s = ns;
    }
    let mip0 = (res * res * 4) as usize;
    let mut image = Image::new(
        Extent3d { width: res, height: res, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data[..mip0].to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// Chunk mesh with analytic normals (seamless across chunks) and edge skirts.
/// Watertight chunk mesh. `neighbours` = cell counts of the chunks across the
/// [z0, z1, x0, x1] edges (0 = world edge). Where a neighbour is coarser, this chunk's extra
/// edge vertices are snapped onto the neighbour's straight edge, so there is no step for
/// the ray tracer to cast slivers of shadow from. Normals are analytic (seamless shading).
fn chunk_mesh(x0: f32, z0: f32, size: f32, cells: usize, neighbours: [usize; 4]) -> Mesh {
    let n = cells + 1;
    let cell = size / cells as f32;
    let mut pos = Vec::with_capacity(n * n);
    let mut nrm = Vec::with_capacity(n * n);
    let mut uv = Vec::with_capacity(n * n);
    let mut idx: Vec<u32> = Vec::with_capacity(cells * cells * 6);
    let e = (cell * 0.5).max(0.5);
    for j in 0..n {
        for i in 0..n {
            let (u, v) = (i as f32 / cells as f32, j as f32 / cells as f32);
            let (x, z) = (x0 + u * size, z0 + v * size);
            pos.push([x, terrain::ground_height(x, z), z]);
            let hx = terrain::ground_height(x + e, z) - terrain::ground_height(x - e, z);
            let hz = terrain::ground_height(x, z + e) - terrain::ground_height(x, z - e);
            nrm.push(Vec3::new(-hx, 2.0 * e, -hz).normalize().to_array());
            uv.push([u, v]);
        }
    }
    // Stitch edges that border a coarser chunk.
    let edges: [Vec<usize>; 4] = [
        (0..n).collect(),                          // z0 edge
        (0..n).map(|i| (n - 1) * n + i).collect(), // z1 edge
        (0..n).map(|j| j * n).collect(),           // x0 edge
        (0..n).map(|j| j * n + n - 1).collect(),   // x1 edge
    ];
    for (edge, &nb) in edges.iter().zip(neighbours.iter()) {
        if nb == 0 || nb >= cells {
            continue;
        }
        debug_assert_eq!(cells % nb, 0, "ring cell counts must divide evenly");
        let r = cells / nb;
        for k in 0..n {
            let m = k % r;
            if m == 0 {
                continue;
            }
            let (k0, k1) = (k - m, k - m + r);
            let t = m as f32 / r as f32;
            let (y0, y1) = (pos[edge[k0]][1], pos[edge[k1]][1]);
            pos[edge[k]][1] = y0 + (y1 - y0) * t;
        }
    }
    for j in 0..cells {
        for i in 0..cells {
            let a = (j * n + i) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            // Counter-clockwise seen from above (+Y).
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
        .with_generated_tangents()
        .expect("terrain tangents")
}

/// CPU work for the whole terrain (background thread): chunk meshes and baked textures.
/// `progress(done, total)` is called after each chunk.
pub fn build_terrain(mut progress: impl FnMut(usize, usize)) -> Vec<(Mesh, Image)> {
    let t0 = std::time::Instant::now();
    let size = 2.0 * terrain::half_size() / CHUNKS as f32;
    let half = CHUNKS / 2;
    let mut specs = Vec::new();
    for cj in -half..=half {
        for ci in -half..=half {
            let ring = (ci.abs().max(cj.abs()) as usize).min(4);
            let cells_at = |a: i32, b: i32| {
                if a.abs() > half || b.abs() > half { 0 } else { RING_CELLS[(a.abs().max(b.abs()) as usize).min(4)] }
            };
            let neighbours = [cells_at(ci, cj - 1), cells_at(ci, cj + 1), cells_at(ci - 1, cj), cells_at(ci + 1, cj)];
            specs.push(((ci as f32 - 0.5) * size, (cj as f32 - 0.5) * size, RING_CELLS[ring], RING_TEX[ring], neighbours));
        }
    }
    // Meshes in parallel (the texture bake below is itself multithreaded per chunk).
    let threads = worker_threads();
    let per = specs.len().div_ceil(threads);
    let chunk_meshes: Vec<Mesh> = std::thread::scope(|s| {
        let handles: Vec<_> = specs
            .chunks(per)
            .map(|group| {
                s.spawn(move || {
                    background_priority();
                    group.iter().map(|&(x0, z0, cells, _, nb)| chunk_mesh(x0, z0, size, cells, nb)).collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let total = specs.len();
    let mut texels = 0u64;
    let mut out = Vec::with_capacity(total);
    for (k, (&(x0, z0, _, res, _), mesh)) in specs.iter().zip(chunk_meshes).enumerate() {
        out.push((mesh, to_image(bake_chunk(x0, z0, size, res), res)));
        texels += res as u64 * res as u64;
        progress(k + 1, total);
    }
    info!("terrain: {total} chunks, {:.0} Mtexels baked in {:.1} s", texels as f64 / 1e6, t0.elapsed().as_secs_f32());
    out
}

/// Main thread: add the baked chunks as assets and spawn them.
pub fn spawn_terrain_chunks(
    chunks: Vec<(Mesh, Image)>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    for (mesh, image) in chunks {
        let mat = materials.add(StandardMaterial {
            base_color_texture: Some(images.add(image)),
            perceptual_roughness: 0.93,
            reflectance: 0.25,
            ..default()
        });
        commands.spawn(rt(meshes.add(mesh), mat, Transform::IDENTITY));
    }
}

/// What grows at a spot: used to place 3D grass and flower cards near the camera.
pub struct Cover {
    /// Clumps per square metre.
    pub density: f32,
    /// Typical clump height (m).
    pub height: f32,
    /// 0..1 chance a clump is a flower clump.
    pub flowers: f32,
    /// Which flower: 0 white, 1 yellow, 2 purple.
    pub flower_kind: u32,
    /// 0 lush .. 1 dry.
    pub dry: f32,
}

pub fn ground_cover(x: f32, z: f32) -> Cover {
    let none = Cover { density: 0.0, height: 0.0, flowers: 0.0, flower_kind: 0, dry: 0.0 };
    if let Some(site) = terrain::real() {
        // Grass where the photo shows open ground: greener or drier like the photo.
        return match site.grass(x, z) {
            Some((amount, dry)) => {
                let kind = fbm(x / 40.0 - 2.0, z / 40.0 + 5.0, 2);
                Cover {
                    density: 10.0 * amount,
                    height: 0.12 + 0.18 * fbm(x / 9.0, z / 9.0, 2),
                    flowers: 0.05 * (1.0 - dry),
                    flower_kind: if kind < 0.5 { 0 } else { 1 },
                    dry,
                }
            }
            None => none,
        };
    }
    if terrain::is_runway(x, z) || terrain::is_water(x, z) || terrain::lake_factor(x, z) < 1.08 {
        return none;
    }
    let h = terrain::ground_height(x, z);
    let n = terrain::ground_normal(x, z);
    let slope = 1.0 - n.y;
    let r = (x * x + z * z).sqrt();
    let field_edge = 205.0 + 15.0 * gradient_noise(x / 40.0, z / 40.0);
    let field = 1.0 - smoothstep(field_edge - 6.0, field_edge, r);
    let forest = smoothstep(0.1, 0.6, terrain::forest_density(x, z)) * smoothstep(140.0, 180.0, r);
    let p = Vec2::new(x, z);
    let pilot = Vec2::new(terrain::pilot_pos().x, terrain::pilot_pos().z);
    let wobble = gradient_noise(x * 0.9, z * 0.9) * 0.4;
    let path = 1.0 - smoothstep(0.6, 1.1, seg_dist(p, pilot, Vec2::new(34.0, 36.0)) + wobble);
    let pad = 1.0 - smoothstep(2.5, 4.5, p.distance(pilot) + wobble * 2.0);
    let rock = smoothstep(0.30, 0.45, slope) + smoothstep(180.0, 240.0, h);
    let bare = path.max(pad).max(rock).clamp(0.0, 1.0);
    let dryness = smoothstep(0.35, 0.75, fbm(x / 90.0 + 7.0, z / 90.0 - 3.0, 4));
    let meadow_h = 0.22 + 0.25 * fbm(x / 9.0, z / 9.0, 2);
    let height = meadow_h + (0.075 - meadow_h) * field;
    let density = (1.0 - bare) * (9.0 + 9.0 * field) * (1.0 - 0.75 * forest);
    let drift = smoothstep(0.50, 0.75, fbm(x / 18.0 + 3.0, z / 18.0 - 8.0, 3)) * (1.0 - field) * (1.0 - forest);
    let kind = fbm(x / 40.0 - 2.0, z / 40.0 + 5.0, 2);
    Cover {
        density,
        height,
        flowers: 0.12 + 0.45 * drift,
        flower_kind: if kind < 0.42 { 0 } else if kind < 0.58 { 1 } else { 2 },
        dry: dryness,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn positions(m: &Mesh) -> Vec<[f32; 3]> {
        match m.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => unreachable!(),
        }
    }

    /// A 192-cell chunk next to a 96-cell chunk (east side, hilly ground): every vertex on the
    /// fine side of the shared edge must lie exactly on the coarse chunk's edge.
    #[test]
    fn lod_seams_are_watertight() {
        let size = 578.0;
        let (x0, z0) = (900.0, -300.0);
        let fine = positions(&chunk_mesh(x0, z0, size, 192, [0, 0, 0, 96]));
        let coarse = positions(&chunk_mesh(x0 + size, z0, size, 96, [0, 0, 192, 0]));
        let (nf, nc) = (193, 97);
        let mut worst = 0.0f32;
        for k in 0..nf {
            let p = fine[k * nf + nf - 1]; // fine chunk's x1 edge
            let t = k as f32 / 2.0;
            let (i0, i1) = (t.floor() as usize, (t.ceil() as usize).min(nc - 1));
            let (a, b) = (coarse[i0 * nc], coarse[i1 * nc]); // coarse chunk's x0 edge
            let y = a[1] + (b[1] - a[1]) * (t - t.floor());
            worst = worst.max((p[1] - y).abs());
        }
        assert!(worst < 1e-3, "seam gap {worst} m");
    }
}
