//! Buildings: plastered walls with windows, under a pitched roof (a gable, its ridge along the
//! long side) or, for a big footprint, a flat one. A building fills the box the map gives it
//! (`terrain::Building`), roof and all, so what the drone hits is what it sees.

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
    let _ = size;
    let empty = || Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, default());
    House { walls: empty(), roof: empty() }
}

/// The wall's albedo at `along` metres along the wall and `up` metres above its foot.
pub fn wall_albedo(along: f32, up: f32) -> [f32; 3] {
    let _ = (along, up);
    [1.0; 3]
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
