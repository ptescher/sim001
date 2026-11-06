//! Buildings: plastered walls with windows, under a pitched roof (a gable, its ridge along the
//! long side) or, for a big footprint, a flat one. A building fills the box the map gives it
//! (`terrain::Building`), roof and all, so what the drone hits is what it sees.

use crate::mesh_util::GeoBuilder;
use bevy::prelude::*;

/// A footprint whose short side is wider than this has a flat roof (m): a warehouse, a hall.
pub const FLAT_ABOVE: f32 = 20.0;
/// The pitch of a gable roof (deg).
pub const PITCH: f32 = 30.0;
/// Walls are at least this high under the eaves (m).
pub const MIN_EAVES: f32 = 2.2;
/// How far the roof reaches past the walls (m).
pub const OVERHANG: f32 = 0.5;
/// A bay of the facade: one window, every this many metres along a wall.
pub const BAY: f32 = 3.0;
/// A storey (m).
pub const STOREY: f32 = 2.8;
/// The wall texture covers this many bays and storeys.
pub const TILE_BAYS: f32 = 2.0;

/// The meshes of one building, centred on its box.
pub struct House {
    pub walls: Mesh,
    pub roof: Mesh,
}

/// A building that fills a box of `size` centred on the origin.
pub fn house(size: Vec3) -> House {
    let (top, foot) = (size.y / 2.0, -size.y / 2.0);
    // Work along the ridge (a) and across it (c); `at` puts a point back in the box's frame.
    let along_x = size.x >= size.z;
    let (la, lc) = if along_x { (size.x / 2.0, size.z / 2.0) } else { (size.z / 2.0, size.x / 2.0) };
    let at = |a: f32, y: f32, c: f32| if along_x { Vec3::new(a, y, c) } else { Vec3::new(c, y, a) };
    let flat = 2.0 * lc > FLAT_ABOVE;
    let slope = PITCH.to_radians().tan();
    let rise = if flat { 0.3 } else { (lc * slope).min(size.y - MIN_EAVES.min(size.y)).max(0.0) };
    let eaves = top - rise;

    // Walls: four sides up to the eaves, and on a gable roof, the gable ends up to the ridge. The
    // texture is laid in metres: along the wall from its corner, and up from its foot.
    let mut w = GeoBuilder::default();
    let (tw, th) = (TILE_BAYS * BAY, 2.0 * STOREY);
    let uv = |along: f32, y: f32| Vec2::new(along / tw, 1.0 - (y - foot) / th);
    for (s, out_a) in [(-1.0f32, true), (1.0, true), (-1.0, false), (1.0, false)] {
        // A wall at a = ±la (out_a) runs across c, else at c = ±lc it runs along a.
        let half = if out_a { lc } else { la };
        let corner = |t: f32, y: f32| if out_a { at(s * la, y, t) } else { at(t, y, s * lc) };
        let out = if out_a { at(s, 0.0, 0.0) } else { at(0.0, 0.0, s) };
        let p0 = w.vert(corner(-half, foot), uv(0.0, foot));
        let p1 = w.vert(corner(half, foot), uv(2.0 * half, foot));
        let p2 = w.vert(corner(half, eaves), uv(2.0 * half, eaves));
        let p3 = w.vert(corner(-half, eaves), uv(0.0, eaves));
        w.quad(p0, p1, p2, p3, out);
        if out_a && !flat && rise > 0.0 {
            let peak = w.vert(corner(0.0, top), uv(half, top));
            let l = w.vert(corner(-half, eaves), uv(0.0, eaves));
            let r = w.vert(corner(half, eaves), uv(2.0 * half, eaves));
            w.tri(l, r, peak, out);
        }
    }

    // Roof: a slab of `thick` that reaches past the walls; on a gable, two slopes that meet at
    // the ridge and run on down past the eaves.
    let mut r = GeoBuilder::default();
    let thick = 0.15;
    let ra = la + OVERHANG;
    let (rc, low) = if flat { (lc + OVERHANG, top) } else { (lc + OVERHANG, eaves - OVERHANG * slope) };
    let ridge = top;
    let face = |pts: [Vec3; 4], out: Vec3, r: &mut GeoBuilder| {
        let uvs = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
        let i: Vec<u32> = pts.iter().zip(uvs).map(|(p, u)| r.vert(*p, u)).collect();
        r.quad(i[0], i[1], i[2], i[3], out);
    };
    if flat {
        let (y1, y0) = (top, top - thick);
        face([at(-ra, y1, -rc), at(ra, y1, -rc), at(ra, y1, rc), at(-ra, y1, rc)], Vec3::Y, &mut r);
        face([at(-ra, y0, -rc), at(ra, y0, -rc), at(ra, y0, rc), at(-ra, y0, rc)], -Vec3::Y, &mut r);
        for s in [-1.0f32, 1.0] {
            face(
                [at(-ra, y0, s * rc), at(ra, y0, s * rc), at(ra, y1, s * rc), at(-ra, y1, s * rc)],
                at(0.0, 0.0, s),
                &mut r,
            );
            face(
                [at(s * ra, y0, -rc), at(s * ra, y0, rc), at(s * ra, y1, rc), at(s * ra, y1, -rc)],
                at(s, 0.0, 0.0),
                &mut r,
            );
        }
    } else {
        for s in [-1.0f32, 1.0] {
            let n = at(0.0, 1.0, s * slope).normalize();
            let top_face = [at(-ra, ridge, 0.0), at(ra, ridge, 0.0), at(ra, low, s * rc), at(-ra, low, s * rc)];
            face(top_face, n, &mut r);
            face(top_face.map(|p| p - Vec3::Y * thick), -n, &mut r);
            // The fascia along the eaves, and the verge at each gable end.
            face(
                [at(-ra, low - thick, s * rc), at(ra, low - thick, s * rc), at(ra, low, s * rc), at(-ra, low, s * rc)],
                at(0.0, 0.0, s),
                &mut r,
            );
            for e in [-1.0f32, 1.0] {
                let edge = [
                    at(e * ra, ridge - thick, 0.0),
                    at(e * ra, low - thick, s * rc),
                    at(e * ra, low, s * rc),
                    at(e * ra, ridge, 0.0),
                ];
                face(edge, at(e, 0.0, 0.0), &mut r);
            }
        }
    }
    House { walls: w.build(), roof: r.build() }
}

/// The wall's albedo at `along` metres along the wall and `up` metres above its foot: plaster,
/// a darker plinth, and in every bay of every storey a window in a light frame.
pub fn wall_albedo(along: f32, up: f32) -> [f32; 3] {
    let plaster = 0.82 * (0.93 + 0.07 * crate::terrain::fbm(along * 1.7, up * 1.7, 3));
    let (bx, sy) = (along.rem_euclid(BAY) - BAY / 2.0, up.rem_euclid(STOREY));
    let (half_w, sill, head) = (0.65, 0.9, 2.2);
    let frame = 0.08;
    let v = if bx.abs() < half_w && (sill..head).contains(&sy) {
        if bx.abs() > half_w - frame || sy < sill + frame || sy > head - frame { 0.85 } else { 0.05 }
    } else if up < 0.4 {
        0.6 * plaster
    } else {
        plaster
    };
    // Glass is a little blue, plaster a little warm.
    if v < 0.1 { [v * 0.9, v, v * 1.15] } else { [v, v * 0.98, v * 0.93] }
}

/// The wall texture: `TILE_BAYS` bays by two storeys, repeated along every wall.
pub fn wall_texture() -> Image {
    let (tw, th) = (TILE_BAYS * BAY, 2.0 * STOREY);
    crate::mesh_util::texture(256, |u, v| wall_albedo(u * tw, (1.0 - v) * th))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn points(m: &Mesh) -> Vec<Vec3> {
        match m.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(p)) => p.iter().map(|v| Vec3::from_array(*v)).collect(),
            _ => vec![],
        }
    }

    fn top(m: &Mesh) -> f32 {
        points(m).iter().map(|p| p.y).fold(f32::MIN, f32::max)
    }

    /// A family house: 12 m long, 8 m deep, 7 m to the ridge.
    const HOME: Vec3 = Vec3::new(12.0, 7.0, 8.0);

    #[test]
    fn a_house_has_a_pitched_roof_with_its_ridge_along_the_long_side() {
        let h = house(HOME);
        let roof = points(&h.roof);
        assert!(!roof.is_empty(), "a house has a roof");
        let ridge_y = top(&h.roof);
        let eaves_y = top(&h.walls.clone());
        let ridge: Vec<Vec3> = roof.iter().copied().filter(|p| p.y > ridge_y - 0.05).collect();
        let span = |f: fn(&Vec3) -> f32| {
            ridge.iter().map(f).fold(f32::MIN, f32::max) - ridge.iter().map(f).fold(f32::MAX, f32::min)
        };
        let rise = ridge_y - roof.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!(rise > 1.5, "the roof rises {rise:.2} m from the eaves to the ridge");
        assert!(span(|p| p.x) > 10.0, "the ridge runs {:.1} m along the 12 m side", span(|p| p.x));
        assert!(span(|p| p.z) < 0.5, "the ridge is a line, {:.1} m across", span(|p| p.z));
        assert!(eaves_y <= ridge_y, "the gable walls reach the ridge, not past it");
    }

    #[test]
    fn a_long_house_turned_the_other_way_has_its_ridge_the_other_way() {
        let h = house(Vec3::new(8.0, 7.0, 12.0));
        let roof = points(&h.roof);
        let ridge_y = top(&h.roof);
        let ridge: Vec<Vec3> = roof.into_iter().filter(|p| p.y > ridge_y - 0.05).collect();
        let along_z =
            ridge.iter().map(|p| p.z).fold(f32::MIN, f32::max) - ridge.iter().map(|p| p.z).fold(f32::MAX, f32::min);
        assert!(along_z > 10.0, "the ridge runs {along_z:.1} m along z");
    }

    #[test]
    fn the_roof_overhangs_the_walls() {
        let h = house(HOME);
        let roof = points(&h.roof);
        let walls = points(&h.walls);
        for (axis, half) in [(0usize, HOME.x / 2.0), (2, HOME.z / 2.0)] {
            let r = roof.iter().map(|p| p[axis].abs()).fold(0.0, f32::max);
            let w = walls.iter().map(|p| p[axis].abs()).fold(0.0, f32::max);
            assert!((w - half).abs() < 0.01, "the walls stand on the footprint ({w:.2} against {half:.2})");
            assert!(r - w >= 0.3, "the roof reaches {:.2} m past the walls on axis {axis}", r - w);
        }
    }

    #[test]
    fn a_building_stays_in_its_box() {
        for size in [HOME, Vec3::new(5.0, 3.0, 4.0), Vec3::new(40.0, 9.0, 25.0), Vec3::new(10.0, 2.5, 6.0)] {
            let h = house(size);
            for p in points(&h.walls).iter().chain(points(&h.roof).iter()) {
                assert!(p.y <= size.y / 2.0 + 0.01, "{size}: a point at {:.2} is above the box", p.y);
                assert!(p.y >= -size.y / 2.0 - 0.01, "{size}: a point at {:.2} is under the box", p.y);
            }
            let walls = points(&h.walls);
            let eaves = walls.iter().filter(|p| p.x.abs() > size.x / 2.0 - 0.01).map(|p| p.y).fold(f32::MIN, f32::max);
            let low = walls.iter().filter(|p| p.z.abs() > size.z / 2.0 - 0.01).map(|p| p.y).fold(f32::MIN, f32::max);
            assert!(
                eaves.min(low) + size.y / 2.0 >= MIN_EAVES.min(size.y) - 0.01,
                "{size}: the walls are tall enough to stand in"
            );
        }
    }

    #[test]
    fn a_big_building_has_a_flat_roof() {
        let size = Vec3::new(40.0, 9.0, 25.0);
        let roof = points(&house(size).roof);
        let lo = roof.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let hi = roof.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!(!roof.is_empty());
        assert!(hi - lo < 1.0, "a hall's roof is flat ({:.2} m from low to high)", hi - lo);
    }

    #[test]
    fn walls_have_windows_a_storey_apart() {
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        // The middle of a window, and the plaster between windows and between storeys.
        let glass = lum(wall_albedo(BAY / 2.0, 1.5));
        let pier = lum(wall_albedo(0.1, 1.5));
        let band = lum(wall_albedo(BAY / 2.0, STOREY - 0.1));
        let upstairs = lum(wall_albedo(BAY / 2.0, STOREY + 1.5));
        assert!(glass < 0.35 * pier, "glass {glass:.2} is dark against the plaster {pier:.2}");
        assert!(glass < 0.35 * band, "glass {glass:.2} is dark against the band {band:.2}");
        assert!((upstairs - glass).abs() < 0.1, "the next storey has a window too");
        assert!(pier > 0.5, "the plaster is light ({pier:.2})");
    }
}
