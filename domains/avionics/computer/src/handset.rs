//! The TX15's end of the serial line: what the radio puts on it, and what it makes of what comes
//! back. The sim's TX15 stand-in and the acceptance tests use it.

use crate::RC_RATE;
use avionics_crsf as crsf;

/// What the radio's screen shows from the telemetry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screen {
    pub mode: String,
    pub armed: bool,
    pub volts: f32,
    pub amps: f32,
    /// Degrees.
    pub roll: f32,
    pub pitch: f32,
    pub quality: u8,
    pub gps: Option<crsf::Gps>,
    /// Seconds since the last frame of any kind came down the line.
    pub last_frame: Option<f64>,
}

pub struct Handset {
    /// The channels it sends, in microseconds.
    pub channels: [u16; 16],
    /// The share (0 to 1) of channel frames that it damages on the line.
    pub corrupt: f32,
    connected: bool,
    next: f64,
    acc: f32,
    framer: crsf::Framer,
    pub screen: Screen,
    from_plane: Vec<u8>,
    for_plane: Vec<u8>,
}

impl Handset {
    pub fn new() -> Handset {
        Handset {
            channels: [1500; 16],
            corrupt: 0.0,
            connected: true,
            next: 0.0,
            acc: 0.0,
            framer: crsf::Framer::default(),
            screen: Screen::default(),
            from_plane: vec![],
            for_plane: vec![],
        }
    }

    pub fn set_connected(&mut self, on: bool) {
        self.connected = on;
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    /// The uplink quality the screen shows: nothing from the plane for a second is 0.
    pub fn quality(&self, now: f64) -> u8 {
        match self.screen.last_frame {
            Some(t) if now - t < 1.0 => self.screen.quality,
            _ => 0,
        }
    }

    /// MAVLink from the ground station, to go up to the plane.
    pub fn send_mavlink(&mut self, bytes: &[u8]) {
        self.for_plane.extend_from_slice(bytes);
    }

    /// The MAVLink that came down, for the ground station.
    pub fn take_mavlink(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.from_plane)
    }

    /// What the radio puts on the line now: the channels at 150 Hz, and any MAVLink that waits.
    pub fn to_line(&mut self, now: f64) -> Vec<u8> {
        if !self.connected {
            return vec![];
        }
        let mut out = vec![];
        while now >= self.next {
            self.next = if self.next == 0.0 { now } else { self.next } + 1.0 / RC_RATE;
            let mut f = crsf::rc_frame(&self.channels);
            self.acc += self.corrupt;
            if self.acc >= 1.0 {
                self.acc -= 1.0;
                let at = f.len() - 3;
                f[at] ^= 0x55;
            }
            out.extend(f);
        }
        if !self.for_plane.is_empty() {
            let bytes = std::mem::take(&mut self.for_plane);
            for f in crsf::envelopes(crsf::ADDR_FC, &bytes) {
                out.extend(f);
            }
        }
        out
    }

    /// Bytes that came down the line from the flight computer.
    pub fn from_line(&mut self, bytes: &[u8], now: f64) {
        for f in self.framer.push(bytes) {
            self.screen.last_frame = Some(now);
            let p = &f.payload;
            match f.kind {
                crsf::TYPE_FLIGHT_MODE => {
                    let text = crsf::parse_flight_mode(p);
                    self.screen.armed = !text.ends_with('*');
                    self.screen.mode = text.trim_end_matches('*').to_string();
                }
                crsf::TYPE_BATTERY => {
                    if let Some((v, a, _, _)) = crsf::parse_battery(p) {
                        self.screen.volts = v;
                        self.screen.amps = a;
                    }
                }
                crsf::TYPE_ATTITUDE => {
                    if let Some((pitch, roll, _)) = crsf::parse_attitude(p) {
                        self.screen.pitch = pitch.to_degrees();
                        self.screen.roll = roll.to_degrees();
                    }
                }
                crsf::TYPE_GPS => self.screen.gps = crsf::parse_gps(p),
                crsf::TYPE_LINK_STATS => {
                    if let Some(s) = crsf::parse_link_stats(p) {
                        self.screen.quality = s.quality;
                    }
                }
                crsf::TYPE_MAVLINK_ENVELOPE => self.from_plane.extend_from_slice(crsf::envelope_data(p)),
                _ => {}
            }
        }
    }
}

impl Default for Handset {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_sends_150_channel_frames_a_second() {
        let mut h = Handset::new();
        let mut n = 0;
        let mut f = crsf::Framer::default();
        let mut t = 0.0;
        while t < 2.0 {
            n += f.push(&h.to_line(t)).len();
            t += 0.005;
        }
        assert!((295..=305).contains(&n), "{n}");
    }

    #[test]
    fn it_can_damage_a_share_of_its_frames() {
        let mut h = Handset::new();
        h.corrupt = 0.5;
        let mut f = crsf::Framer::default();
        let mut t = 0.0;
        while t < 2.0 {
            f.push(&h.to_line(t));
            t += 0.005;
        }
        assert!(f.bad >= 100, "{}", f.bad);
    }

    #[test]
    fn an_unplugged_radio_is_silent() {
        let mut h = Handset::new();
        h.set_connected(false);
        assert!(h.to_line(1.0).is_empty());
    }
}
