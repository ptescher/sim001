//! Builds a real-world map pack for sim001.
//!
//!   cargo run --release -p visualization_mapbuild -- cypress
//!
//! Reads `maps/<name>/map.txt`, downloads USGS 3DEP lidar elevation, USGS NAIP imagery (colour
//! and near-infrared) and OpenStreetMap runways/buildings, and writes `maps/<name>/build/`:
//! `height.f32`, `ortho_wide.jpg`, `ortho_core.jpg`, `landcover.png`, `trees.bin`,
//! `buildings.txt`, `runway.txt`, `pack.txt`, plus the ground station's map in
//! `domains/teleoperation/console/assets/<name>/`. Trees are grown where the photo shows canopy.

mod fetch;

use fetch::{Mosaic, Request};
use image::{GrayImage, RgbImage};
use std::path::{Path, PathBuf};
use visualization_mappack::{self as mappack, MapConfig, PackBox, PackMeta, PackTree, Square, cover};

/// Elevation grid spacing, wide photo and core photo resolutions (m/px), core size (m).
const HEIGHT_M: f32 = 2.0;
const WIDE_M: f32 = 1.0;
const CORE_M: f32 = 0.4;
const CORE_SIZE: f32 = 1600.0;
/// Keep the forest within what the sim can draw.
const MAX_TREES: usize = 60_000;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(name) = args.get(1) else {
        eprintln!("usage: mapbuild <map name>   (a directory in maps/ with a map.txt)");
        std::process::exit(2);
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repo root");
    if let Err(e) = build(&root, name) {
        eprintln!("mapbuild: {e}");
        std::process::exit(1);
    }
}

fn build(root: &Path, name: &str) -> Result<(), String> {
    let t0 = std::time::Instant::now();
    let cfg = MapConfig::load(&root.join("maps"), name)?;
    let out = cfg.build_dir();
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    fetch::set_cache(out.join("cache"));
    let geo = cfg.geo();
    println!(
        "{}: {:.0} m square around {:.5}, {:.5}; home {:.5}, {:.5}",
        cfg.title, cfg.size_m, cfg.center_lat, cfg.center_lon, cfg.home_lat, cfg.home_lon
    );

    let (cn, ce) = geo.to_ne(cfg.center_lat, cfg.center_lon);
    let sq = |size: f32, m: f32, n: f32, e: f32| Square {
        centre_n: n,
        centre_e: e,
        size_m: size,
        px: (size / m).round() as u32,
    };
    let height_sq = sq(cfg.size_m, HEIGHT_M, cn as f32, ce as f32);
    let wide_sq = sq(cfg.size_m, WIDE_M, cn as f32, ce as f32);
    let core_sq = sq(CORE_SIZE, CORE_M, 0.0, 0.0);

    // Lat/lon box covering a square (plus a margin for bilinear sampling).
    let bbox = |s: &Square, m: f32| {
        let half = s.size_m as f64 * 0.5 + 2.0 * m as f64;
        let (lat0, lon0) = geo.to_ll(s.centre_n as f64 - half, s.centre_e as f64 - half);
        let (lat1, lon1) = geo.to_ll(s.centre_n as f64 + half, s.centre_e as f64 + half);
        Request { lat0, lat1, lon0, lon1, m_per_px: m as f64 }
    };

    println!("downloading");
    let dem = fetch::dem(&bbox(&height_sq, HEIGHT_M))?;
    let wide_rgb = fetch::naip(&bbox(&wide_sq, WIDE_M), false)?;
    let wide_nir = fetch::naip(&bbox(&wide_sq, WIDE_M), true)?;
    let core_rgb = fetch::naip(&bbox(&core_sq, CORE_M), false)?;
    let (lat0, lon0) =
        geo.to_ll((wide_sq.centre_n - wide_sq.size_m * 0.5) as f64, (wide_sq.centre_e - wide_sq.size_m * 0.5) as f64);
    let (lat1, lon1) =
        geo.to_ll((wide_sq.centre_n + wide_sq.size_m * 0.5) as f64, (wide_sq.centre_e + wide_sq.size_m * 0.5) as f64);
    let osm = match fetch::osm(lat0, lon0, lat1, lon1) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  OpenStreetMap unavailable ({e}); continuing without runway/buildings");
            vec![]
        }
    };
    println!("  OpenStreetMap: {} features", osm.len());

    println!("processing");
    // Elevation onto the metric grid.
    let heights = resample(&height_sq, |n, e| {
        let (lat, lon) = geo.to_ll(n as f64, e as f64);
        dem.sample(lat, lon, 0).unwrap_or(0.0)
    });
    let heights = despike(heights, height_sq.px as usize);
    let height_at = |n: f32, e: f32| sample_grid(&heights, &height_sq, n, e);
    let home_alt = height_at(0.0, 0.0);
    println!(
        "  home ground {:.1} m AMSL; terrain {:.0}..{:.0} m",
        home_alt,
        heights.iter().cloned().fold(f32::MAX, f32::min),
        heights.iter().cloned().fold(f32::MIN, f32::max)
    );
    let mut hb = Vec::with_capacity(heights.len() * 4);
    for h in &heights {
        hb.extend_from_slice(&h.to_le_bytes());
    }
    std::fs::write(out.join("height.f32"), hb).map_err(|e| e.to_string())?;

    // Photos onto their grids.
    let photo = |s: &Square, m: &Mosaic<u8>| -> RgbImage {
        let mut img = RgbImage::new(s.px, s.px);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let (n, e) = s.to_ne(x, y);
            let (lat, lon) = geo.to_ll(n as f64, e as f64);
            *p = image::Rgb([0, 1, 2].map(|b| m.sample(lat, lon, b).unwrap_or(0.0).round() as u8));
        }
        img
    };
    let wide = photo(&wide_sq, &wide_rgb);
    let core = photo(&core_sq, &core_rgb);
    let nir = resample_u8(&wide_sq, |n, e| {
        let (lat, lon) = geo.to_ll(n as f64, e as f64);
        wide_nir.sample(lat, lon, 0).unwrap_or(0.0)
    });
    save_jpeg(&wide, &out.join("ortho_wide.jpg"))?;
    save_jpeg(&core, &out.join("ortho_core.jpg"))?;

    // OSM features in pack coordinates.
    let (runways, buildings) = osm_boxes(&osm, &geo);
    if let Some(r) = runways.first() {
        println!(
            "  runway {:.0} x {:.0} m, heading {:03.0}, {:.0} m from home",
            r.length,
            r.width,
            r.heading,
            (r.north * r.north + r.east * r.east).sqrt()
        );
    }
    println!("  {} buildings", buildings.len());
    mappack::write_boxes(&out.join("runway.txt"), &runways).map_err(|e| e.to_string())?;
    mappack::write_boxes(&out.join("buildings.txt"), &buildings).map_err(|e| e.to_string())?;

    // MAPBUILD_PROBE="lat,lon;lat,lon": print vegetation statistics around points (for tuning
    // the classification to a region's imagery).
    if let Ok(probe) = std::env::var("MAPBUILD_PROBE") {
        for pt in probe.split(';') {
            let v: Vec<f64> = pt.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() != 2 {
                continue;
            }
            let (pn, pe) = geo.to_ne(v[0], v[1]);
            let (mut ndvi, mut bright, mut ratio) = (vec![], vec![], vec![]);
            for dy in -40..=40 {
                for dx in -40..=40 {
                    let (x, y) = wide_sq.to_px(pn as f32 + dy as f32, pe as f32 + dx as f32);
                    let (x, y) = (x as u32, y as u32);
                    if x >= wide_sq.px || y >= wide_sq.px {
                        continue;
                    }
                    let [r, g, b] = wide.get_pixel(x, y).0.map(|c| c as f32);
                    let ir = nir[(y * wide_sq.px + x) as usize] as f32;
                    ndvi.push((ir - r) / (ir + r + 1.0));
                    bright.push((r + g + b) / 3.0);
                    ratio.push(g / (r + 1.0));
                }
            }
            for a in [&mut ndvi, &mut bright, &mut ratio] {
                a.sort_by(f32::total_cmp);
            }
            let q = |a: &Vec<f32>, k: f32| a[((a.len() - 1) as f32 * k) as usize];
            println!(
                "  probe {pt}: ndvi p10 {:.2} p50 {:.2} p90 {:.2} | brightness p10 {:.0} p50 {:.0} p90 {:.0} | g/r p50 {:.2}",
                q(&ndvi, 0.1),
                q(&ndvi, 0.5),
                q(&ndvi, 0.9),
                q(&bright, 0.1),
                q(&bright, 0.5),
                q(&bright, 0.9),
                q(&ratio, 0.5)
            );
        }
    }

    // Land cover from colour + near-infrared + elevation.
    let mut lc = classify(&wide_sq, &wide, &nir, &height_at);
    for b in runways.iter() {
        paint_box(&mut lc, &wide_sq, b, cover::PAVED);
    }
    for b in buildings.iter() {
        paint_box(&mut lc, &wide_sq, b, cover::BUILDING);
    }
    let counts = (0..5u8)
        .map(|c| lc.as_raw().iter().filter(|&&v| v == c).count() as f32 / lc.as_raw().len() as f32 * 100.0)
        .collect::<Vec<_>>();
    println!(
        "  land cover: ground {:.0}%, trees {:.0}%, water {:.0}%, paved {:.0}%, buildings {:.1}%",
        counts[0], counts[1], counts[2], counts[3], counts[4]
    );
    lc.save(out.join("landcover.png")).map_err(|e| e.to_string())?;
    // A coloured copy for checking the classification against the photo.
    let palette = [[196, 178, 120], [24, 110, 34], [40, 90, 210], [150, 150, 150], [220, 40, 40]];
    RgbImage::from_fn(lc.width(), lc.height(), |x, y| image::Rgb(palette[lc.get_pixel(x, y).0[0].min(4) as usize]))
        .save(out.join("landcover_preview.png"))
        .map_err(|e| e.to_string())?;

    let trees = grow_trees(&cfg, &wide_sq, &lc, &height_at, &runways);
    println!("  {} trees", trees.len());
    mappack::write_trees(&out.join("trees.bin"), &trees).map_err(|e| e.to_string())?;

    let meta = PackMeta {
        title: cfg.title.clone(),
        home_lat: cfg.home_lat,
        home_lon: cfg.home_lon,
        home_alt,
        launch_heading: cfg.launch_heading,
        height: height_sq,
        wide: wide_sq,
        core: core_sq,
        credits:
            "Elevation: USGS 3DEP. Imagery: USGS/USDA NAIP. Runway and buildings: (c) OpenStreetMap contributors, ODbL."
                .into(),
    };
    meta.write(&out).map_err(|e| e.to_string())?;

    write_gcs_assets(
        &root.join("domains/teleoperation/console/assets").join(name),
        &meta,
        &wide,
        &height_at,
        runways.first(),
    )?;
    println!("done in {:.0} s: {}", t0.elapsed().as_secs_f32(), out.display());
    Ok(())
}

fn resample(s: &Square, f: impl Fn(f32, f32) -> f32 + Sync) -> Vec<f32> {
    let n = s.px as usize;
    let mut v = vec![0.0f32; n * n];
    let threads = std::thread::available_parallelism().map(|t| t.get()).unwrap_or(4);
    let rows = n.div_ceil(threads);
    std::thread::scope(|sc| {
        for (k, chunk) in v.chunks_mut(rows * n).enumerate() {
            let f = &f;
            sc.spawn(move || {
                for (i, val) in chunk.iter_mut().enumerate() {
                    let (x, y) = ((i % n) as u32, (k * rows + i / n) as u32);
                    let (nn, e) = s.to_ne(x, y);
                    *val = f(nn, e);
                }
            });
        }
    });
    v
}

/// The elevation service has occasional garbage samples (spikes to thousands of metres and
/// deep pits, mostly over water). Replace anything far from its 5x5 neighbourhood median.
fn despike(mut g: Vec<f32>, n: usize) -> Vec<f32> {
    for _ in 0..2 {
        let src = g.clone();
        let mut fixed = 0;
        for y in 2..n - 2 {
            for x in 2..n - 2 {
                let mut w = [0.0f32; 25];
                let mut k = 0;
                for dy in 0..5 {
                    for dx in 0..5 {
                        w[k] = src[(y + dy - 2) * n + x + dx - 2];
                        k += 1;
                    }
                }
                w.sort_by(f32::total_cmp);
                let med = w[12];
                if (src[y * n + x] - med).abs() > 40.0 {
                    g[y * n + x] = med;
                    fixed += 1;
                }
            }
        }
        // The outermost two rows/columns: copy inward.
        for i in 0..n {
            for (a, b) in [(0, 2), (1, 2), (n - 1, n - 3), (n - 2, n - 3)] {
                g[a * n + i] = g[b * n + i];
                g[i * n + a] = g[i * n + b];
            }
        }
        if fixed == 0 {
            break;
        }
        println!("  despiked {fixed} elevation samples");
    }
    g
}

fn resample_u8(s: &Square, f: impl Fn(f32, f32) -> f32 + Sync) -> Vec<u8> {
    resample(s, f).into_iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect()
}

fn sample_grid(g: &[f32], s: &Square, n: f32, e: f32) -> f32 {
    let w = s.px as usize;
    let (x, y) = s.to_px(n, e);
    let (fx, fy) = ((x - 0.5).clamp(0.0, (w - 1) as f32 - 0.001), (y - 0.5).clamp(0.0, (w - 1) as f32 - 0.001));
    let (i, j) = (fx as usize, fy as usize);
    let (tx, ty) = (fx - i as f32, fy - j as f32);
    let at = |i: usize, j: usize| g[j * w + i];
    let top = at(i, j) + (at(i + 1, j) - at(i, j)) * tx;
    let bot = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * tx;
    top + (bot - top) * ty
}

fn save_jpeg(img: &RgbImage, path: &Path) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), 90);
    enc.encode_image(img).map_err(|e| format!("{}: {e}", path.display()))
}

/// Per pixel: water (dark in near-infrared, at sea level), trees (stronger vegetation signal and
/// darker than the image's typical land, with crown texture where the signal is weak), paved
/// (grey, no vegetation), else ground (grass, dirt, gravel). Thresholds adapt to each region's
/// imagery: NAIP from different states and years differs a lot in brightness and NIR.
fn classify(s: &Square, rgb: &RgbImage, nir: &[u8], height_at: &impl Fn(f32, f32) -> f32) -> GrayImage {
    let n = s.px;
    let at = |x: u32, y: u32| {
        let [r, g, b] = rgb.get_pixel(x, y).0.map(|v| v as f32);
        let ir = nir[(y * n + x) as usize] as f32;
        ((ir - r) / (ir + r + 1.0), (r + g + b) / 3.0, r, g, b, ir)
    };
    // Typical land (not sea): medians of NDVI and brightness.
    let (mut nd, mut br) = (vec![], vec![]);
    for y in (0..n).step_by(7) {
        for x in (0..n).step_by(7) {
            let (ndvi, bright, ..) = at(x, y);
            let (north, east) = s.to_ne(x, y);
            if height_at(north, east) > 1.0 {
                nd.push(ndvi);
                br.push(bright);
            }
        }
    }
    nd.sort_by(f32::total_cmp);
    br.sort_by(f32::total_cmp);
    let (land_ndvi, land_bright) =
        (nd.get(nd.len() / 2).copied().unwrap_or(0.3), br.get(br.len() / 2).copied().unwrap_or(80.0));
    let tree_ndvi = (land_ndvi + 0.12).min(0.2);
    let tree_bright = (land_bright * 0.85).max(95.0);
    println!(
        "  land: median NDVI {land_ndvi:.2}, brightness {land_bright:.0} -> trees: NDVI > {tree_ndvi:.2}, brightness < {tree_bright:.0}"
    );

    let mut lc = GrayImage::new(n, n);
    for (x, y, p) in lc.enumerate_pixels_mut() {
        let (ndvi, bright, r, g, b, ir) = at(x, y);
        let (north, east) = s.to_ne(x, y);
        let h = height_at(north, east);
        // Crown texture: brightness spread in a 5x5 window (canopies are lumpy, lawns smooth).
        let textured = || {
            let (x0, y0) = (x.saturating_sub(2), y.saturating_sub(2));
            let (x1, y1) = ((x + 2).min(n - 1), (y + 2).min(n - 1));
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for yy in y0..=y1 {
                for xx in x0..=x1 {
                    let v = at(xx, yy).1;
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            hi - lo > 28.0
        };
        // Sea: near sea level and not vegetated (catches sun glint too). Inland water: very dark
        // in near-infrared; small patches of that are shadows, removed below.
        p.0[0] = if (h < 0.8 && ndvi < 0.2) || (ndvi < -0.05 && ir < 45.0) {
            cover::WATER
        } else if ndvi > tree_ndvi && bright < tree_bright && g >= r * 0.85 && (ndvi > 0.35 || textured()) {
            cover::TREE
        } else if ndvi < 0.08 && (r - b).abs() < 0.10 * bright && bright > 60.0 {
            // Neutral grey: concrete, asphalt, gravel. (Dry grass is as unvegetated, but warm.)
            cover::PAVED
        } else {
            cover::GROUND
        };
    }
    drop_small_water(&mut lc, 5000);
    // Majority filter for trees: drops single-pixel speckle and fills canopy gaps.
    let src = lc.clone();
    for y in 1..n - 1 {
        for x in 1..n - 1 {
            let c = src.get_pixel(x, y).0[0];
            if c != cover::TREE && c != cover::GROUND {
                continue;
            }
            let mut trees = 0;
            for dy in 0..3 {
                for dx in 0..3 {
                    trees += (src.get_pixel(x + dx - 1, y + dy - 1).0[0] == cover::TREE) as u32;
                }
            }
            lc.put_pixel(
                x,
                y,
                image::Luma([if trees >= 5 {
                    cover::TREE
                } else if c == cover::TREE && trees < 3 {
                    cover::GROUND
                } else {
                    c
                }]),
            );
        }
    }
    lc
}

/// Water regions smaller than `min_px` are shadows (dark in near-infrared like water): they
/// become trees when mostly surrounded by canopy, else ground.
fn drop_small_water(lc: &mut GrayImage, min_px: usize) {
    let (w, h) = (lc.width() as usize, lc.height() as usize);
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut region = Vec::new();
    let mut dropped = 0;
    for start in 0..w * h {
        if seen[start] || lc.as_raw()[start] != cover::WATER {
            continue;
        }
        region.clear();
        stack.push(start);
        seen[start] = true;
        let mut tree_edges = 0usize;
        let mut edges = 0usize;
        while let Some(i) = stack.pop() {
            region.push(i);
            let (x, y) = (i % w, i / w);
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                let c = lc.as_raw()[j];
                if c == cover::WATER {
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                } else {
                    edges += 1;
                    tree_edges += (c == cover::TREE) as usize;
                }
            }
        }
        if region.len() < min_px {
            let fill = if tree_edges * 2 > edges { cover::TREE } else { cover::GROUND };
            for &i in &region {
                lc.put_pixel((i % w) as u32, (i / w) as u32, image::Luma([fill]));
            }
            dropped += 1;
        }
    }
    println!("  dropped {dropped} small dark patches (shadows) from water");
}

fn hash(a: i32, b: i32, salt: u32) -> f32 {
    let mut h =
        (a as u32).wrapping_mul(374_761_393) ^ (b as u32).wrapping_mul(668_265_263) ^ salt.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Jittered-grid trees inside the canopy mask: dense near home, sparser further out (the photo
/// carries the far forest), clear of the runway and the pilot box.
fn grow_trees(
    cfg: &MapConfig,
    s: &Square,
    lc: &GrayImage,
    height_at: &impl Fn(f32, f32) -> f32,
    runways: &[PackBox],
) -> Vec<PackTree> {
    let canopy = |n: f32, e: f32| -> bool {
        let (x, y) = s.to_px(n, e);
        if x < 1.0 || y < 1.0 || x >= s.px as f32 - 1.0 || y >= s.px as f32 - 1.0 {
            return false;
        }
        let mut k = 0;
        for (dx, dy) in [(0.0, 0.0), (1.5, 0.0), (-1.5, 0.0), (0.0, 1.5), (0.0, -1.5)] {
            let (xx, yy) =
                ((x + dx).clamp(0.0, s.px as f32 - 1.0) as u32, (y + dy).clamp(0.0, s.px as f32 - 1.0) as u32);
            k += (lc.get_pixel(xx, yy).0[0] == cover::TREE) as u32;
        }
        k >= 4
    };
    let on_runway = |n: f32, e: f32| {
        runways.iter().any(|r| {
            let (sn, cn) = r.heading.to_radians().sin_cos();
            let (dn, de) = (n - r.north, e - r.east);
            let along = dn * cn + de * sn;
            let across = -dn * sn + de * cn;
            along.abs() < r.length * 0.5 + 15.0 && across.abs() < r.width * 0.5 + 8.0
        })
    };
    let mut scale = 1.0f32;
    loop {
        let mut trees = Vec::new();
        for (spacing, r0, r1) in
            [(5.5 * scale, 0.0, 700.0), (9.0 * scale, 700.0, 1600.0), (16.0 * scale, 1600.0, f32::MAX)]
        {
            let half = s.size_m * 0.5;
            let (n0, n1, e0, e1) = (s.centre_n - half, s.centre_n + half, s.centre_e - half, s.centre_e + half);
            let (i0, i1) = ((e0 / spacing).floor() as i32, (e1 / spacing).ceil() as i32);
            let (j0, j1) = ((n0 / spacing).floor() as i32, (n1 / spacing).ceil() as i32);
            for j in j0..j1 {
                for i in i0..i1 {
                    let e = (i as f32 + 0.15 + 0.7 * hash(i, j, 1)) * spacing;
                    let n = (j as f32 + 0.15 + 0.7 * hash(i, j, 2)) * spacing;
                    let d = (n * n + e * e).sqrt();
                    if d < r0 || d >= r1 || d < 30.0 || !canopy(n, e) || on_runway(n, e) {
                        continue;
                    }
                    let t = hash(i, j, 3);
                    trees.push(PackTree {
                        north: n,
                        east: e,
                        height: cfg.tree_min_m + (cfg.tree_max_m - cfg.tree_min_m) * t.powf(0.8),
                        conifer: hash(i, j, 4) < cfg.conifer_fraction,
                        shade: (hash(i, j, 5) * 4.0) as u8,
                    });
                }
            }
        }
        let _ = height_at;
        if trees.len() <= MAX_TREES {
            return trees;
        }
        scale *= (trees.len() as f32 / MAX_TREES as f32).sqrt() * 1.02;
    }
}

/// Oriented boxes from OSM ways: runways (paved strips) and building footprints.
fn osm_boxes(elements: &[serde_json::Value], geo: &mappack::Geo) -> (Vec<PackBox>, Vec<PackBox>) {
    let mut runways = vec![];
    let mut buildings = vec![];
    for el in elements {
        let Some(geom) = el["geometry"].as_array() else { continue };
        let pts: Vec<(f32, f32)> = geom
            .iter()
            .filter_map(|p| Some(geo.to_ne(p["lat"].as_f64()?, p["lon"].as_f64()?)))
            .map(|(n, e)| (n as f32, e as f32))
            .collect();
        if pts.len() < 2 {
            continue;
        }
        let tags = &el["tags"];
        let num = |k: &str| tags[k].as_str().and_then(|s| s.trim_end_matches(" m").parse::<f32>().ok());
        let mut b = oriented_box(&pts);
        if tags["aeroway"].as_str() == Some("runway") {
            b.width = num("width").unwrap_or(if pts.len() <= 3 { 20.0 } else { b.width.max(8.0) });
            b.height = 0.0;
            runways.push(b);
        } else if tags["building"].is_string() {
            b.height = num("height").or_else(|| num("building:levels").map(|l| l * 3.2 + 1.0)).unwrap_or(
                match tags["building"].as_str() {
                    Some("house" | "residential" | "detached" | "garage" | "shed") => 6.0,
                    _ => 7.0,
                },
            );
            b.width = b.width.max(2.0);
            b.length = b.length.max(2.0);
            buildings.push(b);
        }
    }
    // The longest runway first.
    runways.sort_by(|a, b| b.length.total_cmp(&a.length));
    (runways, buildings)
}

/// Principal-axis box around points (north, east).
fn oriented_box(pts: &[(f32, f32)]) -> PackBox {
    let n = pts.len() as f32;
    let (mn, me) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n));
    let (mut snn, mut see, mut sne) = (0.0, 0.0, 0.0);
    for &(pn, pe) in pts {
        snn += (pn - mn) * (pn - mn);
        see += (pe - me) * (pe - me);
        sne += (pn - mn) * (pe - me);
    }
    // Long-axis direction (as a heading from north toward east).
    let ang = 0.5 * (2.0 * sne).atan2(snn - see);
    let (sa, ca) = ang.sin_cos();
    let (mut a0, mut a1, mut b0, mut b1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for &(pn, pe) in pts {
        let (dn, de) = (pn - mn, pe - me);
        let a = dn * ca + de * sa;
        let b = -dn * sa + de * ca;
        a0 = a0.min(a);
        a1 = a1.max(a);
        b0 = b0.min(b);
        b1 = b1.max(b);
    }
    let (ac, bc) = ((a0 + a1) * 0.5, (b0 + b1) * 0.5);
    PackBox {
        north: mn + ac * ca - bc * sa,
        east: me + ac * sa + bc * ca,
        length: a1 - a0,
        width: b1 - b0,
        height: 0.0,
        heading: ang.to_degrees().rem_euclid(360.0),
    }
}

fn paint_box(lc: &mut GrayImage, s: &Square, b: &PackBox, class: u8) {
    let (sn, cn) = b.heading.to_radians().sin_cos();
    let r = 0.5 * (b.length * b.length + b.width * b.width).sqrt();
    let (x0, y0) = s.to_px(b.north + r, b.east - r);
    let (x1, y1) = s.to_px(b.north - r, b.east + r);
    for y in (y0.max(0.0) as u32)..(y1.min(s.px as f32) as u32) {
        for x in (x0.max(0.0) as u32)..(x1.min(s.px as f32) as u32) {
            let (n, e) = s.to_ne(x, y);
            let (dn, de) = (n - b.north, e - b.east);
            if (dn * cn + de * sn).abs() <= b.length * 0.5 && (-dn * sn + de * cn).abs() <= b.width * 0.5 {
                lc.put_pixel(x, y, image::Luma([class]));
            }
        }
    }
}

/// The ground station's map: the photo itself (it's a real place), a height grid relative to
/// home, and the georeference, in the same format `sim001 --export-map` writes.
fn write_gcs_assets(
    dir: &Path,
    meta: &PackMeta,
    wide: &RgbImage,
    height_at: &impl Fn(f32, f32) -> f32,
    runway: Option<&PackBox>,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let px = 2560.min(wide.width());
    image::imageops::resize(wide, px, px, image::imageops::FilterType::Triangle)
        .save(dir.join("terrain_map.png"))
        .map_err(|e| e.to_string())?;
    let n = 321;
    let s = &meta.wide;
    let mut hb = Vec::with_capacity(n * n * 4);
    let step = s.size_m / (n - 1) as f32;
    for j in 0..n {
        for i in 0..n {
            let north = s.centre_n + s.size_m * 0.5 - j as f32 * step;
            let east = s.centre_e - s.size_m * 0.5 + i as f32 * step;
            hb.extend_from_slice(&(height_at(north, east) - meta.home_alt).to_le_bytes());
        }
    }
    std::fs::write(dir.join("terrain_height.bin"), hb).map_err(|e| e.to_string())?;
    let mut txt = format!(
        "# Built by domains/visualization/mapbuild from {}\nsize_m={}\nmap_px={px}\ncentre_north={}\ncentre_east={}\nheight_n={n}\n\
home_lat={}\nhome_lon={}\nhome_alt={}\n",
        meta.title, s.size_m, s.centre_n, s.centre_e, meta.home_lat, meta.home_lon, meta.home_alt
    );
    if let Some(r) = runway {
        txt.push_str(&format!(
            "runway_north={}\nrunway_east={}\nrunway_length={}\nrunway_width={}\nrunway_heading={}\n",
            r.north, r.east, r.length, r.width, r.heading
        ));
    }
    std::fs::write(dir.join("terrain_map.txt"), txt).map_err(|e| e.to_string())
}
