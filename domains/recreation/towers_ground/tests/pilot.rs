//! A scripted pilot against a fake game server that speaks only the seat protocol (no SITL): it takes
//! the seat of one drone, sends its sticks at 20 Hz or more, reads the mission, and lets go. A seat
//! that commands, on the same fake, learns which drones a person flies.

use glam::Vec2;
use recreation_towers::seat::{self, Join, Mission, Sticks, ToSeat, ToServer};
use recreation_towers::{EdgeObs, Order, Queued, Rules, Team};
use recreation_towers_ground::pilot::{self, Script};
use recreation_towers_ground::seat::{Seat, Status};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn mission(time: f32, order: Order, in_contact: bool, course: f32) -> Mission {
    let v = Vec2::from_angle((90.0 - course).to_radians()) * 20.0;
    Mission {
        drone: 1,
        order: Some(order),
        low: false,
        queue: vec![Queued::done(Order::Home)],
        in_contact,
        obs: EdgeObs { at: Vec2::ZERO, alt: 30.0, enemies: vec![], towers: vec![], battery: None, silent: 0.0 },
        // The velocity is (north, east): a course of 90 degrees is due east.
        vel: Vec2::new(v.y, v.x),
        attitude: [0.0, 0.0, 0.0, 1.0],
        towers: vec![("west".into(), Vec2::new(0.0, -300.0)), ("east".into(), Vec2::new(0.0, 300.0))],
        time,
        time_left: 600.0 - time,
        outcome: None,
    }
}

#[test]
fn a_scripted_pilot_sends_sticks_at_20_hz_reads_the_mission_and_turns_with_its_bank() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", server.local_addr().unwrap().port());
    let (tx, sticks) = channel::<Sticks>();
    std::thread::spawn(move || {
        let (stream, _) = server.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
        let ToServer::Join(join) = seat::decode(&lines.next().unwrap()).unwrap() else { panic!("not a join") };
        assert_eq!(join, Join::Unit { team: Team::Blue, drone: 1 });
        writeln!(out, "{}", seat::encode(&ToSeat::Welcome { join, rules: Rules::default(), air: vec![] })).unwrap();
        // The mission, 10 a second: the drone turns right 5 degrees each time, and its radio is cut
        // in the middle; the commander's order changes meanwhile, and the screen should not.
        let mut writer = out.try_clone().unwrap();
        std::thread::spawn(move || {
            for k in 0..40 {
                let (order, dark) = match k {
                    0..=14 => (Order::Take(0), false),
                    15..=29 => (Order::Take(1), true),
                    _ => (Order::Take(1), false),
                };
                let shown = if (15..30).contains(&k) { Order::Take(0) } else { order };
                let m = mission(k as f32, shown, !dark, 5.0 * k as f32);
                if writeln!(writer, "{}", seat::encode(&ToSeat::Mission(m))).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        for line in lines {
            if let Ok(ToServer::Sticks(s)) = seat::decode(&line) {
                let _ = tx.send(s);
            }
        }
    });
    let script = Script::bank();
    let started = Instant::now();
    let flown = pilot::fly(&addr, Team::Blue, 1, &script, Duration::from_millis(4200), &AtomicBool::new(false));
    let took = started.elapsed().as_secs_f32();
    assert_eq!(flown.refused, None);
    let sent: Vec<Sticks> = sticks.try_iter().collect();
    assert!(sent.len() as f32 / took >= 20.0, "{} sticks in {took:.1} s: at least 20 a second", sent.len());
    assert!(sent.iter().any(|s| s.roll == script.roll), "the script's bank reached the seat");
    assert!(flown.missions >= 30, "{} missions", flown.missions);
    assert_eq!(flown.orders, vec![Some(Order::Take(0)), Some(Order::Take(1))]);
    assert_eq!(flown.queue, vec![Queued::done(Order::Home)]);
    assert!(flown.turned > 150.0, "turned {} degrees", flown.turned);
    assert_eq!(flown.target.as_deref(), Some("take the east tower, 300 m at 90 degrees"));
    assert!(flown.dark >= 10, "{} missions in the dark", flown.dark);
    assert_eq!(flown.changed_in_the_dark, 0, "the mission did not change in the dark");
}

#[test]
fn a_pilot_that_is_refused_says_why_and_a_seat_that_commands_learns_the_piloted_drones() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", server.local_addr().unwrap().port());
    std::thread::spawn(move || {
        // The first seat is refused, the second is a commander and is told of a pilot.
        let (stream, _) = server.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let _ = BufReader::new(stream).lines().next();
        writeln!(out, "{}", seat::encode(&ToSeat::Refused("Blue2 has its pilot".into()))).unwrap();
        let (stream, _) = server.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut lines = BufReader::new(stream).lines();
        let _ = lines.next();
        let join = Join::Command(Team::Blue);
        writeln!(out, "{}", seat::encode(&ToSeat::Welcome { join, rules: Rules::default(), air: vec![] })).unwrap();
        writeln!(out, "{}", seat::encode(&ToSeat::Piloted(vec![1, 3]))).unwrap();
        for _ in lines {}
    });
    let flown = pilot::fly(&addr, Team::Blue, 1, &Script::bank(), Duration::from_millis(300), &AtomicBool::new(false));
    assert_eq!(flown.refused.as_deref(), Some("Blue2 has its pilot"));
    assert_eq!(flown.sticks_sent, 0);
    let seat = Seat::connect(&addr, Join::Command(Team::Blue));
    let end = Instant::now() + Duration::from_secs(5);
    while seat.state().piloted != vec![1, 3] {
        assert!(Instant::now() < end, "the seat was never told: {:?}", seat.state().status);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(matches!(seat.state().status, Status::Seated(Join::Command(Team::Blue))));
    seat.stop();
}
