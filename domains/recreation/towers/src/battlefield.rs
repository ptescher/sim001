//! The battlefield: the commander's camera over the valley, the picture of it that the 3D view
//! (`towers_view`) renders, and the link between them. The commander app steers the camera; the
//! 3D view renders the valley from it with no drones in it, so that the picture shows nothing that
//! the fog hides; the app draws its map on the picture with the same camera.
//!
//! The field's frame is the game's: x north, y east (m from the field's centre), and up.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// The commander's camera: it looks down at `look` on the ground, from `distance` away, towards
/// `heading`, tilted `pitch` below the horizon.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Eye {
    /// The point of the ground in the middle of the picture (north, east; m).
    pub look: Vec2,
    /// Where the camera looks (deg clockwise from north).
    pub heading: f32,
    /// How far below the horizon it looks (deg).
    pub pitch: f32,
    /// From the camera to `look` (m).
    pub distance: f32,
    /// The picture's vertical field of view (deg).
    pub vfov: f32,
}

impl Default for Eye {
    fn default() -> Eye {
        Eye { look: Vec2::ZERO, heading: 0.0, pitch: 55.0, distance: 700.0, vfov: 40.0 }
    }
}

impl Eye {
    /// How high the camera is above the ground at `look` (m).
    pub fn height(&self) -> f32 {
        self.distance * self.pitch.to_radians().sin()
    }

    /// The way the camera looks, its right and its up (unit vectors; north, east, up).
    pub fn axes(&self) -> (Vec3, Vec3, Vec3) {
        let (h, p) = (self.heading.to_radians(), self.pitch.to_radians());
        let forward = Vec3::new(h.cos() * p.cos(), h.sin() * p.cos(), -p.sin());
        let right = Vec3::new(-h.sin(), h.cos(), 0.0);
        (forward, right, forward.cross(right))
    }

    /// Where the camera is (north, east, up; m).
    pub fn position(&self) -> Vec3 {
        self.look.extend(0.0) - self.axes().0 * self.distance
    }

    /// The focal length of a picture `h` pixels high (pixels).
    fn focal(&self, h: f32) -> f32 {
        h * 0.5 / (self.vfov.to_radians() * 0.5).tan()
    }

    /// Where the point `p` (north, east, up) is in a picture `w` by `h` pixels from this camera
    /// (pixels from the top left), or `None` behind the camera.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<(f32, f32)> {
        let (forward, right, up) = self.axes();
        let v = p - self.position();
        let z = v.dot(forward);
        if z < 0.5 {
            return None;
        }
        let f = self.focal(h) / z;
        Some((w * 0.5 + v.dot(right) * f, h * 0.5 - v.dot(up) * f))
    }

    /// The point of the ground (height 0) at pixel (`x`, `y`) of a picture `w` by `h`, or `None`
    /// where the picture shows the sky.
    pub fn ground(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Vec2> {
        let (forward, right, up) = self.axes();
        let f = self.focal(h);
        let ray = forward + right * ((x - w * 0.5) / f) - up * ((y - h * 0.5) / f);
        let from = self.position();
        if ray.z > -1e-4 {
            return None;
        }
        let t = -from.z / ray.z;
        Some((from + ray * t).truncate())
    }
}

/// The battlefield of `team` is on this port: after the cameras of the drones, which start at
/// `first_feed_port`.
pub fn port(first_feed_port: u16, team: crate::Team) -> u16 {
    first_feed_port + 20 + team.index() as u16
}

/// The largest picture the 3D view renders (pixels).
pub const MAX_SIZE: (u16, u16) = (2560, 1600);

/// What the app asks of the 3D view: a picture `width` by `height` from `eye`. It sends one
/// whenever the camera or the size of the battlefield changes; the newest one counts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    pub eye: Eye,
    pub width: u16,
    pub height: u16,
}

const REQUEST_MAGIC: [u8; 4] = *b"BFLD";
const FRAME_MAGIC: [u8; 4] = *b"BFRM";

fn put_eye(b: &mut Vec<u8>, e: &Eye) {
    for x in [e.look.x, e.look.y, e.heading, e.pitch, e.distance, e.vfov] {
        b.extend_from_slice(&x.to_le_bytes());
    }
}

fn get_f32(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn get_eye(b: &[u8]) -> Eye {
    let f = |k: usize| get_f32(b, 4 * k);
    Eye { look: Vec2::new(f(0), f(1)), heading: f(2), pitch: f(3), distance: f(4), vfov: f(5) }
}

impl Request {
    pub const SIZE: usize = 32;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = Vec::with_capacity(Self::SIZE);
        b.extend_from_slice(&REQUEST_MAGIC);
        put_eye(&mut b, &self.eye);
        b.extend_from_slice(&self.width.to_le_bytes());
        b.extend_from_slice(&self.height.to_le_bytes());
        b.try_into().expect("a request is 32 bytes")
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Option<Request> {
        if b[..4] != REQUEST_MAGIC {
            return None;
        }
        Some(Request {
            eye: get_eye(&b[4..28]),
            width: u16::from_le_bytes([b[28], b[29]]),
            height: u16::from_le_bytes([b[30], b[31]]),
        })
    }
}

/// A picture of the battlefield: BGRA, `stride` bytes a row, rendered from `eye`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub stride: u32,
    pub frame: u32,
    pub eye: Eye,
}

const FRAME_HEADER: usize = 40;

pub fn write_frame(w: &mut impl Write, f: &Frame, pixels: &[u8]) -> io::Result<()> {
    let mut b = Vec::with_capacity(FRAME_HEADER);
    b.extend_from_slice(&FRAME_MAGIC);
    b.extend_from_slice(&f.width.to_le_bytes());
    b.extend_from_slice(&f.height.to_le_bytes());
    b.extend_from_slice(&f.stride.to_le_bytes());
    b.extend_from_slice(&f.frame.to_le_bytes());
    put_eye(&mut b, &f.eye);
    w.write_all(&b)?;
    w.write_all(&pixels[..(f.stride * f.height as u32) as usize])?;
    w.flush()
}

/// Reads the next picture: its header, and its pixels into `buf`.
pub fn read_frame(r: &mut impl Read, buf: &mut Vec<u8>) -> io::Result<Frame> {
    let mut h = [0u8; FRAME_HEADER];
    r.read_exact(&mut h)?;
    if h[..4] != FRAME_MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a picture of the battlefield"));
    }
    let f = Frame {
        width: u16::from_le_bytes([h[4], h[5]]),
        height: u16::from_le_bytes([h[6], h[7]]),
        stride: u32::from_le_bytes([h[8], h[9], h[10], h[11]]),
        frame: u32::from_le_bytes([h[12], h[13], h[14], h[15]]),
        eye: get_eye(&h[16..40]),
    };
    let len = f.stride as usize * f.height as usize;
    if len > MAX_SIZE.0 as usize * 4 * MAX_SIZE.1 as usize * 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "a picture too large"));
    }
    buf.resize(len, 0);
    r.read_exact(buf)?;
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const W: f32 = 1200.0;
    const H: f32 = 700.0;

    #[test]
    fn the_point_it_looks_at_is_in_the_middle_of_the_picture() {
        let eye = Eye { look: Vec2::new(-300.0, 120.0), heading: 30.0, ..Eye::default() };
        let (x, y) = eye.project(eye.look.extend(0.0), W, H).unwrap();
        assert!((x - W / 2.0).abs() < 0.01 && (y - H / 2.0).abs() < 0.01, "{x} {y}");
    }

    #[test]
    fn looking_north_east_is_right_and_further_north_is_higher_up() {
        let eye = Eye::default();
        let (cx, cy) = eye.project(Vec3::ZERO, W, H).unwrap();
        let (ex, ey) = eye.project(Vec3::new(0.0, 50.0, 0.0), W, H).unwrap();
        let (nx, ny) = eye.project(Vec3::new(50.0, 0.0, 0.0), W, H).unwrap();
        assert!(ex > cx + 10.0 && (ey - cy).abs() < 1.0);
        assert!(ny < cy - 10.0 && (nx - cx).abs() < 1.0);
        // Something above the ground is higher up the picture.
        let (_, uy) = eye.project(Vec3::new(0.0, 0.0, 50.0), W, H).unwrap();
        assert!(uy < cy - 10.0);
    }

    #[test]
    fn turned_to_the_east_north_is_left() {
        let eye = Eye { heading: 90.0, ..Eye::default() };
        let (nx, _) = eye.project(Vec3::new(50.0, 0.0, 0.0), W, H).unwrap();
        assert!(nx < W / 2.0 - 10.0);
    }

    #[test]
    fn a_point_behind_the_camera_is_not_in_the_picture() {
        let eye = Eye::default();
        assert!(eye.project(eye.position() - Vec3::new(10.0, 0.0, 0.0), W, H).is_none());
    }

    #[test]
    fn the_camera_is_behind_and_above_the_point() {
        let eye = Eye { look: Vec2::new(100.0, 0.0), ..Eye::default() };
        let p = eye.position();
        assert!(p.x < 100.0 && p.y.abs() < 0.01);
        assert!((p.z - eye.height()).abs() < 0.01);
        assert!(((p - eye.look.extend(0.0)).length() - eye.distance).abs() < 0.01);
    }

    #[test]
    fn a_point_of_the_picture_is_the_ground_where_it_projects() {
        let eye = Eye { look: Vec2::new(200.0, -80.0), heading: 135.0, ..Eye::default() };
        for at in [Vec2::new(200.0, -80.0), Vec2::new(150.0, 10.0), Vec2::new(320.0, -200.0)] {
            let (x, y) = eye.project(at.extend(0.0), W, H).unwrap();
            let back = eye.ground(x, y, W, H).unwrap();
            assert!(back.distance(at) < 0.05, "{at} came back as {back}");
        }
    }

    #[test]
    fn above_the_horizon_there_is_no_ground() {
        let eye = Eye { pitch: 20.0, ..Eye::default() };
        assert!(eye.ground(W / 2.0, 0.0, W, H).is_none());
    }
    #[test]
    fn a_request_reads_back_as_it_was_sent() {
        let r = Request {
            eye: Eye { look: Vec2::new(-1.5, 2.25), heading: 45.0, ..Eye::default() },
            width: 1280,
            height: 640,
        };
        assert_eq!(Request::decode(&r.encode()), Some(r));
        assert_eq!(Request::decode(&[0; Request::SIZE]), None);
    }

    #[test]
    fn a_frame_reads_back_with_the_eye_it_was_rendered_from() {
        let eye = Eye { look: Vec2::new(10.0, -20.0), heading: 270.0, pitch: 40.0, distance: 333.0, vfov: 35.0 };
        let header = Frame { width: 3, height: 2, stride: 256, frame: 7, eye };
        let pixels: Vec<u8> = (0..512).map(|i| i as u8).collect();
        let mut wire = Vec::new();
        write_frame(&mut wire, &header, &pixels).unwrap();
        let mut buf = Vec::new();
        let back = read_frame(&mut wire.as_slice(), &mut buf).unwrap();
        assert_eq!(back, header);
        assert_eq!(buf, pixels);
    }

    #[test]
    fn each_team_has_a_port_of_its_own_after_the_drones_cameras() {
        assert_eq!(port(5601, crate::Team::Blue), 5621);
        assert_eq!(port(5601, crate::Team::Red), 5622);
    }
}
