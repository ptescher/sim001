//! The ground: either the procedural Alpine Valley (flat mown field + runway in the middle, a
//! lake, rolling hills further out and mountains on the horizon) or a real place loaded from a
//! map pack (`--map cypress`, see `realsite`). World frame: +X east, +Y up, -Z north.

use crate::realsite::RealSite;
use glam::{Vec2, Vec3};
use simulation_physics::Obstacle;
use std::sync::OnceLock;

static SITE: OnceLock<Option<RealSite>> = OnceLock::new();
static ARENA: OnceLock<Arena> = OnceLock::new();

/// The valley of Towers: flat ground inside a circle, a lake, and a forest that is cut open by
/// lanes and clearings. The game owns the layout (`recreation_towers::map`); the world is told
/// about it once, in world coordinates (x east, z south), before anything asks about the ground.
pub struct Arena {
    /// The ground is flat within this distance of the origin (m).
    pub radius: f32,
    pub lake_centre: Vec2,
    pub lake_radius: f32,
    /// The forest: every tree on the field.
    pub trees: Vec<Tree>,
    /// How far a point is from the nearest open ground (a lane, a clearing, the lake): below 0 on
    /// it, above 0 in the forest (m).
    pub open_distance: Box<dyn Fn(f32, f32) -> f32 + Send + Sync>,
    /// How tall the mountains round the field are at a point (x, z; m): 0 on the field.
    pub mountains: Box<dyn Fn(f32, f32) -> f32 + Send + Sync>,
}

/// Fills in the valley of Towers in place of the Alpine Valley. Call once, before anything asks
/// about the ground.
pub fn set_arena(arena: Arena) -> Result<(), String> {
    if real().is_some() {
        return Err("a real place is loaded".to_string());
    }
    ARENA.set(arena).map_err(|_| "an arena is already set".to_string())
}

/// The valley of Towers, if that is what is flown.
pub fn arena() -> Option<&'static Arena> {
    ARENA.get()
}

/// Where the lake is (x, z), and how big (m): the Alpine Valley's, or the arena's.
pub fn lake_center() -> Vec2 {
    arena().map_or(LAKE_CENTER, |a| a.lake_centre)
}

pub fn lake_radius() -> f32 {
    arena().map_or(LAKE_RADIUS, |a| a.lake_radius)
}

/// Is this the Alpine Valley, with its runway, hangar and outcrops?
pub fn alpine() -> bool {
    real().is_none() && arena().is_none()
}

/// Loads a real place from `maps/<name>/build`. Call once, before anything asks about the
/// ground; without it the sim flies the Alpine Valley.
pub fn load_site(maps_dir: &std::path::Path, name: &str) -> Result<(), String> {
    let site = RealSite::load(&maps_dir.join(name).join("build"))?;
    SITE.set(Some(site)).map_err(|_| "a site is already loaded".to_string())
}

/// The real place being flown, if any.
pub fn real() -> Option<&'static RealSite> {
    SITE.get_or_init(|| None).as_ref()
}

/// Display name of the place.
pub fn site_title() -> String {
    if arena().is_some() {
        return "Towers Valley".to_string();
    }
    real().map_or_else(|| "Alpine Valley".to_string(), |r| r.meta.title.clone())
}

const ALPINE_PILOT_POS: Vec3 = Vec3::new(10.0, 0.0, 20.0);

/// The pilot box (ArduPilot's home). On a real place and on the Towers field it is the origin: the
/// field's centre is home, so that the game, its maps and the ground station share one frame.
pub fn pilot_pos() -> Vec3 {
    if real().is_some() || arena().is_some() { Vec3::ZERO } else { ALPINE_PILOT_POS }
}

/// Launch heading as a rotation about +Y (0 = facing north / -Z).
pub fn launch_heading() -> f32 {
    real().map_or(0.0, |r| -r.meta.launch_heading.to_radians())
}

/// Half-width of the square of terrain, around the world origin.
pub fn half_size() -> f32 {
    real().map_or(TERRAIN_HALF_SIZE, |r| r.half_size)
}

/// Height of open water (the lake, or the sea at a real place).
pub fn water_level() -> f32 {
    real().map_or(WATER_LEVEL, |r| r.water_level)
}

pub const RUNWAY_HALF_WIDTH: f32 = 6.0;
pub const RUNWAY_HALF_LENGTH: f32 = 70.0;
pub const RUNWAY_CENTER: Vec2 = Vec2::new(0.0, -30.0);

pub const LAKE_CENTER: Vec2 = Vec2::new(-190.0, -260.0);
pub const LAKE_RADIUS: f32 = 105.0;
pub const WATER_LEVEL: f32 = -0.6;

pub const TERRAIN_HALF_SIZE: f32 = 2600.0;

fn hash(ix: i32, iz: i32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(374_761_393) ^ (iz as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Hash in [0, 1] for a grid cell (shared with the texture baker).
pub fn hash_pub(ix: i32, iz: i32) -> f32 {
    hash(ix, iz)
}

fn value_noise(x: f32, z: f32) -> f32 {
    let (ix, iz) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let a = hash(ix, iz);
    let b = hash(ix + 1, iz);
    let c = hash(ix, iz + 1);
    let d = hash(ix + 1, iz + 1);
    let ab = a + (b - a) * ux;
    let cd = c + (d - c) * ux;
    ab + (cd - ab) * uz
}

pub fn fbm(x: f32, z: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for _ in 0..octaves {
        sum += value_noise(x * freq, z * freq) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

/// 2D gradient (Perlin-style) noise in roughly [-1, 1].
pub fn gradient_noise(x: f32, z: f32) -> f32 {
    let (ix, iz) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let grad = |cx: i32, cz: i32, dx: f32, dz: f32| {
        let a = hash(cx * 71 + 13, cz * 137 + 7) * std::f32::consts::TAU;
        a.cos() * dx + a.sin() * dz
    };
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(fx), fade(fz));
    let n00 = grad(ix, iz, fx, fz);
    let n10 = grad(ix + 1, iz, fx - 1.0, fz);
    let n01 = grad(ix, iz + 1, fx, fz - 1.0);
    let n11 = grad(ix + 1, iz + 1, fx - 1.0, fz - 1.0);
    let a = n00 + (n10 - n00) * u;
    let b = n01 + (n11 - n01) * u;
    (a + (b - a) * v) * 1.41
}

/// Ridged multifractal: sharp crests, smooth valleys (0..1).
fn ridged(x: f32, z: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm, mut weight) = (0.0, 0.5, 1.0, 0.0, 1.0f32);
    for i in 0..octaves {
        let o = i as f32 * 17.3;
        let n = 1.0 - gradient_noise(x * freq + o, z * freq - o).abs();
        let n = n * n * weight;
        weight = (n * 2.0).clamp(0.0, 1.0);
        sum += n * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.07;
    }
    sum / norm
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Terrain height (lake bed included; water surface is separate).
/// Distance from the lake centre in units of the (noise-shaped) shoreline radius:
/// < 1 is lake, 1 is the shore. Gives the lake an irregular natural outline.
pub fn lake_factor(x: f32, z: f32) -> f32 {
    if real().is_some() {
        return 9.0;
    }
    let d = Vec2::new(x, z) - lake_center();
    // The Towers lake is the game's circle: what the referee and the maps say is water is water.
    if arena().is_some() {
        return d.length() / lake_radius();
    }
    let a = d.y.atan2(d.x);
    let wobble = 1.0
        + 0.22 * gradient_noise(a.cos() * 1.6 + 4.0, a.sin() * 1.6 - 2.0)
        + 0.08 * gradient_noise(a.cos() * 4.0 - 1.0, a.sin() * 4.0 + 6.0);
    d.length() / (lake_radius() * wobble)
}

pub fn ground_height(x: f32, z: f32) -> f32 {
    if let Some(r) = real() {
        return r.height(x, z);
    }
    let r = (x * x + z * z).sqrt();
    let hills = (fbm(x / 380.0 + 11.0, z / 380.0 - 7.0, 5) - 0.35) * 45.0
        + gradient_noise(x / 140.0 + 3.0, z / 140.0 - 9.0) * 4.0;
    // Domain-warped ridged noise for the mountain ranges on the horizon.
    let wx = x + 350.0 * gradient_noise(x / 900.0 + 5.2, z / 900.0 - 1.3);
    let wz = z + 350.0 * gradient_noise(x / 900.0 - 7.7, z / 900.0 + 3.1);
    let mountains = smoothstep(1300.0, 2400.0, r) * ridged(wx / 1150.0, wz / 1150.0, 6).powf(1.4) * 520.0;
    // The Alpine Valley is flat to 200 m and the hills grow to 520 m; the Towers field is flat all
    // across (the game flies over a flat ground) and the hills start past its edge.
    let flat = match arena() {
        Some(a) => 1.0 - smoothstep(a.radius + 60.0, a.radius + 420.0, r),
        None => 1.0 - smoothstep(200.0, 520.0, r),
    };
    // Gentle bumps in the grass, none on the runway (the runway mesh sits at y = 0).
    let rw = (x - RUNWAY_CENTER.x).abs().max(0.0) - RUNWAY_HALF_WIDTH;
    let rl = (z - RUNWAY_CENTER.y).abs() - RUNWAY_HALF_LENGTH;
    let runway_mask = smoothstep(0.0, 4.0, rw.max(rl));
    let micro = if arena().is_some() { 0.0 } else { (fbm(x / 25.0, z / 25.0, 2) - 0.5) * 0.25 * runway_mask };
    let mut h = (hills.max(-4.0) + mountains) * (1.0 - flat) + micro;
    // The mountains of the valley of Towers, round the edge of the field.
    if let Some(a) = arena() {
        h += (a.mountains)(x, z);
    }

    let lf = lake_factor(x, z);
    let bed = WATER_LEVEL - 5.0 * (1.0 - lf.min(1.0)).sqrt() - 0.3;
    let lake_t = smoothstep(0.85, 1.35, lf);
    h = bed + (h - bed) * lake_t;
    h
}

pub fn ground_normal(x: f32, z: f32) -> Vec3 {
    let e = 0.5;
    let hx = ground_height(x + e, z) - ground_height(x - e, z);
    let hz = ground_height(x, z + e) - ground_height(x, z - e);
    Vec3::new(-hx, 2.0 * e, -hz).normalize()
}

pub fn is_water(x: f32, z: f32) -> bool {
    if let Some(r) = real() {
        return r.is_water(x, z);
    }
    ground_height(x, z) < WATER_LEVEL && lake_factor(x, z) < 1.6
}

pub fn is_runway(x: f32, z: f32) -> bool {
    if let Some(r) = real() {
        return r.on_runway(x, z);
    }
    if arena().is_some() {
        return false;
    }
    (x - RUNWAY_CENTER.x).abs() < RUNWAY_HALF_WIDTH && (z - RUNWAY_CENTER.y).abs() < RUNWAY_HALF_LENGTH
}

#[derive(Clone, Copy, Debug)]
pub struct Tree {
    pub pos: Vec3,
    pub height: f32,
    pub conifer: bool,
    pub shade: u8,
    /// A conifer that is dead and still standing.
    pub dead: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlantKind {
    /// A young conifer, a few metres tall.
    Sapling,
    /// A low thicket (willow, juniper).
    Shrub,
}

/// Something growing under, between and beside the trees.
#[derive(Clone, Copy, Debug)]
pub struct Plant {
    pub pos: Vec3,
    pub height: f32,
    pub kind: PlantKind,
    pub shade: u8,
}

/// Deterministic tree scatter. Keeps the field, runway approaches and lake clear.
/// Probability of a tree at a spot: patchy woods plus a ring around the field edge.
pub fn forest_density(x: f32, z: f32) -> f32 {
    if real().is_some() {
        return -1.0;
    }
    if let Some(a) = arena() {
        // Forest right up to the open ground, and a little past the field's edge.
        return if (a.open_distance)(x, z) > 0.0 { 1.0 } else { -1.0 };
    }
    let r = (x * x + z * z).sqrt();
    let density = fbm(x / 160.0 + 40.0, z / 160.0 - 20.0, 3);
    let ring = (-(r - 230.0).powi(2) / 900.0).exp() * 0.5;
    (density - 0.45) * 2.2 + ring
}

pub fn trees() -> Vec<Tree> {
    if let Some(r) = real() {
        return r.trees.clone();
    }
    if let Some(a) = arena() {
        // The forest goes on over the hills past the field: each tree stands on the ground.
        return a
            .trees
            .iter()
            .map(|t| Tree { pos: Vec3::new(t.pos.x, ground_height(t.pos.x, t.pos.z), t.pos.z), ..*t })
            .collect();
    }
    let mut out = Vec::new();
    let spacing = 13.0;
    let max_r = 1750.0;
    let n = (max_r / spacing) as i32;
    for ix in -n..=n {
        for iz in -n..=n {
            let jx = hash(ix * 7 + 1, iz * 13 + 5) - 0.5;
            let jz = hash(ix * 3 + 9, iz * 5 + 2) - 0.5;
            let x = (ix as f32 + jx * 0.9) * spacing;
            let z = (iz as f32 + jz * 0.9) * spacing;
            let r = (x * x + z * z).sqrt();
            if r > max_r || r < 150.0 {
                continue;
            }
            // Keep the approach/climb-out corridors along the runway open.
            if x.abs() < 40.0 && r < 400.0 {
                continue;
            }
            if lake_factor(x, z) < 1.2 {
                continue;
            }
            let h = ground_height(x, z);
            let slope = 1.0 - ground_normal(x, z).y;
            // Woods thicken on the hillsides; ragged treeline around 200-240 m.
            let treeline = 215.0 + 35.0 * gradient_noise(x / 180.0, z / 180.0);
            if h > treeline || slope > 0.5 {
                continue;
            }
            let hill_bonus = smoothstep(450.0, 900.0, r) * 0.35;
            let thin_near_line = smoothstep(treeline - 60.0, treeline, h) * 0.5;
            // Woods grow in stands of 30 m or so with openings between them, not evenly.
            let stand = (fbm(x / 34.0 + 11.0, z / 34.0 - 5.0, 2) - 0.5) * 0.9;
            if hash(ix * 31 + 7, iz * 17 + 3) > forest_density(x, z) + hill_bonus - thin_near_line + stand {
                continue;
            }
            let near_line = 1.0 - smoothstep(treeline - 80.0, treeline, h) * 0.45;
            // The trees of a stand are of an age: tall in an old stand, short in a young one.
            let age = fbm(x / 75.0 - 30.0, z / 75.0 + 12.0, 2);
            let height =
                (10.0 + 18.0 * smoothstep(0.25, 0.75, age)) * (0.82 + 0.36 * hash(ix + 77, iz - 33)) * near_line;
            // Subalpine fir and spruce, with a few aspen groves in the lower, gentler ground.
            let aspen_zone =
                ((1.0 - smoothstep(300.0, 700.0, r)) * 0.45 + fbm(x / 120.0, z / 120.0, 2) * 0.25 - 0.45).max(0.0);
            let conifer = hash(ix - 5, iz + 9) > aspen_zone;
            // A few stand dead, more where the trees are hardest pressed.
            let dead =
                conifer && hash(ix * 5 + 1, iz * 11 + 7) < 0.025 + 0.07 * smoothstep(treeline - 90.0, treeline, h);
            out.push(Tree {
                pos: Vec3::new(x, h, z),
                height,
                conifer,
                shade: (hash(ix + 2, iz + 2) * 4.0) as u8,
                dead,
            });
        }
    }
    out
}

/// Young conifers and shrubs: among the trees and along the edges of the woods, thickets by the
/// lake, and a few in the open. Deterministic, like the trees.
pub fn undergrowth() -> Vec<Plant> {
    if real().is_some() || arena().is_some() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let spacing = 6.0;
    let max_r = 800.0;
    let n = (max_r / spacing) as i32;
    for ix in -n..=n {
        for iz in -n..=n {
            let x = (ix as f32 + hash(ix * 5 + 3, iz * 9 + 1) - 0.5) * spacing;
            let z = (iz as f32 + hash(ix * 11 + 8, iz * 3 + 6) - 0.5) * spacing;
            let r = (x * x + z * z).sqrt();
            if r > max_r || r < 150.0 || (x.abs() < 40.0 && r < 400.0) {
                continue;
            }
            let h = ground_height(x, z);
            let slope = 1.0 - ground_normal(x, z).y;
            if h <= WATER_LEVEL + 0.2 || slope > 0.5 {
                continue;
            }
            let lake = lake_factor(x, z);
            let roll = hash(ix * 19 + 4, iz * 23 + 9);
            let treeline = 215.0 + 35.0 * gradient_noise(x / 180.0, z / 180.0);
            let wood = forest_density(x, z) + (fbm(x / 34.0 + 11.0, z / 34.0 - 5.0, 2) - 0.5) * 0.9;
            let shade = (hash(ix + 6, iz + 6) * 4.0) as u8;
            // Willow and juniper thickets round the lake, in clumps.
            if lake > 1.0 && lake < 1.9 && fbm(x / 22.0, z / 22.0, 2) > 0.45 && roll < 0.35 {
                out.push(Plant {
                    pos: Vec3::new(x, h, z),
                    height: 0.9 + 1.4 * hash(ix + 40, iz - 3),
                    kind: PlantKind::Shrub,
                    shade,
                });
                continue;
            }
            if lake < 1.2 || h > treeline {
                continue;
            }
            // Saplings thicken where the woods are, and thin out into the meadow.
            let sapling_odds = ((wood + 0.35) * 0.32).clamp(0.0, 0.3);
            if roll < sapling_odds {
                let young = hash(ix + 91, iz + 17);
                out.push(Plant {
                    pos: Vec3::new(x, h, z),
                    height: 1.5 + 3.8 * young * young,
                    kind: PlantKind::Sapling,
                    shade,
                });
            } else if wood > -0.2 && wood < 0.3 && roll > 0.93 {
                // Juniper and the like along the woods' edges.
                out.push(Plant {
                    pos: Vec3::new(x, h, z),
                    height: 0.7 + 1.6 * hash(ix + 5, iz + 55),
                    kind: PlantKind::Shrub,
                    shade,
                });
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub struct Building {
    pub center: Vec3,
    pub size: Vec3,
    /// Rotation about +Y.
    pub yaw: f32,
}

pub fn buildings() -> Vec<Building> {
    if let Some(r) = real() {
        return r.buildings.clone();
    }
    if arena().is_some() {
        return Vec::new();
    }
    let b = |x: f32, z: f32, sx: f32, sy: f32, sz: f32| {
        let h = ground_height(x, z);
        Building { center: Vec3::new(x, h + sy * 0.5, z), size: Vec3::new(sx, sy, sz), yaw: 0.0 }
    };
    vec![
        b(38.0, 40.0, 14.0, 6.0, 10.0), // club hangar
        b(26.0, 50.0, 5.0, 3.0, 4.0),   // shed
        b(-120.0, 160.0, 10.0, 7.0, 16.0),
        b(-108.0, 182.0, 8.0, 9.0, 8.0),
    ]
}

pub fn obstacles(trees: &[Tree], buildings: &[Building]) -> Vec<Obstacle> {
    let mut o: Vec<Obstacle> = trees
        .iter()
        .map(|t| Obstacle {
            pos: t.pos,
            radius: t.height * if t.conifer { 0.18 } else { 0.26 },
            top: t.pos.y + t.height * 0.95,
            bottom: t.pos.y - 1.0,
        })
        .collect();
    for b in buildings {
        o.push(Obstacle {
            pos: b.center,
            radius: 0.5 * b.size.x.max(b.size.z),
            top: b.center.y + b.size.y * 0.5,
            bottom: b.center.y - b.size.y,
        });
    }
    o
}

/// The world's ground, as the physics sees it.
pub struct WorldTerrain;

impl simulation_physics::Terrain for WorldTerrain {
    fn height(&self, x: f32, z: f32) -> f32 {
        ground_height(x, z)
    }
    fn normal(&self, x: f32, z: f32) -> Vec3 {
        ground_normal(x, z)
    }
    fn is_water(&self, x: f32, z: f32) -> bool {
        is_water(x, z)
    }
    fn is_runway(&self, x: f32, z: f32) -> bool {
        is_runway(x, z)
    }
}

/// Where a plane starts: on the ground at the pilot box, pointing down the runway.
pub fn start() -> simulation_physics::Start {
    let p = pilot_pos();
    simulation_physics::Start {
        pos: Vec3::new(p.x - 0.5, ground_height(p.x, p.z), p.z - 0.6),
        heading: launch_heading(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn grid(points: impl Iterator<Item = (f32, f32, usize)>, cell: f32) -> HashMap<(i32, i32), Vec<(f32, f32, usize)>> {
        let mut g: HashMap<(i32, i32), Vec<(f32, f32, usize)>> = HashMap::new();
        for p in points {
            g.entry(((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32)).or_default().push(p);
        }
        g
    }

    /// Trees near one another are alike in height, as the trees of one stand are the same age,
    /// where uniformly random heights would make every neighbour a stranger.
    #[test]
    fn woods_grow_in_stands() {
        let trees = trees();
        assert!(trees.len() > 5_000, "{} trees", trees.len());
        let g = grid(trees.iter().enumerate().map(|(i, t)| (t.pos.x, t.pos.z, i)), 25.0);
        let (mut near, mut n) = (0.0f64, 0u32);
        for (i, t) in trees.iter().enumerate().step_by(7) {
            let cell = ((t.pos.x / 25.0).floor() as i32, (t.pos.z / 25.0).floor() as i32);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    for &(x, z, j) in g.get(&(cell.0 + dx, cell.1 + dz)).into_iter().flatten() {
                        if j != i && (x - t.pos.x).hypot(z - t.pos.z) < 20.0 {
                            near += (trees[j].height - t.height).abs() as f64;
                            n += 1;
                        }
                    }
                }
            }
        }
        // Random pairs: the same trees against ones a long way off.
        let far: f64 = trees
            .iter()
            .step_by(7)
            .enumerate()
            .map(|(k, t)| (trees[(k * 7 + trees.len() / 2) % trees.len()].height - t.height).abs() as f64)
            .sum::<f64>()
            / trees.iter().step_by(7).count() as f64;
        let near = near / n.max(1) as f64;
        assert!(near < far * 0.8, "neighbours differ by {near:.2} m, strangers by {far:.2} m");
    }

    /// Here and there a conifer is dead and still standing, more of them where the trees are
    /// hardest pressed, near the treeline.
    #[test]
    fn a_few_conifers_are_dead() {
        let trees = trees();
        let conifers: Vec<_> = trees.iter().filter(|t| t.conifer).collect();
        let dead = conifers.iter().filter(|t| t.dead).count() as f32 / conifers.len() as f32;
        assert!(dead > 0.01 && dead < 0.06, "{:.1}% of the conifers are dead", dead * 100.0);
        assert!(trees.iter().all(|t| t.conifer || !t.dead), "an aspen is not a snag");
        let high: Vec<_> = conifers.iter().filter(|t| t.pos.y > 150.0).collect();
        let low: Vec<_> = conifers.iter().filter(|t| t.pos.y < 60.0).collect();
        let share = |v: &[&&Tree]| v.iter().filter(|t| t.dead).count() as f32 / v.len().max(1) as f32;
        assert!(
            share(&high) > share(&low),
            "no more dead trees near the treeline ({} against {})",
            share(&high),
            share(&low)
        );
    }

    /// Young trees and shrubs grow among the big ones and at the edges of the woods, not in the
    /// middle of the meadow, and never on the runway or in the lake.
    #[test]
    fn saplings_and_shrubs_grow_at_the_edges_of_woods() {
        let plants = undergrowth();
        assert!(plants.len() > 2_000 && plants.len() < 20_000, "{} plants", plants.len());
        for p in &plants {
            let (lo, hi) = match p.kind {
                PlantKind::Sapling => (1.2, 5.5),
                PlantKind::Shrub => (0.6, 2.5),
            };
            assert!(p.height >= lo && p.height <= hi, "{:?} is {} m tall", p.kind, p.height);
            assert!(p.pos.y > WATER_LEVEL, "a plant in the lake at {:?}", p.pos);
            let r = p.pos.x.hypot(p.pos.z);
            assert!(!(p.pos.x.abs() < 40.0 && r < 400.0), "a plant in the runway corridor at {:?}", p.pos);
            assert!(r >= 150.0, "a plant on the flying field at {:?}", p.pos);
        }
        assert!(
            plants.iter().any(|p| p.kind == PlantKind::Sapling) && plants.iter().any(|p| p.kind == PlantKind::Shrub)
        );
        let trees = trees();
        let g = grid(trees.iter().enumerate().map(|(i, t)| (t.pos.x, t.pos.z, i)), 30.0);
        let near_tree = |x: f32, z: f32| {
            let cell = ((x / 30.0).floor() as i32, (z / 30.0).floor() as i32);
            (-1..=1).any(|dx| {
                (-1..=1).any(|dz| {
                    g.get(&(cell.0 + dx, cell.1 + dz))
                        .into_iter()
                        .flatten()
                        .any(|&(tx, tz, _)| (tx - x).hypot(tz - z) < 25.0)
                })
            })
        };
        let saplings: Vec<_> = plants.iter().filter(|p| p.kind == PlantKind::Sapling).collect();
        let among = saplings.iter().filter(|p| near_tree(p.pos.x, p.pos.z)).count() as f32 / saplings.len() as f32;
        assert!(among > 0.7, "only {:.0}% of the saplings are near a tree", among * 100.0);
    }

    #[test]
    fn the_undergrowth_is_the_same_every_time() {
        let (a, b) = (undergrowth(), undergrowth());
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).take(200).all(|(p, q)| p.pos == q.pos && p.height == q.height));
    }
}
