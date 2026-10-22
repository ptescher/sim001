//! The fast arena (F0 in plans/sim2real-tag.md; see plans/league.md): a game of tag with no window
//! and no SITL. Seat 0 is the candidate: the scripted brain with its [`Tactics`] decides a plan
//! once a second, the executor with its [`Tuning`] flies it at 50 Hz, and [`aplite`] flies the
//! demand on the Ranger's physics. Seats 1 to 3 are the computer pilots of `recreation_games`,
//! which fly with their own hands: the same opponents whatever the candidate's code.
//!
//! Each game has a seed, which sets the wind, who is it first, and how late and lossy the tracks
//! of the others are. The same seed and candidate give the same game, to the bit.

pub mod aplite;

pub use cognition_agent::Tactics;
pub use cognition_executor::tuning::Tuning;

use aplite::ApLite;
use cognition_agent::{Brain, Craft, GameView, Picture, ScriptedBrain, world_to_ned, world_vel_to_ned};
use cognition_executor::{Demand, Executor, Fallback, Limits, MAX_TRACKS, State, Track};
use glam::Vec3;
use recreation_games::{Field, Session, Surroundings};
use recreation_pilot::{CLUB, Controls, Pilot};
use serde::{Deserialize, Serialize};
use simulation_aircraft::t1s_ranger;
use simulation_physics::{Environment, Flat, FlightState, Phase, Start, euler, step};
use std::collections::VecDeque;

/// The physics step (s), and the executor's (50 Hz, as AGENT mode).
const DT: f32 = 1.0 / 400.0;
const EXEC_EVERY: u32 = 8;
const EXEC_DT: f32 = DT * EXEC_EVERY as f32;
/// The ground decides once a second.
const DECIDE_EVERY: f32 = 1.0;
/// A virtual touch (m). At the sim's 1.5 m, an executor that keeps its 4 m of separation could
/// never tag anyone. At the plan's 15 m for real planes (which also gives each plane its own height
/// band), three planes in this arena pass "it" round a cluster hundreds of times a game; from
/// 10 m down they do not. (The executor's separation zone is 7 m, so its pursuit has a narrow
/// window to tag in: something for the league to find.)
pub const TOUCH: f32 = 8.0;
/// The executor's floor (m above the ground), and how far inside the arena its fence is (m).
const FLOOR: f32 = 12.0;
const FENCE_INSIDE: f32 = 10.0;
/// After the throw the hand climbs out for this long (s) before the executor flies.
const CLIMB_OUT: f32 = 2.5;
/// A crashed plane is back in the hand after this long (s), as the computer pilots' are.
const DOWN_TIME: f32 = 3.0;
/// The others' tracks are refreshed twice a second (the uplink's rate), when not lost.
const UPLINK: f32 = 0.5;
/// The most wind (m/s), track delay (s) and track loss a seed can give.
const WIND_MAX: f32 = 6.0;
const DELAY: (f32, f32) = (0.2, 1.5);
const LOSS_MAX: f32 = 0.3;
/// Where home is on the globe: the brain speaks in latitude and longitude.
const HOME: (f64, f64) = (-35.363261, 149.165230);
/// The placing bonus, from the least time as it to the most.
const PLACING: [f32; 4] = [1.0, 0.3, -0.3, -1.0];

/// Everything that decides how seat 0 plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub tuning: Tuning,
    pub tactics: Tactics,
}

impl Candidate {
    /// The executor as it is: its default tuning, with the tactics of the ace (Kestrel).
    pub fn default_executor() -> Candidate {
        Candidate { tuning: Tuning::DEFAULT, tactics: Tactics::of(&cognition_agent::scripted::ACE) }
    }

    /// A deliberately weak executor, for the tests of the league: its pursuit does not close on
    /// the target or lead it, and it evades slowly and not far.
    pub fn slow() -> Candidate {
        let tuning = Tuning {
            k_close: 0.05,
            close_max: 0.0,
            lead_max: 0.5,
            lead_min: 0.0,
            evade_speed: 0.0,
            evade_height: 5.0,
            ..Tuning::DEFAULT
        };
        Candidate { tuning, ..Candidate::default_executor() }
    }
}

/// What one game says about seat 0.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameResult {
    pub seed: u64,
    /// The reward of the plan for tag (see [`score`]).
    pub score: f32,
    /// Seconds from GO to the end.
    pub length: f32,
    pub players: usize,
    /// Seconds the executor flew the candidate's plane.
    pub by_plan: f32,
    pub it_time: f32,
    pub tags: u32,
    pub tagged: u32,
    /// Exits from the arena while the executor flew.
    pub exits: u32,
    /// Crashes while the executor flew (the hand's climb-out after a throw is the arena's, and
    /// counted as `launch_crashes`).
    pub crashes: u32,
    #[serde(default)]
    pub launch_crashes: u32,
    /// Exits from the arena in the hand's climb-out (not counted in `exits`).
    #[serde(default)]
    pub launch_exits: u32,
    /// 1 for the least time as it, to 4.
    pub place: u32,
    /// Executor steps, and those where the envelope changed the demand.
    pub steps: u32,
    pub interventions: u32,
    /// Steps where the executor refused its inputs.
    pub bad_inputs: u32,
    /// How many steps each envelope rule fired (separation, fence, ceiling, floor, stall, limits).
    pub rules: [u32; 6],
    /// The closest the candidate came to the plane it chased, while it (m).
    pub closest_chase: f32,
    /// Each crash: the game time (s from GO), the physics' reason, and the last demand and
    /// state (height, airspeed, roll, pitch) before it.
    #[serde(default)]
    pub crash_log: Vec<String>,
    /// Numbers of a game that is not tag, by name: the league judges every game by this record.
    #[serde(default)]
    pub extra: std::collections::BTreeMap<String, f32>,
}

/// The score of a game for a player: its time as it, its tags, the times it was tagged, its exits
/// from the arena and crashes, and its place (1 for the least time as it).
pub fn score(it_time: f32, tags: u32, tagged: u32, exits: u32, crashes: u32, place: u32) -> f32 {
    -it_time / 120.0 + 0.2 * tags as f32 - 0.2 * tagged as f32 - 0.3 * exits as f32 - crashes as f32
        + PLACING[(place.clamp(1, 4) - 1) as usize]
}

/// Where the candidate is in its flight.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// In the hand, until GO.
    Held,
    /// Thrown at this time: the hand climbs out.
    Thrown(f32),
    /// The executor flies.
    Agent,
    /// Crashed this long ago.
    Down(f32),
}

/// Each of the others' position, velocity and whether it flies (world frame).
type Others = [(Vec3, Vec3, bool); 3];

/// The others as the candidate's radios hear them: refreshed every `UPLINK` s unless lost, each
/// refresh `delay` s old.
struct Radios {
    delay: f32,
    loss: f32,
    /// (time, each other's position, velocity and whether it flies), oldest first.
    past: VecDeque<(f32, Others)>,
    tracks: [Track; MAX_TRACKS],
    next: f32,
}

impl Radios {
    fn new(delay: f32, loss: f32) -> Radios {
        Radios { delay, loss, past: VecDeque::new(), tracks: [Track::default(); MAX_TRACKS], next: 0.0 }
    }

    /// The others now (`now`, world frame), at time `t`; refreshes the tracks when it is time.
    fn hear(&mut self, t: f32, now: Others, rng: &mut fastrand::Rng) {
        self.past.push_back((t, now));
        while self.past.len() > 2 && self.past[1].0 <= t - self.delay {
            self.past.pop_front();
        }
        for tr in self.tracks.iter_mut().filter(|tr| tr.valid != 0) {
            tr.age += EXEC_DT;
        }
        if t < self.next {
            return;
        }
        self.next = t + UPLINK;
        let (when, then) = self.past[0];
        for (k, &(pos, vel, flying)) in then.iter().enumerate() {
            if rng.f32() < self.loss {
                continue;
            }
            let tr = &mut self.tracks[k];
            if !flying {
                tr.valid = 0;
                continue;
            }
            let p = world_to_ned(pos, Vec3::ZERO);
            let v = world_vel_to_ned(vel);
            *tr = Track { id: k as u8 + 2, valid: 1, pos: p.to_array(), vel: v.to_array(), age: t - when };
        }
    }
}

/// Plays one game.
pub fn play(candidate: &Candidate, seed: u64) -> GameResult {
    let mut rng = fastrand::Rng::with_seed(seed);
    let field = Field { pilot: Vec3::ZERO, heading: 0.0 };
    let mut session = Session::tag(field);
    {
        let tag = session.tag_ref_mut().expect("tag");
        tag.rules.touch = TOUCH;
        tag.it = rng.usize(0..4);
    }
    let arena = session.arena;
    let (wind_speed, wind_dir) = (rng.f32() * WIND_MAX, rng.f32() * std::f32::consts::TAU);
    // From the direction it blows towards (radians from north), in the world frame (+X east, -Z north).
    let wind = Vec3::new(wind_dir.sin(), 0.0, -wind_dir.cos()) * wind_speed;
    let mut radios = Radios::new(DELAY.0 + rng.f32() * (DELAY.1 - DELAY.0), rng.f32() * LOSS_MAX);

    let params = t1s_ranger::params();
    // The hand throws into the wind (a heading as `Field`'s: forward is (-sin, 0, -cos)).
    let into_wind = if wind_speed > 1.0 { wind.x.atan2(wind.z) } else { field.heading };
    let start = Start { pos: field.pilot, heading: into_wind };
    let mut me = FlightState::new(&params, &start);
    let mut stage = Stage::Held;
    let mut hand = Pilot::new(CLUB);
    let mut ap = ApLite::default();
    let mut exec = Executor::with_tuning(candidate.tuning);
    let mut brain = ScriptedBrain::with_tactics(1, &candidate.tactics);
    let centre = world_to_ned(arena.centre, Vec3::ZERO);
    let limits = Limits {
        fence_centre: [centre.x, centre.y],
        fence_radius: arena.radius - FENCE_INSIDE,
        ceiling: arena.ceiling,
        floor: FLOOR,
        ..Limits::default()
    };

    let mut r = GameResult { seed, players: 4, closest_chase: f32::MAX, ..GameResult::default() };
    let mut plan = None;
    let mut demand = Demand { throttle: -1.0, ..Demand::default() };
    let (mut t, mut next_decision, mut n) = (0.0f32, 0.0f32, 0u32);
    let mut inside = true;
    let mut last_it = session.tag_ref().unwrap().it;
    let mut last_tags: Vec<u32> = session.tag_ref().unwrap().players.iter().map(|p| p.tags).collect();
    let wind_fn = move |_: Vec3, _: f64| wind;
    let world = Surroundings { ground: &Flat, obstacles: &[], wind: &wind_fn };

    // `ARENA_TRACE=SEED`: the candidate's flight in that game, ten times a second, on stderr.
    let trace: Option<u64> = std::env::var("ARENA_TRACE").ok().and_then(|s| s.parse().ok());
    loop {
        t += DT;
        let flying = matches!(me.phase, Phase::Launching | Phase::Flying) && me.carry.is_none() && !me.on_ground;
        stage = match stage {
            Stage::Held if session.may_launch() && session.since_go() >= 0.0 && me.carry.is_none() => {
                me.launch();
                hand.lose_place();
                Stage::Thrown(t)
            }
            Stage::Thrown(at) if t - at > CLIMB_OUT && me.pos.y > 6.0 => {
                exec.reset();
                plan = None;
                next_decision = t;
                Stage::Agent
            }
            Stage::Down(d) if d > DOWN_TIME => {
                me.reset_to_hand(&params, &start);
                Stage::Held
            }
            Stage::Down(d) => Stage::Down(d + DT),
            s if me.phase == Phase::Crashed => {
                if !matches!(s, Stage::Down(_)) {
                    if matches!(s, Stage::Thrown(_)) {
                        r.launch_crashes += 1;
                    } else {
                        r.crashes += 1;
                    }
                    let (roll, pitch, _) = euler(me.rot);
                    r.crash_log.push(format!(
                        "{:.1} s: {} ({:?}; demand roll {:.0} pitch {:.0} thr {:.2} fallback {} rules {:#b}; height {:.1} m, {:.1} m/s, roll {:.0}, pitch {:.0})",
                        session.since_go(),
                        me.crash_reason,
                        s,
                        demand.roll_cd as f32 / 100.0,
                        demand.pitch_cd as f32 / 100.0,
                        demand.throttle,
                        demand.fallback,
                        exec.rules(),
                        me.pos.y,
                        me.airspeed,
                        roll.to_degrees(),
                        pitch.to_degrees()
                    ));
                }
                Stage::Down(0.0)
            }
            s => s,
        };

        let controls = match stage {
            Stage::Held | Stage::Down(_) => Controls { throttle: 0.0, ..Default::default() },
            Stage::Thrown(_) => hand.climb_out(&me, &Flat, DT),
            Stage::Agent => {
                if n % EXEC_EVERY == 0 {
                    let others = [0, 1, 2].map(|k| {
                        let o = &session.opponents[k].state;
                        (o.pos, o.vel, matches!(o.phase, Phase::Launching | Phase::Flying) && !o.on_ground)
                    });
                    radios.hear(t, others, &mut rng);
                    if t >= next_decision {
                        next_decision = t + DECIDE_EVERY;
                        let picture = picture(&session, &me, &radios.tracks, t);
                        plan = brain.decide(&picture).first().map(|o| o.plan.to_plan(HOME.0, HOME.1));
                    }
                    demand = exec.step(&state(&me, t), plan.as_ref(), &radios.tracks, &limits);
                    r.steps += 1;
                    r.interventions += demand.intervened as u32;
                    r.bad_inputs += (demand.fallback == Fallback::BadInput as u8) as u32;
                    for (bit, count) in r.rules.iter_mut().enumerate() {
                        *count += (exec.rules() >> bit & 1) as u32;
                    }
                }
                ap.fly(&me, &demand, DT)
            }
        };
        if trace == Some(seed) && n % 20 == 0 && stage == Stage::Agent {
            let (roll, pitch, _) = euler(me.rot);
            eprintln!(
                "{:6.1} h {:5.1} v {:4.1} roll {:5.0} pitch {:4.0} | d roll {:4.0} pitch {:3.0} thr {:.2} fb {} rules {:#08b} prim {:?} | ail {:+.2} elev {:+.2} cl {:.2} stalled {:?}",
                session.since_go(),
                me.pos.y,
                me.airspeed,
                roll.to_degrees(),
                pitch.to_degrees(),
                demand.roll_cd as f32 / 100.0,
                demand.pitch_cd as f32 / 100.0,
                demand.throttle,
                demand.fallback,
                exec.rules(),
                exec.flown(),
                controls.aileron,
                controls.elevator,
                me.wing_cl,
                me.stalled
            );
        }
        step(&params, &mut me, &controls.mix(), &Environment { wind }, &Flat, &[], DT);
        session.step(&me, &world, DT);
        n += 1;

        let tag = session.tag_ref().unwrap();
        // Exits: the candidate going from inside the arena to outside, in the air.
        let now_inside = !flying || arena.contains(me.pos);
        if inside && !now_inside {
            if matches!(stage, Stage::Thrown(_)) {
                r.launch_exits += 1;
            } else {
                r.exits += 1;
            }
        }
        inside = now_inside;
        // Tagged: it passes to the candidate, and the one that was it has one more tag.
        if tag.it != last_it && tag.it == 0 && tag.players[last_it].tags > last_tags[last_it] {
            r.tagged += 1;
        }
        last_it = tag.it;
        last_tags = tag.players.iter().map(|p| p.tags).collect();
        if tag.it == 0 && flying {
            for o in &session.opponents {
                if matches!(o.state.phase, Phase::Flying) && !o.state.on_ground {
                    r.closest_chase = r.closest_chase.min(o.state.pos.distance(me.pos));
                }
            }
        }
        if tag.over() {
            break;
        }
    }

    let tag = session.tag_ref().unwrap();
    r.length = tag.time;
    r.it_time = tag.players[0].it_time;
    r.tags = tag.players[0].tags;
    r.place = 1 + tag.players.iter().skip(1).filter(|p| p.it_time < r.it_time).count() as u32;
    r.by_plan = r.steps as f32 * EXEC_DT;
    r.score = score(r.it_time, r.tags, r.tagged, r.exits, r.crashes, r.place);
    r
}

/// The executor's state of the candidate's plane (local NED from home, the field's pilot box).
fn state(s: &FlightState, t: f32) -> State {
    let (roll, pitch, yaw) = euler(s.rot);
    State {
        pos: world_to_ned(s.pos, Vec3::ZERO).to_array(),
        vel: world_vel_to_ned(s.vel).to_array(),
        roll,
        pitch,
        yaw,
        airspeed: s.airspeed,
        agl: s.pos.y,
        dt: EXEC_DT,
        time: t,
    }
}

/// What the ground knows: the candidate's own plane (its telemetry) and the others' tracks.
/// Ids are seat + 1: the candidate is 1, the computer pilots 2 to 4.
fn picture(session: &Session, me: &FlightState, tracks: &[Track], t: f32) -> Picture {
    let tag = session.tag_ref().unwrap();
    let mut planes = vec![Craft {
        id: 1,
        pos: world_to_ned(me.pos, Vec3::ZERO),
        vel: world_vel_to_ned(me.vel),
        flying: matches!(me.phase, Phase::Flying | Phase::Launching) && !me.on_ground,
    }];
    for tr in tracks.iter().filter(|tr| tr.valid != 0) {
        let (p, v) = (Vec3::from(tr.pos), Vec3::from(tr.vel));
        planes.push(Craft { id: tr.id, pos: p + v * tr.age, vel: v, flying: true });
    }
    let no_tag_back = (1..tag.players.len()).find(|&w| w != tag.it && !tag.may_tag(w)).map_or(0, |w| w as u8 + 1);
    let centre = world_to_ned(tag.arena.centre, Vec3::ZERO);
    Picture {
        time: t,
        home: HOME,
        planes,
        game: GameView {
            it: tag.it as u8 + 1,
            time_left: tag.rules.length - tag.time,
            no_tag_back,
            centre: [centre.x, centre.y],
            radius: tag.arena.radius,
            ceiling: tag.arena.ceiling,
            floor: FLOOR,
        },
    }
}

/// Plays a game for each seed, on all the cores; the results are in the order of the seeds.
pub fn play_many(candidate: &Candidate, seeds: &[u64]) -> Vec<GameResult> {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut out = vec![GameResult::default(); seeds.len()];
    let chunk = seeds.len().div_ceil(cores).max(1);
    std::thread::scope(|scope| {
        for (seeds, out) in seeds.chunks(chunk).zip(out.chunks_mut(chunk)) {
            scope.spawn(move || {
                for (s, o) in seeds.iter().zip(out.iter_mut()) {
                    *o = play(candidate, *s);
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_score_is_the_plan_s_reward_for_tag() {
        // Never it, never tagged: first place.
        assert_eq!(score(0.0, 0, 0, 0, 0, 1), 1.0);
        // It for the whole game, last.
        assert_eq!(score(120.0, 0, 0, 0, 0, 4), -2.0);
        // Each tag, each time tagged, each exit, each crash.
        let base = score(30.0, 0, 0, 0, 0, 2);
        assert!((score(30.0, 1, 0, 0, 0, 2) - base - 0.2).abs() < 1e-6);
        assert!((score(30.0, 0, 1, 0, 0, 2) - base + 0.2).abs() < 1e-6);
        assert!((score(30.0, 0, 0, 1, 0, 2) - base + 0.3).abs() < 1e-6);
        assert!((score(30.0, 0, 0, 0, 1, 2) - base + 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_hand_throws_into_the_wind_and_its_crashes_are_not_the_candidate_s() {
        // This seed's wind made every throw to the north crash.
        let g = play(&Candidate::default_executor(), 2000179);
        assert_eq!(g.launch_crashes, 0, "{:?}", g.crash_log);
        assert!(g.by_plan > 90.0, "{}", g.by_plan);
    }

    #[test]
    fn leaving_the_arena_in_the_hand_s_climb_out_is_not_the_candidate_s_exit() {
        // This seed throws out of the arena: the pilot box is near its edge, and the wind sets
        // the way of the throw.
        let g = play(&Candidate::default_executor(), 1000000);
        assert_eq!((g.launch_exits, g.exits), (1, 0));
    }

    #[test]
    fn the_others_are_heard_late_and_lossy() {
        let mut rng = fastrand::Rng::with_seed(3);
        let mut radios = Radios::new(0.5, 0.0);
        // The first plane flies north at 10 m/s from the origin.
        let at = |t: f32| [(Vec3::new(0.0, 20.0, -10.0 * t), Vec3::new(0.0, 0.0, -10.0), true); 3];
        let mut t = 0.0;
        while t < 2.0 {
            radios.hear(t, at(t), &mut rng);
            t += EXEC_DT;
        }
        let tr = radios.tracks[0];
        assert_eq!((tr.id, tr.valid), (2, 1));
        // Heard about half a second late, and as old as that (the executor dead-reckons by age).
        let heard_north = tr.pos[0];
        assert!(tr.age >= 0.5 && tr.age < 0.5 + UPLINK + EXEC_DT, "age {}", tr.age);
        assert!((heard_north + 10.0 * tr.age - 10.0 * t).abs() < 0.5, "{heard_north} at {t}");
        // All lost: the tracks only grow old.
        let mut deaf = Radios::new(0.5, 1.0);
        for k in 0..100 {
            deaf.hear(k as f32 * EXEC_DT, at(0.0), &mut rng);
        }
        assert!(deaf.tracks.iter().all(|tr| tr.valid == 0));
    }
}
