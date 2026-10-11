//! MAVLink link to ArduPilot. One thread owns the connection: it decodes telemetry into the
//! shared [`Vehicle`], sends the GCS heartbeat and stream requests, executes UI commands and
//! runs the mission upload/download protocol. It reconnects on its own.

// ArduPilot still drives telemetry rates from REQUEST_DATA_STREAM groups (SRx_ params) and
// may send the old MISSION_REQUEST, so the "deprecated" messages are used on purpose.
#![allow(deprecated)]

use crate::vehicle::{Vehicle, Waypoint, WpKind, mode_name};
use mavlink::dialects::ardupilotmega::*;
use mavlink::error::MessageReadError;
use mavlink::{MavConnection, MavHeader, MavlinkVersion};
use std::io::ErrorKind;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const GCS_SYSID: u8 = 255;
const GCS_COMPID: u8 = 190; // MAV_COMP_ID_MISSIONPLANNER
const MSG_HOME_POSITION: f32 = 242.0;

pub enum Cmd {
    Arm(bool),
    /// Disarm even if ArduPilot thinks it's flying (it refuses a normal disarm then).
    ForceDisarm,
    /// Restart ArduPilot (forced, so it works armed too): clears confused estimators.
    Reboot,
    SetMode(u32),
    /// Fly to and loiter at a point (GUIDED), altitude relative to home.
    Goto { lat: f64, lon: f64, alt: f32 },
    /// Replace the vehicle's mission (home is inserted as item 0).
    Upload(Vec<Waypoint>),
    Download,
    /// Erase the vehicle's mission (MISSION_CLEAR_ALL). Download afterwards to see it empty.
    ClearMission,
    SetCurrentWp(u16),
    /// RC_CHANNELS_OVERRIDE for channels 1-8 (0 releases a channel). Used by --autotest only.
    RcOverride([u16; 8]),
}

pub type Shared = Arc<Mutex<Vehicle>>;

pub fn spawn(address: String, vehicle: Shared) -> Sender<Cmd> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("mavlink".into())
        .spawn(move || run(address, vehicle, rx))
        .expect("spawn mavlink thread");
    tx
}

enum Xfer {
    Idle,
    Upload { items: Vec<MISSION_ITEM_INT_DATA>, plan: Vec<Waypoint>, last: Instant, tries: u8 },
    Download { items: Vec<Option<Waypoint>>, next: u16, last: Instant, tries: u8 },
}

struct Session {
    conn: mavlink::Connection<MavMessage>,
    target: (u8, u8),
    xfer: Xfer,
    last_heartbeat_tx: Instant,
    last_streams: Option<Instant>,
    home_requested: Option<Instant>,
    downloaded_once: bool,
    rate_window: (Instant, u64),
    /// STATUSTEXT longer than one message arrives in chunks sharing an id: (id, next chunk,
    /// severity, text so far).
    text_chunks: Option<(u16, u8, u8, String)>,
}

fn run(address: String, vehicle: Shared, rx: Receiver<Cmd>) {
    loop {
        vehicle.lock().unwrap().link.status = format!("connecting to {address}");
        let mut conn = match mavlink::connect::<MavMessage>(&address) {
            Ok(c) => c,
            Err(e) => {
                {
                    let mut v = vehicle.lock().unwrap();
                    v.link.connected = false;
                    v.link.status = format!("{address}: {e}");
                }
                // Drop commands issued while offline rather than replaying them later.
                while rx.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        conn.set_protocol_version(MavlinkVersion::V2);
        {
            let mut v = vehicle.lock().unwrap();
            v.link.connected = true;
            v.link.status = format!("connected {address}, waiting for heartbeat");
            v.push_log("GCS", 6, format!("Link up: {address}"));
        }
        let mut s = Session {
            conn,
            target: (1, 1),
            xfer: Xfer::Idle,
            last_heartbeat_tx: Instant::now() - Duration::from_secs(5),
            last_streams: None,
            home_requested: None,
            downloaded_once: false,
            rate_window: (Instant::now(), 0),
            text_chunks: None,
        };
        let err = s.run(&vehicle, &rx);
        let mut v = vehicle.lock().unwrap();
        v.link.connected = false;
        v.link.status = format!("link lost: {err}");
        v.push_log("GCS", 4, format!("Link lost: {err}"));
        drop(v);
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn header() -> MavHeader {
    MavHeader { system_id: GCS_SYSID, component_id: GCS_COMPID, sequence: 0 }
}

impl Session {
    fn send(&self, msg: MavMessage) {
        if let Err(e) = self.conn.send(&header(), &msg) {
            eprintln!("mavlink send failed: {e}");
        }
    }

    fn command_long(&self, command: MavCmd, p: [f32; 7]) {
        self.send(MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
            param1: p[0],
            param2: p[1],
            param3: p[2],
            param4: p[3],
            param5: p[4],
            param6: p[5],
            param7: p[6],
            command,
            target_system: self.target.0,
            target_component: self.target.1,
            confirmation: 0,
        }));
    }

    fn run(&mut self, vehicle: &Shared, rx: &Receiver<Cmd>) -> String {
        loop {
            match self.conn.recv() {
                Ok((hdr, msg)) => self.handle(vehicle, hdr, msg),
                Err(MessageReadError::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(MessageReadError::Io(e)) => return e.to_string(),
                Err(_) => {} // bad CRC / unknown message: skip
            }
            while let Ok(cmd) = rx.try_recv() {
                self.execute(vehicle, cmd);
            }
            self.periodic(vehicle);
        }
    }

    fn periodic(&mut self, vehicle: &Shared) {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat_tx) >= Duration::from_secs(1) {
            self.last_heartbeat_tx = now;
            self.send(MavMessage::HEARTBEAT(HEARTBEAT_DATA {
                custom_mode: 0,
                mavtype: MavType::MAV_TYPE_GCS,
                autopilot: MavAutopilot::MAV_AUTOPILOT_INVALID,
                base_mode: MavModeFlag::empty(),
                system_status: MavState::MAV_STATE_ACTIVE,
                mavlink_version: 3,
            }));
        }
        let (have_hb, have_home) = {
            let mut v = vehicle.lock().unwrap();
            let dt = now.duration_since(self.rate_window.0).as_secs_f32();
            if dt >= 1.0 {
                let n = v.link.messages - self.rate_window.1;
                v.link.rate = n as f32 / dt;
                self.rate_window = (now, v.link.messages);
                if v.link.connected {
                    v.link.status = if v.link.alive() {
                        format!("{} msg/s", v.link.rate.round())
                    } else {
                        "no heartbeat".into()
                    };
                }
            }
            (v.link.alive(), v.home.is_some())
        };
        if !have_hb {
            return;
        }
        // ArduPilot streams by SRx rate groups; ask for what the displays need. Re-sent
        // periodically so a rebooted autopilot picks the rates up again.
        if self.last_streams.is_none_or(|t| now.duration_since(t) > Duration::from_secs(15)) {
            self.last_streams = Some(now);
            for (stream, hz) in [
                (MavDataStream::MAV_DATA_STREAM_RAW_SENSORS, 2),
                (MavDataStream::MAV_DATA_STREAM_EXTENDED_STATUS, 4),
                (MavDataStream::MAV_DATA_STREAM_RC_CHANNELS, 4),
                (MavDataStream::MAV_DATA_STREAM_POSITION, 10),
                (MavDataStream::MAV_DATA_STREAM_EXTRA1, 20),
                (MavDataStream::MAV_DATA_STREAM_EXTRA2, 10),
                (MavDataStream::MAV_DATA_STREAM_EXTRA3, 2),
            ] {
                self.send(MavMessage::REQUEST_DATA_STREAM(REQUEST_DATA_STREAM_DATA {
                    req_message_rate: hz,
                    target_system: self.target.0,
                    target_component: self.target.1,
                    req_stream_id: stream,
                    start_stop: 1,
                }));
            }
        }
        if !have_home && self.home_requested.is_none_or(|t| now.duration_since(t) > Duration::from_secs(3)) {
            self.home_requested = Some(now);
            self.command_long(MavCmd::MAV_CMD_REQUEST_MESSAGE, [MSG_HOME_POSITION, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        }
        if !self.downloaded_once && matches!(self.xfer, Xfer::Idle) {
            self.downloaded_once = true;
            self.start_download(vehicle);
        }
        self.xfer_timeouts(vehicle, now);
    }

    fn execute(&mut self, vehicle: &Shared, cmd: Cmd) {
        let log = |sev: u8, s: String| vehicle.lock().unwrap().push_log("GCS", sev, s);
        match cmd {
            Cmd::Arm(arm) => {
                log(6, format!("Command: {}", if arm { "ARM" } else { "DISARM" }));
                self.command_long(MavCmd::MAV_CMD_COMPONENT_ARM_DISARM, [arm as u8 as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            }
            Cmd::ForceDisarm => {
                log(4, "Command: FORCE DISARM".into());
                // param2 21196: ArduPilot's magic value to disarm regardless of state.
                self.command_long(MavCmd::MAV_CMD_COMPONENT_ARM_DISARM, [0.0, 21196.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            }
            Cmd::Reboot => {
                log(4, "Command: REBOOT autopilot".into());
                // param1 1 = reboot; param6 20190226 = ArduPilot's magic value to allow it armed.
                self.command_long(MavCmd::MAV_CMD_PREFLIGHT_REBOOT_SHUTDOWN, [1.0, 0.0, 0.0, 0.0, 0.0, 20190226.0, 0.0]);
            }
            Cmd::SetMode(m) => {
                log(6, format!("Command: mode {}", mode_name(m)));
                // param1 = MAV_MODE_FLAG_CUSTOM_MODE_ENABLED, param2 = custom mode.
                self.command_long(MavCmd::MAV_CMD_DO_SET_MODE, [1.0, m as f32, 0.0, 0.0, 0.0, 0.0, 0.0]);
            }
            Cmd::Goto { lat, lon, alt } => {
                log(6, format!("Command: fly to {lat:.6}, {lon:.6} at {alt:.0} m"));
                vehicle.lock().unwrap().guided_target = Some((lat, lon, alt));
                self.send(MavMessage::COMMAND_INT(COMMAND_INT_DATA {
                    param1: -1.0, // default ground speed
                    param2: 1.0,  // MAV_DO_REPOSITION_FLAGS_CHANGE_MODE: switch to GUIDED
                    param3: 0.0,
                    param4: f32::NAN,
                    x: (lat * 1e7) as i32,
                    y: (lon * 1e7) as i32,
                    z: alt,
                    command: MavCmd::MAV_CMD_DO_REPOSITION,
                    target_system: self.target.0,
                    target_component: self.target.1,
                    frame: MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT,
                    current: 0,
                    autocontinue: 0,
                }));
            }
            Cmd::Upload(plan) => self.start_upload(vehicle, plan),
            Cmd::Download => self.start_download(vehicle),
            Cmd::ClearMission => {
                vehicle.lock().unwrap().push_log("GCS", 6, "Command: clear the vehicle's mission");
                self.send(MavMessage::MISSION_CLEAR_ALL(MISSION_CLEAR_ALL_DATA {
                    target_system: self.target.0,
                    target_component: self.target.1,
                    ..Default::default()
                }));
            }
            Cmd::RcOverride(ch) => {
                self.send(MavMessage::RC_CHANNELS_OVERRIDE(RC_CHANNELS_OVERRIDE_DATA {
                    chan1_raw: ch[0],
                    chan2_raw: ch[1],
                    chan3_raw: ch[2],
                    chan4_raw: ch[3],
                    chan5_raw: ch[4],
                    chan6_raw: ch[5],
                    chan7_raw: ch[6],
                    chan8_raw: ch[7],
                    target_system: self.target.0,
                    target_component: self.target.1,
                    ..Default::default()
                }));
            }
            Cmd::SetCurrentWp(seq) => {
                log(6, format!("Command: set current waypoint {seq}"));
                self.command_long(MavCmd::MAV_CMD_DO_SET_MISSION_CURRENT, [seq as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            }
        }
    }

    // --- mission protocol -------------------------------------------------------------

    fn start_upload(&mut self, vehicle: &Shared, plan: Vec<Waypoint>) {
        let home = {
            let v = vehicle.lock().unwrap();
            v.home.unwrap_or((v.lat, v.lon, v.alt_msl))
        };
        // Item 0 is home on ArduPilot; it is overwritten by the vehicle but must be present.
        let mut items = vec![mission_item(0, &Waypoint { kind: WpKind::Waypoint, lat: home.0, lon: home.1, alt: 0.0 }, self.target)];
        items[0].frame = MavFrame::MAV_FRAME_GLOBAL;
        for (i, wp) in plan.iter().enumerate() {
            items.push(mission_item(i as u16 + 1, wp, self.target));
        }
        {
            let mut v = vehicle.lock().unwrap();
            v.mission_status = format!("uploading {} items", plan.len());
            v.push_log("GCS", 6, format!("Mission upload: {} items", plan.len()));
        }
        self.send_count(items.len() as u16);
        self.xfer = Xfer::Upload { items, plan, last: Instant::now(), tries: 0 };
    }

    fn send_count(&self, count: u16) {
        self.send(MavMessage::MISSION_COUNT(MISSION_COUNT_DATA {
            count,
            target_system: self.target.0,
            target_component: self.target.1,
            ..Default::default()
        }));
    }

    fn start_download(&mut self, vehicle: &Shared) {
        vehicle.lock().unwrap().mission_status = "downloading".into();
        self.send(MavMessage::MISSION_REQUEST_LIST(MISSION_REQUEST_LIST_DATA {
            target_system: self.target.0,
            target_component: self.target.1,
            ..Default::default()
        }));
        self.xfer = Xfer::Download { items: vec![], next: 0, last: Instant::now(), tries: 0 };
    }

    fn request_item(&self, seq: u16) {
        self.send(MavMessage::MISSION_REQUEST_INT(MISSION_REQUEST_INT_DATA {
            seq,
            target_system: self.target.0,
            target_component: self.target.1,
            ..Default::default()
        }));
    }

    fn xfer_timeouts(&mut self, vehicle: &Shared, now: Instant) {
        let timed_out = match &self.xfer {
            Xfer::Upload { last, .. } | Xfer::Download { last, .. } => now.duration_since(*last) > Duration::from_millis(1500),
            Xfer::Idle => false,
        };
        if !timed_out {
            return;
        }
        let mut fail = false;
        match &mut self.xfer {
            Xfer::Upload { items, last, tries, .. } => {
                *tries += 1;
                *last = now;
                fail = *tries > 4;
                if !fail {
                    let n = items.len() as u16;
                    self.send_count(n);
                }
            }
            Xfer::Download { items, next, last, tries } => {
                *tries += 1;
                *last = now;
                fail = *tries > 4;
                if !fail {
                    if items.is_empty() {
                        self.send(MavMessage::MISSION_REQUEST_LIST(MISSION_REQUEST_LIST_DATA {
                            target_system: self.target.0,
                            target_component: self.target.1,
                            ..Default::default()
                        }));
                    } else {
                        let seq = *next;
                        self.request_item(seq);
                    }
                }
            }
            Xfer::Idle => {}
        }
        if fail {
            self.xfer = Xfer::Idle;
            let mut v = vehicle.lock().unwrap();
            v.mission_status = "transfer timed out".into();
            v.push_log("GCS", 4, "Mission transfer timed out");
        }
    }

    // --- telemetry ----------------------------------------------------------------------

    fn handle(&mut self, vehicle: &Shared, hdr: MavHeader, msg: MavMessage) {
        if hdr.system_id == GCS_SYSID {
            return;
        }
        let mut v = vehicle.lock().unwrap();
        v.link.messages += 1;
        match msg {
            MavMessage::HEARTBEAT(hb) => {
                if hb.mavtype == MavType::MAV_TYPE_GCS || hb.autopilot == MavAutopilot::MAV_AUTOPILOT_INVALID {
                    return;
                }
                self.target = (hdr.system_id, hdr.component_id);
                v.sysid = hdr.system_id;
                v.compid = hdr.component_id;
                let armed = hb.base_mode.contains(MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED);
                if armed != v.armed && v.have_heartbeat {
                    v.push_log("GCS", if armed { 4 } else { 6 }, if armed { "Vehicle ARMED" } else { "Vehicle disarmed" });
                }
                if armed && !v.armed {
                    v.armed_since = Some(Instant::now());
                } else if !armed && v.armed {
                    v.last_flight_secs = v.flight_secs();
                    v.armed_since = None;
                }
                if v.have_heartbeat && hb.custom_mode != v.custom_mode {
                    let m = mode_name(hb.custom_mode);
                    v.push_log("GCS", 6, format!("Mode {m}"));
                    if m != "GUIDED" {
                        v.guided_target = None;
                    }
                }
                if !v.have_heartbeat {
                    v.push_log("GCS", 6, format!("Heartbeat from system {} ({:?})", hdr.system_id, hb.mavtype));
                }
                v.have_heartbeat = true;
                v.armed = armed;
                v.custom_mode = hb.custom_mode;
                v.system_status = hb.system_status as u8;
                v.link.last_heartbeat = Some(Instant::now());
            }
            MavMessage::ATTITUDE(a) => {
                v.roll = a.roll;
                v.pitch = a.pitch;
                v.yaw = a.yaw;
                v.yaw_rate = a.yawspeed;
            }
            MavMessage::GLOBAL_POSITION_INT(p) => {
                v.have_position = p.lat != 0 || p.lon != 0;
                v.lat = p.lat as f64 * 1e-7;
                v.lon = p.lon as f64 * 1e-7;
                v.alt_msl = p.alt as f32 * 1e-3;
                v.alt_rel = p.relative_alt as f32 * 1e-3;
                v.vn = p.vx as f32 * 0.01;
                v.ve = p.vy as f32 * 0.01;
                v.vd = p.vz as f32 * 0.01;
                if p.hdg != u16::MAX {
                    v.heading = p.hdg as f32 * 0.01;
                }
                v.record_trail();
                v.record_sample();
            }
            MavMessage::VFR_HUD(h) => {
                v.airspeed = h.airspeed;
                v.groundspeed = h.groundspeed;
                v.climb = h.climb;
                v.throttle = h.throttle;
                v.record_sample();
            }
            MavMessage::SYS_STATUS(s) => {
                v.sensors_present = s.onboard_control_sensors_present.bits();
                v.sensors_enabled = s.onboard_control_sensors_enabled.bits();
                v.sensors_health = s.onboard_control_sensors_health.bits();
                v.batt_v = s.voltage_battery as f32 * 1e-3;
                // A full LiPo cell is 4.2 V, so this never undercounts a charged pack.
                let cells = (v.batt_v / 4.25).ceil() as u8;
                v.batt_cells = v.batt_cells.max(cells);
                v.batt_a = if s.current_battery >= 0 { s.current_battery as f32 * 0.01 } else { 0.0 };
                v.batt_pct = s.battery_remaining;
            }
            MavMessage::BATTERY_STATUS(b) => {
                if b.id == 0 && b.current_consumed >= 0 {
                    v.batt_mah = b.current_consumed;
                }
            }
            MavMessage::GPS_RAW_INT(g) => {
                v.gps_fix = g.fix_type as u8;
                v.sats = g.satellites_visible;
                v.hdop = if g.eph == u16::MAX { 99.9 } else { g.eph as f32 * 0.01 };
            }
            MavMessage::WIND(w) => {
                v.wind_dir = w.direction.rem_euclid(360.0);
                v.wind_speed = w.speed;
                v.wind_valid = true;
            }
            MavMessage::EKF_STATUS_REPORT(e) => {
                v.ekf_flags = e.flags.bits();
                v.ekf_vel_var = e.velocity_variance;
                v.ekf_pos_var = e.pos_horiz_variance;
                v.ekf_hgt_var = e.pos_vert_variance;
                v.ekf_mag_var = e.compass_variance;
            }
            MavMessage::NAV_CONTROLLER_OUTPUT(n) => {
                v.nav_valid = true;
                v.nav_roll = n.nav_roll;
                v.nav_pitch = n.nav_pitch;
                v.nav_bearing = n.nav_bearing as f32;
                v.target_bearing = n.target_bearing as f32;
                v.wp_dist = n.wp_dist as f32;
                v.xtrack = n.xtrack_error;
                v.alt_error = n.alt_error;
                v.aspd_error = n.aspd_error * 0.01;
            }
            MavMessage::HOME_POSITION(h) => {
                let home = (h.latitude as f64 * 1e-7, h.longitude as f64 * 1e-7, h.altitude as f32 * 1e-3);
                if v.home.is_none_or(|o| (o.0 - home.0).abs() + (o.1 - home.1).abs() > 1e-7) {
                    v.push_log("GCS", 6, format!("Home {:.6}, {:.6}, {:.0} m AMSL", home.0, home.1, home.2));
                }
                v.home = Some(home);
            }
            MavMessage::MISSION_CURRENT(m) => v.mission_seq = m.seq,
            MavMessage::RC_CHANNELS(r) => {
                v.rc = [r.chan1_raw, r.chan2_raw, r.chan3_raw, r.chan4_raw, r.chan5_raw, r.chan6_raw, r.chan7_raw, r.chan8_raw];
            }
            MavMessage::SERVO_OUTPUT_RAW(s) => {
                v.servo = [s.servo1_raw, s.servo2_raw, s.servo3_raw, s.servo4_raw, s.servo5_raw, s.servo6_raw, s.servo7_raw, s.servo8_raw];
            }
            MavMessage::STATUSTEXT(t) => {
                let raw = &t.text[..];
                // A chunk that fills all 50 bytes (no NUL) continues in the next one.
                let last_chunk = raw.contains(&0);
                let piece = String::from_utf8_lossy(raw).trim_end_matches('\0').to_string();
                let severity = t.severity as u8;
                let full = if t.id == 0 {
                    Some((severity, piece))
                } else {
                    // Reassemble ArduPilot's chunked text (same id, chunk_seq 0, 1, ...).
                    let mut buf = match self.text_chunks.take() {
                        Some((id, next, sev, text)) if id == t.id && next == t.chunk_seq => (id, next, sev, text),
                        Some((_, _, sev, text)) => {
                            // A different message started: flush what we had.
                            log_text(&mut v, sev, text);
                            (t.id, 0, severity, String::new())
                        }
                        None => (t.id, 0, severity, String::new()),
                    };
                    buf.3.push_str(&piece);
                    if last_chunk {
                        Some((buf.2, buf.3))
                    } else {
                        buf.1 = t.chunk_seq.wrapping_add(1);
                        self.text_chunks = Some(buf);
                        None
                    }
                };
                if let Some((sev, text)) = full {
                    log_text(&mut v, sev, text);
                }
            }
            MavMessage::COMMAND_ACK(a) => {
                let ok = a.result == MavResult::MAV_RESULT_ACCEPTED;
                let name = format!("{:?}", a.command).trim_start_matches("MAV_CMD_").to_string();
                let res = format!("{:?}", a.result).trim_start_matches("MAV_RESULT_").to_string();
                // Stream/home requests are routine; only report their failures.
                if !(ok && name == "REQUEST_MESSAGE") {
                    v.push_log("GCS", if ok { 6 } else { 4 }, format!("{name}: {res}"));
                }
            }
            // Mission protocol: the vehicle pulls items during an upload...
            MavMessage::MISSION_REQUEST_INT(MISSION_REQUEST_INT_DATA { seq, .. }) | MavMessage::MISSION_REQUEST(MISSION_REQUEST_DATA { seq, .. }) => {
                if let Xfer::Upload { items, last, tries, .. } = &mut self.xfer {
                    if let Some(item) = items.get(seq as usize) {
                        *last = Instant::now();
                        *tries = 0;
                        v.mission_status = format!("uploading {}/{}", seq + 1, items.len());
                        let item = item.clone();
                        drop(v);
                        self.send(MavMessage::MISSION_ITEM_INT(item));
                    }
                }
            }
            MavMessage::MISSION_ACK(a) => {
                if let Xfer::Upload { plan, .. } = &self.xfer {
                    if a.mavtype == MavMissionResult::MAV_MISSION_ACCEPTED {
                        let home = v.home.map(|h| Waypoint { kind: WpKind::Waypoint, lat: h.0, lon: h.1, alt: 0.0 });
                        v.mission = home.into_iter().chain(plan.iter().copied()).collect();
                        v.mission_status = format!("{} items on vehicle", plan.len());
                        v.push_log("GCS", 6, "Mission upload accepted");
                    } else {
                        v.mission_status = format!("rejected: {:?}", a.mavtype);
                        v.push_log("GCS", 3, format!("Mission rejected: {:?}", a.mavtype));
                    }
                    self.xfer = Xfer::Idle;
                }
            }
            // ...and pushes them during a download.
            MavMessage::MISSION_COUNT(c) => {
                if let Xfer::Download { items, next, last, tries } = &mut self.xfer {
                    *items = vec![None; c.count as usize];
                    *next = 0;
                    *last = Instant::now();
                    *tries = 0;
                    if c.count == 0 {
                        v.mission.clear();
                        v.mission_status = "no mission on vehicle".into();
                        self.xfer = Xfer::Idle;
                        drop(v);
                        self.send_mission_ack();
                    } else {
                        drop(v);
                        self.request_item(0);
                    }
                }
            }
            MavMessage::MISSION_ITEM_INT(it) => {
                if let Xfer::Download { items, next, last, tries } = &mut self.xfer {
                    if it.seq == *next && (it.seq as usize) < items.len() {
                        items[it.seq as usize] = Some(waypoint_from(&it));
                        *next += 1;
                        *last = Instant::now();
                        *tries = 0;
                        if (*next as usize) < items.len() {
                            let n = *next;
                            drop(v);
                            self.request_item(n);
                        } else {
                            v.mission = items.iter().flatten().copied().collect();
                            v.mission_status = format!("{} items on vehicle", v.mission.len().saturating_sub(1));
                            self.xfer = Xfer::Idle;
                            drop(v);
                            self.send_mission_ack();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn send_mission_ack(&self) {
        self.send(MavMessage::MISSION_ACK(MISSION_ACK_DATA {
            target_system: self.target.0,
            target_component: self.target.1,
            mavtype: MavMissionResult::MAV_MISSION_ACCEPTED,
            ..Default::default()
        }));
    }
}

fn log_text(v: &mut Vehicle, severity: u8, text: String) {
    let text = text.trim().to_string();
    if text.starts_with("PreArm") || text.starts_with("Arm:") {
        v.prearm = Some((Instant::now(), text.clone()));
    }
    v.push_log("FC", severity, text);
}

fn mission_item(seq: u16, wp: &Waypoint, target: (u8, u8)) -> MISSION_ITEM_INT_DATA {
    let (command, p1) = match wp.kind {
        WpKind::Takeoff => (MavCmd::MAV_CMD_NAV_TAKEOFF, 12.0), // param1: min pitch
        WpKind::Waypoint => (MavCmd::MAV_CMD_NAV_WAYPOINT, 0.0),
        WpKind::LoiterUnlim => (MavCmd::MAV_CMD_NAV_LOITER_UNLIM, 0.0),
        WpKind::LoiterTurns => (MavCmd::MAV_CMD_NAV_LOITER_TURNS, 2.0),
        WpKind::Land => (MavCmd::MAV_CMD_NAV_LAND, 0.0),
        WpKind::Rtl => (MavCmd::MAV_CMD_NAV_RETURN_TO_LAUNCH, 0.0),
        WpKind::Other(_) => (MavCmd::MAV_CMD_NAV_WAYPOINT, 0.0),
    };
    MISSION_ITEM_INT_DATA {
        param1: p1,
        param2: 0.0,
        param3: 0.0,
        param4: 0.0,
        x: (wp.lat * 1e7) as i32,
        y: (wp.lon * 1e7) as i32,
        z: wp.alt,
        seq,
        command,
        target_system: target.0,
        target_component: target.1,
        frame: MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT,
        current: 0,
        autocontinue: 1,
        ..Default::default()
    }
}

fn waypoint_from(it: &MISSION_ITEM_INT_DATA) -> Waypoint {
    let kind = match it.command {
        MavCmd::MAV_CMD_NAV_TAKEOFF => WpKind::Takeoff,
        MavCmd::MAV_CMD_NAV_WAYPOINT => WpKind::Waypoint,
        MavCmd::MAV_CMD_NAV_LOITER_UNLIM => WpKind::LoiterUnlim,
        MavCmd::MAV_CMD_NAV_LOITER_TURNS => WpKind::LoiterTurns,
        MavCmd::MAV_CMD_NAV_LAND => WpKind::Land,
        MavCmd::MAV_CMD_NAV_RETURN_TO_LAUNCH => WpKind::Rtl,
        c => WpKind::Other(c as u16),
    };
    Waypoint { kind, lat: it.x as f64 * 1e-7, lon: it.y as f64 * 1e-7, alt: it.z }
}
