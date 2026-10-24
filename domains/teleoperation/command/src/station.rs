//! The app as the ground station of its team (plans/unified-link.md): it puts its drones in AGENT
//! mode and sends each one its plan, its standing order and the traffic as MAVLink, through the air
//! in front of the drone, as the ground station of a real drone does. The game server only plays
//! the world; the app tells the drones what to do from what its team knows (the map) and from the
//! orders that the commander gave.

use crate::seat::{Seat, Status};
use glam::Vec2;
use recreation_towers::seat::Join;
use recreation_towers::{Game, Order, Phase, Rules};
use recreation_towers_ground::map::{self, OwnReport, Seen};
use recreation_towers_ground::{AGENT_MODE, Ground, QLOITER_MODE, Situation, Transport, drone_name, sitl_home};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// How often the ground looks (wall time).
const LOOK_EVERY: Duration = Duration::from_millis(20);
/// ArduPlane's FBWA, the mode of a drone that has taken off and waits for AGENT.
const FBWA_MODE: u32 = 5;

/// The drones' time as the app can know it: the time of the last map, and the rate at which that time
/// runs (a game sped up runs ahead of the clock on the wall).
struct Clock {
    map_time: f32,
    at: Instant,
    rate: f64,
    last: f64,
}

impl Clock {
    fn new() -> Clock {
        Clock { map_time: f32::NEG_INFINITY, at: Instant::now(), rate: 1.0, last: 0.0 }
    }

    /// Game seconds to a wall second.
    fn rate(&self) -> f64 {
        self.rate
    }

    /// The time now, given the time of the latest map.
    fn now(&mut self, map_time: f32) -> f64 {
        if map_time != self.map_time {
            let wall = self.at.elapsed().as_secs_f64();
            if self.map_time.is_finite() && wall > 0.05 && map_time > self.map_time {
                let rate = (map_time - self.map_time) as f64 / wall;
                self.rate = (0.7 * self.rate + 0.3 * rate).clamp(0.5, 20.0);
            }
            (self.map_time, self.at) = (map_time, Instant::now());
        }
        // Never ahead of the next map by more than a second, and never back.
        let now = self.map_time as f64 + (self.at.elapsed().as_secs_f64() * self.rate).min(1.0);
        self.last = self.last.max(now);
        self.last
    }
}

/// Starts the ground of the app's team, on a thread of its own, for as long as the seat commands.
pub fn start(seat: Arc<Seat>, host: String) {
    std::thread::Builder::new().name("station".into()).spawn(move || run(&seat, &host)).expect("the station thread");
}

/// What the ground flies: its connection, for one set of air ports.
struct Flying {
    air: Vec<u16>,
    ground: Ground,
    game: Game,
    rules: Rules,
    first: usize,
    clock: Clock,
    home: (f64, f64, f32),
}

fn run(seat: &Seat, host: &str) {
    let mut flying: Option<Flying> = None;
    loop {
        std::thread::sleep(LOOK_EVERY);
        let (status, rules, umpire, sites, air, orders) = {
            let s = seat.state();
            (s.status.clone(), s.rules, s.umpire, s.sites.clone(), s.air.clone(), s.orders.clone())
        };
        let (Status::Seated(Join::Command(team)), Some(rules), Some(umpire), Some(sites)) =
            (status, rules, umpire, sites)
        else {
            flying = None;
            seat.flown.store(0, Ordering::SeqCst);
            continue;
        };
        if air.len() != rules.drones {
            continue;
        }
        if flying.as_ref().is_none_or(|f| f.air != air) {
            // A new game, or the first look: connect to the drones through the air.
            let n = rules.drones;
            let first = team.index() * n;
            let names: Vec<String> = (0..2 * n).map(|i| drone_name(&rules, i)).collect();
            let indices: Vec<usize> = (first..first + n).collect();
            // Light drones have no wing for AGENT to fly: the world's radio flies them, in QLOITER.
            let modes: Vec<u32> = (0..n)
                .map(|k| if rules.class(k) == recreation_towers::Class::Light { QLOITER_MODE } else { AGENT_MODE })
                .collect();
            let transport = Transport::Tcp { host: host.to_string(), ports: air.clone() };
            let ground = Ground::connect(&transport, &names, &indices, &modes, sitl_home());
            flying = Some(Flying {
                air,
                ground,
                game: Game::new(rules),
                rules,
                first,
                clock: Clock::new(),
                home: sitl_home(),
            });
        }
        let f = flying.as_mut().expect("a ground");
        let n = f.rules.drones;
        // The map: the umpire's word and the reports of the sites, plus what the drones report of
        // themselves and of the enemy in their telemetry.
        let rate = f.clock.rate() as f32;
        let (mut own, mut seen) = (vec![None; n], vec![]);
        for (k, report) in own.iter_mut().enumerate() {
            let (position, sightings) = f.ground.telemetry(k);
            *report = position.map(|p| {
                let (north, east) = cognition_plan::to_local(f.home.0, f.home.1, p.lat, p.lon);
                OwnReport {
                    at: Vec2::new(north, east),
                    alt: p.alt,
                    vel: Vec2::new(p.vel[0], p.vel[1]),
                    age: p.heard.elapsed().as_secs_f32() * rate,
                }
            });
            seen.extend(sightings.iter().map(|s| {
                let (north, east) = cognition_plan::to_local(f.home.0, f.home.1, s.lat, s.lon);
                Seen {
                    enemy: s.enemy,
                    at: Vec2::new(north, east),
                    alt: s.alt_amsl - f.home.2,
                    vel: Vec2::new(s.speed * s.heading.to_radians().cos(), s.speed * s.heading.to_radians().sin()),
                    age: s.heard.elapsed().as_secs_f32() * rate,
                }
            }));
        }
        let view = map::build(&umpire, &sites, &own, &seen, &orders, &f.rules);
        // The picture the ground plans from is the map: the drones where the team has them.
        for d in &view.drones {
            let g = &mut f.game.drones[f.first + d.k];
            (g.at, g.alt, g.vel, g.phase) = (d.at, d.alt, d.vel, d.phase);
        }
        let enemy_first = if team.index() == 0 { n } else { 0 };
        for g in &mut f.game.drones[enemy_first..enemy_first + n] {
            g.phase = Phase::Ground;
        }
        for e in view.enemies.iter().filter(|e| e.seen) {
            let g = &mut f.game.drones[enemy_first + e.k];
            (g.at, g.alt, g.vel, g.phase) = (e.at, e.alt, e.vel, Phase::Flying);
        }
        let mut given = vec![(Some(Order::Home), false); 2 * n];
        for (k, o) in orders.iter().enumerate().take(n) {
            if o.0.is_some() {
                given[f.first + k] = *o;
            }
        }
        let wing: Vec<bool> = (0..2 * n)
            .map(|i| {
                let Some(k) = i.checked_sub(f.first).filter(|&k| k < n) else { return false };
                let flying = view.drones[k].phase == Phase::Flying;
                flying && f.ground.mode_of(k).is_some_and(|m| [FBWA_MODE, AGENT_MODE, QLOITER_MODE].contains(&m))
            })
            .collect();
        let situation = Situation {
            now: f.clock.now(view.time),
            game: &f.game,
            orders: &given.iter().map(|o| o.0).collect::<Vec<_>>(),
            lows: &given.iter().map(|o| o.1).collect::<Vec<_>>(),
            standing: &vec![None; 2 * n],
            wing,
        };
        f.ground.tick(&situation);
        seat.flown.store(f.ground.agent_drones(), Ordering::SeqCst);
        seat.heard.store(f.ground.position_reports() as usize, Ordering::SeqCst);
        seat.state().view = Some(view);
    }
}
