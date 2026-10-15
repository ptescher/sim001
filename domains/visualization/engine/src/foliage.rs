//! Procedurally painted foliage and bark textures for the generated trees (see `treegen`).
//!
//! Needle sprays and leaf clusters are alpha-cut, with mips that preserve alpha-test coverage.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::f32::consts::PI;

fn rnd(seed: u32, k: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B1) ^ k.wrapping_mul(0x85EB_CA77) ^ 0x27d4_eb2f;
    h = (h ^ (h >> 15)).wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 13;
    (h & 0xffffff) as f32 / 0xffffff as f32
}

// ---------------------------------------------------------------- painting

/// Linear-light RGBA canvas with anti-aliased "over" compositing.
pub(crate) struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    pub(crate) fn new(w: usize, h: usize) -> Self {
        Self { w, h, px: vec![[0.0; 4]; w * h] }
    }

    fn blend(&mut self, x: usize, y: usize, c: [f32; 3], a: f32) {
        let p = &mut self.px[y * self.w + x];
        let na = a + p[3] * (1.0 - a);
        if na > 1e-6 {
            for i in 0..3 {
                p[i] = (c[i] * a + p[i] * p[3] * (1.0 - a)) / na;
            }
        }
        p[3] = na;
    }

    /// Tapered line from `a` (width wa) to `b` (width wb), colours interpolated.
    pub(crate) fn line(&mut self, a: Vec2, b: Vec2, wa: f32, wb: f32, ca: [f32; 3], cb: [f32; 3]) {
        let pad = wa.max(wb) + 1.5;
        let (x0, x1) = ((a.x.min(b.x) - pad).max(0.0) as usize, ((a.x.max(b.x) + pad) as usize).min(self.w - 1));
        let (y0, y1) = ((a.y.min(b.y) - pad).max(0.0) as usize, ((a.y.max(b.y) + pad) as usize).min(self.h - 1));
        let ab = b - a;
        let len2 = ab.length_squared().max(1e-6);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
                let d = p.distance(a + ab * t);
                let w = (wa + (wb - wa) * t) * 0.5;
                let cov = (w + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let c = [ca[0] + (cb[0] - ca[0]) * t, ca[1] + (cb[1] - ca[1]) * t, ca[2] + (cb[2] - ca[2]) * t];
                    self.blend(x, y, c, cov);
                }
            }
        }
    }

    /// Filled, rotated ellipse with a darker rim (a leaf).
    pub(crate) fn leaf(&mut self, c: Vec2, rx: f32, ry: f32, angle: f32, col: [f32; 3]) {
        let r = rx.max(ry) + 1.5;
        let (cs, sn) = (angle.cos(), angle.sin());
        for y in ((c.y - r).max(0.0) as usize)..=((c.y + r) as usize).min(self.h - 1) {
            for x in ((c.x - r).max(0.0) as usize)..=((c.x + r) as usize).min(self.w - 1) {
                let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c;
                let (lx, ly) = (d.x * cs + d.y * sn, -d.x * sn + d.y * cs);
                // Pointed tip: narrow the far end.
                let ry_here = ry * (1.0 - 0.35 * (lx / rx).max(0.0));
                let e = (lx / rx).powi(2) + (ly / ry_here.max(0.5)).powi(2);
                let cov = ((1.0 - e) * rx * 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let shade = 0.75 + 0.25 * (1.0 - e).sqrt() - 0.12 * (ly.abs() < 0.8) as i32 as f32;
                    self.blend(x, y, [col[0] * shade, col[1] * shade, col[2] * shade], cov);
                }
            }
        }
    }

    /// RGBA8 sRGB image with mips that preserve alpha-test coverage (no thinning with distance).
    pub(crate) fn into_image(mut self, cutoff: f32) -> Image {
        // Bleed average colour into transparent texels so filtering doesn't darken edges.
        let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
        for p in &self.px {
            if p[3] > 0.5 {
                for i in 0..3 {
                    sum[i] += p[i];
                }
                n += 1.0;
            }
        }
        let avg = [sum[0] / n.max(1.0), sum[1] / n.max(1.0), sum[2] / n.max(1.0)];
        for p in &mut self.px {
            let a = p[3].clamp(0.0, 1.0);
            for i in 0..3 {
                p[i] = p[i] * a + avg[i] * (1.0 - a);
            }
        }
        let coverage = |px: &[[f32; 4]], scale: f32| px.iter().filter(|p| p[3] * scale > cutoff).count() as f32 / px.len() as f32;
        let target = coverage(&self.px, 1.0);
        let to_srgb = |c: f32| {
            let c = c.clamp(0.0, 1.0);
            let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
            (s * 255.0 + 0.5) as u8
        };
        let (mut w, mut h) = (self.w, self.h);
        let mut level = self.px;
        let mut data = Vec::new();
        let mut mips = 0;
        loop {
            // Rescale alpha so the alpha-tested area matches mip 0.
            let scale = if mips == 0 {
                1.0
            } else {
                let (mut lo, mut hi) = (0.5f32, 8.0f32);
                for _ in 0..16 {
                    let mid = 0.5 * (lo + hi);
                    if coverage(&level, mid) < target {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                hi
            };
            for p in &level {
                data.extend_from_slice(&[to_srgb(p[0]), to_srgb(p[1]), to_srgb(p[2]), ((p[3] * scale).clamp(0.0, 1.0) * 255.0) as u8]);
            }
            mips += 1;
            if w == 1 && h == 1 {
                break;
            }
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut next = vec![[0.0f32; 4]; nw * nh];
            for y in 0..nh {
                for x in 0..nw {
                    let mut acc = [0.0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = level[((y * 2 + dy).min(h - 1)) * w + (x * 2 + dx).min(w - 1)];
                        for i in 0..4 {
                            acc[i] += p[i] * 0.25;
                        }
                    }
                    next[y * nw + x] = acc;
                }
            }
            level = next;
            w = nw;
            h = nh;
        }
        let mip0 = self.w * self.h * 4;
        let mut img = Image::new(
            Extent3d { width: self.w as u32, height: self.h as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data[..mip0].to_vec(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        img.data = Some(data);
        img.texture_descriptor.mip_level_count = mips;
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 8,
            ..ImageSamplerDescriptor::linear()
        });
        img
    }
}

/// A spruce spray: woody twig along +u with side twiglets, densely covered in needles.
pub fn spruce_spray_texture(seed: u32) -> Image {
    let (w, h) = (512usize, 256usize);
    let mut c = Canvas::new(w, h);
    let twig = [0.06, 0.04, 0.025];
    let mid = h as f32 * 0.5;
    let main_end = Vec2::new(w as f32 * 0.97, mid + (rnd(seed, 1) - 0.5) * 20.0);
    let main_start = Vec2::new(4.0, mid);
    let mut stems: Vec<(Vec2, Vec2, f32)> = vec![(main_start, main_end, 1.0)];
    let mut k = 0;
    let mut t = 0.06;
    while t < 0.92 {
        let base = main_start.lerp(main_end, t);
        for side in [-1.0f32, 1.0] {
            k += 1;
            let ang = (48.0 + 18.0 * rnd(seed, k)) .to_radians() * side;
            let len = h as f32 * (0.42 - 0.26 * t) * (0.8 + 0.4 * rnd(seed, k + 500));
            let tip = base + Vec2::new(ang.cos(), ang.sin()) * len;
            stems.push((base, tip, 0.6));
        }
        t += 0.07 + 0.03 * rnd(seed, k + 900);
    }
    for (a, b, wscale) in &stems {
        c.line(*a, *b, 3.2 * wscale, 1.2 * wscale, twig, twig);
    }
    // Needles: short strokes from each stem, older/darker near the base, fresh tips lighter.
    let dark = [0.018, 0.045, 0.022];
    let fresh = [0.055, 0.115, 0.045];
    let mut n = 0u32;
    for (si, (a, b, _)) in stems.iter().enumerate() {
        let dir = (*b - *a).normalize_or(Vec2::X);
        let len = a.distance(*b);
        let steps = (len / 2.4) as u32;
        for i in 0..steps {
            let f = i as f32 / steps.max(1) as f32;
            let p = a.lerp(*b, f);
            for side in [-1.0f32, 1.0] {
                n += 1;
                let ang = (35.0 + 45.0 * rnd(seed, n)).to_radians() * side;
                let nd = Vec2::new(dir.x * ang.cos() - dir.y * ang.sin(), dir.x * ang.sin() + dir.y * ang.cos());
                let nl = (11.0 + 7.0 * rnd(seed, n + 7000)) * if si == 0 { 1.15 } else { 0.9 };
                let tipness = (f * (if si == 0 { 0.6 } else { 1.0 }) + 0.25 * rnd(seed, n + 13000)).clamp(0.0, 1.0);
                let col = [
                    dark[0] + (fresh[0] - dark[0]) * tipness,
                    dark[1] + (fresh[1] - dark[1]) * tipness,
                    dark[2] + (fresh[2] - dark[2]) * tipness,
                ];
                let shade = 0.8 + 0.4 * rnd(seed, n + 21000);
                let col = [col[0] * shade, col[1] * shade, col[2] * shade];
                c.line(p, p + nd * nl, 1.8, 1.0, col, [col[0] * 1.2, col[1] * 1.2, col[2] * 1.15]);
            }
        }
    }
    c.into_image(0.5)
}

/// An aspen leaf cluster: twigs radiating from the bottom centre carrying oval leaves.
pub fn aspen_leaf_texture(seed: u32) -> Image {
    let s = 512usize;
    let mut c = Canvas::new(s, s);
    let root = Vec2::new(s as f32 * 0.5, s as f32 * 0.97);
    let twig = [0.10, 0.08, 0.06];
    let mut k = 0;
    for b in 0..7 {
        let ang = -PI * 0.5 + (b as f32 - 3.0) * 0.32 + (rnd(seed, b) - 0.5) * 0.2;
        let len = s as f32 * (0.55 + 0.3 * rnd(seed, b + 50));
        let dir = Vec2::new(ang.cos(), ang.sin());
        let tip = root + dir * len;
        c.line(root, tip, 4.0, 1.5, twig, twig);
        let leaves = 6 + (rnd(seed, b + 90) * 5.0) as u32;
        for l in 0..leaves {
            k += 1;
            let f = 0.25 + 0.75 * (l as f32 + rnd(seed, k)) / leaves as f32;
            let p = root + dir * len * f;
            let side = if l % 2 == 0 { 1.0 } else { -1.0 };
            let la = ang + side * (0.6 + 0.6 * rnd(seed, k + 300));
            let ld = Vec2::new(la.cos(), la.sin());
            let size = 22.0 + 14.0 * rnd(seed, k + 600);
            let petiole = p + ld * size * 0.5;
            c.line(p, petiole, 1.4, 1.0, twig, twig);
            let g = rnd(seed, k + 900);
            let col = [0.07 + 0.08 * g, 0.16 + 0.07 * g, 0.03 + 0.01 * g];
            c.leaf(petiole + ld * size * 0.9, size, size * 0.78, la, col);
        }
    }
    c.into_image(0.5)
}

pub fn bark_spruce_texture() -> Image {
    crate::mesh_util::texture(256, |u, v| {
        let fiss = crate::terrain::fbm(u * 24.0, v * 3.0, 4);
        let grain = crate::terrain::fbm(u * 60.0, v * 60.0, 2);
        let c = 0.035 + 0.05 * fiss + 0.015 * grain;
        [c * 1.15, c * 0.95, c * 0.8]
    })
}

pub fn bark_aspen_texture() -> Image {
    crate::mesh_util::texture(256, |u, v| {
        let base = 0.50 + 0.08 * crate::terrain::fbm(u * 8.0, v * 30.0, 3);
        // Dark "eyes" where branches fell off.
        let eye = ((crate::terrain::fbm(u * 6.0 + 3.0, v * 14.0, 3) - 0.68) * 9.0).clamp(0.0, 1.0);
        let c = base * (1.0 - 0.8 * eye);
        [c, c * 1.0, c * 0.93]
    })
}
