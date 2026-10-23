//! ArduPilot SITL, with no renderer in sight: the JSON physics backend's packets, the lockstep
//! exchange, and starting ArduPlane instances. The sim's window, the headless training
//! environment and the tests all use this.
//!
//! SITL sends a binary servo packet to UDP 9002 + 10·N (instance N) every physics frame and
//! blocks until it gets a JSON sensor packet back, so the flight model runs in lockstep.

pub mod launch;
pub mod lockstep;
pub mod ports;
pub mod sensors;
pub mod service;

pub use lockstep::{Lockstep, Step, dt_for};
pub use ports::Ports;
pub use sensors::sensor_json;

const MAGIC_16: u16 = 18458;
const MAGIC_32: u16 = 29569;

/// What SITL sends every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    let channels = match u16_at(0) {
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

/// The port SITL instance `instance` sends its servo packets to, with instance 0 on `base`.
pub fn port_for(base: u16, instance: u16) -> u16 {
    base + 10 * instance
}

/// The MAVLink TCP port of instance `instance` (SITL's first serial port).
pub fn mavlink_port(instance: u16) -> u16 {
    5760 + 10 * instance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_autopilot_has_its_port() {
        assert_eq!(port_for(9002, 0), 9002);
        assert_eq!(port_for(9002, 1), 9012);
        assert_eq!(port_for(9002, 3), 9032);
        assert_eq!(mavlink_port(2), 5780);
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
        assert_eq!((p.frame_rate, p.frame_count, p.pwm[15]), (1200, 42, 1015));
    }

    #[test]
    fn short_or_foreign_packets_are_ignored() {
        assert!(parse_servo_packet(&[0; 4]).is_none());
        assert!(parse_servo_packet(&[0; 64]).is_none());
    }
}
