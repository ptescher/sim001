//! `--console`: after the game, the first plane (Kestrel, MAVLink system 1) is yours. The physics
//! goes on, the ground control agent stops, and you send plans by hand, one command a line:
//!
//! | Command | Does |
//! | --- | --- |
//! | `mode AGENT` or `mode FBWA` (or a number) | switches the flight mode, and waits until the heartbeat shows it |
//! | `plan PRIMITIVE NORTH EAST ALT RADIUS TTL [TARGET]` | sends a plan (`COMMAND_INT` 31010) with the id after the one the plane reports, and waits for its `COMMAND_ACK` |
//! | `again PRIMITIVE ...` | the same, with the id of the plan the plane reports |
//! | `wait SECONDS` | lets the time of the planes run on |
//! | `watch SECONDS` | the same, and counts the `AG_PLAN` reports that come |
//! | `status` | the latest status |
//! | `silent` / `heard` | stops / starts again the tracks of the other planes (`ADSB_VEHICLE`, sent twice a second) |
//! | `reset` | `mode FBWA`, then `mode AGENT`: the plane forgets its plan |
//! | `quit` | stops the planes (so does the end of the input) |
//!
//! PRIMITIVE is `pursue`, `evade`, `goto`, `orbit`, `band`, `loiter` or a number (`autonomy_plan::
//! Primitive`); NORTH and EAST are metres from home, ALT metres above home, RADIUS metres (the
//! thickness for a band), TTL seconds; TARGET is the system id a pursue or an evade plan names
//! (the other planes are 2, Merlin, and 3, Swift; the console sends them to this plane as tracks).
//! Every command answers with a `status` line, and a plan first
//! with an `ack` line: `result` is the `MAV_RESULT` (0 accepted, 2 denied), `reason` the AGENT
//! mode's reason for a refusal (1 not in AGENT, 2 malformed, 3 no home, 4 no executor).

use autonomy_agent::link::{Link, command_int};
use autonomy_plan::{CMD_PLAN, Primitive, WirePlan, to_global};
use simulation_sitl::launch::{AGENT_MODE, plane_mode_name};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// FBWA's custom mode number.
const FBWA: u32 = 5;
/// After a command, the time of the planes to wait for the next status (it comes once a second).
const STATUS_WAIT: f64 = 1.5;

pub struct Console<'a> {
    link: &'a Link,
    /// Whether the other planes are sent to this one as ADSB_VEHICLE (`silent`, `heard`).
    uplink: &'a AtomicBool,
    /// The time of the planes (s).
    now: Box<dyn Fn() -> f64 + 'a>,
    home: (f64, f64),
    /// `texts_heard` at the last reset: the messages since then.
    texts_from: u64,
    recoveries: u32,
}

impl<'a> Console<'a> {
    pub fn new(link: &'a Link, home: (f64, f64), uplink: &'a AtomicBool, now: impl Fn() -> f64 + 'a) -> Console<'a> {
        let texts_from = link.state().texts_heard;
        Console { link, uplink, now: Box::new(now), home, texts_from, recoveries: 0 }
    }

    /// Reads commands until `quit` or the end of the input.
    pub fn run(&mut self, input: impl BufRead, mut out: impl Write) {
        let _ = writeln!(out, "console ready");
        let _ = out.flush();
        for line in input.lines() {
            let Ok(line) = line else { break };
            let words: Vec<&str> = line.split_whitespace().collect();
            let Some(&command) = words.first() else { continue };
            let reply = match command {
                "quit" | "exit" => break,
                "mode" => words.get(1).and_then(|m| mode_number(m)).map_or_else(
                    || Err("say `mode AGENT` or `mode FBWA`".to_string()),
                    |m| self.mode(m).map(|_| vec![]),
                ),
                "reset" => self.mode(FBWA).and_then(|_| self.mode(AGENT_MODE)).map(|_| {
                    self.texts_from = self.link.state().texts_heard;
                    self.recoveries = 0;
                    vec![]
                }),
                "plan" | "again" => self.plan(&words[1..], command == "again"),
                "wait" | "watch" => match words.get(1).and_then(|s| s.parse::<f64>().ok()) {
                    Some(secs) => {
                        let before = self.link.state().status_reports;
                        self.wait(secs);
                        let reports = self.link.state().status_reports - before;
                        Ok(if command == "watch" { vec![format!("reports {reports}")] } else { vec![] })
                    }
                    None => Err(format!("say `{command} SECONDS`")),
                },
                "status" => Ok(vec![]),
                "silent" | "heard" => {
                    self.uplink.store(command == "heard", Ordering::Relaxed);
                    Ok(vec![format!("tracks {}", if command == "heard" { "sent" } else { "stopped" })])
                }
                _ => Err(format!(
                    "unknown command `{command}`: mode, plan, again, wait, watch, status, silent, heard, reset, quit"
                )),
            };
            match reply {
                Ok(lines) => {
                    for l in lines {
                        let _ = writeln!(out, "{l}");
                    }
                }
                Err(e) => {
                    let _ = writeln!(out, "error {e}");
                }
            }
            let _ = writeln!(out, "{}", self.status());
            let _ = out.flush();
        }
    }

    /// Waits for `secs` of the planes' time (and at most 30 s of wall time more than that).
    fn wait(&self, secs: f64) {
        let end = (self.now)() + secs;
        let wall = Instant::now();
        while (self.now)() < end && wall.elapsed() < Duration::from_secs_f64(secs + 30.0) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn mode(&mut self, mode: u32) -> Result<(), String> {
        self.link.set_mode(mode).map_err(|e| e.to_string())?;
        let wall = Instant::now();
        while self.link.mode() != Some(mode) {
            if wall.elapsed() > Duration::from_secs(10) {
                return Err(format!("the plane did not change to {}", plane_mode_name(mode)));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // The status of the new mode (AGENT sends one as it enters, then one a second).
        self.wait(STATUS_WAIT);
        Ok(())
    }

    fn plan(&mut self, args: &[&str], again: bool) -> Result<Vec<String>, String> {
        let usage = "say `plan PRIMITIVE NORTH EAST ALT RADIUS TTL [TARGET]`";
        if args.len() < 6 {
            return Err(usage.into());
        }
        let primitive = primitive_number(args[0]).ok_or(usage)?;
        let num = |i: usize| args.get(i).and_then(|s| s.parse::<f32>().ok());
        let (Some(north), Some(east), Some(alt), Some(radius), Some(ttl)) = (num(1), num(2), num(3), num(4), num(5))
        else {
            return Err(usage.into());
        };
        let target = args.get(6).and_then(|s| s.parse::<u8>().ok()).unwrap_or(0);
        let held = self.link.state().status.plan.unwrap_or(0).max(0) as u16;
        let id = if again { held } else { held.wrapping_add(1) };
        let (lat, lon) = to_global(self.home.0, self.home.1, north, east);
        let mut command = WirePlan { id, primitive: Primitive::Goto, target, lat, lon, alt, radius, ttl }.to_command();
        // Any number, so that a primitive that does not exist can be sent too.
        command.param1 = primitive as f32;
        let plan_acks = |s: &autonomy_agent::link::LinkState| -> u32 {
            s.ack_counts.iter().filter(|((c, _), _)| *c == CMD_PLAN).map(|(_, n)| *n).sum()
        };
        let before = plan_acks(&self.link.state());
        let to = self.link.state().target;
        self.link.send(&command_int(&command, to).expect("a plan")).map_err(|e| e.to_string())?;
        let wall = Instant::now();
        let ack = loop {
            let s = self.link.state();
            if plan_acks(&s) > before {
                break s.acks.iter().rev().find(|a| a.command == CMD_PLAN).copied();
            }
            if wall.elapsed() > Duration::from_secs(5) {
                return Err("no COMMAND_ACK for the plan".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        self.wait(STATUS_WAIT);
        Ok(ack.map(|a| format!("ack id={id} result={} reason={}", a.result, a.result_param2)).into_iter().collect())
    }

    /// `status mode=… plan=… prim=… env=… fall=… recoveries=…`.
    fn status(&mut self) -> String {
        let s = self.link.state();
        // The texts since the last look that are new: the link keeps the last 32.
        let new = (s.texts_heard - self.texts_from.min(s.texts_heard)).min(s.texts.len() as u64) as usize;
        self.recoveries +=
            s.texts[s.texts.len() - new..].iter().filter(|t| t.trim() == "Agent: fallback 3").count() as u32;
        self.texts_from = s.texts_heard;
        let v = |x: Option<i32>| x.map_or("-".into(), |x| x.to_string());
        format!(
            "status mode={} plan={} prim={} env={} fall={} recoveries={} time={:.1}",
            s.mode.map_or("?".into(), plane_mode_name),
            v(s.status.plan),
            v(s.status.primitive),
            v(s.status.interventions),
            v(s.status.fallback),
            self.recoveries,
            (self.now)()
        )
    }
}

fn mode_number(m: &str) -> Option<u32> {
    match m.to_ascii_uppercase().as_str() {
        "AGENT" => Some(AGENT_MODE),
        "FBWA" => Some(FBWA),
        n => n.parse().ok(),
    }
}

fn primitive_number(p: &str) -> Option<u8> {
    Some(match p.to_ascii_lowercase().as_str() {
        "pursue" => Primitive::Pursue as u8,
        "evade" => Primitive::Evade as u8,
        "goto" | "go-to" => Primitive::Goto as u8,
        "orbit" => Primitive::Orbit as u8,
        "band" => Primitive::Band as u8,
        "loiter" => Primitive::Loiter as u8,
        n => n.parse().ok()?,
    })
}
