//! Remote ID: the broadcast that tells every receiver near an unmanned aircraft who it is and
//! where it is (ASTM F3411-22a, ASD-STAN prEN 4709-002; the open implementation is
//! `opendroneid-core-c`, and the byte layout and scaling here follow its `opendroneid.h` and
//! `opendroneid.c`).
//!
//! Every message is 25 bytes. Byte 0 holds the message type (high nibble) and the protocol
//! version (low nibble). A *message pack* holds up to nine messages behind a 3-byte header; it
//! is what goes out on Bluetooth 5 Long Range and on Wi-Fi. Bluetooth 4 legacy advertising
//! carries one message at a time.
//!
//! This crate only turns messages into bytes and back. Who hears them is the `air` module's job.

use std::fmt;

/// Every message is this many bytes.
pub const MESSAGE_SIZE: usize = 25;
/// A message pack holds at most this many messages.
pub const PACK_MAX: usize = 9;
/// The protocol version in byte 0 of every message.
pub const VERSION: u8 = 2;
/// The length of an ID (Basic ID, Operator ID).
pub const ID_SIZE: usize = 20;
/// The length of the text of a Self ID message.
pub const TEXT_SIZE: usize = 23;

const TYPE_BASIC_ID: u8 = 0;
const TYPE_LOCATION: u8 = 1;
const TYPE_AUTH: u8 = 2;
const TYPE_SELF_ID: u8 = 3;
const TYPE_SYSTEM: u8 = 4;
const TYPE_OPERATOR_ID: u8 = 5;
const TYPE_PACK: u8 = 0xF;

/// What kind of ID a Basic ID message holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdType {
    None,
    /// The manufacturer's serial number (ANSI/CTA-2063-A).
    SerialNumber,
    /// The registration that the aviation authority gave.
    CaaRegistration,
    /// An ID that an UAS traffic management system gave.
    UtmAssigned,
    /// An ID for one flight.
    SpecificSession,
    Other(u8),
}

impl IdType {
    fn code(self) -> u8 {
        match self {
            IdType::None => 0,
            IdType::SerialNumber => 1,
            IdType::CaaRegistration => 2,
            IdType::UtmAssigned => 3,
            IdType::SpecificSession => 4,
            IdType::Other(c) => c & 0xF,
        }
    }

    fn from_code(c: u8) -> IdType {
        match c {
            0 => IdType::None,
            1 => IdType::SerialNumber,
            2 => IdType::CaaRegistration,
            3 => IdType::UtmAssigned,
            4 => IdType::SpecificSession,
            c => IdType::Other(c),
        }
    }
}

/// The type of the unmanned aircraft (the `ua_type` of MAVLink's MAV_ODID_UA_TYPE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UaType {
    None,
    Aeroplane,
    HelicopterOrMultirotor,
    Gyroplane,
    HybridLift,
    Ornithopter,
    Glider,
    Kite,
    FreeBalloon,
    CaptiveBalloon,
    Airship,
    FreeFall,
    Rocket,
    TetheredPowered,
    GroundObstacle,
    Other(u8),
}

impl UaType {
    const ALL: [UaType; 15] = [
        UaType::None,
        UaType::Aeroplane,
        UaType::HelicopterOrMultirotor,
        UaType::Gyroplane,
        UaType::HybridLift,
        UaType::Ornithopter,
        UaType::Glider,
        UaType::Kite,
        UaType::FreeBalloon,
        UaType::CaptiveBalloon,
        UaType::Airship,
        UaType::FreeFall,
        UaType::Rocket,
        UaType::TetheredPowered,
        UaType::GroundObstacle,
    ];

    pub fn code(self) -> u8 {
        match self {
            UaType::Other(c) => c & 0xF,
            t => Self::ALL.iter().position(|x| *x == t).unwrap() as u8,
        }
    }

    pub fn from_code(c: u8) -> UaType {
        Self::ALL.get(c as usize).copied().unwrap_or(UaType::Other(c))
    }
}

/// Whether the aircraft says it is on the ground or in the air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Undeclared,
    Ground,
    Airborne,
    Emergency,
    RemoteIdFailure,
    Other(u8),
}

impl Status {
    fn code(self) -> u8 {
        match self {
            Status::Undeclared => 0,
            Status::Ground => 1,
            Status::Airborne => 2,
            Status::Emergency => 3,
            Status::RemoteIdFailure => 4,
            Status::Other(c) => c & 0xF,
        }
    }

    fn from_code(c: u8) -> Status {
        match c {
            0 => Status::Undeclared,
            1 => Status::Ground,
            2 => Status::Airborne,
            3 => Status::Emergency,
            4 => Status::RemoteIdFailure,
            c => Status::Other(c),
        }
    }
}

/// Who the aircraft is.
#[derive(Clone, Debug, PartialEq)]
pub struct BasicId {
    pub id_type: IdType,
    pub ua_type: UaType,
    /// At most 20 ASCII characters.
    pub uas_id: String,
}

/// Where the aircraft is and where it goes. `None` is the standard's "unknown".
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub status: Status,
    /// Track over the ground (degrees clockwise from true north, 0 to 359).
    pub direction: Option<f32>,
    /// Speed over the ground (m/s).
    pub speed: Option<f32>,
    /// Climb rate (m/s, up is positive).
    pub vertical_speed: Option<f32>,
    pub lat: f64,
    pub lon: f64,
    /// Altitude on the standard atmosphere (m).
    pub pressure_alt: Option<f32>,
    /// Altitude above the WGS-84 ellipsoid (m).
    pub geodetic_alt: Option<f32>,
    /// Height above the take-off point, or above the ground (m).
    pub height: Option<f32>,
    /// Height is above the ground, not above the take-off point.
    pub height_above_ground: bool,
    /// Accuracy codes, as the standard numbers them (0 is unknown).
    pub horizontal_accuracy: u8,
    pub vertical_accuracy: u8,
    pub baro_accuracy: u8,
    pub speed_accuracy: u8,
    pub timestamp_accuracy: u8,
    /// Seconds since the start of the hour, in steps of 0.1 s.
    pub timestamp: Option<f32>,
}

impl Default for Location {
    fn default() -> Self {
        Location {
            status: Status::Undeclared,
            direction: None,
            speed: None,
            vertical_speed: None,
            lat: 0.0,
            lon: 0.0,
            pressure_alt: None,
            geodetic_alt: None,
            height: None,
            height_above_ground: false,
            horizontal_accuracy: 0,
            vertical_accuracy: 0,
            baro_accuracy: 0,
            speed_accuracy: 0,
            timestamp_accuracy: 0,
            timestamp: None,
        }
    }
}

/// Where the operator is, and the operating area.
#[derive(Clone, Debug, PartialEq)]
pub struct System {
    /// 0 take-off point, 1 live GNSS, 2 fixed.
    pub operator_location_type: u8,
    pub classification_type: u8,
    pub operator_lat: f64,
    pub operator_lon: f64,
    pub area_count: u16,
    /// The radius of the area (m, in steps of 10 m).
    pub area_radius: u16,
    pub area_ceiling: Option<f32>,
    pub area_floor: Option<f32>,
    pub category_eu: u8,
    pub class_eu: u8,
    pub operator_alt: Option<f32>,
    /// Seconds since 2019-01-01 00:00 UTC.
    pub timestamp: u32,
}

impl Default for System {
    fn default() -> Self {
        System {
            operator_location_type: 0,
            classification_type: 0,
            operator_lat: 0.0,
            operator_lon: 0.0,
            area_count: 1,
            area_radius: 0,
            area_ceiling: None,
            area_floor: None,
            category_eu: 0,
            class_eu: 0,
            operator_alt: None,
            timestamp: 0,
        }
    }
}

/// The ID of the operator.
#[derive(Clone, Debug, PartialEq)]
pub struct OperatorId {
    pub id_type: u8,
    pub id: String,
}

/// Free text that the operator wrote, such as what the flight is for.
#[derive(Clone, Debug, PartialEq)]
pub struct SelfId {
    pub description_type: u8,
    /// At most 23 ASCII characters.
    pub text: String,
}

/// One 25-byte Remote ID message.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    BasicId(BasicId),
    Location(Location),
    /// Authentication data is passed along as it is.
    Authentication([u8; MESSAGE_SIZE]),
    SelfId(SelfId),
    System(System),
    OperatorId(OperatorId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Fewer bytes than the message needs.
    Short(usize),
    /// The message type is not one of the six.
    UnknownType(u8),
    /// The pack header is wrong: the message size is not 25, or the count is 0 or more than 9.
    BadPack,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Short(n) => write!(f, "Remote ID message too short ({n} bytes)"),
            Error::UnknownType(t) => write!(f, "unknown Remote ID message type {t}"),
            Error::BadPack => write!(f, "bad Remote ID message pack header"),
        }
    }
}

impl std::error::Error for Error {}

/// A latitude or longitude, in degrees, as the standard stores it (1e-7 degree).
fn encode_degrees(v: f64) -> [u8; 4] {
    ((v * 1e7).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32).to_le_bytes()
}

fn decode_degrees(b: &[u8]) -> f64 {
    i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 1e7
}

/// An altitude (m) in steps of 0.5 m from -1000 m. -1000 m is "unknown".
fn encode_altitude(v: Option<f32>) -> [u8; 2] {
    let raw = v.map_or(0.0, |a| ((a + 1000.0) / 0.5).round().clamp(0.0, u16::MAX as f32));
    (raw as u16).to_le_bytes()
}

fn decode_altitude(b: &[u8]) -> Option<f32> {
    let raw = u16::from_le_bytes([b[0], b[1]]);
    (raw != 0).then(|| raw as f32 * 0.5 - 1000.0)
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| if b.is_ascii() { b as char } else { '?' }).collect()
}

fn put_text(out: &mut [u8], s: &str) {
    for (o, c) in out.iter_mut().zip(s.chars()) {
        *o = if c.is_ascii() { c as u8 } else { b'?' };
    }
}

fn header(kind: u8) -> u8 {
    kind << 4 | VERSION
}

/// The 25 bytes of a message.
pub fn encode(m: &Message) -> [u8; MESSAGE_SIZE] {
    let mut b = [0u8; MESSAGE_SIZE];
    match m {
        Message::BasicId(x) => {
            b[0] = header(TYPE_BASIC_ID);
            b[1] = x.id_type.code() << 4 | x.ua_type.code();
            put_text(&mut b[2..2 + ID_SIZE], &x.uas_id);
        }
        Message::Location(x) => {
            b[0] = header(TYPE_LOCATION);
            let dir = x.direction.map_or(361, |d| (d.round() as i32).rem_euclid(360) as u32);
            let (dir_byte, ew) = if dir >= 180 { ((dir - 180) as u8, 1) } else { (dir as u8, 0) };
            let (speed_byte, mult) = match x.speed {
                None => (255, 0),
                Some(s) if s <= 255.0 * 0.25 => ((s / 0.25).round().clamp(0.0, 255.0) as u8, 0),
                Some(s) => (((s - 255.0 * 0.25) / 0.75).round().clamp(0.0, 254.0) as u8, 1),
            };
            b[1] = x.status.code() << 4 | (x.height_above_ground as u8) << 2 | ew << 1 | mult;
            b[2] = dir_byte;
            b[3] = speed_byte;
            b[4] = x.vertical_speed.map_or(63, |v| (v / 0.5).round().clamp(-124.0, 124.0) as i8) as u8;
            b[5..9].copy_from_slice(&encode_degrees(x.lat));
            b[9..13].copy_from_slice(&encode_degrees(x.lon));
            b[13..15].copy_from_slice(&encode_altitude(x.pressure_alt));
            b[15..17].copy_from_slice(&encode_altitude(x.geodetic_alt));
            b[17..19].copy_from_slice(&encode_altitude(x.height));
            b[19] = x.vertical_accuracy << 4 | (x.horizontal_accuracy & 0xF);
            b[20] = x.baro_accuracy << 4 | (x.speed_accuracy & 0xF);
            let ts = x.timestamp.map_or(0xFFFF, |t| (t * 10.0).round().clamp(0.0, 36000.0) as u16);
            b[21..23].copy_from_slice(&ts.to_le_bytes());
            b[23] = x.timestamp_accuracy & 0xF;
        }
        Message::Authentication(raw) => b = *raw,
        Message::SelfId(x) => {
            b[0] = header(TYPE_SELF_ID);
            b[1] = x.description_type;
            put_text(&mut b[2..2 + TEXT_SIZE], &x.text);
        }
        Message::System(x) => {
            b[0] = header(TYPE_SYSTEM);
            b[1] = x.classification_type << 2 | (x.operator_location_type & 3);
            b[2..6].copy_from_slice(&encode_degrees(x.operator_lat));
            b[6..10].copy_from_slice(&encode_degrees(x.operator_lon));
            b[10..12].copy_from_slice(&x.area_count.to_le_bytes());
            b[12] = (x.area_radius / 10).min(255) as u8;
            b[13..15].copy_from_slice(&encode_altitude(x.area_ceiling));
            b[15..17].copy_from_slice(&encode_altitude(x.area_floor));
            b[17] = x.category_eu << 4 | (x.class_eu & 0xF);
            b[18..20].copy_from_slice(&encode_altitude(x.operator_alt));
            b[20..24].copy_from_slice(&x.timestamp.to_le_bytes());
        }
        Message::OperatorId(x) => {
            b[0] = header(TYPE_OPERATOR_ID);
            b[1] = x.id_type;
            put_text(&mut b[2..2 + ID_SIZE], &x.id);
        }
    }
    b
}

/// The message in the first 25 bytes of `b`.
pub fn decode(b: &[u8]) -> Result<Message, Error> {
    if b.len() < MESSAGE_SIZE {
        return Err(Error::Short(b.len()));
    }
    let b = &b[..MESSAGE_SIZE];
    Ok(match b[0] >> 4 {
        TYPE_BASIC_ID => Message::BasicId(BasicId {
            id_type: IdType::from_code(b[1] >> 4),
            ua_type: UaType::from_code(b[1] & 0xF),
            uas_id: text(&b[2..2 + ID_SIZE]),
        }),
        TYPE_LOCATION => {
            let ew = b[1] >> 1 & 1;
            let direction = (b[2] != 181 || ew == 0).then(|| b[2] as f32 + if ew == 1 { 180.0 } else { 0.0 });
            let speed = (b[3] != 255).then(|| {
                if b[1] & 1 == 1 { b[3] as f32 * 0.75 + 255.0 * 0.25 } else { b[3] as f32 * 0.25 }
            });
            let ts = u16::from_le_bytes([b[21], b[22]]);
            Message::Location(Location {
                status: Status::from_code(b[1] >> 4),
                direction,
                speed,
                vertical_speed: (b[4] as i8 != 63).then(|| b[4] as i8 as f32 * 0.5),
                lat: decode_degrees(&b[5..9]),
                lon: decode_degrees(&b[9..13]),
                pressure_alt: decode_altitude(&b[13..15]),
                geodetic_alt: decode_altitude(&b[15..17]),
                height: decode_altitude(&b[17..19]),
                height_above_ground: b[1] >> 2 & 1 == 1,
                vertical_accuracy: b[19] >> 4,
                horizontal_accuracy: b[19] & 0xF,
                baro_accuracy: b[20] >> 4,
                speed_accuracy: b[20] & 0xF,
                timestamp_accuracy: b[23] & 0xF,
                timestamp: (ts != 0xFFFF).then(|| ts as f32 / 10.0),
            })
        }
        TYPE_AUTH => Message::Authentication(b.try_into().unwrap()),
        TYPE_SELF_ID => Message::SelfId(SelfId { description_type: b[1], text: text(&b[2..2 + TEXT_SIZE]) }),
        TYPE_SYSTEM => Message::System(System {
            operator_location_type: b[1] & 3,
            classification_type: b[1] >> 2 & 7,
            operator_lat: decode_degrees(&b[2..6]),
            operator_lon: decode_degrees(&b[6..10]),
            area_count: u16::from_le_bytes([b[10], b[11]]),
            area_radius: b[12] as u16 * 10,
            area_ceiling: decode_altitude(&b[13..15]),
            area_floor: decode_altitude(&b[15..17]),
            category_eu: b[17] >> 4,
            class_eu: b[17] & 0xF,
            operator_alt: decode_altitude(&b[18..20]),
            timestamp: u32::from_le_bytes([b[20], b[21], b[22], b[23]]),
        }),
        TYPE_OPERATOR_ID => Message::OperatorId(OperatorId { id_type: b[1], id: text(&b[2..2 + ID_SIZE]) }),
        t => return Err(Error::UnknownType(t)),
    })
}

/// A message pack: a 3-byte header, then up to nine messages of 25 bytes.
pub fn encode_pack(messages: &[Message]) -> Vec<u8> {
    let messages = &messages[..messages.len().min(PACK_MAX)];
    let mut out = vec![header(TYPE_PACK), MESSAGE_SIZE as u8, messages.len() as u8];
    for m in messages {
        out.extend_from_slice(&encode(m));
    }
    out
}

/// The messages in a pack.
pub fn decode_pack(b: &[u8]) -> Result<Vec<Message>, Error> {
    if b.len() < 3 {
        return Err(Error::Short(b.len()));
    }
    let (size, count) = (b[1] as usize, b[2] as usize);
    if b[0] >> 4 != TYPE_PACK || size != MESSAGE_SIZE || count == 0 || count > PACK_MAX {
        return Err(Error::BadPack);
    }
    if b.len() < 3 + count * size {
        return Err(Error::Short(b.len()));
    }
    b[3..3 + count * size].chunks(size).map(decode).collect()
}

/// What a receiver does with a broadcast: one message, or a pack of them.
pub fn decode_broadcast(b: &[u8]) -> Result<Vec<Message>, Error> {
    match b.first() {
        None => Err(Error::Short(0)),
        Some(&h) if h >> 4 == TYPE_PACK => decode_pack(b),
        Some(_) => Ok(vec![decode(b)?]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flying() -> Location {
        Location {
            status: Status::Airborne,
            direction: Some(271.0),
            speed: Some(18.5),
            vertical_speed: Some(-1.5),
            lat: 37.870_123_4,
            lon: -122.272_345_6,
            pressure_alt: Some(112.5),
            geodetic_alt: Some(115.0),
            height: Some(31.5),
            height_above_ground: true,
            horizontal_accuracy: 11,
            vertical_accuracy: 6,
            baro_accuracy: 4,
            speed_accuracy: 3,
            timestamp_accuracy: 2,
            timestamp: Some(1234.5),
        }
    }

    #[test]
    fn a_message_is_25_bytes_with_its_type_and_version_in_the_first() {
        let b = encode(&Message::BasicId(BasicId {
            id_type: IdType::SerialNumber,
            ua_type: UaType::Aeroplane,
            uas_id: "1596F2301ABC".into(),
        }));
        assert_eq!(b.len(), 25);
        assert_eq!(b[0], 0x02, "basic ID (0) in the high nibble, version 2 in the low");
        assert_eq!(b[1], 0x11, "serial number (1), aeroplane (1)");
        assert_eq!(&b[2..14], b"1596F2301ABC");
        assert!(b[14..].iter().all(|&x| x == 0));
    }

    #[test]
    fn the_location_message_uses_the_standards_scaling() {
        let b = encode(&Message::Location(flying()));
        assert_eq!(b[0], 0x12);
        // Airborne (2), height above ground (bit 2), direction 271 is 91 + the east-west flag (bit 1), speed 18.5 needs no multiplier.
        assert_eq!(b[1], 0x26);
        assert_eq!(b[2], 91);
        assert_eq!(b[3], 74, "18.5 m/s in steps of 0.25");
        assert_eq!(b[4] as i8, -3, "-1.5 m/s in steps of 0.5");
        assert_eq!(i32::from_le_bytes(b[5..9].try_into().unwrap()), 378_701_234);
        assert_eq!(i32::from_le_bytes(b[9..13].try_into().unwrap()), -1_222_723_456);
        assert_eq!(u16::from_le_bytes(b[13..15].try_into().unwrap()), 2225, "(112.5 + 1000) / 0.5");
        assert_eq!(b[19], 0x6B, "vertical accuracy in the high nibble, horizontal in the low");
        assert_eq!(b[20], 0x43, "barometric accuracy in the high nibble, speed in the low");
        assert_eq!(u16::from_le_bytes(b[21..23].try_into().unwrap()), 12345, "tenths of a second");
        assert_eq!(b[23], 2);
        assert_eq!(b[24], 0);
    }

    #[test]
    fn a_location_survives_the_trip() {
        let back = decode(&encode(&Message::Location(flying()))).unwrap();
        let Message::Location(l) = back else { panic!("not a location") };
        let want = flying();
        assert_eq!(l.status, Status::Airborne);
        assert_eq!(l.direction, Some(271.0));
        assert_eq!(l.speed, Some(18.5));
        assert_eq!(l.vertical_speed, Some(-1.5));
        assert!((l.lat - want.lat).abs() < 1e-7 && (l.lon - want.lon).abs() < 1e-7);
        assert_eq!((l.pressure_alt, l.geodetic_alt, l.height), (Some(112.5), Some(115.0), Some(31.5)));
        assert!(l.height_above_ground);
        assert_eq!((l.horizontal_accuracy, l.vertical_accuracy, l.baro_accuracy, l.speed_accuracy), (11, 6, 4, 3));
        assert_eq!(l.timestamp, Some(1234.5));
    }

    #[test]
    fn a_fast_plane_uses_the_speed_multiplier() {
        let m = Message::Location(Location { speed: Some(80.0), ..flying() });
        let b = encode(&m);
        assert_eq!(b[1] & 1, 1);
        assert_eq!(b[3], 22, "(80 - 63.75) / 0.75 rounded");
        let Message::Location(l) = decode(&b).unwrap() else { panic!() };
        assert!((l.speed.unwrap() - 80.25).abs() < 1e-3, "{:?}", l.speed);
    }

    #[test]
    fn unknown_values_stay_unknown() {
        let m = Message::Location(Location { lat: 10.0, lon: 20.0, ..Location::default() });
        let b = encode(&m);
        assert_eq!(b[2], 181, "unknown direction is 361, stored as 181 with the east-west flag");
        assert_eq!(b[3], 255);
        assert_eq!(b[4], 63);
        assert_eq!(&b[13..19], &[0; 6], "unknown altitudes are -1000 m");
        assert_eq!(&b[21..23], &[0xFF, 0xFF]);
        let Message::Location(l) = decode(&b).unwrap() else { panic!() };
        assert_eq!((l.direction, l.speed, l.vertical_speed), (None, None, None));
        assert_eq!((l.pressure_alt, l.geodetic_alt, l.height, l.timestamp), (None, None, None, None));
    }

    #[test]
    fn north_is_zero_and_360_wraps() {
        let b = encode(&Message::Location(Location { direction: Some(360.0), ..Location::default() }));
        assert_eq!(b[2], 0);
        let Message::Location(l) = decode(&b).unwrap() else { panic!() };
        assert_eq!(l.direction, Some(0.0));
    }

    #[test]
    fn the_other_messages_survive_the_trip() {
        let all = [
            Message::BasicId(BasicId { id_type: IdType::CaaRegistration, ua_type: UaType::Glider, uas_id: "FIN87astrdge12k8".into() }),
            Message::SelfId(SelfId { description_type: 0, text: "Pylon race, Berkeley".into() }),
            Message::OperatorId(OperatorId { id_type: 0, id: "FIN87astrdge12k8".into() }),
            Message::System(System {
                operator_location_type: 1,
                classification_type: 1,
                operator_lat: 37.8701,
                operator_lon: -122.2723,
                area_count: 1,
                area_radius: 100,
                area_ceiling: Some(120.0),
                area_floor: Some(0.0),
                category_eu: 1,
                class_eu: 2,
                operator_alt: Some(30.5),
                timestamp: 123_456_789,
            }),
            Message::Authentication([0x20 | 2; 25]),
        ];
        for m in all {
            let back = decode(&encode(&m)).unwrap();
            match (&m, &back) {
                (Message::System(a), Message::System(b)) => {
                    assert!((a.operator_lat - b.operator_lat).abs() < 1e-7);
                    assert_eq!(System { operator_lat: 0.0, operator_lon: 0.0, ..a.clone() }, System { operator_lat: 0.0, operator_lon: 0.0, ..b.clone() });
                }
                _ => assert_eq!(m, back),
            }
        }
    }

    #[test]
    fn long_text_is_cut_to_fit() {
        let m = Message::BasicId(BasicId { id_type: IdType::None, ua_type: UaType::None, uas_id: "X".repeat(30) });
        let Message::BasicId(b) = decode(&encode(&m)).unwrap() else { panic!() };
        assert_eq!(b.uas_id.len(), 20);
    }

    #[test]
    fn a_pack_has_a_three_byte_header_then_the_messages() {
        let basic = Message::BasicId(BasicId { id_type: IdType::SerialNumber, ua_type: UaType::Aeroplane, uas_id: "T1S0001".into() });
        let loc = Message::Location(flying());
        let pack = encode_pack(&[basic.clone(), loc.clone()]);
        assert_eq!(pack.len(), 3 + 2 * 25);
        assert_eq!(&pack[..3], &[0xF2, 25, 2]);
        assert_eq!(decode_pack(&pack).unwrap().len(), 2);
        assert_eq!(decode_broadcast(&pack).unwrap()[0], basic);
        assert_eq!(decode_broadcast(&encode(&loc)).unwrap().len(), 1, "a single message is a broadcast too");
    }

    #[test]
    fn a_pack_holds_at_most_nine_messages() {
        let m = Message::SelfId(SelfId { description_type: 0, text: "x".into() });
        let pack = encode_pack(&vec![m; 12]);
        assert_eq!(pack[2], 9);
        assert_eq!(pack.len(), 3 + 9 * 25);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert_eq!(decode(&[0x12; 10]), Err(Error::Short(10)));
        assert_eq!(decode(&[0x92; 25]), Err(Error::UnknownType(9)));
        assert_eq!(decode_pack(&[0xF2, 24, 1, 0]), Err(Error::BadPack));
        assert_eq!(decode_pack(&[0xF2, 25, 0]), Err(Error::BadPack));
        assert_eq!(decode_pack(&[0xF2, 25, 3, 0, 0]), Err(Error::Short(5)));
        assert_eq!(decode_broadcast(&[]), Err(Error::Short(0)));
    }
}
