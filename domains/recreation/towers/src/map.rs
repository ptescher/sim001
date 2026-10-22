//! The valley of Towers: where the towers and the bases stand, the lake, the forest, and the lanes
//! that are cut through it (see expectations/09-towers/rules/06-trees.expect).
//!
//! The forest covers the field. A drone that flies above the trees goes where it likes, but a drone
//! that flies *low* (below the treetops) has to keep to open ground: the lanes, the clearings
//! round the towers and the bases, and the lake. The trees hide a low drone: a team sees one only
//! along a line of open ground.
//!
//! Positions are on the field: north and east of its centre (m), as everywhere in the game. The
//! map is worked out from the rules alone, so that the referee, the server, the 3D view and the
//! commander app all get the same map.

use crate::Rules;
use glam::Vec2;

/// How wide a lane is, to each side of its middle (m).
pub const LANE_HALF_WIDTH: f32 = 30.0;
/// The clearing round a tower, and round a base (m). A base is an airfield: a drone leaves the
/// ground, turns on the wing and climbs there, 150 m or more from where it started.
pub const TOWER_CLEARING: f32 = 80.0;
pub const BASE_CLEARING: f32 = 200.0;
/// The lake's radius (m).
pub const LAKE_RADIUS: f32 = 85.0;
/// The trees are this tall (m): a drone below the lowest hides.
pub const TREE_MIN: f32 = 20.0;
pub const TREE_MAX: f32 = 30.0;
/// The forest is as thick as on the field this far past it (m), and thins out over the same
/// distance again, so that the field is not a clearing in a bare plain.
const FOREST_MARGIN: f32 = 90.0;
const FOREST_THINS: f32 = 400.0;
/// A tree stands about this far from the next (m).
const TREE_SPACING: f32 = 10.0;
/// The graph of lanes is cut into pieces this long (m).
const STEP: f32 = 20.0;

/// The three lanes from home tower to home tower.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaneName {
    West,
    Mid,
    East,
    /// From a base to its home tower.
    Road,
}

impl LaneName {
    pub fn label(self) -> &'static str {
        match self {
            LaneName::West => "west",
            LaneName::Mid => "mid",
            LaneName::East => "east",
            LaneName::Road => "road",
        }
    }
}

/// A lane: a path through the forest, `LANE_HALF_WIDTH` to each side of its middle.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub name: LaneName,
    pub points: Vec<Vec2>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub at: Vec2,
    pub radius: f32,
}

impl Circle {
    pub fn contains(&self, p: Vec2) -> bool {
        self.at.distance(p) <= self.radius
    }
}

/// A tree of the forest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeSpot {
    pub at: Vec2,
    pub height: f32,
    pub conifer: bool,
}

/// A way along the lanes: where to fly, and which lane it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub points: Vec<Vec2>,
    /// The lane that most of the way is along (the roads from the bases do not count).
    pub lane: Option<LaneName>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub field_radius: f32,
    /// The towers: Blue home, west, centre, east, Red home.
    pub towers: [Vec2; 5],
    pub bases: [Vec2; 2],
    pub lake: Circle,
    pub lanes: Vec<Lane>,
    /// The open ground apart from the lanes and the lake: round each tower and each base.
    pub clearings: Vec<Circle>,
    graph: Graph,
}

/// The lanes cut into pieces: nodes `STEP` apart and the edges between them.
#[derive(Clone, Debug, PartialEq)]
struct Graph {
    nodes: Vec<Vec2>,
    edges: Vec<Vec<(usize, f32, LaneName)>>,
}

impl Map {
    pub fn new(rules: &Rules) -> Map {
        let home = rules.home_distance;
        let flank = rules.tower_distance;
        let towers = [
            Vec2::new(-home, 0.0),
            Vec2::new(45.0, -flank - 30.0),
            Vec2::ZERO,
            Vec2::new(-35.0, flank - 30.0),
            Vec2::new(home, 0.0),
        ];
        let bases = [rules.base(crate::Team::Blue), rules.base(crate::Team::Red)];
        // The lake lies on the far side of the west lane from the field's centre: the lane runs
        // along its shore.
        let (a, b) = (towers[0], towers[1]);
        let along = (b - a).normalize();
        let out = Vec2::new(along.y, -along.x).normalize();
        let out = if out.dot(Vec2::ZERO - (a + b) * 0.5) > 0.0 { -out } else { out };
        let lake = Circle { at: (a + b) * 0.5 + out * (LAKE_RADIUS + 20.0), radius: LAKE_RADIUS };
        let lane = |name: LaneName, points: Vec<Vec2>| Lane { name, points };
        let lanes = vec![
            lane(LaneName::West, vec![towers[0], towers[1], towers[4]]),
            lane(LaneName::Mid, vec![towers[0], towers[2], towers[4]]),
            lane(LaneName::East, vec![towers[0], towers[3], towers[4]]),
            lane(LaneName::Road, vec![bases[0], towers[0]]),
            lane(LaneName::Road, vec![bases[1], towers[4]]),
        ];
        let clearings = towers
            .iter()
            .map(|&at| Circle { at, radius: TOWER_CLEARING })
            .chain(bases.iter().map(|&at| Circle { at, radius: BASE_CLEARING }))
            .collect();
        let graph = Graph::of(&lanes);
        Map { field_radius: rules.field_radius, towers, bases, lake, lanes, clearings, graph }
    }

    /// Is `p` open ground: on a lane, in a clearing, or on the lake?
    pub fn open(&self, p: Vec2) -> bool {
        self.in_clearing(p)
            || self.lake.contains(p)
            || self
                .lanes
                .iter()
                .any(|l| l.points.windows(2).any(|s| segment_distance(p, s[0], s[1]) <= LANE_HALF_WIDTH))
    }

    /// How far `p` is from the nearest open ground: below 0 on it (how far in), above 0 in the
    /// forest (m).
    pub fn open_distance(&self, p: Vec2) -> f32 {
        let lanes = self
            .lanes
            .iter()
            .flat_map(|l| l.points.windows(2))
            .map(|s| segment_distance(p, s[0], s[1]) - LANE_HALF_WIDTH);
        let rounds = self.clearings.iter().chain([&self.lake]).map(|c| c.at.distance(p) - c.radius);
        lanes.chain(rounds).fold(f32::MAX, f32::min)
    }

    /// Is `p` in a clearing or on the lake: open ground wide enough to circle in?
    pub fn in_clearing(&self, p: Vec2) -> bool {
        self.clearings.iter().any(|c| c.contains(p)) || self.lake.contains(p)
    }

    /// Is the line from `a` to `b` open ground all the way? A team sees a low drone only along
    /// such a line.
    pub fn line_open(&self, a: Vec2, b: Vec2) -> bool {
        let n = ((a.distance(b) / 4.0).ceil() as usize).max(1);
        (0..=n).all(|i| self.open(a.lerp(b, i as f32 / n as f32)))
    }

    /// The clearing nearest `p`: its middle and its radius.
    pub fn nearest_clearing(&self, p: Vec2) -> Circle {
        self.clearings
            .iter()
            .copied()
            .chain([self.lake])
            .min_by(|a, b| (a.at.distance(p) - a.radius).total_cmp(&(b.at.distance(p) - b.radius)))
            .expect("a map has clearings")
    }

    /// The open ground nearest `p`: `p` itself if it is open.
    pub fn nearest_open(&self, p: Vec2) -> Vec2 {
        if self.open(p) {
            return p;
        }
        let mut best = (f32::MAX, p);
        let mut consider = |q: Vec2| {
            let d = q.distance(p);
            if d < best.0 {
                best = (d, q);
            }
        };
        for l in &self.lanes {
            for s in l.points.windows(2) {
                consider(closest_on_segment(p, s[0], s[1]));
            }
        }
        for c in self.clearings.iter().chain([&self.lake]) {
            consider(c.at + (p - c.at).normalize_or_zero() * c.radius * 0.9);
        }
        best.1
    }

    /// The way from `from` to `to` along the lanes (the shortest), from the lane nearest `from` to
    /// the lane nearest `to`. `None` if the lanes do not join them.
    pub fn path(&self, from: Vec2, to: Vec2) -> Option<Path> {
        let g = &self.graph;
        let (s, t) = (g.nearest(from), g.nearest(to));
        // Dijkstra on a few hundred nodes.
        let n = g.nodes.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![usize::MAX; n];
        let mut done = vec![false; n];
        dist[s] = 0.0;
        for _ in 0..n {
            let Some(u) =
                (0..n).filter(|&i| !done[i] && dist[i].is_finite()).min_by(|&a, &b| dist[a].total_cmp(&dist[b]))
            else {
                break;
            };
            done[u] = true;
            if u == t {
                break;
            }
            for &(v, w, _) in &g.edges[u] {
                if dist[u] + w < dist[v] {
                    dist[v] = dist[u] + w;
                    prev[v] = u;
                }
            }
        }
        if !dist[t].is_finite() {
            return None;
        }
        let mut nodes = vec![t];
        while *nodes.last().unwrap() != s {
            nodes.push(prev[*nodes.last().unwrap()]);
        }
        nodes.reverse();
        // Which lane the way is along: the one with most of its length.
        let mut length = std::collections::HashMap::new();
        for w in nodes.windows(2) {
            if let Some(&(_, d, name)) = g.edges[w[0]].iter().find(|e| e.0 == w[1]) {
                *length.entry(name).or_insert(0.0) += d;
            }
        }
        let lane = length
            .iter()
            .filter(|(name, _)| **name != LaneName::Road)
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(name, _)| *name)
            .or_else(|| length.keys().next().copied());
        // Only the corners (and the ends) are way points.
        let pts: Vec<Vec2> = nodes.iter().map(|&i| g.nodes[i]).collect();
        let mut points = vec![pts[0]];
        for w in pts.windows(3) {
            let (d0, d1) = ((w[1] - w[0]).normalize_or_zero(), (w[2] - w[1]).normalize_or_zero());
            if d0.dot(d1) < 0.995 {
                points.push(w[1]);
            }
        }
        if pts.len() > 1 {
            points.push(*pts.last().unwrap());
        }
        Some(Path { points, lane })
    }

    /// The trees of the forest: the same every time, with none on open ground.
    pub fn trees(&self) -> Vec<TreeSpot> {
        let reach = self.field_radius + FOREST_MARGIN + FOREST_THINS;
        let n = (reach / TREE_SPACING).ceil() as i32;
        let mut out = Vec::new();
        for ix in -n..=n {
            for iz in -n..=n {
                let jx = hash(ix * 7 + 1, iz * 13 + 5) - 0.5;
                let jz = hash(ix * 3 + 9, iz * 5 + 2) - 0.5;
                let at = Vec2::new((ix as f32 + jx * 0.8) * TREE_SPACING, (iz as f32 + jz * 0.8) * TREE_SPACING);
                // Leave a margin round the open ground, so that no crown hangs over a lane.
                if at.length() > reach || self.open_with_margin(at, 6.0) {
                    continue;
                }
                // Past the field the woods thin out.
                let past = (at.length() - self.field_radius - FOREST_MARGIN).max(0.0) / FOREST_THINS;
                if hash(ix * 19 + 5, iz * 23 + 1) < past {
                    continue;
                }
                // Stands of an age: tall in an old one, short in a young one.
                let age = hash(ix.div_euclid(6) * 31 + 3, iz.div_euclid(6) * 17 + 11);
                let height = (TREE_MIN + (TREE_MAX - TREE_MIN) * age) * (0.9 + 0.2 * hash(ix + 77, iz - 33));
                out.push(TreeSpot {
                    at,
                    height: height.clamp(TREE_MIN, TREE_MAX),
                    conifer: hash(ix - 5, iz + 9) > 0.18,
                });
            }
        }
        out
    }

    fn open_with_margin(&self, p: Vec2, margin: f32) -> bool {
        self.clearings.iter().any(|c| c.at.distance(p) <= c.radius + margin)
            || self.lake.at.distance(p) <= self.lake.radius + margin
            || self
                .lanes
                .iter()
                .any(|l| l.points.windows(2).any(|s| segment_distance(p, s[0], s[1]) <= LANE_HALF_WIDTH + margin))
    }
}

impl Graph {
    fn of(lanes: &[Lane]) -> Graph {
        let mut nodes: Vec<Vec2> = Vec::new();
        let mut edges: Vec<Vec<(usize, f32, LaneName)>> = Vec::new();
        // A node at a point, shared by the lanes that meet there.
        let node = |p: Vec2, nodes: &mut Vec<Vec2>, edges: &mut Vec<Vec<_>>| {
            if let Some(i) = nodes.iter().position(|q| q.distance(p) < 1.0) {
                return i;
            }
            nodes.push(p);
            edges.push(Vec::new());
            nodes.len() - 1
        };
        for lane in lanes {
            for s in lane.points.windows(2) {
                let pieces = ((s[0].distance(s[1]) / STEP).ceil() as usize).max(1);
                let mut last = node(s[0], &mut nodes, &mut edges);
                for i in 1..=pieces {
                    let p = s[0].lerp(s[1], i as f32 / pieces as f32);
                    let next = node(p, &mut nodes, &mut edges);
                    let d = nodes[last].distance(nodes[next]);
                    edges[last].push((next, d, lane.name));
                    edges[next].push((last, d, lane.name));
                    last = next;
                }
            }
        }
        Graph { nodes, edges }
    }

    fn nearest(&self, p: Vec2) -> usize {
        (0..self.nodes.len())
            .min_by(|&a, &b| self.nodes[a].distance(p).total_cmp(&self.nodes[b].distance(p)))
            .expect("a graph has nodes")
    }
}

fn closest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let t = if ab.length_squared() == 0.0 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    a + ab * t
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(closest_on_segment(p, a, b))
}

fn hash(ix: i32, iz: i32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(374_761_393) ^ (iz as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Map {
        Map::new(&Rules::default())
    }

    #[test]
    fn the_towers_and_bases_stand_in_clearings_and_are_open() {
        let m = map();
        for t in m.towers.iter().chain(&m.bases) {
            assert!(m.open(*t) && m.in_clearing(*t), "{t}");
            assert!(t.length() < m.field_radius, "{t} is in the field");
        }
    }

    #[test]
    fn the_towers_are_not_laid_out_the_same_for_each_team() {
        let m = map();
        // The flank towers are not mirror images of each other.
        assert!((m.towers[1].x - m.towers[3].x).abs() > 20.0, "{:?}", m.towers);
        assert!((m.towers[1].length() - m.towers[3].length()).abs() > 20.0);
    }

    #[test]
    fn the_lake_is_between_the_blue_home_tower_and_the_west_tower_and_the_lane_runs_along_it() {
        let m = map();
        let (a, b) = (m.towers[0], m.towers[1]);
        let mid = (a + b) * 0.5;
        assert!(m.lake.at.distance(mid) < m.lake.radius * 2.0, "{:?}", m.lake);
        assert!(m.lake.at.distance(a) > m.lake.radius * 2.0 && m.lake.at.distance(b) > m.lake.radius * 2.0);
        // The west lane's middle is a few metres off the shore, so that its edge meets the lake.
        let west = m.lanes.iter().find(|l| l.name == LaneName::West).unwrap();
        let d = segment_distance(m.lake.at, west.points[0], west.points[1]);
        assert!(d > m.lake.radius && d < m.lake.radius + LANE_HALF_WIDTH, "{d}");
        assert!(m.lake.at.length() + m.lake.radius < m.field_radius, "the lake is in the field");
    }

    #[test]
    fn the_lanes_and_the_clearings_are_open_and_the_forest_is_not() {
        let m = map();
        assert!(m.open(Vec2::new(-200.0, 20.0)), "on the mid lane");
        assert!(!m.open(Vec2::new(-200.0, 60.0)), "in the forest beside it");
        assert!(m.open(m.lake.at), "the lake");
        assert!(!m.open(Vec2::new(300.0, 300.0)));
    }

    #[test]
    fn the_distance_to_open_ground_is_negative_on_it_and_grows_into_the_forest() {
        let m = map();
        assert!(m.open_distance(Vec2::new(-200.0, 0.0)) <= -29.0, "the middle of a lane");
        assert!((m.open_distance(Vec2::new(-200.0, 30.0))).abs() < 0.01, "the edge of a lane");
        assert!((m.open_distance(Vec2::new(-200.0, 50.0)) - 20.0).abs() < 0.01);
        for p in [Vec2::new(-200.0, 70.0), Vec2::new(10.0, 10.0), m.lake.at, Vec2::new(300.0, 300.0)] {
            assert_eq!(m.open_distance(p) <= 0.0, m.open(p), "{p}");
        }
    }

    #[test]
    fn a_line_along_a_lane_or_across_the_lake_is_open_and_one_through_the_forest_is_not() {
        let m = map();
        assert!(m.line_open(Vec2::new(-300.0, 0.0), Vec2::new(-180.0, 0.0)));
        assert!(!m.line_open(Vec2::new(-300.0, 0.0), Vec2::new(-300.0, 130.0)));
        let shore = (m.towers[0] + m.towers[1]) * 0.5;
        assert!(m.line_open(shore, m.lake.at), "from the lane across the shore to the lake");
        assert!(shore.distance(m.lake.at) < 150.0);
    }

    #[test]
    fn the_forest_has_no_tree_on_open_ground_and_trees_are_the_same_every_time() {
        let m = map();
        let trees = m.trees();
        assert!(trees.len() > 8_000 && trees.len() < 60_000, "{} trees", trees.len());
        assert!(trees.iter().all(|t| !m.open(t.at)), "a tree on a lane or in a clearing");
        assert!(trees.iter().all(|t| (TREE_MIN..=TREE_MAX).contains(&t.height)));
        assert_eq!(trees, m.trees());
        // Trees stand right up to the edge of the lanes, so that a lane is a corridor.
        let near =
            trees.iter().filter(|t| t.at.x.abs() < 300.0 && (t.at.y.abs() - LANE_HALF_WIDTH - 6.0).abs() < 6.0).count();
        assert!(near > 20, "{near} trees along the mid lane");
    }

    #[test]
    fn a_path_from_the_blue_home_tower_to_the_west_tower_is_along_the_west_lane() {
        let m = map();
        let p = m.path(m.towers[0], m.towers[1]).unwrap();
        assert_eq!(p.lane, Some(LaneName::West));
        assert!(p.points[0].distance(m.towers[0]) < STEP && p.points.last().unwrap().distance(m.towers[1]) < STEP);
        assert!(p.points.iter().all(|&q| m.open(q)));
    }

    #[test]
    fn a_path_from_a_base_goes_by_its_road_and_then_the_lane() {
        let m = map();
        let p = m.path(m.bases[0], m.towers[3]).unwrap();
        assert_eq!(p.lane, Some(LaneName::East));
        assert!(p.points.len() >= 3, "{:?}", p.points);
    }

    #[test]
    fn the_nearest_open_ground_of_a_point_in_the_forest_is_open() {
        let m = map();
        let q = m.nearest_open(Vec2::new(-250.0, 90.0));
        assert!(m.open(q), "{q}");
        assert!(q.distance(Vec2::new(-250.0, 90.0)) < 70.0);
    }
}
