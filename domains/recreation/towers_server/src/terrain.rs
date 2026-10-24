//! The ground of the valley for the flight model: flat, with the mountains round the edge of the
//! field (`recreation_towers::Map::mountain_height`). A drone that flies into them crashes.

use crate::world::to_field;
use glam::Vec3;
use recreation_towers::Map;
use simulation_physics::Terrain;

pub struct Valley(pub Map);

impl Valley {
    fn at(&self, x: f32, z: f32) -> f32 {
        self.0.mountain_height(to_field(Vec3::new(x, 0.0, z)).0)
    }
}

impl Terrain for Valley {
    fn height(&self, x: f32, z: f32) -> f32 {
        self.at(x, z)
    }
    fn normal(&self, x: f32, z: f32) -> Vec3 {
        let e = 0.5;
        Vec3::new(self.at(x - e, z) - self.at(x + e, z), 2.0 * e, self.at(x, z - e) - self.at(x, z + e)).normalize()
    }
    fn is_water(&self, _: f32, _: f32) -> bool {
        false
    }
    fn is_runway(&self, _: f32, _: f32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::to_world;
    use glam::Vec2;
    use recreation_towers::Rules;

    #[test]
    fn the_ground_is_flat_on_the_field_and_the_mountains_stand_where_the_map_has_them() {
        let v = Valley(Map::new(&Rules::default()));
        let (flat, edge) = (to_world(Vec2::new(-100.0, 50.0), 0.0), to_world(Vec2::new(0.0, 600.0), 0.0));
        assert_eq!(v.height(flat.x, flat.z), 0.0);
        assert_eq!(v.normal(flat.x, flat.z), Vec3::Y);
        assert!(v.height(edge.x, edge.z) >= 250.0);
        assert!(v.normal(edge.x, edge.z).y < 1.0 && v.normal(edge.x, edge.z).is_normalized());
    }
}
