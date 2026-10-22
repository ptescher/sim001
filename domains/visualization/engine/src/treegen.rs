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
    /// A dead spruce standing: bare trunk and stubs of branches.
    Snag,
    /// A thicket of leafy stems (willow, juniper).
    Shrub,
}

/// How a spruce's top ended up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Top {
    Whole,
    /// Snapped off by wind or snow, leaving a thick stub.
    Broken,
    /// A second leader took over beside the first.
    Forked,
}

/// What makes one spruce unlike the next, from its seed alone: so that the full tree and the
/// mid-range one, grown from the same seed, are the same tree.
struct Character {
    /// Where on the trunk (0 to 1) the lowest living branches start.
    crown_base: f32,
    /// The share of the lowest branches that have died and are bare.
    dead_lower: f32,
    /// Which way, and how much, the trunk leans.
    lean: Vec3,
    top: Top,
}

fn character(seed: u32) -> Character {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B1).wrapping_add(0x7F4A_7C15).max(1));
    for _ in 0..4 {
        r.f();
    }
    let crown_base = r.range(0.16, 0.40);
    let dead_lower = r.range(0.0, 0.45);
    // Most stand straight; some lean, as trees do on a slope or in the wind.
    let (lean_size, lean_az) =
        if r.f() < 0.3 { (r.range(0.04, 0.11), r.f() * TAU) } else { (r.range(0.0, 0.012), r.f() * TAU) };
    let top = match r.f() {
        x if x < 0.14 => Top::Broken,
        x if x < 0.26 => Top::Forked,
        _ => Top::Whole,
    };
    Character { crown_base, dead_lower, lean: Vec3::new(lean_az.cos(), 0.0, lean_az.sin()) * lean_size, top }
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
    Node {
        pos: a.pos.lerp(b.pos, k),
        dir: a.dir.lerp(b.dir, k).normalize(),
        radius: a.radius + (b.radius - a.radius) * k,
    }
}

/// Rotate `dir` away from `parent` by `angle`, around azimuth `az` (about the parent axis).
fn child_dir(parent: Vec3, az: f32, angle: f32) -> Vec3 {
    let side = parent.any_orthonormal_vector();
    let radial = Quat::from_axis_angle(parent, az) * side;
    (parent * angle.cos() + radial * angle.sin()).normalize()
}

/// How much of a tree is grown: all of it for close up, or a tenth of the branches (and no
/// twigs of wood) for the middle distance, where the needles are what is seen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lod {
    Full,
    Mid,
    /// Fewer again: for the thousands of saplings, a few hundred triangles each.
    Low,
}

pub fn generate(species: Species, seed: u32, height: f32) -> TreeMeshes {
    generate_lod(species, seed, height, Lod::Full)
}

pub fn generate_lod(species: Species, seed: u32, height: f32, lod: Lod) -> TreeMeshes {
    let mid = lod != Lod::Full;
    let low = lod == Lod::Low;
    let mut rng = Rng(seed.wrapping_mul(2_654_435_761).max(1));
    let mut wood = MeshData::default();
    let mut leaves = MeshData::default();
    let axis = |p: Vec3| Vec3::new(0.0, p.y, 0.0);
    match species {
        Species::Spruce => {
            let ch = character(seed);
            let length = match ch.top {
                Top::Broken => height * rng.range(0.78, 0.9),
                _ => height * 1.02,
            };
            let trunk = Branch {
                origin: Vec3::ZERO,
                dir: Vec3::new(ch.lean.x, 1.0, ch.lean.z),
                length,
                radius: height * 0.0125,
                level: 0,
            };
            let taper = if ch.top == Top::Broken { 0.55 } else { 0.97 };
            let trunk_nodes = grow(&trunk, if mid { 6 } else { 14 }, taper, 0.012, ch.lean * 0.3, 0.0, &mut rng);
            tube(&mut wood, &trunk_nodes, if mid { 6 } else { 9 }, 0.5);
            // The wider sprays of the mid-range tree hang lower, so it starts a little higher.
            let crown_base = if low { ch.crown_base * 0.25 } else { ch.crown_base + if mid { 0.05 } else { 0.0 } };
            let count = if low {
                40
            } else if mid {
                58
            } else {
                150
            };
            // Fewer branches, each carrying a wider spray, so that the crown is as full.
            let fill = if low {
                3.0
            } else if mid {
                1.8
            } else {
                1.0
            };
            let max_len = height * 0.23;
            for k in 0..count {
                let t = crown_base + (1.0 - crown_base) * (k as f32 + rng.f() * 0.5) / count as f32;
                let at = sample(&trunk_nodes, t);
                let from_top = 1.0 - t;
                // Conical crown; upswept near the top, drooping below.
                // The lowest branches are the first to die: bare, and broken short.
                // (Every fourth stays live, and nothing here asks the random numbers, so that the full
                // and the mid-range tree die in the same places.)
                let bare = (k as f32) < ch.dead_lower * count as f32 * 0.6 && k % 4 != 0;
                let len =
                    max_len * from_top.powf(0.8) * rng.range(0.8, 1.15) * if bare { 0.4 } else { 1.0 } + height * 0.015;
                let angle = (55.0 + 50.0 * from_top + rng.range(-8.0, 8.0)).to_radians();
                let az = k as f32 * GOLDEN_ANGLE + rng.range(-0.2, 0.2);
                let dir = child_dir(Vec3::Y, az, angle);
                let b = Branch {
                    origin: at.pos,
                    dir,
                    length: len,
                    radius: (at.radius * 0.35).max(height * 0.0012),
                    level: 1,
                };
                let gravity = Vec3::NEG_Y * (0.06 + 0.08 * from_top + if bare { 0.12 } else { 0.0 });
                let nodes = grow(&b, if mid { 3 } else { 5 }, 0.85, 0.05, gravity, 0.25 * from_top, &mut rng);
                if !mid || bare {
                    tube(&mut wood, &nodes, if mid { 3 } else { 4 }, 2.0);
                }
                if bare {
                    continue;
                }
                // Spray along the branch itself (two crossed cards).
                let spine: Vec<Vec3> = nodes.iter().map(|n| n.pos).collect();
                let flat = nodes[nodes.len() / 2].dir.cross(Vec3::Y).normalize_or(Vec3::X);
                let widths: Vec<f32> =
                    (0..nodes.len()).map(|i| len * 0.75 * fill * (1.0 - 0.4 * i as f32 / nodes.len() as f32)).collect();
                card(&mut leaves, &spine, flat, &widths, axis);
                let steep = (flat * 0.4 + Vec3::Y * 0.9).normalize();
                card(&mut leaves, &spine, steep, &widths, axis);
                // Secondary twigs alternating left/right, roughly horizontal.
                let twigs = if low {
                    0
                } else if mid {
                    2
                } else {
                    5 + (len / height * 45.0) as u32
                };
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
                    let w = tlen * 1.25 * fill;
                    let flat = tdir.cross(Vec3::Y).normalize_or(Vec3::X);
                    card(&mut leaves, &spine, flat, &[w, w * 0.85, w * 0.6], axis);
                    card(&mut leaves, &spine, (flat * 0.3 + Vec3::Y).normalize(), &[w * 0.9, w * 0.75, w * 0.5], axis);
                }
            }
            let top = trunk_nodes.last().unwrap().pos;
            match ch.top {
                // Leader at the top.
                Top::Whole => {
                    for a in [0.0f32, PI * 0.5] {
                        let across = Vec3::new(a.cos(), 0.0, a.sin());
                        let spine = [
                            top - Vec3::Y * height * 0.08,
                            top - Vec3::Y * height * 0.03,
                            top + Vec3::Y * height * 0.02,
                        ];
                        card(&mut leaves, &spine, across, &[height * 0.06, height * 0.04, height * 0.015], axis);
                    }
                }
                // A snapped top: a bare stub, and the branches below it reach up a little.
                Top::Broken => {}
                // A second leader rising beside the first from about four fifths of the way up.
                Top::Forked => {
                    for a in [0.0f32, PI * 0.5] {
                        let across = Vec3::new(a.cos(), 0.0, a.sin());
                        let spine = [
                            top - Vec3::Y * height * 0.08,
                            top - Vec3::Y * height * 0.03,
                            top + Vec3::Y * height * 0.02,
                        ];
                        card(&mut leaves, &spine, across, &[height * 0.06, height * 0.04, height * 0.015], axis);
                    }
                    let at = sample(&trunk_nodes, 0.8);
                    let az = rng.f() * TAU;
                    let b = Branch {
                        origin: at.pos,
                        dir: child_dir(Vec3::Y, az, 0.3),
                        length: height * 0.24,
                        radius: at.radius * 0.45,
                        level: 1,
                    };
                    let nodes = grow(&b, if mid { 3 } else { 6 }, 0.9, 0.03, Vec3::ZERO, 0.0, &mut rng);
                    tube(&mut wood, &nodes, if mid { 4 } else { 6 }, 1.0);
                    for a in [0.0f32, PI * 0.5] {
                        let across = Vec3::new(a.cos(), 0.0, a.sin());
                        let tip = nodes.last().unwrap().pos;
                        let spine = [
                            tip - Vec3::Y * height * 0.1,
                            tip - Vec3::Y * height * 0.04,
                            tip + Vec3::Y * height * 0.02,
                        ];
                        card(&mut leaves, &spine, across, &[height * 0.07, height * 0.045, height * 0.015], axis);
                    }
                }
            }
        }
        Species::Snag => {
            let ch = character(seed);
            let trunk = Branch {
                origin: Vec3::ZERO,
                dir: Vec3::new(ch.lean.x, 1.0, ch.lean.z),
                length: height * 0.95,
                radius: height * 0.0135,
                level: 0,
            };
            let trunk_nodes = grow(&trunk, if mid { 6 } else { 14 }, 0.8, 0.02, ch.lean * 0.3, 0.0, &mut rng);
            tube(&mut wood, &trunk_nodes, if mid { 6 } else { 9 }, 0.5);
            // Stubs and a few longer dead branches, all bare, weathered to points.
            let count = if mid { 26 } else { 46 };
            for k in 0..count {
                let t = 0.22 + 0.7 * (k as f32 + rng.f()) / count as f32;
                let at = sample(&trunk_nodes, t);
                let from_top = 1.0 - t;
                let len = height * rng.range(0.02, 0.1) * (0.4 + from_top * 1.6).min(1.6);
                let angle = rng.range(60.0, 110.0).to_radians();
                let dir = child_dir(Vec3::Y, k as f32 * GOLDEN_ANGLE + rng.range(-0.3, 0.3), angle);
                let b = Branch {
                    origin: at.pos,
                    dir,
                    length: len,
                    radius: (at.radius * 0.3).max(height * 0.001),
                    level: 1,
                };
                let nodes = grow(&b, if mid { 2 } else { 3 }, 0.9, 0.06, Vec3::NEG_Y * 0.05, 0.0, &mut rng);
                tube(&mut wood, &nodes, if mid { 3 } else { 4 }, 2.0);
            }
            // A few dry tufts of needles that never fell.
            for k in 0..2 {
                let at = sample(&trunk_nodes, 0.5 + 0.2 * k as f32);
                let spine = [
                    at.pos,
                    at.pos + Vec3::new(0.4, 0.1, 0.0) * height * 0.05,
                    at.pos + Vec3::new(0.7, -0.1, 0.1) * height * 0.05,
                ];
                card(&mut leaves, &spine, Vec3::Z, &[height * 0.03, height * 0.025, height * 0.01], axis);
            }
        }
        Species::Shrub => {
            let stems = 6 + (rng.f() * 3.0) as usize;
            let centre = Vec3::new(0.0, height * 0.5, 0.0);
            let around = move |_: Vec3| centre;
            for k in 0..stems {
                let az = k as f32 * GOLDEN_ANGLE + rng.range(-0.3, 0.3);
                let origin = Vec3::new(az.cos(), 0.0, az.sin()) * height * 0.06;
                let b = Branch {
                    origin,
                    dir: child_dir(Vec3::Y, az, rng.range(15.0, 45.0).to_radians()),
                    length: height * rng.range(0.8, 1.05),
                    radius: height * 0.012,
                    level: 0,
                };
                let nodes = grow(&b, 3, 0.8, 0.05, Vec3::Y * 0.05, 0.0, &mut rng);
                tube(&mut wood, &nodes, 4, 1.0);
                for c in 0..4 {
                    let q = sample(&nodes, 0.35 + 0.22 * c as f32);
                    let size = height * rng.range(0.24, 0.34);
                    let out = (q.pos - centre).normalize_or(Vec3::Y);
                    let twist = rng.f() * TAU;
                    let tangent = out.cross(Vec3::Y).normalize_or(Vec3::X);
                    let across = (tangent * twist.cos() + out.cross(tangent) * twist.sin()).normalize();
                    let along = (out * 0.6 + Vec3::Y * 0.4).normalize();
                    let spine = [q.pos - along * size * 0.3, q.pos, q.pos + along * size * 0.4];
                    card(&mut leaves, &spine, across, &[size, size, size * 0.7], around);
                    card(
                        &mut leaves,
                        &spine,
                        (across + Vec3::Y * 0.8).normalize(),
                        &[size * 0.8, size * 0.8, size * 0.5],
                        around,
                    );
                }
            }
        }
        Species::Aspen => {
            let lean = Vec3::new(rng.range(-0.04, 0.04), 1.0, rng.range(-0.04, 0.04));
            let trunk =
                Branch { origin: Vec3::ZERO, dir: lean, length: height * 0.88, radius: height * 0.011, level: 0 };
            let trunk_nodes = grow(&trunk, if mid { 8 } else { 12 }, 0.8, 0.02, Vec3::Y * 0.02, 0.0, &mut rng);
            tube(&mut wood, &trunk_nodes, if mid { 6 } else { 8 }, 0.35);
            let crown_centre = Vec3::new(0.0, height * 0.66, 0.0);
            let centre = move |_: Vec3| crown_centre;
            let count = 14;
            for k in 0..count {
                let t = 0.42 + 0.5 * (k as f32 + rng.f()) / count as f32;
                let at = sample(&trunk_nodes, t);
                let len = height * rng.range(0.16, 0.26) * (1.15 - 0.6 * (t - 0.42));
                let dir = child_dir(at.dir, k as f32 * GOLDEN_ANGLE, rng.range(30.0, 50.0).to_radians());
                let b = Branch { origin: at.pos, dir, length: len, radius: at.radius * 0.38, level: 1 };
                let nodes = grow(&b, if mid { 3 } else { 4 }, 0.8, 0.12, Vec3::Y * 0.06, 0.0, &mut rng);
                tube(&mut wood, &nodes, if mid { 4 } else { 5 }, 1.0);
                let (twigs, clusters, bigger) = if mid { (3, 4, 1.4) } else { (7, 7, 1.0) };
                for j in 0..twigs {
                    let tt = 0.3 + 0.65 * (j as f32 + rng.f()) / twigs as f32;
                    let p = sample(&nodes, tt);
                    let tdir = child_dir(p.dir, rng.f() * TAU, rng.range(25.0, 55.0).to_radians());
                    let tb = Branch { origin: p.pos, dir: tdir, length: len * 0.45, radius: p.radius * 0.5, level: 2 };
                    let tn = grow(&tb, 3, 0.7, 0.15, Vec3::Y * 0.05, 0.0, &mut rng);
                    if !mid {
                        tube(&mut wood, &tn, 3, 2.0);
                    }
                    // Leaf clusters around and beyond the twig.
                    for c in 0..clusters {
                        let q = sample(&tn, 0.2 + 0.98 / clusters as f32 * c as f32);
                        let size = height * rng.range(0.085, 0.125) * bigger;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn triangles(m: &Mesh) -> usize {
        m.indices().map_or(0, |i| i.len() / 3)
    }

    /// (tallest point, widest reach from the trunk) of a tree's two meshes.
    fn extent(t: &TreeMeshes) -> (f32, f32) {
        let (mut top, mut reach) = (0.0f32, 0.0f32);
        for m in [&t.wood, &t.foliage] {
            let p = m.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).expect("positions");
            for v in p {
                top = top.max(v[1]);
                reach = reach.max(v[0].hypot(v[2]));
            }
        }
        (top, reach)
    }

    /// The mid-range tree is the full tree with less of it: a few hundred triangles against
    /// thousands, but the same height and about the same spread, so that the switch between them
    /// does not show.
    #[test]
    fn the_mid_range_tree_stands_in_for_the_full_one() {
        for (species, height, budget) in [(Species::Spruce, 20.0, 2_000), (Species::Aspen, 14.0, 2_500)] {
            for seed in [11, 12] {
                let full = generate_lod(species, seed, height, Lod::Full);
                let mid = generate_lod(species, seed, height, Lod::Mid);
                let (full_tris, mid_tris) =
                    (triangles(&full.wood) + triangles(&full.foliage), triangles(&mid.wood) + triangles(&mid.foliage));
                assert!(mid_tris <= budget, "mid tree has {mid_tris} triangles, budget {budget}");
                assert!(mid_tris * 4 <= full_tris, "mid {mid_tris} is not much less than full {full_tris}");
                let ((full_top, full_reach), (mid_top, mid_reach)) = (extent(&full), extent(&mid));
                assert!((mid_top / full_top - 1.0).abs() < 0.06, "height {mid_top} against {full_top}");
                assert!((mid_reach / full_reach - 1.0).abs() < 0.3, "spread {mid_reach} against {full_reach}");
                assert!(triangles(&mid.foliage) > 100, "the mid tree has needles");
            }
        }
    }

    /// (lowest needle, trunk top as a fraction of the height, how far the top is from the base
    /// as a fraction of the height).
    fn character(t: &TreeMeshes, height: f32) -> (f32, f32, f32) {
        let pos = |m: &Mesh| {
            m.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).map(|p| p.to_vec()).unwrap_or_default()
        };
        let lowest = pos(&t.foliage).iter().map(|v| v[1]).fold(f32::MAX, f32::min);
        let top = pos(&t.wood).into_iter().max_by(|a, b| a[1].total_cmp(&b[1])).expect("wood");
        (lowest / height, top[1] / height, top[0].hypot(top[2]) / height)
    }

    /// No two trees are the same kind of tree: how high the lowest needles are, whether the top
    /// is whole, broken or forked, and how far the trunk leans, all differ from seed to seed.
    #[test]
    fn spruces_differ_in_character() {
        let all: Vec<(f32, f32, f32)> =
            (0..24).map(|s| character(&generate_lod(Species::Spruce, 11 + s, 20.0, Lod::Full), 20.0)).collect();
        let lows: Vec<f32> = all.iter().map(|c| c.0).collect();
        let spread = lows.iter().cloned().fold(f32::MIN, f32::max) - lows.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 0.12, "the lowest needles are all at about the same height ({spread})");
        assert!(lows.iter().all(|&l| l > 0.02 && l < 0.45), "{lows:?}");
        assert!(all.iter().any(|c| c.1 < 0.95), "no tree has a broken top");
        assert!(all.iter().any(|c| c.1 > 1.0), "no tree has a whole top");
        let lean = all.iter().map(|c| c.2).fold(0.0f32, f32::max);
        assert!(lean > 0.04, "no tree leans (the most is {lean})");
        assert!(all.iter().filter(|c| c.2 < 0.03).count() >= 8, "most trees should stand straight");
    }

    /// The same seed is the same tree at either level of detail: its character, not just its size.
    #[test]
    fn a_tree_keeps_its_character_at_mid_range() {
        for seed in 11..19 {
            let full = character(&generate_lod(Species::Spruce, seed, 20.0, Lod::Full), 20.0);
            let mid = character(&generate_lod(Species::Spruce, seed, 20.0, Lod::Mid), 20.0);
            assert!((full.1 - mid.1).abs() < 0.04, "seed {seed}: top {} against {}", full.1, mid.1);
            assert!((full.0 - mid.0).abs() < 0.1, "seed {seed}: crown base {} against {}", full.0, mid.0);
        }
    }

    /// A snag is a dead tree standing: a trunk and stubs of branches, and next to no needles.
    #[test]
    fn a_snag_is_a_dead_tree_standing() {
        let snag = generate_lod(Species::Snag, 5, 18.0, Lod::Full);
        let live = generate_lod(Species::Spruce, 5, 18.0, Lod::Full);
        assert!(triangles(&snag.wood) > 100, "a snag has a trunk and stubs");
        assert!(
            triangles(&snag.foliage) * 20 <= triangles(&live.foliage),
            "{} needle triangles",
            triangles(&snag.foliage)
        );
        let (top, _) = extent(&snag);
        assert!(top > 18.0 * 0.8, "a snag stands tall ({top})");
        let mid = generate_lod(Species::Snag, 5, 18.0, Lod::Mid);
        assert!(triangles(&mid.wood) > 20);
    }

    /// Saplings are everywhere under the trees, so each is cheap: a few hundred triangles, a
    /// real little spruce, as tall as asked.
    #[test]
    fn a_sapling_is_a_cheap_little_spruce() {
        for seed in [11, 12, 13] {
            let t = generate_lod(Species::Spruce, seed, 4.0, Lod::Low);
            let tris = triangles(&t.wood) + triangles(&t.foliage);
            assert!(tris <= 900, "a sapling has {tris} triangles");
            assert!(triangles(&t.foliage) > 60, "a sapling has needles");
            let (top, reach) = extent(&t);
            assert!(top > 3.0 && top < 4.4, "height {top}");
            assert!(reach > 0.4, "a sapling is {reach} m across");
        }
    }

    /// A shrub is a thicket of stems with leaves, low and wide, and cheap.
    #[test]
    fn a_shrub_is_a_low_wide_thicket() {
        for seed in [1, 2, 3] {
            let t = generate_lod(Species::Shrub, seed, 1.5, Lod::Low);
            let tris = triangles(&t.wood) + triangles(&t.foliage);
            assert!(tris <= 900, "a shrub has {tris} triangles");
            assert!(triangles(&t.foliage) > 60 && triangles(&t.wood) > 30, "stems and leaves");
            let (top, reach) = extent(&t);
            assert!(top > 1.0 && top < 1.9, "height {top}");
            assert!(reach > top * 0.4, "a shrub is wider than a pole ({reach} m)");
        }
    }

    #[test]
    fn a_full_tree_is_what_generate_makes() {
        let a = generate(Species::Spruce, 11, 20.0);
        let b = generate_lod(Species::Spruce, 11, 20.0, Lod::Full);
        assert_eq!(triangles(&a.wood) + triangles(&a.foliage), triangles(&b.wood) + triangles(&b.foliage));
    }
}
