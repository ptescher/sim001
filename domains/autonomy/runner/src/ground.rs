//! `--agent`: the ground control agent flies the game. The session still sets the game up and
//! takes each plane off (arm, QHOVER to 15 m, FBWA); once a plane is on the wing the ground
//! switches it to AGENT over MAVLink, and from then on the scripted brain's plans fly it. The
//! radio's mode switch stays at FBWA (ArduPilot only acts on the switch when it moves), so it
//! doesn't take the plane back out of AGENT.

use autonomy_agent::link::{Link, LinkState, Traffic};
use autonomy_agent::{Brain, Craft, GameView, Picture, ScriptedBrain, world_to_ned, world_vel_to_ned};
use autonomy_plan::{CMD_GAME, CMD_PLAN, WirePlan};
use glam::Vec3;
use recreation_games::Session;
use simulation_physics::{FlightState, Phase};
use simulation_sitl::launch::{self, AGENT_MODE, plane_mode_name};
use std::path::Path;
use std::process::Child;
use std::time::Duration;

/// The SITL planes of a game, stopped when dropped.
pub enum Fleet {
    /// `simulation_sitl`'s, every plane with MAVLink system id 1.
    Stock(launch::Fleet),
    /// Instances 1 to 3 with system ids 1 to 3 (`--sysid`).
    Numbered(Vec<Child>),
}

impl Fleet {
    pub fn start(root: &Path, dir: &Path, speedup: f32) -> Result<Fleet, String> {
        let program = launch::native_binary(root);
        if !program.exists() {
            return Err(format!("no native ArduPlane at {}: run ardupilot/build_native.sh", program.display()));
        }
        let mut children = vec![];
        for n in 1..4u16 {
            let mut args = launch::native_instance_args(root, "t1s-vtol", n, launch::HOME, speedup);
            args.extend(["--sysid".to_string(), n.to_string()]);
            match launch::spawn(
                &dir.join(format!("instance{n}")),
                launch::Plan::Native { program: program.clone(), args },
            ) {
                Ok((c, _)) => children.push(c),
                Err(e) => {
                    Fleet::Numbered(children).stop();
                    return Err(e);
                }
            }
        }
        Ok(Fleet::Numbered(children))
    }

    fn stop(&mut self) {
        match self {
            Fleet::Stock(f) => f.stop(),
            Fleet::Numbered(children) => {
                for c in children.iter_mut() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                children.clear();
            }
        }
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Plans about once a second, traffic twice (the planes' time, s).
const DECIDE_EVERY: f64 = 1.0;
const TRAFFIC_EVERY: f64 = 0.5;
/// Ask for AGENT again if the plane is not in it this long after asking (s, the planes' time).
const ASK_AGAIN: f64 = 1.0;
/// The executor's floor (m above home): `autonomy_plan::Limits::default().floor`.
const FLOOR: f32 = 12.0;

/// In the air: what the referee counts as flying.
pub fn airborne(s: &FlightState) -> bool {
    matches!(s.phase, Phase::Launching | Phase::Flying) && s.carry.is_none() && !s.on_ground
}

/// What the ground sees: the session's planes (ids 1 to 3, the referee's player numbers) in
/// local NED from home (the pilot box, where SITL's origin is) and the game.
pub fn picture(session: &Session, home: (f64, f64)) -> Picture {
    let tag = session.tag_ref().expect("a game of tag");
    let planes = session
        .opponents
        .iter()
        .enumerate()
        .map(|(i, o)| Craft {
            id: i as u8 + 1,
            pos: world_to_ned(o.state.pos, Vec3::ZERO),
            vel: world_vel_to_ned(o.state.vel),
            flying: airborne(&o.state),
        })
        .collect();
    let it = tag.it;
    let no_tag_back = (1..tag.players.len()).find(|&w| w != it && !tag.may_tag(w)).unwrap_or(0) as u8;
    let centre = world_to_ned(tag.arena.centre, Vec3::ZERO);
    Picture {
        time: session.opponents[0].state.time as f32,
        home,
        planes,
        game: GameView {
            it: it as u8,
            time_left: tag.rules.length - tag.time,
            no_tag_back,
            centre: [centre.x, centre.y],
            radius: tag.arena.radius,
            ceiling: tag.arena.ceiling,
            floor: FLOOR,
        },
    }
}

/// One plane's side of the ground.
struct Seat {
    name: &'static str,
    link: Option<Link>,
    /// When AGENT was last asked for (the plane's time).
    asked: Option<f64>,
    asks: u32,
    /// The plane was seen in AGENT.
    in_agent: bool,
    plan: Option<WirePlan>,
    send_errors: u32,
}

/// The ground control agent of the runner.
pub struct Ground {
    seats: Vec<Seat>,
    brain: ScriptedBrain,
    home: (f64, f64, f32),
    next_decision: f64,
    next_traffic: f64,
}

impl Ground {
    /// Connects to each SITL instance's MAVLink port (`ports[i]` for computer pilot `i`).
    pub fn connect(names: &[&'static str], ports: &[u16], home: (f64, f64, f32)) -> Ground {
        let seats = names
            .iter()
            .zip(ports)
            .map(|(&name, &port)| {
                let link = match Link::connect_patiently(&format!("127.0.0.1:{port}"), Duration::from_secs(20)) {
                    Ok(l) => Some(l),
                    Err(e) => {
                        eprintln!("WARNING: no MAVLink to {name} on tcp {port}: {e}; it stays with its pilot");
                        None
                    }
                };
                Seat { name, link, asked: None, asks: 0, in_agent: false, plan: None, send_errors: 0 }
            })
            .collect();
        Ground { seats, brain: ScriptedBrain::computer_pilots(), home, next_decision: 0.0, next_traffic: 0.0 }
    }

    /// One look at the game: switch planes on the wing to AGENT, decide and send plans, uplink
    /// traffic. `on_the_wing[i]` is whether computer pilot `i` has finished its take-off; `now`
    /// is the planes' time.
    pub fn tick(&mut self, picture: &Picture, on_the_wing: &[bool], now: f64) {
        for (i, seat) in self.seats.iter_mut().enumerate() {
            let Some(link) = &seat.link else { continue };
            let mode = link.mode();
            seat.in_agent |= mode == Some(AGENT_MODE);
            if on_the_wing[i] && mode != Some(AGENT_MODE) && seat.asked.is_none_or(|t| now - t > ASK_AGAIN || now < t) {
                if link.set_mode(AGENT_MODE).is_err() {
                    seat.send_errors += 1;
                }
                seat.asked = Some(now);
                seat.asks += 1;
            }
            if !on_the_wing[i] {
                seat.asked = None;
            }
        }
        if now >= self.next_decision {
            self.next_decision = now + DECIDE_EVERY;
            for order in self.brain.decide(picture) {
                let i = order.plane as usize - 1;
                let Some(seat) = self.seats.get_mut(i) else { continue };
                let Some(link) = seat.link.as_ref().filter(|_| on_the_wing[i]) else { continue };
                if link.send_game(&picture.game_for(order.plane)).is_err() {
                    seat.send_errors += 1;
                }
                // The plane takes plans only in AGENT.
                if link.mode() == Some(AGENT_MODE) {
                    if link.send_plan(&order.plan).is_err() {
                        seat.send_errors += 1;
                    }
                    seat.plan = Some(order.plan);
                }
            }
        }
        if now >= self.next_traffic {
            self.next_traffic = now + TRAFFIC_EVERY;
            let names = self.names();
            for (i, seat) in self.seats.iter_mut().enumerate() {
                let Some(link) = seat.link.as_ref().filter(|_| on_the_wing[i]) else { continue };
                for c in picture.planes.iter().filter(|c| c.flying && c.id as usize != i + 1) {
                    let name = names.get(c.id as usize - 1).copied().unwrap_or("PLANE");
                    if link.send_traffic(&Traffic::from_craft(c, self.home, name)).is_err() {
                        seat.send_errors += 1;
                    }
                }
            }
        }
    }

    fn names(&self) -> Vec<&'static str> {
        self.seats.iter().map(|s| s.name).collect()
    }

    /// A line for the periodic report, about computer pilot `i`.
    pub fn describe(&self, i: usize) -> String {
        let seat = &self.seats[i];
        let Some(link) = &seat.link else { return "no link".into() };
        let s = link.state();
        let mode = s.mode.map_or("?".into(), plane_mode_name);
        let plan = seat.plan.map_or("-".into(), |p| {
            let target = if p.target != 0 { format!("->{}", p.target) } else { String::new() };
            format!("{:?}{target} #{} {:.0}m", p.primitive, p.id, p.alt)
        });
        let st = s.status;
        let v = |x: Option<i32>| x.map_or("-".into(), |x| x.to_string());
        format!(
            "sys {} {mode} plan {plan} | AG_PLAN {} PRIM {} ENV {} FALL {} | acks plan {} game {} mode {}{}",
            s.target.0,
            v(st.plan),
            v(st.primitive),
            v(st.interventions),
            v(st.fallback),
            acks(&s, CMD_PLAN),
            acks(&s, CMD_GAME),
            acks(&s, 176),
            if s.connected { "" } else { " LINK DOWN" },
        )
    }

    /// Warnings at the end: planes that never went into AGENT, or never reported back a plan.
    pub fn verdict(&self) -> Vec<String> {
        let mut out = vec![];
        for seat in &self.seats {
            let Some(link) = &seat.link else {
                out.push(format!("WARNING: {} had no MAVLink link", seat.name));
                continue;
            };
            let s = link.state();
            let in_agent = seat.in_agent || s.mode == Some(AGENT_MODE);
            if seat.asks == 0 {
                out.push(format!("WARNING: {} never got on the wing, so AGENT was never asked for", seat.name));
            } else if !in_agent {
                out.push(format!(
                    "WARNING: {} never went into AGENT (asked {} times; mode acks {})",
                    seat.name,
                    seat.asks,
                    acks(&s, 176)
                ));
            }
            if s.plans_sent > 0 && s.best_plan_echo.is_none_or(|b| b <= 0) {
                out.push(format!(
                    "WARNING: {} never reported a plan id back (AG_PLAN) after {} plans (acks {}): the AGENT mode ignores the plans (the stub?)",
                    seat.name,
                    s.plans_sent,
                    acks(&s, CMD_PLAN)
                ));
            }
            if seat.send_errors > 0 {
                out.push(format!("WARNING: {} had {} send errors", seat.name, seat.send_errors));
            }
            let texts: Vec<&String> = s.texts.iter().filter(|t| t.contains("gent")).collect();
            if !texts.is_empty() {
                out.push(format!("{} said: {:?}", seat.name, texts));
            }
        }
        out
    }
}

/// The acks for `command`, as "accepted/other" counts (MAV_RESULT 0 is accepted).
fn acks(s: &LinkState, command: u16) -> String {
    let ok = s.acks_of(command, 0);
    let other: u32 = s.ack_counts.iter().filter(|((c, r), _)| *c == command && *r != 0).map(|(_, n)| n).sum();
    let last_bad = s
        .acks
        .iter()
        .rev()
        .find(|a| a.command == command && a.result != 0)
        .map(|a| format!(" (last refusal: result {})", a.result));
    format!("{ok}/{other}{}", last_bad.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use recreation_games::Field;

    #[test]
    fn the_ground_sees_the_session_in_ned() {
        let field = Field { pilot: Vec3::ZERO, heading: 0.0 };
        let mut session = Session::tag(field);
        session.tag_ref_mut().unwrap().it = 1;
        // Kestrel 30 m up, 40 m north and 10 m east of the pilot box, flying north.
        let s = &mut session.opponents[0].state;
        s.pos = Vec3::new(10.0, 30.0, -40.0);
        s.vel = Vec3::new(0.0, 0.0, -18.0);
        s.phase = Phase::Flying;
        s.on_ground = false;
        let p = picture(&session, (-35.0, 149.0));
        assert_eq!(p.planes.len(), 3);
        assert_eq!(p.planes[0].id, 1);
        assert_eq!(p.planes[0].pos, Vec3::new(40.0, 10.0, -30.0));
        assert_eq!(p.planes[0].vel, Vec3::new(18.0, 0.0, 0.0));
        assert!(p.planes[0].flying && !p.planes[1].flying, "the others are still in hand");
        assert_eq!((p.game.it, p.game.no_tag_back, p.game.time_left), (1, 0, 120.0));
        assert_eq!((p.game.centre, p.game.radius, p.game.ceiling), ([70.0, 0.0], 90.0, 60.0));
    }
}
