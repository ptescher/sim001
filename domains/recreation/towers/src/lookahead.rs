//! Thinking ahead: a small model of the game that a captain rolls forward from what it believes
//! ([`Sim`]), and a search over changes to its orders ([`best_change`]).
//!
//! The model is not the referee. It keeps what decides a game: where the drones are, which tower
//! each flies to, how fast a tower is won (5 % of the bar each second for each unit of weight
//! more in the circle) and how many points that gives. The enemy drones are guesses drawn from the
//! team's [`Belief`]: each flies to its tower and, when its team holds it, to the nearest tower it
//! does not hold.

use crate::{Belief, Order, Phase, Rules, TeamView};
use glam::Vec2;

/// A drone flies about this fast (m/s).
const CRUISE: f32 = 16.0;
/// How far ahead the model looks (s), and its step (s).
pub const HORIZON: f32 = 45.0;
const STEP: f32 = 1.0;
/// A change of orders must be worth this many points to be made.
const MARGIN: f32 = 2.0;
/// How many futures the search draws for the enemy.
const FUTURES: usize = 8;
/// The value of each point of progress on a tower's bar, in points a second held for a minute.
const PROGRESS: f32 = 10.0;

#[derive(Clone, Copy, Debug)]
struct Mover {
    at: Vec2,
    goal: Option<usize>,
    weight: f32,
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
    capture: f32,
    towers: Vec<SimTower>,
    ours: Vec<Mover>,
    theirs: Vec<Mover>,
    /// Points of our team minus the enemy's so far.
    pub score: f32,
}

impl Sim {
    /// From the team's map and one guess for each enemy drone. `orders` are our drones' orders.
    pub fn new(rules: &Rules, view: &TeamView, orders: &[Option<Order>], belief: &Belief, future: usize) -> Sim {
        let sign = view.team.sign();
        let towers = view
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
            .map(|d| Mover {
                at: d.at,
                goal: match orders.get(d.k).copied().flatten() {
                    Some(Order::Take(t)) => Some(t),
                    _ => None,
                },
                weight: if d.phase == Phase::Down { 0.0 } else { rules.class(d.k).weight() },
            })
            .collect();
        let theirs = (0..belief.len())
            .map(|k| {
                let (at, goal, _) = belief.guess(k, (future * 16 + k * 5) % 128);
                Mover { at, goal: Some(goal), weight: rules.class(k).weight() }
            })
            .collect();
        Sim { circle: rules.circle_radius, capture: rules.capture_time, towers, ours, theirs, score: 0.0 }
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

    fn step(&mut self, dt: f32) {
        for m in &mut self.ours {
            if let Some(t) = m.goal {
                m.at = toward(m.at, self.towers[t].at, CRUISE * dt);
            }
        }
        for i in 0..self.theirs.len() {
            let m = self.theirs[i];
            if let Some(t) = m.goal {
                let there = m.at.distance(self.towers[t].at) < 5.0;
                if there && self.towers[t].holder == -1 {
                    // The tower is theirs: on to the nearest one they do not hold.
                    self.theirs[i].goal = (0..self.towers.len())
                        .filter(|&u| self.towers[u].holder != -1)
                        .min_by(|&a, &b| m.at.distance(self.towers[a].at).total_cmp(&m.at.distance(self.towers[b].at)))
                        .or(Some(t));
                }
                let goal = self.theirs[i].goal.unwrap_or(t);
                self.theirs[i].at = toward(m.at, self.towers[goal].at, CRUISE * dt);
            }
        }
        for t in &mut self.towers {
            let weight = |ms: &[Mover]| -> f32 {
                ms.iter().filter(|m| m.at.distance(t.at) <= self.circle).map(|m| m.weight).sum()
            };
            let diff = weight(&self.ours) - weight(&self.theirs);
            t.control = (t.control + diff * dt / self.capture).clamp(-1.0, 1.0);
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

/// The one change to `orders` that the model likes best: drone `k` goes to tower `t` instead. `None`
/// if no change is worth [`MARGIN`] points. `free` says which drones may be given a new order, and
/// `away` which towers a drone must stay away from.
pub fn best_change(
    rules: &Rules,
    view: &TeamView,
    orders: &[Option<Order>],
    free: &[bool],
    away: &[Vec<bool>],
    belief: &Belief,
) -> Option<(usize, usize)> {
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
    let mut candidates: Vec<(usize, usize)> = vec![];
    for k in (0..orders.len()).filter(|&k| free[k]) {
        for t in 0..towers {
            if orders[k] != Some(Order::Take(t)) && !away[k][t] && lasts(k, t) {
                candidates.push((k, t));
            }
        }
    }
    let mut gain = vec![0.0f32; candidates.len()];
    for future in 0..FUTURES {
        let mut base = Sim::new(rules, view, orders, belief, future);
        base.run(HORIZON);
        let base = base.value();
        for (c, &(k, t)) in candidates.iter().enumerate() {
            let mut changed = orders.to_vec();
            changed[k] = Some(Order::Take(t));
            let mut sim = Sim::new(rules, view, &changed, belief, future);
            sim.run(HORIZON);
            gain[c] += sim.value() - base;
        }
    }
    let (best, total) =
        gain.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).filter(|&(_, g)| g.is_finite())?;
    (total / FUTURES as f32 > MARGIN).then(|| candidates[best])
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

    fn no_away(view: &TeamView) -> Vec<Vec<bool>> {
        vec![vec![false; view.towers.len()]; view.drones.len()]
    }

    #[test]
    fn one_drone_takes_a_neutral_tower_in_the_capture_time() {
        let (rules, mut g) = setup();
        let west = g.tower("west");
        let at = g.towers[west].at;
        fly_to(&mut g, Team::Blue, 0, at);
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
        fly_to(&mut g, Team::Blue, 0, at);
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let mut sim = Sim::new(&rules, &view, &[Some(Order::Take(west)), None, None], &belief, 0);
        sim.theirs[0] = Mover { at, goal: Some(west), weight: 1.0 };
        sim.run(30.0);
        assert!(sim.towers[west].control.abs() < 1e-3, "{}", sim.towers[west].control);
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
            .map(|k| crate::EnemyDrone { k, at: west_at, alt: 50.0, vel: Vec2::ZERO, seen: true, age: 0.0 })
            .collect();
        belief.update(&view);
        let orders = vec![Some(Order::Take(west)), Some(Order::Take(0)), Some(Order::Take(0))];
        let free = vec![true, false, false];
        let change = best_change(&rules, &view, &orders, &free, &no_away(&view), &belief);
        let centre = g.tower("centre");
        // A free tower: the east, or the centre, which is worth more. Not the west.
        assert!(matches!(change, Some((0, t)) if t == east || t == centre), "{change:?}");
    }

    #[test]
    fn it_leaves_the_orders_alone_when_nothing_is_better() {
        let (rules, g) = setup();
        let view = g.view(Team::Blue);
        let belief = Belief::new(rules, Team::Blue);
        let centre = g.tower("centre");
        let orders = vec![Some(Order::Take(centre)), None, None];
        let free = vec![false; 3];
        assert_eq!(best_change(&rules, &view, &orders, &free, &no_away(&view), &belief), None);
    }
}
