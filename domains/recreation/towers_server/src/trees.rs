//! The forest as the flight model meets it: each tree is an obstacle, and a plane is held against
//! only the trees round it (a grid), since the forest has tens of thousands.

use glam::Vec3;
use recreation_towers::map::TreeSpot;
use simulation_physics::Obstacle;
use std::collections::HashMap;

/// The grid's cells are this wide (m).
const CELL: f32 = 40.0;
/// Above this height (m) no tree is in the way.
pub const CLEAR_OF_TREES: f32 = 45.0;

/// The trees by cell.
#[derive(Default)]
pub struct Forest {
    cells: HashMap<(i32, i32), Vec<Obstacle>>,
}

fn cell(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

/// The obstacle a tree is to the flight model: a cylinder as wide as the trunk and crown the same
/// way the 3D view grows them (`visualization_world::terrain::obstacles`).
fn obstacle(t: &TreeSpot) -> Obstacle {
    let pos = crate::world::to_world(t.at, 0.0);
    Obstacle { pos, radius: t.height * if t.conifer { 0.18 } else { 0.26 }, top: t.height * 0.95, bottom: -1.0 }
}

impl Forest {
    pub fn new(trees: &[TreeSpot]) -> Forest {
        let mut cells: HashMap<(i32, i32), Vec<Obstacle>> = HashMap::new();
        for t in trees {
            let o = obstacle(t);
            cells.entry(cell(o.pos.x, o.pos.z)).or_default().push(o);
        }
        Forest { cells }
    }

    /// Puts the trees within reach of a plane at `pos` into `out` (cleared first).
    pub fn near(&self, pos: Vec3, out: &mut Vec<Obstacle>) {
        out.clear();
        if pos.y > CLEAR_OF_TREES {
            return;
        }
        let (cx, cz) = cell(pos.x, pos.z);
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(v) = self.cells.get(&(cx + dx, cz + dz)) {
                    out.extend_from_slice(v);
                }
            }
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cells.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use recreation_towers::{Map, Rules};

    #[test]
    fn a_plane_is_held_against_only_the_trees_round_it_and_none_when_high() {
        let map = Map::new(&Rules::default());
        let trees = map.trees();
        let forest = Forest::new(&trees);
        assert_eq!(forest.len(), trees.len());
        let mut near = vec![];
        forest.near(crate::world::to_world(Vec2::new(-250.0, 100.0), 15.0), &mut near);
        assert!(!near.is_empty() && near.len() < 200, "{} trees", near.len());
        forest.near(crate::world::to_world(Vec2::new(-250.0, 100.0), 60.0), &mut near);
        assert!(near.is_empty(), "no tree reaches 45 m");
    }

    #[test]
    fn no_tree_stands_in_a_lane_so_a_plane_flying_along_one_meets_none() {
        let map = Map::new(&Rules::default());
        let forest = Forest::new(&map.trees());
        let mut near = vec![];
        // Along the middle of the mid lane from the Blue home tower to the centre.
        for k in 0..43 {
            let at = Vec2::new(-430.0 + 10.0 * k as f32, 0.0);
            let p = crate::world::to_world(at, 12.0);
            forest.near(p, &mut near);
            assert!(
                near.iter().all(|o| (o.pos.x - p.x).hypot(o.pos.z - p.z) > o.radius + 15.0),
                "a tree within 15 m of the plane at {at}"
            );
        }
    }
}
