//! A game in play: the computer pilots, their planes on the flight model, and the referee.
//!
//! The host (the sim) owns the pilot's plane and steps it; every physics step it hands the
//! session that plane and the world around it, and the session steps the opponents' planes the
//! same way (`simulation_physics::step`, the same T1-S, the same wind, ground and trees) and
//! lets the referee judge everyone. Player 0 is you; the opponents are players 1 to 3.
//!
//! A game starts with everyone's plane in hand and a countdown. At GO the computer pilots throw,
//! one each second as the pylon racing leagues release their models, and you throw when you like.
//!
//! With autopilots (`Session::with_autopilots`), the computer pilots fly planes like yours, each
//! with an autopilot of its own, through a radio: the host steps each of their planes from its
//! autopilot's outputs, and asks the session what that computer pilot's radio sends
//! (`Session::radio`). Everyone starts on the ground, disarmed, and nobody arms before GO.

pub use recreation_pilot::{self as pilot, Controls};
pub use recreation_racing::{self as racing, Gate, Status};
pub use recreation_tag::{self as tag, Arena};

use glam::{Quat, Vec3};
use recreation_pilot::{ACE, CLUB, Fbwa, Path, Pilot, ROOKIE, Skill};
use recreation_racing::{Course, Race};
use recreation_tag::{Plane, Rules, Tag};
use simulation_aircraft::{AircraftParams, t1s_ranger};
use simulation_physics::{self as physics, Environment, FlightState, Obstacle, Phase, Start, Terrain};

/// From the start of a game to GO (s).
pub const COUNTDOWN: f32 = 5.0;
/// The computer pilots throw this long apart (s).
pub const RELEASE_INTERVAL: f32 = 1.0;
/// How high the computer pilots race (m above the ground): above the pylons.
pub const RACE_HEIGHT: f32 = 10.0;
/// How far in front of the pilot box the middle of the tag arena is (m).
pub const ARENA_AHEAD: f32 = 70.0;
/// About as fast as a T1-S flies flat out (m/s): what a chaser has to catch with.
const TOP_SPEED: f32 = 26.0;
/// A computer pilot that crashes in a game of tag gets its plane back after this long (s).
const DOWN_TIME: f32 = 3.0;
/// With an autopilot, how high a computer pilot climbs in a hover before it flies on the wing
/// (m above the ground).
const TAKEOFF_HEIGHT: f32 = 15.0;
/// With an autopilot, how long it holds its arm switch on before it decides the plane will not
/// arm yet, and how long it waits before it tries again (s).
const ARM_TRY: f32 = 3.0;
const ARM_REST: f32 = 1.0;
/// In FBWA the autopilot limits the bank (50° on the T1-S), so the racing lines are wider.
const RADIO_TURN: f32 = 1.5;

/// Where the games are played: the pilot box, and the way the pilot faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// The pilot box, on the ground.
    pub pilot: Vec3,
    /// The way the pilot faces (radians about +Y, 0 = north), as the plane is thrown.
    pub heading: f32,
}

impl Field {
    pub fn forward(&self) -> Vec3 {
        Quat::from_rotation_y(self.heading) * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y)
    }

    /// The pylon course on this field.
    pub fn course(&self) -> Course {
        Course::in_front_of(self.pilot, self.forward())
    }

    /// The arena for tag: round the middle of the field in front of the pilot box, so that
    /// the pilots stand inside it.
    pub fn arena(&self) -> Arena {
        Arena::around(self.pilot + self.forward() * ARENA_AHEAD)
    }

    /// Where computer pilot `i` stands: in a line to the right of the pilot box.
    pub fn hand(&self, i: usize) -> Start {
        Start { pos: self.pilot + self.right() * 2.5 * (i + 1) as f32, heading: self.heading }
    }
}

/// The games there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Race,
    Tag,
}

/// The game clock, as the pilot sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Clock {
    /// Until you take off (see `Session::after_takeoff`).
    Waiting,
    CountingDown {
        left: f32,
    },
    /// Seconds since GO.
    Running {
        time: f32,
    },
    /// Your game is over (you finished, or are out, or the clock ran out): its time then.
    Stopped {
        time: f32,
    },
}

/// The referee of the game.
#[derive(Clone, Debug)]
pub enum Referee {
    Race(Race),
    Tag(Tag),
}

/// The world the planes fly in.
pub struct Surroundings<'a> {
    pub ground: &'a dyn Terrain,
    pub obstacles: &'a [Obstacle],
    /// The wind at a point and time (world frame, m/s).
    pub wind: &'a dyn Fn(Vec3, f64) -> Vec3,
}

/// A computer pilot and its plane.
#[derive(Clone, Debug)]
pub struct Opponent {
    pub name: &'static str,
    pub skill: Skill,
    /// The colour of its plane's markings (linear RGB).
    pub colour: [f32; 3],
    pub state: FlightState,
    start: Start,
    pilot: Pilot,
    line: Path,
    /// When it throws (s after GO).
    throw_at: f32,
    /// When it last threw (s after GO).
    thrown_at: Option<f32>,
    /// How long it has been crashed.
    down_for: f32,
    /// Who it chases, when it is it, and when it chose.
    target: Option<(usize, f32)>,
    /// With an autopilot: how far through its preflight it is.
    preflight: Preflight,
}

/// Where a preflight is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    /// Before GO, or its plane is being set down: switch off, throttle down.
    #[default]
    Cold,
    /// The arm switch is on, with the throttle down, for this long.
    Arming(f32),
    /// The plane would not arm: switch off for this long, then try again.
    Resting(f32),
    /// Armed: climbing straight up in QHOVER.
    Climbing,
    /// In FBWA, speeding up, for this long.
    Transition(f32),
    Flying,
}

/// What a computer pilot's radio sends: the sticks, the arm switch and the position of the mode
/// switch (its slot in the plane's `rc_modes`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Radio {
    /// Roll, pitch and yaw -1..1 (in `aileron`, `elevator`, `rudder`), throttle 0..1.
    pub sticks: Controls,
    pub arm: bool,
    pub mode: usize,
}

/// A pilot's preflight with an autopilot, done on its radio as a pilot does it: at GO it arms
/// with the throttle down, and tries again if the props don't turn. Once they turn it climbs in
/// QHOVER, pushes FBWA at height, and once the wing has taken over the plane is on the wing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Preflight {
    stage: Stage,
}

impl Preflight {
    /// Back to the start: switch off, throttle down.
    pub fn reset(&mut self) {
        self.stage = Stage::Cold;
    }

    /// The take-off is done: the plane flies on the wing in FBWA.
    pub fn on_the_wing(&self) -> bool {
        self.stage == Stage::Flying
    }

    /// What the radio sends for the next `dt` of the preflight, for the plane `state` of `params`
    /// at `agl` above the ground; `None` once it is on the wing, when the pilot flies it in FBWA.
    pub fn radio(&mut self, params: &AircraftParams, state: &FlightState, agl: f32, dt: f32) -> Option<Radio> {
        let (qhover, fbwa) = (mode_slot(params, "QHOVER"), mode_slot(params, "FBWA"));
        let props = params.motors.iter().zip(&state.spool).any(|(m, &spool)| m.lift && spool > 0.03);
        let off = Radio { sticks: Controls::default(), arm: false, mode: qhover };
        let on =
            |throttle: f32, mode: usize| Radio { sticks: Controls { throttle, ..Default::default() }, arm: true, mode };
        self.stage = match self.stage {
            Stage::Cold => Stage::Arming(0.0),
            Stage::Arming(_) if props => Stage::Climbing,
            Stage::Arming(t) if t > ARM_TRY => Stage::Resting(0.0),
            Stage::Arming(t) => Stage::Arming(t + dt),
            Stage::Resting(t) if t > ARM_REST => Stage::Arming(0.0),
            Stage::Resting(t) => Stage::Resting(t + dt),
            Stage::Climbing if agl > TAKEOFF_HEIGHT => Stage::Transition(0.0),
            Stage::Transition(t) if t > 3.0 && state.airspeed > 15.0 => Stage::Flying,
            Stage::Transition(t) => Stage::Transition(t + dt),
            stage => stage,
        };
        match self.stage {
            Stage::Cold | Stage::Resting(_) => Some(off),
            Stage::Arming(_) => Some(on(0.0, qhover)),
            // Above the middle of the stick, QHOVER climbs.
            Stage::Climbing => Some(on(0.9, qhover)),
            // Wings level and a little nose up while the wing takes over.
            Stage::Transition(_) => Some(Radio {
                sticks: Controls { elevator: 0.2, throttle: 1.0, ..Default::default() },
                arm: true,
                mode: fbwa,
            }),
            Stage::Flying => None,
        }
    }
}

/// The slot of the mode switch that gives `mode` on the plane `params`.
pub fn mode_slot(params: &AircraftParams, mode: &str) -> usize {
    params.rc_modes.iter().position(|m| *m == mode).unwrap_or(0)
}

/// The computer pilots' planes and autopilots, when they fly with them.
#[derive(Clone, Debug)]
struct Autopilots {
    params: AircraftParams,
    qhover: usize,
    fbwa: usize,
}

/// The three computer pilots: an ace, a club pilot and a rookie.
pub const PILOTS: [(&str, Skill, [f32; 3]); 3] =
    [("Kestrel", ACE, [0.05, 0.6, 0.12]), ("Merlin", CLUB, [0.05, 0.25, 0.9]), ("Swift", ROOKIE, [0.85, 0.75, 0.05])];

/// A game in play.
#[derive(Clone, Debug)]
pub struct Session {
    pub kind: Kind,
    pub field: Field,
    pub course: Course,
    pub arena: Arena,
    pub referee: Referee,
    pub opponents: Vec<Opponent>,
    pub clock: Clock,
    params: AircraftParams,
    /// Seconds since the game started (the countdown included).
    time: f32,
    /// Your plane has been in the air since GO.
    you_flew: bool,
    /// A computer chaser goes for you (player 0) whenever it can tag you, whatever else is near.
    /// Off in a game; the acceptance tests turn it on to be tagged without waiting for the
    /// chaser to choose you.
    pub chase_you: bool,
    /// With autopilots: their planes, and the mode switch's QHOVER and FBWA.
    autopilots: Option<Autopilots>,
    /// Your position, velocity and whether you are in the air, as of the last step.
    you_now: (Vec3, Vec3, bool),
}

impl Session {
    pub fn race(field: Field) -> Session {
        let course = field.course();
        Session::new(Kind::Race, field, Referee::Race(Race::new(course, PILOTS.len() + 1)))
    }

    pub fn tag(field: Field) -> Session {
        // You are it first.
        Session::new(Kind::Tag, field, Referee::Tag(Tag::new(field.arena(), Rules::default(), PILOTS.len() + 1, 0)))
    }

    fn new(kind: Kind, field: Field, referee: Referee) -> Session {
        let params = t1s_ranger::params();
        let course = field.course();
        let opponents = PILOTS
            .iter()
            .enumerate()
            .map(|(i, &(name, skill, colour))| {
                let start = field.hand(i);
                Opponent {
                    name,
                    skill,
                    colour,
                    state: FlightState::new(&params, &start),
                    start,
                    pilot: Pilot::new(skill),
                    line: Path::closed(course.outside_line(skill.turn_radius, 1.0)),
                    throw_at: i as f32 * RELEASE_INTERVAL,
                    thrown_at: None,
                    down_for: 0.0,
                    target: None,
                    preflight: Preflight::default(),
                }
            })
            .collect();
        Session {
            kind,
            field,
            arena: field.arena(),
            course,
            referee,
            opponents,
            clock: Clock::CountingDown { left: COUNTDOWN },
            params,
            time: 0.0,
            you_flew: false,
            autopilots: None,
            chase_you: false,
            you_now: (field.pilot, Vec3::ZERO, false),
        }
    }

    /// The computer pilots fly planes like yours (`params`), each with an autopilot of its own,
    /// through a radio whose sticks talk to the autopilot's FBWA (`fbwa`). `planes` are their
    /// planes as they are now: each is carried to its place on the ground, disarmed. The
    /// countdown starts at once, and waits while a plane is being set down.
    pub fn with_autopilots(mut self, params: AircraftParams, fbwa: Fbwa, planes: Vec<FlightState>) -> Session {
        for (o, mut plane) in self.opponents.iter_mut().zip(planes) {
            plane.reset_to_hand(&params, &o.start);
            o.state = plane;
            o.pilot = Pilot::with_radio(o.skill, fbwa);
            o.line = Path::closed(self.course.outside_line(o.skill.turn_radius * RADIO_TURN, 1.0));
            o.preflight.reset();
        }
        self.autopilots =
            Some(Autopilots { qhover: mode_slot(&params, "QHOVER"), fbwa: mode_slot(&params, "FBWA"), params });
        self.clock = Clock::CountingDown { left: COUNTDOWN };
        self
    }

    /// The computer pilots' planes, back to the host at the end of the game.
    pub fn into_planes(self) -> Vec<FlightState> {
        self.opponents.into_iter().map(|o| o.state).collect()
    }

    /// The plane the computer pilots fly: yours with autopilots, else the T1-S.
    pub fn opponent_params(&self) -> &AircraftParams {
        self.autopilots.as_ref().map_or(&self.params, |a| &a.params)
    }

    /// What computer pilot `i`'s radio sends now. With autopilots, the host calls it after each
    /// step its autopilot gives the plane. Before GO the switch is off and the throttle down. At
    /// GO it arms, with the throttle down, and tries again if the props don't turn. Once they
    /// turn it climbs in QHOVER, pushes FBWA at height, and then flies the game.
    pub fn radio(&mut self, i: usize, ground: &dyn Terrain, dt: f32) -> Radio {
        let Some(ap) = &self.autopilots else { return Radio::default() };
        let (qhover, fbwa) = (ap.qhover, ap.fbwa);
        let off = Radio { sticks: Controls::default(), arm: false, mode: qhover };
        let started = self.since_go() >= 0.0 && matches!(self.clock, Clock::Running { .. } | Clock::Stopped { .. });
        let tag = self.kind == Kind::Tag;
        let o = &mut self.opponents[i];
        if o.state.phase == Phase::Crashed {
            // In tag the pilot fetches the plane and starts again; a crash ends a race.
            o.down_for += dt;
            if tag && o.down_for > DOWN_TIME {
                o.down_for = 0.0;
                o.state.reset_to_hand(&ap.params, &o.start);
                o.preflight.reset();
            }
            return off;
        }
        if !started || o.state.carry.is_some() || o.state.phase == Phase::Held {
            o.preflight.reset();
            return off;
        }
        let agl = o.state.pos.y - ground.height(o.state.pos.x, o.state.pos.z);
        if let Some(radio) = o.preflight.radio(&ap.params, &o.state, agl, dt) {
            return radio;
        }
        let planes = self.planes();
        Radio { sticks: self.steer(i, &planes, ground, dt), arm: true, mode: fbwa }
    }

    /// Every player's position, velocity and whether it is in the air (0 is you, as of the last
    /// step).
    fn planes(&self) -> Vec<(Vec3, Vec3, bool)> {
        let airborne =
            |s: &FlightState| matches!(s.phase, Phase::Launching | Phase::Flying) && s.carry.is_none() && !s.on_ground;
        std::iter::once(self.you_now)
            .chain(self.opponents.iter().map(|o| (o.state.pos, o.state.vel, airborne(&o.state))))
            .collect()
    }

    /// A game that waits for you to take off: the countdown starts when your plane is in the air.
    pub fn after_takeoff(mut self) -> Session {
        self.clock = Clock::Waiting;
        self
    }

    /// Whether a plane may be thrown now: not during the countdown.
    pub fn may_launch(&self) -> bool {
        !matches!(self.clock, Clock::CountingDown { .. })
    }

    /// Seconds since GO (negative before it).
    pub fn since_go(&self) -> f32 {
        self.time - COUNTDOWN
    }

    /// Runs the game on by `dt` seconds, with your plane as it is now.
    pub fn step(&mut self, you: &FlightState, world: &Surroundings, dt: f32) {
        let airborne = |s: &FlightState| matches!(s.phase, Phase::Launching | Phase::Flying) && s.carry.is_none();
        if self.clock == Clock::Waiting {
            if !airborne(you) {
                return;
            }
            self.clock = Clock::CountingDown { left: COUNTDOWN };
        }
        self.you_now = (you.pos, you.vel, airborne(you) && !you.on_ground);
        // With autopilots, the countdown waits until every plane is set down at its place.
        let settling = |s: &FlightState| s.carry.is_some() || s.phase == Phase::Held;
        if self.autopilots.is_some()
            && matches!(self.clock, Clock::CountingDown { .. })
            && (settling(you) || self.opponents.iter().any(|o| settling(&o.state)))
        {
            return;
        }
        self.time += dt;
        let go = self.since_go();
        if let Clock::CountingDown { .. } = self.clock {
            self.clock = if go >= 0.0 { Clock::Running { time: go } } else { Clock::CountingDown { left: -go } };
        }

        // The computer pilots fly.
        let planes: Vec<(Vec3, Vec3, bool)> = std::iter::once(you)
            .chain(self.opponents.iter().map(|o| &o.state))
            .map(|s| (s.pos, s.vel, airborne(s) && !s.on_ground))
            .collect();
        // With autopilots, their planes are the host's to step (see `radio`).
        let hands = if self.autopilots.is_none() { self.opponents.len() } else { 0 };
        for i in 0..hands {
            let controls = self.brain(i, &planes, world.ground, dt);
            let o = &mut self.opponents[i];
            let wind = (world.wind)(o.state.pos, o.state.time);
            physics::step(
                &self.params,
                &mut o.state,
                &controls.mix(),
                &Environment { wind },
                world.ground,
                world.obstacles,
                dt,
            );
        }

        if go < 0.0 {
            return;
        }
        if airborne(you) {
            self.you_flew = true;
        }
        // The referee.
        match &mut self.referee {
            Referee::Race(race) => {
                if airborne(you) {
                    race.observe(0, you.pos, go);
                } else if self.you_flew {
                    // Crashed, or picked up.
                    race.retire(0);
                }
                for (i, o) in self.opponents.iter().enumerate() {
                    if airborne(&o.state) {
                        race.observe(i + 1, o.state.pos, go);
                    } else if o.state.phase == Phase::Crashed {
                        race.retire(i + 1);
                    }
                }
                if let (Clock::Running { .. }, s) = (self.clock, race.racers[0].status) {
                    match s {
                        Status::Racing => self.clock = Clock::Running { time: go },
                        Status::Finished { time, .. } => self.clock = Clock::Stopped { time },
                        Status::Disqualified | Status::Out => self.clock = Clock::Stopped { time: go },
                    }
                }
            }
            Referee::Tag(tag) => {
                let planes: Vec<Plane> = std::iter::once(you)
                    .chain(self.opponents.iter().map(|o| &o.state))
                    .map(|s| Plane { pos: s.pos, flying: airborne(s) && !s.on_ground })
                    .collect();
                tag.step(&planes, dt);
                self.clock =
                    if tag.over() { Clock::Stopped { time: tag.time } } else { Clock::Running { time: tag.time } };
            }
        }
    }

    /// What computer pilot `i` does with its controls. `planes` is every player's position,
    /// velocity and whether it is in the air.
    fn brain(&mut self, i: usize, planes: &[(Vec3, Vec3, bool)], ground: &dyn Terrain, dt: f32) -> Controls {
        let go = self.since_go();
        let o = &mut self.opponents[i];
        match o.state.phase {
            Phase::Held => {
                if o.state.carry.is_some() || go < o.throw_at {
                    return Controls::default();
                }
                o.state.launch();
                o.thrown_at = Some(go);
                o.pilot.lose_place();
                return Controls { throttle: 1.0, ..Default::default() };
            }
            Phase::Crashed => {
                // In tag the pilot fetches the plane and throws again; a crash ends a race.
                o.down_for += dt;
                if self.kind == Kind::Tag && o.down_for > DOWN_TIME {
                    o.down_for = 0.0;
                    o.state.reset_to_hand(&self.params, &o.start);
                    o.throw_at = go + 1.0;
                }
                return Controls::default();
            }
            Phase::Launching | Phase::Flying => {}
        }
        let s = &o.state;
        let agl = s.pos.y - ground.height(s.pos.x, s.pos.z);
        if o.thrown_at.is_some_and(|t| go - t < 2.5) || (agl < 6.0 && o.thrown_at.is_some_and(|t| go - t < 6.0)) {
            return o.pilot.climb_out(s, ground, dt);
        }
        self.steer(i, planes, ground, dt)
    }

    /// How computer pilot `i` flies the game once it is in the air: round the course, or after
    /// the others, or away from the one that is it.
    fn steer(&mut self, i: usize, planes: &[(Vec3, Vec3, bool)], ground: &dyn Terrain, dt: f32) -> Controls {
        let go = self.since_go();
        let me = i + 1;
        let o = &mut self.opponents[i];
        let s = &o.state;
        let finished = match &self.referee {
            Referee::Race(race) => race.racers[me].status != Status::Racing,
            Referee::Tag(tag) => tag.over(),
        };
        if finished {
            // Circle over the field until the game is put away.
            let centre = self.course.centre();
            let a = go * 0.35 + i as f32 * 2.1;
            let aim = centre + Vec3::new(a.cos(), 0.0, -a.sin()) * 60.0 + Vec3::Y * (30.0 + 5.0 * i as f32);
            return o.pilot.toward(s, aim, 0.6, ground, dt);
        }
        match &self.referee {
            Referee::Race(_) => o.pilot.follow(s, &o.line, RACE_HEIGHT, o.skill.throttle, ground, dt),
            Referee::Tag(tag) => {
                if tag.it == me {
                    // Choose who to chase now and then, not every step.
                    if o.target.is_none_or(|(_, at)| go - at > 1.5) {
                        o.target = if self.chase_you && me != 0 && planes[0].2 && tag.may_tag(0) {
                            Some((0, go))
                        } else {
                            choose_target(tag, me, planes, o.target.map(|(t, _)| t)).map(|t| (t, go))
                        };
                    }
                    match o.target {
                        Some((t, _)) => chase(&mut o.pilot, s, planes[t], ground, dt),
                        None => wander(&mut o.pilot, s, &self.arena, go, i, ground, dt),
                    }
                } else {
                    o.target = None;
                    let (threat, _, up) = planes[tag.it];
                    if up && threat.distance(s.pos) < o.skill.awareness {
                        evade(&mut o.pilot, s, threat, &self.arena, ground, dt)
                    } else {
                        wander(&mut o.pilot, s, &self.arena, go, i, ground, dt)
                    }
                }
            }
        }
    }

    /// The name of player `i` (0 is you).
    pub fn name(&self, i: usize) -> &'static str {
        if i == 0 { "You" } else { self.opponents[i - 1].name }
    }

    pub fn race_ref(&self) -> Option<&Race> {
        match &self.referee {
            Referee::Race(r) => Some(r),
            Referee::Tag(_) => None,
        }
    }

    pub fn tag_ref(&self) -> Option<&Tag> {
        match &self.referee {
            Referee::Tag(t) => Some(t),
            Referee::Race(_) => None,
        }
    }

    pub fn tag_ref_mut(&mut self) -> Option<&mut Tag> {
        match &mut self.referee {
            Referee::Tag(t) => Some(t),
            Referee::Race(_) => None,
        }
    }

    /// Your laps, in a race.
    pub fn your_laps(&self) -> u32 {
        self.race_ref().map_or(0, |r| r.racers[0].laps)
    }

    /// Your cuts, in a race.
    pub fn your_cuts(&self) -> u32 {
        self.race_ref().map_or(0, |r| r.racers[0].cuts)
    }

    /// Who is it, in a game of tag.
    pub fn chaser(&self) -> Option<usize> {
        self.tag_ref().map(|t| t.it)
    }

    /// With autopilots: computer pilot `i` has taken off and is flying the game on the wing (in
    /// FBWA, the transition done). A ground agent takes over from here.
    pub fn on_the_wing(&self, i: usize) -> bool {
        self.opponents[i].preflight.on_the_wing()
    }
}

/// Every player's position, velocity and whether it is in the air (0 is you).
fn players(game: &Session, you: &FlightState) -> Vec<(Vec3, Vec3, bool)> {
    std::iter::once(you)
        .chain(game.opponents.iter().map(|o| &o.state))
        .map(|s| {
            (s.pos, s.vel, matches!(s.phase, Phase::Launching | Phase::Flying) && s.carry.is_none() && !s.on_ground)
        })
        .collect()
}

/// Who the plane that is it should chase: the one it can catch soonest, by the time it takes to
/// close the distance with the speed it has over it (a slow plane is easy), and longer if it has
/// to turn round first. It sticks with the one it is chasing (`now`) unless another is clearly
/// quicker to catch, or it would dither between them and catch nobody.
fn choose_target(tag: &Tag, me: usize, planes: &[(Vec3, Vec3, bool)], now: Option<usize>) -> Option<usize> {
    let (pos, vel, _) = planes[me];
    let ahead = Vec3::new(vel.x, 0.0, vel.z).normalize_or_zero();
    let time_to_catch = |t: usize| {
        let to = planes[t].0 - pos;
        let behind = if Vec3::new(to.x, 0.0, to.z).normalize_or_zero().dot(ahead) < 0.0 { 3.0 } else { 0.0 };
        to.length() / (TOP_SPEED - planes[t].1.length()).max(3.0) + behind
    };
    let can = |t: usize| t != me && planes[t].2 && tag.may_tag(t);
    let best = (0..planes.len()).filter(|&t| can(t)).min_by(|&a, &b| time_to_catch(a).total_cmp(&time_to_catch(b)))?;
    match now {
        Some(t) if can(t) && time_to_catch(t) < 1.5 * time_to_catch(best) => Some(t),
        _ => Some(best),
    }
}

/// Fly at a plane to touch it: aim where it will be, and match its height.
fn chase(
    pilot: &mut Pilot,
    s: &FlightState,
    (pos, vel, _): (Vec3, Vec3, bool),
    ground: &dyn Terrain,
    dt: f32,
) -> Controls {
    pilot.intercept(s, pos, vel, ground, dt)
}

/// Turn a point to fly to back towards the middle of the arena when the plane is near its edge
/// and going out, early enough for a turn.
fn keep_in(arena: &Arena, s: &FlightState, aim: Vec3) -> Vec3 {
    let from = Vec3::new(s.pos.x - arena.centre.x, 0.0, s.pos.z - arena.centre.z);
    let r = from.length() / arena.radius;
    let going_out = Vec3::new(s.vel.x, 0.0, s.vel.z).normalize_or_zero().dot(from.normalize_or_zero()) > -0.2;
    let w = if going_out { ((r - 0.35) / 0.35).clamp(0.0, 1.0) } else { 0.0 };
    let middle = Vec3::new(arena.centre.x, aim.y, arena.centre.z);
    aim.lerp(middle, w)
}

/// Get away from the plane that is it: break to the side, towards the middle of the arena,
/// and away in height.
fn evade(pilot: &mut Pilot, s: &FlightState, threat: Vec3, arena: &Arena, ground: &dyn Terrain, dt: f32) -> Controls {
    let away = Vec3::new(s.pos.x - threat.x, 0.0, s.pos.z - threat.z).normalize_or_zero();
    let to_centre = Vec3::new(arena.centre.x - s.pos.x, 0.0, arena.centre.z - s.pos.z);
    let side = Vec3::new(-away.z, 0.0, away.x);
    let side = if side.dot(to_centre) >= 0.0 { side } else { -side };
    let edge = (to_centre.length() / arena.radius - 0.6).clamp(0.0, 0.4) / 0.4;
    let dir = (away * 0.5 + side * 0.9 + to_centre.normalize_or_zero() * 2.0 * edge).normalize_or_zero();
    let ground_y = arena.centre.y;
    let height = if s.pos.y > threat.y { s.pos.y + 8.0 } else { s.pos.y - 8.0 };
    let aim = Vec3::new(s.pos.x, 0.0, s.pos.z) + dir * 60.0 + Vec3::Y * height.clamp(ground_y + 14.0, ground_y + 45.0);
    let throttle = pilot.skill.throttle;
    pilot.toward(s, keep_in(arena, s, aim), throttle, ground, dt)
}

/// Fly round the arena at an easy pace, each pilot on its own circle.
fn wander(
    pilot: &mut Pilot,
    s: &FlightState,
    arena: &Arena,
    t: f32,
    i: usize,
    ground: &dyn Terrain,
    dt: f32,
) -> Controls {
    let a = i as f32 * 2.1 + t * 0.12;
    let spot = arena.centre + Vec3::new(a.cos(), 0.0, a.sin()) * arena.radius * 0.45;
    let height = 18.0 + 6.0 * (t * 0.2 + i as f32).sin();
    pilot.toward(s, keep_in(arena, s, spot + Vec3::Y * height), 0.7, ground, dt)
}

/// Jobs a stand-in pilot can do with your plane: the acceptance tests fly with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Errand {
    /// Race round the outside of the pylons.
    Race,
    /// Turn inside the next pylon.
    Cut,
    /// Chase the nearest opponent that can be tagged.
    Tag,
    /// Fly slow, wide circles in the arena.
    Loiter,
    /// Fly at the plane that is it, to be tagged by it.
    Meet,
    /// Fly out of the arena.
    LeaveArena,
}

/// A computer pilot that flies your plane.
#[derive(Clone, Debug)]
pub struct StandIn {
    pilot: Pilot,
    /// The line it flies for an errand, and what it was made for.
    line: Option<(Errand, usize, Path)>,
}

impl Default for StandIn {
    fn default() -> Self {
        StandIn { pilot: Pilot::new(CLUB), line: None }
    }
}

impl StandIn {
    /// The controls for your plane, to do `errand` in `game`.
    pub fn fly(
        &mut self,
        game: &Session,
        you: &FlightState,
        errand: Errand,
        ground: &dyn Terrain,
        dt: f32,
    ) -> Controls {
        if you.phase == Phase::Held {
            return Controls { throttle: 1.0, ..Default::default() };
        }
        let radius = self.pilot.skill.turn_radius;
        match errand {
            Errand::Race | Errand::Cut => {
                // Cut the next pylon (after the line, that's pylon 1).
                let pylon = match game.race_ref().map(|r| r.racers[0].next) {
                    Some(Gate::Pylon(p)) if errand == Errand::Cut => p,
                    Some(Gate::Line) if errand == Errand::Cut => 0,
                    _ => usize::MAX,
                };
                if self.line.as_ref().is_none_or(|(e, p, _)| *e != errand || *p != pylon) {
                    let pts = if errand == Errand::Cut {
                        game.course.cutting_line(pylon, radius, 12.0, 1.0)
                    } else {
                        game.course.outside_line(radius, 1.0)
                    };
                    self.line = Some((errand, pylon, Path::closed(pts)));
                    self.pilot.lose_place();
                }
                let (_, _, line) = self.line.as_ref().unwrap();
                self.pilot.follow(you, line, RACE_HEIGHT, 1.0, ground, dt)
            }
            Errand::Tag => {
                let Some(tag) = game.tag_ref() else {
                    return self.pilot.toward(you, you.pos + you.vel, 0.7, ground, dt);
                };
                let planes = players(game, you);
                match choose_target(tag, 0, &planes, None) {
                    Some(t) => chase(&mut self.pilot, you, planes[t], ground, dt),
                    None => wander(&mut self.pilot, you, &game.arena, game.since_go(), 3, ground, dt),
                }
            }
            Errand::Meet => {
                let Some(tag) = game.tag_ref() else {
                    return self.pilot.toward(you, you.pos + you.vel, 0.7, ground, dt);
                };
                let planes = players(game, you);
                if tag.it == 0 {
                    return wander(&mut self.pilot, you, &game.arena, game.since_go(), 3, ground, dt);
                }
                // Right at it, at its height, at an easy speed so as not to shoot past.
                let (pos, vel, _) = planes[tag.it];
                self.pilot.intercept(you, pos, vel, ground, dt)
            }
            Errand::Loiter => {
                // Slow and steady round the middle of the arena: easy to catch.
                let c = game.arena.centre;
                let to = Vec3::new(you.pos.x - c.x, 0.0, you.pos.z - c.z);
                let a = to.z.atan2(to.x) - 0.6;
                let aim = c + Vec3::new(a.cos(), 0.0, a.sin()) * 45.0 + Vec3::Y * 18.0;
                self.pilot.toward(you, aim, 0.55, ground, dt)
            }
            Errand::LeaveArena => {
                let aim = game.arena.centre + game.field.forward() * (game.arena.radius + 60.0) + Vec3::Y * 25.0;
                self.pilot.toward(you, aim, 1.0, ground, dt)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_physics::Flat;

    const DT: f32 = 1.0 / 600.0;

    fn field() -> Field {
        Field { pilot: Vec3::ZERO, heading: 0.0 }
    }

    fn calm(_: Vec3, _: f64) -> Vec3 {
        Vec3::ZERO
    }

    /// Your plane: in your hand at the pilot box.
    fn your_plane() -> (AircraftParams, FlightState) {
        let p = t1s_ranger::params();
        let s = FlightState::new(&p, &Start { pos: Vec3::new(-0.5, 0.0, -0.6), heading: 0.0 });
        (p, s)
    }

    /// Runs the game for `seconds`, your plane flown by `fly` (or left in your hand), looking
    /// at every step. Stops early when `done` says so; returns the time it took.
    fn play(
        game: &mut Session,
        you: &mut FlightState,
        seconds: f32,
        mut fly: impl FnMut(&Session, &FlightState) -> Option<Controls>,
        mut done: impl FnMut(&Session, &FlightState) -> bool,
    ) -> f32 {
        let p = t1s_ranger::params();
        let world = Surroundings { ground: &Flat, obstacles: &[], wind: &calm };
        let mut t = 0.0;
        while t < seconds {
            if let Some(c) = fly(game, you) {
                if you.phase == Phase::Held && game.may_launch() {
                    you.launch();
                }
                physics::step(&p, you, &c.mix(), &Environment::default(), &Flat, &[], DT);
            }
            game.step(you, &world, DT);
            t += DT;
            if done(game, you) {
                break;
            }
        }
        t
    }

    fn held(_: &Session, _: &FlightState) -> Option<Controls> {
        None
    }

    #[test]
    fn a_game_starts_with_every_plane_in_hand_and_a_countdown() {
        for game in [Session::race(field()), Session::tag(field())] {
            assert_eq!(game.clock, Clock::CountingDown { left: COUNTDOWN });
            assert!(!game.may_launch());
            assert_eq!(game.opponents.len(), 3);
            assert!(game.opponents.iter().all(|o| o.state.phase == Phase::Held && o.state.carry.is_none()));
        }
        assert_eq!(Session::tag(field()).chaser(), Some(0), "you are it first");
    }

    #[test]
    fn at_go_the_computer_pilots_throw_one_each_second() {
        let mut game = Session::race(field());
        let (_, mut you) = your_plane();
        play(&mut game, &mut you, COUNTDOWN + 0.1, held, |_, _| false);
        assert!(matches!(game.clock, Clock::Running { .. }), "{:?}", game.clock);
        assert!(game.may_launch());
        let up = |g: &Session| g.opponents.iter().filter(|o| o.state.phase != Phase::Held).count();
        assert_eq!(up(&game), 1, "the first throws at GO");
        play(&mut game, &mut you, 1.0, held, |_, _| false);
        assert_eq!(up(&game), 2);
        play(&mut game, &mut you, 3.0, held, |_, _| false);
        assert!(game.opponents.iter().all(|o| o.state.phase == Phase::Flying && !o.state.on_ground));
    }

    #[test]
    fn the_computer_pilots_race_five_clean_laps_in_the_order_of_their_skill() {
        let mut game = Session::race(field());
        let (_, mut you) = your_plane();
        let mut lowest = f32::MAX;
        play(&mut game, &mut you, 300.0, held, |g, _| {
            for o in &g.opponents {
                if o.state.phase == Phase::Flying && g.time > COUNTDOWN + 6.0 {
                    lowest = lowest.min(o.state.pos.y);
                }
            }
            (1..4).all(|i| g.race_ref().unwrap().racers[i].status != Status::Racing)
        });
        let race = game.race_ref().unwrap();
        for i in 1..4 {
            let r = &race.racers[i];
            assert_eq!(r.cuts, 0, "{} cut a pylon", game.name(i));
            let Status::Finished { time, .. } = r.status else {
                panic!("{} didn't finish: {:?}, {}", game.name(i), r, game.opponents[i - 1].state.crash_reason)
            };
            // Five laps of about 600 m at 20 to 25 m/s.
            assert!((100.0..180.0).contains(&time), "{} took {time:.0} s", game.name(i));
        }
        let order: Vec<_> = race.standings().into_iter().filter(|&i| i != 0).collect();
        assert_eq!(order, vec![1, 2, 3], "the ace, the club pilot, the rookie");
        assert!(lowest > 4.0, "a computer pilot came down to {lowest:.1} m");
    }

    #[test]
    fn the_computer_pilots_race_in_a_gusty_breeze() {
        // 5 m/s from the north-east, with gusts, as the sim's wind can be.
        let breeze = |p: Vec3, t: f64| {
            let t = t as f32;
            Vec3::new(-3.5, 0.0, 3.5) * (1.0 + 0.3 * (t * 0.7 + p.x * 0.03).sin())
                + Vec3::new((t * 1.9).sin(), 0.3 * (t * 1.3).sin(), (t * 1.1 + p.z * 0.02).cos())
        };
        let p = t1s_ranger::params();
        let mut game = Session::race(field());
        let you = FlightState::new(&p, &Start { pos: Vec3::new(-0.5, 0.0, -0.6), heading: 0.0 });
        let world = Surroundings { ground: &Flat, obstacles: &[], wind: &breeze };
        for _ in 0..(300.0 / DT) as usize {
            game.step(&you, &world, DT);
            if (1..4).all(|i| game.race_ref().unwrap().racers[i].status != Status::Racing) {
                break;
            }
        }
        for i in 1..4 {
            let r = &game.race_ref().unwrap().racers[i];
            assert!(
                matches!(r.status, Status::Finished { .. }),
                "{}: {:?} {}",
                game.name(i),
                r.status,
                game.opponents[i - 1].state.crash_reason
            );
            assert_eq!(r.cuts, 0, "{} cut a pylon", game.name(i));
        }
    }

    #[test]
    fn a_crash_after_go_puts_you_out_of_the_race() {
        let mut game = Session::race(field());
        let (_, mut you) = your_plane();
        play(&mut game, &mut you, COUNTDOWN + 0.5, held, |_, _| false);
        you.phase = Phase::Flying;
        play(&mut game, &mut you, 0.1, held, |_, _| false);
        you.phase = Phase::Crashed;
        play(&mut game, &mut you, 0.1, held, |_, _| false);
        assert_eq!(game.race_ref().unwrap().racers[0].status, Status::Out);
        assert!(matches!(game.clock, Clock::Stopped { .. }));
    }

    #[test]
    fn picking_your_plane_up_puts_you_out_of_the_race() {
        let mut game = Session::race(field());
        let (_, mut you) = your_plane();
        play(&mut game, &mut you, COUNTDOWN + 0.5, held, |_, _| false);
        you.phase = Phase::Flying;
        play(&mut game, &mut you, 0.1, held, |_, _| false);
        you.phase = Phase::Held;
        play(&mut game, &mut you, 0.1, held, |_, _| false);
        assert_eq!(game.race_ref().unwrap().racers[0].status, Status::Out);
    }

    #[test]
    fn a_plane_still_in_hand_at_go_is_not_out() {
        let mut game = Session::race(field());
        let (_, mut you) = your_plane();
        play(&mut game, &mut you, COUNTDOWN + 10.0, held, |_, _| false);
        assert_eq!(game.race_ref().unwrap().racers[0].status, Status::Racing);
        assert!(matches!(game.clock, Clock::Running { .. }));
    }

    #[test]
    fn the_stand_in_flies_a_clean_lap() {
        let mut game = Session::race(field());
        let (_, mut you) = your_plane();
        let mut stand_in = StandIn::default();
        let t = play(
            &mut game,
            &mut you,
            COUNTDOWN + 60.0,
            |g, y| Some(stand_in.fly(g, y, Errand::Race, &Flat, DT)),
            |g, _| g.your_laps() == 1,
        );
        assert_eq!(game.your_laps(), 1, "{:?}", game.race_ref().unwrap().racers[0]);
        assert_eq!(game.your_cuts(), 0);
        assert!(t < COUNTDOWN + 45.0, "the lap took {:.0} s", t - COUNTDOWN);
    }

    #[test]
    fn the_stand_in_can_cut_a_pylon() {
        let mut game = Session::race(field());
        let (_, mut you) = your_plane();
        let mut stand_in = StandIn::default();
        play(
            &mut game,
            &mut you,
            COUNTDOWN + 60.0,
            |g, y| Some(stand_in.fly(g, y, Errand::Cut, &Flat, DT)),
            |g, _| g.your_cuts() == 1,
        );
        assert_eq!(game.your_cuts(), 1);
        // And again, at the next pylon: disqualified.
        play(
            &mut game,
            &mut you,
            60.0,
            |g, y| Some(stand_in.fly(g, y, Errand::Cut, &Flat, DT)),
            |g, _| g.your_cuts() == 2,
        );
        assert_eq!(game.race_ref().unwrap().racers[0].status, Status::Disqualified);
        assert!(matches!(game.clock, Clock::Stopped { .. }));
    }

    #[test]
    fn the_stand_in_tags_an_opponent() {
        let mut game = Session::tag(field());
        let (_, mut you) = your_plane();
        let mut stand_in = StandIn::default();
        let t = play(
            &mut game,
            &mut you,
            COUNTDOWN + 90.0,
            |g, y| Some(stand_in.fly(g, y, Errand::Tag, &Flat, DT)),
            |g, _| g.chaser() != Some(0),
        );
        assert_ne!(game.chaser(), Some(0), "no tag in {t:.0} s");
        assert_eq!(game.tag_ref().unwrap().players[0].tags, 1);
    }

    /// With several planes about, a chaser picks the one it can catch soonest, which is not always
    /// you. The stand-in can fly at the plane that is it, so that the test of being tagged does
    /// not depend on the chaser's choice: it is tagged within the game, whichever opponent is it.
    /// With `chase_you`, a computer chaser goes for you at once, so a test can be tagged without
    /// waiting for it to choose you.
    #[test]
    fn a_chaser_told_to_chase_you_tags_a_loitering_plane() {
        for chaser in 1..=3 {
            let mut game = Session::tag(field());
            game.chase_you = true;
            let (_, mut you) = your_plane();
            let mut stand_in = StandIn::default();
            game.tag_ref_mut().unwrap().it = chaser;
            let t = play(
                &mut game,
                &mut you,
                COUNTDOWN + 60.0,
                |g, y| Some(stand_in.fly(g, y, Errand::Loiter, &Flat, DT)),
                |g, _| g.chaser() == Some(0),
            );
            assert_eq!(game.chaser(), Some(0), "opponent {chaser} did not tag you in {t:.0} s");
        }
    }

    #[test]
    fn the_stand_in_can_meet_the_chaser_and_be_tagged() {
        for chaser in 1..=3 {
            let mut game = Session::tag(field());
            let (_, mut you) = your_plane();
            let mut stand_in = StandIn::default();
            game.tag_ref_mut().unwrap().it = chaser;
            let t = play(
                &mut game,
                &mut you,
                COUNTDOWN + 60.0,
                |g, y| Some(stand_in.fly(g, y, Errand::Meet, &Flat, DT)),
                |g, _| g.chaser() == Some(0),
            );
            assert_eq!(game.chaser(), Some(0), "opponent {chaser} did not tag the stand-in in {t:.0} s");
        }
    }

    #[test]
    fn a_computer_chaser_catches_a_slow_plane() {
        let mut game = Session::tag(field());
        let (_, mut you) = your_plane();
        let mut stand_in = StandIn::default();
        // Kestrel is it, and you fly slow circles.
        game.tag_ref_mut().unwrap().it = 1;
        let t = play(
            &mut game,
            &mut you,
            COUNTDOWN + 90.0,
            |g, y| Some(stand_in.fly(g, y, Errand::Loiter, &Flat, DT)),
            |g, _| g.chaser() == Some(0),
        );
        assert_eq!(game.chaser(), Some(0), "not caught in {t:.0} s; it is {:?}", game.chaser());
        assert_eq!(you.phase, Phase::Flying);
    }

    /// The sim's default wind: 3 m/s from 20°, a boundary layer, and 30 % turbulence (see
    /// `wind_at` in the sim).
    fn breeze(pos: Vec3, t: f64) -> Vec3 {
        let (speed, from, turbulence) = (3.0f32, 20f32.to_radians(), 0.3f32);
        let to = -Vec3::new(from.sin(), 0.0, -from.cos());
        let mean = to * speed * (pos.y.max(0.3) / 10.0).powf(0.16).min(1.4);
        let (t, s, amp) = (t as f32, pos * 0.02, turbulence * (0.6 + 0.25 * speed));
        mean + Vec3::new(
            (t * 0.71 + s.x * 1.3).sin() * 0.6 + (t * 1.93 + s.z * 2.1 + 1.0).sin() * 0.4,
            ((t * 1.17 + s.z * 1.7 + 2.0).sin() * 0.6 + (t * 2.71 + s.x * 2.3).sin() * 0.4) * 0.5,
            (t * 0.83 + s.z * 1.1 + 4.0).sin() * 0.6 + (t * 2.29 + s.x * 1.9 + 3.0).sin() * 0.4,
        ) * amp
    }

    #[test]
    fn every_computer_chaser_catches_a_slow_plane_in_a_breeze() {
        for chaser in 1..=3 {
            let p = t1s_ranger::params();
            let mut game = Session::tag(field());
            let (_, mut you) = your_plane();
            let mut stand_in = StandIn::default();
            game.tag_ref_mut().unwrap().it = chaser;
            let world = Surroundings { ground: &Flat, obstacles: &[], wind: &breeze };
            let mut t = 0.0;
            while t < COUNTDOWN + 120.0 && game.chaser() != Some(0) {
                let c = stand_in.fly(&game, &you, Errand::Loiter, &Flat, DT);
                if you.phase == Phase::Held && game.may_launch() {
                    you.launch();
                }
                let wind = breeze(you.pos, you.time);
                physics::step(&p, &mut you, &c.mix(), &Environment { wind }, &Flat, &[], DT);
                game.step(&you, &world, DT);
                t += DT;
            }
            assert_eq!(
                game.chaser(),
                Some(0),
                "{} didn't catch you in 2 minutes; it is {:?}",
                game.name(chaser),
                game.chaser().map(|i| game.name(i))
            );
        }
    }

    #[test]
    fn leaving_the_arena_makes_you_it() {
        let mut game = Session::tag(field());
        let (_, mut you) = your_plane();
        let mut stand_in = StandIn::default();
        game.tag_ref_mut().unwrap().it = 2;
        play(
            &mut game,
            &mut you,
            COUNTDOWN + 60.0,
            |g, y| Some(stand_in.fly(g, y, Errand::LeaveArena, &Flat, DT)),
            |g, _| g.chaser() == Some(0),
        );
        assert_eq!(game.chaser(), Some(0));
        assert!(!game.arena.contains(you.pos));
    }

    #[test]
    fn the_computer_pilots_play_tag_for_two_minutes() {
        let mut game = Session::tag(field());
        let (_, mut you) = your_plane();
        // You stay in your hand, so you can't be tagged; Merlin starts as it.
        game.tag_ref_mut().unwrap().it = 2;
        let mut crashes = 0;
        let mut was = [false; 3];
        play(&mut game, &mut you, COUNTDOWN + 125.0, held, |g, _| {
            for (i, o) in g.opponents.iter().enumerate() {
                let down = o.state.phase == Phase::Crashed;
                if down && !was[i] {
                    crashes += 1;
                }
                was[i] = down;
            }
            false
        });
        let tag = game.tag_ref().unwrap();
        assert!(tag.over());
        assert!(matches!(game.clock, Clock::Stopped { .. }));
        let tags: u32 = tag.players.iter().map(|p| p.tags).sum();
        assert!(tags >= 2, "only {tags} tags in two minutes: {:?}", tag.players);
        assert_eq!(crashes, 0, "computer pilots crashed");
        assert_eq!(tag.players[0].it_time, 0.0);
    }
    // With autopilots: the computer pilots fly T1-S VTOLs like yours, each through a radio.

    const FBWA: pilot::Fbwa = pilot::Fbwa { roll_limit: 50.0 * DEG, pitch_max: 25.0 * DEG, pitch_min: -25.0 * DEG };
    const DEG: f32 = std::f32::consts::PI / 180.0;

    fn vtol() -> AircraftParams {
        (simulation_aircraft::find("t1s-vtol").unwrap().params)()
    }

    /// Their planes where they were parked: on the ground at their places.
    fn parked(p: &AircraftParams) -> Vec<FlightState> {
        (0..PILOTS.len()).map(|i| FlightState::new(p, &field().hand(i))).collect()
    }

    /// A race with autopilots, and your VTOL on the ground at the pilot box.
    fn race_with_autopilots() -> (Session, FlightState) {
        let p = vtol();
        let game = Session::race(field()).with_autopilots(p.clone(), FBWA, parked(&p));
        let you = FlightState::new(&p, &Start { pos: Vec3::new(-0.5, 0.0, -0.6), heading: 0.0 });
        (game, you)
    }

    /// The host's side: your plane sits on the ground, each computer pilot's autopilot moves its
    /// plane (here: nothing, the motors idle) and the computer pilot answers on its radio.
    /// Returns the last radios.
    fn run_with_autopilots(game: &mut Session, you: &mut FlightState, seconds: f32) -> Vec<Radio> {
        let p = vtol();
        let world = Surroundings { ground: &Flat, obstacles: &[], wind: &calm };
        let mut radios = vec![];
        let mut t = 0.0;
        while t < seconds {
            radios.clear();
            for i in 0..game.opponents.len() {
                physics::step(
                    &p,
                    &mut game.opponents[i].state,
                    &Default::default(),
                    &Environment::default(),
                    &Flat,
                    &[],
                    DT,
                );
                radios.push(game.radio(i, &Flat, DT));
            }
            physics::step(&p, you, &Default::default(), &Environment::default(), &Flat, &[], DT);
            game.step(you, &world, DT);
            t += DT;
        }
        radios
    }

    fn slot(name: &str) -> usize {
        vtol().rc_modes.iter().position(|m| *m == name).unwrap()
    }

    /// With autopilots, a game starts from cold: every plane on the ground, the countdown
    /// running, and the computer pilots' arm switches off, throttles down, in QHOVER.
    #[test]
    fn with_autopilots_a_game_starts_from_cold() {
        let (mut game, mut you) = race_with_autopilots();
        assert!(matches!(game.clock, Clock::CountingDown { .. }), "no waiting for your takeoff: {:?}", game.clock);
        let radios = run_with_autopilots(&mut game, &mut you, 1.0);
        for r in &radios {
            assert!(!r.arm && r.sticks.throttle == 0.0 && r.mode == slot("QHOVER"), "{r:?}");
        }
        assert!(game.opponents.iter().all(|o| o.state.on_ground), "their planes stand on the ground");
    }

    /// The countdown waits until every plane is set down at its place.
    #[test]
    fn with_autopilots_the_countdown_waits_for_the_planes_to_be_set_down() {
        let p = vtol();
        let mut far = parked(&p);
        far[1].pos += Vec3::new(30.0, 0.0, 0.0);
        let mut game = Session::race(field()).with_autopilots(p.clone(), FBWA, far);
        assert!(game.opponents[1].state.carry.is_some(), "carried back from where it was");
        let mut you = FlightState::new(&p, &Start { pos: Vec3::new(-0.5, 0.0, -0.6), heading: 0.0 });
        run_with_autopilots(&mut game, &mut you, 2.0);
        assert_eq!(game.clock, Clock::CountingDown { left: COUNTDOWN }, "held while a plane is carried");
        run_with_autopilots(&mut game, &mut you, 30.0);
        assert!(matches!(game.clock, Clock::Running { .. }), "{:?}", game.clock);
    }

    /// At GO, each computer pilot puts its arm switch on, with the throttle all the way down as
    /// ArduPilot needs. While the props don't turn it isn't armed: it tries again.
    #[test]
    fn with_autopilots_at_go_each_computer_pilot_arms() {
        let (mut game, mut you) = race_with_autopilots();
        let radios = run_with_autopilots(&mut game, &mut you, COUNTDOWN + 0.5);
        for r in &radios {
            assert!(r.arm && r.sticks.throttle == 0.0 && r.mode == slot("QHOVER"), "{r:?}");
        }
        // Nothing has armed (the props have not turned): it takes the switch off, then tries again.
        let mut off = false;
        let mut again = false;
        for _ in 0..(8.0 / DT) as usize {
            let r = run_with_autopilots(&mut game, &mut you, DT)[0];
            off |= !r.arm;
            again |= off && r.arm;
        }
        assert!(off && again, "it tries to arm again");
    }

    /// Once its props turn it climbs straight up in QHOVER, and at height it pushes FBWA.
    #[test]
    fn with_autopilots_it_takes_off_in_qhover_then_flies_in_fbwa() {
        let (mut game, mut you) = race_with_autopilots();
        run_with_autopilots(&mut game, &mut you, COUNTDOWN + 0.5);
        let p = vtol();
        let lift: Vec<usize> = p.motors.iter().enumerate().filter(|(_, m)| m.lift).map(|(i, _)| i).collect();
        // Armed: the autopilot spins the props.
        for &m in &lift {
            game.opponents[0].state.spool[m] = 0.2;
        }
        let r = game.radio(0, &Flat, DT);
        assert!(r.arm && r.mode == slot("QHOVER") && r.sticks.throttle > 0.6, "climbs: {r:?}");
        // High enough: forward flight.
        game.opponents[0].state.pos.y = 25.0;
        game.opponents[0].state.on_ground = false;
        let r = game.radio(0, &Flat, DT);
        assert!(r.arm && r.mode == slot("FBWA"), "flies on the wing: {r:?}");
        assert!(!game.on_the_wing(0), "still in the transition");
        game.opponents[0].state.airspeed = 18.0;
        for _ in 0..(3.5 / DT) as usize {
            game.radio(0, &Flat, DT);
        }
        assert!(game.on_the_wing(0), "the transition is done");
    }

    /// The planes go back to the host at the end of the game.
    #[test]
    fn with_autopilots_the_planes_go_back_to_the_host() {
        let (game, _) = race_with_autopilots();
        assert_eq!(game.into_planes().len(), PILOTS.len());
    }
}

#[cfg(test)]
mod probe {
    use super::*;
    use simulation_physics::Flat;

    #[test]
    #[ignore]
    fn trace_tag() {
        let mut game = Session::tag(Field { pilot: Vec3::ZERO, heading: 0.0 });
        game.tag_ref_mut().unwrap().it = 2;
        let p = t1s_aircraft();
        let you = FlightState::new(&p, &Start { pos: Vec3::new(-0.5, 0.0, -0.6), heading: 0.0 });
        let calm = |_: Vec3, _: f64| Vec3::ZERO;
        let world = Surroundings { ground: &Flat, obstacles: &[], wind: &calm };
        let dt = 1.0 / 600.0;
        let mut it_was = 2;
        let mut closest = f32::MAX;
        for k in 0..(125.0 / dt) as usize {
            game.step(&you, &world, dt);
            let it = game.tag_ref().unwrap().it;
            if it != 0 {
                let o = &game.opponents[it - 1];
                if let Some((t, _)) = o.target
                    && t > 0
                {
                    let q = &game.opponents[t - 1].state;
                    let d = o.state.pos.distance(q.pos);
                    closest = closest.min(d);
                    if d < 10.0 && k % 30 == 0 {
                        let rel = q.pos - o.state.pos;
                        println!(
                            "  t {:.1} {}->{} d {d:.1} horiz {:.1} dy {:+.1} v {:.1}/{:.1} closing {:+.1} bank {:.0}/{:.0}",
                            game.time - COUNTDOWN,
                            o.name,
                            game.name(t),
                            rel.with_y(0.0).length(),
                            rel.y,
                            o.state.vel.length(),
                            q.vel.length(),
                            (o.state.vel - q.vel).dot(rel.normalize()),
                            physics::euler(o.state.rot).0.to_degrees(),
                            physics::euler(q.rot).0.to_degrees()
                        );
                    }
                }
            }
            if it != it_was {
                let q = &game.opponents[it - 1].state;
                println!(
                    "t {:.1}: {} is it (was {}), at r {:.0} y {:.0}; chaser got to {closest:.1} m",
                    game.time - COUNTDOWN,
                    game.name(it),
                    game.name(it_was),
                    (q.pos - game.arena.centre).with_y(0.0).length(),
                    q.pos.y
                );
                it_was = it;
                closest = f32::MAX;
            }
            let _ = k;
        }
        println!("end: closest {closest:.1}");
    }

    fn t1s_aircraft() -> AircraftParams {
        t1s_ranger::params()
    }
}
