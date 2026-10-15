//! A real place, loaded from a map pack built by `tools/mapbuild` (USGS 3DEP lidar elevation,
//! USGS NAIP aerial photos, trees grown from the photo's canopy, OSM runway and buildings).
//!
//! World frame as everywhere in the sim: +X east, +Y up, -Z north, in metres from home (the
//! pilot box), with y = 0 at the ground there. The ground is coloured from the photos.

use crate::terrain::{Building, Tree};
use glam::{Vec2, Vec3};
use std::path::Path;
use t1s_station::mappack::{self, PackBox, PackMeta, Square, cover};

const PHOTO_ALBEDO: f32 = 0.7;

pub struct Photo {
    sq: Square,
    /// Linear RGB, row-major.
    px: Vec<[f32; 3]>,
}

impl Photo {
    fn load(path: &Path, sq: Square) -> Result<Photo, String> {
        let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.into_rgb8();
        if img.width() != sq.px {
            return Err(format!("{}: {} px wide, expected {}", path.display(), img.width(), sq.px));
        }
        let lut: Vec<f32> = (0..256)
            .map(|i| {
                let c = i as f32 / 255.0;
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            })
            .collect();
        // Aerial photos already contain sunlight and shading; lit again by the sim they read too
        // bright, so they are toned down to albedo-like values.
        Ok(Photo { sq, px: img.pixels().map(|p| p.0.map(|v| lut[v as usize] * PHOTO_ALBEDO)).collect() })
    }

    fn sample(&self, north: f32, east: f32) -> [f32; 3] {
        let n = self.sq.px as usize;
        let (x, y) = self.sq.to_px(north, east);
        let (fx, fy) = ((x - 0.5).clamp(0.0, n as f32 - 1.001), (y - 0.5).clamp(0.0, n as f32 - 1.001));
        let (i, j) = (fx as usize, fy as usize);
        let (tx, ty) = (fx - i as f32, fy - j as f32);
        let p = |i: usize, j: usize| self.px[j * n + i];
        let (a, b, c, d) = (p(i, j), p(i + 1, j), p(i, j + 1), p(i + 1, j + 1));
        [0, 1, 2].map(|k| (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty)
    }
}

pub struct RealSite {
    pub meta: PackMeta,
    /// Ground height above home ground, on `meta.height` (sea floor pushed under the water).
    heights: Vec<f32>,
    wide: Photo,
    core: Photo,
    landcover: Vec<u8>,
    pub trees: Vec<Tree>,
    pub buildings: Vec<Building>,
    pub runway: Option<PackBox>,
    /// Sea level in world y (home ground is y = 0).
    pub water_level: f32,
    /// Half-width of the area the terrain covers, around the world origin.
    pub half_size: f32,
}

/// World (x, z) to pack (north, east).
fn ne(x: f32, z: f32) -> (f32, f32) {
    (-z, x)
}

impl RealSite {
    pub fn load(dir: &Path) -> Result<RealSite, String> {
        let meta = PackMeta::read(dir)?;
        let raw = std::fs::read(dir.join("height.f32")).map_err(|e| format!("height.f32: {e}"))?;
        let mut heights: Vec<f32> = raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) - meta.home_alt).collect();
        let n = meta.height.px as usize;
        if heights.len() != n * n {
            return Err(format!("height.f32 has {} samples, expected {}", heights.len(), n * n));
        }
        let landcover = image::open(dir.join("landcover.png")).map_err(|e| format!("landcover.png: {e}"))?.into_luma8().into_raw();
        let water_level = -meta.home_alt;
        // The elevation data holds the sea surface at ~0 m; push sea-level water cells below the
        // water plane so the terrain doesn't poke through it.
        let lc_at = |north: f32, east: f32| {
            let (x, y) = meta.wide.to_px(north, east);
            let w = meta.wide.px as usize;
            landcover[(y.clamp(0.0, w as f32 - 1.0) as usize) * w + x.clamp(0.0, w as f32 - 1.0) as usize]
        };
        for j in 0..n {
            for i in 0..n {
                let (north, east) = meta.height.to_ne(i as u32, j as u32);
                let h = &mut heights[j * n + i];
                if *h < water_level + 1.5 && lc_at(north, east) == cover::WATER {
                    *h = h.min(water_level - 1.5);
                }
            }
        }
        let wide = Photo::load(&dir.join("ortho_wide.jpg"), meta.wide)?;
        let core = Photo::load(&dir.join("ortho_core.jpg"), meta.core)?;
        let mut site = RealSite {
            half_size: 0.5 * meta.height.size_m + meta.height.centre_n.abs().max(meta.height.centre_e.abs()),
            meta,
            heights,
            wide,
            core,
            landcover,
            trees: vec![],
            buildings: vec![],
            runway: None,
            water_level,
        };
        let frac = std::env::var("T1S_TREE_FRAC").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0).clamp(0.0, 1.0);
        let pack_trees = mappack::read_trees(&dir.join("trees.bin")).map_err(|e| format!("trees.bin: {e}"))?;
        let keep = (pack_trees.len() as f32 * frac) as usize;
        site.trees = pack_trees
            .iter()
            .take(keep)
            .map(|t| {
                let (x, z) = (t.east, -t.north);
                Tree { pos: Vec3::new(x, site.height(x, z), z), height: t.height, conifer: t.conifer, shade: t.shade }
            })
            .collect();
        site.buildings = mappack::read_boxes(&dir.join("buildings.txt"))
            .iter()
            .map(|b| {
                let (x, z) = (b.east, -b.north);
                let base = site.height(x, z);
                Building {
                    center: Vec3::new(x, base + b.height * 0.5, z),
                    // Long axis along local Z, rotated by `yaw` (heading clockwise from north).
                    size: Vec3::new(b.width, b.height, b.length),
                    yaw: -b.heading.to_radians(),
                }
            })
            .collect();
        site.runway = mappack::read_boxes(&dir.join("runway.txt")).first().copied();
        Ok(site)
    }

    pub fn height(&self, x: f32, z: f32) -> f32 {
        let (north, east) = ne(x, z);
        let sq = &self.meta.height;
        let n = sq.px as usize;
        let (px, py) = sq.to_px(north, east);
        let (fx, fy) = ((px - 0.5).clamp(0.0, n as f32 - 1.001), (py - 0.5).clamp(0.0, n as f32 - 1.001));
        let (i, j) = (fx as usize, fy as usize);
        let (tx, ty) = (fx - i as f32, fy - j as f32);
        let h = |i: usize, j: usize| self.heights[j * n + i];
        let top = h(i, j) + (h(i + 1, j) - h(i, j)) * tx;
        let bot = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * tx;
        top + (bot - top) * ty
    }

    pub fn cover(&self, x: f32, z: f32) -> u8 {
        let (north, east) = ne(x, z);
        let sq = &self.meta.wide;
        let w = sq.px as usize;
        let (px, py) = sq.to_px(north, east);
        if px < 0.0 || py < 0.0 || px >= w as f32 || py >= w as f32 {
            return cover::WATER;
        }
        self.landcover[py as usize * w + px as usize]
    }

    pub fn is_water(&self, x: f32, z: f32) -> bool {
        self.cover(x, z) == cover::WATER && self.height(x, z) < self.water_level + 0.5
    }

    pub fn on_runway(&self, x: f32, z: f32) -> bool {
        self.runway.is_some_and(|r| {
            let (north, east) = ne(x, z);
            let (s, c) = r.heading.to_radians().sin_cos();
            let (dn, de) = (north - r.north, east - r.east);
            (dn * c + de * s).abs() < r.length * 0.5 && (-dn * s + de * c).abs() < r.width * 0.5
        })
    }

    /// Linear albedo from the photos (the 0.4 m one around home, else the 1 m one).
    pub fn color(&self, x: f32, z: f32) -> [f32; 3] {
        let (north, east) = ne(x, z);
        // Fade from the core photo to the wide one over its last 40 m, so the seam doesn't show.
        let half = self.core.sq.size_m * 0.5;
        let edge = (half - (north - self.core.sq.centre_n).abs().max((east - self.core.sq.centre_e).abs())) / 40.0;
        let wide = self.wide.sample(north, east);
        if edge <= 0.0 {
            return wide;
        }
        let core = self.core.sample(north, east);
        let t = edge.min(1.0);
        [0, 1, 2].map(|k| wide[k] + (core[k] - wide[k]) * t)
    }

    /// 3D grass near the camera: on open ground only, greener or drier as the photo is.
    pub fn grass(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        if self.cover(x, z) != cover::GROUND {
            return None;
        }
        let [r, g, b] = self.color(x, z);
        let green = g / (r + g + b + 1e-4);
        // Bare dirt / gravel reads as low saturation and bright.
        let bright = (r + g + b) / 3.0;
        if green < 0.33 && bright > 0.12 {
            return None;
        }
        let dry = ((0.40 - green) / 0.08).clamp(0.0, 1.0);
        Some((0.6 + 0.4 * (1.0 - dry), dry))
    }

    pub fn pilot(&self) -> Vec2 {
        Vec2::ZERO
    }
}
