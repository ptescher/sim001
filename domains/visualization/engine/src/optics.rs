//! The plane's cameras as hardware: a camera in the nose, in a recess behind a bulkhead, and a
//! gimbal under the belly, a white ball with a dark ring where its halves meet and a window for the
//! lens, as in the manufacturer's pictures. The sensor feed's camera is at that lens, so what it
//! sees is the plane seen from its own eye, and it is a camera looking out of something.

use crate::mesh_util::{finish, loft_n};
use crate::pod;
use bevy::prelude::*;
use simulation_aircraft::t1s_ranger::geom::NOSE_Z;
use std::f32::consts::FRAC_PI_2;

pub const GIMBAL_RADIUS: f32 = 0.021;
/// The gimbal ball's centre, on the plane's centre line under the front of the pod.
pub const GIMBAL_CENTRE: Vec3 = Vec3::new(0.0, -0.047, -0.17);
/// The lens, forward and 25 degrees down from the ball's centre (a radius away: sin 25 and cos 25
/// of it). The sensor feed's camera is here.
pub const SENSOR_MOUNT: Vec3 = Vec3::new(0.0, -0.047 - 0.008875, -0.17 - 0.019032);

/// What a part is made of: the model gives each kind its material.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Black,
    White,
    Glass,
    Metal,
}

pub struct Part {
    pub kind: Kind,
    pub mesh: Mesh,
    pub at: Transform,
    /// Half the part's size in each direction, for checking that it fits.
    pub half: Vec3,
}

pub struct NoseCamera {
    /// Where the wall behind the recess is.
    pub bulkhead_z: f32,
    /// The wall itself, in the pod's coordinates.
    pub bulkhead: Mesh,
    pub parts: Vec<Part>,
}

/// The camera in the nose: a body against the bulkhead, a lens barrel, and its glass.
pub fn nose_camera() -> NoseCamera {
    let bulkhead_z = NOSE_Z + 0.032;
    let (_, rx, ry, cy) = pod::stations_at(bulkhead_z);
    let bulkhead = finish(loft_n(
        &[(bulkhead_z, rx * 0.97, ry * 0.97, cy), (bulkhead_z + 0.002, rx * 0.97, ry * 0.97, cy)],
        0.0,
        40,
        true,
        pod::EXPONENT,
    ));
    let along_z = Quat::from_rotation_x(FRAC_PI_2);
    let y = cy - 0.004;
    let body_z = bulkhead_z - 0.011;
    let barrel_z = bulkhead_z - 0.022 - 0.004;
    let parts = vec![
        Part {
            kind: Kind::Black,
            mesh: finish(Cuboid::new(0.030, 0.024, 0.022).mesh().build()),
            at: Transform::from_xyz(0.0, y, body_z),
            half: Vec3::new(0.015, 0.012, 0.011),
        },
        Part {
            kind: Kind::Metal,
            mesh: finish(Cylinder::new(0.0085, 0.008).mesh().resolution(24).build()),
            at: Transform::from_xyz(0.0, y, barrel_z).with_rotation(along_z),
            half: Vec3::new(0.0085, 0.0085, 0.004),
        },
        Part {
            kind: Kind::Glass,
            mesh: finish(Cylinder::new(0.0065, 0.0016).mesh().resolution(24).build()),
            at: Transform::from_xyz(0.0, y, barrel_z - 0.0044).with_rotation(along_z),
            half: Vec3::new(0.0065, 0.0065, 0.0008),
        },
    ];
    NoseCamera { bulkhead_z, bulkhead, parts }
}

/// The gimbal under the belly: the ball, the ring where its halves meet, the window and its
/// frame, and the plate that holds it to the pod.
pub fn gimbal() -> Vec<Part> {
    let look = (SENSOR_MOUNT - GIMBAL_CENTRE).normalize();
    let facing = Quat::from_rotation_arc(Vec3::Y, look);
    let r = GIMBAL_RADIUS;
    let ring_y = -0.004;
    vec![
        Part {
            kind: Kind::White,
            mesh: finish(Sphere::new(r).mesh().uv(28, 16)),
            at: Transform::from_translation(GIMBAL_CENTRE),
            half: Vec3::splat(r),
        },
        Part {
            kind: Kind::Black,
            mesh: finish(
                Torus::new(0.0010, (r * r - ring_y * ring_y).sqrt() + 0.0003)
                    .mesh()
                    .major_resolution(36)
                    .minor_resolution(6)
                    .build(),
            ),
            at: Transform::from_translation(GIMBAL_CENTRE + Vec3::Y * ring_y),
            half: Vec3::new(r, 0.001, r),
        },
        Part {
            kind: Kind::Black,
            mesh: finish(Cylinder::new(0.0105, 0.0012).mesh().resolution(28).build()),
            at: Transform::from_translation(GIMBAL_CENTRE + look * (r - 0.0002)).with_rotation(facing),
            half: Vec3::splat(0.0105),
        },
        Part {
            kind: Kind::Glass,
            mesh: finish(Cylinder::new(0.0085, 0.0012).mesh().resolution(28).build()),
            at: Transform::from_translation(GIMBAL_CENTRE + look * (r + 0.0003)).with_rotation(facing),
            half: Vec3::splat(0.0085),
        },
        // The plate on the belly the ball hangs from.
        Part {
            kind: Kind::Black,
            mesh: finish(Cuboid::new(0.036, 0.002, 0.042).mesh().build()),
            at: Transform::from_xyz(0.0, pod::surface_y(GIMBAL_CENTRE.z, false) - 0.0004, GIMBAL_CENTRE.z),
            half: Vec3::new(0.018, 0.001, 0.021),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pod;

    /// The gimbal hangs from the belly of the pod ahead of the wing, as in the manufacturer's
    /// renders: a ball standing out of the bottom of the pod by a couple of centimetres.
    #[test]
    fn the_gimbal_hangs_under_the_belly() {
        let c = GIMBAL_CENTRE;
        let belly = pod::surface_y(c.z, false);
        let low = c.y - GIMBAL_RADIUS;
        assert!(belly - low > 0.02 && belly - low < 0.04, "the ball stands {} m out of the belly", belly - low);
        assert!(c.y < belly, "the ball's centre is above the belly");
        assert!(c.z < -0.08 && c.z > -0.22, "the gimbal is at {}: ahead of the wing, behind the nose", c.z);
        assert!(c.x == 0.0);
    }

    /// The sensor looks out of the ball: its mount is on the ball's surface, at the lens, forward
    /// and down, so that whatever the camera sees, it is not seeing the inside of the ball.
    #[test]
    fn the_sensor_looks_out_of_the_lens() {
        let d = SENSOR_MOUNT - GIMBAL_CENTRE;
        assert!((d.length() - GIMBAL_RADIUS).abs() < 0.0015, "the mount is {} from the ball's centre", d.length());
        assert!(d.z < -0.012, "the lens is not at the front");
        assert!(d.y < -0.004 && d.y > -0.014, "the lens is not tipped down a little ({})", d.y);
    }

    /// The nose is a squared recess with the camera in it behind a bulkhead: the camera and its
    /// lens are all inside the pod.
    #[test]
    fn the_nose_camera_sits_in_the_recess() {
        let n = nose_camera();
        assert!(
            n.bulkhead_z > simulation_aircraft::t1s_ranger::geom::NOSE_Z + 0.02
                && n.bulkhead_z < simulation_aircraft::t1s_ranger::geom::NOSE_Z + 0.045,
            "bulkhead at {}",
            n.bulkhead_z
        );
        for p in &n.parts {
            let (_, rx, ry, cy) = pod::stations_at(p.at.translation.z);
            assert!(p.at.translation.x.abs() + p.half.x <= rx * 0.9, "{:?} sticks out of the side", p.kind);
            assert!(
                p.at.translation.y + p.half.y <= cy + ry && p.at.translation.y - p.half.y >= cy - ry,
                "{:?} sticks out of the pod",
                p.kind
            );
            assert!(p.at.translation.z - p.half.z >= n.bulkhead_z - 0.04, "{:?} is outside the pod's nose", p.kind);
        }
        assert!(n.parts.iter().any(|p| p.kind == Kind::Glass), "no lens");
        assert!(n.parts.iter().any(|p| p.kind == Kind::Black), "no camera body");
    }

    /// The gimbal is a white ball with a dark ring where its halves meet, a dark window and a
    /// neck to the belly: a few parts, a few hundred triangles.
    #[test]
    fn the_gimbal_has_its_parts() {
        let g = gimbal();
        for kind in [Kind::White, Kind::Black, Kind::Glass] {
            assert!(g.iter().any(|p| p.kind == kind), "no {kind:?} part");
        }
        let tris: usize = g.iter().map(|p| p.mesh.indices().map_or(0, |i| i.len() / 3)).sum();
        assert!(tris > 200 && tris < 2500, "{tris} triangles");
        // The lens window faces the way the mount looks.
        let window = g.iter().find(|p| p.kind == Kind::Glass).unwrap();
        let facing = window.at.rotation * Vec3::Y;
        let look = (SENSOR_MOUNT - GIMBAL_CENTRE).normalize();
        assert!(
            facing.dot(look) > 0.95 || (-facing).dot(look) > 0.95,
            "the window does not face the lens: {facing:?} against {look:?}"
        );
    }
}
