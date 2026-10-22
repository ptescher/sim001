//! How a drone flies its order, as far as the game cares: where it circles, and how high. The
//! server sends these as plans; anything that flies drones for the game (a headless arena) uses
//! the same numbers.

use crate::Team;

/// How far out a drone circles what it is sent to (m): inside a tower's circle of 60 m.
pub const ORBIT_RADIUS: f32 = 40.0;

/// The height drone `k` of `team` circles at (m): each its own, 4 m apart, so that drones at the
/// same tower are apart above and below.
pub fn orbit_height(team: Team, k: usize) -> f32 {
    40.0 + 8.0 * k as f32 + if team == Team::Red { 4.0 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rules;

    #[test]
    fn drones_circle_inside_the_circle_each_at_a_height_of_its_own() {
        let rules = Rules::default();
        assert!(ORBIT_RADIUS < rules.circle_radius);
        let mut heights: Vec<f32> = Team::BOTH.iter().flat_map(|&t| (0..3).map(move |k| orbit_height(t, k))).collect();
        heights.sort_by(f32::total_cmp);
        assert!(heights.windows(2).all(|w| w[1] - w[0] >= 4.0), "{heights:?}");
        assert!(heights[0] >= 30.0 && heights[5] < rules.circle_ceiling);
    }
}
