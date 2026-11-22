//! The ground of Towers: the MAVLink side of a team's drones, as a ground station does it. Once a
//! drone has taken off and flies on the wing, switch it to AGENT; then send it the plan for its
//! order about once a second, with its standing order ahead of it (what it does on its own if the
//! plans stop), and the other drones as traffic twice a second, so that its executor keeps clear
//! of them. A light drone, a quadcopter, has no wing for AGENT to fly: it is switched to QLOITER and
//! flown by the world's radio.
//!
//! The commander app runs this for its team, and so does a computer commander (`commander`): both
//! take a seat on the game server (`seat`) and fly the team's drones from its station (`station`).
//! It talks
//! through a [`Transport`]: TCP to the simulated ground radio, or a serial line to a real one.
//! What it knows of the game is a [`Situation`].

pub mod advisors;
pub mod claude;
pub mod commander;
pub mod map;
pub mod pilot;
pub mod seat;
pub mod station;

use cognition_agent::Craft;
use cognition_agent::link::{Link, LinkState, PositionReport, Sighting, Traffic};
use cognition_plan::{CMD_PLAN, Primitive, WirePlan, to_global};
use glam::Vec3;
use recreation_towers::flight;
use recreation_towers::{Game, Order, Phase, Standing};
pub use simulation_sitl::launch::AGENT_MODE;
use std::time::Duration;

/// A plan is good for this long at the least (s); the ground sends a new one about once a second.
pub const PLAN_TTL: f32 = 3.0;

/// How long a plan sent to a drone is good for (s): the rest of the game, and at least until its
/// standing plan (after `after` s of silence) would take over. A drone that takes an enemy tower is
/// in the tower's radio shadow (`Game::jammed`), where no plan reaches it, for as long as no friend
/// relays it: a minute or more. A plan that ran out after [`PLAN_TTL`] (3 s), or after the standing
/// time (45 s), made the executor fall back to a loiter round the point where it ran out: a circle
/// of 50 m round a drone that was already 40 or 50 m out from the tower, far past the 80 m clearing
/// and into the trees: 9 of the 33 crashes of 14 games, 12 to 17 s after the drone went out of
/// contact, and the same again at 57 to 61 s with a plan good for the standing time. (Its standing
/// plan did not take over before the plan ran out: the plan id in `AG_PLAN` stayed the same, and
/// `AG_FALL` read 1: not found out.) The team's order stands while the drone is out of contact.
pub fn plan_ttl(rules: &recreation_towers::Rules, after: f32) -> f32 {
    after.max(rules.duration).max(PLAN_TTL)
}

/// How the ground reaches its drones.
#[derive(Clone, Debug, PartialEq)]
pub enum Transport {
    /// TCP to the simulated ground radio: drone `k` at `host:ports[k]`.
    Tcp { host: String, ports: Vec<u16> },
    /// A serial line to a ground radio for each drone: `paths[k]` for drone `k` (a radio on a bench
    /// has one drone behind it).
    Serial { paths: Vec<String>, baud: u32 },
}

impl Transport {
    /// The MAVLink connection string for the drone `k` of the ground's drones.
    pub fn url(&self, k: usize) -> Option<String> {
        match self {
            Transport::Tcp { host, ports } => ports.get(k).map(|p| format!("tcpout:{host}:{p}")),
            Transport::Serial { paths, baud } => paths.get(k).map(|p| format!("serial:{p}:{baud}")),
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
    /// The standing order of each drone: the first `Silent` instruction of its queue. The flight
    /// controller has one standing plan, so this is all of the queue that the ground uploads (the
    /// rest is flown by the server for a drone with no wing, and by the arena). An on-board
    /// queue is a later step (`plans/actors.md`).
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
/// How far a drone of `class` may bank (`ROLL_LIMIT_DEG`, degrees), if not as far as its parameter
/// file says (50). The heavy drone (the T2 Cruza) is held to 45 degrees: at the 17 m/s it once
/// cruised at, it stalled at 50 (the stall speed grows with the root of 1 / cos(bank), and its
/// airspeed minimum of 13 m/s is 15 m/s at 50 degrees), and a step in the roll it asked for
/// overshot to 70 or 80 degrees and ended in a spiral dive. It cruises at 20 m/s now
/// (`t2-cruza.parm`), where 45 degrees leaves it a margin of more than a quarter; the circle the
/// executor flies at that bank (a turn radius and a tenth: 57 m) is wider than either team's ring,
/// and still inside the 60 m circle of a tower.
pub fn bank_limit(class: recreation_towers::Class) -> Option<f32> {
    (class == recreation_towers::Class::Heavy).then_some(HEAVY_BANK)
}

/// The heavy drone's bank limit (degrees).
pub const HEAVY_BANK: f32 = 45.0;

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
    /// The bank limit it has been told (`ROLL_LIMIT_DEG`, degrees).
    bank: Option<f32>,
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
    banks: Vec<(usize, f32)>,
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
                    bank: None,
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
        for (i, bank) in sends.banks {
            let d = &mut self.drones[i];
            if let Some(link) = &d.link {
                if link.set_param("ROLL_LIMIT_DEG", bank).is_err() {
                    d.send_errors += 1;
                }
                d.bank = Some(bank);
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
                let mut plan = plan_for(w.game, i, order, w.lows[i], (self.home.0, self.home.1), d.plan_id);
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
                if let Some(bank) = bank_limit(w.game.drones[i].class)
                    && d.bank != Some(bank)
                {
                    sends.banks.push((k, bank));
                }
                // What the drone does on its own if the plans stop: the team's standing order after
                // its time, or home when the link is lost. The drone flies it itself, so the ground
                // does not have to be heard for it to happen.
                let (then, after) = match w.standing[i] {
                    Some(s) => (s.then, s.after),
                    None => (Order::Home, w.game.rules.link_timeout),
                };
                plan.ttl = plan_ttl(&w.game.rules, after);
                sends.plans.push((k, plan));
                d.standing_id = d.standing_id.wrapping_add(1).max(1);
                let mut standing =
                    standing_for(w.game, i, order, then, &plan, (self.home.0, self.home.1), d.standing_id);
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

    /// What the executor of drone `i` (its number in the game) last reported: `AG_PLAN`, `AG_ENV`
    /// and `AG_FALL`, -1 for what it has not said.
    pub fn status_of(&self, i: usize) -> [i32; 3] {
        let Some(link) = self.drones.iter().find(|d| d.index == i).and_then(|d| d.link.as_ref()) else {
            return [-1; 3];
        };
        let s = link.state().status;
        [s.plan.unwrap_or(-1), s.interventions.unwrap_or(-1), s.fallback.unwrap_or(-1)]
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
        Order::Goto(p) | Order::Circle(p, _) => p,
        Order::Hold => d.at,
        // Landing for a battery swap is not flown on ArduPlane yet: the drone circles over its base.
        Order::Home | Order::Land => game.rules.base(team),
        // An intercept is not flown on ArduPlane yet: the drone goes to the enemy drone's place.
        Order::Intercept(e) => game.drones[game.drone(team.other(), e)].at,
    };
    if order == Order::Hold {
        // Circle where it is: low only if it is over open ground.
        let low = low && game.map.open(d.at);
        let alt = if low { flight::low_height(team, k) } else { flight::orbit_height(team, k) };
        flight::Leg { orbit: false, point: d.at, alt, radius: 0.0 }
    } else {
        let plan = match order {
            // A drone sent to take a tower circles it low, however it gets there.
            Order::Take(_) => flight::take(&game.map, d.at, target, low),
            _ => flight::plan(&game.map, d.at, target, low),
        };
        let leg = flight::leg(&game.map, &plan, team, k, d.at, d.alt);
        // CIRCLE: the circle is as wide as the commander drew it.
        match order {
            Order::Circle(_, radius) if leg.orbit => flight::Leg { radius, ..leg },
            _ => leg,
        }
    }
}

/// The standing plan of drone `i`, which flies `plan` now: what it does by itself once the plans
/// have stopped for the standing time (`then`, home unless the team said otherwise). A drone that
/// flies under the treetops (a capture is flown low) cannot go anywhere in a straight line: the plan
/// that would take a high drone home flies a low one into the forest 12 s later (9 of the 13
/// crashes in a radio shadow, once the plan itself was good for as long as the standing time). It
/// climbs where it is, round the circle of its plan, over the trees, and stays there.
pub fn standing_for(
    game: &Game,
    i: usize,
    order: Order,
    then: Order,
    plan: &WirePlan,
    home: (f64, f64),
    id: u16,
) -> WirePlan {
    // A drone that takes a tower comes down to circle it by its plan, with no word from the ground
    // (and is in the shadow of the tower, if it is the enemy's): the standing plan that it was sent
    // while it was still high would take it home from there, under the treetops.
    let comes_down = matches!(order, Order::Take(_));
    if plan.primitive == Primitive::Orbit && (plan.alt < game.rules.treetop || comes_down) {
        let d = &game.drones[i];
        let alt = flight::orbit_height(d.team, i % game.rules.drones);
        return WirePlan { id, alt, ..*plan };
    }
    plan_for(game, i, then, false, home, id)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_is_good_for_the_rest_of_the_game_so_a_drone_in_a_shadow_keeps_its_orbit() {
        let rules = recreation_towers::Rules::default();
        // The plans go out every second, and the drone is in an enemy tower's shadow for a minute
        // or more: in between, the executor would loiter where the plan ran out, a circle that
        // reaches 50 m past the drone, out of a tower's clearing.
        assert!(rules.link_timeout > PLAN_TTL);
        assert!(plan_ttl(&rules, rules.link_timeout) >= rules.duration);
        assert!(plan_ttl(&rules, 1.0) >= rules.duration, "also for a standing plan that takes over sooner");
        assert!(plan_ttl(&rules, 2.0 * rules.duration) >= 2.0 * rules.duration, "or later");
    }

    /// `AIRSPEED_CRUISE` of `ardupilot/t2-cruza.parm` (m/s).
    const HEAVY_CRUISE: f32 = 20.0;

    #[test]
    fn the_heavy_drone_cruises_at_the_speed_the_tests_assume() {
        let parm =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../ardupilot/t2-cruza.parm")).unwrap();
        let cruise =
            parm.lines().find_map(|l| l.strip_prefix("AIRSPEED_CRUISE ")).map(|v| v.trim().parse::<f32>().unwrap());
        assert_eq!(cruise, Some(HEAVY_CRUISE));
    }

    #[test]
    fn the_heavy_drone_is_held_to_a_bank_it_does_not_stall_in_and_still_fits_a_tower_circle() {
        use recreation_towers::{Class, Team, flight};
        assert_eq!(bank_limit(Class::Standard), None);
        assert_eq!(bank_limit(Class::Heavy), Some(HEAVY_BANK));
        // At its cruise speed of 20 m/s (`t2-cruza.parm`) the stall speed in the bank, from its
        // airspeed minimum of 13 m/s, leaves a margin of more than a quarter.
        let stall = 13.0 / HEAVY_BANK.to_radians().cos().sqrt();
        assert!(HEAVY_CRUISE / stall >= 1.25, "{stall} m/s in a bank of {HEAVY_BANK} degrees");
        // The executor flies at least a turn radius of the cruise speed at 85 % of the bank, and a
        // tenth more: wider than the rings, but inside the circle of a tower.
        let flown = 1.1 * HEAVY_CRUISE * HEAVY_CRUISE / (9.80665 * (0.85 * HEAVY_BANK).to_radians().tan());
        assert!(flown > flight::orbit_radius(Team::Red), "{flown} m");
        assert!(flown < recreation_towers::Rules::default().circle_radius, "{flown} m");
    }

    #[test]
    fn a_drone_that_circles_low_climbs_where_it_circles_when_the_plans_stop_and_does_not_fly_home_low() {
        use recreation_towers::{Rules, Team, flight};
        let mut game = Game::new(Rules::default());
        let home = sitl_home();
        let tower = game.towers[1].at;
        let i = 1;
        game.drones[i].at = tower + glam::Vec2::new(40.0, 0.0);
        game.drones[i].alt = 18.0;
        game.drones[i].phase = Phase::Flying;
        let plan = plan_for(&game, i, Order::Take(1), false, (home.0, home.1), 5);
        assert!(plan.alt < game.rules.treetop, "it circles the tower low: {plan:?}");
        let standing = standing_for(&game, i, Order::Take(1), Order::Home, &plan, (home.0, home.1), 6);
        // The same circle, over the trees.
        assert_eq!(
            (standing.primitive, standing.lat, standing.lon, standing.radius),
            (plan.primitive, plan.lat, plan.lon, plan.radius)
        );
        assert_eq!(standing.alt, flight::orbit_height(Team::Blue, i));
        assert!(standing.alt >= game.rules.treetop + 30.0);
        // A drone that flies high goes home by its plan, as before.
        game.drones[i].alt = 70.0;
        game.drones[i].at = tower + glam::Vec2::new(300.0, 0.0);
        let high = plan_for(&game, i, Order::Take(1), false, (home.0, home.1), 7);
        assert!(high.alt >= game.rules.treetop);
        let standing = standing_for(&game, i, Order::Goto(tower), Order::Home, &high, (home.0, home.1), 8);
        let at_home = plan_for(&game, i, Order::Home, false, (home.0, home.1), 8);
        assert_eq!((standing.lat, standing.lon, standing.alt), (at_home.lat, at_home.lon, at_home.alt));
        // But a drone that is still high and is sent to take a tower will come down to circle it:
        // its standing plan is that circle, over the trees.
        let take = plan_for(&game, i, Order::Take(1), false, (home.0, home.1), 9);
        assert!(take.alt >= game.rules.treetop, "{take:?}");
        let standing = standing_for(&game, i, Order::Take(1), Order::Home, &take, (home.0, home.1), 10);
        assert_eq!((standing.primitive, standing.lat, standing.lon), (take.primitive, take.lat, take.lon));
        assert!(standing.alt >= game.rules.treetop + 30.0);
    }

    #[test]
    fn a_transport_names_the_connection_of_each_drone() {
        let tcp = Transport::Tcp { host: "127.0.0.1".into(), ports: vec![6001, 6002] };
        assert_eq!(tcp.url(1).as_deref(), Some("tcpout:127.0.0.1:6002"));
        assert_eq!(tcp.url(2), None);
        let serial = Transport::Serial { paths: vec!["/dev/cu.usbserial-0".into()], baud: 57600 };
        assert_eq!(serial.url(0).as_deref(), Some("serial:/dev/cu.usbserial-0:57600"));
        assert_eq!(serial.url(1), None, "a drone with no radio has no connection");
    }
}
