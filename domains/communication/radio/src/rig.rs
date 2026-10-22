//! The whole chain, from another aircraft to the station's map.

use crate::downlink::Downlink;
use crate::relay::Relay;
use communication_air::{Air, Emitter, Listener, Technology};
use communication_remoteid as rid;
use communication_telemetry::{
    AUTOPILOT_COMPONENT, Beacon, Decoder, MavMessage, ODID_COMPONENT, forward_remote_id, frame, global_position,
    heartbeat, remote_id_location,
};
use communication_traffic::{Identity, Position, Source, Track, Tracker};
use mavlink::dialects::common::*;
use std::collections::VecDeque;

/// What the other plane transmits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sending {
    Silent,
    RemoteId,
    Mavlink,
    Both,
}

/// The pilot box, where the pilot's plane is.
pub const FIELD: Position = Position { lat: 37.8701, lon: -122.2723, alt: 50.0 };
/// The MAVLink system ID of the pilot's plane.
pub const OWN_SYSID: u8 = 1;
/// The MAVLink system ID of the other plane.
pub const OTHER_SYSID: u8 = 2;
/// The address that the other plane's Remote ID comes from.
pub const OTHER_MAC: [u8; 6] = [0x60, 0, 0, 0, 0, OTHER_SYSID];

const STEP: f64 = 0.05;
const FIRST_CROWD_SYSID: u8 = 10;

/// A rate of one frame in `every` seconds.
struct Every {
    every: f64,
    next: f64,
}

impl Every {
    fn new(every: f64) -> Every {
        Every { every, next: 0.0 }
    }

    fn due(&mut self, now: f64) -> bool {
        if now >= self.next {
            self.next += self.every;
            true
        } else {
            false
        }
    }
}

/// The pilot's plane and the station, with one other plane and a crowd.
pub struct Rig {
    now: f64,
    air: Air,
    /// The other plane first, then the crowd.
    emitters: Vec<Emitter>,
    distance: f32,
    sending: Sending,
    own_remote_id: bool,
    relay: Relay,
    decoder: Decoder,
    tracker: Tracker,
    seq: u8,
    own_timers: [Every; 7],
    own_remote_id_timer: Every,
    own_positions: VecDeque<f64>,
}

fn radios(s: Sending) -> Vec<Technology> {
    match s {
        Sending::Silent => vec![],
        Sending::RemoteId => vec![Technology::Bluetooth5],
        Sending::Mavlink => vec![Technology::MavlinkNetwork],
        Sending::Both => vec![Technology::Bluetooth5, Technology::MavlinkNetwork],
    }
}

fn beacon(sysid: u8, position: Position) -> Beacon {
    Beacon {
        sysid,
        position,
        velocity: [0.0; 3],
        heading: 90.0,
        uas_id: format!("T1S{sysid:04}"),
        mac: [0x60, 0, 0, 0, 0, sysid],
    }
}

impl Default for Rig {
    fn default() -> Self {
        Rig::new()
    }
}

impl Rig {
    /// The pilot's plane at the field with its Remote ID on, the link up, and nobody else.
    pub fn new() -> Rig {
        let other = Emitter::new(beacon(OTHER_SYSID, FIELD.moved(0.0, 200.0, 0.0)), vec![]);
        Rig {
            now: 0.0,
            air: Air::new(1),
            emitters: vec![other],
            distance: 200.0,
            sending: Sending::Silent,
            own_remote_id: true,
            relay: Relay::new(OWN_SYSID, Downlink::DEFAULT_RATE),
            decoder: Decoder::new(OWN_SYSID),
            tracker: Tracker::new(),
            seq: 0,
            // The rates the station asks the autopilot for: heartbeat, position, attitude, speed, status, GPS, navigation.
            own_timers: [
                Every::new(1.0),
                Every::new(0.1),
                Every::new(0.05),
                Every::new(0.1),
                Every::new(0.25),
                Every::new(0.25),
                Every::new(1.0),
            ],
            own_remote_id_timer: Every::new(1.0),
            own_positions: VecDeque::new(),
        }
    }

    pub fn set_sending(&mut self, s: Sending) {
        self.sending = s;
        self.emitters[0].radios = radios(s);
    }

    /// Puts the other plane this far east of the pilot's plane (m).
    pub fn set_distance(&mut self, m: f32) {
        self.distance = m;
        self.emitters[0].beacon.position = FIELD.moved(0.0, m, 0.0);
    }

    /// Puts this many more planes on the MAVLink network, round the field.
    pub fn set_crowd(&mut self, n: usize) {
        self.emitters.truncate(1);
        for i in 0..n {
            let bearing = (i as f32 * 37.0).to_radians();
            let range = 300.0 + 40.0 * i as f32;
            let at = FIELD.moved(range * bearing.cos(), range * bearing.sin(), 30.0);
            self.emitters.push(Emitter::new(beacon(FIRST_CROWD_SYSID + i as u8, at), vec![Technology::MavlinkNetwork]));
        }
    }

    /// The link between the plane and the station: up or lost.
    pub fn set_link(&mut self, up: bool) {
        self.relay.downlink.set_up(up);
    }

    pub fn set_own_remote_id(&mut self, on: bool) {
        self.own_remote_id = on;
    }

    pub fn sending(&self) -> Sending {
        self.sending
    }

    pub fn distance(&self) -> f32 {
        self.distance
    }

    pub fn crowd(&self) -> usize {
        self.emitters.len() - 1
    }

    pub fn link_up(&self) -> bool {
        self.relay.downlink.is_up()
    }

    pub fn own_remote_id(&self) -> bool {
        self.own_remote_id
    }

    pub fn now(&self) -> f64 {
        self.now
    }

    pub fn downlink(&self) -> &Downlink {
        &self.relay.downlink
    }

    /// Lets time pass.
    pub fn run(&mut self, seconds: f64) {
        let steps = (seconds / STEP).round() as usize;
        for _ in 0..steps {
            self.step();
        }
    }

    fn next_seq(&mut self) -> u8 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn own_beacon(&self) -> Beacon {
        beacon(OWN_SYSID, FIELD)
    }

    fn step(&mut self) {
        let now = self.now;
        let own = self.own_beacon();

        // The plane's own telemetry, at the rates the station asks for.
        let messages: [MavMessage; 7] = [
            heartbeat(&own),
            global_position(&own, (now * 1000.0) as u32),
            MavMessage::ATTITUDE(ATTITUDE_DATA {
                time_boot_ms: (now * 1000.0) as u32,
                roll: 0.1,
                pitch: 0.05,
                yaw: 1.5,
                rollspeed: 0.01,
                pitchspeed: 0.02,
                yawspeed: 0.03,
            }),
            MavMessage::VFR_HUD(VFR_HUD_DATA {
                airspeed: 18.0,
                groundspeed: 17.0,
                heading: 90,
                throttle: 55,
                alt: 80.0,
                climb: 0.5,
            }),
            MavMessage::SYS_STATUS(SYS_STATUS_DATA {
                voltage_battery: 15_400,
                current_battery: 1200,
                battery_remaining: 80,
                ..Default::default()
            }),
            MavMessage::GPS_RAW_INT(GPS_RAW_INT_DATA {
                time_usec: (now * 1e6) as u64,
                fix_type: GpsFixType::GPS_FIX_TYPE_3D_FIX,
                lat: 378_701_000,
                lon: -1_222_723_000,
                alt: 50_000,
                eph: 90,
                epv: 120,
                vel: 1700,
                cog: 9000,
                satellites_visible: 14,
                ..Default::default()
            }),
            MavMessage::NAV_CONTROLLER_OUTPUT(NAV_CONTROLLER_OUTPUT_DATA {
                nav_bearing: 90,
                target_bearing: 90,
                wp_dist: 250,
                ..Default::default()
            }),
        ];
        for (i, message) in messages.iter().enumerate() {
            if self.own_timers[i].due(now) {
                let seq = self.next_seq();
                let f = frame(OWN_SYSID, AUTOPILOT_COMPONENT, seq, message);
                self.relay.from_plane(&f, now);
            }
        }
        // The Remote ID transmitter on the plane tells the autopilot what it broadcasts. No sender address: it is the plane's own.
        if self.own_remote_id && self.own_remote_id_timer.due(now) {
            let pack = rid::encode_pack(&[remote_id_location(&own, (now % 3600.0) as f32)]);
            for m in forward_remote_id([0; 6], &pack) {
                let seq = self.next_seq();
                let f = frame(OWN_SYSID, ODID_COMPONENT, seq, &m);
                self.relay.from_plane(&f, now);
            }
        }

        // The other planes transmit; the receivers on the pilot's plane hear what is in range.
        let listener = Listener {
            position: FIELD,
            hears: vec![
                Technology::MavlinkNetwork,
                Technology::Bluetooth4,
                Technology::Bluetooth5,
                Technology::WifiNan,
                Technology::Adsb,
            ],
        };
        let heard = self.air.step(STEP, &mut self.emitters, &[listener]);
        self.relay.pass_on(heard, now);

        // The radio, and the station behind it.
        let bytes = self.relay.step(STEP, now);
        let heard = self.decoder.push(&bytes, now);
        for o in heard.traffic {
            self.tracker.observe(o);
        }
        for (_, m) in heard.own {
            if matches!(m, MavMessage::GLOBAL_POSITION_INT(_)) {
                self.own_positions.push_back(now);
            }
        }
        while self.own_positions.front().is_some_and(|t| now - t > 5.0) {
            self.own_positions.pop_front();
        }
        self.now += STEP;
    }

    /// Everything on the station's map.
    pub fn tracks(&self) -> Vec<Track> {
        self.tracker.tracks(self.now)
    }

    /// The marker of the other plane.
    pub fn other(&self) -> Option<Track> {
        let mav = Identity::new(Source::Mavlink, OTHER_SYSID.to_string());
        let rid =
            Identity::new(Source::RemoteId, OTHER_MAC.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"));
        self.tracks().into_iter().find(|t| t.identities.contains(&mav) || t.identities.contains(&rid))
    }

    /// How many times a second the station got the position of the pilot's plane (over the last 5 s).
    pub fn own_position_rate(&self) -> f32 {
        self.own_positions.len() as f32 / 5.0
    }
}
