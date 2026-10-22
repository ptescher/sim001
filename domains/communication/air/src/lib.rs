//! The air between aircraft: who hears whom.
//!
//! An [`Emitter`] is a device that transmits. For each radio it has, it sends what that radio
//! sends, as real bytes: MAVLink v2 frames on the MAVLink network, and Remote ID messages (a
//! message pack on Bluetooth 5 and Wi-Fi, single messages on Bluetooth 4). A [`Listener`] is a
//! receiver somewhere, tuned to some radios. [`Air::step`] lets time pass and says which
//! listener heard which transmission. Range decides: a transmission is always heard close in,
//! sometimes heard further out, and never heard past the range of its radio.
//!
//! The ranges are those that small unmanned aircraft reach in the open with the antennas that
//! they carry. They are assumptions to tune, not measurements.

use communication_remoteid as rid;
use communication_telemetry::{
    AUTOPILOT_COMPONENT, Beacon, ODID_COMPONENT, adsb, frame, global_position, heartbeat, remote_id_location,
    remote_id_static,
};
use communication_traffic::Position;

/// A radio that carries what aircraft say about themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Technology {
    /// A telemetry radio on a shared MAVLink network (a 900 MHz multipoint radio or a mesh).
    MavlinkNetwork,
    /// Remote ID on Bluetooth 4 legacy advertising: one 25-byte message at a time.
    Bluetooth4,
    /// Remote ID on Bluetooth 5 Long Range: message packs.
    Bluetooth5,
    /// Remote ID on Wi-Fi Neighbor Awareness Networking: message packs.
    WifiNan,
    /// An ADS-B transponder, heard by an ADS-B receiver.
    Adsb,
}

impl Technology {
    /// Past this distance (m) nothing is heard.
    pub fn range(self) -> f32 {
        match self {
            Technology::MavlinkNetwork => 5000.0,
            Technology::Bluetooth4 => 300.0,
            Technology::Bluetooth5 => 1200.0,
            Technology::WifiNan => 800.0,
            Technology::Adsb => 40_000.0,
        }
    }

    /// Inside this distance (m) nearly everything is heard.
    pub fn clear_range(self) -> f32 {
        self.range() * 0.5
    }

    /// The chance that one transmission is heard at `distance` (m).
    pub fn receive_probability(self, distance: f32) -> f32 {
        const CLEAR: f32 = 0.98;
        if distance <= self.clear_range() {
            CLEAR
        } else if distance >= self.range() {
            0.0
        } else {
            CLEAR * (self.range() - distance) / (self.range() - self.clear_range())
        }
    }

    pub fn is_remote_id(self) -> bool {
        matches!(self, Technology::Bluetooth4 | Technology::Bluetooth5 | Technology::WifiNan)
    }
}

/// What a transmission holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    /// One MAVLink v2 frame.
    Mavlink(Vec<u8>),
    /// A Remote ID broadcast (one message or a pack), and the address it came from.
    RemoteId { mac: [u8; 6], bytes: Vec<u8> },
}

/// A device that transmits.
#[derive(Clone, Debug)]
pub struct Emitter {
    pub beacon: Beacon,
    /// The radios it transmits on. None means it is silent.
    pub radios: Vec<Technology>,
    /// The name its ADS-B transponder gives.
    pub callsign: String,
    seq: u8,
    next_heartbeat: f64,
    next_position: f64,
    next_remote_id: f64,
    next_remote_id_static: f64,
}

const HEARTBEAT_INTERVAL: f64 = 1.0;
const POSITION_INTERVAL: f64 = 0.5;
const REMOTE_ID_INTERVAL: f64 = 1.0;
const REMOTE_ID_STATIC_INTERVAL: f64 = 3.0;

impl Emitter {
    pub fn new(beacon: Beacon, radios: Vec<Technology>) -> Emitter {
        // Devices do not all start on the same beat.
        let phase = (beacon.sysid % 10) as f64 * 0.1;
        Emitter {
            callsign: format!("T1S{:03}", beacon.sysid),
            beacon,
            radios,
            seq: 0,
            next_heartbeat: phase,
            next_position: phase,
            next_remote_id: phase,
            next_remote_id_static: phase,
        }
    }

    /// Moves the device on at its velocity.
    pub fn fly(&mut self, dt: f64) {
        let [n, e, d] = self.beacon.velocity;
        self.beacon.position = self.beacon.position.moved(n * dt as f32, e * dt as f32, -d * dt as f32);
    }

    fn next_seq(&mut self) -> u8 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn remote_id_radio(&self) -> Option<Technology> {
        self.radios.iter().copied().find(|t| t.is_remote_id())
    }

    /// Everything that is due before `end`, with the time each was sent.
    fn due(&mut self, end: f64) -> Vec<(f64, Technology, Payload)> {
        let mut out = vec![];
        let radios = self.radios.clone();
        let b = self.beacon.clone();
        if radios.contains(&Technology::MavlinkNetwork) {
            while self.next_heartbeat < end {
                let t = self.next_heartbeat;
                self.next_heartbeat += HEARTBEAT_INTERVAL;
                let f = frame(b.sysid, AUTOPILOT_COMPONENT, self.next_seq(), &heartbeat(&b));
                out.push((t, Technology::MavlinkNetwork, Payload::Mavlink(f)));
            }
            while self.next_position < end {
                let t = self.next_position;
                self.next_position += POSITION_INTERVAL;
                let f = frame(b.sysid, AUTOPILOT_COMPONENT, self.next_seq(), &global_position(&b, (t * 1000.0) as u32));
                out.push((t, Technology::MavlinkNetwork, Payload::Mavlink(f)));
            }
        }
        if radios.contains(&Technology::Adsb) {
            // An ADS-B transponder sends its position twice a second, like the network.
            while self.next_position < end && !radios.contains(&Technology::MavlinkNetwork) {
                let t = self.next_position;
                self.next_position += POSITION_INTERVAL;
                let f = frame(
                    b.sysid,
                    AUTOPILOT_COMPONENT,
                    self.next_seq(),
                    &adsb(&b, 0xA00000 + b.sysid as u32, &self.callsign, 0),
                );
                out.push((t, Technology::Adsb, Payload::Mavlink(f)));
            }
        }
        if let Some(radio) = self.remote_id_radio() {
            let mut with_static = false;
            while self.next_remote_id_static < end {
                with_static = true;
                self.next_remote_id_static += REMOTE_ID_STATIC_INTERVAL;
            }
            while self.next_remote_id < end {
                let t = self.next_remote_id;
                self.next_remote_id += REMOTE_ID_INTERVAL;
                let location = remote_id_location(&b, (t % 3600.0) as f32);
                if radio == Technology::Bluetooth4 {
                    // Legacy advertising carries one message: the ID goes out in turn with the position.
                    if with_static {
                        out.push((
                            t,
                            radio,
                            Payload::RemoteId { mac: b.mac, bytes: rid::encode(&remote_id_static(&b)[0]).to_vec() },
                        ));
                    }
                    out.push((t, radio, Payload::RemoteId { mac: b.mac, bytes: rid::encode(&location).to_vec() }));
                } else {
                    let mut pack = remote_id_static(&b);
                    pack.push(location);
                    out.push((t, radio, Payload::RemoteId { mac: b.mac, bytes: rid::encode_pack(&pack) }));
                }
            }
        }
        out
    }
}

/// A receiver.
#[derive(Clone, Debug)]
pub struct Listener {
    pub position: Position,
    /// The radios it is tuned to.
    pub hears: Vec<Technology>,
}

/// One transmission that one listener heard.
#[derive(Clone, Debug, PartialEq)]
pub struct Reception {
    /// The index of the listener.
    pub listener: usize,
    /// The index of the emitter.
    pub emitter: usize,
    pub technology: Technology,
    /// When it was sent (s).
    pub time: f64,
    pub payload: Payload,
}

/// The air.
#[derive(Clone, Debug)]
pub struct Air {
    time: f64,
    rng: u64,
}

impl Air {
    pub fn new(seed: u64) -> Air {
        Air { time: 0.0, rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1 }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// A number in 0 to 1 (SplitMix64, so the same seed hears the same things).
    fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Lets `dt` seconds pass. Every transmission that falls in them goes to the listeners
    /// that are in range of it.
    pub fn step(&mut self, dt: f64, emitters: &mut [Emitter], listeners: &[Listener]) -> Vec<Reception> {
        let end = self.time + dt;
        let mut heard = vec![];
        for (e, emitter) in emitters.iter_mut().enumerate() {
            for (time, technology, payload) in emitter.due(end) {
                for (l, listener) in listeners.iter().enumerate() {
                    if !listener.hears.contains(&technology) {
                        continue;
                    }
                    let (n, east) = listener.position.offset_to(&emitter.beacon.position);
                    let up = emitter.beacon.position.alt - listener.position.alt;
                    let distance = (n * n + east * east + up * up).sqrt();
                    if self.random() < technology.receive_probability(distance) {
                        heard.push(Reception { listener: l, emitter: e, technology, time, payload: payload.clone() });
                    }
                }
            }
        }
        self.time = end;
        heard.sort_by(|a, b| a.time.total_cmp(&b.time));
        heard
    }
}

/// The component ID that a Remote ID receiver on board sends from.
pub const RECEIVER_COMPONENT: u8 = ODID_COMPONENT;

#[cfg(test)]
mod tests {
    use super::*;
    use communication_telemetry::Decoder;
    use communication_traffic::Source;

    const FIELD: Position = Position { lat: 37.87, lon: -122.27, alt: 50.0 };

    fn beacon(sysid: u8, at: Position) -> Beacon {
        Beacon {
            sysid,
            position: at,
            velocity: [0.0; 3],
            heading: 90.0,
            uas_id: format!("T1S{sysid:04}"),
            mac: [0x60, 0, 0, 0, 0, sysid],
        }
    }

    fn listener(hears: &[Technology]) -> Listener {
        Listener { position: FIELD, hears: hears.to_vec() }
    }

    fn run(air: &mut Air, emitters: &mut [Emitter], listeners: &[Listener], seconds: u32) -> Vec<Reception> {
        (0..seconds * 20).flat_map(|_| air.step(0.05, emitters, listeners)).collect()
    }

    #[test]
    fn the_mavlink_network_carries_a_heartbeat_and_two_positions_a_second() {
        let mut e = [Emitter::new(beacon(2, FIELD.moved(100.0, 0.0, 0.0)), vec![Technology::MavlinkNetwork])];
        let heard = run(&mut Air::new(1), &mut e, &[listener(&[Technology::MavlinkNetwork])], 10);
        let mut d = Decoder::new(1);
        let (mut beats, mut positions) = (0, 0);
        for r in &heard {
            let Payload::Mavlink(f) = &r.payload else { panic!() };
            let n = d.push(f, r.time).traffic.len();
            positions += n;
            beats += (n == 0) as usize;
        }
        assert!((8..=10).contains(&beats), "{beats} heartbeats in 10 s");
        assert!((17..=20).contains(&positions), "{positions} positions in 10 s");
    }

    #[test]
    fn a_bluetooth_5_broadcast_is_a_pack_with_the_id_and_the_position() {
        let mut e = [Emitter::new(beacon(2, FIELD.moved(100.0, 0.0, 0.0)), vec![Technology::Bluetooth5])];
        let heard = run(&mut Air::new(1), &mut e, &[listener(&[Technology::Bluetooth5])], 10);
        assert!((9..=10).contains(&heard.len()), "{}", heard.len());
        let Payload::RemoteId { mac, bytes } = &heard[0].payload else { panic!() };
        assert_eq!(*mac, [0x60, 0, 0, 0, 0, 2]);
        let messages = rid::decode_broadcast(bytes).unwrap();
        assert!(matches!(messages[0], rid::Message::BasicId(_)) && matches!(messages[1], rid::Message::Location(_)));
    }

    #[test]
    fn a_bluetooth_4_broadcast_is_one_message_and_the_id_comes_less_often() {
        let mut e = [Emitter::new(beacon(2, FIELD.moved(50.0, 0.0, 0.0)), vec![Technology::Bluetooth4])];
        let heard = run(&mut Air::new(1), &mut e, &[listener(&[Technology::Bluetooth4])], 12);
        let kinds: Vec<_> = heard
            .iter()
            .map(|r| match &r.payload {
                Payload::RemoteId { bytes, .. } => {
                    assert_eq!(bytes.len(), 25);
                    bytes[0] >> 4
                }
                _ => panic!(),
            })
            .collect();
        let locations = kinds.iter().filter(|k| **k == 1).count();
        let ids = kinds.iter().filter(|k| **k == 0).count();
        assert!((11..=12).contains(&locations), "{locations}");
        assert!((3..=4).contains(&ids), "{ids}");
    }

    #[test]
    fn nothing_is_heard_past_the_range_of_the_radio() {
        for (tech, far) in
            [(Technology::Bluetooth4, 400.0), (Technology::Bluetooth5, 1500.0), (Technology::MavlinkNetwork, 6000.0)]
        {
            let mut e = [Emitter::new(beacon(2, FIELD.moved(far, 0.0, 0.0)), vec![tech])];
            let heard = run(&mut Air::new(1), &mut e, &[listener(&[tech])], 10);
            assert!(heard.is_empty(), "{tech:?} heard at {far} m");
        }
    }

    #[test]
    fn close_in_nearly_everything_is_heard() {
        let mut e = [Emitter::new(beacon(2, FIELD.moved(200.0, 0.0, 0.0)), vec![Technology::MavlinkNetwork])];
        let heard = run(&mut Air::new(7), &mut e, &[listener(&[Technology::MavlinkNetwork])], 60);
        // 60 heartbeats and 120 positions were sent.
        assert!(heard.len() >= 170, "{}", heard.len());
    }

    #[test]
    fn at_the_edge_of_the_range_some_are_lost() {
        let mut e = [Emitter::new(beacon(2, FIELD.moved(3750.0, 0.0, 0.0)), vec![Technology::MavlinkNetwork])];
        let heard = run(&mut Air::new(7), &mut e, &[listener(&[Technology::MavlinkNetwork])], 60);
        // Halfway between the clear range and the range: about half are heard.
        assert!((60..=120).contains(&heard.len()), "{}", heard.len());
    }

    #[test]
    fn the_same_seed_hears_the_same_things() {
        let make = || [Emitter::new(beacon(2, FIELD.moved(3750.0, 0.0, 0.0)), vec![Technology::MavlinkNetwork])];
        let a = run(&mut Air::new(3), &mut make(), &[listener(&[Technology::MavlinkNetwork])], 20);
        let b = run(&mut Air::new(3), &mut make(), &[listener(&[Technology::MavlinkNetwork])], 20);
        let c = run(&mut Air::new(4), &mut make(), &[listener(&[Technology::MavlinkNetwork])], 20);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn a_silent_device_sends_nothing() {
        let mut e = [Emitter::new(beacon(2, FIELD), vec![])];
        assert!(
            run(&mut Air::new(1), &mut e, &[listener(&[Technology::MavlinkNetwork, Technology::Bluetooth5])], 10)
                .is_empty()
        );
    }

    #[test]
    fn a_listener_only_hears_the_radios_it_is_tuned_to() {
        let mut e = [Emitter::new(
            beacon(2, FIELD.moved(100.0, 0.0, 0.0)),
            vec![Technology::MavlinkNetwork, Technology::Bluetooth5],
        )];
        let heard = run(&mut Air::new(1), &mut e, &[listener(&[Technology::Bluetooth5])], 5);
        assert!(!heard.is_empty() && heard.iter().all(|r| r.technology == Technology::Bluetooth5));
    }

    #[test]
    fn each_listener_hears_for_itself() {
        let mut e = [Emitter::new(beacon(2, FIELD.moved(100.0, 0.0, 0.0)), vec![Technology::Bluetooth5])];
        let near = listener(&[Technology::Bluetooth5]);
        let far = Listener { position: FIELD.moved(2000.0, 0.0, 0.0), hears: vec![Technology::Bluetooth5] };
        let heard = run(&mut Air::new(1), &mut e, &[near, far], 5);
        assert!(heard.iter().all(|r| r.listener == 0));
    }

    #[test]
    fn a_moving_device_goes_out_of_range() {
        let mut b = beacon(2, FIELD.moved(100.0, 0.0, 0.0));
        b.velocity = [50.0, 0.0, 0.0];
        let mut e = [Emitter::new(b, vec![Technology::Bluetooth5])];
        let l = [listener(&[Technology::Bluetooth5])];
        let mut air = Air::new(1);
        let mut heard_late = 0;
        for i in 0..400 {
            e[0].fly(0.05);
            let got = air.step(0.05, &mut e, &l);
            if i > 300 {
                heard_late += got.len(); // 15 s on, 850 m away and moving off
            }
        }
        assert!(heard_late > 0, "still in range at 850 m");
        for _ in 0..400 {
            e[0].fly(0.05);
            air.step(0.05, &mut e, &l);
        }
        let past = air.step(1.0, &mut e, &l);
        assert!(past.is_empty(), "40 s on the device is 2 km away, past 1.2 km");
    }

    #[test]
    fn an_adsb_transponder_reaches_far_and_is_traffic() {
        let mut e = [Emitter::new(beacon(9, FIELD.moved(20_000.0, 0.0, 800.0)), vec![Technology::Adsb])];
        let heard = run(&mut Air::new(1), &mut e, &[listener(&[Technology::Adsb])], 5);
        assert!(!heard.is_empty());
        let mut d = Decoder::new(1);
        let Payload::Mavlink(f) = &heard[0].payload else { panic!() };
        let o = &d.push(f, 1.0).traffic[0];
        assert_eq!(o.identity.source, Source::Adsb);
        assert_eq!(o.name.as_deref(), Some("T1S009"));
    }
}
