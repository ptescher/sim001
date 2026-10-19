//! The plan executor, on board. `no_std`, no allocation, no panics, a C ABI: ArduPlane's AGENT
//! mode calls it every control step (about 50 Hz), and the training environment calls the same
//! code ([`Executor`]), so what is trained is what flies. The types are `cognition_plan`'s; the
//! header is `include/agent_executor.h`.
//!
//! Each step:
//! 1. **Inputs.** A NaN, an infinity or an absurd value in the state or the limits (airspeed
//!    below 0, `dt` outside (0, 0.5] s...) gives wings level, nose level and the throttle to the
//!    pilot, with [`Fallback::BadInput`]. A bad track is ignored; so is a bad plan.
//! 2. **The plan.** One whose id differs from the held one is adopted (its ttl counts from this
//!    step); a repeated id is ignored. When `ttl + grace` seconds pass without a new one, the plane
//!    loiters where it is ([`Fallback::PlanExpired`]).
//! 3. **The primitive** (`cognition_plan::Primitive`) gives a lateral acceleration (L1 guidance to
//!    a point, or a vector field round a circle) and a height and an airspeed, which a small
//!    energy controller turns into pitch and throttle. The primitive keeps within the bank and
//!    pitch limits by itself.
//! 4. **The safety envelope** comes after and can't be bypassed: separation from every track,
//!    the geofence, the ceiling, the floor, the stall margin and the attitude limits, in that
//!    order (the later wins). The fence's turn back is decided first, and a separation escape
//!    must turn the same way unless the turn back can wait (so where the two can't both be had,
//!    the fence wins). `Demand::intervened` is 1 when the envelope changed the demand, and
//!    [`Executor::rules`] says which rules did.
//!
//! All the arithmetic is this crate's own (`math`), in every build, so the training build and
//! the one on board compute the same numbers.

#![cfg_attr(feature = "ffi-static", no_std)]

mod math;

use math::*;

pub use cognition_plan::{Demand, Fallback, Limits, MAX_TRACKS, Plan, Primitive, State, Track};

/// Bumped when the C ABI changes, so the C++ side can refuse a library that does not match.
pub const ABI_VERSION: u32 = 2;

/// The numbers that shape how the executor flies. Distances in m, speeds in m/s, angles in rad.
pub mod tuning {
    /// A track not heard for longer than this is ignored (s).
    pub const TRACK_STALE: f32 = 3.0;
    /// The longest step taken as sane (s).
    pub const DT_MAX: f32 = 0.5;

    /// L1 guidance: period (s) and damping, as ArduPilot's `NAVL1_PERIOD` / `NAVL1_DAMPING`. The
    /// L1 distance is `period * damping * v / π` (about 43 m at 18 m/s), or the distance to the
    /// point if that is shorter, but not under `L1_MIN`.
    pub const L1_PERIOD: f32 = 10.0;
    pub const L1_DAMPING: f32 = 0.75;
    pub const L1_MIN: f32 = 8.0;
    /// Course-error gain (1/s): turn rate asked per radian of course error.
    pub const K_COURSE: f32 = 0.9;
    /// Vector-field gain round a circle: how hard the course bends towards the circle.
    pub const K_CIRCLE: f32 = 2.0;
    /// Loiter radius: at least this, and at least `LOITER_TURN_MARGIN` turn radii at the cruise
    /// speed and `LOITER_BANK` of the bank limit.
    pub const LOITER_RADIUS_MIN: f32 = 25.0;
    pub const LOITER_BANK: f32 = 0.75;
    pub const LOITER_TURN_MARGIN: f32 = 1.15;
    /// An orbit's radius is at least `ORBIT_TURN_MARGIN` turn radii at the cruise speed and
    /// `ORBIT_BANK` of the bank limit.
    pub const ORBIT_BANK: f32 = 0.85;
    pub const ORBIT_TURN_MARGIN: f32 = 1.1;
    /// Goto: L1 to the point until within `GOTO_OVER` of it (or it is behind and within
    /// `GOTO_ARRIVED` loiter radii), then circle it at the loiter radius.
    pub const GOTO_OVER: f32 = 10.0;
    pub const GOTO_ARRIVED: f32 = 1.2;
    /// Band: wander round the point at this fraction of the fence radius (or the loiter radius).
    pub const BAND_WANDER: f32 = 0.45;
    /// Circles are kept this far inside the fence, plus what the fence itself needs (its buffer,
    /// `FENCE_LAG_MIN` seconds of flight and the wind's drift in half a turn).
    pub const CIRCLE_FENCE_MARGIN: f32 = 8.0;

    /// Height: climb rate asked per metre of height error (1/s), and the most climb and sink.
    pub const K_HEIGHT: f32 = 0.35;
    pub const CLIMB_MAX: f32 = 4.0;
    pub const SINK_MAX: f32 = 4.0;
    /// Pitch per m/s of climb-rate error, its integral, and the integrator's limit (rad).
    pub const K_CLIMB: f32 = 0.05;
    pub const K_CLIMB_I: f32 = 0.03;
    pub const PITCH_I_MAX: f32 = 0.12;
    /// Height targets stay this far inside the floor and the ceiling.
    pub const HEIGHT_MARGIN: f32 = 8.0;
    /// Throttle: trim, per m/s of airspeed error, per m/s of climb asked, the integral and its limit.
    pub const THR_TRIM: f32 = 0.45;
    pub const K_THR_SPEED: f32 = 0.1;
    pub const K_THR_CLIMB: f32 = 0.06;
    pub const K_THR_I: f32 = 0.05;
    pub const THR_I_MAX: f32 = 0.4;
    /// Below the target airspeed by more than `UNDERSPEED`, lower the nose this much per m/s.
    pub const UNDERSPEED: f32 = 2.0;
    pub const K_UNDERSPEED_PITCH: f32 = 0.03;

    /// Pursue: airspeed added per metre the plane is behind the target (1/s), the most added and
    /// taken off, and the longest look-ahead to the intercept point (s; the look-ahead is the
    /// range over the plane's speed).
    pub const K_CLOSE: f32 = 0.5;
    pub const CLOSE_MAX: f32 = 5.0;
    pub const OPEN_MAX: f32 = 4.0;
    pub const LEAD_MAX: f32 = 3.0;
    /// The shortest look-ahead (s): close behind, the plane follows the target's path to a point
    /// this far ahead of it, as L1 follows a track, instead of chasing a point it is on top of.
    pub const LEAD_MIN: f32 = 1.5;
    /// The target's turn rate is estimated from its velocity, filtered with this time constant
    /// (s), and believed up to `TURN_RATE_MAX` (rad/s).
    pub const TURN_FILTER: f32 = 1.0;
    pub const TURN_RATE_MAX: f32 = 1.0;
    /// With separation on, pursue sits this much beyond the separation zone, behind the target.
    pub const STANDOFF_EXTRA: f32 = 3.0;
    /// Evade: aim this far above or below the threat, at this fraction of the way from cruise to
    /// top speed; the arena edge starts to bend the escape inwards at `EVADE_EDGE` of the fence radius.
    pub const EVADE_HEIGHT: f32 = 20.0;
    pub const EVADE_SPEED: f32 = 0.6;
    pub const EVADE_EDGE: f32 = 0.5;

    /// Fence: the turn back is predicted as `FENCE_LAG_MIN` s more as the plane is, then a roll
    /// to the full bank at `ROLL_RATE` rad/s (slower than the stock roll controller, to be safe),
    /// then the turn; it must stay `FENCE_BUFFER` m inside.
    pub const FENCE_LAG_MIN: f32 = 0.3;
    pub const ROLL_RATE: f32 = 1.0;
    /// The turn back is predicted in steps of `FENCE_STEP` s, for at most `FENCE_STEPS` of them.
    pub const FENCE_STEP: f32 = 0.1;
    pub const FENCE_STEPS: usize = 150;
    pub const FENCE_BUFFER: f32 = 5.0;
    /// The turn back ends when heading inwards at this fraction of the ground speed.
    pub const FENCE_INWARD: f32 = 0.3;
    /// Faster than cruise in the turn back, the throttle is held to this.
    pub const FENCE_THROTTLE: f32 = 0.1;
    /// Floor and ceiling: start acting this far away, plus `LIMIT_LOOKAHEAD` seconds of the vertical speed.
    pub const LIMIT_ZONE: f32 = 4.0;
    pub const LIMIT_LOOKAHEAD: f32 = 2.0;
    /// Below the floor (or above the ceiling), the bank is held to this fraction of the limit.
    pub const LIMIT_BANK: f32 = 0.5;
    /// The wind estimate is filtered with this time constant (s).
    pub const WIND_FILTER: f32 = 5.0;
    /// Separation: the zone round each track is the separation plus a buffer (a fraction of it,
    /// but at least `SEP_BUFFER_MIN` m), horizontally and vertically. Paths are predicted
    /// `SEP_HORIZON` seconds ahead in steps of `SEP_STEP` (`SEP_STEPS` of them), the plane keeping
    /// its present turn and climb for `SEP_REACT` s before an escape takes hold, and climbing at
    /// `SEP_CLIMB_FACTOR` of airspeed × sin(pitch).
    pub const SEP_HORIZON: f32 = 5.0;
    pub const SEP_STEP: f32 = 0.25;
    pub const SEP_STEPS: usize = (SEP_HORIZON / SEP_STEP) as usize;
    pub const SEP_REACT: f32 = 0.5;
    pub const SEP_CLIMB_FACTOR: f32 = 0.8;
    pub const SEP_BUFFER: f32 = 0.75;
    pub const SEP_BUFFER_MIN: f32 = 3.0;
    /// The escape ends when the primitive's own path is clear of this many zones.
    pub const SEP_RELEASE: f32 = 1.4;
    /// Escapes are scored by their least clearance once they have taken hold (up to `SEP_ENOUGH`
    /// zones, beyond which all are as good), then by how much their clearance lacks of
    /// `SEP_ENOUGH` (the sum of the squares, over the horizon); the one being flown is kept
    /// unless another is clearer by `SEP_SWITCH`.
    pub const SEP_ENOUGH: f32 = 2.0;
    pub const SEP_SWITCH: f32 = 0.15;
    /// The penalty on an escape that climbs or dives towards the height of the nearest track (by
    /// the larger of the horizontal and vertical distances).
    pub const SEP_TOWARDS: f32 = 3.0;
    /// Escapes are compared by their clearance from this many seconds on, when they have taken hold.
    pub const SEP_HOLD: f32 = 1.5;
    /// Escaping down, the nose goes to this fraction of the pitch-down limit.
    pub const SEP_DIVE: f32 = 0.7;
    /// Stall: within this of the minimum airspeed, limit the climb; under it, nose down this much,
    /// wings within `STALL_BANK`, full throttle.
    pub const STALL_MARGIN: f32 = 2.0;
    pub const STALL_PITCH: f32 = 0.087;
    pub const STALL_BANK: f32 = 0.17;
}

use tuning::*;

/// Bits of [`Executor::rules`]: which of the envelope's rules changed the demand.
pub mod rules {
    pub const SEPARATION: u8 = 1;
    pub const FENCE: u8 = 2;
    pub const CEILING: u8 = 4;
    pub const FLOOR: u8 = 8;
    pub const STALL: u8 = 16;
    /// The final clamp to the attitude and throttle limits (the primitives keep inside them, so
    /// this only fires when the rules above leave something out of range).
    pub const LIMITS: u8 = 32;
}

/// A circle to fly round: centre (north, east), height above home, direction (+1 clockwise seen
/// from above, i.e. right turns; -1 the other way).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Anchor {
    centre: [f32; 2],
    height: f32,
    dir: f32,
}

/// Roll and pitch (rad) and throttle (0 to 1, or negative for the pilot's).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Cmd {
    roll: f32,
    pitch: f32,
    throttle: f32,
}

/// The limits, checked and made consistent.
#[derive(Clone, Copy, Debug)]
struct Lims {
    fence_centre: [f32; 2],
    fence_radius: f32,
    ceiling: f32,
    floor: f32,
    bank: f32,
    pitch_up: f32,
    pitch_down: f32,
    v_min: f32,
    v_cruise: f32,
    v_max: f32,
    separation: f32,
    grace: f32,
    /// The wind's speed (filtered), set each step: the fence allows for its drift.
    wind: f32,
}

impl Lims {
    fn new(l: &Limits) -> Option<Lims> {
        let all = [
            l.fence_centre[0],
            l.fence_centre[1],
            l.fence_radius,
            l.ceiling,
            l.floor,
            l.bank_max,
            l.pitch_max,
            l.pitch_min,
            l.airspeed_min,
            l.airspeed_cruise,
            l.airspeed_max,
            l.separation,
            l.grace,
        ];
        if !all.iter().all(|v| v.is_finite()) {
            return None;
        }
        let v_min = l.airspeed_min.within(0.0, 100.0);
        let v_max = l.airspeed_max.min(150.0).max(v_min + 2.0);
        Some(Lims {
            fence_centre: l.fence_centre,
            fence_radius: l.fence_radius.max(0.0),
            ceiling: l.ceiling,
            floor: l.floor,
            bank: l.bank_max.within(0.0, 1.4),
            // The header says both are positive; the defaults give pitch_min negative. Either works.
            pitch_up: l.pitch_max.abs().min(0.8),
            pitch_down: l.pitch_min.abs().min(0.8),
            v_min,
            v_cruise: l.airspeed_cruise.within(v_min + 1.0, v_max),
            v_max,
            separation: l.separation,
            grace: l.grace.max(0.0),
            wind: 0.0,
        })
    }

    fn fenced(&self) -> bool {
        self.fence_radius > 0.0
    }

    /// Turn radius at speed `v` and `fraction` of the bank limit.
    fn turn_radius(&self, v: f32, fraction: f32) -> f32 {
        let t = tan((self.bank * fraction).within(0.02, 1.4));
        v * v / (G * t)
    }

    fn loiter_radius(&self) -> f32 {
        (LOITER_TURN_MARGIN * self.turn_radius(self.v_cruise, LOITER_BANK)).max(LOITER_RADIUS_MIN)
    }

    /// How far inside the fence a turn back, begun after `lag` seconds at speed `v`, must stay:
    /// the distance flown in the lag, the wind's drift in half a turn, and the buffer.
    fn fence_margin(&self, v: f32, lag: f32) -> f32 {
        let half_turn = PI * v / (G * tan(self.bank.within(0.02, 1.4)));
        FENCE_BUFFER + v * lag + self.wind * half_turn
    }

    /// Moves a circle's centre (and shrinks it, if it must) to keep it inside the fence.
    fn fit_circle(&self, centre: [f32; 2], radius: f32) -> ([f32; 2], f32) {
        if !self.fenced() {
            return (centre, radius);
        }
        let margin = CIRCLE_FENCE_MARGIN + self.fence_margin(self.v_cruise, FENCE_LAG_MIN);
        let radius = radius.min((self.fence_radius - margin).max(0.0));
        let room = (self.fence_radius - margin - radius).max(0.0);
        let off = sub2(centre, self.fence_centre);
        let d = norm2(off);
        if d <= room {
            return (centre, radius);
        }
        let k = room / d;
        ([self.fence_centre[0] + off[0] * k, self.fence_centre[1] + off[1] * k], radius)
    }

    /// The height target, kept clear of the floor and the ceiling (the floor wins).
    fn safe_height(&self, c: &Ctx, h: f32) -> f32 {
        let ground = c.h - c.agl;
        let low = ground + self.floor + HEIGHT_MARGIN;
        let high = self.ceiling - HEIGHT_MARGIN;
        h.min(high).max(low)
    }
}

/// The state, in the forms the guidance uses.
#[derive(Clone, Copy, Debug)]
struct Ctx {
    pos: [f32; 2],
    /// Height above home (up).
    h: f32,
    /// Climb rate (up).
    climb: f32,
    vel: [f32; 2],
    /// Ground speed.
    gs: f32,
    /// Airspeed, at least 1.
    air: f32,
    /// Course over the ground (the heading when barely moving).
    course: f32,
    yaw: f32,
    roll: f32,
    agl: f32,
    /// The speed turns are made at, for guidance.
    v: f32,
}

impl Ctx {
    fn new(s: &State) -> Ctx {
        let vel = [s.vel[0], s.vel[1]];
        let gs = norm2(vel);
        let course = if gs > 3.0 { atan2(vel[1], vel[0]) } else { wrap_pi(s.yaw) };
        let air = s.airspeed.max(1.0);
        Ctx {
            pos: [s.pos[0], s.pos[1]],
            h: -s.pos[2],
            climb: -s.vel[2],
            vel,
            gs,
            air,
            course,
            yaw: s.yaw,
            roll: s.roll,
            agl: s.agl,
            v: if gs > 3.0 { gs } else { air.max(5.0) },
        }
    }
}

/// A track, checked and dead-reckoned to now.
#[derive(Clone, Copy, Debug)]
struct Other {
    id: u8,
    pos: [f32; 2],
    h: f32,
    vel: [f32; 2],
    climb: f32,
    /// Turn rate (rad/s, clockwise), when known.
    turn: f32,
}

/// What the executor remembers of a track: its last velocity and its turn rate (filtered).
#[derive(Clone, Copy, Debug)]
struct Turn {
    id: u8,
    used: bool,
    /// Heard on this step (cleared at its end).
    seen: bool,
    vel: [f32; 2],
    rate: f32,
}

impl Turn {
    const NONE: Turn = Turn { id: 0, used: false, seen: false, vel: [0.0; 2], rate: 0.0 };
}

fn usable(t: &Track) -> Option<Other> {
    let finite = t.pos.iter().chain(t.vel.iter()).all(|v| v.is_finite()) && t.age.is_finite();
    if t.valid == 0 || !finite || t.age > TRACK_STALE || t.age < -1.0 {
        return None;
    }
    if t.pos.iter().any(|v| v.abs() > 1.0e6) || t.vel.iter().any(|v| v.abs() > 300.0) {
        return None;
    }
    let age = t.age.max(0.0);
    Some(Other {
        id: t.id,
        turn: 0.0,
        pos: [t.pos[0] + t.vel[0] * age, t.pos[1] + t.vel[1] * age],
        h: -(t.pos[2] + t.vel[2] * age),
        vel: [t.vel[0], t.vel[1]],
        climb: -t.vel[2],
    })
}

fn state_ok(s: &State) -> bool {
    let all = [
        s.pos[0], s.pos[1], s.pos[2], s.vel[0], s.vel[1], s.vel[2], s.roll, s.pitch, s.yaw, s.airspeed, s.agl, s.dt,
        s.time,
    ];
    all.iter().all(|v| v.is_finite())
        && s.pos.iter().all(|v| v.abs() < 1.0e6)
        && s.vel.iter().all(|v| v.abs() < 300.0)
        && s.roll.abs() < 10.0
        && s.pitch.abs() < 10.0
        && s.yaw.abs() < 1.0e3
        && (0.0..=150.0).contains(&s.airspeed)
        && s.agl.abs() < 1.0e5
        && s.dt > 0.0
        && s.dt <= DT_MAX
}

fn plan_ok(p: &Plan) -> bool {
    let all = [p.point[0], p.point[1], p.point[2], p.radius, p.band[0], p.band[1], p.ttl];
    Primitive::from_u8(p.primitive).is_some()
        && all.iter().all(|v| v.is_finite())
        && p.point.iter().all(|v| v.abs() < 1.0e6)
        && p.ttl >= 0.0
}

/// Roll for a lateral acceleration (m/s², positive to the right), within the bank limit.
fn roll_for(a: f32, l: &Lims) -> f32 {
    atan(a / G).within(-l.bank, l.bank)
}

/// L1 guidance to a point: the lateral acceleration (positive right).
fn l1_to(c: &Ctx, target: [f32; 2]) -> f32 {
    let d = sub2(target, c.pos);
    let dist = norm2(d);
    if dist < 0.1 {
        return 0.0;
    }
    let eta = wrap_pi(atan2(d[1], d[0]) - c.course);
    let l1 = (L1_PERIOD * L1_DAMPING * c.v / PI).min(dist).max(L1_MIN);
    let k = 2.0 * c.v * c.v / l1;
    if eta.abs() > HALF_PI { k.copysign(eta) } else { k * sin(eta) }
}

/// Course hold: the lateral acceleration towards course `chi`.
fn course_to(c: &Ctx, chi: f32) -> f32 {
    K_COURSE * c.v * wrap_pi(chi - c.course)
}

/// A vector field round a circle: the lateral acceleration to join it and stay on it.
fn circle(c: &Ctx, centre: [f32; 2], radius: f32, dir: f32) -> f32 {
    let radius = radius.max(1.0);
    let r = sub2(c.pos, centre);
    let d = norm2(r);
    // At the centre, the bearing from it is taken as if the plane were already going round.
    let phi = if d > 1.0 { atan2(r[1], r[0]) } else { c.course - dir * HALF_PI };
    let chi = phi + dir * (HALF_PI + atan(K_CIRCLE * (d - radius) / radius));
    let near = (1.0 - (d - radius).abs() / radius).within(0.0, 1.0);
    dir * c.v * c.v / radius * near + course_to(c, chi)
}

/// Which way round a circle the plane is already going (+1 clockwise), from where it is and its velocity.
fn natural_dir(c: &Ctx, centre: [f32; 2]) -> f32 {
    if cross2(sub2(c.pos, centre), c.vel) >= 0.0 { 1.0 } else { -1.0 }
}

/// The plan executor. Holds the plan being flown and the controllers' memory; `step` once per
/// control step. [`Executor::new`] is the state after `agent_exec_reset`.
#[derive(Clone, Copy, Debug)]
pub struct Executor {
    plan: Option<Plan>,
    /// Seconds since the plan was adopted (the sum of `dt`).
    since: f32,
    /// The plan's circle and height, set on the first step after adoption (`None` until then).
    anchor: Option<Anchor>,
    /// Where to loiter when the plan expired or its target is lost.
    fallback: Option<Anchor>,
    /// Goto: the direction to circle the point, once there (0: not yet).
    goto_dir: f32,
    pitch_i: f32,
    thr_i: f32,
    /// Fence turn-back in progress: its direction (+1 right), or 0.
    fence_turn: f32,
    /// The separation escape being flown: 1 + its index in `ESCAPES`, or 0 for none.
    sep_option: u8,
    /// The turn rate of each track, from how its velocity turns.
    turns: [Turn; MAX_TRACKS],
    /// The wind (north, east), filtered.
    wind: [f32; 2],
    /// What was flown on the last step.
    flown: Primitive,
    /// The envelope's rules that changed the last step's demand (`rules`).
    rules: u8,
}

impl Default for Executor {
    fn default() -> Executor {
        Executor::new()
    }
}

impl Executor {
    pub const fn new() -> Executor {
        Executor {
            plan: None,
            since: 0.0,
            anchor: None,
            fallback: None,
            goto_dir: 0.0,
            pitch_i: 0.0,
            thr_i: 0.0,
            fence_turn: 0.0,
            sep_option: 0,
            turns: [Turn::NONE; MAX_TRACKS],
            wind: [0.0, 0.0],
            flown: Primitive::None,
            rules: 0,
        }
    }

    /// Forget the plan and all memory.
    pub fn reset(&mut self) {
        *self = Executor::new();
    }

    /// The plan held (the last adopted), expired or not.
    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    /// Seconds since the held plan was adopted.
    pub fn since_adopted(&self) -> f32 {
        self.since
    }

    /// The primitive flown on the last step (Loiter when fallen back to it).
    pub fn flown(&self) -> Primitive {
        self.flown
    }

    /// The envelope's rules that changed the last demand, as [`rules`] bits (0 for none).
    pub fn rules(&self) -> u8 {
        self.rules
    }

    /// One control step. `tracks` may be any length (the C side passes `MAX_TRACKS`).
    pub fn step(&mut self, s: &State, plan: Option<&Plan>, tracks: &[Track], limits: &Limits) -> Demand {
        let mut lims = match Lims::new(limits) {
            Some(l) if state_ok(s) => l,
            _ => {
                self.rules = 0;
                return bad_input();
            }
        };
        let c = Ctx::new(s);
        let w = wind(&c);
        let k = (s.dt / WIND_FILTER).min(1.0);
        self.wind = [self.wind[0] + (w[0] - self.wind[0]) * k, self.wind[1] + (w[1] - self.wind[1]) * k];
        lims.wind = norm2(self.wind);

        match plan {
            Some(p) if plan_ok(p) && self.plan.map(|h| h.id) != Some(p.id) => {
                self.plan = Some(*p);
                self.since = 0.0;
                self.anchor = None;
                self.fallback = None;
                self.goto_dir = 0.0;
            }
            _ => self.since += s.dt,
        }
        if self.anchor.is_none() {
            self.anchor = Some(self.make_anchor(&c, &lims));
        }
        self.update_turns(tracks, s.dt);

        let (raw, fallback) = self.primitive(&c, tracks, &lims, s.dt);
        let raw = Cmd {
            roll: raw.roll.within(-lims.bank, lims.bank),
            pitch: raw.pitch.within(-lims.pitch_down, lims.pitch_up),
            throttle: raw.throttle.within(0.0, 1.0),
        };
        let (cmd, fallback) = self.envelope(&c, raw, fallback, tracks, &lims);

        let d = Demand {
            roll_cd: centidegrees(cmd.roll),
            pitch_cd: centidegrees(cmd.pitch),
            throttle: cmd.throttle,
            fallback: fallback as u8,
            intervened: differs(&cmd, &raw) as u8,
            _pad: [0; 2],
        };
        if !d.throttle.is_finite() {
            return bad_input();
        }
        d
    }

    /// The circle and height of the held plan (or of no plan), from where the plane is now.
    fn make_anchor(&self, c: &Ctx, l: &Lims) -> Anchor {
        let here = Anchor { centre: c.pos, height: c.h, dir: if c.roll < -0.1 { -1.0 } else { 1.0 } };
        match self.plan.and_then(|p| Primitive::from_u8(p.primitive).map(|q| (p, q))) {
            Some((p, Primitive::Orbit)) => {
                let (centre, _) = l.fit_circle([p.point[0], p.point[1]], self.orbit_radius(&p, l));
                Anchor { centre, height: -p.point[2], dir: natural_dir(c, centre) }
            }
            Some((p, Primitive::Band)) => {
                let (centre, _) = l.fit_circle([p.point[0], p.point[1]], band_radius(l));
                Anchor { centre, height: c.h, dir: natural_dir(c, centre) }
            }
            _ => here,
        }
    }

    fn orbit_radius(&self, p: &Plan, l: &Lims) -> f32 {
        p.radius.max(ORBIT_TURN_MARGIN * l.turn_radius(l.v_cruise, ORBIT_BANK))
    }

    /// The primitive's demand (within the attitude limits), and the fallback taken, if any.
    fn primitive(&mut self, c: &Ctx, tracks: &[Track], l: &Lims, dt: f32) -> (Cmd, Fallback) {
        let anchor = self.anchor.unwrap_or_default();
        let Some(p) = self.plan else {
            self.flown = Primitive::None;
            let (pitch, throttle) = self.vertical(c, anchor.height, l.v_cruise, l, dt);
            return (Cmd { roll: 0.0, pitch, throttle }, Fallback::NotAtAll);
        };
        let prim = Primitive::from_u8(p.primitive).unwrap_or(Primitive::Loiter);
        let target = tracks
            .iter()
            .filter(|t| t.id == p.target)
            .find_map(usable)
            .map(|o| Other { turn: self.turn_of(o.id), ..o });
        let lost = matches!(prim, Primitive::Pursue | Primitive::Evade) && target.is_none();
        if self.since > p.ttl + l.grace || lost {
            let a = *self.fallback.get_or_insert(Anchor { centre: c.pos, height: c.h, ..anchor });
            self.flown = Primitive::Loiter;
            return (self.loiter(c, a, l, dt), Fallback::PlanExpired);
        }
        self.fallback = None;
        self.flown = prim;

        let (accel, height, speed) = match (prim, target) {
            (Primitive::None, _) => (0.0, anchor.height, l.v_cruise),
            (Primitive::Loiter, _) => return (self.loiter(c, anchor, l, dt), Fallback::NotAtAll),
            (Primitive::Orbit, _) => {
                let (centre, radius) = l.fit_circle([p.point[0], p.point[1]], self.orbit_radius(&p, l));
                (circle(c, centre, radius, anchor.dir), -p.point[2], l.v_cruise)
            }
            (Primitive::Band, _) => {
                let (centre, radius) = l.fit_circle([p.point[0], p.point[1]], band_radius(l));
                let (low, high) = (p.band[0].min(p.band[1]), p.band[0].max(p.band[1]));
                let h = if high - low > 2.0 { anchor.height.within(low + 1.0, high - 1.0) } else { 0.5 * (low + high) };
                (circle(c, centre, radius, anchor.dir), h, l.v_cruise)
            }
            (Primitive::Goto, _) => {
                let r = l.loiter_radius();
                let (point, _) = l.fit_circle([p.point[0], p.point[1]], r);
                let to = sub2(point, c.pos);
                let dist = norm2(to);
                // There when over it, or when it is close and behind (passed, or inside the turn).
                let passed = dist < GOTO_ARRIVED * r && wrap_pi(atan2(to[1], to[0]) - c.course).abs() > HALF_PI;
                let a = if dist > GOTO_OVER && !passed && self.goto_dir == 0.0 {
                    l1_to(c, point)
                } else {
                    if self.goto_dir == 0.0 {
                        self.goto_dir = natural_dir(c, point);
                    }
                    circle(c, point, l.fit_circle(point, r).1, self.goto_dir)
                };
                (a, -p.point[2], l.v_cruise)
            }
            (Primitive::Pursue, Some(t)) => pursue(c, &t, l),
            (Primitive::Evade, Some(t)) => evade(c, &t, l),
            (Primitive::Pursue | Primitive::Evade, None) => (0.0, c.h, l.v_cruise), // (`lost` above)
        };
        let (pitch, throttle) = self.vertical(c, height, speed, l, dt);
        (Cmd { roll: roll_for(accel, l), pitch, throttle }, Fallback::NotAtAll)
    }

    /// Updates the turn rates of the tracks, from how their velocities turned since the last step.
    fn update_turns(&mut self, tracks: &[Track], dt: f32) {
        for t in tracks {
            let Some(o) = usable(t) else { continue };
            let k = match self.turns.iter().position(|k| k.used && k.id == t.id) {
                Some(i) => self.turns.get_mut(i),
                None => self.turns.iter_mut().find(|k| !k.used).map(|k| {
                    *k = Turn { id: t.id, used: true, seen: false, vel: o.vel, rate: 0.0 };
                    k
                }),
            };
            let Some(k) = k else { continue };
            if k.seen {
                continue; // the same id twice: the first one counts
            }
            k.seen = true;
            let v2 = dot2(o.vel, o.vel);
            if v2 > 4.0 {
                let w = (cross2(k.vel, o.vel) / (v2 * dt)).within(-TURN_RATE_MAX, TURN_RATE_MAX);
                k.rate += (w - k.rate) * (dt / TURN_FILTER).min(1.0);
            } else {
                k.rate = 0.0;
            }
            k.vel = o.vel;
        }
        for k in self.turns.iter_mut() {
            if k.seen {
                k.seen = false;
            } else {
                *k = Turn::NONE;
            }
        }
    }

    /// A track's turn rate (rad/s, clockwise), 0 if unknown.
    fn turn_of(&self, id: u8) -> f32 {
        self.turns.iter().find(|k| k.used && k.id == id).map_or(0.0, |k| k.rate)
    }

    fn loiter(&mut self, c: &Ctx, a: Anchor, l: &Lims, dt: f32) -> Cmd {
        let (centre, radius) = l.fit_circle(a.centre, l.loiter_radius());
        let accel = circle(c, centre, radius, a.dir);
        let (pitch, throttle) = self.vertical(c, a.height, l.v_cruise, l, dt);
        Cmd { roll: roll_for(accel, l), pitch, throttle }
    }

    /// Pitch and throttle for a height (above home) and an airspeed: climb rate from the height
    /// error, pitch from the climb rate, throttle from the airspeed and the climb asked.
    fn vertical(&mut self, c: &Ctx, height: f32, speed: f32, l: &Lims, dt: f32) -> (f32, f32) {
        let height = l.safe_height(c, height);
        let speed = speed.within(l.v_min + STALL_MARGIN, l.v_max);
        let climb = (K_HEIGHT * (height - c.h)).within(-SINK_MAX, CLIMB_MAX);
        let e_climb = climb - c.climb;
        let mut pitch = climb / c.air.max(5.0) + K_CLIMB * e_climb + self.pitch_i;
        let under = speed - UNDERSPEED - c.air;
        if under > 0.0 {
            pitch -= K_UNDERSPEED_PITCH * under;
        }
        if !((pitch >= l.pitch_up && e_climb > 0.0) || (pitch <= -l.pitch_down && e_climb < 0.0)) {
            self.pitch_i = (self.pitch_i + K_CLIMB_I * e_climb * dt).within(-PITCH_I_MAX, PITCH_I_MAX);
        }
        let e_speed = speed - c.air;
        let throttle = THR_TRIM + K_THR_SPEED * e_speed + K_THR_CLIMB * climb + self.thr_i;
        if !((throttle >= 1.0 && e_speed > 0.0) || (throttle <= 0.0 && e_speed < 0.0)) {
            self.thr_i = (self.thr_i + K_THR_I * e_speed * dt).within(-THR_I_MAX, THR_I_MAX);
        }
        (pitch.within(-l.pitch_down, l.pitch_up), throttle.within(0.0, 1.0))
    }

    /// The envelope. Returns the demand and the fallback.
    fn envelope(&mut self, c: &Ctx, raw: Cmd, mut fallback: Fallback, tracks: &[Track], l: &Lims) -> (Cmd, Fallback) {
        let mut cmd = raw;
        let mut bank = l.bank;
        let mut rules = 0u8;
        let mut was = cmd;
        let mut mark = |cmd: &Cmd, rule: u8| {
            if differs(&was, cmd) {
                rules |= rule;
            }
            was = *cmd;
        };

        let fence = self.fence_turn(c, l);

        // Separation: when the path the primitive asks for comes inside the zone round any track
        // within the horizon, fly whichever of nine escapes (full bank either way or wings level;
        // climb, sink or level) stays clearest of them all. Once begun, the manoeuvre goes on
        // until the primitive's path is clear of a wider zone.
        if l.separation > 0.0 && tracks.iter().any(|t| usable(t).is_some()) {
            let zone = l.separation + (SEP_BUFFER * l.separation).max(SEP_BUFFER_MIN);
            let wanted = self.least_clearance(c, tracks, l, zone, raw.roll);
            let engaged = self.sep_option != 0;
            if (!engaged && wanted < 1.0) || (engaged && wanted < SEP_RELEASE) {
                // While the fence is turning the plane back, only the escapes turning that way.
                // The best is the one whose path, once it has taken hold, stays clearest; between
                // those as clear (to `SEP_SWITCH`), the one with the least penalty; the one being
                // flown is kept unless another is clearer by `SEP_SWITCH`. While the fence is
                // turning the plane back, only the escapes that turn the same way, or that can be
                // flown for `SEP_HOLD` seconds and still turn back inside it.
                let mut best = (ESCAPES.len() - 1, f32::MIN, f32::MAX);
                for (i, &(turn, climb)) in ESCAPES.iter().enumerate() {
                    if let Some(f) = fence
                        && f != turn
                        && self.turn_back_after(c, l, f, turn * l.bank, SEP_HOLD) + FENCE_BUFFER >= l.fence_radius
                    {
                        continue;
                    }
                    let (later, penalty) = self.clearance(c, tracks, l, zone, turn * l.bank, climb);
                    let later =
                        later.min(SEP_ENOUGH) + if i + 1 == self.sep_option as usize { SEP_SWITCH } else { 0.0 };
                    if later > best.1 + SEP_SWITCH || (later > best.1 - SEP_SWITCH && penalty < best.2) {
                        best = (i, later, penalty);
                    }
                }
                self.sep_option = best.0 as u8 + 1;
                let (turn, climb) = escape(self.sep_option);
                cmd.roll = turn * l.bank;
                // Full throttle to climb; diving or level, hold the speed (a fast plane turns wide).
                cmd.pitch = if climb > 0.0 {
                    l.pitch_up
                } else if climb < 0.0 {
                    -SEP_DIVE * l.pitch_down
                } else {
                    0.0
                };
                cmd.throttle = if climb > 0.0 {
                    1.0
                } else if c.air > l.v_cruise {
                    0.0
                } else {
                    cmd.throttle.max(THR_TRIM)
                };
            } else {
                self.sep_option = 0;
            }
        } else {
            self.sep_option = 0;
        }
        mark(&cmd, rules::SEPARATION);

        // Fence (decided above): the turn back. It comes before separation: while it turns the
        // plane, a separation escape must turn the same way, unless the turn back can wait.
        let escaping_inside = self.sep_option != 0 && fence.is_some_and(|f| f != escape(self.sep_option).0);
        if let Some(turn) = fence.filter(|_| !escaping_inside) {
            cmd.roll = turn * l.bank;
            if c.air > l.v_cruise {
                cmd.throttle = cmd.throttle.min(FENCE_THROTTLE); // slower turns tighter
            }
        }
        mark(&cmd, rules::FENCE);

        // Ceiling (above home): pull the nose down near it.
        let zone = LIMIT_ZONE + LIMIT_LOOKAHEAD * c.climb.max(0.0);
        let frac = (l.ceiling - c.h) / zone;
        if frac < 1.0 {
            cmd.pitch = cmd.pitch.min((l.pitch_up * frac).max(-0.5 * l.pitch_down));
            if frac < 0.0 {
                bank = bank.min(LIMIT_BANK * l.bank);
                cmd.roll = cmd.roll.within(-bank, bank);
            }
        }
        mark(&cmd, rules::CEILING);

        // Floor (above the ground): nose up, more throttle; under it, wings nearer level.
        let zone = LIMIT_ZONE + LIMIT_LOOKAHEAD * (-c.climb).max(0.0);
        let frac = (c.agl - l.floor) / zone;
        if frac < 1.0 {
            let up = (1.0 - frac).within(0.0, 1.0);
            cmd.pitch = cmd.pitch.max(0.8 * l.pitch_up * up);
            cmd.throttle = cmd.throttle.max(up);
            if frac < 0.0 {
                bank = bank.min(LIMIT_BANK * l.bank);
                cmd.roll = cmd.roll.within(-bank, bank);
                fallback = Fallback::Recovery;
            }
        }
        mark(&cmd, rules::FLOOR);

        // Stall margin: near it no climbing; under it, nose down, wings level, full throttle.
        if c.air < l.v_min + STALL_MARGIN {
            let k = ((c.air - l.v_min) / STALL_MARGIN).within(0.0, 1.0);
            cmd.pitch = cmd.pitch.min(l.pitch_up * k);
            cmd.throttle = cmd.throttle.max(0.8);
        }
        if c.air < l.v_min {
            let nose = if c.agl < l.floor { 0.0 } else { -STALL_PITCH };
            cmd.pitch = cmd.pitch.min(nose);
            cmd.throttle = 1.0;
            bank = bank.min(STALL_BANK);
            cmd.roll = cmd.roll.within(-bank, bank);
            fallback = Fallback::Recovery;
        }
        mark(&cmd, rules::STALL);

        cmd.roll = finite_or(cmd.roll, 0.0).within(-bank, bank);
        cmd.pitch = finite_or(cmd.pitch, 0.0).within(-l.pitch_down, l.pitch_up);
        cmd.throttle = finite_or(cmd.throttle, 1.0).within(0.0, 1.0);
        mark(&cmd, rules::LIMITS);
        self.rules = rules;
        (cmd, fallback)
    }

    /// The least clearance (see `clearance`) of the path the plane is on, turning as `roll` turns
    /// it after `SEP_REACT` seconds, keeping its height: from now on, 1 or more is clear.
    fn least_clearance(&self, c: &Ctx, tracks: &[Track], l: &Lims, zone: f32, roll: f32) -> f32 {
        self.clearance_from(c, tracks, l, zone, roll, 0.0, 0.0).0
    }

    /// How clear of every track the plane stays over `SEP_HORIZON`, if after `SEP_REACT` seconds
    /// at its present turn it banks to `roll` and pitches to climb (`climb` +1), sink (-1) or hold
    /// its height (0), each track flying on along the arc it is turning on. With the clearance of
    /// a track at a time the larger of the horizontal and the vertical distance over `zone`: the
    /// least clearance once the escape has taken hold (`SEP_HOLD` s), and a penalty, the sum of the
    /// squares of what each clearance lacks of `SEP_ENOUGH`, over the tracks and the time. The
    /// plane's height is kept between the floor and the ceiling.
    fn clearance(&self, c: &Ctx, tracks: &[Track], l: &Lims, zone: f32, roll: f32, climb: f32) -> (f32, f32) {
        self.clearance_from(c, tracks, l, zone, roll, climb, SEP_HOLD)
    }

    #[allow(clippy::too_many_arguments)]
    fn clearance_from(
        &self,
        c: &Ctx,
        tracks: &[Track],
        l: &Lims,
        zone: f32,
        roll: f32,
        climb: f32,
        from: f32,
    ) -> (f32, f32) {
        let v = c.v.max(5.0);
        let turn_now = G * tan(c.roll.within(-1.4, 1.4)) / v;
        let turn_then = G * tan(roll.within(-1.4, 1.4)) / v;
        let (rot_now, rot_then) = (rotate([1.0, 0.0], turn_now * SEP_STEP), rotate([1.0, 0.0], turn_then * SEP_STEP));
        let rate = if climb > 0.0 {
            c.air * sin(l.pitch_up) * SEP_CLIMB_FACTOR
        } else if climb < 0.0 {
            -c.air * sin(SEP_DIVE * l.pitch_down) * SEP_CLIMB_FACTOR
        } else {
            0.0
        };
        let ground = c.h - c.agl;
        let (low, high) = ((ground + l.floor + LIMIT_ZONE).min(c.h), (l.ceiling - LIMIT_ZONE).max(c.h));
        let (mut least, mut penalty) = (f32::MAX, 0.0);
        // Towards the height of the track nearest now (the larger of the horizontal and the
        // vertical distance) counts against an escape: so that of two planes in conflict, each
        // running this, the upper one climbs and the lower one dives.
        let nearest = tracks
            .iter()
            .filter_map(usable)
            .map(|t| (norm2(sub2(t.pos, c.pos)).max((t.h - c.h).abs()), t.h - c.h))
            .fold((f32::MAX, 0.0), |a, b| if b.0 < a.0 { b } else { a });
        if climb * nearest.1 > 0.0 {
            penalty += SEP_TOWARDS;
        }
        for t in tracks.iter().filter_map(usable) {
            let rot_t = rotate([1.0, 0.0], self.turn_of(t.id) * SEP_STEP);
            let (mut p, mut v, mut q, mut u) = (c.pos, c.vel, t.pos, t.vel);
            let mut h = c.h;
            for k in 0..=SEP_STEPS {
                let time = k as f32 * SEP_STEP;
                let dh = norm2(sub2(q, p));
                // The track may hold its height or keep climbing (it may be escaping too, or not):
                // whichever is the nearer.
                let (dz0, dz1) = (t.h - h, t.h + t.climb * time - h);
                let dz = if dz0 * dz1 <= 0.0 { 0.0 } else { dz0.abs().min(dz1.abs()) };
                let clear = dh.max(dz.abs()) / zone;
                if time >= from {
                    least = least.min(clear);
                }
                let lack = (SEP_ENOUGH - clear).max(0.0);
                penalty += lack * lack;
                let rot = if time < SEP_REACT { rot_now } else { rot_then };
                p = [p[0] + v[0] * SEP_STEP, p[1] + v[1] * SEP_STEP];
                q = [q[0] + u[0] * SEP_STEP, q[1] + u[1] * SEP_STEP];
                v = [v[0] * rot[0] - v[1] * rot[1], v[0] * rot[1] + v[1] * rot[0]];
                u = [u[0] * rot_t[0] - u[1] * rot_t[1], u[0] * rot_t[1] + u[1] * rot_t[0]];
                let vz = if time < SEP_REACT { c.climb } else { rate };
                h = (h + vz * SEP_STEP).within(low, high);
            }
        }
        (least, penalty)
    }

    /// The fence: the direction of the turn back (+1 right), if one is needed now: if the turn
    /// back, begun after `FENCE_LAG_MIN` seconds, would only just stay `FENCE_BUFFER` inside.
    /// The turn back is the way that stays further inside; it goes on until the plane heads
    /// inwards.
    fn fence_turn(&mut self, c: &Ctx, l: &Lims) -> Option<f32> {
        if !l.fenced() {
            self.fence_turn = 0.0;
            return None;
        }
        let rel = sub2(c.pos, l.fence_centre);
        let d = norm2(rel);
        let radial = if d > 0.1 { dot2(rel, c.vel) / d } else { 0.0 };
        if radial < -FENCE_INWARD * c.gs {
            self.fence_turn = 0.0;
            return None;
        }
        let (turn, far) = if self.fence_turn != 0.0 {
            (self.fence_turn, self.turn_back(c, l, self.fence_turn))
        } else {
            let (right, left) = (self.turn_back(c, l, 1.0), self.turn_back(c, l, -1.0));
            if right <= left { (1.0, right) } else { (-1.0, left) }
        };
        if far + FENCE_BUFFER >= l.fence_radius || self.fence_turn != 0.0 {
            self.fence_turn = turn;
            Some(turn)
        } else {
            None
        }
    }

    /// How far from the fence's centre the plane gets turning back (`turn` +1 right): it flies on
    /// as it is for `FENCE_LAG_MIN` s, rolls to the full bank at `ROLL_RATE`, and turns until it
    /// heads inwards over the ground, carried by the wind (as estimated).
    fn turn_back(&self, c: &Ctx, l: &Lims, turn: f32) -> f32 {
        self.turn_back_after(c, l, turn, c.roll, FENCE_LAG_MIN)
    }

    /// As `turn_back`, but first rolling towards `first` (rad) for `wait` seconds.
    fn turn_back_after(&self, c: &Ctx, l: &Lims, turn: f32, first: f32, wait: f32) -> f32 {
        // At its airspeed, or the cruise speed it will soon be back at, whichever is the faster.
        let v = c.air.max(l.v_cruise).max(5.0);
        let (mut p, mut psi, mut roll) = (c.pos, c.yaw, c.roll.within(-1.4, 1.4));
        let mut far = norm2(sub2(p, l.fence_centre));
        for k in 0..FENCE_STEPS {
            let t = k as f32 * FENCE_STEP;
            let target = if t < wait { first } else { turn * l.bank };
            roll += (target - roll).within(-ROLL_RATE * FENCE_STEP, ROLL_RATE * FENCE_STEP);
            psi += G * tan(roll) / v * FENCE_STEP;
            let (s, co) = sin_cos(psi);
            let vel = [v * co + self.wind[0], v * s + self.wind[1]];
            p = [p[0] + vel[0] * FENCE_STEP, p[1] + vel[1] * FENCE_STEP];
            let rel = sub2(p, l.fence_centre);
            far = far.max(norm2(rel));
            if t >= wait && dot2(rel, vel) < 0.0 {
                break;
            }
        }
        far
    }
}

/// Escape `option` (1 + its index in `ESCAPES`); level, wings level, for anything else.
fn escape(option: u8) -> (f32, f32) {
    ESCAPES.get((option as usize).wrapping_sub(1)).copied().unwrap_or((0.0, 0.0))
}

/// The separation escapes: bank (a fraction of the limit, + right) and climb (+1), sink (-1) or level (0).
const ESCAPES: [(f32, f32); 9] =
    [(0.0, 1.0), (0.0, -1.0), (1.0, 0.0), (-1.0, 0.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (0.0, 0.0)];

/// Band: the radius wandered round its point.
fn band_radius(l: &Lims) -> f32 {
    let r = l.loiter_radius();
    if l.fenced() { r.max(BAND_WANDER * l.fence_radius) } else { r }
}

/// The wind, roughly: ground velocity less the air velocity along the heading.
fn wind(c: &Ctx) -> [f32; 2] {
    let (s, co) = sin_cos(c.yaw);
    let w = [c.vel[0] - c.air * co, c.vel[1] - c.air * s];
    let n = norm2(w);
    if n > 20.0 { [w[0] * 20.0 / n, w[1] * 20.0 / n] } else { w }
}

/// Pursue: lead pursuit to where the target will be when the plane gets there (along the arc it
/// is turning on), from behind it (beyond the separation zone, when
/// separation is on), at its airspeed plus a closing speed; at its height.
fn pursue(c: &Ctx, t: &Other, l: &Lims) -> (f32, f32, f32) {
    let turn = t.turn;
    let rel = sub2(t.pos, c.pos);
    let range = norm2(rel).max(0.1);
    let lead = (range / c.v.max(5.0)).within(LEAD_MIN, LEAD_MAX);
    let standoff = if l.separation > 0.0 {
        l.separation + (SEP_BUFFER * l.separation).max(SEP_BUFFER_MIN) + STANDOFF_EXTRA
    } else {
        0.0
    };
    let vt = norm2(t.vel);
    let dir = if vt > 1.0 { [t.vel[0] / vt, t.vel[1] / vt] } else { [rel[0] / range, rel[1] / range] };
    // The chord of the arc: the velocity turned half way, for the lead time.
    let chord = rotate(t.vel, 0.5 * turn * lead);
    let back = rotate(dir, turn * lead);
    let aim = [t.pos[0] + chord[0] * lead - back[0] * standoff, t.pos[1] + chord[1] * lead - back[1] * standoff];
    let accel = l1_to(c, aim);

    // How far behind the target the plane is, along the target's track, past the standoff.
    let behind = dot2(rel, dir) - standoff;
    let t_air = norm2(sub2(t.vel, wind(c)));
    let speed = t_air + (K_CLOSE * behind).within(-OPEN_MAX, CLOSE_MAX);
    (accel, t.h, speed)
}

/// The north-east vector turned clockwise (seen from above) by `a` rad.
fn rotate(v: [f32; 2], a: f32) -> [f32; 2] {
    let (s, co) = sin_cos(a);
    [v[0] * co - v[1] * s, v[0] * s + v[1] * co]
}

/// Evade: away from the threat, bent back into the arena near its edge; well above or below it;
/// fast, but less so near the edge, where a wide turn would take it out.
fn evade(c: &Ctx, t: &Other, l: &Lims) -> (f32, f32, f32) {
    let away = sub2(c.pos, t.pos);
    let n = norm2(away);
    let mut dir = if n > 0.5 { [away[0] / n, away[1] / n] } else { [cos(c.course), sin(c.course)] };
    // 0 well inside the arena, rising to 1 at its edge.
    let mut edge = 0.0;
    if l.fenced() {
        let to_c = sub2(l.fence_centre, c.pos);
        let d = norm2(to_c);
        edge = ((d / l.fence_radius - EVADE_EDGE) / (1.0 - EVADE_EDGE)).within(0.0, 1.0);
        if d > 0.5 {
            dir = [dir[0] + 2.0 * to_c[0] / d * edge, dir[1] + 2.0 * to_c[1] / d * edge];
        }
    }
    let accel = course_to(c, atan2(dir[1], dir[0]));
    let height = if t.h >= c.h { t.h - EVADE_HEIGHT } else { t.h + EVADE_HEIGHT };
    // If the floor or the ceiling stops that, go the other way.
    let safe = l.safe_height(c, height);
    let height = if (safe - t.h).abs() < 0.5 * EVADE_HEIGHT {
        if height < t.h { t.h + EVADE_HEIGHT } else { t.h - EVADE_HEIGHT }
    } else {
        height
    };
    (accel, height, lerp(l.v_cruise, l.v_max, EVADE_SPEED * (1.0 - 0.5 * edge)))
}

/// Whether two demands differ by as much as the C side can see.
fn differs(a: &Cmd, b: &Cmd) -> bool {
    centidegrees(a.roll) != centidegrees(b.roll)
        || centidegrees(a.pitch) != centidegrees(b.pitch)
        || (a.throttle - b.throttle).abs() > 1.0e-3
}

fn centidegrees(rad: f32) -> i32 {
    let cd = finite_or(rad, 0.0) * (18000.0 / PI);
    (cd + 0.5f32.copysign(cd)) as i32
}

/// Wings level, nose level, throttle to the pilot: for inputs that can't be trusted.
fn bad_input() -> Demand {
    Demand { roll_cd: 0, pitch_cd: 0, throttle: -1.0, fallback: Fallback::BadInput as u8, intervened: 1, _pad: [0; 2] }
}

/// Wings level, nose level, throttle left to the pilot.
pub fn level() -> Demand {
    Demand { roll_cd: 0, pitch_cd: 0, throttle: -1.0, ..Demand::default() }
}

/// The executor the C ABI drives. The caller (ArduPlane's main loop) is single-threaded.
struct Global(core::cell::UnsafeCell<Executor>);

// SAFETY: only touched through the C functions below, which ArduPilot calls from one thread.
unsafe impl Sync for Global {}

static EXECUTOR: Global = Global(core::cell::UnsafeCell::new(Executor::new()));

#[unsafe(no_mangle)]
pub extern "C" fn agent_exec_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn agent_exec_reset() {
    // SAFETY: single-threaded caller, no other reference is live.
    unsafe { (*EXECUTOR.0.get()).reset() };
}

/// # Safety
/// Each pointer must be null or valid: `tracks` for `MAX_TRACKS` entries, `plan` may be null.
/// Not reentrant: call from one thread only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn agent_exec_step(
    state: *const State,
    plan: *const Plan,
    tracks: *const Track,
    limits: *const Limits,
    out: *mut Demand,
) -> i32 {
    if state.is_null() || limits.is_null() || out.is_null() {
        return -1;
    }
    // SAFETY: the caller promises valid pointers; the global is touched from one thread only.
    unsafe {
        let tracks: &[Track] = if tracks.is_null() { &[] } else { core::slice::from_raw_parts(tracks, MAX_TRACKS) };
        let d = (*EXECUTOR.0.get()).step(&*state, plan.as_ref(), tracks, &*limits);
        *out = d;
    }
    0
}

#[cfg(feature = "ffi-static")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[cfg(test)]
mod tests;
