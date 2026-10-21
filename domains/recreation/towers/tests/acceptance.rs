//! The "Towers" section of the suite (expectations/09-towers), run with no window: the real
//! referee (`recreation_towers::Game`), with the drones put where each step says. The rules do
//! not depend on how a drone flies, so a drone here stays where it was put; the host's tests fly
//! the game with ArduPlane SITL.

use expectation_harness::{Acceptance, Bindings, RunOptions};
use expectation_model::Value;
use expectation_story::Status;
use glam::Vec2;
use recreation_towers::{Commander, Game, Order, Outcome, Phase, Rules, Team};
use std::path::Path;

/// The referee's step (s): the host steps it with the physics, faster than this.
const DT: f32 = 0.1;
/// The height a drone circles a tower at (m).
const ALT: f32 = 50.0;
/// The Red scout: Red's first drone.
const SCOUT: usize = 0;

struct World {
    game: Game,
    /// Blue's computer commander.
    blue: Commander,
}

impl World {
    fn new() -> World {
        let rules = Rules::default();
        World { game: Game::new(rules), blue: Commander::new(rules) }
    }

    /// Blue drone `k`'s order, as the spec words it.
    fn order_of(&self, k: usize) -> String {
        match self.blue.order(k) {
            None => "none".into(),
            Some(Order::Take(t)) => format!("take the {} tower", self.game.towers[t].name),
            Some(o) => format!("{o:?}"),
        }
    }

    fn run(&mut self, secs: f32) {
        for _ in 0..(secs / DT).round() as usize {
            self.game.step(DT);
        }
    }

    fn tower(&self, name: &str) -> &recreation_towers::Tower {
        &self.game.towers[self.game.tower(name)]
    }

    fn tower_mut(&mut self, name: &str) -> &mut recreation_towers::Tower {
        let i = self.game.tower(name);
        &mut self.game.towers[i]
    }

    /// Puts drone `i` at `at`, flying at `alt` or on the ground. In a game the teams look all the
    /// time, so they see it there at once.
    fn put(&mut self, i: usize, at: Vec2, alt: Option<f32>) {
        let d = &mut self.game.drones[i];
        d.at = at;
        d.alt = alt.unwrap_or(0.0);
        d.phase = if alt.is_some() { Phase::Flying } else { Phase::Ground };
        self.game.look();
    }

    /// Back to its base, on the ground.
    fn home(&mut self, i: usize) {
        let fresh = Game::new(self.game.rules);
        self.game.drones[i] = fresh.drones[i];
    }

    /// The order a team's drones go to a circle in: Red keeps its scout for last.
    fn order(team: Team) -> [usize; 3] {
        match team {
            Team::Blue => [0, 1, 2],
            Team::Red => [1, 2, 0],
        }
    }

    /// `n` of `team`'s drones in the circle of `tower` (on the circle's edge, 40 m out, spread
    /// round it), flying or on the ground; the team's drones that were there and are not wanted
    /// go back to the base.
    fn in_circle(&mut self, team: Team, tower: &str, n: usize, flying: bool) {
        let centre = self.tower(tower).at;
        let order = World::order(team);
        let ks: Vec<usize> = if flying { order.to_vec() } else { order.iter().rev().copied().collect() };
        for (j, k) in ks.into_iter().enumerate() {
            let i = self.game.drone(team, k);
            let d = self.game.drones[i];
            let here = (d.phase == Phase::Flying) == flying && (d.at - centre).length() <= 60.0;
            if j < n {
                let a = j as f32 * 2.1;
                self.put(i, centre + 40.0 * Vec2::new(a.cos(), a.sin()), flying.then_some(ALT));
            } else if here {
                self.home(i);
            }
        }
    }

    fn count(&self, team: Team, tower: &str, flying: bool) -> f64 {
        let centre = self.tower(tower).at;
        let phase = if flying { Phase::Flying } else { Phase::Ground };
        self.game
            .drones
            .iter()
            .filter(|d| d.team == team && d.phase == phase && (d.at - centre).length() <= 60.0)
            .count() as f64
    }

    fn hold(&mut self, tower: &str, v: &Value) {
        let holder = holder_of(v);
        let t = self.tower_mut(tower);
        t.holder = holder;
        t.control = holder.map_or(0.0, Team::sign);
    }

    /// Blue holds `n` towers: its home tower, then the centre, the west, the east and Red's home
    /// (the flank towers last: the section's start sets them on their own).
    fn blue_holds(&mut self, n: usize) {
        for (k, name) in ["Blue home", "centre", "west", "east", "Red home"].into_iter().enumerate() {
            let holder = if k < n {
                Some(Team::Blue)
            } else if name == "Red home" {
                Some(Team::Red)
            } else {
                None
            };
            let t = self.tower_mut(name);
            t.holder = holder;
            t.control = holder.map_or(0.0, Team::sign);
        }
    }

    fn scout(&self) -> usize {
        self.game.drone(Team::Red, SCOUT)
    }

    fn place_scout(&mut self, v: &Value) {
        let i = self.scout();
        let blue = self.game.drone(Team::Blue, 0);
        match v.as_str().unwrap_or_default() {
            "near a Blue drone" => {
                self.put(blue, Vec2::ZERO, Some(ALT));
                self.put(i, Vec2::new(100.0, 0.0), Some(ALT));
            }
            "far from anything Blue" => {
                let east = self.tower("east").at;
                self.put(i, east + Vec2::new(100.0, 0.0), Some(ALT));
            }
            "near the Blue home tower" => {
                let home = self.tower("Blue home").at;
                self.put(i, home + Vec2::new(100.0, 0.0), Some(ALT));
            }
            _ => self.home(i),
        }
    }

    fn scout_seen(&self) -> &'static str {
        match self.game.maps[Team::Blue.index()].drones[self.scout()] {
            None => "unknown",
            Some(s) if s.seen => "seen",
            Some(_) => "last seen",
        }
    }
}

fn holder_of(v: &Value) -> Option<Team> {
    match v.as_str() {
        Some("held by Blue") => Some(Team::Blue),
        Some("held by Red") => Some(Team::Red),
        _ => None,
    }
}

fn holder_name(h: Option<Team>) -> &'static str {
    match h {
        Some(Team::Blue) => "held by Blue",
        Some(Team::Red) => "held by Red",
        None => "neutral",
    }
}

/// To a tenth: the referee steps in floats.
fn tenth(x: f32) -> f64 {
    (x as f64 * 10.0).round() / 10.0
}

fn bindings() -> Bindings<World> {
    let count = |n: &Value| n.as_f64().unwrap_or(0.0) as usize;
    Bindings::new(World::new())
        .observe("the game of Towers", |w| match w.game.outcome {
            None => "in play",
            Some(Outcome::Won(Team::Blue)) => "won by Blue",
            Some(Outcome::Won(Team::Red)) => "won by Red",
            Some(Outcome::Draw) => "a draw",
        })
        .arrange("the game of Towers", |w, v| {
            w.game.outcome = match v.as_str() {
                Some("won by Blue") => Some(Outcome::Won(Team::Blue)),
                Some("won by Red") => Some(Outcome::Won(Team::Red)),
                Some("a draw") => Some(Outcome::Draw),
                _ => None,
            }
        })
        .observe("the time left", |w| tenth(w.game.time_left()))
        .arrange("the time left", |w, v| w.game.time = w.game.rules.duration - v.as_f64().unwrap() as f32)
        .observe("the score of Blue", |w| w.game.score[0].floor() as f64)
        .arrange("the score of Blue", |w, v| w.game.score[0] = v.as_f64().unwrap() as f32)
        .observe("the score of Red", |w| w.game.score[1].floor() as f64)
        .arrange("the score of Red", |w, v| w.game.score[1] = v.as_f64().unwrap() as f32)
        .observe("the towers held by Blue", |w| {
            w.game.towers.iter().filter(|t| t.holder == Some(Team::Blue)).count() as f64
        })
        .arrange("the towers held by Blue", move |w, v| w.blue_holds(count(v)))
        // The flank towers and their circles.
        .observe("the west tower", |w| holder_name(w.tower("west").holder))
        .arrange("the west tower", |w, v| w.hold("west", v))
        .observe("the control of the west tower", |w| tenth(100.0 * w.tower("west").control))
        .arrange("the control of the west tower", |w, v| {
            w.tower_mut("west").control = v.as_f64().unwrap() as f32 / 100.0
        })
        .observe("the circle of the west tower", |w| if w.tower("west").contested { "contested" } else { "quiet" })
        .observe("the Blue drones in the west circle", |w| w.count(Team::Blue, "west", true))
        .arrange("the Blue drones in the west circle", move |w, v| w.in_circle(Team::Blue, "west", count(v), true))
        .observe("the Red drones in the west circle", |w| w.count(Team::Red, "west", true))
        .arrange("the Red drones in the west circle", move |w, v| w.in_circle(Team::Red, "west", count(v), true))
        .observe("the Blue drones on the ground in the west circle", |w| w.count(Team::Blue, "west", false))
        .arrange("the Blue drones on the ground in the west circle", move |w, v| {
            w.in_circle(Team::Blue, "west", count(v), false)
        })
        .observe("the east tower", |w| holder_name(w.tower("east").holder))
        .arrange("the east tower", |w, v| w.hold("east", v))
        .observe("the control of the east tower", |w| tenth(100.0 * w.tower("east").control))
        .arrange("the control of the east tower", |w, v| {
            w.tower_mut("east").control = v.as_f64().unwrap() as f32 / 100.0
        })
        .observe("the Red drones in the east circle", |w| w.count(Team::Red, "east", true))
        .arrange("the Red drones in the east circle", move |w, v| w.in_circle(Team::Red, "east", count(v), true))
        // What Blue knows.
        .observe("the Red scout", |w| {
            let d = w.game.drones[w.scout()];
            if d.phase != Phase::Flying {
                "at its base"
            } else if (d.at - w.tower("east").at).length() < 150.0 {
                "far from anything Blue"
            } else if (d.at - w.tower("Blue home").at).length() < 150.0 {
                "near the Blue home tower"
            } else {
                "near a Blue drone"
            }
        })
        .arrange("the Red scout", |w, v| w.place_scout(v))
        .observe("the Red scout on the Blue map", |w| w.scout_seen())
        .observe("the age of the Red scout on the Blue map", |w| {
            w.game.maps[Team::Blue.index()].drones[w.scout()].map_or(0.0, |s| tenth(w.game.time - s.time))
        })
        .observe("the east tower on the Blue map", |w| {
            holder_name(w.game.maps[Team::Blue.index()].towers[w.game.tower("east")].holder)
        })
        .arrange("the east tower on the Blue map", |w, v| {
            let i = w.game.tower("east");
            let time = w.game.time;
            let s = &mut w.game.maps[Team::Blue.index()].towers[i];
            s.holder = holder_of(v);
            s.control = s.holder.map_or(0.0, Team::sign);
            s.time = time;
            s.seen = false;
        })
        .on("the game runs for 10 seconds", |w| w.run(10.0))
        .on("the game runs for 20 seconds", |w| w.run(20.0))
        .on("the game runs for 1 minute", |w| w.run(60.0))
        .on("the Red scout flies into the fog for 10 seconds", |w| {
            let i = w.scout();
            w.put(i, Vec2::new(250.0, 300.0), Some(ALT));
            w.run(10.0);
        })
        .on("a Blue drone flies near the east tower", |w| {
            let i = w.game.drone(Team::Blue, 0);
            let east = w.tower("east").at;
            w.put(i, east + Vec2::new(-50.0, -50.0), Some(ALT));
        })
        // Blue's computer commander.
        .observe("the order of the first Blue drone", |w| w.order_of(0))
        .arrange("the order of the first Blue drone", |w, v| {
            let tower = v.as_str().and_then(|v| v.strip_prefix("take the ")).and_then(|v| v.strip_suffix(" tower"));
            let order = tower.map(|t| Order::Take(w.game.tower(t)));
            w.blue.assign(0, order);
        })
        .observe("the order of the second Blue drone", |w| w.order_of(1))
        .observe("the order of the third Blue drone", |w| w.order_of(2))
        .on("the computer commander of Blue decides", |w| {
            let view = w.game.view(Team::Blue);
            w.blue.decide(&view);
        })
        // The drones stay where they are: what the commander decides, not where the drones go.
        .on("the computer commander of Blue plays for 15 seconds", |w| {
            for _ in 0..15 {
                let view = w.game.view(Team::Blue);
                w.blue.decide(&view);
                w.run(1.0);
            }
            let view = w.game.view(Team::Blue);
            w.blue.decide(&view);
        })
        .on_reset(|w| *w = World::new())
}

fn suite() -> Acceptance {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
    Acceptance::load(&dir).unwrap_or_else(|e| panic!("{e}")).with_coverage()
}

#[test]
fn every_test_of_the_section_passes_with_no_window() {
    let a = suite();
    let section = a.suite.sections.iter().find(|s| s.slug == "towers").expect("a towers section").id;
    let ours = a.suite.subtree(section);
    let report = a.run(&mut bindings(), &RunOptions::default());
    let ran: Vec<_> = a.plans.iter().zip(&report.results).filter(|(p, _)| ours.contains(&p.section)).collect();
    for (p, r) in &ran {
        println!("{:?} {} {:?}", r.status, p.title, r.start.error);
        for (step, sr) in p.steps.iter().zip(&r.steps) {
            for (c, cr) in step.checks.iter().zip(&sr.checks) {
                let var = a.suite.vocabulary.var(c.var);
                println!("    {}: {:?}", var.describe(&c.expect), cr.observed.as_ref().map(|v| v.to_string()));
            }
        }
    }
    println!("{}", report.failures(&a.suite, &a.plans));
    let passed = ran.iter().filter(|(_, r)| r.status == Status::Passed).count();
    assert!(passed >= 22, "only {passed} tests of the section ran and passed");
    // The game server's tests are not this driver's: they stay "not supported yet" here.
    let not_passed: Vec<_> = ran
        .iter()
        .filter(|(_, r)| r.status == Status::Failed)
        .map(|(p, r)| format!("{:?} {}", r.status, p.title))
        .collect();
    assert!(not_passed.is_empty(), "{passed} passed; these did not:\n{}", not_passed.join("\n"));
}
