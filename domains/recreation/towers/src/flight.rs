//! How a drone flies its order, as far as the game cares: where it circles, and how high, and for
//! an order flown low, the way along the lanes. The server sends these as plans; anything that
//! flies drones for the game (a headless arena) uses the same numbers.

use crate::Team;
use crate::map::{LaneName, Map};
use glam::Vec2;

/// How far out a drone circles what it is sent to (m): inside a tower's circle of 60 m.
pub const ORBIT_RADIUS: f32 = 40.0;

/// The height drone `k` of `team` circles at (m): each its own, 4 m apart, so that drones at the
/// same tower are apart above and below. Well over the trees (20 to 30 m): a drone dips 15 m or
/// more in a hard turn.
pub fn orbit_height(team: Team, k: usize) -> f32 {
    55.0 + 8.0 * k as f32 + if team == Team::Red { 4.0 } else { 0.0 }
}

/// The height drone `k` of `team` circles at when it flies low (m): below the treetops (20 m) and
/// over the AGENT envelope's least height (its floor of 8 m and 8 m more), each its own.
pub fn low_height(team: Team, k: usize) -> f32 {
    16.0 + 1.5 * k as f32 + if team == Team::Red { 0.75 } else { 0.0 }
}

/// How far ahead along its way a drone aims, so that it follows a lane (m).
const LOOKAHEAD: f32 = 70.0;
/// A drone this near the end of its way (m) has arrived: it is well inside the clearing, and turns
/// there, not in the lane.
const ARRIVED: f32 = 35.0;
/// A drone that circles stays at it while it is within this of the end of the way (m).
const STAYS: f32 = 75.0;
/// The circle a drone flies at the end of its way is this far to the right of where the way ends
/// (m), so that the way runs along it: the drone flies on into the circle and is not turned
/// across the lane. The circle stays inside the tower's circle (60 m) with its radius of 40 m.
const TANGENT: f32 = 15.0;
/// Above this height (m) a drone is clear of the trees and flies on.
const CLEAR: f32 = 45.0;

/// How a drone gets to its order: straight over the forest, or low along the lanes.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Where the order is (the tower, the point, the base).
    pub target: Vec2,
    pub low: bool,
    /// The way along the lanes, corner to corner, for a drone that flies low. Empty for a drone
    /// that goes straight.
    pub path: Vec<Vec2>,
    /// The lane the way is mostly along.
    pub lane: Option<LaneName>,
    /// The drone circles low at the end (the target is in a clearing, or on the lake); if not,
    /// it climbs above the trees and circles high.
    pub circle_low: bool,
    /// Where a drone that circles high after a low way climbs: the middle of the clearing at the
    /// end of the way.
    pub climb_at: Vec2,
}

/// The plan for a drone at `from` to fly to `target`, `low` or high.
pub fn plan(map: &Map, from: Vec2, target: Vec2, low: bool) -> Plan {
    let straight = Plan { target, low, path: vec![], lane: None, circle_low: false, climb_at: target };
    if !low {
        return straight;
    }
    let circle_low = map.in_clearing(target);
    let end = if circle_low { target } else { map.nearest_clearing(target).at };
    match map.path(map.nearest_open(from), end) {
        Some(path) => Plan { target, low, path: path.points, lane: path.lane, circle_low, climb_at: end },
        None => straight,
    }
}

/// What a drone is told to do now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leg {
    /// Circle the point, or fly to it.
    pub orbit: bool,
    pub point: Vec2,
    pub alt: f32,
    /// How far out it circles (m).
    pub radius: f32,
}

/// The leg of drone `k` of `team`, at `pos` and `alt` now, for `plan`.
pub fn leg(map: &Map, plan: &Plan, team: Team, k: usize, pos: Vec2, alt: f32) -> Leg {
    let high = orbit_height(team, k);
    let circle = |point: Vec2, alt: f32| Leg { orbit: true, point, alt, radius: ORBIT_RADIUS };
    if !plan.low || plan.path.is_empty() {
        // A drone that is still below the treetops (it has just taken off, or it was flying low)
        // climbs before it goes: in the clearing it is in, or along the lane to the nearest one.
        if alt < CLEAR {
            let clearing = map.nearest_clearing(pos);
            if map.in_clearing(pos) {
                return circle(clearing.at, high);
            }
            return match map.path(map.nearest_open(pos), clearing.at) {
                Some(path) => Leg { orbit: false, point: ahead(&path.points, pos, LOOKAHEAD), alt: high, radius: 0.0 },
                None => circle(plan.target, high),
            };
        }
        return circle(plan.target, high);
    }
    // Over the forest, stay up until over open ground.
    if !map.open(pos) {
        return Leg { orbit: false, point: map.nearest_open(pos), alt: high, radius: 0.0 };
    }
    let end = *plan.path.last().unwrap();
    // Arrived: well inside the clearing; and once it circles, it stays on the circle.
    let beside_of = |c: Vec2| c + right_of(&plan.path) * TANGENT;
    let circling = pos.distance(end) <= STAYS
        && pos.distance(beside_of(if plan.circle_low { plan.target } else { plan.climb_at })) <= ORBIT_RADIUS + 20.0;
    if pos.distance(end) <= ARRIVED || circling {
        let beside = |c: Vec2| c + right_of(&plan.path) * TANGENT;
        return if plan.circle_low {
            circle(beside(plan.target), low_height(team, k))
        } else if alt < CLEAR {
            // Climb inside the clearing, then fly over the trees.
            circle(beside(plan.climb_at), high)
        } else {
            circle(plan.target, high)
        };
    }
    // The way runs on past its end, so that the drone is never sent to a point it is about to
    // arrive at (the executor turns a drone round on arrival): it flies on in the lane until it is
    // in the clearing, and then circles.
    let mut way = plan.path.clone();
    let n = way.len();
    if n >= 2 {
        way.push(way[n - 1] + (way[n - 1] - way[n - 2]).normalize_or_zero() * LOOKAHEAD);
    }
    Leg { orbit: false, point: ahead(&way, pos, LOOKAHEAD), alt: low_height(team, k), radius: 0.0 }
}

/// The direction to the right of the way's last leg.
fn right_of(path: &[Vec2]) -> Vec2 {
    let n = path.len();
    if n < 2 {
        return Vec2::ZERO;
    }
    let d = (path[n - 1] - path[n - 2]).normalize_or_zero();
    Vec2::new(-d.y, d.x)
}

/// The point `d` m further along the polyline `path` than the point of it nearest `pos`.
fn ahead(path: &[Vec2], pos: Vec2, d: f32) -> Vec2 {
    if path.len() < 2 {
        return path.first().copied().unwrap_or(pos);
    }
    let mut best = (f32::MAX, 0usize, 0.0f32);
    for (i, s) in path.windows(2).enumerate() {
        let ab = s[1] - s[0];
        let t = ((pos - s[0]).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let dist = pos.distance(s[0] + ab * t);
        if dist < best.0 {
            best = (dist, i, t * ab.length());
        }
    }
    let (_, mut i, mut s) = best;
    let mut left = d;
    loop {
        let len = path[i].distance(path[i + 1]);
        if s + left <= len {
            return path[i].lerp(path[i + 1], (s + left) / len.max(1e-6));
        }
        left -= len - s;
        s = 0.0;
        i += 1;
        if i + 1 >= path.len() {
            return *path.last().unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rules;

    fn map() -> Map {
        Map::new(&Rules::default())
    }

    #[test]
    fn drones_circle_inside_the_circle_each_at_a_height_of_its_own() {
        let rules = Rules::default();
        assert!(ORBIT_RADIUS < rules.circle_radius);
        let mut heights: Vec<f32> = Team::BOTH.iter().flat_map(|&t| (0..3).map(move |k| orbit_height(t, k))).collect();
        heights.sort_by(f32::total_cmp);
        assert!(heights.windows(2).all(|w| w[1] - w[0] >= 4.0), "{heights:?}");
        assert!(heights[0] >= rules.treetop + 30.0 && heights[5] < rules.circle_ceiling);
    }

    #[test]
    fn a_low_drone_flies_below_the_treetops_and_above_the_floor_each_at_a_height_of_its_own() {
        let rules = Rules::default();
        let mut heights: Vec<f32> = Team::BOTH.iter().flat_map(|&t| (0..3).map(move |k| low_height(t, k))).collect();
        heights.sort_by(f32::total_cmp);
        assert!(heights.windows(2).all(|w| w[1] - w[0] >= 0.75), "{heights:?}");
        assert!(heights[0] >= 16.0 && heights[5] < rules.treetop, "{heights:?}");
    }

    #[test]
    fn a_high_order_goes_straight_and_circles_at_the_target() {
        let m = map();
        let p = plan(&m, m.towers[0], m.towers[1], false);
        assert!(p.path.is_empty() && p.lane.is_none() && !p.circle_low);
        let l = leg(&m, &p, Team::Blue, 0, m.towers[0], 60.0);
        assert_eq!((l.orbit, l.point, l.alt, l.radius), (true, m.towers[1], orbit_height(Team::Blue, 0), ORBIT_RADIUS));
    }

    #[test]
    fn a_drone_below_the_treetops_climbs_in_its_clearing_before_a_high_order_takes_it_away() {
        let m = map();
        let p = plan(&m, m.bases[0], m.towers[1], false);
        // Just off the ground at the base: circle over the base, up to its height.
        let l = leg(&m, &p, Team::Blue, 2, m.bases[0] + Vec2::new(0.0, 10.0), 15.0);
        assert_eq!((l.orbit, l.point, l.alt), (true, m.bases[0], orbit_height(Team::Blue, 2)));
        // High enough: on to the tower.
        let l = leg(&m, &p, Team::Blue, 2, m.bases[0] + Vec2::new(0.0, 10.0), 60.0);
        assert_eq!((l.orbit, l.point), (true, m.towers[1]));
        // Flying low in a lane and told to go high: it follows the lane to a clearing, climbing.
        let in_lane = Vec2::new(-300.0, 0.0);
        let l = leg(&m, &p, Team::Blue, 2, in_lane, 12.0);
        assert!(!l.orbit && l.alt == orbit_height(Team::Blue, 2) && m.open(l.point), "{l:?}");
    }

    #[test]
    fn a_low_order_to_a_tower_goes_along_the_west_lane_and_circles_low() {
        let m = map();
        let p = plan(&m, m.towers[0], m.towers[1], true);
        assert_eq!(p.lane, Some(LaneName::West));
        assert!(p.circle_low);
        // On the way it aims ahead along the lane, low.
        let l = leg(&m, &p, Team::Blue, 1, m.towers[0], 15.0);
        assert!(!l.orbit && l.alt == low_height(Team::Blue, 1));
        assert!(m.open(l.point) && l.point.distance(m.towers[0]) > 50.0);
        // At the end it circles the tower, low.
        let l = leg(&m, &p, Team::Blue, 1, m.towers[1] + Vec2::new(10.0, 10.0), 13.0);
        assert_eq!((l.orbit, l.alt), (true, low_height(Team::Blue, 1)));
        assert!((l.point.distance(m.towers[1]) - TANGENT).abs() < 0.01, "a circle beside the way's end");
        assert!(l.radius + TANGENT < crate::map::TOWER_CLEARING - 20.0, "it circles well inside the clearing");
        assert!(l.radius + TANGENT <= 60.0, "and inside the tower's circle");
    }

    #[test]
    fn a_low_order_to_the_forest_goes_to_the_clearing_climbs_and_circles_high() {
        let m = map();
        let target = Vec2::new(-100.0, 100.0);
        assert!(!m.open(target));
        let p = plan(&m, m.towers[0], target, true);
        assert_eq!(p.lane, Some(LaneName::Mid));
        assert!(!p.circle_low && p.climb_at == m.towers[2]);
        let at_end = m.towers[2] + Vec2::new(20.0, 0.0);
        let low = leg(&m, &p, Team::Red, 2, at_end, 15.0);
        assert!(low.orbit && low.point.distance(m.towers[2]) < 16.0, "{low:?}");
        assert!(low.alt >= 55.0, "it climbs in the clearing");
        let up = leg(&m, &p, Team::Red, 2, at_end, 60.0);
        assert_eq!((up.orbit, up.point), (true, target));
    }

    #[test]
    fn a_low_order_to_where_the_drone_is_has_no_way_to_go() {
        let m = map();
        // Over the node of the lane nearest the tower, sent to the tower: a way of one point.
        let p = plan(&m, m.towers[1], m.towers[1], true);
        assert_eq!(p.path.len(), 1);
        let l = leg(&m, &p, Team::Red, 0, m.towers[1] + Vec2::new(0.0, 30.0), 12.0);
        assert!(l.orbit, "it is there: it circles");
        // And a drone sent low to a tower 100 m up the lane, from just before the end of the way.
        let q = plan(&m, m.towers[0], m.towers[0] + Vec2::new(60.0, 0.0), true);
        let _ = leg(&m, &q, Team::Blue, 0, m.towers[0], 12.0);
    }

    #[test]
    fn a_drone_over_the_forest_stays_up_until_it_is_over_open_ground() {
        let m = map();
        let p = plan(&m, Vec2::new(-100.0, 150.0), m.towers[1], true);
        let from = Vec2::new(-100.0, 150.0);
        assert!(!m.open(from));
        let l = leg(&m, &p, Team::Blue, 0, from, 50.0);
        assert!(!l.orbit && m.open(l.point) && l.alt >= 55.0, "{l:?}");
    }

    #[test]
    fn the_leg_never_aims_off_the_lane() {
        let m = map();
        let p = plan(&m, m.bases[0], m.towers[3], true);
        let mut pos = m.bases[0];
        for _ in 0..200 {
            let l = leg(&m, &p, Team::Blue, 0, pos, 14.0);
            assert!(m.open(l.point), "{:?} aims at {}", l, l.point);
            if l.orbit {
                break;
            }
            pos += (l.point - pos).normalize_or_zero() * 15.0;
            assert!(m.open(pos), "a drone flying straight at its aim is off the lane at {pos}");
        }
    }
}
