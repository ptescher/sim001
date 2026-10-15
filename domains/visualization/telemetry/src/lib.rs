//! A real plane's telemetry, as the 3D view needs it: where the plane is and how it sits.
//!
//! The view puts home at the pilot box. Positions are offsets from home in the engine's world
//! frame: +X east, +Y up, -Z north. A body's nose is -Z, its right wing +X and its top +Y.
//! MAVLink uses north-east-down for the world and forward-right-down for the body.

use communication_telemetry::Parser;
use glam::{EulerRot, Mat3, Quat, Vec3};
use mavlink::dialects::common::MavMessage;

/// Metres per degree of latitude on a sphere the size of the Earth.
const M_PER_DEG: f64 = 6_371_000.0 * std::f64::consts::PI / 180.0;

/// Where the plane took off: degrees and metres above mean sea level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Home {
    pub lat: f64,
    pub lon: f64,
    pub alt: f32,
}

impl Home {
    /// `LAT,LON[,ALT]`, as `--home` takes it.
    pub fn parse(s: &str) -> Option<Home> {
        let v: Vec<f64> = s.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
        (v.len() >= 2).then(|| Home { lat: v[0], lon: v[1], alt: v.get(2).copied().unwrap_or(0.0) as f32 })
    }

    /// A position (degrees, metres above mean sea level) as an offset from home in the world frame.
    pub fn offset(&self, lat: f64, lon: f64, alt: f32) -> Vec3 {
        let north = (lat - self.lat) * M_PER_DEG;
        let east = (lon - self.lon) * M_PER_DEG * self.lat.to_radians().cos();
        Vec3::new(east as f32, alt - self.alt, -north as f32)
    }
}

/// North-east-down to the world frame (and forward-right-down to the body frame: the same map).
fn ned_to_world() -> Quat {
    Quat::from_mat3(&Mat3::from_cols(Vec3::NEG_Z, Vec3::X, Vec3::NEG_Y))
}

/// A body's attitude from roll, pitch and yaw (radians, as MAVLink's `ATTITUDE` sends them).
pub fn attitude(roll: f32, pitch: f32, yaw: f32) -> Quat {
    let m = ned_to_world();
    m * Quat::from_euler(EulerRot::ZYX, yaw, pitch, roll) * m.inverse()
}

/// Where the view draws the plane, relative to home.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub offset: Vec3,
    pub rot: Quat,
}

/// Listens to one plane's telemetry.
#[derive(Default)]
pub struct Watcher {
    parser: Parser,
    /// Set with `--home`: wins over what the plane says.
    fixed_home: Option<Home>,
    home: Option<Home>,
    position: Option<(f64, f64, f32)>,
    rot: Option<Quat>,
}

impl Watcher {
    pub fn new(home: Option<Home>) -> Watcher {
        Watcher { fixed_home: home, home, ..Watcher::default() }
    }

    /// Bytes from the link, in any pieces.
    pub fn push(&mut self, bytes: &[u8]) {
        self.parser.push(bytes);
        while let Some((_, msg)) = self.parser.next_message() {
            self.message(&msg);
        }
    }

    pub fn message(&mut self, msg: &MavMessage) {
        match msg {
            MavMessage::HOME_POSITION(h) if self.fixed_home.is_none() => {
                self.home = Some(Home { lat: h.latitude as f64 * 1e-7, lon: h.longitude as f64 * 1e-7, alt: h.altitude as f32 * 1e-3 });
            }
            MavMessage::GLOBAL_POSITION_INT(p) => {
                let (lat, lon, alt) = (p.lat as f64 * 1e-7, p.lon as f64 * 1e-7, p.alt as f32 * 1e-3);
                if lat == 0.0 && lon == 0.0 {
                    return; // No fix yet.
                }
                // Until the plane says where home is, it is under the first fix.
                self.home.get_or_insert(Home { lat, lon, alt: alt - p.relative_alt as f32 * 1e-3 });
                self.position = Some((lat, lon, alt));
            }
            MavMessage::ATTITUDE(a) => self.rot = Some(attitude(a.roll, a.pitch, a.yaw)),
            _ => {}
        }
    }

    /// Home, once known.
    pub fn home(&self) -> Option<Home> {
        self.home
    }

    /// Where to draw the plane, once there is enough telemetry to know. When the telemetry stops,
    /// the plane stays where it was last heard.
    pub fn placement(&self) -> Option<Placement> {
        let (home, (lat, lon, alt)) = (self.home?, self.position?);
        Some(Placement { offset: home.offset(lat, lon, alt), rot: self.rot.unwrap_or(Quat::IDENTITY) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn level_and_north_is_the_identity() {
        assert!(attitude(0.0, 0.0, 0.0).angle_between(Quat::IDENTITY) < 1e-5);
    }

    #[test]
    fn a_right_roll_drops_the_right_wing() {
        let r = attitude(30f32.to_radians(), 0.0, 0.0);
        assert!((r * Vec3::X).y < -0.4, "{:?}", r * Vec3::X);
    }

    #[test]
    fn pitch_up_raises_the_nose() {
        let r = attitude(0.0, 10f32.to_radians(), 0.0);
        assert!((r * Vec3::NEG_Z).y > 0.15);
    }

    #[test]
    fn a_yaw_of_90_points_the_nose_east() {
        assert!(close(attitude(0.0, 0.0, 90f32.to_radians()) * Vec3::NEG_Z, Vec3::X));
    }

    #[test]
    fn offsets_are_east_up_and_minus_north() {
        let home = Home { lat: 45.0, lon: 7.0, alt: 500.0 };
        let o = home.offset(45.0 + 100.0 / M_PER_DEG, 7.0 + 50.0 / (M_PER_DEG * 45f64.to_radians().cos()), 530.0);
        assert!(close(o, Vec3::new(50.0, 30.0, -100.0)), "{o:?}");
    }

    #[test]
    fn a_home_on_the_command_line_wins() {
        let home = Home { lat: 1.0, lon: 2.0, alt: 3.0 };
        let mut w = Watcher::new(Some(home));
        w.message(&MavMessage::HOME_POSITION(mavlink::dialects::common::HOME_POSITION_DATA { latitude: 10, ..Default::default() }));
        assert_eq!(w.home(), Some(home));
    }

    #[test]
    fn without_home_the_first_fix_is_over_home() {
        let mut w = Watcher::new(None);
        w.message(&MavMessage::GLOBAL_POSITION_INT(mavlink::dialects::common::GLOBAL_POSITION_INT_DATA {
            lat: 450_000_000,
            lon: 70_000_000,
            alt: 520_000,
            relative_alt: 20_000,
            ..Default::default()
        }));
        let p = w.placement().unwrap();
        assert!(close(p.offset, Vec3::new(0.0, 20.0, 0.0)), "{:?}", p.offset);
    }

    #[test]
    fn home_parses_from_the_command_line() {
        assert_eq!(Home::parse("-35.36,149.16,584"), Some(Home { lat: -35.36, lon: 149.16, alt: 584.0 }));
        assert_eq!(Home::parse("1,2"), Some(Home { lat: 1.0, lon: 2.0, alt: 0.0 }));
        assert_eq!(Home::parse("x"), None);
    }
}
