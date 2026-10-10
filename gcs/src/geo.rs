//! Local tangent-plane geometry and the terrain map exported by `t1s_sim --export-map`.
//!
//! Everything on the map is in metres north/east of the *origin*: the `--home` location SITL
//! was started with, which is exactly the sim's pilot box (the sim reports NED relative to it).

use std::path::{Path, PathBuf};

const EARTH_R: f64 = 6_378_137.0;

#[derive(Clone, Copy, Debug)]
pub struct Origin {
    pub lat: f64,
    pub lon: f64,
    /// Ground altitude AMSL at the origin, metres.
    pub alt: f32,
}

impl Origin {
    /// Default matches `ardupilot/start.sh` (CMAC field, Canberra).
    pub const SITL_DEFAULT: Origin = Origin { lat: -35.363261, lon: 149.165230, alt: 584.0 };

    pub fn parse(s: &str) -> Option<Origin> {
        let v: Vec<f64> = s.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        match v.as_slice() {
            [lat, lon] => Some(Origin { lat: *lat, lon: *lon, alt: 0.0 }),
            [lat, lon, alt, ..] => Some(Origin { lat: *lat, lon: *lon, alt: *alt as f32 }),
            _ => None,
        }
    }

    /// (north, east) metres of a lat/lon from the origin.
    pub fn to_ne(&self, lat: f64, lon: f64) -> (f32, f32) {
        let n = (lat - self.lat).to_radians() * EARTH_R;
        let e = (lon - self.lon).to_radians() * EARTH_R * self.lat.to_radians().cos();
        (n as f32, e as f32)
    }

    pub fn to_ll(&self, north: f32, east: f32) -> (f64, f64) {
        let lat = self.lat + (north as f64 / EARTH_R).to_degrees();
        let lon = self.lon + (east as f64 / (EARTH_R * self.lat.to_radians().cos())).to_degrees();
        (lat, lon)
    }
}

/// Exported map: an orthophoto plus a coarse height grid, both square and north-up.
pub struct TerrainMap {
    pub png: PathBuf,
    pub size_m: f32,
    pub centre_n: f32,
    pub centre_e: f32,
    pub runway: Option<Runway>,
    heights: Vec<f32>,
    height_n: usize,
}

#[derive(Clone, Copy)]
pub struct Runway {
    pub north: f32,
    pub east: f32,
    pub length: f32,
    pub width: f32,
    pub heading_deg: f32,
}

impl TerrainMap {
    pub fn load(dir: &Path) -> Option<TerrainMap> {
        let meta = std::fs::read_to_string(dir.join("terrain_map.txt")).ok()?;
        let get = |k: &str| -> Option<f32> {
            meta.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(key, _)| key.trim() == k)
                .and_then(|(_, v)| v.trim().parse().ok())
        };
        let png = dir.join("terrain_map.png");
        if !png.exists() {
            return None;
        }
        let height_n = get("height_n")? as usize;
        let heights: Vec<f32> = std::fs::read(dir.join("terrain_height.bin"))
            .ok()
            .map(|b| b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
            .unwrap_or_default();
        let runway = (|| {
            Some(Runway {
                north: get("runway_north")?,
                east: get("runway_east")?,
                length: get("runway_length")?,
                width: get("runway_width")?,
                heading_deg: get("runway_heading").unwrap_or(0.0),
            })
        })();
        Some(TerrainMap {
            png: png.canonicalize().unwrap_or(png),
            size_m: get("size_m")?,
            centre_n: get("centre_north")?,
            centre_e: get("centre_east")?,
            runway,
            heights: if heights.len() == height_n * height_n { heights } else { vec![] },
            height_n,
        })
    }

    /// Ground height above the origin at (north, east), bilinear; None outside the grid.
    pub fn height(&self, north: f32, east: f32) -> Option<f32> {
        if self.heights.is_empty() {
            return None;
        }
        let n = self.height_n;
        let half = self.size_m * 0.5;
        // Grid row 0 is the north edge.
        let gx = (east - self.centre_e + half) / self.size_m * (n - 1) as f32;
        let gy = (self.centre_n + half - north) / self.size_m * (n - 1) as f32;
        if gx < 0.0 || gy < 0.0 || gx >= (n - 1) as f32 || gy >= (n - 1) as f32 {
            return None;
        }
        let (i, j) = (gx as usize, gy as usize);
        let (fx, fy) = (gx - i as f32, gy - j as f32);
        let h = |i: usize, j: usize| self.heights[j * n + i];
        let top = h(i, j) + (h(i + 1, j) - h(i, j)) * fx;
        let bot = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * fx;
        Some(top + (bot - top) * fy)
    }
}

/// Direction cosine matrix body (FRD) → NED for ZYX Euler angles (radians), row-major.
fn dcm(roll: f32, pitch: f32, yaw: f32) -> [[f32; 3]; 3] {
    let (sr, cr) = roll.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    [
        [cp * cy, sr * sp * cy - cr * sy, cr * sp * cy + sr * sy],
        [cp * sy, sr * sp * sy + cr * cy, cr * sp * sy - sr * cy],
        [-sp, sr * cp, cr * cp],
    ]
}

/// Unit line-of-sight in NED for a body-fixed gimbal: `pan` + right, `tilt` + up (degrees).
pub fn sensor_ray(roll: f32, pitch: f32, yaw: f32, pan_deg: f32, tilt_deg: f32) -> [f32; 3] {
    view_ray(roll, pitch, yaw, pan_deg, tilt_deg, 0.0, 0.0)
}

/// Ray through a point of the camera picture, given as tangent offsets from the centre
/// (`right` = u·tan(hfov/2), `down` = v·tan(vfov/2)), in NED.
pub fn view_ray(roll: f32, pitch: f32, yaw: f32, pan_deg: f32, tilt_deg: f32, right: f32, down: f32) -> [f32; 3] {
    let (sp, cp) = pan_deg.to_radians().sin_cos();
    let (st, ct) = tilt_deg.to_radians().sin_cos();
    // Camera frame (forward, right, down) → tilt about the ball's right axis → pan about body down.
    let (x, y, z) = (1.0, right, down);
    let (x, z) = (ct * x + st * z, -st * x + ct * z);
    let (x, y) = (cp * x - sp * y, sp * x + cp * y);
    let len = (x * x + y * y + z * z).sqrt();
    let b = [x / len, y / len, z / len];
    let m = dcm(roll, pitch, yaw);
    [0, 1, 2].map(|i| m[i][0] * b[0] + m[i][1] * b[1] + m[i][2] * b[2])
}

/// Gimbal (pan, tilt) in degrees that points along an NED direction.
pub fn look_angles(roll: f32, pitch: f32, yaw: f32, ned: [f32; 3]) -> (f32, f32) {
    let m = dcm(roll, pitch, yaw);
    // Body = Mᵀ · NED.
    let b = [0, 1, 2].map(|j| m[0][j] * ned[0] + m[1][j] * ned[1] + m[2][j] * ned[2]);
    let pan = b[1].atan2(b[0]).to_degrees();
    let tilt = (-b[2]).atan2((b[0] * b[0] + b[1] * b[1]).sqrt()).to_degrees();
    (pan, tilt)
}

/// Where a ray from (north, east, height above origin ground) hits the ground, marching over
/// the sim's height grid (flat ground at the origin's level without one). Returns
/// (north, east, ground height, slant range).
pub fn ground_hit(terrain: Option<&TerrainMap>, start: (f32, f32, f32), dir: [f32; 3]) -> Option<(f32, f32, f32, f32)> {
    let ground = |n: f32, e: f32| terrain.and_then(|t| t.height(n, e)).unwrap_or(0.0);
    let (n0, e0, h0) = start;
    let at = |r: f32| (n0 + dir[0] * r, e0 + dir[1] * r, h0 - dir[2] * r);
    let mut prev = 0.0f32;
    let mut r = 0.0f32;
    while r < 5000.0 {
        let step = (r * 0.01).clamp(1.0, 10.0);
        r += step;
        let (n, e, h) = at(r);
        if h <= ground(n, e) {
            // Bisect between the last point above ground and this one.
            let (mut lo, mut hi) = (prev, r);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                let (n, e, h) = at(mid);
                if h <= ground(n, e) { hi = mid } else { lo = mid }
            }
            let (n, e, _) = at(hi);
            return Some((n, e, ground(n, e), hi));
        }
        prev = r;
    }
    None
}

pub fn wrap360(deg: f32) -> f32 {
    deg.rem_euclid(360.0)
}

/// Signed smallest difference a - b in degrees, in (-180, 180].
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ne_roundtrip() {
        let o = Origin::SITL_DEFAULT;
        let (lat, lon) = o.to_ll(250.0, -120.0);
        let (n, e) = o.to_ne(lat, lon);
        assert!((n - 250.0).abs() < 0.01 && (e + 120.0).abs() < 0.01);
    }

    #[test]
    fn sensor_ray_and_look_angles_agree() {
        let (roll, pitch, yaw) = (0.3, -0.1, 1.2);
        for (pan, tilt) in [(0.0, 0.0), (40.0, -30.0), (-120.0, -70.0), (10.0, 15.0)] {
            let d = sensor_ray(roll, pitch, yaw, pan, tilt);
            let (p, t) = look_angles(roll, pitch, yaw, d);
            assert!((p - pan).abs() < 0.01 && (t - tilt).abs() < 0.01, "{pan},{tilt} -> {p},{t}");
        }
        // Level, heading east, looking straight down: ray points down.
        let d = sensor_ray(0.0, 0.0, std::f32::consts::FRAC_PI_2, 0.0, -90.0);
        assert!((d[2] - 1.0).abs() < 1e-5);
        // Heading east, boresight: ray points east.
        let d = sensor_ray(0.0, 0.0, std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        assert!((d[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn flat_ground_hit() {
        // 100 m up, looking 45° down to the north: hits 100 m north.
        let d = sensor_ray(0.0, 0.0, 0.0, 0.0, -45.0);
        let (n, e, _, r) = ground_hit(None, (0.0, 0.0, 100.0), d).unwrap();
        assert!((n - 100.0).abs() < 0.5 && e.abs() < 0.01 && (r - 141.4).abs() < 0.5);
    }

    #[test]
    fn angles() {
        assert_eq!(angle_diff(10.0, 350.0), 20.0);
        assert_eq!(angle_diff(350.0, 10.0), -20.0);
        assert_eq!(wrap360(-90.0), 270.0);
    }
}
