//! The TX15 as a relay between the plane and the station.
//!
//! Bytes from the plane (its MAVLink stream) go in. What the plane's receivers heard of other
//! aircraft goes in too. The bytes that come out are what the radio passes to the station, at
//! the rate of the link. Nothing here opens a socket: whoever owns the sockets feeds the relay
//! and sends what comes out.

use crate::downlink::{Downlink, Priority};
use crate::frame::{Framer, info, is_stream};
use communication_air::{Payload, RECEIVER_COMPONENT, Reception};
use communication_telemetry::{forward_remote_id, frame};

pub struct Relay {
    pub downlink: Downlink,
    framer: Framer,
    own_sysid: u8,
    seq: u8,
    unique: u64,
}

/// The key of a frame about something that streams: who sent it and what it is.
fn key(sysid: u8, msgid: u32) -> u64 {
    (sysid as u64) << 32 | msgid as u64
}

impl Relay {
    /// A relay for the plane with this system ID, over a link of `rate` bytes a second.
    pub fn new(own_sysid: u8, rate: f32) -> Relay {
        Relay { downlink: Downlink::new(rate), framer: Framer::default(), own_sysid, seq: 0, unique: 0 }
    }

    /// Bytes that came from the plane (its autopilot and the receivers on board).
    pub fn from_plane(&mut self, bytes: &[u8], now: f64) {
        for f in self.framer.push(bytes) {
            let Some(i) = info(&f) else { continue };
            if is_stream(i.msgid) {
                self.downlink.submit(Priority::Own, key(i.sysid, i.msgid), now, f);
            } else {
                self.unique += 1;
                self.downlink.submit(Priority::Reliable, self.unique, now, f);
            }
        }
    }

    /// What the receivers on the plane heard: MAVLink positions from the network go on as they
    /// are, and a Remote ID broadcast goes on as OPEN_DRONE_ID messages with the address of the sender.
    pub fn pass_on(&mut self, heard: Vec<Reception>, now: f64) {
        for r in heard {
            match r.payload {
                Payload::Mavlink(f) => {
                    let Some(i) = info(&f) else { continue };
                    self.downlink.submit(Priority::Traffic, key(i.sysid, i.msgid), now, f);
                }
                Payload::RemoteId { mac, bytes } => {
                    for m in forward_remote_id(mac, &bytes) {
                        self.seq = self.seq.wrapping_add(1);
                        let f = frame(self.own_sysid, RECEIVER_COMPONENT, self.seq, &m);
                        let msgid = info(&f).map_or(0, |i| i.msgid);
                        // One slot for each sender and each kind of message.
                        let k = key(0xFF, msgid) ^ (mac.iter().fold(0u64, |a, b| a << 5 ^ *b as u64) << 8);
                        self.downlink.submit(Priority::Traffic, k, now, f);
                    }
                }
            }
        }
    }

    /// Lets `dt` seconds pass at time `now`, and returns the bytes that the radio sends.
    pub fn step(&mut self, dt: f64, now: f64) -> Vec<u8> {
        self.downlink.step(dt, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use communication_air::Technology;
    use communication_telemetry::{Beacon, Decoder, ODID_COMPONENT, global_position, heartbeat, remote_id_location};
    use communication_traffic::{Position, Source};

    fn beacon(sysid: u8) -> Beacon {
        Beacon {
            sysid,
            position: Position::new(37.87, -122.27, 60.0),
            velocity: [0.0; 3],
            heading: 0.0,
            uas_id: "T1S".into(),
            mac: [0x60, 0, 0, 0, 0, sysid],
        }
    }

    #[test]
    fn the_planes_stream_goes_through_and_a_reply_is_not_lost() {
        let mut relay = Relay::new(1, 100_000.0);
        let b = beacon(1);
        // A reply nobody may lose: a message id that is not in the stream list (77, COMMAND_ACK).
        let mut raw = vec![0xFD, 3, 0, 0, 1, 1, 1, 77, 0, 0, 1, 2, 3, 0, 0];
        raw.extend_from_slice(&frame(1, 1, 0, &global_position(&b, 1)));
        relay.from_plane(&raw, 0.0);
        let out = relay.step(0.05, 0.05);
        assert_eq!(out, raw);
    }

    #[test]
    fn a_remote_id_broadcast_comes_out_as_a_forwarded_message_the_station_reads() {
        let mut relay = Relay::new(1, 100_000.0);
        let other = beacon(2);
        let heard = vec![Reception {
            listener: 0,
            emitter: 0,
            technology: Technology::Bluetooth5,
            time: 1.0,
            payload: Payload::RemoteId {
                mac: other.mac,
                bytes: communication_remoteid::encode_pack(&[remote_id_location(&other, 1.0)]),
            },
        }];
        relay.pass_on(heard, 1.0);
        let bytes = relay.step(0.05, 1.05);
        let seen = Decoder::new(1).push(&bytes, 1.05);
        assert_eq!(seen.traffic.len(), 1);
        assert_eq!(seen.traffic[0].identity.source, Source::RemoteId);
        let _ = ODID_COMPONENT;
    }

    #[test]
    fn a_peers_position_goes_on_as_it_is() {
        let mut relay = Relay::new(1, 100_000.0);
        let other = beacon(2);
        let f = frame(2, 1, 0, &global_position(&other, 1));
        let heard = vec![Reception {
            listener: 0,
            emitter: 0,
            technology: Technology::MavlinkNetwork,
            time: 1.0,
            payload: Payload::Mavlink(f.clone()),
        }];
        relay.pass_on(heard, 1.0);
        assert_eq!(relay.step(0.05, 1.05), f);
        let _ = heartbeat(&other);
    }
}
