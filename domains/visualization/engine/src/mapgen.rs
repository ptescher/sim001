//! Headless export of a top-down "satellite" map and height grid for the ground station.
//!
//!   cargo run --release -- --export-map domains/groundstation/console/assets
//!
//! Writes `terrain_map.png` (shaded orthophoto: baked ground colours, lake, runway, trees with
//! sun shadows, buildings), `terrain_height.bin` (little-endian f32 grid of ground height above
//! home) and `terrain_map.txt` (georeference). Map coordinates are metres north/east of home,
//! which is the pilot box: the same origin ArduPilot SITL uses for its home position.

use crate::terrain;
use crate::terrain_render::surface_color;
use crate::field::sun_dir;
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::math::{Vec2, Vec3};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::path::Path;

/// Half-width of the exported square, metres (the forest reaches ~1750 m).
const HALF: f32 = 1600.0;
const MAP_PX: usize = 2560;
const HEIGHT_N: usize = 321;
const EXPOSURE: f32 = 2.1;

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// World (x, z) of pixel (i, j); row 0 is north.
fn px_world(i: usize, j: usize, n: usize) -> (f32, f32) {
    let m = 2.0 * HALF / n as f32;
    (-HALF + (i as f32 + 0.5) * m, -HALF + (j as f32 + 0.5) * m)
}

fn base_layer(n: usize) -> Vec<[f32; 3]> {
    let sun = sun_dir();
    let mut out = vec![[0.0f32; 3]; n * n];
    let threads = std::thread::available_parallelism().map(|t| t.get()).unwrap_or(4);
    let rows_per = n.div_ceil(threads);
    std::thread::scope(|s| {
        for (k, rows) in out.chunks_mut(rows_per * n).enumerate() {
            s.spawn(move || {
                for (r, row) in rows.chunks_mut(n).enumerate() {
                    let j = k * rows_per + r;
                    for (i, px) in row.iter_mut().enumerate() {
                        let (x, z) = px_world(i, j, n);
                        let h = terrain::ground_height(x, z);
                        let nrm = terrain::ground_normal(x, z);
                        let lit = 0.42 + 0.95 * nrm.dot(sun).max(0.0);
                        let c = if terrain::is_water(x, z) {
                            let depth = (terrain::WATER_LEVEL - h).clamp(0.0, 5.0) / 5.0;
                            let deep = [0.010, 0.030, 0.045];
                            let shallow = [0.040, 0.075, 0.075];
                            [0, 1, 2].map(|c| (shallow[c] + (deep[c] - shallow[c]) * depth) * 1.6)
                        } else if terrain::is_runway(x, z) {
                            let along = z - terrain::RUNWAY_CENTER.y;
                            let centre = x.abs() < 0.25 && along.rem_euclid(12.0) < 6.0;
                            let edge = (x - terrain::RUNWAY_CENTER.x).abs() > terrain::RUNWAY_HALF_WIDTH - 0.5;
                            let v = if centre || edge { 0.55 } else { 0.075 + 0.01 * terrain::fbm(x, z, 2) };
                            [v, v, v * 1.05].map(|c| c * lit)
                        } else {
                            surface_color(x, z, h, nrm).map(|c| c * lit)
                        };
                        *px = c.map(|c| c * EXPOSURE);
                    }
                }
            });
        }
    });
    out
}

fn blend(img: &mut [[f32; 3]], n: usize, i: i32, j: i32, c: [f32; 3], a: f32) {
    if i < 0 || j < 0 || i >= n as i32 || j >= n as i32 {
        return;
    }
    let p = &mut img[j as usize * n + i as usize];
    for k in 0..3 {
        p[k] += (c[k] - p[k]) * a;
    }
}

/// Soft-edged disc in pixel space.
fn disc(img: &mut [[f32; 3]], n: usize, cx: f32, cy: f32, r: f32, c: [f32; 3], a: f32) {
    let r = r.max(0.6);
    for j in (cy - r - 1.0) as i32..=(cy + r + 1.0) as i32 {
        for i in (cx - r - 1.0) as i32..=(cx + r + 1.0) as i32 {
            let d = Vec2::new(i as f32 + 0.5 - cx, j as f32 + 0.5 - cy).length();
            let cov = (r + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(img, n, i, j, c, a * cov);
            }
        }
    }
}

fn overlay_objects(img: &mut [[f32; 3]], n: usize) {
    let m = 2.0 * HALF / n as f32;
    let to_px = |x: f32, z: f32| ((x + HALF) / m, (z + HALF) / m);
    let sun = sun_dir();
    // Shadows fall away from the sun; length per metre of height.
    let shadow_dir = Vec2::new(-sun.x, -sun.z).normalize();
    let shadow_len = (1.0 - sun.y * sun.y).sqrt() / sun.y;

    let trees = terrain::trees();
    for t in &trees {
        let (cx, cy) = to_px(t.pos.x, t.pos.z);
        let crown = t.height * if t.conifer { 0.17 } else { 0.24 } / m;
        let sh = shadow_dir * t.height * shadow_len * 0.6 / m;
        for k in 0..4 {
            let f = k as f32 / 3.0;
            disc(img, n, cx + sh.x * f, cy + sh.y * f, crown * (1.0 - 0.3 * f), [0.0; 3], 0.35);
        }
    }
    for t in &trees {
        let (cx, cy) = to_px(t.pos.x, t.pos.z);
        let crown = t.height * if t.conifer { 0.17 } else { 0.24 } / m;
        let tint = 0.85 + 0.1 * t.shade as f32;
        let (dark, lit) = if t.conifer {
            ([0.010, 0.030, 0.016], [0.030, 0.075, 0.035])
        } else {
            ([0.040, 0.075, 0.014], [0.110, 0.170, 0.030])
        };
        disc(img, n, cx, cy, crown, dark.map(|c| c * tint * EXPOSURE), 0.95);
        let hl = Vec2::new(sun.x, sun.z).normalize() * crown * 0.3;
        disc(img, n, cx + hl.x, cy + hl.y, crown * 0.55, lit.map(|c| c * tint * EXPOSURE), 0.7);
    }
    for b in terrain::buildings() {
        let (x0, y0) = to_px(b.center.x - b.size.x * 0.5, b.center.z - b.size.z * 0.5);
        let (x1, y1) = to_px(b.center.x + b.size.x * 0.5, b.center.z + b.size.z * 0.5);
        let sh = shadow_dir * b.size.y * shadow_len / m;
        for (dx, dy, c, a) in [(sh.x, sh.y, [0.0; 3], 0.55), (0.0, 0.0, [0.42, 0.40, 0.38], 1.0)] {
            for j in (y0 + dy) as i32..=(y1 + dy) as i32 {
                for i in (x0 + dx) as i32..=(x1 + dx) as i32 {
                    blend(img, n, i, j, c, a);
                }
            }
        }
    }
}

pub fn export(dir: &str) {
    let t0 = std::time::Instant::now();
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).expect("create map output directory");

    let n = MAP_PX;
    let mut img = base_layer(n);
    overlay_objects(&mut img, n);
    let mut bytes = Vec::with_capacity(n * n * 4);
    for p in &img {
        bytes.extend_from_slice(&[to_srgb(p[0]), to_srgb(p[1]), to_srgb(p[2]), 255]);
    }
    let image = Image::new(
        Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    let png = dir.join("terrain_map.png");
    image.try_into_dynamic().expect("convert map").save(&png).expect("write terrain_map.png");

    // Height grid (row 0 north), relative to the ground at home.
    let home_h = terrain::ground_height(terrain::pilot_pos().x, terrain::pilot_pos().z);
    let mut hb = Vec::with_capacity(HEIGHT_N * HEIGHT_N * 4);
    let step = 2.0 * HALF / (HEIGHT_N - 1) as f32;
    for j in 0..HEIGHT_N {
        for i in 0..HEIGHT_N {
            let (x, z) = (-HALF + i as f32 * step, -HALF + j as f32 * step);
            let h = terrain::ground_height(x, z).max(terrain::WATER_LEVEL) - home_h;
            hb.extend_from_slice(&h.to_le_bytes());
        }
    }
    std::fs::write(dir.join("terrain_height.bin"), hb).expect("write terrain_height.bin");

    // Georeference: world (+X east, -Z north) relative to home.
    let centre = Vec3::ZERO - Vec3::new(terrain::pilot_pos().x, 0.0, terrain::pilot_pos().z);
    let meta = format!(
        "# t1s_sim map export. Map centre is given in metres north/east of home (the pilot box).\n\
size_m={size}\nmap_px={n}\ncentre_north={cn}\ncentre_east={ce}\nheight_n={hn}\n\
runway_north={rn}\nrunway_east={re}\nrunway_length={rl}\nrunway_width={rw}\nrunway_heading=0\n",
        size = 2.0 * HALF,
        cn = -centre.z,
        ce = centre.x,
        hn = HEIGHT_N,
        rn = -(terrain::RUNWAY_CENTER.y - terrain::pilot_pos().z),
        re = terrain::RUNWAY_CENTER.x - terrain::pilot_pos().x,
        rl = 2.0 * terrain::RUNWAY_HALF_LENGTH,
        rw = 2.0 * terrain::RUNWAY_HALF_WIDTH,
    );
    std::fs::write(dir.join("terrain_map.txt"), meta).expect("write terrain_map.txt");
    println!("map exported to {} in {:.1} s", dir.display(), t0.elapsed().as_secs_f32());
}
