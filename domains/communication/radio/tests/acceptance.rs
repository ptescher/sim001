//! The "Other aircraft" section of the suite (expectations/04-communication), run against the
//! communication domain with no window: a simulated radio medium, the TX15's downlink and the
//! station's tracker. The same steps are what the console shows on its map.

use communication_radio::rig::FIELD;
use communication_radio::{Rig, Sending};
use communication_traffic::Freshness;
use expectation_harness::{Acceptance, Bindings, RunOptions, Screenshot};
use expectation_story::Status;
use std::path::Path;

/// A rig, and the warm-up that a Given about the map asks for. The warm-up runs when the
/// starting state is complete (`settle`), so that the order of the Givens does not matter.
struct World {
    rig: Rig,
    warm: Option<Warm>,
}

#[derive(Clone, Copy)]
enum Warm {
    Live,
    Stale,
}

impl World {
    fn new() -> World {
        World { rig: Rig::new(), warm: None }
    }

    fn settle(&mut self) {
        match self.warm.take() {
            Some(Warm::Live) => {
                let keep = self.rig.sending();
                if keep == Sending::Silent {
                    self.rig.set_sending(Sending::Mavlink);
                }
                self.rig.run(5.0);
                self.rig.set_sending(keep);
            }
            Some(Warm::Stale) => {
                let keep = self.rig.sending();
                self.rig.set_sending(Sending::Mavlink);
                self.rig.run(5.0);
                self.rig.set_sending(Sending::Silent);
                self.rig.run(10.0);
                self.rig.set_sending(keep);
            }
            None => {}
        }
    }

    fn marker(&self) -> &'static str {
        match self.rig.other() {
            None => "missing",
            Some(t) if t.freshness == Freshness::Live => "live",
            Some(_) => "stale",
        }
    }

    fn source(&self) -> &'static str {
        match self.rig.other().map(|t| t.source_label()).as_deref() {
            Some("RID") => "Remote ID",
            Some("MAV") => "MAVLink",
            Some("MAV RID") => "Remote ID and MAVLink",
            _ => "none",
        }
    }
}

/// The tactical map as the station draws it, from above, with the pilot's plane in the middle.
fn map_svg(w: &World) -> String {
    use std::fmt::Write;
    let tracks = w.rig.tracks();
    let reach = tracks
        .iter()
        .map(|t| {
            let (n, e) = FIELD.offset_to(&t.position);
            n.hypot(e)
        })
        .fold(400.0f32, f32::max)
        * 1.15;
    let scale = 150.0 / reach;
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Helvetica, Arial, sans-serif" font-size="12">
<rect width="640" height="360" fill="#14181c"/>"##
    );
    for ring in [0.33f32, 0.66, 1.0] {
        let _ = write!(
            s,
            r##"<circle cx="320" cy="180" r="{:.0}" fill="none" stroke="#2c343b"/><text x="{:.0}" y="176" fill="#6b7782">{:.0} m</text>"##,
            150.0 * ring,
            324.0 + 150.0 * ring,
            reach * ring
        );
    }
    let _ = write!(
        s,
        r##"<path d="M320 168 L328 188 L320 184 L312 188 Z" fill="#52c76a" stroke="#000"/><text x="334" y="196" fill="#52c76a">YOU</text>"##
    );
    for t in &tracks {
        let (n, e) = FIELD.offset_to(&t.position);
        let (x, y) = (320.0 + e * scale, 180.0 - n * scale);
        let colour = if t.freshness == communication_traffic::Freshness::Live { "#4cc3e0" } else { "#7d8791" };
        let _ = write!(
            s,
            r##"<path d="M{x:.0} {:.0} L{:.0} {:.0} L{x:.0} {:.0} L{:.0} {:.0} Z" fill="{colour}" stroke="#000"/><text x="{:.0}" y="{:.0}" fill="{colour}">{}</text><text x="{:.0}" y="{:.0}" fill="{colour}">{}</text>"##,
            y - 9.0,
            x + 7.0,
            y + 7.0,
            y + 3.0,
            x - 7.0,
            y + 7.0,
            x + 12.0,
            y - 2.0,
            t.name,
            x + 12.0,
            y + 12.0,
            t.source_label()
        );
    }
    let _ = write!(
        s,
        r##"<text x="12" y="22" fill="#d8dde1">TACTICAL SITUATION</text><text x="12" y="344" fill="{}">LINK {}</text></svg>"##,
        if w.rig.link_up() { "#52c76a" } else { "#e5533d" },
        if w.rig.link_up() { "UP" } else { "LOST" }
    );
    s
}

fn sending_of(v: &expectation_model::Value) -> Sending {
    match v.as_str() {
        Some("sending Remote ID") => Sending::RemoteId,
        Some("sending MAVLink") => Sending::Mavlink,
        Some("sending both") => Sending::Both,
        _ => Sending::Silent,
    }
}

fn driver() -> Bindings<World> {
    Bindings::new(World::new())
        .observe("the other plane", |w| match w.rig.sending() {
            Sending::Silent => "silent",
            Sending::RemoteId => "sending Remote ID",
            Sending::Mavlink => "sending MAVLink",
            Sending::Both => "sending both",
        })
        .observe("the distance to the other plane", |w| w.rig.distance() as f64)
        .observe("the map", |w| w.rig.tracks().len() as f64)
        .observe("the marker of the other plane", |w| w.marker())
        .observe("the source of the marker", |w| w.source())
        .observe("the TX15 link", |w| if w.rig.link_up() { "up" } else { "lost" })
        .observe("your Remote ID", |w| if w.rig.own_remote_id() { "on" } else { "off" })
        .observe("the crowd", |w| w.rig.crowd() as f64)
        .observe("the telemetry of your plane", |w| w.rig.own_position_rate() as f64)
        .set("the other plane", |w, v| w.rig.set_sending(sending_of(v)))
        .set("the distance to the other plane", |w, v| w.rig.set_distance(v.as_f64().unwrap() as f32))
        .set("the TX15 link", |w, v| w.rig.set_link(v.as_str() == Some("up")))
        .set("your Remote ID", |w, v| w.rig.set_own_remote_id(v.as_str() == Some("on")))
        .set("the crowd", |w, v| w.rig.set_crowd(v.as_f64().unwrap() as usize))
        // States that the pilot cannot set, only find: the Givens arrange them.
        .arrange("the marker of the other plane", |w, v| {
            w.warm = match v.as_str() {
                Some("live") => Some(Warm::Live),
                Some("stale") => Some(Warm::Stale),
                _ => None,
            }
        })
        .arrange("the map", |w, v| {
            if v.as_f64().is_some_and(|n| n > 0.0) {
                w.warm = Some(Warm::Live);
            }
        })
        .arrange("the source of the marker", |w, v| {
            let s = match v.as_str() {
                Some("Remote ID") => Sending::RemoteId,
                Some("MAVLink") => Sending::Mavlink,
                Some("Remote ID and MAVLink") => Sending::Both,
                _ => Sending::Silent,
            };
            w.rig.set_sending(s);
            w.warm = (s != Sending::Silent).then_some(Warm::Live);
        })
        .arrange("the telemetry of your plane", |_, _| {})
        .on("5 seconds pass", |w| w.rig.run(5.0))
        .on("10 seconds pass", |w| w.rig.run(10.0))
        .on("70 seconds pass", |w| w.rig.run(70.0))
        .on_reset(|w| *w = World::new())
        .on_settle(World::settle)
        .on_screenshot(|w| Screenshot::svg(map_svg(w)))
}

#[test]
fn every_test_of_the_section_passes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    let section = a.suite.sections.iter().find(|s| s.slug == "communication").expect("a communication section").id;
    let ours = a.suite.subtree(section);
    let report = a.run(&mut driver(), &RunOptions::default());
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {}", r.status, p.title);
    }
    assert!(ran.len() >= 4, "only {} tests in the section", ran.len());
    let bad: Vec<_> = ran
        .iter()
        .filter(|(_, r)| r.status != Status::Passed)
        .map(|(p, r)| format!("{}: {:?}", p.title, r.status))
        .collect();
    assert!(bad.is_empty(), "{}\n{}", bad.join("\n"), report.failures(&a.suite, &a.plans));
}

#[test]
fn the_section_documents_itself_with_pictures_of_the_map() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    let a = Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage();
    // With SIM001_DOCS=DIR, the results and pictures go into the project's docs, beside the other runners'.
    let published = std::env::var("SIM001_DOCS").ok().map(std::path::PathBuf::from);
    let out = published.clone().unwrap_or_else(|| Path::new(env!("CARGO_TARGET_TMPDIR")).join("communication_docs"));
    let report = a.run(&mut driver(), &RunOptions { docs_dir: Some(out.clone()), ..Default::default() });
    if published.is_some() {
        a.publish(&out, "communication", &report).unwrap();
    } else {
        a.docs(Some(&report)).write(&out).unwrap();
    }
    let section = std::fs::read_dir(out.join("screenshots"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains("friend"));
    assert!(section, "no pictures of the friend story in {}", out.display());
}
