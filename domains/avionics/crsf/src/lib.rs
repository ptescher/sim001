//! CRSF, the serial protocol between a TX15 (through its ExpressLRS module) and a receiver or
//! flight computer. A frame is `[address][length][type][payload...][crc]`, where the length
//! counts the type, the payload and the checksum, and the checksum (CRC-8, polynomial 0xD5)
//! covers the type and the payload.

/// The address of a flight controller: what the receiver sends to.
pub const ADDR_FC: u8 = 0xC8;
/// The address of the transmitter module in the radio: what the radio sends to.
pub const ADDR_TX: u8 = 0xEE;

pub const TYPE_GPS: u8 = 0x02;
pub const TYPE_VARIO: u8 = 0x07;
pub const TYPE_BATTERY: u8 = 0x08;
pub const TYPE_AIRSPEED: u8 = 0x0A;
pub const TYPE_LINK_STATS: u8 = 0x14;
pub const TYPE_RC_CHANNELS: u8 = 0x16;
pub const TYPE_ATTITUDE: u8 = 0x1E;
pub const TYPE_FLIGHT_MODE: u8 = 0x21;
/// ExpressLRS' envelope for MAVLink bytes.
pub const TYPE_MAVLINK_ENVELOPE: u8 = 0xAA;

/// The most payload a frame carries (the whole frame is at most 64 bytes).
pub const MAX_PAYLOAD: usize = 60;
/// The MAVLink bytes in one envelope: the chunk count and the size take two more.
pub const MAX_ENVELOPE_DATA: usize = 58;

pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0xD5 } else { crc << 1 };
        }
    }
    crc
}

/// A frame that passed its checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub addr: u8,
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// One frame as bytes. Panics if the payload is longer than [`MAX_PAYLOAD`].
pub fn encode(addr: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= MAX_PAYLOAD, "CRSF payload too long");
    let mut f = Vec::with_capacity(payload.len() + 4);
    f.extend_from_slice(&[addr, payload.len() as u8 + 2, kind]);
    f.extend_from_slice(payload);
    f.push(crc8(&f[2..]));
    f
}

/// Cuts a byte stream into frames. A frame with a wrong checksum is counted and dropped, and
/// the framer looks for the next frame from the byte after its address.
#[derive(Default)]
pub struct Framer {
    buf: Vec<u8>,
    /// Frames that passed the checksum, and frames that did not.
    pub good: u64,
    pub bad: u64,
}

fn is_address(b: u8) -> bool {
    matches!(b, ADDR_FC | ADDR_TX | 0xEA | 0xEC | 0x00)
}

impl Framer {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(bytes);
        let mut out = vec![];
        loop {
            let start = self.buf.iter().position(|&b| is_address(b)).unwrap_or(self.buf.len());
            self.buf.drain(..start);
            if self.buf.len() < 2 {
                break;
            }
            let len = self.buf[1] as usize;
            if !(2..=62).contains(&len) {
                self.buf.drain(..1);
                continue;
            }
            if self.buf.len() < len + 2 {
                break;
            }
            let body = &self.buf[2..len + 1];
            if crc8(body) == self.buf[len + 1] {
                out.push(Frame { addr: self.buf[0], kind: body[0], payload: body[1..].to_vec() });
                self.good += 1;
                self.buf.drain(..len + 2);
            } else {
                self.bad += 1;
                self.buf.drain(..1);
            }
        }
        out
    }
}

/// Microseconds to the 11-bit value ExpressLRS sends (988 us is 172, 2012 us is 1811).
pub fn us_to_ticks(us: u16) -> u16 {
    (((us as i32 - 1500) * 8 / 5) + 992).clamp(0, 2047) as u16
}

/// The 11-bit value to microseconds.
pub fn ticks_to_us(t: u16) -> u16 {
    (((t as i32 - 992) * 5 / 8) + 1500) as u16
}

/// The payload of an RC channels frame: 16 channels of 11 bits, least significant bit first.
pub fn pack_channels(us: &[u16; 16]) -> [u8; 22] {
    let mut out = [0u8; 22];
    let mut bit = 0;
    for &c in us {
        let t = us_to_ticks(c) as u32;
        for i in 0..11 {
            if t >> i & 1 == 1 {
                out[bit / 8] |= 1 << (bit % 8);
            }
            bit += 1;
        }
    }
    out
}

pub fn unpack_channels(p: &[u8]) -> Option<[u16; 16]> {
    if p.len() < 22 {
        return None;
    }
    let mut ch = [0u16; 16];
    let mut bit = 0;
    for c in ch.iter_mut() {
        let mut t = 0u16;
        for i in 0..11 {
            if p[bit / 8] >> (bit % 8) & 1 == 1 {
                t |= 1 << i;
            }
            bit += 1;
        }
        *c = ticks_to_us(t);
    }
    Some(ch)
}

pub fn rc_frame(us: &[u16; 16]) -> Vec<u8> {
    encode(ADDR_FC, TYPE_RC_CHANNELS, &pack_channels(us))
}

/// What the link reports about itself (from the receiver).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LinkStats {
    /// Uplink signal strength as a positive number of -dBm.
    pub rssi: u8,
    /// Uplink link quality, percent.
    pub quality: u8,
    pub snr: i8,
    pub rf_mode: u8,
    pub tx_power_mw_index: u8,
}

pub fn link_stats_frame(s: &LinkStats) -> Vec<u8> {
    let p = [s.rssi, s.rssi, s.quality, s.snr as u8, 0, s.rf_mode, s.tx_power_mw_index, s.rssi, s.quality, s.snr as u8];
    encode(ADDR_TX, TYPE_LINK_STATS, &p)
}

pub fn parse_link_stats(p: &[u8]) -> Option<LinkStats> {
    (p.len() >= 10).then(|| LinkStats { rssi: p[0], quality: p[2], snr: p[3] as i8, rf_mode: p[5], tx_power_mw_index: p[6] })
}

pub fn battery_frame(volts: f32, amps: f32, used_mah: u32, percent: u8) -> Vec<u8> {
    let v = ((volts * 10.0).round() as u16).to_be_bytes();
    let a = ((amps * 10.0).round() as u16).to_be_bytes();
    let m = used_mah.min(0xFF_FFFF).to_be_bytes();
    encode(ADDR_TX, TYPE_BATTERY, &[v[0], v[1], a[0], a[1], m[1], m[2], m[3], percent])
}

pub fn parse_battery(p: &[u8]) -> Option<(f32, f32, u32, u8)> {
    (p.len() >= 8).then(|| {
        (
            u16::from_be_bytes([p[0], p[1]]) as f32 / 10.0,
            u16::from_be_bytes([p[2], p[3]]) as f32 / 10.0,
            u32::from_be_bytes([0, p[4], p[5], p[6]]),
            p[7],
        )
    })
}

/// Attitude in radians (pitch, roll, yaw), sent as tenths of a milliradian.
pub fn attitude_frame(pitch: f32, roll: f32, yaw: f32) -> Vec<u8> {
    let t = |r: f32| ((r * 10_000.0).round().clamp(-32768.0, 32767.0) as i16).to_be_bytes();
    let (p, r, y) = (t(pitch), t(roll), t(yaw));
    encode(ADDR_TX, TYPE_ATTITUDE, &[p[0], p[1], r[0], r[1], y[0], y[1]])
}

pub fn parse_attitude(p: &[u8]) -> Option<(f32, f32, f32)> {
    let f = |i: usize| i16::from_be_bytes([p[i], p[i + 1]]) as f32 / 10_000.0;
    (p.len() >= 6).then(|| (f(0), f(2), f(4)))
}

/// The flight mode, as text.
pub fn flight_mode_frame(text: &str) -> Vec<u8> {
    let mut p: Vec<u8> = text.bytes().take(MAX_PAYLOAD - 1).collect();
    p.push(0);
    encode(ADDR_TX, TYPE_FLIGHT_MODE, &p)
}

pub fn parse_flight_mode(p: &[u8]) -> String {
    let end = p.iter().position(|&b| b == 0).unwrap_or(p.len());
    String::from_utf8_lossy(&p[..end]).into_owned()
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Gps {
    pub lat: f64,
    pub lon: f64,
    /// Ground speed, m/s.
    pub speed: f32,
    /// Course, degrees.
    pub heading: f32,
    /// Altitude above sea level, metres.
    pub alt: f32,
    pub sats: u8,
}

pub fn gps_frame(g: &Gps) -> Vec<u8> {
    let mut p = Vec::with_capacity(15);
    p.extend_from_slice(&((g.lat * 1e7).round() as i32).to_be_bytes());
    p.extend_from_slice(&((g.lon * 1e7).round() as i32).to_be_bytes());
    p.extend_from_slice(&((g.speed * 36.0).round().clamp(0.0, 65535.0) as u16).to_be_bytes());
    p.extend_from_slice(&((g.heading.rem_euclid(360.0) * 100.0).round() as u16).to_be_bytes());
    p.extend_from_slice(&((g.alt + 1000.0).round().clamp(0.0, 65535.0) as u16).to_be_bytes());
    p.push(g.sats);
    encode(ADDR_TX, TYPE_GPS, &p)
}

pub fn parse_gps(p: &[u8]) -> Option<Gps> {
    (p.len() >= 15).then(|| Gps {
        lat: i32::from_be_bytes([p[0], p[1], p[2], p[3]]) as f64 / 1e7,
        lon: i32::from_be_bytes([p[4], p[5], p[6], p[7]]) as f64 / 1e7,
        speed: u16::from_be_bytes([p[8], p[9]]) as f32 / 36.0,
        heading: u16::from_be_bytes([p[10], p[11]]) as f32 / 100.0,
        alt: u16::from_be_bytes([p[12], p[13]]) as f32 - 1000.0,
        sats: p[14],
    })
}

/// Vertical speed in m/s, sent as decimetres a second.
pub fn vario_frame(climb: f32) -> Vec<u8> {
    encode(ADDR_TX, TYPE_VARIO, &((climb * 10.0).round().clamp(-32768.0, 32767.0) as i16).to_be_bytes())
}

/// Airspeed in m/s, sent as tenths of a km/h.
pub fn airspeed_frame(ms: f32) -> Vec<u8> {
    encode(ADDR_TX, TYPE_AIRSPEED, &((ms * 36.0).round().clamp(0.0, 65535.0) as u16).to_be_bytes())
}

/// MAVLink bytes as envelope frames. Each one holds the number of the chunk, the size of the
/// bytes and the bytes themselves (up to [`MAX_ENVELOPE_DATA`]).
pub fn envelopes(addr: u8, bytes: &[u8]) -> Vec<Vec<u8>> {
    let chunks: Vec<&[u8]> = bytes.chunks(MAX_ENVELOPE_DATA).collect();
    let n = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut p = vec![((n - i - 1) as u8) << 4 | (i as u8 & 0x0F), c.len() as u8];
            p.extend_from_slice(c);
            encode(addr, TYPE_MAVLINK_ENVELOPE, &p)
        })
        .collect()
}

/// The MAVLink bytes in an envelope's payload.
pub fn envelope_data(p: &[u8]) -> &[u8] {
    match p {
        [_, len, rest @ ..] => &rest[..(*len as usize).min(rest.len())],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksum_matches_the_known_value() {
        // The RC channels frame of ExpressLRS with every channel at the middle (992) is well known.
        assert_eq!(crc8(&[]), 0);
        assert_eq!(crc8(&[0x01]), 0xD5);
        assert_eq!(crc8(&[0x16, 0x00]), crc8(&[0x16, 0x00]));
    }

    #[test]
    fn channels_survive_the_packing() {
        let mut us = [1500u16; 16];
        us[0] = 1000;
        us[2] = 2000;
        us[7] = 1263;
        us[15] = 1900;
        let back = unpack_channels(&pack_channels(&us)).unwrap();
        for (a, b) in us.iter().zip(back.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1, "{a} {b}");
        }
    }

    #[test]
    fn the_middle_is_992_ticks() {
        assert_eq!(us_to_ticks(1500), 992);
        assert_eq!(us_to_ticks(1000), 192);
        assert_eq!(ticks_to_us(992), 1500);
    }

    #[test]
    fn a_stream_in_pieces_gives_whole_frames() {
        let a = rc_frame(&[1500; 16]);
        let b = flight_mode_frame("AUTO");
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        let mut f = Framer::default();
        let mut got = f.push(&stream[..10]);
        got.extend(f.push(&stream[10..]));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, TYPE_RC_CHANNELS);
        assert_eq!(parse_flight_mode(&got[1].payload), "AUTO");
    }

    #[test]
    fn a_damaged_frame_is_dropped_and_counted() {
        let mut bad = rc_frame(&[1700; 16]);
        bad[6] ^= 0x40;
        let good = rc_frame(&[1500; 16]);
        let mut f = Framer::default();
        let mut stream = bad;
        stream.extend_from_slice(&good);
        let got = f.push(&stream);
        assert_eq!(got.len(), 1);
        assert_eq!(unpack_channels(&got[0].payload).unwrap()[0], 1500);
        assert!(f.bad >= 1);
    }

    #[test]
    fn telemetry_frames_read_back() {
        let f = Framer::default().push(&battery_frame(11.1, 4.5, 250, 80));
        assert_eq!(parse_battery(&f[0].payload), Some((11.1, 4.5, 250, 80)));
        let f = Framer::default().push(&attitude_frame(0.1, -0.5, 3.0));
        let (p, r, y) = parse_attitude(&f[0].payload).unwrap();
        assert!((p - 0.1).abs() < 1e-3 && (r + 0.5).abs() < 1e-3 && (y - 3.0).abs() < 1e-3);
        let g = Gps { lat: -35.363261, lon: 149.16523, speed: 18.0, heading: 353.0, alt: 584.0, sats: 12 };
        let f = Framer::default().push(&gps_frame(&g));
        let back = parse_gps(&f[0].payload).unwrap();
        assert!((back.lat - g.lat).abs() < 1e-6 && (back.alt - 584.0).abs() < 1.0 && back.sats == 12);
        assert!((back.speed - 18.0).abs() < 0.1 && (back.heading - 353.0).abs() < 0.1);
    }

    #[test]
    fn mavlink_bytes_go_in_envelopes_and_come_out_whole() {
        let bytes: Vec<u8> = (0..200u32).map(|i| i as u8).collect();
        let frames = envelopes(ADDR_TX, &bytes);
        assert!(frames.len() >= 4);
        assert!(frames.iter().all(|f| f.len() <= 64));
        let mut f = Framer::default();
        let mut got = vec![];
        for fr in &frames {
            for x in f.push(fr) {
                assert_eq!(x.kind, TYPE_MAVLINK_ENVELOPE);
                got.extend_from_slice(envelope_data(&x.payload));
            }
        }
        assert_eq!(got, bytes);
    }
}
