//! The valley of Towers for the world of the 3D view: the game's map (`recreation_towers::map`) in
//! the world's frame (x east, y up, z south).

use bevy::prelude::*;
use recreation_towers::{Map, Rules};
use visualization_world::terrain::{self, Arena, Tree};

/// The world point (x, z) of a field point (north, east).
fn world(p: glam::Vec2) -> Vec2 {
    Vec2::new(p.y, -p.x)
}

/// Tells the world about the valley. Call once, before the world is built.
pub fn set(rules: &Rules) -> Result<(), String> {
    let map = Map::new(rules);
    let trees: Vec<Tree> = map
        .trees()
        .iter()
        .enumerate()
        .map(|(i, t)| Tree {
            pos: Vec3::new(t.at.y, 0.0, -t.at.x),
            height: t.height,
            conifer: t.conifer,
            shade: (i % 4) as u8,
            dead: false,
        })
        .collect();
    let mountains = map.clone();
    let lake = world(map.lake.at);
    let lake_radius = map.lake.radius;
    terrain::set_arena(Arena {
        radius: rules.field_radius,
        lake_centre: lake,
        lake_radius,
        trees,
        open_distance: Box::new(move |x, z| map.open_distance(glam::Vec2::new(-z, x))),
        mountains: Box::new(move |x, z| mountains.mountain_height(glam::Vec2::new(-z, x))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use visualization_world::terrain;

    fn valley() -> Map {
        static SET: std::sync::Once = std::sync::Once::new();
        let rules = Rules::default();
        SET.call_once(|| set(&rules).unwrap());
        Map::new(&rules)
    }

    #[test]
    fn the_world_has_its_lake_where_the_game_has_it() {
        let map = valley();
        let at = world(map.lake.at);
        assert!(terrain::is_water(at.x, at.y));
        // Just inside and just outside the game's circle, on all sides.
        for k in 0..8 {
            let dir = Vec2::from_angle(k as f32 * std::f32::consts::FRAC_PI_4);
            let inside = at + dir * (map.lake.radius - 6.0);
            let outside = at + dir * (map.lake.radius + 6.0);
            assert!(terrain::is_water(inside.x, inside.y), "inside, direction {k}");
            assert!(!terrain::is_water(outside.x, outside.y), "outside, direction {k}");
        }
    }

    #[test]
    fn home_is_the_middle_of_the_field_so_that_the_maps_and_the_game_share_one_frame() {
        valley();
        assert_eq!(terrain::pilot_pos(), Vec3::ZERO);
    }

    #[test]
    fn no_tree_stands_on_a_tower_or_a_lane() {
        let map = valley();
        for t in terrain::trees() {
            let at = glam::Vec2::new(-t.pos.z, t.pos.x);
            if at.length() < map.field_radius {
                assert!(map.open_distance(at) > 0.0, "a tree on open ground at {at}");
            }
        }
        for tower in map.towers {
            assert!(map.open(tower));
        }
    }
}
