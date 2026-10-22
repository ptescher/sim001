//! Procedural mesh helpers. Every mesh produced here is Solari-compatible:
//! TriangleList, U32 indices, POSITION/NORMAL/UV_0/TANGENT.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::DVec3;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

#[derive(Default)]
pub struct GeoBuilder {
    pub pos: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub idx: Vec<u32>,
}

impl GeoBuilder {
    pub fn vert(&mut self, p: Vec3, uv: Vec2) -> u32 {
        self.pos.push(p);
        self.uv.push(uv);
        (self.pos.len() - 1) as u32
    }

    /// Adds a triangle, flipping it if needed so its normal faces `outward`.
    pub fn tri(&mut self, a: u32, b: u32, c: u32, outward: Vec3) {
        let (pa, pb, pc) = (self.pos[a as usize], self.pos[b as usize], self.pos[c as usize]);
        let n = (pb - pa).cross(pc - pa);
        if n.length_squared() < 1e-14 {
            return;
        }
        if n.dot(outward) >= 0.0 {
            self.idx.extend([a, b, c]);
        } else {
            self.idx.extend([a, c, b]);
        }
    }

    pub fn quad(&mut self, a: u32, b: u32, c: u32, d: u32, outward: Vec3) {
        self.tri(a, b, c, outward);
        self.tri(a, c, d, outward);
    }

    pub fn build(self) -> Mesh {
        let pos: Vec<[f32; 3]> = self.pos.iter().map(|p| p.to_array()).collect();
        let uv: Vec<[f32; 2]> = self.uv.iter().map(|p| p.to_array()).collect();
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, smooth_normals(&self.pos, &self.idx))
            .with_inserted_indices(Indices::U32(self.idx))
            .with_generated_tangents()
            .expect("tangent generation")
    }
}

/// Smooth vertex normals, each face counting by the angle it makes at the vertex. Done in double
/// precision: a model in metres has triangles of a few square millimetres, whose normals
/// underflow single precision, and a vertex with no normal leaves a hole in the shading.
fn smooth_normals(pos: &[Vec3], idx: &[u32]) -> Vec<[f32; 3]> {
    let mut acc = vec![DVec3::ZERO; pos.len()];
    let p = |i: u32| pos[i as usize].as_dvec3();
    for t in idx.as_chunks::<3>().0 {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let face = (b - a).cross(c - a).normalize_or_zero();
        for (k, (o, x, y)) in [(a, b, c), (b, c, a), (c, a, b)].into_iter().enumerate() {
            let angle = (x - o).normalize_or_zero().dot((y - o).normalize_or_zero()).clamp(-1.0, 1.0).acos();
            acc[t[k] as usize] += face * angle;
        }
    }
    acc.iter().map(|n| n.normalize_or(DVec3::Y).as_vec3().to_array()).collect()
}

/// Body of revolution-ish loft along +Z. Stations: (z, half_width, half_height, center_y).
pub fn loft(stations: &[(f32, f32, f32, f32)], center_x: f32, segments: usize, cap_ends: bool) -> Mesh {
    loft_n(stations, center_x, segments, cap_ends, 2.0)
}

/// A loft whose sections are `|x|^n + |y|^n = 1`: an ellipse for 2, a rounded rectangle for more.
pub fn loft_n(stations: &[(f32, f32, f32, f32)], center_x: f32, segments: usize, cap_ends: bool, n: f32) -> Mesh {
    let mut b = GeoBuilder::default();
    let mut rings = Vec::new();
    for (si, &(z, rx, ry, cy)) in stations.iter().enumerate() {
        let mut ring = Vec::new();
        for j in 0..=segments {
            let t = j as f32 / segments as f32 * std::f32::consts::TAU;
            let (c, s) = (t.cos(), t.sin());
            let p = Vec3::new(
                center_x + rx * c.signum() * c.abs().powf(2.0 / n),
                cy + ry * s.signum() * s.abs().powf(2.0 / n),
                z,
            );
            ring.push(b.vert(p, Vec2::new(j as f32 / segments as f32, si as f32 / (stations.len() - 1) as f32)));
        }
        rings.push(ring);
    }
    for i in 0..stations.len() - 1 {
        for j in 0..segments {
            let (a, bb, c, d) = (rings[i][j], rings[i][j + 1], rings[i + 1][j + 1], rings[i + 1][j]);
            let mid = (b.pos[a as usize] + b.pos[c as usize]) * 0.5;
            let cy = 0.5 * (stations[i].3 + stations[i + 1].3);
            let out = Vec3::new(mid.x - center_x, mid.y - cy, 0.0);
            // Use the slope of the loft for nose/tail-cone tris.
            let dz = (stations[i].1 - stations[i + 1].1).signum() * 0.2;
            b.quad(a, bb, c, d, out.normalize_or_zero() + Vec3::Z * dz);
        }
    }
    if cap_ends {
        for (si, dir) in [(0usize, -1.0f32), (stations.len() - 1, 1.0)] {
            let (z, _, _, cy) = stations[si];
            let center = b.vert(Vec3::new(center_x, cy, z), Vec2::splat(0.5));
            let ring: Vec<u32> = rings[si]
                .iter()
                .map(|&v| {
                    let p = b.pos[v as usize];
                    b.vert(p, Vec2::splat(0.5))
                })
                .collect();
            for j in 0..segments {
                b.tri(center, ring[j], ring[j + 1], Vec3::Z * dir);
            }
        }
    }
    b.build()
}

/// Cambered airfoil (NACA 4-digit style): returns (camber, half thickness) at chord fraction x.
fn airfoil(x: f32, thickness: f32, camber: f32) -> (f32, f32) {
    let p = 0.35;
    let yt =
        5.0 * thickness * (0.2969 * x.sqrt() - 0.126 * x - 0.3516 * x * x + 0.2843 * x.powi(3) - 0.1036 * x.powi(4));
    let yc = if x < p {
        camber / (p * p) * (2.0 * p * x - x * x)
    } else {
        camber / ((1.0 - p) * (1.0 - p)) * ((1.0 - 2.0 * p) + 2.0 * p * x - x * x)
    };
    (yc, yt.max(0.0))
}

pub struct PanelSpec {
    pub root_le: Vec3,
    pub tip_le: Vec3,
    pub root_chord: f32,
    pub tip_chord: f32,
    /// Panel "up" direction (normal of the chord plane).
    pub up: Vec3,
    /// Chord-fraction range of this piece (e.g. 0..0.74 for the wing, 0.74..1 for the aileron).
    pub x0: f32,
    pub x1: f32,
    pub thickness: f32,
    pub camber: f32,
    /// Subtracted from all positions (e.g. a hinge point).
    pub origin: Vec3,
}

/// Builds a lifting-surface piece as (top skin, bottom skin + edge caps).
pub fn airfoil_panel(s: &PanelSpec) -> (Mesh, Mesh) {
    let n = 14;
    let xs: Vec<f32> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let x = 0.5 * (1.0 - (std::f32::consts::PI * t).cos());
            s.x0 + (s.x1 - s.x0) * x
        })
        .collect();
    let span_dir = (s.tip_le - s.root_le).normalize();
    let point = |k: usize, x: f32, top: bool| -> Vec3 {
        let (le, c) = if k == 0 { (s.root_le, s.root_chord) } else { (s.tip_le, s.tip_chord) };
        let (yc, yt) = airfoil(x, s.thickness, s.camber);
        // Keep a finite trailing edge thickness so the TE is not a knife edge.
        let yt = yt.max(0.004 / c);
        let y = if top { yc + yt } else { yc - yt };
        le + Vec3::Z * x * c + s.up * y * c - s.origin
    };

    let mut top = GeoBuilder::default();
    let mut bot = GeoBuilder::default();
    for (mb, is_top) in [(&mut top, true), (&mut bot, false)] {
        let mut rows = [Vec::new(), Vec::new()];
        for (k, row) in rows.iter_mut().enumerate() {
            for (i, &x) in xs.iter().enumerate() {
                row.push(mb.vert(point(k, x, is_top), Vec2::new(i as f32 / n as f32, k as f32)));
            }
        }
        let out = if is_top { s.up } else { -s.up };
        for i in 0..n {
            mb.quad(rows[0][i], rows[0][i + 1], rows[1][i + 1], rows[1][i], out);
        }
    }

    // Root/tip caps and chordwise cut faces go into the bottom mesh.
    for (k, dir) in [(0usize, -span_dir), (1, span_dir)] {
        let ids: Vec<(u32, u32)> = xs
            .iter()
            .map(|&x| (bot.vert(point(k, x, true), Vec2::ZERO), bot.vert(point(k, x, false), Vec2::ZERO)))
            .collect();
        for i in 0..n {
            bot.quad(ids[i].0, ids[i + 1].0, ids[i + 1].1, ids[i].1, dir);
        }
    }
    for (x, dir) in [(s.x0, -Vec3::Z), (s.x1, Vec3::Z)] {
        let a = bot.vert(point(0, x, true), Vec2::ZERO);
        let b = bot.vert(point(1, x, true), Vec2::ZERO);
        let c = bot.vert(point(1, x, false), Vec2::ZERO);
        let d = bot.vert(point(0, x, false), Vec2::ZERO);
        bot.quad(a, b, c, d, dir);
    }
    (top.build(), bot.build())
}

pub fn cuboid(size: Vec3) -> Mesh {
    Cuboid::new(size.x, size.y, size.z).mesh().build().with_generated_tangents().unwrap()
}

pub fn finish(mesh: Mesh) -> Mesh {
    mesh.with_generated_tangents().unwrap()
}

/// Builds an sRGB texture with a full CPU-generated mip chain and a repeating, anisotropic sampler.
pub fn texture(size: u32, f: impl Fn(f32, f32) -> [f32; 3]) -> Image {
    let mut level: Vec<[f32; 3]> = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            level.push(f(x as f32 / size as f32, y as f32 / size as f32));
        }
    }
    let to_srgb = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    let mut data = Vec::new();
    let mut s = size;
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
        let mut next = Vec::with_capacity((ns * ns) as usize);
        for y in 0..ns {
            for x in 0..ns {
                let mut acc = [0.0f32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = level[((y * 2 + dy) * s + x * 2 + dx) as usize];
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
    // `Image::new` validates the data against mip 0 only, so attach the full chain afterwards.
    let mip0_len = (size * size * 4) as usize;
    let mut image = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data[..mip0_len].to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// Tangent-space normal map (linear RGBA8) from a tileable height function on [0,1)^2,
/// with a mip chain and a repeating sampler.
pub fn normal_map(size: u32, strength: f32, height: impl Fn(f32, f32) -> f32 + Sync) -> Image {
    let s = size as usize;
    let h: Vec<f32> = (0..s * s).map(|i| height((i % s) as f32 / size as f32, (i / s) as f32 / size as f32)).collect();
    let at = |x: isize, y: isize| h[(y.rem_euclid(s as isize) as usize) * s + x.rem_euclid(s as isize) as usize];
    let mut level: Vec<Vec3> = (0..s * s)
        .map(|i| {
            let (x, y) = ((i % s) as isize, (i / s) as isize);
            let dx = (at(x + 1, y) - at(x - 1, y)) * strength;
            let dy = (at(x, y + 1) - at(x, y - 1)) * strength;
            Vec3::new(-dx, -dy, 1.0).normalize()
        })
        .collect();
    let enc = |v: f32| ((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut data = Vec::new();
    let mut n = s;
    let mut mips = 0;
    loop {
        for v in &level {
            data.extend_from_slice(&[enc(v.x), enc(v.y), enc(v.z), 255]);
        }
        mips += 1;
        if n == 1 {
            break;
        }
        let m = n / 2;
        level = (0..m * m)
            .map(|i| {
                let (x, y) = (i % m, i / m);
                (level[2 * y * n + 2 * x]
                    + level[2 * y * n + 2 * x + 1]
                    + level[(2 * y + 1) * n + 2 * x]
                    + level[(2 * y + 1) * n + 2 * x + 1])
                    .normalize_or(Vec3::Z)
            })
            .collect();
        n = m;
    }
    let mut image = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data[..s * s * 4].to_vec(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

#[cfg(test)]
mod tests {
    /// Parts of a model in metres have triangles of a few square millimetres. Their vertices
    /// still get unit normals: a zero one turns the shading into a hole.
    #[test]
    fn tiny_triangles_still_get_unit_normals() {
        use super::*;
        for size in [1.0f32, 1e-2, 3e-3, 1e-3] {
            let mut b = GeoBuilder::default();
            let v: Vec<u32> = [Vec3::ZERO, Vec3::X * size, Vec3::new(size, size, 0.0), Vec3::Y * size]
                .iter()
                .map(|p| b.vert(*p, Vec2::ZERO))
                .collect();
            b.quad(v[0], v[1], v[2], v[3], Vec3::Z);
            let m = b.build();
            let n = m.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(|a| a.as_float3()).unwrap();
            for p in n {
                assert!((Vec3::from(*p).length() - 1.0).abs() < 1e-3, "size {size}: normal {p:?}");
                assert!(p[2] > 0.99, "size {size}: normal {p:?} does not face out");
            }
        }
    }

    #[test]
    fn mip_chain_keeps_colour() {
        let img = super::texture(64, |_, _| [0.1, 0.3, 0.05]);
        let data = img.data.as_ref().unwrap();
        let mip0 = &data[0..4];
        let last = &data[data.len() - 4..];
        println!("mips {} len {} mip0 {:?} last {:?}", img.texture_descriptor.mip_level_count, data.len(), mip0, last);
        assert_eq!(mip0, last);
    }
}
