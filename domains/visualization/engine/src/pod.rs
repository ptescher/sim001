//! The fuselage pod of the T1-S, from the manufacturer's three-view drawing: a long, squarish
//! lozenge, narrow at the nose and wider where the wing joins it, with the wing near its top, and
//! on its top a louvred vent at the nose, a hatch outlined in the foam, a slot, and a louvred vent
//! at the back.
//!
//! The pod is a loft of rounded-rectangle sections; the markings are ribbons and patches draped
//! on its surface (`surface` says where that is), so that they follow it and stand a hair off it.

use crate::mesh_util::loft_n;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use simulation_aircraft::t1s_ranger::geom::NOSE_Z;

/// The sections are `|x|^n + |y|^n = 1`: 2 is an ellipse, more is squarer.
pub const EXPONENT: f32 = 3.0;

/// (z, half width, half height, centre height) down the pod, from the nose to where it meets the
/// boom: a squared nose, a front section of constant width, then the wing's shoulders.
pub fn stations() -> Vec<(f32, f32, f32, f32)> {
    vec![
        (NOSE_Z, 0.022, 0.026, 0.003),
        (NOSE_Z + 0.011, 0.034, 0.033, 0.003),
        (-0.233, 0.039, 0.038, 0.003),
        (-0.1775, 0.040, 0.038, 0.003),
        (-0.1035, 0.041, 0.038, 0.003),
        (-0.0665, 0.052, 0.038, 0.003),
        (-0.011, 0.0575, 0.038, 0.003),
        (0.0334, 0.052, 0.036, 0.003),
        (0.0704, 0.030, 0.028, 0.0045),
        (0.100, 0.0135, 0.0135, 0.0095),
    ]
}

/// The pod itself.
pub fn mesh() -> Mesh {
    loft_n(&stations(), 0.0, 48, false, EXPONENT)
}

/// Half width, half height and centre height at `z`.
pub fn stations_at(z: f32) -> (f32, f32, f32, f32) {
    let s = stations();
    let i = s.windows(2).position(|w| z <= w[1].0).unwrap_or(s.len() - 2);
    let (a, b) = (s[i], s[i + 1]);
    let t = ((z - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
    (z, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t, a.3 + (b.3 - a.3) * t)
}

/// The point of the pod's top (or bottom) surface at `z` along it and `x` across, and the way
/// that surface faces. None beyond the pod's sides and ends.
pub fn surface(z: f32, x: f32, up: bool) -> Option<(Vec3, Vec3)> {
    let s = stations();
    if z < s[0].0 || z > s[s.len() - 1].0 {
        return None;
    }
    let point = |z: f32, x: f32| -> Option<Vec3> {
        let (_, rx, ry, cy) = stations_at(z);
        let u = x.abs() / rx;
        if u > 1.0 {
            return None;
        }
        let y = ry * (1.0 - u.powf(EXPONENT)).max(0.0).powf(1.0 / EXPONENT);
        Some(Vec3::new(x, cy + if up { y } else { -y }, z))
    };
    let p = point(z, x)?;
    let h = 0.0008;
    let (dz, dx) = (point(z + h, x).unwrap_or(p) - point(z - h, x).unwrap_or(p), point(z, x + h).unwrap_or(p) - point(z, x - h).unwrap_or(p));
    let n = dz.cross(dx).normalize_or(Vec3::Y);
    Some((p, if (n.y > 0.0) == up { n } else { -n }))
}

/// Height of the top (or bottom) of the pod on its centre line.
pub fn surface_y(z: f32, up: bool) -> f32 {
    surface(z, 0.0, up).map_or(0.0, |s| s.0.y)
}

fn raw_mesh(pos: Vec<Vec3>, nrm: Vec<Vec3>, idx: Vec<u32>) -> Mesh {
    let uv = vec![[0.0f32, 0.0]; pos.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos.iter().map(|p| p.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm.iter().map(|p| p.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// A ribbon `width` wide along `path` (z, x pairs) on the top of the pod, `lift` above it.
pub fn strip(path: &[Vec2], width: f32, lift: f32) -> Mesh {
    let (mut pos, mut nrm, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for (i, p) in path.iter().enumerate() {
        let d = if i + 1 < path.len() { path[i + 1] - *p } else { *p - path[i - 1] };
        let side = Vec2::new(-d.y, d.x).normalize_or(Vec2::X) * width * 0.5;
        for q in [*p - side, *p + side] {
            let (at, n) = surface(q.x, q.y, true).unwrap_or((Vec3::new(q.y, 0.04, q.x), Vec3::Y));
            pos.push(at + n * lift);
            nrm.push(n);
        }
        if i > 0 {
            let b = (i as u32 - 1) * 2;
            idx.extend_from_slice(&[b, b + 1, b + 3, b, b + 3, b + 2]);
        }
    }
    raw_mesh(pos, nrm, idx)
}

/// A rectangle `z0..z1` by `x0..x1` on the top of the pod, `lift` above it.
pub fn patch(z0: f32, z1: f32, x0: f32, x1: f32, lift: f32) -> Mesh {
    let (nz, nx) = (6usize, 4usize);
    let (mut pos, mut nrm, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..=nz {
        for j in 0..=nx {
            let (z, x) = (z0 + (z1 - z0) * i as f32 / nz as f32, x0 + (x1 - x0) * j as f32 / nx as f32);
            let (at, n) = surface(z, x, true).unwrap_or((Vec3::new(x, 0.04, z), Vec3::Y));
            pos.push(at + n * lift);
            nrm.push(n);
        }
    }
    for i in 0..nz {
        for j in 0..nx {
            let a = (i * (nx + 1) + j) as u32;
            let b = a + nx as u32 + 1;
            idx.extend_from_slice(&[a, b, b + 1, a, b + 1, a + 1]);
        }
    }
    raw_mesh(pos, nrm, idx)
}

/// A rectangle's outline with its corners cut, as a closed path.
fn outline(z0: f32, z1: f32, half: f32, cut: f32) -> Vec<Vec2> {
    vec![
        Vec2::new(z0 + cut, -half),
        Vec2::new(z1 - cut, -half),
        Vec2::new(z1, -half + cut),
        Vec2::new(z1, half - cut),
        Vec2::new(z1 - cut, half),
        Vec2::new(z0 + cut, half),
        Vec2::new(z0, half - cut),
        Vec2::new(z0, -half + cut),
        Vec2::new(z0 + cut, -half),
    ]
}

/// The markings on the pod. The first `dark` of the meshes are dark (recesses and gaps); the rest
/// are raised slats in the foam's own colour.
pub struct Details {
    pub meshes: Vec<Mesh>,
    pub dark: usize,
    pub front_louvres: usize,
    pub rear_louvres: usize,
    /// z extents.
    pub front_vent: (f32, f32),
    pub hatch: (f32, f32),
    pub rear_vent: (f32, f32),
}

pub fn details() -> Details {
    let (front_vent, hatch, slot, rear_vent) = ((-0.258f32, -0.236f32), (-0.172f32, -0.02f32), (-0.075f32, -0.063f32), (0.040f32, 0.062f32));
    let mut dark = Vec::new();
    let mut slats = Vec::new();
    // The vent at the nose: a dark recess crossed by slats.
    dark.push(patch(front_vent.0, front_vent.1, -0.0155, 0.0155, 0.0004));
    let front_louvres = 7;
    for k in 0..front_louvres {
        let z = front_vent.0 + (front_vent.1 - front_vent.0) * (k as f32 + 0.5) / front_louvres as f32;
        slats.push(strip(&[Vec2::new(z, -0.0145), Vec2::new(z, 0.0), Vec2::new(z, 0.0145)], 0.0016, 0.0012));
    }
    // The hatch: a thin gap round it, and a second step inside.
    dark.push(strip(&outline(hatch.0, hatch.1, 0.026, 0.008), 0.0013, 0.0003));
    dark.push(strip(&outline(hatch.0 + 0.007, hatch.1 - 0.007, 0.0195, 0.006), 0.0009, 0.0003));
    // The slot for the plug in the middle.
    dark.push(patch(slot.0, slot.1, -0.0135, 0.0135, 0.0004));
    // The vent at the back.
    dark.push(patch(rear_vent.0, rear_vent.1, -0.0125, 0.0125, 0.0004));
    let rear_louvres = 5;
    for k in 0..rear_louvres {
        let z = rear_vent.0 + (rear_vent.1 - rear_vent.0) * (k as f32 + 0.5) / rear_louvres as f32;
        slats.push(strip(&[Vec2::new(z, -0.0115), Vec2::new(z, 0.0), Vec2::new(z, 0.0115)], 0.0016, 0.0012));
    }
    let n = dark.len();
    dark.extend(slats);
    Details { meshes: dark, dark: n, front_louvres, rear_louvres, front_vent, hatch, rear_vent }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use simulation_aircraft::t1s_ranger::geom::*;

    /// What the manufacturer's three-view drawing says of the pod (the T1's, 730 mm across, scaled
    /// by 790/730 for the T1-S): about 372 mm from the nose to the boom, 80 mm wide at the front
    /// widening to 115 mm where the wing joins it, and 76 mm tall, with the wing near the top of
    /// it rather than half way up.
    #[test]
    fn the_pod_has_the_proportions_of_the_real_one() {
        let s = stations();
        let (first, last) = (s[0], s[s.len() - 1]);
        assert!((first.0 - NOSE_Z).abs() < 1e-4, "starts at the nose");
        let length = last.0 - first.0;
        assert!(length > 0.34 && length < 0.40, "the pod is {length} m long");
        let front = s.iter().filter(|st| st.0 > NOSE_Z + 0.08 && st.0 < -0.12).map(|st| st.1).fold(0.0f32, f32::max);
        assert!(front > 0.036 && front < 0.044, "front half-width {front}");
        let (widest_z, widest) = s.iter().map(|st| (st.0, st.1)).fold((0.0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
        assert!(widest > 0.052 && widest < 0.063, "widest half-width {widest}");
        assert!(widest > front * 1.25, "no shoulders where the wing joins");
        assert!(widest_z > -0.06 && widest_z < 0.02, "the shoulders are at {widest_z}");
        let tallest = s.iter().map(|st| st.2).fold(0.0f32, f32::max);
        assert!(tallest > 0.034 && tallest < 0.042, "half-height {tallest}");
        // The wing's root sits high on the pod: its top a centimetre or so under the pod's.
        let (top, bottom) = (surface_y(WING_LE_Z + 0.08, true), surface_y(WING_LE_Z + 0.08, false));
        assert!(top > WING_Y + 0.010 && top < WING_Y + 0.028, "pod top {top} over a wing at {WING_Y}");
        assert!(bottom < WING_Y - 0.040 && bottom > WING_Y - 0.065, "pod bottom {bottom}");
    }

    /// The cross-section is a rounded rectangle, not an ellipse: at 45 degrees it reaches further
    /// out than an ellipse would.
    #[test]
    fn the_pod_is_squarish_in_section() {
        let (z, rx, ry) = (-0.1, 0.04, 0.038);
        let p = surface(z, rx * 0.78, true).expect("on the pod").0;
        // On an ellipse the point at x = 0.78 rx would be at y = 0.63 ry.
        let ellipse_y = ry * (1.0f32 - 0.78 * 0.78).sqrt();
        assert!(p.y - stations_at(z).3 > ellipse_y * 1.15, "round, not square: {} against {}", p.y, ellipse_y);
    }

    /// The details that are put on the pod lie on its surface, a hair above it.
    #[test]
    fn a_strip_lies_on_the_surface() {
        let path = [Vec2::new(-0.2, -0.02), Vec2::new(-0.1, -0.025), Vec2::new(0.0, -0.03), Vec2::new(0.04, -0.02)];
        let m = strip(&path, 0.002, 0.0004);
        let pos = m.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).unwrap();
        assert!(pos.len() >= 8);
        for p in pos {
            let on = surface(p[2], p[0], true).expect("over the pod");
            assert!((Vec3::from(*p) - on.0).dot(on.1).abs() < 0.0012, "{p:?} is {} off the surface", (Vec3::from(*p) - on.0).dot(on.1));
            assert!((Vec3::from(*p) - on.0).dot(on.1) > 0.0, "{p:?} is under the surface");
        }
    }

    /// The reference's pod has a louvred vent at the front, a hatch outlined on the top, a slot
    /// in the middle and a louvred vent at the back; all of it on the top of the pod.
    #[test]
    fn the_hatches_are_where_the_pictures_show_them() {
        let d = details();
        assert!(d.front_louvres >= 6 && d.rear_louvres >= 4, "louvres {} and {}", d.front_louvres, d.rear_louvres);
        // The hatch outline spans most of the middle of the top.
        assert!(d.hatch.0 > -0.20 && d.hatch.0 < -0.14 && d.hatch.1 > -0.04 && d.hatch.1 < 0.03, "hatch from {} to {}", d.hatch.0, d.hatch.1);
        assert!(d.front_vent.1 < d.hatch.0 && d.hatch.1 < d.rear_vent.0, "the hatches overlap");
        assert!(d.front_vent.0 > NOSE_Z + 0.01, "the vent is on the nose");
        let tris: usize = d.meshes.iter().map(|m| m.indices().map_or(0, |i| i.len() / 3)).sum();
        assert!(tris > 200 && tris <= 1800, "{tris} triangles of detail");
        for m in &d.meshes {
            for p in m.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()).unwrap() {
                assert!(surface(p[2], p[0], true).is_some(), "{p:?} is not over the pod");
                assert!(p[1] > 0.005, "{p:?} is not on top");
            }
        }
    }
}

