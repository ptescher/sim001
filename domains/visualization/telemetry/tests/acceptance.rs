//! The "Watching a real plane" section of the suite (expectations/05-visualization), run with no
//! window: a plane that sends MAVLink as ArduPilot does, the bytes through the `Watcher`, and
//! the placement that the view draws.

use communication_telemetry::frame;
use expectation_harness::{Acceptance, Bindings, RunOptions};
use expectation_story::Status;
use glam::Vec3;
use mavlink::dialects::common::*;
use std::path::Path;
use visualization_telemetry::{Home, Watcher};

const HOME: Home = Home { lat: -35.363261, lon: 149.165230, alt: 584.0 };
/// Metres per degree of latitude, as the plane's GPS turns a position into degrees.
const M_PER_DEG: f64 = 6_371_000.0 * std::f64::consts::PI / 180.0;

/// The real plane, and the view that listens to it.
struct World {
    watcher: Watcher,
    flowing: bool,
    north: f32,
    height: f32,
    bank: f32,
    heading: &'static str,
    t: f64,
    seq: u8,
}

impl World {
    fn new() -> World {
        World { watcher: Watcher::new(None), flowing: false, north: 0.0, height: 0.0, bank: 0.0, heading: "north", t: 0.0, seq: 0 }
    }

    fn yaw(&self) -> f32 {
        match self.heading {
            "east" => 90f32,
            "south" => 180.0,
            "west" => 270.0,
            _ => 0.0,
        }
        .to_radians()
    }

    fn send(&mut self, msg: MavMessage) {
        self.seq = self.seq.wrapping_add(1);
        self.watcher.push(&frame(1, 1, self.seq, &msg));
    }

    /// Time passes; while the telemetry flows, ArduPilot sends its attitude and position at
    /// 10 Hz, and home once a second.
    fn run(&mut self, seconds: f64) {
        let end = self.t + seconds;
        while self.t < end {
            self.t += 0.1;
            if !self.flowing {
                continue;
            }
            let ms = (self.t * 1000.0) as u32;
            if (self.t.fract()) < 0.1 {
                self.send(MavMessage::HOME_POSITION(HOME_POSITION_DATA {
                    latitude: (HOME.lat * 1e7) as i32,
                    longitude: (HOME.lon * 1e7) as i32,
                    altitude: (HOME.alt * 1000.0) as i32,
                    ..Default::default()
                }));
            }
            self.send(MavMessage::ATTITUDE(ATTITUDE_DATA {
                time_boot_ms: ms,
                roll: self.bank.to_radians(),
                pitch: 0.0,
                yaw: self.yaw(),
                ..Default::default()
            }));
            let lat = HOME.lat + self.north as f64 / M_PER_DEG;
            self.send(MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
                time_boot_ms: ms,
                lat: (lat * 1e7).round() as i32,
                lon: (HOME.lon * 1e7) as i32,
                alt: ((HOME.alt + self.height) * 1000.0) as i32,
                relative_alt: (self.height * 1000.0) as i32,
                hdg: (self.yaw().to_degrees() * 100.0) as u16,
                ..Default::default()
            }));
        }
    }

    fn placed(&self) -> Option<visualization_telemetry::Placement> {
        self.watcher.placement()
    }
}

/// The compass point the nose of the plane in the view points to.
fn heading_of(rot: glam::Quat) -> &'static str {
    let nose = rot * Vec3::NEG_Z;
    let deg = nose.x.atan2(-nose.z).to_degrees().rem_euclid(360.0);
    ["north", "east", "south", "west"][(((deg + 45.0) / 90.0) as usize) % 4]
}

fn driver() -> Bindings<World> {
    Bindings::new(World::new())
        .observe("the telemetry", |w| if w.flowing { "flowing" } else { "silent" })
        .observe("the reported distance north of home", |w| w.north as f64)
        .observe("the reported height", |w| w.height as f64)
        .observe("the reported bank", |w| w.bank as f64)
        .observe("the reported heading", |w| w.heading)
        .observe("the plane in the view", |w| if w.placed().is_some() { "shown" } else { "missing" })
        .observe("the distance north of the pilot box in the view", |w| w.placed().map_or(f64::NAN, |p| (-p.offset.z).round() as f64))
        .observe("the height above the pilot box in the view", |w| w.placed().map_or(f64::NAN, |p| p.offset.y.round() as f64))
        .observe("the right wing in the view", |w| match w.placed().map(|p| (p.rot * Vec3::X).y) {
            Some(y) if y < -0.1 => "low",
            Some(y) if y > 0.1 => "high",
            _ => "level",
        })
        .observe("the heading in the view", |w| w.placed().map_or("none", |p| heading_of(p.rot)))
        .set("the telemetry", |w, v| w.flowing = v.as_str() == Some("flowing"))
        .set("the reported distance north of home", |w, v| w.north = v.as_f64().unwrap() as f32)
        .set("the reported height", |w, v| w.height = v.as_f64().unwrap() as f32)
        .set("the reported bank", |w, v| w.bank = v.as_f64().unwrap() as f32)
        .set("the reported heading", |w, v| {
            w.heading = match v.as_str() {
                Some("east") => "east",
                Some("south") => "south",
                Some("west") => "west",
                _ => "north",
            }
        })
        // Found, not set: a plane in the view is one that has sent its telemetry for a while.
        .arrange("the plane in the view", |w, v| {
            if v.as_str() == Some("shown") {
                let keep = w.flowing;
                w.flowing = true;
                w.run(1.0);
                w.flowing = keep;
            }
        })
        .on("1 second passes", |w| w.run(1.0))
        .on("10 seconds pass", |w| w.run(10.0))
        .on_reset(|w| *w = World::new())
}

#[test]
fn every_test_of_the_section_passes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    let section = a.suite.sections.iter().find(|s| s.slug == "visualization").expect("a visualization section").id;
    let ours = a.suite.subtree(section);
    let report = a.run(&mut driver(), &RunOptions::default());
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {}", r.status, p.title);
    }
    assert!(!ran.is_empty(), "only {} tests in the section", ran.len());
    let bad: Vec<_> = ran.iter().filter(|(_, r)| r.status != Status::Passed).map(|(p, r)| format!("{}: {:?}", p.title, r.status)).collect();
    assert!(bad.is_empty(), "{}\n{}", bad.join("\n"), report.failures(&a.suite, &a.plans));
}

/// With SIM001_DOCS=DIR, the results go into the project's docs, beside the other runners'.
#[test]
fn the_section_publishes_its_results_when_asked() {
    let Ok(docs) = std::env::var("SIM001_DOCS") else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    let docs = std::path::PathBuf::from(docs);
    let report = a.run(&mut driver(), &RunOptions { docs_dir: Some(docs.clone()), ..Default::default() });
    a.publish(&docs, "visualization", &report).unwrap();
}
