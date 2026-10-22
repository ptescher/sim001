//! Vehicle state as seen over MAVLink. Written by the link thread, read by the UI.

use communication_telemetry::Decoder;
use communication_traffic::{Track, Tracker};
use std::collections::VecDeque;
use std::time::Instant;

/// ArduPlane custom modes (HEARTBEAT.custom_mode).
pub const PLANE_MODES: &[(u32, &str)] = &[
    (0, "MANUAL"),
    (1, "CIRCLE"),
    (2, "STABILIZE"),
    (3, "TRAINING"),
    (4, "ACRO"),
    (5, "FBWA"),
    (6, "FBWB"),
    (7, "CRUISE"),
    (8, "AUTOTUNE"),
    (10, "AUTO"),
    (11, "RTL"),
    (12, "LOITER"),
    (13, "TAKEOFF"),
    (14, "AVOID_ADSB"),
    (15, "GUIDED"),
    (17, "QSTABILIZE"),
    (18, "QHOVER"),
    (19, "QLOITER"),
    (20, "QLAND"),
    (21, "QRTL"),
    (22, "QAUTOTUNE"),
    (23, "QACRO"),
    (24, "THERMAL"),
    (25, "LOITER2QLAND"),
];

pub fn mode_name(custom_mode: u32) -> &'static str {
    PLANE_MODES.iter().find(|(m, _)| *m == custom_mode).map_or("UNKNOWN", |(_, n)| n)
}

pub fn mode_number(name: &str) -> Option<u32> {
    PLANE_MODES.iter().find(|(_, n)| *n == name).map(|(m, _)| *m)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WpKind {
    Takeoff,
    Waypoint,
    LoiterUnlim,
    LoiterTurns,
    Land,
    Rtl,
    Other(u16),
}

impl WpKind {
    pub fn label(self) -> &'static str {
        match self {
            WpKind::Takeoff => "TKOF",
            WpKind::Waypoint => "WP",
            WpKind::LoiterUnlim => "LOIT",
            WpKind::LoiterTurns => "TURN",
            WpKind::Land => "LAND",
            WpKind::Rtl => "RTL",
            WpKind::Other(_) => "CMD",
        }
    }
}

/// A mission item in a form the UI can draw. Altitude is relative to home.
#[derive(Clone, Copy, Debug)]
pub struct Waypoint {
    pub kind: WpKind,
    pub lat: f64,
    pub lon: f64,
    pub alt: f32,
}

impl Waypoint {
    /// Items with a meaningful position (RTL and DO_ commands have none).
    pub fn has_position(&self) -> bool {
        !matches!(self.kind, WpKind::Rtl | WpKind::Other(_)) && (self.lat != 0.0 || self.lon != 0.0)
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub time: Instant,
    /// MAV_SEVERITY: 0 emergency .. 7 debug. GCS-originated lines use 6 (info) or 4 (warning).
    pub severity: u8,
    pub source: &'static str,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    /// Seconds since the GCS started.
    pub t: f32,
    pub alt: f32,
    pub airspeed: f32,
    pub groundspeed: f32,
    pub throttle: f32,
    pub voltage: f32,
}

#[derive(Default)]
pub struct LinkInfo {
    pub address: String,
    pub connected: bool,
    pub status: String,
    pub last_heartbeat: Option<Instant>,
    pub messages: u64,
    /// Messages per second, smoothed.
    pub rate: f32,
}

impl LinkInfo {
    /// Heartbeat seen within the last 3 s.
    pub fn alive(&self) -> bool {
        self.last_heartbeat.is_some_and(|t| t.elapsed().as_secs_f32() < 3.0)
    }
}

pub struct Vehicle {
    pub start: Instant,
    pub link: LinkInfo,
    pub sysid: u8,
    pub compid: u8,
    pub have_heartbeat: bool,
    pub armed: bool,
    pub custom_mode: u32,
    pub system_status: u8,

    // Attitude, radians / rad/s.
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub yaw_rate: f32,

    // Position.
    pub have_position: bool,
    pub lat: f64,
    pub lon: f64,
    pub alt_msl: f32,
    pub alt_rel: f32,
    pub vn: f32,
    pub ve: f32,
    pub vd: f32,
    pub heading: f32,

    // VFR_HUD.
    pub airspeed: f32,
    pub groundspeed: f32,
    pub climb: f32,
    pub throttle: u16,

    // GPS.
    pub gps_fix: u8,
    pub sats: u8,
    pub hdop: f32,

    // Power.
    pub batt_v: f32,
    pub batt_a: f32,
    pub batt_pct: i8,
    pub batt_mah: i32,
    /// LiPo cell count inferred from the highest voltage seen.
    pub batt_cells: u8,

    // Health.
    pub sensors_present: u32,
    pub sensors_enabled: u32,
    pub sensors_health: u32,
    pub ekf_flags: u16,
    pub ekf_vel_var: f32,
    pub ekf_pos_var: f32,
    pub ekf_hgt_var: f32,
    pub ekf_mag_var: f32,
    pub prearm: Option<(Instant, String)>,

    // Environment.
    pub wind_dir: f32,
    pub wind_speed: f32,
    pub wind_valid: bool,

    // Navigation.
    pub home: Option<(f64, f64, f32)>,
    pub nav_valid: bool,
    pub nav_bearing: f32,
    pub target_bearing: f32,
    pub wp_dist: f32,
    pub xtrack: f32,
    pub alt_error: f32,
    pub aspd_error: f32,
    pub nav_roll: f32,
    pub nav_pitch: f32,
    pub mission_seq: u16,
    pub mission: Vec<Waypoint>,
    pub mission_status: String,
    pub guided_target: Option<(f64, f64, f32)>,

    pub rc: [u16; 8],
    pub servo: [u16; 8],
    /// How fast the plane came down (m/s) when it last touched the ground, if it has.
    pub last_touchdown: Option<f32>,
    airborne: bool,
    fastest_sink: f32,

    pub log: VecDeque<LogLine>,
    /// Everything that happened between here and the autopilot (see `debug`), and the ping.
    pub debug: crate::debug::DebugLog,
    pub ping: crate::debug::PingStats,
    /// What the autopilot last answered to a command.
    pub acks: VecDeque<crate::debug::Ack>,
    /// Ground track, (lat, lon), roughly 2 Hz.
    pub trail: VecDeque<(f64, f64)>,
    pub history: VecDeque<Sample>,
    pub armed_since: Option<Instant>,
    pub last_flight_secs: f32,
    /// How many times the vehicle has armed since the console started (for a test that needs a
    /// plane that has not).
    pub arms: u32,

    /// Other aircraft, as the radio tells the station (see domains/communication).
    pub traffic: Tracker,
    pub traffic_decoder: Decoder,
}

impl Vehicle {
    pub fn new(address: &str) -> Self {
        Self {
            start: Instant::now(),
            link: LinkInfo { address: address.to_string(), status: "starting".into(), ..Default::default() },
            sysid: 1,
            compid: 1,
            have_heartbeat: false,
            armed: false,
            custom_mode: 0,
            system_status: 0,
            roll: 0.0,
            pitch: 0.0,
            yaw: 0.0,
            yaw_rate: 0.0,
            have_position: false,
            lat: 0.0,
            lon: 0.0,
            alt_msl: 0.0,
            alt_rel: 0.0,
            vn: 0.0,
            ve: 0.0,
            vd: 0.0,
            heading: 0.0,
            airspeed: 0.0,
            groundspeed: 0.0,
            climb: 0.0,
            throttle: 0,
            gps_fix: 0,
            sats: 0,
            hdop: 99.9,
            batt_v: 0.0,
            batt_a: 0.0,
            batt_pct: -1,
            batt_mah: 0,
            batt_cells: 0,
            sensors_present: 0,
            sensors_enabled: 0,
            sensors_health: 0,
            ekf_flags: 0,
            ekf_vel_var: 0.0,
            ekf_pos_var: 0.0,
            ekf_hgt_var: 0.0,
            ekf_mag_var: 0.0,
            prearm: None,
            wind_dir: 0.0,
            wind_speed: 0.0,
            wind_valid: false,
            home: None,
            nav_valid: false,
            nav_bearing: 0.0,
            target_bearing: 0.0,
            wp_dist: 0.0,
            xtrack: 0.0,
            alt_error: 0.0,
            aspd_error: 0.0,
            nav_roll: 0.0,
            nav_pitch: 0.0,
            mission_seq: 0,
            mission: Vec::new(),
            mission_status: String::new(),
            guided_target: None,
            rc: [0; 8],
            servo: [0; 8],
            last_touchdown: None,
            airborne: false,
            fastest_sink: 0.0,
            log: VecDeque::new(),
            debug: crate::debug::DebugLog::new(Instant::now()),
            ping: Default::default(),
            acks: VecDeque::new(),
            trail: VecDeque::new(),
            history: VecDeque::new(),
            armed_since: None,
            last_flight_secs: 0.0,
            arms: 0,
            traffic: Tracker::new(),
            traffic_decoder: Decoder::new(1),
        }
    }

    pub fn mode(&self) -> &'static str {
        if self.have_heartbeat { mode_name(self.custom_mode) } else { "----" }
    }

    /// Average cell voltage, if a battery is reported.
    pub fn cell_v(&self) -> Option<f32> {
        (self.batt_cells > 0 && self.batt_v > 0.5).then(|| self.batt_v / self.batt_cells as f32)
    }

    pub fn now(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    /// The other aircraft that the station knows of, as they are now.
    pub fn other_aircraft(&self) -> Vec<Track> {
        self.traffic.tracks(self.start.elapsed().as_secs_f64())
    }

    /// Takes in a message that is about other aircraft, or that other aircraft sent. Returns
    /// whether it was one.
    pub fn hear_traffic(
        &mut self,
        header: mavlink::MavHeader,
        msg: &mavlink::dialects::ardupilotmega::MavMessage,
    ) -> bool {
        use mavlink::dialects::ardupilotmega::MavMessage as M;
        let own = self.sysid;
        let about_others = match msg {
            // A peer's position. The plane's own position is not traffic.
            M::GLOBAL_POSITION_INT(_) => header.system_id != own,
            // What the receivers on the plane heard.
            M::ADSB_VEHICLE(_)
            | M::OPEN_DRONE_ID_BASIC_ID(_)
            | M::OPEN_DRONE_ID_LOCATION(_)
            | M::OPEN_DRONE_ID_MESSAGE_PACK(_) => true,
            _ => false,
        };
        if !about_others {
            return false;
        }
        // The decoder reads the common MAVLink dialect from bytes; these messages are the same in both.
        let mut bytes = Vec::with_capacity(300);
        if mavlink::write_v2_msg(&mut bytes, header, msg).is_ok() {
            let now = self.start.elapsed().as_secs_f64();
            self.traffic_decoder.set_own(own);
            for o in self.traffic_decoder.push(&bytes, now).traffic {
                self.traffic.observe(o);
            }
            self.traffic.prune(now);
        }
        true
    }

    pub fn flight_secs(&self) -> f32 {
        self.armed_since.map_or(self.last_flight_secs, |t| t.elapsed().as_secs_f32())
    }

    /// Notes what the autopilot answered to a command.
    pub fn record_ack(&mut self, command: &str, result: &str) {
        self.acks.push_back(crate::debug::Ack { at: Instant::now(), command: command.into(), result: result.into() });
        while self.acks.len() > crate::debug::ACKS_KEPT {
            self.acks.pop_front();
        }
    }

    pub fn push_log(&mut self, source: &'static str, severity: u8, text: impl Into<String>) {
        let text = text.into();
        // The debug log has every line, repeats and all.
        let (src, level) = if source == "FC" {
            (crate::debug::Source::Fc, crate::debug::Level::from_severity(severity))
        } else {
            (crate::debug::Source::Gcs, crate::debug::Level::from_severity(severity))
        };
        self.debug.record(level, src, text.clone());
        // ArduPilot repeats some messages (e.g. PreArm) every few seconds: collapse repeats.
        if let Some(last) = self.log.back_mut()
            && last.text == text
            && last.time.elapsed().as_secs_f32() < 30.0
        {
            last.time = Instant::now();
            return;
        }
        self.log.push_back(LogLine { time: Instant::now(), severity, source, text });
        while self.log.len() > 400 {
            self.log.pop_front();
        }
    }

    /// Sensor bit present+enabled and healthy? None if the sensor isn't present.
    pub fn sensor_ok(&self, bit: u32) -> Option<bool> {
        if self.sensors_present & bit == 0 || self.sensors_enabled & bit == 0 {
            return None;
        }
        Some(self.sensors_health & bit != 0)
    }

    /// Note the sink rate on the way down, and keep the one at touchdown: the fastest descent
    /// in the last 3 m above the ground. Call it with each position report.
    pub fn track_landing(&mut self) {
        if self.alt_rel > 3.0 {
            self.airborne = true;
            self.fastest_sink = 0.0;
        } else if self.airborne {
            self.fastest_sink = self.fastest_sink.max(self.vd);
            if self.alt_rel < 0.5 {
                self.last_touchdown = Some(self.fastest_sink);
                self.airborne = false;
            }
        }
    }

    pub fn record_sample(&mut self) {
        let t = self.now();
        if self.history.back().is_some_and(|s| t - s.t < 0.2) {
            return;
        }
        self.history.push_back(Sample {
            t,
            alt: self.alt_rel,
            airspeed: self.airspeed,
            groundspeed: self.groundspeed,
            throttle: self.throttle as f32,
            voltage: self.batt_v,
        });
        while self.history.front().is_some_and(|s| t - s.t > 600.0) {
            self.history.pop_front();
        }
    }

    pub fn record_trail(&mut self) {
        if !self.have_position {
            return;
        }
        let p = (self.lat, self.lon);
        if let Some(&(la, lo)) = self.trail.back() {
            // ~1.5 m minimum spacing.
            if ((la - p.0).abs() + (lo - p.1).abs()) < 1.4e-5 {
                return;
            }
        }
        self.trail.push_back(p);
        while self.trail.len() > 3000 {
            self.trail.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mavlink::MavHeader;
    use mavlink::dialects::ardupilotmega::*;

    fn position(lat: f64, lon: f64) -> MavMessage {
        MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
            time_boot_ms: 1000,
            lat: (lat * 1e7) as i32,
            lon: (lon * 1e7) as i32,
            alt: 80_000,
            relative_alt: 30_000,
            vx: 1000,
            vy: 0,
            vz: 0,
            hdg: 0,
        })
    }

    fn from(system_id: u8) -> MavHeader {
        MavHeader { system_id, component_id: 1, sequence: 0 }
    }

    fn report(v: &mut Vehicle, alt: f32, down: f32) {
        v.alt_rel = alt;
        v.vd = down;
        v.track_landing();
    }

    #[test]
    fn a_gentle_touchdown_is_noted_at_its_sink_rate() {
        let mut v = Vehicle::new("test");
        for (alt, down) in [(15.0, 1.5), (5.0, 1.5), (2.0, 1.0), (0.8, 0.6), (0.2, 0.4)] {
            report(&mut v, alt, down);
        }
        assert_eq!(v.last_touchdown, Some(1.0));
    }

    #[test]
    fn a_fast_descent_near_the_ground_is_a_hard_landing() {
        let mut v = Vehicle::new("test");
        for (alt, down) in [(30.0, 10.0), (2.0, 6.0), (0.2, 6.0)] {
            report(&mut v, alt, down);
        }
        assert_eq!(v.last_touchdown, Some(6.0));
    }

    #[test]
    fn a_plane_that_never_left_the_ground_has_not_landed() {
        let mut v = Vehicle::new("test");
        report(&mut v, 0.0, 0.0);
        assert_eq!(v.last_touchdown, None);
    }

    #[test]
    fn the_position_of_another_system_is_another_aircraft_and_not_the_vehicle() {
        let mut v = Vehicle::new("test");
        v.sysid = 1;
        assert!(v.hear_traffic(from(3), &position(37.87, -122.27)));
        let seen = v.other_aircraft();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].name, "MAV 3");
        assert!((seen[0].position.lat - 37.87).abs() < 1e-6);
        assert!(!v.have_position, "the vehicle's own position is untouched");
    }

    #[test]
    fn the_vehicles_own_position_is_not_traffic() {
        let mut v = Vehicle::new("test");
        v.sysid = 1;
        assert!(!v.hear_traffic(from(1), &position(37.87, -122.27)));
        assert!(v.other_aircraft().is_empty());
    }

    #[test]
    fn a_remote_id_receiver_on_the_plane_makes_traffic_and_the_planes_own_transmitter_does_not() {
        let mut v = Vehicle::new("test");
        v.sysid = 1;
        let heard = |mac: [u8; 20]| {
            MavMessage::OPEN_DRONE_ID_LOCATION(OPEN_DRONE_ID_LOCATION_DATA {
                id_or_mac: mac,
                latitude: 378_700_000,
                longitude: -1_222_700_000,
                altitude_geodetic: 80.0,
                direction: 9000,
                speed_horizontal: 1500,
                speed_vertical: 0,
                altitude_barometric: -1000.0,
                height: -1000.0,
                timestamp: 1.0,
                ..Default::default()
            })
        };
        let mut mac = [0u8; 20];
        mac[..6].copy_from_slice(&[0x60, 0, 0, 0, 0, 2]);
        assert!(v.hear_traffic(MavHeader { system_id: 1, component_id: 236, sequence: 0 }, &heard(mac)));
        assert!(v.hear_traffic(MavHeader { system_id: 1, component_id: 236, sequence: 1 }, &heard([0; 20])));
        let seen = v.other_aircraft();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(seen[0].source_label(), "RID");
    }

    #[test]
    fn what_the_vehicle_says_is_not_traffic() {
        let mut v = Vehicle::new("test");
        let hb = MavMessage::HEARTBEAT(HEARTBEAT_DATA::default());
        assert!(!v.hear_traffic(from(1), &hb));
    }
}
