//! A computer commander is a seat like a person's: against a fake game server that speaks only the
//! seat protocol (no SITL), it takes the seat as `Join::Compute`, and from the umpire's word and
//! what its sites report it gives orders; it goes on from the orders the drones have, and it leaves
//! when it is relieved. Its advisors run beside it: it never waits for one, and it takes the
//! proposals that arrive.

use recreation_towers::seat::{self, Join, ToSeat, ToServer};
use recreation_towers::{Game, Order, Rules, Team};
use recreation_towers_ground::advisors::AdvisorChoice;
use recreation_towers_ground::commander::{self, Brain};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A fake game server that speaks only the seat protocol, and the computer commander of Blue on it.
struct Fake {
    out: TcpStream,
    rules: Rules,
    game: Game,
    orders: Receiver<(usize, Order, bool)>,
    done: Receiver<()>,
    stop: Arc<AtomicBool>,
    _held: Arc<Mutex<Vec<TcpStream>>>,
}

fn said(out: &mut TcpStream, m: ToSeat) {
    writeln!(out, "{}", seat::encode(&m)).unwrap();
}

impl Fake {
    fn start(advisor: AdvisorChoice) -> Fake {
        let rules = Rules::default();
        // Air ports for the team's drones that accept and say nothing: no telemetry at all.
        let air: Vec<TcpListener> = (0..rules.drones).map(|_| TcpListener::bind("127.0.0.1:0").unwrap()).collect();
        let ports: Vec<u16> = air.iter().map(|l| l.local_addr().unwrap().port()).collect();
        let held: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        for l in air {
            let held = held.clone();
            std::thread::spawn(move || {
                for s in l.incoming().flatten() {
                    held.lock().unwrap().push(s);
                }
            });
        }
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", server.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done) = channel();
        let (a, s) = (addr.clone(), stop.clone());
        std::thread::spawn(move || {
            commander::run_with(&a, Team::Blue, Brain::Play, None, &s, &advisor);
            done_tx.send(()).unwrap();
        });
        let (stream, _) = server.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
        let ToServer::Join(join) = seat::decode(&lines.next().unwrap()).unwrap() else { panic!("not a join") };
        assert_eq!(join, Join::Compute(Team::Blue));
        said(&mut out, ToSeat::Welcome { join, rules, air: ports });
        // The drones have orders already: a person was commanding them.
        said(&mut out, ToSeat::Orders(vec![(Some(Order::Hold), true); rules.drones]));
        let mut game = Game::new(rules);
        game.time = 5.0;
        let (orders_tx, orders) = channel();
        std::thread::spawn(move || {
            for line in lines {
                if let Ok(ToServer::Order { drone, order, low, .. }) = seat::decode::<ToServer>(&line) {
                    let _ = orders_tx.send((drone, order, low));
                }
            }
        });
        Fake { out, rules, game, orders, done, stop, _held: held }
    }

    /// The game goes on 0.25 s, and the seat is told.
    fn tell(&mut self) {
        self.game.time += 0.25;
        said(&mut self.out, ToSeat::Umpire(self.game.umpire()));
        said(&mut self.out, ToSeat::Sites(self.game.sites(Team::Blue)));
    }
}

fn script(name: &str, body: &str) -> String {
    let path = std::env::temp_dir().join(format!("{name}-{}.sh", std::process::id()));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_computer_commander_takes_a_seat_gives_orders_from_the_sites_and_leaves_when_relieved() {
    let mut fake = Fake::start(AdvisorChoice::Lookahead);
    let rules = fake.rules;
    // Tell it the game until it has given every drone an order.
    let end = Instant::now() + Duration::from_secs(10);
    let mut given = std::collections::BTreeMap::new();
    while given.len() < rules.drones {
        assert!(Instant::now() < end, "it gave orders to {given:?}");
        fake.tell();
        while let Ok((drone, order, low)) = fake.orders.recv_timeout(Duration::from_millis(100)) {
            given.insert(drone, (order, low));
        }
    }
    assert!(given.keys().all(|&k| k < rules.drones));
    for (k, (_, low)) in &given {
        assert_eq!(*low, rules.class(*k) == recreation_towers::Class::Light, "drone {k} is flown low if it is light");
    }
    // Relieved by a person: it leaves, and does not come back.
    said(&mut fake.out, ToSeat::Refused("relieved: a person took the seat".into()));
    fake.done.recv_timeout(Duration::from_secs(5)).expect("the commander leaves a seat it is relieved of");
    fake.stop.store(true, Ordering::SeqCst);
}

#[test]
fn a_commander_with_an_advisor_that_never_answers_still_orders_every_game_second() {
    // The advisor is a process that answers the start and then sleeps: the commander must not wait.
    let sleeper = script("advisor-sleeper", "read s; echo '\"ok\"'\nwhile read l; do sleep 60; done");
    let mut fake = Fake::start(AdvisorChoice::Command(sleeper));
    let started = Instant::now();
    let mut batches = 0;
    let mut seen = std::collections::BTreeSet::new();
    // 12 game seconds in about 2 wall seconds.
    for _ in 0..48 {
        fake.tell();
        std::thread::sleep(Duration::from_millis(40));
        while let Ok((drone, ..)) = fake.orders.try_recv() {
            if !seen.insert(drone) {
                batches += 1;
                seen.clear();
                seen.insert(drone);
            }
        }
    }
    assert!(batches >= 8, "{batches} decisions in {:?}: the commander waited for its advisor", started.elapsed());
    fake.stop.store(true, Ordering::SeqCst);
}

#[test]
fn a_proposal_that_arrives_from_a_station_advisor_is_taken_by_the_commander() {
    let east = Game::new(Rules::default()).tower("east");
    // The advisor sends the first drone to the east tower, good for ever. The commander's own
    // order for the first drone is the west tower.
    let advisor = script(
        "advisor-proposes",
        &format!(
            "read s; echo '\"ok\"'\nwhile read l; do echo '[{{\"drone\":0,\"order\":{{\"Take\":{east}}},\"queue\":[],\"gain\":9.0,\"made\":0.0,\"until\":1000.0,\"why\":\"test\"}}]'; done"
        ),
    );
    let mut fake = Fake::start(AdvisorChoice::Command(advisor));
    let end = Instant::now() + Duration::from_secs(15);
    let mut first = vec![];
    loop {
        assert!(Instant::now() < end, "the first drone was given {first:?}");
        fake.tell();
        while let Ok((drone, order, _)) = fake.orders.recv_timeout(Duration::from_millis(60)) {
            if drone == 0 {
                first.push(order);
            }
        }
        if first.last() == Some(&Order::Take(east)) {
            break;
        }
    }
    assert!(first[0] != Order::Take(east), "it started with its own order: {first:?}");
    fake.stop.store(true, Ordering::SeqCst);
}
