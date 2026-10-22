//! The pylon race: the course of the pylon racing leagues (FAI F3D and F5D) and the referee that
//! judges it.
//!
//! The course is a triangle of three pylons, 400 m round: two pylons 40 m apart at one end and a
//! single pylon 180 m from each of them at the other. Laps go counterclockwise, with left turns
//! round the outside of each pylon: pylon 1 (the single one), pylon 2 (the far twin), pylon 3 (the
//! near twin), then the start line, which crosses the near leg 30 m past pylon 3. A turn inside a
//! pylon is a cut: one adds 10 % to the racer's time, two disqualify.
//!
//! World frame as everywhere in the project: +X east, +Y up, -Z north. Everything here is judged
//! on the ground plane; heights don't matter.

use glam::Vec3;

/// How tall the pylons are (m), as the leagues have them.
pub const PYLON_HEIGHT: f32 = 5.0;
/// From the single pylon to each twin (m).
pub const LEG: f32 = 180.0;
/// Between the twin pylons (m).
pub const TWIN_GAP: f32 = 40.0;
/// From the near twin (pylon 3) to the start line, along the near leg (m).
pub const LINE_PAST_PYLON_3: f32 = 30.0;
/// How far in front of the pilot box the near twin stands (m).
pub const NEAR_TWIN_AHEAD: f32 = 60.0;
/// A race for the T1-S: its pack doesn't last the leagues' ten laps.
pub const LAPS: u32 = 5;
/// A cut adds this fraction of the racer's time.
pub const CUT_PENALTY: f32 = 0.1;
/// This many cuts disqualify.
pub const CUTS_TO_DISQUALIFY: u32 = 2;
/// A crossing of a pylon's line on the inside counts as a cut up to this far from the pylon (m);
/// further in, the plane is crossing the field, not turning at the pylon.
pub const CUT_DEPTH: f32 = 50.0;
/// The judges sight this far out from each pylon (m).
const SIGHT: f32 = 400.0;

/// The course on the ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Course {
    /// In the order of a lap: pylon 1 (the single pylon), pylon 2 (the far twin), pylon 3 (the
    /// near twin). Their feet, on the ground.
    pub pylons: [Vec3; 3],
    /// The start line: from the middle of the course out across the near leg.
    pub line: [Vec3; 2],
    /// The way a racer crosses the start line to count a lap (along the near leg).
    pub line_dir: Vec3,
    pub laps: u32,
}

/// Horizontal cross product: positive when `b` is to the left of `a` (seen from above, flying
/// along `a`).
fn cross(a: Vec3, b: Vec3) -> f32 {
    a.z * b.x - a.x * b.z
}

/// Where segment `a`–`b` crosses segment `c`–`d` on the ground, if it does.
fn crossing(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> Option<Vec3> {
    let (r, s) = (b - a, d - c);
    let den = cross(r, s);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = cross(c - a, s) / den;
    let u = cross(c - a, r) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

impl Course {
    /// The leagues' triangle on the field in front of a pilot standing at `pilot`, looking along
    /// `forward`: the twin pylons on the left, the single pylon on the right, and the near leg
    /// across the view.
    pub fn in_front_of(pilot: Vec3, forward: Vec3) -> Course {
        let f = flat(forward).normalize();
        let r = f.cross(Vec3::Y);
        let depth = (LEG * LEG - 0.25 * TWIN_GAP * TWIN_GAP).sqrt();
        let at = |right: f32, ahead: f32| pilot + r * right + f * ahead;
        let p3 = at(-0.5 * depth, NEAR_TWIN_AHEAD);
        let p2 = at(-0.5 * depth, NEAR_TWIN_AHEAD + TWIN_GAP);
        let p1 = at(0.5 * depth, NEAR_TWIN_AHEAD + 0.5 * TWIN_GAP);
        // Square to the near leg, from the middle of the course to well outside the leg.
        let along = flat(p1 - p3).normalize();
        let out = Vec3::new(-along.z, 0.0, along.x);
        let on_line = p3 + along * LINE_PAST_PYLON_3;
        Course {
            pylons: [p1, p2, p3],
            line: [on_line - out * 0.5 * TWIN_GAP, on_line + out * (NEAR_TWIN_AHEAD + 20.0)],
            line_dir: along,
            laps: LAPS,
        }
    }

    /// The middle of the triangle.
    pub fn centre(&self) -> Vec3 {
        (self.pylons[0] + self.pylons[1] + self.pylons[2]) / 3.0
    }

    /// The length of a lap round the pylons themselves (m).
    pub fn lap_length(&self) -> f32 {
        (0..3).map(|i| flat(self.pylons[(i + 1) % 3] - self.pylons[i]).length()).sum()
    }

    /// Unit vector from pylon `i` away from the course, halfway between its two legs: the line
    /// the judges sight along.
    pub fn outward(&self, i: usize) -> Vec3 {
        let p = self.pylons[i];
        let to_prev = flat(self.pylons[(i + 2) % 3] - p).normalize();
        let to_next = flat(self.pylons[(i + 1) % 3] - p).normalize();
        -(to_prev + to_next).normalize()
    }

    /// The way a racer goes past pylon `i`, across its line.
    fn travel(&self, i: usize) -> Vec3 {
        let t = flat(self.pylons[(i + 1) % 3] - self.pylons[(i + 2) % 3]);
        let b = self.outward(i);
        (t - b * t.dot(b)).normalize()
    }

    /// The outward normal of the leg from pylon `i` to the next one (to the right of the way the
    /// racers fly it).
    fn leg_normal(&self, i: usize) -> Vec3 {
        let d = flat(self.pylons[(i + 1) % 3] - self.pylons[i]).normalize();
        Vec3::new(-d.z, 0.0, d.x)
    }

    /// A closed line round the outside of the course, `radius` from the pylons, with a point
    /// every `step` metres or so: the legs moved out by `radius`, and arcs round the pylons.
    /// It starts as the arc round pylon 1 begins.
    pub fn outside_line(&self, radius: f32, step: f32) -> Vec<Vec3> {
        self.line_with(None, radius, step)
    }

    /// The same line, but it turns inside pylon `cut`, passing `depth` metres inside it: what a
    /// racer who cuts that pylon flies.
    pub fn cutting_line(&self, cut: usize, radius: f32, depth: f32, step: f32) -> Vec<Vec3> {
        self.line_with(Some((cut, depth)), radius, step)
    }

    fn line_with(&self, cut: Option<(usize, f32)>, radius: f32, step: f32) -> Vec<Vec3> {
        let mut pts = Vec::new();
        let straight = |pts: &mut Vec<Vec3>, a: Vec3, b: Vec3| {
            let n = ((b - a).length() / step).ceil().max(1.0) as usize;
            pts.extend((0..n).map(|k| a.lerp(b, k as f32 / n as f32)));
        };
        // Where the line leaves pylon `i`, and where it arrives at it.
        let leave = |i: usize| match cut {
            Some((c, depth)) if c == i => self.pylons[i] - self.outward(i) * depth,
            _ => self.pylons[i] + self.leg_normal(i) * radius,
        };
        let arrive = |i: usize| match cut {
            Some((c, depth)) if c == i => self.pylons[i] - self.outward(i) * depth,
            _ => self.pylons[i] + self.leg_normal((i + 2) % 3) * radius,
        };
        for i in 0..3 {
            let is_cut = cut.is_some_and(|(c, _)| c == i);
            if !is_cut {
                let n0 = self.leg_normal((i + 2) % 3);
                let n1 = self.leg_normal(i);
                let a0 = n0.z.atan2(n0.x);
                let mut delta = n1.z.atan2(n1.x) - a0;
                while delta > std::f32::consts::PI {
                    delta -= std::f32::consts::TAU;
                }
                while delta < -std::f32::consts::PI {
                    delta += std::f32::consts::TAU;
                }
                let n = ((delta.abs() * radius) / step).ceil().max(1.0) as usize;
                for k in 0..n {
                    let a = a0 + delta * k as f32 / n as f32;
                    pts.push(self.pylons[i] + Vec3::new(a.cos(), 0.0, a.sin()) * radius);
                }
            }
            straight(&mut pts, leave(i), arrive((i + 1) % 3));
        }
        pts
    }
}

/// Where a racer is in the race.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Racing,
    /// Across the line after the last lap: the time from GO, and that time with the cuts added.
    Finished {
        time: f32,
        corrected: f32,
    },
    /// Two cuts.
    Disqualified,
    /// Crashed, or picked up.
    Out,
}

/// The next thing a racer must pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Pylon(usize),
    Line,
}

/// What the referee saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Call {
    Rounded { racer: usize, pylon: usize },
    Cut { racer: usize, pylon: usize },
    Lap { racer: usize, laps: u32 },
    Finished { racer: usize },
    Disqualified { racer: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Racer {
    pub laps: u32,
    pub next: Gate,
    pub cuts: u32,
    pub status: Status,
    /// Pylons and lines passed since GO, and when the last one was.
    gates: u32,
    last_gate: f32,
    last_pos: Option<Vec3>,
}

impl Racer {
    fn new() -> Racer {
        Racer {
            laps: 0,
            next: Gate::Pylon(0),
            cuts: 0,
            status: Status::Racing,
            gates: 0,
            last_gate: 0.0,
            last_pos: None,
        }
    }
}

/// The referee of one race.
#[derive(Clone, Debug)]
pub struct Race {
    pub course: Course,
    pub racers: Vec<Racer>,
}

impl Race {
    pub fn new(course: Course, racers: usize) -> Race {
        Race { course, racers: (0..racers).map(|_| Racer::new()).collect() }
    }

    /// Where racer `racer` is at `t` seconds after GO. The referee judges what it crossed since
    /// it was last seen.
    pub fn observe(&mut self, racer: usize, pos: Vec3, t: f32) -> Vec<Call> {
        let mut calls = vec![];
        let course = &self.course;
        let r = &mut self.racers[racer];
        let Some(from) = r.last_pos.replace(pos) else { return calls };
        if r.status != Status::Racing {
            return calls;
        }
        let moved = flat(pos - from);
        match r.next {
            Gate::Pylon(i) => {
                let p = course.pylons[i];
                let b = course.outward(i);
                if moved.dot(course.travel(i)) <= 0.0 {
                    return calls;
                }
                let rounded = crossing(from, pos, p, p + b * SIGHT).is_some();
                let cut = !rounded && crossing(from, pos, p, p - b * CUT_DEPTH).is_some();
                if !(rounded || cut) {
                    return calls;
                }
                r.next = if i == 2 { Gate::Line } else { Gate::Pylon(i + 1) };
                r.gates += 1;
                r.last_gate = t;
                if rounded {
                    calls.push(Call::Rounded { racer, pylon: i });
                } else {
                    r.cuts += 1;
                    calls.push(Call::Cut { racer, pylon: i });
                    if r.cuts >= CUTS_TO_DISQUALIFY {
                        r.status = Status::Disqualified;
                        calls.push(Call::Disqualified { racer });
                    }
                }
            }
            Gate::Line => {
                if moved.dot(course.line_dir) <= 0.0 || crossing(from, pos, course.line[0], course.line[1]).is_none() {
                    return calls;
                }
                r.laps += 1;
                r.next = Gate::Pylon(0);
                r.gates += 1;
                r.last_gate = t;
                calls.push(Call::Lap { racer, laps: r.laps });
                if r.laps >= course.laps {
                    r.status = Status::Finished { time: t, corrected: t * (1.0 + CUT_PENALTY * r.cuts as f32) };
                    calls.push(Call::Finished { racer });
                }
            }
        }
        calls
    }

    /// The racer crashed, or was picked up: it is out of the race.
    pub fn retire(&mut self, racer: usize) {
        let r = &mut self.racers[racer];
        if r.status == Status::Racing {
            r.status = Status::Out;
        }
    }

    /// The racers in the order of their places: the finishers by their times with the cuts
    /// added, then those still racing by how far they have got (and who got there first), then
    /// those out, then the disqualified.
    pub fn standings(&self) -> Vec<usize> {
        let key = |r: &Racer| match r.status {
            Status::Finished { corrected, .. } => (0, corrected),
            Status::Racing => (1, -(r.gates as f32) * 1e6 + r.last_gate),
            Status::Out => (2, -(r.gates as f32)),
            Status::Disqualified => (3, 0.0),
        };
        let mut order: Vec<usize> = (0..self.racers.len()).collect();
        order.sort_by(|&a, &b| {
            let (ka, kb) = (key(&self.racers[a]), key(&self.racers[b]));
            ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        });
        order
    }

    /// The place of a racer, from 1.
    pub fn place(&self, racer: usize) -> usize {
        self.standings().iter().position(|&r| r == racer).unwrap() + 1
    }

    /// Every racer has finished or is out of the race.
    pub fn over(&self) -> bool {
        self.racers.iter().all(|r| r.status != Status::Racing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pilot at the origin, facing north.
    fn course() -> Course {
        Course::in_front_of(Vec3::ZERO, Vec3::NEG_Z)
    }

    fn dist(a: Vec3, b: Vec3) -> f32 {
        flat(a - b).length()
    }

    #[test]
    fn the_course_is_the_leagues_triangle() {
        let c = course();
        let [p1, p2, p3] = c.pylons;
        assert!((dist(p1, p2) - LEG).abs() < 0.01, "pylon 1 to 2: {}", dist(p1, p2));
        assert!((dist(p1, p3) - LEG).abs() < 0.01);
        assert!((dist(p2, p3) - TWIN_GAP).abs() < 0.01);
        assert!((c.lap_length() - 400.0).abs() < 0.01);
        // The twins on the left of the pilot, pylon 1 on the right, all of it in front.
        assert!(p2.x < 0.0 && p3.x < 0.0 && p1.x > 0.0);
        assert!(c.pylons.iter().all(|p| -p.z >= NEAR_TWIN_AHEAD - 0.01), "{:?}", c.pylons);
        // The start line crosses the near leg 30 m past pylon 3.
        let near = crossing(c.line[0], c.line[1], p3, p1).expect("the line crosses the near leg");
        assert!((dist(near, p3) - LINE_PAST_PYLON_3).abs() < 0.01);
    }

    #[test]
    fn a_lap_turns_left_at_every_pylon() {
        let c = course();
        for i in 0..3 {
            let a = c.pylons[(i + 1) % 3] - c.pylons[i];
            let b = c.pylons[(i + 2) % 3] - c.pylons[(i + 1) % 3];
            // Flying west then south, say: left turns are positive.
            assert!(a.cross(b).y > 0.0, "a right turn after pylon {}", i + 1);
        }
    }

    #[test]
    fn the_outside_line_keeps_its_distance_from_the_pylons() {
        let c = course();
        let line = c.outside_line(20.0, 1.0);
        let closest = line
            .iter()
            .map(|p| c.pylons.iter().map(|q| dist(*p, *q)).fold(f32::MAX, f32::min))
            .fold(f32::MAX, f32::min);
        assert!((closest - 20.0).abs() < 0.05, "closest {closest}");
        let length: f32 = (0..line.len()).map(|i| dist(line[i], line[(i + 1) % line.len()])).sum();
        let expected = 400.0 + std::f32::consts::TAU * 20.0;
        assert!((length - expected).abs() < 2.0, "{length} m round, against {expected}");
    }

    /// Flies `racer` along `path` (closed) for `loops` loops from where it starts, at 25 m/s.
    fn fly(race: &mut Race, racer: usize, path: &[Vec3], loops: usize, t: &mut f32) -> Vec<Call> {
        let mut calls = vec![];
        for k in 0..path.len() * loops + 1 {
            let p = path[k % path.len()];
            *t += 1.0 / 25.0;
            calls.extend(race.observe(racer, p, *t));
        }
        calls
    }

    /// A closed line turned so that it starts at its point nearest the pilot box, on the near leg.
    fn from_near_leg(mut path: Vec<Vec3>) -> Vec<Vec3> {
        let nearest = (0..path.len()).min_by(|&a, &b| path[a].length().total_cmp(&path[b].length())).unwrap();
        path.rotate_left(nearest);
        path
    }

    /// From the pilot box out over the course to where the outside line starts (as a throw does).
    fn launch(race: &mut Race, racer: usize, to: Vec3, t: &mut f32) {
        for k in 0..=50 {
            race.observe(racer, Vec3::ZERO.lerp(to, k as f32 / 50.0), *t);
        }
    }

    #[test]
    fn five_laps_round_the_outside_finish_the_race() {
        let mut race = Race::new(course(), 1);
        let line = from_near_leg(race.course.outside_line(20.0, 1.0));
        let mut t = 0.0;
        launch(&mut race, 0, line[0], &mut t);
        let calls = fly(&mut race, 0, &line, 1, &mut t);
        assert_eq!(race.racers[0].laps, 1, "{calls:?}");
        assert!(calls.contains(&Call::Rounded { racer: 0, pylon: 0 }));
        assert!(calls.contains(&Call::Lap { racer: 0, laps: 1 }));
        assert_eq!(race.racers[0].cuts, 0);
        fly(&mut race, 0, &line, 4, &mut t);
        assert_eq!(race.racers[0].laps, 5);
        assert!(
            matches!(race.racers[0].status, Status::Finished { time, corrected } if time == corrected && time > 0.0)
        );
        assert!(race.over());
    }

    #[test]
    fn a_turn_inside_a_pylon_is_a_cut_and_a_second_disqualifies() {
        let mut race = Race::new(course(), 1);
        let line = from_near_leg(race.course.outside_line(20.0, 1.0));
        let mut t = 0.0;
        launch(&mut race, 0, line[0], &mut t);
        // Inside pylon 1: a cut, but the lap goes on.
        let cut = from_near_leg(race.course.cutting_line(0, 20.0, 10.0, 1.0));
        let calls = fly(&mut race, 0, &cut, 1, &mut t);
        assert!(calls.contains(&Call::Cut { racer: 0, pylon: 0 }), "{calls:?}");
        assert_eq!((race.racers[0].cuts, race.racers[0].laps), (1, 1));
        assert_eq!(race.racers[0].status, Status::Racing);
        // Inside pylon 2 on the next lap: out.
        let cut = from_near_leg(race.course.cutting_line(1, 20.0, 10.0, 1.0));
        let calls = fly(&mut race, 0, &cut, 1, &mut t);
        assert!(calls.contains(&Call::Disqualified { racer: 0 }), "{calls:?}");
        assert_eq!(race.racers[0].status, Status::Disqualified);
    }

    #[test]
    fn the_pylons_count_only_in_order() {
        let mut race = Race::new(course(), 1);
        let line = race.course.outside_line(20.0, 1.0);
        // Start halfway down the far leg, past pylon 1 without going round it: pylons 2 and 3
        // and the line don't count until pylon 1 has.
        let mut t = 0.0;
        let mut calls = vec![];
        for p in &line[line.len() / 3..] {
            t += 0.04;
            calls.extend(race.observe(0, *p, t));
        }
        assert!(calls.is_empty(), "{calls:?}");
        assert_eq!(race.racers[0].laps, 0);
    }

    #[test]
    fn crossing_the_line_the_wrong_way_does_not_count() {
        let mut race = Race::new(course(), 1);
        let mut line = race.course.outside_line(20.0, 1.0);
        line.reverse();
        let mut t = 0.0;
        fly(&mut race, 0, &line, 2, &mut t);
        assert_eq!(race.racers[0].laps, 0);
    }

    #[test]
    fn a_throw_across_the_course_is_not_a_cut() {
        let mut race = Race::new(course(), 1);
        let mut t = 0.0;
        // Straight out from the pilot box, across both legs.
        for k in 0..=200 {
            race.observe(0, Vec3::new(0.0, 0.0, -(k as f32)), t);
            t += 0.04;
        }
        assert_eq!(race.racers[0].cuts, 0);
    }

    #[test]
    fn places_go_by_time_with_the_cuts_added() {
        let mut race = Race::new(course(), 4);
        let line = from_near_leg(race.course.outside_line(20.0, 1.0));
        let cut = from_near_leg(race.course.cutting_line(0, 20.0, 10.0, 1.0));
        // Racer 0 cuts once and is a little faster; racer 1 is clean; racer 2 is out; racer 3 is
        // still on its first lap.
        let mut t = 0.0;
        launch(&mut race, 0, line[0], &mut t);
        fly(&mut race, 0, &cut, 1, &mut t);
        fly(&mut race, 0, &line, 4, &mut t);
        let mut t1 = 0.0;
        launch(&mut race, 1, line[0], &mut t1);
        t1 += 0.5;
        fly(&mut race, 1, &line, 5, &mut t1);
        race.retire(2);
        let mut t3 = 0.0;
        launch(&mut race, 3, line[0], &mut t3);
        for p in &line[..line.len() / 2] {
            t3 += 0.04;
            race.observe(3, *p, t3);
        }
        let (Status::Finished { corrected: c0, .. }, Status::Finished { corrected: c1, .. }) =
            (race.racers[0].status, race.racers[1].status)
        else {
            panic!("both finish: {:?}", race.racers)
        };
        assert!(c0 > c1, "the cut costs 10 %: {c0} against {c1}");
        assert_eq!(race.standings(), vec![1, 0, 3, 2]);
        assert_eq!(race.place(3), 3);
        assert!(!race.over(), "racer 3 is still racing");
    }

    #[test]
    fn a_retired_racer_is_out() {
        let mut race = Race::new(course(), 2);
        race.retire(1);
        assert_eq!(race.racers[1].status, Status::Out);
        assert_eq!(race.standings(), vec![0, 1]);
    }
}
