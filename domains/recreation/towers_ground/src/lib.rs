//! The ground of Towers: the MAVLink side of a team's drones, as a ground station does it. Once a
//! drone has taken off and flies on the wing, switch it to AGENT; then send it the plan for its
//! order about once a second, with its standing order ahead of it (what it does on its own if the
//! plans stop), and the other drones as traffic twice a second, so that its executor keeps clear
//! of them. A light drone, a quadcopter, has no wing for AGENT to fly: it is switched to QLOITER and
//! flown by the world's radio.
//!
//! The commander app runs this for its team, and the game server for a computer's. It talks
//! through a [`Transport`]: TCP to the simulated ground radio, or a serial line to a real one.
//! What it knows of the game is a [`Situation`].

pub mod map;

use cognition_agent::Craft;
use cognition_agent::link::{Link, LinkState, PositionReport, Sighting, Traffic};
use cognition_plan::{CMD_PLAN, Primitive, WirePlan, to_global};
use glam::Vec3;
use recreation_towers::flight;
use recreation_towers::{Game, Order, Phase, Standing};
pub use simulation_sitl::launch::AGENT_MODE;
use std::time::Duration;

/// Each plan is good for this long (s); the ground sends a new one about once a second.
pub const PLAN_TTL: f32 = 3.0;

/// How the ground reaches its drones.
#[derive(Clone, Debug, PartialEq)]
pub enum Transport {
    /// TCP to the simulated ground radio: drone `k` at `host:ports[k]`.
    Tcp { host: String, ports: Vec<u16> },
    /// A serial line to a ground radio (one for the team's drones: they answer to their system ids).
    Serial { path: String, baud: u32 },
}

impl Transport {
    /// The MAVLink connection string for the drone `k` of the ground's drones.
    pub fn url(&self, k: usize) -> Option<String> {
        match self {
            Transport::Tcp { host, ports } => ports.get(k).map(|p| format!("tcpout:{host}:{p}")),
            Transport::Serial { path, baud } => Some(format!("serial:{path}:{baud}")),
        }
    }
}

/// SITL's home: where the field's centre is on the globe (latitude, longitude, altitude AMSL).
pub fn sitl_home() -> (f64, f64, f32) {
    cognition_agent::parse_home(simulation_sitl::launch::HOME).expect("the SITL home")
}

/// The name of drone `i` of the game: `Blue1`, ... `Red5`.
pub fn drone_name(rules: &recreation_towers::Rules, i: usize) -> String {
    let team = if i < rules.drones { "Blue" } else { "Red" };
    format!("{team}{}", i % rules.drones + 1)
}

/// What the ground knows of the game when it looks: in the server, the world; in the app, the
/// team's map and the orders that it has given.
pub struct Situation<'a> {
    /// The drones' own time (s).
    pub now: f64,
    /// The field, and where each drone is (the ground's picture of it).
    pub game: &'a Game,
    /// By drone number in the game.
    pub orders: &'a [Option<Order>],
    pub lows: &'a [bool],
    pub standing: &'a [Option<Standing>],
    /// Which drones fly on the wing (taken off, and past the take-off transition).
    pub wing: Vec<bool>,
}

/// Plans about once a second, traffic twice (the drones' time, s).
const PLAN_EVERY: f64 = 1.0;
const TRAFFIC_EVERY: f64 = 0.5;
/// Ask for AGENT again if the drone is not in it this long after asking (s, the drones' time).
const ASK_AGAIN: f64 = 1.0;
/// ArduPlane's QLOITER: the hover that the radio's sticks move.
pub const QLOITER_MODE: u32 = 19;
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
    /// Its number in the game.
    index: usize,
    name: String,
    /// The mode the ground flies it in: AGENT, or QLOITER for a drone with no wing.
    mode: u32,
    link: Option<Link>,
    /// The floor the drone's envelope has been told.
    floor: Option<f32>,
    asked: Option<f64>,
    /// It was seen in AGENT.
    in_agent: bool,
    plan_id: u16,
    plan: Option<WirePlan>,
    /// The standing plan the drone has: its id, and what it is (a new id when it changes).
    standing_id: u16,
    standing: Option<WirePlan>,
    send_errors: u32,
}

/// The standing plan is the one the drone has already: the same primitive, place, height and time.
/// (A loiter circles where the drone is, whatever its point.)
fn same_standing(had: Option<&WirePlan>, now: &WirePlan) -> bool {
    use cognition_plan::Primitive;
    had.is_some_and(|h| {
        h.primitive == now.primitive
            && h.alt == now.alt
            && h.radius == now.radius
            && h.ttl == now.ttl
            && (now.primitive == Primitive::Loiter || (h.lat == now.lat && h.lon == now.lon))
    })
}

pub struct Ground {
    drones: Vec<Drone>,
    /// The names of all the drones of the game, by number (for the traffic).
    names: Vec<String>,
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
    standings: Vec<(usize, WirePlan)>,
    floors: Vec<(usize, f32)>,
    traffic: Vec<(usize, Traffic)>,
}

/// What the ground has worked out to send.
pub struct Sent(Sends);

impl Ground {
    /// Connects to the drones `indices` (their numbers in the game; `names` has all the drones' names),
    /// the `k`th through `transport`. `modes[k]` is the mode it flies in.
    pub fn connect(
        transport: &Transport,
        names: &[String],
        indices: &[usize],
        modes: &[u32],
        home: (f64, f64, f32),
    ) -> Ground {
        let drones = indices
            .iter()
            .zip(modes)
            .enumerate()
            .map(|(k, (&index, &mode))| {
                let name = names[index].clone();
                let link = transport.url(k).and_then(|url| match Link::open_patiently(&url, Duration::from_secs(30)) {
                    Ok(l) => Some(l),
                    Err(e) => {
                        eprintln!("WARNING: no MAVLink to {name} on {url}: {e}");
                        None
                    }
                });
                Drone {
                    index,
                    name,
                    mode,
                    link,
                    floor: None,
                    asked: None,
                    in_agent: false,
                    plan_id: 0,
                    plan: None,
                    standing_id: 0,
                    standing: None,
                    send_errors: 0,
                }
            })
            .collect();
        Ground { drones, names: names.to_vec(), home, next_plans: 0.0, next_traffic: 0.0 }
    }

    /// The telemetry of the `k`th drone: its flight mode, from its last heartbeat.
    pub fn mode_of(&self, k: usize) -> Option<u32> {
        self.drones.get(k)?.link.as_ref()?.mode()
    }

    /// What the `k`th drone last said of itself, and what its sensor reported: its telemetry.
    pub fn telemetry(&self, k: usize) -> (Option<PositionReport>, Vec<Sighting>) {
        self.drones.get(k).and_then(|d| d.link.as_ref()).map_or((None, vec![]), |l| {
            let s = l.state();
            (s.position, s.sightings.clone())
        })
    }

    /// How many position reports the drones have sent.
    pub fn position_reports(&self) -> u64 {
        self.drones.iter().filter_map(|d| d.link.as_ref()).map(|l| l.state().position_reports).sum()
    }

    /// The drones that have been in AGENT mode.
    pub fn agent_drones(&self) -> usize {
        self.drones.iter().filter(|d| d.in_agent).count()
    }

    /// One look at the game: works out what to send, then sends it.
    pub fn tick(&mut self, situation: &Situation) {
        let sent = self.decide(situation);
        self.send(sent);
    }

    /// Sends what [`Ground::decide`] worked out (the server does this with its world unlocked).
    pub fn send(&mut self, sent: Sent) {
        let sends = sent.0;
        for i in sends.modes {
            if let Some(link) = &self.drones[i].link
                && link.set_mode(self.drones[i].mode).is_err()
            {
                self.drones[i].send_errors += 1;
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
        for (i, standing) in sends.standings {
            let d = &mut self.drones[i];
            if let Some(link) = &d.link {
                if link.send_standing(&standing).is_err() {
                    d.send_errors += 1;
                }
                d.standing = Some(standing);
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

    /// Works out what to send, from the situation.
    pub fn decide(&mut self, w: &Situation) -> Sent {
        let now = w.now;
        let mut sends = Sends::default();
        let wing: Vec<bool> = self.drones.iter().map(|d| w.wing[d.index]).collect();
        let agent: Vec<bool> =
            self.drones.iter().map(|d| d.link.as_ref().is_some_and(|l| l.mode() == Some(d.mode))).collect();
        for (k, d) in self.drones.iter_mut().enumerate() {
            d.in_agent |= agent[k];
            if !wing[k] {
                d.asked = None;
            } else if !agent[k] && d.asked.is_none_or(|t| now - t > ASK_AGAIN || now < t) {
                sends.modes.push(k);
                d.asked = Some(now);
            }
        }
        if now >= self.next_plans {
            self.next_plans = now + PLAN_EVERY;
            for (k, d) in self.drones.iter_mut().enumerate().filter(|(k, _)| wing[*k] && agent[*k]) {
                // A drone with no wing is flown by the radio, not by plans.
                if d.mode == QLOITER_MODE {
                    continue;
                }
                let i = d.index;
                let order = w.orders[i].unwrap_or(Order::Home);
                d.plan_id = d.plan_id.wrapping_add(1).max(1);
                let plan = plan_for(w.game, i, order, w.lows[i], (self.home.0, self.home.1), d.plan_id);
                // A plan under the treetops needs the low floor. One over them the high floor, once the
                // drone is up there: a drone that is still climbing out has the low one, or its
                // envelope would fly it straight ahead and up, wherever it points.
                let floor = if plan.alt < w.game.rules.treetop {
                    LOW_FLOOR
                } else if w.game.drones[i].alt >= UP_THERE {
                    HIGH_FLOOR
                } else {
                    d.floor.unwrap_or(LOW_FLOOR)
                };
                if d.floor != Some(floor) {
                    sends.floors.push((k, floor));
                }
                sends.plans.push((k, plan));
                // What the drone does on its own if the plans stop: the team's standing order after
                // its time, or home when the link is lost. The drone flies it itself, so the ground
                // does not have to be heard for it to happen.
                let (then, after) = match w.standing[i] {
                    Some(s) => (s.then, s.after),
                    None => (Order::Home, w.game.rules.link_timeout),
                };
                d.standing_id = d.standing_id.wrapping_add(1).max(1);
                let mut standing = plan_for(w.game, i, then, false, (self.home.0, self.home.1), d.standing_id);
                standing.ttl = after;
                if !same_standing(d.standing.as_ref(), &standing) {
                    sends.standings.push((k, standing));
                } else {
                    d.standing_id = d.standing_id.wrapping_sub(1).max(1);
                }
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
            for k in (0..self.drones.len()).filter(|&k| wing[k]) {
                let own = self.drones[k].index;
                for c in crafts.iter().filter(|c| c.flying && c.id as usize != own + 1) {
                    let name = &self.names[c.id as usize - 1];
                    sends.traffic.push((k, Traffic::from_craft(c, self.home, name)));
                }
            }
        }
        Sent(sends)
    }

    /// A line about drone `i` (its number in the game) for the report.
    pub fn describe(&self, i: usize) -> String {
        let Some(d) = self.drones.iter().find(|d| d.index == i) else {
            return format!("{} is flown by its seat", self.names[i]);
        };
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

/// Where drone `i` flies to carry out `order`, flown `low` or high.
pub fn leg_for(game: &Game, i: usize, order: Order, low: bool) -> flight::Leg {
    let d = &game.drones[i];
    let (team, k) = (d.team, i % game.rules.drones);
    let target = match order {
        Order::Take(t) => game.towers[t].at,
        Order::Goto(p) => p,
        Order::Hold => d.at,
        Order::Home => game.rules.base(team),
    };
    if order == Order::Hold {
        // Circle where it is: low only if it is over open ground.
        let low = low && game.map.open(d.at);
        let alt = if low { flight::low_height(team, k) } else { flight::orbit_height(team, k) };
        flight::Leg { orbit: false, point: d.at, alt, radius: 0.0 }
    } else {
        let plan = flight::plan(&game.map, d.at, target, low);
        flight::leg(&game.map, &plan, team, k, d.at, d.alt)
    }
}

/// The plan for drone `i` (its index in the game) to carry out `order`, flown `low` or high, with
/// home at (`lat`, `lon`).
pub fn plan_for(game: &Game, i: usize, order: Order, low: bool, home: (f64, f64), id: u16) -> WirePlan {
    let leg = leg_for(game, i, order, low);
    let (primitive, radius) = match (order, leg.orbit) {
        (Order::Hold, _) => (Primitive::Loiter, 0.0),
        (_, true) => (Primitive::Orbit, leg.radius),
        (_, false) => (Primitive::Goto, 0.0),
    };
    let (lat, lon) = to_global(home.0, home.1, leg.point.x, leg.point.y);
    WirePlan { id, primitive, target: 0, lat, lon, alt: leg.alt, radius, ttl: PLAN_TTL }
}
