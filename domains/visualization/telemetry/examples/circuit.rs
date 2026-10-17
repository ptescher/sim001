//! A pretend plane for trying `sim001_view` with no plane: it sends MAVLink to UDP port 14550 as
//! ArduPilot does, flying circles 80 m across, 40 m up and 100 m north of home.
//!
//!   cargo run --release -p visualization_telemetry --example circuit [-- HOST:PORT]

use communication_telemetry::frame;
use mavlink::dialects::common::*;
use std::f64::consts::TAU;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

const HOME: (f64, f64, f32) = (-35.363261, 149.165230, 584.0);
const M_PER_DEG: f64 = 6_371_000.0 * std::f64::consts::PI / 180.0;

fn main() {
    let to = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:14550".into());
    let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    let start = Instant::now();
    let mut seq = 0u8;
    let mut send = |msg: MavMessage| {
        seq = seq.wrapping_add(1);
        socket.send_to(&frame(1, 1, seq, &msg), &to).ok();
    };
    println!("flying circles; sending to {to}");
    loop {
        let t = start.elapsed().as_secs_f64();
        let ms = (t * 1000.0) as u32;
        // 12 m/s around a circle of 40 m radius, anticlockwise seen from above.
        let a = t * 12.0 / 40.0;
        let (north, east) = (100.0 + 40.0 * a.sin(), 40.0 * a.cos());
        // The track: d(north) = cos a, d(east) = -sin a, so the heading is -a.
        let yaw = (-a).rem_euclid(TAU) as f32;
        if (t * 10.0) as u64 % 10 == 0 {
            send(MavMessage::HOME_POSITION(HOME_POSITION_DATA {
                latitude: (HOME.0 * 1e7) as i32,
                longitude: (HOME.1 * 1e7) as i32,
                altitude: (HOME.2 * 1000.0) as i32,
                ..Default::default()
            }));
        }
        // An anticlockwise turn at 12 m/s on 40 m: about 20 degrees of left bank.
        send(MavMessage::ATTITUDE(ATTITUDE_DATA { time_boot_ms: ms, roll: -20f32.to_radians(), pitch: 3f32.to_radians(), yaw, ..Default::default() }));
        let lat = HOME.0 + north / M_PER_DEG;
        let lon = HOME.1 + east / (M_PER_DEG * HOME.0.to_radians().cos());
        send(MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
            time_boot_ms: ms,
            lat: (lat * 1e7) as i32,
            lon: (lon * 1e7) as i32,
            alt: ((HOME.2 + 40.0) * 1000.0) as i32,
            relative_alt: 40_000,
            ..Default::default()
        }));
        std::thread::sleep(Duration::from_millis(100));
    }
}
