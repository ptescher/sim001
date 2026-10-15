//! MAVLink about other aircraft: what a device sends about itself, and what a station makes
//! of what it hears.
//!
//! A device with an autopilot sends a HEARTBEAT and its GLOBAL_POSITION_INT. A receiver on
//! board a plane (ADS-B, Remote ID) passes on what it hears: ADSB_VEHICLE for ADS-B, and the
//! OPEN_DRONE_ID messages for Remote ID, with `id_or_mac` set to the sender's address. The
//! same messages with `id_or_mac` all zero are the plane's own transmitter talking to its own
//! autopilot (see <https://mavlink.io/en/services/opendroneid.html>), and are not traffic.
//!
//! The [`Decoder`] turns all of this into [`Observation`]s for the tracker. It works on whole
//! messages (the ground station already has a MAVLink link) or on a byte stream (the tests,
//! and a radio that passes bytes).

use communication_remoteid as rid;
use communication_traffic::{Identity, Observation, Position, Source};
use mavlink::dialects::common::*;
use num_traits::FromPrimitive;
use mavlink::error::MessageReadError;
use mavlink::{MavHeader, MavlinkReader, MavlinkVersion};
use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read};

pub use mavlink::dialects::common::MavMessage;
pub use mavlink::MavHeader as Header;

/// The component ID that the sender of OpenDroneID messages uses (MAV_COMP_ID_ODID_TXRX_1).
pub const ODID_COMPONENT: u8 = 236;
/// The component ID of an autopilot.
pub const AUTOPILOT_COMPONENT: u8 = 1;

/// What a device knows about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Beacon {
    /// Its MAVLink system ID.
    pub sysid: u8,
    pub position: Position,
    /// North, east and down speed (m/s).
    pub velocity: [f32; 3],
    /// Track over the ground (degrees).
    pub heading: f32,
    /// Its Remote ID (serial number).
    pub uas_id: String,
    /// The address its Remote ID transmitter sends from.
    pub mac: [u8; 6],
}

/// A MAVLink v2 frame.
pub fn frame(sysid: u8, component: u8, sequence: u8, msg: &MavMessage) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    mavlink::write_v2_msg(&mut out, MavHeader { system_id: sysid, component_id: component, sequence }, msg)
        .expect("writing to a Vec cannot fail");
    out
}

pub fn heartbeat(_b: &Beacon) -> MavMessage {
    MavMessage::HEARTBEAT(HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: MavType::MAV_TYPE_FIXED_WING,
        autopilot: MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
        base_mode: MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED,
        system_status: MavState::MAV_STATE_ACTIVE,
        mavlink_version: 3,
    })
}

pub fn global_position(b: &Beacon, boot_ms: u32) -> MavMessage {
    MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
        time_boot_ms: boot_ms,
        lat: (b.position.lat * 1e7).round() as i32,
        lon: (b.position.lon * 1e7).round() as i32,
        alt: (b.position.alt * 1000.0).round() as i32,
        relative_alt: (b.position.alt * 1000.0).round() as i32,
        vx: (b.velocity[0] * 100.0).round() as i16,
        vy: (b.velocity[1] * 100.0).round() as i16,
        vz: (b.velocity[2] * 100.0).round() as i16,
        hdg: (b.heading.rem_euclid(360.0) * 100.0).round() as u16 % 36000,
    })
}

/// What an ADS-B receiver reports about a transponder.
pub fn adsb(b: &Beacon, icao: u32, callsign: &str, age_s: u8) -> MavMessage {
    let mut sign = [0u8; 9];
    for (o, c) in sign.iter_mut().zip(callsign.bytes().take(8)) {
        *o = c;
    }
    let ground = b.velocity[0].hypot(b.velocity[1]);
    MavMessage::ADSB_VEHICLE(ADSB_VEHICLE_DATA {
        ICAO_address: icao,
        lat: (b.position.lat * 1e7).round() as i32,
        lon: (b.position.lon * 1e7).round() as i32,
        altitude_type: AdsbAltitudeType::ADSB_ALTITUDE_TYPE_GEOMETRIC,
        altitude: (b.position.alt * 1000.0).round() as i32,
        heading: (b.heading.rem_euclid(360.0) * 100.0).round() as u16 % 36000,
        hor_velocity: (ground * 100.0).round() as u16,
        ver_velocity: (-b.velocity[2] * 100.0).round() as i16,
        callsign: sign.into(),
        emitter_type: AdsbEmitterType::ADSB_EMITTER_TYPE_SMALL,
        tslc: age_s,
        flags: AdsbFlags::ADSB_FLAGS_VALID_COORDS
            | AdsbFlags::ADSB_FLAGS_VALID_ALTITUDE
            | AdsbFlags::ADSB_FLAGS_VALID_HEADING
            | AdsbFlags::ADSB_FLAGS_VALID_VELOCITY
            | AdsbFlags::ADSB_FLAGS_VALID_CALLSIGN,
        squawk: 1200,
    })
}

/// The Remote ID location message of a beacon. `seconds_into_hour` is the clock it stamps.
pub fn remote_id_location(b: &Beacon, seconds_into_hour: f32) -> rid::Message {
    let ground = b.velocity[0].hypot(b.velocity[1]);
    rid::Message::Location(rid::Location {
        status: rid::Status::Airborne,
        direction: Some(b.heading.rem_euclid(360.0)),
        speed: Some(ground),
        vertical_speed: Some(-b.velocity[2]),
        lat: b.position.lat,
        lon: b.position.lon,
        pressure_alt: None,
        geodetic_alt: Some(b.position.alt),
        height: None,
        timestamp: Some(seconds_into_hour % 3600.0),
        ..rid::Location::default()
    })
}

/// The Remote ID messages that go out less often: who the aircraft is.
pub fn remote_id_static(b: &Beacon) -> Vec<rid::Message> {
    vec![rid::Message::BasicId(rid::BasicId {
        id_type: rid::IdType::SerialNumber,
        ua_type: rid::UaType::Aeroplane,
        uas_id: b.uas_id.clone(),
    })]
}

fn padded<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let n = bytes.len().min(N);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

/// What a Remote ID receiver on board sends the autopilot for one broadcast it heard from
/// `mac`: a message pack as it is, or the single message in its own MAVLink message.
pub fn forward_remote_id(mac: [u8; 6], broadcast: &[u8]) -> Vec<MavMessage> {
    let id_or_mac = padded::<20>(&mac);
    let Ok(messages) = rid::decode_broadcast(broadcast) else { return vec![] };
    let packed = broadcast.first().is_some_and(|h| h >> 4 == 0xF);
    if packed {
        let n = messages.len() as u8;
        return vec![MavMessage::OPEN_DRONE_ID_MESSAGE_PACK(OPEN_DRONE_ID_MESSAGE_PACK_DATA {
            target_system: 0,
            target_component: 0,
            id_or_mac,
            single_message_size: rid::MESSAGE_SIZE as u8,
            msg_pack_size: n,
            messages: padded::<225>(&broadcast[3..]),
        })];
    }
    messages
        .iter()
        .map(|m| match m {
            rid::Message::BasicId(x) => MavMessage::OPEN_DRONE_ID_BASIC_ID(OPEN_DRONE_ID_BASIC_ID_DATA {
                target_system: 0,
                target_component: 0,
                id_or_mac,
                id_type: MavOdidIdType::from_u8(match x.id_type {
                    rid::IdType::None => 0,
                    rid::IdType::SerialNumber => 1,
                    rid::IdType::CaaRegistration => 2,
                    rid::IdType::UtmAssigned => 3,
                    rid::IdType::SpecificSession => 4,
                    rid::IdType::Other(c) => c,
                } as u8)
                .unwrap_or_default(),
                ua_type: MavOdidUaType::from_u8(x.ua_type.code()).unwrap_or_default(),
                uas_id: padded::<20>(x.uas_id.as_bytes()),
            }),
            rid::Message::Location(x) => MavMessage::OPEN_DRONE_ID_LOCATION(OPEN_DRONE_ID_LOCATION_DATA {
                target_system: 0,
                target_component: 0,
                id_or_mac,
                status: MavOdidStatus::from_u8(match x.status {
                    rid::Status::Undeclared => 0,
                    rid::Status::Ground => 1,
                    rid::Status::Airborne => 2,
                    rid::Status::Emergency => 3,
                    rid::Status::RemoteIdFailure => 4,
                    rid::Status::Other(c) => c,
                } as u8)
                .unwrap_or_default(),
                direction: x.direction.map_or(36100, |d| (d * 100.0).round() as u16),
                speed_horizontal: x.speed.map_or(25500, |s| (s * 100.0).round() as u16),
                speed_vertical: x.vertical_speed.map_or(6300, |s| (s * 100.0).round() as i16),
                latitude: (x.lat * 1e7).round() as i32,
                longitude: (x.lon * 1e7).round() as i32,
                altitude_barometric: x.pressure_alt.unwrap_or(-1000.0),
                altitude_geodetic: x.geodetic_alt.unwrap_or(-1000.0),
                height_reference: Default::default(),
                height: x.height.unwrap_or(-1000.0),
                horizontal_accuracy: Default::default(),
                vertical_accuracy: Default::default(),
                barometer_accuracy: Default::default(),
                speed_accuracy: Default::default(),
                timestamp: x.timestamp.unwrap_or(f32::from(0xFFFFu16)),
                timestamp_accuracy: Default::default(),
            }),
            other => {
                let n = rid::encode(other);
                MavMessage::OPEN_DRONE_ID_MESSAGE_PACK(OPEN_DRONE_ID_MESSAGE_PACK_DATA {
                    target_system: 0,
                    target_component: 0,
                    id_or_mac,
                    single_message_size: rid::MESSAGE_SIZE as u8,
                    msg_pack_size: 1,
                    messages: padded::<225>(&n),
                })
            }
        })
        .collect()
}

/// A byte source that has nothing to read right now, rather than reaching the end.
#[derive(Default)]
struct Pipe(VecDeque<u8>);

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.is_empty() {
            return Err(ErrorKind::WouldBlock.into());
        }
        let n = buf.len().min(self.0.len());
        for (o, b) in buf.iter_mut().zip(self.0.drain(..n)) {
            *o = b;
        }
        Ok(n)
    }
}

/// Cuts a byte stream into MAVLink messages. Bytes can arrive in any pieces.
pub struct Parser {
    reader: MavlinkReader<Pipe>,
}

impl Default for Parser {
    fn default() -> Self {
        Parser { reader: MavlinkReader::new(Pipe::default()) }
    }
}

impl Parser {
    pub fn push(&mut self, bytes: &[u8]) {
        self.reader.get_mut().0.extend(bytes);
    }

    /// The next whole message, skipping bytes that are not one.
    pub fn next_message(&mut self) -> Option<(MavHeader, MavMessage)> {
        loop {
            match self.reader.read_message::<MavMessage>(MavlinkVersion::V2) {
                Ok(m) => return Some(m),
                Err(MessageReadError::Io(e)) if e.kind() == ErrorKind::WouldBlock => return None,
                Err(MessageReadError::Io(_)) => return None,
                Err(_) => continue, // a bad checksum or an unknown message: look for the next
            }
        }
    }
}

/// What the station knows about a Remote ID transmitter that it has heard.
#[derive(Default)]
struct Transmitter {
    uas_id: Option<String>,
}

/// What a decoder found in what it was given.
#[derive(Debug, Default)]
pub struct Heard {
    /// Reports about other aircraft.
    pub traffic: Vec<Observation>,
    /// Messages from the plane that the station flies.
    pub own: Vec<(MavHeader, MavMessage)>,
}

/// Turns MAVLink into reports for the tracker.
pub struct Decoder {
    own: u8,
    parser: Parser,
    transmitters: HashMap<String, Transmitter>,
}

fn is_zero(b: &[u8]) -> bool {
    b.iter().all(|&x| x == 0)
}

fn mac_key(id_or_mac: &[u8; 20]) -> String {
    id_or_mac[..6].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

fn cstring(b: &[u8]) -> String {
    let end = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

impl Decoder {
    /// A decoder for the station of the plane with system ID `own`.
    pub fn new(own: u8) -> Decoder {
        Decoder { own, parser: Parser::default(), transmitters: HashMap::new() }
    }

    pub fn own(&self) -> u8 {
        self.own
    }

    pub fn set_own(&mut self, sysid: u8) {
        self.own = sysid;
    }

    /// Bytes that came down the radio link at station time `now` (s).
    pub fn push(&mut self, bytes: &[u8], now: f64) -> Heard {
        self.parser.push(bytes);
        let mut heard = Heard::default();
        while let Some((header, msg)) = self.parser.next_message() {
            self.message_into(header, msg, now, &mut heard);
        }
        heard
    }

    /// One message, when the caller has already cut the stream into messages.
    pub fn message(&mut self, header: MavHeader, msg: MavMessage, now: f64) -> Heard {
        let mut heard = Heard::default();
        self.message_into(header, msg, now, &mut heard);
        heard
    }

    fn message_into(&mut self, header: MavHeader, msg: MavMessage, now: f64, heard: &mut Heard) {
        match &msg {
            MavMessage::GLOBAL_POSITION_INT(p) if header.system_id != self.own => {
                if p.lat == 0 && p.lon == 0 {
                    return;
                }
                heard.traffic.push(Observation {
                    identity: Identity::new(Source::Mavlink, header.system_id.to_string()),
                    time: now,
                    position: Position::new(p.lat as f64 * 1e-7, p.lon as f64 * 1e-7, p.alt as f32 * 1e-3),
                    velocity: Some([p.vx as f32 * 0.01, p.vy as f32 * 0.01, p.vz as f32 * 0.01]),
                    heading: (p.hdg != u16::MAX).then(|| p.hdg as f32 * 0.01),
                    name: None,
                });
            }
            MavMessage::ADSB_VEHICLE(a) => {
                if !a.flags.contains(AdsbFlags::ADSB_FLAGS_VALID_COORDS) {
                    return;
                }
                let heading = a.flags.contains(AdsbFlags::ADSB_FLAGS_VALID_HEADING).then(|| a.heading as f32 * 0.01);
                let velocity = a.flags.contains(AdsbFlags::ADSB_FLAGS_VALID_VELOCITY).then(|| {
                    let speed = a.hor_velocity as f32 * 0.01;
                    let course = heading.unwrap_or(0.0).to_radians();
                    [speed * course.cos(), speed * course.sin(), -(a.ver_velocity as f32) * 0.01]
                });
                heard.traffic.push(Observation {
                    identity: Identity::new(Source::Adsb, format!("{:06x}", a.ICAO_address)),
                    time: now - a.tslc as f64,
                    position: Position::new(
                        a.lat as f64 * 1e-7,
                        a.lon as f64 * 1e-7,
                        if a.flags.contains(AdsbFlags::ADSB_FLAGS_VALID_ALTITUDE) { a.altitude as f32 * 1e-3 } else { f32::NAN },
                    ),
                    velocity,
                    heading,
                    name: a.flags.contains(AdsbFlags::ADSB_FLAGS_VALID_CALLSIGN).then(|| cstring(&a.callsign[..])).filter(|s| !s.is_empty()),
                });
            }
            MavMessage::OPEN_DRONE_ID_BASIC_ID(m) if !is_zero(&m.id_or_mac) => {
                let key = mac_key(&m.id_or_mac);
                self.transmitters.entry(key).or_default().uas_id = Some(cstring(&m.uas_id)).filter(|s| !s.is_empty());
            }
            MavMessage::OPEN_DRONE_ID_LOCATION(m) if !is_zero(&m.id_or_mac) => {
                if m.latitude == 0 && m.longitude == 0 {
                    return;
                }
                let key = mac_key(&m.id_or_mac);
                let speed = (m.speed_horizontal != 25500).then(|| m.speed_horizontal as f32 * 0.01);
                let course = (m.direction != 36100).then(|| m.direction as f32 * 0.01);
                let up = (m.speed_vertical != 6300).then(|| m.speed_vertical as f32 * 0.01);
                let alt = [m.altitude_geodetic, m.altitude_barometric].into_iter().find(|a| *a > -999.0).unwrap_or(f32::NAN);
                self.report_remote_id(
                    key,
                    now,
                    Position::new(m.latitude as f64 * 1e-7, m.longitude as f64 * 1e-7, alt),
                    speed,
                    course,
                    up,
                    heard,
                );
            }
            MavMessage::OPEN_DRONE_ID_MESSAGE_PACK(m) if !is_zero(&m.id_or_mac) => {
                let key = mac_key(&m.id_or_mac);
                let n = (m.msg_pack_size as usize).min(rid::PACK_MAX);
                let mut pack = vec![0xF0 | rid::VERSION, m.single_message_size, n as u8];
                pack.extend_from_slice(&m.messages[..n * rid::MESSAGE_SIZE]);
                for message in rid::decode_pack(&pack).unwrap_or_default() {
                    match message {
                        rid::Message::BasicId(b) => {
                            self.transmitters.entry(key.clone()).or_default().uas_id = Some(b.uas_id).filter(|s| !s.is_empty());
                        }
                        rid::Message::Location(l) if l.lat != 0.0 || l.lon != 0.0 => {
                            let alt = l.geodetic_alt.or(l.pressure_alt).unwrap_or(f32::NAN);
                            self.report_remote_id(
                                key.clone(),
                                now,
                                Position::new(l.lat, l.lon, alt),
                                l.speed,
                                l.direction,
                                l.vertical_speed,
                                heard,
                            );
                        }
                        _ => {}
                    }
                }
            }
            _ if header.system_id == self.own => heard.own.push((header, msg)),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn report_remote_id(
        &mut self,
        key: String,
        now: f64,
        position: Position,
        speed: Option<f32>,
        course: Option<f32>,
        up: Option<f32>,
        heard: &mut Heard,
    ) {
        let velocity = match (speed, course) {
            (Some(s), Some(c)) => {
                let c = c.to_radians();
                Some([s * c.cos(), s * c.sin(), -up.unwrap_or(0.0)])
            }
            _ => None,
        };
        let name = self.transmitters.get(&key).and_then(|t| t.uas_id.clone());
        heard.traffic.push(Observation {
            identity: Identity::new(Source::RemoteId, key),
            time: now,
            position,
            velocity,
            heading: course,
            name,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0x60, 0x3d, 0x26, 0x01, 0x02, 0x03];

    fn beacon(sysid: u8) -> Beacon {
        Beacon {
            sysid,
            position: Position::new(37.8701, -122.2723, 82.5),
            velocity: [12.0, -5.0, -1.0],
            heading: 337.0,
            uas_id: format!("T1S{sysid:04}"),
            mac: MAC,
        }
    }

    fn bytes(b: &Beacon, msg: &MavMessage) -> Vec<u8> {
        frame(b.sysid, AUTOPILOT_COMPONENT, 0, msg)
    }

    #[test]
    fn a_stream_in_pieces_gives_whole_messages() {
        let b = beacon(3);
        let mut stream = bytes(&b, &heartbeat(&b));
        stream.extend(bytes(&b, &global_position(&b, 1000)));
        let mut parser = Parser::default();
        let mut got = vec![];
        for piece in stream.chunks(7) {
            parser.push(piece);
            while let Some((h, m)) = parser.next_message() {
                got.push((h.system_id, matches!(m, MavMessage::HEARTBEAT(_))));
            }
        }
        assert_eq!(got, vec![(3, true), (3, false)]);
    }

    #[test]
    fn noise_between_messages_is_skipped() {
        let b = beacon(3);
        let mut stream = vec![0x00, 0xFD, 0x03, 0x99, 0x12];
        stream.extend(bytes(&b, &global_position(&b, 5)));
        let mut parser = Parser::default();
        parser.push(&stream);
        assert!(matches!(parser.next_message(), Some((_, MavMessage::GLOBAL_POSITION_INT(_)))));
    }

    #[test]
    fn a_peers_position_is_traffic_and_the_planes_own_is_not() {
        let mut d = Decoder::new(1);
        let peer = beacon(3);
        let mut stream = bytes(&peer, &global_position(&peer, 1000));
        let own = beacon(1);
        stream.extend(bytes(&own, &global_position(&own, 1000)));
        let heard = d.push(&stream, 20.0);
        assert_eq!(heard.traffic.len(), 1);
        assert_eq!(heard.own.len(), 1);
        let o = &heard.traffic[0];
        assert_eq!(o.identity, Identity::new(Source::Mavlink, "3"));
        assert_eq!(o.time, 20.0);
        assert!(o.position.distance_to(&peer.position) < 0.1 && (o.position.alt - 82.5).abs() < 0.01);
        let v = o.velocity.unwrap();
        assert!((v[0] - 12.0).abs() < 0.01 && (v[1] + 5.0).abs() < 0.01 && (v[2] + 1.0).abs() < 0.01);
        assert!((o.heading.unwrap() - 337.0).abs() < 0.01);
    }

    #[test]
    fn a_position_with_no_fix_is_not_traffic() {
        let mut d = Decoder::new(1);
        let mut b = beacon(3);
        b.position = Position::new(0.0, 0.0, 0.0);
        assert!(d.push(&bytes(&b, &global_position(&b, 1)), 1.0).traffic.is_empty());
    }

    #[test]
    fn adsb_vehicle_is_traffic_with_its_callsign() {
        let mut d = Decoder::new(1);
        let b = beacon(9);
        let heard = d.push(&bytes(&b, &adsb(&b, 0xA1B2C3, "N123AB", 2)), 30.0);
        let o = &heard.traffic[0];
        assert_eq!(o.identity, Identity::new(Source::Adsb, "a1b2c3"));
        assert_eq!(o.name.as_deref(), Some("N123AB"));
        assert_eq!(o.time, 28.0, "the report is two seconds old");
        assert!(o.position.distance_to(&b.position) < 0.1);
    }

    #[test]
    fn a_forwarded_remote_id_pack_is_traffic_named_by_its_id() {
        let b = beacon(5);
        let broadcast = rid::encode_pack(&[remote_id_static(&b).remove(0), remote_id_location(&b, 600.0)]);
        let messages = forward_remote_id(MAC, &broadcast);
        assert_eq!(messages.len(), 1, "a pack goes on as a pack");
        let mut d = Decoder::new(1);
        let mut stream = vec![];
        for m in &messages {
            stream.extend(frame(1, ODID_COMPONENT, 0, m));
        }
        let heard = d.push(&stream, 10.0);
        assert_eq!(heard.traffic.len(), 1);
        let o = &heard.traffic[0];
        assert_eq!(o.identity, Identity::new(Source::RemoteId, "60:3d:26:01:02:03"));
        assert_eq!(o.name.as_deref(), Some("T1S0005"));
        assert!(o.position.distance_to(&b.position) < 0.1, "{:?}", o.position);
        assert!((o.position.alt - 82.5).abs() < 0.5);
        assert!((o.heading.unwrap() - 337.0).abs() < 1.0);
        let v = o.velocity.unwrap();
        assert!((v[0] - 12.0).abs() < 0.5 && (v[1] + 5.0).abs() < 0.5, "{v:?}");
    }

    #[test]
    fn single_remote_id_messages_work_too() {
        let b = beacon(5);
        let mut d = Decoder::new(1);
        let mut stream = vec![];
        for broadcast in [rid::encode(&remote_id_static(&b)[0]).to_vec(), rid::encode(&remote_id_location(&b, 1.0)).to_vec()] {
            for m in forward_remote_id(MAC, &broadcast) {
                assert!(!matches!(m, MavMessage::OPEN_DRONE_ID_MESSAGE_PACK(_)));
                stream.extend(frame(1, ODID_COMPONENT, 0, &m));
            }
        }
        let heard = d.push(&stream, 3.0);
        assert_eq!(heard.traffic.len(), 1);
        assert_eq!(heard.traffic[0].name.as_deref(), Some("T1S0005"));
        assert!(heard.traffic[0].position.distance_to(&b.position) < 0.1);
    }

    #[test]
    fn the_planes_own_remote_id_transmitter_is_not_traffic() {
        // Messages with no sender address are the plane's own transmitter talking to its autopilot.
        let b = beacon(1);
        let broadcast = rid::encode_pack(&[remote_id_location(&b, 1.0)]);
        let own = forward_remote_id([0; 6], &broadcast);
        let mut d = Decoder::new(1);
        let heard = d.push(&frame(1, ODID_COMPONENT, 0, &own[0]), 1.0);
        assert!(heard.traffic.is_empty());
    }

    #[test]
    fn a_remote_id_location_without_a_basic_id_is_still_traffic() {
        let b = beacon(5);
        let broadcast = rid::encode(&remote_id_location(&b, 1.0)).to_vec();
        let mut d = Decoder::new(1);
        let heard = d.push(&frame(1, ODID_COMPONENT, 0, &forward_remote_id(MAC, &broadcast)[0]), 1.0);
        assert_eq!(heard.traffic.len(), 1);
        assert_eq!(heard.traffic[0].name, None);
    }

    #[test]
    fn a_broadcast_that_is_not_remote_id_forwards_nothing() {
        assert!(forward_remote_id(MAC, &[1, 2, 3]).is_empty());
    }
}
