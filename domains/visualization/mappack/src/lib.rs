//! Real-world map packs: `maps/<name>/map.txt` (hand-edited site config) and
//! `maps/<name>/build/` (made by `domains/visualization/mapbuild` from USGS lidar, NAIP imagery and OSM).
//!
//! Geometry in a pack is in metres north / east of *home* (the pilot box, which is also
//! ArduPilot's home). Heights are metres above mean sea level.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const EARTH_R: f64 = 6_378_137.0;

/// `key=value` lines, `#` comments.
fn parse_kv(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn get<T: std::str::FromStr>(kv: &HashMap<String, String>, key: &str, file: &Path) -> Result<T, String> {
    kv.get(key)
        .ok_or_else(|| format!("{}: missing {key}", file.display()))?
        .parse()
        .map_err(|_| format!("{}: bad value for {key}", file.display()))
}

/// The hand-edited site description.
#[derive(Clone, Debug)]
pub struct MapConfig {
    pub name: String,
    pub dir: PathBuf,
    pub title: String,
    pub center_lat: f64,
    pub center_lon: f64,
    pub size_m: f32,
    pub home_lat: f64,
    pub home_lon: f64,
    pub launch_heading: f32,
    pub tree_min_m: f32,
    pub tree_max_m: f32,
    pub conifer_fraction: f32,
}

impl MapConfig {
    /// `maps_dir/<name>/map.txt`.
    pub fn load(maps_dir: &Path, name: &str) -> Result<MapConfig, String> {
        let dir = maps_dir.join(name);
        let file = dir.join("map.txt");
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let kv = parse_kv(&text);
        Ok(MapConfig {
            name: name.to_string(),
            title: kv.get("title").cloned().unwrap_or_else(|| name.to_string()),
            center_lat: get(&kv, "center_lat", &file)?,
            center_lon: get(&kv, "center_lon", &file)?,
            size_m: get(&kv, "size_m", &file)?,
            home_lat: get(&kv, "home_lat", &file)?,
            home_lon: get(&kv, "home_lon", &file)?,
            launch_heading: get(&kv, "launch_heading", &file)?,
            tree_min_m: get(&kv, "tree_min_m", &file).unwrap_or(8.0),
            tree_max_m: get(&kv, "tree_max_m", &file).unwrap_or(20.0),
            conifer_fraction: get(&kv, "conifer_fraction", &file).unwrap_or(0.5),
            dir,
        })
    }

    pub fn build_dir(&self) -> PathBuf {
        self.dir.join("build")
    }

    pub fn geo(&self) -> Geo {
        Geo { lat0: self.home_lat, lon0: self.home_lon }
    }
}

/// Local tangent plane around home (equirectangular; fine over a few km).
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub lat0: f64,
    pub lon0: f64,
}

impl Geo {
    pub fn to_ne(&self, lat: f64, lon: f64) -> (f64, f64) {
        let n = (lat - self.lat0).to_radians() * EARTH_R;
        let e = (lon - self.lon0).to_radians() * EARTH_R * self.lat0.to_radians().cos();
        (n, e)
    }

    pub fn to_ll(&self, north: f64, east: f64) -> (f64, f64) {
        let lat = self.lat0 + (north / EARTH_R).to_degrees();
        let lon = self.lon0 + (east / (EARTH_R * self.lat0.to_radians().cos())).to_degrees();
        (lat, lon)
    }
}

/// A raster covering a square `size_m` wide centred at (`centre_n`, `centre_e`), north-up:
/// row 0 is the north edge, column 0 the west edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Square {
    pub centre_n: f32,
    pub centre_e: f32,
    pub size_m: f32,
    pub px: u32,
}

impl Square {
    pub fn m_per_px(&self) -> f32 {
        self.size_m / self.px as f32
    }

    /// Continuous pixel coordinates (x right, y down) of a point; pixel centres at +0.5.
    pub fn to_px(&self, north: f32, east: f32) -> (f32, f32) {
        let half = self.size_m * 0.5;
        let x = (east - (self.centre_e - half)) / self.m_per_px();
        let y = ((self.centre_n + half) - north) / self.m_per_px();
        (x, y)
    }

    /// (north, east) of a pixel centre.
    pub fn to_ne(&self, x: u32, y: u32) -> (f32, f32) {
        let half = self.size_m * 0.5;
        let m = self.m_per_px();
        (self.centre_n + half - (y as f32 + 0.5) * m, self.centre_e - half + (x as f32 + 0.5) * m)
    }

    pub fn contains(&self, north: f32, east: f32) -> bool {
        let half = self.size_m * 0.5;
        (north - self.centre_n).abs() < half && (east - self.centre_e).abs() < half
    }

    fn write(&self, out: &mut String, prefix: &str) {
        out.push_str(&format!(
            "{prefix}_centre_n={}\n{prefix}_centre_e={}\n{prefix}_size_m={}\n{prefix}_px={}\n",
            self.centre_n, self.centre_e, self.size_m, self.px
        ));
    }

    fn read(kv: &HashMap<String, String>, prefix: &str, file: &Path) -> Result<Square, String> {
        Ok(Square {
            centre_n: get(kv, &format!("{prefix}_centre_n"), file)?,
            centre_e: get(kv, &format!("{prefix}_centre_e"), file)?,
            size_m: get(kv, &format!("{prefix}_size_m"), file)?,
            px: get(kv, &format!("{prefix}_px"), file)?,
        })
    }
}

/// Land cover classes in `landcover.png` (one byte per pixel, on the `wide` square).
pub mod cover {
    pub const GROUND: u8 = 0;
    pub const TREE: u8 = 1;
    pub const WATER: u8 = 2;
    pub const PAVED: u8 = 3;
    pub const BUILDING: u8 = 4;
}

/// What `domains/visualization/mapbuild` wrote, in `build/pack.txt`.
#[derive(Clone, Debug)]
pub struct PackMeta {
    pub title: String,
    pub home_lat: f64,
    pub home_lon: f64,
    /// Ground elevation at home, metres AMSL.
    pub home_alt: f32,
    pub launch_heading: f32,
    /// `height.f32`: little-endian f32 metres AMSL.
    pub height: Square,
    /// `ortho_wide.jpg` and `landcover.png`.
    pub wide: Square,
    /// `ortho_core.jpg`: the higher-resolution photo around home.
    pub core: Square,
    pub credits: String,
}

impl PackMeta {
    pub const FILE: &'static str = "pack.txt";

    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        let mut s = format!(
            "# Written by domains/visualization/mapbuild. Positions are metres north/east of home.\ntitle={}\nhome_lat={}\nhome_lon={}\nhome_alt={}\nlaunch_heading={}\n",
            self.title, self.home_lat, self.home_lon, self.home_alt, self.launch_heading
        );
        self.height.write(&mut s, "height");
        self.wide.write(&mut s, "wide");
        self.core.write(&mut s, "core");
        s.push_str(&format!("credits={}\n", self.credits));
        std::fs::write(dir.join(Self::FILE), s)
    }

    pub fn read(dir: &Path) -> Result<PackMeta, String> {
        let file = dir.join(Self::FILE);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let kv = parse_kv(&text);
        Ok(PackMeta {
            title: kv.get("title").cloned().unwrap_or_default(),
            home_lat: get(&kv, "home_lat", &file)?,
            home_lon: get(&kv, "home_lon", &file)?,
            home_alt: get(&kv, "home_alt", &file)?,
            launch_heading: get(&kv, "launch_heading", &file)?,
            height: Square::read(&kv, "height", &file)?,
            wide: Square::read(&kv, "wide", &file)?,
            core: Square::read(&kv, "core", &file)?,
            credits: kv.get("credits").cloned().unwrap_or_default(),
        })
    }
}

/// A tree grown from the aerial photo's canopy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackTree {
    pub north: f32,
    pub east: f32,
    pub height: f32,
    pub conifer: bool,
    pub shade: u8,
}

pub fn write_trees(path: &Path, trees: &[PackTree]) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(trees.len() * 14);
    for t in trees {
        b.extend_from_slice(&t.north.to_le_bytes());
        b.extend_from_slice(&t.east.to_le_bytes());
        b.extend_from_slice(&t.height.to_le_bytes());
        b.push(t.conifer as u8);
        b.push(t.shade);
    }
    std::fs::write(path, b)
}

pub fn read_trees(path: &Path) -> std::io::Result<Vec<PackTree>> {
    let b = std::fs::read(path)?;
    let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    Ok(b.chunks_exact(14)
        .enumerate()
        .map(|(k, _)| {
            let i = k * 14;
            PackTree { north: f(i), east: f(i + 4), height: f(i + 8), conifer: b[i + 12] != 0, shade: b[i + 13] }
        })
        .collect())
}

/// An oriented box: a building footprint (from OSM) or the runway.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackBox {
    pub north: f32,
    pub east: f32,
    /// Along `heading`.
    pub length: f32,
    pub width: f32,
    pub height: f32,
    /// Degrees true of the long axis.
    pub heading: f32,
}

/// One box per line: `north east length width height heading`.
pub fn write_boxes(path: &Path, boxes: &[PackBox]) -> std::io::Result<()> {
    let s: String = boxes
        .iter()
        .map(|b| format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.1}\n", b.north, b.east, b.length, b.width, b.height, b.heading))
        .collect();
    std::fs::write(path, s)
}

pub fn read_boxes(path: &Path) -> Vec<PackBox> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            (v.len() == 6).then(|| PackBox { north: v[0], east: v[1], length: v[2], width: v[3], height: v[4], heading: v[5] })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_pixel_roundtrip() {
        let s = Square { centre_n: 100.0, centre_e: -50.0, size_m: 1000.0, px: 500 };
        let (n, e) = s.to_ne(10, 20);
        let (x, y) = s.to_px(n, e);
        assert!((x - 10.5).abs() < 1e-3 && (y - 20.5).abs() < 1e-3);
        assert!(s.contains(100.0, -50.0) && !s.contains(700.0, 0.0));
    }

    #[test]
    fn geo_roundtrip() {
        let g = Geo { lat0: 48.57, lon0: -122.69 };
        let (lat, lon) = g.to_ll(1234.0, -567.0);
        let (n, e) = g.to_ne(lat, lon);
        assert!((n - 1234.0).abs() < 1e-6 && (e + 567.0).abs() < 1e-6);
    }

    #[test]
    fn trees_and_boxes_roundtrip() {
        let dir = std::env::temp_dir().join(format!("t1s_pack_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let trees = vec![PackTree { north: 1.0, east: -2.0, height: 30.0, conifer: true, shade: 3 }];
        write_trees(&dir.join("t.bin"), &trees).unwrap();
        assert_eq!(read_trees(&dir.join("t.bin")).unwrap(), trees);
        let boxes = vec![PackBox { north: 5.0, east: 6.0, length: 600.0, width: 20.0, height: 0.0, heading: 345.0 }];
        write_boxes(&dir.join("b.txt"), &boxes).unwrap();
        assert_eq!(read_boxes(&dir.join("b.txt")), boxes);
        std::fs::remove_dir_all(dir).ok();
    }
}
