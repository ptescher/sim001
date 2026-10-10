//! Procedural tree generator: recursive branches grown section by section (taper, gnarl,
//! twist, gravity/phototropism), with foliage cards on the outer twigs.
//!
//! Output per tree: a wood mesh (trunk + branches; Solari-compatible, ray traced) and a
//! foliage-card mesh (alpha-tested needles/leaves; raster only). Trees are generated at real
//! scale (`height` metres) and instanced with a scale factor.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::f32::consts::{PI, TAU};

const GOLDEN_ANGLE: f32 = 2.399_963;

struct Rng(u32);

impl Rng {
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 & 0xffffff) as f32 / 0xffffff as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

#[derive(Default)]
struct MeshData {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl MeshData {
    fn build(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv)
            .with_inserted_indices(Indices::U32(self.idx))
            .with_generated_tangents()
            .expect("tree tangents")
    }
}

/// One node along a grown branch.
#[derive(Clone, Copy)]
struct Node {
    pos: Vec3,
    dir: Vec3,
    radius: f32,
}

#[allow(dead_code)]
struct Branch {
    origin: Vec3,
    dir: Vec3,
    length: f32,
    radius: f32,
    level: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Species {
    Spruce,
    Aspen,
}

pub struct TreeMeshes {
    pub wood: Mesh,
    pub foliage: Mesh,
}

/// Grows a branch: returns its nodes. Forces bend the direction each section.
fn grow(b: &Branch, sections: usize, taper: f32, gnarl: f32, force: Vec3, curl_up: f32, rng: &mut Rng) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(sections + 1);
    let mut p = b.origin;
    let mut d = b.dir.normalize();
    let step = b.length / sections as f32;
    for i in 0..=sections {
        let t = i as f32 / sections as f32;
        let r = b.radius * (1.0 - taper * t).max(0.02);
        nodes.push(Node { pos: p, dir: d, radius: r });
        // Random wobble, then external force (stronger on thin outer parts), tip curl.
        let wobble = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)) * gnarl;
        d = (d + wobble + force * (0.4 + t) + Vec3::Y * curl_up * t * t).normalize();
        p += d * step;
    }
    nodes
}

/// Adds a tube along `nodes` to the wood mesh.
fn tube(m: &mut MeshData, nodes: &[Node], segs: usize, v_scale: f32) {
    let base = m.pos.len() as u32;
    let mut v = 0.0;
    for (i, n) in nodes.iter().enumerate() {
        if i > 0 {
            v += nodes[i - 1].pos.distance(n.pos) * v_scale;
        }
        let side = n.dir.any_orthonormal_vector();
        let up = n.dir.cross(side);
        for s in 0..=segs {
            let a = s as f32 / segs as f32 * TAU;
            let o = side * a.cos() + up * a.sin();
            m.pos.push((n.pos + o * n.radius).to_array());
            m.nrm.push(o.to_array());
            m.uv.push([s as f32 / segs as f32, v]);
        }
    }
    let ring = segs as u32 + 1;
    for i in 0..nodes.len() as u32 - 1 {
        for s in 0..segs as u32 {
            let a = base + i * ring + s;
            let b = a + ring;
            m.idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
}

/// A foliage card strip along `spine`, spread along `across`, normals bent away from `centre`.
fn card(m: &mut MeshData, spine: &[Vec3], across: Vec3, widths: &[f32], centre: impl Fn(Vec3) -> Vec3) {
    let base = m.pos.len() as u32;
    let n = spine.len();
    for i in 0..n {
        let v = i as f32 / (n - 1) as f32;
        for (side, u) in [(-0.5f32, 0.0f32), (0.5, 1.0)] {
            let p = spine[i] + across * widths[i] * side;
            let out = p - centre(p);
            let bent = (out + Vec3::Y * 0.3 * out.length()).normalize_or(Vec3::Y);
            m.pos.push(p.to_array());
            m.nrm.push(bent.to_array());
            m.uv.push([v, u]);
        }
    }
    for i in 0..n as u32 - 1 {
        let a = base + i * 2;
        m.idx.extend_from_slice(&[a, a + 2, a + 1, a + 1, a + 2, a + 3]);
    }
}

/// Point and direction a fraction `t` along a grown branch.
fn sample(nodes: &[Node], t: f32) -> Node {
    let f = t.clamp(0.0, 1.0) * (nodes.len() - 1) as f32;
    let i = (f as usize).min(nodes.len() - 2);
    let k = f - i as f32;
    let (a, b) = (nodes[i], nodes[i + 1]);
    Node { pos: a.pos.lerp(b.pos, k), dir: a.dir.lerp(b.dir, k).normalize(), radius: a.radius + (b.radius - a.radius) * k }
}

/// Rotate `dir` away from `parent` by `angle`, around azimuth `az` (about the parent axis).
fn child_dir(parent: Vec3, az: f32, angle: f32) -> Vec3 {
    let side = parent.any_orthonormal_vector();
    let radial = Quat::from_axis_angle(parent, az) * side;
    (parent * angle.cos() + radial * angle.sin()).normalize()
}

pub fn generate(species: Species, seed: u32, height: f32) -> TreeMeshes {
    let mut rng = Rng(seed.wrapping_mul(2_654_435_761).max(1));
    let mut wood = MeshData::default();
    let mut leaves = MeshData::default();
    let axis = |p: Vec3| Vec3::new(0.0, p.y, 0.0);
    match species {
        Species::Spruce => {
            let trunk = Branch { origin: Vec3::ZERO, dir: Vec3::Y, length: height * 1.02, radius: height * 0.0125, level: 0 };
            let trunk_nodes = grow(&trunk, 14, 0.97, 0.012, Vec3::ZERO, 0.0, &mut rng);
            tube(&mut wood, &trunk_nodes, 9, 0.5);
            let crown_base = 0.12;
            let count = 150;
            let max_len = height * 0.23;
            for k in 0..count {
                let t = crown_base + (1.0 - crown_base) * (k as f32 + rng.f() * 0.5) / count as f32;
                let at = sample(&trunk_nodes, t);
                let from_top = 1.0 - t;
                // Conical crown; upswept near the top, drooping below.
                let len = max_len * from_top.powf(0.8) * rng.range(0.8, 1.15) + height * 0.015;
                let angle = (55.0 + 50.0 * from_top + rng.range(-8.0, 8.0)).to_radians();
                let az = k as f32 * GOLDEN_ANGLE + rng.range(-0.2, 0.2);
                let dir = child_dir(Vec3::Y, az, angle);
                let b = Branch { origin: at.pos, dir, length: len, radius: (at.radius * 0.35).max(height * 0.0012), level: 1 };
                let gravity = Vec3::NEG_Y * (0.06 + 0.08 * from_top);
                let nodes = grow(&b, 5, 0.85, 0.05, gravity, 0.25 * from_top, &mut rng);
                tube(&mut wood, &nodes, 4, 2.0);
                // Spray along the branch itself (two crossed cards).
                let spine: Vec<Vec3> = nodes.iter().map(|n| n.pos).collect();
                let flat = nodes[nodes.len() / 2].dir.cross(Vec3::Y).normalize_or(Vec3::X);
                let widths: Vec<f32> = (0..nodes.len()).map(|i| len * 0.75 * (1.0 - 0.4 * i as f32 / nodes.len() as f32)).collect();
                card(&mut leaves, &spine, flat, &widths, axis);
                let steep = (flat * 0.4 + Vec3::Y * 0.9).normalize();
                card(&mut leaves, &spine, steep, &widths, axis);
                // Secondary twigs alternating left/right, roughly horizontal.
                let twigs = 5 + (len / height * 45.0) as u32;
                for j in 0..twigs {
                    let tt = 0.2 + 0.75 * (j as f32 + rng.f() * 0.5) / twigs as f32;
                    let p = sample(&nodes, tt);
                    let side = if j % 2 == 0 { 1.0 } else { -1.0 };
                    let lateral = p.dir.cross(Vec3::Y).normalize_or(Vec3::X) * side;
                    let tdir = (p.dir * 0.55 + lateral * 0.8 + Vec3::NEG_Y * 0.15).normalize();
                    let tlen = len * (0.42 - 0.25 * tt) * rng.range(0.8, 1.2);
                    let tb = Branch { origin: p.pos, dir: tdir, length: tlen, radius: 0.0, level: 2 };
                    let tn = grow(&tb, 2, 0.6, 0.06, Vec3::NEG_Y * 0.08, 0.1, &mut rng);
                    let spine: Vec<Vec3> = tn.iter().map(|n| n.pos).collect();
                    let w = tlen * 1.25;
                    let flat = tdir.cross(Vec3::Y).normalize_or(Vec3::X);
                    card(&mut leaves, &spine, flat, &[w, w * 0.85, w * 0.6], axis);
                    card(&mut leaves, &spine, (flat * 0.3 + Vec3::Y).normalize(), &[w * 0.9, w * 0.75, w * 0.5], axis);
                }
            }
            // Leader at the top.
            let top = trunk_nodes.last().unwrap().pos;
            for a in [0.0f32, PI * 0.5] {
                let across = Vec3::new(a.cos(), 0.0, a.sin());
                let spine = [top - Vec3::Y * height * 0.08, top - Vec3::Y * height * 0.03, top + Vec3::Y * height * 0.02];
                card(&mut leaves, &spine, across, &[height * 0.06, height * 0.04, height * 0.015], axis);
            }
        }
        Species::Aspen => {
            let lean = Vec3::new(rng.range(-0.04, 0.04), 1.0, rng.range(-0.04, 0.04));
            let trunk = Branch { origin: Vec3::ZERO, dir: lean, length: height * 0.88, radius: height * 0.011, level: 0 };
            let trunk_nodes = grow(&trunk, 12, 0.8, 0.02, Vec3::Y * 0.02, 0.0, &mut rng);
            tube(&mut wood, &trunk_nodes, 8, 0.35);
            let crown_centre = Vec3::new(0.0, height * 0.66, 0.0);
            let centre = move |_: Vec3| crown_centre;
            let count = 14;
            for k in 0..count {
                let t = 0.42 + 0.5 * (k as f32 + rng.f()) / count as f32;
                let at = sample(&trunk_nodes, t);
                let len = height * rng.range(0.16, 0.26) * (1.15 - 0.6 * (t - 0.42));
                let dir = child_dir(at.dir, k as f32 * GOLDEN_ANGLE, rng.range(30.0, 50.0).to_radians());
                let b = Branch { origin: at.pos, dir, length: len, radius: at.radius * 0.38, level: 1 };
                let nodes = grow(&b, 4, 0.8, 0.12, Vec3::Y * 0.06, 0.0, &mut rng);
                tube(&mut wood, &nodes, 5, 1.0);
                for j in 0..7 {
                    let tt = 0.3 + 0.65 * (j as f32 + rng.f()) / 7.0;
                    let p = sample(&nodes, tt);
                    let tdir = child_dir(p.dir, rng.f() * TAU, rng.range(25.0, 55.0).to_radians());
                    let tb = Branch { origin: p.pos, dir: tdir, length: len * 0.45, radius: p.radius * 0.5, level: 2 };
                    let tn = grow(&tb, 3, 0.7, 0.15, Vec3::Y * 0.05, 0.0, &mut rng);
                    tube(&mut wood, &tn, 3, 2.0);
                    // Leaf clusters around and beyond the twig.
                    for c in 0..7 {
                        let q = sample(&tn, 0.2 + 0.14 * c as f32);
                        let size = height * rng.range(0.085, 0.125);
                        let out = (q.pos - crown_centre).normalize_or(Vec3::Y);
                        let twist = rng.f() * TAU;
                        let tangent = out.cross(Vec3::Y).normalize_or(Vec3::X);
                        let across = (tangent * twist.cos() + out.cross(tangent) * twist.sin()).normalize();
                        let along = (out * 0.5 + Vec3::Y * 0.6 + q.dir * 0.4).normalize();
                        let spine = [q.pos - along * size * 0.2, q.pos + along * size * 0.9];
                        card(&mut leaves, &spine, across, &[size, size], centre);
                    }
                }
            }
        }
    }
    TreeMeshes { wood: wood.build(), foliage: leaves.build() }
}
