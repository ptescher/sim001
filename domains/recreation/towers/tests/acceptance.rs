//! The "Towers" section of the suite (expectations/09-towers), run with no window: the real
//! referee (`recreation_towers::Game`), with the drones put where each step says. The rules do
//! not depend on how a drone flies, so a drone here stays where it was put; the host's tests fly
//! the game with ArduPlane SITL.

use expectation_harness::{Acceptance, Bindings, RunOptions, Screenshot};
use expectation_model::Value;
use expectation_story::Status;
use glam::Vec2;
use recreation_towers::flight::{self, Plan};
use recreation_towers::map::LaneName;
use recreation_towers::{Commander, Game, Instruction, Link, Order, Outcome, Phase, Rules, Standing, Team};
use std::path::Path;

/// The referee's step (s): the host steps it with the physics, faster than this.
const DT: f32 = 0.1;
/// The height a drone circles a tower at (m).
const ALT: f32 = 50.0;
/// The Red scout: Red's first drone.
const SCOUT: usize = 0;
/// A drone that flies low, and one above the trees (m).
const LOW: f32 = 17.0;

struct World {
    game: Game,
    /// Blue's computer commander.
    blue: Commander,
    /// Where the steps put the first Blue drone and the Red scout, by the spec's words, and the
    /// place they put each at.
    blue_place: Option<(&'static str, Vec2)>,
    scout_place: Option<(&'static str, Vec2)>,
    /// The plan that the first Blue drone was given by its last order.
    plan: Option<Plan>,
    /// What the first Blue drone was told: its order, and its standing order.
    told: Instruction,
}

impl World {
    fn new() -> World {
        let rules = Rules::default();
        World {
            game: Game::new(rules),
            blue: Commander::new(rules),
            blue_place: None,
            scout_place: None,
            plan: None,
            told: Instruction::default(),
        }
    }

    /// Blue drone `k`'s order, as the spec words it.
    fn order_of(&self, k: usize) -> String {
        self.say(self.blue.order(k))
    }

    /// An order, as the spec words it.
    fn say(&self, order: Option<Order>) -> String {
        match order {
            None => "none".into(),
            Some(Order::Take(t)) => format!("take the {} tower", self.game.towers[t].name),
            Some(Order::Hold) => "hold".into(),
            Some(Order::Home) => "go home".into(),
            Some(Order::Goto(_)) => "go to a point".into(),
        }
    }

    fn run(&mut self, secs: f32) {
        let blue = self.game.drone(Team::Blue, 0);
        for _ in 0..(secs / DT).round() as usize {
            self.game.step(DT);
            self.told.tick(
                self.game.silent[blue],
                self.game.rules.link_timeout,
                self.game.charge(blue),
                self.game.rules.bingo,
            );
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
        // In a game the teams look all the time: the radio shadows move with the towers.
        self.game.look();
    }

    /// Blue holds `n` towers: its home tower, then the west, the east, the centre and Red's home
    /// (the flank towers and the spire last: the section's start sets them on their own).
    fn blue_holds(&mut self, n: usize) {
        for (k, name) in ["Blue home", "west", "east", "centre", "Red home"].into_iter().enumerate() {
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

    fn mid_lane(&self) -> Vec2 {
        Vec2::new(-220.0, 0.0)
    }

    /// A point on the east lane, as far out from the Blue home tower as `mid_lane` is on the mid
    /// lane (210 m, past the sight of the home tower): about 143 m from it, with the forest between.
    fn east_lane(&self) -> Vec2 {
        let lane = self.game.map.lanes.iter().find(|l| l.name == LaneName::East).unwrap();
        lane.points[0] + (lane.points[1] - lane.points[0]).normalize() * 210.0
    }

    /// Where the west lane is nearest the lake.
    fn shore(&self) -> Vec2 {
        let m = &self.game.map;
        (m.towers[0] + m.towers[1]) * 0.5
    }

    /// A point 100 m from the Red home tower, towards the middle of the field.
    fn shadow(&self) -> Vec2 {
        self.game.map.towers[4] - Vec2::new(100.0, 0.0)
    }

    fn place_blue(&mut self, v: &Value) {
        let i = self.game.drone(Team::Blue, 0);
        let m = &self.game.map;
        let (name, at, alt) = match v.as_str().unwrap_or_default() {
            "at the Blue home tower" => ("at the Blue home tower", m.towers[0], Some(15.0)),
            "low in the mid lane" => ("low in the mid lane", self.mid_lane(), Some(LOW)),
            "low in the west lane, on the shore of the lake" => {
                ("low in the west lane, on the shore of the lake", self.shore(), Some(LOW))
            }
            "near the Red home tower" => ("near the Red home tower", self.shadow(), Some(ALT)),
            "over its base" => ("over its base", m.bases[0], Some(ALT)),
            "in the west circle" => ("in the west circle", m.towers[1] + Vec2::new(40.0, 0.0), Some(ALT)),
            other => match away_from(m, other) {
                Some((name, at)) => (name, at, Some(ALT)),
                None => ("at its base", m.bases[0], None),
            },
        };
        self.put(i, at, alt);
        self.blue_place = Some((name, at));
    }

    /// The second Blue drone, flying that far east of the Red home tower, or at its base.
    fn place_second(&mut self, v: &Value) {
        let i = self.game.drone(Team::Blue, 1);
        let words = v.as_str().unwrap_or_default();
        match away_from(&self.game.map, words) {
            Some((_, at)) => self.put(i, at, Some(ALT)),
            None => self.home(i),
        }
    }

    fn place_scout(&mut self, v: &Value) {
        let i = self.scout();
        let blue = self.game.drone(Team::Blue, 0);
        let new = |name: &'static str| Some((name, Vec2::ZERO));
        let (name, at, alt) = match v.as_str().unwrap_or_default() {
            "low in the east lane, near the first Blue drone" => {
                (new("low in the east lane, near the first Blue drone"), self.east_lane(), Some(LOW))
            }
            "high over the east lane, near the first Blue drone" => {
                (new("high over the east lane, near the first Blue drone"), self.east_lane(), Some(ALT))
            }
            "low in the mid lane, ahead of the first Blue drone" => (
                new("low in the mid lane, ahead of the first Blue drone"),
                self.mid_lane() + Vec2::new(120.0, 0.0),
                Some(LOW),
            ),
            "low over the lake" => (new("low over the lake"), self.game.map.lake.at, Some(LOW)),
            "50 m from the first Blue drone" => {
                (new("50 m from the first Blue drone"), self.shadow() + Vec2::new(0.0, 50.0), Some(ALT))
            }
            _ => (None, Vec2::ZERO, None),
        };
        if let Some((name, _)) = name {
            self.put(i, at, alt);
            self.scout_place = Some((name, at));
            return;
        }
        self.scout_place = None;
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

    /// What the Blue drone's last plan is, in the spec's words.
    fn way(&self) -> &'static str {
        match self.plan.as_ref().map(|p| (p.path.is_empty(), p.lane)) {
            Some((false, Some(LaneName::West))) => "along the west lane",
            Some((false, Some(LaneName::Mid))) => "along the mid lane",
            Some((false, Some(LaneName::East))) => "along the east lane",
            _ => "straight over the forest",
        }
    }

    /// The first Blue drone is sent to `target`, `low` or high.
    fn send_blue(&mut self, target: Vec2, low: bool) {
        let d = self.game.drones[self.game.drone(Team::Blue, 0)];
        self.plan = Some(flight::plan(&self.game.map, d.at, target, low));
    }

    fn blue_link(&self) -> &'static str {
        match self.game.link(self.game.drone(Team::Blue, 0)) {
            Link::Up => "in contact",
            Link::Cut => "cut",
            Link::Lost => "lost",
        }
    }

    /// The tallest mountain on the map at `at`, near the point, to the metre.
    fn mountains_at(&self, at: Vec2) -> f64 {
        self.game.map.mountain_height(at).round() as f64
    }

    fn sent_point_clear(&self) -> bool {
        self.plan.as_ref().is_some_and(|p| self.game.map.mountain_height(p.target) < 1.0)
    }

    fn scout_seen(&self) -> &'static str {
        match self.game.maps[Team::Blue.index()].drones[self.scout()] {
            None => "unknown",
            Some(s) if s.seen => "seen",
            Some(_) => "last seen",
        }
    }
}

/// "150 m from the Red home tower": that far east of the tower, and the words.
fn away_from(m: &recreation_towers::Map, words: &str) -> Option<(&'static str, Vec2)> {
    const PLACES: [(&str, f32, usize); 9] = [
        ("130 m from the Red home tower", 130.0, 4),
        ("210 m from the Red home tower", 210.0, 4),
        ("230 m from the Red home tower", 230.0, 4),
        ("150 m from the Red home tower", 150.0, 4),
        ("170 m from the Red home tower", 170.0, 4),
        ("190 m from the centre tower", 190.0, 2),
        ("210 m from the centre tower", 210.0, 2),
        ("110 m from the west tower", 110.0, 1),
        ("130 m from the west tower", 130.0, 1),
    ];
    PLACES.iter().find(|p| p.0 == words).map(|&(name, d, tower)| (name, m.towers[tower] + Vec2::new(0.0, d)))
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
        .set("the score of Blue", |w, v| w.game.score[0] = v.as_f64().unwrap() as f32)
        .observe("the score of Red", |w| w.game.score[1].floor() as f64)
        .arrange("the score of Red", |w, v| w.game.score[1] = v.as_f64().unwrap() as f32)
        .observe("the towers held by Blue", |w| {
            w.game.towers.iter().filter(|t| t.holder == Some(Team::Blue)).count() as f64
        })
        .set("the towers held by Blue", move |w, v| w.blue_holds(count(v)))
        // The flank towers and their circles.
        .observe("the west tower", |w| holder_name(w.tower("west").holder))
        .arrange("the west tower", |w, v| w.hold("west", v))
        .observe("the centre tower", |w| holder_name(w.tower("centre").holder))
        .arrange("the centre tower", |w, v| w.hold("centre", v))
        .observe("the control of the west tower", |w| tenth(100.0 * w.tower("west").control))
        .arrange("the control of the west tower", |w, v| {
            w.tower_mut("west").control = v.as_f64().unwrap() as f32 / 100.0
        })
        .observe("the circle of the west tower", |w| if w.tower("west").contested { "contested" } else { "quiet" })
        .observe("the Blue drones in the west circle", |w| w.count(Team::Blue, "west", true))
        .set("the Blue drones in the west circle", move |w, v| w.in_circle(Team::Blue, "west", count(v), true))
        .observe("the Red drones in the west circle", |w| w.count(Team::Red, "west", true))
        .set("the Red drones in the west circle", move |w, v| w.in_circle(Team::Red, "west", count(v), true))
        .observe("the Blue drones on the ground in the west circle", |w| w.count(Team::Blue, "west", false))
        .set("the Blue drones on the ground in the west circle", move |w, v| {
            w.in_circle(Team::Blue, "west", count(v), false)
        })
        .observe("the east tower", |w| holder_name(w.tower("east").holder))
        .arrange("the east tower", |w, v| w.hold("east", v))
        .observe("the control of the east tower", |w| tenth(100.0 * w.tower("east").control))
        .arrange("the control of the east tower", |w, v| {
            w.tower_mut("east").control = v.as_f64().unwrap() as f32 / 100.0
        })
        .observe("the Red drones in the east circle", |w| w.count(Team::Red, "east", true))
        .set("the Red drones in the east circle", move |w, v| w.in_circle(Team::Red, "east", count(v), true))
        // What Blue knows.
        .observe("the Red scout", |w| {
            let d = w.game.drones[w.scout()];
            if let Some((name, at)) = w.scout_place
                && d.phase == Phase::Flying
                && d.at == at
            {
                name
            } else if d.phase != Phase::Flying {
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
        .on("the game runs for 3 seconds", |w| w.run(3.0))
        .on("the game runs for 110 seconds", |w| w.run(110.0))
        .on("the game runs for 10 seconds", |w| w.run(10.0))
        .on("the game runs for 15 seconds", |w| w.run(15.0))
        .on("the game runs for 20 seconds", |w| w.run(20.0))
        .on("the game runs for 30 seconds", |w| w.run(30.0))
        .on("the game runs for 1 minute", |w| w.run(60.0))
        .on("the Red scout flies into the fog for 10 seconds", |w| {
            let i = w.scout();
            w.put(i, Vec2::new(250.0, 300.0), Some(ALT));
            w.run(10.0);
        })
        .on("a Blue drone flies near the east tower", |w| {
            let i = w.game.drone(Team::Blue, 0);
            let east = w.tower("east").at;
            w.put(i, east + Vec2::new(-100.0, -100.0), Some(ALT));
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
        // The lanes: where the first Blue drone is, and the way it is sent.
        .observe("the first Blue drone", |w| {
            let d = w.game.drones[w.game.drone(Team::Blue, 0)];
            match w.blue_place {
                Some((name, at)) if d.phase == Phase::Flying && d.at == at => name,
                _ => "at its base",
            }
        })
        .arrange("the first Blue drone", |w, v| w.place_blue(v))
        .on("the first Blue drone is sent high to the west tower", |w| {
            let west = w.tower("west").at;
            w.send_blue(west, false)
        })
        .on("the first Blue drone is sent low to the west tower", |w| {
            let west = w.tower("west").at;
            w.send_blue(west, true)
        })
        .on("the first Blue drone is sent low to a point in the forest", |w| {
            w.send_blue(Vec2::new(-100.0, 100.0), true)
        })
        .observe("the way of the first Blue drone", |w| w.way())
        .observe("the orbit of the first Blue drone", |w| {
            if w.plan.as_ref().is_some_and(|p| p.circle_low) { "low" } else { "high" }
        })
        // The radio shadows.
        .observe("the Red home tower", |w| holder_name(w.tower("Red home").holder))
        .arrange("the Red home tower", |w, v| w.hold("Red home", v))
        .observe("the second Blue drone", |w| {
            let d = w.game.drones[w.game.drone(Team::Blue, 1)];
            if d.phase != Phase::Flying {
                return "at its base".to_string();
            }
            let m = &w.game.map;
            let d_red = (d.at - m.towers[4]).length().round();
            format!("{d_red} m from the Red home tower")
        })
        .arrange("the second Blue drone", |w, v| w.place_second(v))
        .observe("the link of the first Blue drone", |w| w.blue_link())
        .on("the first Blue drone flies out of the radio shadow", |w| {
            w.place_blue(&Value::from("at the Blue home tower"))
        })
        .on("the first Blue drone flies into the radio shadow for 10 seconds", |w| {
            w.place_blue(&Value::from("near the Red home tower"));
            w.run(10.0);
        })
        .on("the first Blue drone flies through the radio shadow for 10 seconds", |w| {
            w.place_blue(&Value::from("near the Red home tower"));
            w.run(10.0);
            w.place_blue(&Value::from("at the Blue home tower"));
        })
        .observe("the first Blue drone on the Blue map", |w| {
            match w.game.maps[Team::Blue.index()].drones[w.game.drone(Team::Blue, 0)] {
                Some(s) if s.seen => "live",
                _ => "last in contact",
            }
        })
        .observe("the age of the first Blue drone on the Blue map", |w| {
            let i = w.game.drone(Team::Blue, 0);
            w.game.maps[Team::Blue.index()].drones[i].map_or(0.0, |s| tenth(w.game.time - s.time))
        })
        // The battery.
        .observe("the batteries", |w| if w.game.rules.standard_battery > 0.0 { "on" } else { "off" })
        .arrange("the batteries", |w, v| {
            let on = v.as_str() == Some("on");
            let r = &mut w.game.rules;
            (r.light_battery, r.standard_battery, r.heavy_battery) =
                if on { (150.0, 240.0, 300.0) } else { (0.0, 0.0, 0.0) };
            w.game.fill_batteries();
            // The commander plays by the same rules.
            w.blue = Commander::new(w.game.rules);
        })
        .observe("the battery of the first Blue drone", |w| tenth(w.game.battery[w.game.drone(Team::Blue, 0)]))
        .arrange("the battery of the first Blue drone", |w, v| {
            let i = w.game.drone(Team::Blue, 0);
            w.game.battery[i] = v.as_f64().unwrap() as f32;
            w.game.reported[i] = w.game.battery[i];
        })
        .observe("the battery of the first Blue drone on the Blue map", |w| {
            tenth(w.game.view(Team::Blue).drones[0].battery)
        })
        .observe("the condition of the first Blue drone", |w| match w.game.drones[w.game.drone(Team::Blue, 0)].phase {
            Phase::Flying => "flying",
            Phase::Ground => "on the ground",
            Phase::Down => "down",
        })
        // Standing orders.
        .observe("the order that the first Blue drone flies", |w| w.say(w.told.order))
        .arrange("the order that the first Blue drone flies", |w, v| {
            let tower = v.as_str().and_then(|v| v.strip_prefix("take the ")).and_then(|v| v.strip_suffix(" tower"));
            w.told.order = match (tower, v.as_str()) {
                (Some(t), _) => Some(Order::Take(w.game.tower(t))),
                (_, Some("hold")) => Some(Order::Hold),
                (_, Some("go home")) => Some(Order::Home),
                _ => None,
            };
        })
        .observe("the standing order of the first Blue drone", |w| match w.told.standing {
            None => "none".to_string(),
            Some(s) => format!("{} after {} s", w.say(Some(s.then)), s.after),
        })
        .arrange("the standing order of the first Blue drone", |w, v| {
            w.told.standing = match v.as_str() {
                Some("hold after 5 s") => Some(Standing { after: 5.0, then: Order::Hold }),
                Some("go home after 5 s") => Some(Standing { after: 5.0, then: Order::Home }),
                Some("hold after 30 s") => Some(Standing { after: 30.0, then: Order::Hold }),
                _ => None,
            };
        })
        .on("the first Blue drone is given the order to hold", |w| w.told.give(Order::Hold, None))
        // The mountains.
        .observe("the mountains at the east edge of the field", |w| w.mountains_at(Vec2::new(0.0, 600.0)))
        .observe("the mountains at the west edge of the field", |w| w.mountains_at(Vec2::new(0.0, -600.0)))
        .observe("the mountains at the bases, the towers and the lake", |w| {
            let m = &w.game.map;
            m.bases.iter().chain(&m.towers).chain([&m.lake.at]).map(|&p| w.mountains_at(p)).fold(0.0, f64::max)
        })
        .observe("the way round the edge of the field from base to base", |w| {
            if w.game.map.edge_open(w.game.rules.ceiling) { "open" } else { "closed" }
        })
        .on("the first Blue drone is sent to a point in the mountains", |w| w.send_blue(Vec2::new(0.0, 580.0), false))
        .observe("the point that the first Blue drone flies to", |w| {
            if w.sent_point_clear() { "clear of the mountains" } else { "in the mountains" }
        })
        .on_reset(|w| *w = World::new())
        // Blue's map, as the commander app draws it, with the computer commander's orders.
        .on_screenshot(|w| {
            let mut view = w.game.view(Team::Blue);
            for d in &mut view.drones {
                d.order = w.blue.order(d.k);
            }
            Screenshot::svg(recreation_towers::svg::map_svg(&view, &w.game.rules))
        })
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
    assert!(passed >= 30, "only {passed} tests of the section ran and passed");
    // The game server's tests are not this driver's: they stay "not supported yet" here.
    let not_passed: Vec<_> = ran
        .iter()
        .filter(|(_, r)| r.status == Status::Failed)
        .map(|(p, r)| format!("{:?} {}", r.status, p.title))
        .collect();
    assert!(not_passed.is_empty(), "{passed} passed; these did not:\n{}", not_passed.join("\n"));
}

/// The same, with pictures of Blue's map. With SIM001_DOCS=DIR the results and pictures go into
/// the project's docs, beside the other runners'.
#[test]
fn the_section_documents_itself_with_pictures_of_the_map() {
    let a = suite();
    let published = std::env::var("SIM001_DOCS").ok().map(std::path::PathBuf::from);
    let out = published.clone().unwrap_or_else(|| Path::new(env!("CARGO_TARGET_TMPDIR")).join("towers_docs"));
    let report = a.run(&mut bindings(), &RunOptions { docs_dir: Some(out.clone()), ..Default::default() });
    if published.is_some() {
        a.publish(&out, "towers", &report).unwrap();
    } else {
        a.docs(Some(&report)).write(&out).unwrap();
    }
    let shot = out
        .join("screenshots/towers--rules--coverage--each-tower-that-a-team-holds-gives-one-point-each-second/01.svg");
    let svg = std::fs::read_to_string(&shot).unwrap_or_else(|e| panic!("{}: {e}", shot.display()));
    assert!(svg.contains("BLUE 20"), "the score after 10 s with two towers: {svg}");
}
