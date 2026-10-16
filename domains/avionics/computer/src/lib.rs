//! The simulated flight computer, and the TX15's end of the serial line.
//!
//! The flight computer stands between the TX15 and ArduPilot:
//!
//! - **Up.** CRSF channels from the TX15 become the radio channels ArduPilot reads
//!   ([`FlightComputer::channels`]). If the line goes quiet, it fails safe like a receiver that
//!   lost its link: the throttle channel goes below ArduPilot's failsafe level. MAVLink from
//!   the ground station, in envelopes, goes on to ArduPilot ([`FlightComputer::take_for_autopilot`]).
//! - **Down.** The state of the plane becomes CRSF telemetry for the TX15's screen (attitude,
//!   battery, GPS, flight mode, link statistics). ArduPilot's MAVLink goes down in envelopes, in
//!   whatever room the narrow line has left, so the ground station's telemetry comes from here.
//!
//! Time is passed in (seconds), so the sim and the tests drive it the same way.

mod handset;

mod screen;
pub use handset::{Handset, Message, Screen, StatusTexts};

use avionics_crsf as crsf;
use std::collections::VecDeque;

/// The pulse the throttle channel shows with no link: below ArduPilot's `THR_FS_VALUE` (950).
pub const FAILSAFE_THROTTLE: u16 = 880;
/// A line that is quiet for this long (seconds) has lost the link.
pub const LINK_TIMEOUT: f64 = 0.5;
/// How many channel frames a second the TX15 sends (ExpressLRS 150 Hz).
pub const RC_RATE: f64 = 150.0;
/// The bytes a second that MAVLink may use on the line, besides the telemetry frames.
pub const MAVLINK_RATE: f32 = 3000.0;
/// The most MAVLink that waits to go down. Older telemetry is dropped.
const MAX_WAITING: usize = 6000;

/// What the plane reports to the flight computer each time it sends telemetry.
#[derive(Clone, Debug, Default)]
pub struct Vehicle {
    pub armed: bool,
    pub mode: String,
    pub volts: f32,
    pub amps: f32,
    pub used_mah: u32,
    pub percent: u8,
    /// Radians.
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub lat: f64,
    pub lon: f64,
    /// Metres above sea level.
    pub alt: f32,
    /// Metres a second.
    pub ground_speed: f32,
    pub airspeed: f32,
    pub climb: f32,
    /// Degrees.
    pub heading: f32,
    pub sats: u8,
}

#[derive(Default)]
struct Timers {
    attitude: f64,
    battery: f64,
    gps: f64,
    vario: f64,
    mode: f64,
    link: f64,
}

pub struct FlightComputer {
    framer: crsf::Framer,
    channels: [u16; 16],
    last_rc: Option<f64>,
    window_start: f64,
    window_good: u32,
    quality: u8,
    to_autopilot: Vec<u8>,
    /// MAVLink that the radio forwarded up for the ground station.
    to_ground_station: Vec<u8>,
    /// MAVLink from the ground station, for the radio to carry up to the plane.
    from_gs: VecDeque<u8>,
    waiting: VecDeque<u8>,
    budget: f32,
    last_send: f64,
    mavlink_rate: f32,
    timers: Timers,
}

impl FlightComputer {
    pub fn new() -> FlightComputer {
        FlightComputer::with_mavlink_rate(MAVLINK_RATE)
    }

    pub fn with_mavlink_rate(mavlink_rate: f32) -> FlightComputer {
        FlightComputer {
            framer: crsf::Framer::default(),
            channels: failsafe_channels(),
            last_rc: None,
            window_start: 0.0,
            window_good: 0,
            quality: 0,
            to_autopilot: vec![],
            to_ground_station: vec![],
            from_gs: VecDeque::new(),
            waiting: VecDeque::new(),
            budget: 0.0,
            last_send: 0.0,
            mavlink_rate,
            timers: Timers::default(),
        }
    }

    /// Bytes that came up the line from the TX15.
    pub fn from_line(&mut self, bytes: &[u8], now: f64) {
        for f in self.framer.push(bytes) {
            match f.kind {
                crsf::TYPE_RC_CHANNELS => {
                    if let Some(ch) = crsf::unpack_channels(&f.payload) {
                        self.channels = ch;
                        self.last_rc = Some(now);
                        self.window_good += 1;
                    }
                }
                // From the radio itself: what it carries for the ground station. From the radio's
                // module: the ground station's commands, for the autopilot.
                crsf::TYPE_MAVLINK_ENVELOPE if f.addr == crsf::ADDR_RADIO => {
                    self.to_ground_station.extend_from_slice(crsf::envelope_data(&f.payload))
                }
                crsf::TYPE_MAVLINK_ENVELOPE => self.to_autopilot.extend_from_slice(crsf::envelope_data(&f.payload)),
                _ => {}
            }
        }
    }

    pub fn link_up(&self, now: f64) -> bool {
        self.last_rc.is_some_and(|t| now - t <= LINK_TIMEOUT)
    }

    /// The radio channels for ArduPilot, in microseconds. Without a link: the failsafe.
    pub fn channels(&self, now: f64) -> [u16; 16] {
        if self.link_up(now) { self.channels } else { failsafe_channels() }
    }

    /// The uplink quality in percent.
    pub fn quality(&self, now: f64) -> u8 {
        if self.link_up(now) { self.quality } else { 0 }
    }

    /// MAVLink from ArduPilot, to go down the line when there is room.
    pub fn from_autopilot(&mut self, mavlink: &[u8]) {
        self.waiting.extend(mavlink);
        while self.waiting.len() > MAX_WAITING {
            self.waiting.pop_front();
        }
    }

    /// MAVLink that the TX15 sent up, for ArduPilot.
    pub fn take_for_autopilot(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.to_autopilot)
    }

    /// MAVLink that the radio forwarded up, for the ground station.
    pub fn take_for_ground_station(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.to_ground_station)
    }

    /// MAVLink from the ground station: it goes down to the radio, which carries it up to the plane.
    pub fn from_ground_station(&mut self, mavlink: &[u8]) {
        self.from_gs.extend(mavlink);
        while self.from_gs.len() > MAX_WAITING {
            self.from_gs.pop_front();
        }
    }

    /// The bytes to put on the line now. Nothing goes while the link is down. The telemetry
    /// frames go first, and MAVLink uses the room that is left.
    pub fn to_line(&mut self, v: &Vehicle, now: f64) -> Vec<u8> {
        // Link quality, once a second, from the channel frames that came through.
        if now - self.window_start >= 1.0 {
            let span = now - self.window_start;
            let expected = RC_RATE * span;
            let got = self.window_good as f64;
            self.quality = if got + 2.0 >= expected { 100 } else { (got * 100.0 / expected).round() as u8 };
            self.window_start = now;
            self.window_good = 0;
        }
        let dt = (now - self.last_send).max(0.0) as f32;
        self.last_send = now;
        if !self.link_up(now) {
            return vec![];
        }
        self.budget = (self.budget + self.mavlink_rate * dt).min(self.mavlink_rate);
        let mut out = vec![];
        let due = |t: &mut f64, period: f64| {
            if now >= *t {
                *t = now + period;
                true
            } else {
                false
            }
        };
        if due(&mut self.timers.attitude, 0.1) {
            out.extend(crsf::attitude_frame(v.pitch, v.roll, v.yaw));
        }
        if due(&mut self.timers.battery, 0.2) {
            out.extend(crsf::battery_frame(v.volts, v.amps, v.used_mah, v.percent));
        }
        if due(&mut self.timers.gps, 0.2) {
            out.extend(crsf::gps_frame(&crsf::Gps {
                lat: v.lat,
                lon: v.lon,
                speed: v.ground_speed,
                heading: v.heading,
                alt: v.alt,
                sats: v.sats,
            }));
            out.extend(crsf::airspeed_frame(v.airspeed));
        }
        if due(&mut self.timers.vario, 0.2) {
            out.extend(crsf::vario_frame(v.climb));
        }
        if due(&mut self.timers.mode, 0.5) {
            // ArduPilot marks a disarmed plane with a star after the mode.
            out.extend(crsf::flight_mode_frame(&format!("{}{}", v.mode, if v.armed { "" } else { "*" })));
        }
        if due(&mut self.timers.link, 0.5) {
            out.extend(crsf::link_stats_frame(&crsf::LinkStats { rssi: 60, quality: self.quality, snr: 10, rf_mode: 6, tx_power_mw_index: 2 }));
        }
        // The ground station's commands first (they are few), then ArduPilot's MAVLink, in the
        // room that is left.
        for (queue, addr) in [(&mut self.from_gs, crsf::ADDR_RADIO), (&mut self.waiting, crsf::ADDR_TX)] {
            while !queue.is_empty() {
                let n = queue.len().min(crsf::MAX_ENVELOPE_DATA);
                if self.budget < (n + 5) as f32 {
                    break;
                }
                let chunk: Vec<u8> = queue.drain(..n).collect();
                self.budget -= (n + 5) as f32;
                for f in crsf::envelopes(addr, &chunk) {
                    out.extend(f);
                }
            }
        }
        out
    }
}

impl Default for FlightComputer {
    fn default() -> Self {
        Self::new()
    }
}

fn failsafe_channels() -> [u16; 16] {
    let mut c = [1500u16; 16];
    c[2] = FAILSAFE_THROTTLE;
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sticks(roll: u16, throttle: u16) -> [u16; 16] {
        let mut c = [1500u16; 16];
        c[0] = roll;
        c[2] = throttle;
        c
    }

    #[test]
    fn channels_from_the_line_go_to_the_autopilot() {
        let mut fc = FlightComputer::new();
        fc.from_line(&crsf::rc_frame(&sticks(1800, 1200)), 1.0);
        let ch = fc.channels(1.1);
        assert!((ch[0] as i32 - 1800).abs() <= 1 && (ch[2] as i32 - 1200).abs() <= 1);
    }

    #[test]
    fn a_quiet_line_fails_safe() {
        let mut fc = FlightComputer::new();
        fc.from_line(&crsf::rc_frame(&sticks(1800, 1200)), 1.0);
        assert!(fc.link_up(1.4));
        assert!(!fc.link_up(1.6));
        assert_eq!(fc.channels(2.0)[2], FAILSAFE_THROTTLE);
        assert_eq!(fc.quality(2.0), 0);
    }

    #[test]
    fn before_the_tx15_talks_the_plane_is_in_failsafe() {
        let fc = FlightComputer::new();
        assert!(!fc.link_up(0.0));
        assert!(fc.channels(0.0)[2] < 950);
    }

    #[test]
    fn no_telemetry_goes_down_a_dead_line() {
        let mut fc = FlightComputer::new();
        assert!(fc.to_line(&Vehicle::default(), 1.0).is_empty());
    }

    #[test]
    fn telemetry_and_mavlink_share_the_line_within_its_rate() {
        let mut fc = FlightComputer::with_mavlink_rate(1000.0);
        let v = Vehicle { mode: "AUTO".into(), armed: true, volts: 11.1, ..Default::default() };
        let mut sent_envelope_bytes = 0;
        let mut framer = crsf::Framer::default();
        let mut t = 0.0;
        while t < 10.0 {
            fc.from_line(&crsf::rc_frame(&[1500; 16]), t);
            fc.from_autopilot(&[0xFD; 400]);
            for f in framer.push(&fc.to_line(&v, t)) {
                if f.kind == crsf::TYPE_MAVLINK_ENVELOPE {
                    sent_envelope_bytes += crsf::envelope_data(&f.payload).len();
                }
            }
            t += 1.0 / 150.0;
        }
        assert!(sent_envelope_bytes <= 11_500, "{sent_envelope_bytes}");
        assert!(sent_envelope_bytes >= 8_000, "{sent_envelope_bytes}");
    }

    /// The radio carries MAVLink for the ground station, which is on the other end of the same
    /// cable (a USB serial line): what it forwards up is addressed to the radio, not to the
    /// flight controller, and goes to the ground station.
    #[test]
    fn mavlink_that_the_radio_forwards_goes_to_the_ground_station_not_the_autopilot() {
        let mut fc = FlightComputer::new();
        for f in crsf::envelopes(crsf::ADDR_RADIO, b"for the ground station") {
            fc.from_line(&f, 0.0);
        }
        for f in crsf::envelopes(crsf::ADDR_FC, b"for the autopilot") {
            fc.from_line(&f, 0.0);
        }
        assert_eq!(fc.take_for_ground_station(), b"for the ground station");
        assert_eq!(fc.take_for_autopilot(), b"for the autopilot");
    }

    /// What the ground station sends goes down the line to the radio, which carries it up to the
    /// plane.
    #[test]
    fn what_the_ground_station_sends_goes_down_to_the_radio() {
        let mut fc = FlightComputer::new();
        fc.from_line(&crsf::rc_frame(&[1500; 16]), 0.0);
        fc.from_ground_station(b"a command");
        // The line has room once a little time has passed.
        let bytes = fc.to_line(&Vehicle::default(), 0.3);
        let mut got = vec![];
        for f in crsf::Framer::default().push(&bytes) {
            if f.kind == crsf::TYPE_MAVLINK_ENVELOPE && f.addr == crsf::ADDR_RADIO {
                got.extend_from_slice(crsf::envelope_data(&f.payload));
            }
        }
        assert_eq!(got, b"a command");
    }

    #[test]
    fn a_disarmed_plane_has_a_star_after_its_mode() {
        let mut fc = FlightComputer::new();
        fc.from_line(&crsf::rc_frame(&[1500; 16]), 0.0);
        let bytes = fc.to_line(&Vehicle { mode: "FBWA".into(), ..Default::default() }, 0.0);
        let modes: Vec<_> =
            crsf::Framer::default().push(&bytes).into_iter().filter(|f| f.kind == crsf::TYPE_FLIGHT_MODE).collect();
        assert_eq!(crsf::parse_flight_mode(&modes[0].payload), "FBWA*");
    }
}
