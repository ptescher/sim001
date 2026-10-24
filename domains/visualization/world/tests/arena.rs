//! The valley of Towers in place of the Alpine Valley. The arena can be set once in a process, so
//! this is a test file of its own.

use glam::{Vec2, Vec3};
use visualization_world::terrain::{self, Arena, Tree};

fn arena() -> Arena {
    let trees = vec![Tree { pos: Vec3::new(300.0, 0.0, 300.0), height: 25.0, conifer: true, shade: 0, dead: false }];
    Arena {
        radius: 600.0,
        lake_centre: Vec2::new(-250.0, 280.0),
        lake_radius: 85.0,
        trees,
        // Open ground: a lane along the x axis.
        open_distance: Box::new(|_, z| z.abs() - 30.0),
        // A wall of mountains past 650 m.
        mountains: Box::new(|x, z| if x.hypot(z) > 650.0 { 300.0 } else { 0.0 }),
    }
}

#[test]
fn the_ground_is_flat_over_the_field_with_a_lake_in_it_and_a_forest_cut_open_by_a_lane() {
    assert!(terrain::alpine());
    terrain::set_arena(arena()).unwrap();
    assert!(!terrain::alpine() && terrain::arena().is_some());
    assert_eq!(terrain::site_title(), "Towers Valley");
    assert!(terrain::set_arena(arena()).is_err(), "only once");
    // Flat at the height the game flies over, all across the field, but not in the lake.
    for (x, z) in [(0.0, 0.0), (400.0, -300.0), (-500.0, 100.0), (30.0, 590.0)] {
        assert!(terrain::ground_height(x, z).abs() < 1e-3, "({x}, {z}) is {} m", terrain::ground_height(x, z));
    }
    // The lake is where the arena says, and water there.
    let c = terrain::lake_center();
    assert_eq!((c, terrain::lake_radius()), (Vec2::new(-250.0, 280.0), 85.0));
    assert!(terrain::is_water(c.x, c.y) && terrain::ground_height(c.x, c.y) < terrain::water_level());
    assert!(!terrain::is_water(0.0, 0.0));
    // No runway, no buildings, no alpine undergrowth; the trees are the arena's.
    assert!(!terrain::is_runway(0.0, -30.0) && terrain::buildings().is_empty() && terrain::undergrowth().is_empty());
    assert_eq!(terrain::trees().len(), 1);
    // Forest where it is not open.
    assert!(terrain::forest_density(100.0, 200.0) > 0.0 && terrain::forest_density(100.0, 0.0) < 0.0);
    // The mountains that the arena has stand on the ground.
    assert!(terrain::ground_height(0.0, 700.0) >= 300.0 && terrain::ground_height(0.0, 640.0) < 1.0);
    // The hills start past the field.
    assert!(terrain::ground_height(1500.0, 0.0).abs() > 1.0 || terrain::ground_height(0.0, 1500.0).abs() > 1.0);
}
