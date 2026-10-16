//! Sensor video feed: the sim streams its gimballed camera to the ground station over TCP.
//!
//! Server (sim) → client (GCS): a [`FrameHeader`] followed by `stride * height` bytes of BGRA8
//! (sRGB) pixels, one message per frame. Client → server: 16-byte [`ClientMsg`]s, any time:
//! gimbal commands, and the autopilot's armed state and mode, which the sim can't see itself
//! (JSON SITL only sends it servo outputs).
//! Everything is little-endian. Raw frames keep it simple and latency-free on localhost
//! (even 1280×720 at 30 fps is ~110 MB/s, nothing for loopback; the sim sends 320×240 by default).

use std::io::{self, Read, Write};

pub const DEFAULT_PORT: u16 = 5600;
const FRAME_MAGIC: [u8; 4] = *b"T1SF";
const CMD_MAGIC: [u8; 4] = *b"T1SC";
const STATUS_MAGIC: [u8; 4] = *b"T1SS";
const RADIO_MAGIC: [u8; 4] = *b"T1SR";
const KEY_MAGIC: [u8; 4] = *b"T1SK";
pub const FORMAT_BGRA8: u16 = 1;

/// Gimbal limits (degrees). Pan is body-relative, + right; tilt + up; hfov is the zoom.
pub const PAN_LIMIT: f32 = 170.0;
pub const TILT_MIN: f32 = -90.0;
pub const TILT_MAX: f32 = 20.0;
pub const HFOV_MIN: f32 = 4.0;
pub const HFOV_MAX: f32 = 90.0;
pub const HFOV_DEFAULT: f32 = 75.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gimbal {
    pub pan: f32,
    pub tilt: f32,
    pub hfov: f32,
}

impl Default for Gimbal {
    fn default() -> Self {
        Gimbal { pan: 0.0, tilt: 0.0, hfov: HFOV_DEFAULT }
    }
}

impl Gimbal {
    pub fn clamped(self) -> Gimbal {
        Gimbal {
            pan: self.pan.clamp(-PAN_LIMIT, PAN_LIMIT),
            tilt: self.tilt.clamp(TILT_MIN, TILT_MAX),
            hfov: self.hfov.clamp(HFOV_MIN, HFOV_MAX),
        }
    }

    /// Looking straight ahead along the airframe, so flight symbology is conformal.
    pub fn is_boresight(&self) -> bool {
        self.pan.abs() < 0.5 && self.tilt.abs() < 0.5
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameHeader {
    pub format: u16,
    pub width: u16,
    pub height: u16,
    /// How many times the sim has reset the aircraft (wraps). A change tells the ground station
    /// to restart the autopilot: a teleported aircraft leaves its estimators inconsistent.
    pub resets: u16,
    /// Bytes per row (rows may be padded).
    pub stride: u32,
    pub frame: u32,
    /// Simulation time of the frame, seconds.
    pub sim_time: f32,
    /// Gimbal state the frame was rendered with.
    pub gimbal: Gimbal,
    /// [`FLAG_DIRECT`] etc.
    pub flags: u32,
    /// The arena of a game of tag, if one is being played.
    pub arena: Option<Arena>,
    /// The game being played, if any.
    pub game: Option<GameStatus>,
}

/// Which game the sim is playing, and where its clock is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameStatus {
    pub kind: GameKind,
    pub clock: GameClock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameKind {
    Race,
    Tag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameClock {
    /// Waiting for the pilot to take off.
    Waiting,
    CountingDown,
    Running,
    Stopped,
}

impl GameStatus {
    fn encode(game: Option<GameStatus>) -> [u8; 2] {
        match game {
            None => [0, 0],
            Some(g) => [
                match g.kind {
                    GameKind::Race => 1,
                    GameKind::Tag => 2,
                },
                match g.clock {
                    GameClock::Waiting => 0,
                    GameClock::CountingDown => 1,
                    GameClock::Running => 2,
                    GameClock::Stopped => 3,
                },
            ],
        }
    }

    fn decode(b: [u8; 2]) -> Option<GameStatus> {
        let kind = match b[0] {
            1 => GameKind::Race,
            2 => GameKind::Tag,
            _ => return None,
        };
        let clock = match b[1] {
            1 => GameClock::CountingDown,
            2 => GameClock::Running,
            3 => GameClock::Stopped,
            _ => GameClock::Waiting,
        };
        Some(GameStatus { kind, clock })
    }
}

/// The arena of a game of tag, in metres north and east of the sim's origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arena {
    pub north: f32,
    pub east: f32,
    pub radius: f32,
}

/// How many cones mark the edge of an arena.
pub const ARENA_CONES: usize = 24;

impl Arena {
    /// Where the cones stand (north, east).
    pub fn cones(&self) -> Vec<(f32, f32)> {
        (0..ARENA_CONES)
            .map(|i| {
                let a = i as f32 / ARENA_CONES as f32 * std::f32::consts::TAU;
                (self.north - a.sin() * self.radius, self.east + a.cos() * self.radius)
            })
            .collect()
    }
}

/// The sticks are flying the aircraft through the sim's own modes; ArduPilot only watches.
pub const FLAG_DIRECT: u32 = 1;

impl FrameHeader {
    pub const SIZE: usize = 56;

    pub fn payload_len(&self) -> usize {
        self.stride as usize * self.height as usize
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&FRAME_MAGIC);
        b[4..6].copy_from_slice(&self.format.to_le_bytes());
        b[6..8].copy_from_slice(&self.width.to_le_bytes());
        b[8..10].copy_from_slice(&self.height.to_le_bytes());
        b[10..12].copy_from_slice(&self.resets.to_le_bytes());
        b[12..16].copy_from_slice(&self.stride.to_le_bytes());
        b[16..20].copy_from_slice(&self.frame.to_le_bytes());
        b[20..24].copy_from_slice(&self.sim_time.to_le_bytes());
        b[24..28].copy_from_slice(&self.gimbal.pan.to_le_bytes());
        b[28..32].copy_from_slice(&self.gimbal.tilt.to_le_bytes());
        b[32..36].copy_from_slice(&self.gimbal.hfov.to_le_bytes());
        b[36..40].copy_from_slice(&self.flags.to_le_bytes());
        let a = self.arena.unwrap_or(Arena { north: 0.0, east: 0.0, radius: 0.0 });
        b[40..44].copy_from_slice(&a.north.to_le_bytes());
        b[44..48].copy_from_slice(&a.east.to_le_bytes());
        b[48..52].copy_from_slice(&a.radius.to_le_bytes());
        b[52..54].copy_from_slice(&GameStatus::encode(self.game));
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> io::Result<FrameHeader> {
        if b[0..4] != FRAME_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad feed frame magic"));
        }
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let f32_at = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let h = FrameHeader {
            format: u16_at(4),
            width: u16_at(6),
            height: u16_at(8),
            resets: u16_at(10),
            stride: u32_at(12),
            frame: u32_at(16),
            sim_time: f32_at(20),
            gimbal: Gimbal { pan: f32_at(24), tilt: f32_at(28), hfov: f32_at(32) },
            flags: u32_at(36),
            arena: Some(Arena { north: f32_at(40), east: f32_at(44), radius: f32_at(48) }).filter(|a| a.radius > 0.0),
            game: GameStatus::decode([b[52], b[53]]),
        };
        if h.format != FORMAT_BGRA8 || (h.stride as usize) < h.width as usize * 4 || h.payload_len() > 64 << 20 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported feed frame"));
        }
        Ok(h)
    }
}

/// Writes one frame (header + pixels).
pub fn write_frame(w: &mut impl Write, header: &FrameHeader, pixels: &[u8]) -> io::Result<()> {
    debug_assert_eq!(pixels.len(), header.payload_len());
    w.write_all(&header.encode())?;
    w.write_all(pixels)
}

/// Reads one frame into `buf` (resized as needed).
pub fn read_frame(r: &mut impl Read, buf: &mut Vec<u8>) -> io::Result<FrameHeader> {
    let mut hb = [0u8; FrameHeader::SIZE];
    r.read_exact(&mut hb)?;
    let h = FrameHeader::decode(&hb)?;
    buf.resize(h.payload_len(), 0);
    r.read_exact(buf)?;
    Ok(h)
}

/// Absolute gimbal command from the ground station.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GimbalCmd(pub Gimbal);

impl GimbalCmd {
    pub const SIZE: usize = 16;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&CMD_MAGIC);
        b[4..8].copy_from_slice(&self.0.pan.to_le_bytes());
        b[8..12].copy_from_slice(&self.0.tilt.to_le_bytes());
        b[12..16].copy_from_slice(&self.0.hfov.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> io::Result<GimbalCmd> {
        if b[0..4] != CMD_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad gimbal command magic"));
        }
        let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let g = Gimbal { pan: f(4), tilt: f(8), hfov: f(12) };
        if !(g.pan.is_finite() && g.tilt.is_finite() && g.hfov.is_finite()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "non-finite gimbal command"));
        }
        Ok(GimbalCmd(g.clamped()))
    }
}

/// The autopilot's state as the ground station sees it over MAVLink.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleStatus {
    pub armed: bool,
    /// Flight mode name, e.g. "AUTO" (up to 10 bytes).
    pub mode: String,
}

impl VehicleStatus {
    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&STATUS_MAGIC);
        b[4] = self.armed as u8;
        let name = self.mode.as_bytes();
        let n = name.len().min(10);
        b[5] = n as u8;
        b[6..6 + n].copy_from_slice(&name[..n]);
        b
    }
}

/// A stand-in for the pilot's radio, for acceptance tests: the sticks, and how many times each
/// of the two switches that the pilot gave to ArduPilot has been pressed (the sim sees a press
/// when a count changes, as it does when the radio's button goes down). Only a sim started
/// with `--test-radio` listens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioState {
    /// The radio is plugged in. A test that doesn't use the radio leaves it out, as a pilot who
    /// flies with the keyboard does.
    pub connected: bool,
    /// Right is positive.
    pub roll: f32,
    /// Stick back (nose up) is positive.
    pub pitch: f32,
    /// Right is positive.
    pub yaw: f32,
    /// 0 is all the way down, 1 all the way up.
    pub throttle: f32,
    /// The arm switch (channel 5) is on.
    pub arm: bool,
    /// Which of the mode channel's (channel 8) six positions it is in, 0 to 5.
    pub mode: u8,
    /// How many times the test has asked the sim to start again with a fresh plane, as pressing
    /// R does. With a ground station attached, the station restarts ArduPilot for it.
    pub reset_presses: u8,
}

impl RadioState {
    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&RADIO_MAGIC);
        let q = |v: f32| ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
        b[4..6].copy_from_slice(&q(self.roll));
        b[6..8].copy_from_slice(&q(self.pitch));
        b[8..10].copy_from_slice(&q(self.yaw));
        b[10..12].copy_from_slice(&q(self.throttle.clamp(0.0, 1.0)));
        b[12] = self.arm as u8;
        b[13] = self.mode.min(5);
        b[14] = self.connected as u8;
        b[15] = self.reset_presses;
        b
    }

    fn decode(b: &[u8; 16]) -> RadioState {
        let q = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]) as f32 / 32767.0;
        RadioState {
            connected: b[14] != 0,
            roll: q(4),
            pitch: q(6),
            yaw: q(8),
            throttle: q(10),
            arm: b[12] != 0,
            mode: b[13].min(5),
            reset_presses: b[15],
        }
    }
}

/// One of the sim's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// G: start (or stop) a pylon race.
    Race,
    /// T: start (or stop) a game of tag.
    Tag,
}

/// A key pressed in the sim's window, sent from the station (its tests start games this way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress(pub Key);

impl KeyPress {
    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&KEY_MAGIC);
        b[4] = match self.0 {
            Key::Race => b'G',
            Key::Tag => b'T',
        };
        b
    }
}

/// A message from the ground station to the sim.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMsg {
    Gimbal(Gimbal),
    Status(VehicleStatus),
    Radio(RadioState),
    Key(Key),
}

impl ClientMsg {
    pub const SIZE: usize = 16;

    pub fn decode(b: &[u8; Self::SIZE]) -> io::Result<ClientMsg> {
        if b[0..4] == STATUS_MAGIC {
            let n = (b[5] as usize).min(10);
            let mode = String::from_utf8_lossy(&b[6..6 + n]).into_owned();
            return Ok(ClientMsg::Status(VehicleStatus { armed: b[4] != 0, mode }));
        }
        if b[0..4] == RADIO_MAGIC {
            return Ok(ClientMsg::Radio(RadioState::decode(b)));
        }
        if b[0..4] == KEY_MAGIC {
            return match b[4] {
                b'G' => Ok(ClientMsg::Key(Key::Race)),
                b'T' => Ok(ClientMsg::Key(Key::Tag)),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "unknown key")),
            };
        }
        GimbalCmd::decode(b).map(|c| ClientMsg::Gimbal(c.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let h = FrameHeader {
            format: FORMAT_BGRA8,
            width: 4,
            height: 2,
            resets: 3,
            stride: 16,
            frame: 7,
            sim_time: 12.5,
            gimbal: Gimbal { pan: -30.0, tilt: -10.0, hfov: 20.0 },
            flags: FLAG_DIRECT,
            arena: Some(Arena { north: 12.0, east: -40.0, radius: 90.0 }),
            game: Some(GameStatus { kind: GameKind::Race, clock: GameClock::CountingDown }),
        };
        let px: Vec<u8> = (0..32).collect();
        let mut wire = Vec::new();
        write_frame(&mut wire, &h, &px).unwrap();
        let mut buf = Vec::new();
        let got = read_frame(&mut wire.as_slice(), &mut buf).unwrap();
        assert_eq!(got, h);
        assert_eq!(buf, px);
    }

    #[test]
    fn client_messages() {
        let st = VehicleStatus { armed: true, mode: "TAKEOFF".into() };
        assert_eq!(ClientMsg::decode(&st.encode()).unwrap(), ClientMsg::Status(st));
        let g = Gimbal { pan: 10.0, tilt: -20.0, hfov: 30.0 };
        assert_eq!(ClientMsg::decode(&GimbalCmd(g).encode()).unwrap(), ClientMsg::Gimbal(g));
        let long = VehicleStatus { armed: false, mode: "QSTABILIZE_X".into() };
        assert!(matches!(ClientMsg::decode(&long.encode()).unwrap(), ClientMsg::Status(s) if s.mode == "QSTABILIZE"));
    }

    /// Free flight travels as no game; every game and clock state makes the trip.
    #[test]
    fn the_game_travels_in_the_header() {
        let base = FrameHeader {
            format: FORMAT_BGRA8,
            width: 1,
            height: 1,
            resets: 0,
            stride: 4,
            frame: 0,
            sim_time: 0.0,
            gimbal: Gimbal { pan: 0.0, tilt: 0.0, hfov: 20.0 },
            flags: 0,
            arena: None,
            game: None,
        };
        assert_eq!(FrameHeader::decode(&base.encode()).unwrap().game, None);
        for kind in [GameKind::Race, GameKind::Tag] {
            for clock in [GameClock::Waiting, GameClock::CountingDown, GameClock::Running, GameClock::Stopped] {
                let h = FrameHeader { game: Some(GameStatus { kind, clock }), ..base };
                assert_eq!(FrameHeader::decode(&h.encode()).unwrap().game, h.game);
            }
        }
    }

    /// The station presses the sim's G and T keys for its tests.
    #[test]
    fn a_key_press_roundtrips() {
        for k in [Key::Race, Key::Tag] {
            assert_eq!(ClientMsg::decode(&KeyPress(k).encode()).unwrap(), ClientMsg::Key(k));
        }
    }

    #[test]
    fn a_radio_message_roundtrips() {
        let r = RadioState { connected: true, roll: -0.5, pitch: 0.25, yaw: 1.0, throttle: 0.75, arm: true, mode: 5, reset_presses: 7 };
        let back = match ClientMsg::decode(&r.encode()).unwrap() {
            ClientMsg::Radio(b) => b,
            other => panic!("{other:?}"),
        };
        // Sticks travel as 16-bit integers: close, not exact.
        assert!((back.roll + 0.5).abs() < 1e-3 && (back.pitch - 0.25).abs() < 1e-3);
        assert!((back.yaw - 1.0).abs() < 1e-3 && (back.throttle - 0.75).abs() < 1e-3);
        assert_eq!((back.arm, back.mode), (true, 5));
        assert!(back.connected);
        assert_eq!(back.reset_presses, 7, "the reset counter travels too");
        let unplugged = RadioState { connected: false, ..r };
        let ClientMsg::Radio(u) = ClientMsg::decode(&unplugged.encode()).unwrap() else { panic!() };
        assert!(!u.connected, "a radio that isn't plugged in stays unplugged across the wire");
        // Out-of-range sticks are clamped, not wrapped.
        let wild = RadioState { connected: true, roll: 9.0, pitch: -9.0, yaw: 0.0, throttle: 5.0, arm: false, mode: 0, reset_presses: 0 };
        let ClientMsg::Radio(w) = ClientMsg::decode(&wild.encode()).unwrap() else { panic!() };
        assert!((w.roll - 1.0).abs() < 1e-3 && (w.pitch + 1.0).abs() < 1e-3 && (w.throttle - 1.0).abs() < 1e-3);
    }

    #[test]
    fn gimbal_cmd_clamps() {
        let c = GimbalCmd(Gimbal { pan: 400.0, tilt: -200.0, hfov: 1.0 });
        let back = GimbalCmd::decode(&c.encode()).unwrap().0;
        assert_eq!(back, Gimbal { pan: PAN_LIMIT, tilt: TILT_MIN, hfov: HFOV_MIN });
        assert!(GimbalCmd::decode(&[0; 16]).is_err());
    }
}
