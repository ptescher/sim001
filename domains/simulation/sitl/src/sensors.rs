//! The JSON sensor packet SITL's JSON backend wants back every frame: a plane's state, the wind
//! it flies in, and the RC channels its radio sends.

use glam::{Mat3, Quat, Vec3};
use simulation_physics::FlightState;

/// `home` is the world position that SITL's origin (NED, 0 0 0) sits at: the pilot box, on the ground.
pub fn sensor_json(s: &FlightState, wind: Vec3, rc: &[u16; 8], home: Vec3) -> String {
    // World (+X east, +Y up, -Z north) -> NED.
    let ned = |v: Vec3| [-v.z, v.x, -v.y];
    let pos = ned(s.pos - home);
    let vel = ned(s.vel);
    let wind = ned(wind);
    // Body (+X right, +Y up, -Z fwd) -> FRD.
    let frd = |v: Vec3| [-v.z, v.x, -v.y];
    let gyro = frd(s.omega);
    let acc = frd(s.specific_force);
    // Attitude: body FRD axes expressed in NED.
    let fwd = ned(s.rot * Vec3::NEG_Z);
    let right = ned(s.rot * Vec3::X);
    let down = ned(s.rot * Vec3::NEG_Y);
    let m = Mat3::from_cols(fwd.into(), right.into(), down.into());
    let q = Quat::from_mat3(&m).normalize();
    let mut rc_json = String::new();
    for (i, v) in rc.iter().enumerate() {
        if i > 0 {
            rc_json.push(',');
        }
        rc_json.push_str(&format!("\"rc_{}\":{}", i + 1, v));
    }
    format!(
        "\n{{\"timestamp\":{:.6},\"imu\":{{\"gyro\":[{:.6},{:.6},{:.6}],\"accel_body\":[{:.5},{:.5},{:.5}]}},\
\"position\":[{:.4},{:.4},{:.4}],\"quaternion\":[{:.7},{:.7},{:.7},{:.7}],\"velocity\":[{:.5},{:.5},{:.5}],\
\"airspeed\":{:.4},\"velocity_wind\":[{:.4},{:.4},{:.4}],\"battery\":{{\"voltage\":{:.3},\"current\":{:.3}}},\
\"rc\":{{{}}}}}\n",
        s.time,
        gyro[0], gyro[1], gyro[2],
        acc[0], acc[1], acc[2],
        pos[0], pos[1], pos[2],
        q.w, q.x, q.y, q.z,
        vel[0], vel[1], vel[2],
        s.airspeed,
        wind[0], wind[1], wind[2],
        s.voltage, s.current,
        rc_json
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_physics::Start;

    #[test]
    fn a_plane_at_home_is_at_the_origin_and_carries_the_radio() {
        let p = (simulation_aircraft::find("t1s-ranger").unwrap().params)();
        let s = FlightState::new(&p, &Start { pos: Vec3::new(5.0, 2.0, 5.0), heading: 0.0 });
        let json = sensor_json(&s, Vec3::ZERO, &[1500, 1500, 1000, 1500, 1100, 1500, 1500, 1165], s.pos);
        assert!(json.contains("\"position\":[-0.0000,0.0000,-0.0000]"), "{json}");
        assert!(json.contains("\"rc_8\":1165") && json.contains("\"rc_5\":1100"), "{json}");
        assert!(json.starts_with('\n') && json.ends_with("}\n"));
    }
}
