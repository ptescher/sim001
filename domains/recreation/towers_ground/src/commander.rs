//! The computer commander: a seat like a person's. It takes a `Command` seat on the game server
//! (as `Join::Compute`, a seat that gives way to a person), flies the team's drones from the
//! station of the seat (`station`: its own ground, the map from telemetry) and, once a second of
//! game time, asks [`recreation_towers::Commander`] for orders on the map the station built. It
//! knows only what a seat gets.

use crate::seat::{Seat, Status};
use crate::station::{self, Tally};
use recreation_towers::seat::Join;
use recreation_towers::{Class, Commander, Team};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How often the commander decides (s of the game).
pub const DECIDE_EVERY: f32 = 1.0;

/// What the commander does with its seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brain {
    /// Plays: decides once a second.
    Play,
    /// Sits in the seat and gives no orders: its drones fly home (a game to practise in).
    Idle,
}

/// Takes the seat of `team` on the server at `addr` (`host:port`) and commands it until the game is
/// over, a person takes the seat, or `stop` is set. If the seat is held, it waits for it to be free.
pub fn run(addr: &str, team: Team, brain: Brain, tally: Option<Arc<Tally>>, stop: &AtomicBool) {
    let host = addr.rsplit_once(':').map_or("127.0.0.1", |(h, _)| h).to_string();
    let seat = Seat::connect(addr, Join::Compute(team));
    station::start(seat.clone(), host, tally);
    let mut commander: Option<(Commander, f32)> = None;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
        let (status, rules, view) = {
            let s = seat.state();
            (s.status.clone(), s.rules, s.view.clone())
        };
        match status {
            // Relieved by a person, or refused for good.
            Status::Refused(why) if !why.contains("has its commander") => break,
            Status::Seated(_) => {}
            _ => {
                commander = None;
                continue;
            }
        }
        let (Some(rules), Some(view)) = (rules, view) else { continue };
        if view.outcome.is_some() {
            break;
        }
        if brain == Brain::Idle || view.drones.len() != rules.drones || view.time <= 0.0 {
            continue;
        }
        // The seat starts from the orders the drones have: they are on the map.
        let (c, next) = commander.get_or_insert_with(|| {
            let mut c = Commander::new(rules);
            for d in &view.drones {
                c.assign(d.k, d.order);
            }
            (c, f32::NEG_INFINITY)
        });
        if view.time < *next {
            continue;
        }
        *next = view.time + DECIDE_EVERY;
        for (k, order) in c.decide(&view) {
            seat.order(k, order, rules.class(k) == Class::Light);
        }
    }
    seat.stop();
}
