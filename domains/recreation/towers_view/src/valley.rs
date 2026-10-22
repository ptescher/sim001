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
    let lake = world(map.lake.at);
    let lake_radius = map.lake.radius;
    terrain::set_arena(Arena {
        radius: rules.field_radius,
        lake_centre: lake,
        lake_radius,
        trees,
        open_distance: Box::new(move |x, z| map.open_distance(glam::Vec2::new(-z, x))),
    })
}
