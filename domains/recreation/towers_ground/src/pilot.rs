//! A scripted pilot: a client that takes the seat of one drone ([`Join::Unit`]) and flies it by
//! the sticks, with no person, so that the whole path from sticks to drone and from commander to
//! mission is tested. It is a pilot with one idea: hold a gentle bank, and keep the height and the
//! speed that a drone needs to stay in the air. It reads only what a pilot is shown (the
//! [`Mission`]), as a person does, and writes down what the seat gave it ([`Flown`]).

use crate::seat::{Seat, Status};
use recreation_towers::seat::{Join, Mission, Sticks};
use recreation_towers::{Order, Queued, Team};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often the sticks are sent (Hz): the server wants 20 or more.
pub const STICKS_HZ: f32 = 25.0;

/// What the scripted pilot does with the sticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Script {
    /// The roll stick held (-1 to 1): a bank, and so a turn.
    pub roll: f32,
    /// The height it keeps (m) and the speed (m/s).
    pub height: f32,
    pub speed: f32,
}

impl Script {
    /// A gentle bank to the right at 60 m and 20 m/s.
    pub fn bank() -> Script {
        Script { roll: 0.3, height: 60.0, speed: 20.0 }
    }

    /// The sticks for what the pilot is shown: the roll stick as the script holds it, the elevator
    /// to climb to its height and the throttle to hold its speed.
    pub fn sticks(&self, m: &Mission) -> Sticks {
        let speed = m.vel.length();
        // Pulled back (positive) to climb; not more than a gentle climb or dive.
        let pitch = (0.012 * (self.height - m.obs.alt)).clamp(-0.25, 0.3);
        let throttle = (0.6 + 0.04 * (self.speed - speed)).clamp(0.3, 1.0);
        Sticks { roll: self.roll, pitch, throttle, yaw: 0.0 }
    }
}

/// What the scripted pilot saw and did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flown {
    /// Why the seat was refused, if it was.
    pub refused: Option<String>,
    /// How many sticks were sent, and how many different missions the seat showed.
    pub sticks_sent: usize,
    pub missions: usize,
    /// The distinct orders of the mission, in the order they came.
    pub orders: Vec<Option<Order>>,
    /// The queue of the last mission.
    pub queue: Vec<Queued>,
    /// How far the drone's course turned in all (degrees, right is positive), from its velocity.
    pub turned: f32,
    /// Missions shown out of contact, and how many of those differed from the last one shown in contact.
    pub dark: usize,
    pub changed_in_the_dark: usize,
    /// The towers the mission is about, as the screen would say: name, then bearing and distance.
    pub target: Option<String>,
    /// The game's time at the last mission, and whether the game was over.
    pub time: f32,
    pub over: bool,
}

/// Takes the seat of drone `drone` (from 0) of `team` on the server at `addr`, flies `script` for
/// `for_` (wall time), and lets go. Stops early when `stop` is set or the game is over. Waits up
/// to a minute for the seat.
pub fn fly(addr: &str, team: Team, drone: usize, script: &Script, for_: Duration, stop: &AtomicBool) -> Flown {
    let seat = Seat::connect(addr, Join::Unit { team, drone });
    let mut flown = Flown::default();
    let wait = Instant::now() + Duration::from_secs(60);
    loop {
        match seat.state().status.clone() {
            Status::Seated(_) => break,
            Status::Refused(why) => {
                flown.refused = Some(why);
                seat.stop();
                return flown;
            }
            Status::Connecting => {}
        }
        if Instant::now() > wait || stop.load(Ordering::SeqCst) {
            flown.refused = Some("no seat in time".into());
            seat.stop();
            return flown;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let end = Instant::now() + for_;
    let (mut last_time, mut course, mut last_in_contact) = (f32::NEG_INFINITY, None::<f32>, None::<Mission>);
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        let mission = seat.state().mission.clone();
        let sticks = mission.as_ref().map_or(Sticks::CENTRED, |m| script.sticks(m));
        seat.sticks(sticks);
        flown.sticks_sent += 1;
        if let Some(m) = mission.filter(|m| m.time != last_time || flown.missions == 0) {
            last_time = m.time;
            flown.missions += 1;
            flown.time = m.time;
            flown.over = m.outcome.is_some();
            if flown.orders.last() != Some(&m.order) {
                flown.orders.push(m.order);
            }
            flown.queue = m.queue.clone();
            flown.target = target_text(&m);
            if m.in_contact {
                last_in_contact = Some(m.clone());
            } else if let Some(shown) = &last_in_contact {
                flown.dark += 1;
                if (shown.order, &shown.queue, shown.low) != (m.order, &m.queue, m.low) {
                    flown.changed_in_the_dark += 1;
                }
            }
            if m.vel.length() > 5.0 {
                let now = m.vel.y.atan2(m.vel.x).to_degrees();
                if let Some(before) = course {
                    flown.turned += (now - before + 540.0).rem_euclid(360.0) - 180.0;
                }
                course = Some(now);
            }
            if flown.over {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs_f32(1.0 / STICKS_HZ));
    }
    // Let go: the seat goes, and the drone is its computer's again.
    seat.stop();
    flown
}

/// What the mission says to fly to, as a screen says it: "take the west tower, 310 m at 120 degrees".
pub fn target_text(m: &Mission) -> Option<String> {
    let (what, at) = match m.order? {
        Order::Take(t) => {
            let (name, at) = m.towers.get(t)?;
            (format!("take the {name} tower"), *at)
        }
        Order::Goto(p) => ("go to a point".to_string(), p),
        Order::Circle(p, _) => ("circle a point".to_string(), p),
        Order::Hold => return Some("hold".into()),
        Order::Home => return Some("go home".into()),
        Order::Land => return Some("land at the base".into()),
        Order::Intercept(_) => return Some("intercept".into()),
    };
    let to = at - m.obs.at;
    // The bearing clockwise from north: the field is north, east.
    let bearing = to.y.atan2(to.x).to_degrees().rem_euclid(360.0);
    Some(format!("{what}, {:.0} m at {bearing:.0} degrees", to.length()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use recreation_towers::EdgeObs;

    fn mission(at: Vec2, alt: f32, vel: Vec2, order: Option<Order>) -> Mission {
        Mission {
            drone: 1,
            order,
            low: false,
            queue: vec![],
            in_contact: true,
            obs: EdgeObs { at, alt, enemies: vec![], towers: vec![], battery: None, silent: 0.0 },
            vel,
            attitude: [0.0; 4],
            towers: vec![("Blue home".into(), Vec2::new(-430.0, 0.0)), ("west".into(), Vec2::new(0.0, -300.0))],
            time: 0.0,
            time_left: 600.0,
            outcome: None,
        }
    }

    #[test]
    fn the_scripted_pilot_banks_climbs_when_low_and_speeds_up_when_slow() {
        let script = Script::bank();
        let low_and_slow = script.sticks(&mission(Vec2::ZERO, 20.0, Vec2::new(0.0, 12.0), None));
        assert_eq!(low_and_slow.roll, 0.3);
        assert!(low_and_slow.pitch > 0.2, "pulled back to climb");
        assert!(low_and_slow.throttle > 0.8);
        let high_and_fast = script.sticks(&mission(Vec2::ZERO, 90.0, Vec2::new(0.0, 28.0), None));
        assert!(high_and_fast.pitch < 0.0, "forward to come down");
        assert!(high_and_fast.throttle < 0.4);
    }

    #[test]
    fn the_target_is_said_with_its_bearing_and_distance() {
        let m = mission(Vec2::ZERO, 50.0, Vec2::ZERO, Some(Order::Take(1)));
        assert_eq!(target_text(&m).unwrap(), "take the west tower, 300 m at 270 degrees");
        let home = mission(Vec2::new(-530.0, 0.0), 50.0, Vec2::ZERO, Some(Order::Take(0)));
        assert_eq!(target_text(&home).unwrap(), "take the Blue home tower, 100 m at 0 degrees");
        assert_eq!(target_text(&mission(Vec2::ZERO, 0.0, Vec2::ZERO, Some(Order::Home))).unwrap(), "go home");
        assert_eq!(target_text(&mission(Vec2::ZERO, 0.0, Vec2::ZERO, None)), None);
    }
}
