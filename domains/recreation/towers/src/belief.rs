//! Where the enemy drones are, as far as a team can tell ([`Belief`]): a particle filter.
//!
//! The team sees an enemy drone only near its own drones, its held towers and its base. For the
//! rest of the time it has an old sighting, and the fog. A belief keeps a cloud of guesses for
//! each enemy drone. A guess flies towards a tower, as an enemy drone does. When the team sees the
//! drone, the cloud collapses onto it. When the team looks at a place and the drone is not there,
//! the guesses that were in that place go: not seeing it is also news.

use crate::{Rules, Team, TeamView};
use glam::Vec2;

/// The guesses for each enemy drone.
const PARTICLES: usize = 128;

/// One guess: where the drone is, the tower it flies to and how fast.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Guess {
    at: Vec2,
    goal: usize,
    speed: f32,
}

#[derive(Clone, Debug)]
struct Track {
    guesses: Vec<Guess>,
    /// When the team last saw the drone (game time, s); `NEG_INFINITY` if never.
    seen_at: f32,
    /// Where it was then (or its base, if never).
    last_at: Vec2,
}

/// What a team believes about where the enemy drones are.
#[derive(Clone, Debug)]
pub struct Belief {
    rules: Rules,
    team: Team,
    /// Where the towers stand (the layout's).
    towers: Vec<Vec2>,
    tracks: Vec<Track>,
    /// The game time of the last update.
    time: f32,
    rng: u64,
}

/// A drone flies about this fast (m/s).
const CRUISE: f32 = 16.0;
/// A guess in the sight of the team, with no drone seen, keeps this much of its weight.
const UNSEEN: f32 = 0.01;
/// How much a guess wanders (m per root second).
const WANDER: f32 = 2.5;
/// How likely a guess at a tower that its team holds goes on to another one each second.
const LEAVE: f32 = 0.01;

impl Belief {
    /// At the start: every enemy drone is at its base.
    pub fn new(rules: Rules, team: Team) -> Belief {
        let towers: Vec<Vec2> = crate::map::Layout::of(&rules).towers.iter().map(|t| t.at).collect();
        let mut b = Belief {
            rules,
            team,
            towers,
            tracks: vec![],
            time: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15 ^ (team.index() as u64 + 1),
        };
        let base = rules.base(team.other());
        for _ in 0..rules.drones {
            let guesses = (0..PARTICLES).map(|_| b.guess_at(base, Vec2::ZERO, 12.0)).collect();
            b.tracks.push(Track { guesses, seen_at: f32::NEG_INFINITY, last_at: base });
        }
        b
    }

    fn next(&mut self) -> f32 {
        // xorshift64*: a number in [0, 1).
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// About a normal number (the sum of three uniform ones).
    fn gauss(&mut self) -> f32 {
        (self.next() + self.next() + self.next() - 1.5) * 2.0
    }

    /// A tower to fly to: those the enemy does not hold count double.
    fn goal(&mut self, holders: &[Option<Team>]) -> usize {
        let enemy = self.team.other();
        let weight = |t: usize| if holders.get(t).copied().flatten() == Some(enemy) { 0.5 } else { 2.0 };
        let total: f32 = (0..self.towers.len()).map(weight).sum();
        let mut pick = self.next() * total;
        for t in 0..self.towers.len() {
            pick -= weight(t);
            if pick <= 0.0 {
                return t;
            }
        }
        self.towers.len() - 1
    }

    /// A guess at `at` with some noise (m): flying as `vel` says, or to a tower if it says nothing.
    fn guess_at(&mut self, at: Vec2, vel: Vec2, noise: f32) -> Guess {
        let place = at + Vec2::new(self.gauss(), self.gauss()) * noise;
        let goal = if vel.length() > 3.0 && self.next() < 0.7 {
            // The tower that the drone heads for the most: it flies on in its direction.
            let dir = vel.normalize();
            (0..self.towers.len())
                .max_by(|&a, &b| {
                    let score = |t: usize| dir.dot((self.towers[t] - at).normalize_or_zero());
                    score(a).total_cmp(&score(b))
                })
                .unwrap_or(0)
        } else {
            self.goal(&[])
        };
        let speed = if vel.length() > 3.0 { vel.length() + 2.0 * self.gauss() } else { CRUISE + 3.0 * self.gauss() };
        Guess { at: place, goal, speed: speed.clamp(8.0, 22.0) }
    }

    /// The guesses of every drone fly for `dt` seconds. A drone that has got to its tower circles
    /// it until the tower is held, and then goes on to another.
    fn predict(&mut self, k: usize, dt: f32, holders: &[Option<Team>]) {
        let mut guesses = std::mem::take(&mut self.tracks[k].guesses);
        let enemy = self.team.other();
        for g in &mut guesses {
            let tower = self.towers[g.goal];
            let arrived = (tower - g.at).length() < 40.0;
            let held = holders.get(g.goal).copied().flatten() == Some(enemy);
            let r = self.rules.field_radius;
            if arrived && held && self.next() < LEAVE * dt {
                g.goal = self.goal(holders);
            }
            if arrived {
                // Circles the tower: stays about where it is.
                g.at += Vec2::new(self.gauss(), self.gauss()) * WANDER * dt.sqrt();
            } else {
                let dir = (tower - g.at).normalize_or_zero();
                let noise = Vec2::new(self.gauss(), self.gauss()) * WANDER * dt.sqrt();
                g.at += dir * g.speed * dt + noise;
            }
            if g.at.length() > r {
                g.at = g.at.normalize() * r;
            }
        }
        self.tracks[k].guesses = guesses;
    }

    /// Where the team looks now: a place and how far it sees from it.
    fn eyes(&self, view: &TeamView) -> Vec<(Vec2, f32)> {
        let mut eyes: Vec<(Vec2, f32)> = view
            .drones
            .iter()
            .filter(|d| d.linked && d.phase != crate::Phase::Down)
            .map(|d| (d.at, self.rules.drone_vision))
            .collect();
        eyes.extend(
            view.towers.iter().filter(|t| t.holder == Some(self.team)).map(|t| (t.at, self.rules.vision(t.kind))),
        );
        eyes.push((self.rules.base(self.team), self.rules.base_vision));
        eyes
    }

    /// Takes in the team's map at its time.
    pub fn update(&mut self, view: &TeamView) {
        let dt = (view.time - self.time).clamp(0.0, 5.0);
        self.time = view.time;
        let holders: Vec<Option<Team>> = view.towers.iter().map(|t| t.holder).collect();
        let eyes = self.eyes(view);
        for k in 0..self.tracks.len() {
            let sighting = view.enemies.iter().find(|e| e.k == k);
            let mut seen_now = false;
            match sighting {
                Some(s) if s.seen => {
                    let guesses = (0..PARTICLES).map(|_| self.guess_at(s.at, s.vel, 5.0)).collect();
                    self.tracks[k] = Track { guesses, seen_at: view.time, last_at: s.at };
                    seen_now = true;
                }
                Some(s) if view.time - s.age > self.tracks[k].seen_at + 0.01 => {
                    // A sighting that is new to the belief: the drone was there `age` s ago.
                    let guesses = (0..PARTICLES).map(|_| self.guess_at(s.at, s.vel, 5.0)).collect();
                    self.tracks[k] = Track { guesses, seen_at: view.time - s.age, last_at: s.at };
                    let mut left = s.age;
                    while left > 0.0 {
                        let step = left.min(1.0);
                        self.predict(k, step, &holders);
                        left -= step;
                    }
                }
                _ => self.predict(k, dt, &holders),
            }
            if !seen_now {
                self.rule_out(k, &eyes, view.time);
            }
        }
    }

    /// The guesses in a place that the team looks at, with no drone seen there, lose their weight.
    fn rule_out(&mut self, k: usize, eyes: &[(Vec2, f32)], time: f32) {
        let seen = |at: Vec2| eyes.iter().any(|&(e, r)| e.distance(at) <= r);
        let outside = self.tracks[k].guesses.iter().filter(|g| !seen(g.at)).count();
        if outside < 4 {
            // All of them are where the team looks and sees nothing: the drone has got away.
            // Start again from the place it was last seen, on the ring it can have reached.
            let last = self.tracks[k].last_at;
            let reach = (50.0 + CRUISE * (time - self.tracks[k].seen_at).max(0.0)).min(2.0 * self.rules.field_radius);
            let mut guesses = Vec::with_capacity(PARTICLES);
            while guesses.len() < PARTICLES {
                let mut at = last;
                for _ in 0..20 {
                    let a = self.next() * std::f32::consts::TAU;
                    let r = reach * self.next().sqrt();
                    at = last + Vec2::new(a.cos(), a.sin()) * r;
                    if at.length() <= self.rules.field_radius && !seen(at) {
                        break;
                    }
                }
                let g = self.guess_at(at, Vec2::ZERO, 0.0);
                guesses.push(g);
            }
            self.tracks[k].guesses = guesses;
            return;
        }
        let weights: Vec<f32> = self.tracks[k].guesses.iter().map(|g| if seen(g.at) { UNSEEN } else { 1.0 }).collect();
        let total: f32 = weights.iter().sum();
        // Systematic resampling.
        let step = total / PARTICLES as f32;
        let mut pick = self.next() * step;
        let (mut acc, mut i) = (weights[0], 0);
        let old = self.tracks[k].guesses.clone();
        let mut out = Vec::with_capacity(PARTICLES);
        for _ in 0..PARTICLES {
            while pick > acc && i + 1 < old.len() {
                i += 1;
                acc += weights[i];
            }
            let mut g = old[i];
            g.at += Vec2::new(self.gauss(), self.gauss()) * 3.0;
            out.push(g);
            pick += step;
        }
        self.tracks[k].guesses = out;
    }

    /// The average place of enemy drone `k`.
    pub fn mean(&self, k: usize) -> Vec2 {
        let g = &self.tracks[k].guesses;
        g.iter().map(|g| g.at).sum::<Vec2>() / g.len() as f32
    }

    /// How far the guesses for enemy drone `k` are from their average (m, root mean square).
    pub fn spread(&self, k: usize) -> f32 {
        let (m, g) = (self.mean(k), &self.tracks[k].guesses);
        (g.iter().map(|g| g.at.distance_squared(m)).sum::<f32>() / g.len() as f32).sqrt()
    }

    /// The chance that enemy drone `k` is within `radius` of `at` (0 to 1).
    pub fn chance_within(&self, k: usize, at: Vec2, radius: f32) -> f32 {
        let g = &self.tracks[k].guesses;
        g.iter().filter(|g| g.at.distance(at) <= radius).count() as f32 / g.len() as f32
    }

    /// One guess for enemy drone `k`, the `i`-th of its cloud: where it is, the tower it flies to,
    /// and how fast.
    pub fn guess(&self, k: usize, i: usize) -> (Vec2, usize, f32) {
        let g = &self.tracks[k].guesses;
        let g = g[i % g.len()];
        (g.at, g.goal, g.speed)
    }

    /// How many enemy drones are within `radius` of `at`, on average.
    pub fn expected_near(&self, at: Vec2, radius: f32) -> f32 {
        (0..self.tracks.len()).map(|k| self.chance_within(k, at, radius)).sum()
    }

    /// The team that holds the belief.
    pub fn team(&self) -> Team {
        self.team
    }

    /// How many enemy drones there are.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EnemyDrone, Game, Phase};

    fn view(time: f32) -> (Game, TeamView) {
        let mut g = Game::new(Rules::default());
        g.look();
        let mut v = g.view(Team::Blue);
        v.time = time;
        (g, v)
    }

    fn sighting(k: usize, at: Vec2, vel: Vec2, seen: bool, age: f32) -> EnemyDrone {
        EnemyDrone { k, at, alt: 60.0, vel, seen, age }
    }

    #[test]
    fn at_the_start_an_enemy_drone_is_believed_to_be_at_its_base() {
        let rules = Rules::default();
        let b = Belief::new(rules, Team::Blue);
        let red_base = rules.base(Team::Red);
        assert_eq!(b.len(), rules.drones);
        for k in 0..rules.drones {
            assert!(b.mean(k).distance(red_base) < 30.0, "{k}: {:?}", b.mean(k));
            assert!(b.chance_within(k, red_base, 60.0) > 0.9);
        }
    }

    #[test]
    fn a_drone_the_team_sees_is_where_it_is_seen() {
        let (_, mut v) = view(10.0);
        let p = Vec2::new(100.0, -50.0);
        v.enemies = vec![sighting(1, p, Vec2::new(-10.0, 0.0), true, 0.0)];
        let mut b = Belief::new(Rules::default(), Team::Blue);
        b.update(&v);
        assert!(b.mean(1).distance(p) < 10.0, "{:?}", b.mean(1));
        assert!(b.spread(1) < 20.0, "{}", b.spread(1));
    }

    #[test]
    fn the_guesses_spread_out_while_the_drone_is_out_of_sight() {
        let (_, mut v) = view(10.0);
        let p = Vec2::new(200.0, 0.0);
        v.enemies = vec![sighting(1, p, Vec2::new(-12.0, 0.0), true, 0.0)];
        let mut b = Belief::new(Rules::default(), Team::Blue);
        b.update(&v);
        let seen = b.spread(1);
        for t in 11..=30 {
            v.time = t as f32;
            v.enemies = vec![sighting(1, p, Vec2::new(-12.0, 0.0), false, v.time - 10.0)];
            b.update(&v);
        }
        assert!(b.spread(1) > seen + 30.0, "{} then {}", seen, b.spread(1));
    }

    #[test]
    fn a_place_the_team_looks_at_and_sees_nothing_is_ruled_out() {
        let (g, mut v) = view(10.0);
        // Seen 100 m north of a Blue drone, going away; then not seen while the drone looks.
        let me = Vec2::new(0.0, 0.0);
        let drone = g.drone(Team::Blue, 0);
        v.drones[0].at = me;
        v.drones[0].phase = Phase::Flying;
        let _ = drone;
        let start = me + Vec2::new(100.0, 0.0);
        v.enemies = vec![sighting(1, start, Vec2::ZERO, true, 0.0)];
        let mut b = Belief::new(Rules::default(), Team::Blue);
        b.update(&v);
        for t in 11..=15 {
            v.time = t as f32;
            v.enemies = vec![sighting(1, start, Vec2::ZERO, false, v.time - 10.0)];
            b.update(&v);
        }
        let near = b.chance_within(1, me, Rules::default().drone_vision - 5.0);
        assert!(near < 0.02, "the drone looks there and sees nothing, yet the chance is {near}");
    }

    #[test]
    fn the_chances_of_a_drone_add_up_to_one() {
        let (_, v) = view(3.0);
        let mut b = Belief::new(Rules::default(), Team::Blue);
        b.update(&v);
        let all = b.chance_within(2, Vec2::ZERO, 5000.0);
        assert!((all - 1.0).abs() < 1e-3, "{all}");
    }

    #[test]
    fn the_same_views_give_the_same_belief() {
        let (_, mut v) = view(10.0);
        v.enemies = vec![sighting(0, Vec2::new(50.0, 50.0), Vec2::new(5.0, 5.0), true, 0.0)];
        let (mut a, mut b) = (Belief::new(Rules::default(), Team::Blue), Belief::new(Rules::default(), Team::Blue));
        for t in 0..5 {
            v.time = 10.0 + t as f32;
            a.update(&v);
            b.update(&v);
        }
        assert_eq!(a.mean(0), b.mean(0));
        assert_eq!(a.spread(0), b.spread(0));
    }
}
