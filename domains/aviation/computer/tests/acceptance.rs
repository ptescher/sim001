//! The "flight computer" section of the suite (expectations/06-aviation), run with no window:
//! a TX15's end of a serial line, the flight computer, an autopilot that reports its state and
//! a ground station that only hears what comes down the line.

use aviation_computer::{FlightComputer, Handset, Vehicle};
use communication_telemetry::{MavMessage, Parser, frame};
use expectation_harness::{Acceptance, Bindings, RunOptions};
use expectation_story::Status;
use mavlink::dialects::common::*;
use std::path::Path;

const HOME_LAT: f64 = -35.363261;
const STEP: f64 = 0.005;

struct World {
    t: f64,
    handset: Handset,
    fc: FlightComputer,
    vehicle: Vehicle,
    /// ArduPilot sends MAVLink.
    telemetry: bool,
    north: f32,
    seq: u8,
    next_heartbeat: f64,
    next_position: f64,
    /// The ground station's ears.
    station: Parser,
    last_heartbeat: Option<f64>,
    station_lat: Option<f64>,
    station_bytes: usize,
    started: f64,
    /// ArduPilot's ears.
    autopilot: Parser,
    command_received: bool,
    warm: bool,
    /// ArduPilot's last STATUSTEXT, sent down the line once the TX15 is talking.
    message: Option<String>,
    message_sent: bool,
}

/// A STATUSTEXT as ArduPilot sends it.
fn status_text(seq: u8, text: &str) -> Vec<u8> {
    let mut chars = [0u8; 50];
    chars[..text.len().min(50)].copy_from_slice(&text.as_bytes()[..text.len().min(50)]);
    let severity = if text.starts_with("PreArm") || text.starts_with("Failsafe") {
        MavSeverity::MAV_SEVERITY_CRITICAL
    } else {
        MavSeverity::MAV_SEVERITY_INFO
    };
    frame(1, 1, seq, &MavMessage::STATUSTEXT(STATUSTEXT_DATA { severity, text: chars.into(), id: 0, chunk_seq: 0 }))
}

impl World {
    fn new() -> World {
        World {
            t: 0.0,
            handset: Handset::new(),
            fc: FlightComputer::new(),
            vehicle: Vehicle { mode: "MANUAL".into(), volts: 12.0, percent: 90, lat: HOME_LAT, ..Default::default() },
            telemetry: false,
            north: 0.0,
            seq: 0,
            next_heartbeat: 0.0,
            next_position: 0.0,
            station: Parser::default(),
            last_heartbeat: None,
            station_lat: None,
            station_bytes: 0,
            started: 0.0,
            autopilot: Parser::default(),
            command_received: false,
            warm: true,
            message: None,
            message_sent: false,
        }
    }

    fn step(&mut self) {
        self.t += STEP;
        let t = self.t;
        // The TX15 to the flight computer, and what the flight computer gives ArduPilot.
        let up = self.handset.to_line(t);
        self.fc.from_line(&up, t);
        let for_ap = self.fc.take_for_autopilot();
        self.autopilot.push(&for_ap);
        while let Some((_, m)) = self.autopilot.next_message() {
            if matches!(m, MavMessage::COMMAND_LONG(_)) {
                self.command_received = true;
            }
        }
        // ArduPilot's MAVLink.
        self.vehicle.lat = HOME_LAT + self.north as f64 / 111_320.0;
        if self.telemetry {
            if t >= self.next_heartbeat {
                self.next_heartbeat = t + 1.0;
                let hb = MavMessage::HEARTBEAT(HEARTBEAT_DATA {
                    custom_mode: 0,
                    mavtype: MavType::MAV_TYPE_FIXED_WING,
                    autopilot: MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
                    base_mode: MavModeFlag::empty(),
                    system_status: MavState::MAV_STATE_ACTIVE,
                    mavlink_version: 3,
                });
                self.seq = self.seq.wrapping_add(1);
                self.fc.from_autopilot(&frame(1, 1, self.seq, &hb));
            }
            if t >= self.next_position {
                self.next_position = t + 0.1;
                let pos = MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
                    time_boot_ms: (t * 1000.0) as u32,
                    lat: (self.vehicle.lat * 1e7) as i32,
                    lon: 1_491_652_300,
                    alt: 584_000,
                    relative_alt: 100_000,
                    vx: 0,
                    vy: 0,
                    vz: 0,
                    hdg: 0,
                });
                self.seq = self.seq.wrapping_add(1);
                self.fc.from_autopilot(&frame(1, 1, self.seq, &pos));
            }
        }
        // What ArduPilot says, once the line is up.
        if let (Some(text), false) = (&self.message, self.message_sent)
            && t > 0.5
        {
            self.seq = self.seq.wrapping_add(1);
            self.fc.from_autopilot(&status_text(self.seq, text));
            self.message_sent = true;
        }
        // The flight computer to the TX15, and the TX15 to the ground station.
        let down = self.fc.to_line(&self.vehicle, t);
        self.handset.from_line(&down, t);
        let to_station = self.handset.take_mavlink();
        self.station_bytes += to_station.len();
        self.station.push(&to_station);
        while let Some((_, m)) = self.station.next_message() {
            match m {
                MavMessage::HEARTBEAT(_) => self.last_heartbeat = Some(t),
                MavMessage::GLOBAL_POSITION_INT(p) => self.station_lat = Some(p.lat as f64 / 1e7),
                _ => {}
            }
        }
    }

    fn run(&mut self, seconds: f64) {
        let end = self.t + seconds;
        while self.t < end {
            self.step();
        }
    }

    /// The warm-up that a test starts from: the TX15 has been talking for a second.
    fn settle(&mut self) {
        if std::mem::take(&mut self.warm) {
            self.run(1.0);
            self.started = self.t;
            // A radio keeps the last telemetry it showed. With no link yet, that is what the
            // pilot arranged.
            if self.handset.screen.last_frame.is_none() {
                let s = &mut self.handset.screen;
                s.mode = self.vehicle.mode.clone();
                s.armed = self.vehicle.armed;
                s.volts = self.vehicle.volts;
                s.roll = self.vehicle.roll.to_degrees();
                s.airspeed = self.vehicle.airspeed;
                s.climb = self.vehicle.climb;
                s.sats = self.vehicle.sats;
            }
        }
    }

    fn us(&self, i: usize) -> f64 {
        self.fc.channels(self.t)[i] as f64
    }
}

/// To a tenth, as the screen of the TX15 shows a reading.
fn tenth(x: f32) -> f64 {
    (x as f64 * 10.0).round() / 10.0
}

fn stick(pct: f64) -> u16 {
    (1500.0 + 5.0 * pct).round() as u16
}

fn driver() -> Bindings<World> {
    Bindings::new(World::new())
        .observe("the TX15", |w| if w.handset.connected() { "connected" } else { "not connected" })
        .observe("the TX15 roll stick", |w| (w.handset.channels[0] as f64 - 1500.0) / 5.0)
        .observe("the TX15 pitch stick", |w| (w.handset.channels[1] as f64 - 1500.0) / 5.0)
        .observe("the TX15 throttle stick", |w| (w.handset.channels[2] as f64 - 1000.0) / 10.0)
        .observe("the TX15 arm switch", |w| if w.handset.channels[4] > 1500 { "on" } else { "off" })
        .observe("the corrupted frames", |w| w.handset.corrupt as f64 * 100.0)
        .observe("the roll channel", |w| w.us(0))
        .observe("the pitch channel", |w| w.us(1))
        .observe("the throttle channel", |w| w.us(2))
        .observe("the arm channel", |w| w.us(4))
        .observe(
            "the TX15 screen mode",
            |w| if w.warm { w.vehicle.mode.clone() } else { w.handset.screen.mode.clone() },
        )
        .observe("the TX15 screen arming", |w| {
            if if w.warm { w.vehicle.armed } else { w.handset.screen.armed } { "armed" } else { "disarmed" }
        })
        .observe("the TX15 screen battery", |w| tenth(if w.warm { w.vehicle.volts } else { w.handset.screen.volts }))
        .observe("the TX15 screen bank", |w| {
            // (Degrees to radians and back is not exact: the pilot reads whole tenths.)
            let deg = if w.warm { w.vehicle.roll.to_degrees() } else { w.handset.screen.roll };
            tenth(deg)
        })
        .observe("the TX15 screen airspeed", |w| {
            tenth(if w.warm { w.vehicle.airspeed } else { w.handset.screen.airspeed })
        })
        .observe("the TX15 screen climb", |w| tenth(if w.warm { w.vehicle.climb } else { w.handset.screen.climb }))
        .observe("the TX15 screen satellites", |w| {
            if w.warm { w.vehicle.sats as f64 } else { w.handset.screen.sats as f64 }
        })
        .observe("the TX15 screen message", |w| {
            if w.warm {
                w.message.clone().unwrap_or_else(|| "none".into())
            } else {
                w.handset.screen.messages.last().map_or("none".to_string(), |m| m.text.clone())
            }
        })
        .observe("the TX15 uplink quality", |w| {
            if w.warm { if w.handset.connected() { 100.0 } else { 0.0 } } else { w.handset.quality(w.t) as f64 }
        })
        .observe("the ground station telemetry", |w| {
            if if w.warm { w.telemetry } else { w.last_heartbeat.is_some_and(|h| w.t - h < 3.0) } {
                "flowing"
            } else {
                "silent"
            }
        })
        .observe("the ground station map distance", |w| {
            if w.warm { w.north as f64 } else { w.station_lat.map_or(0.0, |l| ((l - HOME_LAT) * 111_320.0).round()) }
        })
        .observe("the ground station telemetry rate", |w| w.station_bytes as f64 / (w.t - w.started).max(1.0))
        .observe("the autopilot command", |w| if w.command_received { "received" } else { "not received" })
        .set("the TX15", |w, v| w.handset.set_connected(v.as_str() == Some("connected")))
        .set("the TX15 roll stick", |w, v| w.handset.channels[0] = stick(v.as_f64().unwrap()))
        .set("the TX15 pitch stick", |w, v| w.handset.channels[1] = stick(v.as_f64().unwrap()))
        .set("the TX15 throttle stick", |w, v| {
            w.handset.channels[2] = (1000.0 + 10.0 * v.as_f64().unwrap()).round() as u16
        })
        .set("the TX15 arm switch", |w, v| w.handset.channels[4] = if v.as_str() == Some("on") { 1900 } else { 1100 })
        .set("the corrupted frames", |w, v| w.handset.corrupt = v.as_f64().unwrap() as f32 / 100.0)
        .set("the TX15 screen mode", |w, v| w.vehicle.mode = v.as_str().unwrap().to_string())
        .set("the TX15 screen arming", |w, v| w.vehicle.armed = v.as_str() == Some("armed"))
        .set("the TX15 screen battery", |w, v| w.vehicle.volts = v.as_f64().unwrap() as f32)
        .set("the TX15 screen bank", |w, v| w.vehicle.roll = (v.as_f64().unwrap() as f32).to_radians())
        .set("the TX15 screen airspeed", |w, v| w.vehicle.airspeed = v.as_f64().unwrap() as f32)
        .set("the TX15 screen climb", |w, v| w.vehicle.climb = v.as_f64().unwrap() as f32)
        .set("the TX15 screen satellites", |w, v| w.vehicle.sats = v.as_f64().unwrap() as u8)
        .set("the TX15 screen message", |w, v| {
            w.message = v.as_str().filter(|m| *m != "none").map(|m| m.to_string());
            w.message_sent = false;
        })
        .set("the ground station telemetry", |w, v| w.telemetry = v.as_str() == Some("flowing"))
        .set("the ground station map distance", |w, v| w.north = v.as_f64().unwrap() as f32)
        .set("the ground station command", |w, v| {
            if v.as_str() == Some("sent") {
                let cmd = MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
                    param1: 1.0,
                    param2: 0.0,
                    param3: 0.0,
                    param4: 0.0,
                    param5: 0.0,
                    param6: 0.0,
                    param7: 0.0,
                    command: MavCmd::MAV_CMD_COMPONENT_ARM_DISARM,
                    target_system: 1,
                    target_component: 1,
                    confirmation: 0,
                });
                w.handset.send_mavlink(&frame(255, 190, 0, &cmd));
            }
        })
        // States that the pilot cannot set, only find: the Givens arrange them.
        .arrange("the autopilot command", |_, _| {})
        .arrange("the ground station telemetry rate", |_, _| {})
        .arrange("the TX15 uplink quality", |_, _| {})
        .on("5 seconds pass", |w| w.run(5.0))
        .on("10 seconds pass", |w| w.run(10.0))
        .on("70 seconds pass", |w| w.run(70.0))
        .on_reset(|w| *w = World::new())
        .on_settle(World::settle)
        // The screen of the TX15, as the widget draws it (WIDGETS/SIM001FC).
        .on_screenshot(|w| expectation_harness::Screenshot::svg(w.handset.screen.to_svg()))
}

#[test]
fn every_test_of_the_section_passes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    let section = a.suite.sections.iter().find(|s| s.slug == "aviation").expect("an aviation section").id;
    let ours = a.suite.subtree(section);
    let report = a.run(&mut driver(), &RunOptions::default());
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {} {:?}", r.status, p.title, r.start.error);
    }
    println!("{}", report.failures(&a.suite, &a.plans));
    assert!(ran.len() >= 3, "only {} tests in the section", ran.len());
    let bad: Vec<_> = ran
        .iter()
        .filter(|(_, r)| r.status != Status::Passed)
        .map(|(p, r)| format!("{}: {:?}", p.title, r.status))
        .collect();
    assert!(bad.is_empty(), "{}\n{}", bad.join("\n"), report.failures(&a.suite, &a.plans));
}

/// The same, with pictures of the TX15's screen. With SIM001_DOCS=DIR the results and pictures go
/// into the project's docs, beside the other runners'.
#[test]
fn the_section_documents_itself_with_pictures_of_the_screen() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    let published = std::env::var("SIM001_DOCS").ok().map(std::path::PathBuf::from);
    let out = published.clone().unwrap_or_else(|| Path::new(env!("CARGO_TARGET_TMPDIR")).join("aviation_docs"));
    let report = a.run(&mut driver(), &RunOptions { docs_dir: Some(out.clone()), ..Default::default() });
    if published.is_some() {
        a.publish(&out, "aviation", &report).unwrap();
    } else {
        a.docs(Some(&report)).write(&out).unwrap();
    }
    let any = std::fs::read_dir(out.join("screenshots"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains("flight-display"));
    assert!(any, "no pictures of the flight display in {}", out.display());
}
