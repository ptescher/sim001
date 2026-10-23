//! The ground's side of each drone, over MAVLink, as the runner's ground control agent does it:
//! once a drone has taken off and flies on the wing, switch it to AGENT; then send it the plan for
//! its order about once a second (a light drone, a quadcopter, has no wing for AGENT to fly: it is
//! switched to GUIDED and sent to the point of its plan),  and the other drones as traffic twice a second, so that its
//! executor keeps clear of them.

use crate::world::{World, plan_for};
use cognition_agent::Craft;
use cognition_agent::link::{Link, LinkState, Traffic};
use cognition_plan::{CMD_PLAN, WirePlan};
use glam::Vec3;
use recreation_towers::{Order, Phase};
pub use simulation_sitl::launch::AGENT_MODE;
use std::time::Duration;

/// Plans about once a second, traffic twice (the drones' time, s).
const PLAN_EVERY: f64 = 1.0;
const TRAFFIC_EVERY: f64 = 0.5;
/// Ask for AGENT again if the drone is not in it this long after asking (s, the drones' time).
const ASK_AGAIN: f64 = 1.0;
/// ArduPlane's GUIDED, which hovers (QGUIDED) on a quadplane with `Q_GUIDED_MODE`.
pub const GUIDED_MODE: u32 = 15;
/// `MAV_CMD_DO_SET_MODE`.
const MAV_CMD_DO_SET_MODE: u16 = 176;

/// The lowest a drone may fly (`AGENT_FLOOR`, m): under the tops of the trees for a drone that
/// flies low, over them (with room to spare, since the envelope dives a drone to keep clear of
/// others) for one that flies high.
const LOW_FLOOR: f32 = 8.0;
const HIGH_FLOOR: f32 = 35.0;
/// A drone this high (m) that flies high is up over the trees.
const UP_THERE: f32 = 42.0;

struct Drone {
    name: String,
    /// The mode the ground flies it in: AGENT, or GUIDED for a drone with no wing.
    mode: u32,
    link: Option<Link>,
    /// The floor the drone's envelope has been told.
    floor: Option<f32>,
    asked: Option<f64>,
    /// It was seen in AGENT.
    in_agent: bool,
    plan_id: u16,
    plan: Option<WirePlan>,
    send_errors: u32,
}

pub struct Ground {
    drones: Vec<Drone>,
    /// SITL's home: latitude, longitude, altitude AMSL.
    home: (f64, f64, f32),
    next_plans: f64,
    next_traffic: f64,
}

/// What the ground does in one look, worked out with the world locked: the messages to send.
#[derive(Default)]
struct Sends {
    modes: Vec<usize>,
    plans: Vec<(usize, WirePlan)>,
    /// The drones that fly by GUIDED: the point of their plan.
    gotos: Vec<(usize, WirePlan)>,
    floors: Vec<(usize, f32)>,
    traffic: Vec<(usize, Traffic)>,
}

impl Ground {
    /// Connects to each drone's MAVLink port (`ports[i]` for drone `i`).
    pub fn connect(names: &[String], modes: &[u32], ports: &[u16], home: (f64, f64, f32)) -> Ground {
        let drones = names
            .iter()
            .zip(ports.iter().zip(modes))
            .map(|(name, (&port, &mode))| {
                let link = match Link::connect_patiently(&format!("127.0.0.1:{port}"), Duration::from_secs(30)) {
                    Ok(l) => Some(l),
                    Err(e) => {
                        eprintln!("WARNING: no MAVLink to {name} on tcp {port}: {e}");
                        None
                    }
                };
                Drone {
                    name: name.clone(),
                    mode,
                    link,
                    floor: None,
                    asked: None,
                    in_agent: false,
                    plan_id: 0,
                    plan: None,
                    send_errors: 0,
                }
            })
            .collect();
        Ground { drones, home, next_plans: 0.0, next_traffic: 0.0 }
    }

    /// One look at the game.
    pub fn tick(&mut self, world: &std::sync::Mutex<World>) {
        let sends = {
            let w = world.lock().unwrap();
            self.decide(&w)
        };
        for i in sends.modes {
            if let Some(link) = &self.drones[i].link
                && link.set_mode(self.drones[i].mode).is_err()
            {
                self.drones[i].send_errors += 1;
            }
        }
        for (i, plan) in sends.gotos {
            let d = &mut self.drones[i];
            if let Some(link) = &d.link {
                if link.goto(plan.lat, plan.lon, plan.alt).is_err() {
                    d.send_errors += 1;
                }
                d.plan = Some(plan);
            }
        }
        for (i, floor) in sends.floors {
            let d = &mut self.drones[i];
            if let Some(link) = &d.link {
                if link.set_param("AGENT_FLOOR", floor).is_err() {
                    d.send_errors += 1;
                }
                d.floor = Some(floor);
            }
        }
        for (i, plan) in sends.plans {
            let d = &mut self.drones[i];
            if let Some(link) = &d.link {
                if link.send_plan(&plan).is_err() {
                    d.send_errors += 1;
                }
                d.plan = Some(plan);
            }
        }
        for (i, t) in sends.traffic {
            if let Some(link) = &self.drones[i].link
                && link.send_traffic(&t).is_err()
            {
                self.drones[i].send_errors += 1;
            }
        }
    }

    fn decide(&mut self, w: &World) -> Sends {
        let now = w.now();
        let mut sends = Sends::default();
        let wing: Vec<bool> = (0..self.drones.len()).map(|i| w.on_the_wing(i)).collect();
        let agent: Vec<bool> =
            self.drones.iter().map(|d| d.link.as_ref().is_some_and(|l| l.mode() == Some(d.mode))).collect();
        for (i, d) in self.drones.iter_mut().enumerate() {
            d.in_agent |= agent[i];
            if !wing[i] {
                d.asked = None;
            } else if !agent[i] && d.asked.is_none_or(|t| now - t > ASK_AGAIN || now < t) {
                sends.modes.push(i);
                d.asked = Some(now);
            }
        }
        if now >= self.next_plans {
            self.next_plans = now + PLAN_EVERY;
            for (i, d) in self.drones.iter_mut().enumerate().filter(|(i, _)| wing[*i] && agent[*i]) {
                let order = w.orders[i].unwrap_or(Order::Home);
                d.plan_id = d.plan_id.wrapping_add(1).max(1);
                let plan = plan_for(&w.game, i, order, w.lows[i], (self.home.0, self.home.1), d.plan_id);
                // A plan under the treetops needs the low floor. One over them the high floor, once the
                // drone is up there: a drone that is still climbing out has the low one, or its
                // envelope would fly it straight ahead and up, wherever it points.
                if d.mode == GUIDED_MODE {
                    sends.gotos.push((i, plan));
                    continue;
                }
                let floor = if plan.alt < w.game.rules.treetop {
                    LOW_FLOOR
                } else if w.game.drones[i].alt >= UP_THERE {
                    HIGH_FLOOR
                } else {
                    d.floor.unwrap_or(LOW_FLOOR)
                };
                if d.floor != Some(floor) {
                    sends.floors.push((i, floor));
                }
                sends.plans.push((i, plan));
            }
        }
        if now >= self.next_traffic {
            self.next_traffic = now + TRAFFIC_EVERY;
            let crafts: Vec<Craft> = w
                .game
                .drones
                .iter()
                .enumerate()
                .map(|(j, d)| Craft {
                    id: j as u8 + 1,
                    pos: Vec3::new(d.at.x, d.at.y, -d.alt),
                    vel: Vec3::new(d.vel.x, d.vel.y, 0.0),
                    flying: d.phase == Phase::Flying,
                })
                .collect();
            for i in (0..self.drones.len()).filter(|&i| wing[i]) {
                for c in crafts.iter().filter(|c| c.flying && c.id as usize != i + 1) {
                    let name = &self.drones[c.id as usize - 1].name;
                    sends.traffic.push((i, Traffic::from_craft(c, self.home, name)));
                }
            }
        }
        sends
    }

    /// A line about drone `i` for the report.
    pub fn describe(&self, i: usize) -> String {
        let d = &self.drones[i];
        let Some(link) = &d.link else { return format!("{} no link", d.name) };
        let s = link.state();
        let mode = s.mode.map_or("?".into(), simulation_sitl::launch::plane_mode_name);
        let plan = d.plan.map_or("-".into(), |p| {
            let (n, e) = cognition_plan::to_local(self.home.0, self.home.1, p.lat, p.lon);
            format!("{:?} #{} {:.0}m at {n:.0}, {e:.0}", p.primitive, p.id, p.alt)
        });
        let v = |x: Option<i32>| x.map_or("-".into(), |x| x.to_string());
        format!(
            "{} {mode} plan {plan} | AG_PLAN {} ENV {} FALL {}{}",
            d.name,
            v(s.status.plan),
            v(s.status.interventions),
            v(s.status.fallback),
            if s.connected { "" } else { " LINK DOWN" }
        )
    }

    /// The drones that flew in AGENT mode, and the commands they refused.
    pub fn results(&self) -> (usize, u32) {
        let states: Vec<LinkState> = self.drones.iter().filter_map(|d| d.link.as_ref()).map(|l| l.state()).collect();
        let in_agent = self.drones.iter().filter(|d| d.in_agent).count();
        let refused = states
            .iter()
            .flat_map(|s| s.ack_counts.iter())
            .filter(|((c, r), _)| [CMD_PLAN, MAV_CMD_DO_SET_MODE].contains(c) && *r != 0)
            .map(|(_, n)| n)
            .sum();
        (in_agent, refused)
    }

    /// Warnings at the end.
    pub fn verdict(&self) -> Vec<String> {
        let mut out = vec![];
        for d in &self.drones {
            if d.link.is_none() {
                out.push(format!("WARNING: {} had no MAVLink link", d.name));
            } else if !d.in_agent {
                out.push(format!("WARNING: {} never went into AGENT", d.name));
            }
            if d.send_errors > 0 {
                out.push(format!("WARNING: {} had {} send errors", d.name, d.send_errors));
            }
        }
        out
    }
}
