//! ArduPilot SITL bridge using the JSON external-physics backend (`sim_vehicle.py -f JSON`).
//!
//! SITL sends a binary servo packet to UDP 9002 every physics frame and blocks until we reply
//! with a JSON sensor packet, so the flight model runs in lockstep with the autopilot. The
//! packets and the lockstep itself are `simulation_sitl`'s; this binds them to the sim's core.
//!
//! In games, each computer pilot's plane has an autopilot of its own: SITL instance N (`-I N`)
//! sends to port 9002 + 10·N, and computer pilot N - 1 answers it (see `SimCore::step_opponent`).

use crate::sim::SimCore;
use simulation_sitl::{Lockstep, Step};
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Whose plane a bridge flies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Plane {
    Yours,
    /// Computer pilot `i`'s.
    Opponent(usize),
}

/// The port SITL instance `instance` sends its servo packets to, with yours on `base`.
pub fn port_for(base: u16, plane: Plane) -> u16 {
    simulation_sitl::port_for(base, match plane {
        Plane::Yours => 0,
        Plane::Opponent(i) => i as u16 + 1,
    })
}

pub fn spawn_bridge(core: Arc<Mutex<SimCore>>, port: u16, plane: Plane) {
    if let Plane::Opponent(i) = plane {
        return spawn_opponent_bridge(core, port, i);
    }
    std::thread::Builder::new()
        .name("ardupilot-json".into())
        .spawn(move || {
            let sock = match UdpSocket::bind(("0.0.0.0", port)) {
                Ok(s) => s,
                Err(e) => {
                    core.lock().unwrap().ap.status = format!("cannot bind UDP {port}: {e}");
                    return;
                }
            };
            println!("ArduPilot JSON bridge listening on UDP {port}");
            let (step_core, idle_core) = (core.clone(), core);
            Lockstep::default().serve(
                sock,
                move |kind, pkt, dt| {
                    let mut c = step_core.lock().unwrap();
                    c.ap.connected = true;
                    c.ap.last_packet = Some(Instant::now());
                    c.ap.frame_rate = pkt.frame_rate;
                    c.ap.frame_count = pkt.frame_count;
                    c.ap.packets += 1;
                    c.ap.pwm = pkt.pwm;
                    if kind == Step::Restarted {
                        c.ap.status = "SITL restarted".into();
                        println!("SITL restarted (frame counter went backwards)");
                    } else {
                        c.ap.status = format!("lockstep {} Hz", pkt.frame_rate);
                    }
                    let pwm = pkt.pwm;
                    c.step_ardupilot(&pwm, dt);
                    c.sitl_json()
                },
                move || {
                    let mut c = idle_core.lock().unwrap();
                    if c.ap.last_packet.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                        c.ap.connected = false;
                        c.ap.status = "SITL not responding".into();
                    }
                },
            );
        })
        .expect("spawn ArduPilot bridge thread");
}

/// A computer pilot's autopilot: the same lockstep, with that pilot's plane and radio.
fn spawn_opponent_bridge(core: Arc<Mutex<SimCore>>, port: u16, i: usize) {
    std::thread::Builder::new()
        .name(format!("ardupilot-json-{}", i + 1))
        .spawn(move || {
            let Ok(sock) = UdpSocket::bind(("0.0.0.0", port)) else {
                eprintln!("computer pilot {}: cannot bind UDP {port}", i + 1);
                return;
            };
            let (step_core, idle_core) = (core.clone(), core);
            Lockstep::default().serve(
                sock,
                move |_, pkt, dt| step_core.lock().unwrap().step_opponent(i, &pkt.pwm, dt),
                move || {
                    let mut c = idle_core.lock().unwrap();
                    if let Some(link) = c.others.get_mut(i) {
                        if link.last_packet.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                            link.connected = false;
                        }
                    }
                },
            );
        })
        .expect("spawn ArduPilot bridge thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SITL `-I N` sends to 9002 + 10·N: yours is instance 0, the computer pilots 1 to 3.
    #[test]
    fn each_autopilot_has_its_port() {
        assert_eq!(port_for(9002, Plane::Yours), 9002);
        assert_eq!(port_for(9002, Plane::Opponent(0)), 9012);
        assert_eq!(port_for(9002, Plane::Opponent(2)), 9032);
    }
}
