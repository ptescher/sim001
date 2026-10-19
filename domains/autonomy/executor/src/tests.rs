//! Unit tests for each behaviour, closed-loop tests against a small kinematic plane, and fuzzing.

use super::*;

// The plane model, shared with the acceptance tests.
#[path = "../tests/support/plane.rs"]
mod plane;

use plane::*;

fn cruise_state() -> State {
    State { pos: [0.0, 0.0, -30.0], vel: [18.0, 0.0, 0.0], airspeed: 18.0, agl: 30.0, dt: DT, ..State::default() }
}

fn plan(id: u16, primitive: Primitive) -> Plan {
    Plan { id, primitive: primitive as u8, ttl: 1000.0, ..Plan::default() }
}

/// Checks every demand against the limits.
fn assert_within(d: &Demand, l: &Limits) {
    assert!(d.throttle.is_finite());
    if d.fallback == Fallback::BadInput as u8 {
        assert_eq!((d.roll_cd, d.pitch_cd, d.throttle), (0, 0, -1.0));
        return;
    }
    let cd = |r: f32| r.to_degrees() * 100.0 + 1.0;
    assert!(d.roll_cd.abs() as f32 <= cd(l.bank_max.clamp(0.0, 1.4)), "{d:?}");
    assert!(d.pitch_cd as f32 <= cd(l.pitch_max.abs().min(0.8)), "{d:?}");
    assert!(-(d.pitch_cd as f32) <= cd(l.pitch_min.abs().min(0.8)), "{d:?}");
    assert!((0.0..=1.0).contains(&d.throttle), "{d:?}");
}

// ---- The C ABI ------------------------------------------------------------------------------

/// All the tests of the global executor are in this one, as tests run on several threads.
#[test]
fn the_c_abi_steps_and_resets_the_global_executor() {
    assert_eq!(agent_exec_abi_version(), 2);
    use core::ptr::{null, null_mut};
    assert_eq!(unsafe { agent_exec_step(null(), null(), null(), null(), null_mut()) }, -1);

    agent_exec_reset();
    let (s, l) = (cruise_state(), Limits::default());
    let p = Plan { radius: 50.0, point: [0.0, 300.0, -30.0], ..plan(7, Primitive::Orbit) };
    let tracks = [Track::default(); MAX_TRACKS];
    let mut d = Demand { roll_cd: 9, pitch_cd: 9, throttle: 1.0, ..Demand::default() };
    assert_eq!(unsafe { agent_exec_step(&s, &p, tracks.as_ptr(), &l, &mut d) }, 0);
    assert!(d.roll_cd > 0, "the orbit, off to the right, turns it right: {d:?}");
    // The plan is held with no plan passed, and tracks may be null.
    assert_eq!(unsafe { agent_exec_step(&s, null(), null(), &l, &mut d) }, 0);
    assert!(d.roll_cd > 0);
    agent_exec_reset();
    assert_eq!(unsafe { agent_exec_step(&s, null(), null(), &l, &mut d) }, 0);
    assert_eq!(d.roll_cd, 0, "reset forgets the plan: wings level");
    agent_exec_reset();
}

// ---- Unit tests -----------------------------------------------------------------------------

#[test]
fn with_no_plan_it_flies_wings_level_holding_height_at_cruise() {
    let mut x = Executor::new();
    let d = x.step(&cruise_state(), None, &[], &Limits::default());
    assert_eq!((d.roll_cd, d.fallback, d.intervened), (0, 0, 0));
    assert!((0.0..=1.0).contains(&d.throttle));
    assert_eq!(x.flown(), Primitive::None);
    // Below the height held: nose up.
    let low = State { pos: [0.0, 0.0, -20.0], agl: 20.0, ..cruise_state() };
    let d = x.step(&low, None, &[], &Limits::default());
    assert!(d.pitch_cd > 0, "{d:?}");
}

#[test]
fn bad_inputs_give_wings_level_and_the_throttle_to_the_pilot() {
    let l = Limits::default();
    let s = cruise_state();
    let bad_states = [
        State { pos: [f32::NAN, 0.0, -30.0], ..s },
        State { vel: [f32::INFINITY, 0.0, 0.0], ..s },
        State { airspeed: -1.0, ..s },
        State { airspeed: f32::NAN, ..s },
        State { dt: 0.0, ..s },
        State { dt: -0.02, ..s },
        State { dt: 0.6, ..s },
        State { agl: f32::NAN, ..s },
        State { yaw: f32::NEG_INFINITY, ..s },
        State { time: f32::NAN, ..s },
        State { roll: 1.0e9, ..s },
    ];
    for b in bad_states {
        let mut x = Executor::new();
        let d = x.step(&b, Some(&plan(1, Primitive::Goto)), &[], &l);
        assert_eq!((d.roll_cd, d.pitch_cd, d.throttle), (0, 0, -1.0), "{b:?}");
        assert_eq!((d.fallback, d.intervened), (Fallback::BadInput as u8, 1), "{b:?}");
    }
    let mut x = Executor::new();
    let d = x.step(&s, None, &[], &Limits { bank_max: f32::NAN, ..l });
    assert_eq!(d.fallback, Fallback::BadInput as u8);
    // A bad track is only ignored.
    let t = Track { pos: [f32::NAN, 0.0, 0.0], ..track(3, [10.0, 0.0, -30.0], [0.0; 3]) };
    let d = x.step(&s, Some(&Plan { target: 3, ..plan(1, Primitive::Pursue) }), &[t], &l);
    assert_eq!(d.fallback, Fallback::PlanExpired as u8, "its target unusable, it loiters");
}

#[test]
fn a_new_id_is_adopted_a_repeated_one_ignored_a_bad_plan_refused() {
    let (s, l) = (cruise_state(), Limits::default());
    let mut x = Executor::new();
    let p1 = plan(1, Primitive::Goto);
    x.step(&s, Some(&p1), &[], &l);
    assert_eq!(x.plan().map(|p| p.id), Some(1));
    assert_eq!(x.since_adopted(), 0.0);
    for _ in 0..10 {
        x.step(&s, Some(&Plan { primitive: Primitive::Orbit as u8, ..p1 }), &[], &l);
    }
    assert_eq!(x.plan().unwrap().primitive, Primitive::Goto as u8, "the same id again is ignored");
    assert!((x.since_adopted() - 10.0 * DT).abs() < 1.0e-5);
    x.step(&s, None, &[], &l);
    assert_eq!(x.plan().map(|p| p.id), Some(1), "no plan passed: the held one stays");

    x.step(&s, Some(&Plan { primitive: 9, ..plan(2, Primitive::Goto) }), &[], &l);
    x.step(&s, Some(&Plan { ttl: f32::NAN, ..plan(3, Primitive::Goto) }), &[], &l);
    x.step(&s, Some(&Plan { point: [f32::INFINITY, 0.0, 0.0], ..plan(4, Primitive::Goto) }), &[], &l);
    x.step(&s, Some(&Plan { ttl: -1.0, ..plan(5, Primitive::Goto) }), &[], &l);
    assert_eq!(x.plan().map(|p| p.id), Some(1), "bad plans are refused");

    x.step(&s, Some(&plan(0, Primitive::Loiter)), &[], &l);
    assert_eq!((x.plan().map(|p| p.id), x.since_adopted()), (Some(0), 0.0), "any other id, even lower");
}

#[test]
fn a_plan_expires_after_its_ttl_and_the_grace_into_a_loiter() {
    let s = cruise_state();
    let l = Limits { grace: 0.5, ..Limits::default() };
    let mut x = Executor::new();
    let p = Plan { ttl: 1.0, point: [500.0, 0.0, -30.0], ..plan(1, Primitive::Goto) };
    let mut d = x.step(&s, Some(&p), &[], &l);
    for _ in 0..70 {
        d = x.step(&s, Some(&p), &[], &l); // the same plan, repeated, does not renew it
    }
    assert_eq!(d.fallback, 0, "1.4 s: still good");
    for _ in 0..10 {
        d = x.step(&s, None, &[], &l);
    }
    assert_eq!(d.fallback, Fallback::PlanExpired as u8, "1.6 s: expired");
    assert_eq!(x.flown(), Primitive::Loiter);
    let d = x.step(&s, Some(&Plan { id: 2, ..p }), &[], &l);
    assert_eq!((d.fallback, x.flown()), (0, Primitive::Goto), "a new plan ends it");
}

#[test]
fn a_missing_or_stale_target_falls_back_to_a_loiter() {
    let (s, l) = (cruise_state(), Limits::default());
    let p = Plan { target: 5, ..plan(1, Primitive::Pursue) };
    let t = track(5, [60.0, 20.0, -30.0], [15.0, 0.0, 0.0]);
    for (tracks, fallen) in [
        (vec![], true),
        (vec![track(4, [60.0, 20.0, -30.0], [0.0; 3])], true),
        (vec![Track { age: 3.5, ..t }], true),
        (vec![Track { valid: 0, ..t }], true),
        (vec![Track { age: 1.0, ..t }], false),
        (vec![t], false),
    ] {
        let mut x = Executor::new();
        let d = x.step(&s, Some(&p), &tracks, &l);
        let want = if fallen { Fallback::PlanExpired as u8 } else { 0 };
        assert_eq!(d.fallback, want, "{tracks:?}");
        assert_eq!(x.flown(), if fallen { Primitive::Loiter } else { Primitive::Pursue });
    }
}

#[test]
fn bank_and_pitch_stay_inside_their_limits() {
    let l = Limits { bank_max: 0.2, pitch_max: 0.1, pitch_min: -0.05, ..Limits::default() };
    let mut x = Executor::new();
    // A point behind and far below: everything asks for more than the limits.
    let p = Plan { point: [-300.0, -10.0, -200.0], ..plan(1, Primitive::Goto) };
    let d = x.step(&cruise_state(), Some(&p), &[], &l);
    assert!(d.roll_cd.abs() <= 1146, "{d:?}");
    assert!(d.pitch_cd <= 573 && d.pitch_cd >= -287, "{d:?}");
    assert_eq!(d.intervened, 0, "the primitive keeps within the limits by itself");
    let d = x.step(&State { pos: [0.0, 0.0, -50.0], agl: 50.0, ..cruise_state() }, None, &[], &l);
    assert!(d.pitch_cd >= -287, "{d:?}");
}

#[test]
fn near_the_fence_heading_out_it_turns_back_towards_the_centre() {
    let l = Limits { fence_radius: 90.0, ..Limits::default() };
    // Wings level (no plan), so that the turn is the envelope's.
    let mut x = Executor::new();
    // Heading north at 60 m north of the centre, the centre is behind: the turn back starts now.
    let s = State { pos: [60.0, 0.0, -30.0], ..cruise_state() };
    let d = x.step(&s, None, &[], &l);
    assert_eq!(d.intervened, 1);
    assert!(rad(d.roll_cd.abs()) > 0.85, "{d:?}");
    // Heading east 84 m north of the centre (outward a little): the centre is to the right.
    let s = State { pos: [84.0, 0.0, -30.0], vel: [3.0, 18.0, 0.0], yaw: 1.4, ..cruise_state() };
    let mut x = Executor::new();
    let d = x.step(&s, None, &[], &l);
    assert!(d.roll_cd > 4800 && d.intervened == 1, "{d:?}");
    // And when the primitive turns the wrong way, out: pursuing a target outside.
    let p = Plan { target: 5, ..plan(1, Primitive::Pursue) };
    let t = [track(5, [130.0, 40.0, -30.0], [0.0, 15.0, 0.0])];
    let mut x = Executor::new();
    let d = x.step(&s, Some(&p), &t, &Limits { fence_radius: 0.0, ..l });
    assert!(d.roll_cd < 0, "unfenced, the pursuit turns left: {d:?}");
    let mut x = Executor::new();
    let d = x.step(&s, Some(&p), &t, &l);
    assert!(d.roll_cd > 4800 && d.intervened == 1, "{d:?}");
    // Heading in, near the edge: nothing happens.
    let s = State { pos: [80.0, 0.0, -30.0], vel: [-18.0, 0.0, 0.0], yaw: PI, ..cruise_state() };
    let mut x = Executor::new();
    assert_eq!(x.step(&s, None, &[], &l).intervened, 0);
    // In the middle, nothing happens.
    let mut x = Executor::new();
    let d = x.step(&cruise_state(), None, &[], &l);
    assert_eq!(d.intervened, 0);
    // No fence: nothing happens anywhere.
    let mut x = Executor::new();
    let s = State { pos: [500.0, 0.0, -30.0], ..cruise_state() };
    let d = x.step(&s, None, &[], &Limits::default());
    assert_eq!(d.intervened, 0);
}

#[test]
fn near_the_ceiling_the_nose_comes_down_and_near_the_floor_it_goes_up() {
    let l = Limits::default(); // ceiling 60, floor 12
    let p = Plan { point: [400.0, 0.0, -200.0], ..plan(1, Primitive::Goto) };
    let mut x = Executor::new();
    let s = State { pos: [0.0, 0.0, -62.0], vel: [18.0, 0.0, -3.0], agl: 62.0, ..cruise_state() };
    let d = x.step(&s, Some(&p), &[], &l);
    assert!(d.pitch_cd < 0, "{d:?}");
    // Above the ceiling, the wings come nearer level.
    let s = State { pos: [0.0, 0.0, -62.0], vel: [0.0, 18.0, 0.0], ..s };
    let d = x.step(&s, Some(&Plan { id: 2, point: [0.0, -400.0, -200.0], ..p }), &[], &l);
    assert!(rad(d.roll_cd.abs()) <= 0.5 * l.bank_max + 1.0e-3, "{d:?}");

    let p = Plan { point: [400.0, 0.0, 0.0], ..plan(3, Primitive::Goto) };
    let mut x = Executor::new();
    let s = State { pos: [0.0, 0.0, -10.0], vel: [18.0, 0.0, 2.0], agl: 10.0, ..cruise_state() };
    let d = x.step(&s, Some(&p), &[], &l);
    assert!(d.pitch_cd > 1500 && d.throttle > 0.9, "{d:?}");
    assert_eq!((d.fallback, d.intervened), (Fallback::Recovery as u8, 1));
    // Floor and ceiling are against the ground and home: agl is what the floor uses.
    let s = State { pos: [0.0, 0.0, -30.0], vel: [18.0, 0.0, 0.0], agl: 8.0, ..cruise_state() };
    let d = x.step(&s, Some(&p), &[], &l);
    assert!(d.pitch_cd > 0 && d.fallback == Fallback::Recovery as u8, "{d:?}");
}

#[test]
fn under_the_stall_margin_the_nose_drops_the_wings_level_and_the_throttle_opens() {
    let l = Limits::default();
    let p = Plan { point: [-100.0, 300.0, -60.0], ..plan(1, Primitive::Goto) };
    let mut x = Executor::new();
    let s = State { airspeed: 11.0, ..cruise_state() };
    let d = x.step(&s, Some(&p), &[], &l);
    assert!(d.pitch_cd <= -400, "{d:?}");
    assert!(rad(d.roll_cd.abs()) <= STALL_BANK + 1.0e-3, "{d:?}");
    assert_eq!((d.throttle, d.fallback, d.intervened), (1.0, Fallback::Recovery as u8, 1));
    // Just above the minimum: no climbing, more throttle, but no recovery.
    let s = State { airspeed: 13.5, ..cruise_state() };
    let d = x.step(&s, Some(&p), &[], &l);
    assert!(rad(d.pitch_cd) <= 0.25 * l.pitch_max + 1.0e-3 && d.throttle >= 0.8, "{d:?}");
    assert_eq!(d.fallback, 0);
}

#[test]
fn a_track_inside_the_separation_zone_is_escaped_from() {
    let l = Limits { separation: 10.0, ..Limits::default() };
    let s = cruise_state();
    let p = Plan { point: [400.0, 0.0, -30.0], ..plan(1, Primitive::Goto) };
    // Ahead and a little to the right, 2 m above: an escape (full bank, or a dive), never a climb.
    let t = track(9, [12.0, 3.0, -32.0], [0.0, 0.0, 0.0]);
    let mut x = Executor::new();
    let d = x.step(&s, Some(&p), &[t], &l);
    assert_eq!((d.intervened, x.rules() & rules::SEPARATION), (1, rules::SEPARATION));
    assert!(d.pitch_cd <= 0 && (d.roll_cd.abs() > 4900 || d.pitch_cd < -1000), "{d:?}");
    // A little below: never a dive.
    let t = track(9, [12.0, -3.0, -27.0], [0.0, 0.0, 0.0]);
    let mut x = Executor::new();
    let d = x.step(&s, Some(&p), &[t], &l);
    assert_eq!(d.intervened, 1);
    assert!(d.pitch_cd >= 0 && (d.roll_cd.abs() > 4900 || d.pitch_cd > 1000), "{d:?}");
    // Head on, 60 m apart closing at 36 m/s: in conflict within the horizon.
    let t = track(9, [60.0, 2.0, -30.0], [-18.0, 0.0, 0.0]);
    let mut x = Executor::new();
    assert_eq!(x.step(&s, Some(&p), &[t], &l).intervened, 1);
    // Well above, or far away: nothing.
    for t in [track(9, [12.0, 3.0, -60.0], [0.0; 3]), track(9, [200.0, 0.0, -30.0], [0.0; 3])] {
        let mut x = Executor::new();
        assert_eq!(x.step(&s, Some(&p), &[t], &l).intervened, 0, "{t:?}");
    }
    // Separation off (0): nothing, however close.
    let mut x = Executor::new();
    let t = track(9, [2.0, 0.0, -30.0], [18.0, 0.0, 0.0]);
    assert_eq!(x.step(&s, Some(&p), &[t], &Limits { separation: 0.0, ..l }).intervened, 0);
}

#[test]
fn cruising_on_a_plan_the_envelope_leaves_the_demand_alone() {
    let mut x = Executor::new();
    let p = Plan { point: [0.0, 0.0, -30.0], radius: 60.0, ..plan(1, Primitive::Orbit) };
    let s = State { pos: [60.0, 0.0, -30.0], vel: [0.0, 18.0, 0.0], yaw: HALF_PI, ..cruise_state() };
    let d = x.step(&s, Some(&p), &[], &Limits { fence_radius: 200.0, ..Limits::default() });
    assert_eq!((d.intervened, d.fallback), (0, 0));
    assert!(d.roll_cd > 1000, "the circle's turn, clockwise: {d:?}");
}

// ---- Closed loop ----------------------------------------------------------------------------

#[test]
fn goto_reaches_the_point_and_stays_by_it() {
    let l = Limits::default();
    let mut x = Executor::new();
    let mut a = Plane::new(0.0, 0.0, 30.0, 0.0, 18.0);
    let p = Plan { point: [150.0, -250.0, -40.0], ..plan(1, Primitive::Goto) };
    let target = [150.0, -250.0];
    let mut closest = f32::MAX;
    let mut far_after = 0.0f32;
    let mut reached_at = None;
    for i in 0..(200.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        assert_within(&d, &l);
        a.fly_demand(&d);
        closest = closest.min(a.dist2(target));
        if closest < GOTO_OVER && reached_at.is_none() {
            reached_at = Some(i);
        }
        if reached_at.is_some_and(|r| i > r + 500) {
            far_after = far_after.max(a.dist2(target));
        }
    }
    assert!(closest < GOTO_OVER, "closest {closest}");
    assert!(far_after < 1.6 * l_loiter(&l), "then circles it: {far_after}");
    assert!((a.h() - 40.0).abs() < 2.0, "at its height: {}", a.h());
}

fn l_loiter(l: &Limits) -> f32 {
    Lims::new(l).unwrap().loiter_radius()
}

#[test]
fn orbit_holds_its_radius_and_height() {
    let l = Limits::default();
    let mut x = Executor::new();
    let mut a = Plane::new(-120.0, 50.0, 30.0, 1.57, 18.0);
    let p = Plan { point: [0.0, 0.0, -45.0], radius: 60.0, ..plan(1, Primitive::Orbit) };
    let (mut lo, mut hi, mut hmax) = (f32::MAX, 0.0f32, 0.0f32);
    for i in 0..(180.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        assert_within(&d, &l);
        a.fly_demand(&d);
        if i as f32 * DT > 60.0 {
            let r = a.dist2([0.0, 0.0]);
            lo = lo.min(r);
            hi = hi.max(r);
            hmax = hmax.max((a.h() - 45.0).abs());
        }
    }
    assert!(lo > 56.0 && hi < 64.0, "radius {lo}..{hi}");
    assert!(hmax < 2.0, "height error {hmax}");
    // With a wind, too.
    let mut x = Executor::new();
    let mut a = Plane::new(-120.0, 50.0, 30.0, 1.57, 18.0);
    a.wind = [3.0, -2.0];
    let (mut lo, mut hi) = (f32::MAX, 0.0f32);
    for i in 0..(180.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        a.fly_demand(&d);
        if i as f32 * DT > 60.0 {
            lo = lo.min(a.dist2([0.0, 0.0]));
            hi = hi.max(a.dist2([0.0, 0.0]));
        }
    }
    assert!(lo > 52.0 && hi < 68.0, "radius in wind {lo}..{hi}");
}

#[test]
fn an_orbit_too_tight_to_fly_is_widened() {
    let l = Limits::default();
    let mut x = Executor::new();
    let mut a = Plane::new(-100.0, 0.0, 30.0, 0.0, 18.0);
    let p = Plan { point: [0.0, 0.0, -30.0], radius: 5.0, ..plan(1, Primitive::Orbit) };
    let mut spread = (f32::MAX, 0.0f32);
    for i in 0..(120.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        a.fly_demand(&d);
        if i as f32 * DT > 60.0 {
            spread = (spread.0.min(a.dist2([0.0, 0.0])), spread.1.max(a.dist2([0.0, 0.0])));
        }
    }
    assert!(spread.1 - spread.0 < 8.0, "a steady circle: {spread:?}");
}

/// Pursues a target circling at 16 m/s; returns the distances over time.
fn chase(l: &Limits, seconds: f32) -> Vec<f32> {
    let mut x = Executor::new();
    let mut target = circling_target(0.0, -60.0, 35.0, 16.0, (16.0f32 * 16.0 / (G * 60.0)).atan());
    let mut a = Plane::new(-80.0, -80.0, 30.0, 0.0, 18.0);
    let p = Plan { target: 2, ..plan(1, Primitive::Pursue) };
    let mut out = vec![];
    for _ in 0..(seconds / DT) as usize {
        let t = target();
        let d = x.step(&a.state(), Some(&p), &[t.as_track(2)], l);
        assert_within(&d, l);
        a.fly_demand(&d);
        out.push(dist3(a.pos, t.pos));
    }
    out
}

#[test]
fn pursue_closes_on_a_turning_target_and_stays_on_its_tail() {
    let l = Limits { separation: 0.0, ..Limits::default() };
    let d = chase(&l, 120.0);
    let start = d[0];
    let late = &d[(60.0 / DT) as usize..];
    let worst = late.iter().cloned().fold(0.0, f32::max);
    let best = late.iter().cloned().fold(f32::MAX, f32::min);
    assert!(start > 50.0);
    assert!(worst < 5.0, "from a minute on, within {worst} m (closest {best})");
}

#[test]
fn pursue_with_separation_on_never_comes_inside_it() {
    let l = Limits { separation: 8.0, ..Limits::default() };
    let d = chase(&l, 120.0);
    let closest = d.iter().cloned().fold(f32::MAX, f32::min);
    let late = &d[(60.0 / DT) as usize..];
    let worst = late.iter().cloned().fold(0.0, f32::max);
    assert!(closest > 8.0, "closest {closest}");
    assert!(worst < 40.0, "but it still follows: {worst}");
}

#[test]
fn pursue_works_from_tracks_heard_twice_a_second() {
    // As ADS-B or a ground uplink gives them: a new position every half second, aging between.
    let l = Limits { separation: 0.0, ..Limits::default() };
    let mut x = Executor::new();
    let mut target = circling_target(0.0, -60.0, 35.0, 16.0, (16.0f32 * 16.0 / (G * 60.0)).atan());
    let mut a = Plane::new(-80.0, -80.0, 30.0, 0.0, 18.0);
    let p = Plan { target: 2, ..plan(1, Primitive::Pursue) };
    let mut heard = target().as_track(2);
    let mut worst_late = 0.0f32;
    for i in 0..(120.0 / DT) as usize {
        let t = target();
        if i % 25 == 0 {
            heard = t.as_track(2);
        } else {
            heard.age += DT;
        }
        let d = x.step(&a.state(), Some(&p), &[heard], &l);
        assert_within(&d, &l);
        a.fly_demand(&d);
        if i as f32 * DT > 60.0 {
            worst_late = worst_late.max(dist3(a.pos, t.pos));
        }
    }
    assert!(worst_late < 8.0, "from a minute on, within {worst_late} m");
}

#[test]
fn a_stalling_plane_recovers_its_airspeed_without_losing_the_floor() {
    let l = Limits::default();
    let mut x = Executor::new();
    // Slow, nose high, low-ish, asked to climb steeply.
    let mut a = Plane::new(0.0, 0.0, 22.0, 0.0, 10.0);
    a.pitch = 0.4;
    let p = Plan { point: [500.0, 0.0, -55.0], ..plan(1, Primitive::Goto) };
    let (mut low, mut recovered_at) = (f32::MAX, None);
    for i in 0..(30.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        assert_within(&d, &l);
        if a.v < l.airspeed_min {
            assert_eq!(d.fallback, Fallback::Recovery as u8);
            assert!(rad(d.pitch_cd) <= 0.0 && rad(d.roll_cd.abs()) <= STALL_BANK + 1.0e-3 && d.throttle == 1.0);
        } else if recovered_at.is_none() {
            recovered_at = Some(i as f32 * DT);
        }
        a.fly_demand(&d);
        low = low.min(a.h());
    }
    assert!(recovered_at.is_some_and(|t| t < 5.0), "{recovered_at:?}");
    assert!(a.v > l.airspeed_min + 2.0 && low > l.floor, "v {} low {low}", a.v);
}

#[test]
fn evade_opens_the_distance_from_a_pursuer() {
    let l = Limits { fence_radius: 90.0, separation: 0.0, ..Limits::default() };
    let mut x = Executor::new();
    let mut a = Plane::new(0.0, 0.0, 30.0, 0.0, 18.0);
    // The threat: 30 m behind, flying straight at us at our speed, our height.
    let mut threat = Plane::new(-30.0, 0.0, 30.0, 0.0, 17.0);
    let p = Plan { target: 4, ..plan(1, Primitive::Evade) };
    let d0 = dist3(a.pos, threat.pos);
    let mut d_end = 0.0;
    for _ in 0..(10.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[threat.as_track(4)], &l);
        assert_within(&d, &l);
        a.fly_demand(&d);
        // The threat chases with pure pursuit, slower and less agile.
        let bearing = (a.pos[1] - threat.pos[1]).atan2(a.pos[0] - threat.pos[0]);
        let err = (bearing - threat.psi + PI).rem_euclid(TAU) - PI;
        let v0 = threat.v;
        threat.fly((err * 1.5).clamp(-0.6, 0.6), 0.0, 0.45);
        threat.v = v0;
        d_end = dist3(a.pos, threat.pos);
    }
    assert!(d_end > d0 + 5.0, "{d0} -> {d_end}");
    assert!(a.dist2([0.0, 0.0]) < 90.0);
}

#[test]
fn heading_out_of_the_fence_it_turns_back_and_never_leaves() {
    let l = Limits { fence_radius: 90.0, ..Limits::default() };
    // Starting 50 m out heading straight out, chasing a point far outside, for ten minutes.
    let mut x = Executor::new();
    let mut a = Plane::new(50.0, 10.0, 30.0, 0.1, 18.0);
    let p = Plan { point: [600.0, 300.0, -30.0], ..plan(1, Primitive::Goto) };
    let mut worst = 0.0f32;
    for _ in 0..(600.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        assert_within(&d, &l);
        a.fly_demand(&d);
        worst = worst.max(a.dist2([0.0, 0.0]));
    }
    assert!(worst < 90.0, "{worst}");
    // From every heading at 30 m, in a wind, flying wings level.
    for k in 0..16 {
        let psi = k as f32 * TAU / 16.0;
        let mut x = Executor::new();
        let mut a = Plane::new(30.0, 0.0, 30.0, psi, 18.0);
        a.wind = [2.0, 1.0];
        let mut worst = 0.0f32;
        for _ in 0..(120.0 / DT) as usize {
            let d = x.step(&a.state(), Some(&Plan { primitive: Primitive::None as u8, ..p }), &[], &l);
            a.fly_demand(&d);
            worst = worst.max(a.dist2([0.0, 0.0]));
        }
        assert!(worst < 90.0, "heading {psi}: {worst}");
    }
}

#[test]
fn the_floor_and_the_ceiling_are_kept() {
    let l = Limits::default(); // floor 12 above the ground, ceiling 60 above home
    // Asked to go to the ground, diving at it.
    let mut x = Executor::new();
    let mut a = Plane::new(0.0, 0.0, 25.0, 0.0, 22.0);
    a.pitch = -0.4;
    let p = Plan { point: [2000.0, 0.0, 10.0], ..plan(1, Primitive::Goto) };
    let mut lowest = f32::MAX;
    for _ in 0..(60.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        a.fly_demand(&d);
        lowest = lowest.min(a.h());
    }
    assert!(lowest > l.floor, "lowest {lowest}");
    // Asked to go far above the ceiling, climbing hard.
    let mut x = Executor::new();
    let mut a = Plane::new(0.0, 0.0, 50.0, 0.0, 24.0);
    a.pitch = 0.4;
    let p = Plan { point: [2000.0, 0.0, -300.0], ..plan(1, Primitive::Goto) };
    let mut highest = 0.0f32;
    for _ in 0..(60.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        a.fly_demand(&d);
        highest = highest.max(a.h());
    }
    assert!(highest < l.ceiling + 2.0, "highest {highest}");
}

#[test]
fn band_holds_its_heights_inside_the_arena() {
    let l = Limits { fence_radius: 90.0, ..Limits::default() };
    let mut x = Executor::new();
    let mut a = Plane::new(20.0, 0.0, 50.0, 0.0, 18.0);
    let p = Plan { band: [20.0, 30.0], ..plan(1, Primitive::Band) };
    let (mut hmin, mut hmax, mut far) = (f32::MAX, 0.0f32, 0.0f32);
    for i in 0..(120.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        a.fly_demand(&d);
        far = far.max(a.dist2([0.0, 0.0]));
        if i as f32 * DT > 30.0 {
            hmin = hmin.min(a.h());
            hmax = hmax.max(a.h());
        }
    }
    assert!(hmin > 19.0 && hmax < 31.0, "{hmin}..{hmax}");
    assert!(far < 90.0);
}

#[test]
fn an_expired_plan_becomes_a_loiter_where_it_ran_out() {
    let l = Limits::default();
    let mut x = Executor::new();
    let mut a = Plane::new(0.0, 0.0, 30.0, 0.0, 18.0);
    let p = Plan { ttl: 5.0, point: [3000.0, 0.0, -30.0], ..plan(1, Primitive::Goto) };
    let mut at = None;
    let mut far = 0.0f32;
    for i in 0..(120.0 / DT) as usize {
        let d = x.step(&a.state(), if i % 50 == 0 { Some(&p) } else { None }, &[], &l);
        a.fly_demand(&d);
        let t = i as f32 * DT;
        assert_eq!(d.fallback != 0, t > 5.0 + DT / 2.0, "at {t}");
        if d.fallback != 0 && at.is_none() {
            at = Some([a.pos[0], a.pos[1]]);
        }
        if let Some(c) = at {
            far = far.max(a.dist2(c));
        }
    }
    assert!(at.unwrap()[0] > 80.0, "it flew the plan first");
    assert!(far < 2.0 * l_loiter(&l) + 5.0, "then circled where it ran out: {far}");
}

#[test]
fn loiter_circles_where_it_was_adopted() {
    let l = Limits::default();
    let mut x = Executor::new();
    let mut a = Plane::new(40.0, -30.0, 30.0, 0.7, 18.0);
    let p = plan(1, Primitive::Loiter);
    let r = l_loiter(&l);
    let mut spread = (f32::MAX, 0.0f32);
    for i in 0..(150.0 / DT) as usize {
        let d = x.step(&a.state(), Some(&p), &[], &l);
        a.fly_demand(&d);
        if i as f32 * DT > 60.0 {
            let q = a.dist2([40.0, -30.0]);
            spread = (spread.0.min(q), spread.1.max(q));
        }
    }
    assert!(spread.0 > r - 5.0 && spread.1 < r + 5.0, "{spread:?} vs {r}");
}

// ---- Fuzzing --------------------------------------------------------------------------------

/// A linear congruential generator (Numerical Recipes' constants).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn unit(&mut self) -> f32 {
        self.next() as f32 / (1u64 << 31) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// Mostly a value in range; now and then something nasty.
    fn nasty(&mut self, lo: f32, hi: f32) -> f32 {
        match self.next() % 60 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => f32::NEG_INFINITY,
            3 => 1.0e30,
            4 => -1.0e30,
            5 => 0.0,
            6 => -0.0,
            7 => f32::MIN_POSITIVE,
            _ => self.range(lo, hi),
        }
    }
}

fn random_limits(r: &mut Lcg) -> Limits {
    Limits {
        fence_centre: [r.nasty(-100.0, 100.0), r.nasty(-100.0, 100.0)],
        fence_radius: r.nasty(-10.0, 300.0),
        ceiling: r.nasty(-10.0, 150.0),
        floor: r.nasty(-10.0, 50.0),
        bank_max: r.nasty(-0.2, 1.6),
        pitch_max: r.nasty(-0.5, 1.0),
        pitch_min: r.nasty(-1.0, 0.5),
        airspeed_min: r.nasty(-5.0, 30.0),
        airspeed_cruise: r.nasty(-5.0, 40.0),
        airspeed_max: r.nasty(-5.0, 50.0),
        separation: r.nasty(-5.0, 30.0),
        grace: r.nasty(-1.0, 5.0),
    }
}

fn random_state(r: &mut Lcg) -> State {
    State {
        pos: [r.nasty(-500.0, 500.0), r.nasty(-500.0, 500.0), r.nasty(-150.0, 20.0)],
        vel: [r.nasty(-40.0, 40.0), r.nasty(-40.0, 40.0), r.nasty(-15.0, 15.0)],
        roll: r.nasty(-1.6, 1.6),
        pitch: r.nasty(-1.0, 1.0),
        yaw: r.nasty(-10.0, 10.0),
        airspeed: r.nasty(-2.0, 45.0),
        agl: r.nasty(-10.0, 150.0),
        dt: r.nasty(-0.1, 0.6),
        time: r.nasty(0.0, 1000.0),
    }
}

fn random_plan(r: &mut Lcg) -> Plan {
    Plan {
        id: (r.next() % 6) as u16,
        primitive: (r.next() % 9) as u8,
        target: (r.next() % 4) as u8,
        point: [r.nasty(-400.0, 400.0), r.nasty(-400.0, 400.0), r.nasty(-120.0, 10.0)],
        radius: r.nasty(-10.0, 300.0),
        band: [r.nasty(-10.0, 100.0), r.nasty(-10.0, 100.0)],
        ttl: r.nasty(-1.0, 10.0),
    }
}

fn random_tracks(r: &mut Lcg) -> [Track; MAX_TRACKS] {
    let mut t = [Track::default(); MAX_TRACKS];
    for x in t.iter_mut() {
        *x = Track {
            id: (r.next() % 4) as u8,
            valid: (r.next() % 3) as u8,
            pos: [r.nasty(-300.0, 300.0), r.nasty(-300.0, 300.0), r.nasty(-100.0, 0.0)],
            vel: [r.nasty(-30.0, 30.0), r.nasty(-30.0, 30.0), r.nasty(-5.0, 5.0)],
            age: r.nasty(-2.0, 6.0),
        };
    }
    t
}

#[test]
fn random_inputs_always_give_a_finite_demand_within_the_limits() {
    let mut r = Lcg(0x5eed);
    let mut x = Executor::new();
    for i in 0..300_000 {
        if i % 1000 == 0 {
            x.reset();
        }
        let (s, l) = (random_state(&mut r), random_limits(&mut r));
        let p = random_plan(&mut r);
        let t = random_tracks(&mut r);
        let plan = if r.next().is_multiple_of(4) { None } else { Some(&p) };
        let n = (r.next() % (MAX_TRACKS as u32 + 1)) as usize;
        let d = x.step(&s, plan, &t[..n], &l);
        assert!(d.throttle.is_finite(), "{s:?} {p:?} {l:?}");
        if Lims::new(&l).is_some() && state_ok(&s) {
            assert_ne!(d.fallback, Fallback::BadInput as u8);
            assert_within(&d, &l);
        } else {
            assert_eq!(d, bad_input(), "{s:?} {l:?}");
        }
    }
}

/// Twenty minutes of random plans (a new one every 8 s), in a wind, in the 90 m arena, with
/// two other planes circling in it (separation off): one the target of pursuits and evasions,
/// one the other way round and lower.
fn soak(seed: u64) {
    let mut r = Lcg(seed);
    let l = Limits { fence_radius: 90.0, ..Limits::default() };
    let mut x = Executor::new();
    let mut a = Plane::new(0.0, 0.0, 30.0, r.range(-PI, PI), 18.0);
    a.wind = [r.range(-3.0, 3.0), r.range(-3.0, 3.0)];
    let mut target = circling_target(0.0, -54.0, 30.0, 15.0, (15.0f32 * 15.0 / (G * 54.0)).atan());
    let mut other = circling_target(0.0, 40.0, 24.0, 16.0, -(16.0f32 * 16.0 / (G * 40.0)).atan());
    let mut p = plan(0, Primitive::None);
    let (mut far, mut low, mut high) = (0.0f32, f32::MAX, 0.0f32);
    let mut by_rule = [0u32; 6];
    let steps = (1200.0 / DT) as usize;
    for i in 0..steps {
        if i % 400 == 0 {
            let target = 2 + (r.next() % 2) as u8;
            p = random_flyable_plan(&mut r, p.id.wrapping_add(1), target);
        }
        let (t, o) = (target(), other());
        let d = x.step(&a.state(), Some(&p), &[t.as_track(2), o.as_track(3)], &l);
        assert_within(&d, &l);
        count_rules(&x, &mut by_rule);
        a.fly_demand(&d);
        far = far.max(a.dist2([0.0, 0.0]));
        low = low.min(a.h());
        high = high.max(a.h());
    }
    eprintln!("seed {seed}: far {far:.1} low {low:.1} high {high:.1}; {}", rules_summary(&by_rule, steps));
    assert!(far < 90.0, "seed {seed}: {far}");
    assert!(low > l.floor, "seed {seed}: {low}");
    assert!(high < l.ceiling + 2.0, "seed {seed}: {high}");
}

fn random_flyable_plan(r: &mut Lcg, id: u16, target: u8) -> Plan {
    Plan {
        id,
        primitive: (r.next() % 7) as u8,
        target,
        point: [r.range(-200.0, 200.0), r.range(-200.0, 200.0), r.range(-100.0, 0.0)],
        radius: r.range(0.0, 150.0),
        band: [r.range(0.0, 50.0), r.range(0.0, 80.0)],
        ttl: r.range(4.0, 12.0),
    }
}

fn count_rules(x: &Executor, by_rule: &mut [u32; 6]) {
    for (b, n) in by_rule.iter_mut().enumerate() {
        *n += (x.rules() >> b & 1) as u32;
    }
}

fn rules_summary(by_rule: &[u32; 6], steps: usize) -> String {
    let names = ["separation", "fence", "ceiling", "floor", "stall", "limits"];
    let parts: Vec<String> =
        names.iter().zip(by_rule).map(|(n, k)| format!("{n} {:.1}%", 100.0 * *k as f32 / steps as f32)).collect();
    format!("the envelope changed the demand on: {}", parts.join(", "))
}

#[test]
fn random_plans_flown_for_a_long_time_stay_inside_the_envelope() {
    for seed in 0..8 {
        soak(seed);
    }
}

/// Three planes, each flown by its own executor with separation 8 m, on random plans (pursuing
/// and evading each other among them), for twenty minutes, as in a game of tag, inside a fence
/// of `fence` m. Returns the closest any two came horizontally while within the separation
/// vertically.
fn cooperative_soak(seed: u64, fence: f32) -> f32 {
    let separation = 8.0;
    let mut r = Lcg(seed);
    let l = Limits { fence_radius: fence, separation, ..Limits::default() };
    let wind = [r.range(-3.0, 3.0), r.range(-3.0, 3.0)];
    let mut planes: Vec<(Executor, Plane, Plan)> = (0..3)
        .map(|k| {
            let a = k as f32 * TAU / 3.0;
            let mut p = Plane::new(40.0 * a.cos(), 40.0 * a.sin(), 25.0 + 8.0 * k as f32, a + 1.6, 18.0);
            p.wind = wind;
            (Executor::new(), p, plan(0, Primitive::None))
        })
        .collect();
    let (mut far, mut low, mut high, mut closest) = (0.0f32, f32::MAX, 0.0f32, f32::MAX);
    let mut by_rule = [0u32; 6];
    let steps = (1200.0 / DT) as usize;
    for i in 0..steps {
        let tracks: Vec<Track> = planes.iter().enumerate().map(|(k, (_, p, _))| p.as_track(k as u8 + 1)).collect();
        for (k, (x, a, p)) in planes.iter_mut().enumerate() {
            if (i + 133 * k) % 400 == 0 {
                let target = ((k as u32 + 1 + r.next() % 2) % 3) as u8 + 1;
                *p = random_flyable_plan(&mut r, p.id.wrapping_add(1), target);
            }
            let others: Vec<Track> = tracks.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, t)| *t).collect();
            let d = x.step(&a.state(), Some(p), &others, &l);
            assert_within(&d, &l);
            count_rules(x, &mut by_rule);
            a.fly_demand(&d);
            far = far.max(a.dist2([0.0, 0.0]));
            low = low.min(a.h());
            high = high.max(a.h());
        }
        for j in 0..3 {
            for k in j + 1..3 {
                let (a, b) = (&planes[j].1, &planes[k].1);
                if (a.h() - b.h()).abs() < separation {
                    closest = closest.min(a.dist2([b.pos[0], b.pos[1]]));
                }
            }
        }
    }
    eprintln!(
        "seed {seed}, fence {fence}: far {far:.1} low {low:.1} high {high:.1} closest {closest:.1}; {}",
        rules_summary(&by_rule, 3 * steps)
    );
    assert!(far < fence, "seed {seed}: {far}");
    assert!(low > l.floor, "seed {seed}: {low}");
    assert!(high < l.ceiling + 2.0, "seed {seed}: {high}");
    closest
}

#[test]
fn planes_playing_with_room_to_manoeuvre_never_come_inside_the_separation() {
    for seed in 100..108 {
        let closest = cooperative_soak(seed, 300.0);
        assert!(closest > 8.0, "seed {seed}: {closest}");
    }
}

/// In the 90 m tag arena, three planes and an 8 m separation (a 14 m zone with its buffer) are
/// crowded: the fence comes first, and where it and the separation can't both be had, the
/// separation gives. In these eight runs (eight hours of plane time) pairs came within 2.7 m,
/// 3.6 m and 5.7 m for a moment; the rest kept it. The fence, the floor and the
/// ceiling hold; the separation is only reported.
#[test]
fn planes_playing_in_the_tag_arena_keep_the_fence_floor_and_ceiling() {
    for seed in 100..108 {
        cooperative_soak(seed, 90.0);
    }
}
/// An intruder that does not look out for us, flying straight through our path.
#[test]
fn an_intruder_flying_through_is_kept_outside_the_separation() {
    let l = Limits { separation: 8.0, ..Limits::default() };
    // (start north, east, heading) of the intruder, against us at the origin heading north.
    for (n, e, psi) in
        [(150.0, 0.0, PI), (150.0, 3.0, PI), (60.0, 60.0, -HALF_PI), (60.0, -60.0, HALF_PI), (-40.0, 0.0, 0.0)]
    {
        let mut x = Executor::new();
        let mut a = Plane::new(0.0, 0.0, 30.0, 0.0, 18.0);
        let mut b = Plane::new(n, e, 30.0, psi, if n < 0.0 { 24.0 } else { 18.0 });
        let p = Plan { point: [600.0, 0.0, -30.0], ..plan(1, Primitive::Goto) };
        let mut closest = f32::MAX;
        for _ in 0..(20.0 / DT) as usize {
            let d = x.step(&a.state(), Some(&p), &[b.as_track(7)], &l);
            a.fly_demand(&d);
            let v = b.v;
            b.fly(0.0, 0.0, 0.45);
            b.v = v;
            b.pos[2] = -30.0;
            if (a.h() - b.h()).abs() < l.separation {
                closest = closest.min(a.dist2([b.pos[0], b.pos[1]]));
            }
        }
        assert!(closest > l.separation, "intruder from ({n}, {e}): {closest}");
    }
}
