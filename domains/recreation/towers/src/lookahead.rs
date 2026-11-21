//! Thinking ahead: a small model of the game that a captain rolls forward from what it believes
//! ([`Sim`]), and a search over changes to its orders ([`best_change`]).
//!
//! The model is not the referee. It keeps what decides a game: where the drones are, which tower
//! each flies to, how fast a tower is won (5 % of the bar each second for each unit of weight
//! more in the circle, and only by a drone that has come down below the treetops), how a bar
//! drifts back when nobody is in the circle, and how many points that gives. The enemy drones are
//! guesses drawn from the team's [`Belief`]: each flies to its tower and, when its team holds it,
//! to the nearest tower it does not hold; so does one of ours that has won its tower.

use crate::flight::{ORBIT_RADIUS, low_height, orbit_height};
use crate::{Belief, Order, Phase, Rules, TeamView};
use glam::Vec2;

/// A drone flies about this fast (m/s).
const CRUISE: f32 = 16.0;
/// How far ahead the model looks (s), and its step (s).
pub const HORIZON: f32 = 45.0;
const STEP: f32 = 1.0;
/// How many futures the search draws for the enemy.
const FUTURES: usize = 8;
/// The value of each point of progress on a tower's bar, in points a second held for a minute.
const PROGRESS: f32 = 10.0;
/// A drone climbs or sinks this fast (m/s), and a drone that takes off climbs to this height (m),
/// the drones of a team leaving this many seconds apart: the arena's and the server's flight.
const VERTICAL: f32 = 3.0;
const TAKEOFF_HEIGHT: f32 = 15.0;
const TAKEOFF_APART: f32 = 2.0;

/// What the model keeps of the game. Each rule that the referee has, and that a drone's flight
/// makes matter, is a switch, so that a measurement can tell what each is worth. The default is
/// what measured best: flight and charging on; drift and redeploy are right by the referee but
/// measured worse (plans/actors.md, step 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Model {
    /// A drone flies at its orbit height and comes down below the treetops only within reach of its
    /// tower, and only a low drone counts in a circle; a drone on the ground climbs out first.
    pub flight: bool,
    /// A circle with nobody in it drifts back across its bar (`Rules::decay_time`).
    pub drift: bool,
    /// A drone on its way to charge, or down, is not at a tower.
    pub charging: bool,
    /// A drone that has won its tower goes on to the nearest one the team does not hold, as the
    /// commander sends it, and does not stay.
    pub redeploy: bool,
    /// A drone in the circle of the tower it is taking, which the team does not hold yet, is not
    /// moved: what it has won there is lost the moment it leaves, and a measurement of the
    /// proposals that did so (plans/actors.md) found them the worst.
    pub spare_capturing: bool,
    /// How far ahead the model looks (s).
    pub horizon: f32,
    /// How many futures of the enemy it draws.
    pub futures: usize,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            flight: true,
            drift: false,
            charging: true,
            redeploy: false,
            spare_capturing: false,
            horizon: HORIZON,
            futures: FUTURES,
        }
    }
}

impl Model {
    /// The model as the first version of the lookahead played it: no flight, no drift, no
    /// charging, no going on.
    pub fn first() -> Model {
        Model {
            flight: false,
            drift: false,
            charging: false,
            redeploy: false,
            spare_capturing: false,
            ..Model::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Mover {
    at: Vec2,
    alt: f32,
    goal: Option<usize>,
    weight: f32,
    /// The height it flies at, and the height it takes a tower at (m).
    high: f32,
    low: f32,
    /// Seconds before it is in the air (on the ground at the start, or climbing out).
    grounded: f32,
}

#[derive(Clone, Copy, Debug)]
struct SimTower {
    at: Vec2,
    points: f32,
    /// From −1 (the enemy) to +1 (our team).
    control: f32,
    /// +1 our team holds it, −1 the enemy, 0 nobody.
    holder: i8,
}

/// The game as the model plays it.
#[derive(Clone, Debug)]
pub struct Sim {
    circle: f32,
    ceiling: f32,
    capture: f32,
    decay: f32,
    model: Model,
    towers: Vec<SimTower>,
    ours: Vec<Mover>,
    theirs: Vec<Mover>,
    /// Points of our team minus the enemy's so far.
    pub score: f32,
}

impl Sim {
    /// From the team's map and one guess for each enemy drone. `orders` are our drones' orders.
    #[cfg(test)]
    pub fn new(rules: &Rules, view: &TeamView, orders: &[Option<Order>], belief: &Belief, future: usize) -> Sim {
        Sim::with_model(rules, view, orders, belief, future, Model::default())
    }

    /// The same, with the rules of the model chosen.
    pub fn with_model(
        rules: &Rules,
        view: &TeamView,
        orders: &[Option<Order>],
        belief: &Belief,
        future: usize,
        model: Model,
    ) -> Sim {
        let sign = view.team.sign();
        let towers: Vec<SimTower> = view
            .towers
            .iter()
            .map(|t| SimTower {
                at: t.at,
                points: rules.points(t.kind),
                control: t.control * sign,
                holder: match t.holder {
                    Some(h) if h == view.team => 1,
                    Some(_) => -1,
                    None => 0,
                },
            })
            .collect();
        let ours = view
            .drones
            .iter()
            .map(|d| {
                let grounded = match d.phase {
                    Phase::Ground => {
                        (TAKEOFF_APART * d.k as f32 - view.time).max(0.0) + (TAKEOFF_HEIGHT - d.alt).max(0.0) / VERTICAL
                    }
                    _ => 0.0,
                };
                Mover {
                    at: d.at,
                    alt: if model.flight { d.alt } else { 0.0 },
                    goal: match orders.get(d.k).copied().flatten() {
                        Some(Order::Take(t)) => Some(t),
                        _ => None,
                    },
                    weight: if d.phase == Phase::Down { 0.0 } else { rules.class(d.k).weight() },
                    high: orbit_height(view.team, d.k),
                    low: low_height(view.team, d.k),
                    grounded: if model.flight { grounded } else { 0.0 },
                }
            })
            .collect();
        let enemy = view.team.other();
        let theirs = (0..belief.len())
            .map(|k| {
                let (at, goal, _) = belief.guess(k, (future * 16 + k * 5) % 128);
                let (high, low) = (orbit_height(enemy, k), low_height(enemy, k));
                // A guess at its tower is taken to be circling it low; one on its way, high.
                let alt = if at.distance(towers[goal].at) <= ORBIT_RADIUS + 20.0 { low } else { high };
                Mover { at, alt, goal: Some(goal), weight: rules.class(k).weight(), high, low, grounded: 0.0 }
            })
            .collect();
        Sim {
            circle: rules.circle_radius,
            ceiling: rules.circle_ceiling,
            capture: rules.capture_time,
            decay: rules.decay_time,
            model,
            towers,
            ours,
            theirs,
            score: 0.0,
        }
    }

    /// Drone `k` of ours is on its way to charge: it is at no tower.
    pub fn charging(&mut self, k: usize) {
        if self.model.charging {
            self.ours[k].goal = None;
            self.ours[k].weight = 0.0;
        }
    }

    /// Plays `secs` seconds.
    pub fn run(&mut self, secs: f32) {
        let mut left = secs;
        while left > 1e-4 {
            let dt = left.min(STEP);
            left -= dt;
            self.step(dt);
        }
    }

    /// Does the mover count in the circle of the tower at `at`?
    fn counts(&self, m: &Mover, at: Vec2) -> bool {
        m.grounded <= 0.0 && m.at.distance(at) <= self.circle && (!self.model.flight || m.alt < self.ceiling)
    }

    /// One mover flies for `dt` towards its goal.
    fn fly(&self, m: &mut Mover, dt: f32) {
        if m.grounded > 0.0 {
            m.grounded -= dt;
            return;
        }
        let Some(t) = m.goal else { return };
        m.at = toward(m.at, self.towers[t].at, CRUISE * dt);
        if self.model.flight {
            let near = m.at.distance(self.towers[t].at) <= ORBIT_RADIUS + 20.0;
            let want = if near { m.low } else { m.high };
            m.alt += (want - m.alt).clamp(-VERTICAL * dt, VERTICAL * dt);
        }
    }

    fn step(&mut self, dt: f32) {
        for i in 0..self.ours.len() {
            let mut m = self.ours[i];
            self.fly(&mut m, dt);
            if self.model.redeploy
                && let Some(t) = m.goal
                && m.at.distance(self.towers[t].at) < 5.0
                && self.towers[t].holder == 1
                && self.towers[t].control >= 1.0
            {
                // The tower is ours: on to the nearest one the team does not hold, one that no
                // other drone of ours is on its way to first.
                let at = m.at;
                let taken = |u: usize| self.ours.iter().enumerate().any(|(j, o)| j != i && o.goal == Some(u));
                let nearest = |free_only: bool| {
                    (0..self.towers.len())
                        .filter(|&u| self.towers[u].holder != 1 && (!free_only || !taken(u)))
                        .min_by(|&a, &b| at.distance(self.towers[a].at).total_cmp(&at.distance(self.towers[b].at)))
                };
                m.goal = nearest(true).or_else(|| nearest(false)).or(m.goal);
            }
            self.ours[i] = m;
        }
        for i in 0..self.theirs.len() {
            let mut m = self.theirs[i];
            if let Some(t) = m.goal {
                let there = m.at.distance(self.towers[t].at) < 5.0;
                if there && self.towers[t].holder == -1 {
                    // The tower is theirs: on to the nearest one they do not hold.
                    m.goal = (0..self.towers.len())
                        .filter(|&u| self.towers[u].holder != -1)
                        .min_by(|&a, &b| m.at.distance(self.towers[a].at).total_cmp(&m.at.distance(self.towers[b].at)))
                        .or(Some(t));
                }
                self.fly(&mut m, dt);
            }
            self.theirs[i] = m;
        }
        for ti in 0..self.towers.len() {
            let at = self.towers[ti].at;
            let ours: Vec<&Mover> = self.ours.iter().filter(|m| self.counts(m, at)).collect();
            let theirs: Vec<&Mover> = self.theirs.iter().filter(|m| self.counts(m, at)).collect();
            let diff = ours.iter().map(|m| m.weight).sum::<f32>() - theirs.iter().map(|m| m.weight).sum::<f32>();
            let empty = ours.is_empty() && theirs.is_empty();
            let t = &mut self.towers[ti];
            if empty && self.model.drift && self.decay > 0.0 {
                // Nobody in the circle: the bar drifts back towards its holder's end, or the middle.
                let end = f32::from(t.holder);
                t.control += (end - t.control).clamp(-dt / self.decay, dt / self.decay);
            } else {
                t.control = (t.control + diff * dt / self.capture).clamp(-1.0, 1.0);
            }
            if t.control >= 1.0 {
                t.holder = 1;
            } else if t.control <= -1.0 {
                t.holder = -1;
            } else if (t.holder == 1 && t.control <= 0.0) || (t.holder == -1 && t.control >= 0.0) {
                t.holder = 0;
            }
            self.score += f32::from(t.holder) * t.points * dt;
        }
    }

    /// How well it stands: the points so far, and what the bars are worth.
    pub fn value(&self) -> f32 {
        self.score + PROGRESS * self.towers.iter().map(|t| t.control * t.points).sum::<f32>()
    }
}

fn toward(from: Vec2, to: Vec2, step: f32) -> Vec2 {
    let d = to - from;
    if d.length() <= step { to } else { from + d.normalize() * step }
}

/// The one change to `orders` that the model likes best: drone `k` goes to tower `t` instead, and
/// the points it expects to gain by it: `(k, t, gain)`. `None` if there is no change to try. Whether
/// the gain is worth making is for the commander to say. `free` says which drones may be given a new
/// order, and `away` which towers a drone must stay away from.
#[cfg(test)]
pub fn best_change(
    rules: &Rules,
    view: &TeamView,
    orders: &[Option<Order>],
    free: &[bool],
    away: &[Vec<bool>],
    belief: &Belief,
) -> Option<(usize, usize, f32)> {
    best_change_with(rules, view, orders, free, &vec![false; orders.len()], away, belief, Model::default())
}

/// The same with the model's rules chosen, and the drones `hold`ing back from a move.
#[allow(clippy::too_many_arguments)]
pub fn best_change_with(
    rules: &Rules,
    view: &TeamView,
    orders: &[Option<Order>],
    free: &[bool],
    hold: &[bool],
    away: &[Vec<bool>],
    belief: &Belief,
    model: Model,
) -> Option<(usize, usize, f32)> {
    let towers = view.towers.len();
    let base = rules.base(view.team);
    // A drone is not sent where its battery cannot last the flight, the capture and the way home.
    let lasts = |k: usize, t: usize| {
        let d = &view.drones[k];
        let tower = &view.towers[t];
        let need = 1.0 - tower.control * view.team.sign();
        let capture = rules.capture_time * need / rules.class(k).weight();
        rules.battery(rules.class(k)) <= 0.0
            || d.battery >= rules.bingo + (d.at.distance(tower.at) + tower.at.distance(base)) / CRUISE + capture + 15.0
    };
    let capturing = |k: usize| {
        matches!(orders[k], Some(Order::Take(t))
            if view.drones[k].at.distance(view.towers[t].at) <= rules.circle_radius
                && view.towers[t].holder != Some(view.team))
    };
    let mut candidates: Vec<(usize, usize)> = vec![];
    for k in (0..orders.len()).filter(|&k| free[k] && !hold[k] && !(model.spare_capturing && capturing(k))) {
        for (t, &avoid) in away[k].iter().enumerate().take(towers) {
            if orders[k] != Some(Order::Take(t)) && !avoid && lasts(k, t) {
                candidates.push((k, t));
            }
        }
    }
    // A drone that is not free and in contact is on its way to charge (or down): it is at no tower.
    let charging: Vec<usize> = (0..orders.len())
        .filter(|&k| !free[k] && view.drones[k].linked && view.drones[k].phase != Phase::Down)
        .collect();
    let play = |orders: &[Option<Order>], future: usize| {
        let mut sim = Sim::with_model(rules, view, orders, belief, future, model);
        for &k in &charging {
            sim.charging(k);
        }
        sim.run(model.horizon);
        sim.value()
    };
    let mut gain = vec![0.0f32; candidates.len()];
    for future in 0..model.futures {
        let base = play(orders, future);
        for (c, &(k, t)) in candidates.iter().enumerate() {
            let mut changed = orders.to_vec();
            changed[k] = Some(Order::Take(t));
            gain[c] += play(&changed, future) - base;
        }
    }
    let (best, total) =
        gain.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).filter(|&(_, g)| g.is_finite())?;
    let (k, t) = candidates[best];
    Some((k, t, total / model.futures as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Game, Team};

    fn setup() -> (Rules, Game) {
        let rules = Rules { drones: 3, ..Rules::default() };
        let mut g = Game::new(rules);
        g.look();
        (rules, g)
    }

    fn fly_to(g: &mut Game, team: Team, k: usize, at: Vec2) {
        let i = g.drone(team, k);
        g.drones[i] = crate::Drone { at, alt: 50.0, phase: Phase::Flying, ..g.drones[i] };
    }

    /// A drone in the tower's circle, low.
    fn low_at(g: &mut Game, team: Team, k: usize, at: Vec2) {
        let i = g.drone(team, k);
        g.drones[i] = crate::Drone { at, alt: 15.0, phase: Phase::Flying, ..g.drones[i] };
    }

    fn no_away(view: &TeamView) -> Vec<Vec<bool>> {
        vec![vec![false; view.towers.len()]; view.drones.len()]
    }

    #[test]
    fn one_drone_takes_a_neutral_tower_in_the_capture_time() {
        let (rules, mut g) = setup();
        let west = g.tower("west");
        let at = g.towers[west].at;
        low_at(&mut g, Team::Blue, 0, at);
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let mut orders = vec![None; 3];
        orders[0] = Some(Order::Take(west));
        let mut sim = Sim::new(&rules, &view, &orders, &belief, 0);
        sim.run(19.0);
        assert!(sim.towers[west].holder == 0 && sim.towers[west].control > 0.9);
        sim.run(2.0);
        assert_eq!(sim.towers[west].holder, 1);
    }

    #[test]
    fn a_drone_that_comes_to_a_tower_high_wins_it_only_after_it_has_come_down() {
        let (rules, mut g) = setup();
        let west = g.tower("west");
        let at = g.towers[west].at;
        // At the tower, at its orbit height: it comes down at 3 m/s to the treetops before it counts.
        fly_to(&mut g, Team::Blue, 0, at);
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let orders = [Some(Order::Take(west)), None, None];
        let mut sim = Sim::new(&rules, &view, &orders, &belief, 0);
        sim.run(10.0);
        assert!(sim.towers[west].control < 0.05, "still high: {}", sim.towers[west].control);
        sim.run(40.0);
        assert_eq!(sim.towers[west].holder, 1);
        // The model before the measurement had no such wait.
        let mut first = Sim::with_model(&rules, &view, &orders, &belief, 0, Model::first());
        first.run(10.0);
        assert!(first.towers[west].control > 0.45, "{}", first.towers[west].control);
    }

    #[test]
    fn a_bar_that_nobody_is_at_drifts_back_to_its_holders_end_or_the_middle() {
        let (rules, g) = setup();
        let west = g.tower("west");
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let mut sim =
            Sim::with_model(&rules, &view, &[None, None, None], &belief, 0, Model { drift: true, ..Model::default() });
        sim.theirs.clear();
        sim.towers[west].control = 0.5;
        sim.run(30.0);
        assert!(
            (sim.towers[west].control - 0.2).abs() < 1e-3,
            "a bar nobody holds drifts to the middle: {}",
            sim.towers[west].control
        );
        // A held bar drifts back to its end.
        sim.towers[west].holder = 1;
        sim.towers[west].control = 0.5;
        sim.run(30.0);
        assert!((sim.towers[west].control - 0.8).abs() < 1e-3, "{}", sim.towers[west].control);
    }

    #[test]
    fn holding_a_tower_scores_its_points_each_second() {
        let (rules, g) = setup();
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        // Blue holds its home tower and Red holds its own: level. With the drones idle for 10 s the
        // score stays 0.
        let mut sim = Sim::new(&rules, &view, &[None, None, None], &belief, 0);
        sim.run(10.0);
        assert!(sim.score.abs() < 1e-3, "{}", sim.score);
        // Take Red's home tower away: Red's points stop and ours go on.
        let red_home = g.tower("Red home");
        sim.towers[red_home].holder = 0;
        sim.towers[red_home].control = 0.0;
        sim.run(10.0);
        assert!((sim.score - 10.0).abs() < 1e-3, "{}", sim.score);
    }

    #[test]
    fn equal_weight_in_a_circle_stops_the_bar() {
        let (rules, mut g) = setup();
        let west = g.tower("west");
        let at = g.towers[west].at;
        low_at(&mut g, Team::Blue, 0, at);
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let mut sim = Sim::new(&rules, &view, &[Some(Order::Take(west)), None, None], &belief, 0);
        sim.theirs[0] = Mover { at, alt: 15.0, goal: Some(west), weight: 1.0, high: 60.0, low: 15.0, grounded: 0.0 };
        sim.run(30.0);
        assert!(sim.towers[west].control.abs() < 1e-3, "{}", sim.towers[west].control);
    }

    #[test]
    fn a_drone_that_has_won_its_tower_goes_on_to_the_nearest_one_the_team_does_not_hold() {
        let (rules, mut g) = setup();
        let west = g.tower("west");
        let at = g.towers[west].at;
        low_at(&mut g, Team::Blue, 0, at);
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let all = Model { redeploy: true, ..Model::default() };
        let mut sim = Sim::with_model(&rules, &view, &[Some(Order::Take(west)), None, None], &belief, 0, all);
        sim.theirs.clear();
        sim.run(22.0);
        assert_eq!(sim.towers[west].holder, 1);
        assert_ne!(sim.ours[0].goal, Some(west), "it has gone on");
        let mut stays =
            Sim::with_model(&rules, &view, &[Some(Order::Take(west)), None, None], &belief, 0, Model::first());
        stays.theirs.clear();
        stays.run(22.0);
        assert_eq!(stays.ours[0].goal, Some(west));
    }

    #[test]
    fn it_prefers_the_free_tower_over_the_one_three_enemies_sit_on() {
        let (rules, mut g) = setup();
        let (west, east) = (g.tower("west"), g.tower("east"));
        let (west_at, east_at) = (g.towers[west].at, g.towers[east].at);
        // Our drone 0 is told to take the west tower, where the belief says the enemy sits; the east
        // tower is free. Everyone else is busy.
        fly_to(&mut g, Team::Blue, 0, (west_at + east_at) * 0.5);
        let home = g.towers[0].at;
        for k in 1..3 {
            fly_to(&mut g, Team::Blue, k, home);
        }
        let mut view = g.view(Team::Blue);
        // The belief: the enemy was seen at the west tower.
        let mut belief = Belief::new(rules, Team::Blue);
        view.time = 5.0;
        view.enemies = (0..3)
            .map(|k| crate::EnemyDrone {
                class: None,
                payload: None,
                k,
                at: west_at,
                alt: 50.0,
                vel: Vec2::ZERO,
                seen: true,
                age: 0.0,
            })
            .collect();
        belief.update(&view);
        let orders = vec![Some(Order::Take(west)), Some(Order::Take(0)), Some(Order::Take(0))];
        let free = vec![true, false, false];
        let change = best_change(&rules, &view, &orders, &free, &no_away(&view), &belief);
        let centre = g.tower("centre");
        // A free tower: the east, or the centre, which is worth more. Not the west.
        assert!(matches!(change, Some((0, t, gain)) if (t == east || t == centre) && gain > 2.0), "{change:?}");
    }

    #[test]
    fn it_has_no_change_to_try_when_no_drone_is_free() {
        let (rules, g) = setup();
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let centre = g.tower("centre");
        let orders = vec![Some(Order::Take(centre)), None, None];
        let free = vec![false; 3];
        assert_eq!(best_change(&rules, &view, &orders, &free, &no_away(&view), &belief), None);
    }
}
