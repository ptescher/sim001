//! The TX15 stand-in (`--radio PORT`): the ground station's only source of telemetry.
//!
//! The station connects to PORT instead of to the autopilot. This relays between them
//! (`communication_radio::serve`), at the rate of the telemetry link. The sim adds what the
//! planes of a game would tell the pilot's plane: each computer pilot is a device that
//! transmits on its own radios, and the receivers on the pilot's plane hear what is in range
//! (`communication_air`). The station learns of them the way it would in the field. Nothing is
//! passed to it from the sim directly.
//!
//! Kestrel sends Remote ID and is on the MAVLink network, Merlin sends Remote ID only, and
//! Swift is on the MAVLink network only, so that the map shows a joined marker and each kind of
//! single one.

use crate::{SharedSim, SimSet};
use bevy::prelude::*;
use communication_air::{Air, Emitter, Listener, Technology};
use communication_radio::Relay;
use communication_radio::serve::{Handle, Shared};
use communication_telemetry::Beacon;
use communication_traffic::Position;
use std::sync::{Arc, Mutex};

/// The MAVLink system ID of the pilot's plane in SITL.
const OWN_SYSID: u8 = 1;

pub struct Config {
    /// Where the station connects.
    pub port: u16,
    /// Where the autopilot's MAVLink is.
    pub upstream: String,
    /// The pilot box, where the sim's origin is on the earth.
    pub home: Position,
    /// What the link carries (bytes a second).
    pub rate: f32,
}

#[derive(Resource)]
struct Radio {
    shared: Handle,
    home: Position,
    air: Air,
    emitters: Vec<Emitter>,
}

pub fn plugin(app: &mut App, cfg: Config) {
    let shared = Arc::new(Mutex::new(Shared::new(Relay::new(OWN_SYSID, cfg.rate))));
    match communication_radio::serve::spawn(&format!("127.0.0.1:{}", cfg.port), cfg.upstream.clone(), shared.clone()) {
        Ok(addr) => println!("radio: the station connects to {addr}; the autopilot is at {}", cfg.upstream),
        Err(e) => {
            eprintln!("--radio {}: {e}", cfg.port);
            std::process::exit(1);
        }
    }
    app.insert_resource(Radio { shared, home: cfg.home, air: Air::new(1), emitters: vec![] })
        .add_systems(Update, update.in_set(SimSet::Visual));
}

/// A point of the sim's world (x east, y up, -z north) as a point on the earth.
fn geo(home: &Position, p: Vec3) -> Position {
    home.moved(-p.z, p.x, p.y)
}

fn beacon(home: &Position, sysid: u8, name: &str, pos: Vec3, vel: Vec3) -> Beacon {
    let (north, east) = (-vel.z, vel.x);
    Beacon {
        sysid,
        position: geo(home, pos),
        velocity: [north, east, -vel.y],
        heading: east.atan2(north).to_degrees().rem_euclid(360.0),
        uas_id: format!("T1S-{}", name.to_uppercase()),
        mac: [0x60, 0x3d, 0x26, 0, 0, sysid],
    }
}

fn radios(i: usize) -> Vec<Technology> {
    match i % 3 {
        0 => vec![Technology::Bluetooth5, Technology::MavlinkNetwork],
        1 => vec![Technology::Bluetooth5],
        _ => vec![Technology::MavlinkNetwork],
    }
}

fn update(time: Res<Time>, sim: Res<SharedSim>, mut radio: ResMut<Radio>) {
    let dt = time.delta_secs_f64();
    if dt <= 0.0 {
        return;
    }
    let radio = &mut *radio;
    // Where everything is now: the planes of the game, and the pilot's plane that hears them.
    let (own, others) = {
        let c = sim.0.lock().unwrap();
        let others: Vec<(&'static str, Vec3, Vec3)> = c
            .game
            .as_ref()
            .map(|g| g.opponents.iter().map(|o| (o.name, o.state.pos, o.state.vel)).collect())
            .unwrap_or_default();
        (c.state.pos, others)
    };
    if radio.emitters.len() != others.len() {
        radio.emitters = others
            .iter()
            .enumerate()
            .map(|(i, (name, pos, vel))| Emitter::new(beacon(&radio.home, 2 + i as u8, name, *pos, *vel), radios(i)))
            .collect();
    }
    for (e, (name, pos, vel)) in radio.emitters.iter_mut().zip(&others) {
        e.beacon = beacon(&radio.home, e.beacon.sysid, name, *pos, *vel);
    }
    let listener = Listener {
        position: geo(&radio.home, own),
        hears: vec![Technology::MavlinkNetwork, Technology::Bluetooth4, Technology::Bluetooth5, Technology::WifiNan, Technology::Adsb],
    };
    let heard = radio.air.step(dt, &mut radio.emitters, &[listener]);
    if !heard.is_empty() {
        let mut s = radio.shared.lock().unwrap();
        let now = s.now();
        s.relay.pass_on(heard, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worlds_axes_become_north_east_and_up() {
        let home = Position::new(37.87, -122.27, 100.0);
        // x is east, -z is north, y is up.
        let b = beacon(&home, 2, "Kestrel", Vec3::new(30.0, 12.0, -40.0), Vec3::new(5.0, 1.0, -20.0));
        let (n, e) = home.offset_to(&b.position);
        assert!((n - 40.0).abs() < 0.1 && (e - 30.0).abs() < 0.1, "{n} {e}");
        assert!((b.position.alt - 112.0).abs() < 1e-3);
        assert_eq!(b.velocity, [20.0, 5.0, -1.0], "north 20, east 5, down -1");
        assert!((b.heading - 14.04).abs() < 0.1, "{}", b.heading);
        assert_eq!(b.uas_id, "T1S-KESTREL");
    }

    #[test]
    fn each_pilot_has_its_own_radios() {
        assert!(radios(0).contains(&Technology::Bluetooth5) && radios(0).contains(&Technology::MavlinkNetwork));
        assert_eq!(radios(1), vec![Technology::Bluetooth5]);
        assert_eq!(radios(2), vec![Technology::MavlinkNetwork]);
    }
}
