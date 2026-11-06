//! The valley of Towers: where the towers and the bases stand, the lake, the forest, and the lanes
//! that are cut through it (see expectations/09-towers/rules/06-trees.expect).
//!
//! The forest covers the field. A drone that flies above the trees goes where it likes, but a drone
//! that flies *low* (below the treetops) has to keep to open ground: the lanes, the clearings
//! round the towers and the bases, and the lake. The trees hide a low drone: a team sees one only
//! along a line of open ground.
//!
//! Tall mountains stand round the edge of the field (see expectations/09-towers/rules/08-mountains.expect):
//! they rise from [`MOUNTAINS_FROM`] to [`MOUNTAINS_FULL`] from the centre and are far higher than a drone flies.
//! They leave a clearing round each base, each tower and the lake, so that the drones can start,
//! and fight, in the open. The edge of the field is no road from one base to the other.
//!
//! Positions are on the field: north and east of its centre (m), as everywhere in the game. The
//! map is worked out from the rules alone, so that the referee, the server, the 3D view and the
//! commander app all get the same map.

use crate::{Kind, Rules, Team};
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
/// The mountains rise from this distance from the centre (m) and are at full height at the next.
pub const MOUNTAINS_FROM: f32 = 400.0;
pub const MOUNTAINS_FULL: f32 = 500.0;
/// How tall the mountains are at most (m); the ridges are between 75 % and 100 % of it.
pub const MOUNTAIN_HEIGHT: f32 = 400.0;
/// The mountains stay this far off the clearings (m), and then rise over this distance.
const MOUNTAIN_MARGIN: f32 = 60.0;
const MOUNTAIN_FOOT: f32 = 100.0;
/// The ridges are shaped by noise with cells this big (m).
const RIDGE_CELL: f32 = 120.0;
/// A drone that circles a point needs this much room round it (m): the mountains are not nearer.
const ROOM: f32 = 60.0;
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

/// A lane: a path through the forest, about `LANE_HALF_WIDTH` to each side of its middle.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub name: LaneName,
    /// What it joins: towers (or a base and its home tower), in order.
    pub through: Vec<Vec2>,
    /// Its middle, as it winds from one to the next: it starts at the first, passes each and ends
    /// at the last.
    pub points: Vec<Vec2>,
}

/// The lake: round about `at`, `radius` on average, its shore nearer or further in each direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lake {
    pub at: Vec2,
    pub radius: f32,
}

impl Lake {
    /// How far the shore is from `at` in the direction `dir` (m).
    pub fn shore(&self, dir: Vec2) -> f32 {
        let _ = dir;
        self.radius
    }

    /// How far `p` is from the shore: below 0 on the water (m, along the line from `at`).
    pub fn distance(&self, p: Vec2) -> f32 {
        self.at.distance(p) - self.shore(p - self.at)
    }

    pub fn contains(&self, p: Vec2) -> bool {
        self.distance(p) <= 0.0
    }

    /// The shore, as `n` points round it.
    pub fn outline(&self, n: usize) -> Vec<Vec2> {
        (0..n)
            .map(|k| {
                let dir = Vec2::from_angle(k as f32 / n as f32 * std::f32::consts::TAU);
                self.at + dir * self.shore(dir)
            })
            .collect()
    }
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

/// A tower of a layout: its name, kind, place and who holds it at the start.
#[derive(Clone, Debug, PartialEq)]
pub struct TowerSpec {
    pub name: String,
    pub kind: Kind,
    pub at: Vec2,
    pub holder: Option<Team>,
}

/// Where the towers stand and which lanes join them: the part of a map that differs between
/// games. The lakes, the clearings, the forest and the mountains follow from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub towers: Vec<TowerSpec>,
    /// Each lane runs through towers, given by their number in `towers`, from home tower to home
    /// tower. The roads from the bases to the home towers are not here: the map adds them.
    pub lanes: Vec<(LaneName, Vec<usize>)>,
}

/// Which layout a game is played on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LayoutKind {
    /// Five towers: a home tower each, and a west, a centre and an east tower.
    #[default]
    Valley,
    /// Eleven towers: the valley's five, and an outpost halfway along each lane on each side.
    Chain,
}

impl Layout {
    /// The layout that `rules` ask for.
    pub fn of(rules: &Rules) -> Layout {
        match rules.layout {
            LayoutKind::Valley => Layout::valley(rules),
            LayoutKind::Chain => Layout::chain(rules),
        }
    }

    /// The chain: the valley's five towers, then an outpost halfway between a home tower and the
    /// west, the centre and the east tower, first Blue's three, then Red's.
    pub fn chain(rules: &Rules) -> Layout {
        let mut layout = Layout::valley(rules);
        let at = |k: usize| layout.towers[k].at;
        let mut outposts = vec![];
        for (home, side) in [(0, "Blue"), (4, "Red")] {
            for (far, lane) in [(1, "west"), (2, "mid"), (3, "east")] {
                outposts.push(TowerSpec {
                    name: format!("{side} {lane} outpost"),
                    kind: Kind::Outpost,
                    at: (at(home) + at(far)) * 0.5,
                    holder: None,
                });
            }
        }
        layout.towers.extend(outposts);
        layout.lanes = vec![
            (LaneName::West, vec![0, 5, 1, 8, 4]),
            (LaneName::Mid, vec![0, 6, 2, 9, 4]),
            (LaneName::East, vec![0, 7, 3, 10, 4]),
        ];
        layout
    }

    /// The valley: five towers, three lanes.
    pub fn valley(rules: &Rules) -> Layout {
        let (home, flank) = (rules.home_distance, rules.tower_distance);
        let spec = |name: &str, kind, at, holder| TowerSpec { name: name.to_string(), kind, at, holder };
        Layout {
            towers: vec![
                spec("Blue home", Kind::Fort, Vec2::new(-home, 0.0), Some(Team::Blue)),
                spec("west", Kind::Depot, Vec2::new(45.0, -flank - 30.0), None),
                spec("centre", Kind::Spire, Vec2::ZERO, None),
                spec("east", Kind::Depot, Vec2::new(-35.0, flank - 30.0), None),
                spec("Red home", Kind::Fort, Vec2::new(home, 0.0), Some(Team::Red)),
            ],
            lanes: vec![
                (LaneName::West, vec![0, 1, 4]),
                (LaneName::Mid, vec![0, 2, 4]),
                (LaneName::East, vec![0, 3, 4]),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub field_radius: f32,
    /// The towers' places, as the layout has them (the valley: Blue home, west, centre, east, Red home).
    pub towers: Vec<Vec2>,
    pub layout: Layout,
    pub bases: [Vec2; 2],
    pub lake: Lake,
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
        let layout = Layout::of(rules);
        let towers: Vec<Vec2> = layout.towers.iter().map(|t| t.at).collect();
        let bases = [rules.base(crate::Team::Blue), rules.base(crate::Team::Red)];
        // The lake lies on the far side of the west lane from the field's centre: the lane runs
        // along its shore.
        let (a, b) = (towers[0], towers[1]);
        let along = (b - a).normalize();
        let out = Vec2::new(along.y, -along.x).normalize();
        let out = if out.dot(Vec2::ZERO - (a + b) * 0.5) > 0.0 { -out } else { out };
        let lake = Lake { at: (a + b) * 0.5 + out * (LAKE_RADIUS + 20.0), radius: LAKE_RADIUS };
        let lane = |name: LaneName, through: Vec<Vec2>| Lane { name, points: through.clone(), through };
        let mut lanes: Vec<Lane> = layout
            .lanes
            .iter()
            .map(|(name, through)| lane(*name, through.iter().map(|&t| towers[t]).collect()))
            .collect();
        let homes: Vec<usize> = [Team::Blue, Team::Red]
            .iter()
            .map(|&t| layout.towers.iter().position(|s| s.holder == Some(t)).expect("a home tower"))
            .collect();
        lanes.push(lane(LaneName::Road, vec![bases[0], towers[homes[0]]]));
        lanes.push(lane(LaneName::Road, vec![bases[1], towers[homes[1]]]));
        let clearings = towers
            .iter()
            .map(|&at| Circle { at, radius: TOWER_CLEARING })
            .chain(bases.iter().map(|&at| Circle { at, radius: BASE_CLEARING }))
            .collect();
        let graph = Graph::of(&lanes);
        Map { field_radius: rules.field_radius, towers, layout, bases, lake, lanes, clearings, graph }
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
        let rounds = self.clearings.iter().map(|c| c.at.distance(p) - c.radius);
        lanes.chain(rounds).chain([self.lake.distance(p)]).fold(f32::MAX, f32::min)
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
            .chain([Circle { at: self.lake.at, radius: self.lake.shore(p - self.lake.at) }])
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
        for c in &self.clearings {
            consider(c.at + (p - c.at).normalize_or_zero() * c.radius * 0.9);
        }
        consider(self.lake.at + (p - self.lake.at).normalize_or_zero() * self.lake.shore(p - self.lake.at) * 0.9);
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

    /// How tall the mountains are at `p` (m): 0 on the field, and up to [`MOUNTAIN_HEIGHT`] at its edge.
    pub fn mountain_height(&self, p: Vec2) -> f32 {
        let rise = smooth(MOUNTAINS_FROM, MOUNTAINS_FULL, p.length());
        if rise <= 0.0 {
            return 0.0;
        }
        // The distance from the open ground that the mountains leave clear.
        let clear = self
            .bases
            .iter()
            .map(|b| b.distance(p) - BASE_CLEARING)
            .chain(self.towers.iter().map(|t| t.distance(p) - TOWER_CLEARING))
            .chain([self.lake.distance(p)])
            .fold(f32::MAX, f32::min);
        let foot = smooth(0.0, MOUNTAIN_FOOT, clear - MOUNTAIN_MARGIN);
        MOUNTAIN_HEIGHT * rise * foot * (0.75 + 0.25 * ridges(p))
    }

    /// The point nearest `p`, on the line to the centre, that is clear of the mountains with room
    /// to circle: `p` itself if it is. The drones are never sent into the mountains.
    pub fn keep_clear(&self, p: Vec2) -> Vec2 {
        let mut q = p;
        while q.length() > 1.0 && self.mountain_height(q + q.normalize() * ROOM) > 1.0 {
            q *= 0.97;
        }
        q
    }

    /// Can a drone that flies no higher than `ceiling` get round the outside of the field from one
    /// base to the other, outside the tower ground (further out than [`MOUNTAINS_FROM`] + 50 m)?
    pub fn edge_open(&self, ceiling: f32) -> bool {
        let (cell, half) = (10.0, 650.0);
        let n = (2.0 * half / cell) as usize;
        let open = |ix: usize, iz: usize| {
            let p = Vec2::new(ix as f32 * cell - half, iz as f32 * cell - half);
            p.length() >= MOUNTAINS_FROM + 50.0 && p.length() <= half && self.mountain_height(p) < ceiling
        };
        let index = |p: Vec2| (((p.x + half) / cell) as usize, ((p.y + half) / cell) as usize);
        let (from, to) = (index(self.bases[0]), index(self.bases[1]));
        let mut seen = vec![false; (n + 1) * (n + 1)];
        let mut todo = vec![from];
        seen[from.0 * (n + 1) + from.1] = true;
        while let Some((x, z)) = todo.pop() {
            if (x, z) == to {
                return true;
            }
            for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if nx < 0 || nz < 0 || nx > n as i32 || nz > n as i32 {
                    continue;
                }
                let (nx, nz) = (nx as usize, nz as usize);
                if !seen[nx * (n + 1) + nz] && open(nx, nz) {
                    seen[nx * (n + 1) + nz] = true;
                    todo.push((nx, nz));
                }
            }
        }
        false
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
                if at.length() > reach || self.open_with_margin(at, 6.0) || self.mountain_height(at) > 1.0 {
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
            || self.lake.distance(p) <= margin
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

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth noise between 0 and 1, the same every time: the ridges and the valleys of the mountains.
fn ridges(p: Vec2) -> f32 {
    let (x, z) = (p.x / RIDGE_CELL, p.y / RIDGE_CELL);
    let (ix, iz) = (x.floor(), z.floor());
    let (fx, fz) = (smooth(0.0, 1.0, x - ix), smooth(0.0, 1.0, z - iz));
    let h = |dx: f32, dz: f32| hash(ix as i32 + dx as i32 + 101, iz as i32 + dz as i32 - 57);
    let top = h(0.0, 0.0) + (h(1.0, 0.0) - h(0.0, 0.0)) * fx;
    let bottom = h(0.0, 1.0) + (h(1.0, 1.0) - h(0.0, 1.0)) * fx;
    top + (bottom - top) * fz
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
        let (d, near) = west
            .points
            .windows(2)
            .map(|s| (segment_distance(m.lake.at, s[0], s[1]), closest_on_segment(m.lake.at, s[0], s[1])))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap();
        let shore = m.lake.shore(near - m.lake.at);
        assert!(d > shore && d < shore + LANE_HALF_WIDTH - 5.0, "{d} from the middle of the lake, the shore {shore}");
        let far = m.lake.outline(72).iter().map(|p| p.length()).fold(0.0, f32::max);
        assert!(far < m.field_radius, "the lake is in the field");
    }

    #[test]
    fn the_lanes_and_the_clearings_are_open_and_the_forest_is_not() {
        let m = map();
        assert!(m.open(Vec2::new(-200.0, 5.0)), "on the mid lane");
        assert!(!m.open(Vec2::new(-200.0, 60.0)), "in the forest beside it");
        assert!(m.open(m.lake.at), "the lake");
        assert!(!m.open(Vec2::new(300.0, 300.0)));
    }

    #[test]
    fn the_distance_to_open_ground_is_negative_on_it_and_grows_into_the_forest() {
        let m = map();
        let mid = m.lanes.iter().find(|l| l.name == LaneName::Mid).unwrap();
        // The middle of the lane halfway from the Blue home tower to the centre, in the forest.
        let half = mid.through[0].lerp(mid.through[1], 0.5);
        let on = mid
            .points
            .windows(2)
            .map(|s| closest_on_segment(half, s[0], s[1]))
            .min_by(|a, b| a.distance(half).total_cmp(&b.distance(half)))
            .unwrap();
        assert!(m.open_distance(on) <= -24.0, "the middle of a lane");
        // Out from the lane, a metre further is about a metre further into the forest.
        let out = Vec2::new(0.0, 1.0);
        let (a, b) = (m.open_distance(on + out * 50.0), m.open_distance(on + out * 60.0));
        assert!(a > 10.0 && (b - a - 10.0).abs() < 2.0, "{a} then {b}");
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
        assert!(trees.len() > 3_000 && trees.len() < 60_000, "{} trees", trees.len());
        assert!(trees.iter().all(|t| !m.open(t.at)), "a tree on a lane or in a clearing");
        assert!(trees.iter().all(|t| (TREE_MIN..=TREE_MAX).contains(&t.height)));
        assert_eq!(trees, m.trees());
        // Trees stand right up to the edge of the lanes, so that a lane is a corridor.
        let near = trees
            .iter()
            .filter(|t| t.at.x.abs() < 300.0 && t.at.y.abs() < 80.0 && (6.0..12.0).contains(&m.open_distance(t.at)))
            .count();
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
    fn the_mountains_are_tall_at_the_edge_and_leave_the_towers_the_bases_and_the_lake_clear() {
        let m = map();
        for p in [Vec2::new(0.0, 600.0), Vec2::new(0.0, -600.0), Vec2::new(300.0, 500.0), Vec2::new(-300.0, -500.0)] {
            assert!(m.mountain_height(p) >= 250.0, "{p}: {}", m.mountain_height(p));
        }
        for p in m.towers.iter().chain(&m.bases).chain([&m.lake.at]) {
            assert_eq!(m.mountain_height(*p), 0.0, "{p}");
        }
        assert_eq!(m.mountain_height(Vec2::ZERO), 0.0);
        assert!(m.trees().iter().all(|t| m.mountain_height(t.at) <= 1.0), "a tree stands in the mountains");
    }

    #[test]
    fn the_mountains_are_higher_than_a_drone_flies_wherever_they_close_the_edge() {
        let m = map();
        let ceiling = Rules::default().ceiling;
        assert!(MOUNTAIN_HEIGHT * 0.75 > 2.0 * ceiling);
        assert!(!m.edge_open(ceiling), "a way round the edge");
        // The test can tell: with a ceiling above the mountains the way is open.
        assert!(m.edge_open(MOUNTAIN_HEIGHT + 1.0));
    }

    #[test]
    fn the_mountains_are_the_same_every_time_and_smooth() {
        let m = map();
        let p = Vec2::new(100.0, 540.0);
        assert_eq!(m.mountain_height(p), m.mountain_height(p));
        assert!((m.mountain_height(p) - m.mountain_height(p + Vec2::new(0.0, 1.0))).abs() < 15.0);
    }

    #[test]
    fn a_point_in_the_mountains_is_pulled_out_with_room_to_circle() {
        let m = map();
        let q = m.keep_clear(Vec2::new(0.0, 580.0));
        assert!(
            q.length() < 480.0 && m.mountain_height(q) <= 1.0 && m.mountain_height(q + q.normalize() * ROOM) <= 1.0
        );
        assert_eq!(m.keep_clear(m.towers[2]), m.towers[2]);
        assert_eq!(m.keep_clear(m.towers[1]), m.towers[1]);
    }

    #[test]
    fn the_nearest_open_ground_of_a_point_in_the_forest_is_open() {
        let m = map();
        let q = m.nearest_open(Vec2::new(-250.0, 90.0));
        assert!(m.open(q), "{q}");
        assert!(q.distance(Vec2::new(-250.0, 90.0)) < 70.0);
    }

    fn chain() -> Map {
        Map::new(&Rules { layout: LayoutKind::Chain, ..Rules::default() })
    }

    #[test]
    fn the_chain_is_the_valley_with_six_outposts_after_its_five_towers() {
        let (valley, chain) = (Map::new(&Rules::default()), chain());
        assert_eq!(chain.layout.towers.len(), 11);
        assert_eq!(chain.layout.towers[..5], valley.layout.towers[..]);
        assert!(chain.layout.towers[5..].iter().all(|t| t.kind == Kind::Outpost && t.holder.is_none()));
        let names: Vec<_> = chain.layout.towers[5..].iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Blue west outpost",
                "Blue mid outpost",
                "Blue east outpost",
                "Red west outpost",
                "Red mid outpost",
                "Red east outpost"
            ]
        );
    }

    #[test]
    fn an_outpost_stands_halfway_along_its_lane_and_on_open_ground() {
        let m = chain();
        let (blue_home, red_home) = (m.towers[0], m.towers[4]);
        for (k, far) in [(5, 1), (6, 2), (7, 3)] {
            let blue = m.towers[k];
            assert!(
                (blue.distance(blue_home) - blue.distance(m.towers[far])).abs() < 1.0,
                "{k}: halfway home to the tower"
            );
            assert!(m.open(blue), "{k}");
            let red = m.towers[k + 3];
            assert!((red.distance(red_home) - red.distance(m.towers[far])).abs() < 1.0, "{k}: halfway on Red's side");
            assert!(m.open(red) && m.in_clearing(red));
        }
    }

    #[test]
    fn each_lane_of_the_chain_runs_through_its_outposts() {
        let m = chain();
        let west = m.lanes.iter().find(|l| l.name == LaneName::West).unwrap();
        assert_eq!(west.through.len(), 5);
        assert_eq!(west.through[1], m.towers[5]);
        assert_eq!(west.through[3], m.towers[8]);
        assert!(west.points.contains(&m.towers[5]) && west.points.contains(&m.towers[8]));
    }

    fn lanes(m: &Map) -> impl Iterator<Item = &Lane> {
        m.lanes.iter().filter(|l| l.name != LaneName::Road)
    }

    /// Points every `step` m along the middle of a lane, with the way it runs there.
    fn along(lane: &Lane, step: f32) -> Vec<(Vec2, Vec2)> {
        let mut out = vec![];
        for s in lane.points.windows(2) {
            let n = (s[0].distance(s[1]) / step).ceil().max(1.0) as usize;
            let dir = (s[1] - s[0]).normalize_or_zero();
            out.extend((0..n).map(|k| (s[0].lerp(s[1], k as f32 / n as f32), dir)));
        }
        out
    }

    /// The farthest the middle of `points` gets from the straight line from `a` to `b` (m).
    fn bow(points: &[Vec2], a: Vec2, b: Vec2) -> f32 {
        points.iter().map(|&p| segment_distance(p, a, b)).fold(0.0, f32::max)
    }

    /// A lane is a path cut through a forest, not a ruled line: it winds a little as it goes
    /// from tower to tower. Never more than 18 m off the straight line, so that the line stays in
    /// the lane (a lane is at least 24 m to each side of its middle).
    #[test]
    fn the_lanes_wind_through_the_forest() {
        for m in [map(), chain()] {
            for lane in lanes(&m) {
                assert_eq!(lane.points.first(), lane.through.first());
                assert_eq!(lane.points.last(), lane.through.last());
                for w in lane.through.windows(2) {
                    let (i, j) = (
                        lane.points.iter().position(|p| *p == w[0]).expect("it passes each tower"),
                        lane.points.iter().position(|p| *p == w[1]).expect("it passes each tower"),
                    );
                    let between = &lane.points[i..=j];
                    let off = bow(between, w[0], w[1]);
                    assert!((8.0..=18.0).contains(&off), "{:?} bows {off:.1} m from {} to {}", lane.name, w[0], w[1]);
                }
            }
        }
    }

    /// Thus the straight line from one tower to the next is open ground all the way.
    #[test]
    fn the_straight_line_between_two_towers_stays_in_their_lane() {
        for m in [map(), chain()] {
            for lane in &m.lanes {
                for w in lane.through.windows(2) {
                    assert!(m.line_open(w[0], w[1]), "{:?} from {} to {}", lane.name, w[0], w[1]);
                }
            }
        }
    }

    /// A low drone still sees another 120 m along a lane, wherever it is in it.
    #[test]
    fn a_low_drone_sees_120_m_along_a_lane() {
        let m = map();
        for lane in lanes(&m) {
            let pts: Vec<Vec2> = along(lane, 10.0).into_iter().map(|(p, _)| p).collect();
            for k in 0..pts.len().saturating_sub(12) {
                assert!(m.line_open(pts[k], pts[k + 12]), "{:?}: from {} 120 m on", lane.name, pts[k]);
            }
        }
    }

    /// The edge of the forest along a lane is ragged, not ruled: the lane is a little wider or
    /// narrower from place to place, and never narrower than 24 m to each side.
    #[test]
    fn the_forest_edge_is_ragged() {
        let m = map();
        let mid = m.lanes.iter().find(|l| l.name == LaneName::Mid).unwrap();
        let mut widths = vec![];
        for (c, dir) in along(mid, 10.0) {
            let side = dir.perp();
            if m.towers.iter().any(|t| t.distance(c) < TOWER_CLEARING + 40.0) {
                continue;
            }
            for dir in [side, -side] {
                let w = (0..600).map(|k| k as f32 * 0.1).find(|&d| !m.open(c + dir * d)).unwrap_or(60.0);
                widths.push(w);
            }
        }
        let (lo, hi) = widths.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &w| (lo.min(w), hi.max(w)));
        assert!(widths.len() > 10);
        assert!(lo >= 24.0, "the lane narrows to {lo:.1} m");
        assert!(hi - lo >= 4.0, "the lane is {lo:.1} to {hi:.1} m to each side: ruled");
    }

    /// The lake is a lake, not a circle: its shore comes in and goes out, but it is wide enough
    /// everywhere to circle over.
    #[test]
    fn the_lake_shore_comes_in_and_goes_out() {
        let m = map();
        let r: Vec<f32> = (0..72).map(|k| m.lake.shore(Vec2::from_angle(k as f32 * 5f32.to_radians()))).collect();
        let (lo, hi) = r.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        assert!(hi / lo >= 1.25, "the shore is {lo:.0} to {hi:.0} m from the middle: a circle");
        assert!(lo >= 0.75 * LAKE_RADIUS, "the lake narrows to {lo:.0} m");
        for w in r.windows(2) {
            assert!((w[1] - w[0]).abs() < 8.0, "the shore jumps from {:.0} to {:.0} m", w[0], w[1]);
        }
        for p in m.lake.outline(36) {
            assert!(m.open(p - (p - m.lake.at) * 0.02) && m.in_clearing(m.lake.at));
        }
    }

    /// The foot of the mountains wanders in and out, not round a circle: where nothing on the
    /// field (a base, a tower, a lane, the lake) holds it back, it is nearer the centre in some
    /// places than in others, by 40 m or more.
    #[test]
    fn the_foot_of_the_mountains_wanders() {
        let m = map();
        let clear = |p: Vec2| {
            m.bases
                .iter()
                .map(|b| b.distance(p) - BASE_CLEARING)
                .chain(m.towers.iter().map(|t| t.distance(p) - TOWER_CLEARING))
                .chain([m.lake.distance(p), m.open_distance(p) + LANE_HALF_WIDTH])
                .fold(f32::MAX, f32::min)
        };
        let mut feet = vec![];
        for k in 0..360 {
            let dir = Vec2::from_angle((k as f32).to_radians());
            let foot = (0..700).map(|i| i as f32).find(|&r| m.mountain_height(dir * r) > 1.0).unwrap_or(700.0);
            // Only where the field is far behind the foot.
            if clear(dir * (foot - 10.0)) > 100.0 {
                feet.push(foot);
            }
        }
        assert!(feet.len() >= 40, "only {} free feet", feet.len());
        let (lo, hi) = feet.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        assert!(hi - lo >= 40.0, "the foot is {lo:.0} to {hi:.0} m from the centre: a circle");
    }

    /// The mountains never stand on a lane: a drone can fly low all along each one.
    #[test]
    fn the_mountains_keep_off_the_lanes() {
        for m in [map(), chain()] {
            for lane in &m.lanes {
                for p in &lane.points {
                    assert_eq!(m.mountain_height(*p), 0.0, "{:?} at {p}", lane.name);
                }
            }
        }
    }
}
