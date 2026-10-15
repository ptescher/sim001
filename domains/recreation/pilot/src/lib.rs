//! The computer pilot: it flies a T1-S through the same controls a pilot's hands move (aileron,
//! elevator, rudder, throttle), on the flight model in `simulation_physics`. Nothing here moves
//! the plane directly; the physics does, from the controls.
//!
//! With a radio and a flight controller (`Pilot::with_radio`), the hands move the sticks
//! instead, and the autopilot flies the plane in FBWA: the roll stick asks for a bank and the
//! pitch stick for a pitch. The [`Controls`] are then the sticks.
//!
//! It works in layers, as a person does. The hands hold a bank angle and a climb rate (`hold`).
//! Above that, the pilot steers for a point (`toward`) or along a line (`follow`). Above that,
//! the games decide where to go. How well it flies is its [`Skill`].

use glam::Vec3;
use simulation_aircraft::{ActuatorCommands, t1s_ranger};
use simulation_physics::{self as physics, FlightState, GRAVITY, Terrain};

const DEG: f32 = std::f32::consts::PI / 180.0;

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// The elevator that holds a T1-S level at any speed is about `ELEVATOR_LEVEL +
/// ELEVATOR_FOR_LIFT / v²`, and a turn at `n` g needs `n` times the lift: measured on the flight
/// model (the `trim_by_bank` probe), from 19 to 26 m/s and up to 68° of bank.
const ELEVATOR_LEVEL: f32 = -0.33;
const ELEVATOR_FOR_LIFT: f32 = 62.0;
/// The wing's lift coefficient where a pilot feels the buffet: its tips stall at about 1.0.
const BUFFET_CL: f32 = 0.78;

/// How well a computer pilot flies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skill {
    pub name: &'static str,
    /// The steepest bank it will fly (rad).
    pub max_bank: f32,
    /// How close it turns round a pylon (m).
    pub turn_radius: f32,
    /// The throttle it races with (0..1).
    pub throttle: f32,
    /// How far away it sees a chaser coming (m).
    pub awareness: f32,
}

pub const ACE: Skill = Skill { name: "ace", max_bank: 72.0 * DEG, turn_radius: 30.0, throttle: 1.0, awareness: 80.0 };
pub const CLUB: Skill = Skill { name: "club", max_bank: 66.0 * DEG, turn_radius: 34.0, throttle: 0.92, awareness: 60.0 };
pub const ROOKIE: Skill = Skill { name: "rookie", max_bank: 58.0 * DEG, turn_radius: 40.0, throttle: 0.85, awareness: 40.0 };

/// What the pilot's hands do: -1..1 for the surfaces (aileron right, elevator nose up, rudder
/// nose right positive), 0..1 for the throttle. With a radio, they are the sticks that move
/// those channels (roll, pitch, yaw and throttle).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    pub aileron: f32,
    pub elevator: f32,
    pub rudder: f32,
    pub throttle: f32,
}

impl Controls {
    /// The T1-S's outputs for these controls, mixed as the sim mixes the sticks in MANUAL: half
    /// the rudder on the V-tail, and a little differential thrust.
    pub fn mix(&self) -> ActuatorCommands {
        let thr = self.throttle.clamp(0.0, 1.0);
        let rud = self.rudder.clamp(-1.0, 1.0);
        let diff = if thr > 0.05 { 0.1 * rud } else { 0.0 };
        t1s_ranger::mix(
            self.aileron.clamp(-1.0, 1.0),
            self.elevator.clamp(-1.0, 1.0),
            0.5 * rud,
            [(thr + diff).clamp(0.0, 1.0), (thr - diff).clamp(0.0, 1.0)],
        )
    }
}

/// A line to fly along: points on the ground, every metre or so.
#[derive(Clone, Debug)]
pub struct Path {
    points: Vec<Vec3>,
    /// Distance along the line to each point.
    along: Vec<f32>,
    /// How sharply the line turns at each point (1/m, right positive).
    curvature: Vec<f32>,
    closed: bool,
}

impl Path {
    /// A line that joins up with itself (a lap).
    pub fn closed(points: Vec<Vec3>) -> Path {
        assert!(points.len() >= 3, "a closed line needs three points");
        let mut along = Vec::with_capacity(points.len());
        let mut d = 0.0;
        for i in 0..points.len() {
            along.push(d);
            d += flat(points[(i + 1) % points.len()] - points[i]).length();
        }
        along.push(d);
        let n = points.len();
        let heading = |i: usize| {
            let t = flat(points[(i + 1) % n] - points[i]);
            t.x.atan2(-t.z)
        };
        let raw: Vec<f32> = (0..n)
            .map(|i| {
                let mut turn = heading(i) - heading((i + n - 1) % n);
                turn = (turn + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                let ds = 0.5 * (flat(points[(i + 1) % n] - points[i]).length() + flat(points[i] - points[(i + n - 1) % n]).length());
                if ds > 1e-4 { turn / ds } else { 0.0 }
            })
            .collect();
        // Smoothed over a few metres, so a polygon's corners read as the arcs they stand for.
        let spacing = d / n as f32;
        let w = ((3.0 / spacing.max(0.05)) as usize).max(1);
        let curvature = (0..n).map(|i| (0..=2 * w).map(|k| raw[(i + n + k - w) % n]).sum::<f32>() / (2 * w + 1) as f32).collect();
        Path { points, along, curvature, closed: true }
    }

    /// The point at `i`, the way the line goes there (unit), and how sharply it turns `ahead`
    /// metres on.
    fn frame(&self, i: usize, ahead: f32) -> (Vec3, Vec3, f32) {
        let n = self.points.len();
        let tangent = flat(self.points[(i + 1) % n] - self.points[i]).normalize_or_zero();
        let spacing = self.length() / n as f32;
        let j = (i + (ahead / spacing.max(0.05)) as usize) % n;
        (self.points[i], tangent, self.curvature[j])
    }

    pub fn length(&self) -> f32 {
        *self.along.last().unwrap()
    }

    /// The point nearest `p`, searching near `hint` if there is one.
    pub fn nearest(&self, p: Vec3, hint: Option<usize>) -> usize {
        let n = self.points.len();
        let d = |i: usize| flat(self.points[i] - p).length_squared();
        let candidates: Box<dyn Iterator<Item = usize>> = match hint {
            // A plane moves a few metres a step: look a little back and a fair way on.
            Some(h) if self.closed => Box::new((0..120).map(move |k| (h + n + k - 20) % n)),
            _ => Box::new(0..n),
        };
        candidates.min_by(|&a, &b| d(a).total_cmp(&d(b))).unwrap()
    }

    /// The point `dist` metres along the line from point `i`.
    pub fn ahead(&self, i: usize, dist: f32) -> Vec3 {
        let n = self.points.len();
        let length = self.length();
        let target = (self.along[i] + dist).rem_euclid(length);
        // The segment that holds `target`: the last point at or before it.
        let k = self.along[..n].partition_point(|&a| a <= target).saturating_sub(1);
        let (a, b) = (self.points[k], self.points[(k + 1) % n]);
        let seg = self.along[k + 1] - self.along[k];
        a.lerp(b, if seg > 0.0 { (target - self.along[k]) / seg } else { 0.0 })
    }
}

/// How far a plane moving at `vel` (level) and turning at `turn` rad/s about +Y goes in
/// `time` seconds.
fn along_curve(vel: Vec3, turn: f32, time: f32) -> Vec3 {
    let a = turn * time;
    if a.abs() < 1e-3 {
        return vel * time;
    }
    let (s, c) = (a.sin() / turn, (1.0 - a.cos()) / turn);
    Vec3::new(s * vel.x + c * vel.z, 0.0, -c * vel.x + s * vel.z)
}

/// ArduPlane's FBWA, as the plane's parameters set it up: the roll stick all the way over asks
/// for `roll_limit` of bank, and the pitch stick asks for a pitch up to `pitch_max` (back) or
/// down to `pitch_min` (forward).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fbwa {
    /// `ROLL_LIMIT_DEG` (rad).
    pub roll_limit: f32,
    /// `PTCH_LIM_MAX_DEG` (rad).
    pub pitch_max: f32,
    /// `PTCH_LIM_MIN_DEG` (rad, negative).
    pub pitch_min: f32,
}

/// A computer pilot. It keeps a little memory between steps (what the hands have learnt about
/// trim, and where on its line it is).
#[derive(Clone, Debug)]
pub struct Pilot {
    pub skill: Skill,
    trim: f32,
    cursor: Option<usize>,
    /// The other plane's velocity last step, when closing on it: how fast it is turning.
    other_vel: Option<Vec3>,
    other_turn: f32,
    /// What the throttle hand has learnt about the speed to sit behind another plane.
    pace: f32,
    /// With a radio, the autopilot's FBWA that the sticks talk to.
    radio: Option<Fbwa>,
}

impl Pilot {
    pub fn new(skill: Skill) -> Pilot {
        Pilot { skill, trim: 0.0, cursor: None, other_vel: None, other_turn: 0.0, pace: 0.0, radio: None }
    }

    /// A pilot with a radio, flying through a flight controller in FBWA.
    pub fn with_radio(skill: Skill, fbwa: Fbwa) -> Pilot {
        Pilot { radio: Some(fbwa), ..Pilot::new(skill) }
    }

    /// The autopilot this pilot's sticks talk to, if it flies with a radio.
    pub fn radio(&self) -> Option<Fbwa> {
        self.radio
    }

    /// Forget the line it was on (a new game, a new lap line).
    pub fn lose_place(&mut self) {
        self.cursor = None;
    }

    /// Fly at a bank angle (rad, right wing down positive) and a climb rate (m/s), at a
    /// throttle. Close to the ground it levels the wings and climbs, whatever it was asked.
    pub fn hold(&mut self, s: &FlightState, bank: f32, climb: f32, throttle: f32, ground: &dyn Terrain, dt: f32) -> Controls {
        let (roll, pitch, _) = physics::euler(s.rot);
        let (p, q, _) = physics::body_rates(s.omega);
        let v = s.airspeed.max(8.0);
        let agl = s.pos.y - ground.height(s.pos.x, s.pos.z);
        // Slow, or near the ground: gentler banks, and up.
        let mut limit = self.skill.max_bank;
        let mut climb = climb;
        // Slow, the bank comes down from its most at 22 m/s to 30° at 14 m/s.
        limit = limit.min(30.0 * DEG + (limit - 30.0 * DEG).max(0.0) * ((v - 14.0) / 8.0).clamp(0.0, 1.0));
        // Near the most the wing will lift, a tip stalls and rolls the plane over: ease the
        // bank and the pull, as a pilot does at the first buffet.
        let buffet = ((s.wing_cl - BUFFET_CL) / 0.2).clamp(0.0, 1.0);
        limit *= 1.0 - 0.4 * buffet;
        if agl < 6.0 {
            limit = limit.min(30.0 * DEG);
            climb = climb.max(2.5);
        }
        if agl < 3.0 {
            limit = limit.min(10.0 * DEG);
            climb = climb.max(4.0);
        }
        let bank = bank.clamp(-limit, limit);
        if let Some(fbwa) = self.radio {
            return self.sticks(s, fbwa, bank, climb, throttle, dt);
        }
        // Surfaces bite harder the faster the plane goes.
        let gain = (13.0 / v).powi(2).clamp(0.25, 2.5);
        let aileron = (1.8 * (bank - roll) - 0.2 * p) * gain;
        // In a turn the wing must lift more than the weight, and the nose turns about the pitch
        // axis: feed both forward, and let the trim learn the rest from the climb rate.
        let n = 1.0 / roll.cos().abs().max(0.3);
        let turn_rate = GRAVITY * roll.sin() * roll.tan().clamp(-3.5, 3.5) / v;
        let wanted_pitch = (climb / v).clamp(-0.45, 0.45).asin();
        self.trim = (self.trim + 0.12 * (climb - s.vel.y) * dt).clamp(-0.3, 0.3);
        // What the pilot knows of the T1-S's elevator: what the wing must lift, against the
        // airspeed (see `ELEVATOR_FOR_LIFT`), and the trim learns what is left over.
        let feel = ELEVATOR_LEVEL + ELEVATOR_FOR_LIFT * n / (v * v);
        let elevator = feel + (1.6 * (wanted_pitch - pitch) - 0.15 * (q - turn_rate)) * gain + self.trim - 1.5 * buffet * gain;
        // Slow in a turn: open the throttle, or the turn bleeds the speed away.
        let throttle = if v < 20.0 { throttle.max(((20.0 - v) / 4.0).min(1.0)) } else { throttle };
        Controls { aileron, elevator, rudder: 0.3 * aileron, throttle }
    }

    /// The sticks for a bank and a climb rate, in FBWA. The autopilot holds the bank and the
    /// pitch that the sticks ask for; the pitch for a climb rate is the angle of the climb,
    /// and a slow trim learns how much more the wing needs (its angle of attack, more in a turn).
    fn sticks(&mut self, s: &FlightState, fbwa: Fbwa, bank: f32, climb: f32, throttle: f32, dt: f32) -> Controls {
        let v = s.airspeed.max(8.0);
        self.trim = (self.trim + 0.05 * (climb - s.vel.y) * dt).clamp(-0.15, 0.25);
        let pitch = (climb / v).clamp(-0.45, 0.45).asin() + self.trim;
        let stick = |angle: f32, limit: f32| (angle / limit).clamp(-1.0, 1.0);
        let elevator = if pitch >= 0.0 { stick(pitch, fbwa.pitch_max) } else { -stick(pitch, fbwa.pitch_min) };
        let throttle = if v < 18.0 { throttle.max(((18.0 - v) / 4.0).min(1.0)) } else { throttle };
        Controls { aileron: stick(bank, fbwa.roll_limit), elevator, rudder: 0.0, throttle }
    }

    /// Steer for a point in the air, at a throttle.
    pub fn toward(&mut self, s: &FlightState, aim: Vec3, throttle: f32, ground: &dyn Terrain, dt: f32) -> Controls {
        let bank = (self.l1_accel(s, aim) / GRAVITY).atan();
        let climb = self.climb_for(s, aim.y);
        self.hold(s, bank, climb, throttle, ground, dt)
    }

    /// Fly at another plane (at `pos`, moving at `vel`) to touch it. Far off, it heads for where
    /// the other plane will be, at full throttle. Close in, it closes gently, with the throttle,
    /// so that it doesn't overshoot. It matches the other plane's height hard, even in a steep
    /// turn.
    pub fn intercept(&mut self, s: &FlightState, pos: Vec3, vel: Vec3, ground: &dyn Terrain, dt: f32) -> Controls {
        let to = pos - s.pos;
        let d = to.length();
        let closing = (s.vel - vel).dot(to.normalize_or_zero());
        // How fast the other plane is turning (rad/s about +Y), smoothed.
        let level = flat(vel);
        if let Some(prev) = self.other_vel.replace(level) {
            let turn = (level - prev).dot(Vec3::new(prev.z, 0.0, -prev.x)) / prev.length_squared().max(1.0) / dt;
            self.other_turn += (turn - self.other_turn) * (dt / 0.3).min(1.0);
        }
        // Where it will be, along the curve it is flying: far enough ahead (20 m or more) that
        // the steering doesn't snatch from side to side close behind it.
        let mut lead = (d / closing.max(3.0)).clamp(0.3, 1.2);
        let reach = flat(pos + level * lead - s.pos).length();
        if reach < 20.0 {
            lead += (20.0 - reach) / level.length().max(5.0);
        }
        let aim = pos + along_curve(level, self.other_turn, lead);
        let bank = (self.l1_accel(s, aim) / GRAVITY).atan();
        // Match its height, harder the closer it is.
        let firm = 0.5 + 0.8 * (1.0 - d / 30.0).clamp(0.0, 1.0);
        let climb = (firm * (pos.y - s.pos.y) + vel.y).clamp(-6.0, 6.0);
        // Close in, fly the other plane's speed and a little more, so as to sit on its track.
        let throttle = if d < 50.0 {
            let error = vel.length() + (0.3 * d).clamp(4.0, 10.0) - s.airspeed;
            self.pace = (self.pace + 0.06 * error * dt).clamp(-0.4, 0.4);
            (0.6 + 0.1 * error + self.pace).clamp(0.15, 1.0)
        } else {
            1.0
        };
        self.hold(s, bank, climb, throttle, ground, dt)
    }

    /// The climb rate that brings the plane to a height. In a steep bank the elevator mostly
    /// tightens the turn, so the pilot asks it for less, or height and turn swing against each
    /// other.
    fn climb_for(&self, s: &FlightState, height: f32) -> f32 {
        let roll = physics::euler(s.rot).0;
        let gain = 0.6 * roll.cos().abs().max(0.3).powi(2);
        (gain * (height - s.pos.y)).clamp(-4.0, 5.0)
    }

    /// The sideways acceleration (right positive) that arcs the plane onto a point (L1
    /// guidance). Behind it, as hard as it likes, towards the nearer side.
    fn l1_accel(&self, s: &FlightState, aim: Vec3) -> f32 {
        let to = flat(aim - s.pos);
        let mut v = flat(s.vel);
        if v.length() < 3.0 {
            v = flat(s.rot * Vec3::NEG_Z);
        }
        let speed = v.length().max(8.0);
        let (vd, td) = (v.normalize_or_zero(), to.normalize_or_zero());
        // To the right of the way it is going is positive (+X east, -Z north).
        let right = Vec3::new(-vd.z, 0.0, vd.x);
        let (sin, cos) = (td.dot(right), td.dot(vd));
        // Far away, aim along the line to it, so a big heading error still turns hard.
        let reach = to.length().clamp(10.0, 40.0);
        if cos > 0.0 { 2.0 * speed * speed * sin / reach } else { 1e4f32.copysign(sin) }
    }

    /// Fly along a line `height` metres above it, at a throttle.
    pub fn follow(&mut self, s: &FlightState, path: &Path, height: f32, throttle: f32, ground: &dyn Terrain, dt: f32) -> Controls {
        let i = path.nearest(s.pos, self.cursor);
        self.cursor = Some(i);
        let v = flat(s.vel);
        let speed = v.length().max(8.0);
        // On the line: turn as the line turns (looking ahead by the time it takes to roll), and
        // steer back onto it by how far off it is and how fast it is drifting.
        let (on, tangent, curve) = path.frame(i, 0.4 * speed);
        let normal = Vec3::new(-tangent.z, 0.0, tangent.x);
        let off = flat(s.pos - on).dot(normal);
        let drift = v.dot(normal);
        let near = speed * speed * curve - 0.2 * off - 0.8 * drift;
        // Far off it: head for a point on it, as `toward` does.
        let lookahead = self.skill.turn_radius.clamp(15.0, 40.0).max(off.abs());
        let mut aim = path.ahead(i, lookahead);
        aim.y = ground.height(aim.x, aim.z) + height;
        let far = self.l1_accel(s, aim);
        let w = ((off.abs() - 12.0) / 18.0).clamp(0.0, 1.0);
        let accel = near * (1.0 - w) + far * w;
        let climb = self.climb_for(s, aim.y);
        self.hold(s, (accel / GRAVITY).atan(), climb, throttle, ground, dt)
    }

    /// Just after the throw: wings level, full throttle, climbing.
    pub fn climb_out(&mut self, s: &FlightState, ground: &dyn Terrain, dt: f32) -> Controls {
        self.hold(s, 0.0, 3.0, 1.0, ground, dt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    use simulation_aircraft::AircraftParams;
    use simulation_physics::{Environment, Flat, Phase, Start};

    const DT: f32 = 1.0 / 600.0;

    /// A T1-S flying level at `height` m and 22 m/s, heading north from the origin.
    fn flying(height: f32) -> (AircraftParams, FlightState) {
        let p = t1s_ranger::params();
        let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        s.carry = None;
        s.phase = Phase::Flying;
        s.pos = Vec3::new(0.0, height, 0.0);
        s.rot = Quat::IDENTITY;
        s.vel = Vec3::new(0.0, 0.0, -22.0);
        s.spool.fill(0.6);
        (p, s)
    }

    /// Flies for `seconds`, the pilot choosing the controls every step, and looks at each step.
    fn fly(p: &AircraftParams, s: &mut FlightState, seconds: f32, mut pilot: impl FnMut(&FlightState) -> Controls, mut look: impl FnMut(&FlightState)) {
        for _ in 0..(seconds / DT) as usize {
            let c = pilot(s).mix();
            physics::step(p, s, &c, &Environment::default(), &Flat, &[], DT);
            look(s);
        }
    }

    fn flat_dist(a: Vec3, b: Vec3) -> f32 {
        Vec3::new(a.x - b.x, 0.0, a.z - b.z).length()
    }

    /// ArduPlane's FBWA as the T1-S is set up (ardupilot/t1s.parm).
    const T1S_FBWA: Fbwa = Fbwa { roll_limit: 50.0 * DEG, pitch_max: 25.0 * DEG, pitch_min: -25.0 * DEG };

    /// With a radio in FBWA, the roll stick asks the autopilot for a bank: halfway over is half
    /// the bank it allows, and it can ask for no more than all of it.
    #[test]
    fn with_a_radio_the_roll_stick_asks_for_a_bank() {
        let (_, mut s) = flying(60.0);
        s.airspeed = 22.0;
        let mut pilot = Pilot::with_radio(ACE, T1S_FBWA);
        let c = pilot.hold(&s, 25.0 * DEG, 0.0, 0.8, &Flat, 0.01);
        assert!((c.aileron - 0.5).abs() < 0.02, "half the limit is half the stick: {}", c.aileron);
        let c = pilot.hold(&s, -80.0 * DEG, 0.0, 0.8, &Flat, 0.01);
        assert!((c.aileron + 1.0).abs() < 1e-3, "the stick stops at the limit: {}", c.aileron);
        assert_eq!(c.rudder, 0.0, "the autopilot coordinates the turn");
    }

    /// The pitch stick asks for a pitch: up to climb, down to come down.
    #[test]
    fn with_a_radio_the_pitch_stick_asks_for_a_climb() {
        let (_, s) = flying(60.0);
        let mut pilot = Pilot::with_radio(ACE, T1S_FBWA);
        let up = pilot.hold(&s, 0.0, 4.0, 0.8, &Flat, 0.01).elevator;
        let down = pilot.hold(&s, 0.0, -4.0, 0.8, &Flat, 0.01).elevator;
        assert!(up > 0.1 && down < -0.1, "climb {up}, descend {down}");
        assert!(up <= 1.0 && down >= -1.0);
    }

    /// Low down, it levels the wings and climbs, with a radio as with its hands.
    #[test]
    fn with_a_radio_it_will_not_fly_into_the_ground() {
        let (_, s) = flying(2.0);
        let mut pilot = Pilot::with_radio(ACE, T1S_FBWA);
        let c = pilot.hold(&s, 60.0 * DEG, -5.0, 0.8, &Flat, 0.01);
        assert!(c.aileron.abs() < 0.25 && c.elevator > 0.0, "{c:?}");
    }

    #[test]
    fn the_controls_mix_like_the_sims_manual() {
        let c = Controls { aileron: 0.5, elevator: 0.4, rudder: 0.2, throttle: 0.8 }.mix();
        let manual = t1s_ranger::mix(0.5, 0.4, 0.5 * 0.2, [0.8 + 0.1 * 0.2, 0.8 - 0.1 * 0.2]);
        assert_eq!(c.out, manual.out);
    }

    #[test]
    fn it_holds_a_steep_turn_without_losing_height() {
        let (p, mut s) = flying(20.0);
        let mut pilot = Pilot::new(ACE);
        let (mut worst_height, mut worst_bank) = (0.0f32, 0.0f32);
        let mut t = 0.0;
        fly(&p, &mut s, 12.0, |s| pilot.hold(s, 65.0 * DEG, 0.0, 1.0, &Flat, DT), |s| {
            t += DT;
            if t > 3.0 {
                worst_height = worst_height.max((s.pos.y - 20.0).abs());
                worst_bank = worst_bank.max((physics::euler(s.rot).0 - 65.0 * DEG).abs());
            }
        });
        assert_eq!(s.phase, Phase::Flying);
        assert!(worst_height < 3.0, "height off by {worst_height:.1} m");
        assert!(worst_bank < 8.0 * DEG, "bank off by {:.1} deg", worst_bank / DEG);
    }

    #[test]
    fn it_rolls_from_one_steep_bank_to_the_other_and_keeps_its_height() {
        let (p, mut s) = flying(20.0);
        let mut pilot = Pilot::new(ACE);
        let mut worst = 0.0f32;
        let t = std::cell::Cell::new(0.0f32);
        fly(&p, &mut s, 16.0, |s| {
            let bank = if (t.get() / 4.0) as i32 % 2 == 0 { 60.0 } else { -60.0 };
            pilot.hold(s, bank * DEG, 0.0, 1.0, &Flat, DT)
        }, |s| {
            t.set(t.get() + DT);
            if t.get() > 2.0 {
                worst = worst.max((s.pos.y - 20.0).abs());
            }
        });
        assert_eq!(s.phase, Phase::Flying);
        assert!(worst < 4.0, "height off by {worst:.1} m");
    }

    #[test]
    fn it_flies_to_a_point() {
        let (p, mut s) = flying(15.0);
        let mut pilot = Pilot::new(CLUB);
        // Behind it and to the right, and higher.
        let aim = Vec3::new(120.0, 25.0, 60.0);
        let mut closest = f32::MAX;
        fly(&p, &mut s, 20.0, |s| pilot.toward(s, aim, 0.9, &Flat, DT), |s| closest = closest.min(s.pos.distance(aim)));
        assert!(closest < 4.0, "closest {closest:.1} m");
    }

    #[test]
    fn it_follows_a_tight_circle() {
        let (p, mut s) = flying(10.0);
        let radius = ACE.turn_radius;
        let centre = Vec3::new(-radius, 0.0, 0.0);
        // Counterclockwise from above, starting under the plane.
        let pts: Vec<Vec3> = (0..200)
            .map(|k| {
                let a = -(k as f32) / 200.0 * std::f32::consts::TAU;
                centre + Vec3::new(a.cos(), 0.0, a.sin()) * radius
            })
            .collect();
        let path = Path::closed(pts);
        let mut pilot = Pilot::new(ACE);
        let (mut worst_track, mut worst_height) = (0.0f32, 0.0f32);
        let mut t = 0.0;
        fly(&p, &mut s, 40.0, |s| pilot.follow(s, &path, 10.0, 1.0, &Flat, DT), |s| {
            t += DT;
            if t > 5.0 {
                worst_track = worst_track.max((flat_dist(s.pos, centre) - radius).abs());
                worst_height = worst_height.max((s.pos.y - 10.0).abs());
            }
        });
        assert_eq!(s.phase, Phase::Flying);
        assert!(worst_track < 5.0, "off the circle by {worst_track:.1} m");
        assert!(worst_height < 4.0, "height off by {worst_height:.1} m");
    }

    #[test]
    fn a_turning_plane_is_predicted_along_its_curve() {
        // Flying north at 20 m/s, turning left (counterclockwise from above) at 0.5 rad/s: a
        // quarter circle of radius 40 m takes pi seconds and ends 40 m west and 40 m north.
        let vel = Vec3::new(0.0, 0.0, -20.0);
        let mut v = vel;
        let dt = 0.001;
        let turned = v + (Vec3::new(-20.0 * 0.5 * dt, 0.0, 0.0));
        let turn = (turned - v).dot(Vec3::new(v.z, 0.0, -v.x)) / v.length_squared() / dt;
        assert!((turn - 0.5).abs() < 0.01, "turn {turn}");
        v = vel;
        let d = along_curve(v, turn, std::f32::consts::FRAC_PI_2 / 0.5);
        assert!((d - Vec3::new(-40.0, 0.0, -40.0)).length() < 0.5, "{d:?}");
    }

    #[test]
    fn it_closes_on_a_plane_below_it_in_a_turn() {
        // The other plane circles 10 m below; the pilot starts outside its circle, above it.
        let (p, mut s) = flying(30.0);
        let mut pilot = Pilot::new(CLUB);
        let (centre, radius, speed) = (Vec3::new(-60.0, 0.0, -40.0), 40.0f32, 18.0f32);
        let other = |t: f32| {
            let a = t * speed / radius;
            (centre + Vec3::new(a.cos(), 0.0, -a.sin()) * radius + Vec3::Y * 20.0, Vec3::new(-a.sin(), 0.0, -a.cos()) * speed)
        };
        let mut t = 0.0;
        let mut closest = f32::MAX;
        for _ in 0..(40.0 / DT) as usize {
            let (pos, vel) = other(t);
            let c = pilot.intercept(&s, pos, vel, &Flat, DT);
            physics::step(&p, &mut s, &c.mix(), &Environment::default(), &Flat, &[], DT);
            t += DT;
            closest = closest.min(s.pos.distance(other(t).0));
        }
        assert!(closest < 1.5, "closest {closest:.1} m");
    }

    #[test]
    fn after_the_throw_it_climbs_away() {
        let p = t1s_ranger::params();
        let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        let mut pilot = Pilot::new(ROOKIE);
        s.launch();
        fly(&p, &mut s, 5.0, |s| pilot.climb_out(s, &Flat, DT), |_| {});
        assert_eq!(s.phase, Phase::Flying);
        assert!(s.pos.y > 6.0, "only {:.1} m up", s.pos.y);
        assert!(physics::euler(s.rot).0.abs() < 10.0 * DEG);
    }

    #[test]
    fn it_will_not_fly_into_the_ground() {
        let (p, mut s) = flying(15.0);
        let mut pilot = Pilot::new(ACE);
        // Told to dive at the ground, in a steep bank.
        fly(&p, &mut s, 15.0, |s| pilot.hold(s, 70.0 * DEG, -6.0, 1.0, &Flat, DT), |_| {});
        assert_eq!(s.phase, Phase::Flying, "{}", s.crash_reason);
    }
}

/// What the elevator must do to hold height at each bank and throttle, on the flight model:
/// `cargo test --release -p recreation_pilot trim_by_bank -- --ignored --nocapture`. The trim
/// the pilot learns should stay small; if it doesn't, refit `ELEVATOR_LEVEL` and
/// `ELEVATOR_FOR_LIFT`.
#[cfg(test)]
mod probe {
    use super::*;
    use glam::Quat;
    use simulation_physics::{Environment, Flat, Phase, Start};

    #[test]
    #[ignore]
    fn trim_by_bank() {
        for bank in [0.0f32, 30.0, 45.0, 55.0, 62.0, 68.0] {
            for thr in [0.7f32, 1.0] {
                let p = t1s_ranger::params();
                let mut s = FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
                s.carry = None;
                s.phase = Phase::Flying;
                s.pos = Vec3::new(0.0, 30.0, 0.0);
                s.rot = Quat::IDENTITY;
                s.vel = Vec3::new(0.0, 0.0, -22.0);
                let mut pilot = Pilot::new(ACE);
                let dt = 1.0 / 600.0;
                let mut c = Controls::default();
                for _ in 0..(15.0 / dt) as usize {
                    c = pilot.hold(&s, bank * DEG, 0.0, thr, &Flat, dt);
                    simulation_physics::step(&p, &mut s, &c.mix(), &Environment::default(), &Flat, &[], dt);
                }
                let (roll, pitch, _) = physics::euler(s.rot);
                println!("bank {bank:>4} thr {thr}: trim {:+.3} elevator {:+.3} v {:.1} vy {:+.2} roll {:.0} pitch {:+.1} radius {:.1}", pilot.trim, c.elevator, s.vel.length(), s.vel.y, roll / DEG, pitch / DEG, s.vel.length().powi(2) / (GRAVITY * roll.tan().abs().max(1e-3)));
            }
        }
    }
}
