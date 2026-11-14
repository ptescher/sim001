//! The computer commander: a seat like a person's. It takes a `Command` seat on the game server
//! (as `Join::Compute`, a seat that gives way to a person), flies the team's drones from the
//! station of the seat (`station`: its own ground, the map from telemetry) and, once a second of
//! game time, asks [`recreation_towers::Commander`] (as a [`Command`]) for directives, and sends each with its queue, on the map the station built. It
//! knows only what a seat gets.
//!
//! It takes advice and does not wait for it: its advisors ([`AdvisorChoice`], the lookahead by
//! default) run beside it on threads of their own (`advisors`). Before each decision it puts the
//! proposals that have arrived in hand (`Commander::advise`); after it, it shows the advisors the
//! map and what it has in hand. The built-in lookahead of `Commander` is off.

use crate::advisors::{AdvisorChoice, Advisors};
use crate::seat::{Seat, Status};
use crate::station::{self, Tally};
use recreation_towers::seat::Join;
use recreation_towers::{Class, Command, Commander, Proposal, Team};
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
    run_with(addr, team, brain, tally, stop, &AdvisorChoice::Lookahead);
}

/// The same, with the advisor `advisor` beside the commander.
pub fn run_with(
    addr: &str,
    team: Team,
    brain: Brain,
    tally: Option<Arc<Tally>>,
    stop: &AtomicBool,
    advisor: &AdvisorChoice,
) {
    let host = addr.rsplit_once(':').map_or("127.0.0.1", |(h, _)| h).to_string();
    let seat = Seat::connect(addr, Join::Compute(team));
    station::start(seat.clone(), host, tally.clone());
    let mut commander: Option<(Commander, f32, Advisors)> = None;
    // Proposals that arrived and are not yet taken or stale (for the log).
    let mut pending: Vec<Proposal> = vec![];
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
        let (c, next, advisors) = commander.get_or_insert_with(|| {
            let mut c = Commander::new(rules);
            for d in &view.drones {
                c.assign(d.k, d.order);
            }
            (c.with_lookahead(false), f32::NEG_INFINITY, advisor.start(rules, team))
        });
        if view.time < *next {
            continue;
        }
        *next = view.time + DECIDE_EVERY;
        // What the advisors have for the commander by now, and no more: it does not wait for them.
        let arrived = advisors.take(view.time);
        for p in &arrived {
            println!(
                "advice {team:?}: proposal at {:.0} s: drone {} {:?} for {:.1} points, good until {:.0} s",
                view.time,
                p.drone + 1,
                p.order,
                p.gain,
                p.until
            );
        }
        if let Some(t) = &tally {
            t.proposals[team.index()].fetch_add(arrived.len(), Ordering::SeqCst);
        }
        pending.extend(arrived.iter().cloned());
        c.advise(arrived);
        let Ok(directives) = Command::decide(c, &view) else { continue };
        // A proposal is taken when the drone's order is the proposal's now.
        pending.retain(|p| {
            if directives.iter().any(|d| d.drone == p.drone && d.order == p.order) && p.until >= view.time {
                println!("advice {team:?}: taken at {:.0} s: drone {} {:?}", view.time, p.drone + 1, p.order);
                if let Some(t) = &tally {
                    t.taken[team.index()].fetch_add(1, Ordering::SeqCst);
                }
                return false;
            }
            p.until >= view.time
        });
        if let Some(hand) = c.hand() {
            advisors.show(&view, hand);
        }
        for d in directives {
            seat.order_with_queue(d.drone, d.order, rules.class(d.drone) == Class::Light, d.queue);
        }
    }
    seat.stop();
}
