//! Replay: a snapshot of the sim's domain state, and a tape of the inputs that move it on.
//!
//! The state at any tick is a snapshot from before it with the tape played on from there (see
//! plans/replay.md). That needs the sim to be deterministic: the same state and the same inputs
//! give the same next state, to the bit.

use crate::sim::{FlightMode, SimCore, Snapshot, Sticks, WindSettings};
use recreation_games::Kind;
use std::collections::VecDeque;

/// What the pilot does: the sticks as they are now, and the keys that change the sim.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Sticks(Sticks),
    /// Space.
    Throw,
    /// R.
    Reset,
    /// V.
    Recharge,
    /// G or T.
    Game(Kind),
    /// M.
    Mode(FlightMode),
    /// High rates (true) or low.
    Rates(bool),
    /// Differential thrust on or off.
    DiffThrust(bool),
    /// The wind, as the keys set it.
    Wind(WindSettings),
}

/// A run: the snapshot it started from, and each input at the step it came at. Each step is
/// one `step_standalone(dt)`, after that step's inputs.
#[derive(Clone, Debug)]
pub struct Tape {
    #[cfg_attr(not(test), expect(dead_code, reason = "a run reproduced from a file starts here (plans/replay.md)"))]
    start: Snapshot,
    dt: f32,
    inputs: Vec<(u64, Input)>,
    steps: u64,
}

impl Tape {
    /// A tape that starts from the sim as it is now, stepped `dt` at a time.
    pub fn start(core: &SimCore, dt: f32) -> Tape {
        Tape { start: core.snapshot(), dt, inputs: vec![], steps: 0 }
    }

    /// `input` came at step `step` (before that step ran).
    pub fn push(&mut self, step: u64, input: Input) {
        self.inputs.push((step, input));
        self.steps = self.steps.max(step + 1);
    }

    /// The run went on to `steps` steps.
    pub fn end(&mut self, steps: u64) {
        self.steps = self.steps.max(steps);
    }

    /// Put `core` where the run was after `to` steps: back to the start, and the tape played on.
    #[cfg_attr(not(test), expect(dead_code, reason = "a run reproduced from a file (plans/replay.md)"))]
    pub fn replay(&self, core: &mut SimCore, to: u64) {
        self.replay_from(core, &self.start.clone(), 0, to);
    }

    /// Put `core` where the run was after `to` steps, from `snap`, which the run was at after
    /// `from` steps.
    fn replay_from(&self, core: &mut SimCore, snap: &Snapshot, from: u64, to: u64) {
        // The tape has what a hook did: it sits the replay out, and goes on from where it is.
        let hook = core.hook.take();
        core.restore(snap);
        let first = self.inputs.partition_point(|(s, _)| *s < from);
        let mut next = self.inputs[first..].iter().peekable();
        for step in from..to.min(self.steps) {
            while let Some((_, input)) = next.next_if(|(s, _)| *s == step) {
                core.apply(input);
            }
            core.step_standalone(self.dt);
        }
        core.hook = hook;
    }

    /// Forget what came after `steps` steps (a rewind, and a new branch from there).
    fn cut(&mut self, steps: u64) {
        self.inputs.retain(|(s, _)| *s < steps);
        self.steps = steps;
    }
}

/// How often the recorder keeps a snapshot (s of sim time): a rewind replays at most this much.
const SNAPSHOT_EVERY: f32 = 5.0;
/// How many it keeps: ten minutes back.
const SNAPSHOTS: usize = 120;

/// Steps the sim and records its run as it goes: the tape, and a snapshot every few seconds so
/// that a rewind replays a few seconds at most.
pub struct Recorder {
    tape: Tape,
    step: u64,
    every: u64,
    ring: VecDeque<(u64, Snapshot)>,
    /// The sim's reset count when this flight started: a reset (R, or the tests' driver)
    /// starts a new one.
    resets: u16,
}

impl Recorder {
    pub fn new(core: &SimCore, dt: f32) -> Recorder {
        Recorder {
            tape: Tape::start(core, dt),
            step: 0,
            every: ((SNAPSHOT_EVERY / dt).round() as u64).max(1),
            ring: VecDeque::from([(0, core.snapshot())]),
            resets: core.resets,
        }
    }

    /// A new flight starts here, if the sim was reset since the last one did.
    fn new_flight_if_reset(&mut self, core: &SimCore) {
        if core.resets != self.resets {
            *self = Recorder::new(core, self.tape.dt);
        }
    }

    /// The pilot does `input` now.
    pub fn input(&mut self, core: &mut SimCore, input: Input) {
        self.new_flight_if_reset(core);
        core.apply(&input);
        if input == Input::Reset {
            self.new_flight_if_reset(core);
        } else {
            self.tape.push(self.step, input);
        }
    }

    /// One step of the sim.
    pub fn step(&mut self, core: &mut SimCore) {
        // A hook (the tests' stand-in pilot) moves the sticks at the start of a step: the tape
        // has them as inputs of that step, so a replay needs no hook.
        self.new_flight_if_reset(core);
        let before = core.sticks;
        core.step_standalone(self.tape.dt);
        if core.sticks != before {
            self.tape.push(self.step, Input::Sticks(core.sticks));
        }
        self.step += 1;
        self.tape.end(self.step);
        if self.step.is_multiple_of(self.every) {
            self.ring.push_back((self.step, core.snapshot()));
            if self.ring.len() > SNAPSHOTS {
                self.ring.pop_front();
            }
        }
    }

    #[cfg(test)]
    pub fn step_count(&self) -> u64 {
        self.step
    }

    #[cfg(test)]
    pub fn tape(&self) -> &Tape {
        &self.tape
    }

    /// Go back `secs` of sim time (no further than the oldest snapshot kept), and record a new
    /// branch from there. False if there is nothing to go back to.
    pub fn rewind(&mut self, core: &mut SimCore, secs: f32) -> bool {
        self.new_flight_if_reset(core);
        let back = (secs / self.tape.dt).round() as u64;
        let oldest = self.ring.front().map_or(0, |(s, _)| *s);
        let target = self.step.saturating_sub(back).max(oldest);
        let Some((from, snap)) = self.ring.iter().rev().find(|(s, _)| *s <= target).cloned() else {
            return false;
        };
        self.tape.replay_from(core, &snap, from, target);
        self.step = target;
        self.tape.cut(target);
        self.ring.retain(|(s, _)| *s <= target);
        true
    }
}

#[cfg(test)]
mod tests_support {
    use super::*;

    pub const DT: f32 = 1.0 / 240.0;

    pub fn sim() -> SimCore {
        SimCore::new((simulation_aircraft::find("t1s-ranger").unwrap().params)(), vec![], false, 9002)
    }

    /// What the pilot does at step `i` of a 30 s flight: a throw, turns both ways, a race.
    pub fn inputs(i: u64) -> Vec<Input> {
        let t = i as f32 * DT;
        let mut out = vec![Input::Sticks(Sticks {
            throttle: 0.8,
            roll: if t < 3.0 { 0.0 } else { (t * 0.7).sin() * 0.08 },
            pitch: 0.0,
            yaw: if t < 3.0 { 0.0 } else { (t * 0.7).sin() * 0.1 },
        })];
        if i == (12.0 / DT) as u64 {
            out.push(Input::Game(Kind::Race));
        }
        out
    }

    /// Everything a snapshot holds, as text: equal text is equal state, to the bit.
    pub fn fingerprint(c: &SimCore) -> String {
        format!("{:?} | {:?} | {:?} | {:?}", c.state, c.game, c.sticks, c.parked)
    }

    pub fn fly(c: &mut SimCore, from: u64, to: u64, tape: Option<&mut Tape>) {
        let mut tape = tape;
        for i in from..to {
            for input in inputs(i) {
                if let Some(t) = tape.as_deref_mut() {
                    t.push(i, input.clone());
                }
                c.apply(&input);
            }
            c.step_standalone(DT);
        }
    }

    pub const STEPS: u64 = (30.0 / DT) as u64;

    /// The same flight without the race (a game start is a new flight, as a reset is).
    pub fn flight(i: u64) -> Vec<Input> {
        inputs(i).into_iter().filter(|x| !matches!(x, Input::Game(_))).collect()
    }

    pub fn stick(i: Input) -> Sticks {
        match i {
            Input::Sticks(s) => s,
            _ => Sticks::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

    /// The foundation: the same inputs from the same state give the same state.
    #[test]
    fn the_same_inputs_give_the_same_state() {
        let (mut a, mut b) = (sim(), sim());
        fly(&mut a, 0, (11.0 / DT) as u64, None);
        assert!(
            a.state.phase == simulation_physics::Phase::Flying && a.state.pos.length() > 20.0,
            "it flies before the race: {:?} {:?} {}",
            a.state.pos,
            a.state.phase,
            a.state.crash_reason
        );
        fly(&mut a, (11.0 / DT) as u64, STEPS, None);
        fly(&mut b, 0, STEPS, None);
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    /// A snapshot taken mid-run, restored, and stepped with the same inputs ends where the run
    /// did.
    #[test]
    fn a_restored_snapshot_steps_the_same() {
        let mut c = sim();
        // Settings away from a fresh sim's, so that one the snapshot left out shows.
        c.high_rates = false;
        c.diff_thrust = false;
        c.wind.speed = 6.0;
        c.wind.turbulence = 0.6;
        let half = STEPS / 2;
        fly(&mut c, 0, half, None);
        let snap = c.snapshot();
        fly(&mut c, half, STEPS, None);
        let end = fingerprint(&c);

        // Into a fresh sim: a part the snapshot left out would have its starting value there.
        let mut other = sim();
        other.restore(&snap);
        fly(&mut other, half, STEPS, None);
        assert_eq!(fingerprint(&other), end);
        // And into the same sim, going back.
        c.restore(&snap);
        fly(&mut c, half, STEPS, None);
        assert_eq!(fingerprint(&c), end);
    }

    /// The same with the flight controller levelling the plane: its controller's memory (the
    /// integrators) is part of the state too.
    #[test]
    fn a_restored_snapshot_steps_the_same_in_angle_mode() {
        let vtol = || SimCore::new((simulation_aircraft::find("t1s-vtol").unwrap().params)(), vec![], false, 9002);
        let mut c = vtol();
        c.mode = crate::sim::FlightMode::Angle;
        let half = STEPS / 2;
        fly(&mut c, 0, half, None);
        let snap = c.snapshot();
        fly(&mut c, half, STEPS, None);
        let end = fingerprint(&c);

        let mut other = vtol();
        other.restore(&snap);
        fly(&mut other, half, STEPS, None);
        assert_eq!(fingerprint(&other), end);
    }

    /// A tape recorded during a run, replayed from its first snapshot on another sim, ends
    /// where the run did; and part of it rewinds to that moment.
    #[test]
    fn a_tape_replays_the_run() {
        let mut c = sim();
        let mut tape = Tape::start(&c, DT);
        fly(&mut c, 0, STEPS / 2, Some(&mut tape));
        let middle = fingerprint(&c);
        fly(&mut c, STEPS / 2, STEPS, Some(&mut tape));
        tape.end(STEPS);

        let mut other = sim();
        tape.replay(&mut other, STEPS);
        assert_eq!(fingerprint(&other), fingerprint(&c));
        tape.replay(&mut other, STEPS / 2);
        assert_eq!(fingerprint(&other), middle, "rewound to the middle");
    }
}

#[cfg(test)]
mod recorder_tests {
    use super::tests_support::*;
    use super::*;

    /// A rewind of 10 s puts the sim exactly where it was 10 s ago, from the nearest snapshot
    /// and the tape, whatever was changed in between (here the wind and the rates).
    #[test]
    fn a_rewind_goes_back_exactly() {
        let mut c = sim();
        let mut r = Recorder::new(&c, DT);
        let mut at_20 = String::new();
        for i in 0..STEPS {
            if i == (17.0 / DT) as u64 {
                r.input(&mut c, Input::Rates(false));
                let wind = WindSettings { speed: 7.0, ..c.wind };
                r.input(&mut c, Input::Wind(wind));
            }
            for input in flight(i) {
                r.input(&mut c, input);
            }
            r.step(&mut c);
            if r.step_count() == (20.0 / DT) as u64 {
                at_20 = fingerprint(&c);
            }
        }
        assert!(r.rewind(&mut c, 10.0), "there is a run to rewind");
        assert_eq!(r.step_count(), (20.0 / DT) as u64);
        assert_eq!(fingerprint(&c), at_20);
        assert!(!c.high_rates, "the rates as they were at 20 s");
    }

    /// After a rewind the pilot flies on: the tape is cut there, and what is new is recorded.
    #[test]
    fn a_rewind_branches() {
        let mut c = sim();
        let mut r = Recorder::new(&c, DT);
        for i in 0..STEPS {
            for input in flight(i) {
                r.input(&mut c, input);
            }
            r.step(&mut c);
        }
        let first = fingerprint(&c);
        r.rewind(&mut c, 10.0);
        for i in r.step_count()..STEPS {
            r.input(
                &mut c,
                Input::Sticks(Sticks { throttle: 0.0, ..flight(i).into_iter().next().map(stick).unwrap() }),
            );
            r.step(&mut c);
        }
        assert_ne!(fingerprint(&c), first, "another flight");
        let branched = fingerprint(&c);
        r.rewind(&mut c, 5.0);
        r.rewind(&mut c, 0.0);
        let mut again = sim();
        r.tape().replay(&mut again, r.step_count());
        assert_eq!(fingerprint(&again), fingerprint(&c), "the tape is the branch");
        let _ = branched;
    }

    /// A rewind further back than the run goes to its start.
    #[test]
    fn a_rewind_stops_at_the_start() {
        let mut c = sim();
        let start = fingerprint(&c);
        let mut r = Recorder::new(&c, DT);
        for i in 0..(3.0 / DT) as u64 {
            for input in flight(i) {
                r.input(&mut c, input);
            }
            r.step(&mut c);
        }
        r.rewind(&mut c, 60.0);
        assert_eq!(r.step_count(), 0);
        assert_eq!(fingerprint(&c), start);
    }
}

#[cfg(test)]
mod reset_tests {
    use super::tests_support::*;
    use super::*;

    /// A reset starts a new flight: a rewind goes back no further than it, whether R reset the
    /// plane or something else did (the tests' driver resets the sim itself).
    #[test]
    fn a_rewind_stops_at_a_reset() {
        for by_key in [true, false] {
            let mut c = sim();
            let mut r = Recorder::new(&c, DT);
            for i in 0..(20.0 / DT) as u64 {
                for input in flight(i) {
                    r.input(&mut c, input);
                }
                r.step(&mut c);
            }
            if by_key {
                r.input(&mut c, Input::Reset);
            } else {
                c.reset();
            }
            let after_reset = format!("{:?}", c.state.phase);
            for _ in 0..(3.0 / DT) as u64 {
                r.input(&mut c, Input::Sticks(Sticks { throttle: 0.8, ..Sticks::default() }));
                r.step(&mut c);
            }
            r.rewind(&mut c, 10.0);
            assert_eq!(format!("{:?}", c.state.phase), after_reset, "reset by key: {by_key}");
            assert_eq!(r.step_count(), 0, "the flight starts at the reset (by key: {by_key})");
        }
    }
}
