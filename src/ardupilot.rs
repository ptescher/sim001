//! ArduPilot SITL bridge using the JSON external-physics backend (`sim_vehicle.py -f JSON`).
//!
//! SITL sends a binary servo packet to UDP 9002 every physics frame and blocks until we reply
//! with a JSON sensor packet, so the flight model runs in lockstep with the autopilot.
//!
//! In games, each computer pilot's plane has an autopilot of its own: SITL instance N (`-I N`)
//! sends to port 9002 + 10·N, and computer pilot N - 1 answers it (see `SimCore::step_opponent`).

use crate::sim::SimCore;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAGIC_16: u16 = 18458;
const MAGIC_32: u16 = 29569;

pub struct ServoPacket {
    pub frame_rate: u16,
    pub frame_count: u32,
    pub pwm: [u16; 16],
}

pub fn parse_servo_packet(buf: &[u8]) -> Option<ServoPacket> {
    if buf.len() < 8 {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([buf[i], buf[i + 1]]);
    let magic = u16_at(0);
    let channels = match magic {
        MAGIC_16 => 16,
        MAGIC_32 => 32,
        _ => return None,
    };
    if buf.len() < 8 + channels * 2 {
        return None;
    }
    let frame_rate = u16_at(2);
    let frame_count = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let mut pwm = [0u16; 16];
    for (i, p) in pwm.iter_mut().enumerate() {
        *p = u16_at(8 + i * 2);
    }
    Some(ServoPacket { frame_rate, frame_count, pwm })
}

/// Whose plane a bridge flies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Plane {
    Yours,
    /// Computer pilot `i`'s.
    Opponent(usize),
}

/// The port SITL instance `instance` sends its servo packets to, with yours on `base`.
pub fn port_for(base: u16, plane: Plane) -> u16 {
    match plane {
        Plane::Yours => base,
        Plane::Opponent(i) => base + 10 * (i as u16 + 1),
    }
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
            sock.set_read_timeout(Some(Duration::from_millis(200))).ok();
            println!("ArduPilot JSON bridge listening on UDP {port}");
            let mut buf = [0u8; 256];
            let mut last_frame: Option<u32> = None;
            let mut last_reply = String::new();
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        let Some(pkt) = parse_servo_packet(&buf[..n]) else {
                            continue;
                        };
                        let reply = {
                            let mut c = core.lock().unwrap();
                            c.ap.connected = true;
                            c.ap.last_packet = Some(Instant::now());
                            c.ap.frame_rate = pkt.frame_rate;
                            c.ap.frame_count = pkt.frame_count;
                            c.ap.packets += 1;
                            c.ap.pwm = pkt.pwm;
                            match last_frame {
                                // Duplicate frame (SITL re-sent after a timeout): repeat our answer.
                                Some(f) if f == pkt.frame_count && !last_reply.is_empty() => last_reply.clone(),
                                _ => {
                                    if matches!(last_frame, Some(f) if pkt.frame_count < f) {
                                        c.ap.status = "SITL restarted".into();
                                        println!("SITL restarted (frame counter went backwards)");
                                    } else {
                                        c.ap.status = format!("lockstep {} Hz", pkt.frame_rate);
                                    }
                                    let dt = if pkt.frame_rate > 0 {
                                        (1.0 / pkt.frame_rate as f32).clamp(1.0 / 4000.0, 1.0 / 50.0)
                                    } else {
                                        1.0 / 1200.0
                                    };
                                    let pwm = pkt.pwm;
                                    c.step_ardupilot(&pwm, dt);
                                    c.sitl_json()
                                }
                            }
                        };
                        last_frame = Some(pkt.frame_count);
                        if let Err(e) = sock.send_to(reply.as_bytes(), src) {
                            eprintln!("JSON reply to {src} failed: {e}");
                        }
                        last_reply = reply;
                    }
                    Err(_) => {
                        let mut c = core.lock().unwrap();
                        if let Some(t) = c.ap.last_packet {
                            if t.elapsed() > Duration::from_secs(1) {
                                c.ap.connected = false;
                                c.ap.status = "SITL not responding".into();
                            }
                        }
                    }
                }
            }
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
            sock.set_read_timeout(Some(Duration::from_millis(200))).ok();
            let mut buf = [0u8; 256];
            let (mut last_frame, mut last_reply): (Option<u32>, String) = (None, String::new());
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        let Some(pkt) = parse_servo_packet(&buf[..n]) else { continue };
                        let reply = match last_frame {
                            Some(f) if f == pkt.frame_count && !last_reply.is_empty() => last_reply.clone(),
                            _ => {
                                let dt = if pkt.frame_rate > 0 { (1.0 / pkt.frame_rate as f32).clamp(1.0 / 4000.0, 1.0 / 50.0) } else { 1.0 / 1200.0 };
                                core.lock().unwrap().step_opponent(i, &pkt.pwm, dt)
                            }
                        };
                        last_frame = Some(pkt.frame_count);
                        let _ = sock.send_to(reply.as_bytes(), src);
                        last_reply = reply;
                    }
                    Err(_) => {
                        let mut c = core.lock().unwrap();
                        if let Some(link) = c.others.get_mut(i) {
                            if link.last_packet.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                                link.connected = false;
                            }
                        }
                    }
                }
            }
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

    #[test]
    fn parses_16_channel_packet() {
        let mut b = vec![];
        b.extend_from_slice(&MAGIC_16.to_le_bytes());
        b.extend_from_slice(&1200u16.to_le_bytes());
        b.extend_from_slice(&42u32.to_le_bytes());
        for i in 0..16u16 {
            b.extend_from_slice(&(1000 + i).to_le_bytes());
        }
        let p = parse_servo_packet(&b).unwrap();
        assert_eq!(p.frame_rate, 1200);
        assert_eq!(p.frame_count, 42);
        assert_eq!(p.pwm[15], 1015);
    }
}
