//! The ground agent's MAVLink link to one plane (ArduPlane, with the AGENT mode): a TCP
//! connection, a thread that reads the plane's heartbeat, `COMMAND_ACK`s, the agent's
//! `NAMED_VALUE_INT` status and `STATUSTEXT`, and the messages the agent sends:
//!
//! | Sent | Fields |
//! | --- | --- |
//! | plan | `COMMAND_INT`, command 31010 (`MAV_CMD_USER_1`), frame `MAV_FRAME_GLOBAL_RELATIVE_ALT_INT`, `param1..4` primitive, target, plan id, ttl; `x`/`y` lat/lon 1e7; `z` altitude and radius packed (`autonomy_plan::WirePlan::to_command`) |
//! | game | `COMMAND_INT`, command 31011 (`MAV_CMD_USER_2`), frame `MAV_FRAME_MISSION`, `param1..3` role, time left, no-tag-back id |
//! | AGENT | `COMMAND_LONG` `MAV_CMD_DO_SET_MODE`, `param1` 1 (custom mode), `param2` the custom mode (50) |
//! | traffic | `ADSB_VEHICLE`: ICAO [`ICAO_BASE`] + the plane's id in the low byte, lat/lon 1e7, altitude AMSL (mm, geometric), heading (cdeg), horizontal and vertical speed (cm/s, up positive), emitter UAV, callsign, flags valid |
//! | heartbeat | `HEARTBEAT` as a GCS, once a second |

use crate::Craft;
use autonomy_plan::{CMD_GAME, CMD_PLAN, WireGame, WirePlan, status_names, wire::CommandInt};
use mavlink::dialects::ardupilotmega::*;
use mavlink::error::MessageReadError;
use mavlink::{MavConnection, MavHeader, MavlinkVersion};
use std::io::{self, ErrorKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The station's MAVLink system and component ids.
pub const GCS_SYSID: u8 = 255;
pub const GCS_COMPID: u8 = 190;
/// The ICAO addresses the station gives its planes on the uplink: this, with the plane's id in
/// the low byte.
pub const ICAO_BASE: u32 = 0x00_5A_00_00;
/// `MAV_MODE_FLAG_SAFETY_ARMED`.
const ARMED: u8 = 128;

/// How many acks and texts a link keeps.
const KEEP: usize = 32;

fn header() -> MavHeader {
    MavHeader { system_id: GCS_SYSID, component_id: GCS_COMPID, sequence: 0 }
}

/// A plan or game `COMMAND_INT` as the MAVLink message to `target` (system, component).
#[allow(deprecated)] // `_INT` is what the plan protocol names; ArduPilot takes it as `GLOBAL_RELATIVE_ALT`
pub fn command_int(c: &CommandInt, target: (u8, u8)) -> Option<MavMessage> {
    let (command, frame) = match c.command {
        CMD_PLAN => (MavCmd::MAV_CMD_USER_1, MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT_INT),
        CMD_GAME => (MavCmd::MAV_CMD_USER_2, MavFrame::MAV_FRAME_MISSION),
        _ => return None,
    };
    Some(MavMessage::COMMAND_INT(COMMAND_INT_DATA {
        param1: c.param1,
        param2: c.param2,
        param3: c.param3,
        param4: c.param4,
        x: c.x,
        y: c.y,
        z: c.z,
        command,
        target_system: target.0,
        target_component: target.1,
        frame,
        current: 0,
        autocontinue: 0,
    }))
}

/// The fields of a received `COMMAND_INT` (the plane's side, and the tests').
pub fn command_from_message(msg: &MavMessage) -> Option<CommandInt> {
    let MavMessage::COMMAND_INT(d) = msg else { return None };
    Some(CommandInt {
        command: d.command as u32 as u16,
        param1: d.param1,
        param2: d.param2,
        param3: d.param3,
        param4: d.param4,
        x: d.x,
        y: d.y,
        z: d.z,
    })
}

/// `MAV_CMD_DO_SET_MODE` to an ArduPlane custom mode (AGENT is 50).
pub fn set_mode_message(custom_mode: u32, target: (u8, u8)) -> MavMessage {
    MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        param1: 1.0, // MAV_MODE_FLAG_CUSTOM_MODE_ENABLED
        param2: custom_mode as f32,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
        command: MavCmd::MAV_CMD_DO_SET_MODE,
        target_system: target.0,
        target_component: target.1,
        confirmation: 0,
    })
}

/// Another aircraft, as the station uplinks it.
#[derive(Clone, Debug, PartialEq)]
pub struct Traffic {
    /// The plane's id: the low byte of its ICAO address, what a plan's `target` names.
    pub id: u8,
    pub lat: f64,
    pub lon: f64,
    /// Above mean sea level (m).
    pub alt_amsl: f32,
    /// Degrees from north, 0..360.
    pub heading: f32,
    /// Ground speed (m/s).
    pub speed: f32,
    /// Up positive (m/s).
    pub climb: f32,
    pub callsign: String,
}

impl Traffic {
    /// One of the station's planes, with home at (`lat`, `lon`, `alt` AMSL).
    pub fn from_craft(c: &Craft, home: (f64, f64, f32), callsign: &str) -> Traffic {
        let (lat, lon) = autonomy_plan::to_global(home.0, home.1, c.pos.x, c.pos.y);
        let heading = c.vel.y.atan2(c.vel.x).to_degrees().rem_euclid(360.0);
        Traffic {
            id: c.id,
            lat,
            lon,
            alt_amsl: home.2 - c.pos.z,
            heading,
            speed: c.vel.truncate().length(),
            climb: -c.vel.z,
            callsign: callsign.into(),
        }
    }

    pub fn icao(&self) -> u32 {
        ICAO_BASE | self.id as u32
    }
}

/// The `ADSB_VEHICLE` message of a plane.
pub fn adsb_vehicle(t: &Traffic) -> MavMessage {
    let callsign: String =
        t.callsign.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>().to_uppercase();
    MavMessage::ADSB_VEHICLE(ADSB_VEHICLE_DATA {
        ICAO_address: t.icao(),
        lat: (t.lat * 1e7).round() as i32,
        lon: (t.lon * 1e7).round() as i32,
        altitude: (t.alt_amsl * 1000.0).round() as i32,
        heading: ((t.heading.rem_euclid(360.0) * 100.0).round() as u16) % 36000,
        hor_velocity: (t.speed.max(0.0) * 100.0).round().min(u16::MAX as f32) as u16,
        ver_velocity: (t.climb * 100.0).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16,
        flags: AdsbFlags::ADSB_FLAGS_VALID_COORDS
            | AdsbFlags::ADSB_FLAGS_VALID_ALTITUDE
            | AdsbFlags::ADSB_FLAGS_VALID_HEADING
            | AdsbFlags::ADSB_FLAGS_VALID_VELOCITY
            | AdsbFlags::ADSB_FLAGS_VALID_CALLSIGN
            | AdsbFlags::ADSB_FLAGS_VERTICAL_VELOCITY_VALID,
        squawk: 0,
        altitude_type: AdsbAltitudeType::ADSB_ALTITUDE_TYPE_GEOMETRIC,
        callsign: callsign.as_str().into(),
        emitter_type: AdsbEmitterType::ADSB_EMITTER_TYPE_UAV,
        tslc: 0,
    })
}

/// A `COMMAND_ACK` from the plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ack {
    pub command: u16,
    /// `MAV_RESULT`: 0 accepted, 3 unsupported, 4 failed, ...
    pub result: u8,
    /// The reason for a refusal (the plan protocol's).
    pub result_param2: i32,
}

/// The agent's status (`NAMED_VALUE_INT`), as last heard.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AgentStatus {
    /// `AG_PLAN`: the id of the plan being flown, 0 for none.
    pub plan: Option<i32>,
    /// `AG_PRIM`: its primitive.
    pub primitive: Option<i32>,
    /// `AG_ENV`: envelope interventions since AGENT mode was entered.
    pub interventions: Option<i32>,
    /// `AG_FALL`: why the executor fell back, 0 for not at all.
    pub fallback: Option<i32>,
}

/// What a link has heard, and what it has sent.
#[derive(Clone, Debug, Default)]
pub struct LinkState {
    pub connected: bool,
    /// The plane's (system, component), from its heartbeat; (1, 1) until heard.
    pub target: (u8, u8),
    /// The flight mode (ArduPlane custom mode) and armed flag of its last heartbeat.
    pub mode: Option<u32>,
    pub armed: bool,
    pub heartbeats: u64,
    pub messages: u64,
    /// The latest acks, oldest first.
    pub acks: Vec<Ack>,
    /// How many acks of each (command, result) there have been.
    pub ack_counts: Vec<((u16, u8), u32)>,
    pub status: AgentStatus,
    /// The latest STATUSTEXTs, oldest first.
    pub texts: Vec<String>,
    /// The id of the last plan sent, and how many plans have been sent.
    pub last_plan: Option<u16>,
    pub plans_sent: u64,
    /// The highest `AG_PLAN` the plane has reported: whether it is taking the plans at all.
    pub best_plan_echo: Option<i32>,
    /// How the link went down.
    pub error: Option<String>,
}

impl LinkState {
    /// How many acks for `command` had `result`.
    pub fn acks_of(&self, command: u16, result: u8) -> u32 {
        self.ack_counts.iter().find(|(k, _)| *k == (command, result)).map_or(0, |(_, n)| *n)
    }

    fn handle(&mut self, hdr: MavHeader, msg: MavMessage) {
        self.messages += 1;
        match msg {
            MavMessage::HEARTBEAT(h)
                if h.mavtype != MavType::MAV_TYPE_GCS && h.autopilot != MavAutopilot::MAV_AUTOPILOT_INVALID =>
            {
                self.target = (hdr.system_id, hdr.component_id);
                self.mode = Some(h.custom_mode);
                self.armed = h.base_mode.bits() & ARMED != 0;
                self.heartbeats += 1;
            }
            MavMessage::COMMAND_ACK(a) => {
                let ack = Ack {
                    command: a.command as u32 as u16,
                    result: a.result as u32 as u8,
                    result_param2: a.result_param2,
                };
                match self.ack_counts.iter_mut().find(|(k, _)| *k == (ack.command, ack.result)) {
                    Some((_, n)) => *n += 1,
                    None => self.ack_counts.push(((ack.command, ack.result), 1)),
                }
                self.acks.push(ack);
                if self.acks.len() > KEEP {
                    self.acks.remove(0);
                }
            }
            MavMessage::NAMED_VALUE_INT(v) => {
                let name = v.name.to_str().unwrap_or("").trim_end_matches('\0');
                let s = &mut self.status;
                match name {
                    n if n == status_names::PLAN => {
                        s.plan = Some(v.value);
                        self.best_plan_echo = Some(self.best_plan_echo.map_or(v.value, |b| b.max(v.value)));
                    }
                    n if n == status_names::PRIMITIVE => s.primitive = Some(v.value),
                    n if n == status_names::INTERVENTIONS => s.interventions = Some(v.value),
                    n if n == status_names::FALLBACK => s.fallback = Some(v.value),
                    _ => {}
                }
            }
            MavMessage::STATUSTEXT(t) => {
                self.texts.push(t.text.to_str().unwrap_or("").trim_end_matches('\0').to_string());
                if self.texts.len() > KEEP {
                    self.texts.remove(0);
                }
            }
            _ => {}
        }
    }
}

/// A link to one plane.
pub struct Link {
    conn: Arc<mavlink::Connection<MavMessage>>,
    state: Arc<Mutex<LinkState>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    pub address: String,
}

impl Link {
    /// Connects to a plane's MAVLink TCP port (`"127.0.0.1:5770"`: SITL instance N listens on
    /// 5760 + 10·N) and starts reading.
    pub fn connect(address: &str) -> io::Result<Link> {
        let mut conn = mavlink::connect::<MavMessage>(&format!("tcpout:{address}"))?;
        conn.set_protocol_version(MavlinkVersion::V2);
        conn.set_allow_recv_any_version(true);
        let conn = Arc::new(conn);
        let state = Arc::new(Mutex::new(LinkState { connected: true, target: (1, 1), ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let (conn, state, stop) = (conn.clone(), state.clone(), stop.clone());
            std::thread::Builder::new().name(format!("link-{address}")).spawn(move || read(&conn, &state, &stop))?
        };
        Ok(Link { conn, state, stop, reader: Some(reader), address: address.into() })
    }

    /// [`Link::connect`], trying again until `patience` runs out (SITL takes a moment to listen).
    pub fn connect_patiently(address: &str, patience: Duration) -> io::Result<Link> {
        let start = Instant::now();
        loop {
            match Link::connect(address) {
                Ok(l) => return Ok(l),
                Err(e) if start.elapsed() > patience => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    pub fn send(&self, msg: &MavMessage) -> io::Result<()> {
        self.conn.send(&header(), msg).map(|_| ()).map_err(|e| io::Error::other(e.to_string()))
    }

    fn target(&self) -> (u8, u8) {
        self.state.lock().unwrap().target
    }

    pub fn send_plan(&self, plan: &WirePlan) -> io::Result<()> {
        self.send(&command_int(&plan.to_command(), self.target()).expect("a plan"))?;
        let mut s = self.state.lock().unwrap();
        s.last_plan = Some(plan.id);
        s.plans_sent += 1;
        Ok(())
    }

    pub fn send_game(&self, game: &WireGame) -> io::Result<()> {
        self.send(&command_int(&game.to_command(), self.target()).expect("a game"))
    }

    /// Asks for an ArduPlane flight mode (custom mode number).
    pub fn set_mode(&self, custom_mode: u32) -> io::Result<()> {
        self.send(&set_mode_message(custom_mode, self.target()))
    }

    pub fn send_traffic(&self, t: &Traffic) -> io::Result<()> {
        self.send(&adsb_vehicle(t))
    }

    /// What the link has heard so far.
    pub fn state(&self) -> LinkState {
        self.state.lock().unwrap().clone()
    }

    /// The plane's flight mode, as of its last heartbeat.
    pub fn mode(&self) -> Option<u32> {
        self.state.lock().unwrap().mode
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

/// The reader: every message into the state, and a GCS heartbeat each second.
fn read(conn: &mavlink::Connection<MavMessage>, state: &Mutex<LinkState>, stop: &AtomicBool) {
    let mut beat = Instant::now() - Duration::from_secs(5);
    while !stop.load(Ordering::Relaxed) {
        if beat.elapsed() >= Duration::from_secs(1) {
            beat = Instant::now();
            let _ = conn.send(
                &header(),
                &MavMessage::HEARTBEAT(HEARTBEAT_DATA {
                    custom_mode: 0,
                    mavtype: MavType::MAV_TYPE_GCS,
                    autopilot: MavAutopilot::MAV_AUTOPILOT_INVALID,
                    base_mode: MavModeFlag::empty(),
                    system_status: MavState::MAV_STATE_ACTIVE,
                    mavlink_version: 3,
                }),
            );
        }
        match conn.recv() {
            Ok((hdr, msg)) => state.lock().unwrap().handle(hdr, msg),
            Err(MessageReadError::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(MessageReadError::Io(e)) => {
                let mut s = state.lock().unwrap();
                s.connected = false;
                s.error = Some(e.to_string());
                return;
            }
            Err(_) => {} // a bad CRC or a message this dialect doesn't know
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autonomy_plan::{Primitive, wire::Role};
    use glam::Vec3;
    use mavlink::MavlinkReader;
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    #[allow(deprecated)]
    fn a_plan_goes_out_as_command_int_31010_and_comes_back_the_same() {
        let plan = WirePlan {
            id: 77,
            primitive: Primitive::Pursue,
            target: 3,
            lat: -35.36,
            lon: 149.16,
            alt: 42.5,
            radius: 0.0,
            ttl: 2.5,
        };
        let msg = command_int(&plan.to_command(), (1, 1)).unwrap();
        let MavMessage::COMMAND_INT(d) = &msg else { panic!() };
        assert_eq!(d.command as u32, 31010);
        assert_eq!(d.frame, MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT_INT);
        assert_eq!((d.target_system, d.target_component), (1, 1));
        // Through the bytes of a real MAVLink 2 frame.
        let mut bytes = vec![];
        mavlink::write_v2_msg(&mut bytes, header(), &msg).unwrap();
        let (_, back) = MavlinkReader::new(&bytes[..]).read_any_message::<MavMessage>().unwrap();
        let c = command_from_message(&back).unwrap();
        assert_eq!(c, plan.to_command());
        assert_eq!(WirePlan::from_command(&c).unwrap().id, 77);
    }

    #[test]
    fn the_game_goes_out_as_command_int_31011() {
        let g = WireGame { role: Role::It, time_left: 61.0, no_tag_back: 2 };
        let msg = command_int(&g.to_command(), (1, 1)).unwrap();
        let c = command_from_message(&msg).unwrap();
        assert_eq!(c.command, 31011);
        assert_eq!(WireGame::from_command(&c), Some(g));
        assert!(command_int(&CommandInt { command: 1, ..Default::default() }, (1, 1)).is_none());
    }

    #[test]
    fn set_mode_asks_for_the_custom_mode() {
        let MavMessage::COMMAND_LONG(d) = set_mode_message(50, (1, 1)) else { panic!() };
        assert_eq!((d.command, d.param1, d.param2), (MavCmd::MAV_CMD_DO_SET_MODE, 1.0, 50.0));
    }

    #[test]
    fn a_plane_goes_up_as_adsb_vehicle() {
        // 100 m north, 50 m east, 30 m up, flying east at 20 m/s and climbing at 1 m/s.
        let c = Craft { id: 3, pos: Vec3::new(100.0, 50.0, -30.0), vel: Vec3::new(0.0, 20.0, -1.0), flying: true };
        let t = Traffic::from_craft(&c, (-35.363261, 149.16523, 584.0), "Swift");
        let MavMessage::ADSB_VEHICLE(d) = adsb_vehicle(&t) else { panic!() };
        assert_eq!(d.ICAO_address & 0xFF, 3);
        assert_eq!(d.ICAO_address, ICAO_BASE | 3);
        assert_eq!(d.altitude, 614_000, "mm above sea level");
        assert_eq!((d.heading, d.hor_velocity, d.ver_velocity), (9000, 2000, 100));
        assert_eq!(d.callsign.to_str().unwrap(), "SWIFT");
        assert!(d.flags.contains(AdsbFlags::ADSB_FLAGS_VALID_COORDS | AdsbFlags::ADSB_FLAGS_VALID_ALTITUDE));
        let (n, e) = autonomy_plan::to_local(-35.363261, 149.16523, d.lat as f64 * 1e-7, d.lon as f64 * 1e-7);
        assert!((n - 100.0).abs() < 0.05 && (e - 50.0).abs() < 0.05, "{n} {e}");
    }

    /// A pretend ArduPlane: says it is in FBWA, acks the plan and reports its id as AG_PLAN,
    /// changes mode when asked, and passes on what traffic it hears.
    fn fake_plane(listener: TcpListener, heard: mpsc::Sender<MavMessage>) {
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut reader = MavlinkReader::new(stream);
        let me = MavHeader { system_id: 7, component_id: 1, sequence: 0 };
        let beat = |mode: u32| {
            MavMessage::HEARTBEAT(HEARTBEAT_DATA {
                custom_mode: mode,
                mavtype: MavType::MAV_TYPE_FIXED_WING,
                autopilot: MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
                base_mode: MavModeFlag::from_bits_truncate(ARMED | 1),
                system_status: MavState::MAV_STATE_ACTIVE,
                mavlink_version: 3,
            })
        };
        let mut send = |m: MavMessage| mavlink::write_v2_msg(&mut out, me, &m).unwrap();
        send(beat(5));
        let ack = |command: MavCmd, result: MavResult| {
            MavMessage::COMMAND_ACK(COMMAND_ACK_DATA {
                command,
                result,
                progress: 0,
                result_param2: 0,
                target_system: GCS_SYSID,
                target_component: GCS_COMPID,
            })
        };
        while let Ok((hdr, msg)) = reader.read_any_message::<MavMessage>() {
            assert_eq!(hdr.system_id, GCS_SYSID);
            match &msg {
                MavMessage::COMMAND_INT(c) if c.target_system == 7 => {
                    send(ack(c.command, MavResult::MAV_RESULT_ACCEPTED));
                    if c.command == MavCmd::MAV_CMD_USER_1 {
                        let name = status_names::PLAN.into();
                        send(MavMessage::NAMED_VALUE_INT(NAMED_VALUE_INT_DATA {
                            time_boot_ms: 0,
                            value: c.param3 as i32,
                            name,
                        }));
                    }
                }
                MavMessage::COMMAND_LONG(c) if c.command == MavCmd::MAV_CMD_DO_SET_MODE => {
                    send(ack(c.command, MavResult::MAV_RESULT_ACCEPTED));
                    send(beat(c.param2 as u32));
                }
                _ => {}
            }
            if heard.send(msg).is_err() {
                return;
            }
        }
    }

    fn until(link: &Link, what: &str, ok: impl Fn(&LinkState) -> bool) -> LinkState {
        let start = Instant::now();
        loop {
            let s = link.state();
            if ok(&s) {
                return s;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "waited for {what}: {s:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_link_talks_to_a_plane() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (tx, heard) = mpsc::channel();
        let plane = std::thread::spawn(move || fake_plane(listener, tx));
        let link = Link::connect(&address).unwrap();

        let s = until(&link, "a heartbeat", |s| s.heartbeats > 0);
        assert_eq!((s.mode, s.target, s.armed), (Some(5), (7, 1), true));

        let plan = WirePlan {
            id: 12,
            primitive: Primitive::Band,
            lat: -35.36,
            lon: 149.16,
            alt: 20.0,
            radius: 10.0,
            ttl: 2.5,
            ..Default::default()
        };
        link.send_plan(&plan).unwrap();
        let s = until(&link, "the plan's ack and status", |s| s.status.plan == Some(12));
        assert_eq!(s.acks_of(CMD_PLAN, 0), 1);
        assert_eq!((s.last_plan, s.plans_sent, s.best_plan_echo), (Some(12), 1, Some(12)));

        link.send_game(&WireGame { role: Role::Runner, time_left: 90.0, no_tag_back: 0 }).unwrap();
        until(&link, "the game's ack", |s| s.acks_of(CMD_GAME, 0) == 1);

        link.set_mode(50).unwrap();
        let s = until(&link, "AGENT mode", |s| s.mode == Some(50));
        assert_eq!(s.acks_of(MavCmd::MAV_CMD_DO_SET_MODE as u32 as u16, 0), 1);

        let c = Craft { id: 2, pos: Vec3::new(10.0, 0.0, -25.0), vel: Vec3::new(18.0, 0.0, 0.0), flying: true };
        link.send_traffic(&Traffic::from_craft(&c, (-35.36, 149.16, 584.0), "Merlin")).unwrap();

        // The plane heard, in order: the plan, the game, the mode change, the traffic (and the
        // station's heartbeats in between).
        let mut got = vec![];
        let start = Instant::now();
        while got.len() < 4 && start.elapsed() < Duration::from_secs(5) {
            match heard.recv_timeout(Duration::from_secs(5)).unwrap() {
                MavMessage::HEARTBEAT(h) => assert_eq!(h.mavtype, MavType::MAV_TYPE_GCS),
                m => got.push(m),
            }
        }
        assert_eq!(command_from_message(&got[0]), Some(plan.to_command()));
        assert_eq!(command_from_message(&got[1]).unwrap().command, CMD_GAME);
        assert!(matches!(&got[2], MavMessage::COMMAND_LONG(c) if c.param2 == 50.0));
        let MavMessage::ADSB_VEHICLE(a) = &got[3] else { panic!("{:?}", got[3]) };
        assert_eq!((a.ICAO_address & 0xFF, a.hor_velocity, a.heading), (2, 1800, 0));

        drop(link);
        drop(heard);
        plane.join().unwrap();
    }

    #[test]
    fn a_link_to_nowhere_fails() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(Link::connect(&format!("127.0.0.1:{port}")).is_err());
    }
}
