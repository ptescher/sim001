//! The "Flying by plan" section of the suite (expectations/07-cognition), run with no window and
//! no SITL: the real executor (the code linked into ArduPlane's AGENT mode) flies a small
//! point-mass plane (`support/plane.rs`, the unit tests' model) at 50 Hz, with the T1-S limits of
//! `ardupilot/t1s.parm`. Plans go through the ground's real wire format
//! (`cognition_plan::WirePlan`, a `COMMAND_INT` and back) and the tag decisions come from the real
//! scripted brain (`cognition_agent::ScriptedBrain`).
//!
//! What `mode_agent.cpp` adds round the executor is mirrored here, line for line, from the patch
//! (`ardupilot/patches/0002-agent-executor.patch`): the reply to a plan (refused outside AGENT,
//! refused if the wire decode fails, else accepted and handed to the executor every step), the
//! status (AG_PLAN and AG_PRIM are the plan last accepted, AG_ENV counts the steps the executor
//! says it intervened, AG_FALL is its last fallback), entering AGENT (the executor and the status
//! are reset) and a lost position estimate (NaN). The real AGENT mode is tested by the runner's
//! acceptance test, in SITL; where both run a test, the docs show the SITL result.

#[path = "support/plane.rs"]
mod plane;

use cognition_agent::{Brain, Craft, GameView, Order, Picture, ScriptedBrain};
use cognition_executor::{Executor, Limits, Plan, Primitive, Track};
use cognition_plan::{WirePlan, to_global};
use expectation_harness::{Acceptance, Bindings, Driver, Need, Progress, RunOptions, Screenshot};
use expectation_model::{Event, Value, Variable};
use expectation_story::Status;
use glam::Vec3;
use plane::{DT, Plane};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::path::Path;

/// SITL's home (`simulation_sitl::launch::HOME`): where local north-east (0, 0) is.
const HOME: (f64, f64) = (-35.363261, 149.165230);
/// The centre of the fence (AGENT_FENCE_N, AGENT_FENCE_E): the middle of the tag arena.
const ARENA: [f32; 2] = [70.0, 0.0];
const G: f32 = 9.80665;
/// The other aircraft's id (its track id, the target of a pursue or evade plan).
const OTHER: u8 = 2;
/// The time the plane holds the plan of a "last minute" measure over (s).
const MINUTE: f32 = 60.0;

/// The AGENT limits of `ardupilot/t1s.parm`, as `ModeAgent::fill_limits` fills them.
fn limits(fence_radius: f32, separation: f32) -> Limits {
    Limits {
        fence_centre: ARENA,
        fence_radius,
        ceiling: 60.0,
        floor: 12.0,
        bank_max: 50f32.to_radians(),
        pitch_max: 25f32.to_radians(),
        pitch_min: -25f32.to_radians(),
        // AIRSPEED_MIN 11 + AGENT_ASPD_MRG 2
        airspeed_min: 13.0,
        airspeed_cruise: 15.0,
        airspeed_max: 22.0,
        separation,
        grace: 0.0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Agent,
    Fbwa,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Circling,
    Chasing,
    Crossing,
}

/// The other aircraft: how it flies, and the track the plane has of it.
struct Other {
    kind: Kind,
    plane: Plane,
    heard: Track,
    silent: bool,
}

impl Other {
    fn new(kind: Kind, me: &Plane) -> Other {
        let plane = match kind {
            // Clockwise round the arena centre, 40 m out, at 16 m/s and 35 m.
            Kind::Circling => {
                let mut p = Plane::new(ARENA[0], ARENA[1] - 40.0, 35.0, 0.0, 16.0);
                p.roll = (16.0f32 * 16.0 / (G * 40.0)).atan();
                p
            }
            // 30 m behind, at our height and heading.
            Kind::Chasing => {
                Plane::new(me.pos[0] - 30.0 * me.psi.cos(), me.pos[1] - 30.0 * me.psi.sin(), me.h(), me.psi, 15.0)
            }
            // From 60 m ahead and 60 m to the right, flying west across our path at our height.
            Kind::Crossing => Plane::new(me.pos[0] + 60.0, me.pos[1] + 60.0, me.h(), -FRAC_PI_2, 15.0),
        };
        Other { kind, plane, heard: plane.as_track(OTHER), silent: false }
    }

    fn step(&mut self, me: &Plane) {
        let (v, h) = (self.plane.v, self.plane.h());
        match self.kind {
            Kind::Circling => {
                let bank = self.plane.roll;
                self.plane.fly(bank, 0.0, 0.45);
            }
            // Pure pursuit, less agile than we are.
            Kind::Chasing => {
                let bearing = (me.pos[1] - self.plane.pos[1]).atan2(me.pos[0] - self.plane.pos[0]);
                let err = (bearing - self.plane.psi + PI).rem_euclid(TAU) - PI;
                self.plane.fly((err * 1.5).clamp(-0.6, 0.6), 0.0, 0.45);
            }
            Kind::Crossing => self.plane.fly(0.0, 0.0, 0.45),
        }
        self.plane.v = v;
        self.plane.pos[2] = -h;
        if self.silent {
            self.heard.age += DT;
        } else {
            self.heard = self.plane.as_track(OTHER);
        }
    }
}

/// One sample of the flight, for the "last minute" measures.
#[derive(Clone, Copy)]
struct Sample {
    t: f32,
    to_point: f32,
    h: f32,
    to_other: f32,
    /// AG_ENV.
    env: u32,
}

/// The larger of the horizontal and the vertical distance: what the separation counts.
fn apart(a: [f32; 3], b: [f32; 3]) -> f32 {
    let horizontal = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    horizontal.max((a[2] - b[2]).abs())
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn primitive_name(p: u8) -> &'static str {
    match Primitive::from_u8(p) {
        Some(Primitive::Pursue) => "pursue",
        Some(Primitive::Evade) => "evade",
        Some(Primitive::Goto) => "go to",
        Some(Primitive::Orbit) => "orbit",
        Some(Primitive::Band) => "band",
        Some(Primitive::Loiter) => "loiter",
        _ => "none",
    }
}

struct World {
    t: f32,
    mode: Mode,
    plane: Plane,
    start: String,
    other_kind: Option<Kind>,
    other: Option<Other>,
    built: bool,
    limits: Limits,
    ex: Executor,
    /// `ModeAgent::plan` and `have_plan`: the plan last accepted, handed to the executor each step.
    plan: Option<Plan>,
    reply: &'static str,
    /// AG_ENV and AG_FALL.
    interventions: u32,
    fallback: u8,
    /// "Agent: fallback 3" messages: the plane says each change of the fallback.
    recoveries: u32,
    pilot_throttle: bool,
    position_lost: bool,
    /// The point of the plan (north, east), for the measures; a loiter's is where it was adopted.
    point: Option<[f32; 2]>,
    loiter_pending: bool,
    closest_to_point: f32,
    samples: Vec<Sample>,
    lowest: f32,
    highest: f32,
    farthest: f32,
    closest_other: f32,
    closest_pair: f32,
    /// The plane's track and the other aircraft's, for the pictures (every 0.2 s).
    trail: Vec<[f32; 3]>,
    other_trail: Vec<[f32; 3]>,
    /// Tag: the arranged start of the game and the brain's last decision.
    game: Option<String>,
    orders: Vec<Order>,
}

impl World {
    fn new() -> World {
        World {
            t: 0.0,
            mode: Mode::Agent,
            plane: Plane::new(ARENA[0], ARENA[1], 30.0, 0.0, 15.0),
            start: "a level pass over the arena centre".into(),
            other_kind: None,
            other: None,
            built: false,
            limits: limits(85.0, 4.0),
            ex: Executor::new(),
            plan: None,
            reply: "none",
            interventions: 0,
            fallback: 0,
            recoveries: 0,
            pilot_throttle: false,
            position_lost: false,
            point: None,
            loiter_pending: false,
            closest_to_point: f32::MAX,
            samples: vec![],
            lowest: f32::MAX,
            highest: 0.0,
            farthest: 0.0,
            closest_other: f32::MAX,
            closest_pair: f32::MAX,
            trail: vec![],
            other_trail: vec![],
            game: None,
            orders: vec![],
        }
    }

    /// Puts the plane and the other aircraft where the arranged start says (the start is
    /// arranged state by state, in any order).
    fn build(&mut self) {
        if std::mem::replace(&mut self.built, true) {
            return;
        }
        let (n, e) = (ARENA[0], ARENA[1]);
        let mut p = match self.start.as_str() {
            "a level pass over home" => Plane::new(0.0, 0.0, 30.0, 0.0, 15.0),
            "a dive at the ground" => Plane::new(n, e, 16.0, 0.0, 22.0),
            "a steep climb" => Plane::new(n, e, 56.0, 0.0, 24.0),
            "a slow climb" => Plane::new(n, e, 22.0, 0.0, 10.0),
            "a run at the fence" => Plane::new(n + 50.0, e, 30.0, 0.0, 15.0),
            _ => Plane::new(n, e, 30.0, 0.0, 15.0),
        };
        p.pitch = match self.start.as_str() {
            "a dive at the ground" => -0.6,
            "a steep climb" => 0.5,
            "a slow climb" => 0.4,
            _ => 0.0,
        };
        self.plane = p;
        self.other = self.other_kind.map(|k| Other::new(k, &self.plane));
        self.measure();
    }

    fn tracks(&self) -> Vec<Track> {
        self.other.iter().map(|o| o.heard).collect()
    }

    fn step(&mut self) {
        self.build();
        if let Some(o) = &mut self.other {
            o.step(&self.plane);
        }
        match self.mode {
            Mode::Agent => {
                let mut s = self.plane.state();
                if self.position_lost {
                    // ModeAgent::fill_state: what the EKF doesn't have is NaN.
                    s.pos = [f32::NAN; 3];
                    s.vel = [f32::NAN; 3];
                }
                let d = self.ex.step(&s, self.plan.as_ref(), &self.tracks(), &self.limits);
                self.interventions += d.intervened as u32;
                if d.fallback != self.fallback && d.fallback == 3 {
                    self.recoveries += 1;
                }
                self.fallback = d.fallback;
                // mode_agent.cpp: `if (!(demand.throttle >= 0))` the pilot's throttle.
                self.pilot_throttle = d.throttle.is_nan() || d.throttle < 0.0;
                if std::mem::take(&mut self.loiter_pending) {
                    self.point = Some([self.plane.pos[0], self.plane.pos[1]]);
                }
                self.plane.fly_demand(&d);
            }
            // FBWA with the sticks centred: wings level, the pilot's throttle.
            Mode::Fbwa => self.plane.fly(0.0, 0.0, -1.0),
        }
        self.t += DT;
        self.measure();
    }

    fn measure(&mut self) {
        let p = self.plane.pos;
        let to_point = self.point.map_or(f32::MAX, |c| self.plane.dist2(c));
        self.closest_to_point = self.closest_to_point.min(to_point);
        let to_other = self.other.as_ref().map_or(f32::MAX, |o| dist(p, o.plane.pos));
        if let Some(o) = &self.other {
            self.closest_other = self.closest_other.min(apart(p, o.plane.pos));
        }
        self.samples.push(Sample { t: self.t, to_point, h: self.plane.h(), to_other, env: self.interventions });
        self.lowest = self.lowest.min(self.plane.h());
        self.highest = self.highest.max(self.plane.h());
        self.farthest = self.farthest.max(self.plane.dist2(ARENA));
        if ((self.t / DT).round() as u32).is_multiple_of(10) {
            self.trail.push(p);
            if let Some(o) = &self.other {
                self.other_trail.push(o.plane.pos);
            }
        }
    }

    fn run(&mut self, seconds: f32) {
        let steps = (seconds / DT).round() as usize;
        for _ in 0..steps {
            self.step();
        }
    }

    /// The samples of the last minute.
    fn last_minute(&self) -> impl Iterator<Item = &Sample> {
        let from = self.t - MINUTE;
        self.samples.iter().filter(move |s| s.t >= from - 1e-3)
    }

    /// A plan as the runner's console sends it: `plan PRIMITIVE NORTH EAST ALT RADIUS TTL [TARGET]`,
    /// with the id one more than the plan the plane holds (or the same id, `again`). It goes
    /// through the wire format, and the plane answers as `ModeAgent::handle_plan` does.
    #[allow(clippy::too_many_arguments)]
    fn send(&mut self, primitive: u8, north: f32, east: f32, alt: f32, radius: f32, ttl: f32, again: bool) {
        self.build();
        let held = self.plan.map_or(0, |p| p.id);
        let id = if again { held } else { held.wrapping_add(1) };
        let target =
            if matches!(Primitive::from_u8(primitive), Some(Primitive::Pursue | Primitive::Evade)) { OTHER } else { 0 };
        let (lat, lon) = to_global(HOME.0, HOME.1, north, east);
        let wire = WirePlan { id, primitive: Primitive::Goto, target, lat, lon, alt, radius, ttl };
        let mut command = wire.to_command();
        command.param1 = primitive as f32;
        if self.mode != Mode::Agent {
            self.reply = "refused for the mode";
            return;
        }
        let Some(decoded) = WirePlan::from_command(&command) else {
            self.reply = "refused as malformed";
            return;
        };
        self.reply = "accepted";
        let plan = decoded.to_plan(HOME.0, HOME.1);
        let new = self.plan.map(|p| p.id) != Some(plan.id);
        self.plan = Some(plan);
        if new {
            self.point = match decoded.primitive {
                Primitive::Goto | Primitive::Orbit | Primitive::Band => Some([plan.point[0], plan.point[1]]),
                _ => None,
            };
            self.loiter_pending = decoded.primitive == Primitive::Loiter;
            self.closest_to_point = f32::MAX;
            self.samples.clear();
            self.trail.clear();
            self.other_trail.clear();
        }
    }

    /// `ModeAgent::_enter`.
    fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Agent && self.mode != Mode::Agent {
            self.ex.reset();
            self.plan = None;
            self.interventions = 0;
            self.fallback = 0;
        }
        self.mode = mode;
    }

    // ---- Tag ------------------------------------------------------------------------------

    /// The picture the ground has at the arranged start of the game: Kestrel (1) is it.
    fn picture(&self) -> Picture {
        // (id, north, east, height, speed north)
        let planes: [(u8, f32, f32, f32, f32); 3] = match self.game.as_deref() {
            Some("behind Merlin and ahead of Swift") => {
                [(1, 70.0, 0.0, 25.0, 18.0), (2, 110.0, 0.0, 40.0, 12.0), (3, 40.0, 0.0, 52.0, 18.0)]
            }
            Some("near Merlin") => {
                [(1, 70.0, 0.0, 30.0, 18.0), (2, 70.0, 50.0, 30.0, 18.0), (3, 70.0, -75.0, 52.0, 18.0)]
            }
            _ => [(1, 70.0, 0.0, 25.0, 18.0), (2, 70.0, 70.0, 40.0, 18.0), (3, 70.0, -70.0, 52.0, 18.0)],
        };
        let craft = |(id, n, e, h, v): (u8, f32, f32, f32, f32)| Craft {
            id,
            pos: Vec3::new(n, e, -h),
            vel: Vec3::new(v, 0.0, 0.0),
            flying: true,
        };
        Picture {
            time: self.t,
            home: HOME,
            planes: planes.into_iter().map(craft).collect(),
            game: GameView { it: 1, time_left: 100.0, ..Default::default() },
        }
    }

    fn order(&self, id: u8) -> Option<&WirePlan> {
        self.orders.iter().find(|o| o.plane == id).map(|o| &o.plan)
    }

    fn plan_of(&self, id: u8) -> &'static str {
        self.order(id).map_or("none", |p| primitive_name(p.primitive as u8))
    }

    fn band_of(&self, id: u8) -> String {
        match self.order(id) {
            Some(p) if matches!(p.primitive, Primitive::Band | Primitive::Evade) => {
                format!("{} to {} m", p.alt.round(), (p.alt + p.radius).round())
            }
            _ => "none".into(),
        }
    }

    // ---- Three planes on random plans -----------------------------------------------------

    /// Three planes, each with its own executor, on random plans (a new one every 8 s, pursuing
    /// and evading each other among them) for 20 minutes in a light wind, as the executor's
    /// cooperative soak does.
    fn crowd(&mut self) {
        let separation = self.limits.separation;
        let l = self.limits;
        let mut r = Lcg(100);
        let wind = [r.range(-3.0, 3.0), r.range(-3.0, 3.0)];
        let mut planes: Vec<(Executor, Plane, Plan)> = (0..3)
            .map(|k| {
                let a = k as f32 * TAU / 3.0;
                let mut p = Plane::new(
                    ARENA[0] + 40.0 * a.cos(),
                    ARENA[1] + 40.0 * a.sin(),
                    25.0 + 8.0 * k as f32,
                    a + 1.6,
                    15.0,
                );
                p.wind = wind;
                (Executor::new(), p, Plan::default())
            })
            .collect();
        let steps = (1200.0 / DT) as usize;
        for i in 0..steps {
            let tracks: Vec<Track> = planes.iter().enumerate().map(|(k, (_, p, _))| p.as_track(k as u8 + 1)).collect();
            for (k, (x, a, p)) in planes.iter_mut().enumerate() {
                if (i + 133 * k) % 400 == 0 {
                    let target = ((k as u32 + 1 + r.next() % 2) % 3) as u8 + 1;
                    *p = random_plan(&mut r, p.id.wrapping_add(1), target);
                }
                let others: Vec<Track> = tracks.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, t)| *t).collect();
                let d = x.step(&a.state(), Some(p), &others, &l);
                a.fly_demand(&d);
                self.farthest = self.farthest.max(a.dist2(ARENA));
                self.lowest = self.lowest.min(a.h());
                self.highest = self.highest.max(a.h());
            }
            for j in 0..3 {
                for k in j + 1..3 {
                    self.closest_pair = self.closest_pair.min(apart(planes[j].1.pos, planes[k].1.pos));
                }
            }
        }
        eprintln!(
            "three planes, fence {} m, separation {separation} m: closest pair {:.1} m, farthest {:.1} m, lowest {:.1} m",
            l.fence_radius, self.closest_pair, self.farthest, self.lowest
        );
    }

    // ---- The picture ----------------------------------------------------------------------

    /// A top view of the flight, as a map would show it: the fence, home, the point of the plan,
    /// the plane's track and the other aircraft's (north up).
    fn svg(&self) -> String {
        let (w, h) = (360.0f32, 360.0f32);
        let span = 230.0f32; // metres across
        let x = |e: f32| w / 2.0 + (e - ARENA[1]) * w / span;
        let y = |n: f32| h / 2.0 - (n - ARENA[0]) * h / span;
        let mut s = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {w} {h}' width='{w}' height='{h}' font-family='sans-serif' font-size='11'>\
             <rect width='{w}' height='{h}' fill='#f4f1e8'/>"
        );
        let r = self.limits.fence_radius * w / span;
        s += &format!(
            "<circle cx='{}' cy='{}' r='{r}' fill='#e6efe0' stroke='#c0392b' stroke-width='1.5' stroke-dasharray='6 4'/>",
            x(ARENA[1]),
            y(ARENA[0])
        );
        s += &format!("<text x='{}' y='{}' fill='#c0392b'>fence</text>", x(ARENA[1]) - 14.0, y(ARENA[0] + 85.0) + 14.0);
        s += &format!(
            "<rect x='{}' y='{}' width='8' height='8' fill='#555'/><text x='{}' y='{}' fill='#555'>home</text>",
            x(0.0) - 4.0,
            y(0.0) - 4.0,
            x(0.0) + 7.0,
            y(0.0) + 4.0
        );
        let path = |pts: &[[f32; 3]], colour: &str| {
            if pts.len() < 2 {
                return String::new();
            }
            let d: Vec<String> = pts.iter().map(|p| format!("{:.1},{:.1}", x(p[1]), y(p[0]))).collect();
            format!("<polyline points='{}' fill='none' stroke='{colour}' stroke-width='1.6'/>", d.join(" "))
        };
        s += &path(&self.other_trail, "#8e44ad");
        s += &path(&self.trail, "#1f5fa8");
        if let Some(c) = self.point {
            s += &format!(
                "<circle cx='{}' cy='{}' r='4' fill='none' stroke='#e67e22' stroke-width='2'/><text x='{}' y='{}' fill='#e67e22'>point</text>",
                x(c[1]),
                y(c[0]),
                x(c[1]) + 6.0,
                y(c[0]) - 6.0
            );
        }
        if let Some(o) = &self.other {
            s += &format!("<circle cx='{}' cy='{}' r='4' fill='#8e44ad'/>", x(o.plane.pos[1]), y(o.plane.pos[0]));
        }
        let p = self.plane;
        let (px, py) = (x(p.pos[1]), y(p.pos[0]));
        let (dx, dy) = (p.psi.sin() * 9.0, -p.psi.cos() * 9.0);
        s += &format!(
            "<polygon points='{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}' fill='#1f5fa8'/>",
            px + dx,
            py + dy,
            px - dx * 0.6 - dy * 0.5,
            py - dy * 0.6 + dx * 0.5,
            px - dx * 0.6 + dy * 0.5,
            py - dy * 0.6 - dx * 0.5
        );
        let mode = if self.mode == Mode::Agent { "AGENT" } else { "FBWA" };
        let prim = self.plan.map_or(0, |p| p.primitive);
        s += &format!(
            "<text x='8' y='16' fill='#222'>{mode}  AG_PLAN {}  AG_PRIM {prim}  AG_ENV {}  AG_FALL {}</text>\
             <text x='8' y='{}' fill='#222'>{:.0} m high, {:.1} m/s, {:.0} s</text></svg>",
            self.plan.map_or(0, |p| p.id),
            self.interventions,
            self.fallback,
            h - 8.0,
            p.h(),
            p.v,
            self.t
        );
        s
    }
}

/// A linear congruential generator (Numerical Recipes' constants), as the unit tests use.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (self.next() as f32 / (1u64 << 31) as f32)
    }
}

fn random_plan(r: &mut Lcg, id: u16, target: u8) -> Plan {
    Plan {
        id,
        primitive: (r.next() % 7) as u8,
        target,
        point: [ARENA[0] + r.range(-200.0, 200.0), ARENA[1] + r.range(-200.0, 200.0), r.range(-100.0, 0.0)],
        radius: r.range(0.0, 150.0),
        band: [r.range(0.0, 50.0), r.range(0.0, 80.0)],
        ttl: r.range(4.0, 12.0),
    }
}

fn kind(v: &Value) -> Option<Kind> {
    match v.as_str() {
        Some("round the arena") => Some(Kind::Circling),
        Some("after your plane") => Some(Kind::Chasing),
        Some("across your path") => Some(Kind::Crossing),
        _ => None,
    }
}

fn bindings() -> Bindings<World> {
    let low = |w: &World, f: fn(&Sample) -> f32| w.last_minute().map(f).fold(f32::MAX, f32::min) as f64;
    let high = |w: &World, f: fn(&Sample) -> f32| w.last_minute().map(f).fold(0.0, f32::max) as f64;
    Bindings::new(World::new())
        .observe("the flight mode of the plane", |w| if w.mode == Mode::Agent { "AGENT" } else { "FBWA" })
        .set("the flight mode of the plane", |w, v| {
            w.build();
            w.set_mode(if v.as_str() == Some("FBWA") { Mode::Fbwa } else { Mode::Agent })
        })
        .arrange("the flight mode of the plane", |w, v| {
            w.mode = if v.as_str() == Some("FBWA") { Mode::Fbwa } else { Mode::Agent }
        })
        .observe("the start", |w| w.start.clone())
        .arrange("the start", |w, v| w.start = v.as_str().unwrap_or_default().to_string())
        .observe("the other aircraft", |w| match w.other.as_ref().map(|o| o.kind).or(w.other_kind) {
            None => "far away",
            Some(Kind::Circling) => "round the arena",
            Some(Kind::Chasing) => "after your plane",
            Some(Kind::Crossing) => "across your path",
        })
        .arrange("the other aircraft", |w, v| w.other_kind = kind(v))
        .set("the other aircraft", |w, v| {
            w.build();
            w.other_kind = kind(v);
            w.other = w.other_kind.map(|k| Other::new(k, &w.plane));
        })
        .observe("the track of the other aircraft", |w| {
            if w.other.as_ref().is_some_and(|o| o.silent) { "not heard" } else { "heard" }
        })
        .arrange("the track of the other aircraft", |_, _| {})
        .set("the track of the other aircraft", |w, v| {
            w.build();
            let silent = v.as_str() == Some("not heard");
            w.other.iter_mut().for_each(|o| o.silent = silent);
        })
        .observe("the separation limit", |w| w.limits.separation as f64)
        .set("the separation limit", |w, v| w.limits.separation = v.as_f64().unwrap() as f32)
        .observe("the fence radius", |w| w.limits.fence_radius as f64)
        .set("the fence radius", |w, v| w.limits.fence_radius = v.as_f64().unwrap() as f32)
        // What the plane says: arranged only as it is after entering AGENT (see `can_arrange`).
        .observe("the reply to the plan", |w| w.reply)
        .arrange("the reply to the plan", |_, _| {})
        .observe("the time to live of the plan", |w| w.plan.map_or(0.0, |p| p.ttl) as f64)
        .arrange("the time to live of the plan", |_, _| {})
        .observe("the reported plan", |w| w.plan.map_or(0, |p| p.id) as f64)
        .arrange("the reported plan", |_, _| {})
        .observe("the reported primitive", |w| primitive_name(w.plan.map_or(0, |p| p.primitive)))
        .arrange("the reported primitive", |_, _| {})
        .observe("the reported fallback", |w| match w.fallback {
            1 => "plan expired",
            2 => "bad input",
            3 => "recovery",
            _ => "none",
        })
        .arrange("the reported fallback", |_, _| {})
        .observe("the messages about a recovery", |w| w.recoveries as f64)
        .arrange("the messages about a recovery", |_, _| {})
        .observe("the reported interventions", |w| w.interventions as f64)
        .arrange("the reported interventions", |_, _| {})
        .observe("the throttle of the plane", |w| if w.pilot_throttle { "the pilot's" } else { "the executor's" })
        .arrange("the throttle of the plane", |_, _| {})
        // The measures of the flight.
        .observe("the closest approach to the point", |w| w.closest_to_point as f64)
        .observe("the least distance from the point in the last minute", move |w| low(w, |s| s.to_point))
        .observe("the greatest distance from the point in the last minute", move |w| high(w, |s| s.to_point))
        .observe("the lowest height in the last minute", move |w| low(w, |s| s.h))
        .observe("the highest height in the last minute", move |w| high(w, |s| s.h))
        .observe("the greatest distance from the other aircraft in the last minute", move |w| high(w, |s| s.to_other))
        .observe("the interventions in the last minute", |w| {
            let mut m = w.last_minute();
            let first = m.next().map_or(0, |s| s.env);
            (m.last().map_or(first, |s| s.env) - first) as f64
        })
        .observe("the lowest height of the flight", |w| w.lowest as f64)
        .observe("the highest height of the flight", |w| w.highest as f64)
        .observe("the farthest distance from the arena centre", |w| w.farthest as f64)
        .observe("the closest distance to the other aircraft", |w| w.closest_other as f64)
        .observe("the distance to the other aircraft", |w| {
            w.other.as_ref().map_or(f32::MAX, |o| dist(w.plane.pos, o.plane.pos)) as f64
        })
        .observe("the airspeed of the plane", |w| w.plane.v as f64)
        .observe("the closest distance between two planes", |w| w.closest_pair as f64)
        // The plans, as the console sends them.
        .on("the ground sends a go-to plan", |w| w.send(3, 80.0, 10.0, 40.0, 0.0, 600.0, false))
        .on("the ground sends a short go-to plan", |w| w.send(3, 80.0, 10.0, 40.0, 0.0, 5.0, false))
        .on("the ground sends an orbit plan", |w| w.send(4, 70.0, 0.0, 45.0, 45.0, 600.0, false))
        .on("the ground sends a band plan", |w| w.send(5, 70.0, 0.0, 35.0, 10.0, 600.0, false))
        .on("the ground sends a loiter plan", |w| w.send(6, 0.0, 0.0, 0.0, 0.0, 600.0, false))
        .on("the ground sends a pursue plan", |w| w.send(1, 0.0, 0.0, 0.0, 0.0, 600.0, false))
        .on("the ground sends an evade plan", |w| w.send(2, 0.0, 0.0, 0.0, 0.0, 600.0, false))
        .on("the ground sends a malformed plan", |w| w.send(9, 80.0, 10.0, 40.0, 0.0, 600.0, false))
        .on("the ground sends the plan id again with an orbit", |w| w.send(4, 70.0, 0.0, 45.0, 45.0, 600.0, true))
        .on("the plane loses its position estimate", |w| w.position_lost = true)
        .on("the plane gets its position estimate back", |w| w.position_lost = false)
        .on("the plane flies on for 1 second", |w| w.run(1.0))
        .on("the plane flies on for 5 seconds", |w| w.run(5.0))
        .on("the plane flies on for 10 seconds", |w| w.run(10.0))
        .on("the plane flies on for 30 seconds", |w| w.run(30.0))
        .on("the plane flies on for 2 minutes", |w| w.run(120.0))
        .on("the plane flies on for 10 minutes", |w| w.run(600.0))
        .on("three planes fly random plans for 20 minutes", World::crowd)
        // Tag: the scripted brain's decision on the arranged start of the game.
        .arrange("the chaser Kestrel", |w, v| w.game = v.as_str().map(str::to_string))
        .observe("the chaser Kestrel", |w| w.game.clone().unwrap_or_else(|| "far from the others".into()))
        .on("the ground control agent decides", |w| w.orders = ScriptedBrain::computer_pilots().decide(&w.picture()))
        .observe("the plane that Kestrel pursues", |w| match w.order(1) {
            Some(p) if p.primitive == Primitive::Pursue => {
                ["none", "Kestrel", "Merlin", "Swift"][p.target.min(3) as usize]
            }
            _ => "none",
        })
        .observe("the plan of Merlin", |w| w.plan_of(2))
        .observe("the plan of Swift", |w| w.plan_of(3))
        .observe("the band of Merlin", |w| w.band_of(2))
        .observe("the band of Swift", |w| w.band_of(3))
        .on_reset(|w| *w = World::new())
        // Let the executor take a step or two after each action, as the plane does within 0.1 s.
        .on_settle(|w| w.run(0.1))
        .on_screenshot(|w| Screenshot::svg(w.svg()))
}

/// The bindings, with the states that are only results (the reply, the status) arranged only as
/// they are when the plane has just entered AGENT mode: other values are "not supported yet".
struct Windowless(Bindings<World>, bool);

impl Driver for Windowless {
    fn reset(&mut self) -> Result<(), String> {
        self.1 = false;
        Driver::reset(&mut self.0)
    }
    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        Driver::arrange(&mut self.0, var, value)
    }
    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        self.1 = true;
        Driver::set(&mut self.0, var, value)
    }
    fn perform(&mut self, event: &Event) -> Result<(), String> {
        self.1 = true;
        Driver::perform(&mut self.0, event)
    }
    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        Driver::observe(&mut self.0, var)
    }
    fn can_arrange(&self, var: &Variable, value: &Value) -> bool {
        let fresh = match var.subject.as_str() {
            "the reply to the plan" | "the reported primitive" | "the reported fallback" => {
                value.as_str() == Some("none")
            }
            "the time to live of the plan"
            | "the reported plan"
            | "the reported interventions"
            | "the messages about a recovery" => value.as_f64() == Some(0.0),
            "the throttle of the plane" => value.as_str() == Some("the executor's"),
            "the track of the other aircraft" => value.as_str() == Some("heard"),
            _ => true,
        };
        fresh && Driver::can_arrange(&self.0, var, value)
    }
    fn settle(&mut self) {
        // The start is checked as arranged, before the plane has flown a step.
        if std::mem::take(&mut self.1) { Driver::settle(&mut self.0) } else { self.0.world.build() }
    }
    fn advance(&mut self) -> Progress {
        // Deterministic: what the step's actions left is what is checked.
        Progress::Static
    }
    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        Driver::screenshot(&mut self.0)
    }
    fn supports(&self, need: Need<'_>) -> bool {
        Driver::supports(&self.0, need)
    }
}

fn driver() -> Windowless {
    Windowless(bindings(), false)
}

/// Tests that fail because the system does not do what the spec says yet. Each is a bug to fix;
/// remove it from here when it passes.
///
/// - «A repeated plan id changes nothing»: `ModeAgent::handle_plan` copies every accepted plan
///   over the one it holds, so AG_PRIM reports the primitive of a repeated id although the
///   executor (rightly) goes on flying the first plan. `cognition_plan::status_names::PRIMITIVE`
///   says AG_PRIM is the primitive being flown.
///
/// - «The envelope leaves a safe plan alone»: with `ardupilot/t1s.parm` the cruise airspeed
///   (`AIRSPEED_CRUISE` 15) is the edge of the executor's stall margin (`AIRSPEED_MIN` 11 +
///   `AGENT_ASPD_MRG` 2 + `tuning::STALL_MARGIN` 2), so the stall rule changes the demand on about
///   one step in eight of a calm orbit. With a cruise of 18 m/s it never does. The fix is in the
///   tuning (the parameters or the margin), not the spec.
const KNOWN_FAILURES: &[&str] = &["A repeated plan id changes nothing", "The envelope leaves a safe plan alone"];

fn suite() -> Acceptance {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage()
}

#[test]
fn every_test_of_the_section_passes_with_no_window() {
    let a = suite();
    let section = a.suite.sections.iter().find(|s| s.slug == "cognition").expect("an cognition section").id;
    let ours = a.suite.subtree(section);
    let report = a.run(&mut driver(), &RunOptions::default());
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {} {:?}", r.status, p.title, r.start.error);
        for (step, sr) in p.steps.iter().zip(&r.steps) {
            for (c, cr) in step.checks.iter().zip(&sr.checks) {
                let var = a.suite.vocabulary.var(c.var);
                println!("    {}: {:?}", var.describe(&c.expect), cr.observed.as_ref().map(|v| v.to_string()));
            }
        }
    }
    println!("{}", report.failures(&a.suite, &a.plans));
    let passed = ran.iter().filter(|(_, r)| r.status == Status::Passed).count();
    assert!(passed >= 15, "only {passed} tests of the section ran and passed");
    let known = |title: &str| KNOWN_FAILURES.iter().any(|k| title.contains(k));
    let bad: Vec<_> = ran
        .iter()
        .filter(|(p, r)| r.status == Status::Failed && !known(&p.title))
        .map(|(p, r)| format!("{}: {:?}", p.title, r.status))
        .collect();
    assert!(bad.is_empty(), "{}\n{}", bad.join("\n"), report.failures(&a.suite, &a.plans));
    let fixed: Vec<_> = ran
        .iter()
        .filter(|(p, r)| known(&p.title) && r.status == Status::Passed)
        .map(|(p, _)| p.title.clone())
        .collect();
    assert!(fixed.is_empty(), "these pass now; take them out of KNOWN_FAILURES: {fixed:?}");
}

/// The same, with a top view of the flight at each step. With SIM001_DOCS=DIR the results and
/// pictures go into the project's docs, beside the other runners'.
#[test]
fn the_section_documents_itself_with_pictures_of_the_flight() {
    let a = suite();
    let published = std::env::var("SIM001_DOCS").ok().map(std::path::PathBuf::from);
    let out = published.clone().unwrap_or_else(|| Path::new(env!("CARGO_TARGET_TMPDIR")).join("cognition_docs"));
    let report = a.run(&mut driver(), &RunOptions { docs_dir: Some(out.clone()), ..Default::default() });
    if published.is_some() {
        a.publish(&out, "cognition", &report).unwrap();
    } else {
        a.docs(Some(&report)).write(&out).unwrap();
    }
    let any = std::fs::read_dir(out.join("screenshots"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains("cognition--coverage--go-to-flies-over-the-point"));
    assert!(any, "no pictures of the flight in {}", out.display());
}
